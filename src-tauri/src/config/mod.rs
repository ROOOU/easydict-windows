//! Application configuration.
//!
//! Ported from `Easydict/Swift/Feature/Configuration/Defaults.Keys+Extension.swift`
//! and `ServiceConfigurationKey.swift`.
//!
//! Each field's doc comment quotes the upstream `Defaults` key it came from, so
//! the mapping between the two settings schemas is recoverable. The on-disk
//! document is this port's own JSON shape rather than upstream's `UserDefaults`
//! plist; `lib.rs`'s `easydict://writeKeyValue` handler is what accepts the
//! upstream `EZ*` key names, for the documented settings-import flow.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::model::{
    Language, LanguageDetectOptimize, QueryTextType, ReasoningEffort, ServiceType, ServiceUsageStatus,
    TtsServiceType, WindowType,
};

/// Which window a floating-window action opens, `EZShowWindowPosition`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShowWindowPosition {
    Right,
    Mouse,
    Former,
    Center,
}

impl Default for ShowWindowPosition {
    fn default() -> Self {
        ShowWindowPosition::Right
    }
}

/// A remembered window rectangle, in physical desktop coordinates.
///
/// Upstream stores one `CGRect` per window type —
/// `EZConfiguration_kWindowFrameKey_<raw>`, `Defaults.Keys+Extension.swift:260`
/// — and `EZLayoutManager` seeds a default frame the first time a window is
/// asked for its geometry (`EZLayoutManager.m:53/59/65`). This is that record,
/// as the port's own JSON shape. The origin is the same coordinate space
/// `Window::set_position` takes, so a negative monitor origin round-trips.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowFrame {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowFrame {
    /// Nothing usable is remembered: a zero-sized frame is what upstream reads
    /// as "no stored frame yet" (`CGRectEqualToRect(frame, CGRectZero)`).
    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}

impl ShowWindowPosition {
    /// The spelling [`crate::window::Placement::parse`] expects.
    ///
    /// These enums travel to the frontend as camelCase, and the one place that
    /// turns a stored position back into a placement is `show_window`, which
    /// needs the raw string rather than a `serde_json` round trip.
    pub fn as_wire(self) -> &'static str {
        match self {
            ShowWindowPosition::Right => "right",
            ShowWindowPosition::Mouse => "mouse",
            ShowWindowPosition::Former => "former",
            ShowWindowPosition::Center => "center",
        }
    }
}

/// `AppearanceType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AppearanceType {
    Light,
    Dark,
    FollowSystem,
}

impl Default for AppearanceType {
    fn default() -> Self {
        AppearanceType::FollowSystem
    }
}

/// `DeepLAPIUsagePriority`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeepLApiUsagePriority {
    WebFirst,
    AuthKeyFirst,
    AuthKeyOnly,
}

impl Default for DeepLApiUsagePriority {
    fn default() -> Self {
        DeepLApiUsagePriority::WebFirst
    }
}

impl DeepLApiUsagePriority {
    /// Upstream stores this as the string `"0" | "1" | "2"`.
    pub fn as_str(self) -> &'static str {
        match self {
            DeepLApiUsagePriority::WebFirst => "0",
            DeepLApiUsagePriority::AuthKeyFirst => "1",
            DeepLApiUsagePriority::AuthKeyOnly => "2",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "1" => DeepLApiUsagePriority::AuthKeyFirst,
            "2" => DeepLApiUsagePriority::AuthKeyOnly,
            _ => DeepLApiUsagePriority::WebFirst,
        }
    }
}

/// `ServiceAPIType` — Baidu and Alibaba offer both a web and a signed API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ServiceApiType {
    Web,
    SecretKey,
}

impl Default for ServiceApiType {
    fn default() -> Self {
        ServiceApiType::SecretKey
    }
}

/// Per-service configuration, mirroring `ServiceConfigurationKey`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServiceConfig {
    pub usage_status: ServiceUsageStatus,
    /// Whether the service may answer translation queries.
    pub translation: bool,
    pub dictionary: bool,
    pub sentence: bool,
    /// Automatic participation, independent of the service's output/prompt mode.
    pub intelligent_query_text_type: QueryTextType,

    /// Comma-joined in the UI; stored as a list here.
    pub supported_models: Vec<String>,
    pub valid_models: Vec<String>,
    pub model: String,
    pub api_key: String,
    pub endpoint: String,
    /// Custom display name (`CustomOpenAI` instances).
    pub name: String,

    pub enable_custom_prompt: bool,
    pub system_prompt: String,
    pub user_prompt: String,

    /// Strip `<think>…</think>` from streamed output.
    pub hide_think_tag: bool,
    pub temperature: f64,
    pub enable_streaming: bool,
    pub reasoning_effort: ReasoningEffort,
    pub cli_effort: String,

    /// Whether this service is shown in each window.
    pub enabled: bool,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        ServiceConfig {
            usage_status: ServiceUsageStatus::Default,
            translation: true,
            dictionary: false,
            sentence: false,
            intelligent_query_text_type: QueryTextType::INTELLIGENT,
            supported_models: Vec::new(),
            valid_models: Vec::new(),
            model: String::new(),
            api_key: String::new(),
            endpoint: String::new(),
            name: String::new(),
            enable_custom_prompt: false,
            system_prompt: String::new(),
            user_prompt: String::new(),
            hide_think_tag: true,
            temperature: 0.3,
            enable_streaming: true,
            reasoning_effort: ReasoningEffort::Off,
            cli_effort: String::new(),
            enabled: false,
        }
    }
}

/// A recorded query, used by both History and Favorites.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryRecord {
    pub query_text: String,
    pub from_language: Language,
    pub to_language: Language,
    /// Unix millis.
    pub timestamp: i64,
    /// Present on favorites; absent on history entries.
    #[serde(default)]
    pub translated_text: Option<String>,
}

/// The complete application settings.
///
/// Field names and defaults are ported one-for-one from the upstream
/// `Defaults` schema; the doc comments quote the original key where it differs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    // ---- General -------------------------------------------------------
    /// `EZConfiguration_kFromKey`
    pub query_from_language: Language,
    /// `EZConfiguration_kToKey`
    pub query_to_language: Language,
    /// `EZConfiguration_kFirstLanguageKey`
    pub first_language: Language,
    /// `EZConfiguration_kSecondLanguageKey`
    pub second_language: Language,
    /// `EZConfiguration_kApperanceKey`
    pub appearance_type: AppearanceType,
    /// `EZConfiguration_kTranslationControllerFontKey`
    pub font_size_option_index: u32,
    /// `EZConfiguration_kAutoPlayAudioKey`
    pub auto_play_audio: bool,
    /// `EZConfiguration_kPronunciationKey` — `us` (default) or `uk`.
    pub pronunciation_accent: String,
    /// `EZConfiguration_kDefaultTTSServiceTypeKey`
    pub default_tts_service_type: TtsServiceType,
    /// `EZConfiguration_kPreferYoudaoTTSForEnglishWordKey`
    pub prefer_youdao_tts_for_english_word: bool,
    /// `EZConfiguration_kLanguageDetectOptimizeTypeKey`
    pub language_detect_optimize: LanguageDetectOptimize,
    /// `EZConfiguration_kHideMainWindowKey`
    pub hide_main_window: bool,
    /// `EZConfiguration_kClearInputKey`
    pub clear_query_when_input_translate: bool,
    /// `EZConfiguration_kSelectQueryTextWhenWindowActivate`
    pub select_query_text_when_window_activate: bool,
    /// `EZConfiguration_kEnableMarkdownRendering` — default `true`.
    pub enable_markdown_rendering: bool,
    /// `EZConfiguration_kAutoSelectAllTextFieldText`
    pub auto_select_all_text_field_text: bool,

    // ---- Auto query ----------------------------------------------------
    /// `EZConfiguration_kAutoQuerySelectedTextKey`
    pub auto_query_selected_text: bool,
    /// `EZConfiguration_kAutoQueryPastedTextKey`
    pub auto_query_pasted_text: bool,
    /// `EZConfiguration_kAutoQueryOCTTextKey` (upstream typo preserved)
    pub auto_query_ocr_text: bool,
    /// `EZConfiguration_kAutoQueryWhenTextChangedKey`
    pub auto_query_when_text_changed: bool,

    // ---- Auto copy -----------------------------------------------------
    /// `EZConfiguration_kAutoCopySelectedTextKey`
    pub auto_copy_selected_text: bool,
    /// `EZConfiguration_kAutoCopyOCRTextKey`
    pub auto_copy_ocr_text: bool,
    /// `EZConfiguration_kAutoCopyFirstTranslatedTextKey`
    pub auto_copy_first_translated_text: bool,
    /// `EZConfiguration_kKeepPrevResultKey`
    pub keep_prev_result_when_selected_text_is_empty: bool,

    // ---- Select-text / popup icon --------------------------------------
    /// `EZConfiguration_kAutoSelectTextKey`
    pub auto_show_query_icon: bool,
    /// `EZConfiguration_kAutoShowQueryIconExcludedLanguageKey`
    pub auto_show_query_icon_excluded_language: Language,
    /// `EZConfiguration_kAutoShowQueryIconMinTextLengthKey`, clamped 0…50
    pub auto_show_query_icon_min_text_length: i32,
    /// `EZConfiguration_kClickQueryKey`
    pub click_query: bool,

    // ---- Windows & positioning ----------------------------------------
    /// `EZConfiguration_kShowFixedWindowPositionKey`
    pub fixed_window_position: ShowWindowPosition,
    /// `EZConfiguration_kShowMiniWindowPositionKey`
    pub mini_window_position: ShowWindowPosition,
    /// `EZConfiguration_kMouseSelectTranslateWindowTypeKey`
    pub mouse_select_translate_window_type: WindowType,
    /// `EZConfiguration_kShortcutSelectTranslateWindowTypeKey`
    pub shortcut_select_translate_window_type: WindowType,
    // `WindowConfigurationKey.swift` generates one stored key per window type —
    // `EZConfiguration_<key>_Window<raw>_Key`, with `raw` being main/mini/fixed —
    // so every query window decides for itself whether it shows its input field
    // and its language row. Both default to `true` upstream.
    /// `EZConfiguration_InputFieldCellVisible_Window0_Key`
    pub input_field_cell_visible_main: bool,
    /// `EZConfiguration_InputFieldCellVisible_Window1_Key`
    pub input_field_cell_visible_mini: bool,
    /// `EZConfiguration_InputFieldCellVisible_Window2_Key`
    pub input_field_cell_visible_fixed: bool,
    /// `EZConfiguration_SelectLanguageCellVisible_Window0_Key`
    pub select_language_cell_visible_main: bool,
    /// `EZConfiguration_SelectLanguageCellVisible_Window1_Key`
    pub select_language_cell_visible_mini: bool,
    /// `EZConfiguration_SelectLanguageCellVisible_Window2_Key`
    pub select_language_cell_visible_fixed: bool,
    /// `EZConfiguration_kPinWindowWhenDisplayed`
    pub pin_window_when_displayed: bool,
    /// `maxWindowHeightPercentage`
    pub max_window_height_percentage: i32,
    /// `IntelligentQueryMode-window<raw>` — one entry per query window.
    ///
    /// Upstream stores this as a string in `UserDefaults` under a key built by
    /// `EZConstKey.constkey(_:windowType:)` (`Configuration+Extension.swift:51/65`)
    /// and only ever reads it back through `bool(forKey:)`, so a missing key is
    /// "off" — which is what [`Settings::intelligent_query_mode_for`] answers for
    /// a window that never stored one. `easydict://writeKeyValue?IntelligentQueryMode-window1=1`
    /// is the documented way to set it, so the deep-link importer fills this map.
    #[serde(default)]
    pub intelligent_query_mode: BTreeMap<WindowType, bool>,

    // ---- Query-text processing -----------------------------------------
    /// `EZConfiguration_kAutomaticWordSegmentation`
    pub automatic_word_segmentation: bool,
    /// `EZConfiguration_kAutomaticallyRemoveCodeCommentSymbols`
    pub automatically_remove_code_comment_symbols: bool,
    /// `EZConfiguration_kReplaceNewlineWithSpace`
    pub replace_newline_with_space: bool,
    /// `EZConfiguration_kEnableRemoveBooksExcerptInfo`
    pub enable_remove_books_excerpt_info: bool,
    /// `EZConfiguration_kForceAutoGetSelectedText`
    pub enable_force_get_selected_text: bool,
    /// `EZConfiguration_kEnableOCRTextNormalization` (key: `enableOCRTextNormalization`)
    pub enable_ocr_text_normalization: bool,

    // ---- Menu bar / tray ------------------------------------------------
    /// `EZConfiguration_kHideMenuBarIconKey`
    pub hide_menu_bar_icon: bool,
    /// `EZConfiguration_kSelectedMenuBarIconKey`
    pub selected_menu_bar_icon: String,
    /// `EZConfiguration_kShowGoogleLinkKey`
    pub show_google_quick_link: bool,
    /// `EZConfiguration_kShowEudicLinkKey`
    pub show_eudic_quick_link: bool,
    /// `EZConfiguration_kShowAppleDictionaryLinkKey`. The `dict://` target only
    /// resolves on macOS, but the flag is kept so settings exported from the
    /// macOS app round-trip unchanged.
    pub show_apple_dictionary_quick_link: bool,
    /// `EZConfiguration_kShowSettingQuickLink`
    pub show_quick_action_button: bool,

    // ---- OCR -------------------------------------------------------------
    /// `showOCRMenuItems`
    pub show_ocr_menu_items: bool,
    /// `enableYoudaoOCR`
    pub enable_youdao_ocr: bool,
    /// `isScreenshotTipLayerHidden`
    pub is_screenshot_tip_layer_hidden: bool,

    // ---- Beta / advanced -------------------------------------------------
    /// `EZBetaFeatureKey`
    pub enable_beta_feature: bool,
    /// `enableHTTPServer`
    pub enable_http_server: bool,
    /// `httpPort`
    pub http_port: String,
    /// `EZConfiguration_kAllowAnalyticsKey`
    pub allow_analytics: bool,
    /// `EZConfiguration_kAllowCrashLogKey`
    pub allow_crash_log: bool,

    // ---- Launch ----------------------------------------------------------
    /// Windows-specific; the macOS app uses Sparkle/SMLoginItemSetEnabled.
    pub launch_at_startup: bool,
    /// `EZConfiguration_kFirstLaunch`
    pub first_launch: bool,

    // ---- Service secrets (flat, matching `EZConstKey.h`) ------------------
    /// `EZDeepLAuthKey`
    pub deep_l_auth: String,
    /// `EZDeepLTranslateEndPointKey`
    pub deep_l_translate_end_point: String,
    /// `EZDeepLTranslationAPIKey`
    pub deep_l_translation: DeepLApiUsagePriority,
    /// `EZDeepLWebAppVersionCacheKey` — cached iTunes lookup result.
    pub deep_l_web_app_version_cache: String,
    /// Windows-port-only. Youdao's web translate endpoint is signed with a
    /// static key that upstream vendors in source; it is not shipped here, so
    /// the user supplies their own. See `docs/reverse-engineering/01-overview.md`.
    pub youdao_web_api_key: String,
    /// `EZBingCookieKey`
    pub bing_cookie: String,
    /// `EZNiuTransAPIKey`
    pub niu_trans_api_key: String,
    /// `EZCaiyunToken`
    pub caiyun_token: String,
    /// `EZTencentSecretId`
    pub tencent_secret_id: String,
    /// `EZTencentSecretKey`
    pub tencent_secret_key: String,
    /// `EZAliAccessKeyId`
    pub ali_access_key_id: String,
    /// `EZAliAccessKeySecret`
    pub ali_access_key_secret: String,
    /// `EZAliServiceApiTypeKey`
    pub ali_service_api_type: ServiceApiType,
    /// `EZBaiduAppId`
    pub baidu_app_id: String,
    /// `EZBaiduSecretKey`
    pub baidu_secret_key: String,
    /// `EZBaiduServiceApiTypeKey`
    pub baidu_service_api_type: ServiceApiType,
    /// `EZVolcanoAccessKeyID`
    pub volcano_access_key_id: String,
    /// `EZVolcanoSecretAccessKey`
    pub volcano_secret_access_key: String,
    /// `EZDoubaoAPIKey`
    pub doubao_api_key: String,
    /// `EZDoubaoModelKey`
    pub doubao_model: String,

    // ---- Per-service configuration ---------------------------------------
    /// Keyed by `serviceType` or `serviceType#uuid`.
    pub services: BTreeMap<String, ServiceConfig>,
    /// Display order of services in the results list.
    pub service_order: Vec<String>,

    // ---- Shortcuts --------------------------------------------------------
    /// Action id -> shortcut spec, e.g. `"inputTranslate": "Alt+A"`.
    pub shortcuts: BTreeMap<String, String>,

    // ---- Records ----------------------------------------------------------
    /// `EZConfiguration_kQueryHistory`
    pub query_history: Vec<QueryRecord>,
    /// `EZConfiguration_kFavorites`
    pub favorites: Vec<QueryRecord>,

    // ---- Window geometry ----------------------------------------------------
    /// `EZConfiguration_kWindowFrameKey_<raw>`, one entry per query window
    /// (`main`, `mini`, `fixed`). The settings and OCR windows are excluded:
    ///
    /// * the OCR window is deliberately left centred on the captured region,
    ///   which the native side decides at screenshot time, and
    /// * the settings window is a single normal window whose size belongs to
    ///   the user's own desktop layout rather than to a translation mode.
    ///
    /// A missing entry means "no frame remembered yet", which is what the
    /// window module restores from before the first placement is applied.
    #[serde(default)]
    pub window_frames: BTreeMap<WindowType, WindowFrame>,
    pub main_window_width: f64,
    pub main_window_height: f64,
    pub font_size_ratio_index: u32,
}

impl Default for Settings {
    fn default() -> Self {
        let mut services = BTreeMap::new();
        for ty in ServiceType::all() {
            let mut cfg = ServiceConfig::default();
            cfg.enabled = ty.enabled_by_default();
            if ty.is_streaming_service() {
                cfg.model = String::new();
            }
            services.insert(ty.raw().to_string(), cfg);
        }

        Settings {
            query_from_language: Language::Auto,
            query_to_language: Language::Auto,
            first_language: Language::SimplifiedChinese,
            second_language: Language::English,
            appearance_type: AppearanceType::FollowSystem,
            font_size_option_index: 0,
            auto_play_audio: false,
            pronunciation_accent: "us".to_string(),
            default_tts_service_type: TtsServiceType::Youdao,
            prefer_youdao_tts_for_english_word: true,
            language_detect_optimize: LanguageDetectOptimize::None,
            hide_main_window: true,
            clear_query_when_input_translate: false,
            select_query_text_when_window_activate: false,
            enable_markdown_rendering: true,
            auto_select_all_text_field_text: true,

            auto_query_selected_text: true,
            auto_query_pasted_text: false,
            auto_query_ocr_text: true,
            auto_query_when_text_changed: false,

            auto_copy_selected_text: false,
            auto_copy_ocr_text: false,
            auto_copy_first_translated_text: false,
            keep_prev_result_when_selected_text_is_empty: true,

            auto_show_query_icon: true,
            auto_show_query_icon_excluded_language: Language::SimplifiedChinese,
            auto_show_query_icon_min_text_length: 0,
            click_query: false,

            fixed_window_position: ShowWindowPosition::Right,
            mini_window_position: ShowWindowPosition::Mouse,
            mouse_select_translate_window_type: WindowType::Fixed,
            shortcut_select_translate_window_type: WindowType::Fixed,
            input_field_cell_visible_main: true,
            input_field_cell_visible_mini: true,
            input_field_cell_visible_fixed: true,
            select_language_cell_visible_main: true,
            select_language_cell_visible_mini: true,
            select_language_cell_visible_fixed: true,
            pin_window_when_displayed: false,
            max_window_height_percentage: 100,
            // Nothing has opted in: `intelligentQueryModeForWindowType` reads a
            // key that does not exist yet as `false`.
            intelligent_query_mode: BTreeMap::new(),

            automatic_word_segmentation: true,
            automatically_remove_code_comment_symbols: true,
            replace_newline_with_space: false,
            enable_remove_books_excerpt_info: true,
            enable_force_get_selected_text: true,
            enable_ocr_text_normalization: false,

            hide_menu_bar_icon: false,
            selected_menu_bar_icon: "square_menu_bar_icon".to_string(),
            show_google_quick_link: true,
            show_eudic_quick_link: true,
            show_apple_dictionary_quick_link: true,
            show_quick_action_button: true,

            show_ocr_menu_items: false,
            enable_youdao_ocr: false,
            is_screenshot_tip_layer_hidden: false,

            enable_beta_feature: false,
            enable_http_server: false,
            http_port: "8080".to_string(),
            allow_analytics: true,
            allow_crash_log: true,

            launch_at_startup: false,
            first_launch: true,

            deep_l_auth: String::new(),
            deep_l_translate_end_point: String::new(),
            deep_l_translation: DeepLApiUsagePriority::WebFirst,
            deep_l_web_app_version_cache: String::new(),
            youdao_web_api_key: String::new(),
            bing_cookie: String::new(),
            niu_trans_api_key: String::new(),
            caiyun_token: String::new(),
            tencent_secret_id: String::new(),
            tencent_secret_key: String::new(),
            ali_access_key_id: String::new(),
            ali_access_key_secret: String::new(),
            ali_service_api_type: ServiceApiType::SecretKey,
            baidu_app_id: String::new(),
            baidu_secret_key: String::new(),
            baidu_service_api_type: ServiceApiType::SecretKey,
            volcano_access_key_id: String::new(),
            volcano_secret_access_key: String::new(),
            doubao_api_key: String::new(),
            doubao_model: "doubao-seed-translation-250915".to_string(),

            services,
            service_order: ServiceType::all().iter().map(|t| t.raw().to_string()).collect(),
            shortcuts: default_shortcuts(),

            query_history: Vec::new(),
            favorites: Vec::new(),

            // ---- Window geometry -------------------------------------------------
            // No frame is remembered until a query window has actually been
            // placed and the user moved or resized it; the window module then
            // fills these in. `main_window_width/height` stay the fallback for
            // the very first main window, which is what upstream's
            // `defaultWindowFrameWithType` computes from the screen instead.
            window_frames: BTreeMap::new(),
            main_window_width: 500.0,
            main_window_height: 240.0,
            font_size_ratio_index: 0,
        }
    }
}

impl Settings {
    /// `intelligentQueryModeForWindowType` — whether a window filters its
    /// services by the query's own text type.
    ///
    /// A window with no stored entry answers `false`, which is what upstream
    /// reads from a `UserDefaults` key that was never written.
    pub fn intelligent_query_mode_for(&self, window_type: WindowType) -> bool {
        self.intelligent_query_mode.get(&window_type).copied().unwrap_or(false)
    }

    /// Portable exports omit credentials unless the user explicitly opts in.
    pub fn clear_credentials(&mut self) {
        for secret in [
            &mut self.deep_l_auth,
            &mut self.youdao_web_api_key,
            &mut self.bing_cookie,
            &mut self.niu_trans_api_key,
            &mut self.caiyun_token,
            &mut self.tencent_secret_id,
            &mut self.tencent_secret_key,
            &mut self.ali_access_key_id,
            &mut self.ali_access_key_secret,
            &mut self.baidu_app_id,
            &mut self.baidu_secret_key,
            &mut self.volcano_access_key_id,
            &mut self.volcano_secret_access_key,
            &mut self.doubao_api_key,
        ] {
            secret.clear();
        }
        for service in self.services.values_mut() {
            service.api_key.clear();
        }
    }

    /// Configuration for a service identifier (`Type` or `Type#uuid`),
    /// falling back to a default derived from the `ServiceType`.
    pub fn service(&self, identifier: &str) -> ServiceConfig {
        if let Some(cfg) = self.services.get(identifier) {
            return cfg.clone();
        }
        let (ty, _) = ServiceType::split_identifier(identifier);
        let mut cfg = ServiceConfig::default();
        if let Some(ty) = ty {
            cfg.enabled = ty.enabled_by_default();
        }
        cfg
    }

    pub fn service_mut(&mut self, identifier: &str) -> &mut ServiceConfig {
        self.services
            .entry(identifier.to_string())
            .or_insert_with(ServiceConfig::default)
    }

    /// Resolves a secret, preferring the user's value over any built-in.
    pub fn secret(&self, user_value: &str, builtin: Option<&str>) -> String {
        if !user_value.trim().is_empty() {
            user_value.to_string()
        } else {
            builtin.unwrap_or_default().to_string()
        }
    }

    /// Clamped font ratio, matching `MyConfiguration.fontSizeRatio`.
    pub fn font_size_ratio(&self) -> f64 {
        const SIZES: [f64; 5] = [1.0, 1.1, 1.2, 1.3, 1.4];
        let idx = (self.font_size_option_index as usize).min(SIZES.len() - 1);
        SIZES[idx]
    }

    /// Clamped `autoShowQueryIconMinTextLength` (0…50).
    pub fn clamped_icon_min_length(&self) -> usize {
        self.auto_show_query_icon_min_text_length.clamp(0, 50) as usize
    }
}

/// Default global + in-app shortcuts.
///
/// Global actions are ported from `ShortcutManager+Default.swift` with macOS
/// `⌥`/`⌘` mapped to Windows `Alt`/`Ctrl`. In-app shortcuts keep the same
/// letter keys so muscle memory transfers.
pub fn default_shortcuts() -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    let mut put = |k: &str, v: &str| {
        m.insert(k.to_string(), v.to_string());
    };

    // Global shortcuts.
    put("inputTranslate", "Alt+A");
    put("snipTranslate", "Alt+S");
    put("selectTranslate", "Alt+D");
    put("showMiniWindow", "Alt+F");
    put("silentScreenshotOCR", "Alt+Shift+S");
    put("pasteboardTranslate", "");
    put("translateAndReplace", "");
    put("polishAndReplace", "");
    put("toggleAutoSelectText", "");
    put("screenshotOCR", "");
    put("pasteboardOCR", "");
    put("showOCRWindow", "");

    // In-app shortcuts.
    put("clearInput", "Ctrl+K");
    put("clearAll", "Ctrl+Shift+K");
    put("copy", "Ctrl+Shift+C");
    put("copyFirstResult", "Ctrl+Shift+J");
    put("focus", "Ctrl+I");
    put("play", "Ctrl+S");
    put("retry", "Ctrl+R");
    put("toggle", "Ctrl+T");
    put("pin", "Ctrl+P");
    put("hide", "Ctrl+Y");
    put("increaseFontSize", "Ctrl+=");
    put("decreaseFontSize", "Ctrl+-");
    put("google", "Ctrl+Enter");
    // Upstream's `.eudic` is `⌘⇧Return`, which lands on the same physical key as
    // `⌘Return`; the shift keeps the two distinct here.
    put("eudic", "Ctrl+Shift+Enter");
    put("appleDic", "Ctrl+Shift+D");

    m
}

/// Definition of a bindable action, mirroring `ShortcutAction` + its
/// `ActionConfiguration`.
pub struct ShortcutActionDef {
    /// Stable identifier, also the settings key.
    pub id: &'static str,
    /// Localization key.
    pub title_key: &'static str,
    /// English title fallback.
    pub title: &'static str,
    /// `true` for system-wide hotkeys, `false` for in-window shortcuts.
    pub global: bool,
}

/// Every bindable action, in the order the Settings → Shortcut tab lists them.
pub const SHORTCUT_ACTIONS: &[ShortcutActionDef] = &[
    // Global.
    ShortcutActionDef { id: "inputTranslate", title_key: "menu_input_translate", title: "Input Translate", global: true },
    ShortcutActionDef { id: "snipTranslate", title_key: "menu_screenshot_Translate", title: "Screenshot Translate", global: true },
    ShortcutActionDef { id: "selectTranslate", title_key: "menu_selectWord_Translate", title: "Select Translate", global: true },
    ShortcutActionDef { id: "toggleAutoSelectText", title_key: "shortcut_toggle_auto_select_text", title: "Toggle Select Translate", global: true },
    ShortcutActionDef { id: "showMiniWindow", title_key: "menu_show_mini_window", title: "Show Mini Window", global: true },
    ShortcutActionDef { id: "pasteboardTranslate", title_key: "menu_pasteboard_translate", title: "Pasteboard Translate", global: true },
    ShortcutActionDef { id: "polishAndReplace", title_key: "menu_polish_and_replace", title: "Polish and Replace", global: true },
    ShortcutActionDef { id: "translateAndReplace", title_key: "menu_translate_and_replace", title: "Translate and Replace", global: true },
    ShortcutActionDef { id: "silentScreenshotOCR", title_key: "menu_silent_screenshot_OCR", title: "Silent Screenshot OCR", global: true },
    ShortcutActionDef { id: "screenshotOCR", title_key: "menu_screenshot_OCR", title: "Screenshot OCR", global: true },
    ShortcutActionDef { id: "pasteboardOCR", title_key: "menu_pasteboard_OCR", title: "Pasteboard OCR", global: true },
    ShortcutActionDef { id: "showOCRWindow", title_key: "menu_show_ocr_window", title: "Show OCR Window", global: true },
    // In-app.
    ShortcutActionDef { id: "clearInput", title_key: "shortcut_clear_input", title: "Clear Input", global: false },
    ShortcutActionDef { id: "clearAll", title_key: "shortcut_clear_all", title: "Clear All", global: false },
    ShortcutActionDef { id: "copy", title_key: "shortcut_copy", title: "Copy", global: false },
    ShortcutActionDef { id: "copyFirstResult", title_key: "shortcut_copy_first_translated_text", title: "Copy first translated text", global: false },
    ShortcutActionDef { id: "focus", title_key: "shortcut_focus", title: "Focus", global: false },
    ShortcutActionDef { id: "play", title_key: "shortcut_play", title: "Play", global: false },
    ShortcutActionDef { id: "retry", title_key: "retry", title: "Retry", global: false },
    ShortcutActionDef { id: "toggle", title_key: "toggle_languages", title: "Toggle Languages", global: false },
    ShortcutActionDef { id: "pin", title_key: "pin", title: "Pin", global: false },
    ShortcutActionDef { id: "hide", title_key: "hide", title: "Hide", global: false },
    ShortcutActionDef { id: "increaseFontSize", title_key: "shortcut_increase_font", title: "Increase Font Size", global: false },
    ShortcutActionDef { id: "decreaseFontSize", title_key: "shortcut_decrease_font", title: "Decrease Font Size", global: false },
    ShortcutActionDef { id: "google", title_key: "open_in_google", title: "Open in Google", global: false },
    ShortcutActionDef { id: "eudic", title_key: "open_in_eudic", title: "Open in Eudic", global: false },
    ShortcutActionDef { id: "appleDic", title_key: "open_in_apple_dictionary", title: "Open in Apple Dictionary", global: false },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_position_wire_spelling_is_the_serialized_one() {
        // `show_window` feeds `as_wire()` straight to `window::Placement::parse`,
        // while the frontend and `settings.json` see the camelCase serde form.
        // Two spellings of one value is exactly the kind of pair that drifts, so
        // pin them together.
        for position in [
            ShowWindowPosition::Right,
            ShowWindowPosition::Mouse,
            ShowWindowPosition::Former,
            ShowWindowPosition::Center,
        ] {
            assert_eq!(
                serde_json::to_value(position).unwrap(),
                serde_json::Value::String(position.as_wire().to_string())
            );
            // And the parser the string is handed to must read it back as the
            // same placement — `Placement::parse` answers `Right` for anything
            // it does not recognise, so a misspelling would be invisible.
            let expected = match position {
                ShowWindowPosition::Right => crate::window::Placement::Right,
                ShowWindowPosition::Mouse => crate::window::Placement::Mouse,
                ShowWindowPosition::Former => crate::window::Placement::Former,
                ShowWindowPosition::Center => crate::window::Placement::Center,
            };
            assert_eq!(
                crate::window::Placement::parse(Some(position.as_wire())),
                expected
            );
        }
    }

    #[test]
    fn windows_service_defaults_preserve_upstream_preferences() {
        let s = Settings::default();
        assert!(s.service("Google").enabled);
        assert!(s.service("DeepL").enabled);
        assert!(s.service("Bing").enabled);
        assert!(!s.service("Youdao").enabled);
        assert!(!s.service("BuiltInAI").enabled);
        assert!(!s.service("OpenAI").enabled);

        // Documented defaults.
        assert_eq!(s.hide_main_window, true);
        assert_eq!(s.auto_show_query_icon, true);
        assert_eq!(s.click_query, false);
        assert_eq!(s.enable_markdown_rendering, true);
        assert_eq!(s.auto_play_audio, false);
        assert_eq!(s.default_tts_service_type, TtsServiceType::Youdao);
        assert_eq!(s.fixed_window_position, ShowWindowPosition::Right);
        assert_eq!(s.mini_window_position, ShowWindowPosition::Mouse);
        assert_eq!(s.mouse_select_translate_window_type, WindowType::Fixed);
        assert_eq!(s.doubao_model, "doubao-seed-translation-250915");
        assert_eq!(s.temperature_default(), 0.3);
    }

    #[test]
    fn per_window_switches_keep_their_upstream_wire_names_and_defaults() {
        // `app/src/hooks/windowSettings.ts` indexes the settings document with
        // these exact camelCase names, and `AdvancedTab` writes them back by the
        // same names. A rename on either side would leave a switch that still
        // saves but no longer hides anything, so pin the pair here.
        let window_keys = [
            "inputFieldCellVisibleMain",
            "inputFieldCellVisibleMini",
            "inputFieldCellVisibleFixed",
            "selectLanguageCellVisibleMain",
            "selectLanguageCellVisibleMini",
            "selectLanguageCellVisibleFixed",
        ];

        let document = serde_json::to_value(Settings::default()).unwrap();
        for key in window_keys {
            // Upstream defaults both switches to `true` for every window type.
            assert_eq!(document.get(key), Some(&serde_json::Value::Bool(true)), "{key}");
        }

        // A document that predates these keys still opens with visible parts,
        // because the whole struct falls back to `Settings::default()`.
        let restored: Settings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(restored.input_field_cell_visible_main);
        assert!(restored.input_field_cell_visible_mini);
        assert!(restored.input_field_cell_visible_fixed);
        assert!(restored.select_language_cell_visible_main);
        assert!(restored.select_language_cell_visible_mini);
        assert!(restored.select_language_cell_visible_fixed);

        // And an explicit `false` — what the settings switch writes — survives
        // the round trip instead of being replaced by the default.
        let patched: Settings = serde_json::from_value(serde_json::json!({
            "inputFieldCellVisibleMini": false,
            "selectLanguageCellVisibleFixed": false,
        }))
        .unwrap();
        assert!(!patched.input_field_cell_visible_mini);
        assert!(!patched.select_language_cell_visible_fixed);
        assert!(patched.input_field_cell_visible_main);
        assert!(patched.select_language_cell_visible_main);
    }

    #[test]
    fn shortcut_defaults_match_upstream() {
        let m = default_shortcuts();
        assert_eq!(m.get("inputTranslate").map(String::as_str), Some("Alt+A"));
        assert_eq!(m.get("selectTranslate").map(String::as_str), Some("Alt+D"));
        assert_eq!(m.get("snipTranslate").map(String::as_str), Some("Alt+S"));
        assert_eq!(m.get("silentScreenshotOCR").map(String::as_str), Some("Alt+Shift+S"));
        assert_eq!(m.get("showMiniWindow").map(String::as_str), Some("Alt+F"));
        assert_eq!(m.get("clearInput").map(String::as_str), Some("Ctrl+K"));
        assert_eq!(m.get("retry").map(String::as_str), Some("Ctrl+R"));
    }

    #[test]
    fn font_ratio_clamped() {
        let mut s = Settings::default();
        s.font_size_option_index = 99;
        assert_eq!(s.font_size_ratio(), 1.4);
    }

    #[test]
    fn icon_min_length_clamped() {
        let mut s = Settings::default();
        s.auto_show_query_icon_min_text_length = 900;
        assert_eq!(s.clamped_icon_min_length(), 50);
        s.auto_show_query_icon_min_text_length = -5;
        assert_eq!(s.clamped_icon_min_length(), 0);
    }
}

impl Settings {
    /// Test helper mirroring `ServiceConfig::default().temperature`.
    #[cfg(test)]
    fn temperature_default(&self) -> f64 {
        ServiceConfig::default().temperature
    }
}
