//! Per-window geometry memory.
//!
//! Upstream keeps one rectangle per window type: `EZLayoutManager.m:53/59/65`
//! seeds `miniWindowFrame` / `fixedWindowFrame` / `mainWindowFrame` from
//! `windowFrameWithType:` on first use, `:217` writes the frame back whenever a
//! window changes, and `EZWindowManager.m:619/676` reads it for the "former"
//! placement so a window returns to where the user left it, across restarts.
//!
//! This port stores the same records in the settings document under
//! `windowFrames`, keyed by window type. The rectangle is the window's origin
//! plus its *client* size rather than macOS's whole frame: Windows applies
//! `set_size` to the client area, so storing the outer rectangle would add the
//! title bar and borders again on every restore.
//!
//! Two things are different from upstream, both on purpose:
//!
//! * A move or resize arrives on the event loop, so the frame is only recorded
//!   and handed to a worker thread. Persisting the whole settings document
//!   takes a lock and rewrites JSON, which must not happen on the main thread,
//!   and dragging a window raises a stream of events — the short wait in
//!   [`saver`] coalesces the events in each batch into one write.
//! * Only the query windows are remembered. The OCR window is placed around the
//!   region it captured, and the settings window is an ordinary desktop window
//!   whose size belongs to the user's layout rather than to a translation mode.
//!
//! A window that is asked to restore a frame it never recorded keeps whatever
//! position it has: [`stored`] answers `None` for a missing entry and for a
//! zero-sized one, which is exactly how upstream reads "no frame yet".

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

use crate::config::WindowFrame;
use crate::model::WindowType;

/// Minimum interval between geometry batches while events keep arriving.
const SETTLE: Duration = Duration::from_millis(500);

static PENDING: Mutex<Option<HashMap<WindowType, WindowFrame>>> = Mutex::new(None);
static SAVING: AtomicBool = AtomicBool::new(false);
// Serializes dequeue + disk commit, so exit flushing cannot overtake a batch.
// Geometry commits never wait on the event loop.
static WRITING: Mutex<()> = Mutex::new(());

/// The query window a Tauri label belongs to, or `None` for a window whose
/// geometry is not remembered.
pub fn window_type(label: &str) -> Option<WindowType> {
    match label {
        "main" => Some(WindowType::Main),
        "mini" => Some(WindowType::Mini),
        "fixed" => Some(WindowType::Fixed),
        _ => None,
    }
}

/// Records where a window is and how big its client area is, then persists it
/// off-thread.
///
/// Safe to call from the main thread: it only reads the current frame and hands
/// the record to a worker. A window that cannot be read is skipped rather than
/// recorded as a zero-sized frame. The position is recorded even for a window
/// another part of the app placed, because the next "former" placement has to
/// return to where the user left it.
pub fn note(app: &AppHandle, label: &str) {
    let Some(kind) = window_type(label) else {
        return;
    };
    let Some(window) = app.get_webview_window(label) else {
        return;
    };
    let (Ok(position), Ok(size)) = (window.outer_position(), window.inner_size()) else {
        return;
    };
    if size.width == 0 || size.height == 0 {
        return;
    }
    remember(
        kind,
        WindowFrame { x: position.x, y: position.y, width: size.width, height: size.height },
    );
    spawn_saver(app);
}

fn remember(kind: WindowType, frame: WindowFrame) {
    let mut pending = PENDING.lock();
    pending.get_or_insert_with(HashMap::new).insert(kind, frame);
}

/// Starts the writer unless one already runs. The running worker keeps draining
/// until nothing is pending, so a burst of moves costs one thread, not one per
/// event.
fn spawn_saver(app: &AppHandle) {
    if SAVING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || saver(app));
}

/// Persists the latest frames every [`SETTLE`] until the queue is empty.
fn saver(app: AppHandle) {
    loop {
        std::thread::sleep(SETTLE);
        let _writing = WRITING.lock();
        let frames = {
            let mut pending = PENDING.lock();
            match pending.as_mut() {
                Some(frames) if !frames.is_empty() => std::mem::take(frames),
                _ => {
                    // Clear the flag while holding PENDING. The next producer
                    // can only enqueue after this guard drops, then starts a
                    // new worker. Locking PENDING again here deadlocks both the
                    // saver and subsequent main-thread move/resize callbacks.
                    SAVING.store(false, Ordering::Release);
                    return;
                }
            }
        };
        commit(&app, frames);
    }
}

/// Writes the frames into the settings document, one commit for the batch.
///
/// A failure is logged and forgotten: the window is already where the user put
/// it, so the next move records the frame again. Blocking here would keep a
/// worker alive on a locked settings file, which is worse than a lost frame.
fn commit(app: &AppHandle, frames: HashMap<WindowType, WindowFrame>) {
    let Some(state) = app.try_state::<std::sync::Arc<crate::state::AppState>>() else {
        // The native harness builds the app without a settings store, and the
        // OCR probe window never reaches this path.
        return;
    };
    if let Err(error) = state.save_window_frames(&frames) {
        log::debug!("could not remember the window geometry: {error}");
    }
}

/// Waits for an in-flight batch, then commits any newer pending frames before
/// exit. The worker remains responsible for clearing SAVING if the app stays
/// alive; clearing it here could start a second worker while the first sleeps.
pub fn flush(app: &AppHandle) {
    let _writing = WRITING.lock();
    let frames = {
        let mut pending = PENDING.lock();
        match pending.as_mut() {
            Some(frames) if !frames.is_empty() => Some(std::mem::take(frames)),
            _ => None,
        }
    };
    if let Some(frames) = frames {
        commit(app, frames);
    }
}

/// The frame remembered for a window, if it has one that can be applied.
pub fn stored(settings: &crate::config::Settings, label: &str) -> Option<WindowFrame> {
    let kind = window_type(label)?;
    let frame = *settings.window_frames.get(&kind)?;
    (!frame.is_empty()).then_some(frame)
}

/// Where a remembered frame asks the window to be, and how big.
///
/// The origin is used as the window's own coordinate space, so it stays
/// physical: moving a window to a different-DPI monitor must not rescale the
/// desktop coordinate, which is the same rule [`crate::window::position`]
/// follows for its other placements.
///
/// The size is the window's *client* size, because every size the port applies
/// goes through `WebviewWindow::set_size`, which Windows applies to the client
/// area. Storing the outer rectangle instead would add the title bar and borders
/// on every restart, since a restored window measures its client area back and
/// the frame sits outside it.
pub fn placement(frame: &WindowFrame) -> (PhysicalPosition<i32>, PhysicalSize<u32>) {
    (
        PhysicalPosition::new(frame.x, frame.y),
        PhysicalSize::new(frame.width, frame.height),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(x: i32, y: i32, width: u32, height: u32) -> WindowFrame {
        WindowFrame { x, y, width, height }
    }

    #[test]
    fn only_the_query_windows_are_remembered() {
        assert_eq!(window_type("main"), Some(WindowType::Main));
        assert_eq!(window_type("mini"), Some(WindowType::Mini));
        assert_eq!(window_type("fixed"), Some(WindowType::Fixed));
        // The settings and OCR windows keep their own sizing.
        assert_eq!(window_type("settings"), None);
        assert_eq!(window_type("ocr"), None);
        assert_eq!(window_type("unknown"), None);
    }

    #[test]
    fn a_missing_or_zero_sized_frame_is_not_applied() {
        let mut settings = crate::config::Settings::default();
        assert_eq!(stored(&settings, "main"), None);
        assert_eq!(stored(&settings, "settings"), None);

        settings.window_frames.insert(WindowType::Main, frame(10, 20, 0, 300));
        assert_eq!(stored(&settings, "main"), None);
        settings.window_frames.insert(WindowType::Main, frame(10, 20, 300, 0));
        assert_eq!(stored(&settings, "main"), None);

        settings.window_frames.insert(WindowType::Main, frame(-1900, 40, 500, 300));
        assert_eq!(stored(&settings, "main"), Some(frame(-1900, 40, 500, 300)));
        // The frame of one window is never handed to another.
        assert_eq!(stored(&settings, "mini"), None);
    }

    #[test]
    fn a_recorded_frame_replaces_the_previous_one() {
        remember(WindowType::Mini, frame(1, 2, 3, 4));
        remember(WindowType::Mini, frame(5, 6, 7, 8));
        remember(WindowType::Fixed, frame(-1, -2, 9, 10));
        let mut pending = PENDING.lock();
        let frames = pending.as_mut().unwrap();
        assert_eq!(frames.get(&WindowType::Mini), Some(&frame(5, 6, 7, 8)));
        assert_eq!(frames.get(&WindowType::Fixed), Some(&frame(-1, -2, 9, 10)));
        // Leave the process-wide queue as it was found.
        frames.clear();
    }

    #[test]
    fn the_settings_document_round_trips_frames_keyed_by_window_type() {
        // The keys are the camelCase window labels, which is what the frame's
        // own restore path looks up; a rename would silently stop restoring.
        let mut settings = crate::config::Settings::default();
        settings.window_frames.insert(WindowType::Fixed, frame(-2560, 120, 420, 260));
        let document = serde_json::to_value(&settings).unwrap();
        assert_eq!(
            document["windowFrames"]["fixed"],
            serde_json::json!({ "x": -2560, "y": 120, "width": 420, "height": 260 })
        );

        let restored: crate::config::Settings = serde_json::from_value(document).unwrap();
        assert_eq!(stored(&restored, "fixed"), Some(frame(-2560, 120, 420, 260)));

        // A document without the field — every settings file written before
        // this round — opens with nothing remembered rather than an error.
        let old: crate::config::Settings = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(old.window_frames.is_empty());
        assert_eq!(stored(&old, "main"), None);
    }
}
