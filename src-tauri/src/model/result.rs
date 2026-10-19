//! Result model, ported from `Easydict/Swift/Service/Model/QueryResult.swift`.
//!
//! `QueryResult` is the single shape every service populates. The macOS app
//! mutates one shared instance per service; here each service owns one and the
//! frontend receives a serialized snapshot.

use serde::{Deserialize, Serialize};

use super::language::Language;

/// One pronunciation entry — `EZWordPhonetic`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WordPhonetic {
    pub word: String,
    pub language: Language,
    /// The phonetic text, e.g. `ɡʊd`.
    pub value: Option<String>,
    pub speak_url: Option<String>,
    /// Localized label: "美式发音" / "英式发音" / "中文发音".
    pub name: Option<String>,
    /// `us` | `uk`.
    pub accent: Option<String>,
}

/// One part-of-speech group — `EZTranslatePart`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslatePart {
    pub means: Vec<String>,
    /// Abbreviated part of speech, e.g. `adj.`.
    pub part: Option<String>,
}

impl TranslatePart {
    pub fn new(part: impl Into<String>, means: Vec<String>) -> Self {
        let part = part.into();
        TranslatePart {
            means,
            part: Some(abbreviate_part(&part)),
        }
    }
}

/// A word-form group (plural / past tense / …) — `EZTranslateExchange`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateExchange {
    pub name: String,
    pub words: Vec<String>,
}

/// A simple word entry, used for Chinese->English and web translations —
/// `EZTranslateSimpleWord`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateSimpleWord {
    pub word: String,
    pub part: Option<String>,
    pub means: Option<Vec<String>>,
    /// `means.joined("; ")`
    pub means_text: String,
    /// When true the UI prefixes `means_text` with `"<part>  "`.
    pub show_part_means: bool,
}

impl TranslateSimpleWord {
    pub fn new(word: impl Into<String>, means: Vec<String>, part: Option<String>) -> Self {
        let means_text = means.join("; ");
        TranslateSimpleWord {
            word: word.into(),
            part: part.map(|p| abbreviate_part(&p)),
            means: Some(means),
            means_text,
            show_part_means: false,
        }
    }
}

/// The structured dictionary result — `EZTranslateWordResult`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslateWordResult {
    pub phonetics: Option<Vec<WordPhonetic>>,
    pub parts: Option<Vec<TranslatePart>>,
    pub exchanges: Option<Vec<TranslateExchange>>,
    pub simple_words: Option<Vec<TranslateSimpleWord>>,
    pub tags: Option<Vec<String>>,
    pub etymology: Option<String>,
    pub synonyms: Option<Vec<TranslatePart>>,
    pub antonyms: Option<Vec<TranslatePart>>,
    pub collocation: Option<Vec<TranslatePart>>,
}

impl TranslateWordResult {
    /// True when the result has anything worth rendering.
    pub fn is_empty(&self) -> bool {
        self.parts.is_none()
            && self.simple_words.is_none()
            && self.phonetics.is_none()
            && self.exchanges.is_none()
            && self.etymology.is_none()
            && self.synonyms.is_none()
            && self.antonyms.is_none()
            && self.collocation.is_none()
    }
}

/// Error kinds — `QueryError.ErrorType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorType {
    Unknown,
    Api,
    Parameter,
    UnsupportedLanguage,
    MissingSecretKey,
    NoResult,
    Timeout,
    UnsupportedQueryType,
    UnsupportedServiceType,
    ContentTypeMismatch,
}

impl ErrorType {
    /// English description used in `localizedDescription`, matching the
    /// localized strings shipped with Easydict.
    pub const fn description(self) -> &'static str {
        match self {
            ErrorType::Unknown => "Unknown error",
            ErrorType::Api => "API error",
            ErrorType::Parameter => "Parameter error",
            ErrorType::UnsupportedLanguage => "Unsupported language",
            ErrorType::MissingSecretKey => "Missing secret key",
            ErrorType::NoResult => "No result",
            ErrorType::Timeout => "Request timeout",
            ErrorType::UnsupportedQueryType => "Unsupported query type",
            ErrorType::UnsupportedServiceType => "Unsupported service type",
            ErrorType::ContentTypeMismatch => "Content type mismatch",
        }
    }

    /// `isWarningErrorType` — these render as warnings rather than hard errors.
    pub const fn is_warning(self) -> bool {
        matches!(
            self,
            ErrorType::UnsupportedLanguage | ErrorType::NoResult
        )
    }
}

/// A query failure — `QueryError`.
///
/// `Display` reproduces the original format:
/// `"\u{200B}<query_failed>, <type>: <message>\n\n<errorDataMessage>"`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryError {
    #[serde(rename = "type")]
    pub kind: ErrorType,
    pub message: Option<String>,
    pub error_data_message: Option<String>,
    /// Optional "learn more" link shown alongside the message.
    pub prompt_title: Option<String>,
    pub prompt_url: Option<String>,
}

impl QueryError {
    pub fn new(kind: ErrorType) -> Self {
        QueryError {
            kind,
            message: None,
            error_data_message: None,
            prompt_title: None,
            prompt_url: None,
        }
    }

    pub fn with_message(kind: ErrorType, message: impl Into<String>) -> Self {
        QueryError {
            kind,
            message: Some(message.into()),
            error_data_message: None,
            prompt_title: None,
            prompt_url: None,
        }
    }

    pub fn with_data(kind: ErrorType, message: Option<String>, data: impl Into<String>) -> Self {
        QueryError {
            kind,
            message,
            error_data_message: Some(data.into()),
            prompt_title: None,
            prompt_url: None,
        }
    }

    pub fn unsupported_language(service_name: &str) -> Self {
        QueryError::with_message(
            ErrorType::UnsupportedLanguage,
            format!("{service_name} doesn't support this language"),
        )
    }

    pub fn api_http(status: u16, body: impl Into<String>) -> Self {
        QueryError::with_data(ErrorType::Api, Some(format!("HTTP {status}")), body)
    }
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // U+200B zero-width space, as in the original.
        write!(f, "\u{200B}Query failed, {}", self.kind.description())?;
        if let Some(m) = &self.message {
            if !m.is_empty() {
                write!(f, ": {m}")?;
            }
        }
        if let Some(d) = &self.error_data_message {
            if !d.is_empty() {
                write!(f, "\n\n{d}")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for QueryError {}

/// The state a single service's result is in, driving the card UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultState {
    Idle,
    Loading,
    Streaming,
    Finished,
    Failed,
}

/// One service's result — `EZQueryResult`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryResult {
    /// `serviceType` or `serviceType#uuid` for multi-instance services.
    pub service_type_with_unique_identifier: String,
    pub service_name: String,

    pub query_text: String,
    pub from: Language,
    pub to: Language,

    pub state: ResultState,

    /// Translated text split into paragraphs.
    pub translated_results: Option<Vec<String>>,
    /// Structured dictionary content.
    pub word_result: Option<TranslateWordResult>,
    /// Rendered HTML for local dictionaries (MDict).
    pub html_string: Option<String>,

    pub error: Option<QueryError>,
    /// Non-fatal note, e.g. "streaming disabled after validation".
    pub validation_message: Option<String>,

    pub from_speak_url: Option<String>,
    pub to_speak_url: Option<String>,

    /// True for streaming services — enables Markdown rendering + stop button.
    pub is_stream: bool,
    /// Per-result Markdown override; `None` follows the global setting.
    pub markdown_rendering_override: Option<bool>,
    /// Whether the result may be rendered as Markdown right now.
    pub markdown_enabled: bool,
    pub show_replace_button: bool,

    /// Link to the service's web page for this query.
    pub link: Option<String>,

    // --- runtime state, mirroring the ObjC properties of the same names ---
    /// `isShowing` — the card is visible in the result stack.
    #[serde(default)]
    pub is_showing: bool,
    /// `isLoading` — the card shows a spinner.
    #[serde(default)]
    pub is_loading: bool,
    /// `isStreamFinished` — the stop button is hidden once true.
    #[serde(default = "default_true")]
    pub is_stream_finished: bool,
    /// `manualShow` — the user expanded a service that is off by default.
    #[serde(default)]
    pub manual_show: bool,
    /// `resultGeneration` — the query generation this result belongs to.
    #[serde(default)]
    pub generation: u64,
}

fn default_true() -> bool {
    true
}

impl QueryResult {
    pub fn new(
        service_type_with_unique_identifier: impl Into<String>,
        service_name: impl Into<String>,
    ) -> Self {
        QueryResult {
            service_type_with_unique_identifier: service_type_with_unique_identifier.into(),
            service_name: service_name.into(),
            query_text: String::new(),
            from: Language::Auto,
            to: Language::Auto,
            state: ResultState::Idle,
            translated_results: None,
            word_result: None,
            html_string: None,
            error: None,
            validation_message: None,
            from_speak_url: None,
            to_speak_url: None,
            is_stream: false,
            markdown_rendering_override: None,
            markdown_enabled: false,
            show_replace_button: false,
            link: None,
            is_showing: false,
            is_loading: false,
            is_stream_finished: true,
            manual_show: false,
            generation: 0,
        }
    }

    /// `translatedResults.joined("\n")`
    pub fn translated_text(&self) -> Option<String> {
        self.translated_results.as_ref().map(|v| v.join("\n"))
    }

    /// `hasTranslatedResult`
    pub fn has_translated_result(&self) -> bool {
        self.word_result.is_some()
            || self.translated_results.is_some()
            || self.html_string.as_ref().is_some_and(|s| !s.is_empty())
    }

    /// Effective Markdown setting — `isMarkdownRenderingEnabled`.
    pub fn markdown_rendering_enabled(&self, global_default: bool) -> bool {
        self.markdown_rendering_override.unwrap_or(global_default)
    }

    /// `isWarningErrorType`
    pub fn is_warning_error(&self) -> bool {
        self.error
            .as_ref()
            .is_some_and(|e| e.kind.is_warning())
    }

    /// `errorMessage`
    pub fn error_message(&self) -> Option<String> {
        self.error.as_ref().map(|e| e.to_string())
    }

    /// Reset for a new query, keeping identity — `reset()`.
    pub fn reset(&mut self) {
        self.translated_results = None;
        self.word_result = None;
        self.html_string = None;
        self.error = None;
        self.validation_message = None;
        self.from_speak_url = None;
        self.to_speak_url = None;
        self.markdown_rendering_override = None;
        self.is_showing = false;
        self.is_loading = false;
        self.is_stream_finished = true;
        self.manual_show = false;
        self.show_replace_button = false;
        self.state = ResultState::Idle;
    }

    /// `convertToTraditionalChineseResult()`
    pub fn convert_to_traditional_chinese(&mut self) {
        if let Some(results) = &self.translated_results {
            self.translated_results = Some(
                results
                    .iter()
                    .map(|t| crate::service::languages::to_traditional(t))
                    .collect(),
            );
        }
    }

    /// Append a streamed chunk, matching `StreamService`'s accumulator.
    pub fn append_stream_chunk(&mut self, chunk: &str) {
        let current = self.translated_results.take().unwrap_or_default();
        let joined = current.join("");
        let combined = format!("{joined}{chunk}");
        self.translated_results = Some(vec![combined]);
    }
}

/// `EZQueryResult.convertToTraditionalChineseResult()` applied to a whole
/// result, used by the orchestrator after a Chinese-variant query.
pub fn convert_result_to_traditional(result: &mut QueryResult) {
    result.convert_to_traditional_chinese();
}

/// Part-of-speech abbreviation, ported from `EZTranslatePart.partAbbreviation()`.
///
/// Full names (English and Chinese) collapse to the standard dictionary
/// abbreviations; unknown English words get a trailing dot.
pub fn abbreviate_part(part: &str) -> String {
    let trimmed = part.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let lower = trimmed.to_lowercase();
    let abbr = match lower.as_str() {
        "adj" | "adjective" | "形容词" => "adj.",
        "adv" | "adverb" | "副词" => "adv.",
        "v" | "verb" | "动词" => "v.",
        "linkv" | "linking verb" | "系动词" => "linkv.",
        "auxv" | "auxiliary verb" | "助动词" => "auxv.",
        "modalv" | "modal verb" | "情态动词" => "modalv.",
        "n" | "noun" | "名词" => "n.",
        "pron" | "pronoun" | "代词" => "pron.",
        "prep" | "preposition" | "介词" => "prep.",
        "conj" | "conjunction" | "连词" => "conj.",
        "int" | "感叹词" => "int.",
        "interj" | "interjection" => "interj.",
        "det" | "determinative" | "限定词" => "det.",
        "art" | "article" | "冠词" => "art.",
        "abbr" | "abbreviation" | "缩写" => "abbr.",
        "inf" | "infinitive" | "不定词" => "inf.",
        "part" | "participle" | "分词" => "part.",
        "num" | "numeral" | "数词" => "num.",
        "web" | "网络" => "Web",
        _ => {
            // `part.isEnglishWord` — letters only, so a part that already ends
            // in a dot ("adj.") is left alone instead of gaining a second one.
            if crate::util::analysis::is_english_word(trimmed) {
                return format!("{trimmed}.");
            }
            return trimmed.to_string();
        }
    };
    abbr.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_abbreviate() {
        assert_eq!(abbreviate_part("noun"), "n.");
        assert_eq!(abbreviate_part("名词"), "n.");
        assert_eq!(abbreviate_part("网络"), "Web");
        assert_eq!(abbreviate_part("xyz"), "xyz.");
        assert_eq!(abbreviate_part("形容词"), "adj.");
    }

    #[test]
    fn error_display_matches_original() {
        let e = QueryError::api_http(500, "boom");
        assert_eq!(e.to_string(), "\u{200B}Query failed, API error: HTTP 500\n\nboom");
    }

    #[test]
    fn stream_accumulates() {
        let mut r = QueryResult::new("OpenAI", "OpenAI");
        r.append_stream_chunk("Hel");
        r.append_stream_chunk("lo");
        assert_eq!(r.translated_text().as_deref(), Some("Hello"));
    }
}
