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
    /// The short name people use: "Claude", "Codex"...
    pub fn name(self) -> &'static str {
        match self {
            ProviderId::Claude => "Claude",
            ProviderId::Codex => "Codex",
            ProviderId::Gemini => "Gemini",
            ProviderId::Copilot => "Copilot",
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

impl ShareKind {
    /// The SharingOptions field name of a kind ("skills", "history"...), also the kind in setup files.
    pub fn key(self) -> &'static str {
        match self {
            ShareKind::Skills => "skills",
            ShareKind::Plugins => "plugins",
            ShareKind::Agents => "agents",
            ShareKind::Instructions => "instructions",
            ShareKind::Settings => "settings",
            ShareKind::Mcp => "mcp",
            ShareKind::History => "history",
        }
    }
}

impl SharingOptions {
    /// Nothing shared: imported homes start like this, and removal unlinks with it.
    pub const NONE: Self = Self { skills: false, plugins: false, agents: false, instructions: false, settings: false, mcp: false, history: false };

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
    /// Plan picked in the form; the plan read from the signed-in profile wins.
    pub plan: Option<String>,
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
    /// What kind of failure `error` is. Only the poll that fetched the snapshot reads it, so it is
    /// neither sent to the UI nor saved.
    #[serde(skip)]
    pub error_kind: Option<ErrorKind>,
    pub stale: Option<bool>,
}

/// Why a usage fetch failed, so account status doesn't have to be guessed from the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// No sign-in, or the provider rejected it: signing in again fixes it.
    Auth,
    /// The sign-in lapsed: opening the tool once (or signing in again) renews it.
    Expired,
    /// The provider could not be reached.
    Network,
    Other,
}

impl UsageSnapshot {
    /// A snapshot with nothing in it yet. Adapters fill it in; a failed fetch gets its error (and its
    /// kind) from `providers::FetchError`.
    pub fn empty(account: &Account, source: UsageSource) -> Self {
        Self {
            account_id: account.id.clone(),
            provider_id: account.provider_id,
            fetched_at: chrono::Utc::now().to_rfc3339(),
            source,
            windows: vec![],
            plan: None,
            email: None,
            credits: None,
            error: None,
            error_kind: None,
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
    /// Something the user should know after a successful launch (e.g. "sign in once in this window").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
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
    #[serde(default)]
    pub permission_modes: PermissionModes,
}

/// Permission mode Agent Hub starts each CLI in, per tool. Values are the tool's own mode names;
/// "default" leaves the CLI's own setting alone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PermissionModes {
    pub claude: String,
    pub codex: String,
    pub gemini: String,
    pub copilot: String,
}

impl Default for PermissionModes {
    fn default() -> Self {
        let d = || "default".to_string();
        Self { claude: d(), codex: d(), gemini: d(), copilot: d() }
    }
}

impl PermissionModes {
    /// Modes each tool accepts; the first is the default.
    pub fn allowed(provider: ProviderId) -> &'static [&'static str] {
        match provider {
            ProviderId::Claude => &["default", "acceptEdits", "plan", "bypassPermissions"],
            ProviderId::Codex => &["default", "auto", "bypass"],
            ProviderId::Gemini => &["default", "autoEdit", "yolo"],
            ProviderId::Copilot => &["default", "allowAll"],
        }
    }

    pub fn get(&self, provider: ProviderId) -> &str {
        match provider {
            ProviderId::Claude => &self.claude,
            ProviderId::Codex => &self.codex,
            ProviderId::Gemini => &self.gemini,
            ProviderId::Copilot => &self.copilot,
        }
    }

    /// Reset any value a tool does not know to "default".
    pub fn sanitize(&mut self) {
        for provider in ProviderId::ALL {
            let mode = match provider {
                ProviderId::Claude => &mut self.claude,
                ProviderId::Codex => &mut self.codex,
                ProviderId::Gemini => &mut self.gemini,
                ProviderId::Copilot => &mut self.copilot,
            };
            if !Self::allowed(provider).contains(&mode.as_str()) {
                *mode = "default".into();
            }
        }
    }
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
            permission_modes: PermissionModes::default(),
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

/// One item in a tool's shared knowledge folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeItem {
    /// "skills" | "plugins" | "agents" | "commands" | "instructions" | "settings"
    pub kind: String,
    /// Skill folder, agent or command file stem, plugin name, or file name (CLAUDE.md, settings.json).
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeAccount {
    pub account_id: String,
    pub label: String,
    pub is_default_home: bool,
    /// Knowledge kinds this account links from the shared folder ("skills", "plugins", "agents",
    /// "instructions", "settings", "mcp"). Empty for a default home, which is never linked.
    pub kinds: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderKnowledge {
    pub provider_id: ProviderId,
    /// The Agent Hub shared folder for this tool.
    pub folder: String,
    /// The tool's main folder (default home) the shared folder is seeded from.
    pub main_folder: String,
    pub items: Vec<KnowledgeItem>,
    pub accounts: Vec<KnowledgeAccount>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReseedResult {
    /// Files copied from the main folder.
    pub added: usize,
    /// Files already in the shared folder, left as they were.
    pub kept: usize,
}

/// A skill listed in the skills.sh directory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DirectorySkill {
    /// "owner/repo/skill"
    pub id: String,
    pub name: String,
    /// The GitHub repository, "owner/repo".
    pub source: String,
    pub installs: u64,
}

/// A skill's SKILL.md, read before installing it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillPreview {
    pub id: String,
    pub name: String,
    pub description: String,
    /// SKILL.md without its front matter.
    pub body: String,
    /// How many files the skill brings, SKILL.md included.
    pub files: usize,
}

/// A skill folder in a tool's shared skills folder.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct InstalledSkill {
    pub provider_id: ProviderId,
    /// The folder name.
    pub name: String,
    /// The directory id when Agent Hub installed it, else None (copied from the main folder or added by hand).
    pub id: Option<String>,
}

/// How installing a skill into one tool went.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillInstallOutcome {
    pub provider_id: ProviderId,
    pub error: Option<String>,
}

/// Writes `src/shared/__fixtures__/serde.ts`: one value of every type the UI sends or receives, with
/// every Option set, plus every enum value and every registered command. tsc checks that file against
/// `src/shared/types.ts`, so a field or enum value that exists on one side only fails the typecheck.
/// This test fails while the committed file is stale; `UPDATE_SERDE_FIXTURE=1 cargo test serde_fixture`
/// rewrites it.
#[cfg(test)]
mod contract_tests {
    use super::*;
    use crate::transfer::{ExportOptions, ExportSummary, ImportChoice, ImportPreview, ImportReport, PreviewAccount, PreviewKind};
    use serde::Serialize;
    use serde_json::json;

    const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../src/shared/__fixtures__/serde.ts");

    /// Every variant of a fieldless enum. The match stops compiling when a variant is added and not listed.
    macro_rules! variants {
        ($t:ident: $($v:ident),+) => {{
            let _exhaustive = |x: $t| match x {
                $($t::$v)|+ => (),
            };
            [$($t::$v),+]
        }};
    }

    struct Fixture(String);

    impl Fixture {
        /// A value tsc must accept as `ts` with every field present and none extra.
        fn value(&mut self, name: &str, ts: &str, v: &impl Serialize) {
            let json = serde_json::to_string_pretty(v).unwrap();
            self.0 += &format!("\nexport const {name} = {json} satisfies Full<{ts}>\n");
        }

        /// Values that must be exactly the members of the TS union `ts`.
        fn values(&mut self, name: &str, ts: &str, all: &[impl Serialize]) {
            let json = serde_json::to_string(all).unwrap();
            self.0 += &format!("\nexport const {name} = {json} as const\nexport const _{name}: Same<(typeof {name})[number], {ts}> = true\n");
        }
    }

    fn s(v: &str) -> String {
        v.into()
    }

    fn some(v: &str) -> Option<String> {
        Some(v.into())
    }

    /// Commands registered in lib.rs's generate_handler! list, by function name.
    fn registered_commands() -> Vec<String> {
        let src = include_str!("lib.rs");
        let start = src.find("generate_handler![").expect("generate_handler! in lib.rs") + "generate_handler![".len();
        let list = &src[start..start + src[start..].find(']').unwrap()];
        list.split(',').map(|path| path.rsplit("::").next().unwrap().trim().to_string()).filter(|name| !name.is_empty()).collect()
    }

    #[test]
    fn serde_fixture_matches_committed_file() {
        let mut f = Fixture(
            "// Generated by `cargo test` (src-tauri/src/model.rs, contract_tests) from the Rust serde shapes. Do not edit.\n\
             // Update it with: cd src-tauri && UPDATE_SERDE_FIXTURE=1 cargo test serde_fixture\n\
             import type * as T from '../types'\n\
             import type { Full, Same } from './check'\n"
                .into(),
        );

        let provider = ProviderInfo {
            id: ProviderId::Claude,
            name: s("Claude Code"),
            vendor: s("Anthropic"),
            cli_command: s("claude"),
            cli_installed: true,
            cli_path: some("/usr/local/bin/claude"),
            cli_version: some("2.0.0"),
            desktop_name: some("Claude"),
            desktop_installed: true,
            desktop_path: some("/Applications/Claude.app"),
            desktop_multi_account: false,
            home_env_var: some("CLAUDE_CONFIG_DIR"),
            install_hint: s("npm i -g @anthropic-ai/claude-code"),
        };
        f.value("ProviderInfo", "T.ProviderInfo", &provider);

        let account = Account {
            id: s("acc-1"),
            provider_id: ProviderId::Claude,
            label: s("Work"),
            email: some("me@example.com"),
            plan: some("Max 20x"),
            home_dir: s("/home/me/.claude-work"),
            is_default_home: false,
            shared_from: some("/home/me/.claude"),
            sharing: SharingOptions::default(),
            color: s("#3987E5"),
            tags: vec![s("coding")],
            notes: some("notes"),
            status: AccountStatus::Ok,
            status_message: some("signed in"),
            created_at: s("2026-01-01T00:00:00Z"),
            last_launched_at: some("2026-01-02T00:00:00Z"),
            launch_count: 3,
            monthly_cost: Some(200.0),
            pinned: Some(true),
        };
        f.value("Account", "T.Account", &account);

        let input = NewAccountInput {
            provider_id: ProviderId::Codex,
            label: s("Research"),
            home_dir: some("/home/me/.codex-research"),
            sharing: Some(SharingOptions::NONE),
            shared_from: some("/home/me/.codex"),
            color: some("#D95926"),
            tags: Some(vec![s("research")]),
            monthly_cost: Some(20.0),
            plan: some("Plus"),
        };
        f.value("NewAccountInput", "T.NewAccountInput", &input);

        let detected = DetectedAccount {
            provider_id: ProviderId::Gemini,
            home_dir: s("/home/me/.gemini"),
            is_default_home: true,
            email: some("me@example.com"),
            plan: some("Pro"),
            logged_in: true,
            already_managed: false,
            hint: some("found via ~/.local/bin/gemini-h2"),
        };
        f.value("DetectedAccount", "T.DetectedAccount", &detected);

        let window = UsageWindow {
            id: s("five_hour"),
            label: s("5-hour session"),
            used_percent: 42.5,
            resets_at: some("2026-01-01T05:00:00Z"),
            window_minutes: Some(300),
        };
        let snapshot = UsageSnapshot {
            account_id: s("acc-1"),
            provider_id: ProviderId::Claude,
            fetched_at: s("2026-01-01T00:00:00Z"),
            source: UsageSource::Api,
            windows: vec![window.clone()],
            plan: some("Max 20x"),
            email: some("me@example.com"),
            credits: Some(Credits { used: 1.5, limit: Some(10.0), unit: s("USD") }),
            error: some("timed out"),
            error_kind: Some(ErrorKind::Network), // never serialized
            stale: Some(true),
        };
        f.value("UsageSnapshot", "T.UsageSnapshot", &snapshot);

        let point = UsageHistoryPoint { account_id: s("acc-1"), t: s("2026-01-01T00:00:00Z"), windows: HashMap::from([(s("five_hour"), 42.5)]) };
        f.value("UsageHistoryPoint", "T.UsageHistoryPoint", &point);

        let activity = DailyActivity {
            account_id: s("acc-1"),
            date: s("2026-01-01"),
            input_tokens: 1000,
            output_tokens: 2000,
            cache_tokens: 3000,
            messages: 4,
            sessions: 1,
            models: HashMap::from([(s("claude-opus"), 2000)]),
        };
        f.value("DailyActivity", "T.DailyActivity", &activity);

        let launch =
            LaunchRequest { account_id: s("acc-1"), target: LaunchTarget::Cli, cwd: some("/work"), resume: Some(true), extra_args: Some(vec![s("--verbose")]) };
        f.value("LaunchRequest", "T.LaunchRequest", &launch);
        let launched = LaunchResult { ok: true, error: some("not found"), command_line: some("claude --continue"), note: some("sign in once") };
        f.value("LaunchResult", "T.LaunchResult", &launched);

        let notification = AppNotification {
            id: s("n-1"),
            kind: NotificationKind::LimitReset,
            account_id: some("acc-1"),
            title: s("Reset"),
            body: s("Your limit reset"),
            created_at: s("2026-01-01T00:00:00Z"),
            read: false,
        };
        f.value("AppNotification", "T.AppNotification", &notification);

        let pick = Recommendation { account_id: s("acc-1"), score: 80.0, reason: s("62% of weekly left") };
        f.value("Recommendation", "T.Recommendation", &pick);

        let settings = AppSettings { default_cwd: some("/work"), profiles_dir: some("/home/me/profiles"), ..AppSettings::default() };
        f.value("AppSettings", "T.AppSettings", &settings);

        let spawn = TerminalSpawnRequest {
            account_id: s("acc-1"),
            target: TerminalTarget::Login,
            resume: Some(false),
            cwd: some("/work"),
            extra_args: Some(vec![s("--verbose")]),
            cols: 80,
            rows: 24,
        };
        f.value("TerminalSpawnRequest", "T.TerminalSpawnRequest", &spawn);
        let terminal = TerminalInfo {
            id: s("t-1"),
            account_id: s("acc-1"),
            title: s("Claude Work"),
            command_line: s("claude"),
            cwd: s("/work"),
            pid: Some(1234),
            started_at: s("2026-01-01T00:00:00Z"),
            exited: true,
            exit_code: Some(0),
        };
        f.value("TerminalInfo", "T.TerminalInfo", &terminal);

        let knowledge = ProviderKnowledge {
            provider_id: ProviderId::Claude,
            folder: s("/home/me/profiles/shared/claude"),
            main_folder: s("/home/me/.claude"),
            items: vec![KnowledgeItem { kind: s("skills"), name: s("pdf"), path: s("/home/me/profiles/shared/claude/skills/pdf") }],
            accounts: vec![KnowledgeAccount { account_id: s("acc-1"), label: s("Work"), is_default_home: false, kinds: vec![s("skills")] }],
        };
        f.value("ProviderKnowledge", "T.ProviderKnowledge", &knowledge);
        f.value("ReseedResult", "Awaited<ReturnType<T.AgentHubApi['knowledge']['reseed']>>", &ReseedResult { added: 1, kept: 2 });

        let skill = DirectorySkill { id: s("owner/repo/pdf"), name: s("pdf"), source: s("owner/repo"), installs: 10 };
        f.value("DirectorySkill", "T.DirectorySkill", &skill);
        let preview = SkillPreview { id: s("owner/repo/pdf"), name: s("pdf"), description: s("Read PDFs"), body: s("# PDF"), files: 2 };
        f.value("SkillPreview", "T.SkillPreview", &preview);
        let installed = InstalledSkill { provider_id: ProviderId::Codex, name: s("pdf"), id: some("owner/repo/pdf") };
        f.value("InstalledSkill", "T.InstalledSkill", &installed);
        let outcome = SkillInstallOutcome { provider_id: ProviderId::Codex, error: some("already there") };
        f.value("SkillInstallOutcome", "T.SkillInstallOutcome", &outcome);

        // The UI only sends these two, so Rust can't serialize them. Each JSON object must deserialize
        // with every Option set, and the exhaustive destructuring stops compiling when Rust adds a field.
        let export_options = json!({ "accountIds": ["acc-1"], "includeFiles": true, "includeSettings": true, "passphrase": "secret" });
        let ExportOptions { account_ids: _, include_files: _, include_settings: _, passphrase } = serde_json::from_value(export_options.clone()).unwrap();
        assert!(passphrase.is_some(), "ExportOptions.passphrase was not read");
        f.value("ExportOptions", "T.ExportOptions", &export_options);
        let import_choice = json!({ "accountKeys": ["a1"], "signIns": true, "files": true, "settings": false });
        let ImportChoice { account_keys: _, sign_ins: _, files: _, settings: _ } = serde_json::from_value(import_choice.clone()).unwrap();
        f.value("ImportChoice", "T.ImportChoice", &import_choice);

        f.value("ExportSummary", "T.ExportSummary", &ExportSummary { accounts: 2, sign_ins: 1, files: 10, skipped_files: 0 });
        let import_preview = ImportPreview {
            from_os: s("macos"),
            exported_at: s("2026-01-01T00:00:00Z"),
            accounts: vec![PreviewAccount {
                key: s("a1"),
                provider_id: ProviderId::Claude,
                label: s("Work"),
                email: some("me@example.com"),
                plan: some("Max 20x"),
                has_sign_in: true,
                already_here: some("Work"),
                target_home: s("/home/me/profiles/claude-work"),
            }],
            kinds: vec![PreviewKind { provider_id: ProviderId::Claude, kind: s("skills"), files: 3, new_files: 1 }],
            has_settings: true,
            has_sign_ins: true,
            sign_ins_unlocked: false,
        };
        f.value("ImportPreview", "T.ImportPreview", &import_preview);
        let report = ImportReport {
            added: vec![s("Work")],
            skipped: vec![s("Home")],
            sign_ins: 1,
            files_added: 3,
            files_kept: 1,
            settings: true,
            problems: vec![s("one file was unreadable")],
        };
        f.value("ImportReport", "T.ImportReport", &report);

        f.values("ProviderId", "T.ProviderId", &variants!(ProviderId: Claude, Codex, Gemini, Copilot));
        f.values("AccountStatus", "T.AccountStatus", &variants!(AccountStatus: Ok, NeedsLogin, Expired, Error, Unknown));
        f.values("UsageSource", "T.UsageSnapshot['source']", &variants!(UsageSource: Api, Cli, LocalLogs, Manual, None));
        f.values("LaunchTarget", "T.LaunchTarget", &variants!(LaunchTarget: Cli, Desktop, Login));
        f.values(
            "NotificationKind",
            "T.NotificationKind",
            &variants!(NotificationKind: LimitReached, LimitReset, NearlyExhausted, UseItOrLoseIt, SwitchSuggested, LoginRequired, Info),
        );
        f.values("TaskType", "T.TaskType", &variants!(TaskType: Coding, LongRefactor, QuickQuestion, Research, Review, Any));
        f.values("Theme", "T.AppSettings['theme']", &variants!(Theme: System, Dark, Light));
        f.values("TerminalTarget", "T.TerminalSpawnRequest['target']", &variants!(TerminalTarget: Cli, Login));
        for p in variants!(ProviderId: Claude, Codex, Gemini, Copilot) {
            let name = format!("PermissionModes_{}", p.as_str());
            f.values(&name, &format!("T.PermissionModes['{}']", p.as_str()), PermissionModes::allowed(p));
        }
        f.values("commands", "(typeof T.COMMANDS)[keyof typeof T.COMMANDS]", &registered_commands());

        let committed = std::fs::read_to_string(FIXTURE).unwrap_or_default().replace("\r\n", "\n");
        if std::env::var_os("UPDATE_SERDE_FIXTURE").is_some() {
            std::fs::write(FIXTURE, &f.0).unwrap();
        } else {
            assert!(
                committed == f.0,
                "src/shared/__fixtures__/serde.ts is stale. Run `UPDATE_SERDE_FIXTURE=1 cargo test serde_fixture` in src-tauri, then `npm run typecheck`."
            );
        }
    }
}
