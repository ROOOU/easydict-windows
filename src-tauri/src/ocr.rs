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

    // Extract lines with bounding box info to detect paragraph breaks
    let lines = result.Lines().map_err(|e| format!("Lines error: {}", e))?;
    let line_count = lines.Size().map_err(|e| format!("Lines size error: {}", e))? as usize;

    if line_count == 0 {
        let text = result.Text().map_err(|e| format!("Text error: {}", e))?.to_string();
        if text.trim().is_empty() {
            return Err("未识别到文字。请确保截图中有清晰的文字内容。".to_string());
        }
        return Ok(text);
    }

    // Collect line text and vertical positions
    struct LineInfo {
        text: String,
        top: f64,
        height: f64,
    }
    let mut line_infos: Vec<LineInfo> = Vec::with_capacity(line_count);

    for i in 0..line_count {
        let line = lines.GetAt(i as u32).map_err(|e| format!("GetAt error: {}", e))?;
        let text = line.Text().map_err(|e| format!("Line text error: {}", e))?.to_string();

        // Get bounding box from the first word of this line
        let words = line.Words().map_err(|e| format!("Words error: {}", e))?;
        let word_count = words.Size().unwrap_or(0);
        let (top, height) = if word_count > 0 {
            // Use first word's bounding rect for vertical position
            let first_word = words.GetAt(0).map_err(|e| format!("Word error: {}", e))?;
            let rect = first_word.BoundingRect().map_err(|e| format!("Rect error: {}", e))?;
            (rect.Y as f64, rect.Height as f64)
        } else {
            (0.0, 0.0)
        };

        if !text.trim().is_empty() {
            line_infos.push(LineInfo { text, top, height });
        }
    }

    if line_infos.is_empty() {
        return Err("未识别到文字。请确保截图中有清晰的文字内容。".to_string());
    }

    // Build output with paragraph detection based on vertical gaps
    let mut output = String::new();
    for i in 0..line_infos.len() {
        output.push_str(&line_infos[i].text);

        if i + 1 < line_infos.len() {
            let current_bottom = line_infos[i].top + line_infos[i].height;
            let next_top = line_infos[i + 1].top;
            let gap = next_top - current_bottom;
            let line_height = line_infos[i].height.max(line_infos[i + 1].height);

            // If vertical gap > 0.8x line height, treat as paragraph break
            if line_height > 0.0 && gap > line_height * 0.8 {
                output.push_str("\n\n");
            } else {
                output.push('\n');
            }
        }
    }

    Ok(output)
}

#[cfg(not(target_os = "windows"))]
pub fn ocr_from_png_bytes(_png_bytes: &[u8], _lang: &str) -> Result<String, String> {
    Err("OCR is only supported on Windows".to_string())
}

/// Capture a screenshot of the entire primary screen and return raw RGBA bytes + dimensions
pub fn capture_screen() -> Result<(Vec<u8>, u32, u32), String> {
    let monitors = xcap::Monitor::all().map_err(|e| format!("Monitor error: {}", e))?;
    let monitor = monitors
        .into_iter()
        .find(|m| m.is_primary())
        .or_else(|| xcap::Monitor::all().ok()?.into_iter().next())
        .ok_or_else(|| "No monitor found".to_string())?;

    let image = monitor
        .capture_image()
        .map_err(|e| format!("Capture error: {}", e))?;

    let w = image.width();
    let h = image.height();
    let rgba = image.into_raw();
    Ok((rgba, w, h))
}
