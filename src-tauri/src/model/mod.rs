//! Core domain models shared by every service.

pub mod language;
pub mod result;

pub use language::{
    app_language_is_chinese, selectable_languages, set_app_language_is_chinese, Language,
    LanguageInfo,
};
pub use result::{
    abbreviate_part, convert_result_to_traditional, ErrorType, QueryError, QueryResult,
    ResultState, TranslateExchange, TranslatePart, TranslateSimpleWord, TranslateWordResult,
    WordPhonetic,
};

use serde::{Deserialize, Serialize};

use crate::config::Settings;
use crate::detect::LanguageDetector;
use crate::util::input_text::{self, InputTextOptions};

/// `ServiceType` — the raw strings match `EZEnumTypes.m` so that persisted
/// settings stay compatible with the macOS app.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ServiceType {
    AppleDictionary,
    MDict,
    Youdao,
    OpenAI,
    DeepSeek,
    Groq,
    Zhipu,
    MiniMax,
    GitHub,
    BuiltInAI,
    ClaudeCode,
    CodexCLI,
    GitHubCopilot,
    Gemini,
    Claude,
    Ollama,
    Polishing,
    Summary,
    CustomOpenAI,
    DeepL,
    Google,
    Apple,
    Baidu,
    Bing,
    Volcano,
    NiuTrans,
    Caiyun,
    Tencent,
    Alibaba,
    Doubao,
    /// Windows-only: uses the built-in SAPI/OneCore speech synthesiser.
    /// Not present in the macOS app, which uses `NSSpeechSynthesizer`.
    SystemSpeech,
}

impl ServiceType {
    pub const fn raw(self) -> &'static str {
        use ServiceType::*;
        match self {
            AppleDictionary => "AppleDictionary",
            MDict => "MDict",
            Youdao => "Youdao",
            OpenAI => "OpenAI",
            DeepSeek => "DeepSeek",
            Groq => "Groq",
            Zhipu => "Zhipu",
            MiniMax => "MiniMax",
            GitHub => "GitHub",
            BuiltInAI => "BuiltInAI",
            ClaudeCode => "ClaudeCode",
            CodexCLI => "CodexCLI",
            GitHubCopilot => "GitHubCopilot",
            Gemini => "Gemini",
            Claude => "Claude",
            Ollama => "Ollama",
            Polishing => "Polishing",
            Summary => "Summary",
            CustomOpenAI => "CustomOpenAI",
            DeepL => "DeepL",
            Google => "Google",
            Apple => "Apple",
            Baidu => "Baidu",
            Bing => "Bing",
            Volcano => "Volcano",
            NiuTrans => "NiuTrans",
            Caiyun => "Caiyun",
            Tencent => "Tencent",
            Alibaba => "Alibaba",
            Doubao => "Doubao",
            SystemSpeech => "SystemSpeech",
        }
    }

    pub fn from_raw(s: &str) -> Option<ServiceType> {
        ServiceType::all().iter().copied().find(|t| t.raw() == s)
    }

    /// Splits `"CustomOpenAI#<uuid>"` into `(type, Some(uuid))`.
    pub fn split_identifier(identifier: &str) -> (Option<ServiceType>, Option<String>) {
        match identifier.split_once('#') {
            Some((ty, uuid)) => (ServiceType::from_raw(ty), Some(uuid.to_string())),
            None => (ServiceType::from_raw(identifier), None),
        }
    }

    /// Registration order, matching `QueryServiceFactory.serviceRegistrations`.
    /// This is the order services appear in the settings list and in the
    /// result stack before the user reorders them.
    pub const fn all() -> &'static [ServiceType] {
        use ServiceType::*;
        &[
            AppleDictionary,
            MDict,
            Youdao,
            OpenAI,
            DeepSeek,
            Groq,
            Zhipu,
            MiniMax,
            GitHub,
            BuiltInAI,
            ClaudeCode,
            CodexCLI,
            GitHubCopilot,
            Gemini,
            Claude,
            Ollama,
            Polishing,
            Summary,
            CustomOpenAI,
            DeepL,
            Google,
            Apple,
            Baidu,
            Bing,
            Volcano,
            NiuTrans,
            Caiyun,
            Tencent,
            Alibaba,
            Doubao,
        ]
    }

    /// True when this service can run on Windows.
    ///
    /// `AppleDictionary` wraps the private `DictionaryServices.framework`;
    /// `Apple` wraps the macOS system translate UI. Neither has a Windows
    /// equivalent, so they stay in the type list (settings files round-trip)
    /// but never appear in the registry here.
    pub const fn is_available_on_windows(self) -> bool {
        !matches!(self, ServiceType::AppleDictionary | ServiceType::Apple)
    }

    /// Localization key for the service's display name, from
    /// `QueryServiceFactory`.
    pub const fn name_key(self) -> &'static str {
        use ServiceType::*;
        match self {
            AppleDictionary => "apple_dictionary",
            MDict => "service.mdict.name",
            Youdao => "youdao_dict",
            OpenAI => "openai_translate",
            DeepSeek => "deepseek_translate",
            Groq => "groq_translate",
            Zhipu => "zhipu_translate",
            MiniMax => "minimax_translate",
            GitHub => "github_models",
            BuiltInAI => "built_in_ai",
            ClaudeCode => "service.claude_code.name",
            CodexCLI => "service.codex_cli.name",
            GitHubCopilot => "service.github_copilot.name",
            Gemini => "gemini_translate",
            Claude => "claude_translate",
            Ollama => "ollama_translate",
            Polishing => "polishing_service",
            Summary => "summary_service",
            CustomOpenAI => "custom_openai",
            DeepL => "deepL_translate",
            Google => "google_translate",
            Apple => "apple_translate",
            Baidu => "baidu_translate",
            Bing => "bing_translate",
            Volcano => "volcano_translate",
            NiuTrans => "niuTrans_translate",
            Caiyun => "caiyun_translate",
            Tencent => "tencent_translate",
            Alibaba => "ali_translate",
            Doubao => "doubao_translate",
            SystemSpeech => "system_speech",
        }
    }

    /// Default English display name (fallback when no localization is loaded).
    pub const fn default_name(self) -> &'static str {
        use ServiceType::*;
        match self {
            AppleDictionary => "Apple Dictionary",
            MDict => "MDict",
            Youdao => "Youdao Dictionary",
            OpenAI => "OpenAI Translate",
            DeepSeek => "DeepSeek Translate",
            Groq => "Groq Translate",
            Zhipu => "Zhipu Translate",
            MiniMax => "MiniMax Translate",
            GitHub => "GitHub Models",
            BuiltInAI => "Built-In AI Translate",
            ClaudeCode => "Claude Code",
            CodexCLI => "Codex",
            GitHubCopilot => "GitHub Copilot",
            Gemini => "Gemini Translate",
            Claude => "Claude Translate",
            Ollama => "Ollama Translate",
            Polishing => "Polishing",
            Summary => "Summary",
            CustomOpenAI => "Custom OpenAI",
            DeepL => "DeepL Translate",
            Google => "Google Translate",
            Apple => "Apple Translate",
            Baidu => "Baidu Translate",
            Bing => "Bing Translate",
            Volcano => "Volcano Translate",
            NiuTrans => "NiuTrans",
            Caiyun => "LingoCloud",
            Tencent => "Tencent Translate",
            Alibaba => "Ali Translate",
            Doubao => "Doubao Translate",
            SystemSpeech => "System Speech",
        }
    }

    /// `ServiceAPIKeyRequirement`, from `QueryServiceFactory`.
    pub const fn api_key_requirement(self) -> ApiKeyRequirement {
        use ServiceType::*;
        match self {
            AppleDictionary | MDict | DeepL | Google | Apple | Bing | Ollama
            | SystemSpeech => ApiKeyRequirement::None,
            NiuTrans | Caiyun | BuiltInAI | Polishing | Summary => ApiKeyRequirement::BuiltIn,
            ClaudeCode | CodexCLI | GitHubCopilot => ApiKeyRequirement::AgentCli,
            _ => ApiKeyRequirement::UserProvided,
        }
    }

    /// Whether this service emits incremental results.
    pub const fn is_streaming_service(self) -> bool {
        use ServiceType::*;
        matches!(
            self,
            OpenAI
                | CustomOpenAI
                | BuiltInAI
                | Gemini
                | Claude
                | DeepSeek
                | Groq
                | Zhipu
                | MiniMax
                | GitHub
                | Ollama
                | Doubao
                | ClaudeCode
                | CodexCLI
                | GitHubCopilot
                | Polishing
                | Summary
        )
    }

    /// Services overriding upstream's `intelligentQueryTextType`.
    pub const fn configurable_intelligent_query(self) -> bool {
        self.is_streaming_service()
            || matches!(self, Self::Google | Self::Bing | Self::Baidu | Self::Youdao)
    }

    /// Services that support multiple configured instances.
    pub const fn allows_multiple_instances(self) -> bool {
        matches!(self, ServiceType::CustomOpenAI)
    }

    /// Shipped by default on a fresh install — Youdao, DeepL and Built-In AI,
    /// per `docs/user-docs/en/SERVICES.md`.
    pub const fn enabled_by_default(self) -> bool {
        matches!(
            self,
            ServiceType::Google | ServiceType::DeepL | ServiceType::Bing
        )
    }
}

impl std::fmt::Display for ServiceType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.raw())
    }
}

/// `ServiceAPIKeyRequirement`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApiKeyRequirement {
    None,
    BuiltIn,
    UserProvided,
    AgentCli,
}

impl ApiKeyRequirement {
    pub const fn needs_user_provided_key(self) -> bool {
        matches!(self, ApiKeyRequirement::UserProvided)
    }
}

/// `EZQueryTextType` — an option set describing what a service can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryTextType(pub u8);

impl QueryTextType {
    pub const NONE: QueryTextType = QueryTextType(0);
    pub const TRANSLATION: QueryTextType = QueryTextType(1);
    pub const DICTIONARY: QueryTextType = QueryTextType(2);
    pub const SENTENCE: QueryTextType = QueryTextType(4);
    /// `EZQueryTextTypeIntelligent` — all three.
    pub const INTELLIGENT: QueryTextType = QueryTextType(7);

    pub const fn contains(self, other: QueryTextType) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn union(self, other: QueryTextType) -> QueryTextType {
        QueryTextType(self.0 | other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn from_bits(bits: u8) -> QueryTextType {
        QueryTextType(bits & 7)
    }
}

impl Default for QueryTextType {
    fn default() -> Self {
        QueryTextType::TRANSLATION
    }
}

/// `EZWindowType` — which of the three query windows owns a query.
///
/// The ordering is the upstream raw-value order (`main` 0, `mini` 1,
/// `fixed` 2), which is what `Settings::window_frames` is sorted by and what
/// `raw()` is used for elsewhere — keep the variants in that order and the
/// derive stays correct.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WindowType {
    Main,
    Mini,
    Fixed,
}

impl WindowType {
    pub const fn label(self) -> &'static str {
        match self {
            WindowType::Main => "main",
            WindowType::Mini => "mini",
            WindowType::Fixed => "fixed",
        }
    }

    pub const fn raw(self) -> i32 {
        match self {
            WindowType::Main => 0,
            WindowType::Mini => 1,
            WindowType::Fixed => 2,
        }
    }

    /// The inverse of [`WindowType::raw`], for the upstream spellings that carry
    /// the numeric raw value — `EZConstKey.constkey(_:windowType:)` builds
    /// `IntelligentQueryMode-window1`.
    pub const fn from_raw(raw: i32) -> Option<WindowType> {
        match raw {
            0 => Some(WindowType::Main),
            1 => Some(WindowType::Mini),
            2 => Some(WindowType::Fixed),
            _ => None,
        }
    }

    pub const fn name_key(self) -> &'static str {
        match self {
            WindowType::Main => "main_window",
            WindowType::Mini => "mini_window",
            WindowType::Fixed => "fixed_window",
        }
    }
}

/// `EZActionType` — why a query was started; drives auto-copy and history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    InputQuery,
    AutoSelectQuery,
    ShortcutQuery,
    InvokeQuery,
    OcrQuery,
    /// `EZActionTypeScreenshotOCR` — the *silent* screenshot path.
    SilentScreenshotOcr,
    PasteboardTranslate,
}

impl Default for ActionType {
    fn default() -> Self {
        ActionType::InputQuery
    }
}

/// `TTSServiceType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TtsServiceType {
    Youdao,
    Bing,
    Google,
    Baidu,
    /// Windows equivalent of macOS's `Apple` (NSSpeechSynthesizer).
    System,
}

impl TtsServiceType {
    pub fn raw(self) -> &'static str {
        match self {
            TtsServiceType::Youdao => "Youdao",
            TtsServiceType::Bing => "Bing",
            TtsServiceType::Google => "Google",
            TtsServiceType::Baidu => "Baidu",
            TtsServiceType::System => "Apple",
        }
    }

    pub fn from_raw(s: &str) -> Option<TtsServiceType> {
        match s {
            "Youdao" => Some(TtsServiceType::Youdao),
            "Bing" => Some(TtsServiceType::Bing),
            "Google" => Some(TtsServiceType::Google),
            "Baidu" => Some(TtsServiceType::Baidu),
            "Apple" | "System" => Some(TtsServiceType::System),
            _ => None,
        }
    }
}

/// `LanguageDetectOptimize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum LanguageDetectOptimize {
    None,
    Baidu,
    Google,
}

/// `ReasoningEffort` for LLM services.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReasoningEffort {
    Off,
    High,
    Max,
}

impl ReasoningEffort {
    pub const fn is_enabled(self) -> bool {
        !matches!(self, ReasoningEffort::Off)
    }

    pub const fn request_value(self) -> Option<&'static str> {
        match self {
            ReasoningEffort::Off => None,
            ReasoningEffort::High => Some("high"),
            ReasoningEffort::Max => Some("max"),
        }
    }
}

impl Default for ReasoningEffort {
    fn default() -> Self {
        ReasoningEffort::Off
    }
}

/// `ServiceUsageStatus` — whether a service participates in queries.
///
/// The raw values are `"0"`, `"1"`, `"2"` upstream, so the serialised form keeps
/// them for settings compatibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceUsageStatus {
    #[serde(rename = "0")]
    Default,
    #[serde(rename = "1")]
    AlwaysOff,
    #[serde(rename = "2")]
    AlwaysOn,
}

impl ServiceUsageStatus {
    pub const fn raw(self) -> &'static str {
        match self {
            ServiceUsageStatus::Default => "0",
            ServiceUsageStatus::AlwaysOff => "1",
            ServiceUsageStatus::AlwaysOn => "2",
        }
    }

    pub fn from_raw(s: &str) -> ServiceUsageStatus {
        match s {
            "1" => ServiceUsageStatus::AlwaysOff,
            "2" => ServiceUsageStatus::AlwaysOn,
            _ => ServiceUsageStatus::Default,
        }
    }

    /// `service.configuration.openai.usage_status_*.title`
    pub const fn title_key(self) -> &'static str {
        match self {
            ServiceUsageStatus::Default => {
                "service.configuration.openai.usage_status_default.title"
            }
            ServiceUsageStatus::AlwaysOff => {
                "service.configuration.openai.usage_status_always_off.title"
            }
            ServiceUsageStatus::AlwaysOn => {
                "service.configuration.openai.usage_status_always_on.title"
            }
        }
    }

    /// `EZQueryService.enabledAutoQuery` rejects `alwaysOff` outright.
    pub const fn allows_auto_query(self) -> bool {
        !matches!(self, ServiceUsageStatus::AlwaysOff)
    }
}

impl Default for ServiceUsageStatus {
    fn default() -> Self {
        ServiceUsageStatus::Default
    }
}

/// A single query, resolved — `QueryModel`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryModel {
    /// Query text after `handlingInputText()` — what the services actually see.
    pub query_text: String,
    /// Raw input text before handling.
    pub input_text: String,
    pub user_source_language: Language,
    pub user_target_language: Language,
    pub detected_language: Language,
    /// `userSourceLanguage` when set, otherwise `detectedLanguage`.
    pub query_from_language: Language,
    /// `userTargetLanguage` when set, otherwise the auto-derived target.
    pub query_target_language: Language,
    pub need_detect_language: bool,
    pub action_type: ActionType,
    pub window_type: WindowType,
    /// Which of translation/dictionary/sentence the caller wants.
    pub query_type: QueryTextType,
    /// True when the source language was auto-detected (drives the "Detected:"
    /// label in the UI).
    pub show_auto_language: bool,
}

impl QueryModel {
    /// A model built without settings: the query text is only normalised by
    /// `normalize_input_text`.
    ///
    /// The query path builds its models with `new_with_settings`, which runs the
    /// full `String.handlingInputText()` pipeline.
    pub fn new(text: impl Into<String>) -> Self {
        let input = text.into();
        let query_text = normalize_input_text(&input);
        Self::from_parts(input, query_text)
    }

    /// The configured constructor — upstream's `inputText` *setter*
    /// (`Easydict/Swift/Model/QueryModel.swift:99`), where the query text is the
    /// input text run through `String.handlingInputText()`
    /// (`String+HandleInputText.swift:178`) under the three query-text settings
    /// (`Defaults.Keys+Extension.swift:139-149`).
    ///
    /// The detector is built from the same settings fields the app's own detector
    /// is built from (`AppState::new`); upstream detects inside the comment-join
    /// heuristic via `AppleService.shared.detectTextSync`
    /// (`AppleService.swift:133`).
    pub fn new_with_settings(text: impl Into<String>, settings: &Settings) -> Self {
        let input = text.into();
        let mut detector = LanguageDetector::new(
            vec![settings.first_language, settings.second_language],
            settings.enable_beta_feature,
        );
        let options = InputTextOptions {
            automatic_word_segmentation: settings.automatic_word_segmentation,
            automatically_remove_code_comment_symbols: settings
                .automatically_remove_code_comment_symbols,
            replace_newline_with_space: settings.replace_newline_with_space,
        };
        let query_text = input_text::handling_input_text(&input, &options, &mut |t: &str| {
            detector.detect_language(t)
        });
        Self::from_parts(input, query_text)
    }

    fn from_parts(input_text: String, query_text: String) -> Self {
        QueryModel {
            query_text,
            input_text,
            user_source_language: Language::Auto,
            user_target_language: Language::Auto,
            detected_language: Language::Auto,
            query_from_language: Language::Auto,
            query_target_language: Language::Auto,
            need_detect_language: true,
            action_type: ActionType::default(),
            window_type: WindowType::Main,
            query_type: QueryTextType::TRANSLATION,
            show_auto_language: false,
        }
    }

    pub fn has_query_from_language(&self) -> bool {
        !self.query_from_language.is_auto()
    }
}

/// Input normalisation for models built without settings: trim, and drop the
/// zero-width characters pasteboard text tends to carry.
///
/// This is not `handlingInputText` — that lives in
/// `crate::util::input_text::handling_input_text` and is what the query path
/// runs (`QueryModel::new_with_settings`).
pub fn normalize_input_text(text: &str) -> String {
    let mut s = text.trim().to_string();
    // Remove zero-width and other invisible characters.
    s.retain(|c| !matches!(c, '\u{200B}' | '\u{200C}' | '\u{200D}' | '\u{FEFF}'));
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_type_identifier_split() {
        let (t, uuid) = ServiceType::split_identifier("CustomOpenAI#abc-123");
        assert_eq!(t, Some(ServiceType::CustomOpenAI));
        assert_eq!(uuid.as_deref(), Some("abc-123"));
        let (t, uuid) = ServiceType::split_identifier("OpenAI");
        assert_eq!(t, Some(ServiceType::OpenAI));
        assert_eq!(uuid, None);
    }

    #[test]
    fn query_text_type_bits() {
        assert!(QueryTextType::INTELLIGENT.contains(QueryTextType::DICTIONARY));
        assert_eq!(QueryTextType::from_bits(3).0, 3);
        assert!(QueryTextType::NONE.is_empty());
    }

    #[test]
    fn query_text_is_the_handled_input_text() {
        // `QueryModel.inputText` setter: `//` is stripped and the identifier is
        // segmented on the way in.
        let model = QueryModel::new_with_settings("//key_value", &Settings::default());
        assert_eq!(model.query_text, "key value");
        assert_eq!(model.input_text, "//key_value");

        // The settings each still gate their stage.
        let settings = Settings {
            automatic_word_segmentation: false,
            ..Settings::default()
        };
        assert_eq!(
            QueryModel::new_with_settings("//key_value", &settings).query_text,
            "key_value"
        );
        let settings = Settings {
            automatically_remove_code_comment_symbols: false,
            ..Settings::default()
        };
        // The identifier is still segmented; only the comment stage is off.
        assert_eq!(
            QueryModel::new_with_settings("//key_value", &settings).query_text,
            "//key value"
        );

        // Without settings only the light normalisation runs — comment symbols
        // and identifiers are left as typed.
        assert_eq!(QueryModel::new("//key_value").query_text, "//key_value");
    }

    #[test]
    fn windows_defaults_do_not_require_bundled_keys() {
        assert!(ServiceType::Google.enabled_by_default());
        assert!(ServiceType::DeepL.enabled_by_default());
        assert!(ServiceType::Bing.enabled_by_default());
        assert!(!ServiceType::Youdao.enabled_by_default());
        assert!(!ServiceType::BuiltInAI.enabled_by_default());
        assert_eq!(ServiceType::Youdao.api_key_requirement(), ApiKeyRequirement::UserProvided);
        assert!(!ServiceType::OpenAI.enabled_by_default());
        assert_eq!(
            ServiceType::Ollama.api_key_requirement(),
            ApiKeyRequirement::None
        );
        assert_eq!(
            ServiceType::NiuTrans.api_key_requirement(),
            ApiKeyRequirement::BuiltIn
        );
        assert_eq!(
            ServiceType::ClaudeCode.api_key_requirement(),
            ApiKeyRequirement::AgentCli
        );
    }
}
