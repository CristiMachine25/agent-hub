//! Background usage poller, history, account status upkeep and the activity cache.
use crate::model::*;
use crate::providers::Identity;
use crate::store::{ActivityEntry, AppState};
use crate::{notify, util};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};
use tokio::task::JoinSet;

const FETCH_TIMEOUT: Duration = Duration::from_secs(60);
const IDENTITY_TIMEOUT: Duration = Duration::from_secs(10);
const ACTIVITY_TTL_MINUTES: i64 = 10;
/// Activity scans cover at least this many days so short and long chart ranges share one cache entry.
const ACTIVITY_MIN_DAYS: u32 = 30;

/// Start the poll loop. A poll runs right away, then every pollIntervalMinutes, or early when
/// `poll_wake` is notified.
pub fn start(app: AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        loop {
            poll(&app, &state, None).await;
            let mins = state.settings.read().await.poll_interval_minutes.max(1);
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(u64::from(mins) * 60)) => {}
                _ = state.poll_wake.notified() => {}
            }
        }
    });
}

struct Fetched {
    account_id: String,
    snapshot: UsageSnapshot,
    identity: Option<Identity>,
}

/// Fetch usage for every account (or just `only`), merge, persist, notify. Returns the latest map.
pub async fn poll(app: &AppHandle, state: &AppState, only: Option<&str>) -> HashMap<String, UsageSnapshot> {
    let _guard = state.poll_lock.lock().await;
    let accounts = state.accounts.read().await.clone();
    let live = state.settings.read().await.live_usage;

    let mut set = JoinSet::new();
    for account in accounts.iter().filter(|a| only.is_none_or(|id| a.id == id)).cloned() {
        let Ok(adapter) = state.adapter(account.provider_id) else { continue };
        set.spawn(async move {
            let snapshot = tokio::time::timeout(FETCH_TIMEOUT, adapter.fetch_usage(&account, live))
                .await
                .unwrap_or_else(|_| UsageSnapshot::failed(&account, UsageSource::None, "Timed out fetching usage"));
            // Identity is a cheap local read; only needed when the fetch did not tell us enough.
            let identity = if snapshot.error.is_some() || snapshot.email.is_none() {
                tokio::time::timeout(IDENTITY_TIMEOUT, adapter.read_identity(std::path::Path::new(&account.home_dir))).await.ok()
            } else {
                None
            };
            Fetched { account_id: account.id.clone(), snapshot, identity }
        });
    }
    let mut fetched = Vec::new();
    while let Some(res) = set.join_next().await {
        match res {
            Ok(f) => fetched.push(f),
            Err(e) => log::warn!("usage task failed: {e}"),
        }
    }

    let prev = state.latest.read().await.clone();
    let now = Utc::now();
    let mut points = Vec::new();
    {
        let mut latest = state.latest.write().await;
        latest.retain(|id, _| accounts.iter().any(|a| &a.id == id));
        for f in &fetched {
            // Merge onto the current entry, not `prev`: a manual entry saved mid-poll must survive.
            let merged = merge_snapshot(latest.get(&f.account_id), f.snapshot.clone());
            if merged.stale != Some(true) {
                if let Some(p) = history_point(&merged, now) {
                    points.push(p);
                }
            }
            latest.insert(f.account_id.clone(), merged);
        }
    }
    if let Err(e) = state.append_history(&points).await {
        log::warn!("appending usage history failed: {e:#}");
    }
    update_accounts(app, state, &fetched).await;
    publish(app, state, &prev).await
}

/// Persist latest, emit "usage-updated", run notification rules and refresh the tray.
pub async fn publish(app: &AppHandle, state: &AppState, prev: &HashMap<String, UsageSnapshot>) -> HashMap<String, UsageSnapshot> {
    if let Err(e) = state.save_latest().await {
        log::warn!("saving latest usage failed: {e:#}");
    }
    let latest = state.latest.read().await.clone();
    let _ = app.emit("usage-updated", &latest);

    let accounts = state.accounts.read().await.clone();
    let settings = state.settings.read().await.notifications.clone();
    let items = {
        let mut ns = state.notify_state.lock().await;
        notify::evaluate(prev, &latest, &accounts, &settings, &mut ns, Utc::now())
    };
    if let Err(e) = state.save_notify_state().await {
        log::warn!("saving notification state failed: {e:#}");
    }
    notify::deliver(app, state, items).await;
    crate::refresh_tray(app);
    latest
}

/// Keep the last good windows when a fetch fails or returns nothing; manual entries stay as entered.
pub fn merge_snapshot(prev: Option<&UsageSnapshot>, new: UsageSnapshot) -> UsageSnapshot {
    if !new.windows.is_empty() {
        return new;
    }
    match prev.filter(|p| !p.windows.is_empty()) {
        Some(p) if p.source == UsageSource::Manual && new.error.is_none() => p.clone(),
        Some(p) => {
            let mut m = p.clone();
            m.stale = Some(true);
            m.error = new.error;
            m.plan = new.plan.or(m.plan);
            m.email = new.email.or(m.email);
            m
        }
        None => new,
    }
}

fn history_point(s: &UsageSnapshot, now: chrono::DateTime<Utc>) -> Option<UsageHistoryPoint> {
    (!s.windows.is_empty()).then(|| UsageHistoryPoint {
        account_id: s.account_id.clone(),
        t: now.to_rfc3339(),
        windows: s.windows.iter().map(|w| (w.id.clone(), w.used_percent)).collect(),
    })
}

/// A login problem, not a connectivity one: a token refresh that could not reach the server also
/// mentions "login" or "token", but signing in again would not help.
pub fn looks_like_auth_error(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    let network = ["could not reach", "network error", "timed out", "error sending request", "connection refused", "connection reset", "dns error"];
    if network.iter().any(|k| m.contains(k)) {
        return false;
    }
    ["log in", "login", "logged in", "not logged", "sign in", "credential", "unauthorized", "401", "403", "token", "auth", "expired"]
        .iter()
        .any(|k| m.contains(k))
}

/// Status from the latest fetch and offline identity.
pub fn derive_status(snap: &UsageSnapshot, identity: Option<&Identity>) -> (AccountStatus, Option<String>) {
    if snap.error.is_none() && !snap.windows.is_empty() && snap.stale != Some(true) {
        return (AccountStatus::Ok, None);
    }
    if identity.is_some_and(|i| !i.logged_in) {
        return (AccountStatus::NeedsLogin, Some("Not logged in".into()));
    }
    match &snap.error {
        Some(err) if looks_like_auth_error(err) => {
            let status = if err.to_ascii_lowercase().contains("expired") { AccountStatus::Expired } else { AccountStatus::NeedsLogin };
            (status, Some(err.clone()))
        }
        // A transient failure with last-known numbers is still usable.
        Some(err) if !snap.windows.is_empty() => (AccountStatus::Ok, Some(err.clone())),
        Some(err) => (AccountStatus::Error, Some(err.clone())),
        None => (AccountStatus::Ok, None),
    }
}

async fn update_accounts(app: &AppHandle, state: &AppState, fetched: &[Fetched]) {
    let latest = state.latest.read().await.clone();
    let changed = {
        let mut accounts = state.accounts.write().await;
        let mut changed = false;
        for f in fetched {
            let Some(a) = accounts.iter_mut().find(|a| a.id == f.account_id) else { continue };
            changed |= apply_fetch(a, f, latest.get(&f.account_id).unwrap_or(&f.snapshot));
        }
        changed
    };
    if changed {
        if let Err(e) = state.save_accounts().await {
            log::warn!("saving accounts failed: {e:#}");
        }
        let _ = app.emit("accounts-changed", &*state.accounts.read().await);
    }
}

/// Copy one fetch's status, email and plan onto its account; true when anything changed.
/// `snap` is the merged snapshot from `latest`, so last-known windows keep a failed fetch usable.
fn apply_fetch(a: &mut Account, f: &Fetched, snap: &UsageSnapshot) -> bool {
    let (status, message) = derive_status(snap, f.identity.as_ref());
    let email = f.snapshot.email.clone().or_else(|| f.identity.as_ref().and_then(|i| i.email.clone())).or_else(|| snap.email.clone());
    let plan = f.snapshot.plan.clone().or_else(|| f.identity.as_ref().and_then(|i| i.plan.clone())).or_else(|| snap.plan.clone());
    let mut changed = set_if_changed(&mut a.status, status);
    changed |= set_if_changed(&mut a.status_message, message);
    if email.is_some() {
        changed |= set_if_changed(&mut a.email, email);
    }
    if plan.is_some() {
        changed |= set_if_changed(&mut a.plan, plan);
    }
    changed
}

fn set_if_changed<T: PartialEq>(slot: &mut T, value: T) -> bool {
    if *slot == value {
        false
    } else {
        *slot = value;
        true
    }
}

/// Record a manual usage entry for accounts without a live source.
pub async fn set_manual(app: &AppHandle, state: &AppState, account_id: &str, windows: Vec<UsageWindow>) -> anyhow::Result<UsageSnapshot> {
    let account = state.account(account_id).await.ok_or_else(|| anyhow::anyhow!("Account not found"))?;
    let windows = windows
        .into_iter()
        .map(|mut w| {
            w.used_percent = if w.used_percent.is_finite() { w.used_percent.clamp(0.0, 100.0) } else { 0.0 };
            w
        })
        .collect();
    let snap = UsageSnapshot {
        account_id: account.id.clone(),
        provider_id: account.provider_id,
        fetched_at: util::now_iso(),
        source: UsageSource::Manual,
        windows,
        plan: account.plan.clone(),
        email: account.email.clone(),
        credits: None,
        error: None,
        stale: None,
    };
    // No poll_lock: a slow poll holds it for up to a minute, and its merge keeps manual entries.
    let prev = {
        let mut latest = state.latest.write().await;
        let prev = latest.clone();
        latest.insert(account.id.clone(), snap.clone());
        prev
    };
    if let Some(p) = history_point(&snap, Utc::now()) {
        state.append_history(&[p]).await?;
    }
    publish(app, state, &prev).await;
    Ok(snap)
}

/// Daily activity from local logs, cached per account for ACTIVITY_TTL_MINUTES.
pub async fn activity(state: &AppState, account_id: Option<&str>, days: u32) -> Vec<DailyActivity> {
    let days = days.clamp(1, 365);
    let accounts: Vec<Account> = state.accounts.read().await.iter().filter(|a| account_id.is_none_or(|id| a.id == id)).cloned().collect();
    let now = Utc::now();
    let cache = state.activity.read().await.clone();

    let mut set = JoinSet::new();
    let mut result = Vec::new();
    for account in accounts {
        let fresh =
            cache.get(&account.id).filter(|e| e.days >= days && util::parse_iso(&e.scanned_at).is_some_and(|t| (now - t).num_minutes() < ACTIVITY_TTL_MINUTES));
        if let Some(e) = fresh {
            result.extend(e.items.clone());
            continue;
        }
        let Ok(adapter) = state.adapter(account.provider_id) else { continue };
        let scan_days = days.max(ACTIVITY_MIN_DAYS);
        set.spawn(async move {
            let items = adapter.scan_activity(&account, scan_days).await;
            (account.id, scan_days, items)
        });
    }
    let mut scanned = false;
    while let Some(res) = set.join_next().await {
        let Ok((id, scan_days, items)) = res else { continue };
        result.extend(items.clone());
        state.activity.write().await.insert(id, ActivityEntry { scanned_at: now.to_rfc3339(), days: scan_days, items });
        scanned = true;
    }
    if scanned {
        if let Err(e) = state.save_activity().await {
            log::warn!("saving activity cache failed: {e:#}");
        }
    }
    let first = crate::providers::common::first_local_date(days);
    result.retain(|d| d.date >= first);
    result.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.account_id.cmp(&b.account_id)));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommend::tests::{account, snap, window};

    #[test]
    fn failed_fetch_keeps_last_good_windows_as_stale() {
        let now = Utc::now();
        let good = snap("a", vec![window("five_hour", 40.0, Some(2), 300, now)]);
        let mut bad = snap("a", vec![]);
        bad.error = Some("network down".into());
        let m = merge_snapshot(Some(&good), bad);
        assert_eq!(m.windows.len(), 1);
        assert_eq!(m.stale, Some(true));
        assert_eq!(m.error.as_deref(), Some("network down"));
        let fresh = merge_snapshot(Some(&m), good.clone());
        assert_eq!(fresh.stale, None);
    }

    #[test]
    fn manual_entries_survive_empty_fetches() {
        let now = Utc::now();
        let mut manual = snap("a", vec![window("x", 40.0, None, 300, now)]);
        manual.source = UsageSource::Manual;
        let m = merge_snapshot(Some(&manual), snap("a", vec![]));
        assert_eq!(m.source, UsageSource::Manual);
        assert_eq!(m.stale, None);
    }

    #[test]
    fn status_from_errors() {
        let mut s = snap("a", vec![]);
        s.error = Some("Not logged in: run claude /login".into());
        assert_eq!(derive_status(&s, None).0, AccountStatus::NeedsLogin);
        s.error = Some("OAuth token expired".into());
        assert_eq!(derive_status(&s, None).0, AccountStatus::Expired);
        s.error = Some("connection reset".into());
        assert_eq!(derive_status(&s, None).0, AccountStatus::Error);
        let ok = snap("a", vec![window("w", 1.0, None, 300, Utc::now())]);
        assert_eq!(derive_status(&ok, Some(&Identity::default())).0, AccountStatus::Ok);
    }

    #[test]
    fn network_failures_are_not_login_problems() {
        for msg in [
            "Could not refresh the Claude login: network error while refreshing the login.",
            "Could not reach Google to refresh the Gemini login: error sending request for url (https://oauth2.googleapis.com/token)",
            "Could not reach Claude's usage API (timed out).",
            "Could not reach GitHub: error sending request for url (https://api.github.com/copilot_internal/user)",
        ] {
            assert!(!looks_like_auth_error(msg), "{msg}");
            let mut s = snap("a", vec![]);
            s.error = Some(msg.into());
            assert_eq!(derive_status(&s, None).0, AccountStatus::Error, "{msg}");
        }
        assert!(looks_like_auth_error("Sign in again: the saved Claude login is no longer valid."));
        assert!(looks_like_auth_error("Login token expired. Open Claude once to refresh the login."));
    }

    #[test]
    fn a_failed_fetch_with_last_known_windows_keeps_the_account_usable() {
        let now = Utc::now();
        let good = snap("a", vec![window("five_hour", 40.0, Some(2), 300, now)]);
        let mut failed = snap("a", vec![]);
        failed.error = Some("Could not reach Claude's usage API (timed out).".into());
        let merged = merge_snapshot(Some(&good), failed.clone());
        let f = Fetched { account_id: "a".into(), snapshot: failed, identity: None };
        let mut a = account("a", "pro");
        apply_fetch(&mut a, &f, &merged);
        assert_eq!(a.status, AccountStatus::Ok);
        assert_eq!(a.status_message.as_deref(), Some("Could not reach Claude's usage API (timed out)."));
    }
}
