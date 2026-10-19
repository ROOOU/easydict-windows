//! DeepSeek Translate — `DeepSeekService.swift`.
//!
//! DeepSeek layers DeepSeek V4's reasoning parameters (`thinking.type` plus
//! `reasoning_effort`) on top of the OpenAI-compatible pipeline, and brings its
//! own transport: no content-type check, and a parser that joins the `data:`
//! payloads of one event with *no* separator and tolerates undecodable chunks.

use std::collections::VecDeque;

use async_trait::async_trait;
use serde::Deserialize;

use crate::model::{
    ErrorType, QueryError, QueryModel, QueryResult, QueryTextType, ReasoningEffort, ServiceType,
};
use crate::net;
use crate::service::llm::{
    decode_chat_result, handle_stream_error, query_messages, resolve_endpoint, resolve_model,
    supported_query_type, ChunkThrottle, DeepSeekChatRequest, ThinkingSwitch, DONE_FLAG,
};
use crate::service::stream;
use crate::service::{QueryService, ServiceContext, ServiceUpdate, UpdateSender};
use crate::util;

/// `DeepSeekService.defaultEndpoint`
pub const DEFAULT_ENDPOINT: &str = "https://api.deepseek.com/v1/chat/completions";
/// `DeepSeekService.remoteModelsEndpoint`
pub const REMOTE_MODELS_ENDPOINT: &str = "https://api.deepseek.com/models";
/// `DeepSeekModel`
pub const DEFAULT_MODELS: &[&str] = &["deepseek-v4-flash", "deepseek-v4-pro"];
/// `DeepSeekService.defaultModel`
pub const DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// DeepSeek Translate.
pub struct DeepSeekService {
    ctx: ServiceContext,
}

impl DeepSeekService {
    pub fn new(ctx: ServiceContext) -> Self {
        DeepSeekService { ctx }
    }

    /// `StreamService.model`
    pub fn model(&self) -> String {
        resolve_model(&self.ctx, DEFAULT_MODELS, DEFAULT_MODEL)
    }

    /// `StreamService.endpoint`
    pub fn endpoint(&self) -> String {
        resolve_endpoint(&self.ctx, DEFAULT_ENDPOINT)
    }

    /// `DeepSeekService.supportsReasoningEffort` — the shared picker is exposed
    /// and both `thinking` and `reasoning_effort` are sent.
    pub fn reasoning_effort(&self) -> ReasoningEffort {
        self.ctx.config.reasoning_effort
    }

    /// `BaseOpenAIService.fetchRemoteModelIDs()` with DeepSeek's own endpoint.
    pub async fn fetch_remote_model_ids(&self) -> Result<Vec<String>, QueryError> {
        let api_key = self.ctx.api_key();
        if api_key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                "API key is empty",
            ));
        }

        let headers = net::headers(&[
            ("Authorization", &format!("Bearer {api_key}")),
            ("api-key", &api_key),
            ("Accept", "application/json"),
        ]);
        let data = crate::service::llm::fetch_remote_model_data(REMOTE_MODELS_ENDPOINT, headers)
            .await?;
        let ids = crate::service::llm::decode_openai_model_list(&data)?;
        Ok(crate::service::llm::normalized_remote_model_ids(
            &ids,
            crate::service::llm::default_remote_model_lookup_id,
        ))
    }

    /// `makeChatRequest(url:messages:)` — the POST both transports send.
    /// `stream` is the only field that differs between them.
    fn request(
        &self,
        query: &QueryModel,
        stream: bool,
    ) -> Result<reqwest::RequestBuilder, QueryError> {
        let endpoint = self.endpoint();
        let url = reqwest::Url::parse(endpoint.trim()).map_err(|_| {
            QueryError::with_message(
                ErrorType::Parameter,
                format!("`{}` endpoint is invalid", ServiceType::DeepSeek.raw()),
            )
        })?;

        let api_key = self.ctx.api_key();
        if api_key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                "API key is empty",
            ));
        }

        let messages = query_messages(&self.ctx, query);
        let model = self.model();
        let effort = self.reasoning_effort();
        let body = DeepSeekChatRequest {
            messages: &messages,
            model: &model,
            temperature: self.ctx.temperature(),
            stream,
            thinking: ThinkingSwitch::from_effort(effort),
            reasoning_effort: effort.request_value(),
        };

        Ok(net::LLM_CLIENT
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&body))
    }

    /// The streaming transport — `processStreamBytes(_:continuation:)`.
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

        // `validateHTTPResponse(_:)` — DeepSeek reports the status only.
        let status = response.status();
        if !status.is_success() {
            return Err(QueryError::with_message(
                ErrorType::Api,
                format!("HTTP {}", status.as_u16()),
            ));
        }

        let mut reader = DeepSeekSseReader::new(response);
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
    /// Upstream's DeepSeek service has no non-streaming branch — its
    /// `makeChatRequest` hard-codes `stream: true` — so there is no Swift
    /// symbol to reproduce. The response, unlike Claude's, is the ordinary
    /// chat-completions shape (`choices[0].message.content`), so it decodes
    /// through the shared [`decode_chat_result`]; a reasoning model's
    /// `reasoning_content` is ignored, exactly as the streamed `reasoning`
    /// delta is. `validateHTTPResponse(_:)` keeps reporting the status alone.
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
            return Err(QueryError::with_message(
                ErrorType::Api,
                format!("HTTP {}", status.as_u16()),
            ));
        }

        let bytes = response.bytes().await.unwrap_or_default();
        let result = decode_chat_result(&bytes)?;
        let content = result.first_content().unwrap_or_default();
        if content.is_empty() {
            return Err(stream::no_result_error());
        }

        if let Some(tx) = updates {
            let _ = tx.send(ServiceUpdate::Chunk(content.to_string()));
        }
        crate::service::llm::final_text(
            content,
            &query.query_text,
            self.ctx.config.hide_think_tag,
        )
    }
}

#[async_trait]
impl QueryService for DeepSeekService {
    fn service_type(&self) -> ServiceType {
        ServiceType::DeepSeek
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
        Some("https://www.deepseek.com/".to_string())
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

// ---------------------------------------------------------------------------
// SSE parsing
// ---------------------------------------------------------------------------

/// `DeepSeekStreamChunk`
#[derive(Debug, Deserialize)]
pub struct DeepSeekStreamChunk {
    pub choices: Vec<DeepSeekChoice>,
}

/// `DeepSeekStreamChunk.Choice`
#[derive(Debug, Deserialize)]
pub struct DeepSeekChoice {
    pub delta: DeepSeekDelta,
}

/// `DeepSeekStreamChunk.Delta`
#[derive(Debug, Deserialize)]
pub struct DeepSeekDelta {
    #[serde(default)]
    pub content: Option<String>,
}

/// `DeepSeekService.parseSSEEvent(_:)`.
///
/// Every `data:` line of the event is appended with **no separator**, and the
/// whole event is discarded as soon as one of them is `[DONE]`. A payload that
/// fails to decode is logged and skipped rather than failing the query.
pub fn parse_sse_event(event: &str) -> Option<String> {
    let data_prefix = "data:";
    let mut data_string = String::new();

    for line in event.split('\n') {
        if let Some(rest) = line.strip_prefix(data_prefix) {
            let payload = rest.trim();
            if payload == DONE_FLAG {
                return None;
            }
            data_string.push_str(payload);
        }
    }

    if data_string.is_empty() {
        return None;
    }

    match serde_json::from_str::<DeepSeekStreamChunk>(&data_string) {
        Ok(chunk) => chunk
            .choices
            .first()
            .and_then(|choice| choice.delta.content.clone()),
        Err(_) => {
            // Upstream logs and keeps going; a bad chunk is not fatal.
            eprintln!("Failed to decode DeepSeek SSE data: {data_string}");
            None
        }
    }
}

/// `DeepSeekService.processCompleteEvents(from:continuation:)`.
///
/// The buffer is normalised to LF, split on blank lines, and the last split
/// part is kept back as the still-incomplete event.
pub fn process_complete_events(text_buffer: &mut String) -> Vec<String> {
    *text_buffer = text_buffer.replace("\r\n", "\n");
    if !text_buffer.contains("\n\n") {
        return Vec::new();
    }

    // Everything before the last blank line is a complete event; the tail waits
    // for more bytes. Copying first avoids holding a borrow across the
    // assignment to the buffer.
    let index = text_buffer
        .rfind("\n\n")
        .expect("checked for a separator above");
    let complete = text_buffer[..index].to_string();
    *text_buffer = text_buffer[index + 2..].to_string();

    complete
        .split("\n\n")
        .filter(|event| !event.is_empty())
        .filter_map(parse_sse_event)
        .collect()
}

/// Reads a DeepSeek SSE body.
///
/// `processStreamBytes(_:continuation:)` decodes the accumulated bytes whenever
/// a `0x0A` byte arrives — bytes that are not yet valid UTF-8 stay in the buffer
/// and are retried on the next newline.
pub struct DeepSeekSseReader {
    response: reqwest::Response,
    chunk: Vec<u8>,
    chunk_pos: usize,
    /// `dataBuffer`
    data_buffer: Vec<u8>,
    /// `textBuffer`
    text_buffer: String,
    /// Decoded deltas waiting to be delivered.
    pending: VecDeque<String>,
    done: bool,
}

impl DeepSeekSseReader {
    pub fn new(response: reqwest::Response) -> Self {
        DeepSeekSseReader {
            response,
            chunk: Vec::new(),
            chunk_pos: 0,
            data_buffer: Vec::new(),
            text_buffer: String::new(),
            pending: VecDeque::new(),
            done: false,
        }
    }

    /// Returns the next content delta, or `None` at end of stream.
    pub async fn next_event(&mut self) -> Result<Option<String>, QueryError> {
        loop {
            if let Some(content) = self.pending.pop_front() {
                return Ok(Some(content));
            }

            if self.chunk_pos < self.chunk.len() {
                let byte = self.chunk[self.chunk_pos];
                self.chunk_pos += 1;
                self.data_buffer.push(byte);
                // Only a newline triggers a decode pass.
                if byte == 0x0A {
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

    fn flush_data_buffer(&mut self) {
        if self.data_buffer.is_empty() {
            return;
        }
        if let Ok(text) = std::str::from_utf8(&self.data_buffer) {
            self.text_buffer.push_str(text);
            self.data_buffer.clear();
            self.pending
                .extend(process_complete_events(&mut self.text_buffer));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use crate::model::Language;
    use std::sync::Arc;

    fn service() -> DeepSeekService {
        let mut settings = Settings::default();
        settings
            .services
            .entry("DeepSeek".to_string())
            .or_default()
            .api_key = "key".to_string();
        DeepSeekService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::DeepSeek,
            "DeepSeek",
        ))
    }

    fn chunk(content: &str) -> String {
        format!(
            r#"{{"choices":[{{"delta":{{"content":"{content}"}}}}]}}"#
        )
    }

    #[test]
    fn defaults_match_upstream() {
        assert_eq!(DEFAULT_ENDPOINT, "https://api.deepseek.com/v1/chat/completions");
        assert_eq!(DEFAULT_MODEL, "deepseek-v4-flash");
        assert_eq!(DEFAULT_MODELS.len(), 2);
        assert_eq!(REMOTE_MODELS_ENDPOINT, "https://api.deepseek.com/models");

        let svc = service();
        assert_eq!(svc.model(), "deepseek-v4-flash");
        assert_eq!(svc.endpoint(), DEFAULT_ENDPOINT);
        assert_eq!(svc.link().as_deref(), Some("https://www.deepseek.com/"));
        assert!(svc.is_stream());
        assert_eq!(svc.supported_query_type(), QueryTextType::TRANSLATION);
    }

    #[test]
    fn reasoning_effort_switches_thinking_on() {
        let mut svc = service();
        assert_eq!(svc.reasoning_effort(), ReasoningEffort::Off);
        assert_eq!(
            ThinkingSwitch::from_effort(svc.reasoning_effort()).kind,
            "disabled"
        );

        svc.ctx.config.reasoning_effort = ReasoningEffort::Max;
        assert_eq!(svc.reasoning_effort(), ReasoningEffort::Max);
        assert_eq!(
            ThinkingSwitch::from_effort(svc.reasoning_effort()).kind,
            "enabled"
        );
        assert_eq!(svc.reasoning_effort().request_value(), Some("max"));
    }

    #[test]
    fn model_falls_back_to_the_first_valid_model() {
        let mut svc = service();
        assert_eq!(svc.model(), "deepseek-v4-flash");
        svc.ctx.config.model = "deepseek-v4-pro".to_string();
        assert_eq!(svc.model(), "deepseek-v4-pro");
        svc.ctx.config.model = "gpt-5".to_string();
        assert_eq!(svc.model(), "deepseek-v4-flash");
    }

    #[test]
    fn language_codes_are_english_names() {
        // DeepSeek has no `unsupportedLanguages`, so the shared LLM table
        // applies: every language is accepted under its English name.
        let svc = service();
        assert_eq!(
            svc.language_code(Language::SimplifiedChinese).as_deref(),
            Some("Simplified-Chinese")
        );
        assert_eq!(svc.language_code(Language::Uyghur).as_deref(), Some("Uyghur"));
    }

    #[test]
    fn request_body_carries_thinking_and_optional_effort() {
        let svc = service();
        let messages = query_messages(&svc.ctx, &QueryModel::new("hello"));
        let model = svc.model();

        let body = DeepSeekChatRequest {
            messages: &messages,
            model: &model,
            temperature: svc.ctx.temperature(),
            stream: true,
            thinking: ThinkingSwitch::from_effort(ReasoningEffort::Off),
            reasoning_effort: ReasoningEffort::Off.request_value(),
        };
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["model"], "deepseek-v4-flash");
        assert_eq!(json["temperature"], 0.3);
        assert_eq!(json["stream"], true);
        assert_eq!(json["thinking"], serde_json::json!({"type": "disabled"}));
        assert!(json.get("reasoning_effort").is_none());
        // The system prompt stays inside the messages array for DeepSeek.
        assert_eq!(json["messages"][0]["role"], "system");
    }

    #[test]
    fn done_sentinel_discards_the_whole_event() {
        assert_eq!(parse_sse_event("data: [DONE]"), None);
        // A `[DONE]` payload anywhere in the event discards the rest too.
        assert_eq!(
            parse_sse_event(&format!("data: {}\ndata: [DONE]", chunk("x"))),
            None
        );
    }

    #[test]
    fn data_lines_are_joined_without_a_separator() {
        // One JSON object split over two `data:` lines: upstream concatenates
        // the payloads verbatim, so the pieces must line up exactly.
        let event = format!("data: {}\ndata: {}", r#"{"choices":[{"delta":{"content""#, r#":"joined"}}]}"#);
        assert_eq!(parse_sse_event(&event), Some("joined".to_string()));
    }

    #[test]
    fn undecodable_chunks_are_skipped_not_fatal() {
        assert_eq!(parse_sse_event("data: not json"), None);
        assert_eq!(parse_sse_event("data: "), None);
        assert_eq!(parse_sse_event(""), None);
        // `choices` is required; a chunk without it is skipped.
        assert_eq!(parse_sse_event(r#"data: {"delta":{"content":"x"}}"#), None);
        // A delta without content is skipped.
        assert_eq!(parse_sse_event(r#"data: {"choices":[{"delta":{}}]}"#), None);
    }

    #[test]
    fn complete_events_normalise_crlf_and_keep_the_tail() {
        let mut buffer = format!("data: {}\r\n\r\ndata: {}", chunk("one"), chunk("two"));
        let contents = process_complete_events(&mut buffer);
        assert_eq!(contents, vec!["one".to_string()]);
        // The tail is still incomplete and stays buffered.
        assert_eq!(buffer, format!("data: {}", chunk("two")));

        buffer.push_str("\n\n");
        assert_eq!(process_complete_events(&mut buffer), vec!["two".to_string()]);
        assert!(buffer.is_empty());
    }

    #[test]
    fn a_buffer_without_a_blank_line_is_left_alone() {
        let mut buffer = format!("data: {}", chunk("one"));
        let untouched = buffer.clone();
        assert!(process_complete_events(&mut buffer).is_empty());
        assert_eq!(buffer, untouched);
    }
}
