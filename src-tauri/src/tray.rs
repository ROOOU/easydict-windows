//! The tray icon — the Windows stand-in for the macOS `MenuBarExtra`.
//!
//! The menu is rebuilt whenever a menu item's state changes, because Tauri's
//! check items cannot be mutated in place.

use std::sync::Arc;

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

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
    Ok(())
}

/// Rebuilds the tray menu so the check items reflect the current settings.
pub fn refresh(app: &AppHandle, state: &AppState) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    if let Ok(menu) = build_menu(app, state) {
        let _ = tray.set_menu(Some(menu));
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
