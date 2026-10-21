//! Embedded terminals: CLIs running in a pseudo terminal (ConPTY on Windows, openpty on unix)
//! whose output streams to the UI as events. Independent of Tauri: output goes to a [`PtySink`].
//!
//! Per terminal there are four threads:
//! - reader: blocking reads from the pty, forwards raw bytes to the pump;
//! - writer: writes input to the pty in the order it was sent, so a CLI that stops reading
//!   never blocks the caller or lets later keys overtake earlier ones;
//! - waiter: waits for the child to exit and tells the pump the exit code;
//! - pump: batches bytes (~16 ms / 64 KB), decodes UTF-8 across chunk boundaries, keeps the
//!   scrollback and calls the sink. It emits the exit only after all output was delivered.
use crate::launcher;
use crate::model::TerminalInfo;
use crate::providers::CommandSpec;
use anyhow::{anyhow, Context};
use portable_pty::{ChildKiller, CommandBuilder, MasterPty, PtySize};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

pub const BATCH_DELAY: Duration = Duration::from_millis(16);
pub const BATCH_MAX_BYTES: usize = 64 * 1024;
pub const SCROLLBACK_BYTES: usize = 256 * 1024;
/// After the child exits, wait this long for trailing output before closing the pty.
const EXIT_DRAIN: Duration = Duration::from_millis(150);
/// Give up waiting for the reader's EOF this long after the pty was closed.
const EOF_TIMEOUT: Duration = Duration::from_secs(3);
/// How long a closed terminal's CLI gets to exit after SIGHUP before it is killed outright.
const KILL_GRACE: Duration = Duration::from_secs(1);

/// ConPTY asks for the cursor position once at startup and blocks until it gets an answer.
const DSR_REQUEST: &str = "\x1b[6n";
const DSR_REPLY: &[u8] = b"\x1b[1;1R";

/// Where terminal output goes (Tauri events in the app, a buffer in tests).
pub trait PtySink: Send + Sync + 'static {
    fn output(&self, id: &str, data: String);
    fn exit(&self, id: &str, code: Option<i32>);
}

// ---------- UTF-8 decoding across chunk boundaries ----------

/// Decodes a byte stream into text. An incomplete multi-byte sequence at the end of a chunk is
/// kept for the next chunk; invalid bytes become U+FFFD.
#[derive(Default)]
pub struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    pub fn decode(&mut self, bytes: &[u8]) -> String {
        let mut buf = std::mem::take(&mut self.pending);
        buf.extend_from_slice(bytes);
        let mut out = String::with_capacity(buf.len());
        let mut rest: &[u8] = &buf;
        loop {
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    break;
                }
                Err(e) => {
                    let (valid, after) = rest.split_at(e.valid_up_to());
                    // Safe: from_utf8 just validated this prefix.
                    out.push_str(std::str::from_utf8(valid).unwrap_or_default());
                    match e.error_len() {
                        Some(n) => {
                            out.push(char::REPLACEMENT_CHARACTER);
                            rest = &after[n..];
                        }
                        None => {
                            self.pending = after.to_vec(); // incomplete, at most 3 bytes
                            break;
                        }
                    }
                }
            }
        }
        out
    }

    /// End of stream: whatever is still pending is invalid.
    pub fn finish(&mut self) -> String {
        let rest = std::mem::take(&mut self.pending);
        String::from_utf8_lossy(&rest).into_owned()
    }
}

// ---------- output batching ----------

/// Collects bytes until the batch is BATCH_DELAY old or BATCH_MAX_BYTES big.
#[derive(Default)]
pub struct Batcher {
    buf: Vec<u8>,
    started: Option<Instant>,
}

impl Batcher {
    /// Add bytes; true when the batch should be flushed now (size limit).
    pub fn push(&mut self, bytes: &[u8], now: Instant) -> bool {
        if self.buf.is_empty() {
            self.started = Some(now);
        }
        self.buf.extend_from_slice(bytes);
        self.buf.len() >= BATCH_MAX_BYTES
    }

    /// Time left until the current batch is due; None when empty.
    pub fn time_left(&self, now: Instant) -> Option<Duration> {
        self.started.map(|s| (s + BATCH_DELAY).saturating_duration_since(now))
    }

    pub fn take(&mut self) -> Vec<u8> {
        self.started = None;
        std::mem::take(&mut self.buf)
    }
}

// ---------- scrollback ----------

/// Recent output, capped at roughly `cap` bytes. Trims in steps (not on every append) and,
/// when it can, at a line start so a restored pane does not begin mid escape sequence.
pub struct Scrollback {
    text: String,
    cap: usize,
}

impl Scrollback {
    pub fn new(cap: usize) -> Self {
        Self { text: String::new(), cap }
    }

    pub fn push(&mut self, s: &str) {
        self.text.push_str(s);
        if self.text.len() <= self.cap + self.cap / 4 {
            return;
        }
        let mut cut = self.text.len() - self.cap;
        while !self.text.is_char_boundary(cut) {
            cut += 1;
        }
        let window = &self.text[cut..self.text.len().min(cut + 4096)];
        if let Some(nl) = window.find('\n') {
            cut += nl + 1;
        }
        self.text.drain(..cut);
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

// ---------- spawning ----------

/// What to run. `spec.cwd` should already be resolved to an existing directory.
pub struct PtySpawn {
    pub account_id: String,
    pub title: String,
    pub spec: CommandSpec,
    pub cols: u16,
    pub rows: u16,
}

struct Session {
    info: TerminalInfo,
    /// Dropped to close the pty (on Windows that is what ends the reader).
    master: Option<Box<dyn MasterPty + Send>>,
    /// Feeds the writer thread.
    input: Sender<Vec<u8>>,
    killer: Box<dyn ChildKiller + Send + Sync>,
    /// Set by the waiter as soon as the child has exited.
    exited: Arc<AtomicBool>,
    scrollback: Scrollback,
}

/// All live (and exited but not yet closed) terminals. Cheap to clone.
#[derive(Clone, Default)]
pub struct Terminals {
    map: Arc<Mutex<HashMap<String, Session>>>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn is_batch_file(program: &str) -> bool {
    let p = program.to_ascii_lowercase();
    p.ends_with(".cmd") || p.ends_with(".bat") || p.ends_with(".ps1")
}

/// Full path of the program when it can be found, so we know whether it is a .cmd shim.
fn resolve_program(program: &str) -> String {
    let path = std::path::Path::new(program);
    if path.is_absolute() && path.is_file() {
        return program.to_string();
    }
    crate::util::which(program).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| program.to_string())
}

/// Builds the pty command. Windows batch/PowerShell shims run through a tiny .cmd script so
/// their arguments get batch-file quoting (reusing the launcher's tested quoting); the script
/// path is returned so it can be removed after exit.
fn build_command(spec: &CommandSpec) -> anyhow::Result<(CommandBuilder, Option<PathBuf>)> {
    let program = resolve_program(&spec.program);
    let mut script = None;
    let mut cmd = if cfg!(windows) && is_batch_file(&program) {
        let resolved = CommandSpec { program, args: spec.args.clone(), ..Default::default() };
        let body = format!("@echo off\r\n{}\r\nexit /b %ERRORLEVEL%\r\n", launcher::windows_command_line(&resolved));
        let dir = std::env::temp_dir().join("switchyard");
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        let path = dir.join(format!("pty-{}.cmd", uuid::Uuid::new_v4().simple()));
        std::fs::write(&path, body).with_context(|| format!("writing {}", path.display()))?;
        let mut c = CommandBuilder::new("cmd.exe");
        c.args(["/d", "/c"]);
        c.arg(&path);
        script = Some(path);
        c
    } else {
        let mut c = CommandBuilder::new(&program);
        c.args(&spec.args);
        c
    };
    for (k, v) in &spec.env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        }
    }
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    if let Some(cwd) = &spec.cwd {
        cmd.cwd(cwd);
    }
    Ok((cmd, script))
}

fn pty_size(cols: u16, rows: u16) -> PtySize {
    PtySize { rows: rows.clamp(2, 500), cols: cols.clamp(10, 1000), pixel_width: 0, pixel_height: 0 }
}

enum Msg {
    Data(Vec<u8>),
    Eof,
    Exited(Option<i32>),
}

impl Terminals {
    pub fn spawn(&self, req: PtySpawn, sink: Arc<dyn PtySink>) -> anyhow::Result<TerminalInfo> {
        let pair = portable_pty::native_pty_system()
            .openpty(pty_size(req.cols, req.rows))
            .map_err(|e| anyhow!("could not open a terminal: {e}"))?;
        let (cmd, script) = build_command(&req.spec)?;
        let spawned = pair.slave.spawn_command(cmd);
        drop(pair.slave); // the child holds its own handle; ours would keep the pty open
        let mut child = match spawned {
            Ok(c) => c,
            Err(e) => {
                if let Some(s) = &script {
                    let _ = std::fs::remove_file(s);
                }
                return Err(anyhow!("could not start {}: {e}", req.spec.program));
            }
        };
        let reader = pair.master.try_clone_reader().map_err(|e| anyhow!("terminal reader: {e}"))?;
        let input = spawn_writer(pair.master.take_writer().map_err(|e| anyhow!("terminal writer: {e}"))?);
        let exited = Arc::new(AtomicBool::new(false));

        let id = uuid::Uuid::new_v4().to_string();
        let info = TerminalInfo {
            id: id.clone(),
            account_id: req.account_id,
            title: req.title,
            command_line: launcher::display_command_line(&req.spec, launcher::Os::current()),
            cwd: req.spec.cwd.clone().unwrap_or_default(),
            pid: child.process_id(),
            started_at: crate::util::now_iso(),
            exited: false,
            exit_code: None,
        };
        let session = Session {
            info: info.clone(),
            master: Some(pair.master),
            input: input.clone(),
            killer: child.clone_killer(),
            exited: exited.clone(),
            scrollback: Scrollback::new(SCROLLBACK_BYTES),
        };
        lock(&self.map).insert(id.clone(), session);

        let (tx, rx) = mpsc::channel();
        spawn_reader(reader, tx.clone());
        let wait_tx = tx;
        std::thread::spawn(move || {
            let code = child.wait().ok().map(|s| s.exit_code() as i32);
            exited.store(true, Ordering::SeqCst);
            let _ = wait_tx.send(Msg::Exited(code));
        });
        let pump = Pump { id, terminals: self.clone(), sink, input, script };
        std::thread::spawn(move || pump.run(rx));
        Ok(info)
    }

    /// Queue input for the terminal; returns at once, the writer thread delivers it in order.
    pub fn write(&self, id: &str, data: &[u8]) -> anyhow::Result<()> {
        let map = lock(&self.map);
        let session = map.get(id).ok_or_else(|| anyhow!("Terminal not found"))?;
        session.input.send(data.to_vec()).map_err(|_| anyhow!("The terminal is closed"))
    }

    pub fn resize(&self, id: &str, cols: u16, rows: u16) -> anyhow::Result<()> {
        let map = lock(&self.map);
        let session = map.get(id).ok_or_else(|| anyhow!("Terminal not found"))?;
        match &session.master {
            Some(m) => m.resize(pty_size(cols, rows)).map_err(|e| anyhow!("resizing the terminal: {e}")),
            None => Ok(()), // already exited
        }
    }

    /// Kill the process (if still running) and forget the terminal.
    pub fn kill(&self, id: &str) -> anyhow::Result<()> {
        let session = lock(&self.map).remove(id).ok_or_else(|| anyhow!("Terminal not found"))?;
        if let Some(stopping) = close_session(session) {
            std::thread::spawn(move || kill_after_grace(vec![stopping]));
        }
        Ok(())
    }

    /// On quit: waits up to KILL_GRACE for the CLIs to exit, then kills the rest.
    pub fn kill_all(&self) {
        let sessions: Vec<Session> = lock(&self.map).drain().map(|(_, s)| s).collect();
        kill_after_grace(sessions.into_iter().filter_map(close_session).collect());
    }

    pub fn list(&self) -> Vec<TerminalInfo> {
        let mut v: Vec<TerminalInfo> = lock(&self.map).values().map(|s| s.info.clone()).collect();
        v.sort_by(|a, b| a.started_at.cmp(&b.started_at));
        v
    }

    pub fn scrollback(&self, id: &str) -> Option<String> {
        lock(&self.map).get(id).map(|s| s.scrollback.text().to_string())
    }

    fn with_session(&self, id: &str, f: impl FnOnce(&mut Session)) {
        if let Some(s) = lock(&self.map).get_mut(id) {
            f(s);
        }
    }

    /// Close the pty after the child exited so the reader sees EOF (needed with ConPTY).
    fn close_pty(&self, id: &str) {
        let master = lock(&self.map).get_mut(id).and_then(|s| s.master.take());
        drop(master); // outside the lock: ClosePseudoConsole can take a moment
    }
}

/// A child that was asked to stop: its pid and whether the waiter has seen it exit.
type Stopping = (Option<u32>, Arc<AtomicBool>);

/// Ask a running child to stop (SIGHUP on unix, TerminateProcess on Windows) and close the pty.
fn close_session(mut s: Session) -> Option<Stopping> {
    let running = !s.exited.load(Ordering::SeqCst);
    if running {
        let _ = s.killer.kill();
    }
    drop(s.master.take());
    running.then(|| (s.info.pid, s.exited.clone()))
}

/// Give stopping children KILL_GRACE to exit, then SIGKILL the rest with their process groups:
/// a CLI that traps or ignores SIGHUP would otherwise outlive its pane, still signed in.
fn kill_after_grace(children: Vec<Stopping>) {
    let deadline = Instant::now() + KILL_GRACE;
    while Instant::now() < deadline && children.iter().any(|(_, exited)| !exited.load(Ordering::SeqCst)) {
        std::thread::sleep(Duration::from_millis(20));
    }
    #[cfg(unix)]
    for (pid, exited) in children {
        // Not exited means not yet reaped by the waiter, so the pid still names our child. It leads
        // its own session (setsid), so the pid is also its process group id.
        if let (Some(pid), false) = (pid.and_then(|p| i32::try_from(p).ok()), exited.load(Ordering::SeqCst)) {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
                libc::kill(pid, libc::SIGKILL);
            }
        }
    }
}

/// One thread per terminal writes its input, so chunks reach the pty in the order they were sent.
fn spawn_writer(mut writer: Box<dyn Write + Send>) -> Sender<Vec<u8>> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        for data in rx {
            if writer.write_all(&data).and_then(|_| writer.flush()).is_err() {
                break; // the pty is gone
            }
        }
    });
    tx
}

fn spawn_reader(mut reader: Box<dyn Read + Send>, tx: Sender<Msg>) {
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(Msg::Data(buf[..n].to_vec())).is_err() {
                        return;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break, // EIO on unix when the child side closes
            }
        }
        let _ = tx.send(Msg::Eof);
    });
}

struct Pump {
    id: String,
    terminals: Terminals,
    sink: Arc<dyn PtySink>,
    input: Sender<Vec<u8>>,
    script: Option<PathBuf>,
}

impl Pump {
    fn run(self, rx: Receiver<Msg>) {
        let mut batch = Batcher::default();
        let mut decoder = Utf8Decoder::default();
        let mut dsr_answered = false;
        let mut eof = false;
        let mut exit: Option<Option<i32>> = None;
        let mut exited_at: Option<Instant> = None;
        let mut pty_closed_at: Option<Instant> = None;

        while !(eof && exit.is_some()) {
            let now = Instant::now();
            // After exit: wait briefly for trailing output, then close the pty to end the reader.
            if let (Some(t), None) = (exited_at, pty_closed_at) {
                if now >= t + EXIT_DRAIN {
                    self.terminals.close_pty(&self.id);
                    pty_closed_at = Some(now);
                }
            }
            if pty_closed_at.is_some_and(|t| now >= t + EOF_TIMEOUT) {
                break; // a grandchild still holds the pty; do not hang forever
            }
            let timeout = batch.time_left(now).unwrap_or(Duration::from_millis(50)).min(Duration::from_millis(50));
            match rx.recv_timeout(timeout) {
                Ok(Msg::Data(bytes)) => {
                    if batch.push(&bytes, Instant::now()) {
                        self.flush(&mut batch, &mut decoder, &mut dsr_answered);
                    }
                    continue;
                }
                Ok(Msg::Eof) => eof = true,
                Ok(Msg::Exited(code)) => {
                    exit = Some(code);
                    exited_at = Some(Instant::now());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if batch.time_left(Instant::now()).is_some_and(|d| d.is_zero()) {
                self.flush(&mut batch, &mut decoder, &mut dsr_answered);
            }
        }
        self.flush(&mut batch, &mut decoder, &mut dsr_answered);
        let tail = decoder.finish();
        if !tail.is_empty() {
            self.deliver(tail);
        }
        self.finish(exit.flatten());
    }

    fn flush(&self, batch: &mut Batcher, decoder: &mut Utf8Decoder, dsr_answered: &mut bool) {
        let bytes = batch.take();
        if bytes.is_empty() {
            return;
        }
        let mut text = decoder.decode(&bytes);
        if !*dsr_answered {
            if let Some(pos) = text.find(DSR_REQUEST) {
                text.replace_range(pos..pos + DSR_REQUEST.len(), "");
                let _ = self.input.send(DSR_REPLY.to_vec());
                *dsr_answered = true;
            }
        }
        if !text.is_empty() {
            self.deliver(text);
        }
    }

    fn deliver(&self, text: String) {
        self.terminals.with_session(&self.id, |s| s.scrollback.push(&text));
        self.sink.output(&self.id, text);
    }

    fn finish(self, code: Option<i32>) {
        self.terminals.close_pty(&self.id);
        self.terminals.with_session(&self.id, |s| {
            s.info.exited = true;
            s.info.exit_code = code;
        });
        if let Some(s) = &self.script {
            let _ = std::fs::remove_file(s);
        }
        self.sink.exit(&self.id, code);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoder_keeps_split_sequences() {
        let text = "héllo €𝄞!";
        let bytes = text.as_bytes();
        // Split at every possible position, including inside multi-byte characters.
        for i in 0..=bytes.len() {
            let mut d = Utf8Decoder::default();
            let mut out = d.decode(&bytes[..i]);
            out += &d.decode(&bytes[i..]);
            out += &d.finish();
            assert_eq!(out, text, "split at {i}");
        }
        // Byte by byte.
        let mut d = Utf8Decoder::default();
        let out: String = bytes.iter().map(|b| d.decode(&[*b])).collect();
        assert_eq!(out, text);
    }

    #[test]
    fn decoder_replaces_invalid_bytes() {
        let mut d = Utf8Decoder::default();
        assert_eq!(d.decode(b"a\xffb\xc3"), "a\u{FFFD}b");
        assert_eq!(d.decode(b"("), "\u{FFFD}(");
        let mut d = Utf8Decoder::default();
        assert_eq!(d.decode(b"x\xe2\x82"), "x");
        assert_eq!(d.finish(), "\u{FFFD}");
    }

    #[test]
    fn batcher_flushes_on_time_and_size() {
        let t0 = Instant::now();
        let mut b = Batcher::default();
        assert_eq!(b.time_left(t0), None);
        assert!(!b.push(b"abc", t0));
        assert_eq!(b.time_left(t0), Some(BATCH_DELAY));
        assert!(!b.push(b"def", t0 + Duration::from_millis(10)));
        assert_eq!(b.time_left(t0 + Duration::from_millis(10)), Some(Duration::from_millis(6)));
        assert_eq!(b.time_left(t0 + Duration::from_millis(30)), Some(Duration::ZERO));
        assert_eq!(b.take(), b"abcdef");
        assert_eq!(b.time_left(t0), None);
        assert!(b.push(&vec![b'x'; BATCH_MAX_BYTES], t0));
    }

    #[test]
    fn scrollback_is_capped_at_line_start() {
        let mut s = Scrollback::new(100);
        for i in 0..100 {
            s.push(&format!("line {i:03}\n"));
        }
        let t = s.text();
        assert!(t.len() <= 125, "len {}", t.len());
        assert!(t.starts_with("line "));
        assert!(t.ends_with("line 099\n"));
        // Never splits a multi-byte character.
        let mut s = Scrollback::new(10);
        s.push(&"é".repeat(50));
        assert!(s.text().chars().all(|c| c == 'é'));
    }

    #[derive(Default)]
    struct TestSink {
        out: Mutex<String>,
        exit: Mutex<Option<Option<i32>>>,
    }

    impl PtySink for TestSink {
        fn output(&self, _id: &str, data: String) {
            lock(&self.out).push_str(&data);
        }
        fn exit(&self, _id: &str, code: Option<i32>) {
            *lock(&self.exit) = Some(code);
        }
    }

    fn run_to_exit(spec: CommandSpec) -> (String, Option<i32>, Terminals, String) {
        let terminals = Terminals::default();
        let sink = Arc::new(TestSink::default());
        let req = PtySpawn { account_id: "a".into(), title: "t".into(), spec, cols: 80, rows: 24 };
        let info = terminals.spawn(req, sink.clone()).expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(20);
        while lock(&sink.exit).is_none() {
            assert!(Instant::now() < deadline, "terminal did not exit; output: {:?}", lock(&sink.out));
            std::thread::sleep(Duration::from_millis(20));
        }
        let out = lock(&sink.out).clone();
        let code = lock(&sink.exit).flatten();
        (out, code, terminals, info.id)
    }

    fn echo_spec() -> CommandSpec {
        let (program, args) = if cfg!(windows) {
            ("cmd.exe", vec!["/c".to_string(), "echo hello & exit 3".to_string()])
        } else {
            ("sh", vec!["-c".to_string(), "echo hello; exit 3".to_string()])
        };
        CommandSpec { program: program.into(), args, env: HashMap::new(), cwd: None, note: None }
    }

    #[test]
    fn spawns_and_streams_output() {
        let (out, code, terminals, id) = run_to_exit(echo_spec());
        assert!(out.contains("hello"), "output: {out:?}");
        assert_eq!(code, Some(3));
        let info = terminals.list().into_iter().find(|t| t.id == id).expect("kept after exit");
        assert!(info.exited);
        assert_eq!(info.exit_code, Some(3));
        assert!(terminals.scrollback(&id).unwrap_or_default().contains("hello"));
        terminals.kill(&id).expect("remove");
        assert!(terminals.list().is_empty());
    }

    /// npm-style .cmd shim in a folder with spaces, with env set and removed.
    #[cfg(windows)]
    #[test]
    fn runs_cmd_shims_with_env() {
        let t = tempfile::tempdir().expect("tempdir");
        let dir = t.path().join("my tools");
        std::fs::create_dir_all(&dir).expect("mkdir");
        let shim = dir.join("tool.cmd");
        std::fs::write(&shim, "@echo off\r\necho [%MYVAR%] [%~1] [%~2] [%GONE%] [%TERM%]\r\nexit /b 5\r\n").expect("write");
        std::env::set_var("GONE", "still here");
        let spec = CommandSpec {
            program: shim.to_string_lossy().into_owned(),
            args: vec!["a b".into(), "100%".into()],
            env: HashMap::from([("MYVAR".to_string(), Some("x y".to_string())), ("GONE".to_string(), None)]),
            cwd: Some(dir.to_string_lossy().into_owned()),
            note: None,
        };
        let (out, code, _, _) = run_to_exit(spec);
        assert!(out.contains("[x y] [a b] [100%] [] [xterm-256color]"), "output: {out:?}");
        assert_eq!(code, Some(5));
    }

    #[test]
    fn write_and_kill_a_running_terminal() {
        let (program, args) = if cfg!(windows) { ("cmd.exe", vec!["/q".to_string()]) } else { ("sh", vec![]) };
        let terminals = Terminals::default();
        let sink = Arc::new(TestSink::default());
        let spec = CommandSpec { program: program.into(), args, ..Default::default() };
        let info = terminals.spawn(PtySpawn { account_id: "a".into(), title: "t".into(), spec, cols: 80, rows: 24 }, sink.clone()).expect("spawn");
        terminals.resize(&info.id, 100, 30).expect("resize");
        terminals.write(&info.id, b"echo typed-in\r").expect("write");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !lock(&sink.out).contains("typed-in") {
            assert!(Instant::now() < deadline, "no echo; output: {:?}", lock(&sink.out));
            std::thread::sleep(Duration::from_millis(20));
        }
        terminals.kill(&info.id).expect("kill");
        assert!(terminals.list().is_empty());
        let deadline = Instant::now() + Duration::from_secs(20);
        while lock(&sink.exit).is_none() {
            assert!(Instant::now() < deadline, "no exit after kill");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    /// A CLI that is not reading its input must not make writes block (or reorder later keys).
    #[cfg(unix)]
    #[test]
    fn writes_return_while_the_child_is_not_reading() {
        let terminals = Terminals::default();
        let sink = Arc::new(TestSink::default());
        let spec = CommandSpec { program: "sh".into(), args: vec!["-c".into(), "stty raw -echo; echo ready; sleep 5".into()], ..Default::default() };
        let info = terminals.spawn(PtySpawn { account_id: "a".into(), title: "t".into(), spec, cols: 80, rows: 24 }, sink.clone()).expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !lock(&sink.out).contains("ready") {
            assert!(Instant::now() < deadline, "no output: {:?}", lock(&sink.out));
            std::thread::sleep(Duration::from_millis(20));
        }
        let started = Instant::now();
        for _ in 0..64 {
            terminals.write(&info.id, &[b'x'; 16 * 1024]).expect("write");
        }
        assert!(started.elapsed() < Duration::from_secs(2), "writes blocked for {:?}", started.elapsed());
        terminals.kill(&info.id).expect("kill");
    }

    /// Closing a pane ends a CLI even when it ignores SIGHUP.
    #[cfg(unix)]
    #[test]
    fn kill_escalates_when_hangup_is_ignored() {
        let terminals = Terminals::default();
        let sink = Arc::new(TestSink::default());
        let spec = CommandSpec { program: "sh".into(), args: vec!["-c".into(), "trap '' HUP; echo ready; sleep 30".into()], ..Default::default() };
        let info = terminals.spawn(PtySpawn { account_id: "a".into(), title: "t".into(), spec, cols: 80, rows: 24 }, sink.clone()).expect("spawn");
        let deadline = Instant::now() + Duration::from_secs(20);
        while !lock(&sink.out).contains("ready") {
            assert!(Instant::now() < deadline, "no output: {:?}", lock(&sink.out));
            std::thread::sleep(Duration::from_millis(20));
        }
        let started = Instant::now();
        terminals.kill(&info.id).expect("kill");
        while lock(&sink.exit).is_none() {
            assert!(started.elapsed() < Duration::from_secs(8), "still running after kill");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
