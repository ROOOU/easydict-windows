//! Shared application state.
//!
//! Holds the persisted settings, the live service handles, the in-flight query
//! run, and the platform helpers. Everything the Tauri commands touch lives
//! here so the command layer stays a thin translation between JSON and these
//! Rust types.

use std::{collections::HashMap, sync::Arc};

use parking_lot::{Mutex, RwLock};
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{QueryRecord, ServiceConfig, Settings, WindowFrame};
use crate::detect::LanguageDetector;
use crate::model::{Language, QueryModel, QueryTextType, ServiceType, ServiceUsageStatus, WindowType};
use crate::ocr::OcrEngine;
use crate::query::{QueryEvent, QueryRun, ServiceHandle};
use crate::service::registry;
use crate::store::{Store, StoreResult};
use crate::tts::TtsPlayer;

pub struct AppState {
    pub app: AppHandle,
    pub store: Arc<Store>,
    /// The authoritative settings. Cloned out for reads, mutated in place for
    /// writes so the service handles always see a consistent snapshot.
    settings: RwLock<Settings>,
    /// Serialize persistence and its side effects across all windows.
    settings_update: Mutex<()>,
    /// One handle per configured service, rebuilt whenever settings change.
    handles: RwLock<Vec<Arc<ServiceHandle>>>,
    /// The fan-out run currently in flight, if any.
    runs: Mutex<HashMap<WindowType, (String, QueryRun)>>,
    pub tts: TtsPlayer,
    pub ocr: OcrEngine,
    detector: Mutex<LanguageDetector>,
}

impl AppState {
    pub fn new(app: AppHandle, store: Arc<Store>) -> StoreResult<Arc<Self>> {
        let settings = store.load_settings()?;
        let handles = registry::build_all(&settings);
        let detector = Mutex::new(LanguageDetector::new(
            vec![settings.first_language, settings.second_language],
            settings.enable_beta_feature,
        ));

        Ok(Arc::new(AppState {
            app,
            store,
            settings: RwLock::new(settings),
            settings_update: Mutex::new(()),
            handles: RwLock::new(handles),
            runs: Mutex::new(HashMap::new()),
            tts: TtsPlayer::new(),
            ocr: OcrEngine::new(),
            detector,
        }))
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    /// Geometry is not a preference change: retain live services, skip desktop
    /// registrations and do not wake every WebView with a settings broadcast.
    pub(crate) fn save_window_frames(
        &self,
        frames: &HashMap<WindowType, WindowFrame>,
    ) -> Result<(), String> {
        let _update = self.settings_update.lock();
        let mut snapshot = self.settings();
        let mut changed = false;
        for (kind, frame) in frames {
            changed |= snapshot.window_frames.insert(*kind, *frame) != Some(*frame);
        }
        if let Some(main) = frames.get(&WindowType::Main) {
            let (width, height) = (f64::from(main.width), f64::from(main.height));
            changed |= snapshot.main_window_width != width || snapshot.main_window_height != height;
            snapshot.main_window_width = width;
            snapshot.main_window_height = height;
        }
        if changed {
            self.store.save_settings(&snapshot).map_err(|error| error.to_string())?;
            *self.settings.write() = snapshot;
        }
        Ok(())
    }

    /// Mutates the settings, persists them, and rebuilds the service handles.
    ///
    /// Rebuilding on every write is heavier than diffing, but it keeps a
    /// service's cached credentials and model list from going stale — the
    /// failure mode the macOS app avoids by observing individual `Defaults`
    /// keys, which has no direct equivalent here.
    pub fn update_settings(
        &self,
        f: impl FnOnce(&mut Settings) -> Result<(), String>,
    ) -> Result<Settings, String> {
        self.update_settings_with_autostart(f, false)
    }

    pub fn update_settings_with_autostart(
        &self,
        f: impl FnOnce(&mut Settings) -> Result<(), String>,
        apply_autostart: bool,
    ) -> Result<Settings, String> {
        let _update = self.settings_update.lock();
        let mut snapshot = self.settings();
        // Native registration can call the main thread; do not hold a settings
        // write lock while callbacks may need to read the old snapshot.
        commit_settings(&mut snapshot, f, |before, after| {
            crate::desktop::commit(&self.app, before, after, apply_autostart, || {
                self.store.save_settings(after)
                    .map_err(|error| format!("无法保存设置，原设置未更改：{error}"))
            })
        })?;
        *self.settings.write() = snapshot.clone();
        *self.handles.write() = registry::build_all(&snapshot);
        *self.detector.lock() = LanguageDetector::new(
            vec![snapshot.first_language, snapshot.second_language],
            snapshot.enable_beta_feature,
        );
        if let Some(selection) = self.app.try_state::<crate::platform::auto_selection::AutoSelection>() {
            selection.configure(&snapshot);
        }
        crate::tray::refresh(&self.app, self);
        let _ = self.app.emit(crate::commands::events::SETTINGS_CHANGED, &snapshot);
        Ok(snapshot)
    }

    pub fn all_handles(&self) -> Vec<Arc<ServiceHandle>> {
        self.handles.read().clone()
    }

    pub fn handle_for(&self, identifier: &str) -> Option<Arc<ServiceHandle>> {
        self.handles
            .read()
            .iter()
            .find(|h| h.identifier() == identifier)
            .cloned()
    }

    /// The handles that should answer a normal query, in display order.
    ///
    /// Mirrors `ServiceUsageStatus`: `AlwaysOff` hides a service regardless of
    /// its enabled flag, `AlwaysOn` shows it regardless.
    pub fn active_handles(&self) -> Vec<Arc<ServiceHandle>> {
        self.query_handles(None)
    }

    /// Direct OCR/replacement calls pass no model and bypass the window filter.
    pub fn query_handles(&self, model: Option<&QueryModel>) -> Vec<Arc<ServiceHandle>> {
        select_query_handles(&self.settings.read(), &self.handles.read(), model)
    }

    /// Starts a fan-out query, replacing any run already in flight.
    pub fn start_query(
        &self,
        model: QueryModel,
        request_id: String,
        tx: tokio::sync::mpsc::UnboundedSender<QueryEvent>,
    ) {
        let handles = self.query_handles(Some(&model));
        let mut runs = self.runs.lock();
        if let Some((_, run)) = runs.remove(&model.window_type) {
            run.stop();
        }
        let window_type = model.window_type;
        let run = QueryRun::start(handles, model, tx);
        runs.insert(window_type, (request_id, run));
    }

    /// `stopAllService()` — cancels the run and every service's own request.
    pub fn stop_query(&self, window_type: WindowType, request_id: Option<&str>) {
        if let Some((active_id, run)) = self.runs.lock().get(&window_type) {
            if request_id.is_none_or(|request_id| request_id == active_id) {
                run.stop();
            }
        }
    }

    pub fn show_service_result(&self, window_type: WindowType, identifier: &str) {
        if let Some((_, run)) = self.runs.lock().get(&window_type) {
            run.show_service_result(identifier);
        }
    }

    pub fn detect_language(&self, text: &str) -> Language {
        self.detector.lock().detect_language(text)
    }

    /// Records a finished query in the history table.
    pub fn record_history(&self, model: &QueryModel) {
        if model.query_text.trim().is_empty() {
            return;
        }
        let record = QueryRecord {
            query_text: model.query_text.clone(),
            from_language: model.query_from_language,
            to_language: model.query_target_language,
            timestamp: chrono::Utc::now().timestamp_millis(),
            translated_text: None,
        };
        if let Err(err) = self.store.add_history(&record) {
            log::warn!("failed to record query history: {err}");
        }
    }

    /// The window a select-text action should open, from settings.
    pub fn window_type_for_select(&self) -> WindowType {
        self.settings.read().mouse_select_translate_window_type
    }

    /// A handle to the main window, or the first window if it was renamed.
    pub fn main_window(&self) -> Option<tauri::WebviewWindow> {
        self.app.get_webview_window("main")
    }
}

fn select_query_handles(
    settings: &Settings,
    handles: &[Arc<ServiceHandle>],
    model: Option<&QueryModel>,
) -> Vec<Arc<ServiceHandle>> {
    let query_type = model
        .filter(|model| settings.intelligent_query_mode_for(model.window_type))
        .map(|model| {
            // Upstream's automatic filter uses one Chinese character, not the
            // separate dictionary/prompt routing threshold in QueryModel.
            crate::util::analysis::query_type(&model.query_text, model.query_from_language, 1)
        });
    let mut selected: Vec<_> = handles.iter().filter(|handle| {
        let cfg = settings.service(handle.identifier());
        let automatic_types = if handle.service_type().configurable_intelligent_query() {
            cfg.intelligent_query_text_type
        } else {
            QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
        };
        service_is_active(&cfg)
            && !handle.service.supported_query_type().is_empty()
            && query_type.is_none_or(|kind| automatic_types.contains(kind))
    }).cloned().collect();
    selected.sort_by_key(|handle| {
        settings.service_order.iter()
            .position(|id| id == handle.identifier())
            .unwrap_or(usize::MAX)
    });
    selected
}

fn commit_settings(
    current: &mut Settings,
    edit: impl FnOnce(&mut Settings) -> Result<(), String>,
    commit: impl FnOnce(&Settings, &Settings) -> Result<(), String>,
) -> Result<(), String> {
    let mut next = current.clone();
    edit(&mut next)?;
    commit(current, &next)?;
    *current = next;
    Ok(())
}

/// `ServiceUsageStatus` applied to a service's configuration.
pub fn service_is_active(cfg: &ServiceConfig) -> bool {
    match cfg.usage_status {
        ServiceUsageStatus::AlwaysOff => false,
        ServiceUsageStatus::AlwaysOn => true,
        ServiceUsageStatus::Default => cfg.enabled,
    }
}

/// Services that have a working implementation on Windows, in registry order.
pub fn available_service_types() -> Vec<ServiceType> {
    registry::service_types()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::TempDir;

    fn cfg(status: ServiceUsageStatus, enabled: bool) -> ServiceConfig {
        ServiceConfig {
            usage_status: status,
            enabled,
            ..Default::default()
        }
    }

    #[test]
    fn usage_status_overrides_the_enabled_flag() {
        assert!(service_is_active(&cfg(ServiceUsageStatus::Default, true)));
        assert!(!service_is_active(&cfg(ServiceUsageStatus::Default, false)));
        assert!(service_is_active(&cfg(ServiceUsageStatus::AlwaysOn, false)));
        assert!(!service_is_active(&cfg(ServiceUsageStatus::AlwaysOff, true)));
    }

    fn selected(settings: &Settings, window: Option<WindowType>, text: &str, language: Language) -> Vec<String> {
        let mut model = QueryModel::new(text);
        model.window_type = window.unwrap_or(WindowType::Main);
        model.query_from_language = language;
        select_query_handles(settings, &registry::build_all(settings), window.map(|_| &model))
            .iter().map(|handle| handle.identifier().to_string()).collect()
    }

    #[test]
    fn intelligent_query_is_per_window_and_keeps_default_queries_and_order() {
        let mut settings = Settings::default();
        settings.service_order = vec!["DeepL".into(), "Bing".into(), "Google".into()];
        let defaults = vec!["DeepL", "Bing", "Google"];
        for window in [WindowType::Main, WindowType::Mini, WindowType::Fixed] {
            assert_eq!(selected(&settings, Some(window), "apple", Language::English), defaults);
        }
        settings.intelligent_query_mode.insert(WindowType::Mini, true);
        assert_eq!(selected(&settings, Some(WindowType::Mini), "apple", Language::English), vec!["Bing", "Google"]);
        assert_eq!(selected(&settings, Some(WindowType::Main), "apple", Language::English), defaults);
        assert_eq!(selected(&settings, Some(WindowType::Fixed), "apple", Language::English), defaults);
        assert_eq!(selected(&settings, None, "apple", Language::English), defaults);
        for (text, language) in [("Hello world.", Language::English), ("你好", Language::SimplifiedChinese)] {
            assert_eq!(selected(&settings, Some(WindowType::Mini), text, language), defaults);
        }
        assert_eq!(selected(&settings, Some(WindowType::Mini), "好", Language::SimplifiedChinese), vec!["Bing", "Google"]);
    }

    #[test]
    fn intelligent_participation_is_independent_of_output_types_and_instance_ids() {
        let mut settings = Settings::default();
        for cfg in settings.services.values_mut() { cfg.enabled = false; }
        settings.intelligent_query_mode.insert(WindowType::Fixed, true);
        for (id, types) in [
            ("CustomOpenAI#word", QueryTextType::DICTIONARY),
            ("CustomOpenAI#sentence", QueryTextType::SENTENCE),
            ("CustomOpenAI#off", QueryTextType::NONE),
        ] {
            settings.services.insert(id.into(), ServiceConfig {
                enabled: true, intelligent_query_text_type: types, ..Default::default()
            });
        }
        // All three instances still produce translations (not dictionary prompts).
        assert_eq!(selected(&settings, Some(WindowType::Fixed), "apple", Language::English), vec!["CustomOpenAI#word"]);
        assert_eq!(selected(&settings, Some(WindowType::Fixed), "Hello world.", Language::English), vec!["CustomOpenAI#sentence"]);
        assert!(selected(&settings, Some(WindowType::Fixed), "这是一段文本", Language::SimplifiedChinese).is_empty());
        assert_eq!(selected(&settings, None, "apple", Language::English).len(), 3);
        settings.service_mut("CustomOpenAI#word").usage_status = ServiceUsageStatus::AlwaysOff;
        assert!(selected(&settings, Some(WindowType::Fixed), "apple", Language::English).is_empty());
        let cfg = settings.service_mut("CustomOpenAI#word");
        cfg.usage_status = ServiceUsageStatus::AlwaysOn;
        cfg.enabled = false;
        assert_eq!(selected(&settings, Some(WindowType::Fixed), "apple", Language::English), vec!["CustomOpenAI#word"]);
        settings.service_mut("CustomOpenAI#word").translation = false;
        assert!(selected(&settings, Some(WindowType::Fixed), "apple", Language::English).is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn failed_settings_edits_leave_memory_and_disk_unchanged() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = TempDir::new("commit-failure");
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let mut current = Settings::default();
        store.save_settings(&current).unwrap();
        let before = std::fs::read(store.settings_path()).unwrap();

        let persist = |_: &Settings, after: &Settings| store.save_settings(after)
            .map_err(|error| format!("无法保存设置，原设置未更改：{error}"));
        assert!(commit_settings(&mut current, |settings| {
            settings.auto_play_audio = true;
            Err("invalid test patch".into())
        }, persist).is_err());
        assert!(!current.auto_play_audio);

        let lock = std::fs::File::options().read(true).share_mode(0)
            .open(store.settings_path()).unwrap();
        let error = commit_settings(&mut current, |settings| {
            settings.auto_play_audio = true;
            Ok(())
        }, persist).unwrap_err();
        assert!(error.contains("原设置未更改"));
        assert!(!current.auto_play_audio);
        drop(lock);
        assert_eq!(std::fs::read(store.settings_path()).unwrap(), before);
        assert!(!store.load_settings().unwrap().auto_play_audio);
    }

    #[test]
    fn concurrent_patches_merge_into_the_latest_persisted_snapshot() {
        let dir = TempDir::new("concurrent-settings");
        let store = Arc::new(Store::open(dir.path().to_path_buf()).unwrap());
        let current = Arc::new(RwLock::new(Settings::default()));
        let initial = current.read().main_window_width;
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let store = store.clone();
                let current = current.clone();
                scope.spawn(move || {
                    for _ in 0..8 {
                        commit_settings(&mut current.write(), |settings| {
                            settings.main_window_width += 1.0;
                            Ok(())
                        }, |_, after| store.save_settings(after).map_err(|e| e.to_string())).unwrap();
                    }
                });
            }
        });
        assert_eq!(current.read().main_window_width, initial + 32.0);
        assert_eq!(store.load_settings().unwrap().main_window_width, initial + 32.0);
    }
}
