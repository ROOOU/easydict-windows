//! Tencent Cloud Machine Translation — `TencentService.swift`,
//! `TencentSigning.swift`, `TencentTranslateType.swift`.
//!
//! Credentials are a SecretId/SecretKey pair rather than a single API key, so
//! the service overrides [`QueryService::has_private_api_key`] instead of
//! relying on the generic `apiKey` slot.
//!
//! One deliberate difference from upstream: the Swift hashes
//! `JSONSerialization.data(withJSONObject:)` but then hands the same dictionary
//! to Alamofire, which re-encodes it independently — two encoders, two key
//! orders, and a payload hash that only matches by luck. Here the payload is
//! serialized once and those exact bytes are both hashed and sent.

use async_trait::async_trait;
use chrono::Utc;
use serde::Deserialize;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
};
use crate::net;
use crate::service::languages;
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

const ENDPOINT: &str = "https://tmt.tencentcloudapi.com";
const SERVICE: &str = "tmt";
const ACTION: &str = "TextTranslate";
const VERSION: &str = "2018-03-21";
const REGION: &str = "ap-guangzhou";
const ALGORITHM: &str = "TC3-HMAC-SHA256";
const CONTENT_TYPE: &str = "application/json; charset=utf-8";

/// The third path segment of the credential scope. It is a protocol constant
/// from Tencent's public signature-v3 algorithm
/// (<https://cloud.tencent.com/document/api/213/30654>), not a credential.
const SCOPE_TERMINATOR: &str = "request";
const SIGNED_HEADERS: &str = "content-type;host;x-tc-action";

pub struct TencentService {
    ctx: ServiceContext,
}

impl TencentService {
    pub fn new(ctx: ServiceContext) -> Self {
        TencentService { ctx }
    }

    fn secret_id(&self) -> String {
        self.ctx.settings.tencent_secret_id.trim().to_string()
    }

    fn secret_key(&self) -> String {
        self.ctx.settings.tencent_secret_key.trim().to_string()
    }
}

/// `tencentSignHeader(...)` — the value returned is the `Authorization` header.
///
/// `payload` must be the exact bytes that go on the wire; the caller computes
/// it once so the hash and the body cannot disagree.
pub fn tencent_authorization(
    payload: &str,
    secret_id: &str,
    secret_key: &str,
    timestamp: i64,
    date: &str,
) -> String {
    let host = format!("{SERVICE}.tencentcloudapi.com");

    // Step 1 — canonical request.
    let canonical_headers = format!(
        "content-type:{CONTENT_TYPE}\nhost:{host}\nx-tc-action:{}\n",
        ACTION.to_lowercase()
    );
    let canonical_request = format!(
        "POST\n/\n\n{canonical_headers}\n{SIGNED_HEADERS}\n{}",
        util::sha256_hex(payload)
    );

    // Step 2 — string to sign.
    let credential_scope = format!("{date}/{SERVICE}/{SCOPE_TERMINATOR}");
    let string_to_sign = format!(
        "{ALGORITHM}\n{timestamp}\n{credential_scope}\n{}",
        util::sha256_hex(&canonical_request)
    );

    // Step 3 — signature. Each HMAC key is the previous result's raw bytes.
    let secret_date = util::hmac_sha256(format!("TC3{secret_key}").as_bytes(), date.as_bytes());
    let secret_service =
        util::hmac_sha256(&secret_date, SERVICE.as_bytes());
    let secret_signing =
        util::hmac_sha256(&secret_service, SCOPE_TERMINATOR.as_bytes());
    let signature = hex::encode(util::hmac_sha256(
        &secret_signing,
        string_to_sign.as_bytes(),
    ));

    // Step 4 — Authorization.
    format!(
        "{ALGORITHM} Credential={secret_id}/{credential_scope}, SignedHeaders={SIGNED_HEADERS}, Signature={signature}"
    )
}

#[async_trait]
impl QueryService for TencentService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Tencent
    }

    fn identifier(&self) -> &str {
        "Tencent"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
    }

    fn link(&self) -> Option<String> {
        Some("https://fanyi.qq.com".to_string())
    }

    /// Tencent holds a SecretId/SecretKey pair, so the generic key slot is
    /// never populated.
    fn has_private_api_key(&self) -> bool {
        !self.secret_id().is_empty() && !self.secret_key().is_empty()
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);
        let from = query.query_from_language;
        let to = query.query_target_language;

        if !languages::tencent_supports_pair(from, to) {
            return Err(QueryError::with_message(
                ErrorType::UnsupportedLanguage,
                format!("{} --> {}", from.display_name(), to.display_name()),
            ));
        }

        let source = self
            .language_code(from)
            .ok_or_else(|| unsupported(from, to))?;
        let target = self
            .language_code(to)
            .ok_or_else(|| unsupported(from, to))?;

        // `serde_json` orders object keys, so this string is deterministic —
        // which is what makes hashing it and sending it consistent.
        let payload = serde_json::json!({
            "SourceText": query.query_text,
            "Source": source,
            "Target": target,
            "ProjectId": 0,
        })
        .to_string();

        let now = Utc::now();
        let timestamp = now.timestamp();
        let date = now.format("%Y-%m-%d").to_string();
        let authorization = tencent_authorization(
            &payload,
            &self.secret_id(),
            &self.secret_key(),
            timestamp,
            &date,
        );

        let response = net::CLIENT
            .post(ENDPOINT)
            .header("Authorization", authorization)
            .header("Content-Type", CONTENT_TYPE)
            .header("X-TC-Action", ACTION)
            .header("X-TC-Timestamp", timestamp.to_string())
            .header("X-TC-Version", VERSION)
            .header("X-TC-Region", REGION)
            .body(payload)
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        let bytes = response.bytes().await.map_err(net::map_reqwest_error)?;

        // Tencent reports API-level failures inside a 200 response, so the
        // success shape is tried first and the error shape second.
        if let Ok(ok) = serde_json::from_slice::<TencentResponse>(&bytes) {
            result.translated_results = Some(
                ok.response
                    .target_text
                    .split('\n')
                    .map(str::to_string)
                    .collect(),
            );
            return Ok(result);
        }

        if let Ok(err) = serde_json::from_slice::<TencentErrorResponse>(&bytes) {
            return Err(QueryError::with_message(
                ErrorType::Api,
                err.response.error.message,
            ));
        }

        Err(QueryError::with_data(
            ErrorType::Api,
            Some(format!("HTTP {}", status.as_u16())),
            net::truncate(&String::from_utf8_lossy(&bytes), 4096),
        ))
    }
}

fn unsupported(from: Language, to: Language) -> QueryError {
    QueryError::with_message(
        ErrorType::UnsupportedLanguage,
        format!("{} --> {}", from.display_name(), to.display_name()),
    )
}

#[derive(Debug, Deserialize)]
struct TencentResponse {
    #[serde(rename = "Response")]
    response: TencentOk,
}

#[derive(Debug, Deserialize)]
struct TencentOk {
    #[serde(rename = "TargetText")]
    target_text: String,
    #[serde(rename = "RequestId", default)]
    #[allow(dead_code)]
    request_id: String,
    #[serde(rename = "Source", default)]
    #[allow(dead_code)]
    source: String,
    #[serde(rename = "Target", default)]
    #[allow(dead_code)]
    target: String,
}

#[derive(Debug, Deserialize)]
struct TencentErrorResponse {
    #[serde(rename = "Response")]
    response: TencentError,
}

#[derive(Debug, Deserialize)]
struct TencentError {
    #[serde(rename = "Error")]
    error: TencentErrorBody,
    #[serde(rename = "RequestId", default)]
    #[allow(dead_code)]
    request_id: String,
}

#[derive(Debug, Deserialize)]
struct TencentErrorBody {
    #[serde(rename = "Code", default)]
    #[allow(dead_code)]
    code: String,
    #[serde(rename = "Message")]
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> TencentService {
        let mut settings = crate::config::Settings::default();
        settings.tencent_secret_id = "AKID".into();
        settings.tencent_secret_key = "SECRET".into();
        TencentService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::Tencent,
            "Tencent",
        ))
    }

    #[test]
    fn payload_keys_are_ordered() {
        // The signature covers the serialized payload, so the ordering has to
        // be a property of the type rather than of insertion order.
        let payload = serde_json::json!({
            "SourceText": "hello",
            "Source": "en",
            "Target": "zh",
            "ProjectId": 0,
        })
        .to_string();
        assert_eq!(
            payload,
            r#"{"ProjectId":0,"Source":"en","SourceText":"hello","Target":"zh"}"#
        );
    }

    #[test]
    fn authorization_shape() {
        let secret_id = "AKID";
        let auth = tencent_authorization("{}", secret_id, "SECRET", 1_700_000_000, "2023-11-14");
        let expected_prefix = format!(
            "TC3-HMAC-SHA256 Credential={secret_id}/2023-11-14/tmt/{SCOPE_TERMINATOR}, \
             SignedHeaders={SIGNED_HEADERS}, Signature="
        );
        assert!(
            auth.starts_with(&expected_prefix),
            "unexpected authorization: {auth}"
        );
        let signature = auth.rsplit("Signature=").next().unwrap();
        assert_eq!(signature.len(), 64);
        assert!(signature.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn authorization_is_deterministic_and_payload_sensitive() {
        let a = tencent_authorization("{}", "AKID", "SECRET", 1_700_000_000, "2023-11-14");
        let b = tencent_authorization("{}", "AKID", "SECRET", 1_700_000_000, "2023-11-14");
        let c = tencent_authorization(r#"{"x":1}"#, "AKID", "SECRET", 1_700_000_000, "2023-11-14");
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn canonical_request_uses_lowercased_action_and_blank_line() {
        // `x-tc-action` is lowercased inside the canonical headers while the
        // `X-TC-Action` request header keeps its original casing, and TC3
        // requires a blank line between the canonical headers and the signed
        // header list. Both are easy to lose in a rewrite.
        let host = "tmt.tencentcloudapi.com";
        let canonical_headers = format!(
            "content-type:{CONTENT_TYPE}\nhost:{host}\nx-tc-action:{}\n",
            ACTION.to_lowercase()
        );
        assert!(canonical_headers.ends_with("x-tc-action:texttranslate\n"));
        let canonical_request = format!(
            "POST\n/\n\n{canonical_headers}\n{SIGNED_HEADERS}\n{}",
            util::sha256_hex("{}")
        );
        assert!(canonical_request.starts_with("POST\n/\n\ncontent-type:"));
        assert!(canonical_request.contains("\n\ncontent-type;host;x-tc-action\n"));
    }

    #[test]
    fn credentials_require_both_halves() {
        let svc = service();
        assert!(svc.has_private_api_key());

        let mut settings = crate::config::Settings::default();
        settings.tencent_secret_id = "AKID".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Tencent, "Tencent");
        assert!(!TencentService::new(ctx).has_private_api_key());
    }

    #[test]
    fn success_response_decoding() {
        let r: TencentResponse = serde_json::from_str(
            r#"{"Response":{"RequestId":"eb6d17f2","Source":"en","Target":"zh","TargetText":"你好\n世界"}}"#,
        )
        .unwrap();
        assert_eq!(r.response.target_text, "你好\n世界");
        assert_eq!(r.response.target_text.split('\n').count(), 2);
    }

    #[test]
    fn error_response_decoding() {
        let r: TencentErrorResponse = serde_json::from_str(
            r#"{"Response":{"Error":{"Code":"InvalidParameterValue","Message":"不支持的语种：hi_to_zh"},"RequestId":"eb6d17f2"}}"#,
        )
        .unwrap();
        assert_eq!(r.response.error.message, "不支持的语种：hi_to_zh");
        // The success shape must reject the same body.
        assert!(serde_json::from_str::<TencentResponse>(
            r#"{"Response":{"Error":{"Code":"x","Message":"y"},"RequestId":"z"}}"#
        )
        .is_err());
    }

    #[test]
    fn language_pair_matrix() {
        // Supported, including the two rules the docs leave unstated.
        assert!(languages::tencent_supports_pair(
            Language::English,
            Language::SimplifiedChinese
        ));
        assert!(languages::tencent_supports_pair(
            Language::SimplifiedChinese,
            Language::TraditionalChinese
        ));
        assert!(languages::tencent_supports_pair(
            Language::English,
            Language::English
        ));
        // Japanese only reaches four targets.
        assert!(languages::tencent_supports_pair(
            Language::Japanese,
            Language::Korean
        ));
        assert!(!languages::tencent_supports_pair(
            Language::Japanese,
            Language::Arabic
        ));
        // `TencentTranslateType.supportedTypes` lists Arabic and Hindi among
        // English's targets, so this pair is supported both ways.
        assert!(languages::tencent_supports_pair(
            Language::Arabic,
            Language::English
        ));
        assert!(languages::tencent_supports_pair(
            Language::English,
            Language::Arabic
        ));
        // An auto source never matches, and unlisted languages are rejected.
        assert!(!languages::tencent_supports_pair(
            Language::Auto,
            Language::English
        ));
        assert!(!languages::tencent_supports_pair(
            Language::Catalan,
            Language::English
        ));
    }

    #[test]
    fn language_codes() {
        let svc = service();
        assert_eq!(
            svc.language_code(Language::TraditionalChinese).as_deref(),
            Some("zh-TW")
        );
        assert_eq!(svc.language_code(Language::Malay).as_deref(), Some("ms"));
        assert_eq!(svc.language_code(Language::Catalan), None);
    }
}
