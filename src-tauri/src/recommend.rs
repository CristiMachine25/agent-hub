//! Scores accounts 0..100 for "what should I use now", with a plain-language reason.
use crate::model::*;
use crate::util;
use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// A window that resets within this many hours with plenty left earns a "use it or lose it" bonus.
const RESET_SOON_HOURS: i64 = 24;

/// The window with the least headroom (highest usedPercent).
pub fn tightest(snapshot: &UsageSnapshot) -> Option<&UsageWindow> {
    snapshot.windows.iter().max_by(|a, b| a.used_percent.total_cmp(&b.used_percent))
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

/// 0 = unknown/free, 1 = Plus, 2 = Pro/Team/Business, 3 = Max/Pro+/Ultra/Enterprise.
pub fn tier(account: &Account) -> u8 {
    let mut text = account.plan.clone().unwrap_or_default().to_ascii_lowercase();
    for t in &account.tags {
        text.push(' ');
        text.push_str(&t.to_ascii_lowercase());
    }
    if ["max", "pro+", "pro plus", "ultra", "enterprise"].iter().any(|k| text.contains(k)) {
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

pub fn recommend(
    task: TaskType,
    provider: Option<ProviderId>,
    accounts: &[Account],
    latest: &HashMap<String, UsageSnapshot>,
    now: DateTime<Utc>,
) -> Vec<Recommendation> {
    let mut out: Vec<(Recommendation, &str)> = accounts
        .iter()
        .filter(|a| provider.is_none_or(|p| a.provider_id == p))
        .map(|a| (score_account(a, task, latest.get(&a.id), now), a.label.as_str()))
        .collect();
    out.sort_by(|(a, la), (b, lb)| b.score.total_cmp(&a.score).then_with(|| la.cmp(lb)));
    out.into_iter().map(|(r, _)| r).collect()
}

fn rec(account: &Account, score: f64, reason: String) -> Recommendation {
    Recommendation { account_id: account.id.clone(), score: (score.clamp(0.0, 100.0) * 10.0).round() / 10.0, reason }
}

pub fn score_account(account: &Account, task: TaskType, snap: Option<&UsageSnapshot>, now: DateTime<Utc>) -> Recommendation {
    if matches!(account.status, AccountStatus::NeedsLogin | AccountStatus::Expired) {
        return rec(account, 3.0, "Needs login before it can be used.".into());
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
        return rec(account, 2.0, format!("{} limit reached{when}.", tight.label));
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
    rec(account, score, reason)
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
            stale: None,
        }
    }

    #[test]
    fn exhausted_and_logged_out_rank_last() {
        let now = Utc::now();
        let accts = vec![account("a", "Pro"), account("b", "Pro"), { let mut c = account("c", "Max"); c.status = AccountStatus::NeedsLogin; c }];
        let latest = HashMap::from([
            ("a".to_string(), snap("a", vec![window("five_hour", 100.0, Some(2), 300, now)])),
            ("b".to_string(), snap("b", vec![window("five_hour", 30.0, Some(2), 300, now)])),
        ]);
        let r = recommend(TaskType::Coding, None, &accts, &latest, now);
        assert_eq!(r[0].account_id, "b");
        assert!(r[0].reason.starts_with("70% of 5-hour session left"));
        assert!(r.iter().find(|x| x.account_id == "a").unwrap().reason.contains("limit reached"));
        assert!(r.iter().all(|x| (0.0..=100.0).contains(&x.score)));
    }

    #[test]
    fn long_refactor_prefers_max_quick_question_prefers_small() {
        let now = Utc::now();
        let accts = vec![account("max", "Max 20x"), account("pro", "Pro")];
        let latest = HashMap::from([
            ("max".to_string(), snap("max", vec![window("five_hour", 20.0, Some(3), 300, now)])),
            ("pro".to_string(), snap("pro", vec![window("five_hour", 60.0, Some(3), 300, now)])),
        ]);
        assert_eq!(recommend(TaskType::LongRefactor, None, &accts, &latest, now)[0].account_id, "max");
        assert_eq!(recommend(TaskType::QuickQuestion, None, &accts, &latest, now)[0].account_id, "pro");
    }

    #[test]
    fn weekly_resetting_soon_gets_bonus_and_reason() {
        let now = Utc::now();
        let accts = vec![account("a", "Pro"), account("b", "Pro")];
        let latest = HashMap::from([
            ("a".to_string(), snap("a", vec![window("seven_day", 40.0, Some(10), 10080, now)])),
            ("b".to_string(), snap("b", vec![window("seven_day", 40.0, Some(100), 10080, now)])),
        ]);
        let r = recommend(TaskType::Any, None, &accts, &latest, now);
        assert_eq!(r[0].account_id, "a");
        assert!(r[0].reason.contains("Use it before it resets"), "{}", r[0].reason);
    }

    #[test]
    fn provider_filter_and_tags() {
        let now = Utc::now();
        let mut tagged = account("t", "Pro");
        tagged.tags = vec!["Research".into()];
        let mut other = account("o", "Pro");
        other.provider_id = ProviderId::Codex;
        let accts = vec![account("u", "Pro"), tagged, other];
        let r = recommend(TaskType::Research, Some(ProviderId::Claude), &accts, &HashMap::new(), now);
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].account_id, "t");
        assert_eq!(tier(&account("x", "Copilot Pro+")), 3);
        assert_eq!(tier(&account("x", "Plus")), 1);
    }
}
