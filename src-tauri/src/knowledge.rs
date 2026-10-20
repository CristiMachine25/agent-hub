//! The shared profile Switchyard owns for each tool: `<app data>/shared/<provider>/`.
//! Accounts link skills, plugins, agents, instructions and settings from here instead of from
//! the tool's main folder; history keeps linking from where the old chats are.
//! It starts as a copy of the main folder's items: copy only, never move, overwrite or delete.
use crate::model::{Account, ShareKind, SharingOptions};
use crate::providers::ProviderAdapter;
use crate::sharing::{self, CopyCount, SharingReport};
use crate::store::AppState;
use crate::util;
use std::fs;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

/// Written once the shared folder has been seeded, so later starts don't copy again.
const SEEDED_MARKER: &str = ".seeded";

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
}
