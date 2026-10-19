//! Alibaba Cloud machine translation — `AliService.swift`.
//!
//! Aliyun's RPC style: parameters are form-encoded, sorted by key, RFC-3986
//! encoded, and signed with HMAC-SHA1 over `POST&%2F&<encoded query>`. The
//! string-to-sign is passed through `String(data:encoding:.nonLossyASCII)`
//! before hashing, which escapes every non-ASCII character as `\uXXXX` — that
//! escaping is what makes the signature match.

use async_trait::async_trait;
use serde::Deserialize;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
};
use crate::net;
use crate::service::{languages, QueryService, ServiceContext, UpdateSender};
use crate::util;

const ENDPOINT: &str = "https://mt.aliyuncs.com";
const TEXT_LIMIT: usize = 5000;

pub struct AlibabaService {
    ctx: ServiceContext,
}

impl AlibabaService {
    pub fn new(ctx: ServiceContext) -> Self {
        AlibabaService { ctx }
    }

    fn access_key_id(&self) -> String {
        self.ctx.settings.ali_access_key_id.trim().to_string()
    }

    fn access_key_secret(&self) -> String {
        self.ctx.settings.ali_access_key_secret.trim().to_string()
    }
}

#[async_trait]
impl QueryService for AlibabaService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Alibaba
    }

    fn identifier(&self) -> &str {
        "Alibaba"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
    }

    /// Alibaba holds an AccessKey ID/Secret pair (`AliService.swift:31`), so
    /// the generic key slot is never populated.
    fn has_private_api_key(&self) -> bool {
        !self.access_key_id().is_empty() && !self.access_key_secret().is_empty()
    }

    fn link(&self) -> Option<String> {
        Some("https://translate.alibaba.com/".to_string())
    }

    /// Chinese-to-Chinese pairs are answered by the API itself (thanks to the
    /// manual `zh-tw` remap), so no local conversion is needed for them; for
    /// every other pair the shared path converts the result when the target is
    /// Traditional Chinese.
    fn auto_convert_traditional_chinese(&self) -> bool {
        true
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);

        let text = util::trim_to_max_chars(&query.query_text, TEXT_LIMIT);
        let (source, target) = self.trans_type(query.query_from_language, query.query_target_language)?;

        let access_key_id = self.access_key_id();
        let access_key_secret = self.access_key_secret();

        let mut params: Vec<(String, String)> = vec![
            ("FormatType".into(), "text".into()),
            ("SourceLanguage".into(), source),
            ("TargetLanguage".into(), target),
            ("SourceText".into(), text),
            ("Scene".into(), "general".into()),
            ("Action".into(), "TranslateGeneral".into()),
            ("Version".into(), "2018-10-12".into()),
            ("Format".into(), "JSON".into()),
            ("AccessKeyId".into(), access_key_id.clone()),
            ("SignatureNonce".into(), uuid::Uuid::new_v4().to_string().to_uppercase()),
            ("Timestamp".into(), iso8601_now()),
            ("SignatureMethod".into(), "HMAC-SHA1".into()),
            ("SignatureVersion".into(), "1.0".into()),
        ];

        if access_key_id.is_empty() || access_key_secret.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                format!(
                    "Currently using the Secret Key API type, but a certain Secret Key is empty. Please go to Settings > Service > {}, fill in the required configuration information, or switch API types.",
                    self.name()
                ),
            ));
        }

        let signature = sign(&params, &access_key_secret)?;
        params.push(("Signature".into(), signature));

        let response = net::CLIENT
            .post(ENDPOINT)
            .form(&params)
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(net::map_reqwest_error)?;

        let parsed: AliResponse = serde_json::from_slice(&bytes).map_err(|e| {
            let msg = serde_json::from_slice::<AliResponse>(&bytes)
                .ok()
                .and_then(|r| r.message);
            QueryError::with_data(
                ErrorType::Api,
                Some(if status.is_success() {
                    e.to_string()
                } else {
                    format!("HTTP {}", status.as_u16())
                }),
                msg.unwrap_or_else(|| net::truncate(&String::from_utf8_lossy(&bytes), 4096)),
            )
        })?;

        match parsed.data.and_then(|d| d.translated) {
            Some(translated) => {
                result.translated_results = Some(vec![translated]);
                Ok(result)
            }
            None => Err(QueryError::with_data(
                ErrorType::Api,
                parsed.code.map(|c| c.to_string()),
                parsed.message.unwrap_or_default(),
            )),
        }
    }
}

impl AlibabaService {
    /// `AliTranslateType.transType(from:to:)`
    fn trans_type(&self, from: Language, to: Language) -> Result<(String, String), QueryError> {
        let mut from_code = self.language_code(from).ok_or_else(|| {
            QueryError::with_message(
                ErrorType::UnsupportedLanguage,
                format!("{} --> {}", from.display_name(), to.display_name()),
            )
        })?;
        let mut to_code = self.language_code(to).ok_or_else(|| {
            QueryError::with_message(
                ErrorType::UnsupportedLanguage,
                format!("{} --> {}", from.display_name(), to.display_name()),
            )
        })?;

        // Mongolian is only usable as a target.
        if from == Language::Mongolian && (!to.is_chinese() || to == Language::Mongolian) {
            return Err(QueryError::with_message(
                ErrorType::UnsupportedLanguage,
                format!("{} --> {}", from.display_name(), to.display_name()),
            ));
        }

        // Ali only supports zh-Hant <-> zh-Hans, so Traditional is remapped.
        if languages::only_chinese([from, to]) {
            if from == Language::TraditionalChinese {
                from_code = "zh-tw".to_string();
            }
            if to == Language::TraditionalChinese {
                to_code = "zh-tw".to_string();
            }
        }

        Ok((from_code, to_code))
    }
}

/// `ISO8601DateFormatter()` with default options.
fn iso8601_now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// The characters Foundation leaves unescaped for `.urlQueryAllowed`-style
/// Aliyun signing: unreserved only.
fn aliyun_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for byte in s.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `String(data:encoding:.nonLossyASCII)` — escapes every non-ASCII scalar as
/// `\uXXXX` (and surrogate pairs for astral scalars).
fn non_lossy_ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            out.push(c);
        } else {
            let cp = c as u32;
            if cp <= 0xFFFF {
                out.push_str(&format!("\\u{cp:04x}"));
            } else {
                // Encode as a UTF-16 surrogate pair, the way Foundation does.
                let v = cp - 0x10000;
                let high = 0xD800 + (v >> 10);
                let low = 0xDC00 + (v & 0x3FF);
                out.push_str(&format!("\\u{high:04x}\\u{low:04x}"));
            }
        }
    }
    out
}

/// `AliService.sign(params:)`
fn sign(params: &[(String, String)], secret: &str) -> Result<String, QueryError> {
    let mut sorted: Vec<&(String, String)> = params.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));

    let canonical = sorted
        .iter()
        .map(|(k, v)| format!("{}={}", aliyun_encode(k), aliyun_encode(v)))
        .collect::<Vec<_>>()
        .join("&");

    let slash_encode = "%2F";
    let canonical_encode = aliyun_encode(&canonical);

    let string_to_sign = format!("POST&{slash_encode}&{canonical_encode}");
    let signable = non_lossy_ascii(&string_to_sign);

    let key = format!("{secret}&");
    Ok(util::base64_encode(&util::hmac_sha1(key.as_bytes(), signable.as_bytes())))
}

/// `AliAPIResponse` — the RPC endpoint answers in PascalCase.
#[derive(Debug, Deserialize)]
struct AliResponse {
    #[serde(default, rename = "Code")]
    code: Option<AnyCode>,
    #[serde(default, rename = "Message")]
    message: Option<String>,
    #[serde(default, rename = "Data")]
    data: Option<AliResponseData>,
}

#[derive(Debug, Deserialize)]
struct AliResponseData {
    #[serde(default, rename = "Translated")]
    translated: Option<String>,
}

/// Aliyun returns `Code` as a string on success and an integer on some errors,
/// matching the upstream `AnyCodable` decoding.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AnyCode {
    Int(i64),
    Str(String),
}

impl std::fmt::Display for AnyCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnyCode::Int(i) => write!(f, "{i}"),
            AnyCode::Str(s) => write!(f, "{s}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> AlibabaService {
        let mut settings = crate::config::Settings::default();
        settings.ali_access_key_id = "id".into();
        settings.ali_access_key_secret = "secret".into();
        AlibabaService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::Alibaba,
            "Alibaba",
        ))
    }

    #[test]
    fn credentials_require_both_flat_fields() {
        assert!(service().has_private_api_key());

        // `AliService.swift:31` tests the AccessKey pair from the flat
        // settings; the generic service slot is read by nothing.
        let mut settings = crate::config::Settings::default();
        settings
            .services
            .get_mut(ServiceType::Alibaba.raw())
            .unwrap()
            .api_key = "generic".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Alibaba, "Alibaba");
        assert!(!AlibabaService::new(ctx).has_private_api_key());

        let mut settings = crate::config::Settings::default();
        settings.ali_access_key_id = "id".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Alibaba, "Alibaba");
        assert!(!AlibabaService::new(ctx).has_private_api_key());
    }

    #[test]
    fn percent_encoding_is_rfc3986() {
        assert_eq!(aliyun_encode("a b"), "a%20b");
        assert_eq!(aliyun_encode("/"), "%2F");
        assert_eq!(aliyun_encode("a+b"), "a%2Bb");
        assert_eq!(aliyun_encode("~-_. "), "~-_.%20");
    }

    #[test]
    fn non_lossy_ascii_escapes_cjk() {
        assert_eq!(non_lossy_ascii("a你"), "a\\u4f60");
        assert_eq!(non_lossy_ascii("😀"), "\\ud83d\\ude00");
    }

    #[test]
    fn signature_is_stable_for_fixed_input() {
        let params = vec![
            ("Action".to_string(), "TranslateGeneral".to_string()),
            ("SourceText".to_string(), "hello".to_string()),
        ];
        let sig = sign(&params, "secret").unwrap();
        // Same input, same output; the value itself depends only on the
        // canonical string, which the encoding tests above pin down.
        assert_eq!(sig, sign(&params, "secret").unwrap());
        assert!(!sig.is_empty());
    }

    #[test]
    fn mongolian_is_target_only() {
        let svc = service();
        assert!(svc.trans_type(Language::Mongolian, Language::English).is_err());
        assert!(svc.trans_type(Language::Mongolian, Language::SimplifiedChinese).is_ok());
        assert!(svc.trans_type(Language::English, Language::Mongolian).is_ok());
    }

    #[test]
    fn chinese_pair_remaps_traditional() {
        let svc = service();
        let (from, to) = svc
            .trans_type(Language::TraditionalChinese, Language::SimplifiedChinese)
            .unwrap();
        assert_eq!(from, "zh-tw");
        assert_eq!(to, "zh");
    }

    #[test]
    fn response_shapes() {
        let ok: AliResponse =
            serde_json::from_str(r#"{"Code":"200","Data":{"Translated":"你好","WordCount":"5"},"RequestId":""}"#)
                .unwrap();
        assert_eq!(ok.data.unwrap().translated.as_deref(), Some("你好"));

        let err: AliResponse = serde_json::from_str(
            r#"{"Code":"ParamError","Message":"Query length limit exceeded","RequestId":"","Data":null}"#,
        )
        .unwrap();
        assert_eq!(err.code.unwrap().to_string(), "ParamError");
        assert_eq!(err.message.as_deref(), Some("Query length limit exceeded"));
    }
}
