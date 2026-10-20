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
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent(concat!("switchyard/", env!("CARGO_PKG_VERSION")))
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
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        // Owner-only: adapters write credential files through this.
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
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

/// Unix-ms timestamp to ISO-8601.
pub fn ms_to_iso(ms: i64) -> Option<String> {
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.to_rfc3339())
}

/// Unix-seconds timestamp to ISO-8601.
pub fn secs_to_iso(s: i64) -> Option<String> {
    chrono::DateTime::from_timestamp(s, 0).map(|d| d.to_rfc3339())
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
