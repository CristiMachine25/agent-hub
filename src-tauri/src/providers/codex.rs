//! OpenAI Codex CLI adapter (ChatGPT subscription: Plus / Pro / Team / ...).
//!
//! Home layout (CODEX_HOME, default ~/.codex):
//! - `auth.json` → `{auth_mode, OPENAI_API_KEY, tokens: {id_token, access_token, refresh_token, account_id}, last_refresh}`.
//!   Email and plan come from the id_token JWT claims (`email`, `https://api.openai.com/auth.chatgpt_plan_type`).
//! - `config.toml` → `cli_auth_credentials_store = "file"` keeps credentials in auth.json; `chatgpt_base_url`.
//! - `sessions/YYYY/MM/DD/rollout-*.jsonl` (+ `archived_sessions/`) → `event_msg` / `token_count` events carry
//!   cumulative token usage and the latest `rate_limits {primary, secondary, credits, plan_type}`.
//!
//! Live usage: `GET <chatgpt_base_url>/wham/usage`; fallback `codex app-server` JSON-RPC
//! (`account/read`, `account/rateLimits/read`), which also lets Codex refresh its own tokens.
use super::common::*;
use super::{CommandSpec, FetchError, Identity, ProviderAdapter};
use crate::model::*;
use crate::util;
use async_trait::async_trait;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_BASE_URL: &str = "https://chatgpt.com/backend-api/";
const AUTH_CLAIMS: &str = "https://api.openai.com/auth";
const APP_SERVER_TIMEOUT: Duration = Duration::from_secs(10);

pub struct CodexAdapter {
    version: VersionCache,
}

impl CodexAdapter {
    pub fn new() -> Self {
        Self { version: VersionCache::default() }
    }

    fn is_default(&self, home: &Path) -> bool {
        util::path_key(home) == util::path_key(&self.default_home())
    }
}

impl Default for CodexAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------------
// auth.json and identity
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Default, Clone)]
struct CodexAuth {
    access_token: Option<String>,
    refresh_token: Option<String>,
    account_id: Option<String>,
    api_key: Option<String>,
    claims: Option<Value>,
}

/// Decode a JWT payload (no signature check: only used to show email/plan).
fn jwt_claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?.trim_end_matches('=');
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn parse_auth(v: &Value) -> CodexAuth {
    let tokens = v.get("tokens").cloned().unwrap_or(Value::Null);
    let claims = str_field(&tokens, "id_token").and_then(jwt_claims);
    let claim_account = claims.as_ref().and_then(|c| c.get(AUTH_CLAIMS)).and_then(|a| str_field(a, "chatgpt_account_id")).map(str::to_string);
    CodexAuth {
        access_token: str_field(&tokens, "access_token").map(str::to_string),
        refresh_token: str_field(&tokens, "refresh_token").map(str::to_string),
        account_id: str_field(&tokens, "account_id").map(str::to_string).or(claim_account),
        api_key: str_field(v, "OPENAI_API_KEY").map(str::to_string),
        claims,
    }
}

fn read_auth(home: &Path) -> Option<CodexAuth> {
    util::read_json(&home.join("auth.json")).map(|v| parse_auth(&v))
}

/// "plus" → "Plus", "prolite" → "Pro Lite", "self_serve_business_prolite" → "Self Serve Business Prolite".
fn plan_label(raw: &str) -> Option<String> {
    let raw = raw.trim().to_ascii_lowercase();
    let label = match raw.as_str() {
        "" | "unknown" => return None,
        "prolite" => "Pro Lite".to_string(),
        "promax" => "Pro Max".to_string(),
        "edu_plus" => "Edu Plus".to_string(),
        other => other
            .split(['_', '-'])
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut c = w.chars();
                c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" "),
    };
    Some(label)
}

fn identity_from_auth(auth: Option<&CodexAuth>) -> Identity {
    let Some(a) = auth else { return Identity::default() };
    let has_tokens = a.access_token.is_some() || a.refresh_token.is_some();
    let email = a.claims.as_ref().and_then(|c| str_field(c, "email")).map(str::to_string);
    let plan = a
        .claims
        .as_ref()
        .and_then(|c| c.get(AUTH_CLAIMS))
        .and_then(|x| str_field(x, "chatgpt_plan_type"))
        .and_then(plan_label)
        .or_else(|| (!has_tokens && a.api_key.is_some()).then(|| "API key".to_string()));
    Identity { logged_in: has_tokens || a.api_key.is_some(), email, plan }
}

// ---------------------------------------------------------------------------------------------
// config.toml
// ---------------------------------------------------------------------------------------------

fn read_config(home: &Path) -> Option<toml::Table> {
    std::fs::read_to_string(home.join("config.toml")).ok()?.parse::<toml::Table>().ok()
}

/// Usage endpoint for a `chatgpt_base_url` (same normalization as CodexBar).
fn usage_url(config: Option<&toml::Table>) -> String {
    let configured = config.and_then(|c| c.get("chatgpt_base_url")).and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty());
    let mut base = configured.unwrap_or(DEFAULT_BASE_URL).trim_end_matches('/').to_string();
    if (base.starts_with("https://chatgpt.com") || base.starts_with("https://chat.openai.com")) && !base.contains("/backend-api") {
        base.push_str("/backend-api");
    }
    let path = if base.contains("/backend-api") { "/wham/usage" } else { "/api/codex/usage" };
    format!("{base}{path}")
}

/// New config.toml text forcing file credential storage, or None if already set.
/// Edits text (never re-serializes) so comments and formatting survive.
fn ensure_file_store(text: &str) -> anyhow::Result<Option<String>> {
    const KEY: &str = "cli_auth_credentials_store";
    let table: toml::Table = text.parse().map_err(|e| anyhow::anyhow!("config.toml is not valid TOML: {e}"))?;
    let updated = match table.get(KEY) {
        Some(v) if v.as_str() == Some("file") => return Ok(None),
        Some(_) => {
            let re = regex::Regex::new(&format!(r#"(?m)^[ \t]*{KEY}[ \t]*=.*$"#)).expect("valid regex");
            re.replace(text, format!(r#"{KEY} = "file""#)).to_string()
        }
        None => {
            // Root keys must precede every [table], so prepend (after a BOM if any).
            let (bom, rest) = text.strip_prefix('\u{feff}').map_or(("", text), |r| ("\u{feff}", r));
            format!("{bom}{KEY} = \"file\"\n{rest}")
        }
    };
    let check: toml::Table = updated.parse()?;
    anyhow::ensure!(check.get(KEY).and_then(|v| v.as_str()) == Some("file"), "could not set {KEY}");
    Ok(Some(updated))
}

// ---------------------------------------------------------------------------------------------
// Windows / credits mapping (shared by API, app-server and local logs)
// ---------------------------------------------------------------------------------------------

fn make_window(id: &str, used: f64, minutes: Option<u32>, reset: Option<DateTime<Utc>>, now: DateTime<Utc>) -> UsageWindow {
    // A reset in the past means the window rolled over since this reading.
    let rolled = reset.is_some_and(|r| r <= now);
    UsageWindow {
        id: id.to_string(),
        label: minutes.map(window_label).unwrap_or_else(|| if id == "primary" { "Session".into() } else { "Weekly".into() }),
        used_percent: if rolled { 0.0 } else { used.max(0.0) },
        resets_at: if rolled { None } else { reset.map(|r| r.to_rfc3339()) },
        window_minutes: minutes,
    }
}

fn from_unix(secs: f64) -> Option<DateTime<Utc>> {
    DateTime::from_timestamp(secs as i64, 0)
}

/// Prepaid credits are a balance, not a used/limit pair: reported as `used` = balance, unit "credits left".
fn credits_from(c: Option<&Value>) -> Option<Credits> {
    let c = c?;
    let has = c.get("has_credits").or_else(|| c.get("hasCredits")).and_then(Value::as_bool).unwrap_or(false);
    let unlimited = c.get("unlimited").and_then(Value::as_bool).unwrap_or(false);
    if !has || unlimited {
        return None;
    }
    Some(Credits { used: num(c.get("balance")).unwrap_or(0.0).max(0.0), limit: None, unit: "credits left".into() })
}

struct Parsed {
    windows: Vec<UsageWindow>,
    plan: Option<String>,
    credits: Option<Credits>,
}

/// `GET /wham/usage` → `{plan_type, rate_limit: {primary_window: {used_percent, limit_window_seconds,
/// reset_after_seconds, reset_at}, secondary_window}, credits: {has_credits, unlimited, balance}}`.
fn parse_wham(v: &Value, now: DateTime<Utc>) -> Parsed {
    let rl = v.get("rate_limit").cloned().unwrap_or(Value::Null);
    let windows = [("primary", "primary_window"), ("secondary", "secondary_window")]
        .iter()
        .filter_map(|(id, key)| {
            let w = rl.get(*key).filter(|w| w.is_object())?;
            let used = num(w.get("used_percent"))?;
            let minutes = num(w.get("limit_window_seconds")).map(|s| (s / 60.0).round() as u32).filter(|m| *m > 0);
            let reset =
                num(w.get("reset_at")).and_then(from_unix).or_else(|| num(w.get("reset_after_seconds")).map(|s| now + chrono::Duration::seconds(s as i64)));
            Some(make_window(id, used, minutes, reset, now))
        })
        .collect();
    Parsed { windows, plan: v.get("plan_type").and_then(Value::as_str).and_then(plan_label), credits: credits_from(v.get("credits")) }
}

/// app-server `account/rateLimits/read` result → `{rateLimits: {primary: {usedPercent, windowDurationMins, resetsAt},
/// secondary, credits: {hasCredits, unlimited, balance}, planType}}`.
fn parse_app_server_limits(result: &Value, now: DateTime<Utc>) -> Parsed {
    let rl = result
        .get("rateLimitsByLimitId")
        .and_then(|m| m.get("codex"))
        .filter(|v| v.is_object())
        .or_else(|| result.get("rateLimits"))
        .cloned()
        .unwrap_or(Value::Null);
    let windows = ["primary", "secondary"]
        .iter()
        .filter_map(|id| {
            let w = rl.get(*id).filter(|w| w.is_object())?;
            let used = num(w.get("usedPercent"))?;
            let minutes = num(w.get("windowDurationMins")).map(|m| m as u32).filter(|m| *m > 0);
            Some(make_window(id, used, minutes, num(w.get("resetsAt")).and_then(from_unix), now))
        })
        .collect();
    Parsed { windows, plan: rl.get("planType").and_then(Value::as_str).and_then(plan_label), credits: credits_from(rl.get("credits")) }
}

/// Session-log `rate_limits` → windows. `resets_at` (unix s) in newer logs, `resets_in_seconds`
/// (relative to the event) in older ones.
fn parse_log_limits(rl: &Value, event_at: DateTime<Utc>, now: DateTime<Utc>) -> Parsed {
    let windows = ["primary", "secondary"]
        .iter()
        .filter_map(|id| {
            let w = rl.get(*id).filter(|w| w.is_object())?;
            let used = num(w.get("used_percent"))?;
            let minutes = num(w.get("window_minutes")).map(|m| m as u32).filter(|m| *m > 0);
            let reset =
                num(w.get("resets_at")).and_then(from_unix).or_else(|| num(w.get("resets_in_seconds")).map(|s| event_at + chrono::Duration::seconds(s as i64)));
            Some(make_window(id, used, minutes, reset, now))
        })
        .collect();
    Parsed { windows, plan: rl.get("plan_type").and_then(Value::as_str).and_then(plan_label), credits: credits_from(rl.get("credits")) }
}

// ---------------------------------------------------------------------------------------------
// Live sources
// ---------------------------------------------------------------------------------------------

#[derive(Debug)]
enum ApiError {
    Unauthorized,
    RateLimited,
    Http(u16),
    Network(String),
    Parse,
}

async fn fetch_wham(url: &str, access_token: &str, account_id: Option<&str>) -> Result<Value, ApiError> {
    let mut req = util::http().get(url).bearer_auth(access_token).header("Accept", "application/json");
    if let Some(id) = account_id {
        req = req.header("ChatGPT-Account-Id", id);
    }
    let resp = req.send().await.map_err(|e| ApiError::Network(if e.is_timeout() { "timed out".into() } else { "network error".into() }))?;
    match resp.status().as_u16() {
        200 => resp.json::<Value>().await.map_err(|_| ApiError::Parse),
        401 | 403 => Err(ApiError::Unauthorized),
        429 => Err(ApiError::RateLimited),
        s => Err(ApiError::Http(s)),
    }
}

/// Ask `codex app-server` (stdio JSON-RPC) for the account and rate limits. Codex refreshes its own
/// tokens here when needed. Returns (account/read result, account/rateLimits/read result).
async fn app_server_query(program: &Path, codex_home: Option<&Path>) -> Result<(Option<Value>, Value), String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg("app-server")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .env_remove("OPENAI_API_KEY")
        .kill_on_drop(true);
    match codex_home {
        Some(h) => cmd.env("CODEX_HOME", h),
        None => cmd.env_remove("CODEX_HOME"),
    };
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().map_err(|e| format!("could not start codex app-server: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("no stdin")?;
    let stdout = child.stdout.take().ok_or("no stdout")?;

    let session = async {
        let version = env!("CARGO_PKG_VERSION");
        let requests = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"clientInfo": {"name": "switchyard", "version": version}}}),
            json!({"jsonrpc": "2.0", "method": "initialized"}),
            json!({"jsonrpc": "2.0", "id": 2, "method": "account/read", "params": {}}),
            json!({"jsonrpc": "2.0", "id": 3, "method": "account/rateLimits/read", "params": {"excludeResetCreditDetails": true}}),
        ];
        for r in requests {
            stdin.write_all(format!("{r}\n").as_bytes()).await.map_err(|e| e.to_string())?;
        }
        stdin.flush().await.map_err(|e| e.to_string())?;
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        let mut account = None;
        while let Ok(Some(line)) = lines.next_line().await {
            let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
            match msg.get("id").and_then(Value::as_i64) {
                Some(2) => account = msg.get("result").cloned(),
                Some(3) => {
                    if let Some(result) = msg.get("result") {
                        return Ok((account, result.clone()));
                    }
                    let err = msg.get("error").and_then(|e| str_field(e, "message")).unwrap_or("unknown error");
                    return Err(err.to_string());
                }
                _ => {}
            }
        }
        Err("codex app-server exited early".to_string())
    };
    let result = tokio::time::timeout(APP_SERVER_TIMEOUT, session).await.unwrap_or_else(|_| Err("codex app-server timed out".into()));
    let _ = child.start_kill();
    let _ = child.wait().await;
    result
}

// ---------------------------------------------------------------------------------------------
// Local session logs
// ---------------------------------------------------------------------------------------------

fn session_roots(home: &Path) -> Vec<PathBuf> {
    vec![home.join("sessions"), home.join("archived_sessions")]
}

/// Newest `rate_limits` seen in the session logs (only files touched in the last 8 days).
/// Prefers the `codex` limit bucket when logs carry several. Session folders shared with another
/// home are skipped: their limits (and plan) belong to whichever account wrote them.
fn latest_logged_limits(home: &Path) -> Option<(Value, DateTime<Utc>)> {
    let own: Vec<PathBuf> = session_roots(home).into_iter().filter(|r| !crate::sharing::is_link(r)).collect();
    for (path, _) in jsonl_files_since(&own, days_ago(8)) {
        let mut best: Option<(Value, DateTime<Utc>)> = None;
        let mut any: Option<(Value, DateTime<Utc>)> = None;
        for_each_json_line(
            &path,
            |l| l.contains("\"token_count\"") && l.contains("\"rate_limits\""),
            |v| {
                let Some(rl) = v.get("payload").and_then(|p| p.get("rate_limits")).filter(|r| r.get("primary").is_some_and(Value::is_object)) else {
                    return;
                };
                let Some(ts) = v.get("timestamp").and_then(Value::as_str).and_then(|t| DateTime::parse_from_rfc3339(t).ok()) else { return };
                let entry = (rl.clone(), ts.with_timezone(&Utc));
                if matches!(rl.get("limit_id").and_then(Value::as_str), None | Some("codex")) {
                    best = Some(entry.clone());
                }
                any = Some(entry);
            },
        );
        if let Some(found) = best.or(any) {
            return Some(found);
        }
    }
    None
}

#[derive(Default)]
struct FileState {
    prev_total: Option<[u64; 3]>, // input, cached, output (cumulative)
    model: Option<String>,
}

#[derive(Default)]
struct DayAgg {
    activity: DailyActivity,
    sessions: HashSet<String>,
}

fn usage_triplet(u: &Value) -> Option<[u64; 3]> {
    let n = |k: &str| num(u.get(k)).map(|x| x.max(0.0) as u64);
    Some([n("input_tokens")?, n("cached_input_tokens").unwrap_or(0), n("output_tokens").unwrap_or(0)])
}

/// Fold one rollout line into per-day totals. Uses deltas of the cumulative `total_token_usage`
/// (repeated token_count events add nothing); falls back to `last_token_usage`.
fn accumulate_event(v: &Value, session: &str, first_date: &str, st: &mut FileState, days: &mut BTreeMap<String, DayAgg>) {
    let Some(payload) = v.get("payload") else { return };
    match v.get("type").and_then(Value::as_str) {
        Some("turn_context") => {
            if let Some(m) = str_field(payload, "model") {
                st.model = Some(m.to_string());
            }
            return;
        }
        Some("event_msg") if payload.get("type").and_then(Value::as_str) == Some("token_count") => {}
        _ => return,
    }
    let Some(info) = payload.get("info").filter(|i| i.is_object()) else { return };
    let total = info.get("total_token_usage").and_then(usage_triplet);
    let last = info.get("last_token_usage").and_then(usage_triplet);
    let delta = match (total, st.prev_total) {
        (Some(t), Some(p)) if t.iter().zip(p.iter()).all(|(a, b)| a >= b) => [t[0] - p[0], t[1] - p[1], t[2] - p[2]],
        (Some(t), None) => t,
        _ => last.unwrap_or([0; 3]),
    };
    if total.is_some() {
        st.prev_total = total;
    }
    if delta == [0; 3] {
        return;
    }
    let Some(date) = v.get("timestamp").and_then(Value::as_str).and_then(local_date) else { return };
    if date.as_str() < first_date {
        return;
    }
    let input = delta[0].saturating_sub(delta[1]);
    let day = days.entry(date.clone()).or_default();
    let a = &mut day.activity;
    a.date = date;
    a.input_tokens += input;
    a.cache_tokens += delta[1];
    a.output_tokens += delta[2];
    a.messages += 1;
    if let Some(m) = &st.model {
        *a.models.entry(m.clone()).or_default() += input + delta[2];
    }
    day.sessions.insert(session.to_string());
}

fn scan_sessions(account_id: &str, home: &Path, since_days: u32) -> Vec<DailyActivity> {
    let first = first_local_date(since_days);
    let mut days = BTreeMap::new();
    for (path, _) in jsonl_files_since(&session_roots(home), days_ago(since_days.saturating_add(1))) {
        let session = path.to_string_lossy().to_string();
        let mut st = FileState::default();
        for_each_json_line(
            &path,
            |l| l.contains("\"token_count\"") || l.contains("\"turn_context\""),
            |v| accumulate_event(&v, &session, &first, &mut st, &mut days),
        );
    }
    days.into_values().map(|d| DailyActivity { account_id: account_id.to_string(), sessions: d.sessions.len() as u32, ..d.activity }).collect()
}

// ---------------------------------------------------------------------------------------------
// Desktop app
// ---------------------------------------------------------------------------------------------

/// Codex desktop app: (program, args, display path). Windows ships it as the MSIX package
/// `OpenAI.Codex_<id>` (app id `App`, no execution alias), launched through the shell.
fn find_desktop() -> Option<(String, Vec<String>, String)> {
    #[cfg(windows)]
    {
        if let Some(family) = msix_family("OpenAI.Codex_") {
            let target = format!("shell:AppsFolder\\{family}!App");
            return Some(("explorer.exe".into(), vec![target.clone()], target));
        }
    }
    if cfg!(target_os = "macos") {
        let apps = [PathBuf::from("/Applications/Codex.app"), util::home().join("Applications").join("Codex.app")];
        if let Some(app) = apps.into_iter().find(|p| p.exists()) {
            let app = app.to_string_lossy().to_string();
            return Some(("open".into(), vec!["-a".into(), app.clone()], app));
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------------------------

impl CodexAdapter {
    fn base_snapshot(&self, account: &Account, source: UsageSource, ident: &Identity) -> UsageSnapshot {
        let mut s = UsageSnapshot::failed(account, source, "");
        s.error = None;
        s.email = ident.email.clone().or_else(|| account.email.clone());
        s.plan = ident.plan.clone().or_else(|| account.plan.clone());
        s
    }

    fn parsed_snapshot(&self, account: &Account, source: UsageSource, ident: &Identity, p: Parsed) -> UsageSnapshot {
        let mut s = self.base_snapshot(account, source, ident);
        if p.plan.is_some() {
            s.plan = p.plan;
        }
        s.windows = p.windows;
        s.credits = p.credits;
        s
    }

    /// Last-known limits from session logs, carrying `error` (why live data is missing) if any.
    async fn local_snapshot(&self, account: &Account, ident: &Identity, error: Option<FetchError>) -> UsageSnapshot {
        // Walks every recent session file: keep it off the async workers.
        let home = PathBuf::from(&account.home_dir);
        let logged = tokio::task::spawn_blocking(move || latest_logged_limits(&home)).await.ok().flatten();
        let now = Utc::now();
        let Some((rl, at)) = logged else {
            let mut s = self.base_snapshot(account, UsageSource::None, ident);
            error.unwrap_or_else(|| FetchError::other("No recent Codex sessions to read limits from yet.")).apply(&mut s);
            return s;
        };
        let parsed = parse_log_limits(&rl, at, now);
        let shortest = parsed.windows.iter().filter_map(|w| w.window_minutes).min().unwrap_or(300);
        let old = (now - at).num_minutes() > i64::from(shortest);
        let mut s = self.parsed_snapshot(account, UsageSource::LocalLogs, ident, parsed);
        s.fetched_at = at.to_rfc3339();
        s.stale = Some(old || error.is_some());
        if let Some(e) = error {
            e.apply(&mut s);
        }
        s
    }

    fn cli_env(&self, account: &Account) -> HashMap<String, Option<String>> {
        let mut env = HashMap::new();
        let default = account.is_default_home || self.is_default(Path::new(&account.home_dir));
        env.insert("CODEX_HOME".into(), (!default).then(|| account.home_dir.clone()));
        env_unset(&mut env, &["OPENAI_API_KEY"]);
        env
    }

    fn program(&self) -> String {
        which_str("codex").unwrap_or_else(|| "codex".into())
    }
}

#[async_trait]
impl ProviderAdapter for CodexAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Codex
    }

    async fn info(&self) -> ProviderInfo {
        let (cli_path, cli_version) = self.version.get("codex").await;
        let desktop = find_desktop();
        ProviderInfo {
            id: ProviderId::Codex,
            name: "Codex".into(),
            vendor: "OpenAI".into(),
            cli_command: "codex".into(),
            cli_installed: cli_path.is_some(),
            cli_path,
            cli_version,
            desktop_name: Some("Codex app".into()),
            desktop_installed: desktop.is_some(),
            desktop_path: desktop.map(|d| d.2),
            desktop_multi_account: false,
            home_env_var: Some("CODEX_HOME".into()),
            install_hint: "npm i -g @openai/codex".into(),
        }
    }

    fn default_home(&self) -> PathBuf {
        util::home().join(".codex")
    }

    async fn detect_accounts(&self) -> Vec<DetectedAccount> {
        let mut candidates: Vec<(PathBuf, String)> = vec![(self.default_home(), "Default home (~/.codex)".into())];
        for (path, script) in scan_launcher_scripts("CODEX_HOME") {
            candidates.push((path, format!("CODEX_HOME in launcher script {script}")));
        }
        for dir in sibling_dirs(&util::home(), ".codex-") {
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            candidates.push((dir, format!("Side-by-side home ~/{name}")));
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (dir, hint) in candidates {
            if !seen.insert(util::path_key(&dir)) || !dir.is_dir() {
                continue;
            }
            if !["auth.json", "config.toml", "sessions"].iter().any(|f| dir.join(f).exists()) {
                continue;
            }
            let ident = self.read_identity(&dir).await;
            out.push(DetectedAccount {
                provider_id: ProviderId::Codex,
                home_dir: dir.to_string_lossy().to_string(),
                is_default_home: self.is_default(&dir),
                email: ident.email,
                plan: ident.plan,
                logged_in: ident.logged_in,
                already_managed: false,
                hint: Some(hint),
            });
        }
        out
    }

    async fn read_identity(&self, home: &Path) -> Identity {
        identity_from_auth(read_auth(home).as_ref())
    }

    fn shareable_paths(&self) -> Vec<(ShareKind, Vec<&'static str>)> {
        vec![
            (ShareKind::Skills, vec!["skills"]),
            (ShareKind::Plugins, vec!["plugins"]),
            (ShareKind::Agents, vec!["prompts"]),
            (ShareKind::Instructions, vec!["AGENTS.md"]),
            // config.toml also holds [mcp_servers]; sharing settings shares MCP too.
            (ShareKind::Settings, vec!["config.toml"]),
            (ShareKind::Mcp, vec![]),
            (ShareKind::History, vec!["sessions", "archived_sessions", "history.jsonl"]),
        ]
    }

    fn credential_paths(&self) -> Vec<&'static str> {
        vec!["auth.json"]
    }

    async fn init_home(&self, home: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(home)?;
        let path = home.join("config.toml");
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        // A shared (linked) config.toml is the main home's file: leave it alone.
        let linked = std::fs::symlink_metadata(&path).map(|m| m.file_type().is_symlink()).unwrap_or(false);
        if !linked {
            if let Some(updated) = ensure_file_store(&text)? {
                util::write_atomic(&path, updated.as_bytes())?;
            }
        }
        Ok(())
    }

    async fn cli_command(&self, account: &Account, resume: bool, extra_args: &[String]) -> anyhow::Result<CommandSpec> {
        let mut args = Vec::new();
        if resume {
            args.extend(["resume".to_string(), "--last".to_string()]);
        }
        args.extend(extra_args.iter().cloned());
        Ok(CommandSpec { program: self.program(), args, env: self.cli_env(account), cwd: None, note: None })
    }

    async fn login_command(&self, account: &Account) -> anyhow::Result<CommandSpec> {
        Ok(CommandSpec { program: self.program(), args: vec!["login".into()], env: self.cli_env(account), cwd: None, note: None })
    }

    /// The Codex app reads CODEX_HOME, but it is single-instance and (on Windows) an MSIX package
    /// that does not inherit our env, so it always opens the app's own account.
    async fn desktop_command(&self, _account: &Account) -> anyhow::Result<Option<CommandSpec>> {
        Ok(find_desktop().map(|(program, args, _)| CommandSpec { program, args, ..Default::default() }))
    }

    async fn fetch_usage(&self, account: &Account, live: bool) -> UsageSnapshot {
        let home = PathBuf::from(&account.home_dir);
        let auth = read_auth(&home);
        let ident = identity_from_auth(auth.as_ref());
        if !live {
            return self.local_snapshot(account, &ident, None).await;
        }
        let Some(auth) = auth else {
            return self.local_snapshot(account, &ident, Some(FetchError::auth("Not signed in. Use Sign in to log this account in."))).await;
        };
        let Some(token) = auth.access_token.as_deref() else {
            let error = if auth.api_key.is_some() {
                FetchError::other("Signed in with an API key: ChatGPT plan limits do not apply.")
            } else {
                FetchError::auth("Not signed in. Use Sign in to log this account in.")
            };
            return self.local_snapshot(account, &ident, Some(error)).await;
        };

        let url = usage_url(read_config(&home).as_ref());
        let api_error = match fetch_wham(&url, token, auth.account_id.as_deref()).await {
            Ok(v) => return self.parsed_snapshot(account, UsageSource::Api, &ident, parse_wham(&v, Utc::now())),
            Err(e) => e,
        };

        // The CLI's own app-server can refresh an expired login and read limits itself.
        if !matches!(api_error, ApiError::Network(_) | ApiError::RateLimited) {
            if let Some(program) = util::which("codex") {
                let codex_home = (!(account.is_default_home || self.is_default(&home))).then_some(home.as_path());
                if let Ok((acct, limits)) = app_server_query(&program, codex_home).await {
                    let ident = identity_from_auth(read_auth(&home).as_ref());
                    let mut s = self.parsed_snapshot(account, UsageSource::Cli, &ident, parse_app_server_limits(&limits, Utc::now()));
                    if let Some(email) = acct.as_ref().and_then(|a| a.get("account")).and_then(|a| str_field(a, "email")) {
                        s.email = Some(email.to_string());
                    }
                    if !s.windows.is_empty() {
                        return s;
                    }
                }
            }
        }

        let error = match api_error {
            ApiError::Unauthorized => FetchError::expired("Login expired. Launch Codex once or sign in again."),
            ApiError::RateLimited => FetchError::other("ChatGPT's usage API is rate-limiting requests; showing the last known limits."),
            ApiError::Http(code) => FetchError::other(format!("ChatGPT's usage API returned HTTP {code}; showing the last known limits.")),
            ApiError::Network(e) => FetchError::network(format!("Could not reach ChatGPT's usage API ({e}); showing the last known limits.")),
            ApiError::Parse => FetchError::other("ChatGPT's usage API returned an unexpected response; showing the last known limits."),
        };
        self.local_snapshot(account, &ident, Some(error)).await
    }

    async fn scan_activity(&self, account: &Account, since_days: u32) -> Vec<DailyActivity> {
        let id = account.id.clone();
        let home = PathBuf::from(&account.home_dir);
        tokio::task::spawn_blocking(move || scan_sessions(&id, &home, since_days)).await.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b64(v: &Value) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v.to_string())
    }

    fn fake_jwt(claims: Value) -> String {
        format!("{}.{}.sig", b64(&json!({"alg": "RS256", "typ": "JWT"})), b64(&claims))
    }

    fn auth_json() -> Value {
        let id_token = fake_jwt(json!({
            "email": "dev@example.com", "email_verified": true, "iss": "https://auth.openai.com",
            "https://api.openai.com/auth": {"chatgpt_account_id": "acc-from-jwt", "chatgpt_plan_type": "plus",
                                             "chatgpt_user_id": "user-x", "organizations": []}
        }));
        json!({"auth_mode": "chatgpt", "OPENAI_API_KEY": null,
               "tokens": {"id_token": id_token, "access_token": "at", "refresh_token": "rt", "account_id": "acc-1"},
               "last_refresh": "2026-10-20T10:00:00.000Z"})
    }

    #[test]
    fn parses_auth_and_identity() {
        let a = parse_auth(&auth_json());
        assert_eq!(a.account_id.as_deref(), Some("acc-1"));
        let id = identity_from_auth(Some(&a));
        assert!(id.logged_in);
        assert_eq!(id.email.as_deref(), Some("dev@example.com"));
        assert_eq!(id.plan.as_deref(), Some("Plus"));

        let mut v = auth_json();
        v["tokens"]["account_id"] = Value::Null;
        assert_eq!(parse_auth(&v).account_id.as_deref(), Some("acc-from-jwt"));

        let key_only = parse_auth(&json!({"OPENAI_API_KEY": "sk-x", "tokens": null}));
        let id = identity_from_auth(Some(&key_only));
        assert!(id.logged_in);
        assert_eq!(id.plan.as_deref(), Some("API key"));
        assert!(!identity_from_auth(None).logged_in);
        assert!(jwt_claims("not-a-jwt").is_none());
    }

    #[test]
    fn plan_labels() {
        assert_eq!(plan_label("pro").as_deref(), Some("Pro"));
        assert_eq!(plan_label("prolite").as_deref(), Some("Pro Lite"));
        assert_eq!(plan_label("self_serve_business_prolite").as_deref(), Some("Self Serve Business Prolite"));
        assert_eq!(plan_label("unknown"), None);
    }

    #[test]
    fn usage_urls() {
        assert_eq!(usage_url(None), "https://chatgpt.com/backend-api/wham/usage");
        let t: toml::Table = "chatgpt_base_url = \"https://chatgpt.com/\"".parse().unwrap();
        assert_eq!(usage_url(Some(&t)), "https://chatgpt.com/backend-api/wham/usage");
        let t: toml::Table = "chatgpt_base_url = \"http://localhost:8080\"".parse().unwrap();
        assert_eq!(usage_url(Some(&t)), "http://localhost:8080/api/codex/usage");
    }

    #[test]
    fn forces_file_credential_store() {
        let original = "# my config\nmodel = \"gpt-5\"\n\n[mcp_servers.x]\ncommand = \"node\"\n";
        let out = ensure_file_store(original).unwrap().unwrap();
        assert!(out.starts_with("cli_auth_credentials_store = \"file\"\n# my config"));
        assert!(out.contains("[mcp_servers.x]"));
        assert!(ensure_file_store(&out).unwrap().is_none());

        let keyring = "model = \"x\"\ncli_auth_credentials_store = \"keyring\" # comment\n[tui]\na = 1\n";
        let out = ensure_file_store(keyring).unwrap().unwrap();
        let t: toml::Table = out.parse().unwrap();
        assert_eq!(t["cli_auth_credentials_store"].as_str(), Some("file"));
        assert_eq!(t["tui"]["a"].as_integer(), Some(1));

        assert!(ensure_file_store("").unwrap().is_some());
        assert!(ensure_file_store("this is = = not toml").is_err());
    }

    #[test]
    fn parses_wham_usage() {
        let now = DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z").unwrap().with_timezone(&Utc);
        let v: Value = serde_json::from_str(
            r#"{"user_id":"u","account_id":"a","email":"dev@example.com","plan_type":"pro",
                "rate_limit":{"allowed":true,"limit_reached":false,
                  "primary_window":{"used_percent":23,"limit_window_seconds":18000,"reset_after_seconds":3600,"reset_at":1792512000},
                  "secondary_window":{"used_percent":4,"limit_window_seconds":604800,"reset_after_seconds":400000}},
                "credits":{"has_credits":true,"unlimited":false,"balance":"12.5"},
                "additional_rate_limits":[]}"#,
        )
        .unwrap();
        let p = parse_wham(&v, now);
        assert_eq!(p.plan.as_deref(), Some("Pro"));
        assert_eq!(p.windows.len(), 2);
        assert_eq!(p.windows[0].id, "primary");
        assert_eq!(p.windows[0].label, "5-hour session");
        assert_eq!(p.windows[0].window_minutes, Some(300));
        assert_eq!(p.windows[0].used_percent, 23.0);
        assert_eq!(p.windows[0].resets_at, from_unix(1792512000.0).map(|d| d.to_rfc3339()));
        assert_eq!(p.windows[1].label, "Weekly");
        assert_eq!(p.windows[1].resets_at, Some((now + chrono::Duration::seconds(400000)).to_rfc3339()));
        let c = p.credits.unwrap();
        assert_eq!(c.used, 12.5);

        let empty = parse_wham(&json!({"rate_limit": null, "credits": {"has_credits": false}}), now);
        assert!(empty.windows.is_empty() && empty.credits.is_none() && empty.plan.is_none());
    }

    #[test]
    fn parses_app_server_limits() {
        let now = DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z").unwrap().with_timezone(&Utc);
        let v = json!({"rateLimits": {"limitId": "codex", "primary": {"usedPercent": 50, "windowDurationMins": 300, "resetsAt": 1792514865},
                                      "secondary": {"usedPercent": 7, "windowDurationMins": 10080, "resetsAt": 1793101665},
                                      "credits": {"hasCredits": false, "unlimited": false, "balance": "0"}, "planType": "plus"},
                       "rateLimitsByLimitId": null});
        let p = parse_app_server_limits(&v, now);
        assert_eq!(p.windows.len(), 2);
        assert_eq!(p.windows[0].used_percent, 50.0);
        assert_eq!(p.windows[1].label, "Weekly");
        assert_eq!(p.plan.as_deref(), Some("Plus"));
        assert!(p.credits.is_none());
    }

    #[test]
    fn log_limits_roll_over_after_reset() {
        let event = DateTime::parse_from_rfc3339("2026-10-20T10:00:00Z").unwrap().with_timezone(&Utc);
        let now = event + chrono::Duration::hours(6);
        let rl = json!({"limit_id": "codex", "primary": {"used_percent": 80.0, "window_minutes": 300, "resets_at": (event + chrono::Duration::hours(1)).timestamp()},
                        "secondary": {"used_percent": 10.0, "window_minutes": 10080, "resets_in_seconds": 86400},
                        "credits": null, "plan_type": "plus"});
        let p = parse_log_limits(&rl, event, now);
        assert_eq!(p.windows[0].used_percent, 0.0);
        assert_eq!(p.windows[0].resets_at, None);
        assert_eq!(p.windows[1].used_percent, 10.0);
        assert_eq!(p.windows[1].resets_at, Some((event + chrono::Duration::seconds(86400)).to_rfc3339()));
    }

    fn token_count(ts: &str, total: [u64; 3], last: [u64; 3]) -> Value {
        let u = |x: [u64; 3]| {
            json!({"input_tokens": x[0], "cached_input_tokens": x[1], "cache_write_input_tokens": 0,
                                       "output_tokens": x[2], "reasoning_output_tokens": 0, "total_tokens": x[0] + x[2]})
        };
        json!({"timestamp": ts, "type": "event_msg", "payload": {"type": "token_count",
               "info": {"total_token_usage": u(total), "last_token_usage": u(last), "model_context_window": 475000},
               "rate_limits": {"limit_id": "codex", "primary": {"used_percent": 23.0, "window_minutes": 300, "resets_at": 1792514865},
                               "secondary": {"used_percent": 4.0, "window_minutes": 10080, "resets_at": 1793101665},
                               "credits": {"has_credits": false, "unlimited": false, "balance": "0"}, "plan_type": "plus"}}})
    }

    #[test]
    fn activity_uses_cumulative_deltas() {
        let ts = Utc::now().to_rfc3339();
        let first = first_local_date(7);
        let mut st = FileState::default();
        let mut days = BTreeMap::new();
        let ctx = json!({"timestamp": ts, "type": "turn_context", "payload": {"model": "gpt-6.1-sol", "cwd": "/x"}});
        accumulate_event(&ctx, "f1", &first, &mut st, &mut days);
        accumulate_event(&json!({"timestamp": ts, "type": "event_msg", "payload": {"type": "token_count", "info": null}}), "f1", &first, &mut st, &mut days);
        accumulate_event(&token_count(&ts, [1000, 400, 50], [1000, 400, 50]), "f1", &first, &mut st, &mut days);
        // Duplicate emission of the same totals → no double count.
        accumulate_event(&token_count(&ts, [1000, 400, 50], [1000, 400, 50]), "f1", &first, &mut st, &mut days);
        accumulate_event(&token_count(&ts, [2500, 1400, 80], [1500, 1000, 30]), "f1", &first, &mut st, &mut days);
        let d = &days.values().next().unwrap().activity;
        assert_eq!(d.input_tokens, 600 + 500);
        assert_eq!(d.cache_tokens, 1400);
        assert_eq!(d.output_tokens, 80);
        assert_eq!(d.messages, 2);
        assert_eq!(d.models.get("gpt-6.1-sol"), Some(&(1100 + 80)));
    }

    #[test]
    fn scans_session_tree_and_reads_last_limits() {
        let dir = tempfile::tempdir().unwrap();
        let day_dir = dir.path().join("sessions").join("2026").join("10").join("20");
        std::fs::create_dir_all(&day_dir).unwrap();
        let ts = Utc::now().to_rfc3339();
        let lines = [
            json!({"timestamp": ts, "type": "session_meta", "payload": {"id": "s", "cli_version": "0.159.2"}}).to_string(),
            json!({"timestamp": ts, "type": "turn_context", "payload": {"model": "gpt-6.1-sol"}}).to_string(),
            token_count(&ts, [100, 0, 10], [100, 0, 10]).to_string(),
            "garbage line".to_string(),
            token_count(&ts, [300, 50, 30], [200, 50, 20]).to_string(),
        ];
        std::fs::write(day_dir.join("rollout-a.jsonl"), lines.join("\n")).unwrap();
        let out = scan_sessions("acc", dir.path(), 7);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].input_tokens, 250);
        assert_eq!(out[0].output_tokens, 30);
        assert_eq!(out[0].sessions, 1);
        assert_eq!(out[0].messages, 2);

        let (rl, _) = latest_logged_limits(dir.path()).unwrap();
        assert_eq!(rl["primary"]["window_minutes"], 300);

        // A profile that shares this history must not take these limits as its own.
        let other = tempfile::tempdir().unwrap();
        let link = other.path().join("sessions");
        #[cfg(windows)]
        junction::create(dir.path().join("sessions"), &link).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("sessions"), &link).unwrap();
        assert!(latest_logged_limits(other.path()).is_none());
    }

    #[test]
    fn cli_command_env_and_resume() {
        let adapter = CodexAdapter::new();
        let acc: Account = serde_json::from_value(json!({
            "id": "a1", "providerId": "codex", "label": "Work", "email": null, "plan": null,
            "homeDir": "/tmp/codex-work", "isDefaultHome": false, "sharedFrom": null,
            "sharing": {"skills": true, "plugins": true, "agents": true, "instructions": true, "settings": false, "mcp": true, "history": true},
            "color": "#fff", "notes": null, "status": "ok", "statusMessage": null, "createdAt": "2026-10-20T00:00:00Z",
            "lastLaunchedAt": null, "monthlyCost": null, "pinned": null}))
        .unwrap();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let spec = rt.block_on(adapter.cli_command(&acc, true, &["--model".into(), "x".into()])).unwrap();
        assert_eq!(spec.args, ["resume", "--last", "--model", "x"]);
        assert_eq!(spec.env["CODEX_HOME"].as_deref(), Some("/tmp/codex-work"));
        assert_eq!(spec.env["OPENAI_API_KEY"], None);
    }

    #[test]
    fn init_home_writes_config() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("codex-new");
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(CodexAdapter::new().init_home(&home)).unwrap();
        let text = std::fs::read_to_string(home.join("config.toml")).unwrap();
        assert!(text.contains("cli_auth_credentials_store = \"file\""));
        rt.block_on(CodexAdapter::new().init_home(&home)).unwrap();
        assert_eq!(std::fs::read_to_string(home.join("config.toml")).unwrap(), text);
    }
}
