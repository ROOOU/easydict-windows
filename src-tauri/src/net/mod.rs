//! Shared HTTP client.
//!
//! The macOS app uses Alamofire with two timeouts: `networkTimeoutInterval`
//! (15 s) for regular translation APIs and `llmRequestTimeoutInterval` (60 s)
//! for LLM streams. Both are reproduced here.

use std::time::Duration;

use once_cell::sync::Lazy;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, ACCEPT, CONTENT_TYPE, USER_AGENT};
use reqwest::{Client, Response};

use crate::model::{ErrorType, QueryError};

/// `AppConstants.networkTimeoutInterval`
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(15);
/// `AppConstants.llmRequestTimeoutInterval`
pub const LLM_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// `AppConstants.userAgent`
pub const USER_AGENT_ANDROID: &str = "Mozilla/5.0 (Linux; Android 6.0; Nexus 5 Build/MRA58N) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36";

/// The macOS-only UA used for Google requests.
pub const USER_AGENT_MAC_GOOGLE: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_0) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/77.0.3865.120 Safari/537.36";

/// A client with a 15 s timeout, used by every non-LLM service.
pub static CLIENT: Lazy<Client> = Lazy::new(|| build_client(NETWORK_TIMEOUT));

/// A client with a 60 s timeout, used by LLM services.
pub static LLM_CLIENT: Lazy<Client> = Lazy::new(|| build_client(LLM_REQUEST_TIMEOUT));

fn build_client(timeout: Duration) -> Client {
    Client::builder()
        // An *idle* timeout, not a deadline for the whole exchange:
        // `URLRequest.timeoutInterval` (the macOS original) resets every time
        // bytes arrive. reqwest's `timeout` would instead cap the total, which
        // cuts a long LLM stream off at 60 s no matter how steadily it is
        // arriving — so the streaming path needs `read_timeout`.
        .read_timeout(timeout)
        .connect_timeout(Duration::from_secs(10))
        .user_agent(USER_AGENT_ANDROID)
        // Some Chinese providers (Youdao, Bing) serve different bodies based
        // on Accept-Encoding; letting reqwest negotiate keeps parity with
        // URLSession's default behaviour.
        .gzip(true)
        .brotli(true)
        .deflate(true)
        .build()
        .expect("failed to build HTTP client")
}

/// Header list helper that keeps insertion order readable.
///
/// Values are trimmed. A cookie or API key pasted into Settings routinely
/// arrives with a trailing newline, which `HeaderValue` rejects outright — the
/// header would then be dropped and the request would fail with an error that
/// never mentioned whitespace. Trimming cannot change the meaning of a header
/// that was valid to begin with, since HTTP strips surrounding whitespace
/// anyway. An interior CR/LF is still refused: that is injection, not a paste
/// accident.
pub fn headers(pairs: &[(&str, &str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (k, v) in pairs {
        let v = v.trim();
        if v.is_empty() {
            continue;
        }
        match (
            HeaderName::from_bytes(k.as_bytes()),
            HeaderValue::from_str(v),
        ) {
            (Ok(name), Ok(value)) => {
                map.insert(name, value);
            }
            // Dropping is still the only option here, but it must not be
            // silent: the service will fail later for an unrelated-looking
            // reason, and this line is the only trace of why.
            _ => log::warn!("dropping HTTP header {k:?} whose value cannot be sent"),
        }
    }
    map
}

/// Youdao-style default headers.
pub fn youdao_headers() -> HeaderMap {
    headers(&[
        ("User-Agent", USER_AGENT_ANDROID),
        ("Referer", "https://fanyi.youdao.com"),
        ("Cookie", "OUTFOX_SEARCH_USER_ID=1796239350@10.110.96.157;"),
    ])
}

/// Google-style default headers.
pub fn google_headers() -> HeaderMap {
    headers(&[("User-Agent", USER_AGENT_MAC_GOOGLE)])
}

/// Reads a response body as text, decoding with the charset declared in the
/// `Content-Type` header when present.
pub async fn text(response: Response) -> Result<String, QueryError> {
    let status = response.status();
    // `Response::text` is the charset-aware decode — the `charset` feature
    // exists for it, and `URLSession` picks the body's encoding the same way.
    // `from_utf8_lossy` renders a GBK page as replacement characters, which the
    // callers then read as a page that is missing its values.
    let body = response
        .text()
        .await
        .map_err(|e| QueryError::with_message(ErrorType::Api, e.to_string()))?;
    if !status.is_success() {
        return Err(QueryError::api_http(status.as_u16(), truncate(&body, 64 * 1024)));
    }
    Ok(body)
}

/// Reads a response body as JSON, mapping failures to `QueryError`.
pub async fn json<T: serde::de::DeserializeOwned>(response: Response) -> Result<T, QueryError> {
    let status = response.status();
    let bytes = response
        .bytes()
        .await
        .map_err(|e| QueryError::with_message(ErrorType::Api, e.to_string()))?;
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes).to_string();
        return Err(QueryError::api_http(status.as_u16(), truncate(&body, 64 * 1024)));
    }
    serde_json::from_slice(&bytes).map_err(|e| {
        QueryError::with_data(
            ErrorType::Api,
            Some("Failed to parse response".to_string()),
            format!("{e}: {}", truncate(&String::from_utf8_lossy(&bytes), 2048)),
        )
    })
}

/// Reads a response body as raw bytes, mapping non-2xx to `QueryError`.
pub async fn bytes(response: Response) -> Result<Vec<u8>, QueryError> {
    let status = response.status();
    let data = response
        .bytes()
        .await
        .map_err(|e| QueryError::with_message(ErrorType::Api, e.to_string()))?;
    if !status.is_success() {
        let body = String::from_utf8_lossy(&data).to_string();
        return Err(QueryError::api_http(status.as_u16(), truncate(&body, 64 * 1024)));
    }
    Ok(data.to_vec())
}

/// Converts a `reqwest::Error` into a `QueryError`, mapping timeouts and
/// cancellations the same way the macOS app maps `NSURLError*`.
pub fn map_reqwest_error(err: reqwest::Error) -> QueryError {
    if err.is_timeout() {
        return QueryError::new(ErrorType::Timeout);
    }
    if err.is_decode() {
        return QueryError::with_message(ErrorType::Api, err.to_string());
    }
    QueryError::with_message(ErrorType::Api, err.to_string())
}

/// Truncates a string to `n` bytes on a char boundary.
///
/// `String.truncated(_:)` / `NSString.truncated(_:)` take a plain prefix and
/// add no ellipsis, so neither does this.
pub fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        return s.to_string();
    }
    let mut end = n;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

/// Common header constants re-exported for service modules.
pub use reqwest::header::AUTHORIZATION;
pub const _ACCEPT: reqwest::header::HeaderName = ACCEPT;
pub const _CONTENT_TYPE: reqwest::header::HeaderName = CONTENT_TYPE;
pub const _USER_AGENT: reqwest::header::HeaderName = USER_AGENT;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "你好世界";
        // 4 bytes is one 3-byte character, not one character plus a partial one.
        assert_eq!(truncate(s, 4), "你");
        assert_eq!(truncate(s, 20), s);
    }

    #[test]
    fn headers_skip_empty_values() {
        let h = headers(&[("X-A", "1"), ("X-B", "")]);
        assert!(h.contains_key("x-a"));
        assert!(!h.contains_key("x-b"));
    }

    #[test]
    fn headers_survive_a_pasted_trailing_newline() {
        // Whitespace around the value comes free with a copy-paste out of
        // DevTools or Notepad, and used to take the whole header with it.
        let h = headers(&[
            ("Cookie", "  a=b; c=d\n"),
            ("Authorization", "Bearer k\n"),
            ("X-C", "  "),
        ]);
        assert_eq!(h["cookie"], "a=b; c=d");
        assert_eq!(h["authorization"], "Bearer k");
        assert!(!h.contains_key("x-c"));

        // An interior newline is still refused — that is injection, and it is
        // not something trimming is meant to paper over.
        let h = headers(&[("Cookie", "a=b\nInjected: 1")]);
        assert!(h.is_empty());
    }

    /// Serves one request on a loopback socket, writing three body bytes per
    /// chunk with `gap` of silence before each.
    fn dribbling_server(chunks: usize, gap: Duration) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            let mut scratch = [0u8; 1024];
            let _ = socket.read(&mut scratch);
            let _ = socket.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    chunks * 3
                )
                .as_bytes(),
            );
            for _ in 0..chunks {
                std::thread::sleep(gap);
                if socket.write_all(b"abc").is_err() {
                    return; // the client gave up and closed the socket
                }
            }
            let _ = socket.flush();
        });
        (format!("http://{addr}/"), handle)
    }

    #[tokio::test]
    async fn the_timeout_is_idle_not_total() {
        // Four 250 ms gaps make the exchange outlast an 800 ms deadline, yet no
        // single gap comes close to it. That is the whole difference: upstream
        // measures the silence between bytes, so a long LLM stream that keeps
        // arriving is never cut off — a total timeout would abort this.
        let (url, server) = dribbling_server(4, Duration::from_millis(250));
        let client = build_client(Duration::from_millis(800));
        let body = client
            .get(&url)
            .send()
            .await
            .unwrap()
            .text()
            .await
            .expect("the reply was cut off as a whole-exchange deadline");
        assert_eq!(body, "abcabcabcabc");
        server.join().unwrap();
    }

    #[tokio::test]
    async fn a_stall_longer_than_the_timeout_still_fails() {
        // The guard on the test above: an idle timeout has to still be a
        // timeout, so this cannot pass by no timeout being applied at all.
        let (url, server) = dribbling_server(1, Duration::from_millis(1000));
        let client = build_client(Duration::from_millis(250));
        let response = client.get(&url).send().await.unwrap();
        let err = response.text().await.unwrap_err();
        assert!(err.is_timeout(), "{err}");
        let _ = server.join();
    }

    /// A response built without touching the network: `reqwest::Response` is
    /// `From` an `http::Response`, which `tauri` re-exports.
    fn response(status: u16, content_type: Option<&str>, body: &[u8]) -> Response {
        let mut raw = tauri::http::Response::builder().status(status);
        if let Some(ct) = content_type {
            raw = raw.header("Content-Type", ct);
        }
        Response::from(raw.body(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn text_decodes_the_charset_from_content_type() {
        // "你好" in GBK, as an HTML page from a Chinese provider would send it.
        let body = [0xC4u8, 0xE3, 0xBA, 0xC3];
        let decoded = text(response(200, Some("text/html; charset=gbk"), &body))
            .await
            .unwrap();
        assert_eq!(decoded, "你好");

        // The error body the UI shows comes from the same decode.
        let error = text(response(500, Some("text/plain; charset=gbk"), &body))
            .await
            .unwrap_err();
        assert_eq!(error.error_data_message.as_deref(), Some("你好"));
    }
}
