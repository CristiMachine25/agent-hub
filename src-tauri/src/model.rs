//! Rust mirror of src/shared/types.ts. Field names serialize to camelCase so the UI
//! receives exactly the TypeScript shapes. Keep both files in sync.
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderId {
    Claude,
    Codex,
    Gemini,
    Copilot,
}

impl ProviderId {
    pub const ALL: [ProviderId; 4] = [ProviderId::Claude, ProviderId::Codex, ProviderId::Gemini, ProviderId::Copilot];
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderId::Claude => "claude",
            ProviderId::Codex => "codex",
            ProviderId::Gemini => "gemini",
            ProviderId::Copilot => "copilot",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInfo {
    pub id: ProviderId,
    pub name: String,
    pub vendor: String,
    pub cli_command: String,
    pub cli_installed: bool,
    pub cli_path: Option<String>,
    pub cli_version: Option<String>,
    pub desktop_name: Option<String>,
    pub desktop_installed: bool,
    pub desktop_path: Option<String>,
    pub desktop_multi_account: bool,
    pub home_env_var: Option<String>,
    pub install_hint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharingOptions {
    pub skills: bool,
    pub plugins: bool,
    pub agents: bool,
    pub instructions: bool,
    pub settings: bool,
    pub mcp: bool,
    pub history: bool,
}

impl Default for SharingOptions {
    fn default() -> Self {
        Self { skills: true, plugins: true, agents: true, instructions: true, settings: false, mcp: true, history: true }
    }
}

/// Sharing category keys, matching SharingOptions field names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShareKind {
    Skills,
    Plugins,
    Agents,
    Instructions,
    Settings,
    Mcp,
    History,
}

impl SharingOptions {
    pub fn enabled(&self, k: ShareKind) -> bool {
        match k {
            ShareKind::Skills => self.skills,
            ShareKind::Plugins => self.plugins,
            ShareKind::Agents => self.agents,
            ShareKind::Instructions => self.instructions,
            ShareKind::Settings => self.settings,
            ShareKind::Mcp => self.mcp,
            ShareKind::History => self.history,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountStatus {
    Ok,
    NeedsLogin,
    Expired,
    Error,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub provider_id: ProviderId,
    pub label: String,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub home_dir: String,
    pub is_default_home: bool,
    pub shared_from: Option<String>,
    pub sharing: SharingOptions,
    pub color: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub status: AccountStatus,
    pub status_message: Option<String>,
    pub created_at: String,
    pub last_launched_at: Option<String>,
    #[serde(default)]
    pub launch_count: u32,
    pub monthly_cost: Option<f64>,
    pub pinned: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAccountInput {
    pub provider_id: ProviderId,
    pub label: String,
    pub home_dir: Option<String>,
    pub sharing: Option<SharingOptions>,
    pub shared_from: Option<String>,
    pub color: Option<String>,
    pub tags: Option<Vec<String>>,
    pub monthly_cost: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedAccount {
    pub provider_id: ProviderId,
    pub home_dir: String,
    pub is_default_home: bool,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub logged_in: bool,
    #[serde(default)]
    pub already_managed: bool,
    pub hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub id: String,
    pub label: String,
    pub used_percent: f64,
    pub resets_at: Option<String>,
    pub window_minutes: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum UsageSource {
    Api,
    Cli,
    LocalLogs,
    Manual,
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credits {
    pub used: f64,
    pub limit: Option<f64>,
    pub unit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub account_id: String,
    pub provider_id: ProviderId,
    pub fetched_at: String,
    pub source: UsageSource,
    pub windows: Vec<UsageWindow>,
    pub plan: Option<String>,
    pub email: Option<String>,
    pub credits: Option<Credits>,
    pub error: Option<String>,
    pub stale: Option<bool>,
}

impl UsageSnapshot {
    /// Empty snapshot carrying an error message. Adapters return this instead of failing.
    pub fn failed(account: &Account, source: UsageSource, error: impl Into<String>) -> Self {
        Self {
            account_id: account.id.clone(),
            provider_id: account.provider_id,
            fetched_at: chrono::Utc::now().to_rfc3339(),
            source,
            windows: vec![],
            plan: None,
            email: None,
            credits: None,
            error: Some(error.into()),
            stale: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageHistoryPoint {
    pub account_id: String,
    pub t: String,
    pub windows: HashMap<String, f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyActivity {
    pub account_id: String,
    pub date: String, // YYYY-MM-DD local time
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_tokens: u64,
    pub messages: u32,
    pub sessions: u32,
    pub models: HashMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LaunchTarget {
    Cli,
    Desktop,
    Login,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchRequest {
    pub account_id: String,
    pub target: LaunchTarget,
    pub cwd: Option<String>,
    pub resume: Option<bool>,
    pub extra_args: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchResult {
    pub ok: bool,
    pub error: Option<String>,
    pub command_line: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationKind {
    LimitReached,
    LimitReset,
    NearlyExhausted,
    UseItOrLoseIt,
    SwitchSuggested,
    LoginRequired,
    Info,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppNotification {
    pub id: String,
    pub kind: NotificationKind,
    pub account_id: Option<String>,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub read: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskType {
    Coding,
    LongRefactor,
    QuickQuestion,
    Research,
    Review,
    Any,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub account_id: String,
    pub score: f64,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSettings {
    pub enabled: bool,
    pub nearly_exhausted_percent: f64,
    pub use_it_or_lose_it_hours: f64,
    pub use_it_or_lose_it_min_left_percent: f64,
    pub on_reset: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    System,
    Dark,
    Light,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub poll_interval_minutes: u32,
    pub notifications: NotificationSettings,
    pub terminal: String,
    pub default_cwd: Option<String>,
    pub default_sharing: SharingOptions,
    pub profiles_dir: Option<String>,
    pub minimize_to_tray: bool,
    pub launch_at_login: bool,
    pub theme: Theme,
    pub live_usage: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            poll_interval_minutes: 5,
            notifications: NotificationSettings {
                enabled: true,
                nearly_exhausted_percent: 80.0,
                use_it_or_lose_it_hours: 24.0,
                use_it_or_lose_it_min_left_percent: 40.0,
                on_reset: true,
            },
            terminal: "auto".into(),
            default_cwd: None,
            default_sharing: SharingOptions::default(),
            profiles_dir: None,
            minimize_to_tray: true,
            launch_at_login: false,
            theme: Theme::Dark,
            live_usage: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TerminalTarget {
    Cli,
    Login,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalSpawnRequest {
    pub account_id: String,
    pub target: TerminalTarget,
    pub resume: Option<bool>,
    pub cwd: Option<String>,
    pub extra_args: Option<Vec<String>>,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalInfo {
    pub id: String,
    pub account_id: String,
    pub title: String,
    pub command_line: String,
    pub cwd: String,
    pub pid: Option<u32>,
    pub started_at: String,
    pub exited: bool,
    pub exit_code: Option<i32>,
}
