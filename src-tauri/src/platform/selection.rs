//! Reading the text the user selected in another application.
//!
//! Windows has no single API for this. The two strategies here, in order:
//!
//! 1. **UI Automation TextPattern** — the element that owns the focus, then a
//!    few ancestors, is asked for its explicit selection. Rich text controls,
//!    browsers and WPF/UWP text support this, and it never touches the
//!    clipboard, so it is tried first.
//! 2. **A synthetic Ctrl+C** — many controls (plain `EDIT`, WinForms text
//!    boxes, Electron widgets that expose no text provider) publish no
//!    TextPattern at all. For those the only portable route is to ask the
//!    application to copy its selection, read it and put the previous
//!    clipboard contents back. This is the Windows counterpart of what the
//!    macOS app's "force get selected text" setting means.
//!
//! Nothing is copied from a password field: the UIA element's `IsPassword`
//! flag covers accessible controls and the window class/style check covers
//! plain `EDIT` controls that never reach UI Automation.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    E_NOINTERFACE, ERROR_SUCCESS, GetLastError, GlobalFree, HANDLE, HGLOBAL, HWND,
    RPC_E_CHANGED_MODE, SetLastError,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
    CoUninitialize,
};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, EnumClipboardFormats, GetClipboardData, GetClipboardOwner,
    GetClipboardSequenceNumber, OpenClipboard, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
    UIA_E_NOTSUPPORTED, UIA_TextPatternId,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GUITHREADINFO, GWL_STYLE, GetClassNameW, GetForegroundWindow,
    GetGUIThreadInfo, GetWindowLongW, GetWindowThreadProcessId, HWND_MESSAGE, WS_POPUP,
};
use windows::core::w;

use super::PlatformError;

/// How far up the UIA tree a focused element is searched for text.
const MAX_ANCESTOR_DEPTH: usize = 8;
/// Upper bound on the whole UI Automation walk. Cross-process UIA calls cannot
/// be interrupted once they are in flight, so this is checked between steps.
const SELECTION_TIMEOUT: Duration = Duration::from_millis(1200);
/// How long a target application gets to put the selection on the clipboard
/// after Ctrl+C. Office and browsers occasionally need a few hundred
/// milliseconds on their first copy; waiting longer only delays the shortcut.
const COPY_TIMEOUT: Duration = Duration::from_millis(700);
/// How long to wait for Alt/Shift/Win to come up after the hotkey fired.
/// Ctrl+C sent while Alt is still down arrives as Ctrl+Alt+C in most apps.
const MODIFIER_RELEASE_TIMEOUT: Duration = Duration::from_millis(500);
/// How often to try opening the clipboard before giving up. Other processes may
/// hold it for a moment; the selection read is not worth failing over that.
const CLIPBOARD_OPEN_ATTEMPTS: usize = 6;

/// `ES_PASSWORD` from `winuser.h`. The `windows` crate only exposes the
/// control constants behind an extra feature, and this one value is all the
/// password guard needs.
const ES_PASSWORD: u32 = 0x0020;
static READING: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

#[derive(Clone, Copy, PartialEq)]
pub(super) struct SelectionTarget {
    pub window: HWND,
    pub focus: HWND,
}

impl SelectionTarget {
    pub(super) fn current() -> Option<Self> {
        unsafe {
            let window = GetForegroundWindow();
            let mut info = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if window.is_invalid() || GetGUIThreadInfo(0, &mut info).is_err() {
                return None;
            }
            Some(Self {
                window,
                focus: info.hwndFocus,
            })
        }
    }

    pub(super) fn is_current(self) -> bool {
        Self::current() == Some(self)
    }
}

/// The text currently selected in the foreground window.
///
/// `force` enables the clipboard fallback (the `enableForceGetSelectedText`
/// setting). Without it only UI Automation is consulted, so the clipboard is
/// never written.
pub fn selected_text(force: bool) -> Result<String, PlatformError> {
    selected_text_at(force, None)
}

/// Plain integers can safely cross the blocking-worker boundary, unlike HWND.
pub(crate) fn selection_target() -> Option<[isize; 2]> {
    let target = SelectionTarget::current()?;
    Some([target.window.0 as isize, target.focus.0 as isize])
}

pub(crate) fn selected_text_at(
    force: bool,
    target: Option<[isize; 2]>,
) -> Result<String, PlatformError> {
    let expected = target.map(|[window, focus]| SelectionTarget {
        window: HWND(window as *mut _),
        focus: HWND(focus as *mut _),
    });
    read_selection(force, expected, false, false, &|| true)
}

pub(super) fn automatic_selected_text(
    force: bool,
    target: SelectionTarget,
    select_all: bool,
    valid: &dyn Fn() -> bool,
) -> Result<String, PlatformError> {
    read_selection(force, Some(target), true, select_all, valid)
}

fn read_selection(
    force: bool,
    expected: Option<SelectionTarget>,
    automatic: bool,
    select_all: bool,
    valid: &dyn Fn() -> bool,
) -> Result<String, PlatformError> {
    // Do not queue a second copy behind a slow reader: its foreground target
    // may be different by the time the first one finishes.
    if !valid() {
        return Err(PlatformError::NoSelection);
    }
    let _lease = READING.try_lock().ok_or(PlatformError::NoSelection)?;
    read_selection_locked(force, expected, automatic, select_all, valid)
}

fn read_selection_locked(
    force: bool,
    expected: Option<SelectionTarget>,
    automatic: bool,
    select_all: bool,
    valid: &dyn Fn() -> bool,
) -> Result<String, PlatformError> {
    let target = SelectionTarget::current().ok_or(PlatformError::NoSelection)?;
    if expected.is_some_and(|expected| expected != target) || focus_is_password_field() {
        return Err(PlatformError::NoSelection);
    }
    let result = selected_text_via_uia(automatic, select_all);
    if !valid() || !target.is_current() {
        return Err(PlatformError::NoSelection);
    }
    finish_selection(result, force, || selected_text_via_clipboard(target, valid))
}

/// The original text and target are mandatory: never paste into the application's
/// own result card or whichever unrelated control happens to have focus now.
pub(crate) fn replace_selected_text(
    text: &str,
    expected_text: &str,
    expected_target: [isize; 2],
    force: bool,
) -> Result<(), PlatformError> {
    if text.trim().is_empty() || text.contains('\0') || expected_text.trim().is_empty()
        || expected_target.contains(&0)
    {
        return Err(PlatformError::Platform("替换内容或原选区信息无效，已取消替换".into()));
    }
    let _lease = READING.try_lock().ok_or_else(|| {
        PlatformError::Platform("正在读取或替换另一处选区，请稍后重试".into())
    })?;
    let target = SelectionTarget {
        window: HWND(expected_target[0] as *mut _),
        focus: HWND(expected_target[1] as *mut _),
    };
    let mut process = 0;
    if !target.is_current() {
        return Err(PlatformError::Platform("原选区窗口已失去焦点，已取消替换".into()));
    }
    unsafe { GetWindowThreadProcessId(target.window, Some(&mut process)); }
    if process == 0 || process == std::process::id()
        || foreground_class().is_none_or(|class| is_console_class(&class))
    {
        return Err(PlatformError::Platform("请在原应用的文本选区使用替换快捷键；不向本应用或终端粘贴".into()));
    }
    let selected = read_selection_locked(force, Some(target), false, false, &|| true)?;
    if !selection_unchanged(expected_text, &selected) || !wait_for_modifiers(target, &|| true) {
        return Err(PlatformError::Platform("原选区或按键状态已改变，已取消替换".into()));
    }
    paste_with_clipboard(text, &|| target.is_current() && !conflicting_modifiers_held(), || {
        crate::system::send_paste_keystroke().map_err(PlatformError::Platform)?;
        // This is a bounded grace period, not an acknowledgement from the editor.
        std::thread::sleep(Duration::from_millis(120));
        Ok(())
    })
}

fn selection_unchanged(expected: &str, current: &str) -> bool {
    !expected.trim().is_empty() && expected == current
}

fn paste_with_clipboard(
    text: &str,
    valid: &dyn Fn() -> bool,
    paste: impl FnOnce() -> Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    if !valid() { return Err(PlatformError::NoSelection); }
    let mut backup = ClipboardBackup::capture().ok_or_else(|| PlatformError::Platform(
        "为保护剪贴板，已取消替换：剪贴板忙或含无法完整备份的格式。请保存剪贴板内容后再试。".into(),
    ))?;
    let sequence = backup.write_text(text, valid)?;
    let result = if valid() && unsafe { GetClipboardSequenceNumber() } == sequence {
        paste()
    } else {
        Err(PlatformError::Platform("原选区或剪贴板已改变，已取消粘贴".into()))
    };
    // Always attempt cleanup, including a failed SendInput or lost focus.
    finish_paste(result, backup.restore(sequence))
}

fn finish_paste(
    result: Result<(), PlatformError>,
    restored: Result<(), PlatformError>,
) -> Result<(), PlatformError> {
    match restored {
        // A newer clipboard revision belongs to the user, not this transaction.
        Ok(()) | Err(PlatformError::NoSelection) => result,
        Err(error) => Err(PlatformError::Platform(format!(
            "{}剪贴板恢复失败，请检查剪贴板：{error}",
            result.err().map(|error| format!("粘贴失败：{error}；")).unwrap_or_default(),
        ))),
    }
}

fn finish_selection(
    result: Result<Option<String>, PlatformError>,
    force: bool,
    fallback: impl FnOnce() -> Result<String, PlatformError>,
) -> Result<String, PlatformError> {
    match result? {
        Some(text) => Ok(text),
        None if force => fallback(),
        None => Err(PlatformError::NoSelection),
    }
}

/// The TextPattern selection of the focused element or one of its ancestors.
/// Only `Ok(None)` permits fallback; protection/access errors are terminal.
fn selected_text_via_uia(
    automatic: bool,
    select_all: bool,
) -> Result<Option<String>, PlatformError> {
    let _apartment = ComApartment::init();

    unsafe {
        let automation: IUIAutomation = CoCreateInstance(
            &CUIAutomation,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
        .map_err(|error| {
            PlatformError::Platform(format!("UI Automation is unavailable: {error}"))
        })?;

        let Ok(focused) = automation.GetFocusedElement() else {
            // The desktop, a secure prompt or a locked session: nothing owns
            // the focus, so nothing can be selected.
            return Err(PlatformError::NoSelection);
        };
        let allow_copy = if automatic {
            use windows::Win32::UI::Accessibility::{
                UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_TextControlTypeId,
            };
            let kind = focused.CurrentControlType().map_err(uia_error)?;
            if select_all && kind != UIA_EditControlTypeId {
                return Err(PlatformError::NoSelection);
            }
            // UIA can expose an explicit selection on an ancestor of a pane.
            // Only the synthetic-copy fallback needs a known text control.
            kind == UIA_EditControlTypeId
                || kind == UIA_DocumentControlTypeId
                || kind == UIA_TextControlTypeId
        } else {
            true
        };

        let walker = automation.ControlViewWalker().map_err(uia_error)?;
        let deadline = Instant::now() + SELECTION_TIMEOUT;
        let mut element = focused;

        for _ in 0..MAX_ANCESTOR_DEPTH {
            if element
                .CurrentIsPassword()
                .map_or(true, |value| value.as_bool())
            {
                return Err(PlatformError::NoSelection);
            }
            if Instant::now() >= deadline {
                return Err(PlatformError::Platform(
                    "UI Automation did not answer within 1.2 seconds".into(),
                ));
            }

            if let Some(text) = text_from_element(&element).map_err(uia_error)? {
                return Ok(Some(text));
            }

            // Text lives on a container as often as on the focused element
            // itself (a page rather than the caret's leaf node).
            match walker.GetParentElement(&element) {
                Ok(parent) => element = parent,
                Err(error) if error.code() == windows::core::Error::empty().code() => break,
                Err(error) => return Err(uia_error(error)),
            }
        }
        if !allow_copy {
            return Err(PlatformError::NoSelection);
        }
    }

    Ok(None)
}

/// The element's explicit selection, excluding protected controls.
unsafe fn text_from_element(
    element: &IUIAutomationElement,
) -> windows::core::Result<Option<String>> {
    match unsafe { element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) } {
        Ok(pattern) => {
            let selected = unsafe { selection_of(&pattern) }?;
            Ok((!selected.trim().is_empty()).then_some(selected))
        }
        Err(error) if pattern_is_unavailable(&error) => Ok(None),
        Err(error) => Err(error),
    }
}

fn pattern_is_unavailable(error: &windows::core::Error) -> bool {
    error.code() == E_NOINTERFACE
        || error.code().0 as u32 == UIA_E_NOTSUPPORTED
        // windows-rs represents S_OK with a null COM result as Error::empty().
        || error.code() == windows::core::Error::empty().code()
}

fn uia_error(error: windows::core::Error) -> PlatformError {
    PlatformError::Platform(format!("UI Automation: {error}"))
}

/// The TextPattern's current selection, with multiple ranges joined by newlines.
unsafe fn selection_of(pattern: &IUIAutomationTextPattern) -> windows::core::Result<String> {
    let ranges = unsafe { pattern.GetSelection() }?;
    let count = unsafe { ranges.Length() }?;

    let mut selected = String::new();
    for index in 0..count {
        let range = unsafe { ranges.GetElement(index) }?;
        // `-1` means "to the end of the range".
        let text = unsafe { range.GetText(-1) }?;
        let text = text.to_string();
        let text = text.trim_matches('\0');
        if text.trim().is_empty() {
            continue;
        }
        if !selected.is_empty() {
            selected.push('\n');
        }
        selected.push_str(text);
    }
    Ok(selected)
}

/// Console windows, where `Ctrl+C` is not "copy" but "interrupt the running
/// program". The clipboard fallback must not fire there.
const CONSOLE_CLASSES: [&str; 3] = [
    "ConsoleWindowClass",
    "CASCADIA_HOSTING_WINDOW_CLASS",
    "mintty",
];

/// Asks the focused application to copy its selection and reads the result.
///
/// Copy only when every original format can be backed up, and restore only
/// the clipboard revision read here, never a subsequent user/application copy.
fn selected_text_via_clipboard(
    target: SelectionTarget,
    valid: &dyn Fn() -> bool,
) -> Result<String, PlatformError> {
    if focus_is_password_field() {
        log::debug!("selection: the focused control is a password field");
        return Err(PlatformError::NoSelection);
    }
    if foreground_class().is_some_and(|class| is_console_class(&class)) {
        log::debug!("selection: the foreground window is a console, not sending Ctrl+C");
        return Err(PlatformError::NoSelection);
    }

    if !wait_for_modifiers(target, valid) {
        return Err(PlatformError::NoSelection);
    }
    let mut backup = ClipboardBackup::capture().ok_or_else(|| {
        PlatformError::Platform(
            "为保护剪贴板，已跳过模拟复制：剪贴板忙或含无法完整备份的格式。请手动复制后翻译。"
                .into(),
        )
    })?;
    if !valid()
        || !target.is_current()
        || unsafe { GetClipboardSequenceNumber() } != backup.sequence
    {
        return Err(PlatformError::NoSelection);
    }
    crate::system::send_copy_keystroke().map_err(PlatformError::Platform)?;

    let deadline = Instant::now() + COPY_TIMEOUT;
    while Instant::now() < deadline {
        if unsafe { GetClipboardSequenceNumber() } != backup.sequence {
            let _clipboard =
                ClipboardOpen::acquire(backup.window.0).ok_or(PlatformError::NoSelection)?;
            // An unrelated app may have copied while Ctrl+C was in flight.
            if !clipboard_belongs_to(target.window) {
                return Err(PlatformError::NoSelection);
            }
            let text = unsafe { read_clipboard_text() };
            let sequence = unsafe { GetClipboardSequenceNumber() };
            drop(_clipboard);
            backup.restore(sequence)?;
            return text
                .filter(|_| valid() && target.is_current())
                .ok_or(PlatformError::NoSelection);
        }
        if !target.is_current() {
            return Err(PlatformError::NoSelection);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    Err(PlatformError::NoSelection)
}

fn clipboard_belongs_to(window: HWND) -> bool {
    unsafe {
        let Ok(owner) = GetClipboardOwner() else {
            return false;
        };
        let mut expected = 0;
        let mut actual = 0;
        GetWindowThreadProcessId(window, Some(&mut expected));
        GetWindowThreadProcessId(owner, Some(&mut actual));
        expected != 0 && expected == actual
    }
}

/// Caller holds the clipboard open, including while checking its revision.
unsafe fn read_clipboard_text() -> Option<String> {
    let handle = HGLOBAL(unsafe { GetClipboardData(13) }.ok()?.0); // CF_UNICODETEXT
    let length = unsafe { GlobalSize(handle) } / 2;
    let pointer = unsafe { GlobalLock(handle) } as *const u16;
    if pointer.is_null() {
        return None;
    }
    let units = unsafe { std::slice::from_raw_parts(pointer, length) };
    let end = units
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units.len());
    let text = String::from_utf16_lossy(&units[..end]);
    let _ = unsafe { GlobalUnlock(handle) };
    (!text.trim().is_empty()).then_some(text)
}

/// Whether the focused control of the foreground thread is an `EDIT` with
/// `ES_PASSWORD`, which never publishes a usable UIA text provider.
fn focus_is_password_field() -> bool {
    unsafe {
        let mut info = GUITHREADINFO {
            cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if GetGUIThreadInfo(0, &mut info).is_err() || info.hwndFocus.is_invalid() {
            return false;
        }
        let focus: HWND = info.hwndFocus;
        let mut class = [0u16; 256];
        let length = GetClassNameW(focus, &mut class);
        if length <= 0 {
            return false;
        }
        let class = String::from_utf16_lossy(&class[..length as usize]);
        let lower = class.to_ascii_lowercase();
        let is_edit = lower == "edit" || lower.starts_with("windowsforms10.edit");
        if !is_edit {
            return false;
        }
        let style = GetWindowLongW(focus, GWL_STYLE) as u32;
        style & ES_PASSWORD != 0
    }
}

/// Whether a window class belongs to a console, where Ctrl+C interrupts rather
/// than copies.
fn is_console_class(class: &str) -> bool {
    CONSOLE_CLASSES
        .iter()
        .any(|console| class.eq_ignore_ascii_case(console))
}

/// The class name of the foreground window, if there is one.
fn foreground_class() -> Option<String> {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return None;
        }
        let mut class = [0u16; 256];
        let length = GetClassNameW(window, &mut class);
        (length > 0).then(|| String::from_utf16_lossy(&class[..length as usize]))
    }
}

fn conflicting_modifiers_held() -> bool {
    [VK_MENU, VK_SHIFT, VK_LWIN, VK_RWIN].into_iter()
        .any(|key| (unsafe { GetAsyncKeyState(key.0 as i32) } as u16) & 0x8000 != 0)
}

/// Wait for modifiers that would change Ctrl+C / Ctrl+V, without releasing
/// keys the user is still physically holding.
fn wait_for_modifiers(target: SelectionTarget, valid: &dyn Fn() -> bool) -> bool {
    let deadline = Instant::now() + MODIFIER_RELEASE_TIMEOUT;
    loop {
        if !valid() || !target.is_current() {
            return false;
        }
        if !conflicting_modifiers_held() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A duplicate of every clipboard format that can be copied as a global
/// memory block, so the original contents can be put back after the synthetic
/// copy.
///
/// Unsupported or unreadable formats abort the whole capture, not just that
/// format. In particular a metafile's HGLOBAL contains another live handle;
/// copying those bytes does NOT back up the metafile.
struct ClipboardBackup {
    entries: Vec<(u32, HGLOBAL)>,
    sequence: u32,
    window: ClipboardWindow,
}

impl ClipboardBackup {
    /// `None` when the clipboard cannot be read safely; the caller then leaves
    /// it alone instead of replacing it.
    fn capture() -> Option<Self> {
        let window = ClipboardWindow::new()?;
        let Some(_clipboard) = ClipboardOpen::acquire(window.0) else {
            log::warn!("selection: the clipboard stayed busy, not reading the selection");
            return None;
        };

        let mut backup = Self {
            entries: Vec::new(),
            sequence: 0,
            window,
        };
        let mut format = 0;
        loop {
            unsafe { SetLastError(ERROR_SUCCESS) };
            format = unsafe { EnumClipboardFormats(format) };
            if format == 0 {
                if unsafe { GetLastError() } != ERROR_SUCCESS {
                    return None;
                }
                break;
            }
            if !is_global_format(format) {
                log::debug!("selection: clipboard format {format} cannot be safely backed up");
                return None;
            }
            let handle = unsafe { GetClipboardData(format) }.ok()?;
            let copy = unsafe { duplicate_global(HGLOBAL(handle.0)) }?;
            backup.entries.push((format, copy));
        }
        backup.sequence = unsafe { GetClipboardSequenceNumber() };
        Some(backup)
    }

    /// Replaces the clipboard with the duplicate handles. They belong to the
    /// system after this, so the entries are drained and not freed.
    fn restore(&mut self, expected: u32) -> Result<(), PlatformError> {
        let clipboard = ClipboardOpen::acquire(self.window.0).ok_or_else(|| {
            PlatformError::Platform("could not reopen the clipboard to restore it".into())
        })?;
        self.restore_open(expected, &clipboard)
    }

    fn restore_open(&mut self, expected: u32, _clipboard: &ClipboardOpen) -> Result<(), PlatformError> {
        unsafe {
            if GetClipboardSequenceNumber() != expected {
                return Err(PlatformError::NoSelection);
            }
            EmptyClipboard().map_err(|error| PlatformError::Platform(error.to_string()))?;
            while let Some(&(format, handle)) = self.entries.last() {
                SetClipboardData(format, Some(HANDLE(handle.0)))
                    .map_err(|error| PlatformError::Platform(error.to_string()))?;
                self.entries.pop();
            }
        }
        Ok(())
    }

    fn write_text(&mut self, text: &str, valid: &dyn Fn() -> bool) -> Result<u32, PlatformError> {
        let text = ClipboardText::new(text)?;
        let clipboard = ClipboardOpen::acquire(self.window.0).ok_or_else(|| {
            PlatformError::Platform("剪贴板被占用，未执行替换".into())
        })?;
        unsafe {
            if !valid() || GetClipboardSequenceNumber() != self.sequence {
                return Err(PlatformError::Platform("原选区或剪贴板已改变，未执行替换".into()));
            }
            EmptyClipboard().map_err(|error| PlatformError::Platform(error.to_string()))?;
            if let Err(error) = SetClipboardData(13, Some(HANDLE(text.0.0))) {
                // Roll back while still holding the clipboard, before another
                // process can write. The untransferred text handle stays ours.
                let restored = self.restore_open(GetClipboardSequenceNumber(), &clipboard);
                return finish_paste(Err(PlatformError::Platform(error.to_string())), restored)
                    .map(|()| GetClipboardSequenceNumber());
            }
            std::mem::forget(text); // SetClipboardData transferred ownership.
            Ok(GetClipboardSequenceNumber())
        }
    }
}

struct ClipboardText(HGLOBAL);

impl ClipboardText {
    fn new(text: &str) -> Result<Self, PlatformError> {
        if text.trim().is_empty() || text.contains('\0') {
            return Err(PlatformError::Platform("替换文字为空或含空字符".into()));
        }
        let units: Vec<u16> = text.encode_utf16().chain([0]).collect();
        unsafe {
            let memory = Self(GlobalAlloc(GMEM_MOVEABLE, std::mem::size_of_val(units.as_slice()))
                .map_err(|error| PlatformError::Platform(error.to_string()))?);
            let pointer = GlobalLock(memory.0).cast::<u16>();
            if pointer.is_null() {
                return Err(PlatformError::Platform("无法准备替换文字".into()));
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), pointer, units.len());
            let _ = GlobalUnlock(memory.0);
            Ok(memory)
        }
    }
}

impl Drop for ClipboardText {
    fn drop(&mut self) {
        unsafe { let _ = GlobalFree(Some(self.0)); }
    }
}

impl Drop for ClipboardBackup {
    fn drop(&mut self) {
        for (_, handle) in self.entries.drain(..) {
            unsafe {
                let _ = GlobalFree(Some(handle));
            }
        }
    }
}

// GDI handles, nested metafile handles, private and owner-display formats
// cannot be restored with a byte-for-byte HGLOBAL copy.
fn is_global_format(format: u32) -> bool {
    !matches!(format, 0 | 2 | 3 | 9 | 14 | 0x80..=0x8f | 0x200..=0x3ff)
}

struct ClipboardWindow(HWND);

impl ClipboardWindow {
    fn new() -> Option<Self> {
        // OpenClipboard(NULL) + EmptyClipboard leaves no owner and is not a
        // valid SetClipboardData transaction. A message-only window stays hidden.
        unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Easydict clipboard"),
                WS_POPUP,
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
            .ok()
            .map(Self)
        }
    }
}

impl Drop for ClipboardWindow {
    fn drop(&mut self) {
        let _ = unsafe { DestroyWindow(self.0) };
    }
}

struct ClipboardOpen;

impl ClipboardOpen {
    fn acquire(window: HWND) -> Option<Self> {
        for _ in 0..CLIPBOARD_OPEN_ATTEMPTS {
            if unsafe { OpenClipboard(Some(window)) }.is_ok() {
                return Some(Self);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }
}

impl Drop for ClipboardOpen {
    fn drop(&mut self) {
        let _ = unsafe { CloseClipboard() };
    }
}

/// Copies an HGLOBAL after its format is checked by `is_global_format`.
/// GlobalSize alone cannot detect nested handles such as CF_METAFILEPICT.
unsafe fn duplicate_global(source: HGLOBAL) -> Option<HGLOBAL> {
    let size = unsafe { GlobalSize(source) };
    if size == 0 {
        return None;
    }
    let source_ptr = unsafe { GlobalLock(source) };
    if source_ptr.is_null() {
        return None;
    }
    let copy = match unsafe { GlobalAlloc(GMEM_MOVEABLE, size) } {
        Ok(copy) => copy,
        Err(_) => {
            unsafe {
                let _ = GlobalUnlock(source);
            }
            return None;
        }
    };
    let target_ptr = unsafe { GlobalLock(copy) };
    if target_ptr.is_null() {
        unsafe {
            let _ = GlobalFree(Some(copy));
            let _ = GlobalUnlock(source);
        }
        return None;
    }
    unsafe {
        std::ptr::copy_nonoverlapping(source_ptr as *const u8, target_ptr as *mut u8, size);
        let _ = GlobalUnlock(copy);
        let _ = GlobalUnlock(source);
    }
    Some(copy)
}

/// The UIA client apartment, released on the way out.
struct ComApartment {
    uninitialize: bool,
}

impl ComApartment {
    fn init() -> Self {
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // `RPC_E_CHANGED_MODE` means the thread already lives in another
        // apartment: usable for UIA, but not ours to tear down. A successful
        // call is balanced in `drop`, `S_FALSE` ("already initialised") too.
        ComApartment {
            uninitialize: result.is_ok() && result != RPC_E_CHANGED_MODE,
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replacement_checks_the_exact_selection_including_whitespace() {
        assert!(selection_unchanged(" text\r\n", " text\r\n"));
        for (expected, actual) in [("text", "new"), (" text ", "text"), ("a\r\nb", "a\nb"), ("", ""), (" ", " ")] {
            assert!(!selection_unchanged(expected, actual), "{expected:?} / {actual:?}");
        }
    }

    #[test]
    fn invalid_and_concurrent_replacements_never_reach_the_clipboard() {
        for (text, original, target) in [
            ("", "old", [1, 2]), ("new\0text", "old", [1, 2]),
            ("new", "", [1, 2]), ("new", "old", [0, 2]), ("new", "old", [1, 0]),
        ] {
            assert!(replace_selected_text(text, original, target, true).is_err());
        }
        let _lease = READING.lock();
        assert!(replace_selected_text("new", "old", [1, 2], true)
            .unwrap_err().to_string().contains("正在读取或替换"));
        assert!(paste_with_clipboard("new", &|| false, || panic!("must not paste")).is_err());
    }

    #[test]
    fn paste_cleanup_preserves_newer_copies_and_reports_actual_restore_failures() {
        assert!(finish_paste(Ok(()), Err(PlatformError::NoSelection)).is_ok());
        let failure = || Err(PlatformError::Platform("SendInput failed".into()));
        assert!(finish_paste(failure(), Ok(())).unwrap_err().to_string().contains("SendInput failed"));
        assert!(finish_paste(failure(), Err(PlatformError::NoSelection))
            .unwrap_err().to_string().contains("SendInput failed"));
        let error = finish_paste(failure(), Err(PlatformError::Platform("clipboard busy".into())))
            .unwrap_err().to_string();
        assert!(error.contains("SendInput failed") && error.contains("clipboard busy"));
    }

    #[test]
    fn replacement_text_is_a_terminated_utf16_block_and_rejects_truncation() {
        assert!(ClipboardText::new("").is_err());
        assert!(ClipboardText::new("abc\0def").is_err());
        let text = "Hello \u{1f642}";
        let block = ClipboardText::new(text).unwrap();
        let expected: Vec<u16> = text.encode_utf16().chain([0]).collect();
        unsafe {
            let pointer = GlobalLock(block.0).cast::<u16>();
            assert!(!pointer.is_null());
            assert_eq!(std::slice::from_raw_parts(pointer, expected.len()), expected);
            let _ = GlobalUnlock(block.0);
        }
    }

    #[test]
    fn uia_protection_and_errors_never_fall_back_to_copy() {
        use windows::Win32::Foundation::E_ACCESSDENIED;
        use windows::core::{Error, HRESULT};
        assert!(pattern_is_unavailable(&Error::empty()));
        assert!(pattern_is_unavailable(&Error::from_hresult(E_NOINTERFACE)));
        assert!(pattern_is_unavailable(&Error::from_hresult(HRESULT(
            UIA_E_NOTSUPPORTED as i32
        ))));
        assert!(!pattern_is_unavailable(&Error::from_hresult(
            E_ACCESSDENIED
        )));
        for error in [
            PlatformError::NoSelection,
            PlatformError::Platform("UIA access denied".into()),
        ] {
            assert!(finish_selection(Err(error), true, || panic!("must not copy")).is_err());
        }
        assert_eq!(
            finish_selection(Ok(Some("selected".into())), true, || panic!(
                "must not copy"
            ))
            .unwrap(),
            "selected",
        );
        assert!(finish_selection(Ok(None), false, || panic!("fallback disabled")).is_err());
        assert_eq!(
            finish_selection(Ok(None), true, || Ok("copied".into())).unwrap(),
            "copied",
        );
    }

    #[test]
    fn clipboard_formats_with_embedded_or_non_memory_handles_are_rejected() {
        for format in [
            0, 2, 3, 9, 14, 0x80, 0x82, 0x83, 0x8e, 0x200, 0x2ff, 0x300, 0x3ff,
        ] {
            assert!(!is_global_format(format), "{format}");
        }
        for format in [1, 7, 8, 13, 15, 16, 17, 0xc001, 0xffff] {
            assert!(is_global_format(format), "{format}");
        }
    }

    #[test]
    fn concurrent_selection_is_rejected_instead_of_queued() {
        let _lease = READING.lock();
        assert!(matches!(
            selected_text(true),
            Err(PlatformError::NoSelection)
        ));
    }

    #[test]
    fn cancelled_automatic_reads_stop_before_accessing_the_foreground_or_clipboard() {
        assert!(matches!(
            automatic_selected_text(
                true,
                SelectionTarget {
                    window: HWND::default(),
                    focus: HWND::default()
                },
                false,
                &|| false
            ),
            Err(PlatformError::NoSelection)
        ));
    }

    #[test]
    fn clipboard_owner_window_is_created_without_becoming_visible() {
        use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
        let window = ClipboardWindow::new().expect("clipboard owner window");
        assert!(!window.0.is_invalid());
        assert!(!unsafe { IsWindowVisible(window.0) }.as_bool());
    }

    #[test]
    #[ignore = "requires permission to create an isolated Win32 window station; never uses the user's clipboard"]
    fn clipboard_transactions_on_an_isolated_window_station() {
        const CHILD: &str = "EASYDICT_PRIVATE_CLIPBOARD_TEST";
        if std::env::var_os(CHILD).is_none() {
            use std::os::windows::process::CommandExt;
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "platform::selection::tests::clipboard_transactions_on_an_isolated_window_station",
                    "--ignored",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            return;
        }

        use windows::Win32::Graphics::Gdi::CreateBitmap;
        use windows::Win32::System::DataExchange::RegisterClipboardFormatW;
        use windows::Win32::System::StationsAndDesktops::{
            CreateDesktopW, CreateWindowStationW, DESKTOP_CREATEWINDOW, DESKTOP_READOBJECTS,
            DESKTOP_WRITEOBJECTS, SetProcessWindowStation, SetThreadDesktop,
        };
        use windows::Win32::UI::WindowsAndMessaging::{
            WINSTA_ACCESSCLIPBOARD, WINSTA_CREATEDESKTOP, WINSTA_READATTRIBUTES,
        };
        use windows::core::{HSTRING, PCWSTR};

        // Fail before touching any clipboard if isolation is unavailable. The
        // child owns the station/desktop; the OS tears both down on child exit.
        unsafe {
            let name = HSTRING::from(format!("EasydictClipboardTest-{}", std::process::id()));
            let access =
                (WINSTA_ACCESSCLIPBOARD | WINSTA_CREATEDESKTOP | WINSTA_READATTRIBUTES) as u32;
            let station = CreateWindowStationW(&name, 0, access, None).unwrap();
            SetProcessWindowStation(station).unwrap();
            let desktop = CreateDesktopW(
                w!("clipboard-test"),
                PCWSTR::null(),
                None,
                Default::default(),
                DESKTOP_CREATEWINDOW.0 | DESKTOP_READOBJECTS.0 | DESKTOP_WRITEOBJECTS.0,
                None,
            )
            .unwrap();
            SetThreadDesktop(desktop).unwrap();
        }

        fn put_bytes(format: u32, bytes: &[u8]) {
            unsafe {
                let handle = GlobalAlloc(GMEM_MOVEABLE, bytes.len()).unwrap();
                let pointer = GlobalLock(handle);
                assert!(!pointer.is_null());
                std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast(), bytes.len());
                let _ = GlobalUnlock(handle);
                SetClipboardData(format, Some(HANDLE(handle.0))).unwrap();
            }
        }
        fn put_text(text: &str) {
            let bytes: Vec<u8> = text
                .encode_utf16()
                .chain([0])
                .flat_map(u16::to_le_bytes)
                .collect();
            put_bytes(13, &bytes);
        }
        fn replace_text(window: HWND, text: &str) -> u32 {
            let _open = ClipboardOpen::acquire(window).unwrap();
            unsafe { EmptyClipboard() }.unwrap();
            put_text(text);
            unsafe { GetClipboardSequenceNumber() }
        }

        let owner = ClipboardWindow::new().unwrap();
        let rich = unsafe { RegisterClipboardFormatW(w!("Easydict test RTF")) };
        assert_ne!(rich, 0);
        {
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            unsafe { EmptyClipboard() }.unwrap();
            put_text("original clipboard");
            put_bytes(rich, b"{\\rtf1 original}\0");
        }
        let mut backup = ClipboardBackup::capture().expect("all global formats are backed up");
        let copied = replace_text(owner.0, "selected text");
        {
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            assert!(clipboard_belongs_to(owner.0));
        }
        backup.restore(copied).unwrap();
        {
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            assert_eq!(unsafe { GetClipboardOwner() }.unwrap(), backup.window.0);
            assert_eq!(
                unsafe { read_clipboard_text() }.as_deref(),
                Some("original clipboard")
            );
            let handle = HGLOBAL(unsafe { GetClipboardData(rich) }.unwrap().0);
            let pointer = unsafe { GlobalLock(handle) };
            assert!(!pointer.is_null());
            let bytes = unsafe { std::slice::from_raw_parts(pointer as *const u8, 17) };
            assert_eq!(bytes, b"{\\rtf1 original}\0");
            let _ = unsafe { GlobalUnlock(handle) };
        }

        // A newer copy must win even if it happens immediately before restore.
        let mut backup = ClipboardBackup::capture().unwrap();
        let copied = replace_text(owner.0, "selected text");
        replace_text(owner.0, "newer user copy");
        assert!(matches!(
            backup.restore(copied),
            Err(PlatformError::NoSelection)
        ));
        {
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            assert_eq!(
                unsafe { read_clipboard_text() }.as_deref(),
                Some("newer user copy")
            );
        }

        // Text alongside an unsupported format must NOT allow a partial backup.
        {
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            let bitmap = unsafe { CreateBitmap(1, 1, 1, 32, None) };
            assert!(!bitmap.is_invalid());
            unsafe { SetClipboardData(2, Some(HANDLE(bitmap.0))) }.unwrap();
        }
        let before = unsafe { GetClipboardSequenceNumber() };
        assert!(ClipboardBackup::capture().is_none());
        assert!(paste_with_clipboard("replacement", &|| true, || panic!("unsupported format must prevent paste")).is_err());
        assert_eq!(unsafe { GetClipboardSequenceNumber() }, before);
        {
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            assert_eq!(
                unsafe { read_clipboard_text() }.as_deref(),
                Some("newer user copy")
            );
            assert!(!unsafe { GetClipboardData(2) }.unwrap().is_invalid());
            unsafe { EmptyClipboard() }.unwrap();
        }

        let mut empty = ClipboardBackup::capture().unwrap();
        empty
            .restore(replace_text(owner.0, "temporary selection"))
            .unwrap();
        let _open = ClipboardOpen::acquire(owner.0).unwrap();
        assert_eq!(unsafe { EnumClipboardFormats(0) }, 0);
        drop(_open);

        // These closures stand in for SendInput: exercise clipboard ownership,
        // rollback and concurrent writes without sending keys to any desktop.
        paste_with_clipboard("temporary", &|| true, || Ok(())).unwrap();
        {
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            assert_eq!(unsafe { EnumClipboardFormats(0) }, 0, "empty clipboard restored");
            put_text("original clipboard");
            put_bytes(rich, b"{\\rtf1 original}\0");
        }
        for fail in [false, true] {
            let result = paste_with_clipboard("replacement", &|| true, || {
                let _open = ClipboardOpen::acquire(owner.0).unwrap();
                assert_eq!(unsafe { read_clipboard_text() }.as_deref(), Some("replacement"));
                if fail { Err(PlatformError::Platform("simulated SendInput failure".into())) } else { Ok(()) }
            });
            assert_eq!(result.is_err(), fail);
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            assert_eq!(unsafe { read_clipboard_text() }.as_deref(), Some("original clipboard"));
            assert!(!unsafe { GetClipboardData(rich) }.unwrap().is_invalid(), "RTF restored after success or failure");
        }
        let checks = std::cell::Cell::new(0);
        let valid = || { checks.set(checks.get() + 1); checks.get() <= 2 };
        assert!(paste_with_clipboard("replacement", &valid, || panic!("focus changed before paste")).is_err());
        {
            let _open = ClipboardOpen::acquire(owner.0).unwrap();
            assert_eq!(unsafe { read_clipboard_text() }.as_deref(), Some("original clipboard"));
        }
        let mut backup = ClipboardBackup::capture().unwrap();
        replace_text(owner.0, "copy before write");
        assert!(backup.write_text("replacement", &|| true).is_err());
        paste_with_clipboard("replacement", &|| true, || {
            replace_text(owner.0, "copy during paste");
            Ok(())
        }).unwrap();
        let _open = ClipboardOpen::acquire(owner.0).unwrap();
        assert_eq!(unsafe { read_clipboard_text() }.as_deref(), Some("copy during paste"));
    }

    #[test]
    fn duplicate_global_copies_the_whole_block() {
        let bytes = b"hello selection";
        let source = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }.unwrap();
        let target = unsafe { GlobalLock(source) };
        assert!(!target.is_null());
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), target as *mut u8, bytes.len());
            let _ = GlobalUnlock(source);
        }

        let copy = unsafe { duplicate_global(source) }.expect("a global block is copyable");
        assert_ne!(copy.0, source.0);
        assert_eq!(unsafe { GlobalSize(copy) }, bytes.len());
        let copied = unsafe { GlobalLock(copy) };
        assert!(!copied.is_null());
        let read = unsafe { std::slice::from_raw_parts(copied as *const u8, bytes.len()) };
        assert_eq!(read, bytes);
        unsafe {
            let _ = GlobalUnlock(copy);
            let _ = GlobalFree(Some(copy));
            let _ = GlobalFree(Some(source));
        }
    }

    #[test]
    fn console_windows_never_receive_the_synthetic_copy() {
        assert!(is_console_class("ConsoleWindowClass"));
        assert!(is_console_class("cascadia_hosting_window_class"));
        assert!(is_console_class("mintty"));
        assert!(!is_console_class("Chrome_WidgetWin_1"));
        assert!(!is_console_class(""));
    }

    #[test]
    fn a_null_handle_is_not_a_global_block() {
        assert!(unsafe { duplicate_global(HGLOBAL(std::ptr::null_mut())) }.is_none());
    }
}
