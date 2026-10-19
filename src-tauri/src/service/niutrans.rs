//! NiuTrans — `NiuTransService.swift`.
//!
//! A single form-encoded endpoint. The key may come from the user or from the
//! built-in pool; this port ships no built-in pool, so an unset key is reported
//! the same way a missing user key would be.

use async_trait::async_trait;
use serde::Deserialize;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
};
use crate::net;
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

const API_URL: &str = "https://api.niutrans.com/NiuTransServer/translation";
const TEXT_LIMIT: usize = 5000;

pub struct NiuTransService {
    ctx: ServiceContext,
}

impl NiuTransService {
    pub fn new(ctx: ServiceContext) -> Self {
        NiuTransService { ctx }
    }

    fn api_key(&self) -> String {
        self.ctx.settings.niu_trans_api_key.trim().to_string()
    }
}

#[async_trait]
impl QueryService for NiuTransService {
    fn service_type(&self) -> ServiceType {
        ServiceType::NiuTrans
    }

    fn identifier(&self) -> &str {
        "NiuTrans"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
    }

    fn link(&self) -> Option<String> {
        Some("https://niutrans.com".to_string())
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);

        // An unmapped source falls back to `auto`, an unmapped target to "".
        let source_lang = self.language_code(query.query_from_language).unwrap_or_else(|| "auto".to_string());
        let target_lang = self
            .language_code(query.query_target_language)
            .unwrap_or_default();
        let text = util::ns_trim_to_max_length(&query.query_text, TEXT_LIMIT);

        let response = net::CLIENT
            .post(API_URL)
            .form(&[
                ("apikey", self.api_key()),
                ("src_text", text),
                ("from", source_lang),
                ("to", target_lang),
                ("source", "Easydict".to_string()),
            ])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(net::map_reqwest_error)?;

        if !status.is_success() {
            return Err(QueryError::api_http(
                status.as_u16(),
                net::truncate(&String::from_utf8_lossy(&bytes), 4096),
            ));
        }

        let parsed: NiuTransResponse = serde_json::from_slice(&bytes).map_err(|e| {
            QueryError::with_message(
                ErrorType::Api,
                format!("Failed to decode response: {e}: {}", net::truncate(&String::from_utf8_lossy(&bytes), 2048)),
            )
        })?;

        let translated = parsed
            .tgt_text
            .as_deref()
            .map(|t| t.trim_matches('\n').to_string())
            .filter(|t| !t.is_empty());

        match translated {
            Some(text) => {
                result.translated_results = Some(util::to_paragraphs(&text));
                Ok(result)
            }
            None => match parsed.error_code {
                Some(code) => {
                    let message = match parsed.error_msg {
                        Some(msg) => format!("{code}, {msg}"),
                        None => code,
                    };
                    Err(QueryError::with_message(ErrorType::Api, message))
                }
                None => Err(QueryError::with_message(ErrorType::Api, "Unknown error")),
            },
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct NiuTransResponse {
    #[serde(default)]
    #[allow(dead_code)]
    from: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    to: Option<String>,
    #[serde(default, rename = "tgt_text")]
    tgt_text: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    src_text: Option<String>,
    #[serde(default, rename = "error_msg")]
    error_msg: Option<String>,
    #[serde(default, rename = "error_code")]
    error_code: Option<String>,
    #[serde(default)]
    #[allow(dead_code)]
    apikey: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> NiuTransService {
        let mut settings = crate::config::Settings::default();
        settings.niu_trans_api_key = "test".into();
        NiuTransService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::NiuTrans,
            "NiuTrans",
        ))
    }

    #[test]
    fn georgian_uses_jy() {
        let svc = service();
        assert_eq!(
            svc.language_code(Language::Georgian).as_deref(),
            Some("jy")
        );
    }

    #[test]
    fn success_response_decoding() {
        let r: NiuTransResponse =
            serde_json::from_str(r#"{"from":"zh","to":"en","tgt_text":"Hello"}"#).unwrap();
        assert_eq!(r.tgt_text.as_deref(), Some("Hello"));
    }

    #[test]
    fn error_response_decoding() {
        let r: NiuTransResponse = serde_json::from_str(
            r#"{"apikey":"","error_code":"13002","error_msg":"apikey is empty","from":"en","src_text":"good","to":"zh"}"#,
        )
        .unwrap();
        assert_eq!(r.error_code.as_deref(), Some("13002"));
        assert_eq!(r.error_msg.as_deref(), Some("apikey is empty"));
        assert!(r.tgt_text.is_none());
    }

    #[test]
    fn truncation_is_utf16_units() {
        let long = "a".repeat(6000);
        assert_eq!(util::ns_trim_to_max_length(&long, TEXT_LIMIT).len(), 5000);
    }
}
