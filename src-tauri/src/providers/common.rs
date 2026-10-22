//! Helpers shared by the provider adapters: launcher-script scanning, path expansion,
//! JSONL log walking, local dates, version caching and credential-safe file writes.
use crate::model::DailyActivity;
use crate::util;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime};

/// Expand `~`, `$HOME`, `${VAR}`, `$env:VAR` and `%VAR%` in a path taken from a script.
/// Returns None when something stays unresolved (e.g. `%1`, `$PWD` in an odd shell).
pub fn expand_path(raw: &str) -> Option<PathBuf> {
    let raw = raw.trim().trim_matches(|c| c == '"' || c == '\'').trim();
    if raw.is_empty() {
        return None;
    }
    let home = util::home().to_string_lossy().to_string();
    let lookup = |name: &str| -> Option<String> {
        match name.to_ascii_uppercase().as_str() {
            "HOME" | "USERPROFILE" => Some(home.clone()),
            _ => std::env::var(name).ok(),
        }
    };
    let re = regex::Regex::new(r"%([A-Za-z_][A-Za-z0-9_]*)%|\$env:([A-Za-z_][A-Za-z0-9_]*)|\$\{([A-Za-z_][A-Za-z0-9_]*)\}|\$([A-Za-z_][A-Za-z0-9_]*)")
        .expect("valid regex");
    let mut failed = false;
    let mut out = re
        .replace_all(raw, |c: &regex::Captures| {
            let name = (1..=4).find_map(|i| c.get(i)).map(|m| m.as_str()).unwrap_or("");
            lookup(name).unwrap_or_else(|| {
                failed = true;
                String::new()
            })
        })
        .to_string();
    if failed || out.contains('%') {
        return None;
    }
    if out == "~" {
        out = home.clone();
    } else if let Some(rest) = out.strip_prefix("~/").or_else(|| out.strip_prefix("~\\")) {
        out = format!("{home}/{rest}");
    }
    let p = PathBuf::from(out);
    p.is_absolute().then_some(p)
}

/// Extract every value assigned to `var` in a launcher script
/// (`set VAR=..`, `set "VAR=.."`, `export VAR=..`, `$env:VAR = ".."`, `VAR=.. cmd`).
pub fn parse_script_assignments(content: &str, var: &str) -> Vec<String> {
    let mut found = Vec::new();
    for line in content.lines() {
        let mut search_from = 0;
        while let Some(rel) = line[search_from..].find(var) {
            let start = search_from + rel;
            search_from = start + var.len();
            // Must be a whole identifier.
            let before_ok = line[..start].chars().last().is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'));
            if !before_ok {
                continue;
            }
            let rest = line[start + var.len()..].trim_start();
            let Some(value) = rest.strip_prefix('=') else { continue };
            let value = value.trim_start();
            let is_cmd_set = line.trim_start().trim_start_matches('@').to_ascii_lowercase().starts_with("set ");
            let parsed = if let Some(v) = value.strip_prefix('"') {
                v.split('"').next().map(str::to_string)
            } else if let Some(v) = value.strip_prefix('\'') {
                v.split('\'').next().map(str::to_string)
            } else if is_cmd_set {
                // cmd `set VAR=C:\My Dir` takes the rest of the line; `set "VAR=x"` ends at the quote.
                Some(value.split(['"', '&']).next().unwrap_or("").trim().to_string())
            } else {
                value.split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '"' | '\'')).next().map(str::to_string)
            };
            if let Some(v) = parsed.filter(|v| !v.trim().is_empty()) {
                found.push(v.trim().to_string());
            }
        }
    }
    found
}

/// Look through user launcher scripts (~/.local/bin, ~/bin) for `var=<path>`.
/// Returns (expanded path, script file name).
pub fn scan_launcher_scripts(var: &str) -> Vec<(PathBuf, String)> {
    let home = util::home();
    let mut out = Vec::new();
    for dir in [home.join(".local").join("bin"), home.join("bin")] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase());
            let script_like = matches!(ext.as_deref(), None | Some("cmd" | "bat" | "ps1" | "sh" | "bash" | "zsh" | "fish"));
            let small = entry.metadata().map(|m| m.is_file() && m.len() < 64 * 1024).unwrap_or(false);
            if !script_like || !small {
                continue;
            }
            let Ok(bytes) = std::fs::read(&path) else { continue };
            let text = String::from_utf8_lossy(&bytes);
            if !text.contains(var) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            for value in parse_script_assignments(&text, var) {
                if let Some(p) = expand_path(&value) {
                    out.push((p, name.clone()));
                }
            }
        }
    }
    out
}

/// Sibling dirs of the user's home named `<prefix>*` (e.g. `.claude-` → ~/.claude-work).
pub fn sibling_dirs(parent: &Path, prefix: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(parent) else { return vec![] };
    let mut dirs: Vec<PathBuf> =
        entries.flatten().filter(|e| e.file_name().to_string_lossy().starts_with(prefix)).map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs
}

/// `*.jsonl` files under `roots` modified at or after `since`, newest first.
pub fn jsonl_files_since(roots: &[PathBuf], since: SystemTime) -> Vec<(PathBuf, SystemTime)> {
    let mut files = Vec::new();
    for root in roots {
        for entry in walkdir::WalkDir::new(root).follow_links(true).into_iter().flatten() {
            if !entry.file_type().is_file() || entry.path().extension().is_none_or(|e| e != "jsonl") {
                continue;
            }
            if let Some(modified) = entry.metadata().ok().and_then(|m| m.modified().ok()) {
                if modified >= since {
                    files.push((entry.path().to_path_buf(), modified));
                }
            }
        }
    }
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    files
}

/// Stream a JSONL file, calling `f` for each line that parses. `prefilter` skips lines cheaply
/// before JSON parsing (return false to skip).
pub fn for_each_json_line(path: &Path, prefilter: impl Fn(&str) -> bool, mut f: impl FnMut(serde_json::Value)) {
    let Ok(file) = std::fs::File::open(path) else { return };
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        if line.is_empty() || !prefilter(&line) {
            continue;
        }
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) {
            f(v);
        }
    }
}

/// The first line of a JSONL file, if it parses.
pub fn first_json_line(path: &Path) -> Option<serde_json::Value> {
    let mut line = String::new();
    BufReader::new(std::fs::File::open(path).ok()?).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

/// Whether some line of a JSONL file passes `prefilter` (cheap, on the raw text) and then `f`.
/// Stops reading at the first one.
pub fn any_json_line(path: &Path, prefilter: impl Fn(&str) -> bool, f: impl Fn(&serde_json::Value) -> bool) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    BufReader::new(file).lines().map_while(Result::ok).filter(|l| prefilter(l)).any(|l| serde_json::from_str::<serde_json::Value>(&l).is_ok_and(|v| f(&v)))
}

/// How many of a folder's conversations Resume looks through.
pub const RESUME_CANDIDATES: usize = 20;

/// Conversation ids that may reach a command line: uuids only, so nothing from a damaged file
/// can turn into a flag, an index or a "latest".
pub fn is_uuid(s: &str) -> bool {
    s.len() == 36 && s.char_indices().all(|(i, c)| if matches!(i, 8 | 13 | 18 | 23) { c == '-' } else { c.is_ascii_hexdigit() })
}

/// `wanted` (already known to be openable), then this account's own conversations without it.
pub fn wanted_first(wanted: Option<String>, own: Vec<String>) -> Vec<String> {
    let own = own.into_iter().filter(|id| wanted.as_ref() != Some(id));
    wanted.clone().into_iter().chain(own).collect()
}

pub fn days_ago(days: u32) -> SystemTime {
    SystemTime::now().checked_sub(Duration::from_secs(u64::from(days) * 86_400)).unwrap_or(SystemTime::UNIX_EPOCH)
}

/// First local date (YYYY-MM-DD) included in a `since_days` range (today counts as day 1).
pub fn first_local_date(since_days: u32) -> String {
    let today = chrono::Local::now().date_naive();
    let back = i64::from(since_days.max(1)) - 1;
    (today - chrono::Duration::days(back)).format("%Y-%m-%d").to_string()
}

/// Per-day accumulator keyed by local `YYYY-MM-DD`.
pub type DayMap = std::collections::BTreeMap<String, DailyActivity>;

/// Drop days before the cutoff and stamp the account id.
pub fn finish_days(days: DayMap, account_id: &str, cutoff: &str) -> Vec<DailyActivity> {
    days.into_iter()
        .filter(|(day, _)| day.as_str() >= cutoff)
        .map(|(date, mut d)| {
            d.date = date;
            d.account_id = account_id.to_string();
            d
        })
        .collect()
}

/// ISO-8601 timestamp to local calendar date.
pub fn local_date(ts: &str) -> Option<String> {
    let dt = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
    Some(dt.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
}

/// Normalize an ISO timestamp to RFC 3339 (UTC); keeps the raw string if it does not parse.
pub fn normalize_iso(ts: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(ts).map(|d| d.with_timezone(&chrono::Utc).to_rfc3339()).unwrap_or_else(|_| ts.to_string())
}

/// Read a number that may be encoded as a JSON number or a numeric string.
pub fn num(v: Option<&serde_json::Value>) -> Option<f64> {
    match v? {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// Non-empty string field.
pub fn str_field<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str()).map(str::trim).filter(|s| !s.is_empty())
}

/// Read a JSON file that may contain `//` or `/* */` comments and trailing commas (Gemini's settings.json
/// and Copilot's config files are user-editable JSONC).
pub fn read_jsonc(path: &Path) -> Option<serde_json::Value> {
    let s = std::fs::read_to_string(path).ok()?;
    parse_jsonc(&s)
}

pub fn parse_jsonc(s: &str) -> Option<serde_json::Value> {
    let s = s.trim_start_matches('\u{feff}');
    if let Ok(v) = serde_json::from_str(s) {
        return Some(v);
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_str = false;
    while let Some(c) = chars.next() {
        if in_str {
            out.push(c);
            if c == '\\' {
                if let Some(n) = chars.next() {
                    out.push(n);
                }
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_str = true;
                out.push(c);
            }
            '/' if chars.peek() == Some(&'/') => {
                for n in chars.by_ref() {
                    if n == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = ' ';
                for n in chars.by_ref() {
                    if prev == '*' && n == '/' {
                        break;
                    }
                    prev = n;
                }
            }
            _ => out.push(c),
        }
    }
    let re = regex::Regex::new(r",(\s*[}\]])").ok()?;
    serde_json::from_str(&re.replace_all(&out, "$1")).ok()
}

/// "5-hour session", "Weekly", "Daily", "30-day", "90-minute".
pub fn window_label(minutes: u32) -> String {
    match minutes {
        300 => "5-hour session".into(),
        10080 => "Weekly".into(),
        1440 => "Daily".into(),
        m if m > 0 && m % 1440 == 0 => format!("{}-day", m / 1440),
        m if m > 0 && m % 60 == 0 => format!("{}-hour", m / 60),
        m => format!("{m}-minute"),
    }
}

/// Resolve a CLI on PATH, returning a display path.
pub fn which_str(cmd: &str) -> Option<String> {
    util::which(cmd).map(|p| p.to_string_lossy().to_string())
}

/// The version number in `<cli> --version` output ("2.1.3 (Claude Code)", "codex-cli 0.46.0", "0.64.0").
pub fn parse_version(out: &str) -> Option<String> {
    let re = regex::Regex::new(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?").ok()?;
    re.find(out).map(|m| m.as_str().to_string())
}

/// Caches `<cli> --version` for 60 s.
/// (probed at, CLI path, version)
type VersionEntry = (Instant, Option<String>, Option<String>);

#[derive(Default)]
pub struct VersionCache {
    inner: Mutex<Option<VersionEntry>>,
}

impl VersionCache {
    /// Returns (resolved path, parsed version).
    pub async fn get(&self, cmd: &str) -> (Option<String>, Option<String>) {
        if let Ok(guard) = self.inner.lock() {
            if let Some((at, path, ver)) = guard.as_ref() {
                if at.elapsed() < Duration::from_secs(60) {
                    return (path.clone(), ver.clone());
                }
            }
        }
        let path = util::which(cmd);
        let version = match &path {
            Some(p) => util::run_capture(p, &["--version"], Duration::from_secs(10)).await.and_then(|o| parse_version(&o)),
            None => None,
        };
        let path = path.map(|p| p.to_string_lossy().to_string());
        if let Ok(mut guard) = self.inner.lock() {
            *guard = Some((Instant::now(), path.clone(), version.clone()));
        }
        (path, version)
    }
}

/// Atomic write for credential files: temp file + rename. On Unix the temp file is created
/// 0600 (util::write_atomic would use the umask, typically 0644, widening access to the secret).
pub fn write_credentials_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
        let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    }
    #[cfg(not(unix))]
    {
        util::write_atomic(path, bytes)
    }
}

/// Env map entries removing a variable from the child process.
pub fn env_unset(env: &mut HashMap<String, Option<String>>, keys: &[&str]) {
    for k in keys {
        env.insert((*k).to_string(), None);
    }
}

/// Windows: first entry under %LOCALAPPDATA%\Microsoft\WindowsApps whose name starts with `prefix`
/// (MSIX package family dirs such as `Claude_pzs8sxrjxfjjc`). Returns the family name.
#[cfg(windows)]
pub fn msix_family(prefix: &str) -> Option<String> {
    let base = dirs::data_local_dir()?.join("Microsoft").join("WindowsApps");
    std::fs::read_dir(base).ok()?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).find(|n| n.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cmd_set() {
        let s = "@echo off\r\nset CLAUDE_CONFIG_DIR=%USERPROFILE%\\.claude-h1\r\nclaude %*\r\n";
        assert_eq!(parse_script_assignments(s, "CLAUDE_CONFIG_DIR"), vec!["%USERPROFILE%\\.claude-h1"]);
    }

    #[test]
    fn parses_quoted_and_spaces() {
        let s = "set \"CODEX_HOME=C:\\My Dir\\codex\"\nset CODEX_HOME=C:\\Other Dir\\x\n";
        assert_eq!(parse_script_assignments(s, "CODEX_HOME"), vec!["C:\\My Dir\\codex", "C:\\Other Dir\\x"]);
    }

    #[test]
    fn parses_sh_and_ps1() {
        let s =
            "#!/bin/sh\nexport CODEX_HOME=\"$HOME/.codex-work\"\nCODEX_HOME=~/.codex-b exec codex \"$@\"\n$env:CODEX_HOME = 'C:\\x\\y'\nMY_CODEX_HOME=/nope\n";
        assert_eq!(parse_script_assignments(s, "CODEX_HOME"), vec!["$HOME/.codex-work", "~/.codex-b", "C:\\x\\y"]);
    }

    #[test]
    fn expands_home_vars() {
        let home = util::home();
        let a = expand_path("%USERPROFILE%\\.claude-h1").unwrap();
        assert!(a.to_string_lossy().starts_with(&*home.to_string_lossy()));
        assert!(expand_path("$HOME/.codex-x").is_some());
        assert!(expand_path("${HOME}/.codex-x").is_some());
        assert!(expand_path("~/.codex-x").is_some());
        assert!(expand_path("%1").is_none());
        assert!(expand_path("relative/dir").is_none());
        assert!(expand_path("$NOT_A_REAL_VAR_XYZ/x").is_none());
    }

    #[test]
    fn parses_gemini_style_launchers() {
        let s = "#!/bin/sh\nexport GEMINI_CLI_HOME=\"$HOME/.gemini-work\"\nGEMINI_CLI_HOME=/opt/g2 exec gemini \"$@\"\n@set GEMINI_CLI_HOME=%USERPROFILE%\\gem 3\n$env:GEMINI_CLI_HOME = 'D:\\g4'\n";
        assert_eq!(parse_script_assignments(s, "GEMINI_CLI_HOME"), vec!["$HOME/.gemini-work", "/opt/g2", "%USERPROFILE%\\gem 3", "D:\\g4"]);
        assert_eq!(expand_path("$HOME/.gemini-work"), Some(util::home().join(".gemini-work")));
    }

    #[test]
    fn jsonc_and_versions() {
        let v = parse_jsonc("{ /* c */ \"a\": \"http://x//y\", // t\n \"b\": [1,2,], }").unwrap();
        assert_eq!(v["a"], "http://x//y");
        assert_eq!(v["b"][1], 2);
        assert_eq!(parse_version("0.64.0\n").as_deref(), Some("0.64.0"));
        assert_eq!(parse_version("2.1.3 (Claude Code)").as_deref(), Some("2.1.3"));
        assert_eq!(parse_version("codex-cli 0.46.0").as_deref(), Some("0.46.0"));
        assert_eq!(parse_version("1.0.60\nCommit: 1a2b3c4").as_deref(), Some("1.0.60"));
    }

    #[test]
    fn day_ranges_count_today_as_day_one() {
        let today = chrono::Local::now().date_naive();
        let day = |back: i64| (today - chrono::Duration::days(back)).format("%Y-%m-%d").to_string();
        assert_eq!(first_local_date(1), day(0));
        assert_eq!(first_local_date(0), day(0));
        assert_eq!(first_local_date(7), day(6));
        let days: DayMap = (0..9).map(|b| (day(b), DailyActivity::default())).collect();
        let kept = finish_days(days, "acc", &first_local_date(7));
        assert_eq!(kept.len(), 7);
        assert_eq!(kept[0].date, day(6));
        assert!(kept.iter().all(|d| d.account_id == "acc"));
    }

    #[test]
    fn conversation_ids_must_be_uuids() {
        assert!(is_uuid("01a1554c-99d9-7531-8477-a42d7e29b04b"));
        assert!(is_uuid("D6E732E6-506D-4120-988E-C10CAC139665"));
        for bad in [
            "",
            "latest",
            "3",
            "--yolo",
            "01a1554c99d9-7531-8477-a42d7e29b04b-",
            "01a1554c-99d9-7531-8477-a42d7e29b04g",
            "01a1554c-99d9-7531-8477-a42d7e29b04b ",
        ] {
            assert!(!is_uuid(bad), "{bad}");
        }
    }

    #[test]
    fn json_lines_read_lazily() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.jsonl");
        std::fs::write(&f, "{\"n\":1}\nnot json\n{\"n\":2,\"hit\":true}\n").unwrap();
        assert_eq!(first_json_line(&f).unwrap()["n"], 1);
        assert!(any_json_line(&f, |l| l.contains("hit"), |v| v["n"] == 2));
        assert!(!any_json_line(&f, |l| l.contains("hit"), |v| v["n"] == 1));
        std::fs::write(&f, "garbage\n{\"n\":1}\n").unwrap();
        assert!(first_json_line(&f).is_none());
        assert!(first_json_line(&dir.path().join("missing.jsonl")).is_none());
    }

    #[test]
    fn a_wanted_conversation_goes_first_once() {
        let own = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(wanted_first(Some("b".into()), own(&["a", "b", "c"])), ["b", "a", "c"]);
        assert_eq!(wanted_first(Some("x".into()), own(&["a"])), ["x", "a"]);
        assert_eq!(wanted_first(None, own(&["a"])), ["a"]);
    }

    #[test]
    fn labels() {
        assert_eq!(window_label(300), "5-hour session");
        assert_eq!(window_label(10080), "Weekly");
        assert_eq!(window_label(43200), "30-day");
        assert_eq!(window_label(120), "2-hour");
    }
}
