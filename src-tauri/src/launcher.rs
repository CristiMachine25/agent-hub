//! Opens a new visible terminal window running a CommandSpec with its env and cwd, or spawns a
//! desktop app detached. Script/argv builders are pure functions so they can be tested on any OS.
//!
//! Every terminal launch goes through a small temp script (.cmd on Windows, .command/.sh on unix)
//! that sets the env, changes directory and runs the command. That avoids fragile nested quoting
//! through `wt`/`cmd`/`open` and works even when Windows Terminal is already running (new tabs do
//! not reliably inherit the caller's environment).
use crate::model::LaunchResult;
use crate::providers::CommandSpec;
use anyhow::Context;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Linux
        }
    }
}

/// Sorted env entries so scripts and display strings are deterministic.
fn env_sorted(spec: &CommandSpec) -> Vec<(&String, &Option<String>)> {
    let mut v: Vec<_> = spec.env.iter().collect();
    v.sort_by(|a, b| a.0.cmp(b.0));
    v
}

// ---------- quoting ----------

/// POSIX single-quote quoting.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:,+@%".contains(c)) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Quote one argument for a line inside a .cmd batch file. `%` is doubled because batch files
/// expand it; anything with spaces or cmd metacharacters is wrapped in double quotes.
pub fn cmd_quote(s: &str) -> String {
    let s = s.replace('"', "").replace('%', "%%");
    if !s.is_empty() && !s.chars().any(|c| c.is_whitespace() || "&|<>^(),;=!".contains(c)) {
        return s;
    }
    format!("\"{s}\"")
}

/// Human-readable command line (env + command), shown to the user for transparency.
pub fn display_command_line(spec: &CommandSpec, os: Os) -> String {
    let mut parts = Vec::new();
    for (k, v) in env_sorted(spec) {
        match (os, v) {
            (Os::Windows, Some(v)) => parts.push(format!("set \"{k}={v}\" &&")),
            (Os::Windows, None) => parts.push(format!("set \"{k}=\" &&")),
            (_, Some(v)) => parts.push(format!("{k}={}", sh_quote(v))),
            (_, None) => parts.push(format!("-u {k}")),
        }
    }
    if os != Os::Windows && spec.env.values().any(Option::is_none) {
        parts.insert(0, "env".into());
    }
    let quote = |s: &str| if os == Os::Windows { cmd_quote(s).replace("%%", "%") } else { sh_quote(s) };
    parts.push(quote(&spec.program));
    parts.extend(spec.args.iter().map(|a| quote(a)));
    parts.join(" ")
}

// ---------- Windows ----------

/// Command line for the program inside a batch file. `call` makes .cmd shims (npm's gemini.cmd)
/// return to the script; .ps1 shims run through PowerShell.
pub fn windows_command_line(spec: &CommandSpec) -> String {
    let lower = spec.program.to_ascii_lowercase();
    let ps1 = lower.ends_with(".ps1");
    // `call` expands percent signs a second time, so they need doubling once more.
    let quote = |s: &str| if ps1 { cmd_quote(s) } else { cmd_quote(s).replace('%', "%%") };
    let args: Vec<String> = spec.args.iter().map(|a| quote(a)).collect();
    let head = if ps1 {
        format!("powershell -NoLogo -NoProfile -ExecutionPolicy Bypass -File {}", cmd_quote(&spec.program))
    } else {
        format!("call {}", quote(&spec.program))
    };
    std::iter::once(head).chain(args).collect::<Vec<_>>().join(" ")
}

/// The .cmd script run by `cmd /k`. UTF-8 code page so non-ASCII paths survive.
pub fn windows_script(spec: &CommandSpec, title: &str) -> String {
    let mut lines = vec!["@echo off".to_string(), "chcp 65001 >nul".to_string()];
    lines.push(format!("title {}", sanitize_title(title).replace('%', "%%")));
    for (k, v) in env_sorted(spec) {
        let v = v.as_deref().unwrap_or("").replace('%', "%%").replace('"', "");
        lines.push(format!("set \"{k}={v}\""));
    }
    if let Some(cwd) = &spec.cwd {
        lines.push(format!("cd /d {}", cmd_quote(cwd)));
    }
    lines.push(windows_command_line(spec));
    lines.join("\r\n") + "\r\n"
}

/// Raw (already quoted) arguments for `wt.exe` or for `cmd.exe` in a new console.
/// Returns (program, raw args).
pub fn windows_terminal_argv(use_wt: bool, script: &Path, title: &str, cwd: Option<&str>) -> (String, String) {
    let script = script.to_string_lossy();
    if use_wt {
        let mut raw = String::from("-w new");
        raw.push_str(&format!(" --title \"{}\"", sanitize_title(title)));
        if let Some(cwd) = cwd {
            // A trailing backslash would escape the closing quote; "C:" alone would mean "cwd on C:".
            let mut dir = cwd.trim_end_matches('\\').to_string();
            if dir.ends_with(':') {
                dir.push_str("\\.");
            }
            raw.push_str(&format!(" -d \"{dir}\""));
        }
        raw.push_str(&format!(" cmd.exe /k \"{script}\""));
        ("wt.exe".into(), raw)
    } else {
        // `start` opens a fresh, fully interactive console window. Spawning cmd.exe directly with
        // CREATE_NEW_CONSOLE from a GUI app gave the CLI no usable stdin, so it exited at once.
        // /s strips only the outer quotes, leaving `start "title" cmd.exe /k "script"`.
        let title = sanitize_title(title).replace(['%', '^', '&', '|', '<', '>'], "");
        ("cmd.exe".into(), format!("/d /s /c \"start \"{title}\" cmd.exe /k \"{script}\"\""))
    }
}

/// Titles travel through wt's own parser where `;` separates commands.
fn sanitize_title(t: &str) -> String {
    t.chars().filter(|c| !matches!(c, '"' | ';' | '\r' | '\n')).collect()
}

// ---------- macOS / Linux ----------

/// Shell script for unix terminals: exports, cd, run the command, then keep an interactive shell
/// open (like `cmd /k`) so login output and errors stay visible. The script deletes itself.
pub fn unix_script(spec: &CommandSpec, shebang: &str) -> String {
    let mut lines = vec![shebang.to_string(), r#"rm -f "$0" 2>/dev/null"#.to_string()];
    for (k, v) in env_sorted(spec) {
        match v {
            Some(v) => lines.push(format!("export {k}={}", sh_quote(v))),
            None => lines.push(format!("unset {k}")),
        }
    }
    if let Some(cwd) = &spec.cwd {
        lines.push(format!("cd {} || exit 1", sh_quote(cwd)));
    }
    let cmd: Vec<String> = std::iter::once(&spec.program).chain(&spec.args).map(|s| sh_quote(s)).collect();
    lines.push(cmd.join(" "));
    lines.push(r#"exec "${SHELL:-/bin/bash}" -l"#.to_string());
    lines.join("\n") + "\n"
}

/// `open` argv for macOS. `terminal` is the settings value: auto/terminal, iterm, or an app name.
pub fn mac_open_argv(terminal: &str, script: &Path) -> Vec<String> {
    let app = match terminal.trim().to_ascii_lowercase().as_str() {
        "" | "auto" | "terminal" => "Terminal".to_string(),
        "iterm" | "iterm2" => "iTerm".to_string(),
        _ => terminal.trim().to_string(),
    };
    vec!["open".into(), "-a".into(), app, script.to_string_lossy().into_owned()]
}

pub const LINUX_TERMINALS: &[&str] = &[
    "x-terminal-emulator",
    "gnome-terminal",
    "konsole",
    "xfce4-terminal",
    "kitty",
    "alacritty",
    "wezterm",
    "foot",
    "xterm",
];

/// argv to run `bash -l <script>` in the given Linux terminal.
pub fn linux_terminal_argv(terminal: &str, script: &Path) -> Vec<String> {
    let run = vec!["bash".to_string(), "-l".to_string(), script.to_string_lossy().into_owned()];
    let name = Path::new(terminal).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let prefix: Vec<&str> = match name.as_str() {
        "gnome-terminal" | "ptyxis" => vec!["--"],
        "xfce4-terminal" => vec!["-x"],
        "kitty" | "foot" => vec![],
        "wezterm" => vec!["start", "--"],
        _ => vec!["-e"], // x-terminal-emulator, konsole, alacritty, xterm, most others
    };
    std::iter::once(terminal.to_string()).chain(prefix.into_iter().map(String::from)).chain(run).collect()
}

// ---------- process spawning ----------

fn scripts_dir() -> anyhow::Result<PathBuf> {
    let dir = std::env::temp_dir().join("baton");
    std::fs::create_dir_all(&dir)?;
    cleanup_old_scripts(&dir);
    Ok(dir)
}

/// Windows scripts cannot delete themselves while cmd runs them; sweep ones older than a day.
fn cleanup_old_scripts(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let day = std::time::Duration::from_secs(86_400);
    for e in entries.filter_map(Result::ok) {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if old {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn write_script(ext: &str, content: &str) -> anyhow::Result<PathBuf> {
    let path = scripts_dir()?.join(format!("launch-{}.{ext}", uuid::Uuid::new_v4().simple()));
    std::fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}

fn apply_env(cmd: &mut Command, spec: &CommandSpec) {
    for (k, v) in &spec.env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
}

/// Spawn and reap in the background so no zombie is left behind.
fn spawn_detached(mut cmd: Command) -> anyhow::Result<()> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Open a new terminal window running `spec`. `terminal` is AppSettings.terminal.
pub fn launch_in_terminal(spec: &CommandSpec, title: &str, terminal: &str) -> LaunchResult {
    let shown = display_command_line(spec, Os::current());
    match launch_terminal_inner(spec, title, terminal) {
        Ok(()) => LaunchResult { ok: true, error: None, command_line: Some(shown) },
        Err(e) => LaunchResult { ok: false, error: Some(format!("{e:#}")), command_line: Some(shown) },
    }
}

/// Start a GUI app directly with the account env, no terminal.
pub fn launch_desktop(spec: &CommandSpec) -> LaunchResult {
    let shown = display_command_line(spec, Os::current());
    let mut cmd = Command::new(&spec.program);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.current_dir(cwd);
    }
    apply_env(&mut cmd, spec);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    match spawn_detached(cmd) {
        Ok(()) => LaunchResult { ok: true, error: None, command_line: Some(shown) },
        Err(e) => LaunchResult {
            ok: false,
            error: Some(format!("Could not start {}: {e}", spec.program)),
            command_line: Some(shown),
        },
    }
}

#[cfg(windows)]
fn launch_terminal_inner(spec: &CommandSpec, title: &str, terminal: &str) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let script = write_script("cmd", &windows_script(spec, title))?;
    let t = terminal.trim().to_ascii_lowercase();
    let want_wt = matches!(t.as_str(), "auto" | "" | "wt" | "windows-terminal" | "windowsterminal");
    let use_wt = want_wt && crate::util::which("wt").is_some();
    let (program, raw) = windows_terminal_argv(use_wt, &script, title, spec.cwd.as_deref());
    let mut cmd = Command::new(&program);
    cmd.raw_arg(&raw);
    if let Some(cwd) = spec.cwd.as_deref().filter(|c| Path::new(c).is_dir()) {
        cmd.current_dir(cwd);
    }
    apply_env(&mut cmd, spec);
    // Neither wt.exe nor the helper cmd.exe that runs `start` needs a window of its own.
    cmd.creation_flags(CREATE_NO_WINDOW);
    spawn_detached(cmd).with_context(|| format!("starting {program}"))
}

#[cfg(target_os = "macos")]
fn launch_terminal_inner(spec: &CommandSpec, _title: &str, terminal: &str) -> anyhow::Result<()> {
    let script = write_script("command", &unix_script(spec, "#!/bin/zsh -l"))?;
    let argv = mac_open_argv(terminal, &script);
    let mut cmd = Command::new(&argv[0]);
    cmd.args(&argv[1..]);
    apply_env(&mut cmd, spec);
    let status = cmd.status().context("running open")?;
    if !status.success() {
        return Err(anyhow::anyhow!("Could not open {} (is it installed?)", argv[2]));
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn launch_terminal_inner(spec: &CommandSpec, _title: &str, terminal: &str) -> anyhow::Result<()> {
    let script = write_script("sh", &unix_script(spec, "#!/usr/bin/env bash"))?;
    let configured = terminal.trim();
    let candidates: Vec<&str> = if configured.is_empty() || configured.eq_ignore_ascii_case("auto") {
        LINUX_TERMINALS.to_vec()
    } else {
        std::iter::once(configured).chain(LINUX_TERMINALS.iter().copied()).collect()
    };
    for term in candidates {
        let Some(path) = crate::util::which(term) else { continue };
        let argv = linux_terminal_argv(&path.to_string_lossy(), &script);
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        apply_env(&mut cmd, spec);
        if spawn_detached(cmd).is_ok() {
            return Ok(());
        }
    }
    let _ = std::fs::remove_file(&script);
    Err(anyhow::anyhow!("No terminal emulator found. Install one (e.g. gnome-terminal, konsole, xterm) or set it in Settings."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn spec() -> CommandSpec {
        CommandSpec {
            program: r"C:\Program Files\Claude\claude.exe".into(),
            args: vec!["--continue".into(), "fix 100% & ship".into()],
            env: HashMap::from([
                ("CLAUDE_CONFIG_DIR".to_string(), Some(r"C:\Users\me\Baton\profiles\claude\work".to_string())),
                ("ANTHROPIC_API_KEY".to_string(), None),
            ]),
            cwd: Some(r"C:\src\my app".into()),
        }
    }

    #[test]
    fn windows_script_sets_and_unsets_env_and_quotes() {
        let s = windows_script(&spec(), "Work Max (Claude)");
        assert!(s.contains("set \"ANTHROPIC_API_KEY=\"\r\n"));
        assert!(s.contains(r#"set "CLAUDE_CONFIG_DIR=C:\Users\me\Baton\profiles\claude\work""#));
        assert!(s.contains(r#"cd /d "C:\src\my app""#));
        assert!(s.contains(r#"call "C:\Program Files\Claude\claude.exe" --continue "fix 100%%%% & ship""#));
        assert!(s.contains("title Work Max (Claude)"));
    }

    #[test]
    fn windows_ps1_and_cmd_shims() {
        let ps = CommandSpec { program: r"C:\npm\gemini.ps1".into(), ..Default::default() };
        assert!(windows_command_line(&ps).starts_with("powershell "));
        let shim = CommandSpec { program: "gemini.cmd".into(), args: vec!["-y".into()], ..Default::default() };
        assert_eq!(windows_command_line(&shim), "call gemini.cmd -y");
    }

    #[test]
    fn windows_terminal_argv_variants() {
        let script = Path::new(r"C:\Temp\baton\launch-1.cmd");
        let (p, raw) = windows_terminal_argv(true, script, "Work; Max", Some(r"C:\src\my app\"));
        assert_eq!(p, "wt.exe");
        assert_eq!(raw, r#"-w new --title "Work Max" -d "C:\src\my app" cmd.exe /k "C:\Temp\baton\launch-1.cmd""#);
        let (p, raw) = windows_terminal_argv(false, script, "x", None);
        assert_eq!(p, "cmd.exe");
        assert_eq!(raw, r#"/d /s /c "start "x" cmd.exe /k "C:\Temp\baton\launch-1.cmd"""#);
    }

    #[test]
    fn unix_script_exports_and_keeps_shell() {
        let spec = CommandSpec {
            program: "/usr/local/bin/codex".into(),
            args: vec!["resume".into(), "it's".into()],
            env: HashMap::from([("CODEX_HOME".to_string(), Some("/home/me/My Baton/codex".to_string())), ("OPENAI_API_KEY".to_string(), None)]),
            cwd: Some("/home/me/src".into()),
        };
        let s = unix_script(&spec, "#!/bin/zsh -l");
        assert!(s.starts_with("#!/bin/zsh -l\n"));
        assert!(s.contains("export CODEX_HOME='/home/me/My Baton/codex'\n"));
        assert!(s.contains("unset OPENAI_API_KEY\n"));
        assert!(s.contains("cd /home/me/src || exit 1\n"));
        assert!(s.contains(r"/usr/local/bin/codex resume 'it'\''s'"));
        assert!(s.trim_end().ends_with("-l"));
    }

    #[test]
    fn terminal_argvs() {
        let sc = Path::new("/tmp/x.sh");
        assert_eq!(linux_terminal_argv("/usr/bin/gnome-terminal", sc), ["/usr/bin/gnome-terminal", "--", "bash", "-l", "/tmp/x.sh"]);
        assert_eq!(linux_terminal_argv("konsole", sc), ["konsole", "-e", "bash", "-l", "/tmp/x.sh"]);
        assert_eq!(linux_terminal_argv("wezterm", sc), ["wezterm", "start", "--", "bash", "-l", "/tmp/x.sh"]);
        assert_eq!(linux_terminal_argv("kitty", sc), ["kitty", "bash", "-l", "/tmp/x.sh"]);
        assert_eq!(mac_open_argv("auto", sc), ["open", "-a", "Terminal", "/tmp/x.sh"]);
        assert_eq!(mac_open_argv("iterm", sc), ["open", "-a", "iTerm", "/tmp/x.sh"]);
    }

    /// Runs a generated script for real (without a new window) against a .cmd shim in a folder
    /// with spaces, checking env, cwd and argument quoting end to end.
    #[cfg(windows)]
    #[test]
    fn windows_script_runs_cmd_shim() {
        use std::os::windows::process::CommandExt;
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("my tools & more");
        std::fs::create_dir_all(&dir).unwrap();
        let shim = dir.join("my tool.cmd");
        std::fs::write(&shim, "@echo off\r\n>\"%~dp0out.txt\" echo [%MYVAR%] [%~1] [%~2] [%GONE%]\r\n>>\"%~dp0out.txt\" cd\r\n").unwrap();
        let spec = CommandSpec {
            program: shim.to_string_lossy().into_owned(),
            args: vec!["a b".into(), "100%".into()],
            env: HashMap::from([("MYVAR".to_string(), Some("x y".to_string())), ("GONE".to_string(), None)]),
            cwd: Some(dir.to_string_lossy().into_owned()),
        };
        let script = t.path().join("launch.cmd");
        std::fs::write(&script, windows_script(&spec, "test")).unwrap();
        let status = Command::new("cmd.exe")
            .raw_arg(format!("/d /c \"{}\"", script.display()))
            .env("GONE", "should be unset")
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        let out = std::fs::read_to_string(dir.join("out.txt")).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines, ["[x y] [a b] [100%] []".to_string(), dir.display().to_string()]);
    }

    #[test]
    fn display_line_shows_env() {
        let d = display_command_line(&spec(), Os::Linux);
        assert!(d.starts_with("env -u ANTHROPIC_API_KEY CLAUDE_CONFIG_DIR="));
        let w = display_command_line(&spec(), Os::Windows);
        assert!(w.starts_with("set \"ANTHROPIC_API_KEY=\" && set \"CLAUDE_CONFIG_DIR="));
        assert!(w.contains("\"fix 100% & ship\""));
    }
}
