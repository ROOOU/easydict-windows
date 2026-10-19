//! The Tauri command surface.
//!
//! This is the whole contract between the Rust core and the React front-end:
//! every command below has a matching wrapper in `src/api.ts`. Commands stay
//! thin — they translate JSON into the domain types in `model`/`config` and
//! hand the work to [`AppState`].

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::config::{QueryRecord, ServiceConfig, Settings};
use crate::model::{
    ApiKeyRequirement, Language, LanguageInfo, QueryError, QueryModel, QueryResult, QueryTextType,
    ServiceType, WindowType,
};
use crate::query::QueryEvent;
use crate::service::registry;
use crate::state::AppState;

/// Event names the front-end listens on.
pub mod events {
    /// A service produced or updated a result. Payload: `QueryEventPayload`.
    pub const QUERY_UPDATE: &str = "query-update";
    /// Every service settled. Payload: `QueryFinishedPayload`.
    pub const QUERY_FINISHED: &str = "query-finished";
    pub const SETTINGS_CHANGED: &str = "settings-changed";
    /// Wake the current window's action queue; carries no action data.
    pub const ACTION: &str = "action";
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// `serde` default for the many `Language` fields the front-end may omit.
fn auto_language() -> Language {
    Language::Auto
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRequest {
    pub text: String,
    #[serde(default)]
    pub request_id: String,
    #[serde(default = "auto_language")]
    pub from: Language,
    #[serde(default = "auto_language")]
    pub to: Language,
    /// Which window asked, so the result can be routed back to it.
    #[serde(default)]
    pub window_type: WindowTypeDto,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowTypeDto {
    #[default]
    Main,
    Mini,
    Fixed,
}

impl From<WindowTypeDto> for WindowType {
    fn from(value: WindowTypeDto) -> Self {
        match value {
            WindowTypeDto::Main => WindowType::Main,
            WindowTypeDto::Mini => WindowType::Mini,
            WindowTypeDto::Fixed => WindowType::Fixed,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryEventPayload {
    pub request_id: String,
    pub identifier: String,
    pub service_type: ServiceType,
    pub result: QueryResult,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryFinishedPayload {
    pub request_id: String,
    pub cancelled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionPayload {
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_target: Option<[isize; 2]>,
}

impl ActionPayload {
    pub fn new(action: &str, text: Option<String>) -> Self {
        Self { action: action.into(), text, selection_target: None }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDescriptor {
    pub identifier: String,
    pub service_type: ServiceType,
    pub name: String,
    pub name_key: &'static str,
    pub api_key_requirement: ApiKeyRequirement,
    pub is_stream: bool,
    pub link: Option<String>,
    pub supported_query_type: QueryTextType,
    pub configurable_intelligent_query: bool,
    pub config: ServiceConfig,
    pub requires_api_key: bool,
    pub has_api_key: bool,
    /// False for the two services that wrap macOS-only frameworks.
    pub available: bool,
    pub allows_multiple_instances: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceValidation {
    pub success: bool,
    pub message: String,
    /// Set when validation turned streaming off as a side effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub validation_note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
    pub platform: &'static str,
    pub data_dir: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

/// `x, y, width, height`, in the same shape as the front-end's `OcrLine`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrLineDto {
    pub text: String,
    pub bounding_box: [f32; 4],
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrOutput {
    pub text: String,
    pub lines: Vec<OcrLineDto>,
    pub language: Option<Language>,
    pub confidence: Option<f32>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotOutput {
    pub output: Option<OcrOutput>,
    pub restore_window: bool,
}

impl From<crate::ocr::OcrOutput> for OcrOutput {
    fn from(value: crate::ocr::OcrOutput) -> Self {
        OcrOutput {
            text: value.text,
            lines: value
                .lines
                .into_iter()
                .map(|line| OcrLineDto {
                    text: line.text,
                    bounding_box: line.bounding_box,
                })
                .collect(),
            language: value.language,
            confidence: value.confidence,
        }
    }
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: State<'_, Arc<AppState>>) -> Settings {
    state.settings()
}

#[tauri::command]
pub fn get_desktop_status(state: State<'_, Arc<AppState>>) -> crate::desktop::DesktopStatus {
    crate::desktop::status(&state.app, &state.settings())
}

/// Applies a partial settings object: every field present in `patch` replaces
/// the stored value, everything absent is left alone.
#[tauri::command]
pub fn update_settings(
    state: State<'_, Arc<AppState>>,
    patch: serde_json::Value,
) -> Result<Settings, String> {
    state.update_settings_with_autostart(
        |settings| merge_settings(settings, &patch),
        patch.get("launchAtStartup").is_some(),
    )
}

#[tauri::command]
pub fn reset_settings(state: State<'_, Arc<AppState>>) -> Result<Settings, String> {
    state.update_settings_with_autostart(|settings| {
        *settings = Settings::default();
        Ok(())
    }, true)
}

#[tauri::command]
pub fn export_settings(
    state: State<'_, Arc<AppState>>,
    path: String,
    include_secrets: Option<bool>,
) -> Result<(), String> {
    let settings = state.settings();
    state
        .store
        .export_settings(std::path::Path::new(&path), &settings, include_secrets.unwrap_or(false))
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn import_settings(state: State<'_, Arc<AppState>>, path: String) -> Result<Settings, String> {
    let imported = state
        .store
        .import_settings(std::path::Path::new(&path))
        .map_err(|e| e.to_string())?;
    state.update_settings_with_autostart(|settings| {
        *settings = imported;
        Ok(())
    }, true)
}

/// Applies a partial settings object, as `update_settings` does.
///
/// A key containing `.` is a path into the document rather than a field name,
/// which is the shape `lib.rs`'s `easydict://writeKeyValue` import produces.
pub fn merge_settings(settings: &mut Settings, patch: &serde_json::Value) -> Result<(), String> {
    merge_object(settings, patch).map_err(|e| e.to_string())
}

/// Merges a partial JSON object into the settings in place.
///
/// Serialising the current settings and re-deserialising the patched document
/// would be simpler, but it would also lose the `#[serde(default)]` values of
/// any field the front-end omits — which is exactly what a patch is.
fn merge_object<T: Serialize + for<'de> Deserialize<'de>>(
    target: &mut T,
    patch: &serde_json::Value,
) -> Result<(), serde_json::Error> {
    let mut base = serde_json::to_value(&*target)?;
    merge_value(&mut base, patch);
    *target = serde_json::from_value(base)?;
    Ok(())
}

fn merge_value(base: &mut serde_json::Value, patch: &serde_json::Value) {
    match (base, patch) {
        (serde_json::Value::Object(base_map), serde_json::Value::Object(patch_map)) => {
            for (key, value) in patch_map {
                merge_entry(base_map, key, value);
            }
        }
        (slot, value) => *slot = value.clone(),
    }
}

/// Applies one patch entry to an object.
///
/// A dotted key names a path, not a field: the deep-link import
/// (`easydict://writeKeyValue?EZYoudaoAPIKey=…`) maps to
/// `services.Youdao.apiKey`, whose value lives two maps deep. Inserting that
/// key verbatim would leave it to be dropped by `serde` as an unknown field —
/// the import would report success and write nothing.
fn merge_entry(
    object: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: &serde_json::Value,
) {
    let Some((head, rest)) = key.split_once('.') else {
        match object.get_mut(key) {
            Some(slot) => merge_value(slot, value),
            None => {
                object.insert(key.to_string(), value.clone());
            }
        }
        return;
    };

    // A scalar standing where the path needs an object cannot take the entry;
    // the deserialiser then rejects the result unless the schema expects it.
    let mut nested = match object.remove(head) {
        Some(serde_json::Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    };
    merge_entry(&mut nested, rest, value);
    object.insert(head.to_string(), serde_json::Value::Object(nested));
}

// ---------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn list_services(state: State<'_, Arc<AppState>>) -> Vec<ServiceDescriptor> {
    let settings = state.settings();
    registry::descriptors(&settings)
}

#[tauri::command]
pub fn get_service_config(
    state: State<'_, Arc<AppState>>,
    identifier: String,
) -> ServiceConfig {
    state.settings().service(&identifier)
}

#[tauri::command]
pub fn update_service_config(
    state: State<'_, Arc<AppState>>,
    identifier: String,
    patch: serde_json::Value,
) -> Result<ServiceConfig, String> {
    let settings = state.update_settings(|settings| {
        let mut config = settings.service(&identifier);
        merge_object(&mut config, &patch).map_err(|e| e.to_string())?;
        settings.services.insert(identifier.clone(), config);
        Ok(())
    })?;
    Ok(settings.service(&identifier))
}

/// `validate()` — runs one short translation through the service and reports
/// whether it came back. The sample text is upstream's.
#[tauri::command]
pub async fn validate_service(
    state: State<'_, Arc<AppState>>,
    identifier: String,
) -> Result<ServiceValidation, String> {
    let handle = state
        .handle_for(&identifier)
        .ok_or_else(|| format!("unknown service: {identifier}"))?;

    let mut model = QueryModel::new("曾经沧海难为水");
    model.query_from_language = Language::SimplifiedChinese;
    model.query_target_language = Language::English;
    model.query_type = QueryTextType::TRANSLATION;

    let outcome = crate::query::run_direct(&handle, &model).await;

    Ok(match outcome {
        Ok(result) if result.has_translated_result() => ServiceValidation {
            success: true,
            message: "Validation success 🎉".to_string(),
            validation_note: result.validation_message,
        },
        Ok(result) => ServiceValidation {
            success: false,
            message: result
                .error_message()
                .unwrap_or_else(|| "Validation failed 💔".to_string()),
            validation_note: result.validation_message,
        },
        Err(err) => ServiceValidation {
            success: false,
            message: err.to_string(),
            validation_note: None,
        },
    })
}

/// Fetches the provider's model list, when it exposes one.
#[tauri::command]
pub async fn fetch_service_models(
    state: State<'_, Arc<AppState>>,
    identifier: String,
) -> Result<Vec<String>, String> {
    let handle = state
        .handle_for(&identifier)
        .ok_or_else(|| format!("unknown service: {identifier}"))?;
    registry::fetch_remote_models(handle.service.as_ref())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn service_languages(
    state: State<'_, Arc<AppState>>,
    identifier: String,
) -> Vec<LanguageInfo> {
    let Some(handle) = state.handle_for(&identifier) else {
        return Vec::new();
    };
    let mut languages: Vec<Language> = handle.service.supported_languages();
    languages.sort_by_key(|l| l.code());
    languages
        .into_iter()
        .filter(|l| *l != Language::Auto)
        .map(LanguageInfo::of)
        .collect()
}

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn query(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    request: QueryRequest,
) -> Result<(), String> {
    let model = build_query_model(&state, &request);
    if model.query_text.is_empty() {
        return Err("处理后的查询内容为空".into());
    }

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<QueryEvent>();
    let target = model.window_type.label();
    let request_id = request.request_id;
    log::info!(
        "query: window={target} id={} chars={} {}->{}",
        request_id,
        model.query_text.chars().count(),
        model.query_from_language.raw(),
        model.query_target_language.raw()
    );
    state.start_query(model.clone(), request_id.clone(), tx);

    let state = state.inner().clone();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut finished = false;
        while let Some(event) = rx.recv().await {
            match event {
                QueryEvent::Update {
                    identifier,
                    service_type,
                    result,
                } => {
                    log::debug!(
                        "query: {identifier} -> {target} showing={} chars={} error={:?}",
                        result.is_showing,
                        result.translated_text().map(|text| text.chars().count()).unwrap_or(0),
                        result.error.as_ref().map(|error| error.to_string())
                    );
                    let _ = app.emit_to(
                        target,
                        events::QUERY_UPDATE,
                        QueryEventPayload {
                            request_id: request_id.clone(),
                            identifier,
                            service_type,
                            result: *result,
                        },
                    );
                }
                QueryEvent::AllFinished { cancelled } => {
                    finished = !cancelled;
                    log::info!("query: window={target} finished cancelled={cancelled}");
                    let _ = app.emit_to(target, events::QUERY_FINISHED, QueryFinishedPayload {
                        request_id: request_id.clone(),
                        cancelled,
                    });
                }
            }
        }
        if finished {
            state.record_history(&model);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn stop_query(
    state: State<'_, Arc<AppState>>,
    window_type: Option<WindowTypeDto>,
    request_id: Option<String>,
) {
    state.stop_query(window_type.unwrap_or_default().into(), request_id.as_deref());
}

#[tauri::command]
pub async fn retry_query(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    request: QueryRequest,
) -> Result<(), String> {
    query(app, state, request).await
}

/// Runs one service directly and returns its result — used by the "retry this
/// service" button and by the settings validation flow.
#[tauri::command]
pub async fn query_single(
    state: State<'_, Arc<AppState>>,
    identifier: String,
    request: QueryRequest,
) -> Result<QueryResult, String> {
    let handle = state
        .handle_for(&identifier)
        .ok_or_else(|| format!("unknown service: {identifier}"))?;
    let model = build_query_model(&state, &request);
    crate::query::run_direct(&handle, &model)
        .await
        .map_err(|e| e.to_string())
}

fn build_query_model(state: &AppState, request: &QueryRequest) -> QueryModel {
    let settings = state.settings();
    let mut model = QueryModel::new_with_settings(request.text.trim(), &settings);
    model.window_type = request.window_type.into();

    model.query_from_language = if request.from == Language::Auto {
        state.detect_language(&model.query_text)
    } else {
        request.from
    };
    model.query_target_language = if request.to == Language::Auto {
        default_target(&settings, model.query_from_language)
    } else {
        request.to
    };
    model.query_type = crate::util::analysis::query_type(
        &model.query_text,
        model.query_from_language,
        2,
    );
    model.action_type = crate::model::ActionType::InputQuery;
    model
}

/// The target language for an auto-translate, mirroring the macOS picker: go
/// to the first language unless the source already is it.
pub fn default_target(settings: &Settings, from: Language) -> Language {
    if from == settings.first_language {
        settings.second_language
    } else {
        settings.first_language
    }
}

// ---------------------------------------------------------------------------
// Clipboard, selection and text actions
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn clipboard_text() -> Result<String, String> {
    let mut clipboard = arboard_clipboard()?;
    clipboard.get_text().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_clipboard_text(text: String) -> Result<(), String> {
    let mut clipboard = arboard_clipboard()?;
    clipboard.set_text(text).map_err(|e| e.to_string())
}

fn arboard_clipboard() -> Result<arboard::Clipboard, String> {
    arboard::Clipboard::new().map_err(|e| format!("clipboard unavailable: {e}"))
}

/// The text selected in the foreground application.
///
/// `enableForceGetSelectedText` (on by default) allows the clipboard fallback
/// for controls that publish no UI Automation text provider; without it the
/// clipboard is never written.
#[tauri::command]
pub async fn selected_text(
    state: State<'_, Arc<AppState>>,
    target: Option<[isize; 2]>,
) -> Result<Option<String>, String> {
    let force = state.settings().enable_force_get_selected_text;
    tokio::task::spawn_blocking(move || {
        let current = || target.is_none_or(|target| crate::platform::selection_target() == Some(target));
        if !current() {
            return Ok(None);
        }
        let result = crate::platform::selected_text_at(force, target);
        if !current() {
            return Ok(None);
        }
        match result {
            Ok(text) => Ok(Some(text)),
            Err(crate::platform::PlatformError::NoSelection) => Ok(Some(String::new())),
            Err(error) => Err(error.to_string()),
        }
    }).await.map_err(|error| error.to_string())?
}

/// Translates text without touching the UI — used by OCR, "translate and
/// replace", and the mini window's pasteboard flow.
#[tauri::command]
pub async fn translate_text(
    state: State<'_, Arc<AppState>>,
    request: QueryRequest,
) -> Result<QueryResult, String> {
    let handles = state.active_handles();
    let model = build_query_model(&state, &request);

    for handle in handles {
        // The first service that answers wins; the rest stay idle so the
        // action returns promptly.
        if let Ok(result) = crate::query::run_direct(&handle, &model).await {
            if result.has_translated_result() {
                state.record_history(&model);
                return Ok(result);
            }
        }
    }

    Err(QueryError::new(crate::model::ErrorType::NoResult).to_string())
}

/// Replaces the current selection with `text`, via the clipboard.
#[tauri::command]
pub fn replace_selected_text(text: String, target: Option<[isize; 2]>) -> Result<(), String> {
    if target.is_some_and(|target| crate::platform::selection_target() != Some(target)) {
        return Err("原选区窗口已失去焦点，已取消替换".into());
    }
    let mut clipboard = arboard_clipboard()?;
    let previous = clipboard.get_text().ok();
    clipboard.set_text(text).map_err(|e| e.to_string())?;
    if target.is_some_and(|target| crate::platform::selection_target() != Some(target)) {
        if let Some(previous) = previous {
            let _ = clipboard.set_text(previous);
        }
        return Err("原选区窗口已失去焦点，已取消替换".into());
    }
    crate::system::send_paste_keystroke().map_err(|e| e.to_string())?;
    // Give the target application a moment to read the clipboard before the
    // previous contents go back.
    std::thread::sleep(std::time::Duration::from_millis(120));
    if let Some(previous) = previous {
        let _ = clipboard.set_text(previous);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// OCR
// ---------------------------------------------------------------------------

/// `image` is base64 without a data-URL prefix.
#[tauri::command]
pub async fn ocr_image(
    state: State<'_, Arc<AppState>>,
    image: String,
) -> Result<OcrOutput, String> {
    let bytes = crate::util::base64_decode_urlsafe(&image)
        .ok_or_else(|| "image is not valid base64".to_string())?;
    state
        .ocr
        .recognize_image(&bytes)
        .await
        .map(OcrOutput::from)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn screenshot_ocr(
    state: State<'_, Arc<AppState>>,
    region: Option<Rect>,
) -> Result<ScreenshotOutput, String> {
    let (complete, receive) = tokio::sync::oneshot::channel();
    std::thread::Builder::new().name("screenshot-selection".into()).spawn(move || {
        let captured = match region {
            Some(region) => crate::platform::capture_region(region.x, region.y, region.width, region.height)
                .map(|image| crate::platform::ScreenshotSelection { image: Some(image), restore_window: true }),
            None => crate::platform::capture_selection(),
        };
        let _ = complete.send(captured);
    }).map_err(|error| error.to_string())?;
    let captured = receive.await.map_err(|error| error.to_string())?.map_err(|error| error.to_string())?;
    let Some(image) = captured.image else {
        return Ok(ScreenshotOutput { output: None, restore_window: captured.restore_window });
    };
    let output = state
        .ocr
        .recognize_rgba(image.width, image.height, image.rgba)
        .await
        .map(OcrOutput::from)
        .map_err(|e| e.to_string())?;
    Ok(ScreenshotOutput { output: Some(output), restore_window: true })
}

#[tauri::command]
pub async fn pasteboard_ocr(state: State<'_, Arc<AppState>>) -> Result<OcrOutput, String> {
    let bytes = read_clipboard_image()?;
    state
        .ocr
        .recognize_image(&bytes)
        .await
        .map(OcrOutput::from)
        .map_err(|e| e.to_string())
}

fn read_clipboard_image() -> Result<Vec<u8>, String> {
    let mut clipboard = arboard_clipboard()?;
    let image = clipboard
        .get_image()
        .map_err(|e| format!("clipboard holds no image: {e}"))?;
    let buffer = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    )
    .ok_or_else(|| "clipboard image has an inconsistent size".to_string())?;
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(buffer)
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(png)
}

// ---------------------------------------------------------------------------
// Speech
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TtsRequest {
    pub text: String,
    #[serde(default = "auto_language")]
    pub language: Language,
    #[serde(default = "auto_language")]
    pub from: Language,
    #[serde(default = "auto_language")]
    pub to: Language,
    #[serde(default)]
    pub is_english_word: bool,
    /// Preferred when present — the URL a service already returned with its
    /// result, so playback does not rebuild it.
    #[serde(default)]
    pub url: Option<String>,
}

#[tauri::command]
pub async fn play_tts(
    state: State<'_, Arc<AppState>>,
    request: TtsRequest,
) -> Result<(), String> {
    if request.text.trim().is_empty() {
        return Err("nothing to speak".to_string());
    }

    let url = match request.url.clone() {
        Some(url) => Some(url),
        None => crate::tts::tts_url(
            &state.settings(),
            &request.text,
            request.from,
            request.to,
            request.is_english_word,
        ),
    };

    let outcome = match url {
        Some(url) => state.tts.play_url(&url).await,
        None => state.tts.speak(&request.text, request.language).await,
    };
    outcome.map_err(|e| e.to_string())
}

#[tauri::command]
pub fn stop_tts(state: State<'_, Arc<AppState>>) {
    state.tts.stop();
}

// ---------------------------------------------------------------------------
// Languages
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn detect_language(state: State<'_, Arc<AppState>>, text: String) -> Language {
    state.detect_language(&text)
}

#[tauri::command]
pub fn supported_languages() -> Vec<LanguageInfo> {
    crate::model::selectable_languages()
}

#[tauri::command]
pub fn system_languages() -> Vec<String> {
    crate::system::system_languages()
}

#[tauri::command]
pub fn toggle_languages(state: State<'_, Arc<AppState>>) -> Result<(Language, Language), String> {
    let settings = state.update_settings(|settings| {
        std::mem::swap(&mut settings.query_from_language, &mut settings.query_to_language);
        Ok(())
    })?;
    Ok((settings.query_from_language, settings.query_to_language))
}

// ---------------------------------------------------------------------------
// History and favourites
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn history(
    state: State<'_, Arc<AppState>>,
    limit: usize,
    offset: usize,
) -> Result<Vec<QueryRecord>, String> {
    state
        .store
        .history(limit, offset)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn search_history(
    state: State<'_, Arc<AppState>>,
    needle: String,
    limit: usize,
) -> Result<Vec<QueryRecord>, String> {
    state
        .store
        .search_history(&needle, limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn delete_history(
    state: State<'_, Arc<AppState>>,
    record: QueryRecord,
) -> Result<(), String> {
    state
        .store
        .delete_history(&record)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn clear_history(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.store.clear_history().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn history_count(state: State<'_, Arc<AppState>>) -> Result<usize, String> {
    state.store.history_count().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn favorites(state: State<'_, Arc<AppState>>) -> Result<Vec<QueryRecord>, String> {
    state.store.favorites().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn add_favorite(
    state: State<'_, Arc<AppState>>,
    record: QueryRecord,
) -> Result<(), String> {
    state
        .store
        .add_favorite(&record)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn remove_favorite(
    state: State<'_, Arc<AppState>>,
    record: QueryRecord,
) -> Result<(), String> {
    state
        .store
        .remove_favorite(&record)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn is_favorite(
    state: State<'_, Arc<AppState>>,
    record: QueryRecord,
) -> Result<bool, String> {
    state
        .store
        .is_favorite(&record)
        .map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Windows and app
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn show_window(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    label: String,
    position: Option<String>,
) -> Result<(), String> {
    let settings = state.settings();
    tauri::async_runtime::spawn_blocking(move || {
        crate::window::show(&app, &settings, &label, position.as_deref())
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn next_action(window: tauri::WebviewWindow, app: AppHandle) -> Option<ActionPayload> {
    app.state::<crate::window::Windows>().next(window.label())
}

#[tauri::command]
pub async fn open_settings(app: AppHandle, section: String) -> Result<(), String> {
    crate::window::enqueue(&app, "settings", ActionPayload::new(&section, None))?;
    let settings = app.state::<Arc<AppState>>().settings();
    tauri::async_runtime::spawn_blocking(move || {
        crate::window::show(&app, &settings, "settings", None)
    }).await.map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn report_action_error(app: AppHandle, error: String) {
    crate::window::report_error(&app, &error);
}

#[tauri::command]
pub fn hide_window(app: AppHandle, label: String) -> Result<(), String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("no window named {label}"))?;
    window.hide().map_err(|e| e.to_string())?;
    // Hidden windows keep their results but give the renderer's memory back.
    crate::platform::webview_memory::release_when_hidden(&app, &label);
    Ok(())
}

#[tauri::command]
pub fn pin_window(app: AppHandle, label: String, pinned: bool) -> Result<(), String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("no window named {label}"))?;
    window
        .set_always_on_top(pinned)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn set_window_size(
    app: AppHandle,
    label: String,
    width: f64,
    height: f64,
    animate: Option<bool>,
) -> Result<(), String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("no window named {label}"))?;
    crate::window::resize(&window, width, height, animate.unwrap_or(true))
}

/// The height the results area needs, so the window can grow with its content.
#[tauri::command]
pub fn fit_window_to_content(
    app: AppHandle,
    label: String,
    content_height: f64,
) -> Result<tauri::LogicalSize<f64>, String> {
    let window = app
        .get_webview_window(&label)
        .ok_or_else(|| format!("no window named {label}"))?;
    crate::window::fit_to_content(&window, content_height)
}

#[tauri::command]
pub fn open_link(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn app_info(state: State<'_, Arc<AppState>>) -> AppInfo {
    AppInfo {
        name: "Easydict",
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        data_dir: state.store.root().to_string_lossy().to_string(),
    }
}

/// Services grouped the way the results list stacks them.
#[tauri::command]
pub fn service_order(state: State<'_, Arc<AppState>>) -> Vec<String> {
    state.settings().service_order
}

#[tauri::command]
pub fn set_service_order(
    state: State<'_, Arc<AppState>>,
    order: Vec<String>,
) -> Result<(), String> {
    state.update_settings(move |s| {
        s.service_order = order;
        Ok(())
    })?;
    Ok(())
}

/// Manually reveals a service whose result was auto-hidden.
#[tauri::command]
pub fn show_service_result(
    state: State<'_, Arc<AppState>>,
    identifier: String,
    window_type: Option<WindowTypeDto>,
) {
    state.show_service_result(window_type.unwrap_or_default().into(), &identifier);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dismissed_screenshot_reports_cancellation_without_requesting_focus() {
        let output = ScreenshotOutput { output: None, restore_window: false };
        assert_eq!(
            serde_json::to_value(output).unwrap(),
            serde_json::json!({ "output": null, "restoreWindow": false })
        );
    }

    #[test]
    fn query_window_and_request_id_survive_the_wire_format() {
        let request: QueryRequest = serde_json::from_value(serde_json::json!({
            "text": "hello", "windowType": "mini", "requestId": "request-2"
        })).unwrap();
        assert_eq!(WindowType::from(request.window_type).label(), "mini");
        assert_eq!(request.request_id, "request-2");
        let payload = serde_json::to_value(QueryFinishedPayload {
            request_id: request.request_id, cancelled: true,
        }).unwrap();
        assert_eq!(payload, serde_json::json!({"requestId": "request-2", "cancelled": true}));
    }

    #[test]
    fn merge_only_touches_present_keys() {
        let mut settings = Settings::default();
        settings.auto_play_audio = true;
        let patch = serde_json::json!({ "appearanceType": "dark" });
        merge_object(&mut settings, &patch).unwrap();
        assert_eq!(settings.appearance_type, crate::config::AppearanceType::Dark);
        // Untouched fields survive.
        assert!(settings.auto_play_audio);
    }

    #[test]
    fn intelligent_query_settings_migrate_patch_and_round_trip_without_losing_other_windows() {
        let mut settings: Settings = serde_json::from_value(serde_json::json!({
            "services": { "Google": { "enabled": true, "apiKey": "keep" } }
        })).unwrap();
        assert!(!settings.intelligent_query_mode_for(WindowType::Main));
        assert_eq!(settings.service("Google").intelligent_query_text_type, QueryTextType::INTELLIGENT);
        merge_settings(&mut settings, &serde_json::json!({
            "intelligentQueryMode": { "main": true, "mini": true },
            "services": { "Google": { "intelligentQueryTextType": 2 } }
        })).unwrap();
        merge_settings(&mut settings, &serde_json::json!({
            "intelligentQueryMode": { "mini": false, "fixed": true }
        })).unwrap();
        let json = serde_json::to_value(&settings).unwrap();
        assert_eq!(json["intelligentQueryMode"], serde_json::json!({"main": true, "mini": false, "fixed": true}));
        assert_eq!(json["services"]["Google"]["intelligentQueryTextType"], 2);
        settings = serde_json::from_value(json).unwrap();
        assert_eq!(settings.service("Google").api_key, "keep");
        assert!(settings.service("Google").translation);
        let before = serde_json::to_value(&settings).unwrap();
        for patch in [
            serde_json::json!({"intelligentQueryMode": {"main": "false"}}),
            serde_json::json!({"intelligentQueryMode": {"ocr": true}}),
            serde_json::json!({"services": {"Google": {"intelligentQueryTextType": -1}}}),
        ] {
            assert!(merge_settings(&mut settings, &patch).is_err());
            assert_eq!(serde_json::to_value(&settings).unwrap(), before);
        }
    }

    #[test]
    fn merge_recurses_into_objects() {
        let mut settings = Settings::default();
        settings.services.insert(
            "OpenAI".into(),
            ServiceConfig {
                api_key: "keep".into(),
                enabled: true,
                ..Default::default()
            },
        );
        let patch = serde_json::json!({ "services": { "OpenAI": { "enabled": false } } });
        merge_object(&mut settings, &patch).unwrap();
        let cfg = settings.service("OpenAI");
        assert!(!cfg.enabled);
        assert_eq!(cfg.api_key, "keep");
    }

    #[test]
    fn merge_rejects_a_malformed_patch() {
        let mut settings = Settings::default();
        let patch = serde_json::json!({ "autoPlayAudio": "not a bool" });
        assert!(merge_object(&mut settings, &patch).is_err());
    }

    #[test]
    fn merge_applies_a_dotted_path() {
        // `lib.rs::upstream_key_to_field` hands `merge_settings` the field
        // *path* `services.Youdao.apiKey` for
        // `easydict://writeKeyValue?EZYoudaoAPIKey=…`.
        let mut settings = Settings::default();
        let patch = serde_json::json!({ "services.Youdao.apiKey": "sk-youdao" });
        merge_settings(&mut settings, &patch).unwrap();
        assert_eq!(settings.service("Youdao").api_key, "sk-youdao");

        // The same path shape reaches a service instance, which the settings
        // schema only reaches through the `services` map.
        let patch = serde_json::json!({ "services.CustomOpenAI#1.name": "Mine" });
        merge_settings(&mut settings, &patch).unwrap();
        assert_eq!(settings.service("CustomOpenAI#1").name, "Mine");

        // A path whose leaf does not fit the field is still an error, not a
        // mangled document.
        let patch = serde_json::json!({ "services.Youdao.apiKey": 7 });
        assert!(merge_settings(&mut settings, &patch).is_err());
        assert_eq!(settings.service("Youdao").api_key, "sk-youdao");
    }

    #[test]
    fn a_failed_merge_leaves_the_settings_alone() {
        let mut settings = Settings::default();
        settings.bing_cookie = "keep".to_string();
        // One entry applies, the second cannot: the whole patch is rejected
        // and nothing is written, so a half-applied document never reaches the
        // settings file.
        let patch = serde_json::json!({ "bingCookie": "new", "httpPort": 8080 });
        assert!(merge_settings(&mut settings, &patch).is_err());
        assert_eq!(settings.bing_cookie, "keep");
        assert_eq!(settings.http_port, "8080");
    }

    #[test]
    fn default_target_flips_away_from_the_first_language() {
        let settings = Settings::default();
        assert_eq!(
            default_target(&settings, settings.first_language),
            settings.second_language
        );
        assert_eq!(
            default_target(&settings, Language::Japanese),
            settings.first_language
        );
    }
}
