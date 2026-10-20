//! Memory hints for windows that are hidden.
//!
//! Upstream keeps its `NSWindow`s after close (`EZWindowManager.m:206/214/222`),
//! and this port does the same: closing hides a window, so reopening it is
//! instant and its results, scroll position and settings page all survive. A
//! hidden WebView2 is not free, though — it keeps its share of the browser
//! process tree alive. So a hidden window asks the runtime to suspend its
//! WebView and to target a low memory level, and a window that is needed again
//! is resumed. Suspending is what Edge does to a sleeping tab: script timers and
//! animations stop, and the renderer's memory becomes available to other
//! processes.
//!
//! Nothing here destroys a window, so no state is lost and no queued action is
//! dropped. Three runtime rules shape the call sites. `TrySuspend`, `Resume` and
//! the memory target level live on the CoreWebView2 object, not on its
//! controller. `TrySuspend` documents an invisible controller as its
//! precondition, while hiding a Tauri window only hides the Win32 window, so a
//! refusal is retried once with the WebView hidden too. A WebView resumes by
//! itself once it becomes visible, which is why the wake path shows it again
//! rather than relying on a suspension to expire. `with_webview` runs its
//! closure inline on the main thread but only posts it from anywhere else, so
//! `set_hidden` waits for its own request to be handled and every entry point
//! below runs on a worker thread.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tauri::{AppHandle, Manager, WebviewWindow};
use webview2_com::{
    Microsoft::Web::WebView2::Win32::{
        ICoreWebView2_19, ICoreWebView2_3, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
        COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
    },
    TrySuspendCompletedHandler,
};
use windows::core::{Interface, Result as ComResult};

/// What the runtime accepted for one window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryState {
    /// The runtime took the suspension request; the renderer goes to sleep.
    Suspended,
    /// The WebView runs normally.
    Active,
    /// The installed WebView2 runtime predates these APIs, so nothing was asked.
    Unsupported,
}

/// Whether a window may be suspended: a visible or focused window is doing work
/// the user can see, and `TrySuspend` rejects it anyway.
fn should_suspend(visible: bool, focused: bool) -> bool {
    !visible && !focused
}

/// Delivers the one-shot answer of a memory request exactly once: a suspension
/// is reported by the runtime callback, while a rejected request is reported by
/// the caller, and neither may report twice.
#[derive(Clone)]
struct Reporter(Arc<Mutex<Option<Box<dyn FnOnce(bool) + Send>>>>);

impl Reporter {
    fn new(report: impl FnOnce(bool) + Send + 'static) -> Self {
        Self(Arc::new(Mutex::new(Some(Box::new(report)))))
    }

    fn report(&self, accepted: bool) {
        if let Some(report) = self.0.lock().take() {
            report(accepted);
        }
    }
}

/// How long the caller waits for the event loop to run the memory request.
/// The closure only makes local COM calls, so a wait this long means the loop
/// is gone, and reporting a refusal beats hanging a worker thread forever.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);

/// Suspends or resumes one window's WebView.
///
/// Returns what the runtime accepted. A suspension is asynchronous, so its real
/// answer (the renderer may refuse to sleep) is passed to `report` later, on the
/// thread the runtime calls back on. `Ok(MemoryState::Unsupported)` is the only
/// quiet outcome: it means the installed runtime predates these interfaces. Any
/// other refusal is an error that names the runtime's own complaint, because a
/// caller that labels a measurement or a test "suspended" has to be able to tell
/// a refusal from a runtime that was never asked.
///
/// This waits for the request to reach the event loop, so a caller that shows a
/// window immediately afterwards knows the WebView is awake. It may be called
/// from any thread: on the main thread the request runs inline, and elsewhere it
/// is posted and awaited.
pub fn set_hidden(
    window: &WebviewWindow,
    hidden: bool,
    report: impl FnOnce(bool) + Send + 'static,
) -> Result<MemoryState, String> {
    let accepted = Arc::new(Mutex::new(MemoryState::Unsupported));
    let outcome = accepted.clone();
    let trouble = Arc::new(Mutex::new(None::<String>));
    let trouble_in = trouble.clone();
    let report = Reporter::new(report);
    let finished = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
    let finished_in = finished.clone();
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    let target = window.clone();
    window
        .with_webview(move |webview| {
            'request: {
                // Check on the event thread, not when a worker was scheduled:
                // a slow hidden OCR/replacement must receive its IPC response.
                if hidden && (!should_suspend(
                    target.is_visible().unwrap_or(true),
                    target.is_focused().unwrap_or(true),
                ) || app.try_state::<crate::window::Windows>()
                    .is_some_and(|windows| windows.busy(&label)))
                {
                    *outcome.lock() = MemoryState::Active;
                    report.report(false);
                    break 'request;
                }
                let blame = |message: String| *trouble_in.lock() = Some(message);
                let controller = webview.controller();
                // Suspension, resumption and the memory target level belong to
                // the CoreWebView2 object. A controller does not implement them,
                // so casting the controller itself fails on every runtime.
                let core = match unsafe { controller.CoreWebView2() } {
                    Ok(core) => core,
                    Err(error) => {
                        report.report(false);
                        blame(format!("could not reach the window's WebView: {error}"));
                        break 'request;
                    }
                };
                let Ok(suspendable) = core.cast::<ICoreWebView2_3>() else {
                    // A runtime this old cannot suspend; the window still works,
                    // and `report` says the request was not honoured. Its
                    // visibility is left alone, since nothing here changed it.
                    report.report(false);
                    break 'request;
                };
                // The memory target level arrived later than suspension, so it is
                // asked for on its own and its absence is not an error.
                let level = core.cast::<ICoreWebView2_19>().ok();
                if hidden {
                    if let Some(level) = &level {
                        if let Err(error) = unsafe {
                            level
                                .SetMemoryUsageTargetLevel(COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW)
                        } {
                            log::debug!("could not lower the memory target level: {error}");
                        }
                    }
                    let callback = report.clone();
                    let handler = TrySuspendCompletedHandler::create(Box::new(
                        move |result: ComResult<()>, suspended: bool| {
                            if result.is_err() || !suspended {
                                log::debug!("WebView2 did not suspend the hidden window");
                            }
                            callback.report(result.is_ok() && suspended);
                            Ok(())
                        },
                    ));
                    // `TrySuspend` documents an invisible controller as its
                    // precondition, and hiding a Tauri window leaves the
                    // controller visible. Runtimes differ in how strictly they
                    // enforce that: the installed one accepts a hidden window as
                    // it is, so the WebView is only hidden when it has to be.
                    // Hiding it unconditionally would put a blank window behind
                    // any show path that forgot to wake the window first.
                    let mut refused = unsafe { suspendable.TrySuspend(&handler) };
                    if refused.is_err() {
                        match unsafe { controller.SetIsVisible(false) } {
                            Ok(()) => match unsafe { suspendable.TrySuspend(&handler) } {
                                Ok(()) => refused = Ok(()),
                                Err(error) => {
                                    // Leave nothing half-done: an awake window
                                    // must not stay invisible.
                                    if let Err(restore) = unsafe { controller.SetIsVisible(true) } {
                                        log::debug!("could not restore a visible WebView: {restore}");
                                    }
                                    refused = Err(error);
                                }
                            },
                            Err(error) => {
                                log::debug!("could not hide the WebView for the retry: {error}");
                            }
                        }
                    }
                    match refused {
                        Ok(()) => *outcome.lock() = MemoryState::Suspended,
                        Err(error) => {
                            report.report(false);
                            blame(format!(
                                "the runtime refused to suspend a hidden window: {error}"
                            ));
                        }
                    }
                } else {
                    // Resume before showing: a suspended WebView has paused its
                    // timers, so the window would otherwise show a stale frame.
                    if let Err(error) = unsafe { suspendable.Resume() } {
                        report.report(false);
                        blame(format!("the runtime refused to resume the window: {error}"));
                        break 'request;
                    }
                    if let Some(level) = &level {
                        if let Err(error) = unsafe {
                            level.SetMemoryUsageTargetLevel(
                                COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
                            )
                        } {
                            log::debug!("could not restore the memory target level: {error}");
                        }
                    }
                    if let Err(error) = unsafe { controller.SetIsVisible(true) } {
                        report.report(false);
                        blame(format!("could not show the resumed WebView: {error}"));
                        break 'request;
                    }
                    *outcome.lock() = MemoryState::Active;
                    report.report(true);
                }
            }
            let (done, answer) = &*finished_in;
            if let Ok(mut done) = done.lock() {
                *done = true;
            }
            answer.notify_all();
        })
        .map_err(|error| error.to_string())?;
    let (done, answer) = &*finished;
    let mut handled = done
        .lock()
        .map_err(|_| "the memory request of this window was poisoned".to_string())?;
    while !*handled {
        let (guard, waited) = answer
            .wait_timeout(handled, ANSWER_TIMEOUT)
            .map_err(|_| "the memory request of this window was poisoned".to_string())?;
        handled = guard;
        if !*handled && waited.timed_out() {
            return Err("the window did not answer the memory request".into());
        }
    }
    drop(handled);
    if let Some(message) = trouble.lock().take() {
        return Err(message);
    }
    Ok(*accepted.lock())
}

/// Releases a window's WebView memory after it was hidden. Safe to call from
/// the main thread, which is where a close request arrives.
pub fn release_when_hidden(app: &AppHandle, label: &str) {
    let app = app.clone();
    let label = label.to_owned();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(window) = app.get_webview_window(&label) else {
            return;
        };
        let reporting = label.clone();
        match set_hidden(&window, true, move |suspended| {
            if !suspended {
                log::debug!("{reporting} stayed awake after being hidden");
            }
        }) {
            Ok(MemoryState::Suspended) => log::debug!("{label} released its WebView memory"),
            Ok(_) => {}
            Err(error) => log::debug!("could not release {label}: {error}"),
        }
    });
}

/// Wakes a window before it is shown. The runtime also resumes a WebView when it
/// becomes visible; doing it here keeps the order deterministic.
pub fn activate(window: &WebviewWindow) {
    let label = window.label().to_owned();
    if let Err(error) = set_hidden(window, false, move |resumed| {
        if !resumed {
            log::debug!("{label} did not resume");
        }
    }) {
        log::debug!("could not wake {}: {error}", window.label());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_hidden_and_unfocused_window_may_suspend() {
        assert!(should_suspend(false, false));
        assert!(!should_suspend(true, false));
        assert!(!should_suspend(false, true));
        assert!(!should_suspend(true, true));
    }

    #[test]
    fn the_reported_states_stay_distinct() {
        assert_ne!(MemoryState::Suspended, MemoryState::Active);
        assert_ne!(MemoryState::Unsupported, MemoryState::Suspended);
        assert_ne!(MemoryState::Unsupported, MemoryState::Active);
    }
}
