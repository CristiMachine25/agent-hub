//! Claude Code adapter (Anthropic subscription: Pro / Max / Team / Enterprise).
//!
//! Home layout (CLAUDE_CONFIG_DIR, default ~/.claude):
//! - `.credentials.json` → `{"claudeAiOauth": {accessToken, refreshToken, expiresAt(ms), scopes,
//!   subscriptionType, rateLimitTier, ...}}` on Windows/Linux. On macOS the same JSON lives in the
//!   Keychain item `Claude Code-credentials`, suffixed `-<sha256(config dir)[..8]>` when
//!   CLAUDE_CONFIG_DIR is set (verified in the Claude Code 2.1.x binary).
//! - `.claude.json` → `oauthAccount {emailAddress, organizationType, organizationRateLimitTier, ...}`.
//!   For the default home this file is ~/.claude.json, outside ~/.claude.
//! - `projects/**/*.jsonl` → transcripts with per-message token usage.
use super::{CommandSpec, FetchError, Identity, ProviderAdapter, Resume};
use crate::model::*;
use crate::util;
use async_trait::async_trait;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::common::*;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
/// Plan and subscription status; read only to explain why usage is blocked.
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
/// From Claude Code's bundled OAuth config (`TOKEN_URL`, `CLIENT_ID`).
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const OAUTH_BETA: &str = "oauth-2025-04-20";
const KEYCHAIN_SERVICE: &str = "Claude Code-credentials";
const CREDENTIALS_FILE: &str = ".credentials.json";
/// Refresh when the access token expires within this many ms.
const REFRESH_MARGIN_MS: i64 = 60_000;

/// Serializes token refreshes so two polls never spend the same refresh token.
static REFRESH_LOCK: once_cell::sync::Lazy<tokio::sync::Mutex<()>> = once_cell::sync::Lazy::new(|| tokio::sync::Mutex::new(()));

pub struct ClaudeAdapter {
    version: VersionCache,
}

/// Environment variables Claude Code sets for the processes it runs; meaningless for a fresh session.
const SESSION_MARKERS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_ENTRYPOINT",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ATTENDED",
    "CLAUDE_CODE_MESSAGING_SOCKET",
    "CLAUDE_CODE_MESSAGING_TOKEN",
    "CLAUDE_CODE_EXECPATH",
    "CLAUDE_PID",
    "CLAUDE_EFFORT",
];

impl ClaudeAdapter {
    pub fn new() -> Self {
        Self { version: VersionCache::default() }
    }

    fn is_default(&self, home: &Path) -> bool {
        util::path_key(home) == util::path_key(&self.default_home())
    }
}

impl Default for ClaudeAdapter {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------------------------
// Credentials and identity
// ---------------------------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredSource {
    File,
    Keychain,
}

#[derive(Debug, Clone)]
struct OAuthCreds {
    access_token: String,
    refresh_token: Option<String>,
    expires_at_ms: Option<i64>,
    scopes: Vec<String>,
    subscription_type: Option<String>,
    rate_limit_tier: Option<String>,
    source: CredSource,
}

impl OAuthCreds {
    fn needs_refresh(&self, now_ms: i64) -> bool {
        self.access_token.is_empty() || self.expires_at_ms.is_some_and(|e| e < now_ms + REFRESH_MARGIN_MS)
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn parse_creds(v: &Value, source: CredSource) -> Option<OAuthCreds> {
    let o = v.get("claudeAiOauth")?;
    let access_token = str_field(o, "accessToken").unwrap_or_default().to_string();
    let refresh_token = str_field(o, "refreshToken").map(str::to_string);
    if access_token.is_empty() && refresh_token.is_none() {
        return None;
    }
    Some(OAuthCreds {
        access_token,
        refresh_token,
        expires_at_ms: num(o.get("expiresAt")).map(|n| n as i64),
        scopes: o.get("scopes").and_then(Value::as_array).map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect()).unwrap_or_default(),
        subscription_type: str_field(o, "subscriptionType").map(str::to_string),
        rate_limit_tier: str_field(o, "rateLimitTier").map(str::to_string),
        source,
    })
}

fn read_file_creds(home: &Path) -> Option<OAuthCreds> {
    parse_creds(&util::read_json(&home.join(CREDENTIALS_FILE))?, CredSource::File)
}

/// macOS Keychain service names to try for a home.
fn keychain_services(home: &Path, is_default: bool) -> Vec<String> {
    if is_default {
        return vec![KEYCHAIN_SERVICE.to_string()];
    }
    let raw = home.to_string_lossy().to_string();
    let mut variants = vec![raw.clone()];
    let trimmed = raw.trim_end_matches(['/', '\\']).to_string();
    if trimmed != raw {
        variants.push(trimmed);
    }
    variants.into_iter().map(|p| format!("{KEYCHAIN_SERVICE}-{}", keychain_hash(&p))).collect()
}

/// First 8 hex chars of sha256(config dir), as Claude Code computes the Keychain suffix.
fn keychain_hash(config_dir: &str) -> String {
    hex::encode(Sha256::digest(config_dir.as_bytes()))[..8].to_string()
}

/// Keychain secrets are JSON; some writers store them hex-encoded.
fn parse_keychain_secret(raw: &str) -> Option<Value> {
    let raw = raw.trim();
    serde_json::from_str(raw).ok().or_else(|| {
        let bytes = hex::decode(raw).ok()?;
        serde_json::from_slice(&bytes).ok()
    })
}

async fn keychain_read(service: &str) -> Option<Value> {
    let out = util::run_capture(Path::new("/usr/bin/security"), &["find-generic-password", "-s", service, "-w"], Duration::from_secs(10)).await?;
    parse_keychain_secret(&out)
}

/// Add a Keychain item the way Claude Code does (hex-encoded secret), feeding the command through stdin
/// so the token never shows up in a process list.
async fn keychain_write(service: &str, secret: &[u8]) -> anyhow::Result<bool> {
    use tokio::io::AsyncWriteExt;
    let user = std::env::var("USER").unwrap_or_else(|_| "claude".into());
    let line = format!("add-generic-password -a \"{user}\" -s \"{service}\" -X {}\n", hex::encode(secret));
    let mut child = tokio::process::Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(line.as_bytes()).await?;
    }
    let status = tokio::time::timeout(Duration::from_secs(15), child.wait()).await??;
    Ok(status.success() && keychain_exists(service).await)
}

/// Existence check only (no secret read, so no Keychain prompt).
async fn keychain_exists(service: &str) -> bool {
    util::run_capture(Path::new("/usr/bin/security"), &["find-generic-password", "-s", service], Duration::from_secs(5)).await.is_some()
}

/// Credentials for a home: Keychain first on macOS, then the file.
async fn read_creds(home: &Path, is_default: bool) -> Option<OAuthCreds> {
    if cfg!(target_os = "macos") {
        for service in keychain_services(home, is_default) {
            if let Some(c) = keychain_read(&service).await.and_then(|v| parse_creds(&v, CredSource::Keychain)) {
                return Some(c);
            }
        }
    }
    read_file_creds(home)
}

/// `.claude.json` locations for a home (the default home keeps it at ~/.claude.json).
fn metadata_paths(home: &Path, is_default: bool) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if is_default {
        paths.push(util::home().join(".claude.json"));
    }
    paths.push(home.join(".claude.json"));
    paths
}

fn read_oauth_account(home: &Path, is_default: bool) -> Option<Value> {
    metadata_paths(home, is_default).iter().filter_map(|p| util::read_json(p)).find_map(|v| v.get("oauthAccount").filter(|o| o.is_object()).cloned())
}

/// Plan label from `subscriptionType` ("max", or "claude_max" in oauthAccount) and rate-limit tier.
fn plan_label(subscription: Option<&str>, tier: Option<&str>) -> Option<String> {
    let tier = tier.unwrap_or("").to_ascii_lowercase();
    let max_label = || {
        if tier.contains("20x") {
            "Max 20x"
        } else if tier.contains("5x") {
            "Max 5x"
        } else {
            "Max"
        }
    };
    let sub = subscription.map(|s| s.trim().to_ascii_lowercase()).filter(|s| !s.is_empty());
    let label = match sub.as_deref().map(|s| s.strip_prefix("claude_").unwrap_or(s)) {
        Some("max") => max_label().to_string(),
        Some("pro") => "Pro".into(),
        Some("team") => "Team".into(),
        Some("enterprise") => "Enterprise".into(),
        Some("free") => "Free".into(),
        Some(other) => {
            let mut c = other.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str())?
        }
        None if tier.contains("enterprise") => "Enterprise".into(),
        None if tier.contains("team") => "Team".into(),
        None if tier.contains("max") || tier.contains("20x") || tier.contains("5x") => max_label().to_string(),
        None if tier.contains("pro") || tier == "default_claude_ai" => "Pro".into(),
        None => return None,
    };
    Some(label)
}

fn identity_from(creds: Option<&OAuthCreds>, oauth_account: Option<&Value>, logged_in: bool) -> Identity {
    let email = oauth_account.and_then(|o| str_field(o, "emailAddress")).map(str::to_string);
    let plan = creds.and_then(|c| plan_label(c.subscription_type.as_deref(), c.rate_limit_tier.as_deref())).or_else(|| {
        let o = oauth_account?;
        plan_label(str_field(o, "organizationType"), str_field(o, "organizationRateLimitTier"))
    });
    Identity { logged_in, email, plan }
}

// ---------------------------------------------------------------------------------------------
// Usage API
// ---------------------------------------------------------------------------------------------

const ORG_BLOCKED: &str = "This account's organization doesn't allow Claude Code with a subscription login. Use another account or ask an admin.";
const BLOCKED_UNKNOWN: &str = "Claude doesn't allow Claude Code for this account right now. Check its plan at claude.ai, or remove it from Agent Hub.";

/// Why Claude Code is blocked for this account, from its `/api/oauth/profile` answer (None when
/// that couldn't be read). Personal plans never point at an admin.
fn blocked_reason(profile: Option<&Value>) -> String {
    let org = profile.and_then(|p| p.get("organization"));
    let field = |k: &str| org.and_then(|o| str_field(o, k));
    let personal = |t: &str| matches!(t, "claude_pro" | "claude_max");
    match (field("organization_type"), field("subscription_status")) {
        (Some("claude_free"), Some("canceled")) => "This account's subscription was canceled, so it's on the Free plan, which doesn't include Claude Code. Resubscribe at claude.ai, or remove it from Agent Hub.".into(),
        (Some("claude_free"), _) => "This account is on the Free plan, which doesn't include Claude Code. Subscribe at claude.ai, or remove it from Agent Hub.".into(),
        (Some(t), Some("canceled")) if personal(t) => "This account's subscription was canceled, so it can't use Claude Code. Resubscribe at claude.ai, or remove it from Agent Hub.".into(),
        (Some(t), Some(s)) if personal(t) && s != "active" => format!(
            "This account's subscription isn't active ({}), so it can't use Claude Code. Check it at claude.ai, or remove it from Agent Hub.",
            s.replace('_', " ")
        ),
        (Some(t), _) if !personal(t) => ORG_BLOCKED.into(),
        _ => BLOCKED_UNKNOWN.into(),
    }
}

/// The reason already shown for a blocked account, so polls don't ask the profile every time. An
/// unknown reason is asked again.
fn known_blocked_reason(account: &Account) -> Option<String> {
    (account.status == AccountStatus::Blocked).then(|| account.status_message.clone()).flatten().filter(|m| m != BLOCKED_UNKNOWN)
}

#[derive(Debug, PartialEq)]
enum ApiError {
    Unauthorized,
    MissingScope,
    /// The account's organization turned off Claude Code with a subscription login.
    OrgBlocked,
    RateLimited,
    Http(u16),
    Network(String),
    Parse,
}

async fn fetch_oauth_json(url: &str, access_token: &str, user_agent: &str) -> Result<Value, ApiError> {
    let resp = util::http()
        .get(url)
        .bearer_auth(access_token)
        .header("anthropic-beta", OAUTH_BETA)
        .header("Accept", "application/json")
        .header("User-Agent", user_agent)
        .send()
        .await
        .map_err(|e| ApiError::Network(if e.is_timeout() { "timed out".into() } else { "network error".into() }))?;
    let status = resp.status().as_u16();
    let body = resp.text().await.unwrap_or_default();
    if status == 200 {
        return serde_json::from_str(&body).map_err(|_| ApiError::Parse);
    }
    Err(api_error(status, &body))
}

fn api_error(status: u16, body: &str) -> ApiError {
    match status {
        401 => ApiError::Unauthorized,
        403 if body.contains("oauth_not_allowed_for_organization") => ApiError::OrgBlocked,
        403 if body.contains("user:profile") || body.contains("scope") => ApiError::MissingScope,
        403 => ApiError::Unauthorized,
        429 => ApiError::RateLimited,
        s => ApiError::Http(s),
    }
}

/// Map the `/api/oauth/usage` response to windows + monthly extra-usage credits.
/// Unknown keys are ignored; null windows are skipped.
fn parse_usage(v: &Value) -> (Vec<UsageWindow>, Option<Credits>) {
    const WINDOWS: [(&str, &str, u32); 4] = [
        ("five_hour", "5-hour session", 300),
        ("seven_day", "Weekly", 10080),
        ("seven_day_opus", "Weekly Opus", 10080),
        ("seven_day_sonnet", "Weekly Sonnet", 10080),
    ];
    let windows = WINDOWS
        .iter()
        .filter_map(|(key, label, minutes)| {
            let w = v.get(*key).filter(|w| w.is_object())?;
            let used = num(w.get("utilization"))?;
            Some(UsageWindow {
                id: (*key).to_string(),
                label: (*label).to_string(),
                used_percent: used.max(0.0),
                resets_at: str_field(w, "resets_at").map(normalize_iso),
                window_minutes: Some(*minutes),
            })
        })
        .collect();

    // Amounts are in minor units (cents), as CodexBar's ClaudeUsageFetcher divides by 100.
    let credits = v.get("extra_usage").filter(|e| e.get("is_enabled").and_then(Value::as_bool) == Some(true)).and_then(|e| {
        let used = num(e.get("used_credits"));
        let limit = num(e.get("monthly_limit"));
        if used.is_none() && limit.is_none() {
            return None;
        }
        Some(Credits { used: used.unwrap_or(0.0) / 100.0, limit: limit.map(|l| l / 100.0), unit: str_field(e, "currency").unwrap_or("USD").to_string() })
    });
    (windows, credits)
}

// ---------------------------------------------------------------------------------------------
// Token refresh (Windows/Linux file credentials only)
// ---------------------------------------------------------------------------------------------

const LOGIN_INVALID: &str = "Sign in again: the saved Claude login is no longer valid.";
const REFRESH_OFFLINE: &str = "could not reach Anthropic (network error)";
const REFRESH_BAD_RESPONSE: &str = "Anthropic sent an unexpected response";

/// The refresh failure as shown on the account, keeping its kind: being offline must not read as signed out.
fn refresh_failed(e: FetchError) -> FetchError {
    FetchError { message: format!("Could not renew the Claude session: {}.", e.message), ..e }
}

/// The token endpoint refused the refresh without saying invalid_grant.
fn refresh_http_error(status: u16) -> FetchError {
    let msg = format!("Anthropic answered HTTP {status}");
    if matches!(status, 401 | 403) {
        FetchError::auth(msg)
    } else {
        FetchError::other(msg)
    }
}

#[derive(Debug)]
enum RefreshError {
    /// Refresh token rejected: the user has to sign in again.
    InvalidGrant,
    Other(FetchError),
}

/// Apply a token endpoint response onto the credentials file JSON, keeping every other field.
fn apply_refresh(file: &mut Value, resp: &Value, now_ms: i64) -> Option<()> {
    let access = str_field(resp, "access_token")?.to_string();
    let oauth = file.get_mut("claudeAiOauth")?.as_object_mut()?;
    oauth.insert("accessToken".into(), json!(access));
    if let Some(r) = str_field(resp, "refresh_token") {
        oauth.insert("refreshToken".into(), json!(r));
    }
    let expires_in = num(resp.get("expires_in")).unwrap_or(3600.0) as i64;
    oauth.insert("expiresAt".into(), json!(now_ms + expires_in * 1000));
    if let Some(r) = num(resp.get("refresh_token_expires_in")) {
        oauth.insert("refreshTokenExpiresAt".into(), json!(now_ms + (r as i64) * 1000));
    }
    if let Some(scope) = str_field(resp, "scope") {
        oauth.insert("scopes".into(), json!(scope.split_whitespace().collect::<Vec<_>>()));
    }
    Some(())
}

fn is_invalid_grant(status: u16, body: &str) -> bool {
    let err = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str().map(str::to_string).or_else(|| e.get("type").and_then(Value::as_str).map(str::to_string))));
    matches!(err.as_deref(), Some("invalid_grant")) || (status == 400 && body.contains("invalid_grant"))
}

async fn refresh_file_creds(home: &Path, creds: &OAuthCreds) -> Result<OAuthCreds, RefreshError> {
    let _guard = REFRESH_LOCK.lock().await;
    // The CLI (or an earlier poll) may have refreshed in the meantime.
    if let Some(current) = read_file_creds(home) {
        if current.access_token != creds.access_token && !current.needs_refresh(now_ms()) {
            return Ok(current);
        }
    }
    let Some(refresh_token) = creds.refresh_token.clone() else { return Err(RefreshError::InvalidGrant) };
    let mut body = json!({"grant_type": "refresh_token", "refresh_token": refresh_token, "client_id": CLIENT_ID});
    if !creds.scopes.is_empty() {
        body["scope"] = json!(creds.scopes.join(" "));
    }
    let resp = util::http().post(TOKEN_URL).json(&body).send().await.map_err(|_| RefreshError::Other(FetchError::network(REFRESH_OFFLINE)))?;
    let status = resp.status().as_u16();
    let text = resp.text().await.unwrap_or_default();
    if !(200..300).contains(&status) {
        if is_invalid_grant(status, &text) {
            // Lost a race with the CLI? Use its fresh token if one appeared.
            if let Some(current) = read_file_creds(home) {
                if current.refresh_token.as_deref() != Some(refresh_token.as_str()) && !current.needs_refresh(now_ms()) {
                    return Ok(current);
                }
            }
            return Err(RefreshError::InvalidGrant);
        }
        return Err(RefreshError::Other(refresh_http_error(status)));
    }
    let token: Value = serde_json::from_str(&text).map_err(|_| RefreshError::Other(FetchError::other(REFRESH_BAD_RESPONSE)))?;

    let path = home.join(CREDENTIALS_FILE);
    let mut file = util::read_json(&path).ok_or_else(|| RefreshError::Other(FetchError::auth("credentials file disappeared")))?;
    apply_refresh(&mut file, &token, now_ms()).ok_or_else(|| RefreshError::Other(FetchError::other(REFRESH_BAD_RESPONSE)))?;
    let bytes = serde_json::to_vec(&file).map_err(|e| RefreshError::Other(FetchError::other(e.to_string())))?;
    if let Err(e) = write_credentials_atomic(&path, &bytes) {
        log::warn!("claude: could not write refreshed credentials for {}: {e}", home.display());
    }
    parse_creds(&file, CredSource::File).ok_or_else(|| RefreshError::Other(FetchError::other(REFRESH_BAD_RESPONSE)))
}

// ---------------------------------------------------------------------------------------------
// Keychain login renewal (macOS)
// ---------------------------------------------------------------------------------------------
//
// The access token lasts 8 hours. Claude Code renews it under a lock that its own processes
// share, and drops a login whose renewal the server refuses. Renewing it here would race those
// processes, so Agent Hub asks Claude Code to do it: `claude mcp list` loads the claude.ai
// connectors, which renews the login first, and makes no model request (so it uses no quota).

const RENEW_ARGS: [&str; 2] = ["mcp", "list"];
const RENEW_TIMEOUT: Duration = Duration::from_secs(45);
/// How long a usage fetch waits for a renewal. The renewal runs on by itself either way, so the
/// next poll finds the new login; this plus the usage request stays under usage::FETCH_TIMEOUT.
const RENEW_WAIT: Duration = Duration::from_secs(40);
/// After a renewal that left the login expired, wait this long before asking Claude Code again.
const RENEW_RETRY_MS: i64 = 30 * 60_000;
const LOGIN_EXPIRED: &str = "Login token expired. Open Claude once to refresh the login.";
const LOGIN_NEEDS_CLI: &str = "The login expired and Claude Code isn't installed to renew it. Install Claude Code, then open it once or sign in again.";
const LOGIN_RENEWING: &str = "The login expired and is being renewed now.";
const LOGIN_RENEW_FAILED: &str = "Claude Code could not renew this login. Sign in again.";
const LOGIN_RENEW_LATER: &str = "The login expired and could not be renewed yet; Agent Hub will try again later. Starting Claude also renews it.";

/// Claude Code processes renewing logins at once.
static RENEW_SLOTS: once_cell::sync::Lazy<tokio::sync::Semaphore> = once_cell::sync::Lazy::new(|| tokio::sync::Semaphore::new(2));
/// When a renewal last left each home's login expired.
static RENEW_FAILED_AT: once_cell::sync::Lazy<std::sync::Mutex<HashMap<String, i64>>> = once_cell::sync::Lazy::new(Default::default);
/// Homes with a renewal running now, so polls don't start a second one.
static RENEWING: once_cell::sync::Lazy<std::sync::Mutex<HashSet<String>>> = once_cell::sync::Lazy::new(Default::default);

/// Marks a home as renewing for as long as it lives, even if the renewal panics.
struct Renewing(String);

impl Renewing {
    fn start(key: String) -> Option<Self> {
        RENEWING.lock().unwrap().insert(key.clone()).then_some(Self(key))
    }
}

impl Drop for Renewing {
    fn drop(&mut self) {
        RENEWING.lock().unwrap().remove(&self.0);
    }
}

/// Runs the CLI's renewal for one home (waiting for a slot) and reads the login it leaves.
async fn renew_with_cli(program: String, env: HashMap<String, Option<String>>, home: PathBuf, is_default: bool) -> Result<OAuthCreds, FetchError> {
    let _slot = RENEW_SLOTS.acquire().await.map_err(|_| FetchError::expired(LOGIN_EXPIRED))?;
    // Another process may have renewed it while this one waited.
    if let Some(c) = read_creds(&home, is_default).await.filter(|c| !c.needs_refresh(now_ms())) {
        return Ok(c);
    }
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(RENEW_ARGS)
        .current_dir(&home)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    for (k, v) in env {
        match v {
            Some(v) => cmd.env(k, v),
            None => cmd.env_remove(k),
        };
    }
    match tokio::time::timeout(RENEW_TIMEOUT, cmd.status()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => log::warn!("claude: could not start the login renewal for {}: {e}", home.display()),
        Err(_) => log::warn!("claude: login renewal for {} timed out", home.display()),
    }
    let after = read_creds(&home, is_default).await;
    let still_there = after.is_none() && login_still_there(&home, is_default).await;
    renew_outcome(after, still_there, now_ms())
}

fn renew_due(failed_at: Option<i64>, now_ms: i64) -> bool {
    failed_at.is_none_or(|t| now_ms - t >= RENEW_RETRY_MS)
}

/// What a renewal left behind, judged from the login as it reads afterwards. `still_there`: the
/// login item exists even though it couldn't be read (a locked or slow Keychain).
fn renew_outcome(after: Option<OAuthCreds>, still_there: bool, now_ms: i64) -> Result<OAuthCreds, FetchError> {
    match after {
        Some(c) if !c.needs_refresh(now_ms) => Ok(c),
        Some(_) => Err(FetchError::expired(LOGIN_RENEW_LATER)),
        None if still_there => Err(FetchError::expired(LOGIN_RENEW_LATER)),
        // Claude Code removes a login once the server refuses to renew it.
        None => Err(FetchError::auth(LOGIN_RENEW_FAILED)),
    }
}

/// Whether this home still has a login item, without reading its secret.
async fn login_still_there(home: &Path, is_default: bool) -> bool {
    if home.join(CREDENTIALS_FILE).exists() {
        return true;
    }
    if cfg!(target_os = "macos") {
        for service in keychain_services(home, is_default) {
            if keychain_exists(&service).await {
                return true;
            }
        }
    }
    false
}

// ---------------------------------------------------------------------------------------------
// Conversations
// ---------------------------------------------------------------------------------------------
//
// `history.jsonl` holds each prompt typed in a home, with its folder and conversation. Every
// running Claude Code process writes `sessions/<pid>.json` naming the conversation it is in, and
// removes it on a clean exit (a killed process leaves it behind).

/// How many of a folder's conversations Resume looks through.
const RESUME_CANDIDATES: usize = 20;

/// The conversations of prompts typed in `cwd` from this home, newest first, that still have a
/// transcript to reopen.
fn sessions_in(home: &Path, cwd: &Path) -> Vec<String> {
    let Ok(history) = std::fs::read_to_string(home.join("history.jsonl")) else { return Vec::new() };
    let want = util::path_key(cwd);
    let mut tried = HashSet::new();
    history
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| str_field(v, "project").is_some_and(|p| util::path_key(Path::new(p)) == want))
        .filter_map(|v| str_field(&v, "sessionId").map(str::to_string))
        .filter(|id| tried.insert(id.clone()))
        .filter(|id| transcript_exists(home, id))
        .take(RESUME_CANDIDATES)
        .collect()
}

/// Conversation ids are uuids; anything else (say, from a damaged file) never reaches the command line.
fn valid_session_id(id: &str) -> bool {
    id.len() <= 64 && id.starts_with(|c: char| c.is_ascii_alphanumeric()) && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn transcript_exists(home: &Path, id: &str) -> bool {
    let file = format!("{id}.jsonl");
    std::fs::read_dir(home.join("projects")).is_ok_and(|dirs| dirs.flatten().any(|d| d.path().join(&file).is_file()))
}

#[derive(Debug, Clone, PartialEq)]
struct LiveSession {
    pid: u32,
    session_id: String,
    /// When the process started, as `ps -o lstart` prints it in UTC.
    proc_start: Option<String>,
}

fn live_sessions(home: &Path) -> Vec<LiveSession> {
    let Ok(entries) = std::fs::read_dir(home.join("sessions")) else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| util::read_json(&e.path()))
        .filter_map(|v| {
            Some(LiveSession {
                pid: num(v.get("pid")).and_then(|p| u32::try_from(p as i64).ok())?,
                session_id: str_field(&v, "sessionId")?.to_string(),
                proc_start: str_field(&v, "procStart").map(str::to_string),
            })
        })
        .collect()
}

/// Whether the process a sessions file names still runs. A file left by a killed process can name
/// a pid the system has since given to another program, so the start time has to match too.
#[cfg(unix)]
fn process_is_running(s: &LiveSession) -> bool {
    // SAFETY: signal 0 only checks that the pid exists; nothing is sent.
    let exists = unsafe { libc::kill(s.pid as i32, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    if !exists || s.pid <= 1 {
        return false;
    }
    // Without a start time there's no telling a reused pid apart: never let that block a resume.
    let Some(want) = &s.proc_start else { return false };
    process_start(s.pid).is_some_and(|started| same_words(&started, want))
}

/// When a process started, as the CLI writes it: `ps -o lstart` in UTC and the C locale (other
/// locales reorder the date, "Wed 21 Oct").
#[cfg(unix)]
fn process_start(pid: u32) -> Option<String> {
    let out = std::process::Command::new("/bin/ps").args(["-o", "lstart=", "-p", &pid.to_string()]).env("TZ", "UTC").env("LC_ALL", "C").output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(unix))]
fn process_is_running(_s: &LiveSession) -> bool {
    false
}

/// `ps` pads single-digit days ("Oct  1"), so compare word by word.
fn same_words(a: &str, b: &str) -> bool {
    a.split_whitespace().eq(b.split_whitespace())
}

// ---------------------------------------------------------------------------------------------
// Local activity
// ---------------------------------------------------------------------------------------------

#[derive(Default)]
struct DayAgg {
    activity: DailyActivity,
    sessions: HashSet<String>,
}

/// Fold one transcript line into per-day totals. Returns true if it was counted.
fn accumulate_line(v: &Value, first_date: &str, seen: &mut HashSet<String>, days: &mut BTreeMap<String, DayAgg>) -> bool {
    if v.get("type").and_then(Value::as_str) != Some("assistant") {
        return false;
    }
    let Some(msg) = v.get("message") else { return false };
    let Some(usage) = msg.get("usage").filter(|u| u.is_object()) else { return false };
    let Some(date) = v.get("timestamp").and_then(Value::as_str).and_then(local_date) else { return false };
    if date.as_str() < first_date {
        return false;
    }
    let msg_id = str_field(msg, "id");
    let req_id = str_field(v, "requestId");
    if msg_id.is_some() || req_id.is_some() {
        let key = format!("{}:{}", msg_id.unwrap_or(""), req_id.unwrap_or(""));
        if !seen.insert(key) {
            return false;
        }
    }
    let n = |k: &str| num(usage.get(k)).unwrap_or(0.0).max(0.0) as u64;
    let (input, output) = (n("input_tokens"), n("output_tokens"));
    let cache = n("cache_creation_input_tokens") + n("cache_read_input_tokens");

    let day = days.entry(date.clone()).or_default();
    let a = &mut day.activity;
    a.date = date;
    a.input_tokens += input;
    a.output_tokens += output;
    a.cache_tokens += cache;
    a.messages += 1;
    if let Some(model) = str_field(msg, "model").filter(|m| !m.starts_with('<')) {
        *a.models.entry(model.to_string()).or_default() += input + output;
    }
    if let Some(sid) = str_field(v, "sessionId") {
        day.sessions.insert(sid.to_string());
    }
    true
}

fn finish_days(account_id: &str, days: BTreeMap<String, DayAgg>) -> Vec<DailyActivity> {
    days.into_values().map(|d| DailyActivity { account_id: account_id.to_string(), sessions: d.sessions.len() as u32, ..d.activity }).collect()
}

fn scan_projects(account_id: &str, home: &Path, since_days: u32) -> Vec<DailyActivity> {
    let first = first_local_date(since_days);
    let files = jsonl_files_since(&[home.join("projects")], days_ago(since_days.saturating_add(1)));
    let mut seen = HashSet::new();
    let mut days = BTreeMap::new();
    for (path, _) in files {
        for_each_json_line(
            &path,
            |l| l.contains("\"assistant\"") && l.contains("\"usage\""),
            |v| {
                accumulate_line(&v, &first, &mut seen, &mut days);
            },
        );
    }
    finish_days(account_id, days)
}

// ---------------------------------------------------------------------------------------------
// Desktop app
// ---------------------------------------------------------------------------------------------

/// Claude Desktop location. Windows: Squirrel install or the MSIX execution alias
/// `claude-desktop.exe` (verified in the Store package manifest); macOS: Claude.app; Linux: community builds.
fn find_desktop() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        if let Some(local) = dirs::data_local_dir() {
            candidates.push(local.join("AnthropicClaude").join("claude.exe"));
            candidates.push(local.join("Microsoft").join("WindowsApps").join("claude-desktop.exe"));
            candidates.push(local.join("Programs").join("Claude").join("Claude.exe"));
        }
    } else if cfg!(target_os = "macos") {
        candidates.push(PathBuf::from("/Applications/Claude.app"));
        candidates.push(util::home().join("Applications").join("Claude.app"));
    } else {
        candidates.extend(util::which("claude-desktop"));
        candidates.push(PathBuf::from("/usr/bin/claude-desktop"));
        candidates.push(PathBuf::from("/opt/Claude/claude-desktop"));
    }
    // symlink_metadata: MSIX execution aliases are reparse points that `exists()` may not follow.
    candidates.into_iter().find(|p| std::fs::symlink_metadata(p).is_ok())
}

/// Microsoft Store builds run from `WindowsApps`. Windows redirects their writes under AppData into
/// the package, so a data folder there doesn't keep the sign-in.
fn is_store_build(app: &Path) -> bool {
    app.components().any(|c| c.as_os_str().eq_ignore_ascii_case("WindowsApps"))
}

/// The main Claude Desktop install's own config.json (its default profile).
fn desktop_config_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if cfg!(windows) {
        if let Some(packages) = dirs::data_local_dir().map(|d| d.join("Packages")) {
            if let Ok(entries) = std::fs::read_dir(&packages) {
                for e in entries.flatten() {
                    if e.file_name().to_string_lossy().starts_with("Claude_") {
                        paths.push(e.path().join("LocalCache").join("Roaming").join("Claude").join("config.json"));
                    }
                }
            }
        }
    }
    if let Some(config) = dirs::config_dir() {
        // %APPDATA%\Claude, ~/Library/Application Support/Claude, ~/.config/Claude
        paths.push(config.join("Claude").join("config.json"));
    }
    paths
}

/// Account the main Claude Desktop profile was last signed in to.
fn desktop_main_account() -> Option<String> {
    desktop_config_paths().iter().filter_map(|p| util::read_json(p)).find_map(|v| str_field(&v, "lastKnownAccountUuid").map(str::to_string))
}

/// Where a profile's own Claude Desktop data lives. Kept in the profile folder, except for Store builds
/// whose profile folder is under AppData: those go to ~/.switchyard so Windows doesn't redirect them.
fn desktop_data_dir(account: &Account, home: &Path, app: &Path) -> PathBuf {
    let in_profile = home.join("desktop-data");
    let used = std::fs::read_dir(&in_profile).is_ok_and(|mut d| d.next().is_some());
    let under_appdata = [dirs::data_dir(), dirs::data_local_dir()].into_iter().flatten().any(|d| home.starts_with(d));
    if used || !(is_store_build(app) && under_appdata) {
        in_profile
    } else {
        util::home().join(".switchyard").join("claude-desktop").join(&account.id)
    }
}

// ---------------------------------------------------------------------------------------------
// Adapter
// ---------------------------------------------------------------------------------------------

impl ClaudeAdapter {
    fn snapshot(&self, account: &Account, source: UsageSource, ident: &Identity) -> UsageSnapshot {
        let mut s = UsageSnapshot::empty(account, source);
        s.email = ident.email.clone().or_else(|| account.email.clone());
        s.plan = ident.plan.clone().or_else(|| account.plan.clone());
        s
    }

    /// No live data: Claude Code keeps no reliable local copy of its rate limits.
    fn no_data(&self, account: &Account, ident: &Identity, error: FetchError) -> UsageSnapshot {
        let mut s = self.snapshot(account, UsageSource::None, ident);
        error.apply(&mut s);
        s
    }

    fn cli_env(&self, account: &Account) -> HashMap<String, Option<String>> {
        let mut env = HashMap::new();
        let home = Path::new(&account.home_dir);
        let value = (!(account.is_default_home || self.is_default(home))).then(|| account.home_dir.clone());
        env.insert("CLAUDE_CONFIG_DIR".into(), value);
        // Strip API-key style auth so the subscription login is used.
        env_unset(&mut env, &["ANTHROPIC_API_KEY", "ANTHROPIC_AUTH_TOKEN", "CLAUDE_CODE_OAUTH_TOKEN"]);
        // When Agent Hub itself was started from a Claude Code terminal, don't let the new CLI think it is
        // a child of that session (it would turn off transcript saving and talk to the parent's socket).
        let inherited: Vec<&str> = SESSION_MARKERS.iter().copied().filter(|k| std::env::var_os(k).is_some()).collect();
        env_unset(&mut env, &inherited);
        env
    }

    fn program(&self) -> String {
        which_str("claude").unwrap_or_else(|| "claude".into())
    }

    /// Has Claude Code renew an expired Keychain login, then reads it again. Without the CLI it asks
    /// the user to sign in; within RENEW_RETRY_MS of a renewal that didn't help, it reports the login
    /// as expired (still renewing). The
    /// renewal runs in its own task: a poll that stops waiting (or times out) never cuts it short.
    async fn renew_keychain_login(&self, account: &Account, home: &Path, is_default: bool) -> Result<OAuthCreds, FetchError> {
        let Some(program) = which_str("claude") else { return Err(FetchError::auth(LOGIN_NEEDS_CLI)) };
        let key = util::path_key(home);
        if !renew_due(RENEW_FAILED_AT.lock().unwrap().get(&key).copied(), now_ms()) {
            return Err(FetchError::expired(LOGIN_RENEW_LATER));
        }
        let Some(running) = Renewing::start(key.clone()) else { return Err(FetchError::expired(LOGIN_RENEWING)) };
        let (env, home) = (self.cli_env(account), home.to_path_buf());
        let job = tokio::spawn(async move {
            let _running = running;
            let outcome = renew_with_cli(program, env, home, is_default).await;
            let mut failed = RENEW_FAILED_AT.lock().unwrap();
            match &outcome {
                Ok(_) => failed.remove(&key),
                Err(_) => failed.insert(key, now_ms()),
            };
            outcome
        });
        match tokio::time::timeout(RENEW_WAIT, job).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(FetchError::expired(LOGIN_EXPIRED)),
            Err(_) => Err(FetchError::expired(LOGIN_RENEWING)),
        }
    }

    async fn user_agent(&self) -> String {
        let (_, version) = self.version.get("claude").await;
        format!("claude-cli/{} (external, cli)", version.unwrap_or_else(|| "2.1.0".into()))
    }
}

#[async_trait]
impl ProviderAdapter for ClaudeAdapter {
    fn id(&self) -> ProviderId {
        ProviderId::Claude
    }

    async fn info(&self) -> ProviderInfo {
        let (cli_path, cli_version) = self.version.get("claude").await;
        let desktop = find_desktop();
        let install_hint = if cfg!(windows) {
            "irm https://claude.ai/install.ps1 | iex   (PowerShell)  or  npm i -g @anthropic-ai/claude-code"
        } else {
            "curl -fsSL https://claude.ai/install.sh | bash   or  npm i -g @anthropic-ai/claude-code"
        };
        ProviderInfo {
            id: ProviderId::Claude,
            name: "Claude Code".into(),
            vendor: "Anthropic".into(),
            cli_command: "claude".into(),
            cli_installed: cli_path.is_some(),
            cli_path,
            cli_version,
            desktop_name: Some("Claude Desktop".into()),
            desktop_installed: desktop.is_some(),
            desktop_path: desktop.map(|p| p.to_string_lossy().to_string()),
            desktop_multi_account: true,
            home_env_var: Some("CLAUDE_CONFIG_DIR".into()),
            install_hint: install_hint.into(),
            default_home: self.default_home().to_string_lossy().into_owned(),
        }
    }

    fn default_home(&self) -> PathBuf {
        util::home().join(".claude")
    }

    async fn detect_accounts(&self) -> Vec<DetectedAccount> {
        let home = util::home();
        let default = self.default_home();
        let mut candidates: Vec<(PathBuf, String)> = vec![(default.clone(), "Default home (~/.claude)".into())];
        for (path, script) in scan_launcher_scripts("CLAUDE_CONFIG_DIR") {
            candidates.push((path, format!("CLAUDE_CONFIG_DIR in launcher script {script}")));
        }
        for dir in sibling_dirs(&home, ".claude-") {
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            candidates.push((dir, format!("Side-by-side home ~/{name}")));
        }
        for dir in sibling_dirs(&home.join(".config"), "claude") {
            let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            candidates.push((dir, format!("~/.config/{name}")));
        }

        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for (dir, hint) in candidates {
            if !seen.insert(util::path_key(&dir)) || !dir.is_dir() {
                continue;
            }
            let is_default = self.is_default(&dir);
            let ident = self.read_identity(&dir).await;
            let looks_like_home = if is_default {
                ident.logged_in || read_oauth_account(&dir, true).is_some()
            } else {
                ident.logged_in || [CREDENTIALS_FILE, ".claude.json", "settings.json", "projects"].iter().any(|f| dir.join(f).exists())
            };
            if !looks_like_home {
                continue;
            }
            out.push(DetectedAccount {
                provider_id: ProviderId::Claude,
                home_dir: dir.to_string_lossy().to_string(),
                is_default_home: is_default,
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
        let is_default = self.is_default(home);
        let file_creds = read_file_creds(home);
        let mut logged_in = file_creds.is_some();
        if !logged_in && cfg!(target_os = "macos") {
            for service in keychain_services(home, is_default) {
                if keychain_exists(&service).await {
                    logged_in = true;
                    break;
                }
            }
        }
        let account = read_oauth_account(home, is_default);
        identity_from(file_creds.as_ref(), account.as_ref(), logged_in)
    }

    fn shareable_paths(&self) -> Vec<(ShareKind, Vec<&'static str>)> {
        vec![
            (ShareKind::Skills, vec!["skills"]),
            (ShareKind::Plugins, vec!["plugins"]),
            (ShareKind::Agents, vec!["agents", "commands"]),
            (ShareKind::Instructions, vec!["CLAUDE.md"]),
            (ShareKind::Settings, vec!["settings.json"]),
            // MCP servers live in .claude.json next to per-account OAuth state: never linked.
            (ShareKind::Mcp, vec![]),
            (ShareKind::History, vec!["projects", "todos", "file-history", "plans"]),
        ]
    }

    fn credential_paths(&self) -> Vec<&'static str> {
        vec![CREDENTIALS_FILE, ".claude.json"]
    }

    /// The OAuth credentials (from the Keychain on macOS) plus the account details from `.claude.json`.
    /// Only `oauthAccount` is taken from `.claude.json`: the rest (projects, MCP servers) stays here.
    async fn export_sign_in(&self, home: &Path, is_default: bool) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut creds = None;
        if cfg!(target_os = "macos") {
            for service in keychain_services(home, is_default) {
                if let Some(v) = keychain_read(&service).await.filter(|v| v.get("claudeAiOauth").is_some()) {
                    creds = Some(v);
                    break;
                }
            }
        }
        let creds = creds.or_else(|| util::read_json(&home.join(CREDENTIALS_FILE)).filter(|v| v.get("claudeAiOauth").is_some()));
        if let Some(c) = creds {
            out.push((CREDENTIALS_FILE.to_string(), c.to_string().into_bytes()));
        }
        if let Some(account) = read_oauth_account(home, is_default) {
            out.push((".claude.json".to_string(), json!({ "oauthAccount": account }).to_string().into_bytes()));
        }
        out
    }

    async fn import_sign_in(&self, home: &Path, is_default: bool, files: &[(String, Vec<u8>)]) -> anyhow::Result<usize> {
        let mut written = 0;
        for (rel, bytes) in files {
            match rel.as_str() {
                CREDENTIALS_FILE if cfg!(target_os = "macos") => {
                    // Claude Code on macOS reads its login from the Keychain, under a name derived from the folder.
                    let service = keychain_services(home, is_default).remove(0);
                    if !keychain_exists(&service).await && keychain_write(&service, bytes).await? {
                        written += 1;
                    }
                }
                CREDENTIALS_FILE => {
                    let path = home.join(CREDENTIALS_FILE);
                    if !path.exists() {
                        util::write_atomic(&path, bytes)?;
                        written += 1;
                    }
                }
                ".claude.json" => {
                    // The default home keeps it at ~/.claude.json. Only written when there is none yet.
                    let path = metadata_paths(home, is_default).remove(0);
                    if !path.exists() {
                        util::write_atomic(&path, bytes)?;
                        written += 1;
                    }
                }
                _ => {}
            }
        }
        Ok(written)
    }

    async fn init_home(&self, home: &Path) -> anyhow::Result<()> {
        std::fs::create_dir_all(home)?;
        Ok(())
    }

    async fn cli_command(&self, account: &Account, resume: Resume, extra_args: &[String]) -> anyhow::Result<CommandSpec> {
        let mut args = Vec::new();
        match resume {
            Resume::No => {}
            Resume::Last => args.push("--continue".to_string()),
            Resume::Session(id) if valid_session_id(&id) => args.extend(["--resume".to_string(), id]),
            Resume::Session(_) => args.push("--continue".to_string()),
        }
        args.extend(extra_args.iter().cloned());
        Ok(CommandSpec { program: self.program(), args, env: self.cli_env(account), cwd: None, note: None })
    }

    fn resume_candidates(&self, account: &Account, cwd: &Path, wanted: Option<&str>) -> Vec<String> {
        let home = Path::new(&account.home_dir);
        // Without shared history another account's conversation isn't in this home to reopen.
        let wanted = wanted.filter(|id| transcript_exists(home, id)).map(str::to_string);
        let own = sessions_in(home, cwd).into_iter().filter(|id| wanted.as_ref() != Some(id));
        wanted.clone().into_iter().chain(own).collect()
    }

    fn open_sessions(&self, account: &Account) -> Vec<(u32, String)> {
        live_sessions(Path::new(&account.home_dir)).into_iter().filter(process_is_running).map(|s| (s.pid, s.session_id)).collect()
    }

    async fn login_command(&self, account: &Account) -> anyhow::Result<CommandSpec> {
        let mut args = vec!["auth".to_string(), "login".to_string(), "--claudeai".to_string()];
        if let Some(email) = account.email.as_deref().filter(|e| !e.is_empty()) {
            args.push("--email".into());
            args.push(email.to_string());
        }
        Ok(CommandSpec { program: self.program(), args, env: self.cli_env(account), cwd: None, note: None })
    }

    async fn desktop_command(&self, account: &Account) -> anyhow::Result<Option<CommandSpec>> {
        let Some(app) = find_desktop() else { return Ok(None) };
        let home = Path::new(&account.home_dir);
        let is_default = account.is_default_home || self.is_default(home);
        // The account the normal Claude Desktop is signed in to opens there, already signed in.
        let uuid = read_oauth_account(home, is_default).and_then(|o| str_field(&o, "accountUuid").map(str::to_string));
        let signed_in_main = uuid.is_some() && uuid == desktop_main_account();
        let per_account = !is_default && !signed_in_main;
        let mut app_args = Vec::new();
        let mut note = None;
        if per_account {
            // Electron moves all app data (login, config, MCP) into this dir, so the sign-in stays with the profile.
            let data_dir = desktop_data_dir(account, home, &app);
            let first_time = std::fs::read_dir(&data_dir).map_or(true, |mut d| d.next().is_none());
            std::fs::create_dir_all(&data_dir)?;
            app_args.push(format!("--user-data-dir={}", data_dir.display()));
            if first_time {
                let who = account.email.as_deref().map(|e| format!(" as {e}")).unwrap_or_default();
                note = Some(format!("First time for this profile: sign in to Claude Desktop{who} in the new window. It stays signed in from then on."));
            }
        }
        let app = app.to_string_lossy().to_string();
        let spec = if cfg!(target_os = "macos") {
            let mut args = if per_account { vec!["-n".to_string()] } else { vec![] };
            args.extend(["-a".to_string(), app]);
            if !app_args.is_empty() {
                args.push("--args".into());
                args.extend(app_args);
            }
            CommandSpec { program: "open".into(), args, note, ..Default::default() }
        } else {
            CommandSpec { program: app, args: app_args, note, ..Default::default() }
        };
        Ok(Some(spec))
    }

    async fn fetch_usage(&self, account: &Account, live: bool) -> UsageSnapshot {
        let home = PathBuf::from(&account.home_dir);
        let is_default = account.is_default_home || self.is_default(&home);
        let oauth_account = read_oauth_account(&home, is_default);

        if !live {
            let ident = identity_from(read_file_creds(&home).as_ref(), oauth_account.as_ref(), true);
            return self.no_data(account, &ident, FetchError::other("Live usage is off. Claude Code keeps no local copy of its limits."));
        }

        let Some(mut creds) = read_creds(&home, is_default).await else {
            let ident = identity_from(None, oauth_account.as_ref(), false);
            return self.no_data(account, &ident, FetchError::auth("Not signed in. Use Sign in to log this account in."));
        };
        let ident = identity_from(Some(&creds), oauth_account.as_ref(), true);

        let mut refreshed = false;
        if creds.needs_refresh(now_ms()) {
            let renewed = if creds.source == CredSource::Keychain {
                self.renew_keychain_login(account, &home, is_default).await
            } else {
                refresh_file_creds(&home, &creds).await.map_err(|e| match e {
                    RefreshError::InvalidGrant => FetchError::auth(LOGIN_INVALID),
                    RefreshError::Other(e) => refresh_failed(e),
                })
            };
            match renewed {
                Ok(c) => {
                    creds = c;
                    refreshed = true;
                }
                Err(e) => {
                    let keychain = creds.source == CredSource::Keychain;
                    let mut s = self.no_data(account, &ident, e);
                    if keychain {
                        s.stale = Some(true);
                    }
                    return s;
                }
            }
        }

        let ua = self.user_agent().await;
        let mut token = creds.access_token.clone();
        let mut result = fetch_oauth_json(USAGE_URL, &token, &ua).await;
        // A token revoked before its expiry: one refresh attempt, file credentials only.
        if matches!(result, Err(ApiError::Unauthorized)) && !refreshed && creds.source == CredSource::File {
            match refresh_file_creds(&home, &creds).await {
                Ok(c) => {
                    token = c.access_token;
                    result = fetch_oauth_json(USAGE_URL, &token, &ua).await;
                }
                Err(RefreshError::InvalidGrant) => return self.no_data(account, &ident, FetchError::auth(LOGIN_INVALID)),
                Err(RefreshError::Other(_)) => {}
            }
        }

        match result {
            Ok(v) => {
                let (windows, credits) = parse_usage(&v);
                let mut s = self.snapshot(account, UsageSource::Api, &ident);
                if windows.is_empty() && credits.is_none() {
                    FetchError::other("The usage API returned no limits for this account.").apply(&mut s);
                }
                s.windows = windows;
                s.credits = credits;
                s
            }
            Err(ApiError::Unauthorized) => self.no_data(account, &ident, FetchError::auth("Sign in again: Claude rejected the saved login.")),
            Err(ApiError::MissingScope) => self.no_data(
                account,
                &ident,
                FetchError::auth("This login cannot read usage (missing user:profile scope). Sign in again with `claude auth login`."),
            ),
            Err(ApiError::OrgBlocked) => {
                let reason = match known_blocked_reason(account) {
                    Some(r) => r,
                    None => blocked_reason(fetch_oauth_json(PROFILE_URL, &token, &ua).await.ok().as_ref()),
                };
                self.no_data(account, &ident, FetchError::blocked(reason))
            }
            Err(ApiError::RateLimited) => {
                let mut s = self.no_data(account, &ident, FetchError::other("Claude's usage API is rate-limiting requests; will retry later."));
                s.stale = Some(true);
                s
            }
            Err(ApiError::Http(code)) => self.no_data(account, &ident, FetchError::other(format!("Claude's usage API returned HTTP {code}."))),
            Err(ApiError::Network(e)) => self.no_data(account, &ident, FetchError::network(format!("Could not reach Claude's usage API ({e})."))),
            Err(ApiError::Parse) => self.no_data(account, &ident, FetchError::other("Claude's usage API returned an unexpected response.")),
        }
    }

    async fn scan_activity(&self, account: &Account, since_days: u32) -> Vec<DailyActivity> {
        let id = account.id.clone();
        let home = PathBuf::from(&account.home_dir);
        tokio::task::spawn_blocking(move || scan_projects(&id, &home, since_days)).await.unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn history_line(project: &str, session: &str) -> String {
        json!({"display": "hi", "pastedContents": {}, "timestamp": 1, "project": project, "sessionId": session}).to_string()
    }

    #[test]
    fn resume_offers_this_homes_conversations_in_the_folder_newest_first() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let lines = [
            history_line("/work", "old"),
            history_line("/work", "gone"),
            history_line("/elsewhere", "other"),
            history_line("/work", "new"),
            history_line("/work", "gone"),
        ];
        std::fs::write(home.join("history.jsonl"), lines.join("\n")).unwrap();
        std::fs::create_dir_all(home.join("projects/-work")).unwrap();
        std::fs::create_dir_all(home.join("projects/-elsewhere")).unwrap();
        for f in ["-work/old", "-work/new", "-elsewhere/other"] {
            std::fs::write(home.join(format!("projects/{f}.jsonl")), "{}").unwrap();
        }
        let a = ClaudeAdapter::new();
        let acc = Account { home_dir: home.to_string_lossy().into(), ..crate::recommend::tests::account("a", "max") };

        // "gone" is newest but its transcript was deleted, so `--resume gone` would fail.
        assert_eq!(a.resume_candidates(&acc, Path::new("/work"), None), ["new", "old"]);
        // A conversation asked for comes first, but only when this home has it.
        assert_eq!(a.resume_candidates(&acc, Path::new("/work"), Some("old")), ["old", "new"]);
        assert_eq!(a.resume_candidates(&acc, Path::new("/work"), Some("other")), ["other", "new", "old"]);
        assert_eq!(a.resume_candidates(&acc, Path::new("/work"), Some("not-here")), ["new", "old"]);
        assert!(a.resume_candidates(&acc, Path::new("/nowhere"), None).is_empty());
        let missing = Account { home_dir: home.join("missing").to_string_lossy().into(), ..acc };
        assert!(a.resume_candidates(&missing, Path::new("/work"), None).is_empty());
    }

    #[test]
    fn resume_by_id_uses_the_resume_flag() {
        let a = ClaudeAdapter::new();
        let acc = crate::recommend::tests::account("a", "max");
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        let args = |r: Resume| rt.block_on(a.cli_command(&acc, r, &[])).unwrap().args;
        assert_eq!(args(Resume::Session("26a8e785-9c2b-401e-bb76-656b369396be".into())), ["--resume", "26a8e785-9c2b-401e-bb76-656b369396be"]);
        assert_eq!(args(Resume::Last), ["--continue"]);
        assert!(args(Resume::No).is_empty());
        // An id from a damaged file never becomes a flag.
        assert_eq!(args(Resume::Session("--dangerously-skip-permissions".into())), ["--continue"]);
        assert_eq!(args(Resume::Session("a b".into())), ["--continue"]);
    }

    #[cfg(unix)]
    #[test]
    fn a_conversation_counts_as_open_only_while_its_process_runs() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        std::fs::create_dir_all(home.join("sessions")).unwrap();
        let me = std::process::id();
        let started = process_start(me).unwrap();
        let file = |pid: u32, v: Value| std::fs::write(home.join(format!("sessions/{pid}.json")), v.to_string()).unwrap();
        let a = ClaudeAdapter::new();
        let acc = Account { home_dir: home.to_string_lossy().into(), ..crate::recommend::tests::account("a", "max") };

        // Left by a killed process; and this test's pid in a file from a process that started earlier.
        file(999_999, json!({"pid": 999_999, "sessionId": "dead", "procStart": started}));
        file(me, json!({"pid": me, "sessionId": "reused", "procStart": "Thu Jan  1 00:00:00 1970"}));
        assert!(a.open_sessions(&acc).is_empty());
        // No start time to tell a reused pid apart: not counted as open.
        file(me, json!({"pid": me, "sessionId": "unknown"}));
        assert!(a.open_sessions(&acc).is_empty());
        // This test's own process, really running with that conversation.
        file(me, json!({"pid": me, "sessionId": "s", "procStart": started}));
        assert_eq!(a.open_sessions(&acc), [(me, "s".to_string())]);
    }

    #[cfg(unix)]
    #[test]
    fn process_start_reads_the_way_the_cli_writes_it_whatever_the_locale() {
        // The CLI writes "Wed Oct 21 21:45:58 2026"; other locales print "Wed 21 Oct ..." or
        // translate the names. The helper must not depend on the app's own locale.
        let started = process_start(std::process::id()).unwrap();
        let words: Vec<&str> = started.split_whitespace().collect();
        assert_eq!(words.len(), 5, "{started}");
        assert!(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].contains(&words[0]), "{started}");
        assert!(["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"].contains(&words[1]), "{started}");
    }

    #[cfg(unix)]
    #[test]
    fn a_renewal_runs_the_cli_in_the_home_with_its_env_and_reads_the_login_it_leaves() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        // A stand-in CLI that renews the login the way the real one leaves it: a fresh token on disk.
        let fake = dir.path().join("claude");
        let fresh = now_ms() + 8 * 3_600_000;
        let script = format!(
            "#!/bin/sh\n[ \"$1 $2\" = \"mcp list\" ] || exit 9\nprintf '%s %s' \"$PWD\" \"$CLAUDE_CONFIG_DIR\" > ran\nprintf '{{\"claudeAiOauth\":{{\"accessToken\":\"a2\",\"refreshToken\":\"r2\",\"expiresAt\":{fresh}}}}}' > .credentials.json\n"
        );
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = HashMap::from([("CLAUDE_CONFIG_DIR".to_string(), Some(home.to_string_lossy().into_owned())), ("CLAUDECODE".to_string(), None)]);

        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let creds = rt.block_on(renew_with_cli(fake.to_string_lossy().into_owned(), env, home.clone(), false)).unwrap();
        assert_eq!(creds.access_token, "a2");
        let real = home.canonicalize().unwrap();
        assert_eq!(std::fs::read_to_string(home.join("ran")).unwrap(), format!("{} {}", real.display(), home.display()));
    }

    #[test]
    fn process_start_times_compare_word_by_word() {
        assert!(same_words("Wed Oct  1 21:45:58 2026\n", "Wed Oct 1 21:45:58 2026"));
        assert!(!same_words("Wed Oct 1 21:45:58 2026", "Wed Oct 1 21:45:59 2026"));
    }

    #[test]
    fn an_organization_block_is_not_a_sign_in_problem() {
        // As the usage API answered for such an account.
        let body = r#"{"type":"error","error":{"type":"permission_error","message":"OAuth authentication is currently not allowed for this organization.","details":{"error_visibility":"user_facing","error_code":"oauth_not_allowed_for_organization"}}}"#;
        assert_eq!(api_error(403, body), ApiError::OrgBlocked);
        assert_eq!(api_error(403, r#"{"error":{"message":"missing scope user:profile"}}"#), ApiError::MissingScope);
        assert_eq!(api_error(403, "{}"), ApiError::Unauthorized);
        assert_eq!(api_error(429, ""), ApiError::RateLimited);
    }

    #[test]
    fn keychain_renewal_waits_after_a_renewal_that_did_not_help() {
        let now = 10 * RENEW_RETRY_MS;
        assert!(renew_due(None, now));
        assert!(!renew_due(Some(now - RENEW_RETRY_MS + 1), now));
        assert!(renew_due(Some(now - RENEW_RETRY_MS), now));
    }

    #[test]
    fn a_renewal_is_judged_by_the_login_it_leaves() {
        let now = 1_000_000_000_000;
        let creds = |expires_in_ms: i64| {
            parse_creds(&json!({"claudeAiOauth": {"accessToken": "a", "refreshToken": "r", "expiresAt": now + expires_in_ms}}), CredSource::Keychain)
        };
        assert!(renew_outcome(creds(8 * 3_600_000), false, now).is_ok());
        let still_expired = renew_outcome(creds(-1000), false, now).unwrap_err();
        assert_eq!(still_expired.kind, ErrorKind::Expired);
        assert_eq!(still_expired.message, LOGIN_RENEW_LATER);
        // Claude Code drops a login the server refused to renew: that one is signed out.
        assert_eq!(renew_outcome(None, false, now).unwrap_err().kind, ErrorKind::Auth);
        // Still there but unreadable (a locked Keychain): not signed out.
        assert_eq!(renew_outcome(None, true, now).unwrap_err().kind, ErrorKind::Expired);
    }

    #[test]
    fn transient_refresh_failures_do_not_read_as_signed_out() {
        let offline = refresh_failed(FetchError::network(REFRESH_OFFLINE));
        assert_eq!(offline.kind, ErrorKind::Network);
        assert_eq!(offline.message, "Could not renew the Claude session: could not reach Anthropic (network error).");
        assert_eq!(refresh_failed(FetchError::other(REFRESH_BAD_RESPONSE)).kind, ErrorKind::Other);
        assert_eq!(refresh_failed(refresh_http_error(503)).kind, ErrorKind::Other);
        assert_eq!(refresh_failed(refresh_http_error(401)).kind, ErrorKind::Auth);
        assert_eq!(refresh_failed(refresh_http_error(403)).kind, ErrorKind::Auth);
    }

    const CREDS: &str = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-AAA","refreshToken":"sk-ant-ort01-BBB",
        "expiresAt":1792520000000,"refreshTokenExpiresAt":1800000000000,
        "scopes":["user:inference","user:profile","user:sessions:claude_code"],
        "subscriptionType":"max","subscriptionTypeSource":"sign_in","rateLimitTier":"default_claude_max_20x"},
        "mcpOAuth":{"x":{"accessToken":"keep-me"}}}"#;

    #[test]
    fn parses_credentials() {
        let v: Value = serde_json::from_str(CREDS).unwrap();
        let c = parse_creds(&v, CredSource::File).unwrap();
        assert_eq!(c.access_token, "sk-ant-oat01-AAA");
        assert_eq!(c.expires_at_ms, Some(1792520000000));
        assert_eq!(c.scopes.len(), 3);
        assert_eq!(plan_label(c.subscription_type.as_deref(), c.rate_limit_tier.as_deref()).as_deref(), Some("Max 20x"));
        assert!(c.needs_refresh(1792520000000 - 30_000));
        assert!(!c.needs_refresh(1792520000000 - 120_000));
        assert!(parse_creds(&json!({"claudeAiOauth": {"accessToken": ""}}), CredSource::File).is_none());
        assert!(parse_creds(&json!({}), CredSource::File).is_none());
    }

    /// As `/api/oauth/profile` answered for an account whose Max subscription was canceled.
    const CANCELED_PROFILE: &str = r#"{"account":{"has_claude_max":false,"has_claude_pro":false},
        "organization":{"organization_type":"claude_free","billing_type":"none","rate_limit_tier":"default_claude_ai",
        "seat_tier":null,"has_extra_usage_enabled":false,"subscription_status":"canceled"},
        "application":{"name":"Claude Code","slug":"claude-code"},"enabled_plugins":[]}"#;

    #[test]
    fn blocked_reason_names_a_canceled_subscription() {
        let canceled: Value = serde_json::from_str(CANCELED_PROFILE).unwrap();
        assert_eq!(
            blocked_reason(Some(&canceled)),
            "This account's subscription was canceled, so it's on the Free plan, which doesn't include Claude Code. Resubscribe at claude.ai, or remove it from Agent Hub."
        );
        let org = |t: &str, status: Option<&str>| json!({"organization": {"organization_type": t, "subscription_status": status}});
        assert_eq!(
            blocked_reason(Some(&org("claude_free", None))),
            "This account is on the Free plan, which doesn't include Claude Code. Subscribe at claude.ai, or remove it from Agent Hub."
        );
        // A personal plan never points at an admin.
        assert_eq!(
            blocked_reason(Some(&org("claude_max", Some("past_due")))),
            "This account's subscription isn't active (past due), so it can't use Claude Code. Check it at claude.ai, or remove it from Agent Hub."
        );
        assert_eq!(
            blocked_reason(Some(&org("claude_pro", Some("canceled")))),
            "This account's subscription was canceled, so it can't use Claude Code. Resubscribe at claude.ai, or remove it from Agent Hub."
        );
        assert_eq!(blocked_reason(Some(&org("claude_pro", Some("active")))), BLOCKED_UNKNOWN);
        // A team or company organization that turned Claude Code off: the admin decides.
        assert_eq!(blocked_reason(Some(&org("claude_team", Some("active")))), ORG_BLOCKED);
        assert!(!ORG_BLOCKED.contains("Sign"), "no sign-in advice next to the Sign in again button");
        // The profile couldn't be read.
        assert_eq!(blocked_reason(None), BLOCKED_UNKNOWN);
    }

    /// A blocked account keeps the reason it has instead of asking the profile on every poll, so a
    /// failed profile call never swaps a known reason for a vaguer one. An unknown one is asked again.
    #[test]
    fn a_known_blocked_reason_is_reused() {
        let mut a = crate::recommend::tests::account("a", "Max");
        a.status_message = Some("This account's subscription was canceled.".into());
        assert_eq!(known_blocked_reason(&a), None, "not blocked yet");
        a.status = AccountStatus::Blocked;
        assert_eq!(known_blocked_reason(&a).as_deref(), Some("This account's subscription was canceled."));
        a.status_message = Some(BLOCKED_UNKNOWN.into());
        assert_eq!(known_blocked_reason(&a), None);
    }

    #[test]
    fn plan_labels() {
        assert_eq!(plan_label(Some("pro"), Some("default_claude_ai")).as_deref(), Some("Pro"));
        assert_eq!(plan_label(Some("max"), Some("default_claude_max_5x")).as_deref(), Some("Max 5x"));
        assert_eq!(plan_label(Some("claude_max"), Some("default_claude_max_20x")).as_deref(), Some("Max 20x"));
        assert_eq!(plan_label(Some("team"), None).as_deref(), Some("Team"));
        assert_eq!(plan_label(Some("enterprise"), None).as_deref(), Some("Enterprise"));
        assert_eq!(plan_label(None, Some("default_claude_max_5x")).as_deref(), Some("Max 5x"));
        assert_eq!(plan_label(None, Some("default_claude_ai")).as_deref(), Some("Pro"));
        assert_eq!(plan_label(None, None), None);
    }

    #[test]
    fn identity_prefers_creds_then_oauth_account() {
        let acct = json!({"emailAddress": "a@b.c", "organizationType": "claude_pro", "organizationRateLimitTier": "default_claude_ai"});
        let id = identity_from(None, Some(&acct), false);
        assert_eq!(id.email.as_deref(), Some("a@b.c"));
        assert_eq!(id.plan.as_deref(), Some("Pro"));
        let c = parse_creds(&serde_json::from_str(CREDS).unwrap(), CredSource::File).unwrap();
        assert_eq!(identity_from(Some(&c), Some(&acct), true).plan.as_deref(), Some("Max 20x"));
    }

    #[test]
    fn parses_usage_response() {
        let v: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":42.5,"resets_at":"2026-10-20T19:00:00.123456+00:00"},
                "seven_day":{"utilization":12,"resets_at":"2026-10-24T07:00:00+00:00"},
                "seven_day_oauth_apps":null,"seven_day_opus":null,
                "seven_day_sonnet":{"utilization":3.0,"resets_at":null},
                "iguana_necktie":null,"something_new":{"utilization":1},
                "extra_usage":{"is_enabled":true,"monthly_limit":5000,"used_credits":1234.0,"utilization":24.68,"currency":"USD"}}"#,
        )
        .unwrap();
        let (w, credits) = parse_usage(&v);
        assert_eq!(w.iter().map(|w| w.id.as_str()).collect::<Vec<_>>(), ["five_hour", "seven_day", "seven_day_sonnet"]);
        assert_eq!(w[0].used_percent, 42.5);
        assert_eq!(w[0].window_minutes, Some(300));
        assert_eq!(w[0].label, "5-hour session");
        assert!(w[0].resets_at.as_deref().unwrap().starts_with("2026-10-20T19:00:00"));
        assert_eq!(w[1].window_minutes, Some(10080));
        assert_eq!(w[2].resets_at, None);
        let c = credits.unwrap();
        assert!((c.used - 12.34).abs() < 1e-9);
        assert_eq!(c.limit, Some(50.0));
        assert_eq!(c.unit, "USD");
    }

    #[test]
    fn usage_extra_disabled_or_empty() {
        let v = json!({"five_hour": null, "extra_usage": {"is_enabled": false, "monthly_limit": null, "used_credits": null}});
        let (w, c) = parse_usage(&v);
        assert!(w.is_empty());
        assert!(c.is_none());
        assert!(parse_usage(&json!("garbage")).0.is_empty());
    }

    #[test]
    fn refresh_preserves_other_fields() {
        let mut file: Value = serde_json::from_str(CREDS).unwrap();
        let resp = json!({"access_token": "new-access", "refresh_token": "new-refresh", "expires_in": 28800,
                          "refresh_token_expires_in": 2592000, "scope": "user:inference user:profile", "token_type": "Bearer"});
        apply_refresh(&mut file, &resp, 1_000_000).unwrap();
        let o = &file["claudeAiOauth"];
        assert_eq!(o["accessToken"], "new-access");
        assert_eq!(o["refreshToken"], "new-refresh");
        assert_eq!(o["expiresAt"], 1_000_000 + 28_800_000);
        assert_eq!(o["refreshTokenExpiresAt"], 1_000_000i64 + 2_592_000_000);
        assert_eq!(o["scopes"], json!(["user:inference", "user:profile"]));
        assert_eq!(o["subscriptionType"], "max");
        assert_eq!(o["rateLimitTier"], "default_claude_max_20x");
        assert_eq!(file["mcpOAuth"]["x"]["accessToken"], "keep-me");

        // Refresh token omitted → keep the old one.
        let mut file: Value = serde_json::from_str(CREDS).unwrap();
        apply_refresh(&mut file, &json!({"access_token": "x", "expires_in": 60}), 0).unwrap();
        assert_eq!(file["claudeAiOauth"]["refreshToken"], "sk-ant-ort01-BBB");
        assert!(apply_refresh(&mut file, &json!({"error": "invalid_grant"}), 0).is_none());
    }

    #[test]
    fn detects_invalid_grant() {
        assert!(is_invalid_grant(400, r#"{"error":"invalid_grant","error_description":"Refresh token not found or invalid"}"#));
        assert!(is_invalid_grant(400, r#"{"error":{"type":"invalid_grant"}}"#));
        assert!(!is_invalid_grant(500, "oops"));
    }

    #[test]
    fn keychain_names() {
        let def = keychain_services(Path::new("/Users/me/.claude"), true);
        assert_eq!(def, vec!["Claude Code-credentials".to_string()]);
        let custom = keychain_services(Path::new("/Users/me/.claude-work"), false);
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0], format!("Claude Code-credentials-{}", &hex::encode(Sha256::digest(b"/Users/me/.claude-work"))[..8]));
        assert_eq!(keychain_services(Path::new("/x/y/"), false).len(), 2);
        let secret = r#"{"claudeAiOauth":{"accessToken":"a","refreshToken":"b"}}"#;
        assert!(parse_keychain_secret(secret).is_some());
        assert!(parse_keychain_secret(&hex::encode(secret)).is_some());
        assert!(parse_keychain_secret("not json").is_none());
    }

    fn line(ts: &str, msg_id: &str, req: &str, session: &str, model: &str, inp: u64, out: u64) -> Value {
        json!({"parentUuid": "p", "isSidechain": false, "type": "assistant", "timestamp": ts, "sessionId": session,
               "requestId": req, "uuid": "u",
               "message": {"model": model, "id": msg_id, "type": "message", "role": "assistant", "content": [],
                           "usage": {"input_tokens": inp, "cache_creation_input_tokens": 100, "cache_read_input_tokens": 1000,
                                     "cache_creation": {"ephemeral_5m_input_tokens": 0}, "output_tokens": out, "service_tier": "standard"}}})
    }

    #[test]
    fn accumulates_and_dedupes_activity() {
        let now = chrono::Utc::now();
        let ts = now.to_rfc3339();
        let old = (now - chrono::Duration::days(40)).to_rfc3339();
        let first = first_local_date(7);
        let mut seen = HashSet::new();
        let mut days = BTreeMap::new();
        assert!(accumulate_line(&line(&ts, "msg_1", "req_1", "s1", "claude-opus-4-7", 10, 20), &first, &mut seen, &mut days));
        // Same message streamed in several content blocks → counted once.
        assert!(!accumulate_line(&line(&ts, "msg_1", "req_1", "s1", "claude-opus-4-7", 10, 20), &first, &mut seen, &mut days));
        assert!(accumulate_line(&line(&ts, "msg_2", "req_2", "s2", "claude-sonnet-4-6", 1, 2), &first, &mut seen, &mut days));
        assert!(!accumulate_line(&line(&old, "msg_3", "req_3", "s3", "claude-sonnet-4-6", 1, 2), &first, &mut seen, &mut days));
        assert!(!accumulate_line(&json!({"type": "user", "timestamp": ts, "message": {"content": "hi"}}), &first, &mut seen, &mut days));
        assert!(!accumulate_line(
            &line(&ts, "msg_4", "r4", "s1", "<synthetic>", 0, 0)
                .as_object()
                .map(|o| {
                    let mut o = o.clone();
                    o.remove("timestamp");
                    Value::Object(o)
                })
                .unwrap(),
            &first,
            &mut seen,
            &mut days
        ));

        let out = finish_days("acc", days);
        assert_eq!(out.len(), 1);
        let d = &out[0];
        assert_eq!(d.account_id, "acc");
        assert_eq!(d.input_tokens, 11);
        assert_eq!(d.output_tokens, 22);
        assert_eq!(d.cache_tokens, 2200);
        assert_eq!(d.messages, 2);
        assert_eq!(d.sessions, 2);
        assert_eq!(d.models.get("claude-opus-4-7"), Some(&30));
    }

    #[test]
    fn scans_project_files() {
        let dir = tempfile::tempdir().unwrap();
        let proj = dir.path().join("projects").join("C--work").join("sub");
        std::fs::create_dir_all(&proj).unwrap();
        let ts = chrono::Utc::now().to_rfc3339();
        let body = [
            line(&ts, "m1", "r1", "s1", "claude-opus-4-7", 5, 6).to_string(),
            "{not json".to_string(),
            line(&ts, "m1", "r1", "s1", "claude-opus-4-7", 5, 6).to_string(),
            json!({"type": "summary", "summary": "x"}).to_string(),
        ]
        .join("\n");
        std::fs::write(proj.join("a.jsonl"), &body).unwrap();
        std::fs::write(dir.path().join("projects").join("C--work").join("b.jsonl"), line(&ts, "m2", "r2", "s2", "claude-opus-4-7", 1, 1).to_string()).unwrap();
        let out = scan_projects("acc", dir.path(), 30);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].messages, 2);
        assert_eq!(out[0].input_tokens, 6);
        assert_eq!(out[0].sessions, 2);
    }

    #[test]
    fn reads_identity_from_home_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".credentials.json"), CREDS).unwrap();
        std::fs::write(
            dir.path().join(".claude.json"),
            r#"{"numStartups":3,"oauthAccount":{"accountUuid":"u","emailAddress":"me@example.com","organizationType":"claude_max"}}"#,
        )
        .unwrap();
        let adapter = ClaudeAdapter::new();
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let id = rt.block_on(adapter.read_identity(dir.path()));
        assert!(id.logged_in);
        assert_eq!(id.email.as_deref(), Some("me@example.com"));
        assert_eq!(id.plan.as_deref(), Some("Max 20x"));
    }

    #[test]
    fn cli_env_for_custom_and_default_homes() {
        let adapter = ClaudeAdapter::new();
        let mut acc: Account = serde_json::from_value(json!({
            "id": "a1", "providerId": "claude", "label": "Work", "email": null, "plan": null,
            "homeDir": "/tmp/claude-work", "isDefaultHome": false, "sharedFrom": null,
            "sharing": {"skills": true, "plugins": true, "agents": true, "instructions": true, "settings": false, "mcp": true, "history": true},
            "color": "#fff", "notes": null, "status": "ok", "statusMessage": null, "createdAt": "2026-10-20T00:00:00Z",
            "lastLaunchedAt": null, "monthlyCost": null, "pinned": null}))
        .unwrap();
        let env = adapter.cli_env(&acc);
        assert_eq!(env["CLAUDE_CONFIG_DIR"].as_deref(), Some("/tmp/claude-work"));
        assert_eq!(env["ANTHROPIC_API_KEY"], None);
        acc.is_default_home = true;
        assert_eq!(adapter.cli_env(&acc)["CLAUDE_CONFIG_DIR"], None);
    }

    #[test]
    fn cli_env_clears_inherited_session_markers() {
        let adapter = ClaudeAdapter::new();
        let acc: Account = serde_json::from_value(json!({
            "id": "a1", "providerId": "claude", "label": "Work", "homeDir": "/tmp/claude-work", "isDefaultHome": false,
            "sharing": {"skills": true, "plugins": true, "agents": true, "instructions": true, "settings": false, "mcp": true, "history": true},
            "color": "#fff", "status": "ok", "createdAt": "2026-10-20T00:00:00Z"}))
        .unwrap();
        std::env::set_var("CLAUDE_CODE_CHILD_SESSION", "1");
        let env = adapter.cli_env(&acc);
        std::env::remove_var("CLAUDE_CODE_CHILD_SESSION");
        assert!(env.contains_key("CLAUDE_CODE_CHILD_SESSION") && env["CLAUDE_CODE_CHILD_SESSION"].is_none());
    }

    // Store builds and their AppData redirection exist only on Windows.
    #[cfg(windows)]
    #[test]
    fn desktop_data_stays_out_of_redirected_appdata_for_store_builds() {
        let store = Path::new(r"C:\Program Files\WindowsApps\Claude_3.1_x64__abc\app\Claude.exe");
        let plain = Path::new(r"C:\Users\me\AppData\Local\AnthropicClaude\claude.exe");
        assert!(is_store_build(store) && !is_store_build(plain));

        let acc: Account = serde_json::from_value(json!({
            "id": "acc-1", "providerId": "claude", "label": "Work", "homeDir": "x", "isDefaultHome": false,
            "sharing": {"skills": true, "plugins": true, "agents": true, "instructions": true, "settings": false, "mcp": true, "history": true},
            "color": "#fff", "status": "ok", "createdAt": "2026-10-20T00:00:00Z"}))
        .unwrap();
        let Some(local) = dirs::data_local_dir() else { return };
        std::fs::create_dir_all(&local).unwrap();
        let root = tempfile::tempdir_in(&local).unwrap();
        let home = root.path().join("profile");
        std::fs::create_dir_all(&home).unwrap();

        let moved = util::home().join(".switchyard").join("claude-desktop").join("acc-1");
        assert_eq!(desktop_data_dir(&acc, &home, store), moved);
        assert_eq!(desktop_data_dir(&acc, &home, plain), home.join("desktop-data"));
        // A profile that already has desktop data keeps using it.
        std::fs::create_dir_all(home.join("desktop-data").join("Local Storage")).unwrap();
        assert_eq!(desktop_data_dir(&acc, &home, store), home.join("desktop-data"));
    }
}
