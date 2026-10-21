//! GitHub Copilot CLI adapter.
//!
//! Isolation: Copilot CLI keeps config and state in `COPILOT_HOME` (default `~/.copilot`); the CLI's
//! own help says "COPILOT_HOME: override the directory where configuration and state files are stored".
//! (`--config-dir` is deprecated in favour of it; XDG_CONFIG_HOME/XDG_STATE_HOME are only read once to
//! migrate old installs into ~/.copilot.) Verified in @github/copilot 1.0.60 (last npm build shipping
//! plain JS; 1.0.100 is a native binary that still contains `COPILOT_HOME`).
//!
//! Auth: `config.json` in the home records `loggedInUsers` / `lastLoggedInUser` (`{host, login}`;
//! older builds wrote snake_case keys). The OAuth token itself goes to the OS keychain via keytar
//! (service `copilot-cli`, account `<host>:<login>`), or, when `storeTokenPlaintext` is on or no keychain
//! is available, into `config.json` → `copilotTokens["<host>:<login>"]`. Because every home records its
//! own logged-in user, per-home isolation works even with the shared keychain: each home looks up only
//! its own user's entry. COPILOT_GITHUB_TOKEN > GH_TOKEN > GITHUB_TOKEN override stored credentials, so
//! the launcher strips them.
//!
//! Usage: `GET https://api.github.com/copilot_internal/user` (headers as in steipete/CodexBar
//! docs/copilot.md). We never read the keychain (that could pop OS prompts during background polls);
//! the token comes from the plaintext store if present, else from GitHub CLI: `gh auth token --user <login>`.
use super::common::{finish_days, first_local_date, local_date, num, parse_jsonc, scan_launcher_scripts, DayMap, VersionCache};
use super::{CommandSpec, Identity, ProviderAdapter};
use crate::model::*;
use crate::util;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

const HOME_ENV: &str = "COPILOT_HOME";
const DEFAULT_HOST: &str = "https://github.com";
const NEED_GH: &str = "Connect GitHub CLI (gh auth login) to read Copilot usage";

pub struct CopilotAdapter {
    version: VersionCache,
}

impl CopilotAdapter {
    pub fn new() -> Self {
        Self { version: VersionCache::default() }
    }
}

impl Default for CopilotAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// Read `key` in camelCase or snake_case.
fn field<'a>(v: &'a Value, camel: &str, snake: &str) -> Option<&'a Value> {
    v.get(camel).or_else(|| v.get(snake)).filter(|v| !v.is_null())
}

#[derive(Debug, Clone, PartialEq)]
struct GhUser {
    host: String,
    login: String,
}

impl GhUser {
    /// Hostname for gh: "https://github.com" → "github.com".
    fn hostname(&self) -> String {
        self.host.trim_start_matches("https://").trim_start_matches("http://").trim_end_matches('/').to_string()
    }

    fn api_base(&self) -> String {
        match self.hostname().as_str() {
            "github.com" | "" => "https://api.github.com".into(),
            h => format!("https://api.{h}"),
        }
    }

    fn token_key(&self) -> String {
        format!("{}:{}", self.host, self.login)
    }
}

fn parse_user(v: &Value) -> Option<GhUser> {
    let login = v.get("login")?.as_str()?.trim();
    if login.is_empty() {
        return None;
    }
    let host = v.get("host").and_then(Value::as_str).filter(|h| !h.is_empty()).unwrap_or(DEFAULT_HOST);
    Some(GhUser { host: host.to_string(), login: login.to_string() })
}

/// `config.json` (or the legacy extension-less `config`) of a home.
fn read_config(home: &Path) -> Option<Value> {
    ["config.json", "config"].iter().find_map(|f| {
        let s = std::fs::read_to_string(home.join(f)).ok()?;
        parse_jsonc(&s).filter(Value::is_object)
    })
}

/// The active user: lastLoggedInUser, else the first loggedInUsers entry.
fn active_user(config: &Value) -> Option<GhUser> {
    field(config, "lastLoggedInUser", "last_logged_in_user")
        .and_then(parse_user)
        .or_else(|| field(config, "loggedInUsers", "logged_in_users")?.as_array()?.iter().find_map(parse_user))
}

fn plaintext_token(config: &Value, user: &GhUser) -> Option<String> {
    let tokens = field(config, "copilotTokens", "copilot_tokens")?.as_object()?;
    tokens.get(&user.token_key()).and_then(Value::as_str).filter(|t| !t.trim().is_empty()).map(|t| t.trim().to_string())
}

fn read_identity_sync(home: &Path) -> Identity {
    let user = read_config(home).as_ref().and_then(active_user);
    Identity {
        logged_in: user.is_some(),
        // Copilot knows the GitHub login, not an email; show "login" (or "login@host" for enterprise).
        email: user.map(|u| display_login(&u)),
        plan: None,
    }
}

fn display_login(u: &GhUser) -> String {
    if u.hostname() == "github.com" {
        u.login.clone()
    } else {
        format!("{}@{}", u.login, u.hostname())
    }
}

// ---------- usage parsing ----------

#[derive(Debug, Default)]
struct CopilotUsage {
    plan: Option<String>,
    windows: Vec<UsageWindow>,
    credits: Option<Credits>,
}

/// One quota snapshot (`premium_interactions`, `chat`, `completions`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Quota {
    entitlement: Option<f64>,
    remaining: Option<f64>,
    percent_remaining: Option<f64>,
    unlimited: bool,
}

impl Quota {
    fn parse(v: &Value) -> Option<Self> {
        v.is_object().then(|| Self {
            entitlement: num(field(v, "entitlement", "entitlement")),
            remaining: num(field(v, "remaining", "remaining")),
            percent_remaining: num(field(v, "percentRemaining", "percent_remaining")),
            unlimited: field(v, "unlimited", "unlimited").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    /// Zero entitlement and zero remaining carries no signal (token-billed / Business seats).
    fn is_placeholder(&self) -> bool {
        !self.unlimited && self.entitlement.unwrap_or(0.0) == 0.0 && self.remaining.unwrap_or(0.0) == 0.0
    }

    fn used_percent(&self) -> Option<f64> {
        let left = self.percent_remaining.or_else(|| match (self.entitlement, self.remaining) {
            (Some(e), Some(r)) if e > 0.0 => Some(r / e * 100.0),
            _ => None,
        })?;
        Some(((100.0 - left).clamp(0.0, 100.0) * 10.0).round() / 10.0)
    }
}

fn parse_usage(v: &Value) -> CopilotUsage {
    let snapshots = field(v, "quotaSnapshots", "quota_snapshots");
    let snap = |camel: &str, snake: &str| snapshots.and_then(|s| field(s, camel, snake)).and_then(Quota::parse);
    let reset = field(v, "quotaResetDateUtc", "quota_reset_date_utc")
        .or_else(|| field(v, "quotaResetDate", "quota_reset_date"))
        .or_else(|| field(v, "limitedUserResetDate", "limited_user_reset_date"))
        .and_then(Value::as_str)
        .and_then(reset_iso);
    let mut out = CopilotUsage { plan: plan_label(v), ..Default::default() };
    let mut push = |id: &str, label: &str, q: Quota| {
        if q.unlimited || q.is_placeholder() {
            return;
        }
        if let Some(used) = q.used_percent() {
            out.windows.push(UsageWindow { id: id.into(), label: label.into(), used_percent: used, resets_at: reset.clone(), window_minutes: None });
        }
    };
    let premium = snap("premiumInteractions", "premium_interactions");
    if let Some(q) = premium {
        push("premium", "Premium requests", q);
    }
    let chat = snap("chat", "chat").or_else(|| free_quota(v, "chat"));
    if let Some(q) = chat {
        push("chat", "Chat", q);
    }
    let completions = snap("completions", "completions").or_else(|| free_quota(v, "completions"));
    if let Some(q) = completions {
        push("completions", "Code completions", q);
    }
    out.credits = premium.filter(|q| !q.unlimited && !q.is_placeholder()).and_then(|q| {
        let limit = q.entitlement.filter(|e| *e > 0.0)?;
        let used = match (q.remaining, q.percent_remaining) {
            (Some(r), _) => limit - r,
            (None, Some(p)) => limit * (1.0 - p / 100.0),
            _ => return None,
        };
        Some(Credits { used: used.max(0.0).round(), limit: Some(limit), unit: "premium requests".into() })
    });
    out
}

/// Free plan shape: `monthly_quotas` (limits) + `limited_user_quotas` (remaining).
fn free_quota(v: &Value, key: &str) -> Option<Quota> {
    let total = num(field(v, "monthlyQuotas", "monthly_quotas").and_then(|m| m.get(key)))?;
    let left = num(field(v, "limitedUserQuotas", "limited_user_quotas").and_then(|m| m.get(key)))?;
    (total > 0.0).then_some(Quota { entitlement: Some(total), remaining: Some(left.max(0.0)), percent_remaining: None, unlimited: false })
}

/// "2026-11-01" → midnight UTC; full timestamps are normalised.
fn reset_iso(s: &str) -> Option<String> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(d.with_timezone(&chrono::Utc).to_rfc3339());
    }
    let date = chrono::NaiveDate::parse_from_str(s.get(..10)?, "%Y-%m-%d").ok()?;
    Some(date.and_hms_opt(0, 0, 0)?.and_utc().to_rfc3339())
}

fn plan_label(v: &Value) -> Option<String> {
    let plan = field(v, "copilotPlan", "copilot_plan").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let sku = field(v, "accessTypeSku", "access_type_sku").and_then(Value::as_str).unwrap_or("").to_lowercase();
    let label = match plan.as_str() {
        "" | "unknown" => return None,
        "free" | "individual_free" => "Copilot Free".to_string(),
        "individual" if sku.contains("plus") || sku.contains("pro_plus") => "Copilot Pro+".to_string(),
        "individual" => "Copilot Pro".to_string(),
        "individual_pro" | "pro_plus" | "pro+" | "individual_plus" => "Copilot Pro+".to_string(),
        "business" => "Copilot Business".to_string(),
        "enterprise" => "Copilot Enterprise".to_string(),
        other => {
            let mut c = other.replace('_', " ").chars().collect::<Vec<_>>();
            if let Some(f) = c.first_mut() {
                *f = f.to_ascii_uppercase();
            }
            format!("Copilot {}", c.into_iter().collect::<String>())
        }
    };
    Some(label)
}

// ---------- token + fetch ----------

fn gh_path() -> Option<PathBuf> {
    util::which("gh").or_else(|| {
        let p = PathBuf::from(r"C:\Program Files\GitHub CLI\gh.exe");
        (cfg!(windows) && p.is_file()).then_some(p)
    })
}

async fn gh_token(user: &GhUser) -> Option<String> {
    let gh = gh_path()?;
    let host = user.hostname();
    let args = ["auth", "token", "--hostname", host.as_str(), "--user", user.login.as_str()];
    util::run_capture(&gh, &args, Duration::from_secs(10)).await.filter(|t| !t.is_empty() && !t.contains(char::is_whitespace))
}

impl CopilotAdapter {
    async fn fetch_live(&self, account: &Account) -> Result<UsageSnapshot, String> {
        let home = Path::new(&account.home_dir);
        let config = read_config(home);
        let Some(user) = config.as_ref().and_then(active_user) else {
            return Err("Not signed in. Run `copilot login` for this account".into());
        };
        let token = match config.as_ref().and_then(|c| plaintext_token(c, &user)) {
            Some(t) => t,
            None => gh_token(&user).await.ok_or(NEED_GH)?,
        };
        let resp = util::http()
            .get(format!("{}/copilot_internal/user", user.api_base()))
            .header(reqwest::header::AUTHORIZATION, format!("token {token}"))
            .header(reqwest::header::ACCEPT, "application/json")
            .header("Editor-Version", "vscode/1.96.2")
            .header("Editor-Plugin-Version", "copilot-chat/0.26.7")
            .header(reqwest::header::USER_AGENT, "GitHubCopilotChat/0.26.7")
            .header("X-Github-Api-Version", "2025-04-01")
            .send()
            .await
            .map_err(|e| format!("Could not reach GitHub: {e}"))?;
        let status = resp.status().as_u16();
        match status {
            200..=299 => {}
            401 => return Err(format!("GitHub rejected the token for {}. {NEED_GH}", user.login)),
            403 | 404 => return Err(format!("{} has no Copilot access (HTTP {status})", user.login)),
            _ => return Err(format!("Copilot usage request failed (HTTP {status})")),
        }
        let v: Value = resp.json().await.map_err(|_| "GitHub returned an unexpected Copilot usage response".to_string())?;
        let usage = parse_usage(&v);
        Ok(UsageSnapshot {
            account_id: account.id.clone(),
            provider_id: ProviderId::Copilot,
            fetched_at: util::now_iso(),
            source: UsageSource::Api,
            error: (usage.windows.is_empty() && usage.plan.is_none()).then(|| "GitHub returned no Copilot quotas".to_string()),
            windows: usage.windows,
            plan: usage.plan,
            email: Some(display_login(&user)),
            credits: usage.credits,
            stale: None,
        })
    }
}

// ---------- activity: session-state/<id>/events.jsonl ----------

/// Per-model cumulative totals from a `session.shutdown` event.
type ModelTotals = HashMap<String, [u64; 5]>; // input, output, cache_read, cache_write, requests

fn shutdown_totals(data: &Value) -> ModelTotals {
    let mut out = ModelTotals::new();
    for (model, m) in data.get("modelMetrics").and_then(Value::as_object).into_iter().flatten() {
        let u = m.get("usage").cloned().unwrap_or(Value::Null);
        let g = |k: &str| num(u.get(k)).unwrap_or(0.0).max(0.0) as u64;
        let req = num(m.pointer("/requests/count")).unwrap_or(0.0).max(0.0) as u64;
        out.insert(model.clone(), [g("inputTokens"), g("outputTokens") + g("reasoningTokens"), g("cacheReadTokens"), g("cacheWriteTokens"), req]);
    }
    out
}

/// Fold one session's events into per-day totals. Token usage is only persisted in `session.shutdown`
/// (per-call `assistant.usage` events are ephemeral). A resumed session can log several shutdowns; if a
/// later one is cumulative (every counter >= the previous one) only the difference is counted.
/// `inputTokens` follows the OpenAI convention of including cached reads, so cache reads are subtracted.
fn fold_copilot_events(content: &str, days: &mut DayMap) {
    let mut prev: ModelTotals = ModelTotals::new();
    for line in content.lines() {
        let Ok(ev) = serde_json::from_str::<Value>(line) else { continue };
        let Some(kind) = ev.get("type").and_then(Value::as_str) else { continue };
        let Some(day) = ev.get("timestamp").and_then(Value::as_str).and_then(local_date) else { continue };
        match kind {
            "session.start" => days.entry(day).or_default().sessions += 1,
            "assistant.message" => days.entry(day).or_default().messages += 1,
            "session.shutdown" => {
                let data = ev.get("data").cloned().unwrap_or(Value::Null);
                let totals = shutdown_totals(&data);
                let cumulative = !prev.is_empty() && prev.iter().all(|(m, p)| totals.get(m).is_some_and(|t| t.iter().zip(p).all(|(a, b)| a >= b)));
                // Tokens are attributed to the day the run ended (the shutdown event's timestamp).
                let d = days.entry(day).or_default();
                for (model, t) in &totals {
                    let base = if cumulative { prev.get(model).copied().unwrap_or_default() } else { [0; 5] };
                    let delta: Vec<u64> = t.iter().zip(base.iter()).map(|(a, b)| a.saturating_sub(*b)).collect();
                    let (input, output, cache_read, cache_write) = (delta[0], delta[1], delta[2], delta[3]);
                    d.input_tokens += input.saturating_sub(cache_read);
                    d.cache_tokens += cache_read + cache_write;
                    d.output_tokens += output;
                    *d.models.entry(model.clone()).or_default() += input + output;
                }
                prev = totals;
            }
            _ => {}
        }
    }
}

fn scan_copilot_activity(home: &Path, account_id: &str, since_days: u32) -> Vec<DailyActivity> {
    let cutoff = first_local_date(since_days);
    let cutoff_time =
        std::time::SystemTime::now().checked_sub(Duration::from_secs(u64::from(since_days.max(1)) * 86_400 + 86_400)).unwrap_or(std::time::UNIX_EPOCH);
    let mut days = DayMap::new();
    let Ok(sessions) = std::fs::read_dir(home.join("session-state")) else { return vec![] };
    for s in sessions.flatten() {
        let events = s.path().join("events.jsonl");
        let fresh = std::fs::metadata(&events).ok().and_then(|m| m.modified().ok()).is_some_and(|t| t >= cutoff_time);
        if !fresh {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&events) {
            fold_copilot_events(&content, &mut days);
        }
    }
    finish_days(days, account_id, &cutoff)
}

fn base_command(account: &Account) -> CommandSpec {
    let program = util::which("copilot").map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "copilot".into());
    let mut spec = CommandSpec { program, ..Default::default() };
    spec.env.insert(HOME_ENV.into(), Some(account.home_dir.clone()));
    // These take precedence over the home's stored login, so drop them to use this account.
    for var in ["COPILOT_GITHUB_TOKEN", "GH_TOKEN", "GITHUB_TOKEN"] {
        spec.env.insert(var.into(), None);
    }
    spec
}

#[async_trait]
impl ProviderAdapter for CopilotAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Copilot
    }

    async fn info(&self) -> ProviderInfo {
        let (path, version) = self.version.get("copilot").await;
        ProviderInfo {
            id: ProviderId::Copilot,
            name: "GitHub Copilot CLI".into(),
            vendor: "GitHub".into(),
            cli_command: "copilot".into(),
            cli_installed: path.is_some(),
            cli_path: path,
            cli_version: version,
            desktop_name: None,
            desktop_installed: false,
            desktop_path: None,
            desktop_multi_account: false,
            home_env_var: Some(HOME_ENV.into()),
            install_hint: "npm i -g @github/copilot".into(),
        }
    }

    fn default_home(&self) -> PathBuf {
        std::env::var_os(HOME_ENV).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(|| util::home().join(".copilot"))
    }

    async fn detect_accounts(&self) -> Vec<DetectedAccount> {
        let default = self.default_home();
        let mut candidates: Vec<(PathBuf, Option<String>)> = vec![(default.clone(), None)];
        if let Ok(entries) = std::fs::read_dir(util::home()) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(".copilot-") && e.path().is_dir() {
                    candidates.push((e.path(), Some(format!("Found {name}"))));
                }
            }
        }
        for (p, _) in scan_launcher_scripts(HOME_ENV) {
            candidates.push((p, Some(format!("Referenced by a launcher script ({HOME_ENV})"))));
        }
        let mut out: Vec<DetectedAccount> = vec![];
        for (home, hint) in candidates {
            let id = read_identity_sync(&home);
            // A real home has a logged-in user, or at least a config file (side-by-side homes).
            let is_default = util::path_key(&home) == util::path_key(&default);
            if !id.logged_in && (is_default || read_config(&home).is_none()) {
                continue;
            }
            if out.iter().any(|o| util::path_key(Path::new(&o.home_dir)) == util::path_key(&home)) {
                continue;
            }
            out.push(DetectedAccount {
                provider_id: ProviderId::Copilot,
                home_dir: home.to_string_lossy().into_owned(),
                is_default_home: is_default,
                email: id.email,
                plan: None,
                logged_in: id.logged_in,
                already_managed: false,
                hint,
            });
        }
        out
    }

    async fn read_identity(&self, home: &Path) -> Identity {
        read_identity_sync(home)
    }

    fn shareable_paths(&self) -> Vec<(ShareKind, Vec<&'static str>)> {
        vec![
            (ShareKind::Skills, vec!["skills"]),
            // installedPlugins metadata lives in config.json (with the login), so only the payload dirs are shared.
            (ShareKind::Plugins, vec!["installed-plugins", "extensions"]),
            (ShareKind::Agents, vec!["agents"]),
            (ShareKind::Instructions, vec!["copilot-instructions.md"]),
            (ShareKind::Settings, vec!["settings.json", "hooks"]),
            (ShareKind::Mcp, vec!["mcp-config.json"]),
            (ShareKind::History, vec!["session-state", "session-store.db", "command-history-state.json"]),
        ]
    }

    fn credential_paths(&self) -> Vec<&'static str> {
        vec!["config.json", "config", "mcp-oauth-config"]
    }

    async fn init_home(&self, home: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(home)?;
        Ok(())
    }

    async fn cli_command(&self, account: &Account, resume: bool, extra_args: &[String]) -> anyhow::Result<CommandSpec> {
        let mut spec = base_command(account);
        if resume {
            spec.args.push("--continue".into());
        }
        spec.args.extend(extra_args.iter().cloned());
        Ok(spec)
    }

    async fn login_command(&self, account: &Account) -> anyhow::Result<CommandSpec> {
        let mut spec = base_command(account);
        spec.args.push("login".into());
        if let Some(user) = read_config(Path::new(&account.home_dir)).as_ref().and_then(active_user) {
            if user.hostname() != "github.com" {
                spec.args.extend(["--host".into(), user.host.clone()]);
            }
        }
        Ok(spec)
    }

    async fn fetch_usage(&self, account: &Account, live: bool) -> UsageSnapshot {
        if !live {
            return UsageSnapshot::failed(account, UsageSource::None, "Copilot CLI keeps no local record of its limits; turn on live usage to see them");
        }
        match self.fetch_live(account).await {
            Ok(s) => s,
            Err(e) => {
                let mut s = UsageSnapshot::failed(account, UsageSource::Api, e);
                s.email = read_identity_sync(Path::new(&account.home_dir)).email;
                s
            }
        }
    }

    async fn scan_activity(&self, account: &Account, since_days: u32) -> Vec<DailyActivity> {
        let home = PathBuf::from(&account.home_dir);
        let id = account.id.clone();
        tokio::task::spawn_blocking(move || scan_copilot_activity(&home, &id, since_days)).await.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn account(home: &Path) -> Account {
        Account {
            id: "c1".into(),
            provider_id: ProviderId::Copilot,
            label: "Work".into(),
            email: None,
            plan: None,
            home_dir: home.to_string_lossy().into_owned(),
            is_default_home: false,
            shared_from: None,
            sharing: SharingOptions::default(),
            color: "#000".into(),
            tags: vec![],
            notes: None,
            status: AccountStatus::Unknown,
            status_message: None,
            created_at: util::now_iso(),
            last_launched_at: None,
            launch_count: 0,
            monthly_cost: None,
            pinned: None,
        }
    }

    #[test]
    fn config_identity_and_plaintext_token() {
        let cfg = json!({
            "firstLaunchAt": "2026-01-01T00:00:00Z",
            "loggedInUsers": [{"host": "https://github.com", "login": "octo"}, {"host": "https://acme.ghe.com", "login": "octo-acme"}],
            "lastLoggedInUser": {"host": "https://acme.ghe.com", "login": "octo-acme"},
            "copilotTokens": {"https://acme.ghe.com:octo-acme": "gho_secret"}
        });
        let u = active_user(&cfg).unwrap();
        assert_eq!(u.login, "octo-acme");
        assert_eq!(u.hostname(), "acme.ghe.com");
        assert_eq!(u.api_base(), "https://api.acme.ghe.com");
        assert_eq!(display_login(&u), "octo-acme@acme.ghe.com");
        assert_eq!(plaintext_token(&cfg, &u).as_deref(), Some("gho_secret"));

        let snake = parse_jsonc("{\n // legacy\n \"logged_in_users\": [{\"host\":\"https://github.com\",\"login\":\"octo\"}],\n}").unwrap();
        let u = active_user(&snake).unwrap();
        assert_eq!(u.api_base(), "https://api.github.com");
        assert!(plaintext_token(&snake, &u).is_none());
        assert!(active_user(&json!({"loggedInUsers": []})).is_none());
    }

    #[test]
    fn usage_paid_plan() {
        let v = json!({
            "login": "octo",
            "access_type_sku": "plus_monthly_subscriber_quota",
            "copilot_plan": "individual",
            "quota_reset_date": "2026-11-01",
            "quota_snapshots": {
                "chat": {"entitlement": 0, "remaining": 0, "percent_remaining": 100.0, "unlimited": true, "overage_count": 0, "quota_id": "chat"},
                "completions": {"entitlement": 0, "remaining": 0, "percent_remaining": 100.0, "unlimited": true, "quota_id": "completions"},
                "premium_interactions": {"entitlement": 1500, "remaining": 1125, "percent_remaining": 75.0, "unlimited": false, "overage_count": 0, "overage_permitted": true, "quota_id": "premium_interactions"}
            }
        });
        let u = parse_usage(&v);
        assert_eq!(u.plan.as_deref(), Some("Copilot Pro+"));
        assert_eq!(u.windows.len(), 1);
        assert_eq!(u.windows[0].id, "premium");
        assert_eq!(u.windows[0].used_percent, 25.0);
        assert!(u.windows[0].resets_at.as_deref().unwrap().starts_with("2026-11-01T00:00:00"));
        let c = u.credits.unwrap();
        assert_eq!((c.used, c.limit), (375.0, Some(1500.0)));
    }

    #[test]
    fn usage_camel_case_and_free_plan() {
        let camel = json!({
            "copilotPlan": "business",
            "quotaResetDateUtc": "2026-11-01T00:00:00.000Z",
            "quotaSnapshots": {"premiumInteractions": {"entitlement": 300, "remaining": 0, "percentRemaining": 0, "unlimited": false}}
        });
        let u = parse_usage(&camel);
        assert_eq!(u.plan.as_deref(), Some("Copilot Business"));
        assert_eq!(u.windows[0].used_percent, 100.0);

        let free = json!({
            "copilot_plan": "free",
            "access_type_sku": "free_limited_copilot",
            "limited_user_reset_date": "2026-11-05",
            "monthly_quotas": {"chat": 50, "completions": 2000},
            "limited_user_quotas": {"chat": 40, "completions": 500}
        });
        let u = parse_usage(&free);
        assert_eq!(u.plan.as_deref(), Some("Copilot Free"));
        let ids: Vec<_> = u.windows.iter().map(|w| (w.id.as_str(), w.used_percent)).collect();
        assert_eq!(ids, [("chat", 20.0), ("completions", 75.0)]);
        assert!(u.windows[0].resets_at.as_deref().unwrap().starts_with("2026-11-05"));
        assert!(u.credits.is_none());

        // Token-billed seat: zero/zero placeholder is dropped.
        let seat = json!({"copilot_plan": "enterprise", "quota_snapshots": {"premium_interactions": {"entitlement": 0, "remaining": 0, "percent_remaining": 100, "unlimited": false}}});
        let u = parse_usage(&seat);
        assert!(u.windows.is_empty());
        assert_eq!(u.plan.as_deref(), Some("Copilot Enterprise"));
    }

    #[test]
    fn events_activity_with_cumulative_shutdowns() {
        let ts = "2026-10-18T12:00:00.000Z";
        let shutdown = |input: u64, out: u64, cache: u64, req: u64| {
            json!({"type": "session.shutdown", "id": "s", "parentId": null, "timestamp": ts, "data": {
                "shutdownType": "routine", "sessionStartTime": 1_760_788_800_000_i64, "totalApiDurationMs": 10,
                "codeChanges": {"filesModified": [], "linesAdded": 0, "linesRemoved": 0},
                "modelMetrics": {"claude-sonnet-4.5": {"requests": {"count": req, "cost": 1}, "usage": {"inputTokens": input, "outputTokens": out, "cacheReadTokens": cache, "cacheWriteTokens": 0}}}
            }})
            .to_string()
        };
        let lines = [
            json!({"type": "session.start", "id": "1", "parentId": null, "timestamp": ts, "data": {"sessionId": "abc"}}).to_string(),
            json!({"type": "user.message", "id": "2", "parentId": "1", "timestamp": ts, "data": {"content": "hi"}}).to_string(),
            json!({"type": "assistant.message", "id": "3", "parentId": "2", "timestamp": ts, "data": {"content": "hello"}}).to_string(),
            shutdown(1000, 100, 600, 2),
            json!({"type": "session.resume", "id": "4", "parentId": "3", "timestamp": ts}).to_string(),
            json!({"type": "assistant.message", "id": "5", "parentId": "4", "timestamp": ts, "data": {"content": "again"}}).to_string(),
            shutdown(1500, 150, 800, 3),
            "garbage".to_string(),
        ]
        .join("\n");
        let mut days = DayMap::new();
        fold_copilot_events(&lines, &mut days);
        let d = &days[&local_date(ts).unwrap()];
        assert_eq!((d.sessions, d.messages), (1, 2));
        assert_eq!(d.input_tokens, 1500 - 800);
        assert_eq!(d.cache_tokens, 800);
        assert_eq!(d.output_tokens, 150);
        assert_eq!(d.models["claude-sonnet-4.5"], 1650);
    }

    #[tokio::test]
    async fn home_detection_commands_and_errors() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("copilot-work");
        let a = CopilotAdapter::new();
        a.init_home(&home).await.unwrap();
        let acc = account(&home);
        let snap = a.fetch_usage(&acc, true).await;
        assert!(snap.error.unwrap().contains("copilot login"));
        assert!(!a.read_identity(&home).await.logged_in);

        std::fs::write(home.join("config.json"), r#"{"lastLoggedInUser":{"host":"https://github.com","login":"octo"}}"#).unwrap();
        let id = a.read_identity(&home).await;
        assert!(id.logged_in);
        assert_eq!(id.email.as_deref(), Some("octo"));

        let spec = a.cli_command(&acc, true, &["--model".into(), "gpt-5".into()]).await.unwrap();
        assert_eq!(spec.args, ["--continue", "--model", "gpt-5"]);
        assert_eq!(spec.env.get("COPILOT_HOME").cloned().flatten().as_deref(), Some(acc.home_dir.as_str()));
        assert_eq!(spec.env.get("GH_TOKEN"), Some(&None));
        assert_eq!(a.login_command(&acc).await.unwrap().args, ["login"]);
        assert_eq!(a.fetch_usage(&acc, false).await.source, UsageSource::None);
        assert!(a.desktop_command(&acc).await.unwrap().is_none());

        let creds = a.credential_paths();
        for (_, paths) in a.shareable_paths() {
            assert!(paths.iter().all(|p| !creds.contains(p)));
        }
    }
}
