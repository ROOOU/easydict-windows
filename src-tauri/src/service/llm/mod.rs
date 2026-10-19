//! LLM-backed services — the Windows port of `Easydict/Swift/Service/OpenAI`,
//! plus the Claude, DeepSeek and AI-tool providers.
//!
//! Everything here sits on `StreamService`'s pipeline: the prompt builders live
//! in [`prompts`], the OpenAI-compatible transport is [`LlmTransport`], and each
//! provider only supplies its defaults and its transport quirks.
//!
//! Two upstream behaviours are reproduced deliberately and are easy to mistake
//! for bugs:
//!
//! * the SSE framer cuts an event only when the *suffix* of the read buffer is a
//!   blank-line delimiter — see [`openai::OpenAiSseReader`];
//! * a stream that fails after enough text has arrived may have its error
//!   swallowed and the partial translation kept — see
//!   [`should_ignore_stream_error`];
//! * the transport is not fixed: `enable_streaming` picks between the SSE
//!   pipeline and a plain chat completion posted with `stream: false` — see
//!   [`LlmTransport::translate`].

pub mod ai_tool;
pub mod claude;
pub mod deepseek;
pub mod openai;
pub mod prompts;

pub use ai_tool::{PolishingService, SummaryService};
pub use claude::ClaudeService;
pub use deepseek::DeepSeekService;
pub use openai::{
    BuiltInAiService, CustomOpenAiService, GeminiService, GitHubService, GroqService,
    MiniMaxService, OllamaService, OpenAiService, ZhipuService,
};
pub use prompts::ChatMessage;

use std::collections::HashSet;
use std::time::Duration;

use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};

use crate::config::ServiceConfig;
use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryTextType, ReasoningEffort, ServiceType,
};
use crate::net;
use crate::service::stream;
use crate::service::{QueryService, ServiceContext, ServiceUpdate, UpdateSender};
use crate::util;
use crate::util::analysis;
use std::sync::Arc;

/// `AppConstants.llmRequestTimeoutInterval` — LLM streams.
pub const LLM_TIMEOUT_SECS: u64 = 60;
/// `AppConstants.networkTimeoutInterval` — remote model-list fetches.
pub const MODEL_LIST_TIMEOUT_SECS: u64 = 15;

/// `throttleUpdateResultText(_:...interval:)` — one emission per 0.3 s.
pub const STREAM_THROTTLE_INTERVAL: Duration = Duration::from_millis(300);

/// `OpenAIStreamTransport.maximumErrorBodySize`
pub const MAX_ERROR_BODY_SIZE: usize = 64 * 1024;

/// The `[DONE]` sentinel that terminates an OpenAI-compatible stream.
pub const DONE_FLAG: &str = "[DONE]";

// ---------------------------------------------------------------------------
// Request body
// ---------------------------------------------------------------------------

/// The OpenAI-compatible chat-completions payload.
///
/// Field order and content mirror `ChatQuery` as encoded by `JSONEncoder` in
/// `BaseOpenAIService.openAIChatQuery`: nil fields — in practice only
/// `reasoning_effort`, which the shared transport omits — are left out.
///
/// `stream` is the only field the transport switch touches:
/// `makeNonStreamingChatRequest` sets it to `false` on a copy of the same
/// query, everything else is encoded identically.
#[derive(Debug, Serialize)]
pub struct ChatCompletionRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub model: &'a str,
    pub temperature: f64,
    pub stream: bool,
}

/// `DeepSeekChatRequest` — the shared fields plus DeepSeek V4's thinking switch.
#[derive(Debug, Serialize)]
pub struct DeepSeekChatRequest<'a> {
    pub messages: &'a [ChatMessage],
    pub model: &'a str,
    pub temperature: f64,
    pub stream: bool,
    pub thinking: ThinkingSwitch,
    /// `reasoning_effort` — omitted when the configured effort is `off`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<&'a str>,
}

/// `DeepSeekThinking`
#[derive(Debug, Serialize)]
pub struct ThinkingSwitch {
    #[serde(rename = "type")]
    pub kind: &'static str,
}

impl ThinkingSwitch {
    /// `thinking.type` is always present: `enabled` when the reasoning effort is
    /// not `off`, `disabled` otherwise.
    pub fn from_effort(effort: ReasoningEffort) -> Self {
        ThinkingSwitch {
            kind: if effort.is_enabled() { "enabled" } else { "disabled" },
        }
    }
}

// ---------------------------------------------------------------------------
// Transport
// ---------------------------------------------------------------------------

/// The shared OpenAI-compatible chat transport — `OpenAIStreamTransport`.
///
/// The caller is expected to have run [`validate_stream_request`] first, the
/// same way `BaseOpenAIService.contentStream` calls
/// `validateChatStreamRequest()` before handing the endpoint to the transport.
pub struct LlmTransport {
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub temperature: f64,
}

impl LlmTransport {
    pub fn new(
        endpoint: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
        temperature: f64,
    ) -> Self {
        LlmTransport {
            endpoint: endpoint.into(),
            api_key: api_key.into(),
            model: model.into(),
            temperature,
        }
    }

    /// Streams one completion, pushing throttled `ServiceUpdate::Chunk` values
    /// and returning the cleaned-up final text.
    ///
    /// `OpenAIStreamTransport.stream(query:url:apiKey:onResult:)` plus
    /// `StreamService.getFinalResultText(_:)` and the think-tag filter — the
    /// streaming half of `BaseOpenAIService.contentStream(messages:)`.
    pub async fn stream(
        &self,
        messages: &[ChatMessage],
        query_text: &str,
        hide_think_tag: bool,
        updates: Option<&UpdateSender>,
    ) -> Result<String, QueryError> {
        let body = ChatCompletionRequest {
            messages,
            model: &self.model,
            temperature: self.temperature,
            stream: true,
        };

        let mut request = net::LLM_CLIENT
            .post(&self.endpoint)
            .header("Content-Type", "application/json");
        // Azure-style endpoints read `api-key`, everyone else reads the bearer
        // token; upstream sets both whenever a key exists.
        if !self.api_key.is_empty() {
            request = request
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("api-key", self.api_key.as_str());
        }

        let response = request.json(&body).send().await.map_err(net::map_reqwest_error)?;
        let status = response.status();

        if !status.is_success() {
            let bytes = response.bytes().await.unwrap_or_default();
            let text = String::from_utf8_lossy(&bytes).to_string();
            let text = net::truncate(&text, MAX_ERROR_BODY_SIZE);
            // `validate(response:errorBody:)`: a decodable `{"error": {…}}`
            // wins over the raw body.
            if let Some(message) = api_error_message(&text) {
                return Err(QueryError::with_data(ErrorType::Api, None, message));
            }
            return Err(QueryError::api_http(status.as_u16(), text));
        }

        let mime = mime_type(response.headers());
        if mime != "text/event-stream" {
            return Err(content_type_mismatch_error(&mime, &self.endpoint));
        }

        let mut reader = openai::OpenAiSseReader::new(response);
        let mut accumulated = String::new();
        let mut throttle = ChunkThrottle::new();

        let stream_error = loop {
            match reader.next_event().await {
                Ok(Some(event)) => match openai::decode_event(&event)? {
                    Some(openai::StreamEvent::Content(text)) => {
                        accumulated.push_str(&text);
                        throttle.push(&accumulated, updates);
                    }
                    Some(openai::StreamEvent::Done) => break None,
                    None => {}
                },
                Ok(None) => break None,
                Err(error) => break Some(error),
            }
        };

        // `shouldIgnoreCompletionError(_:resultText:)` / `classifiedQueryError(from:)`
        if let Some(error) = stream_error {
            handle_stream_error(&accumulated, error)?;
        }

        throttle.flush(&accumulated, updates);
        final_text(&accumulated, query_text, hide_think_tag)
    }

    /// `BaseOpenAIService.contentStream(messages:)` — the transport switch.
    ///
    /// `usesStreamingTransport` is `streamingOverride ?? enableStreaming`; the
    /// temporary override only exists for upstream's validation retry, which
    /// this port does not run, so the persisted `enable_streaming` toggle is
    /// the whole decision.
    pub async fn translate(
        &self,
        messages: &[ChatMessage],
        query_text: &str,
        hide_think_tag: bool,
        updates: Option<&UpdateSender>,
        streaming: bool,
    ) -> Result<String, QueryError> {
        if streaming {
            self.stream(messages, query_text, hide_think_tag, updates)
                .await
        } else {
            self.non_streaming(messages, query_text, hide_think_tag, updates)
                .await
        }
    }

    /// `nonStreamingTranslate(query:url:)` with `makeNonStreamingChatRequest`:
    /// the same chat query posted with `stream: false`, yielding the whole
    /// answer as one chunk.
    ///
    /// The two transports disagree about the response and about what counts as
    /// a failure, and that is deliberate upstream: there is **no content-type
    /// gate** here — a JSON body is the expected answer — while only the status
    /// code decides whether the body is an error.
    pub async fn non_streaming(
        &self,
        messages: &[ChatMessage],
        query_text: &str,
        hide_think_tag: bool,
        updates: Option<&UpdateSender>,
    ) -> Result<String, QueryError> {
        let body = ChatCompletionRequest {
            messages,
            model: &self.model,
            temperature: self.temperature,
            stream: false,
        };

        let mut request = net::LLM_CLIENT
            .post(&self.endpoint)
            .header("Content-Type", "application/json");
        // `makeNonStreamingChatRequest` sets both headers under the same
        // non-empty-key condition as the streaming request.
        if !self.api_key.is_empty() {
            request = request
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("api-key", self.api_key.as_str());
        }

        let response = request.json(&body).send().await.map_err(net::map_reqwest_error)?;
        let status = response.status();
        let bytes = response.bytes().await.unwrap_or_default();

        if !status.is_success() {
            // The decoded `{"error": {…}}` envelope wins over the raw body —
            // and, unlike `OpenAIStreamTransport.maximumErrorBodySize` on the
            // streaming path, this one reads the body whole.
            let text = String::from_utf8_lossy(&bytes).to_string();
            if let Some(message) = api_error_message(&text) {
                return Err(QueryError::with_data(ErrorType::Api, None, message));
            }
            return Err(QueryError::api_http(status.as_u16(), text));
        }

        let result = decode_chat_result(&bytes)?;
        // `choices.first?.message.content`, required to be non-empty: a
        // truncated or filtered answer is reported as `.noResult` rather than
        // as a successful empty translation.
        let content = result.first_content().unwrap_or_default();
        if content.is_empty() {
            return Err(stream::no_result_error());
        }

        send_chunk(updates, content.to_string());
        final_text(content, query_text, hide_think_tag)
    }
}

// ---------------------------------------------------------------------------
// Non-streaming result
// ---------------------------------------------------------------------------

/// The chat completion returned when `stream` is `false` — `ChatResult` from
/// the OpenAI SDK, as decoded by `BaseOpenAIService.nonStreamingTranslate`.
///
/// DeepSeek's non-streaming response is the same chat-completions shape, so it
/// decodes through here too.
///
/// `choices` and `message` are required, exactly as the SDK declares them:
/// their absence is a decode failure, not an empty answer. Only `content` is
/// optional, which is what lets a missing content field become `.noResult`.
#[derive(Debug, Deserialize)]
pub struct ChatResult {
    pub choices: Vec<ChatResultChoice>,
}

/// `ChatResult.Choice`
#[derive(Debug, Deserialize)]
pub struct ChatResultChoice {
    pub message: ChatResultMessage,
}

/// `ChatResult.Choice.Message` — `reasoning_content` and friends are ignored;
/// only the assistant text is read.
#[derive(Debug, Deserialize)]
pub struct ChatResultMessage {
    #[serde(default)]
    pub content: Option<String>,
}

impl ChatResult {
    /// `chatResult.choices.first?.message.content`
    pub fn first_content(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|choice| choice.message.content.as_deref())
    }
}

/// Decodes a non-streaming chat result.
///
/// A body that does not decode is an `api` error carrying the decoder's own
/// message, mirroring `decodeEvent(dataLines:)`'s handling of an undecodable
/// stream chunk.
pub fn decode_chat_result(data: &[u8]) -> Result<ChatResult, QueryError> {
    serde_json::from_slice(data).map_err(|error| {
        QueryError::with_data(
            ErrorType::Api,
            None,
            format!(
                "{error}: {}",
                net::truncate(&String::from_utf8_lossy(data), 2048)
            ),
        )
    })
}

/// Delivers streamed text at most once every [`STREAM_THROTTLE_INTERVAL`].
///
/// Upstream's `throttleUpdateResultText` re-delivers the whole accumulated
/// snapshot each tick because `updateResultText` *replaces*
/// `translatedResults`. In this port `ServiceUpdate::Chunk` is append-style —
/// see `QueryResult::append_stream_chunk` and `doubao.rs`, which sends the bare
/// delta — so each emission carries only the not-yet-delivered suffix of the
/// accumulator. Concatenating every chunk still yields the same final text.
pub struct ChunkThrottle {
    last_emit: tokio::time::Instant,
    /// Length of the accumulator already handed to `updates`; always on a char
    /// boundary because it is only ever set to a previous `accumulated.len()`.
    delivered: usize,
}

impl ChunkThrottle {
    pub fn new() -> Self {
        ChunkThrottle {
            last_emit: tokio::time::Instant::now(),
            delivered: 0,
        }
    }

    /// The pending suffix, when at least one interval has elapsed.
    fn take_due(&mut self, accumulated: &str, now: tokio::time::Instant) -> Option<String> {
        if now.duration_since(self.last_emit) < STREAM_THROTTLE_INTERVAL {
            return None;
        }
        let pending = accumulated.get(self.delivered..)?;
        if pending.is_empty() {
            return None;
        }
        self.last_emit = now;
        self.delivered = accumulated.len();
        Some(pending.to_string())
    }

    /// The pending suffix, ignoring the interval — the end-of-stream flush.
    fn take_remaining(&mut self, accumulated: &str) -> Option<String> {
        let pending = accumulated.get(self.delivered..)?;
        if pending.is_empty() {
            return None;
        }
        self.delivered = accumulated.len();
        Some(pending.to_string())
    }

    pub fn push(&mut self, accumulated: &str, updates: Option<&UpdateSender>) {
        if let Some(text) = self.take_due(accumulated, tokio::time::Instant::now()) {
            send_chunk(updates, text);
        }
    }

    pub fn flush(&mut self, accumulated: &str, updates: Option<&UpdateSender>) {
        if let Some(text) = self.take_remaining(accumulated) {
            send_chunk(updates, text);
        }
    }
}

impl Default for ChunkThrottle {
    fn default() -> Self {
        ChunkThrottle::new()
    }
}

fn send_chunk(updates: Option<&UpdateSender>, text: String) {
    if let Some(tx) = updates {
        let _ = tx.send(ServiceUpdate::Chunk(text));
    }
}

// ---------------------------------------------------------------------------
// Result cleanup
// ---------------------------------------------------------------------------

/// `getFinalResultText(_:)` followed by the `hideThinkTagContent` filter.
///
/// An empty result with no error is a `noResult` failure, matching
/// `updateResultText(_:...markStreamFinished:)`.
pub fn final_text(
    accumulated: &str,
    query_text: &str,
    hide_think_tag: bool,
) -> Result<String, QueryError> {
    let mut text = stream::get_final_result_text(accumulated, query_text);
    if hide_think_tag {
        text = stream::filter_think_tag_content(&text).trim().to_string();
    }
    if text.is_empty() {
        return Err(stream::no_result_error());
    }
    Ok(text)
}

// ---------------------------------------------------------------------------
// Request pre-checks
// ---------------------------------------------------------------------------

/// `BaseOpenAIService.validateChatStreamRequest()`.
pub fn validate_stream_request(
    ctx: &ServiceContext,
    endpoint: &str,
) -> Result<(), QueryError> {
    if !is_valid_endpoint(endpoint) {
        return Err(QueryError::with_message(
            ErrorType::Parameter,
            format!("`{}` endpoint is invalid", ctx.service_type.raw()),
        ));
    }
    // `apiKeyRequirement().requiresKeyForRequest` — Ollama is the one
    // OpenAI-compatible service that needs no key.
    if ctx.service_type.api_key_requirement().needs_user_provided_key()
        && ctx.api_key().is_empty()
    {
        return Err(QueryError::with_message(
            ErrorType::MissingSecretKey,
            "API key is empty",
        ));
    }
    Ok(())
}

/// `URL(string:).isValid` — an empty or unparsable endpoint is rejected before
/// the request is attempted.
pub fn is_valid_endpoint(endpoint: &str) -> bool {
    let trimmed = endpoint.trim();
    !trimmed.is_empty() && reqwest::Url::parse(trimmed).is_ok()
}

/// `StreamService.model` — the configured model, or the provider default.
///
/// Upstream reads `Defaults[modelKey]`, whose default value is `defaultModel`;
/// an unset model therefore resolves to the provider default rather than to
/// the head of the list. The first valid model only stands in when that value
/// is empty or is no longer listed.
///
/// `valid_models` is the user's editable list; when it is empty the provider's
/// built-in default list stands in for it (as `doubao.rs` does).
pub fn resolve_model(ctx: &ServiceContext, default_models: &[&str], default_model: &str) -> String {
    let valid: Vec<String> = if ctx.config.valid_models.is_empty() {
        default_models.iter().map(|m| m.to_string()).collect()
    } else {
        ctx.config.valid_models.clone()
    };
    let configured = ctx.config.model.trim();
    let current = if configured.is_empty() {
        default_model
    } else {
        configured
    };
    if !current.is_empty() && valid.iter().any(|m| m == current) {
        return current.to_string();
    }
    valid
        .first()
        .cloned()
        .unwrap_or_else(|| default_model.to_string())
}

/// `StreamService.endpoint` — the configured endpoint, or the provider default.
pub fn resolve_endpoint(ctx: &ServiceContext, default_endpoint: &str) -> String {
    let configured = ctx.config.endpoint.trim();
    if configured.is_empty() {
        default_endpoint.to_string()
    } else {
        configured.to_string()
    }
}

/// `StreamService.supportedQueryType()` — the enabled toggles, OR'ed together.
pub fn supported_query_type(config: &ServiceConfig) -> QueryTextType {
    let mut types = QueryTextType::NONE;
    if config.translation {
        types = types.union(QueryTextType::TRANSLATION);
    }
    if config.sentence {
        types = types.union(QueryTextType::SENTENCE);
    }
    if config.dictionary {
        types = types.union(QueryTextType::DICTIONARY);
    }
    types
}

/// `StreamService.queryType(text:from:to:)`.
///
/// A query type already chosen by the caller wins; otherwise the model is asked
/// for a dictionary entry or a sentence breakdown when its toggles allow it.
pub fn routed_query_type(config: &ServiceConfig, query: &QueryModel) -> QueryTextType {
    if !query.query_type.is_empty() {
        return query.query_type;
    }

    let supported = supported_query_type(config);
    if supported.contains(QueryTextType::DICTIONARY)
        && analysis::should_query_dictionary(&query.query_text, query.query_from_language, 2)
    {
        return QueryTextType::DICTIONARY;
    }
    if supported.contains(QueryTextType::SENTENCE)
        && query.query_from_language == Language::English
        && analysis::should_query_sentence(&query.query_text, query.query_from_language)
    {
        return QueryTextType::SENTENCE;
    }
    QueryTextType::TRANSLATION
}

/// The chat messages one query turns into — `contentStreamTranslate` packing a
/// `ChatQueryParam` and calling `chatMessageDicts(_:)`.
pub fn query_messages(ctx: &ServiceContext, query: &QueryModel) -> Vec<ChatMessage> {
    prompts::chat_message_dicts(
        prompts::ChatQueryParam {
            text: &query.query_text,
            source_language: query.query_from_language,
            target_language: query.query_target_language,
            query_type: routed_query_type(&ctx.config, query),
            // `BaseOpenAIService` always asks for the system prompt.
            enable_system_prompt: true,
        },
        query,
        &ctx.config,
        ctx.settings.first_language,
    )
}

// ---------------------------------------------------------------------------
// Response validation
// ---------------------------------------------------------------------------

/// The response MIME type, lowercased and stripped of parameters the way
/// `HTTPURLResponse.mimeType` reports it. `"unknown"` when the header is
/// missing, as upstream's `?? "unknown"`.
pub fn mime_type(headers: &HeaderMap) -> String {
    headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| {
            value
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase()
        })
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

/// `OpenAIStreamTransportError.incorrectContentType(_:url:)`
pub fn content_type_mismatch_error(mime: &str, url: &str) -> QueryError {
    QueryError::with_message(
        ErrorType::ContentTypeMismatch,
        format!(
            "Incorrect Content-Type: {mime}, acceptable type is text/event-stream. This may be caused by a wrong endpoint: {url}"
        ),
    )
}

/// `{"error": {"message": "…"}}`, where the message may be a string or an array
/// of strings — the shape `APIErrorResponse` decodes.
#[derive(Debug, Deserialize)]
struct ApiErrorEnvelope {
    error: ApiErrorObject,
}

#[derive(Debug, Deserialize)]
struct ApiErrorObject {
    message: ApiErrorMessage,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ApiErrorMessage {
    Text(String),
    Lines(Vec<String>),
}

impl ApiErrorMessage {
    /// An array of messages is joined with newlines.
    fn into_message(self) -> String {
        match self {
            ApiErrorMessage::Text(text) => text,
            ApiErrorMessage::Lines(lines) => lines.join("\n"),
        }
    }
}

/// Decodes `{"error": {…}}` into the API's own message, if the body has that
/// shape. Used both for non-2xx bodies and for `error` chunks mid-stream.
pub fn api_error_message(body: &str) -> Option<String> {
    serde_json::from_str::<ApiErrorEnvelope>(body)
        .ok()
        .map(|envelope| envelope.error.message.into_message())
}

/// The lowercased haystack used to classify a failed stream.
///
/// Upstream collects a set of strings from the error and its underlying errors
/// (`errorContextString(_:)`); a `QueryError` only carries two text fields, so
/// both are folded in here.
pub fn error_context(error: &QueryError) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if let Some(message) = &error.message {
        parts.push(message);
    }
    if let Some(data) = &error.error_data_message {
        parts.push(data);
    }
    parts.join(" | ").to_lowercase()
}

/// `shouldIgnoreCompletionError(_:resultText:)` — keep what we have when a
/// content-type mismatch arrives after a usable amount of text.
pub fn should_ignore_stream_error(accumulated: &str, context: &str) -> bool {
    let trimmed = accumulated.trim();
    if trimmed.is_empty() || trimmed.chars().count() < 8 {
        return false;
    }
    let is_content_type_error = is_content_type_mismatch_context(context);
    let is_known_mime =
        context.contains("text/plain") || context.contains("application/json");
    is_content_type_error && is_known_mime
}

/// `isContentTypeMismatchContext(_:)`
pub fn is_content_type_mismatch_context(context: &str) -> bool {
    context.contains("incorrectcontenttype(")
        || context.contains("incorrect content-type:")
        || context.contains("unacceptable content-type:")
}

/// `classifiedQueryError(from:)` — the localized messages the content-type
/// classifier produces, keyed by `error.content_type.*`.
pub fn classified_stream_error(context: &str, fallback: QueryError) -> QueryError {
    if is_content_type_mismatch_context(context) {
        if context.contains("text/html") {
            return QueryError::with_data(
                ErrorType::ContentTypeMismatch,
                Some("Endpoint returned an HTML page".to_string()),
                "Please check that the endpoint URL is correct and includes the full API path.",
            );
        }
        if context.contains("application/json") {
            return QueryError::with_data(
                ErrorType::ContentTypeMismatch,
                Some("Endpoint returned JSON instead of a stream".to_string()),
                "Try disabling \"Enable Streaming\" in the service settings.",
            );
        }
        return QueryError::with_data(
            ErrorType::ContentTypeMismatch,
            Some("Unexpected response format".to_string()),
            "The endpoint may not be fully OpenAI-compatible. Check the endpoint URL and API documentation.",
        );
    }
    fallback
}

/// The mid-stream failure policy every `StreamService` subclass shares: keep the
/// text when [`should_ignore_stream_error`] says so, otherwise surface the
/// classified error.
pub fn handle_stream_error(accumulated: &str, error: QueryError) -> Result<(), QueryError> {
    let context = error_context(&error);
    if should_ignore_stream_error(accumulated, &context) {
        return Ok(());
    }
    Err(classified_stream_error(&context, error))
}

// ---------------------------------------------------------------------------
// Remote model lists
// ---------------------------------------------------------------------------

/// `BaseOpenAIService.remoteModelsURL()` — the `/models` sibling of a
/// chat-completions endpoint.
pub fn derive_models_url(endpoint: &str) -> Result<String, QueryError> {
    let invalid = || QueryError::with_message(ErrorType::Parameter, "Endpoint is invalid");
    let mut url = reqwest::Url::parse(endpoint.trim()).map_err(|_| invalid())?;

    let mut parts: Vec<String> = url
        .path_segments()
        .map(|segments| {
            segments
                .filter(|segment| !segment.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let lowered: Vec<String> = parts.iter().map(|part| part.to_lowercase()).collect();
    let last_two_are_chat_completions = lowered.len() >= 2
        && lowered[lowered.len() - 2] == "chat"
        && lowered[lowered.len() - 1] == "completions";

    if parts.is_empty() {
        parts.push("v1".to_string());
    } else if last_two_are_chat_completions {
        parts.truncate(parts.len() - 2);
    } else if lowered.last().map(String::as_str) == Some("completions") {
        parts.truncate(parts.len() - 1);
    } else if lowered.last().map(String::as_str) == Some("models") {
        parts.truncate(parts.len() - 1);
    }
    parts.push("models".to_string());

    url.set_fragment(None);
    url.set_path(&format!("/{}", parts.join("/")));
    Ok(url.to_string())
}

/// `StreamService.fetchRemoteModelData(url:headers:)`.
///
/// Model lists are not LLM completions and use the 15-second client.
pub async fn fetch_remote_model_data(url: &str, headers: HeaderMap) -> Result<Vec<u8>, QueryError> {
    let response = net::CLIENT
        .get(url)
        .headers(headers)
        .send()
        .await
        .map_err(net::map_reqwest_error)?;
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    if !status.is_success() {
        return Err(remote_model_error(
            status.as_u16(),
            &String::from_utf8_lossy(&bytes),
        ));
    }
    Ok(bytes.to_vec())
}

/// `remoteModelError(statusCode:data:)`
pub fn remote_model_error(status: u16, body: &str) -> QueryError {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body.trim()) {
        let message = value
            .get("error")
            .and_then(|error| {
                error
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| {
                        error
                            .get("message")
                            .and_then(|message| message.as_str())
                            .map(str::to_string)
                    })
            })
            .or_else(|| {
                value
                    .get("message")
                    .and_then(|message| message.as_str())
                    .map(str::to_string)
            });
        if let Some(message) = message.filter(|message| !message.trim().is_empty()) {
            return QueryError::with_message(ErrorType::Api, message.trim());
        }
    }

    let text = body.trim();
    if !text.is_empty() {
        return QueryError::with_message(ErrorType::Api, util::trim_to_max_chars(text, 300));
    }
    QueryError::with_message(ErrorType::Api, format!("HTTP {status}"))
}

/// `normalizedRemoteModelIDs(_:)` — trimmed, de-duplicated on the lookup id.
pub fn normalized_remote_model_ids(ids: &[String], lookup_id: fn(&str) -> String) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    ids.iter()
        .map(|id| id.trim())
        .filter(|id| !id.is_empty())
        .filter(|id| seen.insert(lookup_id(id)))
        .map(str::to_string)
        .collect()
}

/// `StreamService.remoteModelLookupID(_:)`
pub fn default_remote_model_lookup_id(model_id: &str) -> String {
    model_id.trim().to_lowercase()
}

/// `GitHubService.remoteModelLookupID(_:)` — GitHub ids are `publisher/model`,
/// so only the model part takes part in the de-duplication.
pub fn github_remote_model_lookup_id(model_id: &str) -> String {
    let model_id = default_remote_model_lookup_id(model_id);
    match model_id.split_once('/') {
        Some((_, model)) => model.to_string(),
        None => model_id,
    }
}

/// `{"data": [{"id": "…"}]}` — the OpenAI-compatible model list.
#[derive(Debug, Deserialize)]
struct OpenAiModelList {
    data: Vec<OpenAiModelItem>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelItem {
    id: String,
}

/// Decodes the OpenAI-compatible model list, or reports `Invalid models
/// response` exactly as `BaseOpenAIService.fetchRemoteModelIDs` does.
pub fn decode_openai_model_list(data: &[u8]) -> Result<Vec<String>, QueryError> {
    let parsed: OpenAiModelList = serde_json::from_slice(data).map_err(|_| {
        QueryError::with_message(ErrorType::Api, "Invalid models response")
    })?;
    Ok(parsed.data.into_iter().map(|item| item.id).collect())
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// `QueryServiceFactory` entry point for the LLM providers, reached from
/// `registry::build`'s catch-all arm. `None` means the type is not an LLM
/// service.
pub fn build(ctx: ServiceContext) -> Option<Arc<dyn QueryService>> {
    let service: Arc<dyn QueryService> = match ctx.service_type {
        ServiceType::OpenAI => Arc::new(OpenAiService::new(ctx)),
        ServiceType::CustomOpenAI => Arc::new(CustomOpenAiService::new(ctx)),
        ServiceType::BuiltInAI => Arc::new(BuiltInAiService::new(ctx)),
        ServiceType::Gemini => Arc::new(GeminiService::new(ctx)),
        ServiceType::Claude => Arc::new(ClaudeService::new(ctx)),
        ServiceType::DeepSeek => Arc::new(DeepSeekService::new(ctx)),
        ServiceType::Groq => Arc::new(GroqService::new(ctx)),
        ServiceType::Zhipu => Arc::new(ZhipuService::new(ctx)),
        ServiceType::MiniMax => Arc::new(MiniMaxService::new(ctx)),
        ServiceType::GitHub => Arc::new(GitHubService::new(ctx)),
        ServiceType::Ollama => Arc::new(OllamaService::new(ctx)),
        ServiceType::Polishing => Arc::new(PolishingService::new(ctx)),
        ServiceType::Summary => Arc::new(SummaryService::new(ctx)),
        _ => return None,
    };
    Some(service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::model::ServiceType;
    use std::sync::Arc;

    fn ctx(service_type: ServiceType) -> ServiceContext {
        ServiceContext::new(Arc::new(Settings::default()), service_type, service_type.raw())
    }

    #[test]
    fn openai_request_body_uses_the_shared_field_order() {
        let messages = vec![ChatMessage::system("s"), ChatMessage::user("u")];
        let body = ChatCompletionRequest {
            messages: &messages,
            model: "gpt-5-mini",
            temperature: 0.3,
            stream: true,
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "messages": [{"role": "system", "content": "s"}, {"role": "user", "content": "u"}],
                "model": "gpt-5-mini",
                "temperature": 0.3,
                "stream": true,
            })
        );
        assert!(json.get("reasoning_effort").is_none());
    }

    #[test]
    fn chat_result_reads_the_first_choice_content() {
        let body = br#"{"id":"1","object":"chat.completion","created":0,"model":"m",
            "choices":[{"index":0,"message":{"role":"assistant","content":"hi"},"finish_reason":"stop"}]}"#;
        assert_eq!(decode_chat_result(body).unwrap().first_content(), Some("hi"));

        // The SDK's `ChatResult.Choice.Message.content` is optional, so a
        // message without one decodes to no text rather than failing ...
        assert_eq!(
            decode_chat_result(br#"{"choices":[{"message":{"role":"assistant"}}]}"#)
                .unwrap()
                .first_content(),
            None
        );
        // ... while `choices` and `message` themselves are not.
        for missing in [r#"{"choices":[{"index":0}]}"#, r#"{}"#] {
            let error = decode_chat_result(missing.as_bytes()).unwrap_err();
            assert_eq!(error.kind, ErrorType::Api, "{missing}");
        }
    }

    #[test]
    fn deepseek_body_always_carries_thinking() {
        let messages = vec![ChatMessage::user("u")];
        let body = DeepSeekChatRequest {
            messages: &messages,
            model: "deepseek-v4-flash",
            temperature: 0.3,
            stream: true,
            thinking: ThinkingSwitch::from_effort(ReasoningEffort::Off),
            reasoning_effort: ReasoningEffort::Off.request_value(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["thinking"], serde_json::json!({"type": "disabled"}));
        assert!(json.get("reasoning_effort").is_none());

        let body = DeepSeekChatRequest {
            messages: &messages,
            model: "deepseek-v4-pro",
            temperature: 0.3,
            stream: true,
            thinking: ThinkingSwitch::from_effort(ReasoningEffort::High),
            reasoning_effort: ReasoningEffort::High.request_value(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["thinking"], serde_json::json!({"type": "enabled"}));
        assert_eq!(json["reasoning_effort"], serde_json::json!("high"));
    }

    #[test]
    fn model_falls_back_to_the_first_valid_model() {
        let mut context = ctx(ServiceType::OpenAI);
        let defaults = ["gpt-5", "gpt-5-mini"];
        // An unset model is `defaultModel`, not the head of the list, and a
        // provider default that is listed is kept as-is.
        assert_eq!(resolve_model(&context, &defaults, "gpt-5-mini"), "gpt-5-mini");
        assert_eq!(resolve_model(&context, &defaults, "gpt-5"), "gpt-5");

        context.config.model = "gpt-5-mini".to_string();
        assert_eq!(resolve_model(&context, &defaults, "gpt-5"), "gpt-5-mini");

        // A stored model that is not in the list falls back to the first one.
        context.config.model = "not-a-model".to_string();
        assert_eq!(resolve_model(&context, &defaults, "gpt-5"), "gpt-5");

        // A user-edited valid-model list takes precedence over the defaults.
        context.config.valid_models = vec!["custom".to_string()];
        assert_eq!(resolve_model(&context, &defaults, "gpt-5"), "custom");
    }

    #[test]
    fn endpoint_falls_back_to_the_provider_default() {
        let mut context = ctx(ServiceType::OpenAI);
        assert_eq!(
            resolve_endpoint(&context, "https://api.openai.com/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        context.config.endpoint = " https://example.test/v1/chat/completions ".to_string();
        assert_eq!(
            resolve_endpoint(&context, "https://api.openai.com/v1/chat/completions"),
            "https://example.test/v1/chat/completions"
        );
    }

    #[test]
    fn supported_query_type_follows_the_toggles() {
        let mut config = ServiceConfig::default();
        assert_eq!(supported_query_type(&config), QueryTextType::TRANSLATION);

        config.sentence = true;
        assert_eq!(
            supported_query_type(&config),
            QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
        );

        config.dictionary = true;
        assert_eq!(supported_query_type(&config), QueryTextType::INTELLIGENT);

        config.translation = false;
        assert_eq!(
            supported_query_type(&config),
            QueryTextType::DICTIONARY.union(QueryTextType::SENTENCE)
        );
    }

    #[test]
    fn routed_query_type_uses_the_callers_choice_first() {
        let config = ServiceConfig::default();
        let mut query = QueryModel::new("hello world");
        query.query_from_language = Language::English;
        assert_eq!(
            routed_query_type(&config, &query),
            QueryTextType::TRANSLATION
        );

        // With no caller choice, a two-word English input is a sentence only
        // when the sentence toggle is on.
        query.query_type = QueryTextType::NONE;
        assert_eq!(
            routed_query_type(&config, &query),
            QueryTextType::TRANSLATION
        );
        let mut config = ServiceConfig::default();
        config.sentence = true;
        assert_eq!(routed_query_type(&config, &query), QueryTextType::SENTENCE);

        // A short word is a dictionary query when that toggle is on.
        let mut short = QueryModel::new("apple");
        short.query_from_language = Language::English;
        short.query_type = QueryTextType::NONE;
        assert_eq!(routed_query_type(&config, &short), QueryTextType::TRANSLATION);
        config.dictionary = true;
        assert_eq!(
            routed_query_type(&config, &short),
            QueryTextType::DICTIONARY
        );
    }

    #[test]
    fn mime_type_is_lowercased_and_stripped_of_parameters() {
        let mut headers = HeaderMap::new();
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "text/event-stream; charset=utf-8".parse().unwrap(),
        );
        assert_eq!(mime_type(&headers), "text/event-stream");
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            "TEXT/Event-Stream".parse().unwrap(),
        );
        assert_eq!(mime_type(&headers), "text/event-stream");
        assert_eq!(mime_type(&HeaderMap::new()), "unknown");
    }

    #[test]
    fn content_type_mismatch_message_matches_upstream() {
        let error = content_type_mismatch_error("application/json", "https://example.test/v1");
        assert_eq!(error.kind, ErrorType::ContentTypeMismatch);
        assert_eq!(
            error.message.as_deref(),
            Some(
                "Incorrect Content-Type: application/json, acceptable type is text/event-stream. This may be caused by a wrong endpoint: https://example.test/v1"
            )
        );
    }

    #[test]
    fn api_error_message_decodes_string_and_array_messages() {
        assert_eq!(
            api_error_message(r#"{"error":{"message":"bad key","type":"invalid_request_error"}}"#).as_deref(),
            Some("bad key")
        );
        assert_eq!(
            api_error_message(r#"{"error":{"message":["first","second"]}}"#).as_deref(),
            Some("first\nsecond")
        );
        // The upstream envelope needs an object with a message.
        assert!(api_error_message(r#"{"error":"bad key"}"#).is_none());
        assert!(api_error_message("not json").is_none());
    }

    #[test]
    fn content_type_errors_are_suppressed_only_with_enough_content() {
        let context = "incorrectcontenttype(application/json, url: https://example.test)";
        // Fewer than 8 characters: the error is reported.
        assert!(!should_ignore_stream_error("short", context));
        assert!(!should_ignore_stream_error("        ", context));
        // Enough text and a known MIME: the error is swallowed.
        assert!(should_ignore_stream_error("a good answer", context));
        assert!(should_ignore_stream_error(
            "a good answer",
            "incorrect content-type: text/plain"
        ));
        // Not a content-type error: never suppressed.
        assert!(!should_ignore_stream_error(
            "a good answer",
            "connection reset by peer"
        ));
        assert!(!should_ignore_stream_error(
            "a good answer",
            "incorrectcontenttype(text/html, url: https://example.test)"
        ));
    }

    #[test]
    fn content_type_errors_are_classified_for_the_user() {
        let fallback = QueryError::with_message(ErrorType::Api, "boom");

        let html = classified_stream_error(
            "incorrect content-type: text/html",
            fallback.clone(),
        );
        assert_eq!(html.kind, ErrorType::ContentTypeMismatch);
        assert_eq!(html.message.as_deref(), Some("Endpoint returned an HTML page"));
        assert_eq!(
            html.error_data_message.as_deref(),
            Some("Please check that the endpoint URL is correct and includes the full API path.")
        );

        let json = classified_stream_error(
            "incorrectcontenttype(application/json, url: x)",
            fallback.clone(),
        );
        assert_eq!(
            json.message.as_deref(),
            Some("Endpoint returned JSON instead of a stream")
        );
        assert_eq!(
            json.error_data_message.as_deref(),
            Some("Try disabling \"Enable Streaming\" in the service settings.")
        );

        let unknown =
            classified_stream_error("unacceptable content-type: text/xml", fallback.clone());
        assert_eq!(unknown.message.as_deref(), Some("Unexpected response format"));
        assert_eq!(
            unknown.error_data_message.as_deref(),
            Some("The endpoint may not be fully OpenAI-compatible. Check the endpoint URL and API documentation.")
        );

        // Anything else keeps the original error.
        let other = classified_stream_error("connection reset", fallback);
        assert_eq!(other.message.as_deref(), Some("boom"));
    }

    #[test]
    fn error_context_folds_message_and_data() {
        let error = QueryError::with_data(
            ErrorType::Api,
            Some("Incorrect Content-Type".to_string()),
            "incorrectcontenttype(application/json, url: X)",
        );
        let context = error_context(&error);
        assert!(context.contains("incorrect content-type"));
        assert!(context.contains("application/json"));
        assert_eq!(context, context.to_lowercase());
    }

    #[test]
    fn mid_stream_errors_are_swallowed_or_classified() {
        let context_only = QueryError::with_message(
            ErrorType::Api,
            "Incorrect Content-Type: application/json",
        );
        // Too little text: the error stands, classified for the user.
        let error = handle_stream_error("short", context_only.clone()).unwrap_err();
        assert_eq!(error.kind, ErrorType::ContentTypeMismatch);

        // Enough text: the error is swallowed and the content is kept.
        assert!(handle_stream_error("a complete sentence", context_only).is_ok());

        // An unrelated failure is never swallowed.
        let unrelated = QueryError::with_message(ErrorType::Api, "connection reset");
        let error = handle_stream_error("a complete sentence", unrelated).unwrap_err();
        assert_eq!(error.message.as_deref(), Some("connection reset"));
    }

    #[test]
    fn final_text_cleans_markers_quotes_and_think_tags() {
        assert_eq!(final_text("  hello  ", "hi", true).unwrap(), "hello");
        assert_eq!(final_text("hello</s>", "hi", true).unwrap(), "hello");
        assert_eq!(
            final_text("<think>x</think>hello", "hi", true).unwrap(),
            "hello"
        );
        // With the tag filter off, the raw think block survives.
        assert_eq!(
            final_text("<think>x</think>hello", "hi", false).unwrap(),
            "<think>x</think>hello"
        );
        let error = final_text("   ", "hi", true).unwrap_err();
        assert_eq!(error.kind, ErrorType::NoResult);
    }

    #[test]
    fn throttle_delivers_suffixes_at_most_every_interval() {
        let mut throttle = ChunkThrottle::new();
        let start = tokio::time::Instant::now();

        // First tick is not yet due.
        assert_eq!(throttle.take_due("hello", start), None);
        assert_eq!(
            throttle.take_due("hello world", start + Duration::from_millis(300)),
            Some("hello world".to_string())
        );
        // Nothing new to send.
        assert_eq!(
            throttle.take_due("hello world", start + Duration::from_millis(600)),
            None
        );
        // Only the not-yet-delivered suffix.
        assert_eq!(
            throttle.take_due(
                "hello world again",
                start + Duration::from_millis(900)
            ),
            Some(" again".to_string())
        );
        // The final flush ignores the interval.
        assert_eq!(
            throttle.take_remaining("hello world again!"),
            Some("!".to_string())
        );
        assert_eq!(throttle.take_remaining("hello world again!"), None);
    }

    #[test]
    fn validate_stream_request_requires_a_key_unless_the_service_has_none() {
        let mut context = ctx(ServiceType::OpenAI);
        let error = validate_stream_request(&context, "https://api.openai.com/v1/chat/completions")
            .unwrap_err();
        assert_eq!(error.kind, ErrorType::MissingSecretKey);
        assert_eq!(error.message.as_deref(), Some("API key is empty"));

        let error = validate_stream_request(&context, "  ").unwrap_err();
        assert_eq!(error.kind, ErrorType::Parameter);
        assert_eq!(
            error.message.as_deref(),
            Some("`OpenAI` endpoint is invalid")
        );

        context.config.api_key = "key".to_string();
        assert!(validate_stream_request(&context, "https://api.openai.com/v1/chat/completions").is_ok());

        // Ollama requires no key.
        let ollama = ctx(ServiceType::Ollama);
        assert!(validate_stream_request(&ollama, "http://localhost:11434/v1/chat/completions").is_ok());
    }

    #[test]
    fn models_url_is_derived_from_the_chat_endpoint() {
        assert_eq!(
            derive_models_url("https://api.openai.com/v1/chat/completions").unwrap(),
            "https://api.openai.com/v1/models"
        );
        // A `/models` endpoint stays put rather than nesting.
        assert_eq!(
            derive_models_url("https://api.openai.com/v1/models").unwrap(),
            "https://api.openai.com/v1/models"
        );
        // Only the last path component names `completions`.
        assert_eq!(
            derive_models_url("https://example.test/v1/completions").unwrap(),
            "https://example.test/v1/models"
        );
        // An empty path becomes `v1/models`, as upstream does.
        assert_eq!(
            derive_models_url("https://example.test").unwrap(),
            "https://example.test/v1/models"
        );
        assert!(derive_models_url("").is_err());
    }

    #[test]
    fn remote_model_errors_prefer_the_api_message() {
        assert_eq!(
            remote_model_error(401, r#"{"error": {"message": "unauthorized"}}"#)
                .message
                .as_deref(),
            Some("unauthorized")
        );
        assert_eq!(
            remote_model_error(401, r#"{"error": "bad token"}"#)
                .message
                .as_deref(),
            Some("bad token")
        );
        assert_eq!(
            remote_model_error(500, r#"{"message": "boom"}"#)
                .message
                .as_deref(),
            Some("boom")
        );
        assert_eq!(
            remote_model_error(500, "  plain text  ").message.as_deref(),
            Some("plain text")
        );
        assert_eq!(remote_model_error(404, "").message.as_deref(), Some("HTTP 404"));
    }

    #[test]
    fn remote_model_ids_are_trimmed_and_deduplicated() {
        let ids = vec![
            " gpt-4.1 ".to_string(),
            "GPT-4.1".to_string(),
            String::new(),
            "gpt-4o".to_string(),
        ];
        assert_eq!(
            normalized_remote_model_ids(&ids, default_remote_model_lookup_id),
            vec!["gpt-4.1".to_string(), "gpt-4o".to_string()]
        );
        // GitHub collapses `publisher/model` to `model` for de-duplication.
        let ids = vec!["openai/gpt-4.1".to_string(), "azure/gpt-4.1".to_string()];
        assert_eq!(
            normalized_remote_model_ids(&ids, github_remote_model_lookup_id),
            vec!["openai/gpt-4.1".to_string()]
        );
    }

    #[test]
    fn openai_model_list_decodes_or_reports_invalid() {
        let data = br#"{"object":"list","data":[{"id":"a"},{"id":"b"}]}"#;
        assert_eq!(decode_openai_model_list(data).unwrap(), vec!["a", "b"]);
        let error = decode_openai_model_list(b"{}").unwrap_err();
        assert_eq!(error.message.as_deref(), Some("Invalid models response"));
    }

    #[test]
    fn factory_builds_every_llm_service() {
        // `registry::build`'s catch-all arm routes here.
        for service_type in [
            ServiceType::OpenAI,
            ServiceType::CustomOpenAI,
            ServiceType::BuiltInAI,
            ServiceType::Gemini,
            ServiceType::Claude,
            ServiceType::DeepSeek,
            ServiceType::Groq,
            ServiceType::Zhipu,
            ServiceType::MiniMax,
            ServiceType::GitHub,
            ServiceType::Ollama,
            ServiceType::Polishing,
            ServiceType::Summary,
        ] {
            let service = build(ctx(service_type)).expect("LLM service builds");
            assert_eq!(service.service_type(), service_type);
        }

        // The non-LLM services are handled by the registry's earlier arms.
        assert!(build(ctx(ServiceType::Youdao)).is_none());
    }
}
