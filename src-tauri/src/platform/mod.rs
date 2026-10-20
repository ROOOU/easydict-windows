//! Windows platform services: screen capture and the foreground control's text.
//!
//! These replace the macOS-only pieces of the upstream app: `CGWindowListCreateImage`
//! becomes a GDI `BitBlt`, and reading the selected text — which macOS does with
//! the Accessibility API — goes through UI Automation with a clipboard fallback
//! for controls that publish no text provider (see [`selection`]).
//!
//! Windows-only: declare it as `#[cfg(windows)] mod platform;` in the crate root.
#![cfg(windows)]

mod screenshot;
mod selection;
pub mod auto_selection;
pub mod protocol;
pub mod webview_memory;

pub use screenshot::{ScreenshotSelection, capture_selection};
pub use selection::selected_text;
pub(crate) use selection::{replace_selected_text, selected_text_at, selection_target};

use std::mem::size_of;

use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    ReleaseDC, SelectObject, BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CAPTUREBLT, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ, ROP_CODE, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
    SM_YVIRTUALSCREEN,
};

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("capture failed: {0}")]
    Capture(String),
    #[error("no text is selected")]
    NoSelection,
    #[error("platform error: {0}")]
    Platform(String),
}

/// A screen region in top-down RGBA order.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl CapturedImage {
    pub fn crop(&self, x: u32, y: u32, width: u32, height: u32) -> Result<Self, PlatformError> {
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if width == 0
            || height == 0
            || expected != Some(self.rgba.len())
            || x.checked_add(width).is_none_or(|right| right > self.width)
            || y.checked_add(height).is_none_or(|bottom| bottom > self.height)
        {
            return Err(PlatformError::Capture("the selected region is outside the screenshot".into()));
        }
        let stride = self.width as usize * 4;
        let row_bytes = width as usize * 4;
        let mut rgba = Vec::with_capacity(row_bytes * height as usize);
        for row in y..y + height {
            let start = row as usize * stride + x as usize * 4;
            rgba.extend_from_slice(&self.rgba[start..start + row_bytes]);
        }
        Ok(Self { width, height, rgba })
    }

    pub fn encode_png(&self) -> Result<Vec<u8>, PlatformError> {
        let expected = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|pixels| pixels.checked_mul(4));
        if self.width == 0 || self.height == 0 || expected != Some(self.rgba.len()) {
            return Err(PlatformError::Platform(format!(
                "a {}x{} RGBA image needs {} bytes, got {}",
                self.width,
                self.height,
                expected.unwrap_or(0),
                self.rgba.len()
            )));
        }

        let image = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())
            .ok_or_else(|| {
                PlatformError::Platform("the pixel buffer does not match the image size".into())
            })?;
        let mut png = std::io::Cursor::new(Vec::<u8>::new());
        image
            .write_to(&mut png, image::ImageFormat::Png)
            .map_err(|error| PlatformError::Platform(format!("could not encode PNG: {error}")))?;
        Ok(png.into_inner())
    }
}

/// `(x, y, width, height)` of the virtual screen, spanning every monitor.
///
/// A monitor arranged to the left of or above the primary one makes `x` or `y`
/// negative; that is normal and is what [`capture_region`] expects.
pub fn virtual_screen_bounds() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

/// Captures a screen region with GDI `BitBlt`, the Windows stand-in for
/// `CGWindowListCreateImage`. Coordinates are in virtual-screen space, so the
/// caller can pass what [`virtual_screen_bounds`] reports unchanged.
pub fn capture_region(
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Result<CapturedImage, PlatformError> {
    if width <= 0 || height <= 0 {
        return Err(PlatformError::Capture(format!(
            "the capture region must not be empty ({width}x{height})"
        )));
    }
    let byte_len = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= u32::MAX as usize)
        .ok_or_else(|| {
            PlatformError::Capture(format!("the region {width}x{height} is too large"))
        })?;

    unsafe {
        // The screen DC covers the whole virtual desktop, so negative
        // coordinates (a monitor left of or above the primary) work as-is.
        let screen = ScreenDc(GetDC(None));
        if screen.0.is_invalid() {
            return Err(PlatformError::Capture("GetDC failed".into()));
        }

        let memory = MemoryDc(CreateCompatibleDC(Some(screen.0)));
        if memory.0.is_invalid() {
            return Err(PlatformError::Capture(
                "CreateCompatibleDC failed".into(),
            ));
        }

        let bitmap = OwnedBitmap(CreateCompatibleBitmap(screen.0, width, height));
        if bitmap.0.is_invalid() {
            return Err(PlatformError::Capture(
                "CreateCompatibleBitmap failed".into(),
            ));
        }

        let previous = SelectObject(memory.0, HGDIOBJ(bitmap.0.0));
        let blitted = BitBlt(
            memory.0,
            0,
            0,
            width,
            height,
            Some(screen.0),
            x,
            y,
            // CAPTUREBLT picks up layered windows, which a bare SRCCOPY misses.
            ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0),
        );
        // `GetDIBits` refuses a bitmap that is still selected into a DC.
        if !previous.is_invalid() {
            SelectObject(memory.0, previous);
        }
        blitted.map_err(|error| PlatformError::Capture(format!("BitBlt failed: {error}")))?;

        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        // A negative height asks for a top-down DIB, so the rows arrive in
        // screen order and nothing has to be flipped afterwards.
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;
        info.bmiHeader.biSizeImage = byte_len as u32;

        let mut pixels = vec![0u8; byte_len];
        let copied = GetDIBits(
            memory.0,
            bitmap.0,
            0,
            height as u32,
            Some(pixels.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        if copied != height {
            return Err(PlatformError::Capture(format!(
                "GetDIBits copied {copied} of {height} rows"
            )));
        }

        // A 32-bit BI_RGB DIB is BGRA, and its unused alpha byte comes back as
        // zero, which would make the whole capture transparent.
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }

        Ok(CapturedImage {
            width: width as u32,
            height: height as u32,
            rgba: pixels,
        })
    }
}

/// RAII wrappers so every GDI object is freed on every path out, errors
/// included. The screen DC must be released, not deleted.
struct ScreenDc(HDC);
impl Drop for ScreenDc {
    fn drop(&mut self) {
        unsafe { ReleaseDC(None, self.0) };
    }
}

struct MemoryDc(HDC);
impl Drop for MemoryDc {
    fn drop(&mut self) {
        let _ = unsafe { DeleteDC(self.0) };
    }
}

struct OwnedBitmap(HBITMAP);
impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        let _ = unsafe { DeleteObject(HGDIOBJ(self.0.0)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CapturedImage {
        CapturedImage {
            width: 2,
            height: 2,
            rgba: vec![
                255, 0, 0, 255, // red
                0, 255, 0, 255, // green
                0, 0, 255, 255, // blue
                255, 255, 0, 255, // yellow
            ],
        }
    }

    #[test]
    fn encode_png_round_trips_through_the_image_crate() {
        let captured = sample();
        let png = captured.encode_png().unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));

        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (2, 2));
        assert_eq!(decoded.as_raw(), &captured.rgba);
    }

    #[test]
    fn encode_png_rejects_a_mismatched_buffer() {
        let mut captured = sample();
        captured.rgba.truncate(4);
        assert!(captured.encode_png().is_err());

        let empty = CapturedImage {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
        assert!(empty.encode_png().is_err());
    }

    #[test]
    fn an_empty_or_negative_region_is_an_error() {
        // No screen is touched: the size check comes first.
        assert!(matches!(
            capture_region(0, 0, 0, 100),
            Err(PlatformError::Capture(_))
        ));
        assert!(matches!(
            capture_region(0, 0, 100, 0),
            Err(PlatformError::Capture(_))
        ));
        assert!(matches!(
            capture_region(-10, -10, -5, 10),
            Err(PlatformError::Capture(_))
        ));
    }

    #[test]
    fn cropping_keeps_only_the_selected_pixels_in_screen_order() {
        let selected = sample().crop(1, 0, 1, 2).unwrap();
        assert_eq!(selected.width, 1);
        assert_eq!(selected.height, 2);
        assert_eq!(selected.rgba, vec![0, 255, 0, 255, 255, 255, 0, 255]);
        assert_eq!(sample().crop(0, 0, 2, 2).unwrap(), sample());
    }

    #[test]
    fn cropping_rejects_empty_out_of_bounds_overflow_and_bad_buffers() {
        let captured = sample();
        assert!(captured.crop(0, 0, 0, 1).is_err());
        assert!(captured.crop(0, 0, 1, 0).is_err());
        assert!(captured.crop(1, 1, 2, 2).is_err());
        assert!(captured.crop(u32::MAX, 0, 2, 1).is_err());
        assert!(captured.crop(0, u32::MAX, 1, 2).is_err());
        let mut invalid = sample();
        invalid.rgba.pop();
        assert!(invalid.crop(0, 0, 1, 1).is_err());
    }
}
