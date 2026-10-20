//! Moving a Switchyard setup to another computer, on any OS. Export writes one `.switchyard` file.
//! Import only adds: accounts that are already here, files that already exist and sign-ins already
//! present are left exactly as they are, and nothing is ever deleted.
//!
//! Sign-ins are optional and always encrypted (AES-256-GCM, key from a passphrase via PBKDF2-SHA256).
//! Everything else in the file is plain JSON with no secrets in it.

use crate::commands;
use crate::model::*;
use crate::providers::Adapter;
use crate::store::AppState;
use crate::util;
use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{anyhow, bail, Context};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tauri::AppHandle;

const FORMAT: &str = "switchyard-setup";
const VERSION: u32 = 1;
const KDF_ROUNDS: u32 = 310_000;
/// Skill files larger than this are left out; the whole bundle is capped too.
const MAX_FILE: u64 = 5 * 1024 * 1024;
const MAX_BUNDLE: u64 = 100 * 1024 * 1024;
/// Sharing categories that are plain files worth carrying over (no logins, history or plugin state).
const PORTABLE_KINDS: [ShareKind; 3] = [ShareKind::Skills, ShareKind::Agents, ShareKind::Instructions];

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

/// A skill, agent or instructions file from a tool's main folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BundleFile {
    provider_id: ProviderId,
    /// "skills", "agents" or "instructions".
    kind: String,
    /// Relative to the tool's main folder, `/` separators.
    path: String,
    data: String,
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
            email: a.email.clone(),
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

    let (files, skipped_files) = if opts.include_files { collect_files(state, &accounts)? } else { (Vec::new(), 0) };
    let settings = if opts.include_settings { Some(portable_settings(&state.settings.read().await.clone())) } else { None };
    let sealed = match (passphrase, sign_ins.is_empty()) {
        (Some(p), false) => Some(seal(p, &serde_json::to_vec(&sign_ins)?)?),
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
    util::write_atomic(path, serde_json::to_string_pretty(&file)?.as_bytes()).with_context(|| format!("writing {}", path.display()))?;
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

/// Settings that mean the same on any computer (no paths, no terminal program).
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

/// Skills, agents and instructions from the main folder of every tool the exported accounts use.
fn collect_files(state: &AppState, accounts: &[Account]) -> anyhow::Result<(Vec<BundleFile>, usize)> {
    let mut providers: Vec<ProviderId> = accounts.iter().map(|a| a.provider_id).collect();
    providers.sort_by_key(|p| p.as_str());
    providers.dedup();
    let (mut out, mut skipped, mut total) = (Vec::new(), 0, 0u64);
    for provider in providers {
        let adapter = state.adapter(provider)?;
        let root = adapter.default_home();
        for (kind, rels) in adapter.shareable_paths().into_iter().filter(|(k, _)| PORTABLE_KINDS.contains(k)) {
            for rel in rels {
                for file in walk(&root.join(rel)) {
                    let size = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(u64::MAX);
                    if size > MAX_FILE || total + size > MAX_BUNDLE {
                        skipped += 1;
                        continue;
                    }
                    let (Ok(bytes), Ok(rel_path)) = (std::fs::read(&file), file.strip_prefix(&root)) else {
                        skipped += 1;
                        continue;
                    };
                    total += size;
                    out.push(BundleFile { provider_id: provider, kind: kind_name(kind).into(), path: to_portable(rel_path), data: B64.encode(bytes) });
                }
            }
        }
    }
    Ok((out, skipped))
}

/// Every regular file under `p` (or `p` itself), skipping VCS folders and dependency caches.
fn walk(p: &Path) -> Vec<PathBuf> {
    if p.is_file() {
        return vec![p.to_path_buf()];
    }
    walkdir::WalkDir::new(p)
        .follow_links(true)
        .into_iter()
        .filter_entry(|e| !matches!(e.file_name().to_str(), Some(".git" | "node_modules" | "__pycache__" | ".venv")))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect()
}

fn kind_name(k: ShareKind) -> &'static str {
    match k {
        ShareKind::Skills => "skills",
        ShareKind::Agents => "agents",
        ShareKind::Instructions => "instructions",
        _ => "other",
    }
}

fn to_portable(rel: &Path) -> String {
    rel.components().filter_map(|c| c.as_os_str().to_str()).collect::<Vec<_>>().join("/")
}

/// A relative `/` path from the file, refusing anything that could escape the target folder.
fn from_portable(base: &Path, rel: &str) -> Option<PathBuf> {
    let rel = Path::new(rel);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) || rel.as_os_str().is_empty() {
        return None;
    }
    Some(base.join(rel))
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

fn open(passphrase: &str, s: &Sealed) -> Option<Vec<u8>> {
    let salt = B64.decode(&s.salt).ok()?;
    let nonce = B64.decode(&s.nonce).ok().filter(|n| n.len() == 12)?;
    let data = B64.decode(&s.data).ok()?;
    let cipher = Aes256Gcm::new_from_slice(&key_from(passphrase, &salt, s.rounds.clamp(1, 10_000_000))).ok()?;
    cipher.decrypt(Nonce::from_slice(&nonce), data.as_ref()).ok()
}

// ---------- import ----------

fn read_file(path: &Path) -> anyhow::Result<SetupFile> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file: SetupFile = serde_json::from_str(&text).map_err(|_| anyhow!("This isn't a Switchyard setup file"))?;
    if file.format != FORMAT {
        bail!("This isn't a Switchyard setup file");
    }
    if file.version > VERSION {
        bail!("This file comes from a newer Switchyard. Update Switchyard on this computer first.");
    }
    Ok(file)
}

fn unlock(file: &SetupFile, passphrase: Option<&str>) -> Option<SignIns> {
    let plain = open(passphrase?.trim(), file.sign_ins.as_ref()?)?;
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
        HomeRef::InHome { path } => (from_portable(&util::home(), path).unwrap_or_else(|| default_home.clone()), false),
        HomeRef::Elsewhere => (PathBuf::new(), false),
    };
    let accounts = state.accounts.read().await;
    let same_login = |x: &&Account| {
        x.provider_id == a.provider_id
            && a.email.as_deref().is_some_and(|e| x.email.as_deref().is_some_and(|y| y.eq_ignore_ascii_case(e)))
    };
    let same_folder = |x: &&Account| !home.as_os_str().is_empty() && util::path_key(Path::new(&x.home_dir)) == util::path_key(&home);
    let already_here = accounts.iter().find(|x| same_login(x) || same_folder(x)).map(|x| x.label.clone());
    drop(accounts);
    let home = if home.as_os_str().is_empty() { commands::new_home_dir(state, a.provider_id, &a.label).await } else { home };
    Ok(Plan { adapter, home, is_default, already_here })
}

pub async fn preview(state: &Arc<AppState>, path: &Path, passphrase: Option<String>) -> anyhow::Result<ImportPreview> {
    let file = read_file(path)?;
    let unlocked = unlock(&file, passphrase.as_deref()).is_some();
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
        let root = state.adapter(f.provider_id)?.default_home();
        let new = from_portable(&root, &f.path).is_some_and(|p| !p.exists());
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
        match (file.sign_ins.is_some(), unlock(&file, passphrase.as_deref())) {
            (true, None) => bail!("That passphrase doesn't open the sign-ins in this file"),
            (_, s) => s.unwrap_or_default(),
        }
    } else {
        SignIns::new()
    };
    let mut report = ImportReport::default();

    for a in file.accounts.iter().filter(|a| choice.account_keys.contains(&a.key)) {
        let plan = plan_account(state, a).await?;
        if let Some(here) = &plan.already_here {
            report.skipped.push(format!("{} (already here as {here})", a.label));
            continue;
        }
        std::fs::create_dir_all(&plan.home).with_context(|| format!("creating {}", plan.home.display()))?;
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
        match commands::create_account(app, state, input).await {
            Ok(created) => {
                if let Some(notes) = a.notes.as_deref().filter(|n| !n.trim().is_empty()) {
                    let _ = commands::update_account(app, state, &created.id, serde_json::json!({ "notes": notes })).await;
                }
                report.added.push(a.label.clone());
            }
            Err(e) => report.problems.push(format!("{}: {e:#}", a.label)),
        }
    }

    if choice.files {
        let mut roots: HashMap<ProviderId, PathBuf> = HashMap::new();
        for f in &file.files {
            let root = match roots.get(&f.provider_id) {
                Some(r) => r.clone(),
                None => {
                    let r = state.adapter(f.provider_id)?.default_home();
                    roots.insert(f.provider_id, r.clone());
                    r
                }
            };
            let Some(target) = from_portable(&root, &f.path) else { continue };
            if target.exists() {
                report.files_kept += 1;
                continue;
            }
            let Ok(bytes) = B64.decode(&f.data) else { continue };
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            util::write_atomic(&target, &bytes)?;
            report.files_added += 1;
        }
    }

    if let (true, Some(settings)) = (choice.settings, file.settings.clone()) {
        commands::apply_settings_patch(app, state, settings).await?;
        report.settings = true;
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
}
