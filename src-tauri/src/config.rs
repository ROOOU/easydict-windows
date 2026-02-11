use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub hotkeys: HotkeyConfig,
    pub services: ServicesConfig,
    pub general: GeneralConfig,
    #[serde(default)]
    pub select_translate: SelectTranslateConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyEntry {
    pub enabled: bool,
    pub shortcut: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    pub input_translate: HotkeyEntry,
    pub select_translate: HotkeyEntry,
    pub screenshot_translate: HotkeyEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicesConfig {
    pub google: ServiceEntry,
    pub deepl: ServiceEntry,
    pub bing: ServiceEntry,
    pub baidu: BaiduServiceEntry,
    pub openai: OpenAIServiceEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceEntry {
    pub enabled: bool,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaiduServiceEntry {
    pub enabled: bool,
    pub app_id: String,
    pub secret_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAIServiceEntry {
    pub enabled: bool,
    pub api_key: String,
    pub api_url: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralConfig {
    pub theme: String,
    pub auto_start: bool,
    pub source_lang: String,
    pub target_lang: String,
    pub always_on_top: bool,
}

/// Select-to-translate configuration
/// mode: "auto" = translate immediately, "icon" = show floating icon, "hotkey" = hotkey only
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectTranslateConfig {
    pub enabled: bool,
    pub mode: String,
    pub monitor_clipboard: bool,
}

impl Default for SelectTranslateConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            mode: "icon".to_string(),
            monitor_clipboard: true,
        }
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            hotkeys: HotkeyConfig {
                input_translate: HotkeyEntry {
                    enabled: true,
                    shortcut: "Alt+A".to_string(),
                },
                select_translate: HotkeyEntry {
                    enabled: true,
                    shortcut: "Alt+D".to_string(),
                },
                screenshot_translate: HotkeyEntry {
                    enabled: true,
                    shortcut: "Alt+S".to_string(),
                },
            },
            services: ServicesConfig {
                google: ServiceEntry {
                    enabled: true,
                    api_key: String::new(),
                },
                deepl: ServiceEntry {
                    enabled: false,
                    api_key: String::new(),
                },
                bing: ServiceEntry {
                    enabled: true,
                    api_key: String::new(),
                },
                baidu: BaiduServiceEntry {
                    enabled: false,
                    app_id: String::new(),
                    secret_key: String::new(),
                },
                openai: OpenAIServiceEntry {
                    enabled: false,
                    api_key: String::new(),
                    api_url: "https://api.openai.com/v1/chat/completions".to_string(),
                    model: "gpt-4o-mini".to_string(),
                },
            },
            general: GeneralConfig {
                theme: "auto".to_string(),
                auto_start: false,
                source_lang: "auto".to_string(),
                target_lang: "zh-CN".to_string(),
                always_on_top: false,
            },
            select_translate: SelectTranslateConfig::default(),
        }
    }
}

fn config_path() -> PathBuf {
    let dir = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("EasyDictWin");
    fs::create_dir_all(&dir).ok();
    dir.join("config.json")
}

pub fn load_config() -> AppConfig {
    let path = config_path();
    if path.exists() {
        let data = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        let config = AppConfig::default();
        save_config(&config);
        config
    }
}

pub fn save_config(config: &AppConfig) {
    let path = config_path();
    if let Ok(data) = serde_json::to_string_pretty(config) {
        fs::write(&path, data).ok();
    }
}
