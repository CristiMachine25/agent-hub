//! Tauri commands: the HopperApi from src/shared/types.ts. Names and argument names must match
//! COMMANDS there (Tauri maps camelCase JS keys to these snake_case parameters).
use crate::model::*;
use crate::providers::{Adapter, CommandSpec};
use crate::store::AppState;
use crate::{launcher, notify, pty, recommend, sharing, usage, util};
use anyhow::{anyhow, Context};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

type Res<T> = Result<T, String>;
type St<'a> = State<'a, Arc<AppState>>;
type Terms<'a> = State<'a, pty::Terminals>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

const PALETTE: [&str; 10] = [
    "#6366f1", "#f59e0b", "#10b981", "#ef4444", "#8b5cf6", "#06b6d4", "#ec4899", "#84cc16", "#f97316", "#14b8a6",
];

/// First palette color no account uses yet, else rotate by count.
fn next_color(accounts: &[Account]) -> String {
    PALETTE
        .iter()
        .find(|c| !accounts.iter().any(|a| a.color.eq_ignore_ascii_case(c)))
        .unwrap_or(&PALETTE[accounts.len() % PALETTE.len()])
        .to_string()
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

fn non_empty(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub async fn emit_accounts(app: &AppHandle, state: &AppState) {
    let _ = app.emit("accounts-changed", &*state.accounts.read().await);
}

/// Apply sharing off the async runtime; surface backups/copies as an in-app notice.
async fn apply_sharing(app: &AppHandle, state: &AppState, account: &Account, adapter: Adapter) {
    let acct = account.clone();
    let report = tokio::task::spawn_blocking(move || sharing::apply_for_account(&acct, adapter.as_ref())).await;
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

// ---------- providers ----------

#[tauri::command]
pub async fn providers_list(state: St<'_>) -> Res<Vec<ProviderInfo>> {
    let mut out = Vec::new();
    for a in &state.adapters {
        out.push(a.info().await);
    }
    Ok(out)
}

// ---------- accounts ----------

#[tauri::command]
pub async fn accounts_list(state: St<'_>) -> Res<Vec<Account>> {
    Ok(state.accounts.read().await.clone())
}

#[tauri::command]
pub async fn accounts_detect(state: St<'_>) -> Res<Vec<DetectedAccount>> {
    let mut set = tokio::task::JoinSet::new();
    for a in &state.adapters {
        let a = Arc::clone(a);
        set.spawn(async move { a.detect_accounts().await });
    }
    let mut found = Vec::new();
    while let Some(r) = set.join_next().await {
        found.extend(r.unwrap_or_default());
    }
    let managed: Vec<String> = state.accounts.read().await.iter().map(|a| util::path_key(Path::new(&a.home_dir))).collect();
    for d in &mut found {
        d.already_managed = managed.contains(&util::path_key(Path::new(&d.home_dir)));
    }
    found.sort_by(|a, b| (a.provider_id.as_str(), !a.is_default_home, &a.home_dir).cmp(&(b.provider_id.as_str(), !b.is_default_home, &b.home_dir)));
    Ok(found)
}

/// Short, readable name for an imported home: "Claude main" for the stock home,
/// "Claude H2" for ~/.claude-h2. The email stays visible next to it in the UI.
fn import_label(d: &DetectedAccount) -> String {
    let short = match d.provider_id {
        ProviderId::Claude => "Claude",
        ProviderId::Codex => "Codex",
        ProviderId::Gemini => "Gemini",
        ProviderId::Copilot => "Copilot",
    };
    if d.is_default_home {
        return format!("{short} main");
    }
    let dir = Path::new(&d.home_dir).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let suffix = dir
        .trim_start_matches('.')
        .trim_start_matches(d.provider_id.as_str())
        .trim_matches(|c: char| c == '-' || c == '_' || c == '.');
    if suffix.is_empty() {
        return non_empty(d.email.clone()).unwrap_or_else(|| short.to_string());
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
#[tauri::command]
pub async fn accounts_import(app: AppHandle, state: St<'_>, found: Vec<DetectedAccount>) -> Res<Vec<Account>> {
    let state = state.inner().clone();
    let mut created = Vec::new();
    for d in found {
        let home = PathBuf::from(&d.home_dir);
        let key = util::path_key(&home);
        if state.accounts.read().await.iter().any(|a| util::path_key(Path::new(&a.home_dir)) == key) {
            continue;
        }
        let adapter = state.adapter(d.provider_id).map_err(err)?;
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
            sharing: SharingOptions { skills: false, plugins: false, agents: false, instructions: false, settings: false, mcp: false, history: false },
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
        state.save_accounts().await.map_err(err)?;
        emit_accounts(&app, &state).await;
        refresh_later(&app, &state, None);
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

#[tauri::command]
pub async fn accounts_create(app: AppHandle, state: St<'_>, input: NewAccountInput) -> Res<Account> {
    let state = state.inner().clone();
    create_account(&app, &state, input).await.map_err(err)
}

pub(crate) async fn create_account(app: &AppHandle, state: &Arc<AppState>, input: NewAccountInput) -> anyhow::Result<Account> {
    let label = input.label.trim().to_string();
    if label.is_empty() {
        return Err(anyhow!("Give the account a name"));
    }
    let adapter = state.adapter(input.provider_id)?;
    let home = match non_empty(input.home_dir.clone()) {
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
    let shared_from = non_empty(input.shared_from.clone())
        .or_else(|| (!is_default && default_home.is_dir()).then(|| default_home.to_string_lossy().into_owned()));
    let identity = adapter.read_identity(&home).await;
    let default_sharing = state.settings.read().await.default_sharing;

    let account = {
        let mut accounts = state.accounts.write().await;
        let account = Account {
            id: uuid::Uuid::new_v4().to_string(),
            provider_id: input.provider_id,
            label,
            email: identity.email,
            plan: identity.plan.or_else(|| non_empty(input.plan.clone())),
            home_dir: home.to_string_lossy().into_owned(),
            is_default_home: is_default,
            shared_from,
            sharing: input.sharing.unwrap_or(default_sharing),
            color: non_empty(input.color).filter(|c| is_hex_color(c)).unwrap_or_else(|| next_color(&accounts)),
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

#[tauri::command]
pub async fn accounts_update(app: AppHandle, state: St<'_>, id: String, patch: serde_json::Value) -> Res<Account> {
    let state = state.inner().clone();
    update_account(&app, &state, &id, patch).await.map_err(err)
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
        let mut v = serde_json::to_value(&old)?;
        util::merge_json(&mut v, &patch);
        let mut new: Account = serde_json::from_value(v).context("invalid account fields")?;

        new.label = new.label.trim().to_string();
        if new.label.is_empty() {
            return Err(anyhow!("Give the account a name"));
        }
        if !is_hex_color(&new.color) {
            new.color = old.color.clone();
        }
        new.tags = clean_tags(new.tags);
        new.shared_from = non_empty(new.shared_from);
        new.home_dir = non_empty(Some(new.home_dir)).ok_or_else(|| anyhow!("The account folder cannot be empty"))?;
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

#[tauri::command]
pub async fn accounts_remove(app: AppHandle, state: St<'_>, id: String, delete_home: bool) -> Res<()> {
    let state = state.inner().clone();
    remove_account(&app, &state, &id, delete_home).await.map_err(err)
}

async fn remove_account(app: &AppHandle, state: &Arc<AppState>, id: &str, delete_home: bool) -> anyhow::Result<()> {
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

    if delete_home {
        let adapter = state.adapter(account.provider_id)?;
        let home = PathBuf::from(&account.home_dir);
        let profiles = util::path_key(&state.profiles_dir().await);
        let inside_profiles = util::path_key(&home).starts_with(&format!("{profiles}/"));
        let is_default = account.is_default_home || util::path_key(&adapter.default_home()) == util::path_key(&home);
        if inside_profiles && !is_default && home.is_dir() {
            // Remove our links first so deleting the folder can never reach shared data.
            let mut unshared = account.clone();
            unshared.sharing = SharingOptions { skills: false, plugins: false, agents: false, instructions: false, settings: false, mcp: false, history: false };
            let a2 = adapter.clone();
            let h2 = home.clone();
            tokio::task::spawn_blocking(move || {
                sharing::apply_for_account(&unshared, a2.as_ref());
                std::fs::remove_dir_all(&h2)
            })
            .await?
            .with_context(|| format!("deleting {}", home.display()))?;
        } else {
            log::info!("not deleting {}: outside the profiles folder or a default home", home.display());
        }
    }
    emit_accounts(app, state).await;
    crate::refresh_tray(app);
    Ok(())
}

#[tauri::command]
pub async fn accounts_apply_sharing(app: AppHandle, state: St<'_>, id: String) -> Res<Account> {
    let state = state.inner().clone();
    let account = state.account(&id).await.ok_or("Account not found")?;
    let adapter = state.adapter(account.provider_id).map_err(err)?;
    apply_sharing(&app, &state, &account, adapter).await;
    Ok(account)
}

// ---------- usage ----------

#[tauri::command]
pub async fn usage_latest(state: St<'_>) -> Res<HashMap<String, UsageSnapshot>> {
    Ok(state.latest.read().await.clone())
}

#[tauri::command]
pub async fn usage_history(state: St<'_>, account_id: Option<String>, days: u32) -> Res<Vec<UsageHistoryPoint>> {
    Ok(state.read_history(account_id.as_deref(), days).await)
}

#[tauri::command]
pub async fn usage_activity(state: St<'_>, account_id: Option<String>, days: u32) -> Res<Vec<DailyActivity>> {
    Ok(usage::activity(&state, account_id.as_deref(), days).await)
}

#[tauri::command]
pub async fn usage_refresh(app: AppHandle, state: St<'_>, account_id: Option<String>) -> Res<HashMap<String, UsageSnapshot>> {
    Ok(usage::poll(&app, &state, account_id.as_deref()).await)
}

#[tauri::command]
pub async fn usage_set_manual(app: AppHandle, state: St<'_>, account_id: String, windows: Vec<UsageWindow>) -> Res<UsageSnapshot> {
    usage::set_manual(&app, &state, &account_id, windows).await.map_err(err)
}

// ---------- launch ----------

#[tauri::command]
pub async fn launch(app: AppHandle, state: St<'_>, req: LaunchRequest) -> Res<LaunchResult> {
    let state = state.inner().clone();
    launch_account(&app, &state, req).await.map_err(err)
}

fn failed(msg: impl Into<String>) -> LaunchResult {
    LaunchResult { ok: false, error: Some(msg.into()), command_line: None, note: None }
}

fn pick_cwd(req_cwd: Option<String>, default_cwd: Option<String>) -> String {
    non_empty(req_cwd)
        .into_iter()
        .chain(non_empty(default_cwd))
        .find(|c| Path::new(c).is_dir())
        .unwrap_or_else(|| util::home().to_string_lossy().into_owned())
}

/// Command for a terminal target (CLI or login) with its cwd filled in. Re-applies sharing first
/// so skills/plugins added to the shared home since the last launch are picked up.
async fn terminal_command(
    state: &AppState,
    account: &Account,
    adapter: &Adapter,
    login: bool,
    resume: bool,
    extra_args: &[String],
    req_cwd: Option<String>,
) -> anyhow::Result<CommandSpec> {
    let mut spec = if login {
        adapter.login_command(account).await?
    } else {
        adapter.cli_command(account, resume, extra_args).await?
    };
    if spec.cwd.is_none() {
        let default_cwd = state.settings.read().await.default_cwd.clone();
        spec.cwd = Some(pick_cwd(req_cwd, default_cwd));
    }
    if account.shared_from.is_some() {
        let (a, ad) = (account.clone(), adapter.clone());
        let _ = tokio::task::spawn_blocking(move || sharing::apply_for_account(&a, ad.as_ref())).await;
    }
    Ok(spec)
}

/// Bump lastLaunchedAt/launchCount and tell the UI.
async fn record_launch(app: &AppHandle, state: &AppState, account_id: &str) -> anyhow::Result<()> {
    {
        let mut accounts = state.accounts.write().await;
        if let Some(a) = accounts.iter_mut().find(|a| a.id == account_id) {
            a.last_launched_at = Some(util::now_iso());
            a.launch_count = a.launch_count.saturating_add(1);
        }
    }
    state.save_accounts().await?;
    emit_accounts(app, state).await;
    Ok(())
}

/// Shared by the `launch` command and the tray menu.
pub async fn launch_account(app: &AppHandle, state: &Arc<AppState>, req: LaunchRequest) -> anyhow::Result<LaunchResult> {
    let account = state.account(&req.account_id).await.ok_or_else(|| anyhow!("Account not found"))?;
    let adapter = state.adapter(account.provider_id)?;
    let extra = req.extra_args.clone().unwrap_or_default();

    let result = if req.target == LaunchTarget::Desktop {
        let spec = match adapter.desktop_command(&account).await {
            Ok(Some(spec)) => spec,
            Ok(None) => {
                return Ok(failed(format!(
                    "{} has no desktop app that can run with a separate account",
                    notify::provider_name(account.provider_id)
                )))
            }
            Err(e) => return Ok(failed(format!("{e:#}"))),
        };
        tokio::task::spawn_blocking(move || launcher::launch_desktop(&spec)).await?
    } else {
        let login = req.target == LaunchTarget::Login;
        let spec = match terminal_command(state, &account, &adapter, login, req.resume.unwrap_or(false), &extra, req.cwd.clone()).await {
            Ok(s) => s,
            Err(e) => return Ok(failed(format!("{e:#}"))),
        };
        let title = notify::display_name(&account);
        let terminal = state.settings.read().await.terminal.clone();
        tokio::task::spawn_blocking(move || launcher::launch_in_terminal(&spec, &title, &terminal)).await?
    };

    if result.ok {
        record_launch(app, state, &account.id).await?;
        if req.target == LaunchTarget::Login || account.status == AccountStatus::NeedsLogin {
            watch_sign_in(app, state, &account);
        }
    }
    Ok(result)
}

/// How often and how long to look for a finished sign-in after opening a login.
const SIGN_IN_CHECK_EVERY: Duration = Duration::from_secs(3);
const SIGN_IN_WATCH_FOR: Duration = Duration::from_secs(15 * 60);

/// Accounts with a sign-in watch running, so repeated launches don't stack watchers.
fn sign_in_watches() -> &'static Mutex<HashSet<String>> {
    static W: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    W.get_or_init(Default::default)
}

/// After a login is opened, read the profile's sign-in every few seconds. As soon as it shows a new
/// login (signed in now, or as someone else), refresh that account so the UI updates by itself.
pub fn watch_sign_in(app: &AppHandle, state: &Arc<AppState>, account: &Account) {
    if !sign_in_watches().lock().unwrap().insert(account.id.clone()) {
        return;
    }
    let (app, state, id, home, provider) = (app.clone(), state.clone(), account.id.clone(), PathBuf::from(&account.home_dir), account.provider_id);
    tauri::async_runtime::spawn(async move {
        if let Ok(adapter) = state.adapter(provider) {
            let before = adapter.read_identity(&home).await;
            let started = std::time::Instant::now();
            while started.elapsed() < SIGN_IN_WATCH_FOR && state.account(&id).await.is_some() {
                tokio::time::sleep(SIGN_IN_CHECK_EVERY).await;
                let now = adapter.read_identity(&home).await;
                if now.logged_in && (!before.logged_in || now.email != before.email) {
                    log::info!("sign-in finished for {id}, refreshing");
                    usage::poll(&app, &state, Some(&id)).await;
                    break;
                }
            }
        }
        sign_in_watches().lock().unwrap().remove(&id);
    });
}

// ---------- embedded terminals ----------

/// Sends pty output to the UI as `terminal-output` / `terminal-exit` events.
struct EventSink(AppHandle);

impl pty::PtySink for EventSink {
    fn output(&self, id: &str, data: String) {
        let _ = self.0.emit("terminal-output", serde_json::json!({ "id": id, "data": data }));
    }
    fn exit(&self, id: &str, code: Option<i32>) {
        let _ = self.0.emit("terminal-exit", serde_json::json!({ "id": id, "code": code }));
    }
}

#[tauri::command]
pub async fn terminal_spawn(app: AppHandle, state: St<'_>, terms: Terms<'_>, req: TerminalSpawnRequest) -> Res<TerminalInfo> {
    let state = state.inner().clone();
    let terms = terms.inner().clone();
    spawn_terminal(&app, &state, terms, req).await.map_err(err)
}

async fn spawn_terminal(app: &AppHandle, state: &Arc<AppState>, terms: pty::Terminals, req: TerminalSpawnRequest) -> anyhow::Result<TerminalInfo> {
    let account = state.account(&req.account_id).await.ok_or_else(|| anyhow!("Account not found"))?;
    let adapter = state.adapter(account.provider_id)?;
    let extra = req.extra_args.clone().unwrap_or_default();
    let login = req.target == TerminalTarget::Login;
    let spec = terminal_command(state, &account, &adapter, login, req.resume.unwrap_or(false), &extra, req.cwd.clone()).await?;
    let title = notify::display_name(&account);
    let spawn = pty::PtySpawn { account_id: account.id.clone(), title, spec, cols: req.cols, rows: req.rows };
    let sink: Arc<dyn pty::PtySink> = Arc::new(EventSink(app.clone()));
    let info = tokio::task::spawn_blocking(move || terms.spawn(spawn, sink)).await??;
    record_launch(app, state, &account.id).await?;
    if login || account.status == AccountStatus::NeedsLogin {
        watch_sign_in(app, state, &account);
    }
    Ok(info)
}

/// Runs a blocking pty call off the async runtime (writes can block on a full pipe).
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> Res<T> {
    tokio::task::spawn_blocking(f).await.map_err(|e| e.to_string())?.map_err(err)
}

#[tauri::command]
pub async fn terminal_write(terms: Terms<'_>, id: String, data: String) -> Res<()> {
    let terms = terms.inner().clone();
    blocking(move || terms.write(&id, data.as_bytes())).await
}

#[tauri::command]
pub async fn terminal_resize(terms: Terms<'_>, id: String, cols: u16, rows: u16) -> Res<()> {
    let terms = terms.inner().clone();
    blocking(move || terms.resize(&id, cols, rows)).await
}

#[tauri::command]
pub async fn terminal_kill(terms: Terms<'_>, id: String) -> Res<()> {
    let terms = terms.inner().clone();
    blocking(move || terms.kill(&id)).await
}

#[tauri::command]
pub async fn terminal_list(terms: Terms<'_>) -> Res<Vec<TerminalInfo>> {
    Ok(terms.list())
}

#[tauri::command]
pub async fn terminal_scrollback(terms: Terms<'_>, id: String) -> Res<String> {
    terms.scrollback(&id).ok_or_else(|| "Terminal not found".into())
}

// ---------- recommendations, notifications, settings, system ----------

#[tauri::command]
pub async fn recommend(state: St<'_>, task: TaskType, provider_id: Option<ProviderId>) -> Res<Vec<Recommendation>> {
    let accounts = state.accounts.read().await.clone();
    let latest = state.latest.read().await.clone();
    Ok(recommend::recommend(task, provider_id, &accounts, &latest, chrono::Utc::now()))
}

#[tauri::command]
pub async fn notifications_list(state: St<'_>) -> Res<Vec<AppNotification>> {
    Ok(state.notifications.read().await.clone())
}

#[tauri::command]
pub async fn notifications_mark_read(state: St<'_>, id: String) -> Res<()> {
    {
        let mut list = state.notifications.write().await;
        for n in list.iter_mut().filter(|n| id == "all" || n.id == id) {
            n.read = true;
        }
    }
    state.save_notifications().await.map_err(err)
}

#[tauri::command]
pub async fn settings_get(state: St<'_>) -> Res<AppSettings> {
    Ok(state.settings.read().await.clone())
}

#[tauri::command]
pub async fn settings_set(app: AppHandle, state: St<'_>, patch: serde_json::Value) -> Res<AppSettings> {
    apply_settings_patch(&app, &state, patch).await.map_err(err)
}

/// Merge a partial settings object, clamp the numbers, save, and wake the poller if its timing changed.
pub(crate) async fn apply_settings_patch(app: &AppHandle, state: &AppState, patch: serde_json::Value) -> anyhow::Result<AppSettings> {
    if !patch.is_object() {
        return Err(anyhow!("Settings patch must be an object"));
    }
    let (new, wake) = {
        let mut settings = state.settings.write().await;
        let mut v = serde_json::to_value(&*settings)?;
        util::merge_json(&mut v, &patch);
        let mut new: AppSettings = serde_json::from_value(v).map_err(|e| anyhow!("Invalid settings: {e}"))?;
        new.poll_interval_minutes = new.poll_interval_minutes.clamp(1, 24 * 60);
        let n = &mut new.notifications;
        n.nearly_exhausted_percent = n.nearly_exhausted_percent.clamp(1.0, 99.0);
        n.use_it_or_lose_it_hours = n.use_it_or_lose_it_hours.clamp(1.0, 24.0 * 31.0);
        n.use_it_or_lose_it_min_left_percent = n.use_it_or_lose_it_min_left_percent.clamp(0.0, 100.0);
        new.default_cwd = non_empty(new.default_cwd);
        new.profiles_dir = non_empty(new.profiles_dir);
        let wake = new.poll_interval_minutes != settings.poll_interval_minutes || new.live_usage != settings.live_usage;
        if new.launch_at_login != settings.launch_at_login {
            crate::set_autostart(app, new.launch_at_login)?;
        }
        *settings = new.clone();
        (new, wake)
    };
    state.save_settings().await?;
    if wake {
        state.poll_wake.notify_one();
    }
    crate::refresh_tray(app);
    Ok(new)
}

// ---------- moving the setup to another computer ----------

#[tauri::command]
pub async fn transfer_export(state: St<'_>, path: String, opts: crate::transfer::ExportOptions) -> Res<crate::transfer::ExportSummary> {
    crate::transfer::export(&state, Path::new(&path), opts).await.map_err(err)
}

#[tauri::command]
pub async fn transfer_preview(state: St<'_>, path: String, passphrase: Option<String>) -> Res<crate::transfer::ImportPreview> {
    let state = state.inner().clone();
    crate::transfer::preview(&state, Path::new(&path), passphrase).await.map_err(err)
}

#[tauri::command]
pub async fn transfer_import(
    app: AppHandle,
    state: St<'_>,
    path: String,
    passphrase: Option<String>,
    choice: crate::transfer::ImportChoice,
) -> Res<crate::transfer::ImportReport> {
    let state = state.inner().clone();
    crate::transfer::apply(&app, &state, Path::new(&path), passphrase, choice).await.map_err(err)
}

#[tauri::command]
pub async fn system_open_path(app: AppHandle, path: String) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    if !Path::new(&path).exists() {
        return Err(format!("{path} does not exist"));
    }
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_tags_and_cwd() {
        assert!(is_hex_color("#a1B2c3"));
        assert!(!is_hex_color("red"));
        assert_eq!(clean_tags(vec![" a ".into(), "A".into(), "".into(), "b".into()]), vec!["a", "b"]);
        assert_eq!(next_color(&[]), PALETTE[0]);
        let cwd = pick_cwd(Some("/definitely/not/here".into()), None);
        assert_eq!(cwd, util::home().to_string_lossy());
    }
}
