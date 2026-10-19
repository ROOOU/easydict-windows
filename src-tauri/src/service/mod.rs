//! The service layer: a `QueryService` trait, a registry of implementations,
//! and the helpers every service shares.
//!
//! Ported from `Easydict/Swift/Service/Model/QueryService.swift`.

pub mod alibaba;
pub mod baidu;
pub mod bing;
pub mod caiyun;
pub mod doubao;
pub mod deepl;
pub mod google;
pub mod languages;
pub mod llm;
pub mod niutrans;
pub mod registry;
pub mod stream;
pub mod tencent;
pub mod volcano;
pub mod youdao;

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::mpsc::UnboundedSender;

use crate::config::{ServiceConfig, Settings};
use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
    TranslatePart, TranslateWordResult, WordPhonetic,
};

/// Incremental output from a streaming service.
#[derive(Debug, Clone)]
pub enum ServiceUpdate {
    /// A chunk of translated text, appended to the running result.
    Chunk(String),
    /// Non-fatal note shown on the result card.
    Validation(String),
}

pub type UpdateSender = UnboundedSender<ServiceUpdate>;

/// Everything a service needs to run a query.
#[derive(Clone)]
pub struct ServiceContext {
    pub settings: Arc<Settings>,
    /// `"OpenAI"` or `"CustomOpenAI#<uuid>"`.
    pub identifier: String,
    pub service_type: ServiceType,
    pub config: ServiceConfig,
}

impl ServiceContext {
    pub fn new(
        settings: Arc<Settings>,
        service_type: ServiceType,
        identifier: impl Into<String>,
    ) -> Self {
        let identifier = identifier.into();
        let config = settings.service(&identifier);
        ServiceContext {
            settings,
            identifier,
            service_type,
            config,
        }
    }

    /// The `EZQueryService`-style unique identifier.
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    /// API key, falling back to a built-in when the service ships one.
    pub fn api_key(&self) -> String {
        self.config.api_key.trim().to_string()
    }

    pub fn temperature(&self) -> f64 {
        self.config.temperature
    }

    pub fn enable_streaming(&self) -> bool {
        self.config.enable_streaming
    }
}

/// The outcome of `prehandle_query_text`.
#[derive(Debug)]
pub enum Prehandle {
    /// The service already produced a final result without a network call.
    Handled(QueryResult),
    /// Proceed with the request, using the (possibly rewritten) text.
    Proceed,
}

/// A translation backend.
///
/// Mirrors `QueryService`: `translate` is the single required method, and the
/// shared `prehandle` / language-mapping logic lives in the default impls.
#[async_trait]
pub trait QueryService: Send + Sync {
    fn service_type(&self) -> ServiceType;

    /// Display name; `CustomOpenAI` overrides this with the user's name.
    fn name(&self) -> String {
        self.service_type().default_name().to_string()
    }

    /// `serviceTypeWithUniqueIdentifier()`
    fn identifier(&self) -> &str;

    fn ctx(&self) -> &ServiceContext;

    /// Language -> service-specific code. `None` means unsupported.
    fn language_code(&self, language: Language) -> Option<String> {
        languages::default_language_code(self.service_type(), language)
    }

    /// Languages this service can translate between.
    fn supported_languages(&self) -> Vec<Language> {
        Language::all()
            .iter()
            .copied()
            .filter(|l| self.language_code(*l).is_some())
            .collect()
    }

    /// `supportedQueryType()`
    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
    }

    /// `isStream()`
    fn is_stream(&self) -> bool {
        self.service_type().is_streaming_service()
    }

    /// `hasPrivateAPIKey()` — services whose credentials do not live in the
    /// generic `apiKey` slot (Tencent's id/secret pair, Volcano's AK/SK)
    /// override this.
    fn has_private_api_key(&self) -> bool {
        !self.ctx().api_key().is_empty()
    }

    /// Page a "word link" button opens.
    fn word_link(&self, _query: &QueryModel) -> Option<String> {
        None
    }

    /// Page the service-name button opens.
    fn link(&self) -> Option<String> {
        None
    }

    /// True when the service converts zh-Hans <-> zh-Hant itself, so the
    /// shared conversion shortcut must not fire.
    fn auto_convert_traditional_chinese(&self) -> bool {
        false
    }

    /// Performs the request.
    ///
    /// Streaming services push `ServiceUpdate::Chunk` values through `updates`
    /// as they arrive and still return the completed `QueryResult`.
    async fn translate(
        &self,
        query: &QueryModel,
        updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError>;

    /// Starts from a blank result carrying this service's identity.
    fn new_result(&self, query: &QueryModel) -> QueryResult {
        let mut r = QueryResult::new(self.identifier().to_string(), self.name());
        r.query_text = query.query_text.clone();
        r.from = query.query_from_language;
        r.to = query.query_target_language;
        r.is_stream = self.is_stream();
        r.markdown_enabled = self.is_stream();
        r.link = self.word_link(query).or_else(|| self.link());
        r
    }

    /// Shared pre-checks, ported from `prehandleQueryTextOutcome`.
    ///
    /// Returns `Handled` when the service answered without a request:
    /// a Chinese-variant conversion, an unsupported language pair, or a
    /// missing key.
    fn prehandle(&self, query: &QueryModel) -> Result<Prehandle, QueryError> {
        let from = query.query_from_language;
        let to = query.query_target_language;

        // Chinese-variant conversion, answered locally. Upstream runs this
        // *before* the language gate, so a service that converts Chinese itself
        // never needs a language code for the variant it is converting.
        if self.auto_convert_traditional_chinese()
            && languages::only_chinese([from, to])
        {
            let converted = languages::convert_chinese_variant(&query.query_text, from, to);
            let mut r = self.new_result(query);
            r.translated_results = Some(vec![converted]);
            r.state = crate::model::ResultState::Finished;
            return Ok(Prehandle::Handled(r));
        }

        // Unsupported language pair.
        if self.language_code(from).is_none() || self.language_code(to).is_none() {
            return Err(QueryError::unsupported_language(&self.name()));
        }

        // A service that needs a user key cannot answer without one.
        if self.service_type().api_key_requirement().needs_user_provided_key()
            && !self.has_private_api_key()
        {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                format!(
                    "Please enter your {} API key in Settings → Services.",
                    self.name()
                ),
            ));
        }

        Ok(Prehandle::Proceed)
    }
}

/// Builds a single-phonetic `TranslateWordResult`, used by several services.
pub fn word_result_with_phonetic(
    word: &str,
    language: Language,
    phonetic: Option<String>,
    accent: Option<&str>,
    speak_url: Option<String>,
) -> TranslateWordResult {
    let mut wr = TranslateWordResult {
        phonetics: Some(vec![WordPhonetic {
            word: word.to_string(),
            language,
            value: phonetic,
            speak_url,
            name: Some(match accent {
                Some("uk") => "BrE".to_string(),
                Some("us") => "AmE".to_string(),
                _ if language.is_chinese() => "Pronunciation".to_string(),
                _ => "AmE".to_string(),
            }),
            accent: accent.map(str::to_string),
        }]),
        ..Default::default()
    };
    if let Some(p) = wr.phonetics.as_ref() {
        if p.first().is_some_and(|x| x.value.is_none() && x.speak_url.is_none()) {
            wr.phonetics = None;
        }
    }
    wr
}

/// Attaches a simple word/part result if one is not already present.
pub fn ensure_word_result(result: &mut QueryResult) -> &mut TranslateWordResult {
    result
        .word_result
        .get_or_insert_with(TranslateWordResult::default)
}

/// Groups `(part, means)` pairs into a `TranslateWordResult`.
pub fn word_result_from_parts(pairs: Vec<(String, Vec<String>)>) -> TranslateWordResult {
    let parts: Vec<TranslatePart> = pairs
        .into_iter()
        .filter(|(_, means)| !means.is_empty())
        .map(|(part, means)| TranslatePart::new(part, means))
        .collect();
    TranslateWordResult {
        parts: if parts.is_empty() { None } else { Some(parts) },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ResultState;

    struct Fake {
        ctx: ServiceContext,
    }

    #[async_trait]
    impl QueryService for Fake {
        fn service_type(&self) -> ServiceType {
            self.ctx.service_type
        }
        fn identifier(&self) -> &str {
            "OpenAI"
        }
        fn ctx(&self) -> &ServiceContext {
            &self.ctx
        }
        async fn translate(
            &self,
            query: &QueryModel,
            _updates: Option<UpdateSender>,
        ) -> Result<QueryResult, QueryError> {
            let mut r = self.new_result(query);
            r.translated_results = Some(vec![format!("translated:{}", query.query_text)]);
            r.state = ResultState::Finished;
            Ok(r)
        }
    }

    fn ctx() -> ServiceContext {
        ServiceContext::new(Arc::new(Settings::default()), ServiceType::OpenAI, "OpenAI")
    }

    #[test]
    fn missing_key_is_reported() {
        let svc = Fake { ctx: ctx() };
        let mut q = QueryModel::new("hello");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        let err = svc.prehandle(&q).unwrap_err();
        assert_eq!(err.kind, ErrorType::MissingSecretKey);
    }

    #[test]
    fn unsupported_language_is_reported() {
        // Tencent has no Uyghur entry at all, so the pair is rejected before
        // any request is made.
        let ctx = ServiceContext::new(
            Arc::new(Settings::default()),
            ServiceType::Tencent,
            "Tencent",
        );
        let svc = Fake { ctx };
        let mut q = QueryModel::new("hello");
        q.query_from_language = Language::Uyghur;
        q.query_target_language = Language::English;
        let err = svc.prehandle(&q).unwrap_err();
        assert_eq!(err.kind, ErrorType::UnsupportedLanguage);
    }

    #[test]
    fn a_supported_pair_proceeds() {
        // OpenAI accepts every named language, so this pair is not rejected.
        // It needs a user key, so one is supplied for the check to get past
        // the API-key gate and reach `Proceed`.
        let mut settings = Settings::default();
        settings.service_mut("OpenAI").api_key = "sk-test".to_string();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::OpenAI, "OpenAI");
        let svc = Fake { ctx };
        let mut q = QueryModel::new("hello");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        assert!(matches!(svc.prehandle(&q), Ok(Prehandle::Proceed)));
    }
}
