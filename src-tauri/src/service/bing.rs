//! Bing Translate — `BingService.swift`, `BingService+Translate.swift`,
//! `BingRequest.swift`, `BingConfig.swift`, `BingTranslateResponse.swift`,
//! `BingLookupResponse.swift` and `BingLanguageVoice.swift`.
//!
//! Bing has no documented translation API. The app discovers a host (whatever
//! `http://cn.bing.com` redirects to), scrapes an IG/IID pair plus a timestamped
//! key/token out of the `/translator` page, then posts to `ttranslatev3` and
//! `tlookupv3`. The scraped state is cached; upstream persists it to
//! `UserDefaults` and re-scrapes once half of the token's `expirationInterval`
//! has elapsed — here it lives in memory for the process lifetime, which is the
//! only thing `BingRequest` uses it for.
//!
//! English words translated into Chinese take a different route: Bing's
//! dictionary endpoint answers them directly, and a failure silently falls back
//! to the normal translate path (upstream's `bingTranslate(_:useDictQuery:…)`).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
    TranslateExchange, TranslatePart, TranslateSimpleWord, TranslateWordResult, WordPhonetic,
};
use crate::net;
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

/// `BingConfig.chinaHost`
const CHINA_HOST: &str = "cn.bing.com";
/// `BingService.maxTextLength(_:fromLanguage:)`
const TEXT_LIMIT: usize = 1000;
/// `wordLink(_:)` — Chinese text is capped harder or the web page errors.
const WEB_LINK_TEXT_LIMIT: usize = 450;
/// `generateSSML(text:language:accent:)`
const TTS_TEXT_LIMIT: usize = 7000;
const TTS_NON_SPACED_TEXT_LIMIT: usize = 2000;
/// `BingConfig.isBingTokenExpired()` — fallback for `expirationInterval`.
const DEFAULT_EXPIRATION_INTERVAL: f64 = 3_600_000.0;
/// `BingConfig.dictTranslateURLString` — the public `bingdict` web app id.
const DICT_APP_ID: &str = "371E7B2AF0F9B84EC491D731DF90A55719C7D209";
/// `voiceRate` in `generateSSML`; Bing's own web player uses -20%.
const VOICE_RATE: &str = "-10%";
const TTS_MIME_TYPE: &str = "audio/mpeg";

// ---------------------------------------------------------------------------
// Scraped session state
// ---------------------------------------------------------------------------

/// `BingConfig` — everything scraped from the `/translator` page.
#[derive(Debug, Clone, Default)]
pub struct BingConfig {
    /// `IG:"8E24D5A82C3240C8A68683C2484870E6"`
    pub ig: Option<String>,
    /// `data-iid="translator.5029"`
    pub iid: Option<String>,
    /// `params_AbusePreventionHelper[0]` — a millisecond timestamp, despite
    /// the field name.
    pub key: Option<String>,
    /// `params_AbusePreventionHelper[1]`
    pub token: Option<String>,
    /// `params_AbusePreventionHelper[2]` — token lifetime in milliseconds.
    pub expiration_interval: Option<String>,
    pub host: Option<String>,
}

impl BingConfig {
    /// `BingConfig.translatorURLString`
    pub fn translator_url(&self) -> String {
        format!("https://{}/translator", self.host_or_china())
    }

    /// `BingConfig.ttranslatev3URLString`
    pub fn ttranslatev3_url(&self) -> String {
        self.url_with_path("ttranslatev3")
    }

    /// `BingConfig.tlookupv3URLString`
    pub fn tlookupv3_url(&self) -> String {
        self.url_with_path("tlookupv3")
    }

    /// `BingConfig.tfetttsURLString`
    pub fn tfettts_url(&self) -> String {
        self.url_with_path("tfettts")
    }

    /// `BingConfig.dictTranslateURLString`
    pub fn dict_translate_url(&self) -> String {
        format!(
            "https://{}/api/v7/dictionarywords/search?appid={DICT_APP_ID}&mkt=zh-cn&pname=bingdict",
            self.host_or_china()
        )
    }

    fn host_or_china(&self) -> &str {
        self.host.as_deref().unwrap_or(CHINA_HOST)
    }

    fn url_with_path(&self, path: &str) -> String {
        format!(
            "https://{}/{path}?isVertical=1&IG={}&IID={}",
            self.host_or_china(),
            self.ig.as_deref().unwrap_or(""),
            self.iid.as_deref().unwrap_or("")
        )
    }

    /// `BingConfig.resetToken()`
    pub fn reset_token(&mut self) {
        self.ig = None;
        self.iid = None;
        self.key = None;
        self.token = None;
        self.expiration_interval = None;
    }

    /// `BingConfig.isBingTokenExpired()`.
    ///
    /// `expirationInterval` is 3_600_000 ms; the token is refreshed after half
    /// of that for a better user experience.
    pub fn is_bing_token_expired(&self) -> bool {
        self.is_bing_token_expired_at(now_millis())
    }

    /// The same arithmetic with an injected clock, so it can be tested.
    pub fn is_bing_token_expired_at(&self, now_millis: f64) -> bool {
        let Some(key) = self.key.as_deref() else {
            return true;
        };
        // `key` is the scrape timestamp in milliseconds; `Double(key) ?? 0`.
        let token_start = key.parse::<f64>().unwrap_or(0.0);
        let expiration_interval = self
            .expiration_interval
            .as_deref()
            .and_then(|value| value.parse::<f64>().ok())
            .unwrap_or(DEFAULT_EXPIRATION_INTERVAL);
        now_millis - token_start > expiration_interval / 2.0
    }
}

fn now_millis() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

pub struct BingService {
    ctx: ServiceContext,
    /// `BingRequest.bingConfig`
    config: Mutex<Option<BingConfig>>,
    /// `BingService.isDictQueryResult` — picks the `/dict/search` word link.
    is_dict_query_result: AtomicBool,
}

impl BingService {
    pub fn new(ctx: ServiceContext) -> Self {
        BingService {
            ctx,
            config: Mutex::new(None),
            is_dict_query_result: AtomicBool::new(false),
        }
    }

    fn cookie(&self) -> String {
        self.ctx.settings.bing_cookie.clone()
    }

    async fn config_snapshot(&self) -> Option<BingConfig> {
        self.config.lock().await.clone()
    }

    /// Saves the discovered host; upstream writes it back to `UserDefaults`,
    /// so it survives across queries there and across queries here.
    async fn set_host(&self, host: Option<String>) {
        let mut guard = self.config.lock().await;
        match guard.as_mut() {
            Some(config) => config.host = host,
            None => {
                *guard = Some(BingConfig {
                    host,
                    ..BingConfig::default()
                });
            }
        }
    }

    /// `BingRequest.reset()` — drop the token so the next request re-scrapes.
    async fn reset_token(&self) {
        if let Some(config) = self.config.lock().await.as_mut() {
            config.reset_token();
        }
    }

    async fn is_token_expired(&self) -> bool {
        match self.config.lock().await.as_ref() {
            Some(config) => config.is_bing_token_expired(),
            // Nothing scraped yet.
            None => true,
        }
    }

    /// `BingRequest.fetchBingHost(callback:failure:)`
    ///
    /// `www.bing.com` does not always redirect, so `cn.bing.com` is the entry
    /// point; an unreachable host falls back to the China host rather than
    /// failing the query.
    async fn fetch_bing_host(&self) -> String {
        if let Some(host) = self.config.lock().await.as_ref().and_then(|c| c.host.clone()) {
            return host;
        }

        let url = format!("http://{CHINA_HOST}");
        let cookie = self.cookie();
        let host = match net::CLIENT
            .get(&url)
            .headers(net::headers(&[
                ("User-Agent", net::USER_AGENT_ANDROID),
                ("Cookie", cookie.as_str()),
            ]))
            .send()
            .await
        {
            Ok(response) => response
                .url()
                .host_str()
                .unwrap_or(CHINA_HOST)
                .to_string(),
            Err(_) => CHINA_HOST.to_string(),
        };

        self.set_host(Some(host.clone())).await;
        host
    }

    /// `BingRequest.fetchBingConfig(callback:failure:)` — scrapes IG, IID and
    /// the `params_AbusePreventionHelper` triple out of `/translator`.
    async fn fetch_bing_config(&self) -> Result<(), QueryError> {
        if !self.is_token_expired().await {
            return Ok(());
        }

        let host = self.fetch_bing_host().await;
        let url = format!("https://{host}/translator");
        let response = net::CLIENT
            .get(&url)
            .headers(net::headers(&[("User-Agent", net::USER_AGENT_ANDROID)]))
            .send()
            .await
            .map_err(net::map_reqwest_error)?;
        let html = net::text(response).await?;

        let ig = get_ig_value(&html)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| QueryError::with_message(ErrorType::Api, "bing IG is empty"))?;
        let iid = get_data_iid_value(&html)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| QueryError::with_message(ErrorType::Api, "bing IID is empty"))?;
        let helper = get_params_abuse_prevention_helper_array(&html)
            .filter(|array| array.len() == 3)
            .ok_or_else(|| {
                QueryError::with_message(ErrorType::Api, "bing get key and token failed")
            })?;

        let key = helper[0].clone();
        if key.is_empty() {
            return Err(QueryError::with_message(ErrorType::Api, "bing key is empty"));
        }
        let token = helper[1].clone();
        if token.is_empty() {
            return Err(QueryError::with_message(
                ErrorType::Api,
                "bing token is empty",
            ));
        }
        let expiration_interval = helper[2].clone();

        let mut guard = self.config.lock().await;
        let config = guard.get_or_insert_with(BingConfig::default);
        config.ig = Some(ig);
        config.iid = Some(iid);
        config.key = Some(key);
        config.token = Some(token);
        config.expiration_interval = Some(expiration_interval);
        Ok(())
    }

    /// `BingRequest.translateText` — `ttranslatev3` and `tlookupv3` in
    /// parallel, returning `(translate body, lookup body-or-None)`.
    ///
    /// A lookup failure is only logged upstream, so it is dropped here too.
    async fn fetch_translate_data(
        &self,
        text: &str,
        from_code: &str,
        to_code: &str,
        can_retry_fetch_host: &mut bool,
    ) -> Result<(Vec<u8>, Option<Vec<u8>>), QueryError> {
        loop {
            self.fetch_bing_host().await;
            self.fetch_bing_config().await?;

            let config = self.config_snapshot().await.unwrap_or_default();
            let token = config.token.clone().unwrap_or_default();
            let key = config.key.clone().unwrap_or_default();
            let cookie = self.cookie();

            let header_pairs: [(&str, &str); 2] = [
                ("User-Agent", net::USER_AGENT_ANDROID),
                ("Cookie", cookie.as_str()),
            ];
            let translate_params: [(&str, &str); 6] = [
                ("text", text),
                ("to", to_code),
                ("token", token.as_str()),
                ("key", key.as_str()),
                ("fromLang", from_code),
                ("tryFetchingGenderDebiasedTranslations", "true"),
            ];
            let lookup_params: [(&str, &str); 5] = [
                ("text", text),
                ("to", to_code),
                ("token", token.as_str()),
                ("key", key.as_str()),
                ("from", from_code),
            ];

            let translate_url = config.ttranslatev3_url();
            let lookup_url = config.tlookupv3_url();
            let (translate_result, lookup_result) = tokio::join!(
                post_form(&translate_url, &header_pairs, &translate_params),
                post_form(&lookup_url, &header_pairs, &lookup_params),
            );

            let translate_data = translate_result?;
            let lookup_data = lookup_result.ok();

            // After switching to a host in another country the old host may
            // answer 200 with empty bodies; drop it and retry once.
            if *can_retry_fetch_host
                && translate_data.is_empty()
                && lookup_data.as_ref().is_some_and(|body| body.is_empty())
            {
                *can_retry_fetch_host = false;
                self.reset_token().await;
                self.set_host(None).await;
                continue;
            }

            return Ok((translate_data, lookup_data));
        }
    }

    /// `BingRequest.translateTextFromDict(text:)` + `parseBingDictTranslate`.
    ///
    /// The dictionary endpoint needs a host but no token.
    async fn dict_translate(
        &self,
        query: &QueryModel,
        text: &str,
    ) -> Result<QueryResult, QueryError> {
        self.fetch_bing_host().await;
        let config = self.config_snapshot().await.unwrap_or_default();
        let cookie = self.cookie();

        let response = net::CLIENT
            .get(config.dict_translate_url())
            .headers(net::headers(&[
                ("User-Agent", net::USER_AGENT_ANDROID),
                ("Cookie", cookie.as_str()),
            ]))
            .query(&[("q", text)])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let value: Value = net::json(response).await.map_err(|_| {
            QueryError::with_message(ErrorType::Api, "bing dict translate json parse fail")
        })?;

        parse_bing_dict_translate(&value, text, query, self)
    }

    /// `bingTranslate(_:useDictQuery:from:to:completion:)` for the
    /// non-dictionary path.
    async fn bing_translate(
        &self,
        query: &QueryModel,
        raw_text: &str,
    ) -> Result<QueryResult, QueryError> {
        let text = Self::max_text_length(raw_text);
        let from_code = self.language_code(query.query_from_language).unwrap_or_default();
        let to_code = self
            .language_code(query.query_target_language)
            .unwrap_or_default();

        // `BingService.canRetry` — status code 205 only retries once.
        let mut can_retry = true;
        // `BingRequest.canRetryFetchHost`
        let mut can_retry_fetch_host = true;

        loop {
            let (translate_data, lookup_data) = self
                .fetch_translate_data(&text, &from_code, &to_code, &mut can_retry_fetch_host)
                .await?;

            let mut result = self.new_result(query);
            match parse_translate_response(&translate_data, &text, query, &mut result) {
                TranslateOutcome::Parsed => {
                    if let Some(lookup_data) = lookup_data {
                        process_word_simple_word_and_part(&lookup_data, &mut result);
                    }
                    return Ok(result);
                }
                TranslateOutcome::TokenInvalid(message) => {
                    if can_retry {
                        can_retry = false;
                        self.reset_token().await;
                        continue;
                    }
                    return Err(QueryError::with_message(ErrorType::Api, message));
                }
                TranslateOutcome::Failed(message) => {
                    return Err(QueryError::with_message(ErrorType::Api, message));
                }
            }
        }
    }

    /// `BingService.textToAudio(_:fromLanguage:accent:)` — the file cache and
    /// playback live in the caller's audio module here.
    pub async fn text_to_audio(
        &self,
        text: &str,
        from: Language,
        accent: Option<&str>,
    ) -> Result<Option<Vec<u8>>, QueryError> {
        // Classical Chinese has no voice of its own.
        let language = if from == Language::ClassicalChinese {
            Language::SimplifiedChinese
        } else {
            from
        };
        self.fetch_text_to_audio(text, language, accent).await
    }

    /// `BingRequest.fetchTextToAudio(text:fromLanguage:accent:completion:)`
    pub async fn fetch_text_to_audio(
        &self,
        text: &str,
        from: Language,
        accent: Option<&str>,
    ) -> Result<Option<Vec<u8>>, QueryError> {
        let ssml = generate_ssml(text, from, accent);
        // A changed host is retried once, as upstream's recursion does.
        let mut can_retry = true;

        loop {
            self.fetch_bing_host().await;
            self.fetch_bing_config().await?;

            let config = self.config_snapshot().await.unwrap_or_default();
            let token = config.token.clone().unwrap_or_default();
            let key = config.key.clone().unwrap_or_default();
            let cookie = self.cookie();
            let header_pairs: [(&str, &str); 2] = [
                ("User-Agent", net::USER_AGENT_ANDROID),
                ("Cookie", cookie.as_str()),
            ];
            let params: [(&str, &str); 3] = [
                ("ssml", ssml.as_str()),
                ("token", token.as_str()),
                ("key", key.as_str()),
            ];

            let response = net::CLIENT
                .post(config.tfettts_url())
                .headers(net::headers(&header_pairs))
                .form(&params)
                .send()
                .await
                .map_err(net::map_reqwest_error)?;

            let status = response.status();
            let mime_type = response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(|value| value.trim().to_string());
            let response_host = response.url().host_str().map(str::to_string);
            let body = response.bytes().await.map_err(net::map_reqwest_error)?;

            if !status.is_success() {
                return Err(QueryError::api_http(
                    status.as_u16(),
                    net::truncate(&String::from_utf8_lossy(&body), 4096),
                ));
            }

            if mime_type.as_deref() == Some(TTS_MIME_TYPE) {
                return Ok(Some(body.to_vec()));
            }

            // Not audio: if the request landed on another host, remember it and
            // try once more.
            if can_retry {
                if let Some(host) = response_host {
                    if config.host.as_deref() != Some(host.as_str()) {
                        can_retry = false;
                        self.set_host(Some(host)).await;
                        continue;
                    }
                }
            }

            return Ok(None);
        }
    }

    /// `BingService.maxTextLength(_:fromLanguage:)` — a grapheme cap, and it
    /// does not trim.
    pub fn max_text_length(text: &str) -> String {
        if util::grapheme_count(text) > TEXT_LIMIT {
            util::prefix_graphemes(text, TEXT_LIMIT)
        } else {
            text.to_string()
        }
    }
}

#[async_trait]
impl QueryService for BingService {
    fn service_type(&self) -> ServiceType {
        ServiceType::Bing
    }

    fn identifier(&self) -> &str {
        "Bing"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    /// `supportedQueryType()` is left at the base `[.translation, .sentence]`
    /// upstream, but `translate` answers English-word dictionary queries itself
    /// (`isEnglishWordToChinese`) and the config-driven
    /// `intelligentQueryTextType()` defaults to all three, so the port
    /// advertises the union.
    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION
            .union(QueryTextType::SENTENCE)
            .union(QueryTextType::DICTIONARY)
    }

    /// `isStream()`
    fn is_stream(&self) -> bool {
        false
    }

    /// `hasPrivateAPIKey()` — Bing authenticates with a scraped token and an
    /// optional cookie, never with the generic `apiKey`.
    fn has_private_api_key(&self) -> bool {
        false
    }

    /// `link()` is not overridden upstream, so it stays `nil`.
    fn link(&self) -> Option<String> {
        None
    }

    /// `wordLink(_:)`
    fn word_link(&self, query: &QueryModel) -> Option<String> {
        let from = self
            .language_code(query.query_from_language)
            .unwrap_or_default();
        let to = self
            .language_code(query.query_target_language)
            .unwrap_or_default();

        let max_text = BingService::max_text_length(&query.query_text);
        // A long Chinese text makes the web page report an error.
        let text = if query.query_from_language.is_chinese() {
            util::prefix_graphemes(max_text.trim(), WEB_LINK_TEXT_LIMIT)
        } else {
            max_text
        };
        let encoded_text = util::url_encode_query_allowed(&text);

        // `word_link` is synchronous, so the cached host is read with a
        // non-blocking lock; an in-flight refresh falls back to the same host
        // the config starts from.
        let host = self
            .config
            .try_lock()
            .ok()
            .and_then(|guard| guard.as_ref().and_then(|config| config.host.clone()))
            .unwrap_or_else(|| CHINA_HOST.to_string());

        if self.is_dict_query_result.load(Ordering::Relaxed) {
            return Some(format!("https://{host}/dict/search?q={encoded_text}"));
        }
        Some(format!(
            "https://{host}/translator/?text={encoded_text}&from={from}&to={to}"
        ))
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        let text = query.query_text.clone();
        self.is_dict_query_result.store(false, Ordering::Relaxed);

        if is_english_word_to_chinese(&text, query.query_from_language, query.query_target_language)
        {
            if let Ok(mut result) = self.dict_translate(query, &text).await {
                self.is_dict_query_result.store(true, Ordering::Relaxed);
                // Recompute the link, now that the dict path is known to have
                // answered (`wordLink` is evaluated lazily upstream).
                result.link = self.word_link(query);
                return Ok(result);
            }
            // The dictionary query is best-effort: any failure falls through to
            // the normal translate path.
        }

        self.bing_translate(query, &text).await
    }
}

/// `BingService.isEnglishWordToChinese(_:from:to:)`
fn is_english_word_to_chinese(text: &str, from: Language, to: Language) -> bool {
    from == Language::English
        && to == Language::SimplifiedChinese
        && util::analysis::should_query_dictionary(text, from, 1)
}

/// POSTs an `application/x-www-form-urlencoded` body and returns the bytes.
async fn post_form(
    url: &str,
    header_pairs: &[(&str, &str)],
    params: &[(&str, &str)],
) -> Result<Vec<u8>, QueryError> {
    let response = net::CLIENT
        .post(url)
        .headers(net::headers(header_pairs))
        .form(params)
        .send()
        .await
        .map_err(net::map_reqwest_error)?;

    let status = response.status();
    let bytes = response.bytes().await.map_err(net::map_reqwest_error)?;

    if status.is_success() {
        return Ok(bytes.to_vec());
    }
    // https://immersivetranslate.com/docs/faq/#429-错误
    if status.as_u16() == 429 {
        return Err(QueryError::with_message(
            ErrorType::Api,
            "429 error, Bing translate too many requests",
        ));
    }
    Err(QueryError::api_http(
        status.as_u16(),
        net::truncate(&String::from_utf8_lossy(&bytes), 64 * 1024),
    ))
}

// ---------------------------------------------------------------------------
// `ttranslatev3` parsing
// ---------------------------------------------------------------------------

/// The three ways `processTranslateResult` can end.
enum TranslateOutcome {
    Parsed,
    /// `statusCode == 205` — the token went stale; the caller re-scrapes and
    /// retries once.
    TokenInvalid(String),
    Failed(String),
}

/// `BingService.processTranslateResult(_:text:from:to:needRetry:)`
fn parse_translate_response(
    data: &[u8],
    text: &str,
    _query: &QueryModel,
    result: &mut QueryResult,
) -> TranslateOutcome {
    if data.is_empty() {
        return TranslateOutcome::Failed("bing translate data is empty".to_string());
    }

    let json: Value = match serde_json::from_slice(data) {
        Ok(json) => json,
        Err(_) => return TranslateOutcome::Failed("bing json parse failed".to_string()),
    };

    let array = match json.as_array() {
        Some(array) => array,
        None => {
            // Through testing, 205 means the token is invalid, and it must be
            // re-fetched.
            if json.get("statusCode").and_then(Value::as_i64) == Some(205) {
                return TranslateOutcome::TokenInvalid(
                    "token invalid, please try again or restart the app.".to_string(),
                );
            }
            return TranslateOutcome::Failed("bing json parse failed".to_string());
        }
    };

    // Upstream decodes each object and drops the ones that fail (`compactMap`).
    let models: Vec<BingTranslateModel> = array
        .iter()
        .filter_map(|element| serde_json::from_value(element.clone()).ok())
        .collect();
    let Some(model) = models.first() else {
        return TranslateOutcome::Failed("bing translate parse failed".to_string());
    };

    // The phonetic comes from the raw second element: `inputTransliteration`.
    if let Some(transliteration) = array
        .get(1)
        .and_then(|second| second.get("inputTransliteration"))
        .and_then(Value::as_str)
    {
        let from_language = result.from;
        // Long inputs do not get a phonetic line.
        if is_short_word_length(text, from_language) {
            let mut word_result = result.word_result.take().unwrap_or_default();
            word_result.phonetics = Some(vec![WordPhonetic {
                word: text.to_string(),
                language: from_language,
                value: Some(transliteration.to_string()),
                speak_url: None,
                name: Some(
                    if from_language == Language::English {
                        "us_phonetic"
                    } else {
                        "chinese_phonetic"
                    }
                    .to_string(),
                ),
                accent: None,
            }]);
            result.word_result = Some(word_result);
        }
    }

    result.translated_results = model.translations.as_ref().map(|translations| {
        translations
            .iter()
            .filter_map(|translation| translation.text.clone())
            .collect()
    });

    TranslateOutcome::Parsed
}

/// `EZLanguageManager.isShortWordLength(_:language:)` — `word.length` is UTF-16
/// units, and an empty word is never "short".
fn is_short_word_length(word: &str, language: Language) -> bool {
    let length: usize = word.chars().map(char::len_utf16).sum();
    if length == 0 {
        return false;
    }
    if language.words_need_space() {
        length <= util::analysis::ENGLISH_WORD_MAX_LENGTH
    } else {
        length <= 7
    }
}

// ---------------------------------------------------------------------------
// `tlookupv3` parsing
// ---------------------------------------------------------------------------

/// `BingService.processWordSimpleWordAndPart(_:)`
fn process_word_simple_word_and_part(lookup_data: &[u8], result: &mut QueryResult) {
    let Ok(json) = serde_json::from_slice::<Value>(lookup_data) else {
        return;
    };
    let Some(array) = json.as_array() else {
        return;
    };
    let models: Vec<BingLookupModel> = array
        .iter()
        .filter_map(|element| serde_json::from_value(element.clone()).ok())
        .collect();
    let Some(model) = models.first() else {
        return;
    };

    // Group by `posTag`. Upstream uses an unordered Swift `Dictionary`;
    // insertion order is kept here so the rendered parts are stable.
    let mut tags: Vec<(String, Vec<BingLookupTranslation>)> = Vec::new();
    for translation in model.translations.as_deref().unwrap_or_default() {
        let Some(pos_tag) = translation.pos_tag.clone() else {
            continue;
        };
        match tags.iter().position(|(key, _)| *key == pos_tag) {
            Some(index) => tags[index].1.push(translation.clone()),
            None => tags.push((pos_tag, vec![translation.clone()])),
        }
    }

    let from = result.from;
    let to = result.to;
    // Keep any phonetic collected from the translate response.
    let mut word_result = result.word_result.take().unwrap_or_default();

    if matches!(
        from,
        Language::SimplifiedChinese | Language::TraditionalChinese
    ) && to == Language::English
    {
        // Chinese -> English: one simple word per translation.
        let mut simple_words: Vec<TranslateSimpleWord> = Vec::new();
        for (key, translations) in &tags {
            for translation in translations {
                let means = translation.back_translations.as_ref().map(|back| {
                    back.iter()
                        .filter_map(|entry| entry.display_text.clone())
                        .collect()
                });
                simple_words.push(make_simple_word(
                    translation.display_target.clone().unwrap_or_default(),
                    means,
                    Some(key.clone()),
                ));
            }
        }
        if !simple_words.is_empty() {
            word_result.simple_words = Some(simple_words);
        }
    } else {
        let mut parts: Vec<TranslatePart> = Vec::new();
        for (key, translations) in &tags {
            let means: Vec<String> = translations
                .iter()
                .filter_map(|translation| translation.display_target.clone())
                .collect();
            parts.push(make_part(key, means));
        }
        if !parts.is_empty() {
            word_result.parts = Some(parts);
        }
    }

    if word_result.parts.as_ref().is_some_and(|parts| !parts.is_empty())
        || word_result
            .simple_words
            .as_ref()
            .is_some_and(|words| !words.is_empty())
    {
        result.word_result = Some(word_result);
    } else if !word_result.is_empty() {
        result.word_result = Some(word_result);
    }
}

// ---------------------------------------------------------------------------
// `/api/v7/dictionarywords/search` parsing
// ---------------------------------------------------------------------------

/// `BingDictParsedData` — the buckets `parseMeaningGroup` fills.
#[derive(Default)]
struct BingDictParsedData {
    parts: Vec<TranslatePart>,
    exchanges: Vec<TranslateExchange>,
    simple_words: Vec<TranslateSimpleWord>,
    phonetics: Vec<WordPhonetic>,
    synonyms: Vec<TranslatePart>,
    antonyms: Vec<TranslatePart>,
    collocation: Vec<TranslatePart>,
}

/// `BingService.parseBingDictTranslate(_:word:completion:)`
fn parse_bing_dict_translate(
    json: &Value,
    word: &str,
    query: &QueryModel,
    service: &BingService,
) -> Result<QueryResult, QueryError> {
    let first = json
        .get("value")
        .and_then(Value::as_array)
        .and_then(|value| value.first())
        .ok_or_else(|| QueryError::with_message(ErrorType::Api, "bing dict value is empty"))?;

    let meaning_groups = first
        .get("meaningGroups")
        .and_then(Value::as_array)
        .filter(|groups| !groups.is_empty())
        .ok_or_else(|| {
            QueryError::with_message(
                ErrorType::Api,
                "bing dict translate meaning groups is empty",
            )
        })?;
    let audio_url = first.get("pronunciationAudio");

    let mut data = BingDictParsedData::default();
    for meaning_group in meaning_groups {
        parse_meaning_group(meaning_group, word, audio_url, &mut data);
    }

    let mut word_result = TranslateWordResult::default();
    apply_parsed_data(data, &mut word_result);

    let mut result = service.new_result(query);
    // The API has no field for the translated result, so it comes from the
    // first part's first mean.
    if let Some(mean) = word_result
        .parts
        .as_ref()
        .and_then(|parts| parts.first())
        .and_then(|part| part.means.first())
        .filter(|mean| !mean.is_empty())
    {
        result.translated_results = Some(vec![mean.clone()]);
    }
    result.word_result = Some(word_result);
    Ok(result)
}

/// `BingService.parseMeaningGroup(_:word:audioUrl:into:)`
fn parse_meaning_group(
    meaning_group: &Value,
    word: &str,
    audio_url: Option<&Value>,
    data: &mut BingDictParsedData,
) {
    let Some(part_of_speech) = meaning_group
        .get("partsOfSpeech")
        .and_then(Value::as_array)
        .and_then(|parts| parts.first())
    else {
        return;
    };
    let (Some(name), Some(description)) = (
        part_of_speech.get("name").and_then(Value::as_str),
        part_of_speech.get("description").and_then(Value::as_str),
    ) else {
        return;
    };
    let Some(meanings) = meaning_group
        .get("meanings")
        .and_then(Value::as_array)
        .filter(|meanings| !meanings.is_empty())
    else {
        return;
    };

    let rich_definitions = meanings
        .first()
        .and_then(|meaning| meaning.get("richDefinitions"))
        .and_then(Value::as_array)
        .map(Vec::as_slice);
    let fragments = rich_definitions
        .and_then(|definitions| definitions.first())
        .and_then(|definition| definition.get("fragments"))
        .and_then(Value::as_array)
        .map(Vec::as_slice);

    match description {
        "发音" => {
            if let Some(phonetic) = parse_phonetic(name, word, fragments, audio_url) {
                data.phonetics.push(phonetic);
            }
        }
        "快速释义" => data.parts.push(make_part(name, fragment_texts(fragments))),
        "词组" => data.simple_words.extend(parse_phrases(rich_definitions)),
        "分类词典" => parse_synonyms_and_antonyms(meanings, name, data),
        "搭配" => data
            .collocation
            .push(make_part(name, fragment_texts(fragments))),
        _ => {}
    }

    if name == "变形" {
        data.exchanges.extend(parse_exchanges(fragments));
    }
}

/// `BingService.parsePhonetic(name:word:fragments:audioUrl:)`
fn parse_phonetic(
    name: &str,
    word: &str,
    fragments: Option<&[Value]>,
    audio_url: Option<&Value>,
) -> Option<WordPhonetic> {
    if name != "US" && name != "UK" {
        return None;
    }
    let content_url = audio_url
        .and_then(|audio| audio.get("contentUrl"))
        .and_then(Value::as_str);
    let speak_url = if name == "US" {
        content_url.map(str::to_string)
    } else {
        // The UK clip shares the US file name, under a different voice path.
        content_url.map(|url| url.replace("tom", "george"))
    };

    Some(WordPhonetic {
        word: word.to_string(),
        language: Language::English,
        value: fragments
            .and_then(|fragments| fragments.first())
            .and_then(|fragment| fragment.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string),
        speak_url,
        name: Some(if name == "US" { "us_phonetic" } else { "uk_phonetic" }.to_string()),
        accent: Some(name.to_string()),
    })
}

/// `BingService.parsePhrases(richDefinitions:)`
fn parse_phrases(rich_definitions: Option<&[Value]>) -> Vec<TranslateSimpleWord> {
    let mut simple_words = Vec::new();
    for definition in rich_definitions.into_iter().flatten() {
        let Some(examples) = definition.get("examples").and_then(Value::as_array) else {
            continue;
        };
        if examples.len() != 2 {
            continue;
        }
        let (Some(phrase), Some(meaning)) = (examples[0].as_str(), examples[1].as_str()) else {
            continue;
        };
        let means: Vec<String> = meaning.split(';').map(str::to_string).collect();
        simple_words.push(make_simple_word(phrase.to_string(), Some(means), None));
    }
    simple_words
}

/// `BingService.parseSynonymsAndAntonyms(meanings:name:into:)`
fn parse_synonyms_and_antonyms(meanings: &[Value], name: &str, data: &mut BingDictParsedData) {
    let names = |key: &str| -> Vec<String> {
        meanings
            .first()
            .and_then(|meaning| meaning.get(key))
            .and_then(Value::as_array)
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry.get("name"))
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };

    let synonyms = names("synonyms");
    if !synonyms.is_empty() {
        data.synonyms.push(make_part(name, synonyms));
    }
    let antonyms = names("antonyms");
    if !antonyms.is_empty() {
        data.antonyms.push(make_part(name, antonyms));
    }
}

/// `BingService.parseExchanges(fragments:)` — `变形` fragments are
/// `"<name>：<words>"`.
fn parse_exchanges(fragments: Option<&[Value]>) -> Vec<TranslateExchange> {
    let mut exchanges = Vec::new();
    for fragment in fragments.into_iter().flatten() {
        let Some(text) = fragment.get("text").and_then(Value::as_str) else {
            continue;
        };
        let components: Vec<&str> = text.split('：').collect();
        if components.len() == 2 {
            exchanges.push(TranslateExchange {
                name: components[0].to_string(),
                words: vec![components[1].to_string()],
            });
        }
    }
    exchanges
}

/// `BingService.applyParsedData(_:to:)`
fn apply_parsed_data(data: BingDictParsedData, word_result: &mut TranslateWordResult) {
    if !data.phonetics.is_empty() {
        word_result.phonetics = Some(data.phonetics);
    }
    if !data.parts.is_empty() {
        word_result.parts = Some(data.parts);
    }
    if !data.exchanges.is_empty() {
        word_result.exchanges = Some(data.exchanges);
    }
    if !data.simple_words.is_empty() {
        word_result.simple_words = Some(data.simple_words);
    }
    if !data.synonyms.is_empty() {
        word_result.synonyms = Some(data.synonyms);
    }
    if !data.antonyms.is_empty() {
        word_result.antonyms = Some(data.antonyms);
    }
    if !data.collocation.is_empty() {
        word_result.collocation = Some(data.collocation);
    }
}

fn fragment_texts(fragments: Option<&[Value]>) -> Vec<String> {
    fragments
        .map(|fragments| {
            fragments
                .iter()
                .filter_map(|fragment| fragment.get("text"))
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `EZTranslatePart.part` runs `partAbbreviation` on assignment, so Bing's
/// part names go through the shared [`crate::model::abbreviate_part`]; like
/// upstream, that leaves an already-abbreviated `n.` alone.
fn dict_part_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    crate::model::abbreviate_part(trimmed)
}

fn make_part(name: &str, means: Vec<String>) -> TranslatePart {
    TranslatePart {
        means,
        part: Some(dict_part_name(name)),
    }
}

fn make_simple_word(
    word: String,
    means: Option<Vec<String>>,
    part: Option<String>,
) -> TranslateSimpleWord {
    let mut simple_word = TranslateSimpleWord::new(word, means.clone().unwrap_or_default(), None);
    // `meansText` is `means.joined("; ")`, and a missing `means` stays missing.
    simple_word.means = means;
    simple_word.part = part.map(|part| dict_part_name(&part));
    simple_word
}

// ---------------------------------------------------------------------------
// `tfettts` text-to-speech
// ---------------------------------------------------------------------------

/// `BingLanguageVoice`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BingLanguageVoice {
    /// BCP-47 language code, e.g. `en-US`.
    pub lang: &'static str,
    /// Azure voice name, e.g. `en-US-JennyNeural`.
    pub voice_name: &'static str,
}

/// `BingLanguageVoice.languageVoices`
pub fn language_voice(language: Language) -> Option<BingLanguageVoice> {
    use Language::*;
    let (lang, voice_name) = match language {
        SimplifiedChinese => ("zh-CN", "zh-CN-XiaoxiaoNeural"),
        TraditionalChinese => ("zh-TW", "zh-TW-HsiaoChenNeural"),
        English => ("en-US", "en-US-JennyNeural"),
        Japanese => ("ja-JP", "ja-JP-NanamiNeural"),
        Korean => ("ko-KR", "ko-KR-SunHiNeural"),
        French => ("fr-FR", "fr-FR-DeniseNeural"),
        Spanish => ("es-ES", "es-ES-ElviraNeural"),
        Portuguese => ("pt-PT", "pt-PT-RaquelNeural"),
        Italian => ("it-IT", "it-IT-ElsaNeural"),
        German => ("de-DE", "de-DE-KatjaNeural"),
        Russian => ("ru-RU", "ru-RU-SvetlanaNeural"),
        Arabic => ("ar-EG", "ar-EG-SalmaNeural"),
        Swedish => ("sv-SE", "sv-SE-HedvigNeural"),
        Romanian => ("ro-RO", "ro-RO-AlinaNeural"),
        Thai => ("th-TH", "th-TH-PremwadeeNeural"),
        Slovak => ("sk-SK", "sk-SK-ViktoriaNeural"),
        Dutch => ("nl-NL", "nl-NL-ColetteNeural"),
        Czech => ("cs-CZ", "cs-CZ-AntoninNeural"),
        Turkish => ("tr-TR", "tr-TR-EmelNeural"),
        Greek => ("el-GR", "el-GR-AthinaNeural"),
        Danish => ("da-DK", "da-DK-ChristelNeural"),
        Finnish => ("fi-FI", "fi-FI-NooraNeural"),
        Polish => ("pl-PL", "pl-PL-AgnieszkaNeural"),
        Lithuanian => ("lt-LT", "lt-LT-OnaNeural"),
        Latvian => ("lv-LV", "lv-LV-EveritaNeural"),
        Ukrainian => ("uk-UA", "uk-UA-OstapNeural"),
        Bulgarian => ("bg-BG", "bg-BG-KalinaNeural"),
        Indonesian => ("id-ID", "id-ID-DamayantiNeural"),
        Malay => ("ms-MY", "ms-MY-OsmanNeural"),
        Slovenian => ("sl-SI", "sl-SI-PetraNeural"),
        Estonian => ("et-EE", "et-EE-AnuNeural"),
        Vietnamese => ("vi-VN", "vi-VN-HoaiMyNeural"),
        Persian => ("fa-IR", "fa-IR-SimaNeural"),
        Hindi => ("hi-IN", "hi-IN-MadhurNeural"),
        Telugu => ("te-IN", "te-IN-MohanNeural"),
        Tamil => ("ta-IN", "ta-IN-PallaviNeural"),
        Urdu => ("ur-PK", "ur-PK-AsadNeural"),
        Filipino => ("fil-PH", "fil-PH-AlingNeural"),
        Khmer => ("km-KH", "km-KH-PichNeural"),
        Lao => ("lo-LA", "lo-LA-AcharaNeural"),
        Bengali => ("bn-IN", "bn-IN-AnuNeural"),
        Burmese => ("my-MM", "my-MM-ShanNeural"),
        Norwegian => ("nb-NO", "nb-NO-PernilleNeural"),
        Serbian => ("sr-SP", "sr-SP-LjubicaNeural"),
        Croatian => ("hr-HR", "hr-HR-SreckoNeural"),
        Mongolian => ("mn-MN", "mn-MN-NarangerelNeural"),
        Hebrew => ("he-IL", "he-IL-HilaNeural"),
        Georgian => ("ka-GE", "ka-GE-EkaNeural"),
        _ => return None,
    };
    Some(BingLanguageVoice { lang, voice_name })
}

/// `BingRequest.generateSSML(text:language:accent:)`
pub fn generate_ssml(text: &str, language: Language, accent: Option<&str>) -> String {
    let limit = if language.words_need_space() {
        TTS_TEXT_LIMIT
    } else {
        // Long Chinese audio is slow to synthesise.
        TTS_NON_SPACED_TEXT_LIMIT
    };
    let escaped_text = util::escape_xml(&util::prefix_graphemes(text.trim(), limit));

    let voice = if language == Language::English && accent == Some("uk") {
        Some(BingLanguageVoice {
            lang: "en-GB",
            voice_name: "en-GB-SoniaNeural",
        })
    } else {
        language_voice(language)
    };
    let lang = voice.map_or("en-US", |voice| voice.lang);
    let voice_name = voice.map_or("en-US-JennyNeural", |voice| voice.voice_name);

    format!(
        "<speak version=\"1.0\" xml:lang='{lang}'><voice name='{voice_name}'><prosody rate='{VOICE_RATE}'>{escaped_text}</prosody></voice></speak>"
    )
}

// ---------------------------------------------------------------------------
// Response models
// ---------------------------------------------------------------------------

// The fields mirror the JSON one-for-one; some are never read by Bing itself.

/// `BingTranslateModel` — one element of the `ttranslatev3` array.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct BingTranslateModel {
    #[serde(default, rename = "detectedLanguage")]
    detected_language: Option<BingDetectedLanguage>,
    #[serde(default)]
    translations: Option<Vec<BingTranslation>>,
}

/// `BingDetectedLanguage`
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct BingDetectedLanguage {
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    score: Option<f64>,
}

/// `BingTranslation`
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct BingTranslation {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    transliteration: Option<BingTransliteration>,
    #[serde(default)]
    to: Option<String>,
    #[serde(default, rename = "sentLen")]
    sent_len: Option<BingSentLen>,
}

/// `BingTransliteration`
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct BingTransliteration {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    script: Option<String>,
}

/// `BingSentLen`
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct BingSentLen {
    #[serde(default, rename = "srcSentLen")]
    src_sent_len: Option<Vec<i64>>,
    #[serde(default, rename = "transSentLen")]
    trans_sent_len: Option<Vec<i64>>,
}

/// `BingLookupModel`
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct BingLookupModel {
    #[serde(default, rename = "normalizedSource")]
    normalized_source: Option<String>,
    #[serde(default, rename = "displaySource")]
    display_source: Option<String>,
    #[serde(default)]
    translations: Option<Vec<BingLookupTranslation>>,
}

/// `BingLookupTranslation`
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct BingLookupTranslation {
    #[serde(default, rename = "normalizedTarget")]
    normalized_target: Option<String>,
    #[serde(default, rename = "displayTarget")]
    display_target: Option<String>,
    #[serde(default, rename = "posTag")]
    pos_tag: Option<String>,
    #[serde(default)]
    confidence: Option<f64>,
    #[serde(default, rename = "prefixWord")]
    prefix_word: Option<String>,
    #[serde(default, rename = "backTranslations")]
    back_translations: Option<Vec<BingLookupBackTranslation>>,
}

/// `BingLookupBackTranslation`
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
struct BingLookupBackTranslation {
    #[serde(default, rename = "normalizedText")]
    normalized_text: Option<String>,
    #[serde(default, rename = "displayText")]
    display_text: Option<String>,
    #[serde(default, rename = "numExamples")]
    num_examples: Option<i64>,
    #[serde(default, rename = "frequencyCount")]
    frequency_count: Option<i64>,
}

// ---------------------------------------------------------------------------
// Config scraping
// ---------------------------------------------------------------------------

/// `IG:"8E24D5A82C3240C8A68683C2484870E6"`
fn ig_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"IG:\s*"([^"]+)""#).expect("valid IG regex"))
}

/// `data-iid="translator.5029"`
fn data_iid_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r#"data-iid\s*=\s*"([^"]+)""#).expect("valid IID regex"))
}

/// `var params_AbusePreventionHelper = [1693880687457,"0T_WD…",3600000];`
fn params_abuse_prevention_helper_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r#"params_AbusePreventionHelper\s*=\s*\[([^\]]+)\]"#)
            .expect("valid params_AbusePreventionHelper regex")
    })
}

fn first_capture(regex: &Regex, html: &str) -> Option<String> {
    regex
        .captures(html)
        .and_then(|captures| captures.get(1))
        .map(|matched| matched.as_str().to_string())
}

/// `BingRequest.getIGValue(from:)`
fn get_ig_value(html: &str) -> Option<String> {
    first_capture(ig_regex(), html)
}

/// `BingRequest.getDataIidValue(from:)`
fn get_data_iid_value(html: &str) -> Option<String> {
    first_capture(data_iid_regex(), html)
}

/// `BingRequest.getParamsAbusePreventionHelperArray(from:)` — `[key, token,
/// expirationInterval]`, with the quotes stripped.
fn get_params_abuse_prevention_helper_array(html: &str) -> Option<Vec<String>> {
    let array_string = first_capture(params_abuse_prevention_helper_regex(), html)?;
    let cleaned = array_string.replace('"', "");
    Some(cleaned.split(',').map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> BingService {
        BingService::new(ServiceContext::new(
            Arc::new(crate::config::Settings::default()),
            ServiceType::Bing,
            "Bing",
        ))
    }

    fn apple_query() -> QueryModel {
        let mut query = QueryModel::new("apple");
        query.query_from_language = Language::English;
        query.query_target_language = Language::SimplifiedChinese;
        query
    }

    #[test]
    fn scrape_regexes_extract_ig_iid_and_token_params() {
        let html = r#"<!DOCTYPE html><html><head><script type="text/javascript">
            var _G = { IG:"8E24D5A82C3240C8A68683C2484870E6", EventID:"1" };
            </script></head>
            <body><div class="rich_tta" data-iid="translator.5029" data-ibp="1"></div>
            <script>var params_AbusePreventionHelper = [1693880687457,"0T_WDBmVBWjrlS5lBJPS6KYPLOboyyrf",3600000];</script>
            </body></html>"#;

        assert_eq!(
            get_ig_value(html).as_deref(),
            Some("8E24D5A82C3240C8A68683C2484870E6")
        );
        assert_eq!(get_data_iid_value(html).as_deref(), Some("translator.5029"));
        assert_eq!(
            get_params_abuse_prevention_helper_array(html),
            Some(vec![
                "1693880687457".to_string(),
                "0T_WDBmVBWjrlS5lBJPS6KYPLOboyyrf".to_string(),
                "3600000".to_string(),
            ])
        );
    }

    #[test]
    fn scrape_regexes_return_none_when_absent() {
        let html = "<html><body>nothing to see</body></html>";
        assert_eq!(get_ig_value(html), None);
        assert_eq!(get_data_iid_value(html), None);
        assert_eq!(get_params_abuse_prevention_helper_array(html), None);
    }

    #[test]
    fn translate_response_decodes_text_and_phonetic() {
        let body = r#"[{"detectedLanguage":{"language":"en","score":1.0},"translations":[{"text":"苹果","to":"zh-Hans","sentLen":{"srcSentLen":[5],"transSentLen":[2]}}]},{"inputTransliteration":"ˈæpəl"}]"#;
        let query = apple_query();
        let service = service();
        let mut result = service.new_result(&query);

        let outcome = parse_translate_response(body.as_bytes(), "apple", &query, &mut result);
        assert!(matches!(outcome, TranslateOutcome::Parsed));
        assert_eq!(result.translated_text().as_deref(), Some("苹果"));

        let phonetics = result.word_result.unwrap().phonetics.unwrap();
        assert_eq!(phonetics.len(), 1);
        assert_eq!(phonetics[0].value.as_deref(), Some("ˈæpəl"));
        assert_eq!(phonetics[0].name.as_deref(), Some("us_phonetic"));
        assert_eq!(phonetics[0].word, "apple");
        assert_eq!(phonetics[0].language, Language::English);
    }

    #[test]
    fn translate_response_reports_token_invalid() {
        let body = br#"{"statusCode":205,"errorMessage":"Invalid token","errorCode":""}"#;
        let query = apple_query();
        let service = service();
        let mut result = service.new_result(&query);

        match parse_translate_response(body, "apple", &query, &mut result) {
            TranslateOutcome::TokenInvalid(message) => {
                assert_eq!(message, "token invalid, please try again or restart the app.")
            }
            _ => panic!("expected a token-invalid outcome"),
        }
        // Other top-level objects (no 205) stay a plain parse failure.
        let body = br#"{"statusCode":400,"errorMessage":"Bad request"}"#;
        match parse_translate_response(body, "apple", &query, &mut result) {
            TranslateOutcome::Failed(message) => assert_eq!(message, "bing json parse failed"),
            _ => panic!("expected a parse failure"),
        }
    }

    #[test]
    fn translate_response_handles_empty_and_malformed_bodies() {
        let query = apple_query();
        let service = service();
        let mut result = service.new_result(&query);

        match parse_translate_response(b"", "apple", &query, &mut result) {
            TranslateOutcome::Failed(message) => assert_eq!(message, "bing translate data is empty"),
            _ => panic!("expected an empty-data failure"),
        }
        match parse_translate_response(b"not json", "apple", &query, &mut result) {
            TranslateOutcome::Failed(message) => assert_eq!(message, "bing json parse failed"),
            _ => panic!("expected a parse failure"),
        }
        match parse_translate_response(b"[]", "apple", &query, &mut result) {
            TranslateOutcome::Failed(message) => assert_eq!(message, "bing translate parse failed"),
            _ => panic!("expected a parse failure"),
        }
    }

    #[test]
    fn lookup_response_builds_parts() {
        let body = r#"[{"normalizedSource":"apple","displaySource":"apple","translations":[
            {"displayTarget":"苹果","posTag":"noun","confidence":0.9,"backTranslations":[{"displayText":"apple"}]},
            {"displayTarget":"苹果树","posTag":"noun"},
            {"displayTarget":"苹果的","posTag":"adjective"}
        ]}]"#;
        let query = apple_query();
        let service = service();
        let mut result = service.new_result(&query);

        process_word_simple_word_and_part(body.as_bytes(), &mut result);

        let parts = result.word_result.unwrap().parts.unwrap();
        // `posTag` groups keep first-seen order: noun, then adjective.
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].part.as_deref(), Some("n."));
        assert_eq!(parts[0].means, vec!["苹果", "苹果树"]);
        assert_eq!(parts[1].part.as_deref(), Some("adj."));
        assert_eq!(parts[1].means, vec!["苹果的"]);
    }

    #[test]
    fn lookup_response_builds_simple_words() {
        let body = r#"[{"translations":[{"displayTarget":"apple","posTag":"noun",
            "backTranslations":[{"displayText":"苹果"},{"displayText":"苹果树"}]}]}]"#;
        let mut query = QueryModel::new("苹果");
        query.query_from_language = Language::SimplifiedChinese;
        query.query_target_language = Language::English;
        let service = service();
        let mut result = service.new_result(&query);

        process_word_simple_word_and_part(body.as_bytes(), &mut result);

        let word_result = result.word_result.unwrap();
        assert!(word_result.parts.is_none());
        let simple_words = word_result.simple_words.unwrap();
        assert_eq!(simple_words.len(), 1);
        assert_eq!(simple_words[0].word, "apple");
        assert_eq!(simple_words[0].part.as_deref(), Some("n."));
        assert_eq!(simple_words[0].means_text, "苹果; 苹果树");
    }

    #[test]
    fn dict_response_collects_every_section() {
        let body = r#"{"value":[{
            "pronunciationAudio":{"contentUrl":"https://dictionary.example/media/audio/tom/apple.mp3"},
            "meaningGroups":[
                {"partsOfSpeech":[{"name":"US","description":"发音"}],
                 "meanings":[{"richDefinitions":[{"fragments":[{"text":"ˈæpəl"}]}]}]},
                {"partsOfSpeech":[{"name":"UK","description":"发音"}],
                 "meanings":[{"richDefinitions":[{"fragments":[{"text":"ˈæpl"}]}]}]},
                {"partsOfSpeech":[{"name":"n.","description":"快速释义"}],
                 "meanings":[{"richDefinitions":[{"fragments":[{"text":"苹果"},{"text":"苹果树"}]}]}]},
                {"partsOfSpeech":[{"name":"词组","description":"词组"}],
                 "meanings":[{"richDefinitions":[{"examples":["apple pie","苹果派;苹果馅饼"]}]}]},
                {"partsOfSpeech":[{"name":"形容词","description":"分类词典"}],
                 "meanings":[{"synonyms":[{"name":"fruity"}],"antonyms":[{"name":"savoury"}]}]},
                {"partsOfSpeech":[{"name":"变形","description":"其他"}],
                 "meanings":[{"richDefinitions":[{"fragments":[{"text":"复数：apples"}]}]}]}
            ]}]}"#;
        let value: Value = serde_json::from_str(body).unwrap();
        let query = apple_query();
        let service = service();

        let result = parse_bing_dict_translate(&value, "apple", &query, &service).unwrap();
        // Read the translated text before `word_result` is moved out, so the
        // result is not left partially moved.
        let translated = result.translated_text();
        let word_result = result.word_result.unwrap();

        let phonetics = word_result.phonetics.unwrap();
        assert_eq!(phonetics.len(), 2);
        assert_eq!(phonetics[0].value.as_deref(), Some("ˈæpəl"));
        assert_eq!(phonetics[0].accent.as_deref(), Some("US"));
        assert_eq!(
            phonetics[0].speak_url.as_deref(),
            Some("https://dictionary.example/media/audio/tom/apple.mp3")
        );
        // The UK clip swaps the voice directory.
        assert_eq!(
            phonetics[1].speak_url.as_deref(),
            Some("https://dictionary.example/media/audio/george/apple.mp3")
        );
        assert_eq!(phonetics[1].name.as_deref(), Some("uk_phonetic"));

        // `n.` is already abbreviated and must not become `n..`.
        let parts = word_result.parts.unwrap();
        assert_eq!(parts[0].part.as_deref(), Some("n."));
        assert_eq!(parts[0].means, vec!["苹果", "苹果树"]);

        let simple_words = word_result.simple_words.unwrap();
        assert_eq!(simple_words[0].word, "apple pie");
        assert_eq!(simple_words[0].means, Some(vec!["苹果派".to_string(), "苹果馅饼".to_string()]));

        assert_eq!(
            word_result.synonyms.unwrap()[0].means,
            vec!["fruity".to_string()]
        );
        assert_eq!(
            word_result.antonyms.unwrap()[0].means,
            vec!["savoury".to_string()]
        );

        let exchanges = word_result.exchanges.unwrap();
        assert_eq!(exchanges[0].name, "复数");
        assert_eq!(exchanges[0].words, vec!["apples"]);

        // The translated result is the first part's first mean.
        assert_eq!(translated.as_deref(), Some("苹果"));
    }

    #[test]
    fn dict_response_rejects_empty_sections() {
        let query = apple_query();
        let service = service();

        let empty: Value = serde_json::from_str(r#"{"value":[]}"#).unwrap();
        let error = parse_bing_dict_translate(&empty, "apple", &query, &service).unwrap_err();
        assert_eq!(error.message.as_deref(), Some("bing dict value is empty"));

        let no_groups: Value = serde_json::from_str(r#"{"value":[{}]}"#).unwrap();
        let error = parse_bing_dict_translate(&no_groups, "apple", &query, &service).unwrap_err();
        assert_eq!(
            error.message.as_deref(),
            Some("bing dict translate meaning groups is empty")
        );
    }

    #[test]
    fn language_codes_come_from_the_bing_table() {
        let service = service();
        assert_eq!(
            service.language_code(Language::Auto).as_deref(),
            Some("auto-detect")
        );
        assert_eq!(
            service.language_code(Language::SimplifiedChinese).as_deref(),
            Some("zh-Hans")
        );
        assert_eq!(
            service.language_code(Language::Portuguese).as_deref(),
            Some("pt-PT")
        );
        assert_eq!(
            service.language_code(Language::BrazilianPortuguese).as_deref(),
            Some("pt")
        );
        assert_eq!(
            service.language_code(Language::Mongolian).as_deref(),
            Some("mn-Mong")
        );
    }

    #[test]
    fn voice_table_lookup() {
        assert_eq!(
            language_voice(Language::SimplifiedChinese),
            Some(BingLanguageVoice {
                lang: "zh-CN",
                voice_name: "zh-CN-XiaoxiaoNeural"
            })
        );
        assert_eq!(language_voice(Language::Serbian).unwrap().lang, "sr-SP");
        // Catalan has no Bing voice, so `generateSSML` falls back to en-US.
        assert_eq!(language_voice(Language::Catalan), None);
        assert_eq!(language_voice(Language::ClassicalChinese), None);
    }

    #[test]
    fn ssml_body_shape() {
        assert_eq!(
            generate_ssml("hello & <world>", Language::English, None),
            "<speak version=\"1.0\" xml:lang='en-US'><voice name='en-US-JennyNeural'><prosody rate='-10%'>hello &amp; &lt;world&gt;</prosody></voice></speak>"
        );

        // The uk accent overrides the English voice.
        let uk = generate_ssml("hello", Language::English, Some("uk"));
        assert!(uk.contains("xml:lang='en-GB'"));
        assert!(uk.contains("<voice name='en-GB-SoniaNeural'>"));

        let chinese = generate_ssml("你好", Language::SimplifiedChinese, None);
        assert!(chinese.contains("xml:lang='zh-CN'"));
        assert!(chinese.contains("<voice name='zh-CN-XiaoxiaoNeural'>"));

        // Text is trimmed before being escaped, and capped: 2000 for scripts
        // without word spaces, 7000 otherwise.
        let long_chinese = "好".repeat(2500);
        let ssml = generate_ssml(&long_chinese, Language::SimplifiedChinese, None);
        assert!(ssml.contains(&"好".repeat(TTS_NON_SPACED_TEXT_LIMIT)));
        assert!(!ssml.contains(&"好".repeat(TTS_NON_SPACED_TEXT_LIMIT + 1)));

        let long_english = "a".repeat(7001);
        let ssml = generate_ssml(&long_english, Language::English, None);
        assert!(ssml.contains(&"a".repeat(TTS_TEXT_LIMIT)));
        assert!(!ssml.contains(&"a".repeat(TTS_TEXT_LIMIT + 1)));
    }

    #[test]
    fn token_expiry_arithmetic() {
        let mut config = BingConfig::default();
        // Nothing scraped yet: always expired.
        assert!(config.is_bing_token_expired_at(1_700_000_000_000.0));

        config.key = Some("1000".to_string());
        config.expiration_interval = Some("3600000".to_string());
        // The token is refreshed after half of `expirationInterval`.
        assert!(!config.is_bing_token_expired_at(1000.0 + 1_799_999.0));
        assert!(config.is_bing_token_expired_at(1000.0 + 1_800_001.0));

        // A missing interval falls back to 3_600_000 ms.
        config.expiration_interval = None;
        assert!(!config.is_bing_token_expired_at(1000.0 + 1_799_999.0));
        assert!(config.is_bing_token_expired_at(1000.0 + 1_800_001.0));

        // A non-numeric key parses as 0 and is therefore long expired.
        config.key = Some("not-a-timestamp".to_string());
        assert!(config.is_bing_token_expired_at(1_700_000_000_000.0));

        // `resetToken` clears the key.
        config.reset_token();
        assert_eq!(config.key, None);
        assert!(config.is_bing_token_expired_at(0.0));
    }

    #[test]
    fn url_builders_use_the_cached_host() {
        let mut config = BingConfig::default();
        assert_eq!(config.translator_url(), "https://cn.bing.com/translator");
        assert_eq!(
            config.ttranslatev3_url(),
            "https://cn.bing.com/ttranslatev3?isVertical=1&IG=&IID="
        );
        assert!(config
            .dict_translate_url()
            .starts_with("https://cn.bing.com/api/v7/dictionarywords/search?appid="));

        config.host = Some("www.bing.com".to_string());
        config.ig = Some("IG1".to_string());
        config.iid = Some("translator.5029".to_string());
        assert_eq!(
            config.tlookupv3_url(),
            "https://www.bing.com/tlookupv3?isVertical=1&IG=IG1&IID=translator.5029"
        );
        assert_eq!(
            config.tfettts_url(),
            "https://www.bing.com/tfettts?isVertical=1&IG=IG1&IID=translator.5029"
        );
    }

    #[test]
    fn max_text_length_caps_at_graphemes() {
        assert_eq!(BingService::max_text_length("hello"), "hello");

        let long = "a".repeat(1500);
        let capped = BingService::max_text_length(&long);
        assert_eq!(util::grapheme_count(&capped), TEXT_LIMIT);
        assert_eq!(capped.len(), TEXT_LIMIT);

        // A multi-code-point grapheme counts once, and no trim is applied.
        let emoji = "👩👩👧👦".repeat(1200);
        let capped = BingService::max_text_length(&emoji);
        assert_eq!(util::grapheme_count(&capped), TEXT_LIMIT);

        let padded = format!("  {}  ", "a".repeat(1500));
        let capped = BingService::max_text_length(&padded);
        assert!(util::grapheme_count(&capped) <= TEXT_LIMIT);
        assert!(capped.starts_with(' '));
    }

    #[test]
    fn short_word_length_rules() {
        // Spaced languages allow up to `englishWordMaxLength` UTF-16 units.
        assert!(is_short_word_length("apple", Language::English));
        assert!(!is_short_word_length("", Language::English));
        assert!(!is_short_word_length(
            &"a".repeat(util::analysis::ENGLISH_WORD_MAX_LENGTH + 1),
            Language::English
        ));
        // Scripts without word spaces cap at 7.
        let seven = "你好你好你好你";
        assert_eq!(util::grapheme_count(seven), 7);
        assert!(is_short_word_length(seven, Language::SimplifiedChinese));
        assert!(!is_short_word_length(
            "你好你好你好你好",
            Language::SimplifiedChinese
        ));
    }

    #[test]
    fn dict_part_name_keeps_dotted_abbreviations() {
        assert_eq!(dict_part_name("noun"), "n.");
        assert_eq!(dict_part_name("n."), "n.");
        assert_eq!(dict_part_name("adjective"), "adj.");
        assert_eq!(dict_part_name("形容词"), "adj.");
        assert_eq!(dict_part_name(""), "");
    }

    #[test]
    fn word_link_shape_and_identity() {
        let service = service();
        let query = apple_query();
        assert_eq!(
            service.word_link(&query).as_deref(),
            Some("https://cn.bing.com/translator/?text=apple&from=en&to=zh-Hans")
        );
        // `link()` is not overridden upstream.
        assert_eq!(service.link(), None);
        assert_eq!(service.identifier(), "Bing");
        assert!(!service.is_stream());
        assert_eq!(
            service.supported_query_type(),
            QueryTextType::INTELLIGENT
        );
        assert!(service
            .supported_query_type()
            .contains(QueryTextType::DICTIONARY));
    }

    #[test]
    fn word_link_switches_to_the_dict_page() {
        let service = service();
        let query = apple_query();
        service.is_dict_query_result.store(true, Ordering::Relaxed);
        assert_eq!(
            service.word_link(&query).as_deref(),
            Some("https://cn.bing.com/dict/search?q=apple")
        );
    }

    #[test]
    fn english_word_to_chinese_routing() {
        assert!(is_english_word_to_chinese(
            "apple",
            Language::English,
            Language::SimplifiedChinese
        ));
        // Multi-word text is not a dictionary query.
        assert!(!is_english_word_to_chinese(
            "apple pie",
            Language::English,
            Language::SimplifiedChinese
        ));
        // Traditional Chinese targets use the normal translate path.
        assert!(!is_english_word_to_chinese(
            "apple",
            Language::English,
            Language::TraditionalChinese
        ));
        assert!(!is_english_word_to_chinese(
            "苹果",
            Language::SimplifiedChinese,
            Language::English
        ));
    }
}
