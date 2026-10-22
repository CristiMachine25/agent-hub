//! Moving an Agent Hub setup to another computer, on any OS. Export writes one `.agenthub` file;
//! import also opens the `.switchyard` files saved before the rename, which have the same format.
//! Import only adds: accounts that are already here, files that already exist and sign-ins already
//! present are left exactly as they are, and nothing is ever deleted.
//!
//! Sign-ins are optional and always encrypted (AES-256-GCM, key from a passphrase via PBKDF2-SHA256).
//! Everything else in the file is plain JSON with no secrets in it.

use crate::accounts;
use crate::model::*;
use crate::providers::{Adapter, ProviderAdapter};
use crate::store::AppState;
use crate::{sharing, util};
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Context};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::AppHandle;

/// Kept from before the rename: it is inside every setup file already saved.
const FORMAT: &str = "switchyard-setup";
const VERSION: u32 = 1;
const KDF_ROUNDS: u32 = 310_000;
/// Files choose their own cost; more than this only stalls the import.
const MAX_KDF_ROUNDS: u32 = 2_000_000;
/// Skill files larger than this are left out; the whole bundle is capped too.
const MAX_FILE: u64 = 5 * 1024 * 1024;
const MAX_BUNDLE: u64 = 100 * 1024 * 1024;
/// Sharing categories that are plain files worth carrying over (no logins, history or plugin state).
const PORTABLE_KINDS: [ShareKind; 3] = [ShareKind::Skills, ShareKind::Agents, ShareKind::Instructions];
/// The settings export writes, and the only ones import takes.
const PORTABLE_SETTINGS: [&str; 6] = ["pollIntervalMinutes", "notifications", "defaultSharing", "minimizeToTray", "theme", "liveUsage"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupFile {
    format: String,
    version: u32,
    exported_at: String,
    from_os: String,
    app_version: String,
    accounts: Vec<ExportedAccount>,
    #[serde(default)]
    settings: Option<serde_json::Value>,
    #[serde(default)]
    files: Vec<BundleFile>,
    #[serde(default)]
    sign_ins: Option<Sealed>,
}

/// Where an account's folder lives, in terms that make sense on any OS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum HomeRef {
    /// The tool's own folder (~/.claude, ~/.codex...).
    Default,
    /// A folder inside the user's home, e.g. `.claude-work`, stored with `/` separators.
    InHome { path: String },
    /// Anywhere else: a new folder is made in this computer's profiles folder.
    Elsewhere,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportedAccount {
    key: String,
    provider_id: ProviderId,
    label: String,
    email: Option<String>,
    plan: Option<String>,
    color: String,
    #[serde(default)]
    tags: Vec<String>,
    notes: Option<String>,
    monthly_cost: Option<f64>,
    sharing: SharingOptions,
    home: HomeRef,
    has_sign_in: bool,
}

/// A skill, agent or instructions file from a tool's shared folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundleFile {
    provider_id: ProviderId,
    /// "skills", "agents" or "instructions".
    kind: String,
    /// Relative to the tool's shared folder, `/` separators.
    path: String,
    data: String,
    /// A script to run (unix exec bit). Absent in files from older versions.
    #[serde(default)]
    executable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Sealed {
    salt: String,
    nonce: String,
    rounds: u32,
    data: String,
}

/// account key → (relative path, base64 bytes)
type SignIns = BTreeMap<String, Vec<(String, String)>>;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportOptions {
    pub account_ids: Vec<String>,
    pub include_files: bool,
    pub include_settings: bool,
    /// Include sign-ins, encrypted with this passphrase.
    pub passphrase: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub accounts: usize,
    pub sign_ins: usize,
    pub files: usize,
    pub skipped_files: usize,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub from_os: String,
    pub exported_at: String,
    pub accounts: Vec<PreviewAccount>,
    pub kinds: Vec<PreviewKind>,
    pub has_settings: bool,
    /// The file carries sign-ins; they need the passphrase.
    pub has_sign_ins: bool,
    /// The passphrase given opened them.
    pub sign_ins_unlocked: bool,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct PreviewAccount {
    pub key: String,
    pub provider_id: ProviderId,
    pub label: String,
    pub email: Option<String>,
    pub plan: Option<String>,
    pub has_sign_in: bool,
    /// Label of the account here that is the same login or folder. Such accounts are never touched.
    pub already_here: Option<String>,
    /// Folder it would use on this computer.
    pub target_home: String,
}

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(Deserialize))]
#[serde(rename_all = "camelCase")]
pub struct PreviewKind {
    pub provider_id: ProviderId,
    pub kind: String,
    pub files: usize,
    /// Files not on this computer yet (the rest are kept as they are).
    pub new_files: usize,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportChoice {
    pub account_keys: Vec<String>,
    pub sign_ins: bool,
    pub files: bool,
    pub settings: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportReport {
    pub added: Vec<String>,
    pub skipped: Vec<String>,
    pub sign_ins: usize,
    pub files_added: usize,
    pub files_kept: usize,
    pub settings: bool,
    pub problems: Vec<String>,
}

// ---------- export ----------

pub async fn export(state: &AppState, path: &Path, opts: ExportOptions) -> anyhow::Result<ExportSummary> {
    let accounts: Vec<Account> = state.accounts.read().await.iter().filter(|a| opts.account_ids.contains(&a.id)).cloned().collect();
    if accounts.is_empty() {
        bail!("Pick at least one account to export");
    }
    let passphrase = opts.passphrase.as_deref().map(str::trim).filter(|p| !p.is_empty());
    if let Some(p) = passphrase {
        if p.chars().count() < 8 {
            bail!("Use a passphrase of at least 8 characters to protect the sign-ins");
        }
    }

    // Folders inside these mean nothing on another OS: such accounts get a fresh folder there.
    let local_roots: Vec<PathBuf> =
        [Some(state.profiles_dir().await), dirs::data_dir(), dirs::data_local_dir(), dirs::config_dir()].into_iter().flatten().collect();
    let mut exported = Vec::new();
    let mut sign_ins: SignIns = BTreeMap::new();
    for a in &accounts {
        let adapter = state.adapter(a.provider_id)?;
        let home = PathBuf::from(&a.home_dir);
        let is_default = a.is_default_home || util::path_key(&adapter.default_home()) == util::path_key(&home);
        // The stored email can lag behind a fresh sign-in; the profile itself knows for sure.
        let email = match a.email.clone() {
            Some(e) => Some(e),
            None => adapter.read_identity(&home).await.email,
        };
        let mut has_sign_in = false;
        if passphrase.is_some() {
            let files = adapter.export_sign_in(&home, is_default).await;
            if !files.is_empty() {
                has_sign_in = true;
                sign_ins.insert(a.id.clone(), files.into_iter().map(|(p, b)| (p, B64.encode(b))).collect());
            }
        }
        exported.push(ExportedAccount {
            key: a.id.clone(),
            provider_id: a.provider_id,
            label: a.label.clone(),
            email,
            plan: a.plan.clone(),
            color: a.color.clone(),
            tags: a.tags.clone(),
            notes: a.notes.clone(),
            monthly_cost: a.monthly_cost,
            sharing: a.sharing,
            home: home_ref(&home, is_default, &local_roots),
            has_sign_in,
        });
    }

    let (files, skipped_files) = if opts.include_files {
        let sources = file_sources(state, &accounts)?;
        let homes = sign_in_homes(state).await;
        tokio::task::spawn_blocking(move || collect_files(&sources, &homes)).await?
    } else {
        (Vec::new(), 0)
    };
    let settings = if opts.include_settings { Some(portable_settings(&state.settings.read().await.clone())) } else { None };
    let sealed = match (passphrase, sign_ins.is_empty()) {
        // Key stretching takes a while: off the async workers, as in unlock.
        (Some(p), false) => {
            let (p, plain) = (p.to_string(), serde_json::to_vec(&sign_ins)?);
            Some(tokio::task::spawn_blocking(move || seal(&p, &plain)).await??)
        }
        _ => None,
    };

    let file = SetupFile {
        format: FORMAT.into(),
        version: VERSION,
        exported_at: util::now_iso(),
        from_os: std::env::consts::OS.into(),
        app_version: env!("CARGO_PKG_VERSION").into(),
        accounts: exported,
        settings,
        files,
        sign_ins: sealed,
    };
    let summary = ExportSummary { accounts: file.accounts.len(), sign_ins: sign_ins.len(), files: file.files.len(), skipped_files };
    // The files can add up to MAX_BUNDLE: serialize and write off the async workers, as in seal().
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        util::write_atomic(&path, serde_json::to_string_pretty(&file)?.as_bytes()).with_context(|| format!("writing {}", path.display()))
    })
    .await??;
    Ok(summary)
}

fn home_ref(home: &Path, is_default: bool, local_roots: &[PathBuf]) -> HomeRef {
    if is_default {
        return HomeRef::Default;
    }
    if local_roots.iter().any(|r| home.starts_with(r)) {
        return HomeRef::Elsewhere;
    }
    match home.strip_prefix(util::home()) {
        Ok(rel) if !rel.as_os_str().is_empty() => HomeRef::InHome { path: to_portable(rel) },
        _ => HomeRef::Elsewhere,
    }
}

/// Settings that mean the same on any computer (no paths, no terminal program). Keep the keys in
/// step with PORTABLE_SETTINGS.
fn portable_settings(s: &AppSettings) -> serde_json::Value {
    serde_json::json!({
        "pollIntervalMinutes": s.poll_interval_minutes,
        "notifications": s.notifications,
        "defaultSharing": s.default_sharing,
        "minimizeToTray": s.minimize_to_tray,
        "theme": s.theme,
        "liveUsage": s.live_usage,
    })
}

/// Only the settings a setup file is meant to carry: a crafted file can't switch on bypass modes
/// or point the profiles folder (the guard on deleting folders) somewhere else.
fn portable_only(settings: serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(map) = settings else { return serde_json::json!({}) };
    serde_json::Value::Object(map.into_iter().filter(|(k, _)| PORTABLE_SETTINGS.contains(&k.as_str())).collect())
}

/// The tool and the folder to take files from, for every tool the exported accounts use.
fn file_sources(state: &AppState, accounts: &[Account]) -> anyhow::Result<Vec<(Adapter, PathBuf)>> {
    let mut providers: Vec<ProviderId> = accounts.iter().map(|a| a.provider_id).collect();
    providers.sort_by_key(|p| p.as_str());
    providers.dedup();
    providers
        .into_iter()
        .map(|p| {
            let adapter = state.adapter(p)?;
            let root = knowledge_root(state, &adapter);
            Ok((adapter, root))
        })
        .collect()
}

/// Every folder that holds a sign-in: each account's folder and each tool's main folder.
async fn sign_in_homes(state: &AppState) -> Vec<PathBuf> {
    let mut homes: Vec<PathBuf> = state.accounts.read().await.iter().map(|a| PathBuf::from(&a.home_dir)).collect();
    homes.extend(state.adapters.iter().map(|a| a.default_home()));
    homes
}

/// Skills, agents and instructions from each tool's folder. Links are followed (skills are often
/// linked in from a checkout), but never into a folder that holds a sign-in (`homes`).
fn collect_files(sources: &[(Adapter, PathBuf)], homes: &[PathBuf]) -> (Vec<BundleFile>, usize) {
    let homes: Vec<PathBuf> = homes.iter().filter_map(|h| h.canonicalize().ok()).collect();
    let (mut out, mut skipped, mut total) = (Vec::new(), 0, 0u64);
    for (adapter, root) in sources {
        let (provider, credentials) = (adapter.id(), adapter.credential_paths());
        for (kind, rels) in adapter.shareable_paths().into_iter().filter(|(k, _)| PORTABLE_KINDS.contains(k)) {
            for rel in rels {
                let top = root.join(rel);
                let Ok(real_top) = top.canonicalize() else { continue };
                // A kind folder that is (or holds) a home, or a kind linked into one, is a sign-in in disguise.
                if homes.iter().any(|h| h.starts_with(&real_top)) || (sharing::is_link(&top) && homes.iter().any(|h| real_top.starts_with(h))) {
                    skipped += 1;
                    continue;
                }
                let (files, pruned) = walk(&top, &homes);
                skipped += pruned;
                for file in files {
                    // Reached through a link further in: never anything inside a home.
                    let Ok(real) = file.canonicalize() else { continue };
                    if !real.starts_with(&real_top) && homes.iter().any(|h| real.starts_with(h)) {
                        skipped += 1;
                        continue;
                    }
                    let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(u64::MAX);
                    if size > MAX_FILE || total + size > MAX_BUNDLE {
                        skipped += 1;
                        continue;
                    }
                    // Import refuses sign-in paths (see bundle_target), so they are never read.
                    let Some(rel_path) = file.strip_prefix(root).ok().filter(|r| !sharing::touches_credentials(&to_portable(r), &credentials)) else {
                        skipped += 1;
                        continue;
                    };
                    let Ok(bytes) = std::fs::read(&file) else {
                        skipped += 1;
                        continue;
                    };
                    total += size;
                    out.push(BundleFile {
                        provider_id: provider,
                        kind: kind.key().into(),
                        path: to_portable(rel_path),
                        data: B64.encode(bytes),
                        executable: is_executable(&file),
                    });
                }
            }
        }
    }
    (out, skipped)
}

fn is_executable(p: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        false
    }
}

/// The shared knowledge folder all accounts link to, or the tool's main folder before one exists.
fn knowledge_root(state: &AppState, adapter: &Adapter) -> PathBuf {
    let shared = state.shared_dir(adapter.id());
    if shared.is_dir() {
        shared
    } else {
        adapter.default_home()
    }
}

/// Every regular file under `p` (or `p` itself), skipping VCS folders and dependency caches, plus
/// how many linked folders were left out because they lead to or above a home. Links are followed;
/// walkdir drops the ones that loop.
fn walk(p: &Path, homes: &[PathBuf]) -> (Vec<PathBuf>, usize) {
    if p.is_file() {
        return (vec![p.to_path_buf()], 0);
    }
    let pruned = std::cell::Cell::new(0);
    let files = walkdir::WalkDir::new(p)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| {
            if matches!(e.file_name().to_str(), Some(".git" | "node_modules" | "__pycache__" | ".venv")) {
                return false;
            }
            let into_home = e.path_is_symlink() && e.path().canonicalize().is_ok_and(|r| homes.iter().any(|h| h.starts_with(&r) || r.starts_with(h)));
            if into_home {
                pruned.set(pruned.get() + 1);
            }
            !into_home
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();
    (files, pruned.get())
}

fn to_portable(rel: &Path) -> String {
    rel.components().filter_map(|c| c.as_os_str().to_str()).collect::<Vec<_>>().join("/")
}

/// A relative `/` path from the file, refusing anything that could escape the target folder.
fn from_portable(base: &Path, rel: &str) -> Option<PathBuf> {
    sharing::is_safe_rel(rel).then(|| base.join(rel))
}

/// Where a bundled file goes in the tool's shared folder: only inside a path of the portable kind
/// the file says it is, and never a sign-in file. Anything else (a skill that is really
/// `settings.json`, hooks, extensions) is refused.
fn bundle_target(root: &Path, adapter: &dyn ProviderAdapter, f: &BundleFile) -> Option<PathBuf> {
    let fits = adapter
        .shareable_paths()
        .into_iter()
        .filter(|(k, _)| PORTABLE_KINDS.contains(k) && k.key() == f.kind)
        .flat_map(|(_, rels)| rels)
        .find(|rel| f.path == *rel || f.path.starts_with(&format!("{rel}/")))?;
    if sharing::touches_credentials(&f.path, &adapter.credential_paths()) {
        return None;
    }
    let target = from_portable(root, &f.path)?;
    // The kind's own folder may be a link (a skills folder kept in another checkout), but a link
    // further in could lead anywhere, such as back into the tool's home, so nothing goes through one.
    let top = root.join(fits);
    let through_link = target.ancestors().skip(1).take_while(|a| *a != top.as_path() && *a != root).any(sharing::is_link);
    (!through_link).then_some(target)
}

/// An account folder named in the file, inside this computer's home: only a folder of that
/// tool's own (`.codex`, `.codex-work`...), never any other folder of the user's.
fn in_home(provider: ProviderId, path: &str) -> Option<PathBuf> {
    let first = path.split('/').next()?;
    let tool = format!(".{}", provider.as_str());
    let own = (first == tool && !path.contains('/')) || first.strip_prefix(&tool).is_some_and(|rest| rest.starts_with('-'));
    own.then(|| from_portable(&util::home(), path)).flatten()
}

/// A new file with ordinary permissions (scripts stay runnable). Fails when something is there.
fn write_new(path: &Path, bytes: &[u8], executable: bool) -> std::io::Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(if executable { 0o755 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = executable;
    let mut file = opts.open(path)?;
    file.write_all(bytes).inspect_err(|_| {
        let _ = std::fs::remove_file(path);
    })
}

// ---------- encryption ----------

fn key_from(passphrase: &str, salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut key = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<sha2::Sha256>(passphrase.as_bytes(), salt, rounds, &mut key);
    key
}

fn seal(passphrase: &str, plain: &[u8]) -> anyhow::Result<Sealed> {
    let mut salt = [0u8; 16];
    let mut nonce = [0u8; 12];
    getrandom::getrandom(&mut salt).map_err(|e| anyhow!("no random source: {e}"))?;
    getrandom::getrandom(&mut nonce).map_err(|e| anyhow!("no random source: {e}"))?;
    let cipher = Aes256Gcm::new_from_slice(&key_from(passphrase, &salt, KDF_ROUNDS)).map_err(|e| anyhow!("{e}"))?;
    let data = cipher.encrypt(Nonce::from_slice(&nonce), plain).map_err(|_| anyhow!("encrypting the sign-ins failed"))?;
    Ok(Sealed { salt: B64.encode(salt), nonce: B64.encode(nonce), rounds: KDF_ROUNDS, data: B64.encode(data) })
}

fn kdf_rounds(from_file: u32) -> u32 {
    from_file.clamp(1, MAX_KDF_ROUNDS)
}

fn open(passphrase: &str, s: &Sealed) -> Option<Vec<u8>> {
    let salt = B64.decode(&s.salt).ok()?;
    let nonce = B64.decode(&s.nonce).ok().filter(|n| n.len() == 12)?;
    let data = B64.decode(&s.data).ok()?;
    let cipher = Aes256Gcm::new_from_slice(&key_from(passphrase, &salt, kdf_rounds(s.rounds))).ok()?;
    cipher.decrypt(Nonce::from_slice(&nonce), data.as_ref()).ok()
}

// ---------- import ----------

fn read_file(path: &Path) -> anyhow::Result<SetupFile> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file: SetupFile = serde_json::from_str(&text).map_err(|_| anyhow!("This isn't an Agent Hub setup file"))?;
    if file.format != FORMAT {
        bail!("This isn't an Agent Hub setup file");
    }
    if file.version > VERSION {
        bail!("This file comes from a newer Agent Hub. Update Agent Hub on this computer first.");
    }
    Ok(file)
}

/// The sign-ins, when the passphrase opens them. Key stretching takes a while: off the async workers.
async fn unlock(file: &SetupFile, passphrase: Option<&str>) -> Option<SignIns> {
    let (passphrase, sealed) = (passphrase?.trim().to_string(), file.sign_ins.clone()?);
    let plain = tokio::task::spawn_blocking(move || open(&passphrase, &sealed)).await.ok()??;
    serde_json::from_slice(&plain).ok()
}

struct Plan {
    adapter: Adapter,
    home: PathBuf,
    is_default: bool,
    already_here: Option<String>,
}

/// Where an exported account would go here, and whether it is already here.
async fn plan_account(state: &Arc<AppState>, a: &ExportedAccount) -> anyhow::Result<Plan> {
    let adapter = state.adapter(a.provider_id)?;
    let default_home = adapter.default_home();
    let (home, is_default) = match &a.home {
        HomeRef::Default => (default_home.clone(), true),
        // Somewhere that isn't the tool's own kind of folder gets a fresh one, as Elsewhere does.
        HomeRef::InHome { path } => (in_home(a.provider_id, path).unwrap_or_default(), false),
        HomeRef::Elsewhere => (PathBuf::new(), false),
    };
    let accounts = state.accounts.read().await;
    let same_login =
        |x: &&Account| x.provider_id == a.provider_id && a.email.as_deref().is_some_and(|e| x.email.as_deref().is_some_and(|y| y.eq_ignore_ascii_case(e)));
    let same_folder = |x: &&Account| !home.as_os_str().is_empty() && util::path_key(Path::new(&x.home_dir)) == util::path_key(&home);
    // Without an email to compare, the same name for the same tool counts as the same account.
    let same_name = |x: &&Account| x.provider_id == a.provider_id && x.label.trim().eq_ignore_ascii_case(a.label.trim());
    let already_here = accounts.iter().find(|x| same_login(x) || same_folder(x) || same_name(x)).map(|x| x.label.clone());
    drop(accounts);
    let home = if home.as_os_str().is_empty() { accounts::new_home_dir(state, a.provider_id, &a.label).await } else { home };
    Ok(Plan { adapter, home, is_default, already_here })
}

pub async fn preview(state: &Arc<AppState>, path: &Path, passphrase: Option<String>) -> anyhow::Result<ImportPreview> {
    let file = read_file(path)?;
    let unlocked = unlock(&file, passphrase.as_deref()).await.is_some();
    let mut accounts = Vec::new();
    for a in &file.accounts {
        let plan = plan_account(state, a).await?;
        accounts.push(PreviewAccount {
            key: a.key.clone(),
            provider_id: a.provider_id,
            label: a.label.clone(),
            email: a.email.clone(),
            plan: a.plan.clone(),
            has_sign_in: a.has_sign_in,
            already_here: plan.already_here,
            target_home: plan.home.to_string_lossy().into_owned(),
        });
    }
    let mut kinds: BTreeMap<(String, String), PreviewKind> = BTreeMap::new();
    for f in &file.files {
        let Ok(adapter) = state.adapter(f.provider_id) else { continue };
        let Some(target) = bundle_target(&state.shared_dir(f.provider_id), adapter.as_ref(), f) else { continue };
        let new = !target.exists();
        let k = kinds.entry((f.provider_id.as_str().into(), f.kind.clone())).or_insert(PreviewKind {
            provider_id: f.provider_id,
            kind: f.kind.clone(),
            files: 0,
            new_files: 0,
        });
        k.files += 1;
        k.new_files += usize::from(new);
    }
    Ok(ImportPreview {
        from_os: file.from_os.clone(),
        exported_at: file.exported_at.clone(),
        accounts,
        kinds: kinds.into_values().collect(),
        has_settings: file.settings.is_some(),
        has_sign_ins: file.sign_ins.is_some(),
        sign_ins_unlocked: unlocked,
    })
}

pub async fn apply(app: &AppHandle, state: &Arc<AppState>, path: &Path, passphrase: Option<String>, choice: ImportChoice) -> anyhow::Result<ImportReport> {
    let file = read_file(path)?;
    let sign_ins = if choice.sign_ins {
        match (file.sign_ins.is_some(), unlock(&file, passphrase.as_deref()).await) {
            (true, None) => bail!("That passphrase doesn't open the sign-ins in this file"),
            (_, s) => s.unwrap_or_default(),
        }
    } else {
        SignIns::new()
    };
    let mut report = ImportReport::default();

    // A failure is noted and the rest goes on, so the report still shows what was added.
    for a in file.accounts.iter().filter(|a| choice.account_keys.contains(&a.key)) {
        let plan = match plan_account(state, a).await {
            Ok(plan) => plan,
            Err(e) => {
                report.problems.push(format!("{}: {e:#}", a.label));
                continue;
            }
        };
        if let Some(here) = &plan.already_here {
            report.skipped.push(format!("{} (already here as {here})", a.label));
            continue;
        }
        if let Err(e) = std::fs::create_dir_all(&plan.home) {
            report.problems.push(format!("{}: couldn't create {} ({e})", a.label, plan.home.display()));
            continue;
        }
        if let Some(files) = sign_ins.get(&a.key) {
            let decoded: Vec<(String, Vec<u8>)> = files.iter().filter_map(|(p, b)| Some((p.clone(), B64.decode(b).ok()?))).collect();
            match plan.adapter.import_sign_in(&plan.home, plan.is_default, &decoded).await {
                Ok(n) if n > 0 => report.sign_ins += 1,
                Ok(_) => {}
                Err(e) => report.problems.push(format!("{}: couldn't add the sign-in ({e:#})", a.label)),
            }
        }
        let input = NewAccountInput {
            provider_id: a.provider_id,
            label: a.label.clone(),
            home_dir: Some(plan.home.to_string_lossy().into_owned()),
            sharing: Some(a.sharing),
            shared_from: None,
            color: Some(a.color.clone()),
            tags: Some(a.tags.clone()),
            monthly_cost: a.monthly_cost,
            plan: a.plan.clone(),
        };
        match accounts::create_account(app, state, input).await {
            Ok(created) => {
                if let Some(notes) = a.notes.as_deref().filter(|n| !n.trim().is_empty()) {
                    let _ = accounts::update_account(app, state, &created.id, serde_json::json!({ "notes": notes })).await;
                }
                report.added.push(a.label.clone());
            }
            Err(e) => report.problems.push(format!("{}: {e:#}", a.label)),
        }
    }

    if choice.files {
        let mut roots: HashMap<ProviderId, (Adapter, PathBuf)> = HashMap::new();
        for f in &file.files {
            let (adapter, root) = match roots.get(&f.provider_id) {
                Some(r) => r.clone(),
                None => {
                    let Ok(adapter) = state.adapter(f.provider_id) else { continue };
                    let r = state.shared_dir(f.provider_id);
                    // Start from this computer's own main folder before adding what came in the file.
                    if let Err(e) = crate::knowledge::ensure_seeded(&r, adapter.as_ref()) {
                        report.problems.push(format!("Preparing the shared {} folder: {e}", f.provider_id.as_str()));
                    }
                    roots.insert(f.provider_id, (adapter.clone(), r.clone()));
                    (adapter, r)
                }
            };
            let Some(target) = bundle_target(&root, adapter.as_ref(), f) else { continue };
            if target.exists() {
                report.files_kept += 1;
                continue;
            }
            let Ok(bytes) = B64.decode(&f.data) else { continue };
            let written = match target.parent() {
                Some(parent) => std::fs::create_dir_all(parent),
                None => Ok(()),
            }
            .and_then(|_| write_new(&target, &bytes, f.executable));
            match written {
                Ok(()) => report.files_added += 1,
                Err(e) => report.problems.push(format!("{}: {e}", f.path)),
            }
        }
    }

    if let (true, Some(settings)) = (choice.settings, file.settings.clone()) {
        match crate::settings::apply_settings_patch(app, state, portable_only(settings)).await {
            Ok(_) => report.settings = true,
            Err(e) => report.problems.push(format!("Preferences: {e:#}")),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_ins_round_trip_only_with_the_right_passphrase() {
        let sealed = seal("correct horse battery", b"secret tokens").unwrap();
        assert_eq!(open("correct horse battery", &sealed).as_deref(), Some(&b"secret tokens"[..]));
        assert!(open("wrong passphrase", &sealed).is_none());
        // Same input, fresh salt and nonce every time.
        assert_ne!(seal("correct horse battery", b"secret tokens").unwrap().data, sealed.data);
    }

    #[test]
    fn portable_paths_cannot_escape_the_target() {
        let base = Path::new("/home/me/.claude");
        assert_eq!(from_portable(base, "skills/review/SKILL.md"), Some(base.join("skills").join("review").join("SKILL.md")));
        assert!(from_portable(base, "../.ssh/id_rsa").is_none());
        assert!(from_portable(base, "/etc/passwd").is_none());
        assert!(from_portable(base, "").is_none());
    }

    #[test]
    fn homes_are_described_relative_to_the_user_folder() {
        let roots = vec![util::home().join("AppData").join("Roaming")];
        assert!(matches!(home_ref(&util::home().join(".claude"), true, &roots), HomeRef::Default));
        match home_ref(&util::home().join(".claude-work"), false, &roots) {
            HomeRef::InHome { path } => assert_eq!(path, ".claude-work"),
            other => panic!("{other:?}"),
        }
        match home_ref(&util::home().join("work").join("claude"), false, &roots) {
            HomeRef::InHome { path } => assert_eq!(path, "work/claude"),
            other => panic!("{other:?}"),
        }
        // App data folders differ per OS, so those profiles get a new folder on the other computer.
        assert!(matches!(home_ref(&roots[0].join("dev.switchyard.app").join("profiles").join("h12"), false, &roots), HomeRef::Elsewhere));
        assert!(matches!(home_ref(Path::new("/elsewhere/claude"), false, &roots), HomeRef::Elsewhere));
    }

    #[test]
    fn only_portable_settings_are_imported() {
        let exported = portable_settings(&AppSettings::default());
        assert_eq!(portable_only(exported.clone()), exported, "everything export writes comes back");
        let crafted = serde_json::json!({
            "theme": "light",
            "permissionModes": { "claude": "bypassPermissions", "codex": "bypass" },
            "profilesDir": "/",
            "terminal": "evil",
            "defaultCwd": "/tmp",
            "launchAtLogin": true
        });
        assert_eq!(portable_only(crafted), serde_json::json!({ "theme": "light" }));
        assert_eq!(portable_only(serde_json::json!("not an object")), serde_json::json!({}));
    }

    #[test]
    fn bundle_files_only_land_in_their_own_kind() {
        let claude = crate::providers::get(ProviderId::Claude);
        let gemini = crate::providers::get(ProviderId::Gemini);
        let root = Path::new("/shared/claude");
        let f = |kind: &str, path: &str| BundleFile {
            provider_id: ProviderId::Claude,
            kind: kind.into(),
            path: path.into(),
            data: String::new(),
            executable: false,
        };
        assert_eq!(bundle_target(root, claude.as_ref(), &f("skills", "skills/pdf/SKILL.md")), Some(root.join("skills/pdf/SKILL.md")));
        assert_eq!(bundle_target(root, claude.as_ref(), &f("agents", "commands/review.md")), Some(root.join("commands/review.md")));
        assert_eq!(bundle_target(root, claude.as_ref(), &f("instructions", "CLAUDE.md")), Some(root.join("CLAUDE.md")));
        for (kind, path) in [
            ("skills", "settings.json"),   // a settings file named as a skill
            ("settings", "settings.json"), // settings are never carried
            ("skills", "CLAUDE.md"),       // the wrong kind for that path
            ("skills", "skillsx/a.md"),    // only inside the folder itself
            ("skills", "skills/../settings.json"),
            ("instructions", ".credentials.json"),
            ("other", "plugins/x/plugin.json"),
        ] {
            assert_eq!(bundle_target(root, claude.as_ref(), &f(kind, path)), None, "{kind} {path}");
        }
        let g = |kind: &str, path: &str| bundle_target(root, gemini.as_ref(), &BundleFile { provider_id: ProviderId::Gemini, ..f(kind, path) });
        assert!(g("skills", ".agents/skills/x/SKILL.md").is_some());
        assert!(g("skills", ".gemini/extensions/x/gemini-extension.json").is_none());
    }

    #[cfg(unix)]
    #[test]
    fn bundle_files_never_go_through_a_link_inside_the_kind_folder() {
        use std::os::unix::fs::symlink;
        let claude = crate::providers::get(ProviderId::Claude);
        let t = tempfile::tempdir().unwrap();
        let (root, repo, home) = (t.path().join("shared"), t.path().join("repo"), t.path().join("home"));
        for d in [&root, &repo, &home] {
            std::fs::create_dir_all(d).unwrap();
        }
        // The whole skills folder kept in a checkout is fine; a link inside it back to the home is not.
        symlink(&repo, root.join("skills")).unwrap();
        symlink(&home, repo.join("loop")).unwrap();
        let f = |path: &str| BundleFile { provider_id: ProviderId::Claude, kind: "skills".into(), path: path.into(), data: String::new(), executable: false };
        assert_eq!(bundle_target(&root, claude.as_ref(), &f("skills/pdf/SKILL.md")), Some(root.join("skills/pdf/SKILL.md")));
        assert_eq!(bundle_target(&root, claude.as_ref(), &f("skills/loop/settings.json")), None);
        let instructions = BundleFile { kind: "instructions".into(), ..f("CLAUDE.md") };
        assert_eq!(bundle_target(&root, claude.as_ref(), &instructions), Some(root.join("CLAUDE.md")));
    }

    #[cfg(unix)]
    #[test]
    fn export_never_follows_a_link_into_a_tools_home() {
        use std::os::unix::fs::symlink;
        let claude = crate::providers::get(ProviderId::Claude);
        let t = tempfile::tempdir().unwrap();
        let (root, repo, home) = (t.path().join("shared"), t.path().join("repo"), t.path().join("home"));
        for d in [root.join("commands"), repo.join("pdf"), home.clone()] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(home.join(".credentials.json"), "secret").unwrap();
        std::fs::write(home.join("settings.json"), "{}").unwrap();
        std::fs::write(repo.join("pdf").join("SKILL.md"), "skill").unwrap();
        std::fs::write(root.join("commands").join("review.md"), "review").unwrap();
        // A skills folder kept in a checkout is exported, but not a link inside it back to the home,
        symlink(&repo, root.join("skills")).unwrap();
        symlink(&home, repo.join("loop")).unwrap();
        // a kind folder that is itself a link to the home, or a file linked to a sign-in.
        symlink(&home, root.join("agents")).unwrap();
        symlink(home.join(".credentials.json"), root.join("commands").join("creds.md")).unwrap();
        symlink(home.join(".credentials.json"), root.join("CLAUDE.md")).unwrap();

        let (files, skipped) = collect_files(&[(claude.clone(), root.clone())], std::slice::from_ref(&home));
        let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["commands/review.md", "skills/pdf/SKILL.md"]);
        assert_eq!(skipped, 4, "each left-out link is counted");
        let elsewhere = t.path().join("elsewhere");
        assert!(files.iter().all(|f| bundle_target(&elsewhere, claude.as_ref(), f).is_some()), "import takes everything export writes");
    }

    #[cfg(unix)]
    #[test]
    fn export_follows_links_to_skills_kept_elsewhere() {
        use std::os::unix::fs::symlink;
        let claude = crate::providers::get(ProviderId::Claude);
        let t = tempfile::tempdir().unwrap();
        let (root, repo, profiles) = (t.path().join("shared"), t.path().join("repo"), t.path().join("profiles"));
        for d in [root.join("skills").join("own"), repo.join("brainstorming"), repo.join("mixed"), profiles.join("work")] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(root.join("skills").join("own").join("SKILL.md"), "own").unwrap();
        std::fs::write(repo.join("brainstorming").join("SKILL.md"), "linked folder").unwrap();
        std::fs::write(repo.join("mixed").join("SKILL.md"), "linked file").unwrap();
        std::fs::write(profiles.join("work").join(".credentials.json"), "secret").unwrap();
        // The layout skill repos give: one link per skill, to a folder or to a single file.
        symlink(repo.join("brainstorming"), root.join("skills").join("brainstorming")).unwrap();
        std::fs::create_dir_all(root.join("skills").join("mixed")).unwrap();
        symlink(repo.join("mixed").join("SKILL.md"), root.join("skills").join("mixed").join("SKILL.md")).unwrap();
        // A kind folder linked above other accounts' homes is left out.
        symlink(&profiles, root.join("agents")).unwrap();

        let (files, skipped) = collect_files(&[(claude.clone(), root.clone())], &[profiles.join("work")]);
        let mut paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        paths.sort();
        assert_eq!(paths, ["skills/brainstorming/SKILL.md", "skills/mixed/SKILL.md", "skills/own/SKILL.md"]);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn export_reads_the_main_folder_when_there_is_no_shared_one() {
        let claude = crate::providers::get(ProviderId::Claude);
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join(".claude");
        std::fs::create_dir_all(home.join("skills").join("pdf")).unwrap();
        std::fs::write(home.join("skills").join("pdf").join("SKILL.md"), "skill").unwrap();
        std::fs::write(home.join(".credentials.json"), "secret").unwrap();
        let (files, _) = collect_files(&[(claude, home.clone())], &[home]);
        assert_eq!(files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(), ["skills/pdf/SKILL.md"]);
    }

    #[test]
    fn export_keeps_skills_named_like_a_sign_in_file() {
        let copilot = crate::providers::get(ProviderId::Copilot);
        let t = tempfile::tempdir().unwrap();
        let root = t.path().join("shared");
        for skill in ["config", "pdf"] {
            std::fs::create_dir_all(root.join("skills").join(skill)).unwrap();
            std::fs::write(root.join("skills").join(skill).join("SKILL.md"), skill).unwrap();
        }
        let (files, _) = collect_files(&[(copilot, root)], &[]);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn imported_homes_stay_in_that_tools_own_folders() {
        let home = util::home();
        assert_eq!(in_home(ProviderId::Claude, ".claude-work"), Some(home.join(".claude-work")));
        assert_eq!(in_home(ProviderId::Codex, ".codex-team/x"), Some(home.join(".codex-team").join("x")));
        assert_eq!(in_home(ProviderId::Gemini, ".gemini-alt"), Some(home.join(".gemini-alt")));
        for (p, path) in [
            (ProviderId::Codex, "src/myrepo"),
            (ProviderId::Codex, "Documents/x"),
            (ProviderId::Codex, ".claude-work"),
            (ProviderId::Claude, ".claudette"),
            (ProviderId::Claude, ".claude/skills"),
            (ProviderId::Claude, "../.claude-x"),
            (ProviderId::Claude, ""),
        ] {
            assert_eq!(in_home(p, path), None, "{path}");
        }
    }

    #[test]
    fn kdf_cost_from_a_file_is_capped() {
        assert_eq!(kdf_rounds(10_000_000), MAX_KDF_ROUNDS);
        assert_eq!(kdf_rounds(0), 1);
        assert_eq!(kdf_rounds(KDF_ROUNDS), KDF_ROUNDS);
    }

    #[cfg(unix)]
    #[test]
    fn imported_files_keep_their_exec_bit() {
        use std::os::unix::fs::PermissionsExt;
        let t = tempfile::tempdir().unwrap();
        let (script, doc) = (t.path().join("run.sh"), t.path().join("SKILL.md"));
        write_new(&script, b"echo", true).unwrap();
        write_new(&doc, b"doc", false).unwrap();
        assert_eq!(std::fs::metadata(&script).unwrap().permissions().mode() & 0o111, 0o111);
        let mode = std::fs::metadata(&doc).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0);
        assert_eq!(mode & 0o400, 0o400, "readable, not private like a sign-in");
        assert!(write_new(&doc, b"other", false).is_err(), "never overwrites");
        assert_eq!(std::fs::read_to_string(&doc).unwrap(), "doc");
        assert!(is_executable(&script) && !is_executable(&doc));
    }

    #[test]
    fn setup_files_open_by_content_whatever_their_name() {
        let dir = tempfile::tempdir().unwrap();
        let text = r#"{"format":"switchyard-setup","version":1,"exportedAt":"2026-10-01T00:00:00Z","fromOs":"macos","appVersion":"0.2.0","accounts":[]}"#;
        // Files saved before the rename end in .switchyard and carry the same format id.
        for name in ["old.switchyard", "new.agenthub", "copy.json"] {
            let p = dir.path().join(name);
            std::fs::write(&p, text).unwrap();
            assert_eq!(read_file(&p).unwrap().format, FORMAT, "{name}");
        }
        let p = dir.path().join("other.agenthub");
        std::fs::write(&p, text.replace("switchyard-setup", "something-else")).unwrap();
        assert!(read_file(&p).is_err());
    }

    /// The file written off the async workers is the whole setup file.
    #[tokio::test]
    async fn export_writes_a_readable_setup_file() {
        let t = tempfile::tempdir().unwrap();
        let state = AppState::load(t.path().join("data")).unwrap();
        let home = t.path().join(".claude-work").to_string_lossy().into_owned();
        *state.accounts.write().await = vec![Account { email: Some("w@x.io".into()), home_dir: home, ..crate::recommend::tests::account("w", "Max") }];
        let opts = ExportOptions { account_ids: vec!["w".into()], include_files: false, include_settings: true, passphrase: None };
        let path = t.path().join("setup.json");
        let summary = export(&state, &path, opts).await.unwrap();
        let file: SetupFile = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!((summary.accounts, file.format.as_str(), file.accounts[0].email.as_deref()), (1, FORMAT, Some("w@x.io")));
        assert!(file.settings.is_some() && file.sign_ins.is_none());
    }
}
