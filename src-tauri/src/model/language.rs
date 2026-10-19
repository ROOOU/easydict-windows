//! Language model, ported from `Easydict/objc/Service/Language/EZLanguageModel.m`.
//!
//! The raw string values are kept byte-identical to the macOS app so that
//! persisted settings, prompt interpolation and service code tables stay
//! compatible with the original.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Every language Easydict knows about.
///
/// `#[serde(rename = "...")]` values match `EZLanguageModel.m` exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "auto")]
    Auto,
    #[serde(rename = "Simplified-Chinese")]
    SimplifiedChinese,
    #[serde(rename = "Traditional-Chinese")]
    TraditionalChinese,
    #[serde(rename = "Classical-Chinese")]
    ClassicalChinese,
    #[serde(rename = "English")]
    English,
    #[serde(rename = "Japanese")]
    Japanese,
    #[serde(rename = "Korean")]
    Korean,
    #[serde(rename = "French")]
    French,
    #[serde(rename = "Spanish")]
    Spanish,
    #[serde(rename = "Catalan")]
    Catalan,
    #[serde(rename = "Portuguese")]
    Portuguese,
    #[serde(rename = "Brazilian-Portuguese")]
    BrazilianPortuguese,
    #[serde(rename = "Italian")]
    Italian,
    #[serde(rename = "German")]
    German,
    #[serde(rename = "Russian")]
    Russian,
    #[serde(rename = "Arabic")]
    Arabic,
    #[serde(rename = "Swedish")]
    Swedish,
    #[serde(rename = "Romanian")]
    Romanian,
    #[serde(rename = "Thai")]
    Thai,
    #[serde(rename = "Slovak")]
    Slovak,
    #[serde(rename = "Dutch")]
    Dutch,
    #[serde(rename = "Hungarian")]
    Hungarian,
    #[serde(rename = "Greek")]
    Greek,
    #[serde(rename = "Danish")]
    Danish,
    #[serde(rename = "Finnish")]
    Finnish,
    #[serde(rename = "Polish")]
    Polish,
    #[serde(rename = "Czech")]
    Czech,
    #[serde(rename = "Turkish")]
    Turkish,
    #[serde(rename = "Lithuanian")]
    Lithuanian,
    #[serde(rename = "Latvian")]
    Latvian,
    #[serde(rename = "Ukrainian")]
    Ukrainian,
    #[serde(rename = "Bulgarian")]
    Bulgarian,
    #[serde(rename = "Indonesian")]
    Indonesian,
    #[serde(rename = "Malay")]
    Malay,
    #[serde(rename = "Slovenian")]
    Slovenian,
    #[serde(rename = "Estonian")]
    Estonian,
    #[serde(rename = "Vietnamese")]
    Vietnamese,
    #[serde(rename = "Persian")]
    Persian,
    #[serde(rename = "Hindi")]
    Hindi,
    #[serde(rename = "Telugu")]
    Telugu,
    #[serde(rename = "Tamil")]
    Tamil,
    #[serde(rename = "Urdu")]
    Urdu,
    #[serde(rename = "Filipino")]
    Filipino,
    #[serde(rename = "Khmer")]
    Khmer,
    #[serde(rename = "Lao")]
    Lao,
    #[serde(rename = "Bengali")]
    Bengali,
    #[serde(rename = "Burmese")]
    Burmese,
    #[serde(rename = "Norwegian")]
    Norwegian,
    #[serde(rename = "Serbian")]
    Serbian,
    #[serde(rename = "Croatian")]
    Croatian,
    #[serde(rename = "Mongolian")]
    Mongolian,
    #[serde(rename = "Hebrew")]
    Hebrew,
    #[serde(rename = "Georgian")]
    Georgian,
    #[serde(rename = "Uyghur")]
    Uyghur,
    #[serde(rename = "unsupported")]
    Unsupported,
}

impl Default for Language {
    /// `EZLanguageModel` treats a missing value as "auto detect".
    fn default() -> Self {
        Language::Auto
    }
}

impl Language {
    /// The `EZLanguage*` constant value, used as the serialized form.
    pub const fn raw(self) -> &'static str {
        use Language::*;
        match self {
            Auto => "auto",
            SimplifiedChinese => "Simplified-Chinese",
            TraditionalChinese => "Traditional-Chinese",
            ClassicalChinese => "Classical-Chinese",
            English => "English",
            Japanese => "Japanese",
            Korean => "Korean",
            French => "French",
            Spanish => "Spanish",
            Catalan => "Catalan",
            Portuguese => "Portuguese",
            BrazilianPortuguese => "Brazilian-Portuguese",
            Italian => "Italian",
            German => "German",
            Russian => "Russian",
            Arabic => "Arabic",
            Swedish => "Swedish",
            Romanian => "Romanian",
            Thai => "Thai",
            Slovak => "Slovak",
            Dutch => "Dutch",
            Hungarian => "Hungarian",
            Greek => "Greek",
            Danish => "Danish",
            Finnish => "Finnish",
            Polish => "Polish",
            Czech => "Czech",
            Turkish => "Turkish",
            Lithuanian => "Lithuanian",
            Latvian => "Latvian",
            Ukrainian => "Ukrainian",
            Bulgarian => "Bulgarian",
            Indonesian => "Indonesian",
            Malay => "Malay",
            Slovenian => "Slovenian",
            Estonian => "Estonian",
            Vietnamese => "Vietnamese",
            Persian => "Persian",
            Hindi => "Hindi",
            Telugu => "Telugu",
            Tamil => "Tamil",
            Urdu => "Urdu",
            Filipino => "Filipino",
            Khmer => "Khmer",
            Lao => "Lao",
            Bengali => "Bengali",
            Burmese => "Burmese",
            Norwegian => "Norwegian",
            Serbian => "Serbian",
            Croatian => "Croatian",
            Mongolian => "Mongolian",
            Hebrew => "Hebrew",
            Georgian => "Georgian",
            Uyghur => "Uyghur",
            Unsupported => "unsupported",
        }
    }

    /// BCP-47 code, from `EZLanguageModel.m`'s `languageCode` field.
    pub const fn code(self) -> &'static str {
        use Language::*;
        match self {
            Auto => "und",
            SimplifiedChinese => "zh-Hans",
            TraditionalChinese => "zh-Hant",
            ClassicalChinese => "lzh",
            English => "en",
            Japanese => "ja",
            Korean => "ko",
            French => "fr",
            Spanish => "es",
            Catalan => "ca",
            Portuguese => "pt",
            BrazilianPortuguese => "pt-BR",
            Italian => "it",
            German => "de",
            Russian => "ru",
            Arabic => "ar",
            Swedish => "sv",
            Romanian => "ro",
            Thai => "th",
            Slovak => "sk",
            Dutch => "nl",
            Hungarian => "hu",
            Greek => "el",
            Danish => "da",
            Finnish => "fi",
            Polish => "pl",
            Czech => "cs",
            Turkish => "tr",
            Lithuanian => "lt",
            Latvian => "lv",
            Ukrainian => "uk",
            Bulgarian => "bg",
            Indonesian => "id",
            Malay => "ms",
            Slovenian => "sl",
            Estonian => "et",
            Vietnamese => "vi",
            Persian => "fa",
            Hindi => "hi",
            Telugu => "te",
            Tamil => "ta",
            Urdu => "ur",
            Filipino => "fil",
            Khmer => "km",
            Lao => "lo",
            Bengali => "bn",
            Burmese => "my",
            Norwegian => "nb",
            Serbian => "sr-Cyrl",
            Croatian => "hr",
            Mongolian => "mn-Mong",
            Hebrew => "he",
            Georgian => "ka",
            Uyghur => "ug",
            Unsupported => "",
        }
    }

    /// English display name, `EZLanguageModel.m`'s `englishName`.
    pub const fn english_name(self) -> &'static str {
        use Language::*;
        match self {
            Auto => "auto",
            SimplifiedChinese => "Simplified-Chinese",
            TraditionalChinese => "Traditional-Chinese",
            ClassicalChinese => "Classical-Chinese",
            English => "English",
            Japanese => "Japanese",
            Korean => "Korean",
            French => "French",
            Spanish => "Spanish",
            Catalan => "Catalan",
            Portuguese => "Portuguese",
            BrazilianPortuguese => "Brazilian-Portuguese",
            Italian => "Italian",
            German => "German",
            Russian => "Russian",
            Arabic => "Arabic",
            Swedish => "Swedish",
            Romanian => "Romanian",
            Thai => "Thai",
            Slovak => "Slovak",
            Dutch => "Dutch",
            Hungarian => "Hungarian",
            Greek => "Greek",
            Danish => "Danish",
            Finnish => "Finnish",
            Polish => "Polish",
            Czech => "Czech",
            Turkish => "Turkish",
            Lithuanian => "Lithuanian",
            Latvian => "Latvian",
            Ukrainian => "Ukrainian",
            Bulgarian => "Bulgarian",
            Indonesian => "Indonesian",
            Malay => "Malay",
            Slovenian => "Slovenian",
            Estonian => "Estonian",
            Vietnamese => "Vietnamese",
            Persian => "Persian",
            Hindi => "Hindi",
            Telugu => "Telugu",
            Tamil => "Tamil",
            Urdu => "Urdu",
            Filipino => "Filipino",
            Khmer => "Khmer",
            Lao => "Lao",
            Bengali => "Bengali",
            Burmese => "Burmese",
            Norwegian => "Norwegian",
            Serbian => "Serbian",
            Croatian => "Croatian",
            Mongolian => "Mongolian",
            Hebrew => "Hebrew",
            Georgian => "Georgian",
            Uyghur => "Uyghur",
            Unsupported => "unsupported",
        }
    }

    /// Chinese display name, `EZLanguageModel.m`'s `chineseName`.
    pub const fn chinese_name(self) -> &'static str {
        use Language::*;
        match self {
            Auto => "自动检测",
            SimplifiedChinese => "简体中文",
            TraditionalChinese => "繁体中文",
            ClassicalChinese => "文言文",
            English => "英语",
            Japanese => "日语",
            Korean => "韩语",
            French => "法语",
            Spanish => "西班牙语",
            Catalan => "加泰罗尼亚语",
            Portuguese => "葡萄牙语",
            BrazilianPortuguese => "葡萄牙语（巴西）",
            Italian => "意大利语",
            German => "德语",
            Russian => "俄语",
            Arabic => "阿拉伯语",
            Swedish => "瑞典语",
            Romanian => "罗马尼亚语",
            Thai => "泰语",
            Slovak => "斯洛伐克语",
            Dutch => "荷兰语",
            Hungarian => "匈牙利语",
            Greek => "希腊语",
            Danish => "丹麦语",
            Finnish => "芬兰语",
            Polish => "波兰语",
            Czech => "捷克语",
            Turkish => "土耳其语",
            Lithuanian => "立陶宛语",
            Latvian => "拉脱维亚语",
            Ukrainian => "乌克兰语",
            Bulgarian => "保加利亚语",
            Indonesian => "印尼语",
            Malay => "马来语",
            Slovenian => "斯洛文尼亚语",
            Estonian => "爱沙尼亚语",
            Vietnamese => "越南语",
            Persian => "波斯语",
            Hindi => "印地语",
            Telugu => "泰卢固语",
            Tamil => "泰米尔语",
            Urdu => "乌尔都语",
            Filipino => "菲律宾语",
            Khmer => "高棉语",
            Lao => "老挝语",
            Bengali => "孟加拉语",
            Burmese => "缅甸语",
            Norwegian => "挪威语",
            Serbian => "塞尔维亚语",
            Croatian => "克罗地亚语",
            Mongolian => "蒙古语",
            Hebrew => "希伯来语",
            Georgian => "格鲁吉亚语",
            Uyghur => "维吾尔语",
            Unsupported => "unsupported",
        }
    }

    /// Native display name, `EZLanguageModel.m`'s `nativeName`.
    pub const fn native_name(self) -> &'static str {
        use Language::*;
        match self {
            Auto => "auto",
            SimplifiedChinese => "简体中文",
            TraditionalChinese => "繁體中文",
            ClassicalChinese => "文言文",
            English => "English",
            Japanese => "日本語",
            Korean => "한국어",
            French => "Français",
            Spanish => "Español",
            Catalan => "Català",
            Portuguese => "Português",
            BrazilianPortuguese => "Português (Brasil)",
            Italian => "Italiano",
            German => "Deutsch",
            Russian => "Русский",
            Arabic => "العربية",
            Swedish => "Svenska",
            Romanian => "Română",
            Thai => "ไทย",
            Slovak => "Slovenčina",
            Dutch => "Nederlands",
            Hungarian => "Magyar",
            Greek => "Ελληνικά",
            Danish => "Dansk",
            Finnish => "Suomi",
            Polish => "Polski",
            Czech => "Čeština",
            Turkish => "Türkçe",
            Lithuanian => "Lietuvių",
            Latvian => "Latviešu",
            Ukrainian => "Українська",
            Bulgarian => "Български",
            Indonesian => "Bahasa Indonesia",
            Malay => "Bahasa Melayu",
            Slovenian => "Slovenščina",
            Estonian => "Eesti",
            Vietnamese => "Tiếng Việt",
            Persian => "فارسی",
            Hindi => "हिन्दी",
            Telugu => "తెలుగు",
            Tamil => "தமிழ்",
            Urdu => "اردو",
            Filipino => "Filipino",
            Khmer => "ភាសាខ្មែរ",
            Lao => "ພາສາລາວ",
            Bengali => "বাংলা",
            Burmese => "ဗမာစာ",
            Norwegian => "Norsk",
            Serbian => "Српски",
            Croatian => "Hrvatski",
            Mongolian => "Монгол",
            Hebrew => "עברית",
            Georgian => "ქართული",
            Uyghur => "ئۇيغۇرچە",
            Unsupported => "unsupported",
        }
    }

    /// Flag emoji, `EZLanguageModel.m`'s `flagEmoji`.
    pub const fn flag(self) -> &'static str {
        use Language::*;
        match self {
            Auto => "🌐",
            SimplifiedChinese => "🇨🇳",
            TraditionalChinese => "🇭🇰",
            ClassicalChinese => "📜",
            English => "🇬🇧",
            Japanese => "🇯🇵",
            Korean => "🇰🇷",
            French => "🇫🇷",
            Spanish | Catalan => "🇪🇸",
            Portuguese => "🇵🇹",
            BrazilianPortuguese => "🇧🇷",
            Italian => "🇮🇹",
            German => "🇩🇪",
            Russian => "🇷🇺",
            Arabic => "🇸🇦",
            Swedish => "🇸🇪",
            Romanian => "🇷🇴",
            Thai => "🇹🇭",
            Slovak => "🇸🇰",
            Dutch => "🇳🇱",
            Hungarian => "🇭🇺",
            Greek => "🇬🇷",
            Danish => "🇩🇰",
            Finnish => "🇫🇮",
            Polish => "🇵🇱",
            Czech => "🇨🇿",
            Turkish => "🇹🇷",
            Lithuanian => "🇱🇹",
            Latvian => "🇱🇻",
            Ukrainian => "🇺🇦",
            Bulgarian => "🇧🇬",
            Indonesian => "🇮🇩",
            Malay => "🇲🇾",
            Slovenian => "🇸🇮",
            Estonian => "🇪🇪",
            Vietnamese => "🇻🇳",
            Persian => "🇮🇷",
            Hindi | Telugu | Tamil | Urdu => "🇮🇳",
            Filipino => "🇵🇭",
            Khmer => "🇰🇭",
            Lao => "🇱🇦",
            Bengali => "🇧🇩",
            Burmese => "🇲🇲",
            Norwegian => "🇳🇴",
            Serbian => "🇷🇸",
            Croatian => "🇭🇷",
            Mongolian => "🇲🇳",
            Hebrew => "🇮🇱",
            Georgian => "🇬🇪",
            Uyghur => "🇨🇳",
            Unsupported => "",
        }
    }

    /// `voiceLocaleIdentifier` — `xx_YY` form used for system TTS voices.
    pub const fn voice_locale(self) -> &'static str {
        use Language::*;
        match self {
            Auto | Unsupported => "",
            SimplifiedChinese | ClassicalChinese => "zh_CN",
            TraditionalChinese => "zh_TW",
            English => "en_US",
            Japanese => "ja_JP",
            Korean => "ko_KR",
            French => "fr_FR",
            Spanish => "es_ES",
            Catalan => "ca_ES",
            Portuguese => "pt_PT",
            BrazilianPortuguese => "pt_BR",
            Italian => "it_IT",
            German => "de_DE",
            Russian => "ru_RU",
            Arabic => "ar_AE",
            Swedish => "sv_SE",
            Romanian => "ro_RO",
            Thai => "th_TH",
            Slovak => "sk_SK",
            Dutch => "nl_NL",
            Hungarian => "hu_HU",
            Greek => "el_GR",
            Danish => "da_DK",
            Finnish => "fi_FI",
            Polish => "pl_PL",
            Czech => "cs_CZ",
            Turkish => "tr_TR",
            Lithuanian => "lt_LT",
            Latvian => "lv_LV",
            Ukrainian => "uk_UA",
            Bulgarian => "bg_BG",
            Indonesian => "id_ID",
            Malay => "ms_MY",
            Slovenian => "sl_SI",
            Estonian => "et_EE",
            Vietnamese => "vi_VN",
            Persian => "fa_IR",
            Hindi => "hi_IN",
            Telugu => "te_IN",
            Tamil => "ta_IN",
            Urdu => "ur_PK",
            Filipino => "fil_PH",
            Khmer => "km_KH",
            Lao => "lo_LA",
            Bengali => "bn_BD",
            Burmese => "my_MM",
            Norwegian => "nb_NO",
            Serbian => "sr_Cyrl",
            Croatian => "hr_HR",
            Mongolian => "mn_MN",
            Hebrew => "he_IL",
            Georgian => "ka_GE",
            Uyghur => "ug_CN",
        }
    }

    /// Windows BCP-47 tag suitable for `Windows.Media.SpeechSynthesis` and
    /// `Windows.Media.Ocr`. Derived from `voice_locale` (`_` -> `-`), with a
    /// few macOS-specific identifiers normalised to Windows equivalents.
    pub fn windows_locale(self) -> &'static str {
        match self {
            Language::Auto | Language::Unsupported => "en-US",
            Language::Serbian => "sr-Latn-RS",
            Language::Filipino => "fil-PH",
            Language::Mongolian => "mn-MN",
            Language::Uyghur => "ug-CN",
            other => match other.voice_locale() {
                "zh_CN" => "zh-CN",
                "zh_TW" => "zh-TW",
                "en_US" => "en-US",
                "ja_JP" => "ja-JP",
                "ko_KR" => "ko-KR",
                "fr_FR" => "fr-FR",
                "es_ES" => "es-ES",
                "ca_ES" => "ca-ES",
                "pt_PT" => "pt-PT",
                "pt_BR" => "pt-BR",
                "it_IT" => "it-IT",
                "de_DE" => "de-DE",
                "ru_RU" => "ru-RU",
                "ar_AE" => "ar-SA",
                "sv_SE" => "sv-SE",
                "ro_RO" => "ro-RO",
                "th_TH" => "th-TH",
                "sk_SK" => "sk-SK",
                "nl_NL" => "nl-NL",
                "hu_HU" => "hu-HU",
                "el_GR" => "el-GR",
                "da_DK" => "da-DK",
                "fi_FI" => "fi-FI",
                "pl_PL" => "pl-PL",
                "cs_CZ" => "cs-CZ",
                "tr_TR" => "tr-TR",
                "lt_LT" => "lt-LT",
                "lv_LV" => "lv-LV",
                "uk_UA" => "uk-UA",
                "bg_BG" => "bg-BG",
                "id_ID" => "id-ID",
                "ms_MY" => "ms-MY",
                "sl_SI" => "sl-SI",
                "et_EE" => "et-EE",
                "vi_VN" => "vi-VN",
                "fa_IR" => "fa-IR",
                "hi_IN" => "hi-IN",
                "te_IN" => "te-IN",
                "ta_IN" => "ta-IN",
                "ur_PK" => "ur-PK",
                "km_KH" => "km-KH",
                "lo_LA" => "lo-LA",
                "bn_BD" => "bn-BD",
                "my_MM" => "my-MM",
                "nb_NO" => "nb-NO",
                "hr_HR" => "hr-HR",
                "he_IL" => "he-IL",
                "ka_GE" => "ka-GE",
                "ug_CN" => "ug-CN",
                "sr_Cyrl" => "sr-Cyrl-RS",
                _ => "en-US",
            },
        }
    }

    /// Name used when interpolating into LLM prompts.
    ///
    /// Ported from `StreamService+Prompt.swift`'s `queryLanguageName`: the three
    /// Chinese variants get descriptive Chinese names, everything else uses the
    /// English name.
    pub const fn query_language_name(self) -> &'static str {
        match self {
            Language::ClassicalChinese => "简体中文文言文",
            Language::SimplifiedChinese => "简体中文白话文",
            Language::TraditionalChinese => "繁体中文白话文",
            other => other.english_name(),
        }
    }

    /// Name shown in the UI — `EZLanguageManager.showingLanguageName(_:)`.
    ///
    /// The macOS app resolves this against the app's localization. The current
    /// UI language is stored in a process-wide global set at startup from
    /// settings; before it is set, English is used.
    pub fn display_name(self) -> &'static str {
        if app_language_is_chinese() {
            self.chinese_name()
        } else {
            self.english_name()
        }
    }

    pub fn from_raw(s: &str) -> Option<Language> {
        Language::all().iter().copied().find(|l| l.raw() == s)
    }

    /// Resolve a BCP-47-ish code to a Language, returning `Auto` when unknown —
    /// mirrors `languageEnum(fromCode:)`.
    pub fn from_code(code: &str) -> Language {
        if code.is_empty() {
            return Language::Auto;
        }
        let lower = code.to_ascii_lowercase();
        // Exact BCP-47 match first.
        if let Some(l) = Language::all()
            .iter()
            .find(|l| l.code().eq_ignore_ascii_case(code))
        {
            return *l;
        }
        // Chinese script / region variants.
        if lower.starts_with("zh") {
            if lower.contains("hant") || lower.ends_with("-tw") || lower.ends_with("-hk") || lower.ends_with("-mo") {
                return Language::TraditionalChinese;
            }
            if lower.contains("hans") || lower.ends_with("-cn") || lower.ends_with("-sg") {
                return Language::SimplifiedChinese;
            }
            return Language::SimplifiedChinese;
        }
        // Primary subtag match, with regional variants disambiguated.
        let primary = lower.split(['-', '_']).next().unwrap_or("");
        let region = lower.split(['-', '_']).nth(1).unwrap_or("");
        match primary {
            "pt" => {
                if region.eq_ignore_ascii_case("br") {
                    Language::BrazilianPortuguese
                } else {
                    Language::Portuguese
                }
            }
            "nb" | "nn" | "no" => Language::Norwegian,
            "sr" => Language::Serbian,
            "he" | "iw" => Language::Hebrew,
            "fil" | "tl" => Language::Filipino,
            "lzh" => Language::ClassicalChinese,
            _ => Language::all()
                .iter()
                .find(|l| {
                    let c = l.code();
                    c.split('-').next().is_some_and(|p| p.eq_ignore_ascii_case(primary))
                })
                .copied()
                .unwrap_or(Language::Auto),
        }
    }

    pub const fn is_auto(self) -> bool {
        matches!(self, Language::Auto)
    }

    pub const fn is_chinese(self) -> bool {
        matches!(
            self,
            Language::SimplifiedChinese | Language::TraditionalChinese | Language::ClassicalChinese
        )
    }

    pub const fn is_simplified_chinese(self) -> bool {
        matches!(self, Language::SimplifiedChinese)
    }

    pub const fn is_traditional_chinese(self) -> bool {
        matches!(self, Language::TraditionalChinese)
    }

    pub const fn is_english(self) -> bool {
        matches!(self, Language::English)
    }

    /// All languages in `EZLanguageModel.m` order, including `Auto` and
    /// `Unsupported` — this is the iteration order used by every language picker.
    pub const fn all() -> &'static [Language] {
        use Language::*;
        &[
            Auto,
            SimplifiedChinese,
            TraditionalChinese,
            ClassicalChinese,
            English,
            Japanese,
            Korean,
            French,
            Spanish,
            Catalan,
            Portuguese,
            BrazilianPortuguese,
            Italian,
            German,
            Russian,
            Arabic,
            Swedish,
            Romanian,
            Thai,
            Slovak,
            Dutch,
            Hungarian,
            Greek,
            Danish,
            Finnish,
            Polish,
            Czech,
            Turkish,
            Lithuanian,
            Latvian,
            Ukrainian,
            Bulgarian,
            Indonesian,
            Malay,
            Slovenian,
            Estonian,
            Vietnamese,
            Persian,
            Hindi,
            Telugu,
            Tamil,
            Urdu,
            Filipino,
            Khmer,
            Lao,
            Bengali,
            Burmese,
            Norwegian,
            Serbian,
            Croatian,
            Mongolian,
            Hebrew,
            Georgian,
            Uyghur,
            Unsupported,
        ]
    }

    /// Languages that do not use spaces between words, from
    /// `EZLanguageManager.languagesWithoutWordSpaces`.
    pub const fn words_need_space(self) -> bool {
        !matches!(
            self,
            Language::SimplifiedChinese
                | Language::TraditionalChinese
                | Language::ClassicalChinese
                | Language::Japanese
                | Language::Korean
                | Language::Thai
                | Language::Lao
                | Language::Khmer
                | Language::Burmese
        )
    }
}

impl fmt::Display for Language {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.raw())
    }
}

/// Serializable descriptor for the frontend language picker.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LanguageInfo {
    pub value: Language,
    pub code: &'static str,
    pub english_name: &'static str,
    pub chinese_name: &'static str,
    pub native_name: &'static str,
    pub flag: &'static str,
}

impl LanguageInfo {
    pub fn of(l: Language) -> Self {
        LanguageInfo {
            value: l,
            code: l.code(),
            english_name: l.english_name(),
            chinese_name: l.chinese_name(),
            native_name: l.native_name(),
            flag: l.flag(),
        }
    }
}

/// All selectable languages (everything except `Unsupported`).
pub fn selectable_languages() -> Vec<LanguageInfo> {
    Language::all()
        .iter()
        .copied()
        .filter(|l| !matches!(l, Language::Unsupported))
        .map(LanguageInfo::of)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_round_trip() {
        for l in Language::all() {
            if matches!(l, Language::Unsupported) {
                continue;
            }
            assert_eq!(Language::from_raw(l.raw()), Some(*l));
            assert_eq!(Language::from_code(l.code()), *l, "code {}", l.code());
        }
    }

    #[test]
    fn chinese_variants() {
        assert_eq!(Language::from_code("zh-Hans"), Language::SimplifiedChinese);
        assert_eq!(Language::from_code("zh-Hant"), Language::TraditionalChinese);
        assert_eq!(Language::from_code("zh-CN"), Language::SimplifiedChinese);
        assert_eq!(Language::from_code("zh-TW"), Language::TraditionalChinese);
        assert_eq!(Language::from_code("zh"), Language::SimplifiedChinese);
        assert_eq!(Language::from_code("lzh"), Language::ClassicalChinese);
    }

    #[test]
    fn unknown_code_is_auto() {
        assert_eq!(Language::from_code("xx-YY"), Language::Auto);
    }

    #[test]
    fn prompt_names() {
        assert_eq!(Language::SimplifiedChinese.query_language_name(), "简体中文白话文");
        assert_eq!(Language::English.query_language_name(), "English");
    }
}

/// The UI language, mirroring `appLanguage` in the macOS configuration.
///
/// `false` = English, `true` = Chinese. Set once from settings at startup.
static APP_LANGUAGE_IS_CHINESE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn set_app_language_is_chinese(is_chinese: bool) {
    APP_LANGUAGE_IS_CHINESE.store(is_chinese, std::sync::atomic::Ordering::Relaxed);
}

pub fn app_language_is_chinese() -> bool {
    APP_LANGUAGE_IS_CHINESE.load(std::sync::atomic::Ordering::Relaxed)
}
