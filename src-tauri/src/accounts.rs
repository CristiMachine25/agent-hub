//! Accounts: creating, importing, editing and removing them, and applying their sharing. The
//! Tauri commands in `commands.rs` and the setup import in `transfer.rs` both go through here.
use crate::model::*;
use crate::providers::Adapter;
use crate::store::AppState;
use crate::{events, knowledge, notify, sharing, usage, util};
use anyhow::{anyhow, Context};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::AppHandle;

/// Account identity colors: the same validated set as `ACCOUNT_COLORS` in src/lib/providers.ts (see docs/DESIGN.md).
const PALETTE: [&str; 8] = ["#3987E5", "#D95926", "#199E70", "#C98500", "#D55181", "#9085E9", "#E66767", "#008300"];

/// First palette color no account uses yet, else rotate by count.
fn next_color(accounts: &[Account]) -> String {
    PALETTE.iter().find(|c| !accounts.iter().any(|a| a.color.eq_ignore_ascii_case(c))).unwrap_or(&PALETTE[accounts.len() % PALETTE.len()]).to_string()
}

fn is_hex_color(s: &str) -> bool {
    let h = s.strip_prefix('#').unwrap_or("");
    matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit())
}

fn clean_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags.into_iter().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()) {
        if !out.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
            out.push(t);
        }
    }
    out
}

pub(crate) async fn emit_accounts(app: &AppHandle, state: &AppState) {
    events::emit(app, &events::AccountsChanged(&state.accounts.read().await));
}

/// Apply sharing off the async runtime; surface backups/copies as an in-app notice.
/// Knowledge links from the Agent Hub shared folder, history from `shared_from`.
pub(crate) async fn apply_sharing(app: &AppHandle, state: &AppState, account: &Account, adapter: Adapter) {
    let acct = account.clone();
    let shared = state.shared_dir(account.provider_id);
    let report = tokio::task::spawn_blocking(move || knowledge::apply_for_account(&acct, adapter.as_ref(), &shared)).await;
    let Ok(report) = report else { return };
    for n in &report.notes {
        log::info!("sharing {}: {n}", account.label);
    }
    let notable: Vec<&String> = report.notes.iter().filter(|n| n.contains("moved") || n.contains("copied")).collect();
    if !notable.is_empty() {
        let body = notable.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n");
        let n = AppNotification {
            id: uuid::Uuid::new_v4().to_string(),
            kind: NotificationKind::Info,
            account_id: Some(account.id.clone()),
            title: format!("Sharing updated for {}", notify::display_name(account)),
            body,
            created_at: util::now_iso(),
            read: false,
        };
        notify::deliver(app, state, vec![n]).await;
    }
}

/// Refresh one account's usage in the background (after create/import/login).
fn refresh_later(app: &AppHandle, state: &Arc<AppState>, id: Option<String>) {
    let (app, state) = (app.clone(), state.clone());
    tauri::async_runtime::spawn(async move {
        usage::poll(&app, &state, id.as_deref()).await;
    });
}

/// Short, readable name for an imported home: "Claude main" for the stock home,
/// "Claude H2" for ~/.claude-h2. The email stays visible next to it in the UI.
fn import_label(d: &DetectedAccount) -> String {
    let short = d.provider_id.name();
    if d.is_default_home {
        return format!("{short} main");
    }
    let dir = Path::new(&d.home_dir).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let suffix = dir.trim_start_matches('.').trim_start_matches(d.provider_id.as_str()).trim_matches(|c: char| c == '-' || c == '_' || c == '.');
    if suffix.is_empty() {
        return util::non_empty(d.email.clone()).unwrap_or_else(|| short.to_string());
    }
    let pretty: Vec<String> = suffix
        .split(['-', '_', '.'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
        })
        .collect();
    format!("{short} {}", pretty.join(" "))
}

#[cfg(test)]
mod import_label_tests {
    use super::*;

    fn found(provider: ProviderId, home: &str, default: bool) -> DetectedAccount {
        DetectedAccount {
            provider_id: provider,
            home_dir: home.into(),
            is_default_home: default,
            email: Some("me@example.com".into()),
            plan: None,
            logged_in: true,
            already_managed: false,
            hint: None,
        }
    }

    #[test]
    fn names_side_by_side_homes_after_their_folder() {
        assert_eq!(import_label(&found(ProviderId::Claude, "/home/x/.claude-h2", false)), "Claude H2");
        assert_eq!(import_label(&found(ProviderId::Codex, "/home/x/.codex-work_pro", false)), "Codex Work Pro");
        assert_eq!(import_label(&found(ProviderId::Claude, "/home/x/.claude", true)), "Claude main");
        assert_eq!(import_label(&found(ProviderId::Gemini, "/home/x/.gemini-", false)), "me@example.com");
    }
}

/// Existing homes are imported as-is: sharing starts off so nothing in them is moved or linked
/// until the user turns it on.
pub(crate) async fn import_accounts(app: &AppHandle, state: &Arc<AppState>, found: Vec<DetectedAccount>) -> anyhow::Result<Vec<Account>> {
    let mut created = Vec::new();
    for d in found {
        let home = PathBuf::from(&d.home_dir);
        let key = util::path_key(&home);
        if state.accounts.read().await.iter().any(|a| util::path_key(Path::new(&a.home_dir)) == key) {
            continue;
        }
        let adapter = state.adapter(d.provider_id)?;
        let default_home = adapter.default_home();
        let is_default = util::path_key(&default_home) == key;
        if !is_default {
            if let Err(e) = adapter.init_home(&home).await {
                log::warn!("init_home {}: {e:#}", home.display());
            }
        }
        let mut accounts = state.accounts.write().await;
        let account = Account {
            id: uuid::Uuid::new_v4().to_string(),
            provider_id: d.provider_id,
            label: import_label(&d),
            email: d.email.clone(),
            plan: d.plan.clone(),
            home_dir: d.home_dir.clone(),
            is_default_home: is_default,
            shared_from: (!is_default && default_home.is_dir()).then(|| default_home.to_string_lossy().into_owned()),
            sharing: SharingOptions::NONE,
            color: next_color(&accounts),
            tags: vec![],
            notes: d.hint.clone(),
            status: if d.logged_in { AccountStatus::Ok } else { AccountStatus::NeedsLogin },
            status_message: None,
            created_at: util::now_iso(),
            last_launched_at: None,
            launch_count: 0,
            monthly_cost: None,
            pinned: None,
        };
        accounts.push(account.clone());
        created.push(account);
    }
    if !created.is_empty() {
        state.save_accounts().await?;
        emit_accounts(app, state).await;
        refresh_later(app, state, None);
    }
    Ok(created)
}

/// A unique new home under the profiles dir: <profiles>/<provider>/<slug>[-n].
pub(crate) async fn new_home_dir(state: &AppState, provider: ProviderId, label: &str) -> PathBuf {
    let base = state.profiles_dir().await.join(provider.as_str());
    let slug = Some(util::slugify(label)).filter(|s| !s.is_empty()).unwrap_or_else(|| "account".into());
    let taken: Vec<String> = state.accounts.read().await.iter().map(|a| util::path_key(Path::new(&a.home_dir))).collect();
    let mut n = 1;
    loop {
        let name = if n == 1 { slug.clone() } else { format!("{slug}-{n}") };
        let candidate = base.join(name);
        if !candidate.exists() && !taken.contains(&util::path_key(&candidate)) {
            return candidate;
        }
        n += 1;
    }
}

pub(crate) async fn create_account(app: &AppHandle, state: &Arc<AppState>, input: NewAccountInput) -> anyhow::Result<Account> {
    let label = input.label.trim().to_string();
    if label.is_empty() {
        return Err(anyhow!("Give the account a name"));
    }
    let adapter = state.adapter(input.provider_id)?;
    let home = match util::non_empty(input.home_dir.clone()) {
        Some(h) => PathBuf::from(h),
        None => new_home_dir(state, input.provider_id, &label).await,
    };
    let key = util::path_key(&home);
    if let Some(other) = state.accounts.read().await.iter().find(|a| util::path_key(Path::new(&a.home_dir)) == key) {
        return Err(anyhow!("That folder is already used by {}", other.label));
    }
    std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
    adapter.init_home(&home).await.context("preparing the account folder")?;

    let default_home = adapter.default_home();
    let is_default = util::path_key(&default_home) == key;
    let shared_from =
        util::non_empty(input.shared_from.clone()).or_else(|| (!is_default && default_home.is_dir()).then(|| default_home.to_string_lossy().into_owned()));
    let identity = adapter.read_identity(&home).await;
    let default_sharing = state.settings.read().await.default_sharing;

    let account = {
        let mut accounts = state.accounts.write().await;
        let account = Account {
            id: uuid::Uuid::new_v4().to_string(),
            provider_id: input.provider_id,
            label,
            email: identity.email,
            plan: identity.plan.or_else(|| util::non_empty(input.plan.clone())),
            home_dir: home.to_string_lossy().into_owned(),
            is_default_home: is_default,
            shared_from,
            sharing: input.sharing.unwrap_or(default_sharing),
            color: util::non_empty(input.color).filter(|c| is_hex_color(c)).unwrap_or_else(|| next_color(&accounts)),
            tags: clean_tags(input.tags.unwrap_or_default()),
            notes: None,
            status: if identity.logged_in { AccountStatus::Ok } else { AccountStatus::NeedsLogin },
            status_message: None,
            created_at: util::now_iso(),
            last_launched_at: None,
            launch_count: 0,
            monthly_cost: input.monthly_cost.filter(|c| c.is_finite() && *c >= 0.0),
            pinned: None,
        };
        accounts.push(account.clone());
        account
    };
    state.save_accounts().await?;
    apply_sharing(app, state, &account, adapter).await;
    emit_accounts(app, state).await;
    refresh_later(app, state, Some(account.id.clone()));
    Ok(account)
}

pub(crate) async fn update_account(app: &AppHandle, state: &Arc<AppState>, id: &str, mut patch: serde_json::Value) -> anyhow::Result<Account> {
    if let Some(obj) = patch.as_object_mut() {
        for k in ["id", "providerId", "createdAt", "isDefaultHome", "launchCount", "lastLaunchedAt"] {
            obj.remove(k);
        }
    } else {
        return Err(anyhow!("Patch must be an object"));
    }
    let (updated, sharing_changed) = {
        let mut accounts = state.accounts.write().await;
        let idx = accounts.iter().position(|a| a.id == id).ok_or_else(|| anyhow!("Account not found"))?;
        let old = accounts[idx].clone();
        let mut new = merge_account(&old, &patch)?;

        new.label = new.label.trim().to_string();
        if new.label.is_empty() {
            return Err(anyhow!("Give the account a name"));
        }
        if !is_hex_color(&new.color) {
            new.color = old.color.clone();
        }
        new.tags = clean_tags(new.tags);
        new.shared_from = util::non_empty(new.shared_from);
        new.home_dir = util::non_empty(Some(new.home_dir)).ok_or_else(|| anyhow!("The account folder cannot be empty"))?;
        let key = util::path_key(Path::new(&new.home_dir));
        if accounts.iter().any(|a| a.id != new.id && util::path_key(Path::new(&a.home_dir)) == key) {
            return Err(anyhow!("That folder is already used by another account"));
        }
        let default_home = state.adapter(new.provider_id)?.default_home();
        new.is_default_home = util::path_key(&default_home) == key;
        new.monthly_cost = new.monthly_cost.filter(|c| c.is_finite() && *c >= 0.0);

        let sharing_changed = new.sharing != old.sharing || new.shared_from != old.shared_from || key != util::path_key(Path::new(&old.home_dir));
        accounts[idx] = new.clone();
        (new, sharing_changed)
    };
    state.save_accounts().await?;
    if sharing_changed {
        apply_sharing(app, state, &updated, state.adapter(updated.provider_id)?).await;
    }
    emit_accounts(app, state).await;
    crate::refresh_tray(app); // labels appear in the tray menu
    Ok(updated)
}

/// The account with `patch` merged over it. A `null` clears an optional field; a missing key keeps it.
fn merge_account(old: &Account, patch: &serde_json::Value) -> anyhow::Result<Account> {
    let mut v = serde_json::to_value(old)?;
    util::merge_json(&mut v, patch);
    serde_json::from_value(v).context("invalid account fields")
}

/// True when the account's folder was deleted too.
pub(crate) async fn remove_account(app: &AppHandle, state: &Arc<AppState>, id: &str, delete_home: bool) -> anyhow::Result<bool> {
    let account = {
        let mut accounts = state.accounts.write().await;
        let idx = accounts.iter().position(|a| a.id == id).ok_or_else(|| anyhow!("Account not found"))?;
        accounts.remove(idx)
    };
    state.save_accounts().await?;
    state.latest.write().await.remove(id);
    state.activity.write().await.remove(id);
    let _ = state.save_latest().await;
    let _ = state.save_activity().await;

    let mut deleted = false;
    if delete_home {
        let adapter = state.adapter(account.provider_id)?;
        let home = PathBuf::from(&account.home_dir);
        let profiles = util::path_key(&state.profiles_dir().await);
        let inside_profiles = util::path_key(&home).starts_with(&format!("{profiles}/"));
        let is_default = account.is_default_home || util::path_key(&adapter.default_home()) == util::path_key(&home);
        if inside_profiles && !is_default && home.is_dir() {
            // Remove our links first so deleting the folder can never reach shared data.
            let mut unshared = account.clone();
            unshared.sharing = SharingOptions::NONE;
            let a2 = adapter.clone();
            let h2 = home.clone();
            let shared = state.shared_dir(account.provider_id);
            tokio::task::spawn_blocking(move || {
                sharing::apply_for_account(&unshared, a2.as_ref(), Some(&shared));
                std::fs::remove_dir_all(&h2)
            })
            .await?
            .with_context(|| format!("deleting {}", home.display()))?;
            deleted = true;
        } else {
            log::info!("not deleting {}: outside the profiles folder or a default home", home.display());
        }
    }
    emit_accounts(app, state).await;
    crate::refresh_tray(app);
    Ok(deleted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_and_tags() {
        assert!(is_hex_color("#a1B2c3"));
        assert!(!is_hex_color("red"));
        assert_eq!(clean_tags(vec![" a ".into(), "A".into(), "".into(), "b".into()]), vec!["a", "b"]);
        assert_eq!(next_color(&[]), PALETTE[0]);
    }

    #[test]
    fn palette_matches_the_ui() {
        let ts = include_str!("../../src/lib/providers.ts");
        let list = ts.split("ACCOUNT_COLORS = [").nth(1).and_then(|r| r.split(']').next()).expect("ACCOUNT_COLORS in providers.ts");
        let ui: Vec<&str> = list.split(',').map(|c| c.trim().trim_matches('\'')).filter(|c| !c.is_empty()).collect();
        assert_eq!(ui, PALETTE);
    }

    #[test]
    fn account_patch_clears_null_fields_and_keeps_missing_ones() {
        let mut old = crate::recommend::tests::account("a", "Max 5x");
        old.notes = Some("keep me".into());
        old.monthly_cost = Some(100.0);
        let new = merge_account(&old, &serde_json::json!({ "monthlyCost": null, "plan": null, "label": "Renamed" })).unwrap();
        assert_eq!(new.monthly_cost, None);
        assert_eq!(new.plan, None);
        assert_eq!(new.notes.as_deref(), Some("keep me"));
        assert_eq!(new.label, "Renamed");
        let new = merge_account(&new, &serde_json::json!({ "notes": null })).unwrap();
        assert_eq!(new.notes, None);
        assert!(merge_account(&old, &serde_json::json!({ "label": null })).is_err());
    }
}
