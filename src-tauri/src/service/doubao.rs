//! Doubao Translate (豆包翻译) — `DoubaoService.swift`.
//!
//! Uses Volcengine's Ark *Responses* API rather than chat completions, so the
//! SSE stream carries named events and the delta lives at a top-level `delta`
//! field. Only `response.output_text.delta` events are consumed.

use async_trait::async_trait;
use serde::Deserialize;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
};
use crate::net;
use crate::service::stream::{self, SseReader};
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

const ENDPOINT: &str = "https://ark.cn-beijing.volces.com/api/v3/responses";
/// `DoubaoService.defaultModelIdentifier`
pub const DEFAULT_MODEL: &str = "doubao-seed-translation-250915";
const DELTA_EVENT: &str = "response.output_text.delta";
const EVENT_PREFIX: &str = "event:";
const DATA_PREFIX: &str = "data:";
const DONE_FLAG: &str = "[DONE]";

pub struct DoubaoService {
    ctx: ServiceContext,
}

impl DoubaoService {
    pub fn new(ctx: ServiceContext) -> Self {
        DoubaoService { ctx }
    }

    fn api_key(&self) -> String {
        self.ctx.settings.doubao_api_key.trim().to_string()
    }

    /// The configured model, falling back to the default when unset or not in
    /// the valid-model list — `StreamService.model`.
    fn model(&self) -> String {
        let configured = self.ctx.settings.doubao_model.trim();
        if configured.is_empty() {
            return DEFAULT_MODEL.to_string();
        }
        let valid = if self.ctx.config.valid_models.is_empty() {
            vec![DEFAULT_MODEL.to_string()]
        } else {
            self.ctx.config.valid_models.clone()
        };
        if valid.iter().any(|m| m == configured) {
            configured.to_string()
        } else {
            valid
                .first()
                .cloned()
                .unwrap_or_else(|| DEFAULT_MODEL.to_string())
        }
    }

    /// `DoubaoTranslateType.transType(from:to:)`
    fn trans_type(&self, from: Language, to: Language) -> Result<(String, String), QueryError> {
        let from_code = self.language_code(from).ok_or_else(|| {
            QueryError::with_message(
                ErrorType::UnsupportedLanguage,
                format!("{} --> {}", from.display_name(), to.display_name()),
            )
        })?;
        let to_code = self.language_code(to).ok_or_else(|| {
            QueryError::with_message(
                ErrorType::UnsupportedLanguage,
                format!("{} --> {}", from.display_name(), to.display_name()),
            )
        })?;
        Ok((from_code, to_code))
    }
}

#[async_trait]
impl QueryService for DoubaoService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Doubao
    }

    fn identifier(&self) -> &str {
        "Doubao"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    /// The translation model only does translation, not dictionary lookups.
    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION
    }

    fn is_stream(&self) -> bool {
        true
    }

    /// Doubao's key lives in the flat `doubao_api_key` setting
    /// (`DoubaoService.swift:50`), so the generic key slot is never populated.
    fn has_private_api_key(&self) -> bool {
        !self.api_key().is_empty()
    }

    fn link(&self) -> Option<String> {
        Some("https://www.volcengine.com/product/doubao".to_string())
    }

    async fn translate(
        &self,
        query: &QueryModel,
        updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);

        let api_key = self.api_key();
        if api_key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                "Missing Doubao API Key. Get your API key from https://www.volcengine.com/product/doubao",
            ));
        }

        let (source, target) = self.trans_type(query.query_from_language, query.query_target_language)?;

        // The `enable_streaming` setting picks the transport. Upstream's Doubao
        // service has no non-streaming branch — `buildRequestBody` hard-codes
        // `"stream": true` — so `stream` here is the only wire difference, and
        // the answer is read as a Responses result rather than an SSE stream.
        let streaming = self.ctx.enable_streaming();
        let body = serde_json::json!({
            "model": self.model(),
            "stream": streaming,
            "input": [{
                "role": "user",
                "content": [{
                    "type": "input_text",
                    "text": query.query_text,
                    "translation_options": {
                        "source_language": source,
                        "target_language": target,
                    },
                }],
            }],
        });

        let response = net::LLM_CLIENT
            .post(ENDPOINT)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {api_key}"))
            .json(&body)
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        if !(200..300).contains(&status.as_u16()) {
            let bytes = response.bytes().await.unwrap_or_default();
            return Err(QueryError::with_data(
                ErrorType::Api,
                Some(format!("HTTP error: {}", status.as_u16())),
                net::truncate(&String::from_utf8_lossy(&bytes), 4096),
            ));
        }

        let accumulated = if streaming {
            let mut reader = SseReader::new(response);
            let mut accumulated = String::new();

            while let Some(block) = reader.next_event().await {
                let (event, data) = stream::parse_sse_block(&block);
                // Both an `event:` line naming the delta type and a payload are
                // required; bare `data:`-only chunks are ignored.
                if event.as_deref() != Some(DELTA_EVENT) {
                    continue;
                }
                let Some(data) = data else { continue };
                if data == DONE_FLAG {
                    continue;
                }
                let Ok(parsed) = serde_json::from_str::<DoubaoStreamEvent>(&data) else {
                    continue;
                };
                let Some(delta) = parsed.delta else { continue };
                if delta.is_empty() {
                    continue;
                }
                accumulated.push_str(&delta);
                if let Some(tx) = &updates {
                    let _ = tx.send(crate::service::ServiceUpdate::Chunk(delta));
                }
            }
            accumulated
        } else {
            // The whole answer arrives at once; it is still handed to the UI as
            // a single chunk, the way the non-streaming OpenAI transport does.
            let bytes = response.bytes().await.unwrap_or_default();
            let text = decode_responses_result(&bytes)?.text();
            if text.is_empty() {
                return Err(stream::no_result_error());
            }
            if let Some(tx) = &updates {
                let _ = tx.send(crate::service::ServiceUpdate::Chunk(text.clone()));
            }
            text
        };

        let mut final_text = stream::get_final_result_text(&accumulated, &query.query_text);
        if self.ctx.config.hide_think_tag {
            final_text = stream::filter_think_tag_content(&final_text);
        }

        if final_text.is_empty() {
            return Err(stream::no_result_error());
        }

        result.translated_results = Some(util::to_paragraphs(&final_text));
        Ok(result)
    }
}

#[derive(Debug, Deserialize)]
struct DoubaoStreamEvent {
    #[serde(default)]
    delta: Option<String>,
}

/// The Responses result with `stream: false`.
///
/// Doubao's non-streaming shape is the furthest from the OpenAI-compatible
/// chat result: the assistant text is nested inside `output[].content[]` as
/// `output_text` parts, and an entry may carry no content at all. Upstream has
/// no non-streaming branch to reproduce — `buildRequestBody` hard-codes
/// `"stream": true` — so this follows the Ark Responses API itself.
#[derive(Debug, Deserialize)]
struct DoubaoResponseResult {
    output: Vec<DoubaoOutputItem>,
}

/// `output[]`
#[derive(Debug, Deserialize)]
struct DoubaoOutputItem {
    #[serde(default)]
    content: Vec<DoubaoOutputContent>,
}

/// `output[].content[]` — only `output_text` parts carry text.
#[derive(Debug, Deserialize)]
struct DoubaoOutputContent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

impl DoubaoResponseResult {
    /// Every `output_text` part's text, in the order the API returned them.
    fn text(&self) -> String {
        self.output
            .iter()
            .flat_map(|item| item.content.iter())
            .filter(|part| part.kind == "output_text")
            .filter_map(|part| part.text.as_deref())
            .collect()
    }
}

/// Decodes a non-streaming Responses result; an undecodable body is an `api`
/// error carrying the decoder's message.
fn decode_responses_result(body: &[u8]) -> Result<DoubaoResponseResult, QueryError> {
    serde_json::from_slice(body).map_err(|error| {
        QueryError::with_data(
            ErrorType::Api,
            None,
            format!(
                "{error}: {}",
                net::truncate(&String::from_utf8_lossy(body), 4096)
            ),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> DoubaoService {
        let mut settings = crate::config::Settings::default();
        settings.doubao_api_key = "key".into();
        DoubaoService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::Doubao,
            "Doubao",
        ))
    }

    #[test]
    fn credentials_come_from_the_flat_settings() {
        assert!(service().has_private_api_key());

        // `DoubaoService.swift:50` reads the flat `EZDoubaoAPIKey` default,
        // never the generic service slot.
        let mut settings = crate::config::Settings::default();
        settings
            .services
            .get_mut(ServiceType::Doubao.raw())
            .unwrap()
            .api_key = "generic".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Doubao, "Doubao");
        assert!(!DoubaoService::new(ctx).has_private_api_key());
    }

    #[test]
    fn model_falls_back_to_default() {
        let svc = service();
        assert_eq!(svc.model(), DEFAULT_MODEL);
    }

    #[test]
    fn invalid_model_is_replaced() {
        let mut settings = crate::config::Settings::default();
        settings.doubao_api_key = "key".into();
        settings.doubao_model = "not-a-model".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Doubao, "Doubao");
        assert_eq!(DoubaoService::new(ctx).model(), DEFAULT_MODEL);
    }

    #[test]
    fn streaming_is_enabled_and_query_type_is_translation_only() {
        let svc = service();
        assert!(svc.is_stream());
        assert_eq!(svc.supported_query_type(), QueryTextType::TRANSLATION);
    }

    #[test]
    fn delta_event_shape() {
        let block = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"你好\"}";
        let (event, data) = stream::parse_sse_block(block);
        assert_eq!(event.as_deref(), Some(DELTA_EVENT));
        let parsed: DoubaoStreamEvent = serde_json::from_str(&data.unwrap()).unwrap();
        assert_eq!(parsed.delta.as_deref(), Some("你好"));
    }

    #[test]
    fn non_streaming_response_reads_output_text_parts() {
        // The Responses result nests the answer in `output[].content[]`, and an
        // output entry may carry no content at all.
        let body = r#"{"id":"resp_1","object":"response","status":"completed","model":"doubao-seed-translation-250915",
            "output":[{"type":"reasoning","id":"rs_1"},
                      {"type":"message","id":"msg_1","role":"assistant",
                       "content":[{"type":"output_text","text":"你好"},{"type":"output_text","text":"，世界"}]}]}"#;
        let parsed = decode_responses_result(body.as_bytes()).unwrap();
        assert_eq!(parsed.text(), "你好，世界");

        // Nothing to show is an empty string, which the caller reports as
        // `noResult` rather than as a successful translation.
        assert_eq!(decode_responses_result(br#"{"output":[]}"#).unwrap().text(), "");
        assert_eq!(
            decode_responses_result(br#"{"output":[{"type":"message"}]}"#)
                .unwrap()
                .text(),
            ""
        );

        // An undecodable body is an api error, not an empty answer.
        let error = decode_responses_result(b"not json").unwrap_err();
        assert_eq!(error.kind, ErrorType::Api);
    }

    #[test]
    fn other_events_are_ignored() {
        for block in [
            "event: response.created\ndata: {\"type\":\"response.created\"}",
            "event: response.completed\ndata: {\"delta\":\"ignored\"}",
            "data: {\"delta\":\"no event line\"}",
        ] {
            let (event, data) = stream::parse_sse_block(block);
            let accepted = event.as_deref() == Some(DELTA_EVENT)
                && data.is_some_and(|d| d != DONE_FLAG);
            assert!(!accepted, "should have been ignored: {block}");
        }
    }

    #[test]
    fn norwegian_uses_bokmal_code() {
        let svc = service();
        assert_eq!(svc.language_code(Language::Norwegian).as_deref(), Some("nb"));
        assert_eq!(
            svc.language_code(Language::TraditionalChinese).as_deref(),
            Some("zh-Hant")
        );
    }
}
