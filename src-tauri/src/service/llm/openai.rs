//! OpenAI-compatible streaming services and their shared SSE framing.
//!
//! Ported from `OpenAIStreamTransport.swift` (`makeRequest`, `validate`,
//! `decodeEvent`, `completedEvent`), `BaseOpenAIService.swift` and the provider
//! subclasses (`OpenAIService`, `GeminiService`, `GroqService`, `ZhipuService`,
//! `MiniMaxService`, `GitHubService`, `OllamaService`, `CustomOpenAIService`,
//! `BuiltInAIService`).

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::Deserialize;

use crate::model::{ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType};
use crate::net;
use crate::service::llm::{
    api_error_message, decode_openai_model_list, default_remote_model_lookup_id,
    derive_models_url, fetch_remote_model_data, github_remote_model_lookup_id,
    normalized_remote_model_ids, query_messages, resolve_endpoint, resolve_model,
    supported_query_type, validate_stream_request, LlmTransport, DONE_FLAG,
};
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

// ---------------------------------------------------------------------------
// SSE framing
// ---------------------------------------------------------------------------

/// Reads an OpenAI-compatible SSE body one event at a time.
///
/// This is **not** `stream::SseReader`: that one scans the buffer for the first
/// `"\n\n"`, whereas `OpenAIStreamTransport.completedEvent(from:)` appends one
/// byte and only cuts when the buffer's *suffix* is a blank-line delimiter. The
/// two disagree on CRLF framing — a buffer ending in `"\r\n\r\n"` contains no
/// `"\n\n"` at all, so a scan-based reader would never emit a single event for
/// a CRLF stream. Reproducing the byte-by-byte suffix check keeps the framing
/// identical, including the case where one network chunk holds several events.
pub struct OpenAiSseReader {
    response: reqwest::Response,
    /// Bytes of the current network chunk not yet fed to `buffer`.
    chunk: Vec<u8>,
    chunk_pos: usize,
    /// The partially-read SSE event.
    buffer: Vec<u8>,
    done: bool,
}

impl OpenAiSseReader {
    pub fn new(response: reqwest::Response) -> Self {
        OpenAiSseReader {
            response,
            chunk: Vec::new(),
            chunk_pos: 0,
            buffer: Vec::new(),
            done: false,
        }
    }

    /// Returns the next complete event, or `None` at end of stream.
    ///
    /// The trailing partial event is emitted when the body ends, exactly as
    /// `stream(query:url:apiKey:onResult:)` does with `eventBuffer`.
    pub async fn next_event(&mut self) -> Result<Option<String>, QueryError> {
        loop {
            if self.chunk_pos < self.chunk.len() {
                let byte = self.chunk[self.chunk_pos];
                self.chunk_pos += 1;
                self.buffer.push(byte);
                if let Some(event) = take_completed_event(&mut self.buffer) {
                    match String::from_utf8(event) {
                        Ok(text) if text.trim().is_empty() => continue,
                        Ok(text) => return Ok(Some(text)),
                        Err(_) => return Err(invalid_utf8_error()),
                    }
                }
                continue;
            }

            if self.done {
                let rest = std::mem::take(&mut self.buffer);
                if rest.is_empty() {
                    return Ok(None);
                }
                return match String::from_utf8(rest) {
                    Ok(text) if text.trim().is_empty() => Ok(None),
                    Ok(text) => Ok(Some(text)),
                    Err(_) => Err(invalid_utf8_error()),
                };
            }

            match self.response.chunk().await {
                Ok(Some(chunk)) => {
                    self.chunk = chunk.to_vec();
                    self.chunk_pos = 0;
                }
                Ok(None) => self.done = true,
                Err(error) => {
                    self.done = true;
                    return Err(net::map_reqwest_error(error));
                }
            }
        }
    }
}

/// `completedEvent(from:)` — a suffix test on the accumulated bytes.
///
/// Returns the event (delimiter removed) and clears the buffer, or `None` when
/// no delimiter terminates it yet.
pub fn take_completed_event(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let delimiter = if buffer.ends_with(b"\r\n\r\n") {
        4
    } else if buffer.ends_with(b"\n\n") || buffer.ends_with(b"\r\r") {
        2
    } else {
        return None;
    };
    let mut event = std::mem::take(buffer);
    event.truncate(event.len() - delimiter);
    Some(event)
}

/// `QueryError(type: .api, message: "Invalid UTF-8 in OpenAI stream response")`
fn invalid_utf8_error() -> QueryError {
    QueryError::with_message(ErrorType::Api, "Invalid UTF-8 in OpenAI stream response")
}

// ---------------------------------------------------------------------------
// Event decoding
// ---------------------------------------------------------------------------

/// One decoded event: either more content, or the `[DONE]` sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    Content(String),
    Done,
}

/// `ChatStreamResult` from the OpenAI SDK.
///
/// `id`, `object`, `created`, `model` and `choices` are all non-optional, so a
/// chunk that omits any of them fails to decode — upstream uses a plain
/// `JSONDecoder` and expects an error there.
#[derive(Debug, Deserialize)]
pub struct ChatStreamChunk {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<ChatStreamChoice>,
}

/// `ChatStreamResult.Choice`
#[derive(Debug, Deserialize)]
pub struct ChatStreamChoice {
    pub index: i64,
    pub delta: ChatStreamDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

/// `ChatStreamResult.Choice.Delta`
#[derive(Debug, Deserialize)]
pub struct ChatStreamDelta {
    #[serde(default)]
    pub content: Option<String>,
}

/// `eventData(from:)` — a strict `data:` prefix test on the **untrimmed** line;
/// only the value after the prefix is trimmed.
fn event_data(line: &str) -> Option<&str> {
    line.strip_prefix("data:").map(str::trim)
}

/// `emitEvent(_:onResult:)` + `decodeEvent(dataLines:)`.
pub fn decode_event(event: &str) -> Result<Option<StreamEvent>, QueryError> {
    // `components(separatedBy: .newlines)` — splitting on `\r` as well as `\n`
    // leaves an empty component between the two, which carries no `data:`
    // prefix and is dropped.
    let data_lines: Vec<&str> = event.split(['\n', '\r']).filter_map(event_data).collect();
    let payload = data_lines.join("\n");
    let payload = payload.trim();

    if payload.is_empty() {
        return Ok(None);
    }
    if payload == DONE_FLAG {
        return Ok(Some(StreamEvent::Done));
    }

    match serde_json::from_str::<ChatStreamChunk>(payload) {
        Ok(chunk) => Ok(chunk
            .choices
            .first()
            .and_then(|choice| choice.delta.content.clone())
            .map(StreamEvent::Content)),
        Err(error) => {
            // `decodeEvent(dataLines:)` retries the payload as an error object
            // and rethrows that, so an API failure mid-stream reads well.
            if let Some(message) = api_error_message(payload) {
                return Err(QueryError::with_data(ErrorType::Api, None, message));
            }
            Err(QueryError::with_data(
                ErrorType::Api,
                None,
                format!("{error}: {}", net::truncate(payload, 2048)),
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Shared provider plumbing
// ---------------------------------------------------------------------------

/// Expands one OpenAI-compatible provider.
///
/// Each provider in `OpenAIService.swift` and its subclasses only overrides
/// `defaultModels`, `defaultModel`, `defaultEndpoint`, `link()` and — for
/// Gemini — `unsupportedLanguages`; everything else is inherited from
/// `BaseOpenAIService`, which is what this macro reproduces.
macro_rules! openai_compatible_service {
    (
        $(#[$meta:meta])*
        $ty:ident {
            service: $service_type:expr,
            endpoint: $endpoint:expr,
            models: [$($model:literal),* $(,)?],
            default_model: $default_model:expr,
            link: $link:expr,
            unsupported: [$($unsupported:expr),* $(,)?],
        }
    ) => {
        $(#[$meta])*
        pub struct $ty {
            ctx: ServiceContext,
        }

        impl $ty {
            pub const SERVICE_TYPE: ServiceType = $service_type;
            pub const DEFAULT_ENDPOINT: &'static str = $endpoint;
            pub const DEFAULT_MODELS: &'static [&'static str] = &[$($model),*];
            pub const DEFAULT_MODEL: &'static str = $default_model;
            pub const LINK: Option<&'static str> = $link;
            /// `StreamService.unsupportedLanguages`
            pub const UNSUPPORTED_LANGUAGES: &'static [Language] = &[$($unsupported),*];

            pub fn new(ctx: ServiceContext) -> Self {
                $ty { ctx }
            }

            /// `StreamService.model`
            pub fn model(&self) -> String {
                resolve_model(&self.ctx, Self::DEFAULT_MODELS, Self::DEFAULT_MODEL)
            }

            /// `StreamService.endpoint`
            pub fn endpoint(&self) -> String {
                resolve_endpoint(&self.ctx, Self::DEFAULT_ENDPOINT)
            }
        }

        #[async_trait]
        impl QueryService for $ty {
            fn service_type(&self) -> ServiceType {
                Self::SERVICE_TYPE
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

            fn language_code(&self, language: Language) -> Option<String> {
                if Self::UNSUPPORTED_LANGUAGES.contains(&language) {
                    return None;
                }
                crate::service::languages::default_language_code(self.service_type(), language)
            }

            fn link(&self) -> Option<String> {
                Self::LINK.map(str::to_string)
            }

            async fn translate(
                &self,
                query: &QueryModel,
                updates: Option<UpdateSender>,
            ) -> Result<QueryResult, QueryError> {
                let endpoint = self.endpoint();
                // `validateChatStreamRequest()`
                validate_stream_request(&self.ctx, &endpoint)?;

                let messages = query_messages(&self.ctx, query);
                let transport = LlmTransport::new(
                    endpoint,
                    self.ctx.api_key(),
                    self.model(),
                    self.ctx.temperature(),
                );
                // `contentStream(messages:)` — the transport follows the
                // `enable_streaming` toggle rather than always streaming.
                let text = transport
                    .translate(
                        &messages,
                        &query.query_text,
                        self.ctx.config.hide_think_tag,
                        updates.as_ref(),
                        self.ctx.enable_streaming(),
                    )
                    .await?;

                let mut result = self.new_result(query);
                result.translated_results = Some(util::to_paragraphs(&text));
                Ok(result)
            }
        }
    };
}

/// `BaseOpenAIService.fetchRemoteModelIDs()`.
async fn openai_remote_model_ids(
    ctx: &ServiceContext,
    endpoint: &str,
    strip_models_prefix: bool,
) -> Result<Vec<String>, QueryError> {
    let api_key = ctx.api_key();
    if api_key.is_empty() {
        return Err(QueryError::with_message(
            ErrorType::MissingSecretKey,
            "API key is empty",
        ));
    }

    let url = derive_models_url(endpoint)?;
    let headers = net::headers(&[
        ("Authorization", &format!("Bearer {api_key}")),
        ("api-key", &api_key),
        ("Accept", "application/json"),
    ]);
    let data = fetch_remote_model_data(&url, headers).await?;
    let ids = decode_openai_model_list(&data)?;

    // `GeminiService.normalizedRemoteModelIDs(_:)` drops Google's `models/`
    // prefix before de-duplicating.
    let ids: Vec<String> = if strip_models_prefix {
        ids.iter()
            .map(|id| {
                let trimmed = id.trim();
                trimmed
                    .strip_prefix("models/")
                    .unwrap_or(trimmed)
                    .to_string()
            })
            .collect()
    } else {
        ids
    };

    Ok(normalized_remote_model_ids(
        &ids,
        default_remote_model_lookup_id,
    ))
}

// ---------------------------------------------------------------------------
// Providers
// ---------------------------------------------------------------------------

openai_compatible_service! {
    /// OpenAI Translate — `OpenAIService`.
    OpenAiService {
        service: ServiceType::OpenAI,
        endpoint: "https://api.openai.com/v1/chat/completions",
        models: ["gpt-5", "gpt-5-mini", "gpt-5-nano", "gpt-4.1"],
        default_model: "gpt-5-mini",
        link: Some("https://chatgpt.com"),
        unsupported: [],
    }
}

impl OpenAiService {
    /// `BaseOpenAIService.fetchRemoteModelIDs()`
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        openai_remote_model_ids(&self.ctx, &self.endpoint(), false).await
    }
}

openai_compatible_service! {
    /// Gemini Translate — `GeminiService`. Uses Google's OpenAI-compatible
    /// endpoint; `unsupportedLanguages` is `ai.google.dev/available_regions`.
    GeminiService {
        service: ServiceType::Gemini,
        endpoint: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions",
        models: ["gemini-flash-lite-latest", "gemini-flash-latest", "gemini-pro-latest"],
        default_model: "gemini-flash-lite-latest",
        link: Some("https://gemini.google.com/"),
        unsupported: [
            Language::Persian,
            Language::Filipino,
            Language::Khmer,
            Language::Lao,
            Language::Malay,
            Language::Mongolian,
            Language::Burmese,
            Language::Telugu,
            Language::Tamil,
            Language::Urdu,
        ],
    }
}

impl GeminiService {
    /// `BaseOpenAIService.fetchRemoteModelIDs()` with Gemini's `models/` strip.
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        openai_remote_model_ids(&self.ctx, &self.endpoint(), true).await
    }
}

openai_compatible_service! {
    /// Groq Translate — `GroqService`.
    GroqService {
        service: ServiceType::Groq,
        endpoint: "https://api.groq.com/openai/v1/chat/completions",
        models: [
            "openai/gpt-oss-120b",
            "openai/gpt-oss-20b",
            "llama-3.1-8b-instant",
            "moonshotai/kimi-k2-instruct-0905",
            "meta-llama/llama-4-maverick-17b-128e-instruct",
            "qwen/qwen3-32b",
        ],
        default_model: "openai/gpt-oss-120b",
        link: Some("https://groq.com/"),
        unsupported: [],
    }
}

impl GroqService {
    /// `BaseOpenAIService.fetchRemoteModelIDs()`
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        openai_remote_model_ids(&self.ctx, &self.endpoint(), false).await
    }
}

openai_compatible_service! {
    /// Zhipu Translate (智谱) — `ZhipuService`.
    ZhipuService {
        service: ServiceType::Zhipu,
        endpoint: "https://open.bigmodel.cn/api/paas/v4/chat/completions",
        models: [
            "glm-5",
            "glm-4.7",
            "glm-4.7-flash",
            "glm-4.7-flashx",
            "glm-4.6",
            "glm-4.5-air",
            "glm-4.5-airx",
            "glm-4.5-flash",
            "glm-4-flash-250414",
            "glm-4-flashx-250414",
            "glm-4.6v",
            "glm-4.6v-flash",
            "glm-4.6v-flashx",
            "glm-4v-flash",
            "glm-4.1v-thinking-flashx",
            "glm-4.1v-thinking-flash",
        ],
        default_model: "glm-4-flash-250414",
        link: Some("https://bigmodel.cn/"),
        unsupported: [],
    }
}

impl ZhipuService {
    /// `BaseOpenAIService.fetchRemoteModelIDs()`
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        openai_remote_model_ids(&self.ctx, &self.endpoint(), false).await
    }
}

openai_compatible_service! {
    /// MiniMax Translate — `MiniMaxService`.
    MiniMaxService {
        service: ServiceType::MiniMax,
        endpoint: "https://api.minimaxi.com/v1/chat/completions",
        models: [
            "MiniMax-M2.5",
            "MiniMax-M2.5-highspeed",
            "MiniMax-M2.1",
            "MiniMax-M2.1-highspeed",
            "MiniMax-M2",
        ],
        default_model: "MiniMax-M2.5",
        link: Some("https://platform.minimaxi.com/"),
        unsupported: [],
    }
}

impl MiniMaxService {
    /// `BaseOpenAIService.fetchRemoteModelIDs()`
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        openai_remote_model_ids(&self.ctx, &self.endpoint(), false).await
    }
}

openai_compatible_service! {
    /// GitHub Models — `GitHubService`.
    GitHubService {
        service: ServiceType::GitHub,
        endpoint: "https://models.github.ai/inference/chat/completions",
        models: ["gpt-4.1", "gpt-4.1-mini", "gpt-4.1-nano", "gpt-4o", "gpt-4o-mini", "deepseek-v3-0324"],
        default_model: "gpt-4.1",
        link: Some("https://github.com/marketplace/models"),
        unsupported: [],
    }
}

/// `GitHubService.remoteModelsEndpoint`
const GITHUB_CATALOG_ENDPOINT: &str = "https://models.github.ai/catalog/models";

impl GitHubService {
    /// `GitHubService.fetchRemoteModelIDs()` — the catalog lists far more than
    /// the chat endpoint can serve, so only text-generation models are kept.
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        let api_key = self.ctx.api_key();
        if api_key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                "GitHub Models token is empty",
            ));
        }

        let headers = net::headers(&[
            ("Authorization", &format!("Bearer {api_key}")),
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2026-03-10"),
        ]);
        let data = fetch_remote_model_data(GITHUB_CATALOG_ENDPOINT, headers).await?;
        let models: Vec<GitHubCatalogModel> = serde_json::from_slice(&data)
            .map_err(|_| QueryError::with_message(ErrorType::Api, "Invalid models response"))?;

        let ids: Vec<String> = models
            .into_iter()
            .filter(GitHubCatalogModel::supports_text_generation)
            .map(|model| model.id)
            .collect();
        Ok(normalized_remote_model_ids(
            &ids,
            github_remote_model_lookup_id,
        ))
    }
}

/// `GitHubCatalogModel`
#[derive(Debug, Deserialize)]
struct GitHubCatalogModel {
    id: String,
    #[serde(default)]
    supported_input_modalities: Option<Vec<String>>,
    #[serde(default)]
    supported_output_modalities: Option<Vec<String>>,
}

impl GitHubCatalogModel {
    /// `supportsTextGeneration` — a missing modality list means "text only".
    fn supports_text_generation(&self) -> bool {
        let supports_text_input = self
            .supported_input_modalities
            .as_ref()
            .is_none_or(|modalities| modalities.iter().any(|m| m == "text"));
        let supports_text_output = self
            .supported_output_modalities
            .as_ref()
            .is_none_or(|modalities| modalities.iter().any(|m| m == "text"));
        supports_text_input && supports_text_output
    }
}

openai_compatible_service! {
    /// Ollama Translate — `OllamaService`. Runs against a local server, so it
    /// needs no API key (`apiKeyRequirement() == .none`) and its model list is
    /// whatever the user has pulled.
    OllamaService {
        service: ServiceType::Ollama,
        endpoint: "http://localhost:11434/v1/chat/completions",
        models: [],
        default_model: "",
        link: None,
        unsupported: [],
    }
}

impl OllamaService {
    /// `OllamaService.fetchRemoteModelIDs()` — `GET <root>/api/tags`.
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        let url = local_models_url(&self.endpoint())?;
        let data = fetch_remote_model_data(&url, HeaderMap::new()).await?;
        let models: OllamaModels = serde_json::from_slice(&data)
            .map_err(|error| QueryError::with_message(ErrorType::Api, error.to_string()))?;
        let ids: Vec<String> = models
            .models
            .into_iter()
            .map(|model| model.name)
            .collect();
        Ok(normalized_remote_model_ids(
            &ids,
            default_remote_model_lookup_id,
        ))
    }
}

/// `OllamaService.localModels()` — the chat endpoint is
/// `http://localhost:11434/v1/chat/completions`, the tags endpoint is
/// `http://localhost:11434/api/tags`.
pub fn local_models_url(endpoint: &str) -> Result<String, QueryError> {
    let mut url = reqwest::Url::parse(endpoint.trim()).map_err(|_| {
        QueryError::with_message(
            ErrorType::Parameter,
            format!("`{}` endpoint is invalid", ServiceType::Ollama.raw()),
        )
    })?;
    url.set_path("/api/tags");
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

/// `OllamaModels`
#[derive(Debug, Deserialize)]
struct OllamaModels {
    models: Vec<OllamaModel>,
}

/// `OllamaModel` — upstream decodes `size`, `digest` and `details` as well;
/// only `name` is ever read here.
#[derive(Debug, Deserialize)]
struct OllamaModel {
    name: String,
}

// ---------------------------------------------------------------------------
// Custom OpenAI
// ---------------------------------------------------------------------------

/// Custom OpenAI — `CustomOpenAIService`.
///
/// Endpoint and model are free-form: the user supplies both, so there are no
/// defaults to fall back on.
pub struct CustomOpenAiService {
    ctx: ServiceContext,
}

impl CustomOpenAiService {
    pub fn new(ctx: ServiceContext) -> Self {
        CustomOpenAiService { ctx }
    }

    /// `defaultModels` is `[""]` upstream — the configured value wins, and an
    /// empty model stays empty rather than falling back to a provider default.
    pub fn model(&self) -> String {
        let configured = self.ctx.config.model.trim();
        if !configured.is_empty() {
            return configured.to_string();
        }
        self.ctx
            .config
            .valid_models
            .first()
            .map(|model| model.trim().to_string())
            .unwrap_or_default()
    }

    /// `StreamService.endpoint` with `defaultEndpoint == ""`.
    pub fn endpoint(&self) -> String {
        self.ctx.config.endpoint.trim().to_string()
    }
}

#[async_trait]
impl QueryService for CustomOpenAiService {
    fn service_type(&self) -> ServiceType {
        ServiceType::CustomOpenAI
    }

    /// `CustomOpenAIService.name()` — the user's custom name when set.
    fn name(&self) -> String {
        let name = self.ctx.config.name.trim();
        if name.is_empty() {
            ServiceType::CustomOpenAI.default_name().to_string()
        } else {
            name.to_string()
        }
    }

    /// `serviceTypeWithUniqueIdentifier()` — `CustomOpenAI#<uuid>`.
    fn identifier(&self) -> &str {
        &self.ctx.identifier
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        supported_query_type(&self.ctx.config)
    }

    async fn translate(
        &self,
        query: &QueryModel,
        updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let endpoint = self.endpoint();
        validate_stream_request(&self.ctx, &endpoint)?;

        let messages = query_messages(&self.ctx, query);
        let transport = LlmTransport::new(
            endpoint,
            self.ctx.api_key(),
            self.model(),
            self.ctx.temperature(),
        );
        let text = transport
            .translate(
                &messages,
                &query.query_text,
                self.ctx.config.hide_think_tag,
                updates.as_ref(),
                self.ctx.enable_streaming(),
            )
            .await?;

        let mut result = self.new_result(query);
        result.translated_results = Some(util::to_paragraphs(&text));
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Built-In AI
// ---------------------------------------------------------------------------

/// Built-In AI Translate — `BuiltInAIService`.
///
/// **Deliberate, documented deviation.** Upstream's `BuiltInAIService` reads a
/// vendor-supplied key and endpoint from the app's `EncryptedSecretKeys.plist`
/// (`builtInAIAPIKey`, `builtInAIEndpoint`), which the Windows port does not
/// ship and must not ship. This port therefore has no bundled credential: when
/// no key is configured, `translate` fails with
/// [`ErrorType::MissingSecretKey`](crate::model::ErrorType::MissingSecretKey)
/// and points the user at `Settings::services["BuiltInAI"].api_key`. Once a key
/// (and, for a non-Zhipu provider, an endpoint) is configured there, the
/// service behaves like any other OpenAI-compatible provider.
///
/// The default endpoint and model list mirror the Zhipu models upstream uses as
/// its defaults; `builtInAIEndpoint` itself is unknown, so the Zhipu endpoint is
/// the port's documented stand-in.
pub struct BuiltInAiService {
    ctx: ServiceContext,
}

impl BuiltInAiService {
    pub const DEFAULT_ENDPOINT: &'static str = "https://open.bigmodel.cn/api/paas/v4/chat/completions";
    /// `BuiltInAIService.defaultModels` — also the source of the upstream
    /// "Supported Models" list, which the user cannot edit.
    pub const DEFAULT_MODELS: &'static [&'static str] =
        &["glm-4.5-flash", "glm-4-flash-250414", "llama-3.1-8b-instant"];
    pub const DEFAULT_MODEL: &'static str = "glm-4-flash-250414";

    /// The message shown when the port has nothing to authenticate with.
    pub const MISSING_KEY_MESSAGE: &'static str = "The Windows port does not ship a built-in API key for Built-In AI. Add your own key under Settings → Services → Built-In AI (Settings::services[\"BuiltInAI\"].api_key) and set the endpoint to the OpenAI-compatible provider you want to use.";

    pub fn new(ctx: ServiceContext) -> Self {
        BuiltInAiService { ctx }
    }

    /// `StreamService.model`
    pub fn model(&self) -> String {
        resolve_model(&self.ctx, Self::DEFAULT_MODELS, Self::DEFAULT_MODEL)
    }

    /// `StreamService.endpoint`
    pub fn endpoint(&self) -> String {
        resolve_endpoint(&self.ctx, Self::DEFAULT_ENDPOINT)
    }
}

#[async_trait]
impl QueryService for BuiltInAiService {
    fn service_type(&self) -> ServiceType {
        ServiceType::BuiltInAI
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

    /// `BuiltInAIService.link()` — upstream does not override `link()`.
    fn link(&self) -> Option<String> {
        None
    }

    async fn translate(
        &self,
        query: &QueryModel,
        updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let endpoint = self.endpoint();
        validate_stream_request(&self.ctx, &endpoint)?;

        // `apiKeyRequirement() == .builtIn`: the shared check above only covers
        // user-provided keys, so the vendored-key substitute is checked here.
        if self.ctx.api_key().is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                Self::MISSING_KEY_MESSAGE,
            ));
        }

        let messages = query_messages(&self.ctx, query);
        let transport = LlmTransport::new(
            endpoint,
            self.ctx.api_key(),
            self.model(),
            self.ctx.temperature(),
        );
        let text = transport
            .translate(
                &messages,
                &query.query_text,
                self.ctx.config.hide_think_tag,
                updates.as_ref(),
                self.ctx.enable_streaming(),
            )
            .await?;

        let mut result = self.new_result(query);
        result.translated_results = Some(util::to_paragraphs(&text));
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ServiceConfig, Settings};
    use std::sync::Arc;

    fn ctx(service_type: ServiceType) -> ServiceContext {
        ServiceContext::new(Arc::new(Settings::default()), service_type, service_type.raw())
    }

    fn settings_with_key(service_type: ServiceType) -> ServiceContext {
        let mut settings = Settings::default();
        settings
            .services
            .entry(service_type.raw().to_string())
            .or_default()
            .api_key = "key".to_string();
        ServiceContext::new(Arc::new(settings), service_type, service_type.raw())
    }

    #[test]
    fn sse_framing_cuts_on_a_delimiter_suffix_only() {
        let mut buffer = Vec::new();
        // A buffer that merely *contains* a delimiter is not cut.
        buffer.extend_from_slice(b"data: a\n\ndata: b");
        assert!(take_completed_event(&mut buffer).is_none());
        assert_eq!(buffer, b"data: a\n\ndata: b");

        // Only once the suffix is a delimiter does the event come out whole.
        buffer.extend_from_slice(b"\n\n");
        assert_eq!(
            take_completed_event(&mut buffer).unwrap(),
            b"data: a\n\ndata: b".to_vec()
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn crlf_framing_is_supported() {
        let mut buffer = Vec::new();
        // This is the case a `find("\n\n")` scan would miss entirely: the
        // delimiter is `\r\n\r\n`, which contains no `\n\n`.
        buffer.extend_from_slice(b"data: x\r\n\r\n");
        assert_eq!(take_completed_event(&mut buffer).unwrap(), b"data: x".to_vec());
        assert!(buffer.is_empty());

        buffer.extend_from_slice(b"data: y\r\r");
        assert_eq!(take_completed_event(&mut buffer).unwrap(), b"data: y".to_vec());
    }

    #[test]
    fn events_are_cut_one_at_a_time_as_bytes_arrive() {
        // Feed the stream byte by byte, the way `completedEvent(from:)` is
        // called by `stream(query:url:apiKey:onResult:)`.
        let stream = b"data: one\n\ndata: two\n\n";
        let mut buffer = Vec::new();
        let mut events: Vec<String> = Vec::new();
        for byte in stream {
            buffer.push(*byte);
            if let Some(event) = take_completed_event(&mut buffer) {
                events.push(String::from_utf8(event).unwrap());
            }
        }
        assert_eq!(events, vec!["data: one".to_string(), "data: two".to_string()]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn data_lines_are_detected_by_a_strict_prefix() {
        assert_eq!(event_data("data: hello "), Some("hello"));
        assert_eq!(event_data("data:hello"), Some("hello"));
        // Leading whitespace means the line is not a `data:` line at all.
        assert_eq!(event_data(" data: hello"), None);
        assert_eq!(event_data("event: message"), None);
        assert_eq!(event_data(""), None);
    }

    #[test]
    fn done_sentinel_terminates_the_stream() {
        let event = decode_event("data: [DONE]").unwrap();
        assert_eq!(event, Some(StreamEvent::Done));
        // Trailing whitespace is trimmed before the comparison.
        assert_eq!(
            decode_event("data: [DONE] \n").unwrap(),
            Some(StreamEvent::Done)
        );
        // An empty event yields nothing.
        assert_eq!(decode_event("event: ping").unwrap(), None);
        assert_eq!(decode_event("data: ").unwrap(), None);
    }

    #[test]
    fn multiple_data_lines_are_joined_with_newlines() {
        // A payload split over two `data:` lines — with CRLF framing the blank
        // component between `\r` and `\n` is ignored.
        let event = "data: {\"id\": \"1\", \"object\": \"chat.completion.chunk\",\r\ndata: \"created\": 1, \"model\": \"m\", \"choices\": []}";
        assert_eq!(
            decode_event(event).unwrap(),
            None,
            "choices is empty, so there is no content to yield"
        );

        let event = "data: {\"id\": \"1\", \"object\": \"chat.completion.chunk\",\ndata: \"created\": 1, \"model\": \"m\", \"choices\": [{\"index\": 0, \"delta\": {\"content\": \"hi\"}}]}";
        assert_eq!(
            decode_event(event).unwrap(),
            Some(StreamEvent::Content("hi".to_string()))
        );
    }

    #[test]
    fn chunk_decoding_is_strict() {
        let payload = r#"{"id":"chatcmpl-1","object":"chat.completion.chunk","created":1694268190,"model":"gpt-4o-mini","choices":[{"index":0,"delta":{"content":"Hel"},"finish_reason":null}]}"#;
        let chunk: ChatStreamChunk = serde_json::from_str(payload).unwrap();
        assert_eq!(chunk.id, "chatcmpl-1");
        assert_eq!(chunk.object, "chat.completion.chunk");
        assert_eq!(chunk.created, 1694268190);
        assert_eq!(chunk.model, "gpt-4o-mini");
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hel"));

        // A delta without content (the final chunk) decodes to no text.
        assert_eq!(
            decode_event("data: {\"id\":\"1\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}").unwrap(),
            None
        );

        // Each of the five required fields is load-bearing.
        for missing in ["id", "object", "created", "model", "choices"] {
            let mut value: serde_json::Value = serde_json::from_str(payload).unwrap();
            value.as_object_mut().unwrap().remove(missing);
            let event = format!("data: {value}");
            let error = decode_event(&event).unwrap_err();
            assert_eq!(error.kind, ErrorType::Api, "missing {missing}");
        }
    }

    #[test]
    fn error_chunks_surface_the_api_message() {
        let error = decode_event(r#"data: {"error":{"message":"rate limited"}}"#).unwrap_err();
        assert_eq!(error.kind, ErrorType::Api);
        assert_eq!(error.error_data_message.as_deref(), Some("rate limited"));
        assert_eq!(error.message, None);
    }

    #[test]
    fn provider_defaults_match_upstream() {
        assert_eq!(
            OpenAiService::DEFAULT_ENDPOINT,
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(OpenAiService::DEFAULT_MODEL, "gpt-5-mini");
        assert_eq!(OpenAiService::DEFAULT_MODELS.len(), 4);

        assert_eq!(
            GeminiService::DEFAULT_ENDPOINT,
            "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
        );
        assert_eq!(GeminiService::DEFAULT_MODEL, "gemini-flash-lite-latest");

        assert_eq!(
            GroqService::DEFAULT_ENDPOINT,
            "https://api.groq.com/openai/v1/chat/completions"
        );
        assert_eq!(GroqService::DEFAULT_MODEL, "openai/gpt-oss-120b");

        assert_eq!(
            ZhipuService::DEFAULT_ENDPOINT,
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(ZhipuService::DEFAULT_MODEL, "glm-4-flash-250414");
        assert_eq!(ZhipuService::DEFAULT_MODELS.len(), 16);

        assert_eq!(
            MiniMaxService::DEFAULT_ENDPOINT,
            "https://api.minimaxi.com/v1/chat/completions"
        );
        assert_eq!(MiniMaxService::DEFAULT_MODEL, "MiniMax-M2.5");

        assert_eq!(
            GitHubService::DEFAULT_ENDPOINT,
            "https://models.github.ai/inference/chat/completions"
        );
        assert_eq!(GitHubService::DEFAULT_MODEL, "gpt-4.1");

        assert_eq!(
            OllamaService::DEFAULT_ENDPOINT,
            "http://localhost:11434/v1/chat/completions"
        );
        // Ollama's model list is whatever the local server reports.
        assert!(OllamaService::DEFAULT_MODELS.is_empty());
    }

    #[test]
    fn model_selection_follows_the_configured_value() {
        let svc = OpenAiService::new(ctx(ServiceType::OpenAI));
        // `OpenAIService.defaultModel` is `gpt-5-mini`, which the model key
        // stores as its default, so "gpt-5" is not the unset value.
        assert_eq!(svc.model(), "gpt-5-mini");

        let mut context = settings_with_key(ServiceType::OpenAI);
        context.config.model = "gpt-4.1".to_string();
        assert_eq!(OpenAiService::new(context).model(), "gpt-4.1");

        let mut context = settings_with_key(ServiceType::OpenAI);
        context.config.model = "nope".to_string();
        assert_eq!(OpenAiService::new(context).model(), "gpt-5");
    }

    #[test]
    fn configured_endpoint_wins_over_the_default() {
        let mut context = settings_with_key(ServiceType::Groq);
        assert_eq!(
            GroqService::new(context.clone()).endpoint(),
            "https://api.groq.com/openai/v1/chat/completions"
        );
        context.config.endpoint = "https://proxy.test/v1/chat/completions".to_string();
        assert_eq!(
            GroqService::new(context).endpoint(),
            "https://proxy.test/v1/chat/completions"
        );
    }

    #[test]
    fn query_type_toggles_and_links_come_from_the_provider_table() {
        let svc = GroqService::new(settings_with_key(ServiceType::Groq));
        assert_eq!(svc.supported_query_type(), QueryTextType::TRANSLATION);
        assert!(svc.is_stream());
        assert_eq!(svc.link().as_deref(), Some("https://groq.com/"));
        assert_eq!(svc.identifier(), "Groq");
        assert_eq!(svc.name(), "Groq Translate");

        // Gemini has no link override beyond its own, and rejects its
        // unsupported regions.
        let gemini = GeminiService::new(settings_with_key(ServiceType::Gemini));
        assert_eq!(gemini.language_code(Language::Persian), None);
        assert_eq!(gemini.language_code(Language::English).as_deref(), Some("English"));
        assert!(!gemini.supported_languages().contains(&Language::Persian));
        assert!(gemini.supported_languages().contains(&Language::English));

        // Dictionary and sentence toggles widen the supported query types.
        let mut context = settings_with_key(ServiceType::Zhipu);
        context.config.sentence = true;
        context.config.dictionary = true;
        assert_eq!(
            ZhipuService::new(context).supported_query_type(),
            QueryTextType::INTELLIGENT
        );
    }

    #[test]
    fn ollama_needs_no_api_key() {
        let ollama = OllamaService::new(ctx(ServiceType::Ollama));
        assert!(!ollama.has_private_api_key());
        assert!(validate_stream_request(&ollama.ctx, &ollama.endpoint()).is_ok());
        assert_eq!(ollama.model(), "");

        // Every other OpenAI-compatible service does need one.
        let openai = OpenAiService::new(ctx(ServiceType::OpenAI));
        assert_eq!(
            validate_stream_request(&openai.ctx, &openai.endpoint())
                .unwrap_err()
                .kind,
            ErrorType::MissingSecretKey
        );
    }

    #[test]
    fn custom_openai_uses_the_configured_name_endpoint_and_model() {
        let mut settings = Settings::default();
        let config = settings.service_mut("CustomOpenAI#abc");
        config.name = "My Proxy".to_string();
        config.endpoint = "https://proxy.test/v1/chat/completions".to_string();
        config.model = "my-model".to_string();
        config.api_key = "key".to_string();

        let svc = CustomOpenAiService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::CustomOpenAI,
            "CustomOpenAI#abc",
        ));
        assert_eq!(svc.name(), "My Proxy");
        assert_eq!(svc.identifier(), "CustomOpenAI#abc");
        assert_eq!(svc.endpoint(), "https://proxy.test/v1/chat/completions");
        assert_eq!(svc.model(), "my-model");
        assert_eq!(svc.service_type(), ServiceType::CustomOpenAI);

        // Without a configured name the localized default stands in, and the
        // model stays empty rather than borrowing another provider's default.
        let svc = CustomOpenAiService::new(ctx(ServiceType::CustomOpenAI));
        assert_eq!(svc.name(), "Custom OpenAI");
        assert_eq!(svc.endpoint(), "");
        assert_eq!(svc.model(), "");
    }

    #[test]
    fn built_in_ai_explains_the_missing_bundled_key() {
        let svc = BuiltInAiService::new(ctx(ServiceType::BuiltInAI));
        assert_eq!(
            BuiltInAiService::DEFAULT_ENDPOINT,
            "https://open.bigmodel.cn/api/paas/v4/chat/completions"
        );
        assert_eq!(BuiltInAiService::DEFAULT_MODEL, "glm-4-flash-250414");
        assert_eq!(BuiltInAiService::DEFAULT_MODELS.len(), 3);
        assert_eq!(svc.model(), "glm-4-flash-250414");
        // `apiKeyRequirement() == .builtIn`, so the shared prehandle check does
        // not reject it — the message comes from `translate` itself.
        assert_eq!(
            ServiceType::BuiltInAI.api_key_requirement(),
            crate::model::ApiKeyRequirement::BuiltIn
        );
        assert!(BuiltInAiService::MISSING_KEY_MESSAGE.contains("services[\"BuiltInAI\"].api_key"));
    }

    #[test]
    fn github_catalog_filtering_and_dedup_key() {
        let text_only = GitHubCatalogModel {
            id: "openai/gpt-4.1".to_string(),
            supported_input_modalities: Some(vec!["text".to_string()]),
            supported_output_modalities: Some(vec!["text".to_string()]),
        };
        assert!(text_only.supports_text_generation());

        // Missing modality lists mean "text only", as upstream defaults them.
        let unlisted = GitHubCatalogModel {
            id: "openai/gpt-4.1".to_string(),
            supported_input_modalities: None,
            supported_output_modalities: None,
        };
        assert!(unlisted.supports_text_generation());

        let image_only = GitHubCatalogModel {
            id: "x/image-model".to_string(),
            supported_input_modalities: Some(vec!["image".to_string()]),
            supported_output_modalities: Some(vec!["image".to_string()]),
        };
        assert!(!image_only.supports_text_generation());

        assert_eq!(github_remote_model_lookup_id("openai/gpt-4.1"), "gpt-4.1");
        assert_eq!(github_remote_model_lookup_id("gpt-4.1"), "gpt-4.1");
    }

    #[test]
    fn ollama_tags_url_derives_from_the_chat_endpoint() {
        assert_eq!(
            local_models_url("http://localhost:11434/v1/chat/completions").unwrap(),
            "http://localhost:11434/api/tags"
        );
        // A custom host and port are preserved.
        assert_eq!(
            local_models_url("http://192.168.1.5:8080/v1/chat/completions").unwrap(),
            "http://192.168.1.5:8080/api/tags"
        );
        assert!(local_models_url("not a url").is_err());
    }

    #[test]
    fn custom_prompt_settings_reach_the_transport_messages() {
        let mut context = settings_with_key(ServiceType::OpenAI);
        context.config.enable_custom_prompt = true;
        context.config.system_prompt = "You translate into ${{queryTargetLanguage}}".to_string();
        context.config.user_prompt = "Translate: ${{queryText}}".to_string();

        let mut query = QueryModel::new("hello");
        query.query_from_language = Language::English;
        query.query_target_language = Language::SimplifiedChinese;

        let messages = query_messages(&context, &query);
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].content, "You translate into Simplified-Chinese");
        assert_eq!(messages[1].content, "Translate: hello");
    }

    #[test]
    fn dict_and_sentence_routing_influences_the_built_in_prompt() {
        let context = settings_with_key(ServiceType::OpenAI);
        let mut query = QueryModel::new("apple");
        query.query_from_language = Language::English;
        query.query_target_language = Language::SimplifiedChinese;
        query.query_type = QueryTextType::DICTIONARY;

        let messages = query_messages(&context, &query);
        let last = messages.last().unwrap();
        assert!(last.content.starts_with("Using Simplified-Chinese: \nHere is a English word: \"\"\"apple\"\"\", "));

        // The default translation toggle sends the translation prompt.
        let mut query = QueryModel::new("apple");
        query.query_from_language = Language::English;
        query.query_target_language = Language::SimplifiedChinese;
        let messages = query_messages(&context, &query);
        assert_eq!(
            messages.last().unwrap().content,
            "Translate the following English text into 简体中文白话文 text: \"\"\"apple\"\"\""
        );
    }

    #[test]
    fn settings_round_trip_for_llm_services() {
        // A service configuration default keeps the LLM-streaming defaults.
        let config = ServiceConfig::default();
        assert!(config.enable_streaming);
        assert!(config.hide_think_tag);
        assert_eq!(config.temperature, 0.3);
        assert!(!config.enable_custom_prompt);
    }
}
