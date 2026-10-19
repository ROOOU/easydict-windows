//! End-to-end transport test: a real HTTP round trip through the streaming and
//! non-streaming pipelines, against a server on localhost.
//!
//! The unit tests in `service::llm` feed fixture *strings* to the parsers. This
//! drives the whole path instead — reqwest, the request headers, the transport
//! switch, the content-type gate, the SSE reader and result assembly — which is
//! the part that can only fail at runtime.
//!
//! Everything stays on the loopback interface: no request leaves the machine.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

use app_lib::config::Settings;
use app_lib::model::{ErrorType, QueryModel, ServiceType};
use app_lib::service::llm::claude::ClaudeService;
use app_lib::service::llm::deepseek::DeepSeekService;
use app_lib::service::llm::openai::CustomOpenAiService;
use app_lib::service::{QueryService, ServiceContext};

/// One OpenAI-compatible streaming delta, with every field the port's
/// `ChatStreamChunk` requires.
fn chunk(content: &str) -> String {
    format!(
        r#"{{"id":"1","object":"chat.completion.chunk","created":0,"model":"test-model","choices":[{{"index":0,"delta":{{"content":"{content}"}},"finish_reason":null}}]}}"#
    )
}

/// One OpenAI-compatible non-streaming completion, with every field a real
/// server sends (`ChatResult` reads only `choices[].message.content`).
fn completion(content: &str) -> String {
    format!(
        r#"{{"id":"1","object":"chat.completion","created":0,"model":"test-model","choices":[{{"index":0,"message":{{"role":"assistant","content":"{content}"}},"finish_reason":"stop"}}],"usage":{{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}}}"#
    )
}

fn header_end(raw: &[u8]) -> Option<usize> {
    raw.windows(4).position(|w| w == b"\r\n\r\n")
}

fn content_length(head: &str) -> usize {
    head.lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse().ok())?
        })
        .unwrap_or(0)
}

/// Serves exactly one request, then returns the body it was given.
///
/// Returns the port to point the service at, and a slot holding the raw request
/// once it has been read — so the assertions can cover the request side of the
/// round trip as well as the response side.
fn spawn_server(content_type: &str, body: &str) -> (u16, Arc<Mutex<String>>) {
    spawn_response(200, content_type, body)
}

/// [`spawn_server`] with an arbitrary status code, for the error paths.
fn spawn_response(status: u16, content_type: &str, body: &str) -> (u16, Arc<Mutex<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();
    let captured = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&captured);

    let content_type = content_type.to_string();
    let body = body.to_string();

    std::thread::spawn(move || {
        let Ok((mut stream, _)) = listener.accept() else {
            return;
        };

        let mut raw = Vec::new();
        let mut buffer = [0u8; 4096];
        let mut expected = None;

        loop {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => raw.extend_from_slice(&buffer[..read]),
            }
            if expected.is_none() {
                if let Some(end) = header_end(&raw) {
                    let head = String::from_utf8_lossy(&raw[..end]).to_string();
                    expected = Some(end + 4 + content_length(&head));
                }
            }
            if let Some(total) = expected {
                if raw.len() >= total {
                    break;
                }
            }
        }

        *sink.lock().expect("request slot") = String::from_utf8_lossy(&raw).to_string();

        let reason = match status {
            200 => "OK",
            401 => "Unauthorized",
            500 => "Internal Server Error",
            _ => "Mock",
        };
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
        let _ = stream.flush();
    });

    (port, captured)
}

/// A `CustomOpenAI` instance pointed at `port`, which is how the user-supplied
/// endpoint reaches the transport.
fn service_on(port: u16) -> CustomOpenAiService {
    service_with_streaming(port, true)
}

/// As [`service_on`], with the settings toggle that picks the transport.
fn service_with_streaming(port: u16, streaming: bool) -> CustomOpenAiService {
    let mut settings = Settings::default();
    let config = settings
        .services
        .entry(ServiceType::CustomOpenAI.raw().to_string())
        .or_default();
    config.api_key = "test-key".to_string();
    config.model = "test-model".to_string();
    config.endpoint = format!("http://127.0.0.1:{port}/v1/chat/completions");
    config.enable_streaming = streaming;

    CustomOpenAiService::new(ServiceContext::new(
        Arc::new(settings),
        ServiceType::CustomOpenAI,
        ServiceType::CustomOpenAI.raw(),
    ))
}

/// A `Claude` instance pointed at `port`, with the transport toggle set.
fn claude_on(port: u16, streaming: bool) -> ClaudeService {
    let mut settings = Settings::default();
    let config = settings
        .services
        .entry(ServiceType::Claude.raw().to_string())
        .or_default();
    config.api_key = "test-key".to_string();
    config.model = "claude-sonnet-4-6".to_string();
    config.endpoint = format!("http://127.0.0.1:{port}/v1/messages");
    config.enable_streaming = streaming;

    ClaudeService::new(ServiceContext::new(
        Arc::new(settings),
        ServiceType::Claude,
        ServiceType::Claude.raw(),
    ))
}

/// A `DeepSeek` instance pointed at `port`, with the transport toggle set.
fn deepseek_on(port: u16, streaming: bool) -> DeepSeekService {
    let mut settings = Settings::default();
    let config = settings
        .services
        .entry(ServiceType::DeepSeek.raw().to_string())
        .or_default();
    config.api_key = "test-key".to_string();
    config.model = "deepseek-v4-flash".to_string();
    config.endpoint = format!("http://127.0.0.1:{port}/v1/chat/completions");
    config.enable_streaming = streaming;

    DeepSeekService::new(ServiceContext::new(
        Arc::new(settings),
        ServiceType::DeepSeek,
        ServiceType::DeepSeek.raw(),
    ))
}

/// The raw request the mock server received.
fn request_body(captured: &Arc<Mutex<String>>) -> String {
    captured.lock().expect("request slot").clone()
}

#[tokio::test]
async fn a_query_round_trips_through_the_streaming_transport() {
    let body = format!("data: {}\n\ndata: [DONE]\n\n", chunk("你好，世界"));
    let (port, captured) = spawn_server("text/event-stream", &body);

    let result = service_on(port)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect("the stream should complete");

    assert_eq!(result.translated_text().as_deref(), Some("你好，世界"));

    // The request half matters too: a wrong path or a missing credential would
    // still let the assertions above pass against a permissive mock.
    let request = captured.lock().expect("request slot").clone();
    assert!(
        request.starts_with("POST /v1/chat/completions HTTP/1.1"),
        "{request}"
    );
    let lower = request.to_lowercase();
    assert!(lower.contains("authorization: bearer test-key"), "{request}");
    assert!(request.contains("\"model\":\"test-model\""), "{request}");
    assert!(request.contains("\"stream\":true"), "{request}");
    // The prompt the user typed reaches the wire.
    assert!(request.contains("hello"), "{request}");
}

#[tokio::test]
async fn deltas_arriving_in_separate_writes_are_accumulated() {
    // The same content, delivered as three events rather than one — which is
    // what a real server does and what the incremental reader has to survive.
    let body = format!(
        "data: {}\n\ndata: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        chunk("你好"),
        chunk("，"),
        chunk("世界")
    );
    let (port, _) = spawn_server("text/event-stream", &body);

    let result = service_on(port)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect("the stream should complete");

    assert_eq!(result.translated_text().as_deref(), Some("你好，世界"));
}

#[tokio::test]
async fn a_non_sse_response_is_reported_as_a_content_type_mismatch() {
    // A proxy or a wrong endpoint answering JSON must be named as such rather
    // than surfacing as an empty translation.
    let (port, _) = spawn_server("application/json", r#"{"error":"nope"}"#);

    let error = service_on(port)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect_err("JSON is not an acceptable stream");

    assert_eq!(error.kind, ErrorType::ContentTypeMismatch);
    assert!(
        error.message.as_deref().unwrap_or("").contains("text/event-stream"),
        "{:?}",
        error.message
    );
}

#[tokio::test]
async fn a_non_streaming_request_sends_stream_false_and_returns_the_content() {
    // `nonStreamingTranslate`: the same chat query with `stream: false`, a JSON
    // answer, and `choices[0].message.content` taken from it.
    let (port, captured) = spawn_server("application/json", &completion("你好，世界"));

    let result = service_with_streaming(port, false)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect("the completion should decode");

    assert_eq!(result.translated_text().as_deref(), Some("你好，世界"));

    let request = request_body(&captured);
    assert!(
        request.starts_with("POST /v1/chat/completions HTTP/1.1"),
        "{request}"
    );
    let lower = request.to_lowercase();
    assert!(lower.contains("authorization: bearer test-key"), "{request}");
    assert!(request.contains("\"model\":\"test-model\""), "{request}");
    // The point of the whole test: no `stream: true` went out.
    assert!(request.contains("\"stream\":false"), "{request}");
    assert!(!request.contains("\"stream\":true"), "{request}");
    assert!(request.contains("hello"), "{request}");
}

#[tokio::test]
async fn the_streaming_toggle_selects_the_transport() {
    // Toggle on: the request asks for SSE and the reply is framed as one. If
    // the toggle were ignored and the JSON decoder were used instead, the SSE
    // body would not decode at all.
    let body = format!("data: {}\n\ndata: [DONE]\n\n", chunk("流式"));
    let (port, captured) = spawn_server("text/event-stream", &body);
    let result = service_with_streaming(port, true)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect("the stream should complete");
    assert_eq!(result.translated_text().as_deref(), Some("流式"));
    assert!(
        request_body(&captured).contains("\"stream\":true"),
        "{}",
        request_body(&captured)
    );

    // Toggle off: the same service, the same endpoint, a JSON completion. Were
    // the toggle ignored, the content-type gate would reject this body as not
    // being `text/event-stream`.
    let (port, captured) = spawn_server("application/json", &completion("非流式"));
    let result = service_with_streaming(port, false)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect("the completion should decode");
    assert_eq!(result.translated_text().as_deref(), Some("非流式"));
    assert!(
        request_body(&captured).contains("\"stream\":false"),
        "{}",
        request_body(&captured)
    );
}

#[tokio::test]
async fn a_non_streaming_error_body_prefers_the_api_envelope() {
    // A decodable `{"error": {…}}` wins over the raw body ...
    let (port, _) = spawn_response(
        401,
        "application/json",
        r#"{"error":{"message":"bad key","type":"invalid_request_error"}}"#,
    );
    let error = service_with_streaming(port, false)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect_err("401 is a failure");
    assert_eq!(error.kind, ErrorType::Api);
    assert_eq!(error.error_data_message.as_deref(), Some("bad key"));
    // The status-only message did not win.
    assert_eq!(error.message, None);

    // ... and when it is not decodable, the status and the raw body are kept.
    let (port, _) = spawn_response(500, "text/plain", "upstream exploded");
    let error = service_with_streaming(port, false)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect_err("500 is a failure");
    assert_eq!(error.kind, ErrorType::Api);
    assert_eq!(error.message.as_deref(), Some("HTTP 500"));
    assert_eq!(error.error_data_message.as_deref(), Some("upstream exploded"));
}

#[tokio::test]
async fn a_non_streaming_answer_without_content_is_no_result() {
    // No choices at all.
    let (port, _) = spawn_server("application/json", r#"{"choices":[]}"#);
    let error = service_with_streaming(port, false)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect_err("an empty answer is not a translation");
    assert_eq!(error.kind, ErrorType::NoResult);

    // A choice whose content is present but empty is the same failure.
    let (port, _) = spawn_server("application/json", &completion(""));
    let error = service_with_streaming(port, false)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect_err("an empty answer is not a translation");
    assert_eq!(error.kind, ErrorType::NoResult);
}

#[tokio::test]
async fn claude_non_streaming_reads_its_own_content_blocks() {
    // Claude's non-streaming answer is not `choices[0].message.content`: the
    // text lives in content blocks, and a non-`text` block carries none.
    let body = r#"{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-6","content":[{"type":"thinking","thinking":"thinking…"},{"type":"text","text":"你好"},{"type":"text","text":"，世界"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}"#;
    let (port, captured) = spawn_server("application/json", body);

    let result = claude_on(port, false)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect("the message response should decode");

    assert_eq!(result.translated_text().as_deref(), Some("你好，世界"));

    let request = request_body(&captured);
    assert!(request.starts_with("POST /v1/messages HTTP/1.1"), "{request}");
    let lower = request.to_lowercase();
    assert!(lower.contains("x-api-key: test-key"), "{request}");
    assert!(lower.contains("anthropic-version: 2023-06-01"), "{request}");
    // Claude has no bearer token; the shared headers must not leak in.
    assert!(!lower.contains("authorization: bearer"), "{request}");
    assert!(request.contains("\"stream\":false"), "{request}");
}

#[tokio::test]
async fn deepseek_non_streaming_uses_the_chat_completions_shape() {
    // DeepSeek's non-streaming answer is the ordinary chat-completions shape,
    // so it decodes through the shared OpenAI-compatible result.
    let (port, captured) = spawn_server("application/json", &completion("你好"));

    let result = deepseek_on(port, false)
        .translate(&QueryModel::new("hello"), None)
        .await
        .expect("the completion should decode");

    assert_eq!(result.translated_text().as_deref(), Some("你好"));

    let request = request_body(&captured);
    assert!(
        request.starts_with("POST /v1/chat/completions HTTP/1.1"),
        "{request}"
    );
    let lower = request.to_lowercase();
    assert!(lower.contains("authorization: bearer test-key"), "{request}");
    assert!(request.contains("\"stream\":false"), "{request}");
    // DeepSeek's own reasoning fields ride along on either transport.
    assert!(request.contains("\"thinking\":{\"type\":\"disabled\"}"), "{request}");
}
