//! Background usage poller, history, account status upkeep and the activity cache.
use crate::model::*;
use crate::providers::{FetchError, Identity};
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
                .unwrap_or_else(|_| FetchError::network("Timed out fetching usage").snapshot(&account, UsageSource::None));
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
/// A blocked account keeps nothing: its old numbers no longer mean anything.
pub fn merge_snapshot(prev: Option<&UsageSnapshot>, new: UsageSnapshot) -> UsageSnapshot {
    if !new.windows.is_empty() || new.error_kind == Some(ErrorKind::Blocked) {
        return new;
    }
    match prev.filter(|p| !p.windows.is_empty()) {
        Some(p) if p.source == UsageSource::Manual && new.error.is_none() => p.clone(),
        Some(p) => {
            let mut m = p.clone();
            m.stale = Some(true);
            m.error = new.error;
            m.error_kind = new.error_kind;
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

/// Status from the latest fetch and offline identity.
pub fn derive_status(snap: &UsageSnapshot, identity: Option<&Identity>) -> (AccountStatus, Option<String>) {
    if snap.error.is_none() && !snap.windows.is_empty() && snap.stale != Some(true) {
        return (AccountStatus::Ok, None);
    }
    if identity.is_some_and(|i| !i.logged_in) {
        return (AccountStatus::NeedsLogin, Some("Not logged in".into()));
    }
    let Some(err) = &snap.error else { return (AccountStatus::Ok, None) };
    // Every adapter error is typed (providers::FetchError); anything else is just an error.
    let status = match snap.error_kind.unwrap_or(ErrorKind::Other) {
        ErrorKind::Auth => AccountStatus::NeedsLogin,
        ErrorKind::Expired => AccountStatus::Expired,
        ErrorKind::Blocked => AccountStatus::Blocked,
        // A transient failure with last-known numbers is still usable.
        _ if !snap.windows.is_empty() => AccountStatus::Ok,
        _ => AccountStatus::Error,
    };
    (status, Some(err.clone()))
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
    let (mut status, mut message) = derive_status(snap, f.identity.as_ref());
    // Blocked holds until a fetch succeeds or the account is signed out: a failed check says nothing new.
    if a.status == AccountStatus::Blocked && !matches!(status, AccountStatus::Ok | AccountStatus::NeedsLogin | AccountStatus::Blocked) {
        status = AccountStatus::Blocked;
        message = a.status_message.clone();
    }
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
        error_kind: None,
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

    fn failed(e: FetchError) -> UsageSnapshot {
        let mut s = snap("a", vec![]);
        e.apply(&mut s);
        s
    }

    #[test]
    fn failed_fetch_keeps_last_good_windows_as_stale() {
        let now = Utc::now();
        let good = snap("a", vec![window("five_hour", 40.0, Some(2), 300, now)]);
        let m = merge_snapshot(Some(&good), failed(FetchError::network("network down")));
        assert_eq!(m.windows.len(), 1);
        assert_eq!(m.stale, Some(true));
        assert_eq!(m.error.as_deref(), Some("network down"));
        assert_eq!(m.error_kind, Some(ErrorKind::Network));
        let fresh = merge_snapshot(Some(&m), good.clone());
        assert_eq!(fresh.stale, None);
        assert_eq!(fresh.error_kind, None);
    }

    /// A blocked account's last numbers mean nothing now: drop them instead of keeping them stale.
    #[test]
    fn a_blocked_fetch_drops_last_known_windows() {
        let now = Utc::now();
        let mut good = snap("a", vec![window("five_hour", 40.0, Some(2), 300, now)]);
        good.credits = Some(Credits { used: 1.0, limit: Some(10.0), unit: "USD".into() });
        let fetched = failed(FetchError::blocked("No Claude Code on the Free plan."));
        let m = merge_snapshot(Some(&good), fetched.clone());
        assert!(m.windows.is_empty());
        assert!(m.credits.is_none());
        assert_eq!(m.stale, None);
        let f = Fetched { account_id: "a".into(), snapshot: fetched, identity: Some(Identity { logged_in: true, ..Identity::default() }) };
        let mut a = account("a", "Max");
        apply_fetch(&mut a, &f, &m);
        assert_eq!(a.status, AccountStatus::Blocked);
        assert_eq!(a.status_message.as_deref(), Some("No Claude Code on the Free plan."));
    }

    /// Run fetches through merge, status, scoring and notifications as a poll does. Returns how
    /// many notifications they raised.
    fn poll_sequence(a: &mut Account, fetches: Vec<UsageSnapshot>, st: &mut notify::NotifyState, mut check: impl FnMut(&Account, &UsageSnapshot)) -> usize {
        let now = Utc::now();
        let settings = AppSettings::default().notifications;
        let mut latest: Option<UsageSnapshot> = None;
        let mut notes = 0;
        for fetched in fetches {
            let merged = merge_snapshot(latest.as_ref(), fetched.clone());
            let f = Fetched { account_id: a.id.clone(), snapshot: fetched, identity: Some(Identity { logged_in: true, ..Identity::default() }) };
            apply_fetch(a, &f, &merged);
            check(a, &merged);
            let new = HashMap::from([(a.id.clone(), merged.clone())]);
            notes += notify::evaluate(&HashMap::new(), &new, std::slice::from_ref(a), &settings, st, now).len();
            latest = Some(merged);
        }
        notes
    }

    /// A blocked account stays blocked through offline, rate-limited and live-usage-off checks:
    /// only a successful fetch or a sign-out ends it. It is told once.
    #[test]
    fn blocked_holds_through_temporary_failures() {
        let reason = "This account's subscription was canceled.";
        let now = Utc::now();
        let mut rate_limited = failed(FetchError::other("Claude's usage API is rate-limiting requests; will retry later."));
        rate_limited.stale = Some(true);
        let fetches = vec![
            failed(FetchError::blocked(reason)),
            failed(FetchError::network("Could not reach Claude's usage API (network error).")),
            rate_limited,
            failed(FetchError::blocked(reason)),
            failed(FetchError::other("Live usage is off. Claude Code keeps no local copy of its limits.")),
            failed(FetchError::network("Timed out fetching usage")),
            failed(FetchError::blocked(reason)),
        ];
        let mut a = account("a", "Max");
        let notes = poll_sequence(&mut a, fetches, &mut notify::NotifyState::default(), |a, merged| {
            assert_eq!(a.status, AccountStatus::Blocked);
            assert_eq!(a.status_message.as_deref(), Some(reason));
            assert_eq!(crate::recommend::score_account(a, TaskType::Any, Some(merged), now).score, 0.0);
        });
        assert_eq!(notes, 1);
    }

    /// Resubscribing ends Blocked: fresh numbers, no message, the plan untouched, and a later block
    /// is told again. Signing out ends it too.
    #[test]
    fn blocked_recovers_on_a_successful_fetch() {
        let now = Utc::now();
        let mut a = account("a", "Max 20x");
        let mut st = notify::NotifyState::default();
        let blocked = || failed(FetchError::blocked("Canceled."));
        let mut good = snap("a", vec![window("seven_day", 40.0, Some(100), 10080, now)]);
        good.credits = Some(Credits { used: 1.0, limit: Some(10.0), unit: "USD".into() });
        let mut seen = vec![];
        let notes = poll_sequence(&mut a, vec![blocked(), good, blocked()], &mut st, |a, merged| {
            seen.push((a.status, a.status_message.clone(), a.plan.clone(), merged.windows.len(), merged.credits.is_some()));
        });
        let plan = Some("Max 20x".to_string());
        assert_eq!(
            seen,
            vec![
                (AccountStatus::Blocked, Some("Canceled.".into()), plan.clone(), 0, false),
                (AccountStatus::Ok, None, plan.clone(), 1, true),
                (AccountStatus::Blocked, Some("Canceled.".into()), plan, 0, false),
            ]
        );
        assert_eq!(notes, 2);

        // Signing out ends it too, but blocked again right after was never usable: no new note.
        let signed_out = failed(FetchError::auth("Not signed in. Use Sign in to log this account in."));
        let mut statuses = vec![];
        assert_eq!(poll_sequence(&mut a, vec![signed_out, blocked()], &mut st, |a, _| statuses.push(a.status)), 1);
        assert_eq!(statuses, [AccountStatus::NeedsLogin, AccountStatus::Blocked]);
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
        let status = |e: FetchError| derive_status(&failed(e), None).0;
        assert_eq!(status(FetchError::auth("Not signed in. Use Sign in to log this account in.")), AccountStatus::NeedsLogin);
        assert_eq!(status(FetchError::expired("Login expired. Launch Codex once or sign in again.")), AccountStatus::Expired);
        assert_eq!(status(FetchError::other("Claude's usage API returned HTTP 500.")), AccountStatus::Error);
        // The kind decides, not the wording.
        assert_eq!(status(FetchError::auth("GitHub refused the request")), AccountStatus::NeedsLogin);
        let ok = snap("a", vec![window("w", 1.0, None, 300, Utc::now())]);
        assert_eq!(derive_status(&ok, Some(&Identity::default())).0, AccountStatus::Ok);
    }

    #[test]
    fn network_failures_are_not_login_problems() {
        for msg in [
            // What claude::refresh_failed and gemini::offline_refresh say when offline.
            "Could not renew the Claude session: could not reach Anthropic (network error).",
            "Could not reach Google to renew the Gemini session (error sending request)",
            // Network failures that name the login or its token still aren't login problems.
            "Token refresh failed: error sending request for url (https://oauth2.googleapis.com/token)",
            "Could not renew the login token",
        ] {
            assert_eq!(derive_status(&failed(FetchError::network(msg)), None).0, AccountStatus::Error, "{msg}");
        }
    }

    #[test]
    fn a_failed_fetch_with_last_known_windows_keeps_the_account_usable() {
        let now = Utc::now();
        let good = snap("a", vec![window("five_hour", 40.0, Some(2), 300, now)]);
        let fetched = failed(FetchError::network("Could not reach Claude's usage API (timed out)."));
        let merged = merge_snapshot(Some(&good), fetched.clone());
        let f = Fetched { account_id: "a".into(), snapshot: fetched, identity: None };
        let mut a = account("a", "pro");
        apply_fetch(&mut a, &f, &merged);
        assert_eq!(a.status, AccountStatus::Ok);
        assert_eq!(a.status_message.as_deref(), Some("Could not reach Claude's usage API (timed out)."));
    }
}
