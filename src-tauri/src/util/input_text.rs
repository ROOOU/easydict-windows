//! `String.handlingInputText()` — the pipeline every query text runs through
//! before a `QueryModel` is built.
//!
//! Ported from `Easydict/Swift/Utility/Extensions/String/String+HandleInputText.swift`.
//! Upstream calls it from the `inputText` setter
//! (`Easydict/Swift/Model/QueryModel.swift:99`), gated by
//! `automaticWordSegmentation`, `automaticallyRemoveCodeCommentSymbols` and
//! `replaceNewlineWithSpace` (`Defaults.Keys+Extension.swift:139-149`; the
//! defaults are `true`, `true`, `false`).
//!
//! The stage order is upstream's and observable: word segmentation, then comment
//! symbols, then newline replacement, then a final trim.

use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;

use crate::model::Language;
use crate::util::{analysis, try_remove_quotes};

/// The three switches that gate `handlingInputText()`, in upstream order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputTextOptions {
    /// `automaticWordSegmentation`
    pub automatic_word_segmentation: bool,
    /// `automaticallyRemoveCodeCommentSymbols`
    pub automatically_remove_code_comment_symbols: bool,
    /// `replaceNewlineWithSpace`
    pub replace_newline_with_space: bool,
}

/// `handleInputText()` — `String+HandleInputText.swift:178`.
///
/// `detect` stands in for `AppleService.shared.detectTextSync`
/// (`AppleService.swift:133`), which the comment-join heuristic calls with the
/// block it is joining.
pub fn handling_input_text(
    text: &str,
    options: &InputTextOptions,
    detect: &mut dyn FnMut(&str) -> Language,
) -> String {
    // `guard length > 0 else { return self }` — only the empty string is
    // rejected; whitespace runs the whole pipeline and is trimmed at the end.
    if text.is_empty() {
        return String::new();
    }

    let mut query_text = text.to_string();

    if options.automatic_word_segmentation {
        query_text = segment_words(&query_text);
    }
    if options.automatically_remove_code_comment_symbols {
        query_text = remove_comment_block_symbols(&query_text, detect);
    }
    if options.replace_newline_with_space {
        query_text = replace_newline_with_space(&query_text);
    }

    // `trimmingCharacters(in: .whitespacesAndNewlines)`
    query_text.trim().to_string()
}

/// `segmentWords()` — `String+HandleInputText.swift:101`.
///
/// `key_value` → `key value`. A single word is left alone when it is a real
/// English word, and split as code otherwise.
///
/// Approximation: upstream asks `AppleDictionary.queryDictionary(forText:language:)`
/// (`Easydict/Swift/Service/Dictionary/AppleDictionary/AppleDictionary.swift:193`)
/// whether the word is in an installed system dictionary, and there is no such
/// dictionary here. `analysis::is_english_word` — the port's `isEnglishWord`
/// predicate — stands in for the lexicon. It is a *shape* test (`^[a-zA-Z]+$`,
/// at most 20 characters, seen through quote pairs), not a word list, so two
/// upstream cases cannot be reproduced: `simpleWord` reads as a word here and is
/// left unsplit where upstream's dictionary misses it, and `'UIKit'` reads as a
/// hit so the quotes stay where upstream's lookup misses and strips them.
/// `LaTeX` does come out as upstream's test expects, which is the case the
/// upstream comment calls out.
pub fn segment_words(text: &str) -> String {
    let mut query_text = text.to_string();

    if is_single_word(text) {
        let is_dictionary_word = analysis::is_english_word(text);
        if !is_dictionary_word {
            if has_quotes_pair(text) {
                query_text = try_remove_quotes(text);
            } else {
                query_text = split_code_text(text);
            }
        }
    }

    query_text
}

/// `isSingleWord` — `String+Detection.swift:159`.
///
/// `components(separatedBy: " ")` splits on the space character alone, so a text
/// separated by tabs or newlines still counts as one word.
pub fn is_single_word(text: &str) -> bool {
    !text.is_empty() && !text.contains(' ')
}

/// `hasQuotesPair` — `String+QuoteHandling.swift:110`: stripping the matching
/// outer quote pair changes the text and leaves something behind.
pub fn has_quotes_pair(text: &str) -> bool {
    let removed = try_remove_quotes(text);
    !removed.is_empty() && removed != text
}

/// `splitCodeText()` — `String+HandleInputText.swift:18`.
///
/// `anchored_draggable_state` → `anchored draggable state`,
/// `anchoredDraggableState` → `anchored Draggable State`.
pub fn split_code_text(text: &str) -> String {
    let query_text = split_camel_case_text(&split_snake_case_text(text));

    // Filter empty text parts.
    query_text
        .split(' ')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// `splitSnakeCaseText()` — `String+Split.swift:60`.
fn split_snake_case_text(text: &str) -> String {
    text.replace('_', " ")
}

/// `splitCamelCaseText()` — `String+Split.swift:23`.
///
/// A space goes in front of an uppercase character that follows a lowercase one,
/// or that is followed by a lowercase one (`GetHTTPCode` → `Get HTTP Code`).
/// Swift walks `Character`s, i.e. graphemes, so this does too.
fn split_camel_case_text(text: &str) -> String {
    use unicode_segmentation::UnicodeSegmentation;

    let chars: Vec<&str> = text.graphemes(true).collect();
    let mut output_text = String::with_capacity(text.len());

    for (index, current) in chars.iter().enumerate() {
        if is_uppercase_char(current) && index > 0 {
            // Upstream nests these two tests (`if … else if …`), one space
            // either way, so the branch is written as the single condition it is.
            let follows_lowercase = is_lowercase_char(chars[index - 1]);
            let precedes_lowercase = index < chars.len() - 1 && is_lowercase_char(chars[index + 1]);
            if follows_lowercase || precedes_lowercase {
                output_text.push(' ');
            }
        }
        output_text.push_str(current);
    }

    output_text
}

/// `isUppercaseChar(_:)` — every scalar of the character is uppercase.
fn is_uppercase_char(text: &str) -> bool {
    text.chars().all(|c| c.is_uppercase())
}

/// `isLowercaseChar(_:)` — every scalar of the character is lowercase.
fn is_lowercase_char(text: &str) -> bool {
    text.chars().all(|c| c.is_lowercase())
}

/// `removeCommentBlockSymbols()` — `String+HandleInputText.swift:84`.
///
/// Two modes, and which one runs is the whole point of the function: when
/// *every* line starts with a comment symbol, all the prefixes are stripped;
/// otherwise only `/* … */` blocks are rewritten, in place, keeping the text
/// they marked — a plain block of `#` or `//` lines mixed with other text is
/// left exactly as it is.
pub fn remove_comment_block_symbols(
    text: &str,
    detect: &mut dyn FnMut(&str) -> Language,
) -> String {
    if !all_line_starts_with_comment_symbol(text) {
        return remove_comment_block_symbols_inline(text, detect);
    }

    remove_comment_symbol_prefix_and_join_texts(text, detect)
}

/// `allLineStartsWithCommentSymbol()` — `String+HandleInputText.swift:94`.
pub fn all_line_starts_with_comment_symbol(text: &str) -> bool {
    split_newlines(text)
        .iter()
        .all(|line| starts_with_comment_symbol(line))
}

/// `startsWithCommentSymbol(_:)` — `String+HandleInputText.swift:295`.
fn starts_with_comment_symbol(line: &str) -> bool {
    comment_symbol_prefix_regex().is_match(line)
}

/// `removeCommentBlockSymbolsInline()` — `String+HandleInputText.swift:206`.
fn remove_comment_block_symbols_inline(
    text: &str,
    detect: &mut dyn FnMut(&str) -> Language,
) -> String {
    let mut mutable_self = text.to_string();

    // The block regex runs over the original text, so the recorded byte ranges
    // stay valid as long as replacements happen from the end backwards.
    let blocks: Vec<(Range<usize>, String)> = block_comment_regex()
        .captures_iter(text)
        .map(|captures| {
            let whole = captures.get(0).expect("group 0 always matches");
            let content = captures.get(1).expect("group 1 always matches");
            (whole.start()..whole.end(), content.as_str().to_string())
        })
        .collect();

    // Process matches in reverse order to preserve indices.
    for (range, content) in blocks.into_iter().rev() {
        let content = trim_whitespaces(&content).to_string();
        let modified_text = remove_comment_symbol_prefix_and_join_texts(&content, detect);
        mutable_self.replace_range(range, &modified_text);
    }

    mutable_self
}

/// `removeCommentSymbolPrefixAndJoinTexts(_:)` — `String+HandleInputText.swift:240`.
///
/// Strips the comment prefix from every line and decides per line whether the
/// previous line was a wrapped continuation of the same sentence — those join,
/// the rest keep their line break.
///
/// Approximation: upstream measures each line with the system font
/// (`textsWidths`, `NSFont.systemFont`) and compares point widths. There is no
/// font metric here, so the character count is the width proxy. That makes
/// `singleAlphabetWidth` (widest line ÷ its character count) exactly 1, which
/// leaves upstream's `alphabetCount` threshold expressed in *characters*: 15 for
/// languages that separate words with spaces, 1.5 for the ones that run them
/// together. Those are the numbers upstream's font arithmetic averages out to,
/// so the join decisions match on the text lengths the heuristic was tuned for.
fn remove_comment_symbol_prefix_and_join_texts(
    content: &str,
    detect: &mut dyn FnMut(&str) -> Language,
) -> String {
    let lines = split_newlines(content);
    let widths: Vec<usize> = lines.iter().map(|line| line.chars().count()).collect();

    let Some(max_width) = widths.iter().copied().max() else {
        return content.to_string();
    };
    let max_width_index = widths
        .iter()
        .position(|width| *width == max_width)
        .unwrap_or(0);

    // `guard !lineAtMaxWidth.isEmpty else { return content }`
    if lines[max_width_index].is_empty() {
        return content.to_string();
    }

    let single_alphabet_width = max_width as f64 / lines[max_width_index].chars().count() as f64;

    // `EZLanguageManager.shared().isLanguageWordsNeedSpace(detectedLanguage)`
    // (`EZLanguageManager.m:309`) — the same question `Language::words_need_space`
    // answers.
    let is_english_type_language = detect(content).words_need_space();
    let alphabet_count: f64 = if is_english_type_language { 15.0 } else { 1.5 };
    let threshold = alphabet_count * single_alphabet_width;

    let mut modified_block_text = String::new();

    for (index, line) in lines.iter().enumerate() {
        // The prefix pattern here has no trailing `\s*`, so the space after the
        // symbol survives in `new_text` (and therefore on the lines that keep
        // their break) — unlike the standalone `remove_comment_symbol_prefix`.
        let new_text = comment_symbol_prefix_regex()
            .replace_all(line, "")
            .to_string();

        if index == 0 {
            modified_block_text = new_text;
            continue;
        }

        let is_prev_line_long_text = (max_width - widths[index - 1]) as f64 <= threshold;
        // Note that this measures the line *with* its comment prefix; the last
        // character is the same either way, so it makes no difference.
        let is_prev_line_end = analysis::has_end_punctuation_suffix(lines[index - 1]);
        let new_trimmed_text = trim_whitespaces(&new_text);

        if !new_trimmed_text.is_empty() && is_prev_line_long_text && !is_prev_line_end {
            let word_connector = if is_english_type_language { " " } else { "" };
            modified_block_text.push_str(word_connector);
            modified_block_text.push_str(new_trimmed_text);
        } else {
            modified_block_text.push('\n');
            modified_block_text.push_str(&new_text);
        }
    }

    modified_block_text
}

/// `removeCommentSymbolPrefix()` — `String+HandleInputText.swift:58`.
///
/// Spelled out as its own function because its pattern differs from the one the
/// join helper uses: this one ends in `\s*` and consumes the space after the
/// symbol. Upstream only declares it (the port has no caller for it yet); it is
/// reproduced so the pair stays together.
pub fn remove_comment_symbol_prefix(text: &str) -> String {
    comment_symbol_prefix_with_space_regex()
        .replace_all(text, "")
        .to_string()
}

/// `replacingNewlinesWithWhitespace()` — `String+Extension.swift:91`.
///
/// CRLF first, then the single characters, so `\r\n` becomes one space and not
/// two.
pub fn replace_newline_with_space(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// `^\s*(//+|#+|\*+)` — `commentSymbolPrefixPattern`,
/// `String+HandleInputText.swift:203`. Used both to test for a comment symbol
/// and to strip one, keeping the space that follows it.
fn comment_symbol_prefix_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"^\s*(//+|#+|\*+)").expect("valid comment prefix regex"))
}

/// `^\s*(//+|#+|\*+)\s*` — the pattern of `removeCommentSymbolPrefix()`,
/// `String+HandleInputText.swift:59`.
fn comment_symbol_prefix_with_space_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| {
        Regex::new(r"^\s*(//+|#+|\*+)\s*").expect("valid comment prefix with space regex")
    })
}

/// `/\*+(.*?)\*+/` with `.dotMatchesLineSeparators`
/// (`String+HandleInputText.swift:209`) — `(?s)` is the Rust spelling of that
/// option.
fn block_comment_regex() -> &'static Regex {
    static REGEX: OnceLock<Regex> = OnceLock::new();
    REGEX.get_or_init(|| Regex::new(r"(?s)/\*+(.*?)\*+/").expect("valid block comment regex"))
}

/// `CharacterSet.newlines`.
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// `components(separatedBy: .newlines)`.
///
/// A character *set*, not a sequence: `\r\n` is two separators and yields an
/// empty component between them. That is not a detail — an empty line does not
/// start with a comment symbol, so a CRLF block of comment lines takes the
/// inline path instead of the prefix-stripping one. Reproduced as-is, since CRLF
/// is the common line ending on Windows.
fn split_newlines(text: &str) -> Vec<&str> {
    text.split(is_newline).collect()
}

/// `trimmingCharacters(in: .whitespaces)` — spaces and tabs only. `.whitespaces`
/// holds every newline character out, unlike the `.whitespacesAndNewlines` the
/// pipeline ends with, so a leading newline survives this trim.
fn trim_whitespaces(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() && !is_newline(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::detect::LanguageDetector;

    /// All three switches at their upstream defaults.
    const DEFAULTS: InputTextOptions = InputTextOptions {
        automatic_word_segmentation: true,
        automatically_remove_code_comment_symbols: true,
        replace_newline_with_space: false,
    };

    /// The app's own detector (`AppState::new`), so the join heuristic is
    /// exercised against a real language rather than a stub.
    fn detector() -> LanguageDetector {
        LanguageDetector::new(vec![Language::English, Language::SimplifiedChinese], false)
    }

    fn handled(text: &str, options: InputTextOptions) -> String {
        let mut detector = detector();
        handling_input_text(text, &options, &mut |t| detector.detect_language(t))
    }

    fn comments(text: &str) -> String {
        let mut detector = detector();
        remove_comment_block_symbols(text, &mut |t| detector.detect_language(t))
    }

    // ---- handleInputText -------------------------------------------------

    #[test]
    fn empty_input_is_returned_unchanged() {
        assert_eq!(handled("", DEFAULTS), "");
    }

    #[test]
    fn every_setting_gates_its_stage() {
        // `automaticWordSegmentation`
        assert_eq!(handled("key_value", DEFAULTS), "key value");
        assert_eq!(
            handled(
                "key_value",
                InputTextOptions {
                    automatic_word_segmentation: false,
                    ..DEFAULTS
                }
            ),
            "key_value"
        );

        // `automaticallyRemoveCodeCommentSymbols`
        assert_eq!(handled("// hello", DEFAULTS), "hello");
        assert_eq!(
            handled(
                "// hello",
                InputTextOptions {
                    automatically_remove_code_comment_symbols: false,
                    ..DEFAULTS
                }
            ),
            "// hello"
        );

        // `replaceNewlineWithSpace`, off by default
        assert_eq!(handled("a\nb", DEFAULTS), "a\nb");
        assert_eq!(
            handled(
                "a\nb",
                InputTextOptions {
                    replace_newline_with_space: true,
                    ..DEFAULTS
                }
            ),
            "a b"
        );
    }

    #[test]
    fn stages_run_in_order_and_the_result_is_trimmed() {
        // Segmentation first: the `//` prefix is not a word, so it is not split,
        // and the prefix is then removed by the comment stage.
        assert_eq!(handled("//key_value", DEFAULTS), "key value");
        assert_eq!(handled("  hello  ", DEFAULTS), "hello");
    }

    // ---- allLineStartsWithCommentSymbol ----------------------------------

    #[test]
    fn comment_symbol_detection() {
        // Upstream's own table, `EasydictTests/Utility/String/StringCommentHandlingTests.swift:20`.
        let cases = [
            ("# This is a comment\n// Another comment", true),
            ("# Comment\nNot a comment", false),
            ("/* Block comment */\n* Another", false),
            ("Normal text\n// Comment", false),
            ("// Only comments\n# More comments\n* And more", true),
            ("", false),
            ("   # Indented comment", true),
        ];
        for (input, expected) in cases {
            assert_eq!(
                all_line_starts_with_comment_symbol(input),
                expected,
                "allLineStartsWithCommentSymbol({input:?})"
            );
        }
    }

    #[test]
    fn a_crlf_block_is_not_all_comment_lines() {
        // `\r\n` is two separators for `CharacterSet.newlines`, and the empty
        // component between them fails the test, so this block takes the inline
        // path and keeps its `#` prefixes. Odd, but upstream's behaviour, and
        // CRLF is what Windows sends.
        assert!(!all_line_starts_with_comment_symbol("# a\r\n# b"));
        assert_eq!(handled("# a\r\n# b", DEFAULTS), "# a\r\n# b");
        // The LF spelling of the same block does strip the prefixes.
        assert_eq!(handled("# a\n# b", DEFAULTS), "a b");
    }

    // ---- the two comment modes -------------------------------------------

    #[test]
    fn an_all_comment_block_keeps_its_text() {
        // The regression this port fixes: stripping `#` lines used to delete the
        // comment text and leave an empty query.
        assert_eq!(handled("# hello\n# world", DEFAULTS), "hello world");
        assert_eq!(handled("// one\n// two", DEFAULTS), "one two");
        assert_eq!(handled("* one\n* two", DEFAULTS), "one two");
        assert_eq!(
            handled("   // indented\n   // lines", DEFAULTS),
            "indented lines"
        );
    }

    #[test]
    fn a_mixed_block_leaves_hash_comments_alone() {
        // Only `/* … */` is rewritten when the lines are not all comments.
        assert_eq!(handled("# hello\nworld", DEFAULTS), "# hello\nworld");
        assert_eq!(
            handled("some text\n# trailing note", DEFAULTS),
            "some text\n# trailing note"
        );
        // Wrapped prose is joined: the first line is as long as the longest line
        // and does not end a sentence.
        assert_eq!(
            comments("// This is a comment\n// with multiple lines"),
            " This is a comment with multiple lines"
        );
    }

    #[test]
    fn an_inline_block_keeps_the_text_it_marked() {
        assert_eq!(handled("/* hi */ world", DEFAULTS), "hi world");
        assert_eq!(handled("/* hello\nworld */", DEFAULTS), "hello world");
        assert_eq!(handled("a /* note */ b", DEFAULTS), "a note b");
        // Several blocks on one line, rewritten right to left.
        assert_eq!(handled("x /* one */ y /* two */", DEFAULTS), "x one y two");
    }

    #[test]
    fn a_line_ending_in_punctuation_keeps_its_break() {
        // `hasEndPunctuationSuffix` on the previous line suppresses the join.
        assert_eq!(handled("/* Hello.\nWorld */", DEFAULTS), "Hello.\nWorld");
        assert_eq!(comments("// 你好。\n// 世界"), " 你好。\n 世界");
    }

    #[test]
    fn an_unterminated_block_is_left_alone() {
        assert_eq!(handled("/* hello", DEFAULTS), "/* hello");
        assert_eq!(handled("a /* b */ c /* d", DEFAULTS), "a b c /* d");
    }

    #[test]
    fn a_comment_only_input_can_become_empty() {
        // `//` is a comment symbol and nothing else, so the query ends up empty.
        assert_eq!(handled("//", DEFAULTS), "");
        assert_eq!(handled("/* */", DEFAULTS), "");
        assert_eq!(handled("   ", DEFAULTS), "");
    }

    #[test]
    fn chinese_comment_lines_join_without_a_space() {
        // `isLanguageWordsNeedSpace` is false for Chinese, so the connector is
        // empty (the `alphabetCount = 1.5` branch).
        assert_eq!(handled("# 你好\n# 世界", DEFAULTS), "你好世界");
    }

    // ---- splitCodeText ---------------------------------------------------

    #[test]
    fn camel_case_is_split() {
        // Upstream's table, `EasydictTests/Utility/String/StringCodeSplittingTests.swift:19`.
        let cases = [
            ("anchoredDraggableState", "anchored Draggable State"),
            ("AnchoredDraggableState", "Anchored Draggable State"),
            ("GetHTTP", "Get HTTP"),
            ("GetHTTPCode", "Get HTTP Code"),
            ("simpleWord", "simple Word"),
            ("", ""),
            ("XMLHttpRequest", "XML Http Request"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                split_camel_case_text(input),
                expected,
                "splitCamelCaseText({input:?})"
            );
        }
    }

    #[test]
    fn snake_case_is_split() {
        assert_eq!(
            split_snake_case_text("multiple___underscores"),
            "multiple   underscores"
        );
        assert_eq!(split_snake_case_text("already spaced"), "already spaced");
    }

    #[test]
    fn code_text_is_split_both_ways_and_collapsed() {
        let cases = [
            ("anchored_draggable_state", "anchored draggable state"),
            ("GetHTTPCode", "Get HTTP Code"),
            ("anchoredDraggableState", "anchored Draggable State"),
            ("simple_test_case", "simple test case"),
            ("XMLHttpRequest", "XML Http Request"),
            ("a_b_c", "a b c"),
            // Empty parts are filtered, so the spaces left by `_` collapse.
            ("multiple___underscores", "multiple underscores"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(split_code_text(input), expected, "splitCodeText({input:?})");
        }
    }

    // ---- segmentWords ----------------------------------------------------

    #[test]
    fn words_are_segmented_only_when_they_are_code() {
        let cases = [
            ("key_value", "key value"),
            ("test_case", "test case"),
            // "LaTeX" is a real dictionary word upstream and stays whole; the
            // shape test agrees.
            ("LaTeX", "LaTeX"),
            // A multi-word text is never segmented (`isSingleWord` is false).
            ("already spaced", "already spaced"),
            ("", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(segment_words(input), expected, "segmentWords({input:?})");
        }
    }

    #[test]
    fn quoted_code_loses_its_quotes_but_is_not_split() {
        // Upstream: the dictionary misses, `hasQuotesPair` strips the quotes, and
        // no splitting happens on that branch.
        assert_eq!(segment_words("'key_value'"), "key_value");
    }

    #[test]
    fn the_dictionary_stand_in_cannot_tell_code_from_words() {
        // Documented divergence from `StringFormattingTests.swift:19`: upstream's
        // dictionary misses `simpleWord` (so it becomes `simple Word`) and hits
        // `UIKit` (so the quotes come off). Without a lexicon this port treats any
        // 20-letter ASCII word as a dictionary hit, so both are left as typed.
        assert_eq!(segment_words("simpleWord"), "simpleWord");
        assert_eq!(segment_words("'UIKit'"), "'UIKit'");
    }

    // ---- removeCommentSymbolPrefix ---------------------------------------

    #[test]
    fn the_prefix_removal_eats_the_space_too() {
        // Upstream's table, `StringCommentHandlingTests.swift:54`.
        let cases = [
            ("// Comment", "Comment"),
            ("# Hash comment", "Hash comment"),
            ("* Asterisk comment", "Asterisk comment"),
            ("    // Indented comment", "Indented comment"),
            ("Normal text", "Normal text"),
        ];
        for (input, expected) in cases {
            assert_eq!(
                remove_comment_symbol_prefix(input),
                expected,
                "for {input:?}"
            );
        }
    }

    // ---- replaceNewlinesWithWhitespace -----------------------------------

    #[test]
    fn newlines_become_whitespace() {
        // Upstream's table, `StringFormattingTests.swift:58`.
        let cases = [
            ("line1\nline2", "line1 line2"),
            ("line1\n\nline2", "line1  line2"),
            ("line1   \n   line2", "line1       line2"),
            ("single line", "single line"),
            ("", ""),
            ("\nleading newline", " leading newline"),
            // A CRLF pair is one separator, not two.
            ("a\r\nb", "a b"),
        ];
        for (input, expected) in cases {
            assert_eq!(replace_newline_with_space(input), expected, "for {input:?}");
        }
    }

    // ---- predicates ------------------------------------------------------

    #[test]
    fn single_word_needs_a_space_to_stop_being_one() {
        assert!(is_single_word("key_value"));
        assert!(is_single_word("\t"));
        assert!(!is_single_word(""));
        assert!(!is_single_word("key value"));
    }

    #[test]
    fn quotes_pair_needs_a_non_empty_inside() {
        assert!(has_quotes_pair("'UIKit'"));
        assert!(has_quotes_pair("“你好”"));
        assert!(!has_quotes_pair("UIKit"));
        assert!(!has_quotes_pair("''"));
    }
}
