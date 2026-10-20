//! App setup: plugins, state, commands, background poller and the system tray.
pub mod commands;
pub mod knowledge;
pub mod launcher;
pub mod model;
pub mod notify;
pub mod providers;
pub mod pty;
pub mod recommend;
pub mod sharing;
pub mod store;
pub mod transfer;
pub mod usage;
pub mod util;

use model::{LaunchRequest, LaunchTarget};
use std::sync::Arc;
use store::AppState;
use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

/// User-facing product name for Rust-side strings (tray, messages). Rename here only.
pub const APP_NAME: &str = "Switchyard";
const TRAY_ID: &str = "main";
const MAIN_WINDOW: &str = "main";
/// Passed by the login item; the app then starts hidden in the tray.
const MINIMIZED_ARG: &str = "--minimized";

pub fn run() {
    let app = tauri::Builder::default()
        // Must be the first plugin: a second launch just focuses the running window.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main_window(app)))
        .plugin(tauri_plugin_autostart::Builder::new().arg(MINIMIZED_ARG).build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            let state = Arc::new(AppState::load(dir)?);
            app.manage(state.clone());
            app.manage(pty::Terminals::default());
            build_tray(app.handle())?;
            sync_autostart(app.handle(), &state);
            if std::env::args().any(|a| a == MINIMIZED_ARG) {
                if let Some(w) = app.get_webview_window(MAIN_WINDOW) {
                    let _ = w.hide();
                }
            }
            // Seeding copies files: keep it off the main thread.
            tauri::async_runtime::spawn(knowledge::migrate(app.handle().clone(), state.clone()));
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
            commands::terminal_spawn,
            commands::terminal_write,
            commands::terminal_resize,
            commands::terminal_kill,
            commands::terminal_list,
            commands::terminal_scrollback,
            commands::recommend,
            commands::notifications_list,
            commands::notifications_mark_read,
            commands::settings_get,
            commands::settings_set,
            commands::transfer_export,
            commands::transfer_preview,
            commands::transfer_import,
            commands::system_open_path,
        ])
        .build(tauri::generate_context!());
    match app {
        Ok(app) => app.run(|app, event| {
            if let RunEvent::Exit = event {
                if let Some(terms) = app.try_state::<pty::Terminals>() {
                    terms.kill_all();
                }
            }
        }),
        Err(e) => {
            eprintln!("{APP_NAME} failed to start: {e}");
            std::process::exit(1);
        }
    }
}

/// Register or remove the OS login item.
pub fn set_autostart(app: &AppHandle, enabled: bool) -> anyhow::Result<()> {
    let manager = app.autolaunch();
    let result = if enabled { manager.enable() } else { manager.disable() };
    result.map_err(|e| anyhow::anyhow!("Could not change start at login: {e}"))
}

/// Make the OS login item match settings.launchAtLogin (e.g. after the app moved or the user
/// removed the item by hand). Only touches the OS when they differ.
fn sync_autostart(app: &AppHandle, state: &AppState) {
    let wanted = state.settings.try_read().map(|s| s.launch_at_login).unwrap_or(false);
    let current = app.autolaunch().is_enabled().unwrap_or(!wanted);
    if current != wanted {
        if let Err(e) = set_autostart(app, wanted) {
            log::warn!("{e:#}");
        }
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
        .tooltip(APP_NAME)
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
    menu.append(&MenuItem::with_id(app, "terminals", "Open terminal grid", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "show", format!("Show {APP_NAME}"), true, None::<&str>)?)?;
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
        "terminals" => {
            show_main_window(app);
            let _ = app.emit("navigate", "terminals");
        }
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
