//! App settings: merging a partial patch from the UI or a setup file, then saving and applying it.
use crate::model::*;
use crate::store::AppState;
use crate::util;
use crate::util::non_empty;
use anyhow::anyhow;
use tauri::AppHandle;

/// Deep-merge a partial settings patch onto `current`, then clamp and clean the result.
fn merge_settings(current: &AppSettings, patch: &serde_json::Value) -> Result<AppSettings, String> {
    let mut v = serde_json::to_value(current).map_err(|e| e.to_string())?;
    util::merge_json(&mut v, patch);
    let mut new: AppSettings = serde_json::from_value(v).map_err(|e| format!("Invalid settings: {e}"))?;
    new.poll_interval_minutes = new.poll_interval_minutes.clamp(1, 24 * 60);
    let n = &mut new.notifications;
    n.nearly_exhausted_percent = n.nearly_exhausted_percent.clamp(1.0, 99.0);
    n.use_it_or_lose_it_hours = n.use_it_or_lose_it_hours.clamp(1.0, 24.0 * 31.0);
    n.use_it_or_lose_it_min_left_percent = n.use_it_or_lose_it_min_left_percent.clamp(0.0, 100.0);
    new.default_cwd = non_empty(new.default_cwd);
    new.profiles_dir = non_empty(new.profiles_dir);
    new.permission_modes.sanitize();
    Ok(new)
}

/// Merge a partial settings object, clamp the numbers, save, and wake the poller if its timing changed.
pub(crate) async fn apply_settings_patch(app: &AppHandle, state: &AppState, patch: serde_json::Value) -> anyhow::Result<AppSettings> {
    if !patch.is_object() {
        return Err(anyhow!("Settings patch must be an object"));
    }
    let (new, wake) = {
        let mut settings = state.settings.write().await;
        let new = merge_settings(&settings, &patch).map_err(|e| anyhow!(e))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_patch_merges_and_cleans_permission_modes() {
        let current = AppSettings::default();
        let new = merge_settings(&current, &serde_json::json!({ "permissionModes": { "claude": "bypassPermissions" } })).unwrap();
        assert_eq!(new.permission_modes.claude, "bypassPermissions");
        assert_eq!(new.permission_modes.codex, "default");
        assert_eq!(new.poll_interval_minutes, current.poll_interval_minutes);

        let new = merge_settings(&new, &serde_json::json!({ "permissionModes": { "codex": "bypass", "copilot": "everything" } })).unwrap();
        assert_eq!(new.permission_modes.claude, "bypassPermissions");
        assert_eq!(new.permission_modes.codex, "bypass");
        assert_eq!(new.permission_modes.copilot, "default");
    }
}
