//! Optical character recognition.
//!
//! `Windows.Media.Ocr` takes the place of the macOS Vision `VNRecognizeTextRequest`
//! that `AppleOCREngine` drives. Windows picks the recogniser language the same
//! way Vision is asked to when upstream enables `automaticallyDetectsLanguage`:
//! from the user's profile languages.
//!
//! Windows-only: declare it as `#[cfg(windows)] mod ocr;` in the crate root.
#![cfg(windows)]

use std::collections::HashMap;
use std::sync::Arc;

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use windows::core::HSTRING;
use windows::Graphics::Imaging::{BitmapDecoder, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr as win_ocr;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::model::Language;

#[derive(Debug, thiserror::Error)]
pub enum OcrError {
    #[error("no OCR language pack is installed for this system")]
    NoLanguagePack,
    #[error("nothing was recognised")]
    NoResult,
    #[error("platform error: {0}")]
    Platform(String),
    #[error("image error: {0}")]
    Image(String),
}

/// One recognised line. `bounding_box` is `[x, y, width, height]` in pixels,
/// with the origin at the image's top-left corner.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub bounding_box: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct OcrOutput {
    pub text: String,
    pub lines: Vec<OcrLine>,
    pub language: Option<Language>,
    pub confidence: Option<f32>,
}

/// Recognises text in images, caching one recogniser per language tag.
///
/// The WinRT engines themselves live in the process-wide `ENGINES` cache, so
/// every handle and every thread shares them.
pub struct OcrEngine {
    /// Language tag of the auto-detected recogniser, recorded by the first
    /// recognition. `Arc` because the blocking task that resolves it outlives
    /// this borrow.
    default_tag: Arc<Mutex<Option<String>>>,
}

/// WinRT OCR engines are expensive to create and agile, so one is kept per
/// language tag for the whole process instead of being rebuilt per call.
static ENGINES: Lazy<Mutex<HashMap<String, win_ocr::OcrEngine>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

impl OcrEngine {
    pub fn new() -> Self {
        OcrEngine {
            default_tag: Arc::new(Mutex::new(None)),
        }
    }

    /// Recognises PNG or JPEG bytes.
    ///
    /// Every WinRT call runs inside `spawn_blocking`: `BitmapDecoder` and
    /// `RecognizeAsync` are awaited by blocking on their `IAsyncOperation`, and
    /// the thread is COM-initialised first. The method itself stays `async`.
    pub async fn recognize_image(&self, bytes: &[u8]) -> Result<OcrOutput, OcrError> {
        let encoded = bytes.to_vec();
        let default_tag = Arc::clone(&self.default_tag);
        tokio::task::spawn_blocking(move || {
            ensure_com();
            let tag = resolve_default_tag(&default_tag)?;
            let engine = engine_for(&tag)?;
            recognize(&engine, &encoded, &tag)
        })
        .await
        .map_err(|error| OcrError::Platform(format!("the OCR task failed: {error}")))?
    }

    /// Recognises raw 8-bit RGBA pixels — what the screenshot path returns.
    ///
    /// The buffer is encoded to PNG in memory first, which is the only way to
    /// hand a `SoftwareBitmap` to `OcrEngine` without the `IBuffer` plumbing of
    /// `Windows.Security.Cryptography`.
    pub async fn recognize_rgba(
        &self,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    ) -> Result<OcrOutput, OcrError> {
        let default_tag = Arc::clone(&self.default_tag);
        tokio::task::spawn_blocking(move || {
            let png = encode_png(width, height, &rgba)?;
            ensure_com();
            let tag = resolve_default_tag(&default_tag)?;
            let engine = engine_for(&tag)?;
            recognize(&engine, &png, &tag)
        })
        .await
        .map_err(|error| OcrError::Platform(format!("the OCR task failed: {error}")))?
    }

    /// Installed recogniser language tags, e.g. `["en-US", "zh-Hans-CN"]`.
    ///
    /// Runs on its own thread so that the caller's COM apartment — the UI
    /// thread is STA for WebView2 — is never re-initialised behind its back.
    pub fn available_languages(&self) -> Vec<String> {
        std::thread::spawn(recognizer_language_tags)
            .join()
            .unwrap_or_default()
    }
}

impl Default for OcrEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Language tags of every installed recogniser.
fn recognizer_language_tags() -> Vec<String> {
    ensure_com();
    let Ok(languages) = win_ocr::OcrEngine::AvailableRecognizerLanguages() else {
        return Vec::new();
    };
    let Ok(count) = languages.Size() else {
        return Vec::new();
    };
    (0..count)
        .filter_map(|index| {
            languages
                .GetAt(index)
                .ok()?
                .LanguageTag()
                .ok()
                .map(|tag| tag.to_string())
        })
        .collect()
}

/// The engine used when the caller has no language in mind: Windows' own
/// auto-detection, which is the counterpart of Vision's
/// `automaticallyDetectsLanguage`.
fn resolve_default_tag(slot: &Mutex<Option<String>>) -> Result<String, OcrError> {
    if let Some(tag) = slot.lock().clone() {
        return Ok(tag);
    }

    let detected = win_ocr::OcrEngine::TryCreateFromUserProfileLanguages()
        .ok()
        .and_then(|engine| engine.RecognizerLanguage().ok())
        .and_then(|language| language.LanguageTag().ok())
        .map(|tag| tag.to_string());

    // No profile language has a pack: fall back to any installed recogniser.
    let tag = match detected.or_else(|| recognizer_language_tags().into_iter().next()) {
        Some(tag) => tag,
        None => return Err(OcrError::NoLanguagePack),
    };

    *slot.lock() = Some(tag.clone());
    Ok(tag)
}

/// The cached engine for a language tag, created on first use.
fn engine_for(tag: &str) -> Result<win_ocr::OcrEngine, OcrError> {
    if let Some(engine) = ENGINES.lock().get(tag) {
        return Ok(engine.clone());
    }

    let language = windows::Globalization::Language::CreateLanguage(&HSTRING::from(tag))
        .map_err(|error| OcrError::Platform(format!("\"{tag}\" is not a language tag: {error}")))?;
    if !win_ocr::OcrEngine::IsLanguageSupported(&language).unwrap_or(false) {
        return Err(OcrError::NoLanguagePack);
    }
    let engine =
        win_ocr::OcrEngine::TryCreateFromLanguage(&language).map_err(|_| OcrError::NoLanguagePack)?;

    ENGINES.lock().insert(tag.to_string(), engine.clone());
    Ok(engine)
}

/// Decode `encoded` and recognise it. Blocking; runs on the OCR task's thread.
fn recognize(
    engine: &win_ocr::OcrEngine,
    encoded: &[u8],
    tag: &str,
) -> Result<OcrOutput, OcrError> {
    let stream = InMemoryRandomAccessStream::new().map_err(platform)?;
    write_bytes(&stream, encoded)?;
    // `BitmapDecoder` reads from the stream's current position, which `DataWriter`
    // left at the end.
    stream.Seek(0).map_err(platform)?;

    let decoder = BitmapDecoder::CreateAsync(&stream)
        .and_then(|operation| operation.join())
        .map_err(image_error)?;
    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .and_then(|operation| operation.join())
        .map_err(image_error)?;
    // The decoder keeps whatever format the file used; OCR wants a canonical one.
    let bitmap = SoftwareBitmap::Convert(&bitmap, BitmapPixelFormat::Bgra8).map_err(image_error)?;

    let width = bitmap.PixelWidth().map_err(image_error)?;
    let height = bitmap.PixelHeight().map_err(image_error)?;
    let max = win_ocr::OcrEngine::MaxImageDimension().unwrap_or(0);
    if max > 0 && (width > max as i32 || height > max as i32) {
        return Err(OcrError::Image(format!(
            "the image is {width}x{height}, larger than the OCR limit of {max}px"
        )));
    }

    let result = engine
        .RecognizeAsync(&bitmap)
        .and_then(|operation| operation.join())
        .map_err(platform)?;

    into_output(&result, tag)
}

/// Copies the bytes into a stream, leaving the writer detached so the stream
/// outlives it.
fn write_bytes(stream: &InMemoryRandomAccessStream, bytes: &[u8]) -> Result<(), OcrError> {
    let output = stream.GetOutputStreamAt(0).map_err(platform)?;
    let writer = DataWriter::CreateDataWriter(&output).map_err(platform)?;
    writer.WriteBytes(bytes).map_err(platform)?;
    writer
        .StoreAsync()
        .and_then(|operation| operation.join())
        .map_err(platform)?;
    writer
        .FlushAsync()
        .and_then(|operation| operation.join())
        .map_err(platform)?;
    // Without detaching, dropping the writer would close the stream under the
    // decoder.
    writer.DetachStream().map_err(platform)?;
    Ok(())
}

fn into_output(result: &win_ocr::OcrResult, tag: &str) -> Result<OcrOutput, OcrError> {
    let text = result.Text().map_err(platform)?.to_string();

    let mut lines = Vec::new();
    let line_views = result.Lines().map_err(platform)?;
    let count = line_views.Size().map_err(platform)?;
    for index in 0..count {
        let Ok(line) = line_views.GetAt(index) else {
            continue;
        };
        let Ok(line_text) = line.Text() else {
            continue;
        };
        let line_text = line_text.to_string();
        if line_text.trim().is_empty() {
            continue;
        }
        lines.push(OcrLine {
            text: line_text,
            bounding_box: line_bounding_box(&line),
        });
    }

    if text.trim().is_empty() && lines.is_empty() {
        return Err(OcrError::NoResult);
    }

    Ok(OcrOutput {
        text,
        lines,
        language: language_of(tag),
        // `Windows.Media.Ocr` reports no confidence value at all, so Vision's
        // `VNConfidence` has nothing to map onto.
        confidence: None,
    })
}

/// The union of a line's word rectangles, in pixels.
fn line_bounding_box(line: &win_ocr::OcrLine) -> [f32; 4] {
    let Ok(words) = line.Words() else {
        return [0.0; 4];
    };
    let Ok(count) = words.Size() else {
        return [0.0; 4];
    };

    let mut box_ = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
    let mut any = false;
    for index in 0..count {
        let Ok(word) = words.GetAt(index) else {
            continue;
        };
        let Ok(rect) = word.BoundingRect() else {
            continue;
        };
        box_[0] = box_[0].min(rect.X);
        box_[1] = box_[1].min(rect.Y);
        box_[2] = box_[2].max(rect.X + rect.Width);
        box_[3] = box_[3].max(rect.Y + rect.Height);
        any = true;
    }

    if !any {
        return [0.0; 4];
    }
    [box_[0], box_[1], box_[2] - box_[0], box_[3] - box_[1]]
}

fn language_of(tag: &str) -> Option<Language> {
    match Language::from_code(tag) {
        Language::Auto => None,
        language => Some(language),
    }
}

/// Encodes RGBA pixels as PNG.
fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Result<Vec<u8>, OcrError> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4));
    if width == 0 || height == 0 || expected != Some(rgba.len()) {
        return Err(OcrError::Image(format!(
            "a {width}x{height} RGBA image needs {} bytes, got {}",
            expected.unwrap_or(0),
            rgba.len()
        )));
    }

    let image = image::RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| OcrError::Image("the pixel buffer does not match the image size".into()))?;
    let mut png = std::io::Cursor::new(Vec::<u8>::new());
    image
        .write_to(&mut png, image::ImageFormat::Png)
        .map_err(|error| OcrError::Image(format!("could not encode the image as PNG: {error}")))?;
    Ok(png.into_inner())
}

/// WinRT needs an initialised apartment. `RPC_E_CHANGED_MODE` simply means the
/// thread already belongs to another apartment, which is fine for WinRT, so the
/// result is ignored and the thread is never uninitialised — it usually belongs
/// to tokio and outlives this call.
fn ensure_com() {
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
}

fn platform(error: windows::core::Error) -> OcrError {
    OcrError::Platform(error.to_string())
}

fn image_error(error: windows::core::Error) -> OcrError {
    OcrError::Image(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_must_match_the_declared_size() {
        assert!(encode_png(0, 4, &[]).is_err());
        assert!(encode_png(4, 0, &[]).is_err());
        assert!(encode_png(2, 2, &[0; 4]).is_err());
        assert!(encode_png(2, 2, &[0; 16]).is_ok());
    }

    #[test]
    fn rgba_becomes_a_png() {
        let png = encode_png(2, 2, &[255; 16]).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
    }

    #[test]
    fn language_tags_map_back_to_languages() {
        assert_eq!(language_of("en-US"), Some(Language::English));
        assert_eq!(language_of("zh-Hans-CN"), Some(Language::SimplifiedChinese));
        assert_eq!(language_of(""), None);
        assert_eq!(language_of("xx-YY"), None);
    }
}
