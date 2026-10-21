//! The shared profile Agent Hub owns for each tool: `<app data>/shared/<provider>/`.
//! Accounts link skills, plugins, agents, instructions and settings from here instead of from
//! the tool's main folder; history keeps linking from where the old chats are.
//! It starts as a copy of the main folder's items: copy only, never move, overwrite or delete.
use crate::model::{Account, KnowledgeItem, ShareKind, SharingOptions};
use crate::providers::ProviderAdapter;
use crate::sharing::{self, CopyCount, SharingReport};
use crate::store::AppState;
use crate::util;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

/// Written once the shared folder has been seeded, so later starts don't copy again.
pub(crate) const SEEDED_MARKER: &str = ".seeded";

pub const KNOWLEDGE_KINDS: [ShareKind; 6] =
    [ShareKind::Skills, ShareKind::Plugins, ShareKind::Agents, ShareKind::Instructions, ShareKind::Settings, ShareKind::Mcp];

/// Seeding and reseeding never run at the same time (startup migration vs. a launch).
static SEED_LOCK: Mutex<()> = Mutex::new(());

/// Everything except history.
pub fn is_knowledge(kind: ShareKind) -> bool {
    KNOWLEDGE_KINDS.contains(&kind)
}

/// Copy the knowledge items of `main` into `shared`, adding only what `shared` lacks.
/// Credential and unsafe paths are skipped; an item that fails is logged and the rest go on.
pub fn seed(main: &Path, shared: &Path, shareable: &[(ShareKind, Vec<&'static str>)], credentials: &[&str]) -> io::Result<CopyCount> {
    fs::create_dir_all(shared)?;
    let mut total = CopyCount::default();
    for (kind, rels) in shareable {
        if !is_knowledge(*kind) {
            continue;
        }
        for rel in rels {
            if !sharing::is_safe_rel(rel) || sharing::touches_credentials(rel, credentials) {
                continue;
            }
            let src = main.join(rel);
            if fs::metadata(&src).is_err() {
                continue;
            }
            let dst = shared.join(rel);
            if let Some(parent) = dst.parent() {
                fs::create_dir_all(parent)?;
            }
            match sharing::copy_missing(&src, &dst) {
                Ok(c) => total += c,
                Err(e) => log::warn!("seeding {rel} from {}: {e}", main.display()),
            }
        }
    }
    Ok(total)
}

/// Copy the main folder's items into `shared` with this provider's path lists.
fn seed_from_main(shared: &Path, adapter: &dyn ProviderAdapter) -> io::Result<CopyCount> {
    let count = seed(&adapter.default_home(), shared, &adapter.shareable_paths(), &adapter.credential_paths())?;
    fs::write(shared.join(SEEDED_MARKER), crate::util::now_iso())?;
    Ok(count)
}

/// Create the shared folder and seed it from the main folder the first time only.
/// Returns the counts when it seeded now.
pub fn ensure_seeded(shared: &Path, adapter: &dyn ProviderAdapter) -> io::Result<Option<CopyCount>> {
    let _guard = SEED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if shared.join(SEEDED_MARKER).exists() {
        return Ok(None);
    }
    let count = seed_from_main(shared, adapter)?;
    log::info!("seeded {} from {}: {count:?}", shared.display(), adapter.default_home().display());
    Ok(Some(count))
}

/// Copy anything new from the main folder into the shared folder (asked for by the user).
pub fn reseed(shared: &Path, adapter: &dyn ProviderAdapter) -> io::Result<CopyCount> {
    let _guard = SEED_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    seed_from_main(shared, adapter)
}

/// The SharingOptions field name of a kind ("skills", "history"...).
pub fn kind_key(kind: ShareKind) -> &'static str {
    match kind {
        ShareKind::Skills => "skills",
        ShareKind::Plugins => "plugins",
        ShareKind::Agents => "agents",
        ShareKind::Instructions => "instructions",
        ShareKind::Settings => "settings",
        ShareKind::Mcp => "mcp",
        ShareKind::History => "history",
    }
}

/// Knowledge kinds an account shares. A default home links nothing, so it shares none.
pub fn shared_kinds(account: &Account) -> Vec<String> {
    if account.is_default_home {
        return vec![];
    }
    KNOWLEDGE_KINDS.iter().filter(|k| account.sharing.enabled(**k)).map(|k| kind_key(*k).to_string()).collect()
}

/// What the shared folder holds, by kind: skill folders, agent and command files, plugins,
/// the instructions file and settings. Missing paths are simply absent.
pub fn list_items(shared: &Path, shareable: &[(ShareKind, Vec<&'static str>)], credentials: &[&str]) -> Vec<KnowledgeItem> {
    let mut items = Vec::new();
    for (kind, rels) in shareable {
        for rel in rels {
            if !sharing::is_safe_rel(rel) || sharing::touches_credentials(rel, credentials) {
                continue;
            }
            let path = shared.join(rel);
            if fs::metadata(&path).is_err() {
                continue;
            }
            match kind {
                ShareKind::Skills => {
                    for (name, p) in visible_entries(&path).into_iter().filter(|(_, p)| p.is_dir()) {
                        items.push(item("skills", name, &p));
                    }
                }
                ShareKind::Agents => {
                    // Claude agents/ hold subagents; commands/ (Codex prompts/) hold slash commands.
                    let k = if Path::new(rel).file_name().is_some_and(|n| n == "agents") { "agents" } else { "commands" };
                    let mut files = Vec::new();
                    collect_files(&path, "", 0, &mut files);
                    for (name, p) in files {
                        items.push(item(k, name, &p));
                    }
                }
                ShareKind::Plugins => {
                    for (name, p) in plugins(&path) {
                        items.push(item("plugins", name, &p));
                    }
                }
                ShareKind::Instructions | ShareKind::Settings => {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| rel.to_string());
                    items.push(item(kind_key(*kind), name, &path));
                }
                ShareKind::Mcp | ShareKind::History => {}
            }
        }
    }
    let order = ["skills", "plugins", "agents", "commands", "instructions", "settings"];
    items.sort_by_key(|i| (order.iter().position(|k| *k == i.kind), i.name.to_lowercase()));
    items.dedup_by(|a, b| a.kind == b.kind && a.name == b.name);
    items
}

fn item(kind: &str, name: String, path: &Path) -> KnowledgeItem {
    KnowledgeItem { kind: kind.to_string(), name, path: path.to_string_lossy().into_owned() }
}

/// Entries of a folder without hidden files or our backups, links followed.
fn visible_entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(rd) = fs::read_dir(dir) else { return vec![] };
    rd.filter_map(Result::ok)
        .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
        .filter(|(n, p)| !n.starts_with('.') && !n.contains(".hopper-backup-") && fs::metadata(p).is_ok())
        .collect()
}

/// Files under `dir` named by their relative path without extension ("review", "git/commit").
fn collect_files(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<(String, PathBuf)>) {
    if depth > 8 {
        return;
    }
    for (name, p) in visible_entries(dir) {
        if p.is_dir() {
            collect_files(&p, &format!("{prefix}{name}/"), depth + 1, out);
        } else {
            let stem = Path::new(&name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(name);
            out.push((format!("{prefix}{stem}"), p));
        }
    }
}

/// Installed plugins: Claude lists them in installed_plugins.json ("name@marketplace" keys);
/// other tools keep one folder per plugin or extension.
fn plugins(dir: &Path) -> Vec<(String, PathBuf)> {
    let listed = util::read_json(&dir.join("installed_plugins.json"));
    if let Some(map) = listed.as_ref().and_then(|v| v.get("plugins")).and_then(|v| v.as_object()) {
        let inside = util::path_key(dir) + "/";
        return map
            .iter()
            .map(|(k, v)| {
                let name = k.split('@').next().filter(|n| !n.is_empty()).unwrap_or(k).to_string();
                // v2 lists installs per scope, v1 has one object; paths copied from the main
                // folder still point there, so only a path inside this folder is used.
                let entry = v.as_array().and_then(|a| a.first()).unwrap_or(v);
                let path = entry
                    .get("installPath")
                    .and_then(|p| p.as_str())
                    .map(PathBuf::from)
                    .filter(|p| util::path_key(p).starts_with(&inside) && p.exists())
                    .unwrap_or_else(|| dir.to_path_buf());
                (name, path)
            })
            .collect();
    }
    visible_entries(dir).into_iter().filter(|(_, p)| p.is_dir()).collect()
}

/// True when any knowledge kind is shared.
pub fn shares_any(opts: &SharingOptions) -> bool {
    KNOWLEDGE_KINDS.iter().any(|k| opts.enabled(*k))
}

/// Link an account's items: knowledge from `shared` (seeded first if needed), history from
/// `shared_from`. A tool's default home is never touched. If the shared folder cannot be
/// prepared nothing is changed, so the account keeps whatever it links now.
pub fn apply_for_account(account: &Account, adapter: &dyn ProviderAdapter, shared: &Path) -> SharingReport {
    if account.is_default_home || util::path_key(Path::new(&account.home_dir)) == util::path_key(&adapter.default_home()) {
        return SharingReport::default();
    }
    if let Err(e) = ensure_seeded(shared, adapter) {
        let mut report = SharingReport::default();
        report.notes.push(format!("shared folder {}: {e}", shared.display()));
        return report;
    }
    sharing::apply_for_account(account, adapter, Some(shared))
}

/// On app start: seed the shared folder of every tool that has a non-default account, then
/// relink those accounts from it. Accounts that share nothing are left alone.
pub async fn migrate(app: AppHandle, state: Arc<AppState>) {
    let accounts = state.accounts.read().await.clone();
    for adapter in &state.adapters {
        let mine: Vec<&Account> = accounts.iter().filter(|a| a.provider_id == adapter.id() && !a.is_default_home).collect();
        if mine.is_empty() {
            continue;
        }
        let (shared, ad) = (state.shared_dir(adapter.id()), adapter.clone());
        if let Ok(Err(e)) = tokio::task::spawn_blocking(move || ensure_seeded(&shared, ad.as_ref())).await {
            log::warn!("shared folder for {}: {e}", adapter.id().as_str());
            continue;
        }
        for account in mine.into_iter().filter(|a| shares_any(&a.sharing) || a.sharing.history) {
            crate::commands::apply_sharing(&app, &state, account, adapter.clone()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<(ShareKind, Vec<&'static str>)> {
        vec![
            (ShareKind::Skills, vec!["skills"]),
            (ShareKind::Agents, vec!["agents", "commands"]),
            (ShareKind::Instructions, vec!["CLAUDE.md"]),
            (ShareKind::Settings, vec!["settings.json", ".credentials.json"]),
            (ShareKind::History, vec!["projects"]),
        ]
    }

    const CREDS: &[&str] = &[".credentials.json"];

    #[test]
    fn seeding_copies_knowledge_without_overwriting() {
        let t = tempfile::tempdir().unwrap();
        let (main, shared) = (t.path().join("main"), t.path().join("shared"));
        fs::create_dir_all(main.join("skills").join("a")).unwrap();
        fs::write(main.join("skills").join("a").join("SKILL.md"), "main a").unwrap();
        fs::create_dir_all(main.join("skills").join("b")).unwrap();
        fs::write(main.join("skills").join("b").join("SKILL.md"), "main b").unwrap();
        fs::create_dir_all(main.join("agents")).unwrap();
        fs::write(main.join("agents").join("rev.md"), "agent").unwrap();
        fs::write(main.join("CLAUDE.md"), "main instructions").unwrap();
        fs::write(main.join(".credentials.json"), "secret").unwrap();
        fs::create_dir_all(main.join("projects")).unwrap();
        fs::write(main.join("projects").join("chat.jsonl"), "chat").unwrap();
        fs::create_dir_all(shared.join("skills").join("a")).unwrap();
        fs::write(shared.join("skills").join("a").join("SKILL.md"), "edited in shared").unwrap();

        let c = seed(&main, &shared, &paths(), CREDS).unwrap();
        assert_eq!(c, CopyCount { added: 3, kept: 1 });
        assert_eq!(fs::read_to_string(shared.join("skills").join("a").join("SKILL.md")).unwrap(), "edited in shared");
        assert_eq!(fs::read_to_string(shared.join("skills").join("b").join("SKILL.md")).unwrap(), "main b");
        assert_eq!(fs::read_to_string(shared.join("agents").join("rev.md")).unwrap(), "agent");
        assert_eq!(fs::read_to_string(shared.join("CLAUDE.md")).unwrap(), "main instructions");
        assert!(!shared.join(".credentials.json").exists(), "credentials never copied");
        assert!(!shared.join("projects").exists(), "history is not knowledge");
        assert!(!shared.join("commands").exists(), "missing items are not created");
        // Main is untouched and a second seed adds nothing.
        assert_eq!(fs::read_to_string(main.join("skills").join("a").join("SKILL.md")).unwrap(), "main a");
        assert_eq!(seed(&main, &shared, &paths(), CREDS).unwrap(), CopyCount { added: 0, kept: 4 });
    }

    #[test]
    fn lists_items_by_kind() {
        let t = tempfile::tempdir().unwrap();
        let shared = t.path();
        fs::create_dir_all(shared.join("skills").join("pdf")).unwrap();
        fs::create_dir_all(shared.join("skills").join(".hidden")).unwrap();
        fs::write(shared.join("skills").join("README.md"), "not a skill").unwrap();
        fs::create_dir_all(shared.join("agents")).unwrap();
        fs::write(shared.join("agents").join("reviewer.md"), "a").unwrap();
        fs::create_dir_all(shared.join("commands").join("git")).unwrap();
        fs::write(shared.join("commands").join("git").join("commit.md"), "c").unwrap();
        fs::create_dir_all(shared.join("plugins")).unwrap();
        fs::write(shared.join("plugins").join("installed_plugins.json"), r#"{"version":2,"plugins":{"docs@official":[{"installPath":"/elsewhere/docs"}]}}"#)
            .unwrap();
        fs::write(shared.join("CLAUDE.md"), "i").unwrap();
        let mut p = paths();
        p.push((ShareKind::Plugins, vec!["plugins"]));
        let got: Vec<(String, String)> = list_items(shared, &p, CREDS).into_iter().map(|i| (i.kind, i.name)).collect();
        let want: Vec<(String, String)> =
            [("skills", "pdf"), ("plugins", "docs"), ("agents", "reviewer"), ("commands", "git/commit"), ("instructions", "CLAUDE.md")]
                .iter()
                .map(|(k, n)| (k.to_string(), n.to_string()))
                .collect();
        assert_eq!(got, want);
    }
}
