#![cfg(windows)]

use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, PhysicalPosition, RunEvent, WebviewUrl};

const PROFILE_ENV: &str = "EASYDICT_NATIVE_TEST_PROFILE";

#[test]
fn replacement_command_rejects_missing_bindings_without_accessing_the_desktop() {
    in_isolated_app(
        "replacement_command_rejects_missing_bindings_without_accessing_the_desktop",
        std::time::Duration::from_secs(15),
        |app| {
            use app_lib::{commands::replace_selected_text, state::AppState, store::Store};
            let data = std::path::PathBuf::from(std::env::var_os(PROFILE_ENV).unwrap()).join("store");
            let store = Arc::new(Store::open(data).map_err(|e| e.to_string())?);
            app.manage(AppState::new(app.clone(), store.clone()).map_err(|e| e.to_string())?);
            for (text, target, original) in [
                ("new", None, None),
                ("new", Some([1, 2]), None),
                ("new\0text", Some([1, 2]), Some("old".to_string())),
                ("new", Some([0, 0]), Some("old".to_string())),
            ] {
                let error = tauri::async_runtime::block_on(
                    replace_selected_text(app.state(), text.into(), target, original),
                ).unwrap_err();
                assert!(error.contains("原选区信息"), "{error}");
            }
            assert!(app.webview_windows().is_empty());
            assert!(store.history(10, 0).unwrap().is_empty());
            Ok(())
        },
    );
}

#[test]
fn polishing_action_uses_its_disabled_service_and_preserves_errors() {
    in_isolated_app(
        "polishing_action_uses_its_disabled_service_and_preserves_errors",
        std::time::Duration::from_secs(30),
        |app| {
            use app_lib::{
                commands::{polish_text, QueryRequest},
                config::{ServiceConfig, Settings},
                model::Language,
                service::llm::prompts::POLISHING_SYSTEM_PROMPT,
                state::AppState,
                store::Store,
            };
            use std::{io::{Read, Write}, net::TcpListener, time::Duration};
            use serde_json::json;

            let data = std::path::PathBuf::from(std::env::var_os(PROFILE_ENV).unwrap()).join("store");
            let store = Arc::new(Store::open(data).map_err(|e| e.to_string())?);
            let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
            let endpoint = format!("http://{}/v1/chat/completions", listener.local_addr().unwrap());
            let mut settings = Settings::default();
            settings.shortcuts.clear();
            settings.auto_show_query_icon = false;
            for config in settings.services.values_mut() { config.enabled = false; }
            settings.services.insert("CustomOpenAI#not-polishing".into(), ServiceConfig {
                enabled: true, api_key: "test-key".into(), endpoint: endpoint.clone(),
                ..Default::default()
            });
            settings.services.insert("Polishing".into(), ServiceConfig {
                enabled: false, endpoint, model: "local-polish".into(),
                valid_models: vec!["local-polish".into()],
                ..Default::default()
            });
            let request = |text: &str| serde_json::from_value::<QueryRequest>(json!({
                "text": text, "from": "French", "to": "Simplified-Chinese", "windowType": "fixed",
            })).unwrap();
            store.save_settings(&settings).map_err(|e| e.to_string())?;
            app.manage(AppState::new(app.clone(), store.clone()).map_err(|e| e.to_string())?);
            let error = tauri::async_runtime::block_on(polish_text(app.state(), request("She don't like the weather.")))
                .unwrap_err();
            assert!(error.contains("Polishing") && error.contains("key"), "{error}");
            assert!(tauri::async_runtime::block_on(polish_text(app.state(), request(" "))).is_err());
            // Even with a normal translator enabled, missing polishing keys must
            // not start any HTTP request or silently translate in another language.
            listener.set_nonblocking(true).unwrap();
            assert_eq!(listener.accept().unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
            listener.set_nonblocking(false).unwrap();
            assert!(store.history(10, 0).unwrap().is_empty());

            for (streaming, status, output) in [
                (false, 200, "She does not like the weather."),
                (true, 200, "She does not like the weather."),
                (false, 200, " "),
                (false, 401, "Unauthorized"),
            ] {
                let listener = listener.try_clone().unwrap();
                let server = std::thread::spawn(move || {
                    let (mut socket, _) = listener.accept().unwrap();
                    socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
                    let mut raw = Vec::new();
                    let mut buffer = [0; 4096];
                    let body_start = loop {
                        let read = socket.read(&mut buffer).unwrap();
                        assert_ne!(read, 0, "incomplete HTTP request");
                        raw.extend_from_slice(&buffer[..read]);
                        if let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&raw[..end]);
                            let length: usize = head.lines().find_map(|line| {
                                let (key, value) = line.split_once(':')?;
                                key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse().unwrap())
                            }).unwrap();
                            if raw.len() >= end + 4 + length { break end + 4; }
                        }
                    };
                    let input: serde_json::Value = serde_json::from_slice(&raw[body_start..]).unwrap();
                    assert_eq!(input["model"], "local-polish");
                    assert_eq!(input["stream"], streaming);
                    assert_eq!(input["messages"][0]["content"], POLISHING_SYSTEM_PROMPT);
                    let prompt = input["messages"].as_array().unwrap().last().unwrap()["content"].as_str().unwrap();
                    assert!(prompt.starts_with("Polish the following English text"), "{prompt}");
                    assert!(prompt.ends_with("\"\"\"She don't like the weather.\"\"\""), "{prompt}");
                    let json = json!({"choices": [{"message": {"content": output}}]}).to_string();
                    let chunk = json!({"id":"1", "object":"chat.completion.chunk", "created":0,
                        "model":"local-polish", "choices":[{"index":0,"delta":{"content":output},"finish_reason":null}]}).to_string();
                    let body = if streaming { format!("data: {chunk}\n\ndata: [DONE]\n\n") } else { json };
                    let content_type = if streaming { "text/event-stream" } else { "application/json" };
                    write!(socket, "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
                });
                let config = settings.services.get_mut("Polishing").unwrap();
                config.api_key = "test-key".into();
                config.enable_streaming = streaming;
                app.state::<Arc<AppState>>().update_settings(|saved| {
                    *saved = settings.clone();
                    Ok(())
                })?;
                let result = tauri::async_runtime::block_on(polish_text(app.state(), request("She don't like the weather.")));
                server.join().expect("local polishing request");
                if status == 401 {
                    let error = result.unwrap_err();
                    assert!(error.contains("401") && error.contains("Polishing"), "{error}");
                } else if output.trim().is_empty() {
                    let error = result.unwrap_err();
                    assert!(error.contains("No result") || error.contains("未替换原文"), "{error}");
                } else {
                    let result = result?;
                    assert_eq!(result.service_type_with_unique_identifier, "Polishing");
                    assert_eq!(result.from, Language::English);
                    assert_eq!(result.to, Language::English);
                    assert_eq!(result.translated_results, Some(vec![output.into()]));
                }
            }
            eprintln!("polishing: disabled service, source detection, JSON/SSE, missing key, empty result and HTTP error passed");
            Ok(())
        },
    );
}

#[test]
fn tray_check_tracks_saved_settings_and_rolls_back_failed_clicks() {
    in_isolated_app(
        "tray_check_tracks_saved_settings_and_rolls_back_failed_clicks",
        std::time::Duration::from_secs(20),
        |app| {
            use app_lib::{state::AppState, store::Store};
            use std::{os::windows::fs::OpenOptionsExt, time::{Duration, Instant}};
            use tauri::menu::{ContextMenu, Menu};
            use windows::{core::BOOL, Win32::{
                Foundation::{HWND, LPARAM, WPARAM},
                UI::WindowsAndMessaging::*,
            }};

            unsafe extern "system" fn find_tray(hwnd: HWND, data: LPARAM) -> BOOL {
                let mut pid = 0;
                unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)); }
                if pid == std::process::id() {
                    let mut class = [0; 64];
                    let len = unsafe { GetClassNameW(hwnd, &mut class) } as usize;
                    if String::from_utf16_lossy(&class[..len]) == "tray_icon_app" {
                        unsafe { *(data.0 as *mut HWND) = hwnd; }
                        return BOOL(0);
                    }
                }
                BOOL(1)
            }

            let data = std::path::PathBuf::from(std::env::var_os(PROFILE_ENV).unwrap()).join("store");
            let store = Arc::new(Store::open(data).map_err(|e| e.to_string())?);
            let state = AppState::new(app.clone(), store.clone()).map_err(|e| e.to_string())?;
            store.save_settings(&state.settings()).map_err(|e| e.to_string())?;
            app.manage(state.clone());
            app_lib::tray::install(&app, &state).map_err(|e| e.to_string())?;
            app.tray_by_id(app_lib::tray::TRAY_ID).unwrap()
                .set_visible(false).map_err(|e| e.to_string())?;
            let menu = app.state::<Menu<tauri::Wry>>().inner().clone();
            let popup = menu.hpopupmenu().map_err(|e| e.to_string())?;
            let check = menu.get("autoSelect").unwrap().as_check_menuitem().unwrap().clone();
            let wait = |done: &dyn Fn() -> bool| -> Result<(), String> {
                let deadline = Instant::now() + Duration::from_secs(3);
                while !done() {
                    if Instant::now() >= deadline { return Err("tray update timed out".into()); }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(())
            };
            let click = || -> Result<(), String> {
                let handle = app.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                app.run_on_main_thread(move || {
                    let menu = handle.state::<Menu<tauri::Wry>>();
                    let position = menu.items().unwrap().iter()
                        .position(|item| item.id().as_ref() == "autoSelect").unwrap();
                    let native_menu = HMENU(menu.hpopupmenu().unwrap() as *mut _);
                    let id = unsafe { GetMenuItemID(native_menu, position as i32) };
                    let mut hwnd = HWND::default();
                    unsafe { let _ = EnumWindows(Some(find_tray), LPARAM(&mut hwnd as *mut _ as isize)); }
                    assert!(!hwnd.is_invalid(), "isolated tray window not found");
                    // Exercise muda's native toggle and Tauri's real menu event,
                    // without clicking Explorer or touching the user's app.
                    unsafe { SendMessageW(hwnd, WM_COMMAND, Some(WPARAM(id as usize)), None); }
                    let _ = tx.send(());
                }).map_err(|e| e.to_string())?;
                rx.recv_timeout(Duration::from_secs(3)).map_err(|e| e.to_string())
            };
            let assert_checked = |expected: bool| -> Result<(), String> {
                // Publishing settings precedes the queued UI refresh.
                wait(&|| state.settings().auto_show_query_icon == expected
                    && check.is_checked().ok() == Some(expected))?;
                assert_eq!(check.is_checked().unwrap(), expected, "native check differs from saved setting");
                assert_eq!(state.settings().auto_show_query_icon, expected);
                assert_eq!(store.load_settings().unwrap().auto_show_query_icon, expected);
                assert_eq!(menu.hpopupmenu().unwrap(), popup, "tray menu was unnecessarily replaced");
                let handle = app.clone();
                let (tx, rx) = std::sync::mpsc::channel();
                app.run_on_main_thread(move || {
                    let menu = handle.state::<Menu<tauri::Wry>>();
                    let position = menu.items().unwrap().iter()
                        .position(|item| item.id().as_ref() == "autoSelect").unwrap();
                    let native_menu = HMENU(menu.hpopupmenu().unwrap() as *mut _);
                    let flags = unsafe { GetMenuState(native_menu, position as u32, MF_BYPOSITION) };
                    let _ = tx.send(flags);
                }).map_err(|e| e.to_string())?;
                let flags = rx.recv_timeout(Duration::from_secs(3)).map_err(|e| e.to_string())?;
                assert_ne!(flags, u32::MAX, "Windows could not read the actual popup item");
                assert_eq!(flags & MF_CHECKED.0 != 0, expected, "Windows popup check differs from cached state");
                Ok(())
            };

            assert_checked(true)?;
            // Deny the atomic file replacement, then exercise the real callback.
            let locked = std::fs::OpenOptions::new().read(true).share_mode(0)
                .open(store.settings_path()).map_err(|e| e.to_string())?;
            click()?;
            wait(&|| app.state::<app_lib::window::Windows>().next("main")
                .is_some_and(|payload| payload.action == "selectionError"))?;
            drop(locked);
            assert_checked(true)?;

            for expected in [false, true, false, true] {
                click()?;
                assert_checked(expected)?;
            }
            // Settings-page edits, including structural OCR changes, reuse the
            // same check item and popup rather than leaving stale native handles.
            for visible in [true, false, true, false] {
                state.update_settings(|settings| {
                    settings.show_ocr_menu_items = visible;
                    settings.auto_show_query_icon = !visible;
                    Ok(())
                })?;
                assert_checked(!visible)?;
                let items = menu.items().map_err(|e| e.to_string())?;
                assert_eq!(items.iter().filter(|item| item.id().as_ref() == "showOCRWindow").count(),
                    usize::from(visible));
                assert!(menu.get("inputTranslate").unwrap().as_check_menuitem().is_none());
                click()?;
                assert_checked(visible)?;
            }
            app.remove_tray_by_id(app_lib::tray::TRAY_ID);
            eprintln!("tray: native clicks, saved checks, failed-save rollback and OCR edits passed");
            Ok(())
        },
    );
}

#[test]
fn intelligent_query_dispatch_filters_only_the_opted_in_window() {
    in_isolated_app(
        "intelligent_query_dispatch_filters_only_the_opted_in_window",
        std::time::Duration::from_secs(30),
        |app| {
            use app_lib::{
                config::{ServiceConfig, Settings},
                model::{Language, QueryModel, QueryTextType, WindowType},
                query::QueryEvent,
                state::AppState,
                store::Store,
            };
            let data = std::path::PathBuf::from(std::env::var_os(PROFILE_ENV).unwrap()).join("store");
            let store = Arc::new(Store::open(data).map_err(|e| e.to_string())?);
            let mut settings = Settings::default();
            for config in settings.services.values_mut() { config.enabled = false; }
            settings.intelligent_query_mode.insert(WindowType::Mini, true);
            settings.services.insert("CustomOpenAI#isolated".into(), ServiceConfig {
                enabled: true,
                intelligent_query_text_type: QueryTextType::NONE,
                // No real credentials or external requests, even if filtering regresses.
                endpoint: "http://127.0.0.1:1/v1/chat/completions".into(),
                ..Default::default()
            });
            store.save_settings(&settings).map_err(|e| e.to_string())?;
            let state = AppState::new(app, store).map_err(|e| e.to_string())?;
            tauri::async_runtime::block_on(async {
                let (main_tx, mut main_rx) = tokio::sync::mpsc::unbounded_channel();
                let (mini_tx, mut mini_rx) = tokio::sync::mpsc::unbounded_channel();
                let mut model = QueryModel::new("apple");
                model.query_from_language = Language::English;
                model.query_target_language = Language::SimplifiedChinese;
                state.start_query(model.clone(), "main-request".into(), main_tx);
                model.window_type = WindowType::Mini;
                state.start_query(model, "mini-request".into(), mini_tx);
                let check = async {
                    assert!(matches!(mini_rx.recv().await, Some(QueryEvent::AllFinished { cancelled: false })));
                    assert!(mini_rx.recv().await.is_none(), "filtered services emitted an update");
                    let mut updates = 0;
                    while let Some(event) = main_rx.recv().await {
                        match event {
                            QueryEvent::Update { identifier, .. } => {
                                assert_eq!(identifier, "CustomOpenAI#isolated");
                                updates += 1;
                            }
                            QueryEvent::AllFinished { cancelled } => assert!(!cancelled),
                        }
                    }
                    assert!(updates > 0, "the mini window filtered or cancelled the main query");
                };
                tokio::time::timeout(std::time::Duration::from_secs(5), check)
                    .await.map_err(|e| e.to_string())
            })
        },
    );
}

/// Runs `check` on a worker thread of a real Tauri event loop whose windows load
/// `about:blank` from an isolated WebView profile.
///
/// The harness re-executes this test binary in a child process: WebView2 holds
/// its profile locks until the process exits, and the parent has to be able to
/// remove the profile afterwards.
fn in_isolated_app(
    test_name: &str,
    watchdog: std::time::Duration,
    check: impl FnOnce(AppHandle) -> Result<(), String> + Send + 'static,
) {
    let Some(profile) = std::env::var_os(PROFILE_ENV) else {
        let root = std::env::temp_dir();
        let data = root.join(format!("easydict-native-{}", uuid::Uuid::new_v4()));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", test_name, "--nocapture"])
            .env(PROFILE_ENV, &data)
            .output()
            .unwrap();
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        if data.parent() == Some(root.as_path()) && data.exists() {
            let resolved = data.canonicalize().unwrap();
            let root = root.canonicalize().unwrap();
            assert_eq!(resolved.parent(), Some(root.as_path()));
            let mut removed = Ok(());
            for _ in 0..50 {
                removed = std::fs::remove_dir_all(&resolved);
                if removed.is_ok() {
                    break;
                }
                // The browser child can outlive the Tauri child briefly.
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            removed.expect("remove only this test's WebView profile");
        }
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    };
    let data = std::path::PathBuf::from(profile);
    assert_eq!(data.parent(), Some(std::env::temp_dir().as_path()));
    assert!(data
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("easydict-native-"));
    let output = Arc::new(Mutex::new(None));
    let mut context = tauri::generate_context!();
    for config in &mut context.config_mut().app.windows {
        config.url = WebviewUrl::External("about:blank".parse().unwrap());
        config.data_directory = Some(data.clone());
        config.visible = false;
        config.focus = false;
        config.skip_taskbar = true;
        config.width = 420.0;
        config.height = 200.0;
        config.min_width = Some(320.0);
        config.min_height = Some(160.0);
    }
    let app = tauri::Builder::default()
        .any_thread()
        // Both plugins stand in for production wiring: errors raised on a worker
        // thread are logged to this test's stdout instead of disappearing into
        // the real application log, and `window::report_error` notifies through
        // the notification plugin rather than panicking over its missing state.
        .plugin(
            tauri_plugin_log::Builder::new()
                .clear_targets()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            app.manage(app_lib::window::Windows::default());
            Ok(())
        })
        .build(context)
        .expect("build isolated native harness");
    let result = output.clone();
    // `run_return` wants a callback it may call more than once, while the check
    // is a one-shot; hand it over exactly once, on the single `Ready` event.
    let check: Arc<Mutex<Option<Box<dyn FnOnce(AppHandle) -> Result<(), String> + Send>>>> =
        Arc::new(Mutex::new(Some(Box::new(check))));
    let code = app.run_return(move |app, event| {
        if let RunEvent::ExitRequested {
            ref api,
            code: None,
            ..
        } = event
        {
            api.prevent_exit();
        }
        if matches!(event, RunEvent::Ready) {
            std::thread::spawn(move || {
                std::thread::sleep(watchdog);
                // Posting Exit cannot stop a deadlocked event loop.
                eprintln!("native app watchdog expired");
                std::process::exit(2);
            });
            let app = app.clone();
            let result = result.clone();
            let check = check.clone();
            std::thread::spawn(move || {
                let check = check.lock().unwrap().take();
                let outcome = match check {
                    Some(check) => std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        check(app.clone())
                    }))
                    .unwrap_or_else(|_| Err("native worker panicked".into())),
                    None => Err("the harness ran its check twice".into()),
                };
                *result.lock().unwrap() = Some(outcome);
                app.exit(0);
            });
        }
    });
    assert_eq!(code, 0);
    output
        .lock()
        .unwrap()
        .take()
        .expect("native setup ran")
        .unwrap();
}

#[test]
fn geometry_flush_waits_for_the_inflight_batch_before_returning() {
    in_isolated_app(
        "geometry_flush_waits_for_the_inflight_batch_before_returning",
        std::time::Duration::from_secs(15),
        |app| {
            use app_lib::{model::WindowType, state::AppState, store::Store};
            use std::time::Duration;

            let data = std::path::PathBuf::from(std::env::var_os(PROFILE_ENV).unwrap()).join("store");
            let store = Arc::new(Store::open(data).map_err(|e| e.to_string())?);
            let state = AppState::new(app.clone(), store.clone()).map_err(|e| e.to_string())?;
            app.manage(state.clone());
            app_lib::window::ensure(&app, "main")?;
            let (locked_tx, locked_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            let editor = state.clone();
            let edit = std::thread::spawn(move || {
                editor.update_settings(|settings| {
                    locked_tx.send(()).unwrap();
                    release_rx.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?;
                    settings.show_ocr_menu_items = !settings.show_ocr_menu_items;
                    Ok(())
                })
            });
            locked_rx.recv_timeout(Duration::from_secs(3)).map_err(|e| e.to_string())?;
            app_lib::window_frame::note(&app, "main");
            // Let the 500ms saver dequeue the frame and block behind the edit.
            std::thread::sleep(Duration::from_millis(1200));
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let handle = app.clone();
            app.run_on_main_thread(move || {
                app_lib::window_frame::flush(&handle);
                let _ = done_tx.send(());
            }).map_err(|e| e.to_string())?;
            let returned_early = done_rx.recv_timeout(Duration::from_millis(300)).is_ok();
            release_tx.send(()).map_err(|e| e.to_string())?;
            edit.join().map_err(|_| "settings editor panicked")??;
            assert!(!returned_early, "flush returned while a dequeued frame was still unsaved");
            done_rx.recv_timeout(Duration::from_secs(3))
                .map_err(|_| "main-thread flush did not finish after the edit")?;
            let disk = store.load_settings().map_err(|e| e.to_string())?;
            assert!(disk.window_frames.contains_key(&WindowType::Main));
            assert_eq!(disk.window_frames, state.settings().window_frames);
            assert_eq!(disk.show_ocr_menu_items, state.settings().show_ocr_menu_items);
            eprintln!("main-thread flush waited for geometry and preserved the concurrent preference edit");
            Ok(())
        },
    );
}

#[test]
fn geometry_persistence_keeps_services_and_unchanged_settings_untouched() {
    in_isolated_app(
        "geometry_persistence_keeps_services_and_unchanged_settings_untouched",
        std::time::Duration::from_secs(20),
        |app| {
            use app_lib::{model::WindowType, state::AppState, store::Store};
            use std::sync::atomic::{AtomicUsize, Ordering};
            use tauri::Listener;

            let data = std::path::PathBuf::from(std::env::var_os(PROFILE_ENV).unwrap()).join("store");
            let store = Arc::new(Store::open(data).map_err(|e| e.to_string())?);
            let state = AppState::new(app.clone(), store.clone()).map_err(|e| e.to_string())?;
            app.manage(state.clone());
            let handles = state.all_handles();
            assert!(!handles.is_empty());
            let notifications = Arc::new(AtomicUsize::new(0));
            let observed = notifications.clone();
            app.listen_any(app_lib::commands::events::SETTINGS_CHANGED, move |_| {
                observed.fetch_add(1, Ordering::SeqCst);
            });

            let window = app_lib::window::ensure(&app, "main")?;
            app_lib::window_frame::note(&app, "main");
            app_lib::window_frame::flush(&app);
            let saved = store.load_settings().map_err(|e| e.to_string())?;
            assert!(saved.window_frames.contains_key(&WindowType::Main));
            assert_eq!(saved.window_frames, state.settings().window_frames);
            let after = state.all_handles();
            let rebuilt = handles.iter().zip(&after).filter(|(a, b)| !Arc::ptr_eq(a, b)).count();
            assert_eq!((rebuilt, notifications.load(Ordering::SeqCst)), (0, 0),
                "geometry rebuilt services or broadcast preferences");

            // Compare encrypted bytes: a redundant save changes the DPAPI blob.
            let bytes = std::fs::read(store.settings_path()).map_err(|e| e.to_string())?;
            for _ in 0..3 {
                app_lib::window_frame::note(&app, "main");
                app_lib::window_frame::flush(&app);
            }
            assert_eq!(bytes, std::fs::read(store.settings_path()).unwrap(),
                "an unchanged frame rewrote settings");

            // A failed atomic replacement must not publish an unsaved frame.
            let before = state.settings().window_frames;
            let path = store.settings_path();
            let backup = path.with_extension("backup");
            std::fs::rename(&path, &backup).map_err(|e| e.to_string())?;
            std::fs::create_dir(&path).map_err(|e| e.to_string())?;
            let origin = window.outer_position().map_err(|e| e.to_string())?;
            let position = PhysicalPosition::new(origin.x + 30, origin.y + 20);
            window.set_position(position).map_err(|e| e.to_string())?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            while window.outer_position().map_err(|e| e.to_string())? != position {
                if std::time::Instant::now() >= deadline { return Err("move timed out".into()); }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            app_lib::window_frame::note(&app, "main");
            app_lib::window_frame::flush(&app);
            assert_eq!(state.settings().window_frames, before, "failed save published new geometry");
            std::fs::remove_dir(&path).map_err(|e| e.to_string())?;
            std::fs::rename(&backup, &path).map_err(|e| e.to_string())?;
            app_lib::window_frame::note(&app, "main");
            app_lib::window_frame::flush(&app);
            let final_settings = store.load_settings().map_err(|e| e.to_string())?;
            assert_eq!(final_settings.window_frames[&WindowType::Main].x, position.x);
            assert_eq!(state.settings().window_frames, final_settings.window_frames);
            assert_eq!(notifications.load(Ordering::SeqCst), 0);
            eprintln!("geometry: 0 rebuilt services, 0 settings events, unchanged bytes, failed save rolled back");
            Ok(())
        },
    );
}

#[test]
fn geometry_saver_restarts_after_idle_without_blocking_window_events() {
    in_isolated_app(
        "geometry_saver_restarts_after_idle_without_blocking_window_events",
        std::time::Duration::from_secs(20),
        |app| {
            use app_lib::{model::WindowType, state::AppState, store::Store};
            use std::time::{Duration, Instant};

            let data = std::path::PathBuf::from(std::env::var_os(PROFILE_ENV).unwrap()).join("store");
            let store = Arc::new(Store::open(data).map_err(|e| e.to_string())?);
            let state = AppState::new(app.clone(), store.clone()).map_err(|e| e.to_string())?;
            app.manage(state.clone());
            let window = app_lib::window::ensure(&app, "main")?;
            let handle = app.clone();
            // Match production's geometry event handler, not just set_size().
            window.on_window_event(move |event| {
                if matches!(event, tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_)) {
                    app_lib::window_frame::note(&handle, "main");
                }
            });

            let origin = window.outer_position().map_err(|e| e.to_string())?;
            for step in 1..=3 {
                let position = PhysicalPosition::new(origin.x + step * 10, origin.y + step * 10);
                window.set_position(position).map_err(|e| e.to_string())?;
                window.set_size(tauri::PhysicalSize::new(440 + step as u32 * 10, 260))
                    .map_err(|e| e.to_string())?;
                let deadline = Instant::now() + Duration::from_secs(4);
                loop {
                    let saved = store.load_settings().map_err(|e| e.to_string())?;
                    if saved.window_frames.get(&WindowType::Main).is_some_and(|frame| {
                        frame.x == position.x && frame.y == position.y
                            && frame.width == 440 + step as u32 * 10 && frame.height == 260
                    }) {
                        break;
                    }
                    if Instant::now() >= deadline {
                        return Err(format!("geometry batch {step} was not persisted after idle"));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                // Let the worker hit its empty-queue exit before the next move.
                std::thread::sleep(Duration::from_millis(1200));
                let (tx, rx) = std::sync::mpsc::channel();
                let handle = app.clone();
                app.run_on_main_thread(move || {
                    app_lib::window_frame::note(&handle, "main");
                    let _ = tx.send(());
                }).map_err(|e| e.to_string())?;
                rx.recv_timeout(Duration::from_secs(2))
                    .map_err(|_| format!("main-thread geometry recording blocked after batch {step}"))?;
                eprintln!("geometry batch {step}: persisted, idle, main-thread recording responsive");
            }
            app_lib::window_frame::flush(&app);
            assert_eq!(state.settings().window_frames, store.load_settings().unwrap().window_frames);
            Ok(())
        },
    );
}

#[test]
fn a_remembered_frame_is_restored_and_a_zero_sized_one_is_ignored() {
    in_isolated_app(
        "a_remembered_frame_is_restored_and_a_zero_sized_one_is_ignored",
        std::time::Duration::from_secs(30),
        |app| {
            let window = app_lib::window::ensure(&app, "main")?;
            let read = |window: &tauri::WebviewWindow| -> Result<
                (PhysicalPosition<i32>, tauri::PhysicalSize<u32>),
                String,
            > {
                Ok((
                    window.outer_position().map_err(|e| e.to_string())?,
                    window.outer_size().map_err(|e| e.to_string())?,
                ))
            };
            // `set_position` and `set_size` are posted to the event loop, so the
            // window only reports the requested frame a moment later.
            let settle = |expected: (PhysicalPosition<i32>, i32, i32)| -> Result<(), String> {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
                loop {
                    let (position, size) = read(&window)?;
                    if position.x == expected.0.x
                        && position.y == expected.0.y
                        && (size.width as i32 - expected.1).abs() <= 4
                        && (size.height as i32 - expected.2).abs() <= 4
                    {
                        return Ok(());
                    }
                    if std::time::Instant::now() > deadline {
                        return Err(format!(
                            "window stayed at {position:?} {size:?}, expected {:?} {}x{}",
                            expected.0, expected.1, expected.2
                        ));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            };

            let (origin, size) = read(&window)?;
            let client = window.inner_size().map_err(|e| e.to_string())?;
            if origin.x == 0 && origin.y == 0 {
                return Err("the harness window never got a real origin".into());
            }
            // Move it somewhere else first, so the restore has work to do.
            window
                .set_position(PhysicalPosition::new(origin.x + 60, origin.y + 40))
                .map_err(|e| e.to_string())?;
            settle((PhysicalPosition::new(origin.x + 60, origin.y + 40), size.width as i32, size.height as i32))?;

            // The remembered frame is the client rectangle under the origin the
            // window had, which is exactly what the geometry tracker records.
            // Restoring it must put the window back without changing its size:
            // the client size is what `set_size` takes, so a frame stored as the
            // outer rectangle would grow the window by the title bar and borders
            // on every restore.
            let frame = app_lib::config::WindowFrame {
                x: origin.x,
                y: origin.y,
                width: client.width,
                height: client.height,
            };
            app_lib::window::restore(&window, frame)?;
            settle((origin, size.width as i32, size.height as i32))?;
            eprintln!("remembered frame restored: {origin:?} {size:?} (client {client:?})");

            let (restored, restored_size) = read(&window)?;
            let restored_client = window.inner_size().map_err(|e| e.to_string())?;
            if restored != origin || restored_size != size || restored_client != client {
                return Err(format!(
                    "restored {restored:?} {restored_size:?} client {restored_client:?} \
                     instead of {origin:?} {size:?} client {client:?}"
                ));
            }

            // A zero-sized entry is "nothing remembered": it must not move or
            // resize the window. The frame is applied through `restore`, which
            // is what `show` calls for the "former" placement.
            let (before, before_size) = read(&window)?;
            app_lib::window::restore(
                &window,
                app_lib::config::WindowFrame { x: before.x + 300, y: before.y + 300, width: 0, height: 0 },
            )?;
            std::thread::sleep(std::time::Duration::from_millis(300));
            let (after, after_size) = read(&window)?;
            if after != before || after_size != before_size {
                return Err(format!(
                    "a zero-sized frame moved the window from {before:?} {before_size:?} to {after:?} {after_size:?}"
                ));
            }
            Ok(())
        },
    );
}

#[test]
fn hidden_native_window_uses_work_area_and_reports_its_applied_client_size() {
    in_isolated_app(
        "hidden_native_window_uses_work_area_and_reports_its_applied_client_size",
        std::time::Duration::from_secs(30),
        |app| {
            if !app.webview_windows().is_empty() {
                return Err("tray-only startup created a WebView".into());
            }
            if app_lib::window::ensure(&app, "unknown").is_ok() {
                return Err("created an unknown window".into());
            }
            app_lib::window::enqueue(
                &app,
                "main",
                app_lib::commands::ActionPayload::new("inputText", Some("first-use".into())),
            )?;
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let workers: Vec<_> = (0..2)
                .map(|_| {
                    let app = app.clone();
                    let barrier = barrier.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        app_lib::window::ensure(&app, "main")
                    })
                })
                .collect();
            for worker in workers {
                worker.join().map_err(|_| "creation worker panicked")??;
            }
            if app.webview_windows().len() != 1 {
                return Err("concurrent first use must create exactly one WebView".into());
            }
            let window = app_lib::window::ensure(&app, "main")?;
            let first = app_lib::commands::next_action(window.clone(), app.clone())
                .ok_or("first-use action was lost before the page subscribed")?;
            if first.text.as_deref() != Some("first-use")
                || app_lib::commands::next_action(window.clone(), app.clone()).is_some()
            {
                return Err("first-use action was corrupted or duplicated".into());
            }
            eprintln!(
                "lazy native creation: 0 startup WebViews -> 1 after concurrent first use"
            );
            let check = || -> Result<(), String> {
                let monitor = window
                    .current_monitor()
                    .map_err(|e| e.to_string())?
                    .ok_or("no monitor")?;
                let work = *monitor.work_area();
                window
                    .set_position(PhysicalPosition::new(
                        work.position.x + work.size.width as i32 - 5,
                        work.position.y + work.size.height as i32 - 5,
                    ))
                    .map_err(|e| e.to_string())?;
                app_lib::window::position(&window, Some("former"), false)?;
                let monitor = window
                    .current_monitor()
                    .map_err(|e| e.to_string())?
                    .ok_or("no monitor after recovery")?;
                let work = *monitor.work_area();
                eprintln!(
                    "geometry work area: {work:?}, scale: {}",
                    monitor.scale_factor()
                );
                for content in [300.0, 100_000.0, 120.0] {
                    let applied = app_lib::window::fit_to_content(&window, content)?;
                    let scale = window.scale_factor().map_err(|e| e.to_string())?;
                    let actual = window
                        .inner_size()
                        .map_err(|e| e.to_string())?
                        .to_logical::<f64>(scale);
                    if (applied.width - actual.width).abs() > 1.0
                        || (applied.height - actual.height).abs() > 1.0
                    {
                        return Err(format!("reported {applied:?}, actual {actual:?}"));
                    }
                    if content == 300.0 && (actual.height - 300.0).abs() > 1.0 {
                        return Err(format!(
                            "native frame was added to client height: {actual:?}"
                        ));
                    }
                    let position = window.outer_position().map_err(|e| e.to_string())?;
                    let size = window.outer_size().map_err(|e| e.to_string())?;
                    if position.x < work.position.x
                        || position.y < work.position.y
                        || position.x as i64 + size.width as i64
                            > work.position.x as i64 + work.size.width as i64
                        || position.y as i64 + size.height as i64
                            > work.position.y as i64 + work.size.height as i64
                    {
                        return Err(format!(
                            "outside work area: {position:?} {size:?}, {work:?}"
                        ));
                    }
                    if window.is_visible().map_err(|e| e.to_string())?
                        || window.is_focused().map_err(|e| e.to_string())?
                    {
                        return Err("geometry test must remain hidden and unfocused".into());
                    }
                }
                for placement in ["right", "center"] {
                    app_lib::window::position(&window, Some(placement), false)?;
                    let monitor = window
                        .current_monitor()
                        .map_err(|e| e.to_string())?
                        .ok_or("no monitor after placement")?;
                    let work = *monitor.work_area();
                    let position = window.outer_position().map_err(|e| e.to_string())?;
                    let size = window.outer_size().map_err(|e| e.to_string())?;
                    let dx = work.size.width.saturating_sub(size.width) as i32;
                    let dy = work.size.height.saturating_sub(size.height) as i32;
                    let expected = if placement == "right" {
                        PhysicalPosition::new(work.position.x + dx, work.position.y)
                    } else {
                        PhysicalPosition::new(
                            work.position.x + dx / 2,
                            work.position.y + dy / 2,
                        )
                    };
                    if (position.x as i64 - expected.x as i64).abs() > 1
                        || (position.y as i64 - expected.y as i64).abs() > 1
                    {
                        return Err(format!(
                            "{placement}: {position:?}, expected {expected:?}"
                        ));
                    }
                }
                if window.is_visible().map_err(|e| e.to_string())?
                    || window.is_focused().map_err(|e| e.to_string())?
                {
                    return Err("placement activated the hidden test window".into());
                }
                Ok(())
            };
            let result = check();
            window.destroy().map_err(|e| e.to_string())?;
            result
        },
    );
}

/// Sets a window's WebView2 controller visibility from the test, which is what
/// showing the window does in the real application. The harness never shows a
/// window, so a test that skipped this would only ever suspend a WebView the
/// runtime already considered invisible — the case that cannot fail.
fn set_controller_visible(window: &tauri::WebviewWindow, visible: bool) -> Result<(), String> {
    use std::sync::mpsc;
    use std::time::Duration;

    let (reported, answer) = mpsc::channel();
    let label = window.label().to_owned();
    window
        .with_webview(move |webview| {
            let controller = webview.controller();
            let result = unsafe { controller.SetIsVisible(visible) };
            let _ = reported.send(result.map_err(|error| error.to_string()));
        })
        .map_err(|error| error.to_string())?;
    answer
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| format!("the runtime never answered the visibility change of {label}"))?
        .map_err(|error| format!("could not change the visibility of {label}: {error}"))
}

/// Reads back what the runtime thinks of a window's controller visibility.
fn controller_is_visible(window: &tauri::WebviewWindow) -> Result<bool, String> {
    use std::sync::mpsc;
    use std::time::Duration;

    let (reported, answer) = mpsc::channel();
    let label = window.label().to_owned();
    window
        .with_webview(move |webview| {
            let controller = webview.controller();
            // `IsVisible` reports through an out-parameter, like the rest of the
            // WebView2 property getters.
            let mut visible = windows::core::BOOL::default();
            let read = unsafe { controller.IsVisible(&mut visible) }
                .map(|()| visible.as_bool())
                .map_err(|error| error.to_string());
            let _ = reported.send(read);
        })
        .map_err(|error| error.to_string())?;
    answer
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| format!("the runtime never reported the visibility of {label}"))?
}

/// A registered `easydict://…` link starts this executable, so the link arrives
/// in its own command line rather than through a second instance. This is that
/// path end to end: a real command line goes in, and the query window has to
/// come up carrying the text. The window is created for real, so it is briefly
/// on screen before this test destroys it.
#[test]
fn a_deep_link_in_the_command_line_opens_the_query_window_with_its_text() {
    /// `request_show` runs on a worker thread, so a link returns before its
    /// window exists; the window has to be waited for, not sampled.
    fn wait_for_visible(
        app: &AppHandle,
        label: &str,
        what: &str,
    ) -> Result<tauri::WebviewWindow, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
        loop {
            if let Some(window) = app.get_webview_window(label) {
                if window.is_visible().map_err(|error| error.to_string())? {
                    return Ok(window);
                }
            }
            if std::time::Instant::now() > deadline {
                return Err(format!("the {what} stayed hidden after its deep link"));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }

    in_isolated_app(
        "a_deep_link_in_the_command_line_opens_the_query_window_with_its_text",
        std::time::Duration::from_secs(60),
        |app| {
            // Windows passes the executable first and may append switches; only
            // arguments that really are links may be routed.
            //
            // The links are spelled the way `ShellExecute` actually delivers
            // them: the registered `%1` is normalised, so a URL with a host and
            // no path arrives with a trailing slash. Handing the tidy
            // `easydict://query?…` form here is what let a port that matched the
            // raw text pass while every real link from Explorer was dropped.
            let argv = vec![
                r"C:\Program Files\Easydict\app.exe".to_string(),
                "--flag-that-is-not-a-link".to_string(),
                "easydict://query/?text=cold%20start%20link".to_string(),
            ];
            if app_lib::dispatch_deep_links(&app, &argv) != 1 {
                return Err("a command line link was not recognised".into());
            }
            if app_lib::dispatch_deep_links(&app, &argv[..1]) != 0 {
                return Err("the executable path was mistaken for a link".into());
            }

            let window = wait_for_visible(&app, "fixed", "query window")?;
            let payload = app_lib::commands::next_action(window.clone(), app.clone())
                .ok_or("the deep link's text was lost before the page subscribed")?;
            if payload.action != "inputText" || payload.text.as_deref() != Some("cold start link") {
                return Err(format!("the deep link queued {payload:?} instead of its text"));
            }
            eprintln!("cold start deep link opened the query window with its text");
            window.destroy().map_err(|error| error.to_string())?;

            if app_lib::dispatch_deep_links(&app, &["app.exe".into(), "easydict://ocr/".into()]) != 1 {
                return Err("the ocr link was not recognised".into());
            }
            let ocr = wait_for_visible(&app, "ocr", "OCR window")?;
            eprintln!("the ocr deep link opened the OCR window");
            ocr.destroy().map_err(|error| error.to_string())?;
            Ok(())
        },
    );
}

/// The hidden windows this port keeps alive must not keep a renderer awake. This
/// checks the whole agreement against the installed runtime: a window that has
/// been on screen really suspends when it is hidden, and it comes back visible
/// and running when it is needed again.
#[test]
fn hiding_a_window_suspends_its_webview_and_showing_it_resumes() {
    use app_lib::platform::webview_memory::{MemoryState, set_hidden};
    use std::sync::mpsc;
    use std::time::Duration;

    in_isolated_app(
        "hiding_a_window_suspends_its_webview_and_showing_it_resumes",
        Duration::from_secs(45),
        |app| {
            let window = app_lib::window::ensure(&app, "main")?;
            // A window becomes visible before the user can close it, and only a
            // visible WebView may not be suspended.
            set_controller_visible(&window, true)?;
            if !controller_is_visible(&window)? {
                return Err("the test could not make the WebView visible".into());
            }
            let (woke, wakes) = mpsc::channel();
            let listener = tauri::Listener::listen(&window, app_lib::commands::events::ACTION, move |_| {
                let _ = woke.send(());
            });
            for _ in 0..2 {
                app_lib::window::enqueue(&app, "main",
                    app_lib::commands::ActionPayload::new("polishAndReplace", None))?;
            }
            for _ in 0..2 { wakes.recv_timeout(Duration::from_secs(5)).map_err(|e| e.to_string())?; }
            tauri::Listener::unlisten(&window, listener);
            let windows = app.state::<app_lib::window::Windows>();
            windows.next("main").unwrap();
            assert_eq!(set_hidden(&window, true, |_| {})?, MemoryState::Active);
            assert!(controller_is_visible(&window)?);
            assert!(!windows.complete("main"), "queued action must keep the window awake");
            windows.next("main").unwrap();
            assert_eq!(set_hidden(&window, true, |_| {})?, MemoryState::Active);
            assert!(controller_is_visible(&window)?);
            assert!(windows.complete("main"));
            eprintln!("pending and in-flight hidden actions refuse suspension until completion");
            let (reported, answer) = mpsc::channel();
            let state = set_hidden(&window, true, move |accepted| {
                let _ = reported.send(accepted);
            })
            .map_err(|error| format!("suspending a hidden window failed: {error}"))?;
            if state == MemoryState::Unsupported {
                return Err(format!(
                    "WebView2 {} has no ICoreWebView2_3, so suspension cannot be verified",
                    tauri::webview_version().unwrap_or_else(|_| "unknown".into())
                ));
            }
            if state != MemoryState::Suspended {
                return Err(format!("hiding the window asked for {state:?}"));
            }
            if !answer
                .recv_timeout(Duration::from_secs(30))
                .map_err(|_| "the runtime never answered the suspension request".to_string())?
            {
                return Err("the runtime refused to suspend the hidden WebView".into());
            }
            if controller_is_visible(&window)? {
                return Err("a suspended WebView is still visible, so it will keep painting".into());
            }
            eprintln!("hidden window suspended its WebView");

            let (reported, answer) = mpsc::channel();
            let state = set_hidden(&window, false, move |accepted| {
                let _ = reported.send(accepted);
            })
            .map_err(|error| format!("resuming a hidden window failed: {error}"))?;
            if state != MemoryState::Active {
                return Err(format!("showing the window asked for {state:?}"));
            }
            if !answer
                .recv_timeout(Duration::from_secs(5))
                .map_err(|_| "the runtime never answered the resume".to_string())?
            {
                return Err("the runtime refused to resume the window".into());
            }
            // A resumed window that stayed invisible would come back blank.
            if !controller_is_visible(&window)? {
                return Err("a resumed WebView is still invisible, so the window would be blank".into());
            }
            eprintln!("the window came back visible and running");

            // The window keeps its WebView: the action queue and the page state
            // are what make reopening instant, so nothing may be destroyed here.
            if app.webview_windows().len() != 1 {
                return Err("a memory hint changed the number of windows".into());
            }
            let window = app_lib::window::ensure(&app, "main")?;
            window.destroy().map_err(|e| e.to_string())?;
            Ok(())
        },
    );
}
