//! JSON persistence in the app data dir, plus the shared in-memory state managed by Tauri.
use crate::model::*;
use crate::notify::NotifyState;
use crate::providers::{self, Adapter};
use crate::util;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use tokio::sync::{Mutex, Notify, RwLock};

pub const MAX_NOTIFICATIONS: usize = 200;
pub const HISTORY_DAYS: i64 = 90;

const ACCOUNTS_FILE: &str = "accounts.json";
const SETTINGS_FILE: &str = "settings.json";
const NOTIFICATIONS_FILE: &str = "notifications.json";
const LATEST_FILE: &str = "usage-latest.json";
const HISTORY_FILE: &str = "usage-history.jsonl";
const ACTIVITY_FILE: &str = "activity-cache.json";
const NOTIFY_STATE_FILE: &str = "notify-state.json";
/// App data folder names used before the renames (same parent dir as the new one), newest first.
const LEGACY_DIR_NAMES: [&str; 2] = ["dev.baton.app", "dev.aihopper.app"];

/// Cached per-account activity scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    pub scanned_at: String,
    pub days: u32,
    pub items: Vec<DailyActivity>,
}

/// Everything the commands, poller and tray share. Managed by Tauri as `Arc<AppState>`.
pub struct AppState {
    dir: PathBuf,
    /// Built once so adapter-internal caches (version probes, tokens) survive between calls.
    pub adapters: Vec<Adapter>,
    pub accounts: RwLock<Vec<Account>>,
    pub settings: RwLock<AppSettings>,
    pub notifications: RwLock<Vec<AppNotification>>,
    pub latest: RwLock<HashMap<String, UsageSnapshot>>,
    pub activity: RwLock<HashMap<String, ActivityEntry>>,
    pub notify_state: Mutex<NotifyState>,
    /// Wakes the poller early (manual refresh, settings change).
    pub poll_wake: Notify,
    /// Serializes polls so a manual refresh and the timer never overlap.
    pub poll_lock: Mutex<()>,
    history_lock: Mutex<()>,
}

impl AppState {
    /// Load every file from `dir`, falling back to defaults for anything missing or unreadable.
    pub fn load(dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        for legacy in LEGACY_DIR_NAMES.iter().filter_map(|n| dir.parent().map(|p| p.join(n))) {
            match migrate_legacy(&legacy, &dir) {
                Ok(true) => {
                    log::info!("copied data from {}", legacy.display());
                    break;
                }
                Ok(false) => {}
                Err(e) => log::warn!("migrating {} failed: {e:#}", legacy.display()),
            }
        }
        let adapters = providers::all();
        let mut accounts = load_accounts(&dir.join(ACCOUNTS_FILE));
        flag_default_homes(&mut accounts, &adapters);
        let state = Self {
            adapters,
            accounts: RwLock::new(accounts),
            settings: RwLock::new(load_settings(&dir.join(SETTINGS_FILE))),
            notifications: RwLock::new(load_or_default(&dir.join(NOTIFICATIONS_FILE))),
            latest: RwLock::new(load_or_default(&dir.join(LATEST_FILE))),
            activity: RwLock::new(load_or_default(&dir.join(ACTIVITY_FILE))),
            notify_state: Mutex::new(load_or_default(&dir.join(NOTIFY_STATE_FILE))),
            poll_wake: Notify::new(),
            poll_lock: Mutex::new(()),
            history_lock: Mutex::new(()),
            dir,
        };
        if let Err(e) = prune_history(&state.dir.join(HISTORY_FILE), chrono::Utc::now()) {
            log::warn!("pruning usage history failed: {e:#}");
        }
        Ok(state)
    }

    pub fn adapter(&self, id: ProviderId) -> anyhow::Result<Adapter> {
        self.adapters.iter().find(|a| a.id() == id).cloned().ok_or_else(|| anyhow::anyhow!("no adapter for {}", id.as_str()))
    }

    pub fn data_dir(&self) -> &Path {
        &self.dir
    }

    /// The shared profile Agent Hub owns for a tool: <app data>/shared/<provider>.
    pub fn shared_dir(&self, p: ProviderId) -> PathBuf {
        self.dir.join("shared").join(p.as_str())
    }

    /// Where new isolated homes live: settings.profilesDir or <app data>/profiles.
    pub async fn profiles_dir(&self) -> PathBuf {
        match self.settings.read().await.profiles_dir.as_deref().map(str::trim) {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => self.dir.join("profiles"),
        }
    }

    pub async fn save_accounts(&self) -> anyhow::Result<()> {
        let accounts = self.accounts.read().await;
        util::write_json_atomic(&self.dir.join(ACCOUNTS_FILE), &*accounts)
    }

    pub async fn save_settings(&self) -> anyhow::Result<()> {
        let settings = self.settings.read().await;
        util::write_json_atomic(&self.dir.join(SETTINGS_FILE), &*settings)
    }

    pub async fn save_notifications(&self) -> anyhow::Result<()> {
        let list = self.notifications.read().await;
        util::write_json_atomic(&self.dir.join(NOTIFICATIONS_FILE), &*list)
    }

    pub async fn save_latest(&self) -> anyhow::Result<()> {
        let latest = self.latest.read().await;
        util::write_json_atomic(&self.dir.join(LATEST_FILE), &*latest)
    }

    pub async fn save_activity(&self) -> anyhow::Result<()> {
        let activity = self.activity.read().await;
        util::write_json_atomic(&self.dir.join(ACTIVITY_FILE), &*activity)
    }

    pub async fn save_notify_state(&self) -> anyhow::Result<()> {
        let st = self.notify_state.lock().await;
        util::write_json_atomic(&self.dir.join(NOTIFY_STATE_FILE), &*st)
    }

    /// Newest first, capped at MAX_NOTIFICATIONS.
    pub async fn push_notifications(&self, items: Vec<AppNotification>) -> anyhow::Result<()> {
        if items.is_empty() {
            return Ok(());
        }
        {
            let mut list = self.notifications.write().await;
            for n in items {
                list.insert(0, n);
            }
            list.truncate(MAX_NOTIFICATIONS);
        }
        self.save_notifications().await
    }

    pub async fn append_history(&self, points: &[UsageHistoryPoint]) -> anyhow::Result<()> {
        if points.is_empty() {
            return Ok(());
        }
        let _guard = self.history_lock.lock().await;
        append_history_file(&self.dir.join(HISTORY_FILE), points)
    }

    pub async fn read_history(&self, account_id: Option<&str>, days: u32) -> Vec<UsageHistoryPoint> {
        let _guard = self.history_lock.lock().await;
        let since = chrono::Utc::now() - chrono::Duration::days(i64::from(days.max(1)));
        let (path, account_id) = (self.dir.join(HISTORY_FILE), account_id.map(str::to_string));
        // Parsing up to 90 days of history is blocking work: keep it off the async workers.
        tokio::task::spawn_blocking(move || read_history_file(&path, account_id.as_deref(), since)).await.unwrap_or_default()
    }

    pub async fn account(&self, id: &str) -> Option<Account> {
        self.accounts.read().await.iter().find(|a| a.id == id).cloned()
    }
}

fn load_or_default<T: Default + for<'de> Deserialize<'de>>(path: &Path) -> T {
    match std::fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_else(|e| {
            log::warn!("{} is unreadable ({e}); starting fresh", path.display());
            backup_corrupt(path);
            T::default()
        }),
        Err(_) => T::default(),
    }
}

/// Keep a copy of a file we could not parse, so a bad write never silently loses data.
fn backup_corrupt(path: &Path) {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let _ = std::fs::copy(path, path.with_file_name(format!("{name}.corrupt-{stamp}")));
}

/// Accounts load one by one: a single malformed entry is skipped (the file is backed up) instead of
/// wiping the whole list.
fn load_accounts(path: &Path) -> Vec<Account> {
    let Some(raw) = util::read_json(path) else {
        if path.exists() {
            backup_corrupt(path);
        }
        return vec![];
    };
    let entries = raw.as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    let mut bad = false;
    for v in entries {
        match serde_json::from_value::<Account>(v) {
            Ok(a) => out.push(a),
            Err(e) => {
                log::warn!("skipping malformed account: {e}");
                bad = true;
            }
        }
    }
    if bad {
        backup_corrupt(path);
    }
    out
}

/// An account at its tool's default home is the main folder, even if it was saved without the flag.
/// Create, update and import set it the same way (accounts.rs), so the flag alone decides.
fn flag_default_homes(accounts: &mut [Account], adapters: &[Adapter]) {
    for a in accounts {
        if let Some(adapter) = adapters.iter().find(|x| x.id() == a.provider_id) {
            a.is_default_home |= util::path_key(&adapter.default_home()) == util::path_key(Path::new(&a.home_dir));
        }
    }
}

/// Settings merge onto defaults key by key, so new fields get defaults and a bad value only resets itself.
fn load_settings(path: &Path) -> AppSettings {
    let raw = util::read_json(path).unwrap_or(serde_json::Value::Null);
    settings_from_value(&raw)
}

pub fn settings_from_value(raw: &serde_json::Value) -> AppSettings {
    let defaults = AppSettings::default();
    let Ok(mut merged) = serde_json::to_value(&defaults) else {
        return defaults;
    };
    if let Some(obj) = raw.as_object() {
        for (k, v) in obj {
            let mut candidate = merged.clone();
            util::merge_json(&mut candidate, &serde_json::json!({ k: v }));
            if serde_json::from_value::<AppSettings>(candidate.clone()).is_ok() {
                merged = candidate;
            }
        }
    }
    let mut settings: AppSettings = serde_json::from_value(merged).unwrap_or(defaults);
    settings.permission_modes.sanitize();
    settings
}

fn append_history_file(path: &Path, points: &[UsageHistoryPoint]) -> anyhow::Result<()> {
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    let mut buf = Vec::new();
    for p in points {
        serde_json::to_writer(&mut buf, p)?;
        buf.push(b'\n');
    }
    f.write_all(&buf)?;
    Ok(())
}

fn read_history_file(path: &Path, account_id: Option<&str>, since: chrono::DateTime<chrono::Utc>) -> Vec<UsageHistoryPoint> {
    let Ok(f) = std::fs::File::open(path) else {
        return vec![];
    };
    std::io::BufReader::new(f)
        .lines()
        .map_while(Result::ok)
        .filter_map(|l| serde_json::from_str::<UsageHistoryPoint>(&l).ok())
        .filter(|p| account_id.is_none_or(|id| p.account_id == id))
        .filter(|p| util::parse_iso(&p.t).is_some_and(|t| t >= since))
        .collect()
}

/// Drop points older than HISTORY_DAYS (and unparseable lines). Rewrites the file atomically.
pub fn prune_history(path: &Path, now: chrono::DateTime<chrono::Utc>) -> anyhow::Result<()> {
    let Ok(f) = std::fs::File::open(path) else {
        return Ok(());
    };
    let cutoff = now - chrono::Duration::days(HISTORY_DAYS);
    let mut kept = Vec::new();
    let mut dropped = 0usize;
    for line in std::io::BufReader::new(f).lines().map_while(Result::ok) {
        let keep = serde_json::from_str::<UsageHistoryPoint>(&line).ok().and_then(|p| util::parse_iso(&p.t)).is_some_and(|t| t >= cutoff);
        if keep {
            kept.extend_from_slice(line.as_bytes());
            kept.push(b'\n');
        } else {
            dropped += 1;
        }
    }
    if dropped > 0 {
        util::write_atomic(path, &kept)?;
    }
    Ok(())
}

/// One-time copy of the pre-rename data into the new app data dir. Runs only while the new dir
/// has no accounts file; the legacy dir is left untouched. Profiles made by the old app stay where
/// they are, so the profiles dir setting points at them unless the user already chose one.
fn migrate_legacy(legacy: &Path, dir: &Path) -> anyhow::Result<bool> {
    if dir.join(ACCOUNTS_FILE).exists() || !legacy.join(ACCOUNTS_FILE).is_file() {
        return Ok(false);
    }
    for entry in std::fs::read_dir(legacy)? {
        let path = entry?.path();
        let is_data = matches!(path.extension().and_then(|e| e.to_str()), Some("json" | "jsonl"));
        if let (true, Some(name)) = (is_data && path.is_file(), path.file_name()) {
            std::fs::copy(&path, dir.join(name)).with_context(|| format!("copying {}", path.display()))?;
        }
    }
    let old_profiles = legacy.join("profiles");
    if old_profiles.is_dir() {
        let settings_path = dir.join(SETTINGS_FILE);
        let mut settings: serde_json::Value =
            std::fs::read_to_string(&settings_path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
        let unset = settings.get("profilesDir").and_then(|v| v.as_str()).is_none_or(|v| v.trim().is_empty());
        if let (true, Some(obj)) = (unset, settings.as_object_mut()) {
            obj.insert("profilesDir".into(), old_profiles.to_string_lossy().into_owned().into());
            util::write_atomic(&settings_path, serde_json::to_string_pretty(&settings)?.as_bytes())?;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_data_once_and_keeps_the_old_dir() {
        let root = tempfile::tempdir().unwrap();
        let (legacy, dir) = (root.path().join(LEGACY_DIR_NAMES[0]), root.path().join("dev.switchyard.app"));
        std::fs::create_dir_all(legacy.join("profiles")).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(legacy.join(ACCOUNTS_FILE), "[]").unwrap();
        std::fs::write(legacy.join(SETTINGS_FILE), r#"{"theme":"dark"}"#).unwrap();
        std::fs::write(legacy.join(HISTORY_FILE), "{}\n").unwrap();
        std::fs::write(legacy.join("notes.txt"), "x").unwrap();

        assert!(migrate_legacy(&legacy, &dir).unwrap());
        assert!(dir.join(ACCOUNTS_FILE).is_file() && dir.join(HISTORY_FILE).is_file());
        assert!(!dir.join("notes.txt").exists());
        assert!(legacy.join(ACCOUNTS_FILE).is_file());
        let raw = std::fs::read_to_string(dir.join(SETTINGS_FILE)).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(settings["theme"], "dark");
        assert_eq!(settings["profilesDir"], legacy.join("profiles").to_string_lossy().as_ref());

        // Second start: the new dir already has accounts, so nothing is copied again.
        std::fs::write(legacy.join(ACCOUNTS_FILE), "[1]").unwrap();
        assert!(!migrate_legacy(&legacy, &dir).unwrap());
        assert_eq!(std::fs::read_to_string(dir.join(ACCOUNTS_FILE)).unwrap(), "[]");
    }

    /// Saved before the flag was set (or by an older build): an account at the tool's default home
    /// still loads as the main folder.
    #[test]
    fn load_flags_accounts_at_the_default_home() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("dev.switchyard.app");
        std::fs::create_dir_all(&dir).unwrap();
        let main = providers::all().into_iter().find(|a| a.id() == ProviderId::Claude).unwrap().default_home();
        let account = |id: &str, home: &str| {
            serde_json::json!({
                "id": id, "providerId": "claude", "label": id, "homeDir": home, "isDefaultHome": false,
                "sharing": SharingOptions::default(), "color": "#3987E5", "status": "ok", "createdAt": "2026-01-01T00:00:00Z", "launchCount": 0
            })
        };
        let main_slash = format!("{}/", main.display());
        let raw = serde_json::json!([account("main", &main_slash), account("work", "/somewhere/else")]);
        std::fs::write(dir.join(ACCOUNTS_FILE), raw.to_string()).unwrap();

        let state = AppState::load(dir).unwrap();
        let accounts = state.accounts.blocking_read();
        assert_eq!(accounts.iter().map(|a| (a.id.as_str(), a.is_default_home)).collect::<Vec<_>>(), [("main", true), ("work", false)]);
    }

    #[test]
    fn migration_skips_when_there_is_no_legacy_data() {
        let root = tempfile::tempdir().unwrap();
        assert!(!migrate_legacy(&root.path().join("missing"), root.path()).unwrap());
    }

    fn point(id: &str, t: chrono::DateTime<chrono::Utc>) -> UsageHistoryPoint {
        UsageHistoryPoint { account_id: id.into(), t: t.to_rfc3339(), windows: HashMap::from([("w".into(), 10.0)]) }
    }

    #[test]
    fn settings_merge_keeps_good_fields_and_defaults_bad_ones() {
        let raw = serde_json::json!({
            "pollIntervalMinutes": 7,
            "theme": "not-a-theme",
            "notifications": { "enabled": false },
            "unknownField": 1
        });
        let s = settings_from_value(&raw);
        assert_eq!(s.poll_interval_minutes, 7);
        assert_eq!(s.theme, Theme::Dark);
        assert!(!s.notifications.enabled);
        assert_eq!(s.notifications.nearly_exhausted_percent, 80.0);
    }

    #[test]
    fn settings_permission_modes_load_with_and_without_the_field() {
        let old = serde_json::json!({ "pollIntervalMinutes": 7, "theme": "light" });
        assert_eq!(settings_from_value(&old).permission_modes, PermissionModes::default());
        let mut plain = serde_json::to_value(AppSettings::default()).unwrap();
        plain.as_object_mut().unwrap().remove("permissionModes");
        let parsed: AppSettings = serde_json::from_value(plain).unwrap();
        assert_eq!(parsed.permission_modes.claude, "default");

        let raw = serde_json::json!({ "permissionModes": { "claude": "bypassPermissions", "codex": "nope", "gemini": "yolo" } });
        let m = settings_from_value(&raw).permission_modes;
        assert_eq!((m.claude.as_str(), m.codex.as_str(), m.gemini.as_str(), m.copilot.as_str()), ("bypassPermissions", "default", "yolo", "default"));
    }

    #[test]
    fn history_append_read_and_prune() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(HISTORY_FILE);
        let now = chrono::Utc::now();
        let old = now - chrono::Duration::days(100);
        append_history_file(&path, &[point("a", old), point("a", now), point("b", now)]).unwrap();
        assert_eq!(read_history_file(&path, Some("a"), old - chrono::Duration::days(1)).len(), 2);
        assert_eq!(read_history_file(&path, None, now - chrono::Duration::days(1)).len(), 2);
        prune_history(&path, now).unwrap();
        assert_eq!(read_history_file(&path, None, old - chrono::Duration::days(1)).len(), 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn overlapping_saves_all_succeed_and_keep_the_newest_state() {
        let dir = tempfile::tempdir().unwrap();
        let state = std::sync::Arc::new(AppState::load(dir.path().to_path_buf()).unwrap());
        let tasks: Vec<_> = (1..=16u32)
            .map(|i| {
                let state = state.clone();
                tokio::spawn(async move {
                    state.settings.write().await.poll_interval_minutes = i;
                    state.save_settings().await
                })
            })
            .collect();
        for t in tasks {
            t.await.unwrap().unwrap();
        }
        let saved = load_settings(&dir.path().join(SETTINGS_FILE));
        assert_eq!(saved.poll_interval_minutes, state.settings.read().await.poll_interval_minutes);
    }

    #[test]
    fn bad_account_entries_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(ACCOUNTS_FILE);
        std::fs::write(&path, r#"[{"id":"broken"}]"#).unwrap();
        assert!(load_accounts(&path).is_empty());
    }
}
