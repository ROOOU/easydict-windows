#[cfg(target_os = "windows")]
use windows::{
    Graphics::Imaging::BitmapDecoder,
    Media::Ocr::OcrEngine,
    Storage::Streams::{DataWriter, InMemoryRandomAccessStream},
    Globalization::Language,
};

/// Perform OCR on a PNG image buffer using Windows OCR API.
#[cfg(target_os = "windows")]
pub fn ocr_from_png_bytes(png_bytes: &[u8], lang: &str) -> Result<String, String> {
    // Write PNG bytes to in-memory stream
    let stream = InMemoryRandomAccessStream::new().map_err(|e| format!("Stream error: {}", e))?;
    let writer = DataWriter::CreateDataWriter(&stream).map_err(|e| format!("Writer error: {}", e))?;
    writer.WriteBytes(png_bytes).map_err(|e| format!("Write error: {}", e))?;
    writer.StoreAsync().map_err(|e| format!("Store error: {}", e))?
        .get().map_err(|e| format!("Store get error: {}", e))?;
    writer.FlushAsync().map_err(|e| format!("Flush error: {}", e))?
        .get().map_err(|e| format!("Flush get error: {}", e))?;
    writer.DetachStream().map_err(|e| format!("Detach error: {}", e))?;

    // Reset stream position to beginning
    stream.Seek(0).map_err(|e| format!("Seek error: {}", e))?;

    // Decode image
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|e| format!("Decoder create error: {}", e))?
        .get()
        .map_err(|e| format!("Decoder get error: {}", e))?;

    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|e| format!("Bitmap async error: {}", e))?
        .get()
        .map_err(|e| format!("Bitmap get error: {}", e))?;

    // Create OCR engine
    let engine = if lang.is_empty() || lang == "auto" {
        OcrEngine::TryCreateFromUserProfileLanguages()
            .map_err(|e| format!("OCR 引擎创建失败: {}。请在 Windows 设置 > 时间和语言 > 语言 中安装 OCR 语言包。", e))?
    } else {
        let language = Language::CreateLanguage(&windows::core::HSTRING::from(lang))
            .map_err(|e| format!("Language error: {}", e))?;
        if !OcrEngine::IsLanguageSupported(&language).unwrap_or(false) {
            return Err(format!("不支持的 OCR 语言: {}。请在 Windows 设置中安装对应语言包。", lang));
        }
        OcrEngine::TryCreateFromLanguage(&language)
            .map_err(|e| format!("OCR engine lang error: {}", e))?
    };

    // Recognize
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("Recognize async error: {}", e))?
        .get()
        .map_err(|e| format!("Recognize get error: {}", e))?;

    let text = result
        .Text()
        .map_err(|e| format!("Text error: {}", e))?
        .to_string();

    if text.trim().is_empty() {
        return Err("未识别到文字。请确保截图中有清晰的文字内容。".to_string());
    }

    Ok(text)
}

#[cfg(not(target_os = "windows"))]
pub fn ocr_from_png_bytes(_png_bytes: &[u8], _lang: &str) -> Result<String, String> {
    Err("OCR is only supported on Windows".to_string())
}

/// Capture a screenshot of the entire primary screen and return PNG bytes
pub fn capture_screen() -> Result<Vec<u8>, String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("Monitor error: {}", e))?;
    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary())
        .or_else(|| xcap::Monitor::all().ok()?.into_iter().next())
        .ok_or_else(|| "No monitor found".to_string())?;

    let image = monitor
        .capture_image()
        .map_err(|e| format!("Capture error: {}", e))?;

    let mut png_bytes: Vec<u8> = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
    image::ImageEncoder::write_image(
        encoder,
        image.as_raw(),
        image.width(),
        image.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|e| format!("PNG encode error: {}", e))?;

    Ok(png_bytes)
}
