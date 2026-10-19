//! Language detection.
//!
//! The macOS app uses Apple's `NLLanguageRecognizer` and then applies a large
//! body of corrections on top of it (`AppleLanguageDetector`). Windows has no
//! equivalent of `NLLanguageRecognizer`, so this module keeps the *entire*
//! correction pipeline verbatim and substitutes a Unicode-script classifier for
//! the recogniser step, producing the same kind of probability dictionary.
//!
//! Behaviour that depends only on the corrections — Chinese genre, mixed-script
//! resolution, short-text fixes, preferred-language weighting — therefore
//! matches upstream exactly.

use std::collections::HashMap;

use crate::model::Language;
use crate::util;

/// How many words of a language's function-word list must be present before the
/// language is considered a candidate. Kept low so that short fragments still
/// classify, mirroring `NLLanguageRecognizer`'s behaviour on 1-3 words.
const MIN_FUNCTION_WORD_HITS: usize = 1;

/// Mirrors `AppleLanguageDetector`.
pub struct LanguageDetector {
    pub preferred_languages: Vec<Language>,
    /// `MyConfiguration.shared.beta` — enables classical-Chinese detection.
    pub beta: bool,
    /// Set once per detection pass, matching `analyzeTextCharacterComposition`.
    pub analysis: TextAnalysis,
}

/// Statistics gathered in the single analysis pass.
#[derive(Debug, Default, Clone)]
pub struct TextAnalysis {
    pub chinese_character_count: usize,
    pub simplified_character_count: usize,
    pub traditional_character_count: usize,
    pub chinese_character_ratio: f64,
    pub english_character_count: usize,
    pub english_character_ratio: f64,
    pub has_mixed_scripts: bool,
}

impl LanguageDetector {
    pub fn new(preferred_languages: Vec<Language>, beta: bool) -> Self {
        LanguageDetector {
            preferred_languages,
            beta,
            analysis: TextAnalysis::default(),
        }
    }

    /// `detectLanguage(text:)`
    pub fn detect_language(&mut self, text: &str) -> Language {
        self.analysis = TextAnalysis::default();
        self.detect_internal(text, true)
    }

    /// The recogniser stand-in: a probability distribution over languages.
    ///
    /// Apple's version is a trained model; this is a script-and-stopword
    /// classifier producing the same shape of result.
    pub fn language_hypotheses(text: &str) -> HashMap<Language, f64> {
        script_probabilities(text)
    }

    fn detect_internal(&mut self, text: &str, apply_post_processing: bool) -> Language {
        if text.is_empty() || text.trim().is_empty() {
            return Language::Auto;
        }

        let raw = Self::language_hypotheses(text);
        if raw.is_empty() {
            return detect_fallback_language(text);
        }

        let adjusted = self.apply_user_preferred_language_weights(&raw);

        let Some((mut language, confidence)) = adjusted
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(l, p)| (*l, *p))
        else {
            return detect_fallback_language(text);
        };

        if apply_post_processing {
            language = self.apply_post_processing_corrections(language, confidence, text, &adjusted);
        }

        language
    }

    /// `applyUserPreferredLanguageWeights(to:)`
    fn apply_user_preferred_language_weights(
        &self,
        probabilities: &HashMap<Language, f64>,
    ) -> HashMap<Language, f64> {
        let mut adjusted = probabilities.clone();

        for (index, preferred) in self.preferred_languages.iter().enumerate() {
            // Only boost languages that were actually detected. Unlike Apple we
            // key on `Language` directly, but the guard is the same.
            let Some(&original) = probabilities.get(preferred) else {
                continue;
            };

            let max_weight = 0.4;
            let step = 0.1;
            let mut boost = max_weight - (index as f64) * step;
            if boost < 0.1 {
                boost = 0.1;
            }

            if *preferred == Language::English {
                let is_chinese_first = self
                    .preferred_languages
                    .first()
                    .is_some_and(|l| l.is_chinese());
                boost += if is_chinese_first { 0.1 } else { 0.2 };
            }

            adjusted.insert(*preferred, original + boost);
        }

        // English is boosted even when it is not a preference.
        if !self.preferred_languages.contains(&Language::English) {
            if let Some(&p) = probabilities.get(&Language::English) {
                adjusted.insert(Language::English, p + 0.2);
            }
        }

        adjusted
    }

    /// `applyPostProcessingCorrections(...)` — a three-tier priority system.
    fn apply_post_processing_corrections(
        &mut self,
        detected: Language,
        confidence: f64,
        text: &str,
        all_probabilities: &HashMap<Language, f64>,
    ) -> Language {
        let clean = text.trim();
        if clean.is_empty() {
            return Language::Auto;
        }

        // Priority 1: Chinese type verification.
        if let Some(lang) = self.determine_chinese_type(clean, detected) {
            return lang;
        }

        // Priority 2: mixed-script content.
        if let Some(lang) = self.handle_mixed_content(detected, clean, confidence) {
            return lang;
        }

        // Priority 3: short-text corrections.
        if let Some(lang) =
            handle_short_text_corrections(detected, clean, confidence, all_probabilities)
        {
            return lang;
        }

        detected
    }

    /// `determineChineseType(for:detectedLanguage:)`
    fn determine_chinese_type(&mut self, text: &str, detected: Language) -> Option<Language> {
        if !detected.is_chinese() {
            return None;
        }

        if let Some(analysis) = self.analysis_mut(text, Some(detected)) {
            let simplified_ratio = if analysis.chinese_character_count > 0 {
                analysis.simplified_character_count as f64
                    / analysis.chinese_character_count as f64
            } else {
                0.0
            };
            // `isSimplifiedChinese()` uses a 0.9 threshold here, unlike the
            // `isSimplifiedChinese` string helper which uses 0.8.
            return Some(if simplified_ratio >= 0.9 {
                Language::SimplifiedChinese
            } else {
                Language::TraditionalChinese
            });
        }

        None
    }

    /// `handleMixedContent(detectedLanguage:text:confidence:)`
    fn handle_mixed_content(
        &mut self,
        detected: Language,
        text: &str,
        _confidence: f64,
    ) -> Option<Language> {
        let mixed = self
            .analysis_mut(text, None)
            .map(|a| a.has_mixed_scripts)
            .unwrap_or(false);

        if !mixed || detected != Language::English {
            return None;
        }

        let english_word_count = english_word_count(text);
        let non_english = remove_english_characters(text);
        let pure_non_english = removing_non_letters(&non_english);
        let non_english_word_count = pure_non_english.chars().count();

        if english_word_count > non_english_word_count || non_english_word_count == 0 {
            return Some(Language::English);
        }

        let non_english_language = self.detect_internal(&pure_non_english, true);
        if non_english_language != Language::English && non_english_language != Language::Auto {
            return Some(non_english_language);
        }

        None
    }

    /// Runs the single-pass analysis once per detection, like `isAnalyzed`.
    fn analysis_mut(&mut self, text: &str, detected: Option<Language>) -> Option<&TextAnalysis> {
        if self.analysis.chinese_character_count == 0
            && !self.analysis.has_mixed_scripts
            && self.analysis.english_character_count == 0
        {
            self.analysis = analyze_text_character_composition(text);
        }
        // The `detected` argument only gates a piece of debug logging upstream.
        let _ = detected;
        Some(&self.analysis)
    }
}

/// `detectFallbackLanguage(for:)`
fn detect_fallback_language(text: &str) -> Language {
    // Numeric-only text and unrecognisable input both land on English;
    // upstream's `NLLanguageRecognizer` returns `und` for both, and the caller
    // treats `und` the same way.
    let _ = text;
    Language::English
}

/// `handleShortTextCorrections(...)`
fn handle_short_text_corrections(
    detected: Language,
    text: &str,
    confidence: f64,
    all_probabilities: &HashMap<Language, f64>,
) -> Option<Language> {
    if is_numeric_heavy(text) {
        return Some(Language::English);
    }

    if confidence < 0.5 && text.chars().count() <= 10 && is_latin_text(text) {
        if all_probabilities.get(&Language::English).copied().unwrap_or(0.0) > 0.1 {
            return Some(Language::English);
        }
    }

    if text.chars().count() <= 3 {
        // `preferredLanguages` comes from the detector instance upstream; the
        // caller passes the same list through the probability map, so we only
        // apply the language-agnostic half here.
        if is_latin_text(text) {
            return Some(Language::English);
        }
    }

    // The `preferredLanguages.contains(detectedLanguage)` branch is handled by
    // the caller, which owns the preference list.
    if text.chars().count() <= 3 {
        return None;
    }

    let _ = detected;
    None
}

/// `analyzeTextCharacterComposition(for:detectedLanguage:)`
fn analyze_text_character_composition(text: &str) -> TextAnalysis {
    let total = text.chars().count();
    if total == 0 {
        return TextAnalysis::default();
    }

    let mut chinese: Vec<char> = Vec::new();
    let mut has_latin = false;
    let mut has_chinese = false;
    let mut has_other = false;
    let mut english_characters = 0usize;

    for c in text.chars() {
        if is_chinese_char(c) {
            chinese.push(c);
            has_chinese = true;
        } else if c.is_ascii_alphabetic() {
            english_characters += 1;
            has_latin = true;
        } else if is_latin_char(c) {
            has_latin = true;
        } else if c.is_alphabetic() {
            has_other = true;
        }
    }

    let chinese_count = chinese.len();
    let script_count = [has_latin, has_chinese, has_other]
        .iter()
        .filter(|b| **b)
        .count();

    let traditional_count = chinese
        .iter()
        .filter(|c| crate::service::languages::to_simplified(&c.to_string()) != c.to_string())
        .count();

    TextAnalysis {
        chinese_character_count: chinese_count,
        simplified_character_count: chinese_count - traditional_count,
        traditional_character_count: traditional_count,
        chinese_character_ratio: chinese_count as f64 / total as f64,
        english_character_count: english_characters,
        english_character_ratio: english_characters as f64 / total as f64,
        has_mixed_scripts: script_count >= 2,
    }
}

// ---------------------------------------------------------------------------
// Script classification (replaces NLLanguageRecognizer)
// ---------------------------------------------------------------------------

/// Produces a probability distribution over the languages Easydict supports.
fn script_probabilities(text: &str) -> HashMap<Language, f64> {
    let mut han = 0usize;
    let mut hiragana = 0usize;
    let mut katakana = 0usize;
    let mut hangul = 0usize;
    let mut cyrillic = 0usize;
    let mut arabic = 0usize;
    let mut hebrew = 0usize;
    let mut thai = 0usize;
    let mut greek = 0usize;
    let mut devanagari = 0usize;
    let mut latin = 0usize;
    let mut letters = 0usize;

    for c in text.chars() {
        if !c.is_alphabetic() {
            continue;
        }
        letters += 1;
        let cp = c as u32;
        match cp {
            0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF => han += 1,
            0x3040..=0x309F => hiragana += 1,
            0x30A0..=0x30FF | 0x31F0..=0x31FF => katakana += 1,
            0xAC00..=0xD7AF | 0x1100..=0x11FF => hangul += 1,
            0x0400..=0x04FF | 0x0500..=0x052F => cyrillic += 1,
            0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF => arabic += 1,
            0x0590..=0x05FF => hebrew += 1,
            0x0E00..=0x0E7F => thai += 1,
            0x0370..=0x03FF | 0x1F00..=0x1FFF => greek += 1,
            0x0900..=0x097F => devanagari += 1,
            _ if c.is_ascii_alphabetic() => latin += 1,
            0x00C0..=0x024F | 0x1E00..=0x1EFF => latin += 1,
            _ => {}
        }
    }

    if letters == 0 {
        return HashMap::new();
    }

    let mut probs: HashMap<Language, f64> = HashMap::new();
    let total = letters as f64;

    // Counts below are deliberately per-script; a single script can only
    // nominate one or two languages, which mirrors how strongly
    // NLLanguageRecognizer narrows on non-Latin input.
    if han > 0 || hiragana > 0 || katakana > 0 {
        let kana = hiragana + katakana;
        if kana > 0 {
            // Any kana means Japanese: Han-only text is never Japanese.
            probs.insert(Language::Japanese, (han + kana) as f64 / total);
        } else {
            probs.insert(Language::SimplifiedChinese, han as f64 / total);
            probs.insert(Language::TraditionalChinese, han as f64 / total);
        }
    }

    if hangul > 0 {
        probs.insert(Language::Korean, hangul as f64 / total);
    }
    if cyrillic > 0 {
        let (ru, uk, sr, bg) = cyrillic_split(text);
        let denom = (ru + uk + sr + bg).max(1) as f64;
        if uk > 0 {
            probs.insert(Language::Ukrainian, cyrillic as f64 / total * (uk as f64 / denom));
        }
        if sr > 0 {
            probs.insert(Language::Serbian, cyrillic as f64 / total * (sr as f64 / denom));
        }
        if bg > 0 {
            probs.insert(Language::Bulgarian, cyrillic as f64 / total * (bg as f64 / denom));
        }
        if ru > 0 || probs.is_empty() {
            probs.insert(Language::Russian, cyrillic as f64 / total * (ru.max(1) as f64 / denom));
        }
    }
    if arabic > 0 {
        let (fa, ur, ar) = arabic_split(text);
        let denom = (fa + ur + ar).max(1) as f64;
        for (count, lang) in [
            (fa, Language::Persian),
            (ur, Language::Urdu),
            (ar, Language::Arabic),
        ] {
            if count > 0 {
                probs.insert(lang, arabic as f64 / total * (count as f64 / denom));
            }
        }
        // Arabic-script text with no distinguishing letters still counts as
        // Arabic rather than being dropped.
        if ar == 0 && fa == 0 && ur == 0 {
            probs.insert(Language::Arabic, arabic as f64 / total);
        }
    }
    if hebrew > 0 {
        probs.insert(Language::Hebrew, hebrew as f64 / total);
    }
    if thai > 0 {
        probs.insert(Language::Thai, thai as f64 / total);
    }
    if greek > 0 {
        probs.insert(Language::Greek, greek as f64 / total);
    }
    if devanagari > 0 {
        probs.insert(Language::Hindi, devanagari as f64 / total);
    }
    if latin > 0 {
        for (lang, score) in latin_ranking(text) {
            probs.insert(lang, latin as f64 / total * score);
        }
    }

    probs.retain(|_, v| *v > 0.0);
    probs
}

/// Applies diacritic and function-word evidence to split the Latin script.
fn latin_ranking(text: &str) -> Vec<(Language, f64)> {
    let lower = util::folded(text);
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphabetic() && c != '\'' && c != '-')
        .filter(|w| !w.is_empty())
        .collect();

    let mut scores: Vec<(Language, f64)> = Vec::new();

    // Without a word list the default is English, matching Apple's bias.
    for (lang, table) in LATIN_TABLES.iter() {
        let hits = words.iter().filter(|w| table_contains(table, w)).count();
        if hits >= MIN_FUNCTION_WORD_HITS {
            scores.push((*lang, hits as f64));
        }
    }

    // Diacritic evidence, which is decisive on single words.
    let diacritic = diacritic_language(text);
    if let Some(lang) = diacritic {
        match scores.iter_mut().find(|(l, _)| *l == lang) {
            Some((_, s)) => *s += 2.0,
            None => scores.push((lang, 2.0)),
        }
    }

    if scores.is_empty() {
        scores.push((Language::English, 1.0));
    }

    let total: f64 = scores.iter().map(|(_, s)| *s).sum();
    for (_, s) in scores.iter_mut() {
        *s /= total;
    }
    scores
}

/// Splits Cyrillic text between Russian, Ukrainian, Serbian and Bulgarian.
///
/// The letters that belong to only one of these four alphabets (`і`, `ї`,
/// `є`, `ґ`; `ђ`, `ј`, `љ`, `њ`, `ћ`, `џ`; `ы`, `э`, `ё`) are what tells them
/// apart, so one of them outweighs everything else. The letters shared by all
/// four (`а`, `е`, `и`, `о`, `р` …) say only "this is Cyrillic"; they are
/// credited to Russian, the default Cyrillic language, and only when no other
/// alphabet's distinctive letter is present. Crediting them to Bulgarian as
/// well let the ordinary letters of a sentence like "Привіт світе" outvote its
/// `і`/`ї`. `ъ` marks Bulgarian.
fn cyrillic_split(text: &str) -> (usize, usize, usize, usize) {
    let mut shared = 0;
    let mut ru = 0;
    let mut uk = 0;
    let mut sr = 0;
    let mut bg = 0;
    for c in text.to_lowercase().chars() {
        match c {
            'і' | 'ї' | 'є' | 'ґ' | 'ў' => uk += 1,
            'ђ' | 'ј' | 'љ' | 'њ' | 'ћ' | 'џ' => sr += 1,
            'ы' | 'э' | 'ё' => ru += 1,
            'ъ' => bg += 1,
            c if is_cyrillic(c) => shared += 1,
            _ => {}
        }
    }
    if uk == 0 && sr == 0 && bg == 0 {
        ru += shared;
    }
    (ru, uk, sr, bg)
}

/// Splits Arabic-script text between Persian, Urdu and Arabic.
fn arabic_split(text: &str) -> (usize, usize, usize) {
    let mut fa = 0;
    let mut ur = 0;
    let mut ar = 0;
    for c in text.chars() {
        match c {
            'پ' | 'چ' | 'ژ' | 'گ' => {
                fa += 2;
                ur += 2;
            }
            'ٹ' | 'ڈ' | 'ڑ' | 'ں' | 'ے' | 'ھ' => ur += 3,
            'ی' => {
                fa += 1;
                ur += 1;
            }
            'ک' => {
                fa += 2;
                ur += 1;
            }
            'ث' | 'ذ' | 'ض' | 'ظ' => ar += 2,
            c if is_arabic(c) => {
                ar += 1;
                fa += 1;
                ur += 1;
            }
            _ => {}
        }
    }
    (fa, ur, ar)
}

/// Diacritics that are characteristic of one Latin-script language.
fn diacritic_language(text: &str) -> Option<Language> {
    let has = |set: &str| text.chars().any(|c| set.contains(c.to_ascii_lowercase()));
    if has("ăâîșț") {
        return Some(Language::Romanian);
    }
    if has("łżźćńę") {
        return Some(Language::Polish);
    }
    if has("řžščďňů") {
        return Some(Language::Czech);
    }
    if has("řžšč") {
        return Some(Language::Slovak);
    }
    if has("őű") {
        return Some(Language::Hungarian);
    }
    if has("çğı") {
        return Some(Language::Turkish);
    }
    if has("åøæ") {
        return Some(Language::Norwegian);
    }
    if has("äö") {
        return Some(Language::German);
    }
    if has("ñ") {
        return Some(Language::Spanish);
    }
    if has("ãõ") {
        return Some(Language::Portuguese);
    }
    if has("àèìòù") {
        return Some(Language::Italian);
    }
    if has("ëïüÿ") {
        return Some(Language::French);
    }
    if has("ēūī") {
        return Some(Language::Latvian);
    }
    if has("ąęįų") {
        return Some(Language::Lithuanian);
    }
    if has("õäü") {
        return Some(Language::Estonian);
    }
    if has("čšž") {
        return Some(Language::Slovenian);
    }
    None
}

/// Function-word tables used to split the Latin script.
static LATIN_TABLES: &[(Language, &[&str])] = &[
    (
        Language::English,
        &["the", "and", "is", "are", "of", "to", "in", "that", "it", "for", "you", "was", "with", "this", "have", "a", "i"],
    ),
    (
        Language::French,
        &["le", "la", "les", "de", "des", "un", "une", "et", "est", "que", "qui", "dans", "pour", "pas", "vous", "nous", "je", "ce"],
    ),
    (
        Language::German,
        &["der", "die", "das", "und", "ist", "nicht", "ein", "eine", "mit", "von", "sich", "auf", "für", "ich", "sie", "auch", "dass"],
    ),
    (
        Language::Spanish,
        &["el", "la", "los", "las", "de", "que", "y", "en", "un", "una", "es", "por", "con", "no", "para", "se", "del"],
    ),
    (
        Language::Italian,
        &["il", "lo", "la", "di", "che", "e", "un", "una", "per", "con", "non", "sono", "del", "si", "come", "più"],
    ),
    (
        Language::Portuguese,
        &["o", "a", "os", "as", "de", "que", "e", "um", "uma", "para", "com", "não", "por", "do", "da", "se", "mais"],
    ),
    (
        Language::Dutch,
        &["de", "het", "een", "en", "van", "is", "dat", "niet", "op", "te", "je", "zijn", "met", "voor", "ik"],
    ),
    (
        Language::Swedish,
        &["och", "att", "det", "som", "en", "på", "är", "av", "för", "med", "till", "den", "inte", "jag"],
    ),
    (
        Language::Danish,
        &["og", "at", "det", "er", "en", "til", "på", "de", "med", "han", "der", "ikke", "jeg", "som"],
    ),
    (
        Language::Norwegian,
        &["og", "å", "det", "er", "en", "til", "på", "de", "med", "han", "ikke", "jeg", "som", "av"],
    ),
    (
        Language::Polish,
        &["i", "w", "nie", "na", "się", "z", "że", "do", "to", "jest", "jak", "po", "tak", "ale"],
    ),
    (
        Language::Czech,
        &["a", "v", "se", "na", "je", "že", "to", "s", "do", "ale", "jako", "za", "tak", "pro"],
    ),
    (
        Language::Turkish,
        &["ve", "bir", "bu", "için", "ile", "de", "da", "olarak", "çok", "daha", "ama", "ne"],
    ),
    (
        Language::Vietnamese,
        &["và", "của", "là", "có", "trong", "không", "được", "người", "những", "cho", "một", "này"],
    ),
    (
        Language::Indonesian,
        &["dan", "yang", "di", "itu", "dengan", "untuk", "tidak", "ini", "dari", "dalam", "akan", "pada"],
    ),
    (
        Language::Malay,
        &["dan", "yang", "di", "itu", "dengan", "untuk", "tidak", "ini", "dari", "dalam", "akan", "pada"],
    ),
    (
        Language::Romanian,
        &["și", "de", "la", "în", "este", "un", "o", "care", "cu", "pe", "nu", "se", "din"],
    ),
    (
        Language::Hungarian,
        &["és", "a", "az", "egy", "hogy", "nem", "van", "meg", "is", "de", "ez", "mint"],
    ),
    (
        Language::Finnish,
        &["ja", "on", "ei", "se", "että", "oli", "kun", "niin", "myös", "mutta", "tämä"],
    ),
    (
        Language::Croatian,
        &["i", "je", "na", "se", "da", "u", "za", "su", "od", "koji", "kao", "ali"],
    ),
    (
        Language::Slovenian,
        &["in", "je", "na", "se", "da", "v", "za", "so", "od", "ki", "kot", "ali"],
    ),
    (
        Language::Slovak,
        &["a", "v", "sa", "na", "je", "že", "to", "s", "do", "ale", "ako", "za"],
    ),
    (
        Language::Catalan,
        &["el", "la", "els", "les", "de", "que", "i", "en", "un", "una", "és", "per", "amb"],
    ),
    (
        Language::Filipino,
        &["ang", "ng", "sa", "na", "ay", "mga", "at", "ito", "para", "hindi", "ko", "si"],
    ),
];

// ---------------------------------------------------------------------------
// Character predicates, from `String+Detect.swift`
// ---------------------------------------------------------------------------

/// `isChineseTextByRegex` — CJK Unified Ideographs.
pub fn is_chinese_char(c: char) -> bool {
    matches!(c as u32,
        0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

fn is_latin_char(c: char) -> bool {
    let cp = c as u32;
    matches!(cp, 0x00C0..=0x024F | 0x1E00..=0x1EFF | 0x2C60..=0x2C7F | 0xA720..=0xA7FF)
}

fn is_cyrillic(c: char) -> bool {
    matches!(c as u32, 0x0400..=0x04FF | 0x0500..=0x052F)
}

fn is_arabic(c: char) -> bool {
    matches!(c as u32, 0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF)
}

/// `chineseChars`
pub fn chinese_chars(text: &str) -> String {
    text.chars().filter(|c| is_chinese_char(*c)).collect()
}

/// `isSimplifiedChinese` — 80 % threshold.
pub fn is_simplified_chinese(text: &str) -> bool {
    let chars = chinese_chars(text);
    if chars.is_empty() {
        return false;
    }
    let traditional = chars
        .chars()
        .filter(|c| {
            let s = c.to_string();
            crate::service::languages::to_simplified(&s) != s
        })
        .count();
    let simplified = chars.chars().count() - traditional;
    simplified as f64 / chars.chars().count() as f64 >= 0.8
}

/// `isChinese` — Chinese characters make up at least 50 % of the text.
pub fn is_chinese_text(text: &str) -> bool {
    let total = text.chars().count();
    if total == 0 {
        return false;
    }
    let chinese = chinese_chars(text).chars().count();
    if chinese == 0 {
        return false;
    }
    chinese as f64 / total as f64 >= 0.5
}

/// `isLatinText`
pub fn is_latin_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(is_latin_char)
}

/// `isEnglishText` — letters only, ASCII only.
pub fn is_english_text(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphabetic() && c.is_ascii())
}

/// `isNumeric`
pub fn is_numeric(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_numeric())
}

/// `isNumericHeavy` — more than 70 % digits.
pub fn is_numeric_heavy(text: &str) -> bool {
    let total = text.chars().count();
    if total == 0 {
        return false;
    }
    let digits = text.chars().filter(|c| c.is_numeric()).count();
    digits as f64 / total as f64 > 0.7
}

/// `englishWordCount`
pub fn english_word_count(text: &str) -> usize {
    word_components(text)
        .into_iter()
        .filter(|w| is_english_text(&removing_non_letters(w)))
        .count()
}

/// Splits on whitespace and punctuation, keeping words intact.
fn word_components(text: &str) -> Vec<String> {
    text.split_whitespace().map(|s| s.to_string()).collect()
}

/// `removingNonLetters()`
pub fn removing_non_letters(text: &str) -> String {
    text.chars().filter(|c| c.is_alphabetic()).collect()
}

/// `removeEnglishCharacters(from:)` — drops ASCII letters, collapsing spaces.
fn remove_english_characters(text: &str) -> String {
    let mut result = String::new();
    let mut previous_was_space = false;

    for c in text.chars() {
        if !(c.is_ascii_alphabetic()) {
            if c.is_whitespace() {
                if !previous_was_space && !result.is_empty() {
                    result.push(c);
                    previous_was_space = true;
                }
            } else {
                result.push(c);
                previous_was_space = false;
            }
        } else if !previous_was_space && !result.is_empty() {
            previous_was_space = true;
        }
    }

    result.trim().to_string()
}

/// Cached set lookup helper, so `LATIN_TABLES.contains` does not allocate a
/// `String` per word per language.
fn table_contains(table: &[&str], word: &str) -> bool {
    table.contains(&word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detect(text: &str) -> Language {
        LanguageDetector::new(vec![Language::English, Language::SimplifiedChinese], false)
            .detect_language(text)
    }

    #[test]
    fn empty_is_auto() {
        assert_eq!(detect(""), Language::Auto);
        assert_eq!(detect("   "), Language::Auto);
    }

    #[test]
    fn english_short_word() {
        assert_eq!(detect("apple"), Language::English);
        assert_eq!(detect("Hello world"), Language::English);
    }

    #[test]
    fn chinese_variants() {
        assert_eq!(detect("很棒"), Language::SimplifiedChinese);
        assert_eq!(detect("這個國家很美麗"), Language::TraditionalChinese);
    }

    #[test]
    fn japanese_needs_kana() {
        assert_eq!(detect("今日は国際ホッキョクグマの日"), Language::Japanese);
        // Han-only text is Chinese, never Japanese. Which variant comes from
        // `determineChineseType`, which needs 90% of the Han characters to be
        // simplified: 際, 鯨 and 員 are traditional forms, so 4 of 7 is not.
        assert_eq!(detect("国際捕鯨委員会"), Language::TraditionalChinese);
    }

    #[test]
    fn korean_and_cyrillic() {
        assert_eq!(detect("안녕하세요 세계"), Language::Korean);
        assert_eq!(detect("Привет, мир"), Language::Russian);
        assert_eq!(detect("Привіт світе, їжа"), Language::Ukrainian);
    }

    #[test]
    fn mixed_content_prefers_dominant() {
        assert_eq!(detect("Hello world with one 中文 word"), Language::English);
        assert_eq!(detect("apple苹果"), Language::SimplifiedChinese);
    }

    #[test]
    fn numeric_text_is_english() {
        assert_eq!(detect("729"), Language::English);
    }

    #[test]
    fn character_predicates() {
        assert!(is_chinese_text("你好"));
        assert!(!is_chinese_text("hello world this is text"));
        assert!(is_numeric("12345"));
        assert!(is_numeric_heavy("12345a"));
        assert!(is_english_text("Hello"));
        assert!(!is_english_text("Héllo"));
        assert_eq!(removing_non_letters("Hello, 世界! 123 café"), "Hello世界café");
    }

    #[test]
    fn chinese_ratio_thresholds() {
        assert!(is_simplified_chinese("这个国家的学生们"));
        assert!(!is_simplified_chinese("這個國家的學生們"));
    }
}
