//! Provider adapters: one per AI tool. Each must work on Windows, macOS and Linux.
use crate::model::*;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;

pub mod claude;
pub mod codex;
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

    /// Prepare a new home (e.g. force file-based credential storage). Idempotent.
    async fn init_home(&self, home: &std::path::Path) -> anyhow::Result<()>;

    /// Command that starts the CLI for this account.
    async fn cli_command(&self, account: &Account, resume: bool, extra_args: &[String]) -> anyhow::Result<CommandSpec>;

    /// Command that starts the provider's login flow for this account's home.
    async fn login_command(&self, account: &Account) -> anyhow::Result<CommandSpec>;

    /// Desktop app for this account. Ok(None) when the provider has no desktop app
    /// or it cannot run per-account.
    async fn desktop_command(&self, _account: &Account) -> anyhow::Result<Option<CommandSpec>> {
        Ok(None)
    }

    /// Current limits. `live` = allowed to call the provider's usage API with the account's own
    /// token. Never panic or error: return UsageSnapshot::failed(..) with a clear message instead.
    async fn fetch_usage(&self, account: &Account, live: bool) -> UsageSnapshot;

    /// Per-day token activity from local session logs (for charts). Empty vec if unsupported.
    async fn scan_activity(&self, account: &Account, since_days: u32) -> Vec<DailyActivity>;
}

pub type Adapter = Arc<dyn ProviderAdapter>;

pub fn all() -> Vec<Adapter> {
    vec![
        Arc::new(claude::ClaudeAdapter::new()),
        Arc::new(codex::CodexAdapter::new()),
        Arc::new(gemini::GeminiAdapter::new()),
        Arc::new(copilot::CopilotAdapter::new()),
    ]
}

pub fn get(id: ProviderId) -> Adapter {
    all().into_iter().find(|a| a.id() == id).expect("adapter registered")
}
