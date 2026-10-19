//! DeepL — `DeepLService.swift`, `DeepLService+Translate.swift`,
//! `DeepLTranslateResponse.swift` and `DeepLWebAppVersionProvider.swift`.
//!
//! Two transports exist upstream, picked by `DeepLApiUsagePriority`:
//!
//! * the anonymous oneshot web endpoint, which needs no key but speaks a
//!   DeepL-iOS-client dialect (App Store version, instance/session ids);
//! * the official `v2/translate` API, whose host depends on whether the key
//!   ends in `:fx`.
//!
//! The default priority is web-first, and each transport falls back to the
//! other on error exactly where upstream's completion handlers do.
//!
//! Cancellation (`DeepLWebTranslationCancellation`, `queryModel.setStop`) has
//! no equivalent: the query engine drops the future instead.

use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::DeepLApiUsagePriority;
use crate::model::{
    ErrorType, Language, QueryError, QueryModel, QueryResult, QueryTextType, ServiceType,
};
use crate::net;
use crate::service::{QueryService, ServiceContext, UpdateSender};
use crate::util;

/// `kDeepLTranslateURL`
const TRANSLATE_URL: &str = "https://www.deepl.com/translator";
/// `kDeepLWebURL`
const WEB_URL: &str = "https://oneshot-free.www.deepl.com/v1/translate";
/// `kDeepLWebOSVersion`
const WEB_OS_VERSION: &str = "26.0";
/// `kDeepLWebAppBuild`
const WEB_APP_BUILD: &str = "5443737";
/// Tail of `deepLWebUserAgent(appVersion:)`.
const WEB_USER_AGENT_SUFFIX: &str = "CFNetwork/3826.600.41 Darwin/25.0.0";

/// DeepL documents a 128 KiB cap on a `/v2/translate` request body. The Swift
/// code does not trim, so this is the port's safety net, applied with the same
/// `ns_trimToMaxLength` helper the other services use.
const TEXT_LIMIT: usize = 128 * 1024;

/// `kDeepLAppStoreLookupURL`
const APP_STORE_LOOKUP_URL: &str = "https://itunes.apple.com/lookup?id=1552407475&country=us";
/// `kDeepLWebAppVersionFallback`
const WEB_APP_VERSION_FALLBACK: &str = "26.52";
/// `kDeepLWebAppVersionCacheLifetime`
const WEB_APP_VERSION_CACHE_LIFETIME: f64 = 24.0 * 60.0 * 60.0;
/// `kDeepLWebAppVersionRetryDelay`
const WEB_APP_VERSION_RETRY_DELAY: f64 = 15.0 * 60.0;
/// `kDeepLWebAppVersionLookupTimeout`
const WEB_APP_VERSION_LOOKUP_TIMEOUT: Duration = Duration::from_secs(3);

// `DeepLAppStoreVersionParser.version(from:)` only trusts this exact app.
const APP_STORE_TRACK_ID: i64 = 1_552_407_475;
const APP_STORE_BUNDLE_ID: &str = "com.linguee.DeepLMobileTranslator";
const APP_STORE_ARTIST_NAME: &str = "DeepL SE";

pub struct DeepLService {
    ctx: ServiceContext,
}

impl DeepLService {
    pub fn new(ctx: ServiceContext) -> Self {
        DeepLService { ctx }
    }

    /// `Defaults[.deepLAuth]`
    fn auth_key(&self) -> String {
        self.ctx.settings.deep_l_auth.trim().to_string()
    }

    /// `Defaults[.deepLTranslation]`
    fn priority(&self) -> DeepLApiUsagePriority {
        self.ctx.settings.deep_l_translation
    }

    fn max_text_length(&self, text: &str) -> String {
        util::ns_trim_to_max_length(text, TEXT_LIMIT)
    }
}

#[async_trait]
impl QueryService for DeepLService {
    fn service_type(&self) -> ServiceType {
        ServiceType::DeepL
    }

    fn identifier(&self) -> &str {
        "DeepL"
    }

    fn ctx(&self) -> &ServiceContext {
        &self.ctx
    }

    /// DeepL does not override `supportedQueryType()`, so it keeps the base
    /// `[.translation, .sentence]`.
    fn supported_query_type(&self) -> QueryTextType {
        QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
    }

    fn is_stream(&self) -> bool {
        false
    }

    fn link(&self) -> Option<String> {
        Some(TRANSLATE_URL.to_string())
    }

    /// https://www.deepl.com/translator#en/zh-hans/good
    fn word_link(&self, query: &QueryModel) -> Option<String> {
        let from = remove_language_variant(&self.language_code(query.query_from_language)?);
        let to = self.language_code(query.query_target_language)?;
        let text = util::url_encode_query_allowed(&query.query_text);
        // `urlQueryAllowed` leaves `/` alone; DeepL's fragment wants it escaped
        // (Easydict issue #60): computer/server -> computer%5C%2Fserver.
        let encoded = text.replace('/', "%5C%2F");
        Some(format!("{TRANSLATE_URL}#{from}/{to}/{encoded}"))
    }

    /// `autoConvertTraditionalChinese()`
    fn auto_convert_traditional_chinese(&self) -> bool {
        true
    }

    async fn translate(
        &self,
        query: &QueryModel,
        _updates: Option<UpdateSender>,
    ) -> Result<QueryResult, QueryError> {
        // `performDeepLWebTranslate` rejects a missing or `auto` target before
        // it builds a request. Checked up front so a language problem can never
        // trigger the official-API fallback.
        if self
            .language_code(query.query_target_language)
            .map_or(true, |code| code == "auto")
        {
            return Err(QueryError::with_message(
                ErrorType::Api,
                "Invalid DeepL target language",
            ));
        }

        let text = self.max_text_length(&query.query_text);
        let from = query.query_from_language;
        let to = query.query_target_language;

        match self.priority() {
            // `deepLWebTranslate`, whose error handler retries on the official
            // API when an auth key is configured.
            DeepLApiUsagePriority::WebFirst => {
                match self.web_translate(query, &text, from, to).await {
                    Ok(result) => Ok(result),
                    Err(web_error) => {
                        if self.auth_key().is_empty() {
                            return Err(web_error);
                        }
                        self.official_translate(query, &text, from, to).await
                    }
                }
            }
            // `deepLTranslate`, whose error handler retries on the web client.
            DeepLApiUsagePriority::AuthKeyFirst => {
                match self.official_translate(query, &text, from, to).await {
                    Ok(result) => Ok(result),
                    Err(_) => self.web_translate(query, &text, from, to).await,
                }
            }
            DeepLApiUsagePriority::AuthKeyOnly => {
                self.official_translate(query, &text, from, to).await
            }
        }
    }
}

impl DeepLService {
    /// `deepLWebTranslate(_:from:to:)`
    async fn web_translate(
        &self,
        query: &QueryModel,
        text: &str,
        from: Language,
        to: Language,
    ) -> Result<QueryResult, QueryError> {
        let source_code = self
            .language_code(from)
            .unwrap_or_else(|| "auto".to_string());
        let target_code = self.language_code(to).unwrap_or_default();

        let provider = DeepLWebAppVersionProvider::shared();
        // Upstream persists the cache in `Defaults[.deepLWebAppVersionCache]`;
        // the settings layer owns that string, so it is only read back here.
        provider
            .restore_persisted(&self.ctx.settings.deep_l_web_app_version_cache)
            .await;
        let app_version = provider.current_version().await;

        let body = web_request_body(
            text,
            &source_code,
            &target_code,
            &app_version,
            web_instance_id(),
        );
        let user_agent = deep_l_web_user_agent(&app_version);
        let headers = net::headers(&[
            ("Content-Type", "application/json"),
            ("Authorization", "None"),
            ("User-Agent", user_agent.as_str()),
            ("x-app-os-version", WEB_OS_VERSION),
            ("x-app-instance-id", web_instance_id()),
            ("x-app-session-id", web_session_id()),
        ]);

        let response = net::CLIENT
            .post(WEB_URL)
            .headers(headers)
            .json(&body)
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        // Alamofire's `.validate(200..<300)` funnels HTTP failures into the same
        // error path as transport failures; do the same here so the API's own
        // message can be surfaced.
        let status = response.status();
        let bytes = response.bytes().await.map_err(net::map_reqwest_error)?;
        if !status.is_success() {
            return Err(web_error(status.as_u16(), &bytes));
        }

        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| QueryError::with_message(ErrorType::Api, "Invalid response"))?;
        let mut result = self.new_result(query);
        result.translated_results = Some(decode_official_response(&value)?);
        Ok(result)
    }

    /// `deepLTranslate(_:from:to:)`
    async fn official_translate(
        &self,
        query: &QueryModel,
        text: &str,
        from: Language,
        to: Language,
    ) -> Result<QueryResult, QueryError> {
        let auth_key = self.auth_key();
        // Only the *source* code drops its variant; DeepL wants the target
        // variant (`zh-hans`, `pt-BR`) preserved.
        let source_code = remove_language_variant(
            &self
                .language_code(from)
                .unwrap_or_else(|| "auto".to_string()),
        );
        let target_code = self.language_code(to).unwrap_or_default();
        let url = official_url(&auth_key, &self.ctx.settings.deep_l_translate_end_point);

        let response = net::CLIENT
            .post(&url)
            .header("Authorization", format!("DeepL-Auth-Key {auth_key}"))
            .form(&[
                ("text", text),
                ("source_lang", source_code.as_str()),
                ("target_lang", target_code.as_str()),
            ])
            .send()
            .await
            .map_err(net::map_reqwest_error)?;

        let status = response.status();
        let bytes = response.bytes().await.map_err(net::map_reqwest_error)?;
        if !status.is_success() {
            return Err(web_error(status.as_u16(), &bytes));
        }

        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|_| QueryError::with_message(ErrorType::Api, "Invalid response"))?;
        let mut result = self.new_result(query);
        // Upstream leaves `translatedResults` nil *and* reports no error on a
        // parse failure here; a blank card is worse than the message the web
        // path already uses, so both transports report the same way.
        result.translated_results = Some(decode_official_response(&value)?);
        Ok(result)
    }
}

// ---------------------------------------------------------------------------
// Web request / response shapes
// ---------------------------------------------------------------------------

/// `DeepLWebTranslateRequest`
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct WebTranslateRequest {
    text: Vec<String>,
    target_lang: String,
    /// Omitted for auto-detection, like the Swift optional.
    #[serde(skip_serializing_if = "Option::is_none")]
    source_lang: Option<String>,
    usage_type: String,
    app_information: AppInformation,
}

/// `DeepLAppInformation`
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct AppInformation {
    os: String,
    os_version: String,
    app_version: String,
    app_build: String,
    instance_id: String,
}

/// `DeepLOfficialResponse`
#[derive(Debug, Deserialize)]
struct OfficialResponse {
    #[serde(default)]
    translations: Option<Vec<OfficialTranslation>>,
}

/// `DeepLOfficialTranslation`
#[derive(Debug, Deserialize)]
struct OfficialTranslation {
    #[serde(default)]
    text: Option<String>,
}

/// Builds the oneshot payload. `source_code` is the service-level code, where
/// `"auto"` means "let DeepL detect it".
fn web_request_body(
    text: &str,
    source_code: &str,
    target_code: &str,
    app_version: &str,
    instance_id: &str,
) -> WebTranslateRequest {
    WebTranslateRequest {
        text: vec![text.to_string()],
        target_lang: oneshot_language_code(target_code, true),
        source_lang: (source_code != "auto")
            .then(|| oneshot_language_code(source_code, false)),
        usage_type: "translate".to_string(),
        app_information: AppInformation {
            os: "iOS".to_string(),
            os_version: WEB_OS_VERSION.to_string(),
            app_version: app_version.to_string(),
            app_build: WEB_APP_BUILD.to_string(),
            instance_id: instance_id.to_string(),
        },
    }
}

fn deep_l_web_user_agent(app_version: &str) -> String {
    format!("DeepL/{app_version} {WEB_USER_AGENT_SUFFIX}")
}

/// `kDeepLWebInstanceID` — one UUID per process, like the Swift top-level `let`.
fn web_instance_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// `kDeepLWebSessionID`
fn web_session_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| uuid::Uuid::new_v4().to_string())
}

/// `oneshotLanguageCode(_:isTarget:)` — the oneshot endpoint speaks BCP-47-ish
/// tags, and a bare `en` *target* must be spelled `en-US`.
fn oneshot_language_code(code: &str, is_target: bool) -> String {
    match code.to_lowercase().as_str() {
        "zh-hans" => "zh-Hans".to_string(),
        "zh-hant" => "zh-Hant".to_string(),
        "pt-pt" => "pt-PT".to_string(),
        "pt-br" => "pt-BR".to_string(),
        "en" if is_target => "en-US".to_string(),
        _ => code.to_lowercase(),
    }
}

/// `removeLanguageVariant(_:)` — source codes carry no variant.
fn remove_language_variant(code: &str) -> String {
    code.split('-').next().unwrap_or(code).to_string()
}

/// `parseOfficialResponse(_:)` — the first translation, split into paragraphs.
/// `None` when the payload is missing, empty or whitespace-only.
fn parse_official_response(value: &Value) -> Option<Vec<String>> {
    let response: OfficialResponse = serde_json::from_value(value.clone()).ok()?;
    let text = response.translations?.first()?.text.clone()?;
    if text.trim().is_empty() {
        return None;
    }
    Some(util::to_paragraphs(&text))
}

/// Decodes a translate response with the two upstream failure messages.
fn decode_official_response(value: &Value) -> Result<Vec<String>, QueryError> {
    if !value.is_object() {
        return Err(QueryError::with_message(ErrorType::Api, "Invalid response"));
    }
    parse_official_response(value)
        .ok_or_else(|| QueryError::with_message(ErrorType::Api, "Failed to parse response"))
}

/// `parseDeepLErrorMessage(from:)` — DeepL's own message, when the body is one
/// of the several shapes it uses.
fn parse_deep_l_error_message(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let non_empty = |value: Option<&Value>| {
        value
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    non_empty(json.get("message"))
        .or_else(|| non_empty(json.get("title")))
        .or_else(|| non_empty(json.get("error")))
        .or_else(|| non_empty(json.get("error").and_then(|e| e.get("message"))))
}

/// The error both transports report for an HTTP failure.
fn web_error(status: u16, body: &[u8]) -> QueryError {
    match parse_deep_l_error_message(body) {
        Some(message) => QueryError::with_data(ErrorType::Api, None, message),
        None => QueryError::api_http(
            status,
            net::truncate(&String::from_utf8_lossy(body), 4096),
        ),
    }
}

/// `"\(host)/v2/translate"`, or the user's endpoint override when set.
fn official_url(auth_key: &str, endpoint: &str) -> String {
    let endpoint = endpoint.trim();
    if !endpoint.is_empty() {
        return endpoint.to_string();
    }
    // Free and Pro keys live on different hosts. The check is case-sensitive,
    // matching `authKey.hasSuffix(":fx")`.
    let host = if auth_key.ends_with(":fx") {
        "https://api-free.deepl.com"
    } else {
        "https://api.deepl.com"
    };
    format!("{host}/v2/translate")
}

// ---------------------------------------------------------------------------
// DeepLWebAppVersionProvider
// ---------------------------------------------------------------------------

/// The one shared provider, mirroring `DeepLWebAppVersionProvider.shared`.
static PROVIDER: OnceLock<DeepLWebAppVersionProvider> = OnceLock::new();

/// `DeepLWebAppVersionProvider` — resolves and caches the App Store version of
/// DeepL's iOS app, which the oneshot request's user agent and
/// `app_information` echo back.
///
/// Upstream is an `actor` that holds its lock across the lookup, so concurrent
/// callers coalesce onto a single request; the mutex below does the same.
pub struct DeepLWebAppVersionProvider {
    state: Mutex<CachedState>,
}

/// `CachedState`
///
/// The Swift state stores Foundation `Date`s as an encoded double; a value
/// imported from a macOS settings file therefore reads as very old here, which
/// only means one extra lookup.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedState {
    version: Option<String>,
    fetched_at: Option<f64>,
    last_attempt_at: Option<f64>,
}

impl DeepLWebAppVersionProvider {
    pub fn shared() -> &'static DeepLWebAppVersionProvider {
        PROVIDER.get_or_init(|| DeepLWebAppVersionProvider {
            state: Mutex::new(CachedState::default()),
        })
    }

    /// `currentVersion()` — cached while fresh, otherwise looked up; the
    /// fallback is returned whenever a lookup is skipped or fails.
    pub async fn current_version(&self) -> String {
        let mut state = self.state.lock().await;
        let now = now_seconds();

        if cache_is_usable(&state, now) {
            return valid_cached_version(&state)
                .unwrap_or_else(|| WEB_APP_VERSION_FALLBACK.to_string());
        }

        // Recorded before the request, like upstream: a failure still arms the
        // 15-minute backoff.
        state.last_attempt_at = Some(now);
        match Self::fetch_latest_version().await {
            Some(version) => {
                let fetched_at = now_seconds();
                state.version = Some(version.clone());
                state.fetched_at = Some(fetched_at);
                state.last_attempt_at = Some(fetched_at);
                version
            }
            None => valid_cached_version(&state)
                .unwrap_or_else(|| WEB_APP_VERSION_FALLBACK.to_string()),
        }
    }

    /// `loadState()` — seeds the in-memory state from
    /// `Settings::deep_l_web_app_version_cache`, once per process.
    pub async fn restore_persisted(&self, saved: &str) {
        if saved.trim().is_empty() {
            return;
        }
        let Ok(saved) = serde_json::from_str::<CachedState>(saved) else {
            return;
        };
        let mut state = self.state.lock().await;
        if state.version.is_none() {
            *state = saved;
        }
    }

    /// `DeepLWebAppVersionProvider.fetchLatestVersion()`
    async fn fetch_latest_version() -> Option<String> {
        let response = net::CLIENT
            .get(APP_STORE_LOOKUP_URL)
            // Unlike the translation paths, this check is optional and only
            // gates a refresh, so it wants a hard bound on the whole exchange
            // rather than an idle one.
            .timeout(WEB_APP_VERSION_LOOKUP_TIMEOUT)
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        let value: Value = response.json().await.ok()?;
        parse_app_store_version(&value)
    }
}

/// Seconds since the Unix epoch, the port's stand-in for `Date()`.
fn now_seconds() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

/// `isWithin(_:lifetime:now:)` — future-dated values are not "within".
fn is_within(date: f64, lifetime: f64, now: f64) -> bool {
    let age = now - date;
    age >= 0.0 && age < lifetime
}

fn valid_cached_version(state: &CachedState) -> Option<String> {
    state
        .version
        .as_ref()
        .filter(|version| is_valid_version(version))
        .cloned()
}

/// True when `currentVersion()` must not hit the network: the cache is fresh,
/// or the last attempt is still inside the retry delay.
fn cache_is_usable(state: &CachedState, now: f64) -> bool {
    if let Some(fetched_at) = state.fetched_at {
        if valid_cached_version(state).is_some()
            && is_within(fetched_at, WEB_APP_VERSION_CACHE_LIFETIME, now)
        {
            return true;
        }
    }
    state
        .last_attempt_at
        .is_some_and(|attempt| is_within(attempt, WEB_APP_VERSION_RETRY_DELAY, now))
}

/// `DeepLAppStoreVersionParser.version(from:)`
fn parse_app_store_version(value: &Value) -> Option<String> {
    if value.get("resultCount")?.as_i64()? != 1 {
        return None;
    }
    let results = value.get("results")?.as_array()?;
    if results.len() != 1 {
        return None;
    }
    let app = results.first()?;
    if app.get("trackId")?.as_i64()? != APP_STORE_TRACK_ID
        || app.get("bundleId")?.as_str()? != APP_STORE_BUNDLE_ID
        || app.get("artistName")?.as_str()? != APP_STORE_ARTIST_NAME
    {
        return None;
    }
    let version = app.get("version")?.as_str()?;
    is_valid_version(version).then(|| version.to_string())
}

/// `DeepLAppStoreVersionParser.isValidVersion(_:)` — two to four dot-separated
/// runs of ASCII digits.
fn is_valid_version(version: &str) -> bool {
    let components: Vec<&str> = version.split('.').collect();
    (2..=4).contains(&components.len())
        && components
            .iter()
            .all(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn service() -> DeepLService {
        DeepLService::new(ServiceContext::new(
            Arc::new(crate::config::Settings::default()),
            ServiceType::DeepL,
            "DeepL",
        ))
    }

    #[test]
    fn service_identity_matches_upstream() {
        let svc = service();
        assert_eq!(svc.service_type(), ServiceType::DeepL);
        assert_eq!(svc.identifier(), "DeepL");
        assert_eq!(
            svc.supported_query_type(),
            QueryTextType::TRANSLATION.union(QueryTextType::SENTENCE)
        );
        assert!(!svc.is_stream());
        assert!(svc.auto_convert_traditional_chinese());
        assert_eq!(svc.link().as_deref(), Some(TRANSLATE_URL));
    }

    #[test]
    fn official_url_follows_the_fx_suffix() {
        assert_eq!(
            official_url("abc:fx", ""),
            "https://api-free.deepl.com/v2/translate"
        );
        assert_eq!(
            official_url("abc", ""),
            "https://api.deepl.com/v2/translate"
        );
        // The suffix check is case-sensitive, as in Swift.
        assert_eq!(
            official_url("abc:FX", ""),
            "https://api.deepl.com/v2/translate"
        );
        // The endpoint override wins over the host selection.
        assert_eq!(
            official_url("abc:fx", "https://proxy.example/v2/translate"),
            "https://proxy.example/v2/translate"
        );
    }

    #[test]
    fn language_codes_and_variants() {
        let svc = service();
        assert_eq!(
            svc.language_code(Language::SimplifiedChinese).as_deref(),
            Some("zh-hans")
        );
        assert_eq!(
            svc.language_code(Language::BrazilianPortuguese).as_deref(),
            Some("pt-BR")
        );
        assert_eq!(svc.language_code(Language::Auto).as_deref(), Some("auto"));
        // DeepL has no Hindi in its table.
        assert_eq!(svc.language_code(Language::Hindi), None);

        assert_eq!(remove_language_variant("zh-hans"), "zh");
        assert_eq!(remove_language_variant("pt-BR"), "pt");
        assert_eq!(remove_language_variant("en"), "en");
    }

    #[test]
    fn oneshot_codes_are_bcp47_ish() {
        assert_eq!(oneshot_language_code("zh-hans", true), "zh-Hans");
        assert_eq!(oneshot_language_code("zh-hant", false), "zh-Hant");
        assert_eq!(oneshot_language_code("pt-PT", true), "pt-PT");
        assert_eq!(oneshot_language_code("pt-BR", false), "pt-BR");
        // Only a *target* English becomes en-US.
        assert_eq!(oneshot_language_code("en", true), "en-US");
        assert_eq!(oneshot_language_code("en", false), "en");
        assert_eq!(oneshot_language_code("JA", false), "ja");
    }

    #[test]
    fn web_request_body_shape() {
        let body = web_request_body("hello", "auto", "zh-hans", "26.52", "instance");
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["text"], serde_json::json!(["hello"]));
        assert_eq!(json["target_lang"], "zh-Hans");
        assert_eq!(json["usage_type"], "translate");
        // Auto-detection omits source_lang entirely.
        assert!(json.get("source_lang").is_none());
        assert_eq!(json["app_information"]["os"], "iOS");
        assert_eq!(json["app_information"]["os_version"], WEB_OS_VERSION);
        assert_eq!(json["app_information"]["app_version"], "26.52");
        assert_eq!(json["app_information"]["app_build"], WEB_APP_BUILD);
        assert_eq!(json["app_information"]["instance_id"], "instance");

        let body = web_request_body("hi", "en", "pt-BR", "26.52", "instance");
        let json = serde_json::to_value(&body).unwrap();
        assert_eq!(json["source_lang"], "en");
        assert_eq!(json["target_lang"], "pt-BR");
    }

    #[test]
    fn user_agent_and_ids() {
        assert_eq!(
            deep_l_web_user_agent("26.52"),
            "DeepL/26.52 CFNetwork/3826.600.41 Darwin/25.0.0"
        );

        let instance = web_instance_id();
        assert_eq!(instance, web_instance_id());
        assert_eq!(instance.len(), 36);
        assert!(instance
            .chars()
            .all(|c| c.is_ascii_hexdigit() || c == '-'));
        assert_eq!(instance, instance.to_lowercase());

        let session = web_session_id();
        assert_eq!(session, web_session_id());
        assert_eq!(session.len(), 36);
    }

    #[test]
    fn official_response_decoding() {
        let value: Value = serde_json::from_str(
            r#"{"translations":[{"detected_source_language":"EN","text":"你好\n世界"}]}"#,
        )
        .unwrap();
        assert_eq!(
            decode_official_response(&value).unwrap(),
            vec!["你好".to_string(), "世界".to_string()]
        );

        // Whitespace-only text counts as no result.
        let value: Value =
            serde_json::from_str(r#"{"translations":[{"text":"   "}]}"#).unwrap();
        let error = decode_official_response(&value).unwrap_err();
        assert_eq!(error.kind, ErrorType::Api);
        assert_eq!(error.message.as_deref(), Some("Failed to parse response"));

        // An empty list and a missing key are the same failure.
        for body in [r#"{"translations":[]}"#, r#"{}"#] {
            let value: Value = serde_json::from_str(body).unwrap();
            assert!(decode_official_response(&value).is_err());
        }

        // A non-object payload is reported differently.
        let value: Value = serde_json::from_str("[1,2]").unwrap();
        let error = decode_official_response(&value).unwrap_err();
        assert_eq!(error.message.as_deref(), Some("Invalid response"));
    }

    #[test]
    fn error_message_parsing() {
        assert_eq!(
            parse_deep_l_error_message(br#"{"message":"Quota exceeded"}"#).as_deref(),
            Some("Quota exceeded")
        );
        assert_eq!(
            parse_deep_l_error_message(br#"{"title":"Bad request"}"#).as_deref(),
            Some("Bad request")
        );
        assert_eq!(
            parse_deep_l_error_message(br#"{"error":"nope"}"#).as_deref(),
            Some("nope")
        );
        assert_eq!(
            parse_deep_l_error_message(br#"{"error":{"message":"nested"}}"#).as_deref(),
            Some("nested")
        );
        // Empty strings are skipped, and so is a non-JSON body.
        assert_eq!(parse_deep_l_error_message(br#"{"message":""}"#), None);
        assert_eq!(parse_deep_l_error_message(b"not json"), None);
    }

    #[test]
    fn version_validation() {
        assert!(is_valid_version("26.52"));
        assert!(is_valid_version("1.2.3"));
        assert!(is_valid_version("1.2.3.4"));
        assert!(!is_valid_version("26"));
        assert!(!is_valid_version("1.2.3.4.5"));
        assert!(!is_valid_version("26."));
        assert!(!is_valid_version("26..52"));
        assert!(!is_valid_version("26.a"));
        assert!(!is_valid_version(""));
    }

    #[test]
    fn app_store_lookup_parsing() {
        let body = r#"{"resultCount":1,"results":[{
            "trackId":1552407475,
            "bundleId":"com.linguee.DeepLMobileTranslator",
            "artistName":"DeepL SE",
            "version":"26.53.1"
        }]}"#;
        let value: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            parse_app_store_version(&value).as_deref(),
            Some("26.53.1")
        );

        // A missing track, a wrong bundle and a bad version are all rejected.
        let body = r#"{"resultCount":0,"results":[]}"#;
        let value: Value = serde_json::from_str(body).unwrap();
        assert_eq!(parse_app_store_version(&value), None);

        let body = r#"{"resultCount":1,"results":[{
            "trackId":1552407475,
            "bundleId":"com.example.other",
            "artistName":"DeepL SE",
            "version":"26.53.1"
        }]}"#;
        let value: Value = serde_json::from_str(body).unwrap();
        assert_eq!(parse_app_store_version(&value), None);

        let body = r#"{"resultCount":1,"results":[{
            "trackId":1552407475,
            "bundleId":"com.linguee.DeepLMobileTranslator",
            "artistName":"DeepL SE",
            "version":"26"
        }]}"#;
        let value: Value = serde_json::from_str(body).unwrap();
        assert_eq!(parse_app_store_version(&value), None);
    }

    #[test]
    fn cache_expiry_arithmetic() {
        assert_eq!(WEB_APP_VERSION_FALLBACK, "26.52");
        assert_eq!(WEB_APP_VERSION_CACHE_LIFETIME, 86_400.0);
        assert_eq!(WEB_APP_VERSION_RETRY_DELAY, 900.0);

        let now = 10_000_000.0_f64;
        let with_version = |fetched_at: f64, last_attempt_at: f64| CachedState {
            version: Some("26.53".to_string()),
            fetched_at: Some(fetched_at),
            last_attempt_at: Some(last_attempt_at),
        };

        // Fresh: one hour old, well inside the 24 h lifetime.
        let state = with_version(now - 3600.0, now - 3600.0);
        assert!(cache_is_usable(&state, now));
        assert_eq!(valid_cached_version(&state).as_deref(), Some("26.53"));

        // Exactly the lifetime is already expired (`age < lifetime` is strict).
        let state = with_version(
            now - WEB_APP_VERSION_CACHE_LIFETIME,
            now - WEB_APP_VERSION_CACHE_LIFETIME,
        );
        assert!(!cache_is_usable(&state, now));

        // A failed lookup leaves `last_attempt_at` behind: the 15-minute
        // backoff serves the stale cache instead of retrying.
        let state = with_version(now - 90_000.0, now - 60.0);
        assert!(cache_is_usable(&state, now));
        assert_eq!(valid_cached_version(&state).as_deref(), Some("26.53"));

        // Past the backoff, a lookup runs again.
        let state = with_version(now - 90_000.0, now - WEB_APP_VERSION_RETRY_DELAY);
        assert!(!cache_is_usable(&state, now));

        // No cache at all, but an attempt a minute ago: usable, and the caller
        // falls back to "26.52".
        let state = CachedState {
            version: None,
            fetched_at: None,
            last_attempt_at: Some(now - 60.0),
        };
        assert!(cache_is_usable(&state, now));
        assert_eq!(valid_cached_version(&state), None);

        // A clock that jumped backwards makes the stamp unusable.
        let state = with_version(now + 10.0, now + 10.0);
        assert!(!cache_is_usable(&state, now));
    }

    #[test]
    fn text_is_trimmed_to_the_deepl_limit() {
        let svc = service();
        assert_eq!(svc.max_text_length("  hello  "), "hello");
        let long = "a".repeat(TEXT_LIMIT + 100);
        assert_eq!(svc.max_text_length(&long).len(), TEXT_LIMIT);
    }

    #[test]
    fn word_link_shape() {
        let svc = service();
        let mut q = QueryModel::new("computer/server");
        q.query_from_language = Language::English;
        q.query_target_language = Language::SimplifiedChinese;
        assert_eq!(
            svc.word_link(&q).as_deref(),
            Some("https://www.deepl.com/translator#en/zh-hans/computer%5C%2Fserver")
        );

        // The source variant is dropped, the target variant is kept.
        let mut q = QueryModel::new("bom dia");
        q.query_from_language = Language::Portuguese;
        q.query_target_language = Language::English;
        assert_eq!(
            svc.word_link(&q).as_deref(),
            Some("https://www.deepl.com/translator#pt/en/bom%20dia")
        );

        // Unsupported languages have no link.
        let mut q = QueryModel::new("नमस्ते");
        q.query_from_language = Language::Hindi;
        q.query_target_language = Language::English;
        assert_eq!(svc.word_link(&q), None);
    }
}
