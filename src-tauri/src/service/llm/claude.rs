//! Claude Translate — `ClaudeService.swift`, `ClaudeSSEParser.swift` and
//! `ClaudeResponse.swift`.
//!
//! Claude speaks the Anthropic Messages API rather than the OpenAI chat
//! completions shape: `x-api-key` instead of a bearer token, a top-level
//! `system` string instead of a system message, and its own SSE parser.

use std::collections::VecDeque;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::model::{ErrorType, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType};
use crate::net;
use crate::service::llm::prompts::{ChatMessage, ChatRole};
use crate::service::llm::{
    fetch_remote_model_data, handle_stream_error, normalized_remote_model_ids, query_messages,
    resolve_endpoint, resolve_model, supported_query_type, ChunkThrottle,
};
use crate::service::stream;
use crate::service::{QueryService, ServiceContext, ServiceUpdate, UpdateSender};
use crate::util;

/// `ClaudeService.anthropicVersion`
pub const ANTHROPIC_VERSION: &str = "2023-06-01";
/// `ClaudeService.maxTokens`
pub const MAX_TOKENS: u32 = 4096;
/// `processStreamBytes` — `bufferThreshold`
const BUFFER_THRESHOLD: usize = 1024;
/// `validateHTTPResponse` reads at most 4 KB of the error body.
const MAX_ERROR_BODY_SIZE: usize = 4096;
/// `ClaudeService.defaultModels`
pub const DEFAULT_MODELS: &[&str] = &[
    "claude-sonnet-4-6",
    "claude-haiku-4-5",
    "claude-opus-4-6",
    "claude-opus-4-7",
];
/// `ClaudeService.defaultModel`
pub const DEFAULT_MODEL: &str = "claude-sonnet-4-6";
/// `ClaudeService.defaultEndpoint`
pub const DEFAULT_ENDPOINT: &str = "https://api.anthropic.com/v1/messages";

/// Claude Translate.
pub struct ClaudeService {
    ctx: ServiceContext,
}

impl ClaudeService {
    pub fn new(ctx: ServiceContext) -> Self {
        ClaudeService { ctx }
    }

    /// `StreamService.model`
    pub fn model(&self) -> String {
        resolve_model(&self.ctx, DEFAULT_MODELS, DEFAULT_MODEL)
    }

    /// `StreamService.endpoint`
    pub fn endpoint(&self) -> String {
        resolve_endpoint(&self.ctx, DEFAULT_ENDPOINT)
    }

    /// `ClaudeService.fetchRemoteModelIDs()` — paginated, 1000 at a time.
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        let api_key = self.ctx.api_key();
        if api_key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                "Claude API key is empty.",
            ));
        }

        let mut ids: Vec<String> = Vec::new();
        let mut after_id: Option<String> = None;
        loop {
            let url = claude_models_url(&self.endpoint(), after_id.as_deref())?;
            let headers = net::headers(&[
                ("x-api-key", &api_key),
                ("anthropic-version", ANTHROPIC_VERSION),
                ("Accept", "application/json"),
            ]);
            let data = fetch_remote_model_data(&url, headers).await?;
            let page: ClaudeModelList = serde_json::from_slice(&data)
                .map_err(|_| QueryError::with_message(ErrorType::Api, "Invalid models response"))?;

            ids.extend(page.data.into_iter().map(|model| model.id));
            after_id = if page.has_more { page.last_id } else { None };
            if after_id.as_ref().is_none_or(|id| id.is_empty()) {
                break;
            }
        }

        Ok(normalized_remote_model_ids(
            &ids,
            crate::service::llm::default_remote_model_lookup_id,
        ))
    }

    /// `createURLRequest(body:)` plus `buildRequestBody(systemPrompt:messages:)`
    /// — the POST both transports send. `stream` is the only field that
    /// differs between them.
    fn request(
        &self,
        query: &QueryModel,
        stream: bool,
    ) -> Result<reqwest::RequestBuilder, QueryError> {
        let endpoint = self.endpoint();
        let url = reqwest::Url::parse(endpoint.trim()).map_err(|_| {
            QueryError::with_message(
                ErrorType::Api,
                format!("Invalid Claude API endpoint: {endpoint}"),
            )
        })?;

        let api_key = self.ctx.api_key();
        if api_key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                "Claude API key is empty.",
            ));
        }

        let messages = query_messages(&self.ctx, query);
        let (system_prompt, user_messages) = separate_system_messages(&messages);
        let body = ClaudeRequest {
            model: self.model(),
            max_tokens: MAX_TOKENS,
            temperature: claude_temperature(self.ctx.temperature()),
            stream,
            messages: &user_messages,
            system: if system_prompt.is_empty() {
                None
            } else {
                Some(system_prompt)
            },
        };

        Ok(net::LLM_CLIENT
            .post(url)
            .header("Content-Type", "application/json")
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body))
    }

    /// The streaming transport — `contentStreamTranslate(_:from:to:)` and
    /// `processStreamBytes(_:continuation:)`.
    async fn stream(
        &self,
        query: &QueryModel,
        updates: Option<&UpdateSender>,
    ) -> Result<String, QueryError> {
        let response = self
            .request(query, true)?
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        if !status.is_success() {
            return Err(claude_http_error(response).await);
        }

        let mut reader = ClaudeSseReader::new(response);
        let mut accumulated = String::new();
        let mut throttle = ChunkThrottle::new();

        let stream_error = loop {
            match reader.next_event().await {
                Ok(Some(text)) => {
                    accumulated.push_str(&text);
                    throttle.push(&accumulated, updates);
                }
                Ok(None) => break None,
                Err(error) => break Some(error),
            }
        };

        if let Some(error) = stream_error {
            handle_stream_error(&accumulated, error)?;
        }

        throttle.flush(&accumulated, updates);
        crate::service::llm::final_text(
            &accumulated,
            &query.query_text,
            self.ctx.config.hide_think_tag,
        )
    }

    /// The non-streaming transport: the same body with `stream: false`.
    ///
    /// Upstream's Claude service has no non-streaming branch — it hard-codes
    /// `"stream": true` in `buildRequestBody` — so there is no Swift symbol to
    /// reproduce here. The answer is the documented Messages response, whose
    /// shape is Claude's own: **not** `choices[0].message.content`, but a list
    /// of content blocks. `validateHTTPResponse(_:asyncBytes:)` still owns the
    /// error path, so a rejected request reports the API's own message.
    async fn non_streaming(
        &self,
        query: &QueryModel,
        updates: Option<&UpdateSender>,
    ) -> Result<String, QueryError> {
        let response = self
            .request(query, false)?
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        if !status.is_success() {
            return Err(claude_http_error(response).await);
        }

        let bytes = response.bytes().await.unwrap_or_default();
        let text = decode_message_response(&bytes)?.text();
        if text.is_empty() {
            return Err(stream::no_result_error());
        }

        if let Some(tx) = updates {
            let _ = tx.send(ServiceUpdate::Chunk(text.clone()));
        }
        crate::service::llm::final_text(
            &text,
            &query.query_text,
            self.ctx.config.hide_think_tag,
        )
    }
}

#[async_trait]
impl QueryService for ClaudeService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Claude
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
        Some("https://claude.ai/".to_string())
    }

    async fn translate(
        &self,
        query: &QueryModel,
        updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        // The `enable_streaming` setting picks the transport;
        // `QueryService::is_stream` stays `true` either way, because it
        // describes capability rather than this request's transport.
        let text = if self.ctx.enable_streaming() {
            self.stream(query, updates.as_ref()).await?
        } else {
            self.non_streaming(query, updates.as_ref()).await?
        };
        let mut result = self.new_result(query);
        result.translated_results = Some(util::to_paragraphs(&text));
        Ok(result)
    }
}

/// `min(max(temperature, 0), 1)` — the settings UI caps Claude at 1.
pub fn claude_temperature(temperature: f64) -> f64 {
    temperature.clamp(0.0, 1.0)
}

/// `separateSystemMessages(_:)` — Anthropic wants the system prompt as a
/// top-level parameter, not as a message.
pub fn separate_system_messages(messages: &[ChatMessage]) -> (String, Vec<ChatMessage>) {
    let mut system_parts: Vec<String> = Vec::new();
    let mut other_messages: Vec<ChatMessage> = Vec::new();
    for message in messages {
        if message.role == ChatRole::System {
            system_parts.push(message.content.clone());
        } else {
            other_messages.push(message.clone());
        }
    }
    (system_parts.join("\n\n"), other_messages)
}

/// `ClaudeService.buildRequestBody(systemPrompt:messages:)`
#[derive(Debug, Serialize)]
struct ClaudeRequest<'a> {
    model: String,
    max_tokens: u32,
    temperature: f64,
    stream: bool,
    messages: &'a [ChatMessage],
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
}

/// `validateHTTPResponse(_:asyncBytes:)` — a non-2xx body is read (up to 4 KB)
/// so the API's own message can be shown.
async fn claude_http_error(response: reqwest::Response) -> QueryError {
    let status = response.status();
    let bytes = response.bytes().await.unwrap_or_default();
    let end = bytes.len().min(MAX_ERROR_BODY_SIZE);
    let error_data = &bytes[..end];

    let mut error_message = format!("Claude API error: HTTP {}", status.as_u16());
    if let Ok(stream_error) = serde_json::from_slice::<ClaudeStreamError>(error_data) {
        error_message = stream_error.error.message;
    } else if let Ok(body_string) = std::str::from_utf8(error_data) {
        if !body_string.is_empty() {
            error_message = body_string.to_string();
        }
    }

    QueryError::with_data(ErrorType::Api, None, error_message)
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

/// `ClaudeStreamDelta` — a `content_block_delta` event.
#[derive(Debug, Deserialize)]
pub struct ClaudeStreamDelta {
    /// The event's own `type` field; required by upstream's `Codable`.
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub kind: String,
    #[allow(dead_code)]
    pub index: Option<i64>,
    pub delta: Option<ClaudeDeltaContent>,
}

/// `ClaudeStreamDelta.DeltaContent`
#[derive(Debug, Deserialize)]
pub struct ClaudeDeltaContent {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub kind: String,
    pub text: Option<String>,
}

/// `ClaudeStreamError` — an `error` event.
#[derive(Debug, Deserialize)]
pub struct ClaudeStreamError {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub kind: String,
    pub error: ClaudeErrorDetail,
}

/// `ClaudeStreamError.ErrorDetail`
#[derive(Debug, Deserialize)]
pub struct ClaudeErrorDetail {
    #[serde(rename = "type")]
    #[allow(dead_code)]
    pub kind: String,
    pub message: String,
}

// ---------------------------------------------------------------------------
// Non-streaming response
// ---------------------------------------------------------------------------

/// The Messages response with `stream: false`.
///
/// This is where Claude's shape parts company with the OpenAI-compatible
/// providers: the answer is not `choices[0].message.content` but a list of
/// content blocks, and a response may carry more than one. There is no Swift
/// symbol to reproduce — `ClaudeService` always streams — so the shape follows
/// the Messages API itself.
#[derive(Debug, Deserialize)]
pub struct ClaudeMessageResponse {
    pub content: Vec<ClaudeContentBlock>,
}

/// `ClaudeMessageResponse.Content` — only `text` blocks carry `text`.
#[derive(Debug, Deserialize)]
pub struct ClaudeContentBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub text: Option<String>,
}

impl ClaudeMessageResponse {
    /// The text of every `text` block, in the order the API returned them.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter(|block| block.kind == "text")
            .filter_map(|block| block.text.as_deref())
            .collect()
    }
}

/// Decodes a non-streaming Messages response. An undecodable body is an `api`
/// error carrying the decoder's message, as in the OpenAI-compatible decoder.
pub fn decode_message_response(body: &[u8]) -> Result<ClaudeMessageResponse, QueryError> {
    serde_json::from_slice(body).map_err(|error| {
        QueryError::with_data(
            ErrorType::Api,
            None,
            format!(
                "{error}: {}",
                net::truncate(&String::from_utf8_lossy(body), 2048)
            ),
        )
    })
}

/// `ClaudeModelListResponse`
#[derive(Debug, Deserialize)]
struct ClaudeModelList {
    data: Vec<ClaudeRemoteModel>,
    has_more: bool,
    #[serde(default)]
    last_id: Option<String>,
}

/// `ClaudeRemoteModel`
#[derive(Debug, Deserialize)]
struct ClaudeRemoteModel {
    id: String,
}

/// `ClaudeService.remoteModelsURL(afterID:)` — `/messages` becomes `/models`.
pub fn claude_models_url(endpoint: &str, after_id: Option<&str>) -> Result<String, QueryError> {
    let invalid = || QueryError::with_message(ErrorType::Parameter, "Claude endpoint is invalid");
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
    if parts.is_empty() {
        parts.push("v1".to_string());
    } else if matches!(
        parts.last().map(|part| part.to_lowercase()).as_deref(),
        Some("messages") | Some("models")
    ) {
        parts.pop();
    }
    parts.push("models".to_string());

    // Paging parameters are replaced rather than appended to, so a caller
    // cannot end up with two `limit` values.
    let mut query: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(name, _)| name != "limit" && name != "after_id")
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect();
    query.push(("limit".to_string(), "1000".to_string()));
    if let Some(after_id) = after_id {
        query.push(("after_id".to_string(), after_id.to_string()));
    }

    url.set_fragment(None);
    url.set_query(None);
    url.set_path(&format!("/{}", parts.join("/")));
    {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in query {
            pairs.append_pair(&name, &value);
        }
    }
    Ok(url.to_string())
}

// ---------------------------------------------------------------------------
// SSE parsing
// ---------------------------------------------------------------------------

/// `ClaudeSSEParser.extractCompleteEvents(from:)`.
///
/// A trailing standalone `\r` may still pair with the next chunk's leading
/// `\n`, so it is held back and re-appended after the CRLF normalisation.
pub fn extract_complete_events(text_buffer: &mut String) -> Vec<String> {
    let has_trailing_carriage_return = text_buffer.ends_with('\r');
    if has_trailing_carriage_return {
        text_buffer.pop();
    }

    let mut normalized = text_buffer.replace("\r\n", "\n").replace('\r', "\n");
    if has_trailing_carriage_return {
        normalized.push('\r');
    }

    if !normalized.contains("\n\n") {
        *text_buffer = normalized;
        return Vec::new();
    }

    let parts: Vec<&str> = normalized.split("\n\n").collect();
    let (remaining, events) = parts.split_last().expect("split yields at least one part");
    let events: Vec<String> = events
        .iter()
        .filter(|event| !event.is_empty())
        .map(|event| (*event).to_string())
        .collect();
    *text_buffer = (*remaining).to_string();
    events
}

/// `ClaudeSSEParser.parseEvent(_:)`.
///
/// Lines are trimmed before the `event:`/`data:` prefix test — unlike the
/// OpenAI transport, which tests the untrimmed line.
pub fn parse_event(event: &str) -> Result<Option<String>, QueryError> {
    let mut event_type: Option<String> = None;
    let mut json_data_string: Option<String> = None;

    for line in event.split('\n') {
        let trimmed_line = line.trim();
        if let Some(rest) = trimmed_line.strip_prefix("event:") {
            event_type = Some(rest.trim().to_string());
        } else if let Some(rest) = trimmed_line.strip_prefix("data:") {
            json_data_string = Some(rest.trim().to_string());
        }
    }

    if event_type.as_deref() == Some("error") {
        if let Some(data_string) = &json_data_string {
            if let Ok(stream_error) = serde_json::from_str::<ClaudeStreamError>(data_string) {
                return Err(QueryError::with_data(
                    ErrorType::Api,
                    None,
                    stream_error.error.message,
                ));
            }
        }
    }

    if event_type.as_deref() != Some("content_block_delta") {
        return Ok(None);
    }
    let Some(data_string) = json_data_string else {
        return Ok(None);
    };

    match serde_json::from_str::<ClaudeStreamDelta>(&data_string) {
        Ok(stream_delta) => Ok(stream_delta.delta.and_then(|delta| delta.text)),
        Err(_) => {
            // Upstream logs and keeps streaming.
            eprintln!("Failed to decode Claude SSE data");
            Ok(None)
        }
    }
}

/// Reads a Claude SSE body.
///
/// `processStreamBytes(_:continuation:)` accumulates raw bytes and decodes them
/// once the buffer reaches 1 KB or a `0x0A` byte arrives — the byte-level
/// trigger matters, because decoding a partial multi-byte character would
/// otherwise be skipped and retried on the next flush.
pub struct ClaudeSseReader {
    response: reqwest::Response,
    chunk: Vec<u8>,
    chunk_pos: usize,
    /// `dataBuffer` — bytes not yet decoded into `text_buffer`.
    data_buffer: Vec<u8>,
    /// `textBuffer` — decoded text not yet split into complete events.
    text_buffer: String,
    /// Complete events waiting to be parsed.
    pending: VecDeque<String>,
    done: bool,
}

impl ClaudeSseReader {
    pub fn new(response: reqwest::Response) -> Self {
        ClaudeSseReader {
            response,
            chunk: Vec::new(),
            chunk_pos: 0,
            data_buffer: Vec::new(),
            text_buffer: String::new(),
            pending: VecDeque::new(),
            done: false,
        }
    }

    /// Returns the next delta text, or `None` at end of stream.
    pub async fn next_event(&mut self) -> Result<Option<String>, QueryError> {
        loop {
            if let Some(event) = self.pending.pop_front() {
                return parse_event(&event);
            }

            if self.chunk_pos < self.chunk.len() {
                let byte = self.chunk[self.chunk_pos];
                self.chunk_pos += 1;
                self.data_buffer.push(byte);
                if self.data_buffer.len() >= BUFFER_THRESHOLD || byte == 0x0A {
                    self.flush_data_buffer();
                }
                continue;
            }

            if self.done {
                self.flush_data_buffer();
                if self.pending.is_empty() {
                    return Ok(None);
                }
                continue;
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

    /// Decodes the pending bytes when they are valid UTF-8 and splits out any
    /// complete events. Invalid UTF-8 is kept for the next flush, as upstream's
    /// `if let text = String(data:encoding:.utf8)` does.
    fn flush_data_buffer(&mut self) {
        if self.data_buffer.is_empty() {
            return;
        }
        if let Ok(text) = std::str::from_utf8(&self.data_buffer) {
            self.text_buffer.push_str(text);
            self.data_buffer.clear();
            self.pending.extend(extract_complete_events(&mut self.text_buffer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use std::sync::Arc;

    fn content_delta_payload(text: &str) -> String {
        format!("{{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{text}\"}}}}")
    }

    fn content_delta_event(text: &str, line_separator: &str) -> String {
        format!(
            "event: content_block_delta{sep}data: {payload}{sep}{sep}",
            sep = line_separator,
            payload = content_delta_payload(text)
        )
    }

    fn normalized_content_delta_event(text: &str) -> String {
        format!(
            "event: content_block_delta\ndata: {}",
            content_delta_payload(text)
        )
    }

    fn error_event(message: &str, line_separator: &str) -> String {
        let payload = format!(
            "{{\"type\":\"error\",\"error\":{{\"type\":\"invalid_request_error\",\"message\":\"{message}\"}}}}"
        );
        format!(
            "event: error{sep}data: {payload}{sep}{sep}",
            sep = line_separator
        )
    }

    #[test]
    fn extracts_a_single_lf_event() {
        let mut buffer = content_delta_event("Hello", "\n");
        let events = extract_complete_events(&mut buffer);
        assert_eq!(events, vec![normalized_content_delta_event("Hello")]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn extracts_a_single_crlf_event() {
        let mut buffer = content_delta_event("Hello", "\r\n");
        let events = extract_complete_events(&mut buffer);
        assert_eq!(events, vec![normalized_content_delta_event("Hello")]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn preserves_a_trailing_carriage_return_across_chunks() {
        let first_chunk = "event: content_block_delta\r";
        let second_chunk = format!(
            "\ndata: {}\r\n\r\n",
            content_delta_payload("Hello")
        );

        let mut buffer = first_chunk.to_string();
        let first_pass = extract_complete_events(&mut buffer);
        assert!(first_pass.is_empty());
        assert_eq!(buffer, first_chunk);

        buffer.push_str(&second_chunk);
        let second_pass = extract_complete_events(&mut buffer);
        assert_eq!(second_pass, vec![normalized_content_delta_event("Hello")]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn extracts_multiple_crlf_events() {
        let mut buffer = format!(
            "{}{}",
            content_delta_event("Hello", "\r\n"),
            content_delta_event("World", "\r\n")
        );
        let events = extract_complete_events(&mut buffer);
        assert_eq!(
            events,
            vec![
                normalized_content_delta_event("Hello"),
                normalized_content_delta_event("World")
            ]
        );
        assert!(buffer.is_empty());
    }

    #[test]
    fn parses_content_deltas() {
        let event = normalized_content_delta_event("Hello");
        assert_eq!(parse_event(&event).unwrap(), Some("Hello".to_string()));

        // Non-content events yield nothing.
        assert_eq!(
            parse_event("event: message_start\ndata: {\"type\":\"message_start\"}").unwrap(),
            None
        );
        assert_eq!(parse_event("").unwrap(), None);
        // A `content_block_delta` whose payload does not decode is skipped.
        assert_eq!(
            parse_event("event: content_block_delta\ndata: not json").unwrap(),
            None
        );
    }

    #[test]
    fn recognises_error_events_after_normalization() {
        let mut buffer = error_event("Claude request failed", "\r\n");
        let events = extract_complete_events(&mut buffer);
        assert_eq!(events.len(), 1);
        assert!(buffer.is_empty());

        let error = parse_event(&events[0]).unwrap_err();
        assert_eq!(error.kind, ErrorType::Api);
        assert_eq!(error.error_data_message.as_deref(), Some("Claude request failed"));
    }

    #[test]
    fn non_streaming_response_joins_text_blocks() {
        // The non-streaming answer is the content-block list, and only the
        // `text` blocks carry text.
        let body = r#"{"id":"msg_1","type":"message","role":"assistant","content":[
            {"type":"thinking","thinking":"…"},{"type":"text","text":"你好"},{"type":"text","text":"，世界"}]}"#;
        assert_eq!(
            decode_message_response(body.as_bytes()).unwrap().text(),
            "你好，世界"
        );

        // A block without text and a response with no blocks both yield
        // nothing, which the caller reports as `noResult`.
        assert_eq!(
            decode_message_response(br#"{"content":[{"type":"text"}]}"#)
                .unwrap()
                .text(),
            ""
        );
        assert_eq!(decode_message_response(br#"{"content":[]}"#).unwrap().text(), "");

        // An undecodable body is an api error, not an empty answer.
        assert_eq!(
            decode_message_response(b"not json").unwrap_err().kind,
            ErrorType::Api
        );
    }

    #[test]
    fn system_messages_are_hoisted_into_the_system_prompt() {
        let messages = vec![
            ChatMessage::system("first"),
            ChatMessage::user("hello"),
            ChatMessage::assistant("hi"),
            ChatMessage::system("second"),
        ];
        let (system, rest) = separate_system_messages(&messages);
        assert_eq!(system, "first\n\nsecond");
        assert_eq!(rest.len(), 2);
        assert_eq!(rest[0].role, ChatRole::User);
        assert_eq!(rest[1].role, ChatRole::Assistant);
    }

    #[test]
    fn temperature_is_clamped_to_one() {
        assert_eq!(claude_temperature(0.3), 0.3);
        assert_eq!(claude_temperature(1.5), 1.0);
        assert_eq!(claude_temperature(-2.0), 0.0);
    }

    #[test]
    fn model_selection_and_defaults_match_upstream() {
        let ctx = ServiceContext::new(
            Arc::new(Settings::default()),
            ServiceType::Claude,
            "Claude",
        );
        let service = ClaudeService::new(ctx);
        assert_eq!(DEFAULT_ENDPOINT, "https://api.anthropic.com/v1/messages");
        assert_eq!(service.endpoint(), DEFAULT_ENDPOINT);
        assert_eq!(service.model(), "claude-sonnet-4-6");
        assert_eq!(DEFAULT_MODELS.len(), 4);
        assert_eq!(service.link().as_deref(), Some("https://claude.ai/"));
        assert_eq!(service.supported_query_type(), QueryTextType::TRANSLATION);
        assert!(service.is_stream());
    }

    #[test]
    fn configured_model_and_endpoint_win() {
        let mut settings = Settings::default();
        {
            let config = settings.service_mut("Claude");
            config.model = "claude-opus-4-7".to_string();
            config.endpoint = "https://proxy.test/v1/messages".to_string();
        }
        let service = ClaudeService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::Claude,
            "Claude",
        ));
        assert_eq!(service.model(), "claude-opus-4-7");
        assert_eq!(service.endpoint(), "https://proxy.test/v1/messages");
    }

    #[test]
    fn models_url_replaces_the_messages_path() {
        assert_eq!(
            claude_models_url("https://api.anthropic.com/v1/messages", None).unwrap(),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
        assert_eq!(
            claude_models_url("https://api.anthropic.com/v1/models", Some("abc")).unwrap(),
            "https://api.anthropic.com/v1/models?limit=1000&after_id=abc"
        );
        // An endpoint with no path becomes `/v1/models`.
        assert_eq!(
            claude_models_url("https://api.anthropic.com", None).unwrap(),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
        assert!(claude_models_url("", None).is_err());
    }

    #[test]
    fn model_list_page_decodes_anthropic_shape() {
        let page: ClaudeModelList = serde_json::from_str(
            r#"{"data":[{"id":"claude-sonnet-4-6"}],"has_more":true,"last_id":"claude-sonnet-4-6"}"#,
        )
        .unwrap();
        assert_eq!(page.data.len(), 1);
        assert!(page.has_more);
        assert_eq!(page.last_id.as_deref(), Some("claude-sonnet-4-6"));

        // `has_more` is required, as it is in `ClaudeModelListResponse`; a page
        // without it fails to decode and is reported as an invalid response.
        assert!(serde_json::from_str::<ClaudeModelList>(r#"{"data":[]}"#).is_err());
    }
}
