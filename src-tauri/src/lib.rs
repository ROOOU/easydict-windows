//! Easydict for Windows — application entry point.
//!
//! The macOS app is built around three floating windows, a menu-bar item and a
//! set of system-wide shortcuts. The Windows port keeps that shape: a
//! translation window, a mini window, a fixed window, an OCR window, a settings
//! window and a tray icon.
//!
//! Module map (mirroring the upstream `Easydict/Swift` layout where it maps):
//!
//! | here             | upstream                                        |
//! |------------------|-------------------------------------------------|
//! | `model`          | `Model/`, `Service/Model/`                      |
//! | `config`         | `Feature/Configuration/`                        |
//! | `service`        | `Service/`                                      |
//! | `detect`         | `Service/Detect/` + `NLLanguageRecognizer`      |
//! | `query`          | `objc/Service/Model/EZQueryService.*`           |
//! | `store`          | `Model/Storage/`                                |
//! | `ocr`            | Vision (`VNRecognizeTextRequest`)               |
//! | `tts`            | `NSSpeechSynthesizer` + `AVPlayer`              |
//! | `platform`       | `Accessibility` + `CGWindowListCreateImage`     |
//! | `window`         | `EZWindowManager`                               |

pub mod commands;
pub mod config;
pub mod desktop;
pub mod detect;
pub mod model;
pub mod net;
pub mod ocr;
pub mod platform;
pub mod query;
pub mod service;
pub mod state;
pub mod store;
pub mod system;
pub mod tray;
pub mod tts;
pub mod util;
pub mod window;
pub mod window_frame;

use std::sync::Arc;

use tauri::{Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Shortcut, ShortcutState};

use crate::config::SHORTCUT_ACTIONS;
use crate::state::AppState;
use crate::store::Store;

// Cargo's rustc-link-arg-tests covers integration tests, not this lib's test
// harness. Give native unit tests the same v6 controls manifest as the app.
#[cfg(all(test, target_env = "msvc"))]
#[link(name = "resource", kind = "static", modifiers = "-bundle")]
unsafe extern "C" {}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    handle_maintenance_argument();
    register_url_scheme();

    builder()
        .build(tauri::generate_context!())
        .unwrap_or_else(|error| {
            log::error!("Easydict could not start: {error}");
            #[cfg(windows)]
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
                use windows::core::{PCWSTR, w};
                let message: Vec<u16> = error.to_string().encode_utf16().chain(Some(0)).collect();
                let _ = MessageBoxW(None, PCWSTR(message.as_ptr()), w!("Easydict"), MB_OK | MB_ICONERROR);
            }
            std::process::exit(1);
        })
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                // A window dragged in the last moments before quitting is still
                // waiting for its frame to settle; the process is about to end,
                // so write it now instead of losing it.
                crate::window_frame::flush(app);
            }
        });
}

/// `app.exe --unregister-url-scheme`: the way back out of the registration
/// written on every start. A Windows GUI process has no console to report to,
/// so the exit code carries the outcome — 0 removed, 1 nothing to remove,
/// 2 failed. Handled before the event loop, so a running instance cannot
/// swallow the flag by forwarding it to itself.
fn handle_maintenance_argument() {
    if !std::env::args().skip(1).any(|argument| argument == "--unregister-url-scheme") {
        return;
    }
    let code = match crate::platform::protocol::unregister_self() {
        Ok(true) => 0,
        Ok(false) => 1,
        Err(error) => {
            log::error!("could not remove the easydict:// registration: {error}");
            2
        }
    };
    std::process::exit(code);
}

/// The Windows counterpart of `CFBundleURLTypes` in upstream's `Info.plist`.
///
/// Runs before the logger plugin exists, and a GUI process has no console to
/// print to, so a failure is not reported anywhere — it is *visible* as a
/// missing `shell\open\command` value, which is what the protocol check reads
/// back. A link is one input method among several, so a registration that fails
/// must not stop the app from starting.
fn register_url_scheme() {
    // A development build lives in `target\…`, is rebuilt constantly, and would
    // otherwise take `easydict://` away from an installed copy that outlives it.
    if cfg!(debug_assertions) {
        return;
    }
    // Deliberately ignored: see above. The next start retries the write, and
    // `protocol-check.ps1` reads the value back.
    let _ = crate::platform::protocol::register_self();
}

/// Shared production plugins and IPC, also used by the isolated resource probe.
pub fn builder() -> tauri::Builder<tauri::Wry> {
    let builder = tauri::Builder::default();

    // The single-instance plugin must be registered before any other plugin so
    // a second launch is forwarded to the running window instead of starting a
    // new process.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        // A launch of a registered `easydict://` link reaches this process as
        // the second instance's command line.
        if dispatch_deep_links(app, &argv) == 0 && app.try_state::<Arc<AppState>>().is_some() {
            crate::window::request_show(app, "main");
        }
    }));

    builder
        .plugin(tauri_plugin_log::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    on_global_shortcut(app, shortcut);
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle().clone();

            let data_dir = handle
                .path()
                .app_data_dir()?;

            let store = Arc::new(Store::open(data_dir.clone())?);
            let state = AppState::new(handle.clone(), store).map_err(|error| {
                anyhow::anyhow!(
                    "无法读取或保护设置，原文件未被覆盖。\n{}\n\n请使用原 Windows 账户重试，或在备份后移走该文件再启动，并从导出文件恢复。\n\n{error}",
                    data_dir.join("settings.json").display()
                )
            })?;
            let settings = state.settings();

            app.manage(state.clone());
            app.manage(window::Windows::default());
            desktop::initialize_shortcuts(&handle, &settings);

            app.manage(platform::auto_selection::AutoSelection::start(&state));

            // The tray is the only route back to a hidden window, so a tray that
            // fails to install must not abort startup: log it and fall through to
            // showing the main window, rather than leaving an invisible process
            // that no shortcut and no window can reach.
            let tray_ready = match crate::tray::install(&handle, &state) {
                Ok(()) => true,
                Err(error) => {
                    log::error!("could not install the tray icon: {error}");
                    false
                }
            };

            // A fresh install hides the window and lives in the tray, which is
            // what `EZConfiguration_kHideMainWindowKey` defaults to.
            if !settings.hide_main_window || !tray_ready || settings.first_launch {
                crate::window::request_show(&handle, "main");
            }
            if settings.first_launch {
                state.update_settings(|settings| {
                    settings.first_launch = false;
                    Ok(())
                }).map_err(anyhow::Error::msg)?;
            }

            // A link that started this process is in this process's own command
            // line — Windows runs the registered command directly — so there is
            // no second instance to forward it here. Links that arrive while the
            // app already runs come through the single-instance plugin instead.
            // Routed last, because the handler needs the settings above.
            dispatch_deep_links(&handle, &std::env::args().collect::<Vec<_>>());

            Ok(())
        })
        .on_window_event(|window, event| match event {
            WindowEvent::CloseRequested { api, .. } => {
                // Closing a floating window hides it; only the tray's Quit
                // entry actually exits, so shortcuts keep working.
                api.prevent_close();
                let _ = window.hide();
                // The window keeps its results for the next open, but its
                // WebView does not have to keep the renderer alive meanwhile.
                crate::platform::webview_memory::release_when_hidden(window.app_handle(), window.label());
            }
            // Where the user put a window is remembered, so the "former"
            // placement can return to it after a restart — `EZLayoutManager.m:217`
            // does the same on every change. Both arms only record the frame and
            // hand it to a worker; the settings document is never written on the
            // event loop.
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => {
                crate::window_frame::note(window.app_handle(), window.label());
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::get_desktop_status,
            commands::update_settings,
            commands::reset_settings,
            commands::export_settings,
            commands::import_settings,
            commands::list_services,
            commands::get_service_config,
            commands::update_service_config,
            commands::validate_service,
            commands::fetch_service_models,
            commands::service_languages,
            commands::query,
            commands::stop_query,
            commands::retry_query,
            commands::query_single,
            commands::clipboard_text,
            commands::set_clipboard_text,
            commands::selected_text,
            commands::translate_text,
            commands::replace_selected_text,
            commands::ocr_image,
            commands::screenshot_ocr,
            commands::pasteboard_ocr,
            commands::play_tts,
            commands::stop_tts,
            commands::detect_language,
            commands::supported_languages,
            commands::system_languages,
            commands::toggle_languages,
            commands::history,
            commands::search_history,
            commands::delete_history,
            commands::clear_history,
            commands::history_count,
            commands::favorites,
            commands::add_favorite,
            commands::remove_favorite,
            commands::is_favorite,
            commands::show_window,
            commands::next_action,
            commands::open_settings,
            commands::report_action_error,
            commands::hide_window,
            commands::pin_window,
            commands::set_window_size,
            commands::fit_window_to_content,
            commands::open_link,
            commands::app_info,
            commands::service_order,
            commands::set_service_order,
            commands::show_service_result,
        ])
}

/// Maps a pressed global shortcut back to its action id and dispatches it.
fn on_global_shortcut(app: &tauri::AppHandle, shortcut: &Shortcut) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let settings = state.settings();

    let action = SHORTCUT_ACTIONS.iter().find(|a| {
        a.global
            && settings
                .shortcuts
                .get(a.id)
                .and_then(|b| b.trim().parse::<Shortcut>().ok())
                .as_ref()
                == Some(shortcut)
    });

    match action.map(|a| a.id) {
        Some("inputTranslate") => {
            deliver(app, "main", commands::ActionPayload::new("inputText", None));
        }
        Some("showMiniWindow") => {
            crate::window::request_show(app, "mini");
        }
        Some("selectTranslate") => dispatch(app, "selectTranslate", None),
        Some("snipTranslate") => dispatch(app, "snipTranslate", None),
        Some("pasteboardTranslate") => dispatch(app, "pasteboardTranslate", None),
        Some("translateAndReplace") => dispatch(app, "translateAndReplace", None),
        Some("polishAndReplace") => dispatch(app, "polishAndReplace", None),
        Some("toggleAutoSelectText") => toggle_auto_selection(app),
        Some("screenshotOCR") => dispatch(app, "screenshotOCR", None),
        Some("silentScreenshotOCR") => dispatch(app, "silentScreenshotOCR", None),
        Some("pasteboardOCR") => dispatch(app, "pasteboardOCR", None),
        Some("showOCRWindow") => {
            crate::window::request_show(app, "ocr");
        }
        _ => {}
    }
}

pub(crate) fn toggle_auto_selection(app: &tauri::AppHandle) {
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        if let Err(error) = state.update_settings(|settings| {
            settings.auto_show_query_icon = !settings.auto_show_query_icon;
            Ok(())
        }) {
            log::warn!("could not change automatic selection: {error}");
            dispatch(app, "selectionError", Some(error));
        }
    }
}

/// Hands an action to the focused window's front-end, which owns the
/// interaction.
pub(crate) fn dispatch(app: &tauri::AppHandle, action: &str, text: Option<String>) {
    let mut payload = commands::ActionPayload::new(action, text);
    if matches!(action, "selectTranslate" | "translateAndReplace" | "polishAndReplace") {
        // Capture the source before loading a WebView. Never read or paste into
        // whichever unrelated app happens to be foreground after it loads.
        payload.selection_target = Some(platform::selection_target().unwrap_or([0, 0]));
    }

    let focused = ["main", "mini", "fixed"].into_iter().find(|label| {
        app.get_webview_window(label).is_some_and(|window| window.is_focused().unwrap_or(false))
    });
    let selection = app.try_state::<Arc<AppState>>()
        .map(|state| state.settings().shortcut_select_translate_window_type)
        .unwrap_or(crate::model::WindowType::Fixed);
    let target = action_target(action, selection, focused);
    deliver(app, target, payload);
}

pub(crate) fn deliver(app: &tauri::AppHandle, target: &str, payload: commands::ActionPayload) {
    if let Err(error) = window::enqueue(app, target, payload) {
        window::report_error(app, &error);
    }
}

fn action_target(action: &str, selection: crate::model::WindowType, focused: Option<&'static str>) -> &'static str {
    match action {
        "selectTranslate" => selection.label(),
        "screenshotOCR" | "silentScreenshotOCR" | "pasteboardOCR" => "ocr",
        "favorites" | "history" | "about" | "openSettings" => "settings",
        _ => focused.unwrap_or("main"),
    }
}

/// Routes every `easydict://…` link in a full command line, the executable
/// first, as Windows passes it, and reports how many were handled.
pub fn dispatch_deep_links(app: &tauri::AppHandle, arguments: &[String]) -> usize {
    let mut handled = 0;
    for argument in arguments.iter().skip(1) {
        if let Some(link) = crate::platform::protocol::parse_link(argument) {
            handled += 1;
            handle_deep_link(app, link);
        }
    }
    handled
}

/// Splits a link into its route and query string.
///
/// Windows does not hand the registered `%1` through untouched: `ShellExecute`
/// normalises the URL first, so `easydict://query?text=hi` arrives as
/// `easydict://query/?text=hi` — a trailing slash after the host, and `ocr`
/// arrives as `easydict://ocr/`. Matching the raw text silently dropped every
/// link, which is exactly what the log showed. A trailing slash (and a bare
/// root path) therefore belongs to the route, not to the command.
fn split_link(link: &str) -> (&str, &str) {
    let (host, query) = match link.split_once('?') {
        Some((host, query)) => (host, query),
        None => (link, ""),
    };
    (host.trim_end_matches('/'), query)
}

/// `easydict://…` deep links, mirroring upstream's URL scheme.
pub fn handle_deep_link(app: &tauri::AppHandle, link: &str) {
    let (host, query) = split_link(link);
    // A URL's host is case-insensitive, and a hand-typed link may not match the
    // spelling the app documents (`easydict://Query?…`).
    let host = host.to_ascii_lowercase();
    // Only the route is logged: `writeKeyValue` links carry API keys.
    log::info!("deep link: {host}");

    let param = |key: &str| -> Option<String> {
        query.split('&').find_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            (k == key).then(|| percent_decode(v))
        })
    };

    match host.as_str() {
        "query" => {
            let text = param("text");
            deliver(app, "fixed", commands::ActionPayload::new("inputText", text));
            // Queueing the text alone left the link looking broken whenever the
            // query window was hidden; upstream brings the window up for a link.
            crate::window::request_show(app, "fixed");
        }
        "ocr" => {
            crate::window::request_show(app, "ocr");
        }
        // `easydict://writeKeyValue?EZYoudaoAPIKey=…` — the settings-import
        // entry point the macOS app documents.
        "writeKeyValue" => {
            let Some(state) = app.try_state::<Arc<AppState>>() else {
                return;
            };
            let patch = linked_settings_patch(query);
            if !patch.is_empty() {
                let patch = serde_json::Value::Object(patch);
                if let Err(error) = state.update_settings(|settings| {
                    crate::commands::merge_settings(settings, &patch)
                }) {
                    log::warn!("could not import linked settings: {error}");
                }
            }
        }
        _ => {}
    }
}

fn linked_settings_patch(query: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut patch = serde_json::Map::new();
    for pair in query.split('&') {
        let Some((key, value)) = pair.split_once('=') else { continue };
        let key = percent_decode(key);
        let value = percent_decode(value);
        if let Some(window_type) = intelligent_query_window(&key) {
            patch.insert(
                format!("intelligentQueryMode.{}", window_type.label()),
                serde_json::Value::Bool(upstream_switch(&value)),
            );
        } else if let Some(field) = upstream_key_to_field(&key) {
            patch.insert(field.to_string(), serde_json::Value::String(value));
        }
    }
    patch
}

/// Maps the `EZ…` defaults keys upstream's URL scheme accepts to this port's
/// settings fields.
fn upstream_key_to_field(key: &str) -> Option<&'static str> {
    Some(match key {
        "EZDeepLAuthKey" => "deepLAuth",
        "EZYoudaoAPIKey" => "youdaoWebApiKey",
        "EZBingCookieKey" => "bingCookie",
        "EZNiuTransAPIKey" => "niuTransApiKey",
        "EZCaiyunToken" => "caiyunToken",
        "EZTencentSecretId" => "tencentSecretId",
        "EZTencentSecretKey" => "tencentSecretKey",
        "EZAliAccessKeyId" => "aliAccessKeyId",
        "EZAliAccessKeySecret" => "aliAccessKeySecret",
        "EZBaiduAppId" => "baiduAppId",
        "EZBaiduSecretKey" => "baiduSecretKey",
        "EZVolcanoAccessKeyID" => "volcanoAccessKeyId",
        "EZVolcanoSecretAccessKey" => "volcanoSecretAccessKey",
        "EZDoubaoAPIKey" => "doubaoApiKey",
        _ => return None,
    })
}

/// `IntelligentQueryMode-window<raw>` — the one per-window key upstream's URL
/// scheme documents (`EZSchemeParser.m:181`).
///
/// `EZConstKey.constkey(_:windowType:)` spells the window as its numeric raw
/// value, so `window0`/`window1`/`window2` are the only accepted forms; anything
/// else is not this key.
fn intelligent_query_window(key: &str) -> Option<crate::model::WindowType> {
    let raw = key.strip_prefix("IntelligentQueryMode-window")?;
    if !matches!(raw, "0" | "1" | "2") { return None; }
    crate::model::WindowType::from_raw(raw.parse().ok()?)
}

/// Accept the documented numeric switch and common boolean spellings.
fn upstream_switch(value: &str) -> bool {
    matches!(value.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on")
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len()
                && bytes[i + 1].is_ascii_hexdigit()
                && bytes[i + 2].is_ascii_hexdigit() =>
            {
                // Both digits are hex, so parsing cannot fail — but `from_str_radix`
                // also accepts a leading `+`, which is why it is not trusted to
                // decide whether an escape is well formed.
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("00");
                match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(bytes[i]);
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shell_normalised_link_still_reaches_its_route() {
        // What Windows actually delivers for the documented links: `%1` is
        // normalised by `ShellExecute` and gains a trailing slash. The observed
        // log line was `deep link: query/`, with no query started at all.
        assert_eq!(split_link("query/?text=hello"), ("query", "text=hello"));
        assert_eq!(split_link("ocr/"), ("ocr", ""));
        assert_eq!(split_link("query?text=hello"), ("query", "text=hello"));
        assert_eq!(split_link("ocr"), ("ocr", ""));
        assert_eq!(split_link(""), ("", ""));
        // Only the route's trailing slash is normalisation; a value keeps its own.
        assert_eq!(
            split_link("query?text=https://example.com/"),
            ("query", "text=https://example.com/")
        );
        // `writeKeyValue` links must keep their parameters after normalisation.
        assert_eq!(
            split_link("writeKeyValue/?EZYoudaoAPIKey=abc"),
            ("writeKeyValue", "EZYoudaoAPIKey=abc")
        );
        // A URL host is case-insensitive; the route is matched lowercased.
        assert_eq!(
            split_link("Query/?text=hi").0.to_ascii_lowercase(),
            "query"
        );
    }

    #[test]
    fn linked_intelligent_query_switches_use_canonical_windows_and_typed_values() {
        let mut settings = crate::config::Settings::default();
        let patch = linked_settings_patch(
            "IntelligentQueryMode-window0=1&IntelligentQueryMode-window1=false&IntelligentQueryMode%2Dwindow2=TRUE"
        );
        crate::commands::merge_settings(&mut settings, &serde_json::Value::Object(patch)).unwrap();
        assert!(settings.intelligent_query_mode_for(crate::model::WindowType::Main));
        assert!(!settings.intelligent_query_mode_for(crate::model::WindowType::Mini));
        assert!(settings.intelligent_query_mode_for(crate::model::WindowType::Fixed));
        for suffix in ["", "-1", "3", "01", "+1", "mini"] {
            assert!(intelligent_query_window(&format!("IntelligentQueryMode-window{suffix}")).is_none());
        }
    }

    #[test]
    fn all_webviews_wait_for_backend_initialization() {
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let windows = config["app"]["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 5);
        for window in windows {
            assert_eq!(window["create"], false, "window {} loads before setup", window["label"]);
        }
    }

    #[test]
    fn actions_have_one_target_even_when_all_query_windows_are_hidden() {
        use crate::model::WindowType;
        assert_eq!(action_target("pasteboardTranslate", WindowType::Fixed, None), "main");
        assert_eq!(action_target("selectTranslate", WindowType::Mini, Some("main")), "mini");
        assert_eq!(action_target("snipTranslate", WindowType::Fixed, Some("main")), "main");
        assert_eq!(action_target("snipTranslate", WindowType::Fixed, Some("mini")), "mini");
        for action in ["screenshotOCR", "silentScreenshotOCR", "pasteboardOCR"] {
            assert_eq!(action_target(action, WindowType::Fixed, Some("main")), "ocr");
        }
        for action in ["favorites", "history", "about"] {
            assert_eq!(action_target(action, WindowType::Fixed, Some("main")), "settings");
        }
    }

    #[test]
    fn percent_decoding_handles_multibyte_and_plus() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("a+b"), "a b");
        // Multi-byte sequences must round-trip byte by byte.
        assert_eq!(percent_decode("%E4%BD%A0%E5%A5%BD"), "你好");
        // A truncated escape is left alone rather than dropped.
        assert_eq!(percent_decode("50%"), "50%");
    }

    #[test]
    fn percent_decoding_leaves_malformed_escapes_literal() {
        // `u8::from_str_radix` accepts a leading `+`, so `%+1` used to decode to
        // the control byte 0x01. The `%` has to stay literal; the `+` still
        // means a space, because this is form-style decoding.
        assert_eq!(percent_decode("a%+1b"), "a% 1b");
        assert_eq!(percent_decode("%+F"), "% F");
        assert!(!percent_decode("a%+1b").contains('\u{1}'));
        // Non-hex and truncated escapes are literal as well.
        assert_eq!(percent_decode("%zz"), "%zz");
        assert_eq!(percent_decode("%4"), "%4");
        assert_eq!(percent_decode("%"), "%");
        // A well-formed escape still decodes, including at the very end.
        assert_eq!(percent_decode("100%25"), "100%");
        assert_eq!(percent_decode("a%2Fb"), "a/b");
    }

    #[test]
    fn upstream_keys_map_to_settings_fields() {
        assert_eq!(
            upstream_key_to_field("EZTencentSecretId"),
            Some("tencentSecretId")
        );
        // Youdao's key lives in the flat `youdaoWebApiKey`. This used to name
        // `services.Youdao.apiKey`, a slot `youdao.rs` never reads, so the
        // `easydict://writeKeyValue?key=EZYoudaoAPIKey` link wrote into nothing.
        assert_eq!(
            upstream_key_to_field("EZYoudaoAPIKey"),
            Some("youdaoWebApiKey")
        );
        assert_eq!(upstream_key_to_field("EZNotAKey"), None);

        // Every target must be a real top-level settings field. A typo, or a
        // name left behind by a rename, makes the deep link write into nothing
        // again — and silently, since nothing checks the path at runtime.
        let document = serde_json::to_value(crate::config::Settings::default()).unwrap();
        let fields = document.as_object().unwrap();
        for upstream in [
            "EZDeepLAuthKey",
            "EZYoudaoAPIKey",
            "EZBingCookieKey",
            "EZNiuTransAPIKey",
            "EZCaiyunToken",
            "EZTencentSecretId",
            "EZTencentSecretKey",
            "EZAliAccessKeyId",
            "EZAliAccessKeySecret",
            "EZBaiduAppId",
            "EZBaiduSecretKey",
            "EZVolcanoAccessKeyID",
            "EZVolcanoSecretAccessKey",
            "EZDoubaoAPIKey",
        ] {
            let target = upstream_key_to_field(upstream).expect(upstream);
            assert!(
                fields.contains_key(target),
                "{upstream} maps to {target:?}, which is not a settings field"
            );
        }
    }
}
