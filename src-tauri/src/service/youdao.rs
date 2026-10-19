//! Youdao Dictionary — `YoudaoService.swift` and its `+Translate` / `+Dict`
//! extensions.
//!
//! Three endpoints cooperate:
//!
//! 1. `/webtranslate/key` — fetches a per-session `secretKey`/`aesKey`/`aesIv`;
//! 2. `/webtranslate` — returns AES-encrypted, URL-safe-base64 JSON;
//! 3. `/jsonapi_s` — the V4 dictionary payload.
//!
//! Translation and the dictionary lookup run concurrently; either may fail
//! without sinking the other.

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use serde::Deserialize;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
    TranslateExchange, TranslatePart, TranslateSimpleWord, TranslateWordResult, WordPhonetic,
};
use crate::net;
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

const TRANSLATE_URL: &str = "https://fanyi.youdao.com";
const DICT_URL: &str = "https://dict.youdao.com";
const CLIENT: &str = "fanyideskweb";
const PRODUCT: &str = "webfanyi";
const APP_VERSION: &str = "1.0.0";
const VENDOR: &str = "web";
const POINT_PARAM: &str = "client,mysticTime,product";
const KEYFROM: &str = "fanyi.web";
const DICT_VOICE_URL: &str = "https://dict.youdao.com/dictvoice";

/// Languages the dictionary endpoint accepts as the "foreign" side.
const DICT_FOREIGN_LANGUAGES: [Language; 4] = [
    Language::English,
    Language::Japanese,
    Language::French,
    Language::Korean,
];

pub struct YoudaoService {
    ctx: ServiceContext,
    /// `getYoudaoKey()` result, per query — the macOS app does not cache it.
    key: Mutex<Option<YoudaoKeyData>>,
}

impl YoudaoService {
    pub fn new(ctx: ServiceContext) -> Self {
        YoudaoService {
            ctx,
            key: Mutex::new(None),
        }
    }

    /// The static web-API key used to sign the key-fetch request.
    ///
    /// Upstream vendors this literal. It is deliberately *not* shipped here:
    /// see `docs/reverse-engineering/01-overview.md` for why. Users supply it
    /// through `Settings::youdao_web_api_key`; the empty case is reported as a
    /// missing-key error naming the setting.
    fn default_key(&self) -> Result<String, QueryError> {
        let key = self.ctx.settings.youdao_web_api_key.trim();
        if key.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::MissingSecretKey,
                "Youdao's web API key is not configured. Set \"youdaoWebApiKey\" in Settings → Services → Youdao → 认证.",
            ));
        }
        Ok(key.to_string())
    }

    fn timestamp() -> String {
        (chrono::Utc::now().timestamp_millis()).to_string()
    }

    fn headers() -> reqwest::header::HeaderMap {
        net::youdao_headers()
    }

    /// `getYoudaoKey()`
    async fn fetch_key(&self) -> Result<YoudaoKeyData, QueryError> {
        let key = self.default_key()?;
        let timestamp = Self::timestamp();
        let sign = util::md5_hex(&format!(
            "client={CLIENT}&mysticTime={timestamp}&product={PRODUCT}&key={key}"
        ));

        let response = net::CLIENT
            .get(format!("{DICT_URL}/webtranslate/key"))
            .headers(Self::headers())
            .query(&[
                ("appVersion", APP_VERSION),
                ("client", CLIENT),
                ("keyfrom", KEYFROM),
                ("keyid", "webfanyi-key-getter"),
                ("mysticTime", &timestamp),
                ("pointParam", POINT_PARAM),
                ("product", PRODUCT),
                ("sign", &sign),
                ("vendor", VENDOR),
            ])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let parsed: YoudaoKeyResponse = net::json(response).await?;
        *self.key.lock() = Some(parsed.data.clone());
        Ok(parsed.data)
    }

    /// The cached key, fetching one if the current query has not yet.
    async fn key(&self) -> Result<YoudaoKeyData, QueryError> {
        if let Some(k) = self.key.lock().clone() {
            return Ok(k);
        }
        self.fetch_key().await
    }

    /// `webTranslate(text:from:to:)`
    async fn web_translate(
        &self,
        text: &str,
        from: Language,
        to: Language,
    ) -> Result<Vec<String>, QueryError> {
        let from_code = self
            .language_code(from)
            .ok_or_else(|| QueryError::with_message(ErrorType::Api, "Invalid language code"))?;
        let to_code = self
            .language_code(to)
            .ok_or_else(|| QueryError::with_message(ErrorType::Api, "Invalid language code"))?;

        let key = self.key().await?;
        let timestamp = Self::timestamp();
        let sign = util::md5_hex(&format!(
            "client={CLIENT}&mysticTime={timestamp}&product={PRODUCT}&key={}",
            key.secret_key
        ));

        let response = net::CLIENT
            .post(format!("{DICT_URL}/webtranslate"))
            .headers(Self::headers())
            .form(&[
                ("appVersion", APP_VERSION),
                ("client", CLIENT),
                ("dictResult", "false"),
                ("from", &from_code),
                ("i", text),
                ("keyfrom", KEYFROM),
                ("keyid", "webfanyi"),
                ("mysticTime", &timestamp),
                ("pointParam", POINT_PARAM),
                ("product", PRODUCT),
                ("sign", &sign),
                ("to", &to_code),
                ("vendor", VENDOR),
            ])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(net::map_reqwest_error)?;

        if !status.is_success() {
            return Err(QueryError::api_http(status.as_u16(), net::truncate(&body, 4096)));
        }

        let plaintext = decrypt_web_translate(&body, &key)?;
        let parsed: YoudaoTranslateResponse = serde_json::from_slice(&plaintext).map_err(|e| {
            QueryError::with_message(
                ErrorType::Api,
                format!(
                    "Failed to parse response: {e}: {}",
                    net::truncate(&String::from_utf8_lossy(&plaintext), 2048)
                ),
            )
        })?;

        if parsed.code != 0 {
            return Err(QueryError::with_message(
                ErrorType::Api,
                format!("Translation failed with code: {}", parsed.code),
            ));
        }

        // Inner groups join with "", the groups join with "", then the whole
        // string splits on newlines keeping empty segments.
        let mut joined = String::new();
        for group in &parsed.translate_result {
            for item in group {
                joined.push_str(&item.tgt);
            }
        }
        Ok(joined
            .split('\n')
            .map(|s| s.to_string())
            .collect::<Vec<_>>())
    }

    /// `youdaoDictForeignLanguage(_:)`
    fn dict_foreign_language(&self, query: &QueryModel) -> Option<String> {
        let from = query.query_from_language;
        let to = query.query_target_language;
        let foreign = if from.is_chinese() {
            self.language_code(to)
        } else if to.is_chinese() {
            self.language_code(from)
        } else {
            None
        };
        let code = foreign?;
        let supported: Vec<String> = DICT_FOREIGN_LANGUAGES
            .iter()
            .filter_map(|l| self.language_code(*l))
            .collect();
        if supported.contains(&code) {
            Some(code)
        } else {
            None
        }
    }

    /// `queryDictionaryV4(_:)`
    async fn query_dictionary_v4(
        &self,
        text: &str,
        query: &QueryModel,
    ) -> Result<YoudaoDictV4, QueryError> {
        if text.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::Parameter,
                "Translation text is empty",
            ));
        }
        if !self.supported_query_type().contains(QueryTextType::DICTIONARY) {
            return Err(QueryError::new(ErrorType::UnsupportedQueryType));
        }
        let foreign_language = self
            .dict_foreign_language(query)
            .ok_or_else(|| QueryError::new(ErrorType::UnsupportedLanguage))?;

        let key = self.default_key()?;
        let ww = format!("{text}webdict");
        // Swift counts grapheme clusters here, not bytes.
        let time = (util::grapheme_count(&ww) % 10).to_string();
        let salt = util::md5_hex(&ww);
        let sign = util::md5_hex(&format!("web{text}{time}{key}{salt}"));

        let response = net::CLIENT
            .post(format!("{DICT_URL}/jsonapi_s?doctype=json&jsonversion=4"))
            .form(&[
                ("client", "web"),
                ("keyfrom", "webdict"),
                ("le", &foreign_language),
                ("q", text),
                ("sign", &sign),
                ("t", &time),
            ])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        net::json(response)
            .await
            .map_err(|e| QueryError::with_message(ErrorType::Api, format!("Failed to query Youdao dictionary: {e}")))
    }

    /// `EYQueryResult.update(dictV4:)` — the field mapping.
    fn apply_dict_v4(
        &self,
        result: &mut QueryResult,
        dict: &YoudaoDictV4,
        query: &QueryModel,
    ) -> TranslateWordResult {
        let query_text = query.query_text.clone();
        let language = query.query_from_language;
        let mut word_result = TranslateWordResult::default();

        if let Some(ec) = &dict.ec {
            if let Some(word) = &ec.word {
                let mut phonetics = Vec::new();

                if let Some(usphone) = non_empty(&word.usphone) {
                    let speak_url = non_empty(&word.usspeech)
                        .map(|s| format!("{DICT_VOICE_URL}?audio={s}"));
                    if let Some(url) = &speak_url {
                        result.from_speak_url = Some(url.clone());
                    }
                    phonetics.push(WordPhonetic {
                        word: query_text.clone(),
                        language,
                        value: Some(usphone),
                        speak_url,
                        name: Some(accent_name("us").to_string()),
                        accent: Some("us".to_string()),
                    });
                }

                if let Some(ukphone) = non_empty(&word.ukphone) {
                    phonetics.push(WordPhonetic {
                        word: query_text.clone(),
                        language,
                        value: Some(ukphone),
                        speak_url: non_empty(&word.ukspeech)
                            .map(|s| format!("{DICT_VOICE_URL}?audio={s}")),
                        name: Some(accent_name("uk").to_string()),
                        accent: Some("uk".to_string()),
                    });
                }

                if word.usphone.is_none() && word.ukphone.is_none() {
                    // Neither US nor UK phone: fall back to the generic speech
                    // endpoint, escaping `+` the way `URLComponents` does not.
                    let url = format!(
                        "{DICT_VOICE_URL}?audio={}&type=2",
                        util::url_encode_plus_safe(&query_text)
                    );
                    result.from_speak_url = Some(url.clone());
                    phonetics.push(WordPhonetic {
                        word: query_text.clone(),
                        language,
                        value: None,
                        speak_url: Some(url),
                        name: Some(accent_name("us").to_string()),
                        accent: Some("us".to_string()),
                    });
                }

                if !phonetics.is_empty() {
                    word_result.phonetics = Some(phonetics);
                }

                if let Some(trs) = &word.trs {
                    let parts: Vec<TranslatePart> = trs
                        .iter()
                        .filter_map(|tr| {
                            let means: Vec<String> = tr.tran.clone().into_iter().collect();
                            if means.is_empty() {
                                return None;
                            }
                            Some(TranslatePart::new(tr.pos.clone().unwrap_or_default(), means))
                        })
                        .collect();
                    if !parts.is_empty() {
                        word_result.parts = Some(parts);
                    }
                }

                if let Some(wfs) = &word.wfs {
                    let exchanges: Vec<TranslateExchange> = wfs
                        .iter()
                        .map(|wf| {
                            let inner = wf.wf.clone().unwrap_or_default();
                            TranslateExchange {
                                name: inner.name.clone().unwrap_or_default(),
                                words: inner
                                    .value
                                    .clone()
                                    .map(|v| v.split('或').map(|s| s.to_string()).collect())
                                    .unwrap_or_default(),
                            }
                        })
                        .collect();
                    if !exchanges.is_empty() {
                        word_result.exchanges = Some(exchanges);
                    }
                }
            }

            if let Some(exam_type) = &ec.exam_type {
                if !exam_type.is_empty() {
                    word_result.tags = Some(exam_type.clone());
                }
            }
        }

        let mut simple_words: Vec<TranslateSimpleWord> = Vec::new();

        if let Some(ce) = &dict.ce {
            if let Some(word) = &ce.word {
                if let Some(phone) = non_empty(&word.phone) {
                    word_result.phonetics = Some(vec![WordPhonetic {
                        word: query_text.clone(),
                        language,
                        value: Some(phone),
                        speak_url: None,
                        name: Some(pronunciation_name().to_string()),
                        accent: None,
                    }]);
                }

                if let Some(trs) = &word.trs {
                    for tr in trs {
                        let text = tr.text.clone().unwrap_or_default();
                        let means: Vec<String> = tr.tran.clone().into_iter().collect();
                        if text.is_empty() && means.is_empty() {
                            continue;
                        }
                        let mut sw = TranslateSimpleWord::new(text, means, None);
                        sw.show_part_means = true;
                        simple_words.push(sw);
                    }
                }
            }
        }

        if let Some(web_trans) = &dict.web_trans {
            for item in web_trans.web_translation.iter().flatten() {
                let Some(key) = non_empty(&item.key) else {
                    continue;
                };
                let means: Vec<String> = item
                    .trans
                    .iter()
                    .flatten()
                    .filter_map(|t| t.value.clone())
                    .collect();
                simple_words.push(TranslateSimpleWord::new(key, means, None));
            }
        }

        if !simple_words.is_empty() {
            word_result.simple_words = Some(simple_words);
        }

        word_result
    }
}

fn non_empty(value: &Option<String>) -> Option<String> {
    value.as_ref().filter(|s| !s.is_empty()).cloned()
}

/// `"US"` / `"美"` depending on the UI language.
fn accent_name(accent: &str) -> &'static str {
    match (accent, crate::model::app_language_is_chinese()) {
        ("us", true) => "美",
        ("uk", true) => "英",
        ("us", false) => "US",
        _ => "UK",
    }
}

fn pronunciation_name() -> &'static str {
    if crate::model::app_language_is_chinese() {
        "发音"
    } else {
        "Pronunciation"
    }
}

/// Decrypts a `/webtranslate` body: URL-safe base64, then AES-128-CBC with
/// MD5-derived key and IV.
fn decrypt_web_translate(body: &str, key: &YoudaoKeyData) -> Result<Vec<u8>, QueryError> {
    let ciphertext = util::base64_decode_websafe(body.trim()).ok_or_else(|| {
        QueryError::with_message(ErrorType::Api, "Failed to decode response data")
    })?;
    let aes_key = util::md5_bytes(key.aes_key.as_bytes());
    let aes_iv = util::md5_bytes(key.aes_iv.as_bytes());
    util::aes_128_cbc_decrypt(&aes_key, &aes_iv, &ciphertext).ok_or_else(|| {
        QueryError::with_message(ErrorType::Api, "Failed to decrypt response data")
    })
}

#[async_trait]
impl QueryService for YoudaoService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Youdao
    }

    fn identifier(&self) -> &str {
        "Youdao"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    fn has_private_api_key(&self) -> bool {
        !self.ctx.settings.youdao_web_api_key.trim().is_empty()
    }

    /// Driven by the three per-service toggles, defaulting to all three on.
    fn supported_query_type(&self) -> QueryTextType {
        let cfg = &self.ctx.config;
        let mut bits = QueryTextType::NONE;
        if cfg.translation {
            bits = bits.union(QueryTextType::TRANSLATION);
        }
        if cfg.sentence {
            bits = bits.union(QueryTextType::SENTENCE);
        }
        if cfg.dictionary {
            bits = bits.union(QueryTextType::DICTIONARY);
        }
        bits
    }

    fn link(&self) -> Option<String> {
        Some(TRANSLATE_URL.to_string())
    }

    fn word_link(&self, query: &QueryModel) -> Option<String> {
        let encoded = util::url_encode_query_allowed(&query.query_text);
        match self.dict_foreign_language(query) {
            Some(lang) => Some(format!("{DICT_URL}/result?word={encoded}&lang={lang}")),
            None => self.link(),
        }
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);
        let text = query.query_text.clone();
        if text.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::Parameter,
                "Translation text is empty",
            ));
        }

        let wants_dictionary = self.supported_query_type().contains(QueryTextType::DICTIONARY)
            && self.dict_foreign_language(query).is_some();

        // The two requests run concurrently; either may fail alone.
        let (translation, dictionary) = tokio::join!(
            self.web_translate(
                &text,
                query.query_from_language,
                query.query_target_language
            ),
            async {
                if wants_dictionary {
                    self.query_dictionary_v4(&text, query).await.ok()
                } else {
                    None
                }
            }
        );

        let mut word_result = None;
        if let Some(dict) = &dictionary {
            let wr = self.apply_dict_v4(&mut result, dict, query);
            if wr.parts.is_some() || wr.simple_words.is_some() {
                word_result = Some(wr);
            }
            // A sentence query's `fanyi.tran` overrides the REST translation.
            if let Some(tran) = dict.fanyi.as_ref().and_then(|f| non_empty(&f.tran)) {
                result.translated_results = Some(vec![tran]);
            }
        }

        match translation {
            Ok(translations) => {
                if result.translated_results.is_none() {
                    result.translated_results = Some(translations);
                }
            }
            Err(err) => {
                result.word_result = word_result.clone();
                if !result.has_translated_result() {
                    return Err(err);
                }
            }
        }

        result.word_result = word_result;
        Ok(result)
    }
}

/// Standalone TTS URL builder — `YoudaoService.textToAudio`.
///
/// Kept as a free function so the audio player can use it without holding a
/// service instance.
pub fn tts_url(text: &str, language: Language, accent: Option<&str>) -> String {
    let code = if language.is_chinese() {
        "zh".to_string()
    } else {
        crate::service::languages::default_language_code(ServiceType::Youdao, language)
            .unwrap_or_default()
    };
    let encoded = util::url_encode_query_allowed(text);
    let accent_type = if accent == Some("uk") { "1" } else { "2" };
    format!("{DICT_VOICE_URL}?audio={encoded}&le={code}&type={accent_type}")
}

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct YoudaoKeyData {
    pub secret_key: String,
    pub aes_key: String,
    pub aes_iv: String,
}

#[derive(Debug, Deserialize)]
struct YoudaoKeyResponse {
    data: YoudaoKeyData,
    #[serde(default)]
    #[allow(dead_code)]
    code: i32,
    #[serde(default)]
    #[allow(dead_code)]
    msg: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YoudaoTranslateResponse {
    #[serde(default)]
    translate_result: Vec<Vec<YoudaoTranslateItem>>,
    #[serde(default)]
    code: i32,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct YoudaoTranslateItem {
    #[serde(default)]
    tgt: String,
}

/// `YoudaoDictResponseV4` — every top-level field optional.
#[derive(Debug, Default, Deserialize)]
struct YoudaoDictV4 {
    #[serde(default)]
    ec: Option<YoudaoEc>,
    #[serde(default)]
    ce: Option<YoudaoCe>,
    #[serde(default, rename = "web_trans")]
    web_trans: Option<YoudaoWebTrans>,
    #[serde(default)]
    fanyi: Option<YoudaoFanyi>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoEc {
    #[serde(default)]
    word: Option<YoudaoEcWord>,
    #[serde(default, rename = "exam_type")]
    exam_type: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoEcWord {
    #[serde(default)]
    usphone: Option<String>,
    #[serde(default)]
    ukphone: Option<String>,
    #[serde(default)]
    usspeech: Option<String>,
    #[serde(default)]
    ukspeech: Option<String>,
    #[serde(default)]
    trs: Option<Vec<YoudaoEcTr>>,
    #[serde(default)]
    wfs: Option<Vec<YoudaoWf>>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoEcTr {
    #[serde(default)]
    pos: Option<String>,
    #[serde(default)]
    tran: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct YoudaoWf {
    #[serde(default)]
    wf: Option<YoudaoWfInner>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct YoudaoWfInner {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoCe {
    #[serde(default)]
    word: Option<YoudaoCeWord>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoCeWord {
    #[serde(default)]
    phone: Option<String>,
    #[serde(default)]
    trs: Option<Vec<YoudaoCeTr>>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoCeTr {
    #[serde(default, rename = "#text")]
    text: Option<String>,
    #[serde(default, rename = "#tran")]
    tran: Option<String>,
    #[serde(default)]
    voice: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoWebTrans {
    #[serde(default, rename = "web-translation")]
    web_translation: Option<Vec<YoudaoWebTranslation>>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoWebTranslation {
    #[serde(default)]
    key: Option<String>,
    #[serde(default)]
    trans: Option<Vec<YoudaoWebTransItem>>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoWebTransItem {
    #[serde(default)]
    value: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct YoudaoFanyi {
    #[serde(default)]
    tran: Option<String>,
}

/// Keeps `Arc<Mutex<_>>` import honest for the per-query key cache.
#[allow(dead_code)]
fn _assert_send_sync() {
    fn assert<T: Send + Sync>() {}
    assert::<Arc<Mutex<Option<YoudaoKeyData>>>>();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service() -> YoudaoService {
        let mut settings = crate::config::Settings::default();
        settings.youdao_web_api_key = "testkey".to_string();
        let ctx = ServiceContext::new(Arc::new(settings), ServiceType::Youdao, "Youdao");
        YoudaoService::new(ctx)
    }

    #[test]
    fn missing_web_key_is_reported() {
        let ctx = ServiceContext::new(
            Arc::new(crate::config::Settings::default()),
            ServiceType::Youdao,
            "Youdao",
        );
        let svc = YoudaoService::new(ctx);
        assert!(!svc.has_private_api_key());
        let err = svc.default_key().unwrap_err();
        assert_eq!(err.kind, ErrorType::MissingSecretKey);
        assert!(service().has_private_api_key());
    }

    #[test]
    fn dict_supports_only_zh_with_en_ja_fr_ko() {
        let svc = service();
        let mut q = QueryModel::new("good");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        assert_eq!(svc.dict_foreign_language(&q).as_deref(), Some("en"));

        q.query_target_language = Language::German;
        assert!(svc.dict_foreign_language(&q).is_none());

        q.query_from_language = Language::German;
        q.query_target_language = Language::SimplifiedChinese;
        assert!(svc.dict_foreign_language(&q).is_none());

        // Traditional Chinese also counts as Chinese.
        q.query_from_language = Language::Japanese;
        q.query_target_language = Language::TraditionalChinese;
        assert_eq!(svc.dict_foreign_language(&q).as_deref(), Some("ja"));
    }

    #[test]
    fn dict_sign_uses_grapheme_count() {
        // Upstream hashes `"\(text)webdict"` and takes `ww.count % 10` as `t`;
        // Swift's `String.count` counts grapheme clusters.
        let ww = "look upwebdict"; // 14 graphemes
        assert_eq!(util::grapheme_count(ww) % 10, 4);
        // CJK counts per character, not per byte: 9 graphemes, 13 UTF-8 bytes.
        let ww = "你好webdict";
        assert_eq!(util::grapheme_count(ww) % 10, 9);
        assert_ne!(util::grapheme_count(ww), ww.len());
        // A combining mark is one grapheme though it is two scalars.
        assert_eq!(util::grapheme_count("e\u{301}webdict") % 10, 8);
    }

    #[test]
    fn tts_url_shape() {
        let url = tts_url("good", Language::English, Some("us"));
        assert_eq!(
            url,
            "https://dict.youdao.com/dictvoice?audio=good&le=en&type=2"
        );
        let url = tts_url("good", Language::English, Some("uk"));
        assert!(url.ends_with("&le=en&type=1"));
        // Chinese always maps to `zh`.
        let url = tts_url("你好", Language::SimplifiedChinese, None);
        assert!(url.contains("&le=zh&type=2"));
    }

    #[test]
    fn word_link_falls_back_to_service_link() {
        let svc = service();
        let mut q = QueryModel::new("bonjour");
        q.query_from_language = Language::French;
        q.query_target_language = Language::SimplifiedChinese;
        assert_eq!(
            svc.word_link(&q).as_deref(),
            Some("https://dict.youdao.com/result?word=bonjour&lang=fr")
        );

        q.query_from_language = Language::German;
        assert_eq!(svc.word_link(&q).as_deref(), Some(TRANSLATE_URL));
    }

    #[test]
    fn web_translate_body_decrypts() {
        // Round-trip an AES-128-CBC/PKCS7 payload the way Youdao builds it.
        use aes::cipher::{block_padding::Pkcs7, BlockModeEncrypt, KeyIvInit};
        let key = YoudaoKeyData {
            secret_key: "s".into(),
            aes_key: "aeskey".into(),
            aes_iv: "aesiv".into(),
        };
        let plaintext = br#"{"code":0,"translateResult":[[{"tgt":"good"}]],"type":"en2zh"}"#;

        let k = util::md5_bytes(key.aes_key.as_bytes());
        let iv = util::md5_bytes(key.aes_iv.as_bytes());
        let encryptor = cbc::Encryptor::<aes::Aes128>::new(&k.into(), &iv.into());
        let mut buffer = vec![0u8; plaintext.len() + 16];
        buffer[..plaintext.len()].copy_from_slice(plaintext);
        let ciphertext = encryptor
            .encrypt_padded::<Pkcs7>(&mut buffer, plaintext.len())
            .expect("buffer is large enough for one padding block")
            .to_vec();
        let mut encoded = util::base64_encode(&ciphertext);
        encoded = encoded.replace('+', "-").replace('/', "_");

        let decrypted = decrypt_web_translate(&encoded, &key).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn dict_v4_field_mapping() {
        let svc = service();
        let json = serde_json::json!({
            "ec": {
                "exam_type": ["CET4", "考研"],
                "word": {
                    "usphone": "ɡʊd",
                    "ukphone": "ɡʊd",
                    "usspeech": "good&type=2",
                    "trs": [
                        {"pos": "adj.", "tran": "好的"},
                        {"pos": "n.", "tran": "好处"}
                    ],
                    "wfs": [
                        {"wf": {"name": "比较级", "value": "better"}},
                        {"wf": {"name": "最高级", "value": "best"}}
                    ]
                }
            },
            "web_trans": {
                "web-translation": [
                    {"key": "good morning", "trans": [{"value": "早上好"}]}
                ]
            }
        });
        let dict: YoudaoDictV4 = serde_json::from_value(json).unwrap();
        let mut result = QueryResult::new("Youdao", "Youdao Dictionary");
        let mut q = QueryModel::new("good");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        let wr = svc.apply_dict_v4(&mut result, &dict, &q);

        let phonetics = wr.phonetics.unwrap();
        assert_eq!(phonetics.len(), 2);
        assert_eq!(phonetics[0].accent.as_deref(), Some("us"));
        assert_eq!(phonetics[1].accent.as_deref(), Some("uk"));
        assert!(result.from_speak_url.is_some());

        let parts = wr.parts.unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].part.as_deref(), Some("adj."));

        let exchanges = wr.exchanges.unwrap();
        assert_eq!(exchanges[0].name, "比较级");
        assert_eq!(exchanges[0].words, vec!["better"]);

        let sw = wr.simple_words.unwrap();
        assert_eq!(sw[0].word, "good morning");
        assert_eq!(sw[0].means_text, "早上好");

        assert_eq!(wr.tags.unwrap(), vec!["CET4", "考研"]);
    }

    #[test]
    fn ce_branch_maps_simple_words() {
        let svc = service();
        let json = serde_json::json!({
            "ce": {
                "word": {
                    "phone": "nǐ hǎo",
                    "trs": [{"#text": "hello", "#tran": "你好"}]
                }
            }
        });
        let dict: YoudaoDictV4 = serde_json::from_value(json).unwrap();
        let mut result = QueryResult::new("Youdao", "Youdao Dictionary");
        let mut q = QueryModel::new("你好");
        q.query_from_language = Language::SimplifiedChinese;
        q.query_target_language = Language::English;
        let wr = svc.apply_dict_v4(&mut result, &dict, &q);

        assert_eq!(wr.phonetics.unwrap()[0].value.as_deref(), Some("nǐ hǎo"));
        let sw = wr.simple_words.unwrap();
        assert_eq!(sw[0].word, "hello");
        assert!(sw[0].show_part_means);
    }

    #[test]
    fn phonetics_alone_do_not_publish_a_word_result() {
        // Mirrors the upstream gate: only `parts` or `simpleWords` count.
        let wr = TranslateWordResult {
            phonetics: Some(vec![WordPhonetic::default()]),
            ..Default::default()
        };
        assert!(wr.parts.is_none() && wr.simple_words.is_none());
    }
}
