//! The AI tools: Polishing and Summary — `AIToolService.swift`,
//! `PolishingService.swift` and `SummaryService.swift`.
//!
//! Upstream both extend `BuiltInAIService`, so they inherit the vendored
//! built-in key and endpoint. **Deliberate, documented deviation:** the Windows
//! port ships no built-in credential (`BuiltInAI` is a stub here — see
//! `openai::BuiltInAiService`), so these two services read the endpoint, model
//! and key from their own `Settings::services[…]` entry and report a
//! `MissingSecretKey` error pointing at it when no key is configured. The
//! defaults are the Zhipu endpoint and the models upstream lists as the
//! built-in defaults.
//!
//! Both override `chatMessageDicts(_:)` wholesale, which in practice means the
//! `enableCustomPrompt` settings are ignored: only the built-in polishing /
//! summary prompts are ever sent.

use async_trait::async_trait;

use crate::model::{ErrorType, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType};
use crate::service::llm::prompts::{self, ChatMessage};
use crate::service::llm::{
    resolve_endpoint, resolve_model, supported_query_type, validate_stream_request, LlmTransport,
};
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

/// `BuiltInAIService.defaultEndpoint` in this port — upstream's value lives in
/// `EncryptedSecretKeys.plist`, so the Zhipu endpoint of the default model
/// stands in.
pub const DEFAULT_ENDPOINT: &str = "https://open.bigmodel.cn/api/paas/v4/chat/completions";
/// `BuiltInAIService.defaultModels`
pub const DEFAULT_MODELS: &[&str] =
    &["glm-4.5-flash", "glm-4-flash-250414", "llama-3.1-8b-instant"];
/// `BuiltInAIService.defaultModel`
pub const DEFAULT_MODEL: &str = "glm-4-flash-250414";

/// `StreamService.model`
pub fn model(ctx: &ServiceContext) -> String {
    resolve_model(ctx, DEFAULT_MODELS, DEFAULT_MODEL)
}

/// `StreamService.endpoint`
pub fn endpoint(ctx: &ServiceContext) -> String {
    resolve_endpoint(ctx, DEFAULT_ENDPOINT)
}

/// The error reported when the port has no credential to authenticate with.
pub fn missing_built_in_key(ctx: &ServiceContext) -> QueryError {
    QueryError::with_message(
        ErrorType::MissingSecretKey,
        format!(
            "The Windows port does not ship a built-in API key for {}. Add your own key under Settings → Services → {} (Settings::services[\"{}\"].api_key) and set the endpoint to the OpenAI-compatible provider you want to use.",
            ctx.service_type.raw(),
            ctx.service_type.default_name(),
            ctx.identifier
        ),
    )
}

/// `StreamService.translate(_:from:to:)` for an AI tool: the messages are
/// supplied by the caller because both tools build their own dictionary.
async fn ai_tool_text(
    ctx: &ServiceContext,
    query: &QueryModel,
    messages: Vec<ChatMessage>,
    updates: Option<&UpdateSender>,
) -> Result<String, QueryError> {
    let endpoint = endpoint(ctx);
    validate_stream_request(ctx, &endpoint)?;

    // `AIToolService.apiKeyRequirement() == .builtIn`: the shared check above
    // only covers user-provided keys.
    if ctx.api_key().is_empty() {
        return Err(missing_built_in_key(ctx));
    }

    let transport = LlmTransport::new(endpoint, ctx.api_key(), model(ctx), ctx.temperature());
    transport
        .translate(
            &messages,
            &query.query_text,
            ctx.config.hide_think_tag,
            updates,
            ctx.enable_streaming(),
        )
        .await
}

/// The `ChatQueryParam` both tools unpack; `BaseOpenAIService` always asks for
/// the system prompt.
fn chat_query_param(query: &QueryModel) -> prompts::ChatQueryParam<'_> {
    prompts::ChatQueryParam {
        text: &query.query_text,
        source_language: query.query_from_language,
        target_language: query.query_target_language,
        query_type: QueryTextType::TRANSLATION,
        enable_system_prompt: true,
    }
}

/// `PolishingService.chatMessageDicts(_:)` — no custom prompts, no dictionary or
/// sentence routing.
pub fn polishing_query_messages(query: &QueryModel) -> Vec<ChatMessage> {
    prompts::polishing_messages(chat_query_param(query))
}

/// `SummaryService.chatMessageDicts(_:)`
pub fn summary_query_messages(query: &QueryModel, answer_language: crate::model::Language) -> Vec<ChatMessage> {
    prompts::summary_messages(chat_query_param(query), answer_language)
}

/// Polishing — `PolishingService`.
pub struct PolishingService {
    ctx: ServiceContext,
}

impl PolishingService {
    pub fn new(ctx: ServiceContext) -> Self {
        PolishingService { ctx }
    }
}

#[async_trait]
impl QueryService for PolishingService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Polishing
    }

    fn identifier(&self) -> &str {
        &self.ctx.identifier
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        supported_query_type(&self.ctx.config)
    }

    /// `AIToolService` does not override `link()`.
    fn link(&self) -> Option<String> {
        None
    }

    async fn translate(
        &self,
        query: &QueryModel,
        updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let messages = polishing_query_messages(query);
        let text = ai_tool_text(&self.ctx, query, messages, updates.as_ref()).await?;

        let mut result = self.new_result(query);
        result.translated_results = Some(util::to_paragraphs(&text));
        Ok(result)
    }
}

/// Summary — `SummaryService`.
pub struct SummaryService {
    ctx: ServiceContext,
}

impl SummaryService {
    pub fn new(ctx: ServiceContext) -> Self {
        SummaryService { ctx }
    }
}

#[async_trait]
impl QueryService for SummaryService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Summary
    }

    fn identifier(&self) -> &str {
        &self.ctx.identifier
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        supported_query_type(&self.ctx.config)
    }

    fn link(&self) -> Option<String> {
        None
    }

    async fn translate(
        &self,
        query: &QueryModel,
        updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let answer_language = self.ctx.settings.first_language;
        let messages = summary_query_messages(query, answer_language);
        let text = ai_tool_text(&self.ctx, query, messages, updates.as_ref()).await?;

        let mut result = self.new_result(query);
        result.translated_results = Some(util::to_paragraphs(&text));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::model::Language;
    use std::sync::Arc;

    fn ctx(service_type: ServiceType) -> ServiceContext {
        ServiceContext::new(Arc::new(Settings::default()), service_type, service_type.raw())
    }

    fn query(text: &str) -> QueryModel {
        let mut query = QueryModel::new(text);
        query.query_from_language = Language::English;
        query.query_target_language = Language::SimplifiedChinese;
        query
    }

    #[test]
    fn defaults_are_the_built_in_ai_models() {
        let polishing = PolishingService::new(ctx(ServiceType::Polishing));
        assert_eq!(model(&polishing.ctx), "glm-4-flash-250414");
        assert_eq!(endpoint(&polishing.ctx), DEFAULT_ENDPOINT);
        assert_eq!(DEFAULT_MODELS.len(), 3);
        assert!(polishing.is_stream());
        assert_eq!(polishing.link(), None);
        assert_eq!(polishing.name(), "Polishing");
        assert_eq!(
            ServiceType::Polishing.api_key_requirement(),
            crate::model::ApiKeyRequirement::BuiltIn
        );

        let summary = SummaryService::new(ctx(ServiceType::Summary));
        assert_eq!(summary.name(), "Summary");
        assert!(summary.is_stream());
        assert_eq!(
            ServiceType::Summary.api_key_requirement(),
            crate::model::ApiKeyRequirement::BuiltIn
        );
    }

    #[test]
    fn polishing_prompt_ignores_the_query_type_and_custom_prompts() {
        let mut dictionary_query = query("apple");
        dictionary_query.query_type = QueryTextType::DICTIONARY;
        let messages = polishing_query_messages(&dictionary_query);
        // The built-in polishing dictionary is used wholesale: a dictionary
        // query still gets the polishing system prompt and no dictionary
        // few-shot.
        assert_eq!(messages[0].content, prompts::POLISHING_SYSTEM_PROMPT);
        assert_eq!(messages.len(), 8);
        assert_eq!(
            messages.last().unwrap().content,
            "Polish the following English text to improve its clarity, coherence, grammar, and overall quality while maintaining the original meaning and intent: \"\"\"apple\"\"\""
        );
        assert!(!messages.iter().any(|m| m.content.contains("Here is a English word")));
    }

    #[test]
    fn summary_prompt_answers_in_the_first_language() {
        let messages = summary_query_messages(&query("long text"), Language::SimplifiedChinese);
        assert_eq!(messages[0].content, prompts::SUMMARY_SYSTEM_PROMPT);
        assert_eq!(messages.len(), 6);
        assert_eq!(
            messages.last().unwrap().content,
            "Using Simplified-Chinese to summarize the following English text: \"\"\"long text\"\"\"."
        );

        let messages = summary_query_messages(&query("long text"), Language::English);
        assert_eq!(
            messages.last().unwrap().content,
            "Using English to summarize the following English text: \"\"\"long text\"\"\"."
        );
    }

    #[test]
    fn missing_key_error_names_the_settings_slot() {
        let polishing = PolishingService::new(ctx(ServiceType::Polishing));
        let error = missing_built_in_key(&polishing.ctx);
        assert_eq!(error.kind, ErrorType::MissingSecretKey);
        let message = error.message.unwrap();
        assert!(message.contains("Polishing"));
        assert!(message.contains("Settings::services[\"Polishing\"].api_key"));

        let summary = SummaryService::new(
            ServiceContext::new(
                Arc::new(Settings::default()),
                ServiceType::Summary,
                "Summary",
            ),
        );
        assert!(missing_built_in_key(&summary.ctx)
            .message
            .unwrap()
            .contains("Settings::services[\"Summary\"].api_key"));
    }

    #[test]
    fn configured_key_endpoint_and_model_are_used() {
        let mut settings = Settings::default();
        {
            let config = settings.service_mut("Polishing");
            config.api_key = "key".to_string();
            config.endpoint = "https://proxy.test/v1/chat/completions".to_string();
            config.model = "my-model".to_string();
            // `StreamService.model` only honours a model the user has marked
            // valid for this service.
            config.valid_models = vec!["my-model".to_string()];
        }
        let service = PolishingService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::Polishing,
            "Polishing",
        ));
        assert_eq!(endpoint(&service.ctx), "https://proxy.test/v1/chat/completions");
        assert_eq!(model(&service.ctx), "my-model");
        assert_eq!(service.ctx.api_key(), "key");
    }

    #[test]
    fn supported_query_type_follows_the_toggles() {
        let mut context = ctx(ServiceType::Polishing);
        assert_eq!(
            PolishingService::new(context.clone()).supported_query_type(),
            QueryTextType::TRANSLATION
        );
        context.config.sentence = true;
        assert_eq!(
            PolishingService::new(context).supported_query_type(),
            QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
        );
    }
}
