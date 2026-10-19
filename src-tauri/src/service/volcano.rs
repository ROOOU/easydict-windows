//! Volcano (火山翻译) — `VolcanoService.swift`.
//!
//! Signed with the Volcengine flavour of HMAC-SHA256: the algorithm identifier
//! is the bare `HMAC-SHA256` (no `AWS4-` prefix), the credential scope ends in
//! `request`, and the Authorization header carries the project's original
//! double space after the comma.

use async_trait::async_trait;
use serde::Deserialize;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
};
use crate::net;
use crate::service::{languages, QueryService, ServiceContext, UpdateSender};
use crate::util;

const HOST: &str = "https://translate.volcengineapi.com";
const URI: &str = "/";
const QUERY_STRING: &str = "Action=TranslateText&Version=2020-06-01";
const REGION: &str = "cn-north-1";
const SERVICE: &str = "translate";
const ALGORITHM: &str = "HMAC-SHA256";

pub struct VolcanoService {
    ctx: ServiceContext,
}

impl VolcanoService {
    pub fn new(ctx: ServiceContext) -> Self {
        VolcanoService { ctx }
    }

    fn access_key_id(&self) -> String {
        self.ctx.settings.volcano_access_key_id.trim().to_string()
    }

    fn secret_access_key(&self) -> String {
        self.ctx.settings.volcano_secret_access_key.trim().to_string()
    }

    fn validate_api_key(&self, key: &str, key_type: &str) -> Result<(), QueryError> {
        if key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                format!(
                    "Missing Volcano {key_type}. Volcano Service requires users' own API Key. Get it at https://www.volcengine.com"
                ),
            ));
        }
        Ok(())
    }
}

#[async_trait]
impl QueryService for VolcanoService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Volcano
    }

    fn identifier(&self) -> &str {
        "Volcano"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
    }

    /// Volcano holds an Access Key ID/Secret Access Key pair
    /// (`VolcanoService.swift:47`), so the generic key slot is never populated.
    fn has_private_api_key(&self) -> bool {
        !self.access_key_id().is_empty() && !self.secret_access_key().is_empty()
    }

    fn link(&self) -> Option<String> {
        Some("https://translate.volcengine.com".to_string())
    }

    fn word_link(&self, query: &QueryModel) -> Option<String> {
        let from = self.language_code(query.query_from_language)?;
        let to = self.language_code(query.query_target_language)?;
        let text = util::url_encode_query_allowed(&query.query_text);
        Some(format!(
            "https://translate.volcengine.com/?source_language={from}&target_language={to}&text={text}"
        ))
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);

        let access_key_id = self.access_key_id();
        let secret_access_key = self.secret_access_key();
        self.validate_api_key(&access_key_id, "AccessKeyID")?;
        self.validate_api_key(&secret_access_key, "SecretAccessKey")?;

        let (source, target) = self.trans_type(query.query_from_language, query.query_target_language)?;

        // Key order matters: the signature hashes this exact serialization.
        let body = serde_json::json!({
            "SourceLanguage": source,
            "TargetLanguage": target,
            "TextList": [query.query_text],
        });
        let body_bytes = serde_json::to_vec(&body)
            .map_err(|e| QueryError::with_message(ErrorType::Api, e.to_string()))?;

        let x_date = utc_x_date();
        let headers = sign_request(
            &access_key_id,
            &secret_access_key,
            &x_date,
            &body_bytes,
        );

        let url = format!("{HOST}{URI}?{QUERY_STRING}");
        let mut request = net::CLIENT
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Host", HOST.trim_start_matches("https://"))
            .header("X-Date", &x_date)
            .header("Authorization", headers.authorization)
            .body(body_bytes);

        request = request.header("Accept", "*/*");

        let response = request
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(net::map_reqwest_error)?;

        let parsed: VolcanoResponse = serde_json::from_slice(&bytes).map_err(|e| {
            // The upstream catch path re-decodes the body to surface the API's
            // own error message, so mirror that.
            let detail = serde_json::from_slice::<VolcanoResponse>(&bytes)
                .ok()
                .and_then(|r| r.response_metadata.error)
                .map(|e| e.message);
            if !status.is_success() {
                QueryError::with_data(
                    ErrorType::Api,
                    Some(format!("HTTP {}", status.as_u16())),
                    detail.unwrap_or_else(|| net::truncate(&String::from_utf8_lossy(&bytes), 4096)),
                )
            } else {
                QueryError::with_data(
                    ErrorType::Api,
                    Some(e.to_string()),
                    detail.unwrap_or_default(),
                )
            }
        })?;

        if let Some(error) = parsed.response_metadata.error {
            return Err(QueryError::with_message(ErrorType::Api, error.message));
        }

        match parsed.translation_list {
            Some(list) => {
                result.translated_results =
                    Some(list.into_iter().map(|t| t.translation).collect());
                Ok(result)
            }
            None => Err(QueryError::with_message(
                ErrorType::Unknown,
                "Unexpected response format",
            )),
        }
    }
}

impl VolcanoService {
    /// `VolcanoTranslateType.transType(from:to:)`
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

        if !languages::volcano_supports_pair(from, to)
            || from == Language::Slovak
        {
            return Err(QueryError::with_message(
                ErrorType::UnsupportedLanguage,
                format!("{} --> {}", from.display_name(), to.display_name()),
            ));
        }

        Ok((from_code, to_code))
    }
}

/// `yyyyMMdd'T'HHmmss'Z'` in UTC.
fn utc_x_date() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string()
}

struct SignedHeaders {
    authorization: String,
}

/// `volcanoSigning(accessKeyId:secretAccessKey:host:uri:queryString:region:service:parameters:)`
fn sign_request(
    access_key_id: &str,
    secret_access_key: &str,
    x_date: &str,
    body_bytes: &[u8],
) -> SignedHeaders {
    let content_hashed = util::hex_lower(&util::sha256_bytes(body_bytes));
    let host = HOST.trim_start_matches("https://");

    let header_pairs = [
        ("content-type", "application/json".to_string()),
        ("host", host.to_string()),
        ("x-date", x_date.to_string()),
    ];
    let mut lines: Vec<String> = header_pairs
        .iter()
        .map(|(k, v)| format!("{k}:{}", v.trim()))
        .collect();
    lines.sort();
    // The canonical headers block ends with a newline, which is what puts the
    // blank line before the signed-headers entry in the canonical request.
    let canonical_headers = format!("{}\n", lines.join("\n"));

    let signed_headers = "content-type;host;x-date";

    let canonical_request = [
        "POST",
        URI,
        QUERY_STRING,
        &canonical_headers,
        signed_headers,
        &content_hashed,
    ]
    .join("\n");

    let short_date = &x_date[..8];
    let credential_scope = format!("{short_date}/{REGION}/{SERVICE}/request");

    let canonical_request_hashed = util::sha256_hex(&canonical_request);

    let string_to_sign = [
        ALGORITHM,
        x_date,
        &credential_scope,
        &canonical_request_hashed,
    ]
    .join("\n");

    let k_date = util::hmac_sha256(secret_access_key.as_bytes(), short_date.as_bytes());
    let k_region = util::hmac_sha256(&k_date, REGION.as_bytes());
    let k_service = util::hmac_sha256(&k_region, SERVICE.as_bytes());
    let k_signing = util::hmac_sha256(&k_service, b"request");
    let signature = util::hmac_sha256_hex(&k_signing, string_to_sign.as_bytes());

    // The double space after the comma is in the original.
    let authorization = format!(
        "{ALGORITHM} Credential={access_key_id}/{credential_scope},  SignedHeaders={signed_headers}, Signature={signature}"
    );

    SignedHeaders { authorization }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolcanoResponse {
    #[serde(default)]
    translation_list: Option<Vec<VolcanoTranslation>>,
    response_metadata: VolcanoResponseMetadata,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolcanoTranslation {
    translation: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolcanoResponseMetadata {
    #[serde(default)]
    request_id: String,
    #[serde(default)]
    action: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    service: String,
    #[serde(default)]
    region: String,
    #[serde(default)]
    error: Option<VolcanoError>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct VolcanoError {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> VolcanoService {
        let mut settings = crate::config::Settings::default();
        settings.volcano_access_key_id = "AKID".into();
        settings.volcano_secret_access_key = "SECRET".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Volcano, "Volcano");
        VolcanoService::new(ctx)
    }

    #[test]
    fn credentials_require_both_flat_fields() {
        assert!(service().has_private_api_key());

        // `VolcanoService.swift:47` tests the AK/SK pair from the flat
        // settings; the generic service slot is read by nothing.
        let mut settings = crate::config::Settings::default();
        settings
            .services
            .get_mut(ServiceType::Volcano.raw())
            .unwrap()
            .api_key = "generic".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Volcano, "Volcano");
        assert!(!VolcanoService::new(ctx).has_private_api_key());

        let mut settings = crate::config::Settings::default();
        settings.volcano_access_key_id = "AKID".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Volcano, "Volcano");
        assert!(!VolcanoService::new(ctx).has_private_api_key());
    }

    #[test]
    fn word_link_encodes_the_query_through_the_shared_helper() {
        // `VolcanoService.wordLink` uses the same
        // `addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed)` as
        // every sibling, so this must agree with them character for character —
        // hand-rolling `replace(" ", "%20")` left `&`/`#`/`+` able to cut the
        // query short and sent non-ASCII raw.
        let svc = service();
        let mut q = QueryModel::new("你好");
        q.query_from_language = Language::SimplifiedChinese;
        q.query_target_language = Language::English;
        assert_eq!(
            svc.word_link(&q).as_deref(),
            Some(
                "https://translate.volcengine.com/?source_language=zh&target_language=en&text=%E4%BD%A0%E5%A5%BD"
            )
        );

        let mut q = QueryModel::new("A&B #tag");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        assert_eq!(
            svc.word_link(&q).as_deref(),
            Some(
                "https://translate.volcengine.com/?source_language=en&target_language=zh&text=A&B%20#tag"
            )
        );
    }

    #[test]
    fn canonical_headers_have_trailing_newline() {
        let body = br#"{"SourceLanguage":"en","TargetLanguage":"zh","TextList":["hi"]}"#;
        let signed = sign_request("AKID", "SECRET", "20240811T123456Z", body);
        assert!(signed
            .authorization
            .starts_with("HMAC-SHA256 Credential=AKID/20240811/cn-north-1/translate/request,"));
        // The double space is deliberate: it is in the upstream format string.
        assert!(signed
            .authorization
            .contains(",  SignedHeaders=content-type;host;x-date, Signature="));
    }

    #[test]
    fn signature_is_deterministic() {
        let body = b"{}";
        let a = sign_request("AKID", "SECRET", "20240811T123456Z", body);
        let b = sign_request("AKID", "SECRET", "20240811T123456Z", body);
        assert_eq!(a.authorization, b.authorization);
    }

    #[test]
    fn missing_key_message() {
        let svc = service();
        let err = svc.validate_api_key("", "AccessKeyID").unwrap_err();
        assert_eq!(err.kind, ErrorType::MissingSecretKey);
        assert!(err.message.unwrap().starts_with("Missing Volcano AccessKeyID."));
    }

    #[test]
    fn uyghur_and_slovak_restrictions() {
        let svc = service();
        assert!(svc.trans_type(Language::Uyghur, Language::SimplifiedChinese).is_ok());
        assert!(svc.trans_type(Language::Uyghur, Language::English).is_err());
        assert!(svc.trans_type(Language::English, Language::Uyghur).is_err());
        // Slovak is a target language only.
        assert!(svc.trans_type(Language::Slovak, Language::English).is_err());
        assert!(svc.trans_type(Language::English, Language::Slovak).is_ok());
    }

    #[test]
    fn x_date_format() {
        let d = utc_x_date();
        assert_eq!(d.len(), 16);
        assert_eq!(&d[8..9], "T");
        assert!(d.ends_with('Z'));
    }
}
