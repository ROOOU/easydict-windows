//! Small utilities shared across services: hashing, string handling, HTML.

pub mod analysis;
pub mod input_text;

use base64::Engine;
use md5::{Digest as _, Md5};
use sha2::Sha256;

/// Lowercase hex MD5, matching `String.md5()` in the macOS app.
pub fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    hex_lower(&hasher.finalize())
}

/// Lowercase hex MD5 of raw bytes.
pub fn md5_hex_bytes(input: &[u8]) -> String {
    let mut hasher = Md5::new();
    hasher.update(input);
    hex_lower(&hasher.finalize())
}

/// Raw MD5 digest bytes — used by Youdao's AES key derivation.
pub fn md5_bytes(input: &[u8]) -> [u8; 16] {
    let mut hasher = Md5::new();
    hasher.update(input);
    let out = hasher.finalize();
    let mut buf = [0u8; 16];
    buf.copy_from_slice(&out);
    buf
}

/// Lowercase hex SHA-256, matching `String.sha256()`.
pub fn sha256_hex(input: &str) -> String {
    use sha2::Digest as _;
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    hex_lower(&hasher.finalize())
}

/// Raw SHA-256 digest bytes.
pub fn sha256_bytes(input: &[u8]) -> Vec<u8> {
    use sha2::Digest as _;
    let mut hasher = Sha256::new();
    hasher.update(input);
    hasher.finalize().to_vec()
}

/// HMAC-SHA1, returning raw bytes. Used by Alibaba's RPC signature.
pub fn hmac_sha1(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, KeyInit, Mac};
    use sha1::Sha1;
    let mut mac = <Hmac<Sha1> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// HMAC-SHA256, returning raw bytes.
pub fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    use hmac::{Hmac, KeyInit, Mac};
    let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

/// HMAC-SHA256 as lowercase hex.
pub fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    hex_lower(&hmac_sha256(key, data))
}

/// AES-128-CBC decryption with PKCS#7 padding.
///
/// The caller supplies the raw 16-byte key and IV; Youdao derives both by
/// MD5-hashing the two strings it fetched from its key endpoint.
pub fn aes_128_cbc_decrypt(
    key: &[u8; 16],
    iv: &[u8; 16],
    ciphertext: &[u8],
) -> Option<Vec<u8>> {
    use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
    type Decryptor = cbc::Decryptor<aes::Aes128>;
    let decryptor = Decryptor::new(key.into(), iv.into());
    // The 0.5 API decrypts in place, so the caller's slice is copied first.
    let mut buffer = ciphertext.to_vec();
    decryptor
        .decrypt_padded::<Pkcs7>(&mut buffer)
        .ok()
        .map(<[u8]>::to_vec)
}

/// Decodes URL-safe base64 where `-`/`_` stand in for `+`/`/`.
pub fn base64_decode_websafe(input: &str) -> Option<Vec<u8>> {
    base64_decode_urlsafe(input)
}

/// Percent-encodes using `.urlQueryAllowed`, which — as in Foundation —
/// deliberately leaves `&`, `=`, `+`, `?`, `/`, `:` and `#` unescaped.
///
/// Used by the TTS URL builders, which reproduce the upstream behaviour
/// including its known quirk of not escaping `&` inside a query value.
pub fn url_encode_query_allowed(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => out.push(*byte as char),
            // `.urlQueryAllowed` subtracts only these.
            b' ' => out.push_str("%20"),
            b'"' | b'<' | b'>' | b'`' | b'{' | b'}' | b'|' | b'\\' | b'^' | b'[' | b']' => {
                out.push_str(&format!("%{byte:02X}"))
            }
            // `.urlQueryAllowed` holds ASCII graphics only, so everything below
            // 0x21 or above 0x7E is escaped too. This is not a detail: the query
            // text itself travels through here, and pushing a byte ≥ 0x80 as a
            // `char` would re-read it as Latin-1 — `你好` would reach the server
            // as the mojibake `ä½ å¥½`, double-encoded on the wire. Written as
            // its UTF-8 bytes it arrives intact.
            _ if !(0x21..=0x7E).contains(byte) => out.push_str(&format!("%{byte:02X}")),
            _ => out.push(*byte as char),
        }
    }
    out
}

/// Counts grapheme clusters, matching Swift's `String.count`.
pub fn grapheme_count(text: &str) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    text.graphemes(true).count()
}

/// Truncates to `max` grapheme clusters (Swift `String(prefix(_:))`).
pub fn prefix_graphemes(text: &str, max: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    text.graphemes(true).take(max).collect()
}

/// Truncates to `max` UTF-16 code units, matching Foundation's
/// `NSString.substring(to:)` as used by `ns_trimToMaxLength`.
pub fn ns_trim_to_max_length(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    let mut units = 0usize;
    let mut end = 0usize;
    for (idx, c) in trimmed.char_indices() {
        let len = c.len_utf16();
        if units + len > max {
            break;
        }
        units += len;
        end = idx + c.len_utf8();
    }
    trimmed[..end].to_string()
}

pub fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0x0f) as u32, 16).unwrap());
    }
    s
}

pub fn base64_encode(input: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(input)
}

/// Decodes URL-safe base64 (Youdao returns `-`/`_`), tolerating missing padding.
pub fn base64_decode_urlsafe(input: &str) -> Option<Vec<u8>> {
    let normalized: String = input
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            other => other,
        })
        .collect();
    let padded = match normalized.len() % 4 {
        2 => format!("{normalized}=="),
        3 => format!("{normalized}="),
        _ => normalized,
    };
    base64::engine::general_purpose::STANDARD.decode(padded).ok()
}

/// Percent-encodes a string for use in a URL query, matching
/// `addingPercentEncoding(withAllowedCharacters: .urlQueryAllowed)`.
pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for byte in input.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Percent-encodes, additionally escaping `+` — needed for Youdao's
/// `dictvoice?audio=` parameter, which misreads a bare `+` as a space.
pub fn url_encode_plus_safe(input: &str) -> String {
    url_encode(input).replace('+', "%2B")
}

/// Escapes the five XML entities, matching `escapeXMLEntities()`.
pub fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('\'', "&apos;")
        .replace('"', "&quot;")
}

/// Escapes HTML text content.
pub fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Truncates to `max` characters (not bytes), matching `trimToMaxLength`.
pub fn trim_to_max_chars(input: &str, max: usize) -> String {
    if input.chars().count() <= max {
        return input.to_string();
    }
    input.chars().take(max).collect()
}

/// Splits translated text into paragraphs on `\n`, keeping empty lines —
/// matches `toParagraphs()`.
pub fn to_paragraphs(text: &str) -> Vec<String> {
    text.split('\n').map(|s| s.to_string()).collect()
}

/// Removes a leading/trailing quote pair, matching `tryToRemoveQuotes()`.
pub fn try_remove_quotes(text: &str) -> String {
    let t = text.trim();
    let pairs = [
        ('"', '"'),
        ('\'', '\''),
        ('“', '”'),
        ('‘', '’'),
        ('「', '」'),
        ('《', '》'),
        ('"', '"'),
    ];
    for (open, close) in pairs {
        if t.starts_with(open) && t.ends_with(close) && t.chars().count() >= 2 {
            let mut chars: Vec<char> = t.chars().collect();
            chars.remove(0);
            chars.pop();
            return chars.into_iter().collect::<String>().trim().to_string();
        }
    }
    text.to_string()
}

/// Strips `<think>…</think>` blocks, matching `filterThinkTagContent()`.
pub fn filter_think_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<think>") {
        out.push_str(&rest[..start]);
        match rest[start..].find("</think>") {
            Some(end) => rest = &rest[start + end + "</think>".len()..],
            None => return out, // unterminated: drop the remainder
        }
    }
    out.push_str(rest);
    out
}

/// Normalises whitespace for cache keys and comparisons.
pub fn collapse_whitespace(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for c in text.chars() {
        if c.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(c);
            last_space = false;
        }
    }
    out.trim().to_string()
}

/// Case- and diacritic-insensitive fold, approximating `foldedString()`.
pub fn folded(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' => Some('a'),
            'é' | 'è' | 'ê' | 'ë' => Some('e'),
            'í' | 'ì' | 'î' | 'ï' => Some('i'),
            'ó' | 'ò' | 'ô' | 'ö' | 'õ' => Some('o'),
            'ú' | 'ù' | 'û' | 'ü' => Some('u'),
            'ç' => Some('c'),
            'ñ' => Some('n'),
            'ý' | 'ÿ' => Some('y'),
            other => Some(other),
        })
        .collect::<String>()
        .to_lowercase()
}

/// Counts words using whitespace splitting, matching `wordCount`.
pub fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

/// True when the text is a single English word (letters plus internal
/// hyphen/apostrophe only).
pub fn is_english_word(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let mut has_letter = false;
    for c in text.chars() {
        if c.is_ascii_alphabetic() {
            has_letter = true;
        } else if c == '-' || c == '\'' || c == '’' {
            continue;
        } else {
            return false;
        }
    }
    has_letter
}

/// True when the text contains any CJK ideograph.
pub fn contains_cjk(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32,
            0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
    })
}

/// Counts CJK ideographs.
pub fn cjk_count(text: &str) -> usize {
    text.chars()
        .filter(|c| {
            matches!(*c as u32,
                0x4E00..=0x9FFF | 0x3400..=0x4DBF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
        })
        .count()
}

/// Counts characters that are simplified-Chinese-only, and those that are
/// traditional-Chinese-only. Used to pick between the two variants.
pub fn chinese_variant_counts(text: &str) -> (usize, usize) {
    let mut simplified = 0;
    let mut traditional = 0;
    for c in text.chars() {
        if SIMPLIFIED_ONLY.contains(&c) {
            simplified += 1;
        }
        if TRADITIONAL_ONLY.contains(&c) {
            traditional += 1;
        }
    }
    (simplified, traditional)
}

/// A compact set of characters that are unambiguously simplified.
const SIMPLIFIED_ONLY: &[char] = &[
    '们', '个', '为', '这', '说', '时', '会', '对', '来', '学', '国', '后', '么', '过', '样', '点',
    '现', '发', '样', '问', '题', '书', '写', '听', '读', '讲', '话', '语', '词', '译', '词', '词',
    '爱', '欢', '乐', '车', '门', '马', '鸟', '鱼', '龙', '风', '云', '电', '头', '长', '万', '与',
    '专', '业', '东', '丝', '严', '丽', '义', '乌', '乔', '习', '乡', '书', '买', '乱', '争', '于',
    '亏', '亚', '产', '亲', '亿', '仅', '从', '仑', '仓', '仪', '们', '价', '众', '优', '会', '传',
];

/// A compact set of characters that are unambiguously traditional.
const TRADITIONAL_ONLY: &[char] = &[
    '們', '個', '為', '這', '說', '時', '會', '對', '來', '學', '國', '後', '麼', '過', '樣', '點',
    '現', '發', '問', '題', '書', '寫', '聽', '讀', '講', '話', '語', '詞', '譯', '愛', '歡', '樂',
    '車', '門', '馬', '鳥', '魚', '龍', '風', '雲', '電', '頭', '長', '萬', '與', '專', '業', '東',
    '絲', '嚴', '麗', '義', '烏', '喬', '習', '鄉', '買', '亂', '爭', '於', '虧', '亞', '產', '親',
    '億', '僅', '從', '侖', '倉', '儀', '價', '眾', '優', '傳',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashing_matches_known_vectors() {
        // Known vector: md5("abc")
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
        // Known vector: sha256("abc")
        assert_eq!(
            sha256_hex("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn xml_escaping() {
        assert_eq!(escape_xml("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }

    #[test]
    fn think_tag_filter() {
        assert_eq!(filter_think_tags("<think>secret</think>Hello"), "Hello");
        assert_eq!(filter_think_tags("Hello"), "Hello");
        assert_eq!(filter_think_tags("Hi <think>unclosed"), "Hi ");
    }

    #[test]
    fn quotes_removed() {
        assert_eq!(try_remove_quotes("\"hello\""), "hello");
        assert_eq!(try_remove_quotes("“你好”"), "你好");
        assert_eq!(try_remove_quotes("hello"), "hello");
    }

    #[test]
    fn chinese_variant_detection() {
        let (s, t) = chinese_variant_counts("这个国家");
        assert!(s > 0 && t == 0);
        let (s, t) = chinese_variant_counts("這個國家");
        assert!(t > 0 && s == 0);
    }

    #[test]
    fn english_word_detection() {
        assert!(is_english_word("good"));
        assert!(is_english_word("well-known"));
        assert!(!is_english_word("good bad"));
        assert!(!is_english_word("good1"));
    }

    #[test]
    fn url_encoding() {
        assert_eq!(url_encode("a b"), "a%20b");
        assert_eq!(url_encode_plus_safe("a+b"), "a%2Bb");
    }

    #[test]
    fn url_query_encoding_escapes_non_ascii_as_utf8() {
        // The TTS and "open in browser" URLs carry the query text itself, so a
        // Chinese query has to leave here as its UTF-8 bytes rather than as
        // each byte re-read as a Latin-1 scalar.
        assert_eq!(url_encode_query_allowed("你好"), "%E4%BD%A0%E5%A5%BD");
        assert_eq!(url_encode_query_allowed("café"), "caf%C3%A9");
        assert_eq!(url_encode_query_allowed("😀"), "%F0%9F%98%80");
        // A raw control character would leave the URL malformed.
        assert_eq!(url_encode_query_allowed("a\nb"), "a%0Ab");
        assert_eq!(url_encode_query_allowed("a\u{7f}b"), "a%7Fb");
        // The Foundation quirks this reproduces are untouched: only space and
        // the non-graphic ASCII characters are escaped, and the sub-delimiters
        // stay literal.
        assert_eq!(url_encode_query_allowed("a b"), "a%20b");
        assert_eq!(url_encode_query_allowed("a&b=c+d?e/f:g"), "a&b=c+d?e/f:g");
    }

    #[test]
    fn base64_urlsafe_roundtrip() {
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b"hello?");
        let decoded = base64_decode_urlsafe(&encoded).unwrap();
        assert_eq!(decoded, b"hello?");
    }
}
