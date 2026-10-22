//! Provider adapters: one per AI tool. Each must work on Windows, macOS and Linux.
use crate::model::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub mod claude;
pub mod codex;
pub(crate) mod common;
pub mod copilot;
pub mod gemini;

/// A process to start. The launcher wraps it in a terminal for CLI targets.
#[derive(Debug, Clone, Default)]
pub struct CommandSpec {
    /// Executable: resolved absolute path or bare name on PATH.
    pub program: String,
    pub args: Vec<String>,
    /// Env vars to set. `None` means "remove this var from the child env"
    /// (e.g. strip ANTHROPIC_API_KEY so the subscription login is used).
    pub env: HashMap<String, Option<String>>,
    pub cwd: Option<String>,
    /// Shown to the user after a successful launch.
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Identity {
    pub logged_in: bool,
    pub email: Option<String>,
    pub plan: Option<String>,
}

/// A failed usage fetch: what to tell the user, and what kind of failure it is.
#[derive(Debug, Clone, PartialEq)]
pub struct FetchError {
    pub kind: ErrorKind,
    pub message: String,
}

impl FetchError {
    pub fn auth(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Auth, message: message.into() }
    }

    pub fn expired(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Expired, message: message.into() }
    }

    pub fn network(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Network, message: message.into() }
    }

    pub fn other(message: impl Into<String>) -> Self {
        Self { kind: ErrorKind::Other, message: message.into() }
    }

    /// Put this error on a snapshot.
    pub fn apply(self, s: &mut UsageSnapshot) {
        s.error = Some(self.message);
        s.error_kind = Some(self.kind);
    }

    /// Empty snapshot carrying this error.
    pub fn snapshot(self, account: &Account, source: UsageSource) -> UsageSnapshot {
        let mut s = UsageSnapshot::empty(account, source);
        self.apply(&mut s);
        s
    }
}

#[async_trait]
pub trait ProviderAdapter: Send + Sync {
    fn id(&self) -> ProviderId;

    /// Static + detected info (CLI on PATH, version, desktop app). Cheap enough to call often:
    /// cache expensive probes (e.g. `--version`) for ~60s inside the adapter.
    async fn info(&self) -> ProviderInfo;

    /// Stock home dir on this OS, e.g. ~/.claude
    fn default_home(&self) -> std::path::PathBuf;

    /// Homes already on disk: the default home plus side-by-side patterns such as ~/.claude-*,
    /// ~/.codex-*, dirs referenced by launcher scripts in ~/.local/bin (e.g. `set CLAUDE_CONFIG_DIR=...`).
    /// Return only dirs that look like real homes for this provider.
    async fn detect_accounts(&self) -> Vec<DetectedAccount>;

    /// Who is logged in to a home, offline (read local files/keychain only).
    async fn read_identity(&self, home: &std::path::Path) -> Identity;

    /// Relative paths inside a home for each sharing category. The core links
    /// <shared_from>/<path> into <home>/<path>. Credential files must NEVER appear here.
    fn shareable_paths(&self) -> Vec<(ShareKind, Vec<&'static str>)>;

    /// Relative paths that hold credentials (never shared, never copied).
    fn credential_paths(&self) -> Vec<&'static str>;

    /// The sign-in of a home, as (relative path, bytes), for moving the account to another computer.
    /// Default: the credential files that exist in the home.
    async fn export_sign_in(&self, home: &std::path::Path, _is_default: bool) -> Vec<(String, Vec<u8>)> {
        self.credential_paths().into_iter().filter_map(|rel| std::fs::read(home.join(rel)).ok().map(|b| (rel.to_string(), b))).collect()
    }

    /// Write a sign-in exported on another computer. Never replaces anything already in the home.
    /// Returns how many items were written.
    async fn import_sign_in(&self, home: &std::path::Path, _is_default: bool, files: &[(String, Vec<u8>)]) -> anyhow::Result<usize> {
        let allowed = self.credential_paths();
        let mut written = 0;
        for (rel, bytes) in files.iter().filter(|(rel, _)| allowed.contains(&rel.as_str())) {
            let path = home.join(rel);
            if path.exists() {
                continue;
            }
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            crate::util::write_atomic(&path, bytes)?;
            written += 1;
        }
        Ok(written)
    }

    /// Prepare a new home (e.g. force file-based credential storage). Idempotent.
    async fn init_home(&self, home: &std::path::Path) -> anyhow::Result<()>;

    /// Command that starts the CLI for this account.
    async fn cli_command(&self, account: &Account, resume: Resume, extra_args: &[String]) -> anyhow::Result<CommandSpec>;

    /// Conversations Resume may reopen for this account in `cwd`, best first: `wanted` when this
    /// account can open it, then its own ones in that folder, newest first. Empty leaves the
    /// choice to the CLI's "most recent".
    fn resume_candidates(&self, _account: &Account, _cwd: &std::path::Path, _wanted: Option<&str>) -> Vec<String> {
        Vec::new()
    }

    /// The conversations this account's CLI processes have open right now, as (pid, conversation).
    fn open_sessions(&self, _account: &Account) -> Vec<(u32, String)> {
        Vec::new()
    }

    /// Command that starts the provider's login flow for this account's home.
    async fn login_command(&self, account: &Account) -> anyhow::Result<CommandSpec>;

    /// Desktop app for this account. Ok(None) when the provider has no desktop app
    /// or it cannot run per-account.
    async fn desktop_command(&self, _account: &Account) -> anyhow::Result<Option<CommandSpec>> {
        Ok(None)
    }

    /// Current limits. `live` = allowed to call the provider's usage API with the account's own
    /// token. Never panic or error: return a snapshot carrying a `FetchError` (a clear message and
    /// its kind) instead.
    async fn fetch_usage(&self, account: &Account, live: bool) -> UsageSnapshot;

    /// Per-day token activity from local session logs (for charts). Empty vec if unsupported.
    async fn scan_activity(&self, account: &Account, since_days: u32) -> Vec<DailyActivity>;
}

pub type Adapter = Arc<dyn ProviderAdapter>;

/// Which conversation a CLI start reopens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resume {
    /// None: a new conversation.
    No,
    /// The CLI's own "most recent" one.
    Last,
    /// This one. A CLI that can't pick one reopens its most recent instead.
    Session(String),
}

pub fn all() -> Vec<Adapter> {
    vec![
        Arc::new(claude::ClaudeAdapter::new()),
        Arc::new(codex::CodexAdapter::new()),
        Arc::new(gemini::GeminiAdapter::new()),
        Arc::new(copilot::CopilotAdapter::new()),
    ]
}

/// One adapter by id, for tests. The app keeps a single set in `AppState::adapters`.
#[cfg(test)]
pub fn get(id: ProviderId) -> Adapter {
    all().into_iter().find(|a| a.id() == id).expect("adapter registered")
}

/// CLI flags that start `provider` in the chosen permission mode. Empty for "default".
pub fn permission_flags(provider: ProviderId, modes: &PermissionModes) -> Vec<String> {
    let flags: &[&str] = match (provider, modes.get(provider)) {
        (ProviderId::Claude, m @ ("acceptEdits" | "plan" | "bypassPermissions")) => return vec!["--permission-mode".into(), m.into()],
        // What `--full-auto` expanded to; newer Codex releases dropped that alias.
        (ProviderId::Codex, "auto") => &["--sandbox", "workspace-write", "--ask-for-approval", "on-request"],
        (ProviderId::Codex, "bypass") => &["--dangerously-bypass-approvals-and-sandbox"],
        (ProviderId::Gemini, "autoEdit") => &["--approval-mode", "auto_edit"],
        (ProviderId::Gemini, "yolo") => &["--yolo"],
        (ProviderId::Copilot, "allowAll") => &["--allow-all-tools"],
        _ => &[],
    };
    flags.iter().map(|f| f.to_string()).collect()
}

/// Flags that already pick a permission mode; if the user passed one, theirs wins.
fn permission_overrides(provider: ProviderId) -> &'static [&'static str] {
    match provider {
        ProviderId::Claude => &["--permission-mode", "--dangerously-skip-permissions"],
        ProviderId::Codex => {
            &["--full-auto", "--dangerously-bypass-approvals-and-sandbox", "--yolo", "--approve-for-me", "-s", "--sandbox", "-a", "--ask-for-approval"]
        }
        ProviderId::Gemini => &["--approval-mode", "--yolo", "-y"],
        ProviderId::Copilot => &["--allow-all-tools", "--allow-all", "--yolo"],
    }
}

/// Put the permission-mode flags first, unless `args` already choose a mode. First matters for
/// Codex: they go before `resume`, where Codex reads them as top-level options.
pub fn apply_permission_mode(provider: ProviderId, modes: &PermissionModes, args: &mut Vec<String>) {
    let overrides = permission_overrides(provider);
    let chosen = args.iter().any(|a| {
        let flag = a.split_once('=').map_or(a.as_str(), |(f, _)| f);
        overrides.contains(&flag)
    });
    if !chosen {
        args.splice(0..0, permission_flags(provider, modes));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modes(claude: &str, codex: &str, gemini: &str, copilot: &str) -> PermissionModes {
        PermissionModes { claude: claude.into(), codex: codex.into(), gemini: gemini.into(), copilot: copilot.into() }
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn permission_flags_per_tool() {
        let d = PermissionModes::default();
        for p in ProviderId::ALL {
            assert!(permission_flags(p, &d).is_empty());
        }
        assert_eq!(permission_flags(ProviderId::Claude, &modes("bypassPermissions", "", "", "")), ["--permission-mode", "bypassPermissions"]);
        assert_eq!(permission_flags(ProviderId::Claude, &modes("acceptEdits", "", "", "")), ["--permission-mode", "acceptEdits"]);
        assert_eq!(permission_flags(ProviderId::Claude, &modes("plan", "", "", "")), ["--permission-mode", "plan"]);
        assert!(permission_flags(ProviderId::Claude, &modes("rm -rf", "", "", "")).is_empty());
        assert_eq!(permission_flags(ProviderId::Codex, &modes("", "auto", "", "")), ["--sandbox", "workspace-write", "--ask-for-approval", "on-request"]);
        assert_eq!(permission_flags(ProviderId::Codex, &modes("", "bypass", "", "")), ["--dangerously-bypass-approvals-and-sandbox"]);
        assert_eq!(permission_flags(ProviderId::Gemini, &modes("", "", "autoEdit", "")), ["--approval-mode", "auto_edit"]);
        assert_eq!(permission_flags(ProviderId::Gemini, &modes("", "", "yolo", "")), ["--yolo"]);
        assert_eq!(permission_flags(ProviderId::Copilot, &modes("", "", "", "allowAll")), ["--allow-all-tools"]);
        // One tool's mode never leaks into another's flags.
        assert!(permission_flags(ProviderId::Codex, &modes("bypassPermissions", "default", "yolo", "allowAll")).is_empty());
    }

    #[test]
    fn permission_flags_go_before_codex_resume() {
        let m = modes("", "bypass", "", "");
        let mut a = args(&["resume", "--last", "--model", "x"]);
        apply_permission_mode(ProviderId::Codex, &m, &mut a);
        assert_eq!(a, ["--dangerously-bypass-approvals-and-sandbox", "resume", "--last", "--model", "x"]);
    }

    #[test]
    fn permission_flags_skip_when_user_chose_a_mode() {
        let m = modes("bypassPermissions", "auto", "yolo", "allowAll");
        let mut a = args(&["--continue", "--permission-mode=plan"]);
        apply_permission_mode(ProviderId::Claude, &m, &mut a);
        assert_eq!(a, ["--continue", "--permission-mode=plan"]);
        let mut a = args(&["-s", "read-only"]);
        apply_permission_mode(ProviderId::Codex, &m, &mut a);
        assert_eq!(a, ["-s", "read-only"]);
        let mut a = args(&["-y"]);
        apply_permission_mode(ProviderId::Gemini, &m, &mut a);
        assert_eq!(a, ["-y"]);
        let mut a = args(&["--continue"]);
        apply_permission_mode(ProviderId::Copilot, &m, &mut a);
        assert_eq!(a, ["--allow-all-tools", "--continue"]);
    }
}
