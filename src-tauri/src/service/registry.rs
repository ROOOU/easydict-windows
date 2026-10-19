//! The service registry — the Windows port's `QueryServiceFactory`.
//!
//! Upstream registers `(ServiceType, Class, titleKey, apiKeyRequirement,
//! allowsMultipleInstances)` tuples and instantiates on demand. The shape is
//! kept here, minus the Objective-C runtime: one `match` builds a boxed
//! [`QueryService`] from an identifier.
//!
//! Two upstream services have no Windows implementation and are therefore
//! absent from [`service_types`]:
//!
//! * `AppleDictionary` — wraps the private `DictionaryServices.framework`.
//! * `Apple` — wraps the macOS system-translate panel.
//!
//! They stay in [`ServiceType::all`] so a settings file round-trips, and
//! [`ServiceType::is_available_on_windows`] is the single place that decides.

use std::sync::Arc;

use serde_json::Value;

use crate::config::Settings;
use crate::model::{
    ApiKeyRequirement, Language, QueryError, QueryTextType, ServiceType,
};
use crate::net;
use crate::query::ServiceHandle;
use crate::service::{
    alibaba::AlibabaService, baidu::BaiduService, bing::BingService, caiyun::CaiyunService,
    deepl::DeepLService, doubao::DoubaoService, google::GoogleService, llm, niutrans::NiuTransService,
    tencent::TencentService, volcano::VolcanoService, youdao::YoudaoService, QueryService,
    ServiceContext,
};

/// Every service that has a working implementation here, in registration order.
pub fn service_types() -> Vec<ServiceType> {
    ServiceType::all()
        .iter()
        .copied()
        .filter(|t| t.is_available_on_windows() && is_implemented(*t))
        .collect()
}

/// True when [`build`] can produce this service.
///
/// The three agent-CLI services (`ClaudeCode`, `CodexCLI`, `GitHubCopilot`) and
/// `SystemSpeech` are declared in `ServiceType` for settings compatibility but
/// are not query services in this port; see `docs/reverse-engineering/03`.
pub fn is_implemented(service: ServiceType) -> bool {
    use ServiceType::*;
    matches!(
        service,
        Youdao
            | DeepL
            | Google
            | Baidu
            | Bing
            | Volcano
            | NiuTrans
            | Caiyun
            | Tencent
            | Alibaba
            | Doubao
            | OpenAI
            | CustomOpenAI
            | BuiltInAI
            | Gemini
            | Claude
            | DeepSeek
            | Groq
            | Zhipu
            | MiniMax
            | GitHub
            | Ollama
            | Polishing
            | Summary
    )
}

/// Builds one service, or `None` when the identifier names something this port
/// does not implement.
pub fn build(settings: &Settings, identifier: &str) -> Option<Arc<dyn QueryService>> {
    let (service_type, _uuid) = ServiceType::split_identifier(identifier);
    let service_type = service_type?;
    if !service_type.is_available_on_windows() || !is_implemented(service_type) {
        return None;
    }

    let ctx = ServiceContext::new(
        Arc::new(settings.clone()),
        service_type,
        identifier.to_string(),
    );

    Some(match service_type {
        ServiceType::Youdao => Arc::new(YoudaoService::new(ctx)),
        ServiceType::DeepL => Arc::new(DeepLService::new(ctx)),
        ServiceType::Google => Arc::new(GoogleService::new(ctx)),
        ServiceType::Baidu => Arc::new(BaiduService::new(ctx)),
        ServiceType::Bing => Arc::new(BingService::new(ctx)),
        ServiceType::Volcano => Arc::new(VolcanoService::new(ctx)),
        ServiceType::NiuTrans => Arc::new(NiuTransService::new(ctx)),
        ServiceType::Caiyun => Arc::new(CaiyunService::new(ctx)),
        ServiceType::Tencent => Arc::new(TencentService::new(ctx)),
        ServiceType::Alibaba => Arc::new(AlibabaService::new(ctx)),
        ServiceType::Doubao => Arc::new(DoubaoService::new(ctx)),
        _ => llm::build(ctx)?,
    })
}

/// One handle per configured service.
///
/// A `CustomOpenAI` with no uuid is skipped: upstream only materialises it once
/// the user creates an instance.
pub fn build_all(settings: &Settings) -> Vec<Arc<ServiceHandle>> {
    let mut identifiers: Vec<String> = service_types()
        .into_iter()
        .map(|t| t.raw().to_string())
        .collect();

    // Multi-instance services contribute one entry per configured uuid.
    for key in settings.services.keys() {
        let (Some(service_type), Some(uuid)) = ServiceType::split_identifier(key) else {
            continue;
        };
        if uuid.is_empty() || !service_type.allows_multiple_instances() {
            continue;
        }
        if !identifiers.contains(key) {
            identifiers.push(key.clone());
        }
    }

    identifiers
        .iter()
        .filter_map(|id| build(settings, id))
        .map(|service| Arc::new(ServiceHandle::new(service)))
        .collect()
}

/// Everything the settings window needs to render a service row.
pub fn descriptors(settings: &Settings) -> Vec<crate::commands::ServiceDescriptor> {
    let mut out = Vec::new();

    for service_type in ServiceType::all().iter().copied() {
        let available = service_type.is_available_on_windows() && is_implemented(service_type);
        let identifier = service_type.raw().to_string();
        let config = settings.service(&identifier);

        out.push(describe(
            settings,
            &identifier,
            service_type,
            config,
            available,
        ));

        if !service_type.allows_multiple_instances() {
            continue;
        }
        for key in settings.services.keys() {
            if key == &identifier {
                continue;
            }
            let (Some(ty), Some(uuid)) = ServiceType::split_identifier(key) else {
                continue;
            };
            if ty != service_type || uuid.is_empty() {
                continue;
            }
            let config = settings.service(key);
            out.push(describe(settings, key, ty, config, true));
        }
    }

    // Services the user is not supposed to see are dropped, but the two
    // macOS-only ones stay so the settings tab can explain why they are absent.
    out
}

fn describe(
    settings: &Settings,
    identifier: &str,
    service_type: ServiceType,
    config: crate::config::ServiceConfig,
    available: bool,
) -> crate::commands::ServiceDescriptor {
    let built = if available {
        build(settings, identifier)
    } else {
        None
    };

    let name = if !config.name.trim().is_empty() {
        config.name.clone()
    } else {
        service_type.default_name().to_string()
    };

    crate::commands::ServiceDescriptor {
        identifier: identifier.to_string(),
        service_type,
        name,
        name_key: service_type.name_key(),
        api_key_requirement: service_type.api_key_requirement(),
        is_stream: service_type.is_streaming_service(),
        link: built.as_ref().and_then(|s| s.link()),
        supported_query_type: built
            .as_ref()
            .map(|s| s.supported_query_type())
            .unwrap_or(QueryTextType::TRANSLATION),
        configurable_intelligent_query: service_type.configurable_intelligent_query(),
        requires_api_key: service_type.api_key_requirement().needs_user_provided_key(),
        // "Has the credential this service actually needs", not "is the generic
        // slot filled". Ten keyed services read flat settings fields
        // (`baidu_app_id`, `tencent_secret_id`, …) and never touch
        // `config.api_key`, so the raw `config.api_key` test lit the "no key"
        // badge in the service list no matter what the user entered. Services
        // that are unavailable have no instance to ask, so they fall back to
        // the slot they would be built from.
        has_api_key: match built.as_ref() {
            Some(service) => service.has_private_api_key(),
            None => !config.api_key.trim().is_empty(),
        },
        available,
        allows_multiple_instances: service_type.allows_multiple_instances(),
        config,
    }
}

// ---------------------------------------------------------------------------
// Remote model lists — `BaseOpenAIService.fetchRemoteModelIDs`
// ---------------------------------------------------------------------------

/// Fetches the provider's model list.
///
/// Upstream derives the URL from the configured endpoint, dropping a trailing
/// `chat/completions`, and three providers override it. The same three are
/// special-cased here; their request shapes share nothing with the generic
/// OpenAI-compatible list call.
pub async fn fetch_remote_models(service: &dyn QueryService) -> Result<Vec<String>, QueryError> {
    let ctx = service.ctx();
    let service_type = service.service_type();
    let key = ctx.config.api_key.trim().to_string();

    match service_type {
        ServiceType::Ollama => fetch_ollama_models().await,
        ServiceType::Claude => {
            if key.is_empty() {
                return Err(QueryError::with_message(
                    crate::model::ErrorType::MissingSecretKey,
                    "Claude API key is empty.",
                ));
            }
            let url = format!("{}models?limit=1000", base_url(&ctx.config.endpoint));
            let response = net::CLIENT
                .get(&url)
                .header("x-api-key", key)
                .header("anthropic-version", "2023-06-01")
                .header("Accept", "application/json")
                .send()
                .await
                .map_err(net::map_reqwest_error)?;
            let value: Value = net::json(response).await?;
            Ok(string_list(&value, "data", "id"))
        }
        ServiceType::GitHub => {
            if key.is_empty() {
                return Err(QueryError::with_message(
                    crate::model::ErrorType::MissingSecretKey,
                    "GitHub Models token is empty",
                ));
            }
            let response = net::CLIENT
                .get("https://models.github.ai/catalog/models")
                .header("Authorization", format!("Bearer {key}"))
                .header("Accept", "application/vnd.github+json")
                .send()
                .await
                .map_err(net::map_reqwest_error)?;
            // The catalogue is a bare array whose entries carry the org in the
            // id; only text-in/text-out models are offered.
            let value: Value = net::json(response).await?;
            let mut out: Vec<String> = value
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter(|item| modalities_include_text(item))
                        .filter_map(|item| item.get("id").and_then(Value::as_str))
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            dedupe_lowercase(&mut out);
            Ok(out)
        }
        _ => {
            if key.is_empty() && service_type.api_key_requirement() != ApiKeyRequirement::None {
                return Err(QueryError::with_message(
                    crate::model::ErrorType::MissingSecretKey,
                    "API key is empty",
                ));
            }
            let url = models_url(&ctx.config.endpoint)?;
            let mut request = net::CLIENT
                .get(&url)
                .header("Accept", "application/json");
            // No per-request deadline: `net::CLIENT` already applies an idle
            // timeout of `MODEL_LIST_TIMEOUT_SECS`, and a total one layered on
            // top would only cut off a listing that is still arriving.
            if !key.is_empty() {
                request = request
                    .header("Authorization", format!("Bearer {key}"))
                    .header("api-key", key);
            }
            let response = request.send().await.map_err(net::map_reqwest_error)?;
            let value: Value = net::json(response).await?;
            let mut out = string_list(&value, "data", "id");
            if service_type == ServiceType::Gemini {
                // Gemini's OpenAI-compat list prefixes ids with `models/`.
                for id in out.iter_mut() {
                    if let Some(stripped) = id.strip_prefix("models/") {
                        *id = stripped.to_string();
                    }
                }
                dedupe_lowercase(&mut out);
            }
            Ok(out)
        }
    }
}

async fn fetch_ollama_models() -> Result<Vec<String>, QueryError> {
    // As above, `net::CLIENT`'s idle timeout already bounds this.
    let response = net::CLIENT
        .get("http://localhost:11434/api/tags")
        .send()
        .await
        .map_err(net::map_reqwest_error)?;
    let value: Value = net::json(response).await?;
    // The local endpoint reports `name`; the OpenAI-compatible one reports `id`.
    Ok(value
        .get("models")
        .and_then(Value::as_array)
        .map(|models| {
            models
                .iter()
                .filter_map(|m| {
                    m.get("name")
                        .or_else(|| m.get("model"))
                        .and_then(Value::as_str)
                })
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

fn modalities_include_text(item: &Value) -> bool {
    let accepts = |field: &str| match item.get(field).and_then(Value::as_array) {
        // A missing list means "unrestricted" upstream.
        None => true,
        Some(list) => list.iter().any(|m| m.as_str() == Some("text")),
    };
    accepts("supported_input_modalities") && accepts("supported_output_modalities")
}

fn string_list(value: &Value, array_field: &str, item_field: &str) -> Vec<String> {
    value
        .get(array_field)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get(item_field).and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn dedupe_lowercase(items: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    items.retain(|id| {
        let key = id.trim().to_ascii_lowercase();
        !key.is_empty() && seen.insert(key)
    });
}

/// `https://host/path/…` -> `https://host/` — the origin plus a trailing slash.
fn base_url(endpoint: &str) -> String {
    match endpoint.split_once("://") {
        Some((scheme, rest)) => match rest.split_once('/') {
            Some((host, _)) => format!("{scheme}://{host}/"),
            None => format!("{scheme}://{rest}/"),
        },
        None => endpoint.to_string(),
    }
}

/// `remoteModelsURL()` — derive the model-list URL from the chat endpoint.
pub fn models_url(endpoint: &str) -> Result<String, QueryError> {
    let base = base_url(endpoint);
    if base == endpoint {
        return Err(QueryError::with_message(
            crate::model::ErrorType::Parameter,
            "Endpoint is invalid",
        ));
    }
    let path = endpoint
        .strip_prefix(&base)
        .unwrap_or_default()
        .trim_end_matches('/');
    let mut segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    match segments.last().copied() {
        Some("completions") => {
            segments.pop();
            // `chat/completions` keeps its `v1` sibling; a bare `completions`
            // sits directly under the version segment.
            if segments.last().copied() == Some("chat") {
                segments.pop();
            }
        }
        Some("models") => {
            segments.pop();
        }
        _ => {}
    }

    if segments.is_empty() {
        segments.push("v1");
    }
    Ok(format!("{base}{}/models", segments.join("/")))
}

/// The languages a service accepts, minus `Auto`, for the settings pickers.
pub fn supported_languages(service: &dyn QueryService) -> Vec<Language> {
    service
        .supported_languages()
        .into_iter()
        .filter(|l| *l != Language::Auto)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macos_only_services_are_never_built() {
        for service_type in [ServiceType::AppleDictionary, ServiceType::Apple] {
            assert!(!service_types().contains(&service_type));
            assert!(build(&Settings::default(), service_type.raw()).is_none());
        }
    }

    #[test]
    fn every_registered_type_builds() {
        let settings = Settings::default();
        for service_type in service_types() {
            let identifier = service_type.raw();
            let service = build(&settings, identifier)
                .unwrap_or_else(|| panic!("{identifier} did not build"));
            assert_eq!(service.identifier(), identifier);
            assert_eq!(service.service_type(), service_type);
        }
    }

    #[test]
    fn build_all_covers_every_registered_type() {
        let settings = Settings::default();
        let handles = build_all(&settings);
        let built: Vec<&str> = handles.iter().map(|h| h.identifier()).collect();
        for service_type in service_types() {
            assert!(
                built.contains(&service_type.raw()),
                "{} missing from build_all",
                service_type.raw()
            );
        }
    }

    #[test]
    fn custom_openai_instances_get_their_own_handle() {
        let mut settings = Settings::default();
        let mut config = crate::config::ServiceConfig::default();
        config.name = "My Endpoint".into();
        config.endpoint = "https://example.test/v1/chat/completions".into();
        settings
            .services
            .insert("CustomOpenAI#abc123".into(), config);

        let handles = build_all(&settings);
        let ids: Vec<&str> = handles.iter().map(|h| h.identifier()).collect();
        assert!(ids.contains(&"CustomOpenAI#abc123"));
    }

    #[test]
    fn descriptors_hide_nothing_but_flag_availability() {
        let settings = Settings::default();
        let all = descriptors(&settings);
        assert_eq!(all.len(), ServiceType::all().len());
        let apple = all
            .iter()
            .find(|d| d.service_type == ServiceType::Apple)
            .unwrap();
        assert!(!apple.available);
        let youdao = all
            .iter()
            .find(|d| d.service_type == ServiceType::Youdao)
            .unwrap();
        assert!(youdao.available);
        assert_eq!(youdao.api_key_requirement, ApiKeyRequirement::UserProvided);
        assert!(youdao.requires_api_key);
        assert!(!youdao.has_api_key);
    }

    #[test]
    fn the_key_badge_follows_the_credential_the_service_reads() {
        // The service list's "no key" badge has to clear when the user fills
        // the flat field the service actually reads, and stay lit for a value
        // in the generic slot — which Baidu, Alibaba, Volcano and Doubao never
        // look at, so a key there would be silently ignored.
        let mut settings = Settings::default();
        let badge = |s: &Settings| {
            descriptors(s)
                .into_iter()
                .find(|d| d.service_type == ServiceType::Baidu)
                .unwrap()
                .has_api_key
        };

        assert!(!badge(&settings), "a fresh install has no Baidu credentials");

        settings
            .services
            .get_mut(ServiceType::Baidu.raw())
            .unwrap()
            .api_key = "generic".into();
        assert!(!badge(&settings));

        settings.baidu_app_id = "appid".into();
        assert!(!badge(&settings), "half a pair is not a pair");

        settings.baidu_secret_key = "secret".into();
        assert!(badge(&settings));
    }

    #[test]
    fn models_url_strips_the_completion_path() {
        assert_eq!(
            models_url("https://api.openai.com/v1/chat/completions").unwrap(),
            "https://api.openai.com/v1/models"
        );
        assert_eq!(
            models_url("https://api.deepseek.com/v1/chat/completions").unwrap(),
            "https://api.deepseek.com/v1/models"
        );
        // No `/v1` in the endpoint: one is supplied.
        assert_eq!(
            models_url("https://example.test/chat/completions").unwrap(),
            "https://example.test/v1/models"
        );
        // A bare version path.
        assert_eq!(
            models_url("https://example.test/v1/completions").unwrap(),
            "https://example.test/v1/models"
        );
        // Already a models URL.
        assert_eq!(
            models_url("https://example.test/v1/models").unwrap(),
            "https://example.test/v1/models"
        );
        // Nothing to derive from.
        assert!(models_url("not a url").is_err());
    }

    #[test]
    fn base_url_keeps_the_origin() {
        assert_eq!(base_url("https://a.test/b/c"), "https://a.test/");
        assert_eq!(base_url("http://localhost:11434/v1/x"), "http://localhost:11434/");
        assert_eq!(base_url("https://a.test"), "https://a.test/");
    }

    #[test]
    fn github_modalities_filter() {
        // Absent modality lists are treated as unrestricted.
        let open = serde_json::json!({ "id": "a" });
        assert!(modalities_include_text(&open));
        // An explicit list must mention text.
        let text = serde_json::json!({
            "id": "b",
            "supported_input_modalities": ["text"],
            "supported_output_modalities": ["text"],
        });
        assert!(modalities_include_text(&text));
        let image_only = serde_json::json!({
            "id": "c",
            "supported_input_modalities": ["image"],
            "supported_output_modalities": ["image"],
        });
        assert!(!modalities_include_text(&image_only));
    }

    #[test]
    fn dedupe_is_case_insensitive_and_drops_blanks() {
        let mut ids = vec![
            "GPT-4.1".to_string(),
            "gpt-4.1".to_string(),
            "  ".to_string(),
            "o3".to_string(),
        ];
        dedupe_lowercase(&mut ids);
        assert_eq!(ids, vec!["GPT-4.1", "o3"]);
    }
}
