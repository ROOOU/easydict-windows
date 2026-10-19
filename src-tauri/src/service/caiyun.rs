//! LingoCloud (彩云小译) — `CaiyunService.swift`.
//!
//! A JSON endpoint keyed by an `x-authorization: token <token>` header. The
//! request splits the text on newlines and asks the server to detect the
//! language when the source is `auto`.

use async_trait::async_trait;
use serde::Deserialize;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
};
use crate::net;
use crate::service::{QueryService, ServiceContext, UpdateSender};

const ENDPOINT: &str = "https://api.interpreter.caiyunai.com/v1/translator";

pub struct CaiyunService {
    ctx: ServiceContext,
}

impl CaiyunService {
    pub fn new(ctx: ServiceContext) -> Self {
        CaiyunService { ctx }
    }

    fn token(&self) -> String {
        self.ctx.settings.caiyun_token.trim().to_string()
    }

    /// `CaiyunTranslateType.transType(from:to:)` — the raw value is
    /// `"<from>2<to>"`, or `unsupported` when the pair is invalid.
    fn trans_type(&self, from: Language, to: Language) -> Result<String, QueryError> {
        if from == to {
            return Err(self.unsupported(from, to));
        }
        let from_code = self
            .language_code(from)
            .ok_or_else(|| self.unsupported(from, to))?;
        let to_code = self
            .language_code(to)
            .ok_or_else(|| self.unsupported(from, to))?;
        Ok(format!("{from_code}2{to_code}"))
    }

    fn unsupported(&self, from: Language, to: Language) -> QueryError {
        QueryError::with_message(
            ErrorType::UnsupportedLanguage,
            format!("{} --> {}", from.display_name(), to.display_name()),
        )
    }
}

#[async_trait]
impl QueryService for CaiyunService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Caiyun
    }

    fn identifier(&self) -> &str {
        "Caiyun"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
    }

    fn link(&self) -> Option<String> {
        Some("https://fanyi.caiyunapp.com".to_string())
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);
        let trans_type = self.trans_type(query.query_from_language, query.query_target_language)?;

        // Blank lines are preserved as empty segments.
        let source: Vec<String> = query
            .query_text
            .split('\n')
            .map(|s| s.to_string())
            .collect();

        let body = serde_json::json!({
            "source": source,
            "trans_type": trans_type,
            "media": "text",
            "request_id": "Easydict",
            "detect": trans_type.starts_with("auto"),
        });

        let response = net::CLIENT
            .post(ENDPOINT)
            .header("content-type", "application/json")
            .header("x-authorization", format!("token {}", self.token()))
            .json(&body)
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(net::map_reqwest_error)?;

        if !status.is_success() {
            return Err(QueryError::with_data(
                ErrorType::Api,
                Some(format!("HTTP {}", status.as_u16())),
                String::from_utf8_lossy(&bytes).to_string(),
            ));
        }

        let parsed: CaiyunResponse = serde_json::from_slice(&bytes).map_err(|e| {
            QueryError::with_data(
                ErrorType::Api,
                Some(e.to_string()),
                String::from_utf8_lossy(&bytes).to_string(),
            )
        })?;

        result.translated_results = Some(parsed.target);
        Ok(result)
    }
}

#[derive(Debug, Deserialize)]
struct CaiyunResponse {
    #[serde(default)]
    #[allow(dead_code)]
    confidence: f64,
    #[serde(default)]
    #[allow(dead_code)]
    rc: i32,
    #[serde(default)]
    target: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> CaiyunService {
        let mut settings = crate::config::Settings::default();
        settings.caiyun_token = "tok".into();
        CaiyunService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::Caiyun,
            "Caiyun",
        ))
    }

    #[test]
    fn trans_type_format() {
        let svc = service();
        assert_eq!(
            svc.trans_type(Language::English, Language::SimplifiedChinese).unwrap(),
            "en2zh"
        );
        assert_eq!(
            svc.trans_type(Language::Auto, Language::SimplifiedChinese).unwrap(),
            "auto2zh"
        );
        assert_eq!(
            svc.trans_type(Language::TraditionalChinese, Language::English).unwrap(),
            "zh-Hant2en"
        );
    }

    #[test]
    fn same_language_is_unsupported() {
        let svc = service();
        let err = svc
            .trans_type(Language::English, Language::English)
            .unwrap_err();
        assert_eq!(err.kind, ErrorType::UnsupportedLanguage);
        assert_eq!(err.message.as_deref(), Some("English --> English"));
    }

    #[test]
    fn unsupported_language_is_reported() {
        let svc = service();
        // Catalan is not in the Caiyun table.
        assert!(svc.trans_type(Language::Catalan, Language::English).is_err());
    }

    #[test]
    fn detect_flag_follows_auto_source() {
        let svc = service();
        let auto = svc.trans_type(Language::Auto, Language::English).unwrap();
        assert!(auto.starts_with("auto"));
        let fixed = svc.trans_type(Language::English, Language::SimplifiedChinese).unwrap();
        assert!(!fixed.starts_with("auto"));
    }
}
