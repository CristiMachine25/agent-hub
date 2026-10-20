//! App setup: plugins, state, commands, background poller and the system tray.
pub mod commands;
pub mod launcher;
pub mod model;
pub mod notify;
pub mod providers;
pub mod recommend;
pub mod sharing;
pub mod store;
pub mod usage;
pub mod util;

use model::{LaunchRequest, LaunchTarget};
use std::sync::Arc;
use store::AppState;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};

const TRAY_ID: &str = "main";
const MAIN_WINDOW: &str = "main";

pub fn run() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let state = Arc::new(AppState::load(dir)?);
            app.manage(state.clone());
            build_tray(app.handle())?;
            usage::start(app.handle().clone(), state);
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == MAIN_WINDOW && minimize_to_tray(window.app_handle()) {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::providers_list,
            commands::accounts_list,
            commands::accounts_detect,
            commands::accounts_import,
            commands::accounts_create,
            commands::accounts_update,
            commands::accounts_remove,
            commands::accounts_apply_sharing,
            commands::usage_latest,
            commands::usage_history,
            commands::usage_activity,
            commands::usage_refresh,
            commands::usage_set_manual,
            commands::launch,
            commands::recommend,
            commands::notifications_list,
            commands::notifications_mark_read,
            commands::settings_get,
            commands::settings_set,
            commands::system_open_path,
        ])
        .run(tauri::generate_context!());
    if let Err(e) = result {
        eprintln!("AI Hopper failed to start: {e}");
        std::process::exit(1);
    }
}

fn minimize_to_tray(app: &AppHandle) -> bool {
    app.try_state::<Arc<AppState>>()
        .and_then(|s| s.settings.try_read().ok().map(|s| s.minimize_to_tray))
        .unwrap_or(true)
}

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let menu = static_menu(app, &[])?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("AI Hopper")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

/// Launch items first, then the fixed entries.
fn static_menu(app: &AppHandle, launch_items: &[(String, String)]) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for (id, text) in launch_items {
        menu.append(&MenuItem::with_id(app, id, text, true, None::<&str>)?)?;
    }
    if !launch_items.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&MenuItem::with_id(app, "refresh", "Refresh usage", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "show", "Show AI Hopper", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?)?;
    Ok(menu)
}

/// Rebuild the tray menu from current accounts and usage. Cheap; call after polls and edits.
pub fn refresh_tray(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>().map(|s| s.inner().clone()) else { return };
        let mut accounts = state.accounts.read().await.clone();
        let latest = state.latest.read().await.clone();
        accounts.sort_by_key(|a| (a.pinned != Some(true), a.provider_id.as_str(), a.label.to_lowercase()));
        let items: Vec<(String, String)> = accounts
            .iter()
            .map(|a| {
                let left = latest
                    .get(&a.id)
                    .and_then(recommend::left_percent)
                    .map(|l| format!(" \u{2014} {l:.0}% left"))
                    .unwrap_or_default();
                (format!("launch:{}", a.id), format!("Launch {}{left}", notify::display_name(a)))
            })
            .collect();
        let app2 = app.clone();
        // Menus must be built on the main thread on macOS.
        let _ = app.run_on_main_thread(move || {
            let Some(tray) = app2.tray_by_id(TRAY_ID) else { return };
            match static_menu(&app2, &items) {
                Ok(menu) => {
                    let _ = tray.set_menu(Some(menu));
                }
                Err(e) => log::warn!("tray menu: {e}"),
            }
        });
    });
}

fn on_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id().as_ref() {
        "quit" => app.exit(0),
        "show" => show_main_window(app),
        "refresh" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Some(state) = app.try_state::<Arc<AppState>>().map(|s| s.inner().clone()) {
                    usage::poll(&app, &state, None).await;
                }
            });
        }
        id => {
            if let Some(account_id) = id.strip_prefix("launch:") {
                tray_launch(app, account_id.to_string());
            }
        }
    }
}

fn tray_launch(app: &AppHandle, account_id: String) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let Some(state) = app.try_state::<Arc<AppState>>().map(|s| s.inner().clone()) else { return };
        let req = LaunchRequest { account_id, target: LaunchTarget::Cli, cwd: None, resume: None, extra_args: None };
        let error = match commands::launch_account(&app, &state, req).await {
            Ok(r) if r.ok => None,
            Ok(r) => r.error,
            Err(e) => Some(format!("{e:#}")),
        };
        if let Some(e) = error {
            notify::show_native(&app, "Could not launch", &e);
        }
    });
}
