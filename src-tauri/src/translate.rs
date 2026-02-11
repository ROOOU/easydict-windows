use serde::{Deserialize, Serialize};
use reqwest::Client;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateResult {
    pub service: String,
    pub translated: String,
    pub source_lang: String,
    pub target_lang: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangPair {
    pub code: String,
    pub name: String,
    pub name_en: String,
}

pub fn supported_languages() -> Vec<LangPair> {
    vec![
        LangPair { code: "auto".into(), name: "自动检测".into(), name_en: "Auto Detect".into() },
        LangPair { code: "zh-CN".into(), name: "简体中文".into(), name_en: "Chinese (Simplified)".into() },
        LangPair { code: "zh-TW".into(), name: "繁體中文".into(), name_en: "Chinese (Traditional)".into() },
        LangPair { code: "en".into(), name: "英语".into(), name_en: "English".into() },
        LangPair { code: "ja".into(), name: "日语".into(), name_en: "Japanese".into() },
        LangPair { code: "ko".into(), name: "韩语".into(), name_en: "Korean".into() },
        LangPair { code: "fr".into(), name: "法语".into(), name_en: "French".into() },
        LangPair { code: "de".into(), name: "德语".into(), name_en: "German".into() },
        LangPair { code: "es".into(), name: "西班牙语".into(), name_en: "Spanish".into() },
        LangPair { code: "pt".into(), name: "葡萄牙语".into(), name_en: "Portuguese".into() },
        LangPair { code: "ru".into(), name: "俄语".into(), name_en: "Russian".into() },
        LangPair { code: "ar".into(), name: "阿拉伯语".into(), name_en: "Arabic".into() },
        LangPair { code: "th".into(), name: "泰语".into(), name_en: "Thai".into() },
        LangPair { code: "vi".into(), name: "越南语".into(), name_en: "Vietnamese".into() },
        LangPair { code: "it".into(), name: "意大利语".into(), name_en: "Italian".into() },
        LangPair { code: "nl".into(), name: "荷兰语".into(), name_en: "Dutch".into() },
        LangPair { code: "pl".into(), name: "波兰语".into(), name_en: "Polish".into() },
        LangPair { code: "uk".into(), name: "乌克兰语".into(), name_en: "Ukrainian".into() },
        LangPair { code: "id".into(), name: "印度尼西亚语".into(), name_en: "Indonesian".into() },
        LangPair { code: "ms".into(), name: "马来语".into(), name_en: "Malay".into() },
        LangPair { code: "hi".into(), name: "印地语".into(), name_en: "Hindi".into() },
        LangPair { code: "tr".into(), name: "土耳其语".into(), name_en: "Turkish".into() },
    ]
}

/// Simple language detection heuristic (fallback when API doesn't provide detection)
pub fn detect_language(text: &str) -> String {
    let text = text.trim();
    let mut cn_count = 0u32;
    let mut ja_count = 0u32;
    let mut ko_count = 0u32;
    let mut latin_count = 0u32;
    let mut cyrillic_count = 0u32;
    let mut arabic_count = 0u32;
    let mut thai_count = 0u32;

    for c in text.chars() {
        match c {
            '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' => cn_count += 1,
            '\u{3040}'..='\u{309f}' | '\u{30a0}'..='\u{30ff}' | '\u{31f0}'..='\u{31ff}' => {
                ja_count += 1
            }
            '\u{ac00}'..='\u{d7af}' | '\u{1100}'..='\u{11ff}' => ko_count += 1,
            '\u{0400}'..='\u{04ff}' => cyrillic_count += 1,
            '\u{0600}'..='\u{06ff}' => arabic_count += 1,
            '\u{0e00}'..='\u{0e7f}' => thai_count += 1,
            'a'..='z' | 'A'..='Z' => latin_count += 1,
            _ => {}
        }
    }

    let total = cn_count + ja_count + ko_count + latin_count + cyrillic_count + arabic_count + thai_count;
    if total == 0 {
        return "en".to_string();
    }

    if ja_count > 0 && (ja_count as f64 / total as f64) > 0.1 {
        "ja".to_string()
    } else if ko_count > 0 && (ko_count as f64 / total as f64) > 0.15 {
        "ko".to_string()
    } else if cn_count > 0 && (cn_count as f64 / total as f64) > 0.2 {
        "zh-CN".to_string()
    } else if cyrillic_count > latin_count {
        "ru".to_string()
    } else if arabic_count > latin_count {
        "ar".to_string()
    } else if thai_count > latin_count {
        "th".to_string()
    } else {
        "en".to_string()
    }
}

/// Determine best target language based on detected source
pub fn auto_target_lang(source_lang: &str, default_target: &str) -> String {
    if source_lang.starts_with("zh") {
        "en".to_string()
    } else if source_lang == "en" && default_target.starts_with("zh") {
        default_target.to_string()
    } else {
        default_target.to_string()
    }
}

// ==================== Google Translate (Free) ====================

pub async fn google_translate(
    client: &Client,
    text: &str,
    source: &str,
    target: &str,
) -> TranslateResult {
    let sl = if source == "auto" { "auto" } else { source };
    let url = format!(
        "https://translate.googleapis.com/translate_a/single?client=gtx&sl={}&tl={}&dt=t&q={}",
        sl,
        target,
        urlencoding::encode(text)
    );

    match client.get(&url).send().await {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => {
                let mut translated = String::new();
                let detected = json[2].as_str().unwrap_or(sl).to_string();
                if let Some(sentences) = json[0].as_array() {
                    for sentence in sentences {
                        if let Some(t) = sentence[0].as_str() {
                            translated.push_str(t);
                        }
                    }
                }
                TranslateResult {
                    service: "Google".to_string(),
                    translated,
                    source_lang: detected,
                    target_lang: target.to_string(),
                    error: None,
                }
            }
            Err(e) => TranslateResult {
                service: "Google".to_string(),
                translated: String::new(),
                source_lang: source.to_string(),
                target_lang: target.to_string(),
                error: Some(format!("Parse error: {}", e)),
            },
        },
        Err(e) => TranslateResult {
            service: "Google".to_string(),
            translated: String::new(),
            source_lang: source.to_string(),
            target_lang: target.to_string(),
            error: Some(format!("Network error: {}", e)),
        },
    }
}

// ==================== Bing Translate (Free) ====================

pub async fn bing_translate(
    client: &Client,
    text: &str,
    source: &str,
    target: &str,
) -> TranslateResult {
    let bing_target = match target {
        "zh-CN" => "zh-Hans",
        "zh-TW" => "zh-Hant",
        other => other,
    };
    let bing_source = match source {
        "auto" => "",
        "zh-CN" => "zh-Hans",
        "zh-TW" => "zh-Hant",
        other => other,
    };

    let url = if bing_source.is_empty() {
        format!(
            "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&to={}",
            bing_target
        )
    } else {
        format!(
            "https://api.cognitive.microsofttranslator.com/translate?api-version=3.0&from={}&to={}",
            bing_source, bing_target
        )
    };

    let body = serde_json::json!([{"Text": text}]);

    match client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => {
                let translated = json[0]["translations"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let detected = json[0]["detectedLanguage"]["language"]
                    .as_str()
                    .unwrap_or(source)
                    .to_string();
                TranslateResult {
                    service: "Bing".to_string(),
                    translated,
                    source_lang: detected,
                    target_lang: target.to_string(),
                    error: None,
                }
            }
            Err(e) => TranslateResult {
                service: "Bing".to_string(),
                translated: String::new(),
                source_lang: source.to_string(),
                target_lang: target.to_string(),
                error: Some(format!("Parse error: {}", e)),
            },
        },
        Err(e) => TranslateResult {
            service: "Bing".to_string(),
            translated: String::new(),
            source_lang: source.to_string(),
            target_lang: target.to_string(),
            error: Some(format!("Network error: {}", e)),
        },
    }
}

// ==================== DeepL Translate ====================

pub async fn deepl_translate(
    client: &Client,
    text: &str,
    source: &str,
    target: &str,
    api_key: &str,
) -> TranslateResult {
    if api_key.is_empty() {
        return TranslateResult {
            service: "DeepL".to_string(),
            translated: String::new(),
            source_lang: source.to_string(),
            target_lang: target.to_string(),
            error: Some("API key not configured".to_string()),
        };
    }

    let deepl_target = match target {
        "zh-CN" => "ZH",
        "zh-TW" => "ZH",
        "en" => "EN",
        other => other,
    };

    let base_url = if api_key.ends_with(":fx") {
        "https://api-free.deepl.com/v2/translate"
    } else {
        "https://api.deepl.com/v2/translate"
    };

    let mut params: HashMap<&str, &str> = HashMap::new();
    params.insert("text", text);
    params.insert("target_lang", deepl_target);
    if source != "auto" {
        let deepl_source = match source {
            "zh-CN" | "zh-TW" => "ZH",
            "en" => "EN",
            other => other,
        };
        params.insert("source_lang", deepl_source);
    }

    match client
        .post(base_url)
        .header("Authorization", format!("DeepL-Auth-Key {}", api_key))
        .form(&params)
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => {
                let translated = json["translations"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let detected = json["translations"][0]["detected_source_language"]
                    .as_str()
                    .unwrap_or(source)
                    .to_lowercase();
                TranslateResult {
                    service: "DeepL".to_string(),
                    translated,
                    source_lang: detected,
                    target_lang: target.to_string(),
                    error: None,
                }
            }
            Err(e) => TranslateResult {
                service: "DeepL".to_string(),
                translated: String::new(),
                source_lang: source.to_string(),
                target_lang: target.to_string(),
                error: Some(format!("Parse error: {}", e)),
            },
        },
        Err(e) => TranslateResult {
            service: "DeepL".to_string(),
            translated: String::new(),
            source_lang: source.to_string(),
            target_lang: target.to_string(),
            error: Some(format!("Network error: {}", e)),
        },
    }
}

// ==================== Baidu Translate ====================

pub async fn baidu_translate(
    client: &Client,
    text: &str,
    source: &str,
    target: &str,
    app_id: &str,
    secret_key: &str,
) -> TranslateResult {
    if app_id.is_empty() || secret_key.is_empty() {
        return TranslateResult {
            service: "Baidu".to_string(),
            translated: String::new(),
            source_lang: source.to_string(),
            target_lang: target.to_string(),
            error: Some("API credentials not configured".to_string()),
        };
    }

    let baidu_source = match source {
        "auto" => "auto",
        "zh-CN" | "zh-TW" => "zh",
        "en" => "en",
        "ja" => "jp",
        "ko" => "kor",
        "fr" => "fra",
        "de" => "de",
        "es" => "spa",
        "ru" => "ru",
        other => other,
    };

    let baidu_target = match target {
        "zh-CN" | "zh-TW" => "zh",
        "en" => "en",
        "ja" => "jp",
        "ko" => "kor",
        "fr" => "fra",
        "de" => "de",
        "es" => "spa",
        "ru" => "ru",
        other => other,
    };

    let salt: u32 = rand::random();
    let sign_str = format!("{}{}{}{}", app_id, text, salt, secret_key);
    let sign = format!("{:x}", md5::compute(sign_str.as_bytes()));

    let params = [
        ("q", text),
        ("from", baidu_source),
        ("to", baidu_target),
        ("appid", app_id),
        ("salt", &salt.to_string()),
        ("sign", &sign),
    ];

    match client
        .post("https://fanyi-api.baidu.com/api/trans/vip/translate")
        .form(&params)
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => {
                if let Some(err_code) = json.get("error_code") {
                    return TranslateResult {
                        service: "Baidu".to_string(),
                        translated: String::new(),
                        source_lang: source.to_string(),
                        target_lang: target.to_string(),
                        error: Some(format!(
                            "Error {}: {}",
                            err_code,
                            json["error_msg"].as_str().unwrap_or("Unknown")
                        )),
                    };
                }
                let mut translated = String::new();
                if let Some(results) = json["trans_result"].as_array() {
                    for r in results {
                        if let Some(dst) = r["dst"].as_str() {
                            if !translated.is_empty() {
                                translated.push('\n');
                            }
                            translated.push_str(dst);
                        }
                    }
                }
                let detected = json["from"].as_str().unwrap_or(source).to_string();
                TranslateResult {
                    service: "Baidu".to_string(),
                    translated,
                    source_lang: detected,
                    target_lang: target.to_string(),
                    error: None,
                }
            }
            Err(e) => TranslateResult {
                service: "Baidu".to_string(),
                translated: String::new(),
                source_lang: source.to_string(),
                target_lang: target.to_string(),
                error: Some(format!("Parse error: {}", e)),
            },
        },
        Err(e) => TranslateResult {
            service: "Baidu".to_string(),
            translated: String::new(),
            source_lang: source.to_string(),
            target_lang: target.to_string(),
            error: Some(format!("Network error: {}", e)),
        },
    }
}

// ==================== OpenAI / Custom LLM Translate ====================

pub async fn openai_translate(
    client: &Client,
    text: &str,
    source: &str,
    target: &str,
    api_key: &str,
    api_url: &str,
    model: &str,
) -> TranslateResult {
    if api_key.is_empty() {
        return TranslateResult {
            service: "AI".to_string(),
            translated: String::new(),
            source_lang: source.to_string(),
            target_lang: target.to_string(),
            error: Some("API key not configured".to_string()),
        };
    }

    fn lang_name(code: &str) -> &str {
        match code {
            "zh-CN" => "Simplified Chinese",
            "zh-TW" => "Traditional Chinese",
            "en" => "English",
            "ja" => "Japanese",
            "ko" => "Korean",
            "fr" => "French",
            "de" => "German",
            "es" => "Spanish",
            "ru" => "Russian",
            _ => code,
        }
    }

    let source_desc = if source == "auto" {
        "auto-detected language".to_string()
    } else {
        lang_name(source).to_string()
    };
    let target_desc = lang_name(target);

    let system_prompt = format!(
        "You are a professional translator. Translate the following text from {} to {}. \
         Only output the translation, no explanations or extra text.",
        source_desc, target_desc
    );

    let body = serde_json::json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": text}
        ],
        "temperature": 0.3,
        "max_tokens": 4096
    });

    match client
        .post(api_url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => {
                let translated = json["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                TranslateResult {
                    service: format!("AI ({})", model),
                    translated,
                    source_lang: source.to_string(),
                    target_lang: target.to_string(),
                    error: None,
                }
            }
            Err(e) => TranslateResult {
                service: "AI".to_string(),
                translated: String::new(),
                source_lang: source.to_string(),
                target_lang: target.to_string(),
                error: Some(format!("Parse error: {}", e)),
            },
        },
        Err(e) => TranslateResult {
            service: "AI".to_string(),
            translated: String::new(),
            source_lang: source.to_string(),
            target_lang: target.to_string(),
            error: Some(format!("Network error: {}", e)),
        },
    }
}
