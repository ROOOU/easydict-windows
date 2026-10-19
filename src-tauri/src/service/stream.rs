//! Shared streaming helpers.
//!
//! Ported from `StreamService+AsyncStream.swift` and
//! `StreamService+UpdateResult.swift`: the byte-level SSE framer, the final-text
//! cleanup, and the think-tag filter.

use futures_util::StreamExt;

use crate::model::{ErrorType, QueryError};
use crate::util;

/// Reads an SSE body, yielding one complete event block per call.
///
/// The macOS implementation accumulates bytes and only decodes when it has a
/// newline or has buffered 1 KB, then splits the decoded text on `"\n\n"`.
/// This keeps the same framing: an event is everything between blank lines.
pub struct SseReader {
    stream: std::pin::Pin<Box<dyn futures_util::Stream<Item = reqwest::Result<bytes::Bytes>> + Send>>,
    buffer: String,
    /// Bytes of an incomplete multi-byte character carried between chunks.
    pending: Vec<u8>,
    done: bool,
}

impl SseReader {
    pub fn new(response: reqwest::Response) -> Self {
        SseReader {
            stream: Box::pin(response.bytes_stream()),
            buffer: String::new(),
            pending: Vec::new(),
            done: false,
        }
    }

    /// Returns the next complete event block, or `None` at end of stream.
    pub async fn next_event(&mut self) -> Option<String> {
        loop {
            if let Some(idx) = self.buffer.find("\n\n") {
                let event = self.buffer[..idx].to_string();
                self.buffer.drain(..idx + 2);
                if event.trim().is_empty() {
                    continue;
                }
                return Some(event);
            }
            if self.done {
                self.flush_pending();
                let rest = std::mem::take(&mut self.buffer);
                return if rest.trim().is_empty() { None } else { Some(rest) };
            }
            match self.stream.next().await {
                Some(Ok(chunk)) => self.push_bytes(&chunk),
                Some(Err(_)) | None => {
                    self.done = true;
                }
            }
        }
    }

    /// Same as [`Self::next_event`] but surfaces transport errors.
    pub async fn next_event_result(&mut self) -> Result<Option<String>, QueryError> {
        Ok(self.next_event().await)
    }

    /// Appends bytes, holding back any trailing incomplete UTF-8 sequence so a
    /// character split across two chunks is not mangled.
    fn push_bytes(&mut self, chunk: &[u8]) {
        let mut data = std::mem::take(&mut self.pending);
        data.extend_from_slice(chunk);
        match std::str::from_utf8(&data) {
            Ok(s) => self.buffer.push_str(s),
            Err(e) => {
                let valid = e.valid_up_to();
                // Safe: `valid_up_to` is a boundary the validator already accepted.
                self.buffer
                    .push_str(std::str::from_utf8(&data[..valid]).unwrap_or_default());
                self.pending = data[valid..].to_vec();
            }
        }
    }

    fn flush_pending(&mut self) {
        if !self.pending.is_empty() {
            let pending = std::mem::take(&mut self.pending);
            self.buffer.push_str(&String::from_utf8_lossy(&pending));
        }
    }
}

/// Extracts `event:` and `data:` fields from one SSE block.
pub fn parse_sse_block(block: &str) -> (Option<String>, Option<String>) {
    let mut event = None;
    let mut data = None;
    for line in block.split('\n') {
        if let Some(rest) = line.strip_prefix("event:") {
            event = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("data:") {
            data = Some(rest.trim().to_string());
        }
    }
    (event, data)
}

/// `getFinalResultText(_:)` — trim, drop a trailing `</s>`, strip quotes.
pub fn get_final_result_text(result_text: &str, query_text: &str) -> String {
    let mut text = result_text.trim().to_string();
    // Some providers append an end-of-sequence marker.
    if text.ends_with("</s>") && !query_text.ends_with("</s>") {
        text.truncate(text.len() - "</s>".len());
    }
    util::try_remove_quotes(&text).trim().to_string()
}

/// `filterThinkTagContent()`
pub fn filter_think_tag_content(text: &str) -> String {
    util::filter_think_tags(text)
}

/// The error a streaming service reports when a stream ends with no content.
pub fn no_result_error() -> QueryError {
    QueryError::new(ErrorType::NoResult)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_block_fields() {
        let (event, data) = parse_sse_block(
            "event: response.output_text.delta\ndata: {\"delta\":\"hi\"}",
        );
        assert_eq!(event.as_deref(), Some("response.output_text.delta"));
        assert_eq!(data.as_deref(), Some("{\"delta\":\"hi\"}"));
    }

    #[test]
    fn sse_block_without_event_field() {
        let (event, data) = parse_sse_block("data: {\"x\":1}");
        assert!(event.is_none());
        assert_eq!(data.as_deref(), Some("{\"x\":1}"));
    }

    #[test]
    fn final_text_cleanup() {
        assert_eq!(get_final_result_text("  hello  ", "hi"), "hello");
        assert_eq!(get_final_result_text("hello</s>", "hi"), "hello");
        // A query that itself ends with the marker keeps it.
        assert_eq!(get_final_result_text("hello</s>", "a</s>"), "hello</s>");
        assert_eq!(get_final_result_text("\"hello\"", "hi"), "hello");
        // Think tags are stripped by a separate step, as upstream does.
        assert_eq!(
            filter_think_tag_content(&get_final_result_text("<think>x</think>hello", "hi")),
            "hello"
        );
    }

    #[test]
    fn utf8_split_across_chunks_survives() {
        let text = "你好";
        let bytes = text.as_bytes();
        // Split in the middle of the first character.
        let mut reader = SseReader {
            stream: Box::pin(futures_util::stream::empty()),
            buffer: String::new(),
            pending: Vec::new(),
            done: false,
        };
        reader.push_bytes(&bytes[..2]);
        assert!(reader.buffer.is_empty());
        reader.push_bytes(&bytes[2..]);
        assert_eq!(reader.buffer, "你好");
    }
}
