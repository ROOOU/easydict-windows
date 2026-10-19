use std::cell::RefCell;
use std::mem::size_of;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DrawTextW, EndPaint, FillRect, FrameRect, GetMonitorInfoW, GetStockObject,
    IntersectClipRect, InvalidateRect, MonitorFromPoint, RestoreDC, SaveDC, SelectObject,
    SetBkMode, SetDIBitsToDevice, SetTextColor, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    BLACK_BRUSH, DEFAULT_GUI_FONT, DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER,
    HBRUSH, HDC, MONITORINFO, MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetForegroundWindow, GetMessageW, GetWindowLongPtrW, IsWindow, LoadCursorW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    CREATESTRUCTW, GWLP_USERDATA, IDC_CROSS, MSG, SW_SHOW, WA_INACTIVE, WM_ACTIVATE,
    WM_CANCELMODE, WM_CAPTURECHANGED, WM_CLOSE, WM_DESTROY, WM_DISPLAYCHANGE, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE,
    WM_NCDESTROY, WM_PAINT, WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::{CapturedImage, PlatformError, capture_region, virtual_screen_bounds};

static SELECTING: AtomicBool = AtomicBool::new(false);
static WINDOW_CLASS: OnceLock<Result<(), String>> = OnceLock::new();

struct SelectionLease<'a>(&'a AtomicBool);

impl<'a> SelectionLease<'a> {
    fn acquire(active: &'a AtomicBool) -> Result<Self, PlatformError> {
        active
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| PlatformError::Capture("已有截图选区正在进行，请先完成或按 Esc 取消".into()))?;
        Ok(Self(active))
    }
}

impl Drop for SelectionLease<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

struct DpiContext(DPI_AWARENESS_CONTEXT);

impl DpiContext {
    fn physical_pixels() -> Result<Self, PlatformError> {
        let previous = unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
        if previous.0.is_null() {
            return Err(PlatformError::Capture("无法启用截图的物理像素坐标".into()));
        }
        Ok(Self(previous))
    }
}

impl Drop for DpiContext {
    fn drop(&mut self) {
        unsafe { SetThreadDpiAwarenessContext(self.0) };
    }
}

struct SelectionWindow(HWND);

impl Drop for SelectionWindow {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(Some(self.0)).as_bool() {
                let _ = DestroyWindow(self.0);
            }
        }
    }
}

struct DesktopSelection {
    bounds: (i32, i32, i32, i32),
    pixels: Vec<u8>,
    dimmed: Vec<u8>,
    anchor: Option<POINT>,
    cursor: POINT,
    selected: Option<RECT>,
    restore_window: bool,
    finished: bool,
}

pub struct ScreenshotSelection {
    pub image: Option<CapturedImage>,
    pub restore_window: bool,
}

pub fn capture_selection() -> Result<ScreenshotSelection, PlatformError> {
    let _lease = SelectionLease::acquire(&SELECTING)?;
    let _dpi = DpiContext::physical_pixels()?;
    let bounds = virtual_screen_bounds();
    let captured = capture_region(bounds.0, bounds.1, bounds.2, bounds.3)?;
    let mut pixels = captured.rgba;
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let mut dimmed = pixels.clone();
    for pixel in dimmed.chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            *channel = (*channel as u16 * 3 / 5) as u8;
        }
    }
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&mut cursor) }.map_err(capture_error)?;
    let selection = Box::new(RefCell::new(DesktopSelection {
        bounds,
        pixels,
        dimmed,
        anchor: None,
        cursor,
        selected: None,
        restore_window: true,
        finished: false,
    }));
    show_selector(&selection)?;
    let mut selection = selection.into_inner();
    let Some(region) = selection.selected else {
        return Ok(ScreenshotSelection { image: None, restore_window: selection.restore_window });
    };
    for pixel in selection.pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let image = CapturedImage {
        width: bounds.2 as u32,
        height: bounds.3 as u32,
        rgba: selection.pixels,
    };
    let image = image.crop(region.left as u32, region.top as u32,
        (region.right - region.left) as u32, (region.bottom - region.top) as u32)?;
    Ok(ScreenshotSelection { image: Some(image), restore_window: true })
}

fn show_selector(selection: &RefCell<DesktopSelection>) -> Result<(), PlatformError> {
    let instance = unsafe { GetModuleHandleW(None) }.map_err(capture_error)?.into();
    let registered = WINDOW_CLASS.get_or_init(|| {
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: unsafe { LoadCursorW(None, IDC_CROSS) }.map_err(|error| error.to_string())?,
            lpszClassName: w!("EasydictScreenshotSelection"),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            return Err(windows::core::Error::from_thread().to_string());
        }
        Ok(())
    });
    registered.as_ref().map_err(|message| PlatformError::Capture(message.clone()))?;
    let (left, top, width, height) = selection.borrow().bounds;
    let previous_foreground = unsafe { GetForegroundWindow() };
    let window = SelectionWindow(unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("EasydictScreenshotSelection"),
            w!("Easydict 截图 — 拖动选择区域，Esc 或右键取消"),
            WS_POPUP,
            left, top, width, height,
            None, None, Some(instance),
            Some((selection as *const RefCell<DesktopSelection>).cast()),
        )
    }.map_err(capture_error)?);
    unsafe {
        let _ = ShowWindow(window.0, SW_SHOW);
        let _ = SetForegroundWindow(window.0);
        let _ = SetFocus(Some(window.0));
    }
    let mut message = MSG::default();
    loop {
        let received = unsafe { GetMessageW(&mut message, None, 0, 0) }.0;
        if received == -1 {
            return Err(capture_error(windows::core::Error::from_thread()));
        }
        if received == 0 {
            break;
        }
        unsafe {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    if selection.borrow().selected.is_some() && unsafe { IsWindow(Some(previous_foreground)) }.as_bool() {
        let _ = unsafe { SetForegroundWindow(previous_foreground) };
    }
    Ok(())
}

fn selection_rect(bounds: (i32, i32, i32, i32), start: POINT, end: POINT) -> Option<RECT> {
    let local_x = |value: i32| (value as i64 - bounds.0 as i64).clamp(0, bounds.2 as i64) as i32;
    let local_y = |value: i32| (value as i64 - bounds.1 as i64).clamp(0, bounds.3 as i64) as i32;
    let region = RECT {
        left: local_x(start.x.min(end.x)),
        top: local_y(start.y.min(end.y)),
        right: local_x(start.x.max(end.x)),
        bottom: local_y(start.y.max(end.y)),
    };
    (region.right - region.left >= 2 && region.bottom - region.top >= 2).then_some(region)
}

fn finish(window: HWND, selection: &RefCell<DesktopSelection>, region: Option<RECT>, restore_window: bool) {
    {
        let mut state = selection.borrow_mut();
        if state.finished {
            return;
        }
        state.finished = true;
        state.selected = region;
        state.restore_window = restore_window;
        state.anchor = None;
    }
    unsafe {
        let _ = ReleaseCapture();
        let _ = DestroyWindow(window);
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, word: WPARAM, long: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let creation = &*(long.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, creation.lpCreateParams as isize);
            return LRESULT(1);
        }
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const RefCell<DesktopSelection>;
        let Some(selection) = pointer.as_ref() else {
            return DefWindowProcW(window, message, word, long);
        };
        match message {
            WM_PAINT => paint(window, &selection.borrow()),
            WM_ERASEBKGND => return LRESULT(1),
            WM_LBUTTONDOWN | WM_MOUSEMOVE | WM_LBUTTONUP => {
                let mut cursor = POINT::default();
                if GetCursorPos(&mut cursor).is_err() {
                    finish(window, selection, None, true);
                    return LRESULT(0);
                }
                {
                    let mut state = selection.borrow_mut();
                    state.cursor = cursor;
                    if message == WM_LBUTTONDOWN {
                        state.anchor = Some(cursor);
                    }
                }
                if message == WM_LBUTTONDOWN {
                    SetCapture(window);
                } else if message == WM_LBUTTONUP {
                    let region = {
                        let mut state = selection.borrow_mut();
                        state.anchor.take().and_then(|start| selection_rect(state.bounds, start, cursor))
                    };
                    if region.is_some() {
                        finish(window, selection, region, true);
                    } else {
                        let _ = ReleaseCapture();
                    }
                }
                let _ = InvalidateRect(Some(window), None, false);
            }
            WM_KEYDOWN if word.0 == VK_ESCAPE.0 as usize => finish(window, selection, None, true),
            WM_RBUTTONUP | WM_CLOSE => finish(window, selection, None, true),
            WM_CANCELMODE | WM_DISPLAYCHANGE => finish(window, selection, None, false),
            WM_ACTIVATE if word.0 & 0xffff == WA_INACTIVE as usize => finish(window, selection, None, false),
            WM_CAPTURECHANGED => {
                if selection.borrow().anchor.is_some() {
                    finish(window, selection, None, false);
                }
            }
            WM_DPICHANGED => return LRESULT(0),
            WM_DESTROY => {
                PostQuitMessage(0);
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                return DefWindowProcW(window, message, word, long);
            }
            _ => return DefWindowProcW(window, message, word, long),
        }
        LRESULT(0)
    }
}

fn paint(window: HWND, selection: &DesktopSelection) {
    let (_, _, width, height) = selection.bounds;
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut painting = PAINTSTRUCT::default();
    unsafe {
        let context = BeginPaint(window, &mut painting);
        draw_pixels(context, &selection.dimmed, &info, width, height);
        if let Some(region) = selection.anchor.and_then(|start| selection_rect(selection.bounds, start, selection.cursor)) {
            let saved = SaveDC(context);
            if saved != 0 {
                IntersectClipRect(context, region.left, region.top, region.right, region.bottom);
                draw_pixels(context, &selection.pixels, &info, width, height);
                let _ = RestoreDC(context, saved);
            }
            FrameRect(context, &region, HBRUSH(GetStockObject(WHITE_BRUSH).0));
        }
        let mut monitor = MONITORINFO { cbSize: size_of::<MONITORINFO>() as u32, ..Default::default() };
        if GetMonitorInfoW(MonitorFromPoint(selection.cursor, MONITOR_DEFAULTTONEAREST), &mut monitor).as_bool() {
            let monitor = monitor.rcMonitor;
            let hint_width = 480.min(monitor.right - monitor.left);
            let hint_left = monitor.left - selection.bounds.0 + (monitor.right - monitor.left - hint_width) / 2;
            let hint_top = monitor.top - selection.bounds.1 + 24;
            let mut hint = RECT { left: hint_left, top: hint_top, right: hint_left + hint_width, bottom: hint_top + 34 };
            FillRect(context, &hint, HBRUSH(GetStockObject(BLACK_BRUSH).0));
            SetBkMode(context, TRANSPARENT);
            SetTextColor(context, COLORREF(0x00ffffff));
            let previous_font = SelectObject(context, GetStockObject(DEFAULT_GUI_FONT));
            let mut text: Vec<u16> = "拖动选择区域，松开鼠标识别 · Esc / 右键取消".encode_utf16().collect();
            DrawTextW(context, &mut text, &mut hint, DT_CENTER | DT_SINGLELINE | DT_VCENTER);
            SelectObject(context, previous_font);
        }
        let _ = EndPaint(window, &painting);
    }
}

unsafe fn draw_pixels(context: HDC, pixels: &[u8], info: &BITMAPINFO, width: i32, height: i32) {
    unsafe {
        SetDIBitsToDevice(context, 0, 0, width as u32, height as u32, 0, 0, 0,
            height as u32, pixels.as_ptr().cast(), info, DIB_RGB_COLORS);
    }
}

fn capture_error(error: windows::core::Error) -> PlatformError {
    PlatformError::Capture(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_drag_on_a_negative_origin_monitor_uses_physical_pixels() {
        let region = selection_rect(
            (-2560, -1440, 6400, 3600),
            POINT { x: -1200, y: -300 },
            POINT { x: -2400, y: -1200 },
        ).unwrap();
        assert_eq!((region.left, region.top, region.right, region.bottom), (160, 240, 1360, 1140));
    }

    #[test]
    fn drag_is_clamped_to_the_snapshot_even_at_integer_extremes() {
        let region = selection_rect(
            (-100, -200, 2000, 1500),
            POINT { x: i32::MIN, y: i32::MIN },
            POINT { x: i32::MAX, y: i32::MAX },
        ).unwrap();
        assert_eq!((region.left, region.top, region.right, region.bottom), (0, 0, 2000, 1500));
    }

    #[test]
    fn clicks_tiny_drags_and_regions_outside_the_screen_are_not_captures() {
        let bounds = (0, 0, 1920, 1080);
        assert!(selection_rect(bounds, POINT { x: 10, y: 10 }, POINT { x: 10, y: 10 }).is_none());
        assert!(selection_rect(bounds, POINT { x: 10, y: 10 }, POINT { x: 11, y: 100 }).is_none());
        assert!(selection_rect(bounds, POINT { x: -100, y: -100 }, POINT { x: -10, y: -10 }).is_none());
        assert!(selection_rect(bounds, POINT { x: 10, y: 10 }, POINT { x: 12, y: 12 }).is_some());
    }

    #[test]
    fn only_one_selector_can_run_and_the_lease_is_released_on_exit() {
        let active = AtomicBool::new(false);
        let lease = SelectionLease::acquire(&active).unwrap();
        assert!(SelectionLease::acquire(&active).is_err());
        drop(lease);
        assert!(SelectionLease::acquire(&active).is_ok());
    }
}
