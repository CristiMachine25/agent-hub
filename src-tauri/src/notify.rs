//! Notification rules: compare the previous and new usage per window and decide what to tell the
//! user. Each rule fires once and re-arms only when the state changes (usage drops back below the
//! threshold, or the window resets), so polls never repeat the same alert.
use crate::model::*;
use crate::recommend;
use crate::store::AppState;
use crate::util;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tauri::{AppHandle, Emitter};

/// At or above this, a window counts as exhausted.
pub const LIMIT_PERCENT: f64 = 99.5;
/// Hysteresis: re-arm "limit reached" once usage drops below this.
const LIMIT_REARM_PERCENT: f64 = 95.0;
/// Re-arm "nearly exhausted" once usage drops this far below the threshold.
const NEARLY_REARM_GAP: f64 = 5.0;

/// Persisted dedupe state: rule key -> marker (window cycle or "1").
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct NotifyState {
    #[serde(default)]
    pub fired: HashMap<String, String>,
}

fn key(kind: &str, account: &str, window: &str) -> String {
    format!("{kind}|{account}|{window}")
}

pub fn provider_name(p: ProviderId) -> &'static str {
    match p {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Gemini => "Gemini",
        ProviderId::Copilot => "Copilot",
    }
}

/// "Work Max (Claude)"
pub fn display_name(a: &Account) -> String {
    format!("{} ({})", a.label, provider_name(a.provider_id))
}

fn note(kind: NotificationKind, account: &Account, title: String, body: String, now: DateTime<Utc>) -> AppNotification {
    AppNotification {
        id: uuid::Uuid::new_v4().to_string(),
        kind,
        account_id: Some(account.id.clone()),
        title,
        body,
        created_at: now.to_rfc3339(),
        read: false,
    }
}

/// " resets at 14:30 (in 2 h)" or "".
fn reset_phrase(w: &UsageWindow, now: DateTime<Utc>) -> Option<String> {
    let t = util::parse_iso(w.resets_at.as_deref()?)?;
    (t > now).then(|| format!("resets at {} ({})", util::local_clock(t, now), util::human_until(t, now)))
}

/// Best other usable account of the same provider, with its percent left.
fn best_alternative<'a>(
    account: &Account,
    accounts: &'a [Account],
    latest: &HashMap<String, UsageSnapshot>,
    now: DateTime<Utc>,
) -> Option<(&'a Account, Recommendation)> {
    recommend::recommend(TaskType::Any, Some(account.provider_id), accounts, latest, now)
        .into_iter()
        .filter(|r| r.account_id != account.id && r.score >= 10.0)
        .find_map(|r| accounts.iter().find(|a| a.id == r.account_id).map(|a| (a, r)))
}

fn alternative_sentence(alt: Option<&(&Account, Recommendation)>, latest: &HashMap<String, UsageSnapshot>) -> String {
    let Some((a, _)) = alt else { return String::new() };
    match latest.get(&a.id).and_then(recommend::left_percent) {
        Some(left) => format!(" {} has {:.0}% left.", a.label, left),
        None => format!(" {} is available.", a.label),
    }
}

/// Run every rule. `prev` is the usage before this poll, `new` after it (stale entries are skipped).
pub fn evaluate(
    prev: &HashMap<String, UsageSnapshot>,
    new: &HashMap<String, UsageSnapshot>,
    accounts: &[Account],
    settings: &NotificationSettings,
    state: &mut NotifyState,
    now: DateTime<Utc>,
) -> Vec<AppNotification> {
    let mut out = Vec::new();
    for account in accounts {
        login_rule(account, state, now, &mut out);
        let Some(snap) = new.get(&account.id) else { continue };
        if snap.stale == Some(true) || snap.windows.is_empty() {
            continue;
        }
        let prev_snap = prev.get(&account.id);
        for w in &snap.windows {
            let p = prev_snap.and_then(|s| s.windows.iter().find(|x| x.id == w.id));
            let ctx = WindowCtx { account, accounts, w, latest: new, settings, now };
            if let Some(p) = p {
                reset_rule(&ctx, p, state, &mut out);
            }
            limit_rules(&ctx, state, &mut out);
            use_it_rule(&ctx, state, &mut out);
        }
    }
    out
}

struct WindowCtx<'a> {
    account: &'a Account,
    accounts: &'a [Account],
    w: &'a UsageWindow,
    latest: &'a HashMap<String, UsageSnapshot>,
    settings: &'a NotificationSettings,
    now: DateTime<Utc>,
}

impl WindowCtx<'_> {
    fn key(&self, kind: &str) -> String {
        key(kind, &self.account.id, &self.w.id)
    }
}

fn login_rule(account: &Account, state: &mut NotifyState, now: DateTime<Utc>, out: &mut Vec<AppNotification>) {
    let k = key("login", &account.id, "");
    if !matches!(account.status, AccountStatus::NeedsLogin | AccountStatus::Expired) {
        state.fired.remove(&k);
        return;
    }
    if state.fired.contains_key(&k) {
        return;
    }
    state.fired.insert(k, "1".into());
    let why = account.status_message.as_deref().map(|m| format!(" ({m})")).unwrap_or_default();
    out.push(note(
        NotificationKind::LoginRequired,
        account,
        format!("{} needs you to log in", display_name(account)),
        format!("Usage and launches for this account will not work until you log in again{why}. Use Log in on its card."),
        now,
    ));
}

/// A window reset: usage fell from at/above the threshold to well below it, or its reset time passed.
fn reset_rule(ctx: &WindowCtx, p: &UsageWindow, state: &mut NotifyState, out: &mut Vec<AppNotification>) {
    let th = ctx.settings.nearly_exhausted_percent;
    let w = ctx.w;
    let dropped = p.used_percent >= th && w.used_percent < th && p.used_percent - w.used_percent >= 25.0;
    let passed = p.resets_at.as_deref().and_then(util::parse_iso).is_some_and(|t| t <= ctx.now) && w.used_percent < p.used_percent;
    if !(dropped || passed) {
        return;
    }
    for kind in ["limit", "nearly", "lose"] {
        state.fired.remove(&ctx.key(kind));
    }
    if ctx.settings.on_reset && p.used_percent >= th {
        out.push(note(
            NotificationKind::LimitReset,
            ctx.account,
            format!("{} is ready again", display_name(ctx.account)),
            format!("{} reset. {:.0}% is available now.", w.label, (100.0 - w.used_percent).clamp(0.0, 100.0)),
            ctx.now,
        ));
    }
}

fn limit_rules(ctx: &WindowCtx, state: &mut NotifyState, out: &mut Vec<AppNotification>) {
    let used = ctx.w.used_percent;
    let th = ctx.settings.nearly_exhausted_percent;
    let (limit_k, nearly_k) = (ctx.key("limit"), ctx.key("nearly"));
    if used < LIMIT_REARM_PERCENT {
        state.fired.remove(&limit_k);
    }
    if used < th - NEARLY_REARM_GAP {
        state.fired.remove(&nearly_k);
    }
    let name = display_name(ctx.account);
    let resets = reset_phrase(ctx.w, ctx.now);
    if used >= LIMIT_PERCENT {
        if state.fired.contains_key(&limit_k) {
            return;
        }
        state.fired.insert(limit_k, "1".into());
        state.fired.insert(nearly_k, "1".into());
        let alt = best_alternative(ctx.account, ctx.accounts, ctx.latest, ctx.now);
        let when = resets.map(|r| format!("It {r}.")).unwrap_or_else(|| "Reset time unknown.".into());
        out.push(note(
            NotificationKind::LimitReached,
            ctx.account,
            format!("{name} hit its {} limit", ctx.w.label),
            format!("{when}{}", alternative_sentence(alt.as_ref(), ctx.latest)),
            ctx.now,
        ));
        if let Some((a, r)) = alt {
            out.push(note(
                NotificationKind::SwitchSuggested,
                a,
                format!("Switch to {}", display_name(a)),
                format!("{} hit its {} limit. {}", ctx.account.label, ctx.w.label, r.reason),
                ctx.now,
            ));
        }
    } else if used >= th && !state.fired.contains_key(&nearly_k) && !state.fired.contains_key(&limit_k) {
        state.fired.insert(nearly_k, "1".into());
        let alt = best_alternative(ctx.account, ctx.accounts, ctx.latest, ctx.now);
        let when = resets.map(|r| format!("{} {r}.", ctx.w.label)).unwrap_or_else(|| format!("{} is nearly used up.", ctx.w.label));
        out.push(note(
            NotificationKind::NearlyExhausted,
            ctx.account,
            format!("{name} is at {used:.0}%"),
            format!("{when}{}", alternative_sentence(alt.as_ref(), ctx.latest)),
            ctx.now,
        ));
    }
}

/// A long window resets soon with a lot left: once per window cycle.
fn use_it_rule(ctx: &WindowCtx, state: &mut NotifyState, out: &mut Vec<AppNotification>) {
    let w = ctx.w;
    if recommend::is_short_window(w) {
        return;
    }
    let Some(t) = w.resets_at.as_deref().and_then(util::parse_iso) else { return };
    let left = (100.0 - w.used_percent).clamp(0.0, 100.0);
    let hours = (t - ctx.now).num_minutes() as f64 / 60.0;
    if hours <= 0.0 || hours > ctx.settings.use_it_or_lose_it_hours || left < ctx.settings.use_it_or_lose_it_min_left_percent {
        return;
    }
    let k = ctx.key("lose");
    // Providers that report "resets in N seconds" jitter slightly between polls: same cycle within 3 h.
    let same_cycle = state
        .fired
        .get(&k)
        .and_then(|m| m.parse::<i64>().ok())
        .is_some_and(|prev| (prev - t.timestamp()).abs() < 3 * 3600);
    if same_cycle {
        return;
    }
    state.fired.insert(k, t.timestamp().to_string());
    out.push(note(
        NotificationKind::UseItOrLoseIt,
        ctx.account,
        format!("Use {} before it resets", display_name(ctx.account)),
        format!("{left:.0}% of {} is still available and {}.", w.label, reset_phrase(w, ctx.now).unwrap_or_default()),
        ctx.now,
    ));
}

/// Store, emit to the UI, and show native notifications when enabled.
pub async fn deliver(app: &AppHandle, state: &AppState, items: Vec<AppNotification>) {
    if items.is_empty() {
        return;
    }
    let os_enabled = state.settings.read().await.notifications.enabled;
    if let Err(e) = state.push_notifications(items.clone()).await {
        log::warn!("saving notifications failed: {e:#}");
    }
    for n in &items {
        let _ = app.emit("notification", n);
        if os_enabled {
            show_native(app, &n.title, &n.body);
        }
    }
}

pub fn show_native(app: &AppHandle, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("native notification failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::recommend::tests::{account, snap, window};

    fn settings() -> NotificationSettings {
        AppSettings::default().notifications
    }

    fn run(
        prev: &HashMap<String, UsageSnapshot>,
        new: &HashMap<String, UsageSnapshot>,
        accts: &[Account],
        st: &mut NotifyState,
        now: DateTime<Utc>,
    ) -> Vec<NotificationKind> {
        evaluate(prev, new, accts, &settings(), st, now).into_iter().map(|n| n.kind).collect()
    }

    fn one(id: &str, w: UsageWindow) -> HashMap<String, UsageSnapshot> {
        HashMap::from([(id.to_string(), snap(id, vec![w]))])
    }

    #[test]
    fn nearly_exhausted_fires_once_then_rearms() {
        let now = Utc::now();
        let accts = vec![account("a", "Pro")];
        let mut st = NotifyState::default();
        let empty = HashMap::new();
        let s85 = one("a", window("five_hour", 85.0, Some(2), 300, now));
        let notes = evaluate(&empty, &s85, &accts, &settings(), &mut st, now);
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].kind, NotificationKind::NearlyExhausted);
        assert_eq!(notes[0].title, "a (Claude) is at 85%");
        assert!(run(&s85, &s85, &accts, &mut st, now).is_empty(), "deduped");
        let s50 = one("a", window("five_hour", 50.0, Some(2), 300, now));
        // Usage only falls when a window resets, so a big drop counts as a reset.
        assert_eq!(run(&s85, &s50, &accts, &mut st, now), vec![NotificationKind::LimitReset]);
        assert!(run(&s50, &s50, &accts, &mut st, now).is_empty());
        assert_eq!(run(&s50, &s85, &accts, &mut st, now), vec![NotificationKind::NearlyExhausted]);
    }

    #[test]
    fn limit_reached_suggests_switch_and_reset_notifies() {
        let now = Utc::now();
        let accts = vec![account("a", "Max"), account("b", "Pro")];
        let mut st = NotifyState::default();
        let mut new = one("a", window("five_hour", 100.0, Some(2), 300, now));
        new.insert("b".into(), snap("b", vec![window("five_hour", 30.0, Some(2), 300, now)]));
        let notes = evaluate(&HashMap::new(), &new, &accts, &settings(), &mut st, now);
        let kinds: Vec<_> = notes.iter().map(|n| n.kind).collect();
        assert_eq!(kinds, vec![NotificationKind::LimitReached, NotificationKind::SwitchSuggested]);
        assert!(notes[0].body.contains("b has 70% left."), "{}", notes[0].body);
        assert_eq!(notes[1].account_id.as_deref(), Some("b"));
        assert!(run(&new, &new, &accts, &mut st, now).is_empty());

        let mut after = one("a", window("five_hour", 2.0, Some(5), 300, now));
        after.insert("b".into(), new["b"].clone());
        assert_eq!(run(&new, &after, &accts, &mut st, now), vec![NotificationKind::LimitReset]);
        assert!(!st.fired.keys().any(|k| k.starts_with("limit|a")));
    }

    #[test]
    fn use_it_or_lose_it_once_per_cycle_and_not_for_short_windows() {
        let now = Utc::now();
        let accts = vec![account("a", "Pro")];
        let mut st = NotifyState::default();
        let weekly = one("a", window("seven_day", 20.0, Some(10), 10080, now));
        assert_eq!(run(&HashMap::new(), &weekly, &accts, &mut st, now), vec![NotificationKind::UseItOrLoseIt]);
        assert!(run(&weekly, &weekly, &accts, &mut st, now).is_empty());
        let short = one("a", window("five_hour", 10.0, Some(2), 300, now));
        assert!(run(&HashMap::new(), &short, &accts, &mut NotifyState::default(), now).is_empty());
        let far = one("a", window("seven_day", 20.0, Some(100), 10080, now));
        assert!(run(&HashMap::new(), &far, &accts, &mut NotifyState::default(), now).is_empty());
    }

    #[test]
    fn login_required_once_until_fixed_and_stale_skipped() {
        let now = Utc::now();
        let mut a = account("a", "Pro");
        a.status = AccountStatus::NeedsLogin;
        let mut st = NotifyState::default();
        let mut stale = one("a", window("five_hour", 100.0, Some(2), 300, now));
        stale.get_mut("a").unwrap().stale = Some(true);
        assert_eq!(run(&HashMap::new(), &stale, std::slice::from_ref(&a), &mut st, now), vec![NotificationKind::LoginRequired]);
        assert!(run(&HashMap::new(), &stale, std::slice::from_ref(&a), &mut st, now).is_empty());
        a.status = AccountStatus::Ok;
        run(&HashMap::new(), &HashMap::new(), std::slice::from_ref(&a), &mut st, now);
        a.status = AccountStatus::Expired;
        assert_eq!(run(&HashMap::new(), &HashMap::new(), &[a], &mut st, now), vec![NotificationKind::LoginRequired]);
    }
}
