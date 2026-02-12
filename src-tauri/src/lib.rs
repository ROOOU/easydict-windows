mod config;
mod ocr;
mod translate;
mod tts;

use config::{AppConfig, load_config, save_config};
use reqwest::Client;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{
    AppHandle, Emitter, Manager,
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    WebviewWindowBuilder, WebviewUrl,
};

pub struct ScreenshotData {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

pub struct AppState {
    pub config: Mutex<AppConfig>,
    pub client: Client,
    pub clipboard_monitoring: Arc<AtomicBool>,
    pub screenshot_data: Mutex<Option<ScreenshotData>>,
    pub screenshot_in_progress: AtomicBool,
}

// ==================== Tauri Commands ====================

#[tauri::command]
fn get_config(state: tauri::State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

#[tauri::command]
fn update_config(state: tauri::State<AppState>, config: AppConfig) {
    state.clipboard_monitoring.store(
        config.select_translate.enabled && config.select_translate.monitor_clipboard,
        Ordering::Relaxed,
    );
    save_config(&config);
    *state.config.lock().unwrap() = config;
}

#[tauri::command]
fn get_languages() -> Vec<translate::LangPair> {
    translate::supported_languages()
}

#[tauri::command]
fn detect_language(text: String) -> String {
    translate::detect_language(&text)
}

#[tauri::command]
async fn translate_text(
    state: tauri::State<'_, AppState>,
    text: String,
    source: String,
    target: String,
) -> Result<Vec<translate::TranslateResult>, String> {
    let config = state.config.lock().unwrap().clone();
    let client = &state.client;

    let actual_source = if source == "auto" {
        translate::detect_language(&text)
    } else {
        source.clone()
    };

    let actual_target = if target == "auto" {
        translate::auto_target_lang(&actual_source, &config.general.target_lang)
    } else {
        target.clone()
    };

    let mut handles: Vec<tokio::task::JoinHandle<translate::TranslateResult>> = Vec::new();

    if config.services.google.enabled {
        let c = client.clone();
        let t = text.clone();
        let s = source.clone();
        let tgt = actual_target.clone();
        handles.push(tokio::spawn(async move {
            translate::google_translate(&c, &t, &s, &tgt).await
        }));
    }
    if config.services.bing.enabled {
        let c = client.clone();
        let t = text.clone();
        let s = source.clone();
        let tgt = actual_target.clone();
        handles.push(tokio::spawn(async move {
            translate::bing_translate(&c, &t, &s, &tgt).await
        }));
    }
    if config.services.deepl.enabled {
        let c = client.clone();
        let t = text.clone();
        let s = source.clone();
        let tgt = actual_target.clone();
        let key = config.services.deepl.api_key.clone();
        handles.push(tokio::spawn(async move {
            translate::deepl_translate(&c, &t, &s, &tgt, &key).await
        }));
    }
    if config.services.baidu.enabled {
        let c = client.clone();
        let t = text.clone();
        let s = source.clone();
        let tgt = actual_target.clone();
        let app_id = config.services.baidu.app_id.clone();
        let secret = config.services.baidu.secret_key.clone();
        handles.push(tokio::spawn(async move {
            translate::baidu_translate(&c, &t, &s, &tgt, &app_id, &secret).await
        }));
    }
    if config.services.openai.enabled {
        let c = client.clone();
        let t = text.clone();
        let s = source.clone();
        let tgt = actual_target.clone();
        let key = config.services.openai.api_key.clone();
        let url = config.services.openai.api_url.clone();
        let model = config.services.openai.model.clone();
        handles.push(tokio::spawn(async move {
            translate::openai_translate(&c, &t, &s, &tgt, &key, &url, &model).await
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        match handle.await {
            Ok(result) => results.push(result),
            Err(e) => results.push(translate::TranslateResult {
                service: "Unknown".to_string(),
                translated: String::new(),
                source_lang: source.clone(),
                target_lang: actual_target.clone(),
                error: Some(format!("Task error: {}", e)),
            }),
        }
    }
    Ok(results)
}

#[tauri::command]
fn speak(text: String) -> Result<(), String> {
    tts::speak_text(&text)
}

#[tauri::command]
fn get_clipboard_text() -> Result<String, String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("Clipboard error: {}", e))?;
    clipboard.get_text().map_err(|e| format!("Clipboard read error: {}", e))
}

/// Called when the floating icon is clicked
#[tauri::command]
fn float_icon_clicked(app: AppHandle) {
    if let Some(float_win) = app.get_webview_window("float-icon") {
        float_win.hide().ok();
    }
    app.emit("select-translate", ()).ok();
    if let Some(win) = app.get_webview_window("main") {
        win.show().ok();
        win.set_focus().ok();
    }
}

// ==================== Screenshot OCR Commands ====================

/// Step 1: Capture full screen, store it, open region selector window
#[tauri::command]
async fn start_screenshot_ocr(app: AppHandle, state: tauri::State<'_, AppState>) -> Result<(), String> {
    // Prevent multiple simultaneous triggers
    if state.screenshot_in_progress.swap(true, Ordering::SeqCst) {
        eprintln!("[OCR] Screenshot already in progress, ignoring");
        return Ok(());
    }

    eprintln!("[OCR] Starting screenshot capture...");

    // Hide main window first so it doesn't appear in the screenshot
    if let Some(win) = app.get_webview_window("main") {
        win.hide().ok();
    }
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // Destroy any existing screenshot-select window
    if let Some(win) = app.get_webview_window("screenshot-select") {
        win.destroy().ok();
    }

    // Capture the screen (raw RGBA, no PNG encoding)
    let (rgba, w, h) = match tokio::task::spawn_blocking(|| {
        ocr::capture_screen()
    }).await {
        Ok(Ok(data)) => data,
        Ok(Err(e)) => {
            state.screenshot_in_progress.store(false, Ordering::SeqCst);
            if let Some(win) = app.get_webview_window("main") {
                win.show().ok();
                win.set_focus().ok();
            }
            return Err(e);
        }
        Err(e) => {
            state.screenshot_in_progress.store(false, Ordering::SeqCst);
            if let Some(win) = app.get_webview_window("main") {
                win.show().ok();
                win.set_focus().ok();
            }
            return Err(format!("Task join error: {}", e));
        }
    };

    eprintln!("[OCR] Screenshot captured: {}x{}, {} bytes RGBA", w, h, rgba.len());

    // Store raw RGBA data for later processing
    *state.screenshot_data.lock().unwrap() = Some(ScreenshotData { rgba, width: w, height: h });

    // Open the region selection window
    let url = WebviewUrl::App("screenshot-select.html".into());
    match WebviewWindowBuilder::new(&app, "screenshot-select", url)
        .title("截图选区")
        .maximized(true)
        .position(0.0, 0.0)
        .decorations(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .focused(true)
        .build()
    {
        Ok(_) => {
            eprintln!("[OCR] Screenshot selection window created");
        }
        Err(e) => {
            eprintln!("[OCR] Failed to create window: {}", e);
            state.screenshot_in_progress.store(false, Ordering::SeqCst);
            if let Some(win) = app.get_webview_window("main") {
                win.show().ok();
                win.set_focus().ok();
            }
            return Err(format!("Failed to create selection window: {}", e));
        }
    }

    Ok(())
}

/// Return screenshot as base64 JPEG string for the selection window canvas (fast)
#[tauri::command]
fn get_screenshot_base64(state: tauri::State<AppState>) -> Result<String, String> {
    use base64::Engine;
    let guard = state.screenshot_data.lock().unwrap();
    let data = guard.as_ref().ok_or("No screenshot data")?;

    // Encode raw RGBA to JPEG (much faster than PNG for preview)
    let mut jpeg_buf: Vec<u8> = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg_buf, 85);
    encoder.encode(&data.rgba, data.width, data.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| format!("JPEG encode error: {}", e))?;

    let b64 = base64::engine::general_purpose::STANDARD.encode(&jpeg_buf);
    eprintln!("[OCR] get_screenshot_base64: JPEG {}KB, base64 {} chars", jpeg_buf.len() / 1024, b64.len());
    Ok(format!("data:image/jpeg;base64,{}", b64))
}

/// Step 2: OCR the selected region and send result to main window
#[tauri::command]
async fn ocr_selected_region(
    app: AppHandle,
    state: tauri::State<'_, AppState>,
    x: u32, y: u32, w: u32, h: u32,
) -> Result<(), String> {
    eprintln!("[OCR] ocr_selected_region: x={}, y={}, w={}, h={}", x, y, w, h);

    // Extract raw RGBA data and dimensions
    let (rgba, img_w, img_h) = {
        let guard = state.screenshot_data.lock().unwrap();
        let data = guard.as_ref().ok_or("No screenshot data".to_string())?;
        (data.rgba.clone(), data.width, data.height)
    };

    let result = tokio::task::spawn_blocking(move || {
        // Clamp crop region to image bounds
        let cx = x.min(img_w.saturating_sub(1));
        let cy = y.min(img_h.saturating_sub(1));
        let cw = w.min(img_w.saturating_sub(cx));
        let ch = h.min(img_h.saturating_sub(cy));

        eprintln!("[OCR] Cropping: {}x{} at ({},{})", cw, ch, cx, cy);

        // Crop directly from raw RGBA buffer (no image decode needed)
        let stride = (img_w * 4) as usize;
        let mut cropped_rgba: Vec<u8> = Vec::with_capacity((cw * ch * 4) as usize);
        for row in cy..(cy + ch) {
            let start = row as usize * stride + cx as usize * 4;
            let end = start + cw as usize * 4;
            cropped_rgba.extend_from_slice(&rgba[start..end]);
        }

        // Only PNG-encode the small cropped region for OCR
        let mut png_buf: Vec<u8> = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_buf);
        image::ImageEncoder::write_image(
            encoder,
            &cropped_rgba,
            cw,
            ch,
            image::ExtendedColorType::Rgba8,
        ).map_err(|e| format!("PNG encode error: {}", e))?;

        ocr::ocr_from_png_bytes(&png_buf, "auto")
    }).await.map_err(|e| format!("Task join error: {}", e))?;

    // Clear stored screenshot and reset flag
    *state.screenshot_data.lock().unwrap() = None;
    state.screenshot_in_progress.store(false, Ordering::SeqCst);

    // Send result via GLOBAL event
    match result {
        Ok(text) => {
            eprintln!("[OCR] OCR success: {} chars", text.len());
            app.emit("ocr-result", text).ok();
        }
        Err(e) => {
            eprintln!("[OCR] OCR error: {}", e);
            app.emit("ocr-error", e).ok();
        }
    }

    // Show main window
    if let Some(win) = app.get_webview_window("main") {
        win.show().ok();
        win.set_focus().ok();
    }

    Ok(())
}

/// Cancel screenshot: clean up and show main window again
#[tauri::command]
fn cancel_screenshot(app: AppHandle, state: tauri::State<AppState>) {
    eprintln!("[OCR] cancel_screenshot called");
    *state.screenshot_data.lock().unwrap() = None;
    state.screenshot_in_progress.store(false, Ordering::SeqCst);
    if let Some(win) = app.get_webview_window("main") {
        win.show().ok();
        win.set_focus().ok();
    }
}

// ==================== Select-to-Translate: Mouse Monitor ====================

#[cfg(target_os = "windows")]
fn get_cursor_pos() -> (i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
    use windows::Win32::Foundation::POINT;
    let mut point = POINT { x: 0, y: 0 };
    unsafe { let _ = GetCursorPos(&mut point); }
    (point.x, point.y)
}

#[cfg(not(target_os = "windows"))]
fn get_cursor_pos() -> (i32, i32) { (100, 100) }

#[cfg(target_os = "windows")]
fn simulate_ctrl_c() {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;
    use std::mem;

    let mut inputs: [INPUT; 4] = unsafe { mem::zeroed() };

    inputs[0].r#type = INPUT_KEYBOARD;
    inputs[0].Anonymous.ki.wVk = VIRTUAL_KEY(0x11); // VK_CONTROL
    inputs[1].r#type = INPUT_KEYBOARD;
    inputs[1].Anonymous.ki.wVk = VIRTUAL_KEY(0x43); // VK_C
    inputs[2].r#type = INPUT_KEYBOARD;
    inputs[2].Anonymous.ki.wVk = VIRTUAL_KEY(0x43);
    inputs[2].Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;
    inputs[3].r#type = INPUT_KEYBOARD;
    inputs[3].Anonymous.ki.wVk = VIRTUAL_KEY(0x11);
    inputs[3].Anonymous.ki.dwFlags = KEYEVENTF_KEYUP;

    unsafe { SendInput(&inputs, mem::size_of::<INPUT>() as i32); }
}

#[cfg(target_os = "windows")]
fn is_mouse_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    unsafe { GetAsyncKeyState(0x01) & (0x8000u16 as i16) != 0 }
}

fn start_select_monitor(app: &AppHandle, monitoring_flag: Arc<AtomicBool>) {
    let app_handle = app.clone();

    std::thread::spawn(move || {
        let mut was_pressed = false;
        let mut press_pos: (i32, i32) = (0, 0);
        let mut last_click_time = std::time::Instant::now()
            .checked_sub(std::time::Duration::from_secs(10))
            .unwrap_or_else(std::time::Instant::now);
        let mut last_click_pos: (i32, i32) = (0, 0);

        let mut prev_clipboard = match arboard::Clipboard::new() {
            Ok(mut cb) => cb.get_text().unwrap_or_default(),
            Err(_) => String::new(),
        };

        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));

            if !monitoring_flag.load(Ordering::Relaxed) {
                continue;
            }

            let is_pressed = is_mouse_down();
            let cursor = get_cursor_pos();

            if is_pressed && !was_pressed {
                press_pos = cursor;
            } else if !is_pressed && was_pressed {
                let dx = (cursor.0 - press_pos.0).abs();
                let dy = (cursor.1 - press_pos.1).abs();
                let since_last_click = last_click_time.elapsed();
                let click_dx = (cursor.0 - last_click_pos.0).abs();
                let click_dy = (cursor.1 - last_click_pos.1).abs();

                let is_drag = dx > 15 || dy > 8;
                let is_double_click = since_last_click < std::time::Duration::from_millis(500)
                    && click_dx < 15
                    && click_dy < 15
                    && !is_drag;

                if is_drag || is_double_click {
                    // Wait longer after double-click for browser to complete selection
                    let wait_ms = if is_double_click { 300 } else { 100 };
                    std::thread::sleep(std::time::Duration::from_millis(wait_ms));

                    let old_clip = match arboard::Clipboard::new() {
                        Ok(mut cb) => cb.get_text().unwrap_or_default(),
                        Err(_) => prev_clipboard.clone(),
                    };

                    simulate_ctrl_c();
                    std::thread::sleep(std::time::Duration::from_millis(200));

                    let new_clip = match arboard::Clipboard::new() {
                        Ok(mut cb) => cb.get_text().unwrap_or_default(),
                        Err(_) => String::new(),
                    };

                    // Only check: clipboard changed after our Ctrl+C and has content
                    if !new_clip.trim().is_empty() && new_clip != old_clip {
                        prev_clipboard = new_clip.clone();

                        let mode = {
                            let state = app_handle.state::<AppState>();
                            let config = state.config.lock().unwrap();
                            if !config.select_translate.enabled {
                                last_click_time = std::time::Instant::now();
                                last_click_pos = cursor;
                                was_pressed = is_pressed;
                                continue;
                            }
                            config.select_translate.mode.clone()
                        };

                        match mode.as_str() {
                            "auto" => {
                                app_handle.emit("clipboard-translate", new_clip).ok();
                                if let Some(win) = app_handle.get_webview_window("main") {
                                    win.show().ok();
                                    win.set_focus().ok();
                                }
                            }
                            "icon" => {
                                show_float_icon(&app_handle);
                            }
                            _ => {}
                        }
                    } else if new_clip != old_clip {
                        prev_clipboard = new_clip;
                    }
                }

                last_click_time = std::time::Instant::now();
                last_click_pos = cursor;
            }

            // Also monitor clipboard for manual Ctrl+C
            if !is_pressed && !was_pressed {
                if let Ok(mut cb) = arboard::Clipboard::new() {
                    if let Ok(current) = cb.get_text() {
                        if !current.trim().is_empty() && current != prev_clipboard {
                            prev_clipboard = current.clone();

                            let mode = {
                                let state = app_handle.state::<AppState>();
                                let config = state.config.lock().unwrap();
                                if !config.select_translate.enabled {
                                    was_pressed = is_pressed;
                                    continue;
                                }
                                config.select_translate.mode.clone()
                            };

                            match mode.as_str() {
                                "auto" => {
                                    app_handle.emit("clipboard-translate", current).ok();
                                    if let Some(win) = app_handle.get_webview_window("main") {
                                        win.show().ok();
                                        win.set_focus().ok();
                                    }
                                }
                                "icon" => {
                                    show_float_icon(&app_handle);
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }

            was_pressed = is_pressed;
        }
    });
}

fn show_float_icon(app: &AppHandle) {
    let (cx, cy) = get_cursor_pos();
    if let Some(win) = app.get_webview_window("float-icon") {
        win.set_position(tauri::PhysicalPosition::new(cx + 15, cy - 45)).ok();
        win.show().ok();
    } else {
        let url = WebviewUrl::App("float-icon.html".into());
        let _ = WebviewWindowBuilder::new(app, "float-icon", url)
            .title("")
            .inner_size(42.0, 42.0)
            .position((cx + 15) as f64, (cy - 45) as f64)
            .decorations(false)
            .transparent(true)
            .always_on_top(true)
            .resizable(false)
            .skip_taskbar(true)
            .focused(false)
            .build();
    }
}

// ==================== App Setup ====================

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItemBuilder::with_id("show", "显示主窗口").build(app)?;
    let input = MenuItemBuilder::with_id("input_translate", "输入翻译").build(app)?;
    let screenshot = MenuItemBuilder::with_id("screenshot_ocr", "截图翻译").build(app)?;
    let separator = tauri::menu::PredefinedMenuItem::separator(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;

    let menu = MenuBuilder::new(app)
        .items(&[&show, &input, &screenshot, &separator, &quit])
        .build()?;

    let _tray = TrayIconBuilder::new()
        .tooltip("EasyDict")
        .menu(&menu)
        .on_menu_event(move |app, event| {
            match event.id().as_ref() {
                "show" | "input_translate" => {
                    if let Some(win) = app.get_webview_window("main") {
                        win.show().ok();
                        win.set_focus().ok();
                    }
                }
                "screenshot_ocr" => {
                    app.emit("trigger-screenshot", ()).ok();
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, event| {
            if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                let app = tray.app_handle();
                if let Some(win) = app.get_webview_window("main") {
                    win.show().ok();
                    win.set_focus().ok();
                }
            }
        })
        .build(app)?;

    Ok(())
}

fn register_shortcuts_from_config(app: &AppHandle) {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    let config = {
        let state = app.state::<AppState>();
        let cfg = state.config.lock().unwrap().clone();
        cfg
    };

    // Input translate
    if config.hotkeys.input_translate.enabled && !config.hotkeys.input_translate.shortcut.is_empty() {
        let shortcut = config.hotkeys.input_translate.shortcut.clone();
        let app_handle = app.clone();
        if let Err(e) = app.global_shortcut().on_shortcut(shortcut.as_str(), move |_app, _shortcut, _event| {
            app_handle.emit("focus-input", ()).ok();
            if let Some(win) = app_handle.get_webview_window("main") {
                win.show().ok();
                win.set_focus().ok();
            }
        }) {
            eprintln!("Failed to register {}: {}", shortcut, e);
        }
    }

    // Select translate
    if config.hotkeys.select_translate.enabled && !config.hotkeys.select_translate.shortcut.is_empty() {
        let shortcut = config.hotkeys.select_translate.shortcut.clone();
        let app_handle = app.clone();
        if let Err(e) = app.global_shortcut().on_shortcut(shortcut.as_str(), move |_app, _shortcut, _event| {
            app_handle.emit("select-translate", ()).ok();
            if let Some(win) = app_handle.get_webview_window("main") {
                win.show().ok();
                win.set_focus().ok();
            }
        }) {
            eprintln!("Failed to register {}: {}", shortcut, e);
        }
    }

    // Screenshot translate
    if config.hotkeys.screenshot_translate.enabled && !config.hotkeys.screenshot_translate.shortcut.is_empty() {
        let shortcut = config.hotkeys.screenshot_translate.shortcut.clone();
        let app_handle = app.clone();
        if let Err(e) = app.global_shortcut().on_shortcut(shortcut.as_str(), move |_app, _shortcut, _event| {
            app_handle.emit("trigger-screenshot", ()).ok();
        }) {
            eprintln!("Failed to register {}: {}", shortcut, e);
        }
    }
}

fn setup_shortcuts(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    register_shortcuts_from_config(app);
    Ok(())
}

#[tauri::command]
fn update_shortcuts(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;

    // Unregister all existing shortcuts
    if let Err(e) = app.global_shortcut().unregister_all() {
        eprintln!("Failed to unregister shortcuts: {}", e);
    }

    // Re-register from current config
    register_shortcuts_from_config(&app);
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let config = load_config();
    let client = Client::new();
    let monitoring = Arc::new(AtomicBool::new(
        config.select_translate.enabled && config.select_translate.monitor_clipboard,
    ));
    let state = AppState {
        config: Mutex::new(config),
        client,
        clipboard_monitoring: monitoring.clone(),
        screenshot_data: Mutex::new(None),
        screenshot_in_progress: AtomicBool::new(false),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_shell::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            get_config,
            update_config,
            get_languages,
            detect_language,
            translate_text,
            speak,
            get_clipboard_text,
            float_icon_clicked,
            start_screenshot_ocr,
            get_screenshot_base64,
            ocr_selected_region,
            cancel_screenshot,
            update_shortcuts,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            setup_tray(&handle)?;
            setup_shortcuts(&handle)?;
            start_select_monitor(&handle, monitoring.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
