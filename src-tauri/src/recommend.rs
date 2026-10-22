//! Scores accounts 0..100 for "what should I use now", with a plain-language reason.
//! 0 means it can't be used right now (signed out, blocked or at a limit); the UI never offers those.
use crate::model::*;
use crate::util;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// A window that resets within this many hours with plenty left earns a "use it or lose it" bonus.
const RESET_SOON_HOURS: i64 = 24;

/// Windows that only cap one model (e.g. "Weekly Opus"): running out still leaves the account usable.
/// Same rule as `isModelWindow` in src/lib/quota.ts.
pub fn is_model_window(w: &UsageWindow) -> bool {
    let id = w.id.to_ascii_lowercase();
    ["opus", "sonnet", "haiku"].iter().any(|m| id.contains(m))
}

/// The window with the least headroom (highest usedPercent): the one that limits the account.
/// Model-only windows are ignored when a general window exists (as `bindingWindow` in src/lib/quota.ts).
pub fn tightest(snapshot: &UsageSnapshot) -> Option<&UsageWindow> {
    let only_model = snapshot.windows.iter().all(is_model_window);
    snapshot.windows.iter().filter(|w| only_model || !is_model_window(w)).max_by(|a, b| a.used_percent.total_cmp(&b.used_percent))
}

/// Percent left in the tightest window, if any usage is known.
pub fn left_percent(snapshot: &UsageSnapshot) -> Option<f64> {
    tightest(snapshot).map(|w| (100.0 - w.used_percent).clamp(0.0, 100.0))
}

/// Short rolling windows (5-hour sessions) reset constantly, so they never count as "about to be lost".
pub fn is_short_window(w: &UsageWindow) -> bool {
    match w.window_minutes {
        Some(m) => m < 24 * 60,
        None => {
            let id = w.id.to_ascii_lowercase();
            let label = w.label.to_ascii_lowercase();
            id.contains("five_hour") || id.contains("5h") || label.contains("5-hour") || label.contains("hour")
        }
    }
}

/// 0 = unknown/free, 1 = Plus, 2 = Pro/Team/Business, 3 = Max/Pro+/Ultra/Enterprise and ChatGPT Pro 100/200/500.
pub fn tier(account: &Account) -> u8 {
    let mut text = account.plan.clone().unwrap_or_default().to_ascii_lowercase();
    for t in &account.tags {
        text.push(' ');
        text.push_str(&t.to_ascii_lowercase());
    }
    if ["max", "pro+", "pro plus", "ultra", "enterprise", "pro 100", "pro 200", "pro 500", "premium"].iter().any(|k| text.contains(k)) {
        3
    } else if ["pro", "team", "business"].iter().any(|k| text.contains(k)) {
        2
    } else if text.contains("plus") {
        1
    } else {
        0
    }
}

fn task_keywords(task: TaskType) -> &'static [&'static str] {
    match task {
        TaskType::Coding => &["coding", "code", "dev", "development"],
        TaskType::LongRefactor => &["long-refactor", "refactor", "long", "heavy"],
        TaskType::QuickQuestion => &["quick-question", "quick", "chat", "questions"],
        TaskType::Research => &["research"],
        TaskType::Review => &["review", "reviews"],
        TaskType::Any => &[],
    }
}

fn tag_matches(account: &Account, task: TaskType) -> bool {
    let kws = task_keywords(task);
    account.tags.iter().any(|t| kws.contains(&t.trim().to_ascii_lowercase().as_str()))
}

/// Best first, ties by label. Accounts of a provider in `cli_missing` can still be used in its desktop
/// app, so they score 1 (not 0) and sort after every other usable account, best quota first.
pub fn recommend(
    task: TaskType,
    provider: Option<ProviderId>,
    accounts: &[Account],
    latest: &HashMap<String, UsageSnapshot>,
    cli_missing: &[ProviderId],
    now: DateTime<Utc>,
) -> Vec<Recommendation> {
    let mut out: Vec<(Recommendation, bool, f64, &str)> = accounts
        .iter()
        .filter(|a| provider.is_none_or(|p| a.provider_id == p))
        .map(|a| {
            let mut r = score_account(a, task, latest.get(&a.id), now);
            let (missing, raw) = (cli_missing.contains(&a.provider_id), r.score);
            if missing && raw > 0.0 {
                // Keep what is left first; the task-fit advice doesn't apply without the CLI.
                let left = r.reason.split_inclusive(". ").next().unwrap_or_default().trim_end().to_string();
                r.score = 1.0;
                r.reason = format!("{left} The {} CLI isn't installed on this computer.", a.provider_id.name());
            }
            (r, missing, raw, a.label.as_str())
        })
        .collect();
    out.sort_by(|(a, ma, ra, la), (b, mb, rb, lb)| b.score.total_cmp(&a.score).then(ma.cmp(mb)).then(rb.total_cmp(ra)).then_with(|| la.cmp(lb)));
    out.into_iter().map(|(r, ..)| r).collect()
}

fn rec(account: &Account, score: f64, reason: String) -> Recommendation {
    Recommendation { account_id: account.id.clone(), score: (score.clamp(0.0, 100.0) * 10.0).round() / 10.0, reason }
}

pub fn score_account(account: &Account, task: TaskType, snap: Option<&UsageSnapshot>, now: DateTime<Utc>) -> Recommendation {
    if matches!(account.status, AccountStatus::NeedsLogin | AccountStatus::Expired) {
        return rec(account, 0.0, "Needs login before it can be used.".into());
    }
    if account.status == AccountStatus::Blocked {
        let why = account.status_message.as_deref().unwrap_or("this account has no access.");
        return rec(account, 0.0, format!("Can't be used: {why}"));
    }
    let tier = f64::from(tier(account));
    let fit = match task {
        TaskType::LongRefactor => tier * 8.0,
        TaskType::Coding => tier * 5.0,
        TaskType::Research | TaskType::Review => tier * 2.0,
        TaskType::QuickQuestion => -tier * 4.0,
        TaskType::Any => 0.0,
    };
    let tag_bonus = if tag_matches(account, task) { 10.0 } else { 0.0 };
    let pin_bonus = if account.pinned == Some(true) { 2.0 } else { 0.0 };

    let Some((snap, tight)) = snap.and_then(|s| tightest(s).map(|w| (s, w))) else {
        return rec(account, 40.0 + fit + tag_bonus + pin_bonus, "No usage data yet.".into());
    };
    let left = (100.0 - tight.used_percent).clamp(0.0, 100.0);
    let resets = tight.resets_at.as_deref().and_then(util::parse_iso).filter(|t| *t > now);

    if left < 1.0 {
        let when = resets.map(|t| format!(", resets {}", util::human_until(t, now))).unwrap_or_default();
        return rec(account, 0.0, format!("{} limit reached{when}.", tight.label));
    }

    let base = match task {
        // Save big accounts: prefer ones with little headroom left, as long as there is enough.
        TaskType::QuickQuestion if left >= 10.0 => 50.0 + (100.0 - left) * 0.25,
        TaskType::QuickQuestion => left * 4.0,
        _ => left * 0.65,
    };
    let lose = use_it_or_lose_it(snap, now);
    let reset_bonus = lose.map(|(_, wleft)| 12.0 * wleft / 100.0).unwrap_or(0.0);
    let stale_penalty = if snap.stale == Some(true) { 5.0 } else { 0.0 };
    let score = base + fit + tag_bonus + pin_bonus + reset_bonus - stale_penalty;

    let mut reason = format!("{:.0}% of {} left", left, tight.label);
    if let Some(t) = resets {
        reason.push_str(&format!(", resets {}", util::human_until(t, now)));
    }
    reason.push('.');
    if let Some((w, wleft)) = lose {
        if w.id == tight.id {
            reason.push_str(" Use it before it resets.");
        } else {
            reason.push_str(&format!(" {:.0}% of {} resets soon, use it before then.", wleft, w.label));
        }
    }
    match task {
        TaskType::LongRefactor | TaskType::Coding if tier >= 3.0 => reason.push_str(" Top-tier plan suits long sessions."),
        TaskType::QuickQuestion if left < 50.0 => reason.push_str(" Good for a quick task, saves your bigger accounts."),
        _ => {}
    }
    if tag_bonus > 0.0 {
        reason.push_str(" Tagged for this kind of task.");
    }
    if stale_penalty > 0.0 {
        reason.push_str(" (Last known values.)");
    }
    // 0 means "can't use it" (the Use now card skips those), so a usable account always scores at least 1.
    rec(account, score.max(1.0), reason)
}

/// A long window (weekly, monthly) resetting within RESET_SOON_HOURS with at least 30% left.
fn use_it_or_lose_it(snap: &UsageSnapshot, now: DateTime<Utc>) -> Option<(&UsageWindow, f64)> {
    snap.windows
        .iter()
        .filter(|w| !is_short_window(w))
        .filter_map(|w| {
            let t = util::parse_iso(w.resets_at.as_deref()?)?;
            let left = (100.0 - w.used_percent).clamp(0.0, 100.0);
            (t > now && (t - now).num_hours() < RESET_SOON_HOURS && left >= 30.0).then_some((w, left))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn account(id: &str, plan: &str) -> Account {
        Account {
            id: id.into(),
            provider_id: ProviderId::Claude,
            label: id.into(),
            email: None,
            plan: Some(plan.into()),
            home_dir: format!("/h/{id}"),
            is_default_home: false,
            shared_from: None,
            sharing: SharingOptions::default(),
            color: "#000".into(),
            tags: vec![],
            notes: None,
            status: AccountStatus::Ok,
            status_message: None,
            created_at: util::now_iso(),
            last_launched_at: None,
            launch_count: 0,
            monthly_cost: None,
            pinned: None,
        }
    }

    pub fn window(id: &str, used: f64, resets_in_h: Option<i64>, minutes: u32, now: DateTime<Utc>) -> UsageWindow {
        UsageWindow {
            id: id.into(),
            label: if minutes < 1440 { "5-hour session".into() } else { "Weekly".into() },
            used_percent: used,
            resets_at: resets_in_h.map(|h| (now + chrono::Duration::hours(h)).to_rfc3339()),
            window_minutes: Some(minutes),
        }
    }

    pub fn snap(id: &str, windows: Vec<UsageWindow>) -> UsageSnapshot {
        UsageSnapshot {
            account_id: id.into(),
            provider_id: ProviderId::Claude,
            fetched_at: util::now_iso(),
            source: UsageSource::Api,
            windows,
            plan: None,
            email: None,
            credits: None,
            error: None,
            error_kind: None,
            stale: None,
        }
    }

    #[test]
    fn exhausted_and_logged_out_rank_last() {
        let now = Utc::now();
        let accts = vec![account("a", "Pro"), account("b", "Pro"), {
            let mut c = account("c", "Max");
            c.status = AccountStatus::NeedsLogin;
            c
        }];
        let latest = HashMap::from([
            ("a".to_string(), snap("a", vec![window("five_hour", 100.0, Some(2), 300, now)])),
            ("b".to_string(), snap("b", vec![window("five_hour", 30.0, Some(2), 300, now)])),
        ]);
        let r = recommend(TaskType::Coding, None, &accts, &latest, &[], now);
        assert_eq!(r[0].account_id, "b");
        assert!(r[0].reason.starts_with("70% of 5-hour session left"));
        assert!(r.iter().find(|x| x.account_id == "a").unwrap().reason.contains("limit reached"));
        assert!(r.iter().all(|x| (0.0..=100.0).contains(&x.score)));
        // The UI's "Use now" only offers accounts scoring above 0.
        let unusable: Vec<_> = r.iter().filter(|x| x.score == 0.0).map(|x| x.account_id.as_str()).collect();
        assert_eq!(unusable, ["a", "c"]);
    }

    /// Same vectors as "never recommends a blocked account" in src/lib/quota.test.ts.
    #[test]
    fn blocked_accounts_are_never_recommended() {
        let now = Utc::now();
        let mut a = account("a", "Max");
        a.status = AccountStatus::Blocked;
        a.status_message = Some("This account's subscription was canceled.".into());
        // Neither without usage nor with last-known room left.
        let s = snap("a", vec![window("five_hour", 10.0, Some(2), 300, now)]);
        for snap in [None, Some(&s)] {
            let r = score_account(&a, TaskType::Any, snap, now);
            assert_eq!(r.score, 0.0);
            assert_eq!(r.reason, "Can't be used: This account's subscription was canceled.");
        }
    }

    #[test]
    fn long_refactor_prefers_max_quick_question_prefers_small() {
        let now = Utc::now();
        let accts = vec![account("max", "Max 20x"), account("pro", "Pro")];
        let latest = HashMap::from([
            ("max".to_string(), snap("max", vec![window("five_hour", 20.0, Some(3), 300, now)])),
            ("pro".to_string(), snap("pro", vec![window("five_hour", 60.0, Some(3), 300, now)])),
        ]);
        assert_eq!(recommend(TaskType::LongRefactor, None, &accts, &latest, &[], now)[0].account_id, "max");
        assert_eq!(recommend(TaskType::QuickQuestion, None, &accts, &latest, &[], now)[0].account_id, "pro");
    }

    #[test]
    fn weekly_resetting_soon_gets_bonus_and_reason() {
        let now = Utc::now();
        let accts = vec![account("a", "Pro"), account("b", "Pro")];
        let latest = HashMap::from([
            ("a".to_string(), snap("a", vec![window("seven_day", 40.0, Some(10), 10080, now)])),
            ("b".to_string(), snap("b", vec![window("seven_day", 40.0, Some(100), 10080, now)])),
        ]);
        let r = recommend(TaskType::Any, None, &accts, &latest, &[], now);
        assert_eq!(r[0].account_id, "a");
        assert!(r[0].reason.contains("Use it before it resets"), "{}", r[0].reason);
    }

    /// Same vectors as `bindingWindow` in src/lib/quota.test.ts: a model-only window (Weekly Sonnet)
    /// running out leaves the account usable, so it never counts as the tightest window.
    #[test]
    fn model_only_windows_do_not_bind() {
        let now = Utc::now();
        let mut sonnet = window("seven_day_sonnet", 100.0, Some(30), 10080, now);
        sonnet.label = "Weekly Sonnet".into();
        let s = snap("a", vec![window("five_hour", 20.0, Some(2), 300, now), window("seven_day", 30.0, Some(100), 10080, now), sonnet.clone()]);
        assert_eq!(tightest(&s).map(|w| w.id.as_str()), Some("seven_day"));
        assert_eq!(left_percent(&s), Some(70.0));
        let r = score_account(&account("a", "Pro"), TaskType::Any, Some(&s), now);
        assert!(r.score > 10.0 && !r.reason.contains("limit reached"), "{r:?}");

        let opus = UsageWindow { id: "seven_day_opus".into(), label: "Weekly Opus".into(), ..sonnet.clone() };
        let s =
            snap("a", vec![window("five_hour", 64.0, None, 300, now), window("seven_day", 41.0, None, 10080, now), UsageWindow { used_percent: 90.0, ..opus }]);
        assert_eq!(tightest(&s).map(|w| w.id.as_str()), Some("five_hour"));

        // With only model windows known, they still count.
        let s = snap("a", vec![sonnet]);
        assert_eq!(tightest(&s).map(|w| w.id.as_str()), Some("seven_day_sonnet"));
        assert_eq!(left_percent(&s), Some(0.0));
    }

    #[test]
    fn usable_accounts_never_score_zero() {
        let now = Utc::now();
        // A quick question on a top-tier plan with 3% left: 3 * 4 - 3 * 4 would otherwise land on 0.
        let a = account("a", "Max 20x");
        let s = snap("a", vec![window("w", 97.0, Some(2), 300, now)]);
        assert!(score_account(&a, TaskType::QuickQuestion, Some(&s), now).score >= 1.0);
        let full = snap("a", vec![window("w", 100.0, Some(2), 300, now)]);
        assert_eq!(score_account(&a, TaskType::QuickQuestion, Some(&full), now).score, 0.0);
    }

    /// Same vectors as "sorts accounts without their CLI last" in src/lib/quota.test.ts.
    #[test]
    fn missing_cli_sorts_last_but_stays_usable() {
        let now = Utc::now();
        let codex = |id: &str, status: AccountStatus| Account { provider_id: ProviderId::Codex, status, ..account(id, "Pro") };
        let accts = vec![account("a", "Pro"), codex("b", AccountStatus::Ok), codex("c", AccountStatus::Ok), codex("d", AccountStatus::NeedsLogin)];
        let latest = HashMap::from([
            ("a".to_string(), snap("a", vec![window("five_hour", 30.0, None, 300, now)])),
            ("b".to_string(), snap("b", vec![window("five_hour", 90.0, None, 300, now)])),
            ("c".to_string(), snap("c", vec![window("five_hour", 10.0, None, 300, now)])),
        ]);
        let r = recommend(TaskType::Any, None, &accts, &latest, &[ProviderId::Codex], now);
        let order: Vec<_> = r.iter().map(|x| (x.account_id.as_str(), x.score)).collect();
        // Still usable (its desktop app may work), so 1 and not 0; the one with more left comes first.
        assert_eq!(order, [("a", 45.5), ("c", 1.0), ("b", 1.0), ("d", 0.0)]);
        assert_eq!(r[1].reason, "90% of 5-hour session left. The Codex CLI isn't installed on this computer.");
        assert_eq!(r[3].reason, "Needs login before it can be used.");
        // Without the CLI check the same accounts rank on quota alone.
        assert_eq!(recommend(TaskType::Any, None, &accts, &latest, &[], now)[0].account_id, "c");
    }

    #[test]
    fn provider_filter_and_tags() {
        let now = Utc::now();
        let mut tagged = account("t", "Pro");
        tagged.tags = vec!["Research".into()];
        let mut other = account("o", "Pro");
        other.provider_id = ProviderId::Codex;
        let accts = vec![account("u", "Pro"), tagged, other];
        let r = recommend(TaskType::Research, Some(ProviderId::Claude), &accts, &HashMap::new(), &[], now);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].account_id, "t");
        assert_eq!(tier(&account("x", "Copilot Pro+")), 3);
        assert_eq!(tier(&account("x", "Plus")), 1);
        assert_eq!(tier(&account("x", "Pro 200")), 3);
        assert_eq!(tier(&account("x", "Team Premium")), 3);
        assert_eq!(tier(&account("x", "Google AI Pro")), 2);
        assert_eq!(tier(&account("x", "Google AI Plus")), 1);
    }
}
