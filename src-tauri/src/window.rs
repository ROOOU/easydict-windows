//! Window placement and sizing.
//!
//! Ported from `EZWindowManager` + `EZShowWindowPosition`: the floating windows
//! follow the same four placements, and the translation window grows with its
//! results up to a share of the screen.

use std::collections::{HashMap, VecDeque};

use parking_lot::Mutex;
use tauri::{
    AppHandle, Emitter, LogicalSize, Manager, Monitor, PhysicalPosition, PhysicalRect, PhysicalSize,
    WebviewWindow,
};

use crate::{commands::ActionPayload, config::Settings};

const LABELS: &[&str] = &["main", "mini", "fixed", "settings", "ocr"];
const MAX_PENDING: usize = 32;
const MAX_ACTION_BYTES: usize = 256 * 1024;

#[derive(Default)]
pub struct Windows {
    // Only worker threads take this lock. Main-thread callbacks must never
    // wait for a builder: WebView2 creation needs the event loop to keep running.
    creating: Mutex<()>,
    pending: Mutex<HashMap<String, VecDeque<ActionPayload>>>,
}

impl Windows {
    fn push(&self, label: &str, payload: ActionPayload) -> Result<(), String> {
        if !LABELS.contains(&label) {
            return Err(format!("unknown window: {label}"));
        }
        if payload.action.len() > 64 {
            return Err("invalid action name".into());
        }
        if payload.text.as_ref().is_some_and(|text| text.len() > MAX_ACTION_BYTES) {
            return Err("action text exceeds 256 KiB".into());
        }
        let mut pending = self.pending.lock();
        let queue = pending.entry(label.into()).or_default();
        if queue.len() >= MAX_PENDING {
            return Err(format!("{label}: too many pending actions; please retry"));
        }
        queue.push_back(payload);
        Ok(())
    }

    pub fn next(&self, label: &str) -> Option<ActionPayload> {
        self.pending.lock().get_mut(label)?.pop_front()
    }
}

/// Worker-thread only, including callers of `show`. Creates no other WebViews.
pub fn ensure(app: &AppHandle, label: &str) -> Result<WebviewWindow, String> {
    if !LABELS.contains(&label) {
        return Err(format!("unknown window: {label}"));
    }
    let windows = app.state::<Windows>();
    let _creating = windows.creating.lock();
    if let Some(window) = app.get_webview_window(label) {
        return Ok(window);
    }
    let config = app.config().app.windows.iter().find(|config| config.label == label)
        .ok_or_else(|| format!("no window configuration: {label}"))?;
    tauri::WebviewWindowBuilder::from_config(app, config)
        .map_err(|error| error.to_string())?
        .visible(false)
        .focused(false)
        .build()
        .map_err(|error| error.to_string())
}

pub fn report_error(app: &AppHandle, error: &str) {
    use tauri_plugin_notification::NotificationExt;
    log::error!("window action failed: {error}");
    if let Err(notification_error) = app.notification().builder()
        .title("Easydict")
        .body(format!("窗口操作失败，请重试：{error}"))
        .show()
    {
        log::warn!("could not display action error: {notification_error}");
    }
}

/// Safe in tray/shortcut/native callbacks: never blocks their event loop.
pub fn request_show(app: &AppHandle, label: &str) {
    let app = app.clone();
    let label = label.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        // The settings only supply a stored position for the floating windows;
        // the native harness builds the app without a settings store, and a
        // window that cannot be positioned yet still has to open.
        let settings = app
            .try_state::<std::sync::Arc<crate::state::AppState>>()
            .map(|state| state.settings())
            .unwrap_or_default();
        if let Err(error) = show(&app, &settings, &label, None) {
            report_error(&app, &error);
        }
    });
}

/// The queue, not the event, owns actions. A loading WebView pulls the first
/// item after subscribing; events merely wake an already loaded consumer.
pub fn enqueue(app: &AppHandle, label: &str, payload: ActionPayload) -> Result<(), String> {
    app.state::<Windows>().push(label, payload)?;
    let app = app.clone();
    let label = label.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let result = ensure(&app, &label).and_then(|window| {
            // A suspended WebView cannot run the action, and the OCR window is
            // expected to work while hidden.
            crate::platform::webview_memory::wake_for_action(&window);
            window.emit_to(label.as_str(), crate::commands::events::ACTION, ()).map_err(|error| error.to_string())
        });
        if let Err(error) = result {
            // Keep accepted actions queued: the next open/action retries creation.
            report_error(&app, &error);
        }
    });
    Ok(())
}

pub fn show(
    app: &AppHandle,
    settings: &Settings,
    label: &str,
    placement: Option<&str>,
) -> Result<(), String> {
    let window = ensure(app, label)?;
    // A window hidden earlier had its WebView suspended; wake it before it is
    // shown so the first frame runs script.
    crate::platform::webview_memory::activate(&window);
    let stored = match label {
        "fixed" => Some(settings.fixed_window_position.as_wire()),
        "mini" => Some(settings.mini_window_position.as_wire()),
        _ => None,
    };
    let requested = placement.or(stored);
    window.unminimize().map_err(|error| error.to_string())?;
    // "former" means the frame the user left the window in, which is remembered
    // per window type across restarts (`EZWindowManager.m:619/676`). The other
    // placements recompute their position from the cursor and the work area
    // every time, so only this one restores a size as well.
    let restored = match (Placement::parse(requested), crate::window_frame::stored(settings, label)) {
        (Placement::Former, Some(frame)) => {
            if let Err(error) = restore(&window, frame) {
                log::warn!("could not restore the remembered frame of {label}: {error}");
            }
            true
        }
        _ => false,
    };
    if let Err(error) = position(&window, requested, restored) {
        log::warn!("could not position {label}: {error}");
    }
    window.show().map_err(|error| error.to_string())?;
    // Remember where this window ended up. A window whose frame was restored
    // keeps it; a placement the user changes by dragging is recorded by the
    // window-event handler instead.
    crate::window_frame::note(app, label);
    window.emit_to(label, crate::commands::events::ACTION, ()).map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

/// `EZShowWindowPosition`, as sent over the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    Right,
    Mouse,
    Former,
    Center,
}

impl Placement {
    pub fn parse(raw: Option<&str>) -> Placement {
        match raw.unwrap_or("right").to_ascii_lowercase().as_str() {
            "mouse" => Placement::Mouse,
            "former" => Placement::Former,
            "center" => Placement::Center,
            _ => Placement::Right,
        }
    }
}

fn monitor(
    window: &WebviewWindow,
    point: Option<PhysicalPosition<i32>>,
) -> Result<Monitor, String> {
    point
        .and_then(|point| {
            window
                .monitor_from_point(point.x as f64, point.y as f64)
                .ok()
                .flatten()
        })
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten())
        .ok_or_else(|| "no monitor work area is available".into())
}

fn placed_origin(
    placement: Placement,
    anchor: PhysicalPosition<i32>,
    size: PhysicalSize<u32>,
    work: PhysicalRect<i32, u32>,
) -> PhysicalPosition<i32> {
    let left = work.position.x as i64;
    let top = work.position.y as i64;
    let max_x = left + work.size.width.saturating_sub(size.width) as i64;
    let max_y = top + work.size.height.saturating_sub(size.height) as i64;
    let (x, y) = match placement {
        Placement::Mouse | Placement::Former => (anchor.x as i64, anchor.y as i64),
        Placement::Center => ((left + max_x) / 2, (top + max_y) / 2),
        Placement::Right => (max_x, top),
    };
    PhysicalPosition::new(
        x.clamp(left, max_x).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
        y.clamp(top, max_y).clamp(i32::MIN as i64, i32::MAX as i64) as i32,
    )
}

fn frame_size(outer: PhysicalSize<u32>, inner: PhysicalSize<u32>) -> PhysicalSize<u32> {
    PhysicalSize::new(
        outer.width.saturating_sub(inner.width),
        outer.height.saturating_sub(inner.height),
    )
}

/// Desktop coordinates stay physical; only sizes are converted between DPIs.
/// Former preserves a usable position, but recovers a disconnected/off-screen window.
///
/// `restored` says the window was just put at its remembered frame by
/// [`restore`]: the origin already read back is that frame, so the placement
/// must not pull it to the cursor or the screen edge again.
pub fn position(window: &WebviewWindow, raw: Option<&str>, restored: bool) -> Result<(), String> {
    if window.is_maximized().map_err(|e| e.to_string())? {
        return Ok(());
    }
    let placement = Placement::parse(raw);
    let origin = window.outer_position().map_err(|e| e.to_string())?;
    let cursor = if placement == Placement::Former || restored {
        None
    } else {
        window
            .cursor_position()
            .ok()
            .map(|point| point.cast::<i32>())
    };
    let target = monitor(window, cursor)?;
    let work = *target.work_area();
    let anchor = cursor.unwrap_or(origin);
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let estimated = size
        .to_logical::<f64>(scale)
        .to_physical::<u32>(target.scale_factor());
    let point = placed_origin(placement, anchor, estimated, work);
    if point != origin {
        window.set_position(point).map_err(|e| e.to_string())?;
    }

    // Moving to another DPI changes the native frame too. Read it back before
    // capping the client size and applying the final edge/center placement.
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    apply_size(window, inner, work)?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    window
        .set_position(placed_origin(placement, anchor, size, work))
        .map_err(|e| e.to_string())
}

/// Puts a window back at its remembered frame.
///
/// The monitor is chosen before the size is applied, because an oversized frame
/// is clamped against the work area of the monitor the window is on. A
/// maximised window is left alone: it is not sitting at its remembered frame,
/// and resizing it here would drop it out of the maximised state.
///
/// Public so the native integration test can exercise the restore path without
/// a settings store; the production caller is [`show`].
pub fn restore(window: &WebviewWindow, frame: crate::config::WindowFrame) -> Result<(), String> {
    if frame.is_empty() {
        // Nothing was ever remembered — a zero-sized entry is how both the
        // stored map and upstream's `CGRectZero` spell that.
        return Ok(());
    }
    if window.is_maximized().map_err(|e| e.to_string())? {
        return Ok(());
    }
    let (point, size) = crate::window_frame::placement(&frame);
    window.set_position(point).map_err(|e| e.to_string())?;
    window.set_size(size).map_err(|e| e.to_string())
}

/// Clamps a size without the panic `f64::clamp` raises when its bounds cross.
///
/// Those bounds really do cross here: [`fit_to_content`] lets the ceiling fall
/// to 20% of the screen height, which on a short display lands *below* the
/// 120 px floor. A panic in a placement call would take down the thread that
/// issued it, so the ceiling is lifted to meet the floor instead.
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value.is_nan() {
        return min;
    }
    value.max(min).min(max.max(min))
}

/// Resizes a floating window, keeping it on screen.
pub fn resize(
    window: &WebviewWindow,
    width: f64,
    height: f64,
    _animate: bool,
) -> Result<(), String> {
    let work = *monitor(window, None)?.work_area();
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let requested = LogicalSize::new(
        clamp(width, 180.0, work.size.width as f64 / scale),
        clamp(height, 80.0, work.size.height as f64 / scale),
    )
    .to_physical::<u32>(scale);
    apply_size(window, requested, work).map(|_| ())
}

fn apply_size(
    window: &WebviewWindow,
    requested: PhysicalSize<u32>,
    work: PhysicalRect<i32, u32>,
) -> Result<LogicalSize<f64>, String> {
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    if !window.is_maximized().map_err(|e| e.to_string())? {
        let frame = frame_size(window.outer_size().map_err(|e| e.to_string())?, inner);
        let size = PhysicalSize::new(
            requested
                .width
                .min(work.size.width.saturating_sub(frame.width).max(1)),
            requested
                .height
                .min(work.size.height.saturating_sub(frame.height).max(1)),
        );
        if size != inner {
            window.set_size(size).map_err(|e| e.to_string())?;
        }
        let origin = window.outer_position().map_err(|e| e.to_string())?;
        let outer = window.outer_size().map_err(|e| e.to_string())?;
        let safe = placed_origin(Placement::Former, origin, outer, work);
        if safe != origin {
            window.set_position(safe).map_err(|e| e.to_string())?;
        }
    }
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    Ok(window
        .inner_size()
        .map_err(|e| e.to_string())?
        .to_logical(scale))
}

fn fitted_height(
    content: f64,
    work_height: u32,
    frame_height: u32,
    scale: f64,
    percentage: i32,
) -> f64 {
    let ceiling = (work_height as f64 * percentage.clamp(20, 100) as f64 / 100.0
        - frame_height as f64)
        .max(1.0)
        / scale;
    // set_size takes CLIENT dimensions: reserve native decorations in the
    // ceiling, rather than adding them to the requested content a second time.
    clamp(content, 120.0, ceiling)
}

/// Grows the window so `content_height` of result content fits, clamped by
/// `settings.max_window_height_percentage`.
/// Returns the actual logical client size, including native minimum-size constraints.
pub fn fit_to_content(
    window: &WebviewWindow,
    content_height: f64,
) -> Result<LogicalSize<f64>, String> {
    let settings = window
        .app_handle()
        .try_state::<std::sync::Arc<crate::state::AppState>>()
        .map(|s| s.settings());

    let work = *monitor(window, None)?.work_area();
    let percentage = settings
        .as_ref()
        .map(|s: &Settings| s.max_window_height_percentage)
        .unwrap_or(100);
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let inner = window.inner_size().map_err(|e| e.to_string())?;
    let frame = frame_size(window.outer_size().map_err(|e| e.to_string())?, inner);
    let height = fitted_height(
        content_height,
        work.size.height,
        frame.height,
        scale,
        percentage,
    );
    apply_size(
        window,
        PhysicalSize::new(inner.width, (height * scale).round() as u32),
        work,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbox_is_bounded_fifo_and_isolated_to_known_windows() {
        let windows = Windows::default();
        assert!(windows.push("unknown", ActionPayload::new("inputText", None)).is_err());
        assert!(windows.push("main", ActionPayload::new("inputText", Some("x".repeat(MAX_ACTION_BYTES + 1)))).is_err());
        for i in 0..MAX_PENDING {
            windows.push("main", ActionPayload::new("inputText", Some(i.to_string()))).unwrap();
        }
        assert!(windows.push("main", ActionPayload::new("overflow", None)).is_err());
        windows.push("ocr", ActionPayload::new("silentScreenshotOCR", None)).unwrap();
        assert!(windows.next("settings").is_none());
        assert_eq!(windows.next("ocr").unwrap().action, "silentScreenshotOCR");
        for i in 0..MAX_PENDING {
            assert_eq!(windows.next("main").unwrap().text, Some(i.to_string()));
        }
        assert!(windows.next("main").is_none());
        windows.push("main", ActionPayload::new("retry", None)).unwrap();
        assert_eq!(windows.next("main").unwrap().action, "retry");
    }

    #[test]
    fn placement_parsing_is_forgiving() {
        assert_eq!(Placement::parse(Some("right")), Placement::Right);
        assert_eq!(Placement::parse(Some("Mouse")), Placement::Mouse);
        assert_eq!(Placement::parse(Some("CENTER")), Placement::Center);
        assert_eq!(Placement::parse(Some("former")), Placement::Former);
        // Anything unrecognised falls back to the default placement.
        assert_eq!(Placement::parse(None), Placement::Right);
        assert_eq!(Placement::parse(Some("nonsense")), Placement::Right);
    }

    #[test]
    fn clamping_survives_bounds_that_cross() {
        // A 480 px logical height — a 1280x720 display at 150% scaling — at the
        // 20% setting puts the ceiling at 96, under the 120 px floor. That is
        // the case `f64::clamp` panics on.
        assert_eq!(clamp(400.0, 120.0, 96.0), 120.0);
        assert_eq!(clamp(10.0, 120.0, 96.0), 120.0);
        assert_eq!(clamp(400.0, 180.0, 100.0), 180.0);

        // Ordinary bounds behave exactly as before.
        assert_eq!(clamp(300.0, 120.0, 900.0), 300.0);
        assert_eq!(clamp(10.0, 120.0, 900.0), 120.0);
        assert_eq!(clamp(5000.0, 120.0, 900.0), 900.0);
        assert_eq!(clamp(120.0, 120.0, 900.0), 120.0);
        assert_eq!(clamp(900.0, 120.0, 900.0), 900.0);
    }

    #[test]
    fn a_non_numeric_content_height_cannot_reach_the_window() {
        // `content_height` arrives from the front-end, so it can be NaN.
        assert_eq!(clamp(f64::NAN, 120.0, 900.0), 120.0);
        assert_eq!(clamp(f64::NAN, 180.0, 96.0), 180.0);
        assert!(!clamp(f64::NAN, 120.0, 900.0).is_nan());
    }

    #[test]
    fn mouse_placement_keeps_negative_monitor_origins_and_avoids_the_taskbar() {
        let work = PhysicalRect {
            position: PhysicalPosition::new(-1920, -1080),
            size: PhysicalSize::new(1920, 1040),
        };
        let size = PhysicalSize::new(500, 300);
        assert_eq!(
            placed_origin(
                Placement::Mouse,
                PhysicalPosition::new(-1900, -1000),
                size,
                work
            ),
            PhysicalPosition::new(-1900, -1000)
        );
        assert_eq!(
            placed_origin(
                Placement::Mouse,
                PhysicalPosition::new(-10, -10),
                size,
                work
            ),
            PhysicalPosition::new(-500, -340)
        );
    }

    #[test]
    fn right_is_top_right_like_upstream_and_center_includes_work_area_offsets() {
        let work = PhysicalRect {
            position: PhysicalPosition::new(80, 40),
            size: PhysicalSize::new(1840, 1000),
        };
        let anchor = PhysicalPosition::new(0, 0);
        let size = PhysicalSize::new(500, 300);
        assert_eq!(
            placed_origin(Placement::Right, anchor, size, work),
            PhysicalPosition::new(1420, 40)
        );
        assert_eq!(
            placed_origin(Placement::Center, anchor, size, work),
            PhysicalPosition::new(750, 390)
        );
    }

    #[test]
    fn former_preserves_usable_positions_and_recovers_offscreen_or_oversized_windows() {
        let work = PhysicalRect {
            position: PhysicalPosition::new(-1600, 40),
            size: PhysicalSize::new(1600, 860),
        };
        let size = PhysicalSize::new(500, 300);
        let origin = PhysicalPosition::new(-1000, 200);
        assert_eq!(placed_origin(Placement::Former, origin, size, work), origin);
        assert_eq!(
            placed_origin(
                Placement::Former,
                PhysicalPosition::new(1000, 200),
                size,
                work
            ),
            PhysicalPosition::new(-500, 200)
        );
        assert_eq!(
            placed_origin(
                Placement::Center,
                origin,
                PhysicalSize::new(u32::MAX, u32::MAX),
                work
            ),
            work.position
        );
    }

    #[test]
    fn placement_uses_the_target_dpi_for_size_but_never_scales_the_desktop_origin() {
        let work = PhysicalRect {
            position: PhysicalPosition::new(-1920, 0),
            size: PhysicalSize::new(1920, 1040),
        };
        let old_size = PhysicalSize::new(800, 480);
        let target_size = old_size.to_logical::<f64>(2.0).to_physical::<u32>(1.5);
        assert_eq!(target_size, PhysicalSize::new(600, 360));
        assert_eq!(
            placed_origin(
                Placement::Right,
                PhysicalPosition::new(-1000, 400),
                target_size,
                work
            ),
            PhysicalPosition::new(-600, 0)
        );
        let extreme = PhysicalRect {
            position: PhysicalPosition::new(i32::MAX - 10, i32::MIN),
            size: PhysicalSize::new(u32::MAX, u32::MAX),
        };
        assert_eq!(
            placed_origin(
                Placement::Right,
                work.position,
                PhysicalSize::new(1, 1),
                extreme
            ),
            PhysicalPosition::new(i32::MAX, i32::MIN)
        );
    }

    #[test]
    fn content_fit_reserves_native_frame_without_adding_it_to_client_height() {
        assert_eq!(fitted_height(300.0, 1440, 48, 1.5, 80), 300.0);
        assert_eq!(fitted_height(9999.0, 1440, 48, 1.5, 80), 736.0);
        assert_eq!(fitted_height(9999.0, 1000, 64, 2.0, 100), 468.0);
        assert_eq!(fitted_height(f64::NAN, 720, 48, 1.5, 20), 120.0);
        assert_eq!(fitted_height(f64::INFINITY, 1000, 0, 1.0, 999), 1000.0);
        assert_eq!(
            frame_size(PhysicalSize::new(500, 400), PhysicalSize::new(484, 361)),
            PhysicalSize::new(16, 39)
        );
    }
}
