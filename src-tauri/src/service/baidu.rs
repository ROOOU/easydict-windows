//! Baidu Translate — `BaiduService.swift` / `BaiduApiTranslate.swift`.
//!
//! The signed endpoint is `md5(appid + text + salt + secret)`, where the salt
//! is a hyphenated uppercase UUID rather than a numeric nonce.

use async_trait::async_trait;
use serde::Deserialize;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
};
use crate::net;
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

const BAIDU_TRANSLATE_URL: &str = "https://fanyi.baidu.com";
const API_URL: &str = "https://fanyi-api.baidu.com/api/trans/vip/translate";
const TEXT_LIMIT: usize = 5000;
const DETECT_TEXT_LIMIT: usize = 73;
const TTS_TEXT_LIMIT: usize = 1000;

pub struct BaiduService {
    ctx: ServiceContext,
}

impl BaiduService {
    pub fn new(ctx: ServiceContext) -> Self {
        BaiduService { ctx }
    }

    fn app_id(&self) -> String {
        self.ctx.settings.baidu_app_id.trim().to_string()
    }

    fn secret_key(&self) -> String {
        self.ctx.settings.baidu_secret_key.trim().to_string()
    }

    /// `detectText(_:)` — `POST /langdetect` with the text capped at 73 chars.
    pub async fn detect_text(&self, text: &str) -> Result<Language, QueryError> {
        if text.is_empty() {
            return Err(QueryError::with_message(ErrorType::Parameter, "识别语言的文本为空"));
        }
        let query = util::ns_trim_to_max_length(text, DETECT_TEXT_LIMIT);

        let response = net::CLIENT
            .post(format!("{BAIDU_TRANSLATE_URL}/langdetect"))
            .form(&[("query", query.as_str())])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let parsed: BaiduDetectResponse = net::json(response)
            .await
            .map_err(|_| QueryError::with_message(ErrorType::Api, "判断语言失败"))?;

        match parsed.lan.as_deref() {
            Some(lan) if !lan.is_empty() => Ok(Language::from_code(lan)),
            _ => Err(QueryError::new(ErrorType::UnsupportedLanguage)),
        }
    }
}

#[async_trait]
impl QueryService for BaiduService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Baidu
    }

    fn identifier(&self) -> &str {
        "Baidu"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn supported_query_type(&self) -> QueryTextType {
        let cfg = &self.ctx.config;
        let mut bits = QueryTextType::NONE;
        if cfg.translation {
            bits = bits.union(QueryTextType::TRANSLATION);
        }
        if cfg.dictionary {
            bits = bits.union(QueryTextType::DICTIONARY);
        }
        if cfg.sentence {
            bits = bits.union(QueryTextType::SENTENCE);
        }
        if bits.is_empty() {
            QueryTextType::INTELLIGENT
        } else {
            bits
        }
    }

    /// Baidu holds an App ID/Secret Key pair (`BaiduService.swift:53`), so the
    /// generic key slot is never populated.
    fn has_private_api_key(&self) -> bool {
        !self.app_id().is_empty() && !self.secret_key().is_empty()
    }

    fn link(&self) -> Option<String> {
        Some(BAIDU_TRANSLATE_URL.to_string())
    }

    fn word_link(&self, query: &QueryModel) -> Option<String> {
        let from = self.language_code(query.query_from_language)?;
        let to = self.language_code(query.query_target_language)?;
        let text = util::url_encode_query_allowed(&query.query_text);
        Some(format!("{BAIDU_TRANSLATE_URL}/#{from}/{to}/{text}"))
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);

        if query.query_text.is_empty() {
            return Err(QueryError::with_message(ErrorType::Parameter, "翻译的文本为空"));
        }
        let text = util::ns_trim_to_max_length(&query.query_text, TEXT_LIMIT);

        let app_id = self.app_id();
        let secret_key = self.secret_key();
        if app_id.is_empty() || secret_key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                format!(
                    "Currently using the Secret Key API type, but a certain Secret Key is empty. Please go to Settings > Service > {}, fill in the required configuration information, or switch API types.",
                    self.name()
                ),
            ));
        }

        // Upstream sends `languageCode(forLanguage:)` and falls back to the raw
        // Language string when the lookup fails, so an unmapped language goes
        // out verbatim rather than as `auto`.
        let from = self
            .language_code(query.query_from_language)
            .unwrap_or_else(|| query.query_from_language.raw().to_string());
        let to = self
            .language_code(query.query_target_language)
            .unwrap_or_else(|| query.query_target_language.raw().to_string());

        let salt = uuid::Uuid::new_v4().to_string().to_uppercase();
        let sign = util::md5_hex(&format!("{app_id}{text}{salt}{secret_key}"));

        let response = net::CLIENT
            .post(API_URL)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .form(&[
                ("q", text.as_str()),
                ("from", from.as_str()),
                ("to", to.as_str()),
                ("appid", app_id.as_str()),
                ("salt", salt.as_str()),
                ("sign", sign.as_str()),
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
            let detail = serde_json::from_slice::<BaiduErrorResponse>(&bytes)
                .ok()
                .map(|e| format!("code:{}, msg:{}", e.error_code, e.error_msg));
            return Err(QueryError::with_data(
                ErrorType::Api,
                Some(format!("HTTP {}", status.as_u16())),
                detail.unwrap_or_default(),
            ));
        }

        let parsed: BaiduApiResponse = serde_json::from_slice(&bytes).map_err(|e| {
            let detail = serde_json::from_slice::<BaiduErrorResponse>(&bytes)
                .ok()
                .map(|e| format!("code:{}, msg:{}", e.error_code, e.error_msg));
            QueryError::with_data(
                ErrorType::Api,
                Some(e.to_string()),
                detail.unwrap_or_else(|| net::truncate(&String::from_utf8_lossy(&bytes), 4096)),
            )
        })?;

        result.query_text = text;
        if let Some(f) = parsed.from.as_deref() {
            result.from = Language::from_code(f);
        }
        if let Some(t) = parsed.to.as_deref() {
            result.to = Language::from_code(t);
        }
        result.translated_results = Some(
            parsed
                .trans_result
                .into_iter()
                .map(|t| t.dst)
                .collect(),
        );
        Ok(result)
    }
}

/// `BaiduService.getAudioURL(with:langCode:)`
pub fn tts_url(text: &str, lang_code: &str) -> String {
    let trimmed = util::ns_trim_to_max_length(text, TTS_TEXT_LIMIT);
    let encoded = util::url_encode_query_allowed(&trimmed);
    let speed = if lang_code == "zh" { 5 } else { 3 };
    format!("{BAIDU_TRANSLATE_URL}/gettts?text={encoded}&lan={lang_code}&spd={speed}&source=web")
}

/// `BaiduService.getTTSLanguageCode(_:accent:)` — English splits into `uk`/`en`.
pub fn tts_language_code(language: Language, accent: Option<&str>) -> String {
    if language == Language::English {
        return if accent == Some("uk") { "uk" } else { "en" }.to_string();
    }
    let code = if language == Language::ClassicalChinese {
        Language::SimplifiedChinese
    } else {
        language
    };
    crate::service::languages::default_language_code(ServiceType::Baidu, code).unwrap_or_default()
}

#[derive(Debug, Deserialize)]
struct BaiduApiResponse {
    #[serde(default)]
    from: Option<String>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default, rename = "trans_result")]
    trans_result: Vec<BaiduTransResult>,
}

#[derive(Debug, Deserialize)]
struct BaiduTransResult {
    #[serde(default)]
    dst: String,
}

#[derive(Debug, Deserialize)]
struct BaiduErrorResponse {
    #[serde(default, rename = "error_code")]
    error_code: String,
    #[serde(default, rename = "error_msg")]
    error_msg: String,
}

#[derive(Debug, Deserialize)]
struct BaiduDetectResponse {
    #[serde(default)]
    lan: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> BaiduService {
        let mut settings = crate::config::Settings::default();
        settings.baidu_app_id = "appid".into();
        settings.baidu_secret_key = "secret".into();
        BaiduService::new(ServiceContext::new(
            Arc::new(settings),
            ServiceType::Baidu,
            "Baidu",
        ))
    }

    #[test]
    fn credentials_require_both_flat_fields() {
        assert!(service().has_private_api_key());

        // `BaiduService.swift:53` tests the App ID/Secret Key pair, which lives
        // in the flat settings. Nothing reads the generic
        // `services.Baidu.apiKey` slot, so a value there must not open the gate
        // — the request would go out with no App ID to sign with.
        let mut settings = crate::config::Settings::default();
        settings
            .services
            .get_mut(ServiceType::Baidu.raw())
            .unwrap()
            .api_key = "generic".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Baidu, "Baidu");
        assert!(!BaiduService::new(ctx).has_private_api_key());

        // Half a pair is not a pair.
        let mut settings = crate::config::Settings::default();
        settings.baidu_app_id = "appid".into();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Baidu, "Baidu");
        assert!(!BaiduService::new(ctx).has_private_api_key());
    }

    #[test]
    fn the_flat_credentials_satisfy_prehandle() {
        // What the user actually hits: an App ID and Secret Key typed into the
        // panel must let the query through the shared pre-check. Before the
        // override existed the check read the generic `services.Baidu.apiKey`
        // slot, so this returned "Please enter your Baidu API key in Settings →
        // Services." pointing at a field the panel does not even show.
        let svc = service();
        let mut q = QueryModel::new("apple");
        q.query_from_language = Language::SimplifiedChinese;
        q.query_target_language = Language::English;
        assert!(matches!(
            svc.prehandle(&q),
            Ok(crate::service::Prehandle::Proceed)
        ));
    }

    #[test]
    fn signature_shape() {
        // md5(appid + text + salt + secret), no separators, lowercase hex.
        let app_id = "2015063000000001";
        let text = "apple";
        let salt = "1435660288";
        let secret = "12345678";
        assert_eq!(
            util::md5_hex(&format!("{app_id}{text}{salt}{secret}")),
            "f89f9594663708c1605f3d736d01d2d4"
        );
    }

    #[test]
    fn uuid_salt_is_uppercase_and_hyphenated() {
        let salt = uuid::Uuid::new_v4().to_string().to_uppercase();
        assert_eq!(salt.len(), 36);
        assert!(salt.chars().all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert!(salt.chars().filter(|c| c.is_ascii_alphabetic()).all(|c| c.is_ascii_uppercase()));
    }

    #[test]
    fn word_link_shape() {
        let svc = service();
        let mut q = QueryModel::new("apple");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        assert_eq!(
            svc.word_link(&q).as_deref(),
            Some("https://fanyi.baidu.com/#en/zh/apple")
        );
    }

    #[test]
    fn tts_url_shapes() {
        assert_eq!(
            tts_url("hello", "en"),
            "https://fanyi.baidu.com/gettts?text=hello&lan=en&spd=3&source=web"
        );
        // Chinese uses the slower speed.
        assert!(tts_url("你好", "zh").contains("spd=5"));
    }

    #[test]
    fn tts_language_codes() {
        assert_eq!(tts_language_code(Language::English, None), "en");
        assert_eq!(tts_language_code(Language::English, Some("uk")), "uk");
        assert_eq!(tts_language_code(Language::SimplifiedChinese, None), "zh");
        // Classical Chinese aliases to Simplified, like the base implementation.
        assert_eq!(tts_language_code(Language::ClassicalChinese, None), "zh");
    }

    #[test]
    fn response_decoding() {
        let ok: BaiduApiResponse =
            serde_json::from_str(r#"{"from":"en","to":"zh","trans_result":[{"src":"apple","dst":"苹果"}]}"#)
                .unwrap();
        assert_eq!(ok.trans_result[0].dst, "苹果");

        let err: BaiduErrorResponse =
            serde_json::from_str(r#"{"error_code":"54001","error_msg":"Invalid Sign"}"#).unwrap();
        assert_eq!(err.error_code, "54001");
    }

    #[test]
    fn detect_limit_is_utf16_units() {
        // 100 CJK characters exceed 73 UTF-16 units and get truncated.
        let text = "好".repeat(100);
        assert_eq!(util::ns_trim_to_max_length(&text, 73).chars().count(), 73);
        // Emoji count as two units each.
        let text = "😀".repeat(50);
        assert_eq!(util::ns_trim_to_max_length(&text, 73).chars().count(), 36);
    }
}
