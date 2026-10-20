//! The tray icon — the Windows stand-in for the macOS `MenuBarExtra`.
//!
//! Keep native items alive and update their state in place.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::state::AppState;

pub const TRAY_ID: &str = "easydict-tray";
const CHECK_AUTO_SELECT: &str = "autoSelect";

/// Installs the tray icon. Safe to call once per process.
pub fn install(app: &AppHandle, state: &Arc<AppState>) -> tauri::Result<()> {
    let menu = build_menu(app, state)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("Easydict")
        .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            // The macOS app shows the main window on a menu-bar click; the
            // closest Windows equivalent is a left click on the icon.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                show(app, "main");
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    app.manage(menu);
    Ok(())
}

/// Queue UI work without holding a settings transaction on the event loop.
pub fn refresh(app: &AppHandle) {
    if app.try_state::<Menu<tauri::Wry>>().is_none() {
        return;
    }
    let handle = app.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        let Some(state) = handle.try_state::<Arc<AppState>>() else { return };
        // Read on delivery: an older queued refresh must not undo a newer edit.
        let settings = state.settings();
        let menu = handle.state::<Menu<tauri::Wry>>();
        let result = (|| -> tauri::Result<()> {
            if let Some(item) = menu.get(CHECK_AUTO_SELECT) {
                if let Some(check) = item.as_check_menuitem() {
                    check.set_checked(settings.auto_show_query_icon)?;
                }
            }
            match (settings.show_ocr_menu_items, menu.get("showOCRWindow")) {
                (true, None) => menu.insert(
                    &MenuItem::with_id(&handle, "showOCRWindow", "OCR 窗口", true, None::<&str>)?,
                    6,
                )?,
                (false, Some(item)) => menu.remove(&item)?,
                _ => {}
            }
            Ok(())
        })();
        if let Err(error) = result {
            log::error!("could not refresh the tray menu: {error}");
        }
    }) {
        log::error!("could not schedule tray menu refresh: {error}");
    }
}

fn build_menu(app: &AppHandle, state: &AppState) -> tauri::Result<Menu<tauri::Wry>> {
    let settings = state.settings();
    let menu = Menu::new(app)?;

    let item = |id: &str, label: &str| MenuItem::with_id(app, id, label, true, None::<&str>);
    let separator = || PredefinedMenuItem::separator(app);

    menu.append(&item("inputTranslate", "输入翻译")?)?;
    menu.append(&item("screenshotTranslate", "截图翻译")?)?;
    menu.append(&item("selectTranslate", "划词翻译")?)?;
    menu.append(&item("pasteboardTranslate", "剪贴板翻译")?)?;
    menu.append(&separator()?)?;

    menu.append(&item("showMiniWindow", "迷你窗口")?)?;
    if settings.show_ocr_menu_items {
        menu.append(&item("showOCRWindow", "OCR 窗口")?)?;
    }
    menu.append(&separator()?)?;

    let auto_select = CheckMenuItem::with_id(
        app,
        CHECK_AUTO_SELECT,
        "自动划词",
        true,
        settings.auto_show_query_icon,
        None::<&str>,
    )?;
    menu.append(&auto_select)?;
    menu.append(&separator()?)?;

    menu.append(&item("favorites", "收藏")?)?;
    menu.append(&item("history", "历史记录")?)?;
    menu.append(&separator()?)?;

    menu.append(&item("settings", "偏好设置…")?)?;
    menu.append(&item("about", "关于 Easydict")?)?;
    menu.append(&separator()?)?;

    menu.append(&PredefinedMenuItem::quit(app, Some("退出 Easydict"))?)?;
    Ok(menu)
}

fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        CHECK_AUTO_SELECT => crate::toggle_auto_selection(app),
        "inputTranslate" => show(app, "main"),
        "showMiniWindow" => show(app, "mini"),
        "showOCRWindow" => show(app, "ocr"),
        // Everything else is an interaction the front-end owns.
        action @ ("screenshotTranslate" | "selectTranslate" | "pasteboardTranslate") => {
            dispatch(app, action);
        }
        "settings" => open_settings(app),
        "favorites" | "history" => {
            open_settings(app);
            dispatch(app, id);
        }
        "about" => {
            open_settings(app);
            dispatch(app, "about");
        }
        _ => {}
    }
}

fn show(app: &AppHandle, label: &str) {
    crate::window::request_show(app, label);
}

pub fn open_settings(app: &AppHandle) {
    show(app, "settings");
    crate::dispatch(app, "openSettings", None);
}

fn dispatch(app: &AppHandle, action: &str) {
    if action.is_empty() {
        return;
    }
    crate::dispatch(app, action, None);
}
