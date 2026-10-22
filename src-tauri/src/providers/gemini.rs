//! Gemini CLI (Google) adapter.
//!
//! Isolation: Gemini CLI resolves its "home" as `process.env.GEMINI_CLI_HOME || os.homedir()` and keeps
//! everything in `<home>/.gemini` (packages/core/src/utils/paths.ts `homedir()`, used by
//! `Storage.getGlobalGeminiDir()`). So GEMINI_CLI_HOME is NOT the `.gemini` dir itself: it replaces the
//! user's home. An Relay account home for Gemini is therefore a directory that *contains* `.gemini/`,
//! and every relative path below is prefixed with `.gemini/`. The stock account's home is the user's
//! real home dir (`~`, holding `~/.gemini`). No HOME/USERPROFILE override is needed, so git, ssh and
//! the rest of the user's environment keep working inside Gemini sessions.
//!
//! Verified against @google/gemini-cli 0.64.0 (npm bundle) and steipete/CodexBar docs/gemini.md.
use super::common::{
    finish_days, first_json_line, first_local_date, folder_forms, is_uuid, local_date, normalize_iso, num, read_jsonc, scan_launcher_scripts, wanted_first,
    DayMap, VersionCache, RESUME_CANDIDATES,
};
use super::{CommandSpec, FetchError, Identity, ProviderAdapter, Resume};
use crate::model::*;
use crate::util;
use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::time::Duration;

const HOME_ENV: &str = "GEMINI_CLI_HOME";
const GEMINI_DIR: &str = ".gemini";
const CODE_ASSIST: &str = "https://cloudcode-pa.googleapis.com/v1internal";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

const NEED_CLI: &str = "Install Gemini CLI or launch it once to refresh the login";
const SIGN_IN_AGAIN: &str = "Sign in to Gemini again";

/// Max model windows shown per account (Pro and Flash families come first).
const MAX_WINDOWS: usize = 4;

pub struct GeminiAdapter {
    version: VersionCache,
}

impl GeminiAdapter {
    pub fn new() -> Self {
        Self { version: VersionCache::default() }
    }
}

impl Default for GeminiAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn gemini_dir(home: &Path) -> PathBuf {
    home.join(GEMINI_DIR)
}

/// A dir is a Gemini home when its `.gemini` holds a login or settings.
fn looks_like_home(home: &Path) -> bool {
    let g = gemini_dir(home);
    ["oauth_creds.json", "google_accounts.json", "settings.json"].iter().any(|f| g.join(f).is_file())
}

/// How the home authenticates, from `.gemini/settings.json` (`security.auth.selectedType`, older
/// builds: top-level `selectedAuthType`) and `.gemini/.env`.
#[derive(Debug, PartialEq, Eq)]
enum AuthKind {
    OAuth,
    ApiKey,
    Unknown,
}

fn auth_kind(home: &Path) -> AuthKind {
    let g = gemini_dir(home);
    let selected = read_jsonc(&g.join("settings.json"))
        .and_then(|s| s.pointer("/security/auth/selectedType").or_else(|| s.get("selectedAuthType")).and_then(Value::as_str).map(str::to_string));
    match selected.as_deref() {
        Some("oauth-personal") => return AuthKind::OAuth,
        Some("gemini-api-key") | Some("vertex-ai") => return AuthKind::ApiKey,
        _ => {}
    }
    if g.join("oauth_creds.json").is_file() {
        return AuthKind::OAuth;
    }
    let env = std::fs::read_to_string(g.join(".env")).unwrap_or_default();
    if env.lines().any(|l| {
        let l = l.trim_start().trim_start_matches("export ").trim_start();
        l.starts_with("GEMINI_API_KEY=") || l.starts_with("GOOGLE_API_KEY=")
    }) {
        return AuthKind::ApiKey;
    }
    AuthKind::Unknown
}

/// Email from the `email` claim of a JWT (no signature check: display only).
fn jwt_email(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: Value = serde_json::from_slice(&bytes).ok()?;
    v.get("email").and_then(Value::as_str).map(str::to_string)
}

fn read_identity_sync(home: &Path) -> Identity {
    let g = gemini_dir(home);
    let creds = util::read_json(&g.join("oauth_creds.json"));
    let active =
        util::read_json(&g.join("google_accounts.json")).and_then(|v| v.get("active").and_then(Value::as_str).map(str::to_string)).filter(|s| !s.is_empty());
    let email = active.or_else(|| creds.as_ref()?.get("id_token")?.as_str().and_then(jwt_email));
    let logged_in = creds.as_ref().is_some_and(|c| c.get("refresh_token").or_else(|| c.get("access_token")).is_some()) || auth_kind(home) == AuthKind::ApiKey;
    Identity { logged_in, email, plan: None }
}

// ---------- usage: Code Assist API ----------

/// Result of `loadCodeAssist`: the quota project and a friendly plan name.
#[derive(Debug, Default, PartialEq)]
struct LoadInfo {
    project: Option<String>,
    plan: Option<String>,
    /// Set when Google refuses this account for Gemini CLI (`ineligibleTiers` without `currentTier`).
    ineligible: Option<String>,
}

fn parse_load_code_assist(v: &Value) -> LoadInfo {
    // `cloudaicompanionProject` is a string; tolerate the object form `{ "id": ... }` too.
    let project = match v.get("cloudaicompanionProject") {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Object(o)) => o.get("id").and_then(Value::as_str).map(str::to_string),
        _ => None,
    };
    let paid_name = v.pointer("/paidTier/name").and_then(Value::as_str);
    let tier_id = v.pointer("/paidTier/id").or_else(|| v.pointer("/currentTier/id")).and_then(Value::as_str);
    let tier_name = v.pointer("/currentTier/name").and_then(Value::as_str);
    let plan = plan_label(paid_name, tier_id, tier_name);
    let ineligible = if v.get("currentTier").is_none() {
        v.get("ineligibleTiers").and_then(Value::as_array).filter(|a| !a.is_empty()).map(|tiers| {
            let reasons: Vec<&str> = tiers.iter().filter_map(|t| t.get("reasonMessage").or_else(|| t.get("reasonCode")).and_then(Value::as_str)).collect();
            if reasons.iter().any(|r| r.contains("UNSUPPORTED_CLIENT")) || reasons.is_empty() {
                "Google no longer serves Gemini CLI quotas for this account type (personal accounts moved to Antigravity)".to_string()
            } else {
                reasons.join("; ")
            }
        })
    } else {
        None
    };
    LoadInfo { project, plan, ineligible }
}

/// Friendly plan name. Google's own paid tier name wins (e.g. "Google AI Pro").
fn plan_label(paid_name: Option<&str>, tier_id: Option<&str>, tier_name: Option<&str>) -> Option<String> {
    let nice = |n: &str| {
        let l = n.to_lowercase();
        if l.contains("ultra") {
            "Google AI Ultra".to_string()
        } else if l.contains("ai pro") {
            "Google AI Pro".to_string()
        } else {
            n.to_string()
        }
    };
    if let Some(n) = paid_name.filter(|n| !n.is_empty()) {
        return Some(nice(n));
    }
    match tier_id {
        Some("free-tier") => Some("Free".into()),
        Some("legacy-tier") => Some("Legacy".into()),
        Some("standard-tier") => Some("Standard".into()),
        Some(other) if !other.is_empty() => Some(tier_name.map(nice).unwrap_or_else(|| other.to_string())),
        _ => tier_name.map(nice),
    }
}

/// `retrieveUserQuota` buckets → one window per model (lowest remainingFraction wins), Pro then Flash first.
fn parse_quota(v: &Value) -> Vec<UsageWindow> {
    struct Best {
        remaining: f64,
        reset: Option<String>,
    }
    let mut per_model: BTreeMap<String, Best> = BTreeMap::new();
    for b in v.get("buckets").and_then(Value::as_array).into_iter().flatten() {
        let Some(model) = b.get("modelId").and_then(Value::as_str).filter(|m| !m.is_empty()) else {
            continue;
        };
        let Some(rem) = num(b.get("remainingFraction")) else {
            continue;
        };
        let reset = b.get("resetTime").and_then(Value::as_str).map(normalize_iso);
        let model = model.trim_end_matches("_vertex").to_string();
        match per_model.get_mut(&model) {
            Some(best) if best.remaining <= rem => {}
            Some(best) => {
                best.remaining = rem;
                best.reset = reset.or(best.reset.take());
            }
            None => {
                per_model.insert(model, Best { remaining: rem, reset });
            }
        }
    }
    let mut windows: Vec<UsageWindow> = per_model
        .into_iter()
        .map(|(id, b)| UsageWindow {
            label: model_label(&id),
            used_percent: ((1.0 - b.remaining.clamp(0.0, 1.0)) * 100.0 * 10.0).round() / 10.0,
            resets_at: b.reset,
            window_minutes: None,
            id,
        })
        .collect();
    windows.sort_by(|a, b| model_rank(&a.id).cmp(&model_rank(&b.id)).then_with(|| version_desc(&a.id, &b.id)));
    windows.truncate(MAX_WINDOWS);
    windows
}

/// 0 = Pro, 1 = Flash, 2 = Flash Lite, 3 = anything else.
fn model_rank(id: &str) -> u8 {
    let l = id.to_lowercase();
    if l.contains("flash-lite") || l.contains("flash_lite") {
        2
    } else if l.contains("flash") {
        1
    } else if l.contains("pro") {
        0
    } else {
        3
    }
}

/// Newer model versions first ("gemini-3-pro" before "gemini-2.5-pro"), then stable before preview.
fn version_desc(a: &str, b: &str) -> std::cmp::Ordering {
    let ver = |s: &str| -> f64 { s.split('-').find_map(|p| p.parse::<f64>().ok()).unwrap_or(0.0) };
    ver(b).partial_cmp(&ver(a)).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.contains("preview").cmp(&b.contains("preview"))).then_with(|| a.cmp(b))
}

/// "gemini-2.5-flash-lite" → "Gemini 2.5 Flash Lite".
fn model_label(id: &str) -> String {
    id.split(['-', '_'])
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut c = p.chars();
            match c.next() {
                Some(f) if f.is_ascii_alphabetic() => f.to_uppercase().collect::<String>() + c.as_str(),
                _ => p.to_string(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

enum CallError {
    Unauthorized,
    Http(u16, String),
    Network(String),
}

async fn post_code_assist(method: &str, token: &str, body: &Value) -> Result<Value, CallError> {
    let resp =
        util::http().post(format!("{CODE_ASSIST}:{method}")).bearer_auth(token).json(body).send().await.map_err(|e| CallError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    match status {
        200..=299 => serde_json::from_str(&text).map_err(|_| CallError::Http(status, "unexpected response".into())),
        401 => Err(CallError::Unauthorized),
        _ => Err(CallError::Http(status, text)),
    }
}

/// A refresh that never reached Google: a network failure, so being offline doesn't read as signed out.
fn offline_refresh(e: impl std::fmt::Display) -> FetchError {
    FetchError::network(format!("Could not reach Google to renew the Gemini session ({e})"))
}

/// Google answers a refresh token it no longer accepts with 400 (`invalid_grant`) or 401; anything
/// else (5xx, 429) is Google having a bad moment, not the user being signed out.
fn refresh_refused(status: u16) -> FetchError {
    match status {
        400 | 401 => FetchError::auth(SIGN_IN_AGAIN),
        _ => FetchError::other(format!("Google could not renew the Gemini session (HTTP {status}); will retry later")),
    }
}

/// OAuth credentials file with an in-place refresh that keeps the CLI's file valid.
struct Creds {
    path: PathBuf,
    raw: Value,
}

impl Creds {
    fn load(home: &Path) -> Option<Self> {
        let path = gemini_dir(home).join("oauth_creds.json");
        let raw = util::read_json(&path)?;
        raw.is_object().then_some(Self { path, raw })
    }

    fn access_token(&self) -> Option<&str> {
        self.raw.get("access_token").and_then(Value::as_str).filter(|s| !s.is_empty())
    }

    fn expired(&self) -> bool {
        let Some(exp) = num(self.raw.get("expiry_date")) else {
            return self.access_token().is_none();
        };
        exp as i64 <= chrono::Utc::now().timestamp_millis() + 60_000
    }

    async fn refresh(&mut self) -> Result<(), FetchError> {
        let again = || FetchError::auth(SIGN_IN_AGAIN);
        let refresh = self.raw.get("refresh_token").and_then(Value::as_str).ok_or_else(again)?;
        // Without the CLI's OAuth client the login can't renew by itself: the user has to act.
        let client = oauth_client::resolve().await.ok_or_else(|| FetchError::auth(NEED_CLI))?;
        let body = form_encode(&[("client_id", &client.id), ("client_secret", &client.secret), ("refresh_token", refresh), ("grant_type", "refresh_token")]);
        let resp = util::http()
            .post(TOKEN_URL)
            .header(reqwest::header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await
            .map_err(|e| offline_refresh(e.without_url()))?;
        let status = resp.status();
        if !status.is_success() {
            return Err(refresh_refused(status.as_u16()));
        }
        let odd = || FetchError::other("Google sent an unexpected reply while renewing the Gemini session; will retry later");
        let v: Value = resp.json().await.map_err(|_| odd())?;
        apply_refresh(&mut self.raw, &v, chrono::Utc::now().timestamp_millis()).ok_or_else(odd)?;
        let bytes = serde_json::to_vec_pretty(&self.raw).map_err(|e| FetchError::other(e.to_string()))?;
        util::write_atomic(&self.path, &bytes).map_err(|e| FetchError::other(format!("Could not save the renewed Gemini session: {e}")))?;
        restrict_permissions(&self.path);
        Ok(())
    }
}

/// Merge a token-endpoint response into the creds JSON, keeping every other field.
fn apply_refresh(raw: &mut Value, resp: &Value, now_ms: i64) -> Option<()> {
    let token = resp.get("access_token")?.as_str()?.to_string();
    let obj = raw.as_object_mut()?;
    obj.insert("access_token".into(), json!(token));
    if let Some(secs) = num(resp.get("expires_in")) {
        obj.insert("expiry_date".into(), json!(now_ms + (secs * 1000.0) as i64));
    }
    for key in ["id_token", "scope", "token_type", "refresh_token"] {
        if let Some(v) = resp.get(key).filter(|v| v.is_string()) {
            obj.insert(key.into(), v.clone());
        }
    }
    Some(())
}

fn form_encode(pairs: &[(&str, &str)]) -> String {
    fn enc(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    pairs.iter().map(|(k, v)| format!("{}={}", enc(k), enc(v))).collect::<Vec<_>>().join("&")
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}
#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) {}

fn friendly_http(status: u16, body: &str) -> String {
    if body.contains("SUBSCRIPTION_REQUIRED") || body.contains("UNSUPPORTED_CLIENT") {
        return "Google no longer serves Gemini CLI quotas for this account type (personal accounts moved to Antigravity)".into();
    }
    match status {
        403 => "Google refused the quota request for this account (HTTP 403)".into(),
        429 => "Google is rate limiting usage requests; will retry later".into(),
        _ => format!("Gemini usage request failed (HTTP {status})"),
    }
}

impl GeminiAdapter {
    async fn fetch_live(&self, account: &Account) -> Result<UsageSnapshot, FetchError> {
        let home = Path::new(&account.home_dir);
        let Some(mut creds) = Creds::load(home) else {
            return Err(if auth_kind(home) == AuthKind::ApiKey {
                FetchError::other("API-key accounts have no subscription limits to show")
            } else {
                FetchError::auth("Not signed in. Launch Gemini and choose \"Sign in with Google\"")
            });
        };
        let mut refreshed = false;
        if creds.expired() {
            creds.refresh().await?;
            refreshed = true;
        }
        let meta = json!({"metadata": {"ideType": "IDE_UNSPECIFIED", "platform": "PLATFORM_UNSPECIFIED", "pluginType": "GEMINI"}});
        let load = loop {
            let token = creds.access_token().ok_or_else(|| FetchError::auth(SIGN_IN_AGAIN))?.to_string();
            match post_code_assist("loadCodeAssist", &token, &meta).await {
                Ok(v) => break v,
                Err(CallError::Unauthorized) if !refreshed => {
                    creds.refresh().await?;
                    refreshed = true;
                }
                Err(CallError::Unauthorized) => return Err(FetchError::auth(SIGN_IN_AGAIN)),
                Err(CallError::Http(s, body)) => return Err(FetchError::other(friendly_http(s, &body))),
                Err(CallError::Network(e)) => return Err(FetchError::network(format!("Could not reach Google: {e}"))),
            }
        };
        let info = parse_load_code_assist(&load);
        if info.project.is_none() {
            if let Some(reason) = info.ineligible {
                return Err(FetchError::other(reason));
            }
        }
        let token = creds.access_token().ok_or_else(|| FetchError::auth(SIGN_IN_AGAIN))?.to_string();
        let body = match &info.project {
            Some(p) => json!({ "project": p }),
            None => json!({}),
        };
        let quota = match post_code_assist("retrieveUserQuota", &token, &body).await {
            Ok(v) => v,
            Err(CallError::Unauthorized) => return Err(FetchError::auth(SIGN_IN_AGAIN)),
            Err(CallError::Http(s, body)) => return Err(FetchError::other(friendly_http(s, &body))),
            Err(CallError::Network(e)) => return Err(FetchError::network(format!("Could not reach Google: {e}"))),
        };
        let windows = parse_quota(&quota);
        let identity = read_identity_sync(home);
        Ok(UsageSnapshot {
            account_id: account.id.clone(),
            provider_id: ProviderId::Gemini,
            fetched_at: util::now_iso(),
            source: UsageSource::Api,
            error: windows.is_empty().then(|| "Google returned no quota buckets for this account".to_string()),
            error_kind: windows.is_empty().then_some(ErrorKind::Other),
            windows,
            plan: info.plan,
            email: identity.email,
            credits: None,
            stale: None,
        })
    }
}

// ---------- activity: tmp/<project>/chats/session-*.jsonl ----------

/// Fold one chat recording into per-day totals. Handles the current JSONL format (first line metadata,
/// then message records; a message id may be re-appended when its tokens arrive, last one wins) and the
/// legacy single-JSON format `{ sessionId, startTime, messages: [...] }`.
/// `tokens.input` is promptTokenCount, which already includes `cached`.
fn fold_gemini_chat(content: &str, count_session: bool, days: &mut DayMap) {
    let mut start: Option<String> = None;
    let mut order: Vec<String> = vec![];
    let mut msgs: std::collections::HashMap<String, Value> = Default::default();
    let mut push = |m: Value, order: &mut Vec<String>| {
        if let Some(id) = m.get("id").and_then(Value::as_str).map(str::to_string) {
            if msgs.insert(id.clone(), m).is_none() {
                order.push(id);
            }
        }
    };
    let trimmed = content.trim_start();
    if let Some(whole) = trimmed.starts_with('{').then(|| serde_json::from_str::<Value>(trimmed).ok()).flatten().filter(|v| v.get("messages").is_some()) {
        start = whole.get("startTime").and_then(Value::as_str).map(str::to_string);
        for m in whole["messages"].as_array().cloned().unwrap_or_default() {
            push(m, &mut order);
        }
    } else {
        for line in content.lines() {
            let Ok(v) = serde_json::from_str::<Value>(line) else { continue };
            if v.get("type").is_some() && v.get("id").is_some() {
                push(v, &mut order);
            } else if start.is_none() {
                start = v.get("startTime").and_then(Value::as_str).map(str::to_string);
            }
        }
    }
    let mut first_day: Option<String> = start.as_deref().and_then(local_date);
    for id in &order {
        let m = &msgs[id];
        let Some(day) = m.get("timestamp").and_then(Value::as_str).and_then(local_date) else { continue };
        first_day.get_or_insert_with(|| day.clone());
        if m.get("type").and_then(Value::as_str) != Some("gemini") {
            continue;
        }
        let d = days.entry(day).or_default();
        d.messages += 1;
        if let Some(t) = m.get("tokens").filter(|t| t.is_object()) {
            let g = |k: &str| num(t.get(k)).unwrap_or(0.0).max(0.0) as u64;
            let (input, cached) = (g("input"), g("cached"));
            let output = g("output") + g("thoughts");
            d.input_tokens += input.saturating_sub(cached);
            d.cache_tokens += cached;
            d.output_tokens += output;
            if let Some(model) = m.get("model").and_then(Value::as_str) {
                *d.models.entry(model.to_string()).or_default() += input + output;
            }
        }
    }
    if count_session {
        if let Some(day) = first_day {
            days.entry(day).or_default().sessions += 1;
        }
    }
}

fn scan_gemini_activity(home: &Path, account_id: &str, since_days: u32) -> Vec<DailyActivity> {
    let cutoff = first_local_date(since_days);
    let cutoff_time =
        std::time::SystemTime::now().checked_sub(Duration::from_secs(u64::from(since_days.max(1)) * 86_400 + 86_400)).unwrap_or(std::time::UNIX_EPOCH);
    let mut days = DayMap::new();
    let tmp = gemini_dir(home).join("tmp");
    let Ok(projects) = std::fs::read_dir(&tmp) else { return vec![] };
    for project in projects.flatten() {
        let chats = project.path().join("chats");
        if !chats.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&chats).max_depth(2).into_iter().flatten() {
            let p = entry.path();
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if !entry.file_type().is_file() || !(ext == "jsonl" || ext == "json") {
                continue;
            }
            if entry.metadata().ok().and_then(|m| m.modified().ok()).is_some_and(|t| t < cutoff_time) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(p) else { continue };
            // Sub-agent transcripts live in chats/<parentSessionId>/ and are not separate sessions.
            fold_gemini_chat(&content, entry.depth() == 1, &mut days);
        }
    }
    finish_days(days, account_id, &cutoff)
}

// ---------- conversations: tmp/<project>/chats/session-*.jsonl ----------
//
// A chat's first line (or, in the legacy `.json` format, the whole file) carries `sessionId` (a uuid),
// `projectHash` (sha256 of the project root, i.e. the folder Gemini started in:
// packages/core/src/utils/paths.ts `getProjectHash`, config.ts `getProjectRoot`) and `kind`
// ("subagent" for a sub-agent's chat). `gemini --resume <uuid>` looks only in the current project's
// chats and skips sub-agent chats (packages/cli/src/utils/sessionUtils.ts). Gemini CLI leaves no
// per-process marker naming its chat, so `open_sessions` stays empty.

/// What Gemini hashes for the project of `cwd`: its absolute path as given and, where the folder
/// is reached through a symlink, the real path the process sees.
fn project_hashes(cwd: &Path) -> Vec<String> {
    folder_forms(cwd)
        .iter()
        .map(|p| {
            let s = p.to_string_lossy();
            let s = if s.len() > 1 { s.trim_end_matches(['/', '\\']) } else { &s };
            format!("{:x}", Sha256::digest(s.as_bytes()))
        })
        .collect()
}

/// Whether a legacy `.json` chat names one of `hashes` as its project, from its first few KB:
/// Gemini wrote `projectHash` near the top, and these files can be large.
fn legacy_chat_is_for(path: &Path, hashes: &[String]) -> bool {
    use std::io::Read;
    let mut head = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(4096).read_to_end(&mut head));
    let head = String::from_utf8_lossy(&head);
    read.is_ok() && hashes.iter().any(|h| head.contains(h.as_str()))
}

/// A chat file's metadata: its first line, or the whole legacy `.json` file when it belongs to one
/// of `hashes`.
fn chat_meta(path: &Path, hashes: &[String]) -> Option<Value> {
    if path.extension().is_some_and(|e| e == "jsonl") {
        return first_json_line(path);
    }
    legacy_chat_is_for(path, hashes).then(|| serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()).flatten()
}

/// A message's text as Gemini reads it for resuming (`partListUnionToString`): strings and text
/// parts as they are, other parts as a bracketed label.
fn content_text(content: &Value) -> String {
    const LABELLED: [&str; 8] =
        ["videoMetadata", "thought", "codeExecutionResult", "executableCode", "fileData", "functionCall", "functionResponse", "inlineData"];
    match content {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().map(content_text).collect(),
        Value::Object(part) if LABELLED.iter().any(|k| part.get(*k).is_some_and(|v| !v.is_null())) => "[part]".into(),
        Value::Object(part) => part.get("text").and_then(Value::as_str).unwrap_or_default().to_string(),
        _ => String::new(),
    }
}

/// Prompts Gemini doesn't count as conversation: empty, slash commands, `?` help and injected
/// context (core `isIgnoredUserContent`).
fn ignored_prompt(text: &str) -> bool {
    text.is_empty() || ["/", "?", "<session_context>", "<hook_context>"].iter().any(|p| text.starts_with(p))
}

/// Gemini's `isResumableMessageRecord`: a real prompt, or a reply with text, tool calls or thoughts.
fn resumable_message(m: &Value) -> bool {
    let text = m.get("content").map(content_text).unwrap_or_default();
    let non_empty = |k: &str| m.get(k).and_then(Value::as_array).is_some_and(|a| !a.is_empty());
    match m.get("type").and_then(Value::as_str) {
        Some("user") => !ignored_prompt(text.trim()),
        Some("gemini") => !text.trim().is_empty() || non_empty("toolCalls") || non_empty("thoughts"),
        _ => false,
    }
}

/// Whether Gemini would resume a JSONL chat: some message left after its rewinds, patches and
/// removals is resumable (the metadata-only pass of `chatRecordingService.ts` loading a chat).
fn jsonl_chat_resumable(path: &Path) -> bool {
    let Ok(file) = std::fs::File::open(path) else { return false };
    // Message ids in order, and each one's (is a prompt, is resumable).
    let mut ids: Vec<String> = Vec::new();
    let mut kinds: HashMap<String, (bool, bool)> = HashMap::new();
    fn add(m: &Value, ids: &mut Vec<String>, kinds: &mut HashMap<String, (bool, bool)>) {
        let Some(id) = m.get("id").and_then(Value::as_str).filter(|_| m.get("$patch").is_none()) else { return };
        if !kinds.contains_key(id) {
            ids.push(id.to_string());
        }
        kinds.insert(id.to_string(), (m.get("type").and_then(Value::as_str) == Some("user"), resumable_message(m)));
    }
    fn patch(p: &Value, kinds: &mut HashMap<String, (bool, bool)>) {
        let (Some(id), Some(content)) = (p.get("id").and_then(Value::as_str), p.get("content")) else { return };
        if let Some((is_user, resumable)) = kinds.get_mut(id) {
            let text = content_text(content);
            if *is_user {
                *resumable = !ignored_prompt(text.trim());
            } else if !text.trim().is_empty() {
                *resumable = true;
            }
        }
    }
    for v in std::io::BufReader::new(file).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str::<Value>(&l).ok()) {
        if let Some(to) = v.get("$rewindTo").and_then(Value::as_str) {
            match ids.iter().position(|i| i == to) {
                Some(at) => {
                    for gone in ids.split_off(at) {
                        kinds.remove(&gone);
                    }
                }
                None => {
                    ids.clear();
                    kinds.clear();
                }
            }
        } else if let Some(p) = v.get("$patch").filter(|p| p.is_object()) {
            patch(p, &mut kinds);
            p.get("updates").and_then(Value::as_array).into_iter().flatten().for_each(|u| patch(u, &mut kinds));
            for gone in p.get("removeIds").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
                ids.retain(|i| i != gone);
                kinds.remove(gone);
            }
        } else if v.get("id").is_some_and(Value::is_string) {
            add(&v, &mut ids, &mut kinds);
        } else if let Some(set) = v.get("$set").filter(|s| s.is_object()) {
            if let Some(messages) = set.get("messages").and_then(Value::as_array) {
                ids.clear();
                kinds.clear();
                messages.iter().for_each(|m| add(m, &mut ids, &mut kinds));
            }
        } else if v.get("sessionId").is_some_and(Value::is_string) && v.get("projectHash").is_some_and(Value::is_string) {
            v.get("messages").and_then(Value::as_array).into_iter().flatten().for_each(|m| add(m, &mut ids, &mut kinds));
        }
    }
    kinds.values().any(|(_, resumable)| *resumable)
}

/// Whether Gemini would resume a chat: startup-only, command-only and context-only ones it skips,
/// and `--resume <id>` of one of those is a fatal error.
fn chat_resumable(path: &Path, meta: &Value) -> bool {
    match meta.get("messages").and_then(Value::as_array) {
        Some(legacy) => legacy.iter().any(resumable_message),
        None => jsonl_chat_resumable(path),
    }
}

/// The chats of `cwd`'s project in this home that Gemini would resume, latest activity (file
/// mtime) first; Gemini's own `--resume latest` takes the latest started (`startTime`) instead.
fn chats_in(home: &Path, cwd: &Path) -> Vec<String> {
    let hashes = project_hashes(cwd);
    let mut files: Vec<(PathBuf, std::time::SystemTime)> = std::fs::read_dir(gemini_dir(home).join("tmp"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|project| std::fs::read_dir(project.path().join("chats")).ok())
        .flatten()
        .flatten()
        .filter(|f| f.file_name().to_string_lossy().starts_with("session-"))
        .filter(|f| f.path().extension().is_some_and(|e| e == "jsonl" || e == "json"))
        .filter_map(|f| Some((f.path(), f.metadata().ok()?.modified().ok()?)))
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    let mut out: Vec<String> = Vec::new();
    for (path, _) in files {
        let Some(meta) = chat_meta(&path, &hashes) else { continue };
        let Some(id) = meta.get("sessionId").and_then(Value::as_str).filter(|id| is_uuid(id)) else { continue };
        let ours = meta.get("projectHash").and_then(Value::as_str).is_some_and(|h| hashes.iter().any(|x| x == h));
        let subagent = meta.get("kind").and_then(Value::as_str) == Some("subagent");
        if ours && !subagent && !out.iter().any(|o| o == id) && chat_resumable(&path, &meta) {
            out.push(id.to_string());
        }
    }
    out
}

/// Gemini CLI's OAuth client (an "installed application" client the CLI ships in
/// packages/core/src/code_assist/oauth2.ts as `OAUTH_CLIENT_ID` / `OAUTH_CLIENT_SECRET`). We do not embed it:
/// it is read at runtime, in order, from
/// 1. env GEMINI_OAUTH_CLIENT_ID + GEMINI_OAUTH_CLIENT_SECRET,
/// 2. the file at env GEMINI_OAUTH2_JS_PATH,
/// 3. the user's installed Gemini CLI package (`gemini` on PATH → npm shim/symlink → package root,
///    `npm root -g`, Homebrew prefixes): `oauth2.js` of @google/gemini-cli-core or the bundle chunks.
///
/// Results are cached (misses for 10 minutes, so a fresh install is picked up).
mod oauth_client {
    use crate::util;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    #[derive(Debug, Clone, PartialEq)]
    pub struct OAuthClient {
        pub id: String,
        pub secret: String,
    }

    const PKG: [&str; 2] = ["@google", "gemini-cli"];
    const MAX_JS_BYTES: u64 = 32 * 1024 * 1024;
    const MISS_TTL: Duration = Duration::from_secs(600);

    static CACHE: Mutex<Option<(Instant, Option<OAuthClient>)>> = Mutex::new(None);

    pub async fn resolve() -> Option<OAuthClient> {
        if let (Ok(id), Ok(secret)) = (std::env::var("GEMINI_OAUTH_CLIENT_ID"), std::env::var("GEMINI_OAUTH_CLIENT_SECRET")) {
            if !id.trim().is_empty() && !secret.trim().is_empty() {
                return Some(OAuthClient { id: id.trim().into(), secret: secret.trim().into() });
            }
        }
        if let Some(found) = std::env::var_os("GEMINI_OAUTH2_JS_PATH").and_then(|p| from_file(Path::new(&p))) {
            return Some(found);
        }
        if let Ok(guard) = CACHE.lock() {
            match guard.as_ref() {
                Some((_, Some(hit))) => return Some(hit.clone()),
                Some((at, None)) if at.elapsed() < MISS_TTL => return None,
                _ => {}
            }
        }
        let npm_root = npm_global_root().await;
        let found = tokio::task::spawn_blocking(move || search_installed(npm_root)).await.ok().flatten();
        if let Ok(mut guard) = CACHE.lock() {
            *guard = Some((Instant::now(), found.clone()));
        }
        found
    }

    /// `npm root -g` (e.g. %APPDATA%\npm\node_modules, /usr/local/lib/node_modules).
    async fn npm_global_root() -> Option<PathBuf> {
        let npm = util::which("npm")?;
        util::run_capture(&npm, &["root", "-g"], Duration::from_secs(10)).await.map(PathBuf::from).filter(|p| p.is_dir())
    }

    fn search_installed(npm_root: Option<PathBuf>) -> Option<OAuthClient> {
        package_roots(npm_root).iter().find_map(|root| from_package(root))
    }

    /// Candidate @google/gemini-cli package dirs, most specific first.
    fn package_roots(npm_root: Option<PathBuf>) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = vec![];
        let pkg_in = |modules: &Path| modules.join(PKG[0]).join(PKG[1]);
        if let Some(bin) = util::which("gemini") {
            // unix: the shim is a symlink into the package; Windows: gemini.cmd sits next to node_modules.
            let real = std::fs::canonicalize(&bin).unwrap_or_else(|_| bin.clone());
            for start in [real, bin] {
                for dir in start.ancestors().skip(1) {
                    if is_cli_package(dir) {
                        out.push(dir.to_path_buf());
                    }
                    out.push(pkg_in(&dir.join("node_modules")));
                    out.push(pkg_in(&dir.join("lib").join("node_modules")));
                    out.push(pkg_in(&dir.join("libexec").join("lib").join("node_modules")));
                }
            }
        }
        if let Some(root) = npm_root {
            out.push(pkg_in(&root));
        }
        for prefix in ["/opt/homebrew", "/usr/local", "/home/linuxbrew/.linuxbrew"] {
            let prefix = Path::new(prefix);
            out.push(pkg_in(&prefix.join("lib").join("node_modules")));
            out.push(pkg_in(&prefix.join("opt").join("gemini-cli").join("libexec").join("lib").join("node_modules")));
            let cellar = prefix.join("Cellar").join("gemini-cli").join("*").join("libexec").join("lib").join("node_modules");
            if let Ok(paths) = glob::glob(&cellar.to_string_lossy()) {
                out.extend(paths.flatten().map(|p| pkg_in(&p)));
            }
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|p| p.is_dir() && seen.insert(util::path_key(p)));
        out
    }

    fn is_cli_package(dir: &Path) -> bool {
        crate::util::read_json(&dir.join("package.json"))
            .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(|n| n == "@google/gemini-cli"))
            .unwrap_or(false)
    }

    /// Look in the core package's oauth2.js (nested or hoisted), then the bundle's .js files.
    fn from_package(root: &Path) -> Option<OAuthClient> {
        let oauth2 = Path::new("dist").join("src").join("code_assist").join("oauth2.js");
        let core_nested = root.join("node_modules").join(PKG[0]).join("gemini-cli-core");
        let core_sibling = root.parent().map(|p| p.join("gemini-cli-core"));
        for core in std::iter::once(core_nested).chain(core_sibling) {
            if let Some(found) = from_file(&core.join(&oauth2)) {
                return Some(found);
            }
        }
        for dir in [root.join("bundle"), root.join("dist")] {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "js")).collect();
            files.sort();
            if let Some(found) = files.iter().find_map(|f| from_file(f)) {
                return Some(found);
            }
        }
        None
    }

    fn from_file(path: &Path) -> Option<OAuthClient> {
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > MAX_JS_BYTES {
            return None;
        }
        let bytes = std::fs::read(path).ok()?;
        let needle = b"OAUTH_CLIENT_SECRET";
        if !bytes.windows(needle.len()).any(|w| w == needle) {
            return None;
        }
        extract(&String::from_utf8_lossy(&bytes))
    }

    /// Pull `OAUTH_CLIENT_ID = "..."` and `OAUTH_CLIENT_SECRET = "..."` out of JS source.
    pub fn extract(js: &str) -> Option<OAuthClient> {
        let grab = |name: &str| {
            let re = regex::Regex::new(&format!(r#"\b{name}\s*=\s*["'`]([^"'`\s]+)["'`]"#)).ok()?;
            re.captures(js).map(|c| c[1].to_string())
        };
        let id = grab("OAUTH_CLIENT_ID")?;
        let secret = grab("OAUTH_CLIENT_SECRET")?;
        id.ends_with(".apps.googleusercontent.com").then_some(OAuthClient { id, secret })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        const FIXTURE: &str = r#"
// packages/core/src/code_assist/oauth2.ts
var userAccountManager2 = new UserAccountManager();
var OAUTH_CLIENT_ID = "fake-client-id.apps.googleusercontent.com";
var OAUTH_CLIENT_SECRET = "fake-secret";
var OAUTH_SCOPE = ["https://www.googleapis.com/auth/cloud-platform"];
"#;

        #[test]
        fn extracts_from_js() {
            let c = extract(FIXTURE).unwrap();
            assert_eq!(c.id, "fake-client-id.apps.googleusercontent.com");
            assert_eq!(c.secret, "fake-secret");
            assert!(extract("const OAUTH_CLIENT_ID = 'x';").is_none());
        }

        #[test]
        fn finds_in_bundle_and_core_package() {
            let dir = tempfile::tempdir().unwrap();
            let bundled = dir.path().join("bundled-cli");
            std::fs::create_dir_all(bundled.join("bundle")).unwrap();
            std::fs::write(bundled.join("bundle").join("chunk-A.js"), "console.log(1)").unwrap();
            std::fs::write(bundled.join("bundle").join("chunk-B.js"), FIXTURE).unwrap();
            assert_eq!(from_package(&bundled).unwrap().secret, "fake-secret");

            let modules = dir.path().join("node_modules").join("@google");
            let cli = modules.join("gemini-cli");
            let core = modules.join("gemini-cli-core").join("dist").join("src").join("code_assist");
            std::fs::create_dir_all(&cli).unwrap();
            std::fs::create_dir_all(&core).unwrap();
            std::fs::write(core.join("oauth2.js"), FIXTURE).unwrap();
            assert_eq!(from_package(&cli).unwrap().id, "fake-client-id.apps.googleusercontent.com");
            let empty = tempfile::tempdir().unwrap();
            assert!(from_package(empty.path()).is_none());
        }
    }
}

fn dedup_push(out: &mut Vec<DetectedAccount>, acc: DetectedAccount) {
    if !out.iter().any(|o| util::path_key(Path::new(&o.home_dir)) == util::path_key(Path::new(&acc.home_dir))) {
        out.push(acc);
    }
}

#[async_trait]
impl ProviderAdapter for GeminiAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Gemini
    }

    async fn info(&self) -> ProviderInfo {
        let (path, version) = self.version.get("gemini").await;
        ProviderInfo {
            id: ProviderId::Gemini,
            name: "Gemini CLI".into(),
            vendor: "Google".into(),
            cli_command: "gemini".into(),
            cli_installed: path.is_some(),
            cli_path: path,
            cli_version: version,
            desktop_name: None,
            desktop_installed: false,
            desktop_path: None,
            desktop_multi_account: false,
            home_env_var: Some(HOME_ENV.into()),
            install_hint: "npm i -g @google/gemini-cli".into(),
            default_home: self.default_home().to_string_lossy().into_owned(),
        }
    }

    /// The dir that contains `.gemini` (GEMINI_CLI_HOME semantics), i.e. the user's home by default.
    fn default_home(&self) -> PathBuf {
        std::env::var_os(HOME_ENV).filter(|v| !v.is_empty()).map(PathBuf::from).unwrap_or_else(util::home)
    }

    async fn detect_accounts(&self) -> Vec<DetectedAccount> {
        let default = self.default_home();
        let mut candidates: Vec<(PathBuf, Option<String>)> = vec![(default.clone(), None)];
        if let Ok(entries) = std::fs::read_dir(util::home()) {
            for e in entries.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                if name.starts_with(".gemini-") && e.path().is_dir() {
                    candidates.push((e.path(), Some(format!("Found {name}"))));
                }
            }
        }
        for (p, _) in scan_launcher_scripts(HOME_ENV) {
            candidates.push((p, Some(format!("Referenced by a launcher script ({HOME_ENV})"))));
        }
        let mut out = vec![];
        for (home, hint) in candidates {
            if !looks_like_home(&home) {
                continue;
            }
            let id = read_identity_sync(&home);
            let is_default = util::path_key(&home) == util::path_key(&default);
            dedup_push(
                &mut out,
                DetectedAccount {
                    provider_id: ProviderId::Gemini,
                    home_dir: home.to_string_lossy().into_owned(),
                    is_default_home: is_default,
                    email: id.email,
                    plan: id.plan,
                    logged_in: id.logged_in,
                    already_managed: false,
                    hint,
                },
            );
        }
        out
    }

    async fn read_identity(&self, home: &Path) -> Identity {
        read_identity_sync(home)
    }

    fn shareable_paths(&self) -> Vec<(ShareKind, Vec<&'static str>)> {
        vec![
            // ~/.gemini/skills and the cross-tool ~/.agents/skills (both resolved from GEMINI_CLI_HOME).
            (ShareKind::Skills, vec![".gemini/skills", ".agents/skills"]),
            (ShareKind::Plugins, vec![".gemini/extensions"]),
            (ShareKind::Agents, vec![".gemini/commands", ".gemini/agents"]),
            (ShareKind::Instructions, vec![".gemini/GEMINI.md"]),
            (ShareKind::Settings, vec![".gemini/settings.json", ".gemini/policies"]),
            // MCP servers live inside settings.json (mcpServers), shared via Settings.
            (ShareKind::Mcp, vec![]),
            // Chats: tmp/<project-id>/chats; project ids come from projects.json; shell history in history/.
            (ShareKind::History, vec![".gemini/tmp", ".gemini/projects.json", ".gemini/history"]),
        ]
    }

    fn credential_paths(&self) -> Vec<&'static str> {
        vec![
            ".gemini/oauth_creds.json",
            ".gemini/google_accounts.json",
            ".gemini/.env",
            ".gemini/gemini-credentials.json",
            ".gemini/mcp-oauth-tokens.json",
            ".gemini/a2a-oauth-tokens.json",
        ]
    }

    async fn init_home(&self, home: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(gemini_dir(home))?;
        Ok(())
    }

    async fn cli_command(&self, account: &Account, resume: Resume, extra_args: &[String]) -> anyhow::Result<CommandSpec> {
        let mut spec = base_command(account);
        match resume {
            Resume::No => {}
            Resume::Session(id) if is_uuid(&id) => spec.args.extend(["--resume".to_string(), id]),
            Resume::Last | Resume::Session(_) => spec.args.extend(["--resume".to_string(), "latest".to_string()]),
        }
        spec.args.extend(extra_args.iter().cloned());
        Ok(spec)
    }

    fn resume_candidates(&self, account: &Account, cwd: &Path, wanted: Option<&str>) -> Vec<String> {
        let mut own = chats_in(Path::new(&account.home_dir), cwd);
        // Gemini opens a chat by id only from its own project's chats in this home.
        let wanted = wanted.filter(|id| own.iter().any(|o| o == id)).map(str::to_string);
        own.truncate(RESUME_CANDIDATES);
        wanted_first(wanted, own)
    }

    /// Gemini CLI has no login subcommand: on first run (or after /auth) it asks how to sign in.
    async fn login_command(&self, account: &Account) -> anyhow::Result<CommandSpec> {
        Ok(base_command(account))
    }

    async fn fetch_usage(&self, account: &Account, live: bool) -> UsageSnapshot {
        if !live {
            return FetchError::other("Gemini CLI keeps no local record of its limits; turn on live usage to see them").snapshot(account, UsageSource::None);
        }
        match self.fetch_live(account).await {
            Ok(s) => s,
            Err(e) => {
                let mut s = e.snapshot(account, UsageSource::Api);
                s.email = read_identity_sync(Path::new(&account.home_dir)).email;
                s
            }
        }
    }

    async fn scan_activity(&self, account: &Account, since_days: u32) -> Vec<DailyActivity> {
        let home = PathBuf::from(&account.home_dir);
        let id = account.id.clone();
        tokio::task::spawn_blocking(move || scan_gemini_activity(&home, &id, since_days)).await.unwrap_or_default()
    }
}

fn base_command(account: &Account) -> CommandSpec {
    let home = Path::new(&account.home_dir);
    let program = util::which("gemini").map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "gemini".into());
    let mut spec = CommandSpec { program, ..Default::default() };
    spec.env.insert(HOME_ENV.into(), Some(account.home_dir.clone()));
    // An inherited API key would silently override the account's Google login.
    if auth_kind(home) != AuthKind::ApiKey {
        spec.env.insert("GEMINI_API_KEY".into(), None);
        spec.env.insert("GOOGLE_API_KEY".into(), None);
    }
    spec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_refresh_does_not_read_as_signed_out() {
        let e = offline_refresh("error sending request");
        assert_eq!(e.kind, ErrorKind::Network);
        assert_eq!(e.message, "Could not reach Google to renew the Gemini session (error sending request)");
    }

    #[test]
    fn only_a_refused_refresh_token_means_signed_out() {
        assert_eq!(refresh_refused(400).kind, ErrorKind::Auth);
        assert_eq!(refresh_refused(401).kind, ErrorKind::Auth);
        for status in [429, 500, 503] {
            assert_eq!(refresh_refused(status).kind, ErrorKind::Other, "{status}");
        }
    }

    fn account(home: &Path) -> Account {
        Account {
            id: "acc1".into(),
            provider_id: ProviderId::Gemini,
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

    fn jwt(payload: &Value) -> String {
        let e = |v: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(v);
        format!("{}.{}.sig", e(br#"{"alg":"RS256"}"#), e(payload.to_string().as_bytes()))
    }

    #[test]
    fn identity_prefers_google_accounts_then_id_token() {
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path().join(".gemini");
        std::fs::create_dir_all(&g).unwrap();
        let creds =
            json!({"access_token": "ya29.x", "refresh_token": "1//r", "id_token": jwt(&json!({"email": "jwt@example.com"})), "expiry_date": 1}).to_string();
        std::fs::write(g.join("oauth_creds.json"), creds).unwrap();
        let id = read_identity_sync(dir.path());
        assert!(id.logged_in);
        assert_eq!(id.email.as_deref(), Some("jwt@example.com"));
        std::fs::write(g.join("google_accounts.json"), r#"{"active":"me@example.com","old":["a@b.c"]}"#).unwrap();
        assert_eq!(read_identity_sync(dir.path()).email.as_deref(), Some("me@example.com"));
        assert!(looks_like_home(dir.path()));
    }

    #[test]
    fn api_key_auth_detected_from_settings_and_env() {
        let dir = tempfile::tempdir().unwrap();
        let g = dir.path().join(".gemini");
        std::fs::create_dir_all(&g).unwrap();
        assert_eq!(auth_kind(dir.path()), AuthKind::Unknown);
        std::fs::write(g.join(".env"), "# key\nexport GEMINI_API_KEY=abc\n").unwrap();
        assert_eq!(auth_kind(dir.path()), AuthKind::ApiKey);
        std::fs::write(g.join("settings.json"), "{\n  // comment\n  \"security\": { \"auth\": { \"selectedType\": \"oauth-personal\" } },\n}").unwrap();
        assert_eq!(auth_kind(dir.path()), AuthKind::OAuth);
    }

    #[test]
    fn load_code_assist_parsing() {
        let v = json!({
            "currentTier": {"id": "standard-tier", "name": "Gemini Code Assist"},
            "paidTier": {"id": "g1-pro-tier", "name": "Google AI Pro"},
            "cloudaicompanionProject": "proj-123"
        });
        assert_eq!(parse_load_code_assist(&v), LoadInfo { project: Some("proj-123".into()), plan: Some("Google AI Pro".into()), ineligible: None });
        let free = json!({"currentTier": {"id": "free-tier", "name": "Gemini Code Assist for individuals"}, "cloudaicompanionProject": "p"});
        assert_eq!(parse_load_code_assist(&free).plan.as_deref(), Some("Free"));
        let gone = json!({"ineligibleTiers": [{"reasonCode": "UNSUPPORTED_CLIENT", "reasonMessage": "UNSUPPORTED_CLIENT"}]});
        let info = parse_load_code_assist(&gone);
        assert!(info.project.is_none());
        assert!(info.ineligible.unwrap().contains("Antigravity"));
    }

    #[test]
    fn quota_parsing_keeps_lowest_per_model_and_orders_pro_flash() {
        let v = json!({"buckets": [
            {"modelId": "gemini-2.5-flash-lite", "remainingFraction": 1.0, "resetTime": "2026-10-21T00:00:00Z", "tokenType": "REQUESTS"},
            {"modelId": "gemini-2.5-flash", "remainingFraction": 0.75, "resetTime": "2026-10-21T00:00:00Z", "tokenType": "REQUESTS"},
            {"modelId": "gemini-2.5-pro", "remainingFraction": 0.6, "resetTime": "2026-10-21T00:00:00Z", "tokenType": "REQUESTS"},
            {"modelId": "gemini-2.5-pro", "remainingFraction": 0.25, "resetTime": "2026-10-21T07:00:00Z", "tokenType": "TOKENS"},
            {"modelId": "gemini-3-pro-preview", "remainingFraction": "0.5", "resetTime": "2026-10-21T00:00:00Z"},
            {"modelId": "gemini-2.0-flash", "remainingFraction": 0.9},
            {"remainingFraction": 0.1}
        ]});
        let w = parse_quota(&v);
        let ids: Vec<&str> = w.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["gemini-3-pro-preview", "gemini-2.5-pro", "gemini-2.5-flash", "gemini-2.0-flash"]);
        assert_eq!(w[1].used_percent, 75.0);
        assert_eq!(w[1].label, "Gemini 2.5 Pro");
        assert!(w[1].resets_at.as_deref().unwrap().starts_with("2026-10-21T07:00:00"));
        assert_eq!(w[0].used_percent, 50.0);
        assert_eq!(model_label("gemini-2.5-flash-lite"), "Gemini 2.5 Flash Lite");
    }

    #[test]
    fn refresh_merge_keeps_other_fields() {
        let mut raw = json!({"access_token": "old", "refresh_token": "r", "id_token": "i", "expiry_date": 1, "scope": "s", "extra": true});
        apply_refresh(&mut raw, &json!({"access_token": "new", "expires_in": 3599, "token_type": "Bearer"}), 1_000).unwrap();
        assert_eq!(raw["access_token"], "new");
        assert_eq!(raw["expiry_date"], 1_000 + 3_599_000);
        assert_eq!(raw["refresh_token"], "r");
        assert_eq!(raw["extra"], true);
        assert!(apply_refresh(&mut raw, &json!({"error": "invalid_grant"}), 0).is_none());
        assert_eq!(form_encode(&[("a", "x y+/"), ("b", "1")]), "a=x%20y%2B%2F&b=1");
    }

    #[test]
    fn chat_jsonl_activity() {
        let jsonl = [
            r#"{"sessionId":"s1","projectHash":"h","startTime":"2026-10-18T12:00:00.000Z","lastUpdated":"2026-10-18T12:00:00.000Z","kind":"main"}"#,
            r#"{"id":"u1","timestamp":"2026-10-18T12:00:01.000Z","type":"user","content":[{"text":"hi"}]}"#,
            r#"{"id":"g1","timestamp":"2026-10-18T12:00:02.000Z","type":"gemini","content":"hello","thoughts":[],"tokens":null,"model":"gemini-2.5-pro"}"#,
            r#"{"$set":{"lastUpdated":"2026-10-18T12:00:02.000Z"}}"#,
            r#"{"id":"g1","timestamp":"2026-10-18T12:00:02.000Z","type":"gemini","content":"hello","thoughts":[],"tokens":{"input":1000,"output":50,"cached":400,"thoughts":20,"tool":0,"total":1070},"model":"gemini-2.5-pro"}"#,
            r#"{"id":"g2","timestamp":"2026-10-18T12:01:00.000Z","type":"gemini","content":"more","tokens":{"input":200,"output":10,"cached":0,"thoughts":0,"tool":0,"total":210},"model":"gemini-2.5-flash"}"#,
            "not json",
        ]
        .join("\n");
        let mut days = DayMap::new();
        fold_gemini_chat(&jsonl, true, &mut days);
        let day = local_date("2026-10-18T12:00:00Z").unwrap();
        let d = &days[&day];
        assert_eq!(d.sessions, 1);
        assert_eq!(d.messages, 2);
        assert_eq!(d.input_tokens, 600 + 200);
        assert_eq!(d.cache_tokens, 400);
        assert_eq!(d.output_tokens, 70 + 10);
        assert_eq!(d.models["gemini-2.5-pro"], 1070);
    }

    #[test]
    fn legacy_chat_json_activity() {
        let legacy = json!({"sessionId": "s", "startTime": "2026-10-17T12:00:00Z", "messages": [
            {"id": "a", "timestamp": "2026-10-17T12:00:00Z", "type": "user", "content": "q"},
            {"id": "b", "timestamp": "2026-10-17T12:00:05Z", "type": "gemini", "content": "a", "tokens": {"input": 10, "output": 5, "cached": 0, "thoughts": 0}, "model": "gemini-2.5-pro"}
        ]})
        .to_string();
        let mut days = DayMap::new();
        fold_gemini_chat(&legacy, false, &mut days);
        let d = &days[&local_date("2026-10-17T12:00:00Z").unwrap()];
        assert_eq!((d.sessions, d.messages, d.input_tokens, d.output_tokens), (0, 1, 10, 5));
    }

    #[test]
    fn scan_activity_walks_tmp_chats() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join(".gemini/tmp/proj/chats");
        std::fs::create_dir_all(chats.join("parent")).unwrap();
        let ts = chrono::Utc::now().to_rfc3339();
        let line = |id: &str| json!({"id": id, "timestamp": ts, "type": "gemini", "tokens": {"input": 5, "output": 1}, "model": "m"}).to_string();
        std::fs::write(chats.join("session-2026-10-20T10-00-abcd1234.jsonl"), line("x")).unwrap();
        std::fs::write(chats.join("parent/sub.jsonl"), line("y")).unwrap();
        let out = scan_gemini_activity(dir.path(), "acc1", 7);
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].sessions, out[0].messages, out[0].input_tokens), (1, 2, 10));
        assert_eq!(out[0].account_id, "acc1");
    }

    /// A chat recording as Gemini CLI writes it: metadata first, then message records.
    fn chat(home: &Path, project: &str, file: &str, meta: Value, messages: &[Value]) {
        let dir = home.join(".gemini/tmp").join(project).join("chats");
        std::fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
        let lines: Vec<String> = std::iter::once(&meta).chain(messages).map(Value::to_string).collect();
        std::fs::write(dir.join(file), lines.join("\n")).unwrap();
        // Distinct modification times, in call order.
        std::thread::sleep(Duration::from_millis(20));
    }

    /// What Gemini records as `projectHash` for a folder.
    fn hash(project: &str) -> String {
        format!("{:x}", Sha256::digest(project.as_bytes()))
    }

    fn meta(id: &str, project: &str, kind: &str) -> Value {
        json!({"sessionId": id, "projectHash": hash(project), "startTime": "2026-10-20T10:00:00.000Z", "lastUpdated": "2026-10-20T10:00:00.000Z", "kind": kind})
    }

    fn said(text: &str) -> Value {
        json!({"id": "m1", "timestamp": "2026-10-20T10:00:01.000Z", "type": "user", "content": [{"text": text}]})
    }

    #[test]
    fn a_legacy_chat_of_another_project_is_not_read_in_full() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("session-x.json");
        // As Gemini wrote them: pretty-printed, `projectHash` right after `sessionId`.
        let big = json!([{"id": "u", "type": "user", "content": "x".repeat(100_000)}]);
        let text =
            format!("{{\n  \"sessionId\": \"a1b2c3d4-e5f6-7890-abcd-ef1234567890\",\n  \"projectHash\": \"{}\",\n  \"messages\": {big}\n}}", hash("/work"));
        std::fs::write(&f, text).unwrap();
        assert!(legacy_chat_is_for(&f, &[hash("/work")]));
        assert!(!legacy_chat_is_for(&f, &[hash("/elsewhere")]));
        assert!(!legacy_chat_is_for(&dir.path().join("missing.json"), &[hash("/work")]));
    }

    /// Gemini resumes a chat only when something in it is worth resuming (its
    /// `isResumableMessageRecord`); asking it for any other one by id is a fatal error.
    #[test]
    fn only_chats_gemini_would_resume_are_offered() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let id = |n: u8| format!("a1b2c3d4-e5f6-7890-abcd-ef12345678{n:02}");
        let msg = |id: &str, kind: &str, content: Value| json!({"id": id, "timestamp": "2026-10-20T10:00:01.000Z", "type": kind, "content": content});
        let cases: Vec<(u8, Vec<Value>, bool)> = vec![
            (1, vec![said("/model")], false),
            (2, vec![said("  <session_context>cwd</session_context>")], false),
            (3, vec![said("?"), said("<hook_context>x")], false),
            (4, vec![msg("g", "gemini", json!(""))], false),
            (5, vec![msg("i", "info", json!("started"))], false),
            (6, vec![said("fix the bug")], true),
            (7, vec![msg("u", "user", json!("plain string prompt"))], true),
            (8, vec![msg("u", "user", json!([{"inlineData": {"mimeType": "image/png", "data": "AA=="}}]))], true),
            (9, vec![json!({"id": "g", "type": "gemini", "content": "", "toolCalls": [{"id": "t", "name": "ls"}]})], true),
            (10, vec![json!({"id": "g", "type": "gemini", "content": "", "thoughts": [{"subject": "s"}]})], true),
            // A prompt later rewound away, and one removed by a patch.
            (11, vec![said("hello"), json!({"$rewindTo": "m1"})], false),
            (12, vec![said("hello"), json!({"$patch": {"removeIds": ["m1"]}})], false),
            // An empty reply patched with its text once it streamed in.
            (13, vec![msg("g", "gemini", json!("")), json!({"$patch": {"id": "g", "content": "done"}})], true),
            // A prompt rewritten into a command.
            (14, vec![said("hello"), json!({"$patch": {"updates": [{"id": "m1", "content": [{"text": "/clear"}]}]}})], false),
            // Messages replaced wholesale.
            (15, vec![said("/help"), json!({"$set": {"messages": [msg("u2", "user", json!("real"))]}})], true),
        ];
        for (n, messages, _) in &cases {
            chat(home, "work", &format!("session-2026-10-20T10-{n:02}-a1b2c3d4.jsonl"), meta(&id(*n), "/work", "main"), messages);
        }
        let legacy = |n: u8, content: &str| {
            let v =
                json!({"sessionId": id(n), "projectHash": hash("/work"), "startTime": "t", "lastUpdated": "t", "messages": [msg("u", "user", json!(content))]});
            std::fs::write(home.join(format!(".gemini/tmp/work/chats/session-2026-10-19T10-{n:02}-a1b2c3d4.json")), serde_json::to_string_pretty(&v).unwrap())
                .unwrap();
        };
        legacy(20, "/stats");
        legacy(21, "a real legacy prompt");

        let offered = GeminiAdapter::new().resume_candidates(&account(home), Path::new("/work"), None);
        for (n, _, ok) in &cases {
            assert_eq!(offered.contains(&id(*n)), *ok, "chat {n}: {offered:?}");
        }
        assert!(!offered.contains(&id(20)));
        assert!(offered.contains(&id(21)));
    }

    #[test]
    fn resume_offers_this_homes_chats_in_the_folder_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let id = |n: u8| format!("a1b2c3d4-e5f6-7890-abcd-ef12345678{n:02}");
        let legacy = json!({"sessionId": id(1), "projectHash": hash("/work"), "startTime": "2026-10-19T10:00:00Z", "lastUpdated": "2026-10-19T10:00:00Z",
            "messages": [{"id": "m", "timestamp": "2026-10-19T10:00:00Z", "type": "user", "content": "hi"}]});
        let legacy_file = home.join(".gemini/tmp/work/chats/session-2026-10-19T10-00-a1b2c3d4.json");
        std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
        std::fs::write(&legacy_file, serde_json::to_string_pretty(&legacy).unwrap()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        chat(home, "work", "session-2026-10-20T10-00-a1b2c3d4.jsonl", meta(&id(2), "/work", "main"), &[said("old")]);
        chat(home, "work", "session-2026-10-20T11-00-a1b2c3d4.jsonl", meta(&id(3), "/work", "main"), &[said("new")]);
        chat(home, "elsewhere", "session-2026-10-20T12-00-a1b2c3d4.jsonl", meta(&id(4), "/elsewhere", "main"), &[said("other")]);
        // A sub-agent's chat, and a session that only started up.
        chat(home, "work", &format!("{}/{}.jsonl", id(3), id(5)), meta(&id(5), "/work", "subagent"), &[said("sub")]);
        chat(home, "work", "session-2026-10-20T13-00-a1b2c3d4.jsonl", meta(&id(6), "/work", "subagent"), &[said("sub")]);
        chat(home, "work", "session-2026-10-20T14-00-a1b2c3d4.jsonl", meta(&id(7), "/work", "main"), &[]);

        let a = GeminiAdapter::new();
        let acc = account(home);
        let work = Path::new("/work");
        assert_eq!(a.resume_candidates(&acc, work, None), [id(3), id(2), id(1)]);
        // A chat asked for comes first, but only when Gemini can open it from here: `--resume <id>`
        // looks in the folder's own chats only.
        assert_eq!(a.resume_candidates(&acc, work, Some(&id(2))), [id(2), id(3), id(1)]);
        assert_eq!(a.resume_candidates(&acc, work, Some(&id(4))), [id(3), id(2), id(1)]);
        assert_eq!(a.resume_candidates(&acc, Path::new("/elsewhere"), Some(&id(4))), [id(4)]);
        assert!(a.resume_candidates(&acc, Path::new("/nowhere"), None).is_empty());
        let missing = account(&home.join("missing"));
        assert!(a.resume_candidates(&missing, work, None).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_reached_through_a_symlink_finds_its_chats() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // Gemini hashes the folder as the process sees it: the real path.
        let id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";
        let seen = std::fs::canonicalize(&real).unwrap();
        chat(dir.path(), "real", "session-2026-10-20T10-00-a1b2c3d4.jsonl", meta(id, &seen.to_string_lossy(), "main"), &[said("hi")]);
        let acc = account(dir.path());
        assert_eq!(GeminiAdapter::new().resume_candidates(&acc, &link, None), [id]);
        assert_eq!(GeminiAdapter::new().resume_candidates(&acc, &seen, None), [id]);
    }

    #[tokio::test]
    async fn resume_by_id_names_the_chat() {
        let dir = tempfile::tempdir().unwrap();
        let (a, acc) = (GeminiAdapter::new(), account(dir.path()));
        let args = |r: Resume| {
            let (a, acc) = (&a, &acc);
            async move { a.cli_command(acc, r, &[]).await.unwrap().args }
        };
        assert_eq!(args(Resume::Session("a1b2c3d4-e5f6-7890-abcd-ef1234567890".into())).await, ["--resume", "a1b2c3d4-e5f6-7890-abcd-ef1234567890"]);
        assert_eq!(args(Resume::Last).await, ["--resume", "latest"]);
        assert!(args(Resume::No).await.is_empty());
        // Gemini reads a number as a list index; an id from a damaged file never becomes one.
        assert_eq!(args(Resume::Session("3".into())).await, ["--resume", "latest"]);
        assert_eq!(args(Resume::Session("--yolo".into())).await, ["--resume", "latest"]);
    }

    #[tokio::test]
    async fn commands_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        let a = GeminiAdapter::new();
        a.init_home(dir.path()).await.unwrap();
        assert!(dir.path().join(".gemini").is_dir());
        let acc = account(dir.path());
        let spec = a.cli_command(&acc, Resume::Last, &["-m".into(), "gemini-2.5-pro".into()]).await.unwrap();
        assert_eq!(spec.env.get(HOME_ENV).cloned().flatten().as_deref(), Some(acc.home_dir.as_str()));
        assert_eq!(spec.env.get("GEMINI_API_KEY"), Some(&None));
        assert_eq!(spec.args, ["--resume", "latest", "-m", "gemini-2.5-pro"]);
        let creds = a.credential_paths();
        for (_, paths) in a.shareable_paths() {
            for p in paths {
                assert!(!creds.contains(&p));
            }
        }
        let snap = a.fetch_usage(&acc, false).await;
        assert_eq!(snap.source, UsageSource::None);
        let snap = a.fetch_usage(&acc, true).await;
        assert!(snap.error.unwrap().contains("Not signed in"));
        assert!(a.desktop_command(&acc).await.unwrap().is_none());
    }
}
