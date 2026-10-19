//! Event-driven selection icon. No extra WebView, clipboard polling or UIA
//! work in low-level hooks. One bounded worker reads only after a gesture.

use std::{
    cell::{Cell, RefCell},
    sync::{
        Arc, Weak,
        atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering},
    },
    time::Instant,
};

use parking_lot::Mutex;
use tauri::Emitter;
use unicode_segmentation::UnicodeSegmentation;
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent},
            HiDpi::{
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow,
                SetThreadDpiAwarenessContext,
            },
            Input::KeyboardAndMouse::*,
            WindowsAndMessaging::*,
        },
    },
    core::w,
};

use super::selection::{SelectionTarget, automatic_selected_text};
use crate::{
    commands::ActionPayload,
    config::Settings,
    model::{Language, WindowType},
    state::AppState,
};

const CONFIGURE: u32 = WM_APP + 1;
const READ_READY: u32 = WM_APP + 2;
const POINTER_DOWN: u32 = WM_APP + 3;
const POINTER_UP: u32 = WM_APP + 4;
const POINTER_MOVE: u32 = WM_APP + 5;
const DISMISS: u32 = WM_APP + 6;
const SELECT_ALL: u32 = WM_APP + 7;
const READ_TIMER: usize = 1;

thread_local! {
    static HOOK_WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static TRACK_MOVES: Cell<bool> = const { Cell::new(false) };
    static LAST_MOVE: Cell<u32> = const { Cell::new(0) };
    static SELECT_ALL_HELD: Cell<bool> = const { Cell::new(false) };
    static WATCH_INPUT: Cell<bool> = const { Cell::new(false) };
}

#[derive(Clone, PartialEq)]
struct Options {
    enabled: bool,
    force: bool,
    min_length: usize,
    excluded: Language,
    hover: bool,
    auto_query: bool,
    target: WindowType,
}

impl From<&Settings> for Options {
    fn from(settings: &Settings) -> Self {
        Self {
            enabled: settings.auto_show_query_icon,
            force: settings.enable_force_get_selected_text,
            min_length: settings.auto_show_query_icon_min_text_length.clamp(0, 50) as usize,
            excluded: settings.auto_show_query_icon_excluded_language,
            hover: !settings.click_query || !settings.hide_main_window,
            auto_query: settings.auto_query_selected_text,
            target: settings.mouse_select_translate_window_type,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub active: bool,
    pub error: Option<String>,
}

struct Reply {
    generation: u64,
    text: Option<String>,
}

struct Shared {
    window: AtomicIsize,
    generation: AtomicU64,
    busy: AtomicBool,
    stopped: AtomicBool,
    options: Mutex<Options>,
    status: Mutex<Status>,
    reply: Mutex<Option<Reply>>,
}

impl Shared {
    fn post(&self, message: u32) {
        let window = self.window.load(Ordering::Acquire);
        if window != 0 {
            let _ = unsafe {
                PostMessageW(Some(HWND(window as *mut _)), message, WPARAM(0), LPARAM(0))
            };
        }
    }
}

pub struct AutoSelection(Arc<Shared>);

impl AutoSelection {
    pub fn start(state: &Arc<AppState>) -> Self {
        let shared = Arc::new(Shared {
            window: AtomicIsize::new(0),
            generation: AtomicU64::new(0),
            busy: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            options: Mutex::new(Options::from(&state.settings())),
            status: Mutex::new(Status::default()),
            reply: Mutex::new(None),
        });
        let worker_shared = shared.clone();
        let state = Arc::downgrade(state);
        if let Err(error) = std::thread::Builder::new()
            .name("selection-events".into())
            .spawn(move || {
                if let Err(error) = run_loop(state.clone(), worker_shared.clone()) {
                    *worker_shared.status.lock() = Status {
                        active: false,
                        error: Some(error),
                    };
                    notify_status(&state);
                }
                worker_shared.window.store(0, Ordering::Release);
            })
        {
            shared.status.lock().error = Some(error.to_string());
        }
        Self(shared)
    }

    pub fn configure(&self, settings: &Settings) {
        let next = Options::from(settings);
        let mut options = self.0.options.lock();
        if *options != next {
            *options = next;
            self.0.generation.fetch_add(1, Ordering::AcqRel);
            self.0.post(CONFIGURE);
        }
    }

    pub fn status(&self) -> Status {
        let mut status = self.0.status.lock().clone();
        status.active &= self.0.window.load(Ordering::Acquire) != 0;
        status
    }
}

fn notify_status(state: &Weak<AppState>) {
    if let Some(state) = state.upgrade() {
        let _ = state.app.emit_to("settings", "desktop-status-changed", ());
    }
}

impl Drop for AutoSelection {
    fn drop(&mut self) {
        self.0.stopped.store(true, Ordering::Release);
        self.0.generation.fetch_add(1, Ordering::AcqRel);
        self.0.post(WM_CLOSE);
    }
}

#[derive(Default)]
struct Gesture {
    down: Option<(i32, i32)>,
    previous_up: Option<((i32, i32), u64)>,
    multiple: bool,
    shift: bool,
}

impl Gesture {
    fn down(&mut self, point: (i32, i32), now: u64, double_click_ms: u32, shift: bool) {
        self.multiple = self.previous_up.is_some_and(|(last, time)| {
            now.saturating_sub(time) <= double_click_ms as u64 && nearby(point, last, 4)
        });
        self.down = Some(point);
        self.shift = shift;
    }

    fn up(&mut self, point: (i32, i32), now: u64) -> Option<(i32, i32)> {
        let trigger = self
            .down
            .take()
            .filter(|start| !nearby(point, *start, 4) || self.multiple || self.shift);
        self.previous_up = Some((point, now));
        trigger
    }
}

fn nearby(a: (i32, i32), b: (i32, i32), radius: i64) -> bool {
    (a.0 as i64 - b.0 as i64).abs() <= radius && (a.1 as i64 - b.1 as i64).abs() <= radius
}

fn eligible(text: &str, detected: Language, options: &Options) -> bool {
    options.enabled
        && !text.is_empty()
        && text.graphemes(true).count() >= options.min_length
        && (options.excluded == Language::Auto || options.excluded != detected)
}

fn placement(point: (i32, i32), work: RECT, size: i32) -> (i32, i32) {
    let x = (point.0 as i64 + 12).clamp(
        work.left as i64,
        (work.right as i64 - size as i64).max(work.left as i64),
    );
    let y = (point.1 as i64 + 12).clamp(
        work.top as i64,
        (work.bottom as i64 - size as i64).max(work.top as i64),
    );
    (x as i32, y as i32)
}

struct Hooks {
    mouse: HHOOK,
    keyboard: HHOOK,
    foreground: HWINEVENTHOOK,
}

impl Hooks {
    fn install() -> windows::core::Result<Self> {
        let mut hooks = Self {
            mouse: HHOOK::default(),
            keyboard: HHOOK::default(),
            foreground: HWINEVENTHOOK::default(),
        };
        unsafe {
            let module = GetModuleHandleW(None)?.into();
            hooks.mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), Some(module), 0)?;
            hooks.keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_hook), Some(module), 0)?;
            hooks.foreground = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(foreground_hook),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            if hooks.foreground.is_invalid() {
                return Err(windows::core::Error::from_thread());
            }
        }
        Ok(hooks)
    }
}

impl Drop for Hooks {
    fn drop(&mut self) {
        unsafe {
            if !self.mouse.is_invalid() {
                let _ = UnhookWindowsHookEx(self.mouse);
            }
            if !self.keyboard.is_invalid() {
                let _ = UnhookWindowsHookEx(self.keyboard);
            }
            if !self.foreground.is_invalid() {
                let _ = UnhookWinEvent(self.foreground);
            }
        }
    }
}

fn post_hook(message: u32, point: (i32, i32)) {
    HOOK_WINDOW.with(|window| {
        let _ = unsafe {
            PostMessageW(
                Some(window.get()),
                message,
                WPARAM(point.0 as u32 as usize),
                LPARAM(point.1 as isize),
            )
        };
    });
}

unsafe extern "system" fn mouse_hook(code: i32, word: WPARAM, long: LPARAM) -> LRESULT {
    if code >= 0 {
        let data = unsafe { &*(long.0 as *const MSLLHOOKSTRUCT) };
        if data.flags & LLMHF_INJECTED == 0 {
            let point = (data.pt.x, data.pt.y);
            match word.0 as u32 {
                WM_LBUTTONDOWN => post_hook(POINTER_DOWN, point),
                WM_LBUTTONUP => post_hook(POINTER_UP, point),
                WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                    post_hook(DISMISS, point)
                }
                WM_MOUSEMOVE
                    if TRACK_MOVES.get() && data.time.wrapping_sub(LAST_MOVE.get()) >= 100 =>
                {
                    LAST_MOVE.set(data.time);
                    post_hook(POINTER_MOVE, point);
                }
                _ => {}
            }
        }
    }
    unsafe { CallNextHookEx(None, code, word, long) }
}

fn key_down(key: VIRTUAL_KEY) -> bool {
    (unsafe { GetAsyncKeyState(key.0 as i32) } as u16) & 0x8000 != 0
}

unsafe extern "system" fn key_hook(code: i32, word: WPARAM, long: LPARAM) -> LRESULT {
    if code >= 0 {
        let key = unsafe { &*(long.0 as *const KBDLLHOOKSTRUCT) };
        if !key.flags.contains(LLKHF_INJECTED) {
            let down = matches!(word.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
            if key.vkCode == VK_A.0 as u32 && !down {
                SELECT_ALL_HELD.set(false);
            }
            if down {
                let control = key_down(VK_CONTROL);
                let plain_control = control
                    && !key_down(VK_MENU)
                    && !key_down(VK_SHIFT)
                    && !key_down(VK_LWIN)
                    && !key_down(VK_RWIN);
                if plain_control && key.vkCode == VK_A.0 as u32 {
                    if !SELECT_ALL_HELD.replace(true) {
                        post_hook(SELECT_ALL, (0, 0));
                    }
                } else if WATCH_INPUT.get()
                    && !(plain_control && key.vkCode == VK_C.0 as u32)
                    && !matches!(key.vkCode, 0x10..=0x12 | 0x5b..=0x5c | 0xa0..=0xa5)
                {
                    post_hook(DISMISS, (0, 0));
                }
            }
        }
    }
    unsafe { CallNextHookEx(None, code, word, long) }
}

unsafe extern "system" fn foreground_hook(
    _: HWINEVENTHOOK,
    _: u32,
    _: HWND,
    _: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    if WATCH_INPUT.get() {
        post_hook(DISMISS, (0, 0));
    }
}

struct Pending {
    generation: u64,
    target: SelectionTarget,
    point: (i32, i32),
    start: (i32, i32),
    select_all: bool,
}

struct Controller {
    shared: Arc<Shared>,
    state: Weak<AppState>,
    hooks: Option<Hooks>,
    gesture: Gesture,
    clock: Instant,
    pending: Option<Pending>,
    selected: Option<String>,
    area: RECT,
    visible: bool,
}

impl Controller {
    fn dismiss(&mut self, window: HWND) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        self.pending = None;
        self.selected = None;
        self.visible = false;
        TRACK_MOVES.set(false);
        WATCH_INPUT.set(false);
        unsafe {
            let _ = KillTimer(Some(window), READ_TIMER);
            let _ = ShowWindow(window, SW_HIDE);
        }
    }

    fn configure(&mut self, window: HWND) {
        self.dismiss(window);
        self.gesture = Gesture::default();
        SELECT_ALL_HELD.set(false);
        if self.shared.options.lock().enabled {
            if self.hooks.is_none() {
                match Hooks::install() {
                    Ok(hooks) => {
                        self.hooks = Some(hooks);
                        *self.shared.status.lock() = Status {
                            active: true,
                            error: None,
                        };
                    }
                    Err(error) => {
                        *self.shared.status.lock() = Status {
                            active: false,
                            error: Some(format!("无法启用划词监听：{error}")),
                        }
                    }
                }
            }
        } else {
            self.hooks = None;
            *self.shared.status.lock() = Status::default();
        }
        notify_status(&self.state);
    }

    fn schedule(&mut self, window: HWND, start: (i32, i32), point: (i32, i32), select_all: bool) {
        self.dismiss(window);
        if !self.shared.options.lock().enabled {
            return;
        }
        let Some(target) = SelectionTarget::current() else {
            return;
        };
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(target.window, Some(&mut pid));
        }
        if pid == 0 || pid == std::process::id() {
            return;
        }
        self.pending = Some(Pending {
            generation: self.shared.generation.load(Ordering::Acquire),
            target,
            point,
            start,
            select_all,
        });
        WATCH_INPUT.set(true);
        unsafe {
            SetTimer(Some(window), READ_TIMER, 100, None);
        }
    }

    fn read(&mut self, window: HWND) {
        unsafe {
            let _ = KillTimer(Some(window), READ_TIMER);
        }
        let Some(pending) = &self.pending else { return };
        if !pending.target.is_current() {
            self.dismiss(window);
            return;
        }
        if self.shared.busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let shared = self.shared.clone();
        let source = (
            pending.target.window.0 as isize,
            pending.target.focus.0 as isize,
        );
        let generation = pending.generation;
        let select_all = pending.select_all;
        let force = self.shared.options.lock().force;
        let spawned = std::thread::Builder::new()
            .name("selection-read".into())
            .spawn(move || {
                struct Lease(Arc<Shared>);
                impl Drop for Lease {
                    fn drop(&mut self) {
                        self.0.busy.store(false, Ordering::Release);
                        self.0.post(READ_READY);
                    }
                }
                let _lease = Lease(shared.clone());
                if shared.generation.load(Ordering::Acquire) != generation {
                    return;
                }
                let target = SelectionTarget {
                    window: HWND(source.0 as *mut _),
                    focus: HWND(source.1 as *mut _),
                };
                let valid = || shared.generation.load(Ordering::Acquire) == generation;
                let text = automatic_selected_text(force, target, select_all, &valid)
                    .ok()
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty());
                *shared.reply.lock() = Some(Reply { generation, text });
            });
        if let Err(error) = spawned {
            self.shared.busy.store(false, Ordering::Release);
            log::warn!("selection reader could not start: {error}");
        }
    }

    fn ready(&mut self, window: HWND) {
        let reply = self.shared.reply.lock().take();
        let Some(pending) = &self.pending else { return };
        let generation = self.shared.generation.load(Ordering::Acquire);
        if generation != pending.generation || !pending.target.is_current() {
            self.dismiss(window);
            return;
        }
        let Some(reply) = reply.filter(|reply| reply.generation == generation) else {
            // Only the newest gesture survives a busy UIA worker.
            self.read(window);
            return;
        };
        let Some(text) = reply.text else { return };
        let Some(state) = self.state.upgrade() else {
            return;
        };
        if !eligible(
            &text,
            state.detect_language(&text),
            &self.shared.options.lock(),
        ) {
            return;
        }
        self.selected = Some(text);
        self.show(window);
    }

    fn show(&mut self, window: HWND) {
        let Some(pending) = &self.pending else { return };
        unsafe {
            let point = POINT {
                x: pending.point.0,
                y: pending.point.1,
            };
            let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if !GetMonitorInfoW(monitor, &mut info).as_bool() {
                return;
            }
            let _ = SetWindowPos(
                window,
                Some(HWND_TOPMOST),
                point.x,
                point.y,
                1,
                1,
                SWP_NOACTIVATE,
            );
            let size = (32 * GetDpiForWindow(window).max(96) / 96) as i32;
            let (x, y) = placement(pending.point, info.rcWork, size);
            self.area = RECT {
                left: x,
                top: y,
                right: x + size,
                bottom: y + size,
            };
            if SetWindowPos(
                window,
                Some(HWND_TOPMOST),
                x,
                y,
                size,
                size,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
            .is_ok()
            {
                self.visible = true;
                TRACK_MOVES.set(true);
                let _ = InvalidateRect(Some(window), None, true);
            }
        }
    }

    fn activate(&mut self, window: HWND) {
        let Some(pending) = &self.pending else { return };
        let options = self.shared.options.lock().clone();
        if !options.enabled
            || pending.generation != self.shared.generation.load(Ordering::Acquire)
            || !pending.target.is_current()
        {
            self.dismiss(window);
            return;
        }
        let Some(text) = self.selected.take() else {
            return;
        };
        self.dismiss(window);
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let action = if options.auto_query {
            "selectedText"
        } else {
            "selectedTextPreview"
        };
        crate::deliver(&state.app, options.target.label(), ActionPayload::new(action, Some(text)));
    }

    fn point_inside(&self, point: (i32, i32)) -> bool {
        self.visible
            && point.0 >= self.area.left
            && point.0 < self.area.right
            && point.1 >= self.area.top
            && point.1 < self.area.bottom
    }
}

fn run_loop(state: Weak<AppState>, shared: Arc<Shared>) -> Result<(), String> {
    if shared.stopped.load(Ordering::Acquire) {
        return Ok(());
    }
    unsafe {
        let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        if previous.0.is_null() {
            return Err("无法启用划词图标的 DPI 感知".into());
        }
        let instance = GetModuleHandleW(None).map_err(|e| e.to_string())?.into();
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            hCursor: LoadCursorW(None, IDC_HAND).map_err(|e| e.to_string())?,
            lpszClassName: w!("EasydictSelectionIcon"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_thread().to_string());
        }
        let controller = RefCell::new(Controller {
            shared: shared.clone(),
            state,
            hooks: None,
            gesture: Gesture::default(),
            clock: Instant::now(),
            pending: None,
            selected: None,
            area: RECT::default(),
            visible: false,
        });
        let window = CreateWindowExW(
            WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            w!("EasydictSelectionIcon"),
            w!("翻译选中的文字"),
            WS_POPUP,
            0,
            0,
            32,
            32,
            None,
            None,
            Some(instance),
            Some((&controller as *const RefCell<Controller>).cast()),
        )
        .map_err(|e| e.to_string())?;
        HOOK_WINDOW.set(window);
        shared.window.store(window.0 as isize, Ordering::Release);
        if !shared.stopped.load(Ordering::Acquire) {
            controller.borrow_mut().configure(window);
        }
        if shared.stopped.load(Ordering::Acquire) {
            shared.post(WM_CLOSE);
        }
        let mut message = MSG::default();
        let error = loop {
            let result = GetMessageW(&mut message, None, 0, 0).0;
            if result == 0 {
                break None;
            }
            if result == -1 {
                break Some(windows::core::Error::from_thread().to_string());
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        };
        controller.borrow_mut().hooks = None;
        shared.generation.fetch_add(1, Ordering::AcqRel);
        shared.window.store(0, Ordering::Release);
        HOOK_WINDOW.set(HWND::default());
        if IsWindow(Some(window)).as_bool() {
            let _ = DestroyWindow(window);
        }
        let _ = UnregisterClassW(w!("EasydictSelectionIcon"), Some(instance));
        SetThreadDpiAwarenessContext(previous);
        if let Some(error) = error {
            return Err(error);
        }
    }
    Ok(())
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    word: WPARAM,
    long: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(long.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
            return LRESULT(1);
        }
        if message == WM_MOUSEACTIVATE {
            return LRESULT(MA_NOACTIVATE as isize);
        }
        if message == WM_PAINT {
            paint(window);
            return LRESULT(0);
        }
        if message == WM_CLOSE {
            let _ = DestroyWindow(window);
            return LRESULT(0);
        }
        if message == WM_DESTROY {
            PostQuitMessage(0);
            return LRESULT(0);
        }
        if message == WM_NCDESTROY {
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
        }
        let pointer = GetWindowLongPtrW(window, GWLP_USERDATA) as *const RefCell<Controller>;
        let Some(controller) = pointer.as_ref() else {
            return DefWindowProcW(window, message, word, long);
        };
        let Ok(mut controller) = controller.try_borrow_mut() else {
            return DefWindowProcW(window, message, word, long);
        };
        let point = (word.0 as u32 as i32, long.0 as i32);
        match message {
            CONFIGURE => controller.configure(window),
            READ_READY => controller.ready(window),
            DISMISS | WM_DISPLAYCHANGE => controller.dismiss(window),
            WM_TIMER if word.0 == READ_TIMER => controller.read(window),
            POINTER_DOWN => {
                if !controller.point_inside(point) {
                    controller.dismiss(window);
                    let now = controller.clock.elapsed().as_millis() as u64;
                    controller
                        .gesture
                        .down(point, now, GetDoubleClickTime(), key_down(VK_SHIFT));
                }
            }
            POINTER_UP => {
                if !controller.point_inside(point) {
                    let now = controller.clock.elapsed().as_millis() as u64;
                    if let Some(start) = controller.gesture.up(point, now) {
                        controller.schedule(window, start, point, false);
                    }
                }
            }
            SELECT_ALL => {
                let mut cursor = POINT::default();
                if GetCursorPos(&mut cursor).is_ok() {
                    controller.schedule(window, (cursor.x, cursor.y), (cursor.x, cursor.y), true);
                }
            }
            POINTER_MOVE if controller.visible => {
                if let Some(pending) = &controller.pending {
                    let radius = (120 * GetDpiForWindow(window).max(96) / 96) as i64;
                    let inside = point.0 as i64
                        >= pending.start.0.min(pending.point.0) as i64 - radius
                        && point.0 as i64 <= pending.start.0.max(pending.point.0) as i64 + radius
                        && point.1 as i64 >= pending.start.1.min(pending.point.1) as i64 - radius
                        && point.1 as i64 <= pending.start.1.max(pending.point.1) as i64 + radius;
                    if !inside && !controller.point_inside(point) {
                        controller.dismiss(window);
                    }
                }
            }
            WM_LBUTTONUP => controller.activate(window),
            WM_MOUSEMOVE if controller.visible => {
                let hover = controller.shared.options.lock().hover;
                if hover {
                    controller.activate(window);
                }
            }
            WM_ERASEBKGND => return LRESULT(1),
            _ => return DefWindowProcW(window, message, word, long),
        }
        LRESULT(0)
    }
}

unsafe fn paint(window: HWND) {
    unsafe {
        let mut ps = PAINTSTRUCT::default();
        let dc = BeginPaint(window, &mut ps);
        let mut rect = RECT::default();
        let _ = GetClientRect(window, &mut rect);
        let brush = CreateSolidBrush(COLORREF(0x00bd6a21));
        FillRect(dc, &rect, brush);
        let _ = DeleteObject(brush.into());
        let font = CreateFontW(
            -((18 * GetDpiForWindow(window).max(96) / 96) as i32),
            0,
            0,
            0,
            600,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            0,
            w!("Segoe UI"),
        );
        let previous = SelectObject(dc, font.into());
        SetBkMode(dc, TRANSPARENT);
        SetTextColor(dc, COLORREF(0x00ffffff));
        DrawTextW(
            dc,
            &mut ['译' as u16],
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(dc, previous);
        let _ = DeleteObject(font.into());
        let _ = EndPaint(window, &ps);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_selection_gestures_request_a_read() {
        let mut gesture = Gesture::default();
        gesture.down((10, 10), 0, 500, false);
        assert!(gesture.up((10, 10), 10).is_none());
        gesture.down((10, 10), 90, 500, false);
        assert!(gesture.up((10, 10), 100).is_some());
        gesture.down((50, 50), 1000, 500, false);
        assert!(gesture.up((90, 50), 1100).is_some());
        gesture.down((0, 0), 2000, 500, true);
        assert!(gesture.up((0, 0), 2100).is_some());
        assert!(gesture.up((100, 100), 2200).is_none());
        assert!(!nearby((i32::MIN, 0), (i32::MAX, 0), 4));
    }

    #[test]
    fn icon_filters_count_graphemes_and_auto_means_no_language_exclusion() {
        let mut settings = Settings::default();
        settings.auto_show_query_icon_min_text_length = 2;
        let mut options = Options::from(&settings);
        assert!(!eligible("中", Language::SimplifiedChinese, &options));
        assert!(!eligible("中文", Language::SimplifiedChinese, &options));
        assert!(!eligible("e\u{301}", Language::English, &options));
        assert!(eligible("e\u{301}a", Language::English, &options));
        options.excluded = Language::Auto;
        assert!(eligible("中文", Language::SimplifiedChinese, &options));
        options.enabled = false;
        assert!(!eligible("hello", Language::English, &options));
    }

    #[test]
    fn popup_stays_in_the_target_work_area_with_negative_origins() {
        let work = RECT {
            left: -1920,
            top: -1080,
            right: 0,
            bottom: 0,
        };
        assert_eq!(placement((-15, -15), work, 64), (-64, -64));
        assert_eq!(placement((-1900, -1000), work, 32), (-1888, -988));
        assert_eq!(placement((i32::MAX, i32::MAX), work, 64), (-64, -64));
    }

    #[test]
    fn configuration_matches_the_upstream_activation_and_window_rules() {
        let mut settings = Settings::default();
        settings.click_query = true;
        settings.hide_main_window = true;
        settings.auto_query_selected_text = false;
        settings.mouse_select_translate_window_type = WindowType::Mini;
        let options = Options::from(&settings);
        assert!(!options.hover);
        assert!(!options.auto_query);
        assert_eq!(options.target, WindowType::Mini);
        settings.click_query = false;
        assert!(Options::from(&settings).hover);
        settings.click_query = true;
        settings.hide_main_window = false;
        assert!(Options::from(&settings).hover);
    }

    #[test]
    fn disabled_monitor_has_no_hooks_or_visible_window_and_shuts_down() {
        let mut settings = Settings::default();
        settings.auto_show_query_icon = false;
        let shared = Arc::new(Shared {
            window: AtomicIsize::new(0),
            generation: AtomicU64::new(0),
            busy: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            options: Mutex::new(Options::from(&settings)),
            status: Mutex::new(Status::default()),
            reply: Mutex::new(None),
        });
        let worker = shared.clone();
        let thread = std::thread::spawn(move || run_loop(Weak::new(), worker));
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while shared.window.load(Ordering::Acquire) == 0
            && !thread.is_finished()
            && Instant::now() < deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let handle = shared.window.load(Ordering::Acquire);
        assert_ne!(handle, 0, "native icon thread did not initialize");
        assert!(!unsafe { IsWindowVisible(HWND(handle as *mut _)) }.as_bool());
        assert!(!shared.status.lock().active);
        assert!(!shared.busy.load(Ordering::Acquire));
        let monitor = AutoSelection(shared.clone());
        monitor.configure(&settings);
        drop(monitor);
        thread.join().unwrap().unwrap();
        assert_eq!(shared.window.load(Ordering::Acquire), 0);
        assert!(!unsafe { IsWindow(Some(HWND(handle as *mut _))) }.as_bool());
    }
}
