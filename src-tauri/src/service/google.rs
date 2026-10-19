//! Google Translate — `GoogleService.swift` and friends.
//!
//! The `tk` request parameter is the well-known `sM` hash from Google's own
//! front-end. It is reproduced here exactly, including the detail that the `yr`
//! cache freezes the first TKK read: upstream's `updateWebAppTKK` replaces
//! `window.TKK`, but that write happens *after* the first `sign` call in every
//! code path, so the literal below is what actually signs every request.

use async_trait::async_trait;
use serde_json::Value;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
    TranslatePart, TranslateSimpleWord, TranslateWordResult, WordPhonetic,
};
use crate::net;
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

const TRANSLATE_URL: &str = "https://translate.google.com";
/// `window.TKK` in `google-translate-sign.js`.
const TKK: &str = "444000.1270171236";
const WEB_APP_PATH: &str = "/translate_a/single";
const TEXT_LIMIT: usize = 5000;
const CHINESE_TEXT_LIMIT: usize = 1800;

pub struct GoogleService {
    ctx: ServiceContext,
    /// The cached TKK — `yr` in the JS, which never changes after first read.
    tkk: String,
}

impl GoogleService {
    pub fn new(ctx: ServiceContext) -> Self {
        GoogleService {
            ctx,
            tkk: TKK.to_string(),
        }
    }

    fn max_text_length(&self, text: &str, from_language: Language) -> String {
        if from_language.is_chinese() && text.chars().count() > CHINESE_TEXT_LIMIT {
            return util::prefix_graphemes(text, CHINESE_TEXT_LIMIT);
        }
        util::ns_trim_to_max_length(text, TEXT_LIMIT)
    }

    /// `sign` from `google-translate-sign.js`.
    pub fn sign(&self, text: &str) -> String {
        google_sign(text, &self.tkk)
    }
}

/// The `sM` hash, transliterated from the JavaScript.
///
/// `xr`'s bit operations are JS 32-bit coercions applied to a value that is
/// carried as a double, so the accumulator stays `f64` while every shift and
/// xor round-trips through `i32`/`u32`.
pub fn google_sign(text: &str, tkk: &str) -> String {
    let parts: Vec<&str> = tkk.split('.').collect();
    let b: f64 = parts
        .first()
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);

    // The JS builds the UTF-8 byte sequence by hand; it is exactly UTF-8.
    let bytes = text.as_bytes();

    let mut a = b;
    for byte in bytes {
        a += *byte as f64;
        a = xr(a, "+-a^+6");
    }
    a = xr(a, "+-3^+b+-f");

    let d1: f64 = parts
        .get(1)
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let mut a = to_int32(a) as f64;
    a = (to_int32(a) ^ to_int32(d1)) as f64;

    if a < 0.0 {
        a = (to_int32(a) & 0x7FFF_FFFF) as f64 + 2147483648.0;
    }
    a %= 1_000_000.0;

    let b_i = to_int32(b);
    format!("{}.{}", a as i64, to_int32(a) ^ b_i)
}

/// `xr(a, b)`
fn xr(a: f64, b: &str) -> f64 {
    let chars: Vec<char> = b.chars().collect();
    let mut a = a;
    let mut c = 0usize;
    // `c < b.length - 2`
    while chars.len() >= 3 && c + 2 < chars.len() {
        let c2 = chars[c + 2];
        let shift: u32 = if c2 >= 'a' {
            (c2 as u32) - 87
        } else {
            c2.to_digit(10).unwrap_or(0)
        };
        let d: f64 = if chars[c + 1] == '+' {
            // `a >>> d` — ToUint32 then shift.
            (to_uint32(a) >> (shift & 31)) as f64
        } else {
            // `a << d` — ToInt32 then shift.
            (to_int32(a).wrapping_shl(shift & 31)) as f64
        };
        a = if chars[c] == '+' {
            // `(a + d) & 4294967295` is ToInt32(a + d): 4294967295 is -1 in
            // ToInt32 terms, and `x & -1 == x`.
            to_int32(a + d) as f64
        } else {
            (to_int32(a) ^ to_int32(d)) as f64
        };
        c += 3;
    }
    a
}

/// JavaScript `ToInt32`.
fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let m = x.trunc() % 4_294_967_296.0;
    let m = if m < 0.0 { m + 4_294_967_296.0 } else { m };
    if m >= 2_147_483_648.0 {
        (m - 4_294_967_296.0) as i32
    } else {
        m as i32
    }
}

/// JavaScript `ToUint32`.
fn to_uint32(x: f64) -> u32 {
    to_int32(x) as u32
}

#[async_trait]
impl QueryService for GoogleService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Google
    }

    fn identifier(&self) -> &str {
        "Google"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    /// Google serves translation, dictionary and sentence queries.
    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::INTELLIGENT
    }

    fn link(&self) -> Option<String> {
        Some(TRANSLATE_URL.to_string())
    }

    fn word_link(&self, query: &QueryModel) -> Option<String> {
        let from = self.language_code(query.query_from_language)?;
        let to = self.language_code(query.query_target_language)?;
        let text = util::url_encode_query_allowed(&self.max_text_length(
            &query.query_text,
            query.query_from_language,
        ));
        Some(format!(
            "{TRANSLATE_URL}/?sl={from}&tl={to}&text={text}&op=translate"
        ))
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let text = self.max_text_length(&query.query_text, query.query_from_language);
        let is_dictionary = crate::util::analysis::should_query_dictionary(
            &text,
            query.query_from_language,
            1,
        );

        if is_dictionary {
            if let Ok(result) = self.web_app_translate(query, &text).await {
                if result.has_translated_result() {
                    return Ok(result);
                }
            }
        }
        self.gtx_translate(query, &text).await
    }
}

impl GoogleService {
    /// `webAppTranslate(_:from:to:)`
    async fn web_app_translate(
        &self,
        query: &QueryModel,
        text: &str,
    ) -> Result<QueryResult, QueryError> {
        let sl = self
            .language_code(query.query_from_language)
            .unwrap_or_else(|| "auto".to_string());
        let tl = self
            .language_code(query.query_target_language)
            .unwrap_or_else(|| "auto".to_string());
        let tk = self.sign(text);

        let url = format!("{TRANSLATE_URL}{WEB_APP_PATH}");
        let response = net::CLIENT
            .get(&url)
            .headers(net::google_headers())
            .query(&[
                ("dt", "at"),
                ("dt", "bd"),
                ("dt", "ex"),
                ("dt", "ld"),
                ("dt", "md"),
                ("dt", "qca"),
                ("dt", "rw"),
                ("dt", "rm"),
                ("dt", "ss"),
                ("dt", "t"),
                ("client", "webapp"),
                ("sl", &sl),
                ("tl", &tl),
                ("hl", "en"),
                ("otf", "2"),
                ("ssel", "3"),
                ("tsel", "0"),
                ("kc", "6"),
                ("tk", &tk),
                ("q", text),
            ])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let value: Value = net::json(response).await?;
        parse_web_app_response(&value, query, self)
    }

    /// `gtxTranslate(_:from:to:)`
    async fn gtx_translate(
        &self,
        query: &QueryModel,
        text: &str,
    ) -> Result<QueryResult, QueryError> {
        let mut result = self.new_result(query);
        let sl = self
            .language_code(query.query_from_language)
            .unwrap_or_else(|| "auto".to_string());
        let tl = self
            .language_code(query.query_target_language)
            .unwrap_or_else(|| "auto".to_string());

        let url = format!("{TRANSLATE_URL}{WEB_APP_PATH}");
        let response = net::CLIENT
            .get(&url)
            .headers(net::google_headers())
            .query(&[
                ("q", text),
                ("sl", &sl),
                ("tl", &tl),
                ("dt", "t"),
                ("dj", "1"),
                ("ie", "UTF-8"),
                ("client", "gtx"),
            ])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let value: Value = net::json(response).await?;

        if let Some(src) = value.get("src").and_then(Value::as_str) {
            result.from = Language::from_code(src);
        }
        let joined = value
            .get("sentences")
            .and_then(Value::as_array)
            .map(|sentences| {
                sentences
                    .iter()
                    .filter_map(|s| s.get("trans").and_then(Value::as_str))
                    .collect::<String>()
            })
            .unwrap_or_default();

        if joined.is_empty() {
            return Err(QueryError::new(ErrorType::NoResult));
        }

        result.translated_results = Some(util::to_paragraphs(&joined));
        result.to_speak_url = self.tts_url(&joined, &tl);
        result.from_speak_url = self.tts_url(text, &sl);
        Ok(result)
    }

    /// `translate_tts` URL, or `None` for English (which uses the RPC path).
    fn tts_url(&self, text: &str, language_code: &str) -> Option<String> {
        if language_code.starts_with("en") {
            return None;
        }
        let trimmed = util::ns_trim_to_max_length(text, 200);
        let encoded = util::url_encode_query_allowed(&trimmed);
        let tk = self.sign(&trimmed);
        let len = util::grapheme_count(&trimmed);
        Some(format!(
            "{TRANSLATE_URL}/translate_tts?ie=UTF-8&q={encoded}&tl={language_code}&total=1&idx=0&textlen={len}&tk={tk}&client=webapp&prev=input"
        ))
    }
}

/// `processWebAppTranslateResponse` — array indices matter.
fn parse_web_app_response(
    value: &Value,
    query: &QueryModel,
    service: &GoogleService,
) -> Result<QueryResult, QueryError> {
    let mut result = service.new_result(query);
    let array = value
        .as_array()
        .ok_or_else(|| QueryError::with_message(ErrorType::Api, "Failed to parse response"))?;

    // [2] is the detected source language.
    if let Some(src) = array.get(2).and_then(Value::as_str) {
        result.from = Language::from_code(src);
    }

    let mut normal_results: Vec<String> = Vec::new();

    if let Some(rows) = array.first().and_then(Value::as_array) {
        for row in rows {
            if let Some(text) = row
                .as_array()
                .and_then(|r| r.first())
                .and_then(Value::as_str)
            {
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    normal_results.push(trimmed.to_string());
                }
            }
        }

        // Phonetic from the second row's fourth element.
        if let Some(second) = rows.get(1).and_then(Value::as_array) {
            if second.len() > 1 {
                if let Some(phonetic) = second.get(3).and_then(Value::as_str) {
                    let is_english = query.query_from_language == Language::English;
                    let name = if is_english {
                        match service.ctx.settings.pronunciation_accent.as_str() {
                            "uk" => "uk_phonetic",
                            _ => "us_phonetic",
                        }
                    } else {
                        "chinese_phonetic"
                    };
                    result.word_result.get_or_insert_with(Default::default).phonetics =
                        Some(vec![WordPhonetic {
                            word: query.query_text.clone(),
                            language: query.query_from_language,
                            value: Some(phonetic.to_string()),
                            speak_url: if is_english {
                                None
                            } else {
                                result.from_speak_url.clone()
                            },
                            name: Some(name.to_string()),
                            accent: if is_english {
                                Some(service.ctx.settings.pronunciation_accent.clone())
                            } else {
                                None
                            },
                        }]);
                }
            }
        }
    }

    if !normal_results.is_empty() {
        result.translated_results = Some(normal_results.clone());
        let merged = normal_results.join("\n");
        let tl = service
            .language_code(query.query_target_language)
            .unwrap_or_default();
        result.to_speak_url = service.tts_url(&merged, &tl);
    }

    // [1] is the dictionary block.
    if let Some(dict_rows) = array.get(1).and_then(Value::as_array) {
        let mut word_result = TranslateWordResult::default();
        let mut parts: Vec<TranslatePart> = Vec::new();
        let mut simple_words: Vec<TranslateSimpleWord> = Vec::new();

        for row in dict_rows {
            let Some(cols) = row.as_array() else { continue };
            let part = cols.first().and_then(Value::as_str).unwrap_or_default();

            if let Some(means) = cols.get(1).and_then(Value::as_array) {
                // English -> Chinese: a plain list of meanings.
                let means: Vec<String> = means
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                if !means.is_empty() {
                    parts.push(TranslatePart::new(part, means));
                    continue;
                }
            }

            if let Some(entries) = cols.get(2).and_then(Value::as_array) {
                // Chinese -> English: each entry is [word, [means...]].
                for entry in entries {
                    let Some(fields) = entry.as_array() else { continue };
                    let word = fields.first().and_then(Value::as_str).unwrap_or_default();
                    let means: Vec<String> = fields
                        .get(1)
                        .and_then(Value::as_array)
                        .map(|m| {
                            m.iter()
                                .filter_map(Value::as_str)
                                .map(|s| s.to_string())
                                .collect()
                        })
                        .unwrap_or_default();
                    if word.is_empty() && means.is_empty() {
                        continue;
                    }
                    simple_words.push(TranslateSimpleWord::new(
                        word,
                        means,
                        if part.is_empty() {
                            None
                        } else {
                            Some(part.to_string())
                        },
                    ));
                }
            }
        }

        if !parts.is_empty() {
            word_result.parts = Some(parts);
        }
        if !simple_words.is_empty() {
            word_result.simple_words = Some(simple_words);
        }

        // The word result is published when it has content, or when the query
        // was short enough to be a word in any case.
        let publish = word_result.parts.is_some()
            || word_result.simple_words.is_some()
            || query.query_text.chars().count() <= 4;

        if publish {
            let existing = result.word_result.take();
            if let Some(existing) = existing {
                if word_result.phonetics.is_none() {
                    word_result.phonetics = existing.phonetics;
                }
            }
            result.word_result = Some(word_result);
        }
    }

    if result.word_result.is_none() && result.translated_results.is_none() {
        return Err(QueryError::new(ErrorType::NoResult));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> GoogleService {
        GoogleService::new(ServiceContext::new(
            Arc::new(crate::config::Settings::default()),
            ServiceType::Google,
            "Google",
        ))
    }

    #[test]
    fn to_int32_matches_js_coercion() {
        assert_eq!(to_int32(0.0), 0);
        assert_eq!(to_int32(-1.5), -1);
        assert_eq!(to_int32(4294967296.0), 0);
        assert_eq!(to_int32(4294967297.0), 1);
        assert_eq!(to_int32(2147483648.0), -2147483648);
        assert_eq!(to_int32(-2147483648.0), -2147483648);
        assert_eq!(to_int32(f64::NAN), 0);
    }

    #[test]
    fn to_uint32_matches_js_coercion() {
        assert_eq!(to_uint32(-1.0), 4294967295);
        assert_eq!(to_uint32(4294967296.0), 0);
    }

    #[test]
    fn xr_known_vector() {
        // '+a^b+-f' style op string; the loop only runs while c + 2 < len.
        // "abc" has length 3, so exactly one iteration runs.
        assert_eq!(xr(0.0, "abc"), 0.0);
        // A shorter op string performs no iterations.
        assert_eq!(xr(12345.0, "ab"), 12345.0);
    }

    #[test]
    fn sign_is_deterministic_and_shaped_like_google() {
        let svc = service();
        let a = svc.sign("hello");
        let b = svc.sign("hello");
        assert_eq!(a, b);
        let (head, tail) = a.split_once('.').expect("sign has a dot");
        assert!(head.parse::<u64>().is_ok());
        assert!(tail.parse::<i64>().is_ok());
        assert!(head.parse::<u64>().unwrap() < 1_000_000);
    }

    #[test]
    fn sign_differs_per_input() {
        let svc = service();
        assert_ne!(svc.sign("hello"), svc.sign("world"));
    }

    #[test]
    fn sign_handles_non_ascii() {
        let svc = service();
        // Multi-byte input must not panic and must produce a well-formed tk.
        let s = svc.sign("你好世界");
        assert!(s.contains('.'));
    }

    #[test]
    fn text_limits() {
        let svc = service();
        // Chinese caps at 1800 characters.
        let long = "好".repeat(2000);
        let out = svc.max_text_length(&long, Language::SimplifiedChinese);
        assert_eq!(out.chars().count(), CHINESE_TEXT_LIMIT);
        // Everything else caps at 5000.
        let long = "a".repeat(6000);
        let out = svc.max_text_length(&long, Language::English);
        assert_eq!(out.chars().count(), TEXT_LIMIT);
    }

    #[test]
    fn word_link_shape() {
        let svc = service();
        let mut q = QueryModel::new("apple");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        assert_eq!(
            svc.word_link(&q).as_deref(),
            Some("https://translate.google.com/?sl=en&tl=zh-CN&text=apple&op=translate")
        );

        // The same path with Chinese input, which is what "open in browser"
        // carries for this app's main direction.
        let mut q = QueryModel::new("你好");
        q.query_from_language = Language::SimplifiedChinese;
        q.query_target_language = Language::English;
        assert_eq!(
            svc.word_link(&q).as_deref(),
            Some("https://translate.google.com/?sl=zh-CN&tl=en&text=%E4%BD%A0%E5%A5%BD&op=translate")
        );
    }

    #[test]
    fn tts_url_is_none_for_english() {
        let svc = service();
        assert!(svc.tts_url("hello", "en").is_none());
        let url = svc.tts_url("你好", "zh-CN").unwrap();
        assert!(url.starts_with("https://translate.google.com/translate_tts?ie=UTF-8&q="));
        assert!(url.contains("&tl=zh-CN&total=1&idx=0&textlen=2&tk="));
        // The query is the whole payload here, so it has to be the text's UTF-8
        // bytes — not each byte re-read as a Latin-1 scalar.
        assert!(url.contains("&q=%E4%BD%A0%E5%A5%BD&"), "{url}");
    }

    #[test]
    fn webapp_response_parsing() {
        // `[0]` is the list of translated-sentence rows, `[1]` the dictionary
        // rows and `[2]` the detected source language.
        let value: Value = serde_json::from_str(
            r#"[[["你好","hello",null,null,10]],[["good",["好的","良好"]]],"zh-CN",null,null,null,null,[["en"],null,["en"],null,"en"]]"#,
        )
        .unwrap();
        let mut q = QueryModel::new("hello");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        let svc = service();
        let result = parse_web_app_response(&value, &q, &svc).unwrap();
        assert_eq!(result.from, Language::SimplifiedChinese);
        assert_eq!(
            result.translated_text().as_deref(),
            Some("你好")
        );
        assert!(result.word_result.is_some());
    }

    #[test]
    fn webapp_phonetic_from_second_row() {
        // The phonetic is `[0][1][3]` — the four-element second row of the
        // translated-sentence array.
        let value: Value = serde_json::from_str(
            r#"[[["你好","hello",null,null,10],["hello","həˈloʊ","həˈləʊ","həˈloʊ"]],"zh-CN"]"#,
        )
        .unwrap();
        let mut q = QueryModel::new("hello");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        let svc = service();
        let result = parse_web_app_response(&value, &q, &svc).unwrap();
        let phonetics = result.word_result.unwrap().phonetics.unwrap();
        assert_eq!(phonetics[0].value.as_deref(), Some("həˈloʊ"));
    }
}
