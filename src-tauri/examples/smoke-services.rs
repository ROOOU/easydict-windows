use std::time::{Duration, Instant};

use app_lib::{config::Settings, model::{Language, QueryModel}, query, service::registry};

#[tokio::main]
async fn main() {
    let settings = Settings::default();
    let mut model = QueryModel::new_with_settings("hello", &settings);
    model.query_from_language = Language::English;
    model.query_target_language = Language::SimplifiedChinese;
    model.query_type = app_lib::util::analysis::query_type("hello", Language::English, 2);

    let probes = registry::build_all(&settings).into_iter()
        .filter(|handle| ["Google", "Bing", "DeepL"].contains(&handle.identifier()))
        .map(|handle| {
            let model = model.clone();
            async move {
                let started = Instant::now();
                let outcome = tokio::time::timeout(Duration::from_secs(20), query::run_direct(&handle, &model)).await;
                let (ok, detail) = match outcome {
                    Ok(Ok(result)) if result.has_translated_result() => (
                        true,
                        result.translated_text().unwrap_or_else(|| "dictionary result".into()),
                    ),
                    Ok(Ok(result)) => (false, result.error.map(|error| error.to_string()).unwrap_or_else(|| "empty result".into())),
                    Ok(Err(error)) => (false, error.to_string()),
                    Err(_) => (false, "timed out after 20 seconds".into()),
                };
                println!("{}\t{}\t{} ms\t{}", handle.identifier(), if ok { "PASS" } else { "FAIL" }, started.elapsed().as_millis(), detail.replace(['\r', '\n'], " "));
                ok
            }
        });
    let outcomes = futures_util::future::join_all(probes).await;
    if outcomes.len() != 3 || outcomes.iter().any(|ok| !ok) {
        std::process::exit(1);
    }
}
