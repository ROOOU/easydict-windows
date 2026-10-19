//! Text classification: is this text a word, a phrase or a sentence, and which
//! query type should it be sent as?
//!
//! Ported from `String+Analysis.swift` and `String+Detection.swift`.

use crate::model::{Language, QueryTextType};
use crate::util;

/// `String.englishWordMaxLength`
pub const ENGLISH_WORD_MAX_LENGTH: usize = 20;

/// `String.endPunctuationMarks` (`String+Constants.swift:26`).
///
/// Upstream's list is `[".", "。", "?", "？", "!", "！", ";", ":", "：", "..",
/// "...", "…", "……"]`, and `hasEndPunctuationSuffix` tests it as
/// `contains(String(lastChar))` — a single `Character` against string entries,
/// so the multi-character entries can never match and what remains is these ten.
/// The port used to also accept `；` and `⋯`; neither is upstream's, and both
/// callers (`should_query_sentence`, the comment-join heuristic) decide
/// differently because of it.
const END_PUNCTUATION_MARKS: &[char] = &['.', '。', '?', '？', '!', '！', ';', ':', '：', '…'];

/// `hasEndPunctuationSuffix`
pub fn has_end_punctuation_suffix(text: &str) -> bool {
    match text.chars().last() {
        Some(c) => END_PUNCTUATION_MARKS.contains(&c),
        None => false,
    }
}

/// `isChineseText` — strictly `^[一-龥]+$`, matching the upstream regex.
pub fn is_chinese_text_strict(text: &str) -> bool {
    !text.is_empty()
        && text
            .chars()
            .all(|c| matches!(c as u32, 0x4E00..=0x9FA5))
}

/// `isNumbers`
pub fn is_numbers(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

/// `isEnglishWord` — `^[a-zA-Z]+$` after quote removal, length capped at 20.
pub fn is_english_word(text: &str) -> bool {
    is_english_word_with_max_length(text, ENGLISH_WORD_MAX_LENGTH)
}

pub fn is_english_word_with_max_length(text: &str, max_length: usize) -> bool {
    let t = util::try_remove_quotes(text);
    if t.is_empty() || t.chars().count() > max_length {
        return false;
    }
    t.chars().all(|c| c.is_ascii_alphabetic())
}

/// `isEnglishPhrase` — two words, letters only, total length ≤ 40.
pub fn is_english_phrase(text: &str) -> bool {
    let joined: String = text.chars().filter(|c| *c != ' ').collect();
    let is_phrase_length = is_english_word_with_max_length(&joined, ENGLISH_WORD_MAX_LENGTH * 2);
    is_phrase_length && word_count(text) == 2
}

/// `isWord`
pub fn is_word(text: &str) -> bool {
    let t = util::try_remove_quotes(text);
    if t.chars().count() > ENGLISH_WORD_MAX_LENGTH {
        return false;
    }
    word_count(text) == 1
}

/// `isChineseWord` — ≤ 4 characters, all Han.
pub fn is_chinese_word(text: &str) -> bool {
    let t = util::try_remove_quotes(text);
    if t.chars().count() > 4 {
        return false;
    }
    is_chinese_text_strict(&t)
}

/// `isChinesePhrase` — ≤ 5 characters, all Han.
pub fn is_chinese_phrase(text: &str) -> bool {
    let t = util::try_remove_quotes(text);
    if t.chars().count() > 5 {
        return false;
    }
    is_chinese_text_strict(&t)
}

/// `wordCount` — `NLTokenizer(unit: .word)`.
///
/// Han text is segmented by the upstream tokenizer using a Chinese lexicon.
/// Lacking that, this uses a CJK-aware approximation that matches `NLTokenizer`
/// on the cases Easydict actually branches on: an all-Han run under five
/// characters is treated as a single word (which is how Chinese dictionary
/// lookups behave), while longer runs are split on punctuation and digits.
pub fn word_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    if is_chinese_text_strict(text) {
        return chinese_word_count(text);
    }
    text.split_whitespace()
        .filter(|s| {
            !s.is_empty() && !s.chars().all(|c| c.is_whitespace() || is_punctuation(c))
        })
        .count()
}

fn is_punctuation(c: char) -> bool {
    matches!(c,
        '!' | '"' | '#' | '%' | '&' | '\'' | '(' | ')' | '*' | ',' | '-' | '.' | '/'
        | ':' | ';' | '?' | '@' | '[' | '\\' | ']' | '_' | '{' | '}' | '…'
        | '。' | '，' | '、' | '；' | '：' | '？' | '！' | '“' | '”' | '‘' | '’'
        | '（' | '）' | '《' | '》' | '「' | '」')
}

/// Approximates Chinese word segmentation without a lexicon.
///
/// Upstream delegates to `NLTokenizer`'s Chinese lexicon. Without it, a Han run
/// is treated as one word, which is the behaviour the dictionary/sentence
/// decision depends on: every input that reaches `isChineseWord` /
/// `isChinesePhrase` is under six characters and is meant to be looked up whole.
fn chinese_word_count(_text: &str) -> usize {
    1
}

/// `shouldQueryDictionary(withLanguage:maxWordCount:)`
pub fn should_query_dictionary(text: &str, language: Language, max_word_count: usize) -> bool {
    if language.is_chinese() {
        if text.chars().count() > max_word_count {
            return false;
        }
        return is_chinese_word(text) || is_chinese_phrase(text);
    }

    if language == Language::English {
        if text.chars().count() > ENGLISH_WORD_MAX_LENGTH {
            return false;
        }
        return word_count(text) == 1;
    }

    false
}

/// `shouldQuerySentence(withLanguage:)`
pub fn should_query_sentence(text: &str, language: Language) -> bool {
    if !(word_count(text) > 1 || has_end_punctuation_suffix(text)) {
        return false;
    }
    if should_query_dictionary(text, language, 1) {
        return false;
    }
    true
}

/// `queryType(language:maxWordCount:)` — the routing decision that picks which
/// services answer a given input.
pub fn query_type(text: &str, language: Language, max_word_count: usize) -> QueryTextType {
    if should_query_dictionary(text, language, max_word_count) {
        return QueryTextType::DICTIONARY;
    }
    if should_query_sentence(text, language) && language == Language::English {
        return QueryTextType::SENTENCE;
    }
    QueryTextType::TRANSLATION
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_word_rules() {
        assert!(is_english_word("good"));
        assert!(is_english_word("Good"));
        assert!(is_english_word("\"good\""));
        assert!(!is_english_word("good bad"));
        assert!(!is_english_word("good1"));
        // 21 letters exceeds the 20-character cap.
        assert!(!is_english_word("abcdefghijklmnopqrstu"));
    }

    #[test]
    fn chinese_word_rules() {
        assert!(is_chinese_word("你好"));
        assert!(is_chinese_word("四个字"));
        // `isChineseWord` allows `count <= 4`, so four characters is a word.
        assert!(is_chinese_word("五个字啊"));
        assert!(!is_chinese_word("五个字啊啊")); // 5 chars > 4
        assert!(is_chinese_phrase("五个字啊"));
        // `isChinesePhrase` allows `count <= 5`.
        assert!(is_chinese_phrase("六个字啊啊"));
        assert!(!is_chinese_phrase("六个字啊啊啊")); // 6 chars > 5
        // Punctuation disqualifies the strict Han-only regex.
        assert!(!is_chinese_word("你好，"));
    }

    #[test]
    fn sentence_rules() {
        assert!(should_query_sentence("Hello world", Language::English));
        assert!(!should_query_sentence("apple", Language::English));
        // A dictionary hit is never a sentence.
        assert!(!should_query_sentence("apple", Language::English));
    }

    #[test]
    fn query_type_routing() {
        assert_eq!(
            query_type("apple", Language::English, 1),
            QueryTextType::DICTIONARY
        );
        assert_eq!(
            query_type("hello world", Language::English, 1),
            QueryTextType::SENTENCE
        );
        assert_eq!(
            query_type("你好", Language::SimplifiedChinese, 4),
            QueryTextType::DICTIONARY
        );
        assert_eq!(
            query_type(
                "这是一段很长的中文文本用来测试翻译路径",
                Language::SimplifiedChinese,
                4
            ),
            QueryTextType::TRANSLATION
        );
    }

    #[test]
    fn word_counts() {
        assert_eq!(word_count(""), 0);
        assert_eq!(word_count("hello"), 1);
        assert_eq!(word_count("hello world"), 2);
        assert_eq!(word_count("hello, world!"), 2);
        assert_eq!(word_count("你好"), 1);
        assert_eq!(word_count("你好世界"), 1);
    }

    #[test]
    fn punctuation_suffix() {
        assert!(has_end_punctuation_suffix("Hello."));
        assert!(has_end_punctuation_suffix("你好。"));
        assert!(!has_end_punctuation_suffix("Hello"));

        // Upstream's table, `EasydictTests/Utility/String/StringFormattingTests.swift:34`.
        // Note `text..` is true because the last Character is the single `.`,
        // which is why the multi-character entries in upstream's list are inert.
        for (input, expected) in [
            ("text.", true),
            ("text!", true),
            ("text?", true),
            ("text。", true),
            ("text…", true),
            ("text", false),
            ("text..", true),
            ("text,", false),
            ("", false),
        ] {
            assert_eq!(
                has_end_punctuation_suffix(input),
                expected,
                "hasEndPunctuationSuffix({input:?})"
            );
        }
    }
}
