//! Small cross-platform helpers shared by providers and core.
use std::path::{Path, PathBuf};
use std::time::Duration;

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

/// Resolve an executable on PATH (handles .cmd/.exe on Windows).
pub fn which(cmd: &str) -> Option<PathBuf> {
    which::which(cmd).ok()
}

/// Run `program args...` with a timeout and return trimmed stdout. Never opens a console window on Windows.
pub async fn run_capture(program: &Path, args: &[&str], timeout: Duration) -> Option<String> {
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = tokio::time::timeout(timeout, cmd.output()).await.ok()?.ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// Shared HTTP client with a sane timeout.
pub fn http() -> reqwest::Client {
    static CLIENT: once_cell::sync::Lazy<reqwest::Client> = once_cell::sync::Lazy::new(|| {
        reqwest::Client::builder().timeout(Duration::from_secs(15)).user_agent(concat!("relay/", env!("CARGO_PKG_VERSION"))).build().expect("http client")
    });
    CLIENT.clone()
}

/// HTTP client for downloads (skill files): the same 15 s to connect, but a whole response may
/// take up to 2 minutes, since the shared client's 15 s limit includes reading the body.
pub fn download_http() -> reqwest::Client {
    static CLIENT: once_cell::sync::Lazy<reqwest::Client> = once_cell::sync::Lazy::new(|| {
        reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(120))
            .user_agent(concat!("relay/", env!("CARGO_PKG_VERSION")))
            .build()
            .expect("http client")
    });
    CLIENT.clone()
}

pub fn read_json(path: &Path) -> Option<serde_json::Value> {
    let s = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&s).ok()
}

/// Write a file atomically (temp file + rename) so a crash never leaves half a credential file.
/// Each call gets its own temp file, so overlapping writes of one path never mix their bytes.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("tmp-{}-{seq}", std::process::id()));
    let result = write_and_rename(&tmp, path, bytes);
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

fn write_and_rename(tmp: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        // Owner-only: adapters write credential files through this.
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(tmp, path)
}

/// The trimmed text, or None when there is none.
pub fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

/// Normalize a path for comparisons (case-insensitive on Windows, no trailing slash).
pub fn path_key(p: &Path) -> String {
    let s = p.to_string_lossy().trim_end_matches(['/', '\\']).replace('\\', "/");
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s
    }
}

/// Serialize `value` as pretty JSON and write it atomically, creating parent dirs.
pub fn write_json_atomic<T: serde::Serialize + ?Sized>(path: &Path, value: &T) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec_pretty(value)?;
    write_atomic(path, &bytes)?;
    Ok(())
}

/// Deep-merge `patch` into `base`: objects merge key by key, anything else (null included) replaces.
pub fn merge_json(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(b), serde_json::Value::Object(p)) => {
            for (k, v) in p {
                match b.get_mut(k) {
                    Some(existing) if existing.is_object() && v.is_object() => merge_json(existing, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, p) => *b = p.clone(),
    }
}

/// Parse an ISO-8601 timestamp into UTC.
pub fn parse_iso(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|d| d.with_timezone(&chrono::Utc))
}

/// Filesystem-safe slug: lowercase ascii letters/digits separated by single dashes.
pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// "in 45 min", "in 3 h", "in 2 days" (or "now" when already past).
pub fn human_until(t: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>) -> String {
    let mins = (t - now).num_minutes();
    match mins {
        m if m <= 0 => "now".into(),
        m if m < 60 => format!("in {m} min"),
        m if m < 48 * 60 => {
            let (h, rem) = (m / 60, m % 60);
            if h < 6 && rem >= 5 {
                format!("in {h} h {rem} min")
            } else {
                format!("in {} h", (m + 30) / 60)
            }
        }
        m => format!("in {} days", (m + 720) / 1440),
    }
}

/// Local wall-clock time: "14:30" today, "Tue 14:30" within a week, else "Oct 27 14:30".
pub fn local_clock(t: chrono::DateTime<chrono::Utc>, now: chrono::DateTime<chrono::Utc>) -> String {
    let local = t.with_timezone(&chrono::Local);
    let today = now.with_timezone(&chrono::Local).date_naive();
    let days = (local.date_naive() - today).num_days();
    if days == 0 {
        local.format("%H:%M").to_string()
    } else if (0..7).contains(&days) {
        local.format("%a %H:%M").to_string()
    } else {
        local.format("%b %-d %H:%M").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_atomic_writes_never_mix() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("accounts.json");
        let threads: Vec<_> = (0..8u8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || (0..20).map(|_| write_atomic(&path, &[i; 64 * 1024]).map_err(|e| e.to_string())).collect::<Vec<_>>())
            })
            .collect();
        for h in threads {
            for r in h.join().unwrap() {
                assert_eq!(r, Ok(()));
            }
        }
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes.len(), 64 * 1024);
        assert!(bytes.iter().all(|b| *b == bytes[0]), "bytes from several writers");
        assert_eq!(std::fs::read_dir(t.path()).unwrap().count(), 1, "temp files left behind");
    }
}

/// GUI apps on macOS start with launchd's PATH (/usr/bin:/bin:/usr/sbin:/sbin) when opened from
/// Finder, the Dock or a login item, so CLIs in Homebrew, ~/.local/bin or npm's prefix look missing
/// and `#!/usr/bin/env node` scripts cannot start.
#[cfg(target_os = "macos")]
pub mod gui_path {
    use std::io::Read;
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::Duration;

    const MARK: &str = "__RELAY_PATH__";

    /// Put the PATH a new terminal would get in front of ours, plus the usual install dirs.
    /// Call first thing in main, before other threads read the environment.
    pub fn fix() {
        let current = std::env::var("PATH").unwrap_or_default();
        let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
        let from_shell = login_shell_path(Path::new(&shell), Duration::from_secs(3));
        let home = super::home();
        let fallbacks: Vec<PathBuf> =
            [home.join(".local/bin"), "/opt/homebrew/bin".into(), "/usr/local/bin".into()].into_iter().filter(|p| p.is_dir()).collect();
        let merged = merge_path(from_shell.as_deref(), &current, &fallbacks);
        if merged != current {
            std::env::set_var("PATH", merged);
        }
    }

    /// `$PATH` from an interactive login shell (which reads .zprofile and .zshrc), or None if it
    /// fails or takes longer than `timeout`. Markers skip anything the rc files print.
    /// `printenv` (not `"$PATH"`) so fish, where PATH is a list, prints it colon-separated too.
    fn login_shell_path(shell: &Path, timeout: Duration) -> Option<String> {
        let mut child = Command::new(shell)
            .args(["-ilc", &format!("printf %s {MARK}; /usr/bin/printenv PATH; printf %s {MARK}")])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        // Read on a helper thread and stop at the closing marker: a background job started by an
        // rc file can keep the pipe open long after the shell exits, so EOF may never come.
        let mut stdout = child.stdout.take()?;
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut out, mut buf) = (Vec::new(), [0u8; 4096]);
            while let Ok(n @ 1..) = stdout.read(&mut buf) {
                out.extend_from_slice(&buf[..n]);
                if let Some(v) = marked_value(&String::from_utf8_lossy(&out)) {
                    let _ = tx.send(v);
                    return;
                }
            }
        });
        let value = rx.recv_timeout(timeout).ok();
        let _ = child.kill();
        let _ = child.wait();
        value
    }

    fn marked_value(out: &str) -> Option<String> {
        let (_, rest) = out.split_once(MARK)?;
        let (value, _) = rest.split_once(MARK)?;
        Some(value.trim().to_string()).filter(|v| !v.is_empty())
    }

    /// Shell entries first, then ours, then fallbacks; no duplicates or empty entries.
    fn merge_path(shell: Option<&str>, current: &str, fallbacks: &[PathBuf]) -> String {
        let fallbacks = fallbacks.iter().map(|p| p.to_string_lossy());
        let mut seen = std::collections::HashSet::new();
        let entries: Vec<String> = shell
            .into_iter()
            .chain(std::iter::once(current))
            .flat_map(|p| p.split(':'))
            .map(str::to_string)
            .chain(fallbacks.map(|p| p.into_owned()))
            .filter(|e| !e.is_empty() && seen.insert(e.clone()))
            .collect();
        entries.join(":")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn marked_value_ignores_shell_noise() {
            let out = format!("Welcome!\n{MARK}/opt/homebrew/bin:/usr/bin{MARK}\nbye\n");
            assert_eq!(marked_value(&out).as_deref(), Some("/opt/homebrew/bin:/usr/bin"));
            assert_eq!(marked_value("no markers here"), None);
            assert_eq!(marked_value(&format!("{MARK}{MARK}")), None);
        }

        #[test]
        fn merge_puts_shell_first_keeps_current_and_adds_fallbacks_once() {
            let merged = merge_path(
                Some("/opt/homebrew/bin:/usr/bin::/Users/me/.local/bin"),
                "/usr/bin:/bin:/Applications/Thing.app/bin",
                &[PathBuf::from("/usr/local/bin"), PathBuf::from("/opt/homebrew/bin")],
            );
            assert_eq!(merged, "/opt/homebrew/bin:/usr/bin:/Users/me/.local/bin:/bin:/Applications/Thing.app/bin:/usr/local/bin");
            assert_eq!(merge_path(None, "/usr/bin:/bin", &[PathBuf::from("/opt/homebrew/bin")]), "/usr/bin:/bin:/opt/homebrew/bin");
        }

        #[test]
        fn reads_path_from_a_login_shell() {
            let path = login_shell_path(Path::new("/bin/zsh"), Duration::from_secs(10)).expect("zsh prints its PATH");
            assert!(path.split(':').any(|d| d == "/usr/bin"), "{path}");
        }

        #[test]
        fn gives_up_on_a_shell_that_hangs() {
            let t = tempfile::tempdir().unwrap();
            let shell = t.path().join("slow-shell");
            std::fs::write(&shell, "#!/bin/sh\nsleep 30\n").unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
            let started = std::time::Instant::now();
            assert_eq!(login_shell_path(&shell, Duration::from_millis(300)), None);
            assert!(started.elapsed() < Duration::from_secs(5));
        }

        #[test]
        fn a_background_job_holding_stdout_does_not_block() {
            let t = tempfile::tempdir().unwrap();
            let shell = t.path().join("rc-daemon-shell");
            let script = format!("#!/bin/sh\nsleep 30 &\nprintf '%s%s%s' {MARK} /from/shell {MARK}\n");
            std::fs::write(&shell, script).unwrap();
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o755)).unwrap();
            let started = std::time::Instant::now();
            assert_eq!(login_shell_path(&shell, Duration::from_secs(2)).as_deref(), Some("/from/shell"));
            assert!(started.elapsed() < Duration::from_secs(5));
        }
    }
}
