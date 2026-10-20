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
            .user_agent(concat!("ai-hopper/", env!("CARGO_PKG_VERSION")))
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
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
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
