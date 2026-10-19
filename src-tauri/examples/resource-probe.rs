//! Controlled Release measurement of real React/WebView2 windows, without
//! production data, tray UI, input hooks, global hotkeys, or translation.
//! Build: cargo build --release --example resource-probe --features tauri/custom-protocol
//! Run with tools/dev/resource-probe.ps1; never included in the installer.

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    collections::HashSet,
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};
use tauri::{Listener, Manager, RunEvent};

#[cfg(target_env = "msvc")]
#[link(name = "resource", kind = "static", modifiers = "-bundle")]
unsafe extern "C" {}

const READY_SCRIPT: &str = r#"
(() => {
  const observer = new MutationObserver(() => {
    if (document.querySelector('.ez-query-window,.ezy-root,.ezo-root')) {
      observer.disconnect();
      window.__TAURI_INTERNALS__.invoke('plugin:event|emit', {
        event: 'resource-probe-ready',
        payload: window.__TAURI_INTERNALS__.metadata.currentWindow.label
      });
    }
  });
  observer.observe(document, { childList: true, subtree: true });
})();
"#;

fn main() -> anyhow::Result<()> {
    // Fail closed: the non-custom-protocol build could contact a Vite server.
    if cfg!(dev) {
        anyhow::bail!("build with --features tauri/custom-protocol");
    }

    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("missing isolated profile"))?,
    );
    let count: usize = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("missing window count"))?
        .parse()?;
    let scenario = args.next().unwrap_or_else(|| "idle".into());
    anyhow::ensure!(
        args.next().is_none() && matches!(count, 0 | 1 | 5),
        "expected 0, 1 or 5 windows"
    );
    anyhow::ensure!(
        matches!(scenario.as_str(), "idle" | "suspend"),
        "expected the idle or suspend scenario"
    );
    let root = root.canonicalize()?;
    let temp = std::env::temp_dir().canonicalize()?;
    anyhow::ensure!(
        root.parent() == Some(temp.as_path())
            && root
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("easydict-resource-")
            && root.read_dir()?.next().is_none(),
        "profile must be a new empty easydict-resource-* directory under TEMP"
    );
    let mut context = tauri::generate_context!();
    context.config_mut().identifier =
        format!("com.easydict.resource-probe.{}", uuid::Uuid::new_v4());
    context.config_mut().product_name = Some("Easydict Resource Probe".into());
    for window in &mut context.config_mut().app.windows {
        window.create = false;
        window.visible = false;
        window.focus = false;
        window.skip_taskbar = true;
        window.data_directory = Some(root.join("webview"));
    }
    let ready = Arc::new((Mutex::new(HashSet::<String>::new()), Condvar::new()));
    let ready_setup = ready.clone();
    let state_root = root.join("state");
    let started = Instant::now();
    let app = app_lib::builder()
        .plugin(
            tauri_plugin_log::Builder::new()
                .targets([tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Folder {
                        path: root.join("logs"),
                        file_name: Some("probe".into()),
                    },
                )])
                .build(),
        )
        .plugin(
            tauri::plugin::Builder::<tauri::Wry>::new("resource-probe")
                .js_init_script(READY_SCRIPT)
                .build(),
        )
        .setup(move |app| {
            let store = Arc::new(app_lib::store::Store::open(state_root.clone())?);
            let mut settings = app_lib::config::Settings::default();
            settings.first_launch = false;
            settings.auto_show_query_icon = false;
            settings.shortcuts.clear();
            store.save_settings(&settings)?;
            let state = app_lib::state::AppState::new(app.handle().clone(), store)?;
            app.manage(state.clone());
            app.manage(app_lib::window::Windows::default());
            app_lib::desktop::initialize_shortcuts(app.handle(), &settings);
            app.manage(app_lib::platform::auto_selection::AutoSelection::start(
                &state,
            ));
            app.listen("resource-probe-ready", move |event| {
                if let Ok(label) = serde_json::from_str::<String>(event.payload()) {
                    let (labels, changed) = &*ready_setup;
                    labels.lock().unwrap().insert(label);
                    changed.notify_all();
                }
            });
            Ok(())
        })
        .build(context)?;
    let code = app.run_return(move |app, event| {
        if let RunEvent::ExitRequested {
            ref api,
            code: None,
            ..
        } = event
        {
            api.prevent_exit();
        }
        if matches!(event, RunEvent::Ready) {
            let app = app.clone();
            let ready = ready.clone();
            let root = root.clone();
            // `run_return` takes an `FnMut`, so the scenario is cloned into the
            // worker instead of being moved out of the callback.
            let scenario = scenario.clone();
            std::thread::spawn(move || {
                let check = || -> anyhow::Result<()> {
                    let labels = &["main", "mini", "fixed", "settings", "ocr"][..count];
                    for label in labels {
                        app_lib::window::ensure(&app, label).map_err(anyhow::Error::msg)?;
                    }
                    let (seen, changed) = &*ready;
                    let (seen, _) = changed
                        .wait_timeout_while(seen.lock().unwrap(), Duration::from_secs(30), |seen| {
                            labels.iter().any(|label| !seen.contains(*label))
                        })
                        .unwrap();
                    anyhow::ensure!(
                        labels.iter().all(|label| seen.contains(*label)),
                        "business frontend did not become ready: {seen:?}"
                    );
                    drop(seen);
                    let hidden = || -> anyhow::Result<()> {
                        let windows = app.webview_windows();
                        anyhow::ensure!(windows.len() == count, "unexpected WebView count");
                        for window in windows.values() {
                            anyhow::ensure!(
                                !window.is_visible()? && !window.is_focused()?,
                                "probe window became visible/focused"
                            );
                        }
                        Ok(())
                    };
                    hidden()?;
                    // The suspended scenario reproduces what the app does when a
                    // window is closed: the window and its WebView survive, so
                    // results are kept, but the renderer is asked to sleep. The
                    // runtime is waited for, so a measurement labelled
                    // "suspend" cannot be a request that was quietly refused.
                    let mut suspended = 0usize;
                    if scenario == "suspend" {
                        for window in app.webview_windows().values() {
                            let (reported, answer) = std::sync::mpsc::channel();
                            let state = app_lib::platform::webview_memory::set_hidden(
                                window,
                                true,
                                move |accepted| {
                                    let _ = reported.send(accepted);
                                },
                            )
                            .map_err(anyhow::Error::msg)?;
                            anyhow::ensure!(
                                state != app_lib::platform::webview_memory::MemoryState::Unsupported,
                                "this WebView2 runtime cannot suspend a WebView"
                            );
                            anyhow::ensure!(
                                answer.recv_timeout(Duration::from_secs(30))?,
                                "the runtime refused to suspend a hidden window"
                            );
                            suspended += 1;
                        }
                    }
                    println!(
                        "RESOURCE_PROBE {}",
                        serde_json::json!({
                            "event": "ready", "windows": labels, "profile": root,
                            "readyMillis": started.elapsed().as_secs_f64() * 1000.0,
                            "webview2": tauri::webview_version()?,
                            "scenario": scenario, "suspended": suspended,
                        })
                    );
                    io::stdout().flush()?;
                    // A blocking pipe read adds no timer/polling load to idle samples.
                    let mut input = String::new();
                    io::stdin().read_line(&mut input)?;
                    anyhow::ensure!(input.trim() == "quit", "expected quit on stdin");
                    hidden()?;
                    Ok(())
                };
                let result = check();
                if let Err(error) = &result {
                    eprintln!("resource probe failed: {error:#}");
                }
                app.exit(if result.is_ok() { 0 } else { 1 });
            });
        }
    });
    anyhow::ensure!(code == 0, "probe exited with code {code}");
    Ok(())
}
