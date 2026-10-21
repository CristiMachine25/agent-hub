//! Tauri commands: the AgentHubApi from src/shared/types.ts. Names and argument names must match
//! COMMANDS there (Tauri maps camelCase JS keys to these snake_case parameters).
use crate::accounts::{self, apply_sharing, emit_accounts, non_empty};
use crate::model::*;
use crate::providers::{self, Adapter, CommandSpec};
use crate::store::AppState;
use crate::{knowledge, launcher, notify, pty, recommend, settings, skills, usage, util};
use anyhow::anyhow;
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

#[tauri::command]
pub async fn accounts_import(app: AppHandle, state: St<'_>, found: Vec<DetectedAccount>) -> Res<Vec<Account>> {
    let state = state.inner().clone();
    accounts::import_accounts(&app, &state, found).await.map_err(err)
}

#[tauri::command]
pub async fn accounts_create(app: AppHandle, state: St<'_>, input: NewAccountInput) -> Res<Account> {
    let state = state.inner().clone();
    accounts::create_account(&app, &state, input).await.map_err(err)
}

#[tauri::command]
pub async fn accounts_update(app: AppHandle, state: St<'_>, id: String, patch: serde_json::Value) -> Res<Account> {
    let state = state.inner().clone();
    accounts::update_account(&app, &state, &id, patch).await.map_err(err)
}

#[tauri::command]
/// True when the account's folder was deleted too.
pub async fn accounts_remove(app: AppHandle, state: St<'_>, id: String, delete_home: bool) -> Res<bool> {
    let state = state.inner().clone();
    accounts::remove_account(&app, &state, &id, delete_home).await.map_err(err)
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
    non_empty(req_cwd).into_iter().chain(non_empty(default_cwd)).find(|c| Path::new(c).is_dir()).unwrap_or_else(|| util::home().to_string_lossy().into_owned())
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
    let mut spec = if login { adapter.login_command(account).await? } else { adapter.cli_command(account, resume, extra_args).await? };
    {
        let settings = state.settings.read().await;
        if !login {
            providers::apply_permission_mode(account.provider_id, &settings.permission_modes, &mut spec.args);
        }
        if spec.cwd.is_none() {
            spec.cwd = Some(pick_cwd(req_cwd, settings.default_cwd.clone()));
        }
    }
    if account.shared_from.is_some() || knowledge::shares_any(&account.sharing) {
        let (a, ad) = (account.clone(), adapter.clone());
        let shared = state.shared_dir(account.provider_id);
        let _ = tokio::task::spawn_blocking(move || knowledge::apply_for_account(&a, ad.as_ref(), &shared)).await;
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
            Ok(None) => return Ok(failed(format!("{} has no desktop app that can run with a separate account", notify::provider_name(account.provider_id)))),
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

/// Not async: sync commands run one at a time on the main thread, so input keeps its order. The
/// write only queues the bytes for the terminal's writer thread, so it never blocks.
#[tauri::command]
pub fn terminal_write(terms: Terms<'_>, id: String, data: String) -> Res<()> {
    terms.write(&id, data.as_bytes()).map_err(err)
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
    // Each probe may run `<cli> --version` (cached for a minute), so ask all tools at once.
    let mut probes = tokio::task::JoinSet::new();
    for a in state.adapters.clone() {
        probes.spawn(async move { (a.id(), a.info().await.cli_installed) });
    }
    let mut cli_missing = Vec::new();
    while let Some(probe) = probes.join_next().await {
        if let Ok((id, false)) = probe {
            cli_missing.push(id);
        }
    }
    Ok(recommend::recommend(task, provider_id, &accounts, &latest, &cli_missing, chrono::Utc::now()))
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
    settings::apply_settings_patch(&app, &state, patch).await.map_err(err)
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

// ---------- shared knowledge ----------

/// The shared folder of every tool that has accounts: what it holds and who links it.
#[tauri::command]
pub async fn knowledge_get(state: St<'_>) -> Res<Vec<ProviderKnowledge>> {
    let accounts = state.accounts.read().await.clone();
    let mut out = Vec::new();
    for adapter in &state.adapters {
        let mine: Vec<&Account> = accounts.iter().filter(|a| a.provider_id == adapter.id()).collect();
        if mine.is_empty() {
            continue;
        }
        let shared = state.shared_dir(adapter.id());
        let (dir, ad) = (shared.clone(), adapter.clone());
        let items = tokio::task::spawn_blocking(move || knowledge::list_items(&dir, &ad.shareable_paths(), &ad.credential_paths())).await.unwrap_or_default();
        out.push(ProviderKnowledge {
            provider_id: adapter.id(),
            folder: shared.to_string_lossy().into_owned(),
            main_folder: adapter.default_home().to_string_lossy().into_owned(),
            items,
            accounts: mine
                .iter()
                .map(|a| KnowledgeAccount {
                    account_id: a.id.clone(),
                    label: a.label.clone(),
                    is_default_home: a.is_default_home,
                    kinds: knowledge::shared_kinds(a),
                })
                .collect(),
        });
    }
    Ok(out)
}

/// Copy anything new from the tool's main folder into its shared folder (never overwrites),
/// then relink its accounts so new top-level items (e.g. a CLAUDE.md) show up there too.
#[tauri::command]
pub async fn knowledge_reseed(app: AppHandle, state: St<'_>, provider_id: ProviderId) -> Res<ReseedResult> {
    let state = state.inner().clone();
    let adapter = state.adapter(provider_id).map_err(err)?;
    let (dir, ad) = (state.shared_dir(provider_id), adapter.clone());
    let count = tokio::task::spawn_blocking(move || knowledge::reseed(&dir, ad.as_ref()))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("Could not copy into the shared folder: {e}"))?;
    let accounts = state.accounts.read().await.clone();
    for a in accounts.iter().filter(|a| a.provider_id == provider_id && !a.is_default_home && knowledge::shares_any(&a.sharing)) {
        apply_sharing(&app, &state, a, adapter.clone()).await;
    }
    Ok(ReseedResult { added: count.added, kept: count.kept })
}

// ---------- skills directory ----------

fn skill_ref(id: &str) -> Res<skills::SkillRef> {
    skills::SkillRef::parse(id).ok_or_else(|| format!("{id} isn't a skill Agent Hub can install"))
}

/// Search skills.sh. Shorter than two characters finds nothing, as the directory requires.
#[tauri::command]
pub async fn skills_search(query: String) -> Res<Vec<DirectorySkill>> {
    let q = query.trim();
    if q.chars().count() < 2 {
        return Ok(Vec::new());
    }
    skills::search(q).await.map_err(err)
}

#[tauri::command]
pub async fn skills_preview(id: String) -> Res<SkillPreview> {
    skills::preview(&skill_ref(&id)?).await.map_err(err)
}

/// The skill folders in the shared folder of every tool that has accounts.
#[tauri::command]
pub async fn skills_installed(state: St<'_>) -> Res<Vec<InstalledSkill>> {
    let used: HashSet<ProviderId> = state.accounts.read().await.iter().map(|a| a.provider_id).collect();
    let mut out = Vec::new();
    for adapter in state.adapters.iter().filter(|a| used.contains(&a.id())) {
        let Some(dir) = skills::skills_dir(&state.shared_dir(adapter.id()), adapter.as_ref()) else { continue };
        let provider_id = adapter.id();
        let found = tokio::task::spawn_blocking(move || skills::list_installed(&dir)).await.unwrap_or_default();
        out.extend(found.into_iter().map(|(name, id)| InstalledSkill { provider_id, name, id }));
    }
    Ok(out)
}

/// Download the skill once and add it to each tool's shared folder, then relink the accounts
/// that share skills so a skills folder made just now shows up in them too.
#[tauri::command]
pub async fn skills_install(app: AppHandle, state: St<'_>, id: String, provider_ids: Vec<ProviderId>) -> Res<Vec<SkillInstallOutcome>> {
    let r = skill_ref(&id)?;
    let mut seen = HashSet::new();
    let targets: Vec<ProviderId> = provider_ids.into_iter().filter(|p| seen.insert(*p)).collect();
    if targets.is_empty() {
        return Err("Pick at least one tool to install it for".into());
    }
    let state = state.inner().clone();
    let download = Arc::new(skills::download(&r).await.map_err(err)?);
    let accounts = state.accounts.read().await.clone();
    let mut out = Vec::new();
    for provider_id in targets {
        let adapter = match state.adapter(provider_id) {
            Ok(a) => a,
            Err(e) => {
                out.push(SkillInstallOutcome { provider_id, error: Some(err(e)) });
                continue;
            }
        };
        let (shared, ad, r, d) = (state.shared_dir(provider_id), adapter.clone(), r.clone(), download.clone());
        let written = tokio::task::spawn_blocking(move || skills::install_into(&shared, ad.as_ref(), &r, &d)).await.map_err(|e| e.to_string())?;
        if written.is_ok() {
            let sharing_skills = |a: &&Account| a.provider_id == provider_id && !a.is_default_home && a.sharing.enabled(ShareKind::Skills);
            for a in accounts.iter().filter(sharing_skills) {
                apply_sharing(&app, &state, a, adapter.clone()).await;
            }
        }
        out.push(SkillInstallOutcome { provider_id, error: written.err().map(err) });
    }
    Ok(out)
}

/// Remove a skill Agent Hub installed from one tool's shared folder.
#[tauri::command]
pub async fn skills_remove(state: St<'_>, id: String, provider_id: ProviderId) -> Res<()> {
    let r = skill_ref(&id)?;
    let adapter = state.adapter(provider_id).map_err(err)?;
    let shared = state.shared_dir(provider_id);
    tokio::task::spawn_blocking(move || skills::remove_from(&shared, adapter.as_ref(), &r)).await.map_err(|e| e.to_string())?.map_err(err)
}

/// Only skills.sh and GitHub pages, over https.
fn openable(url: &str) -> bool {
    reqwest::Url::parse(url).is_ok_and(|u| u.scheme() == "https" && matches!(u.host_str(), Some("skills.sh" | "github.com")))
}

/// Open a skill's page in the browser.
#[tauri::command]
pub async fn system_open_url(app: AppHandle, url: String) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    if !openable(&url) {
        return Err(format!("Agent Hub only opens skills.sh and GitHub links, not {url}"));
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// A folder Agent Hub keeps (shared folders, profiles in its data folder) or an account's folder.
/// Never a file or an app bundle, which the system would open or run, and never a path with `..`,
/// which would make the prefix check below meaningless.
fn openable_folder(path: &Path, data_dir: &Path, homes: &[String]) -> bool {
    if path.components().any(|c| c == std::path::Component::ParentDir) {
        return false;
    }
    let Ok(real) = path.canonicalize() else { return false };
    let bundle = real
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["app", "bundle", "framework", "kext", "mpkg", "pkg", "plugin", "prefpane", "workflow"].contains(&e.to_ascii_lowercase().as_str()));
    let key = util::path_key(path);
    let ours = key.starts_with(&format!("{}/", util::path_key(data_dir)));
    real.is_dir() && !bundle && (ours || homes.iter().any(|h| util::path_key(Path::new(h)) == key))
}

/// Show one of those folders in the file manager.
#[tauri::command]
pub async fn system_open_path(app: AppHandle, state: St<'_>, path: String) -> Res<()> {
    use tauri_plugin_opener::OpenerExt;
    let homes: Vec<String> = state.accounts.read().await.iter().map(|a| a.home_dir.clone()).collect();
    if !openable_folder(Path::new(&path), state.data_dir(), &homes) {
        return Err(format!("{path} isn't a folder Agent Hub manages"));
    }
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_only_skills_sh_and_github_pages() {
        assert!(openable("https://skills.sh/anthropics/skills/pdf"));
        assert!(openable("https://github.com/anthropics/skills/tree/main/skills/pdf"));
        for no in [
            "http://github.com/x",
            "https://github.com.evil.io/x",
            "https://evil.io/?u=https://github.com",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "github.com/x",
            "",
        ] {
            assert!(!openable(no), "{no}");
        }
    }

    #[test]
    fn opens_only_its_own_folders_and_account_folders() {
        let t = tempfile::tempdir().unwrap();
        let data = t.path().join("app data");
        let (shared, home, other) = (data.join("shared").join("claude"), t.path().join(".claude-work"), t.path().join("Documents"));
        for d in [&shared, &home, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        let file = shared.join("run.command");
        std::fs::write(&file, "x").unwrap();
        let homes = vec![home.to_string_lossy().into_owned()];
        assert!(openable_folder(&shared, &data, &homes));
        assert!(openable_folder(&home, &data, &homes));
        assert!(!openable_folder(&other, &data, &homes), "not one of ours");
        assert!(!openable_folder(&file, &data, &homes), "files could run");
        assert!(!openable_folder(&data.join("missing"), &data, &homes));
        assert!(!openable_folder(&data.join("..").join("Documents"), &data, &homes), "no climbing out");
        let app = shared.join("Tool.app");
        std::fs::create_dir_all(&app).unwrap();
        assert!(!openable_folder(&app, &data, &homes), "opening a bundle launches it");
        #[cfg(unix)]
        {
            let link = shared.join("linked-skills");
            std::os::unix::fs::symlink(&other, &link).unwrap();
            assert!(openable_folder(&link, &data, &homes), "a folder linked into ours still opens");
            let app_link = shared.join("innocent");
            std::os::unix::fs::symlink(&app, &app_link).unwrap();
            assert!(!openable_folder(&app_link, &data, &homes), "a link to a bundle is still a bundle");
        }
    }

    #[test]
    fn cwd_falls_back_to_home() {
        let cwd = pick_cwd(Some("/definitely/not/here".into()), None);
        assert_eq!(cwd, util::home().to_string_lossy());
    }
}
