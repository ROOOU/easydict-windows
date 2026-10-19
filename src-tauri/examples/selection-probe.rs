//! Diagnostic probe for the selection reader.
//!
//! Without arguments it runs the real `platform::selected_text(true)` — UI
//! Automation first, clipboard fallback second — against whatever owns the
//! focus, and reports how long it took. With an HWND (hex, `0x` optional) it
//! inspects that element's UI Automation tree instead, which needs no
//! foreground window.
//!
//! ```text
//! cargo run --example selection-probe            # the real read, focused window
//! cargo run --example selection-probe -- --uia   # UI Automation only
//! cargo run --example selection-probe -- 0x1234  # a specific control
//! ```

#[cfg(windows)]
fn main() {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CLSCTX_INPROC_SERVER, CoCreateInstance,
    };
    use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, IUIAutomationTextPattern,
        UIA_TextPatternId,
    };

    /// The probe's own OLE apartment for the inspection below. `selected_text`
    /// initialises and releases its own, so this must be separate.
    struct Apartment {
        active: bool,
    }
    impl Apartment {
        fn init() -> Self {
            Self {
                active: unsafe { OleInitialize(None) }.is_ok(),
            }
        }
    }
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.active {
                unsafe { OleUninitialize() };
            }
        }
    }

    let argument = std::env::args().nth(1);
    let uia_only = argument.as_deref() == Some("--uia");
    let target = match argument.as_deref() {
        None | Some("focused") | Some("--uia") => None,
        Some(raw) => {
            let raw = raw.trim_start_matches("0x");
            Some(HWND(usize::from_str_radix(raw, 16).expect("hwnd must be hex") as *mut _))
        }
    };

    if target.is_none() {
        let started = std::time::Instant::now();
        match app_lib::platform::selected_text(!uia_only) {
            Ok(text) => println!(
                "selected_text ok in {:?}: {text:?}",
                started.elapsed()
            ),
            Err(error) => println!(
                "selected_text err in {:?}: {error}",
                started.elapsed()
            ),
        }
        println!("--- UI Automation view of the focused element ---");
    }

    let _apartment = Apartment::init();
    let automation: IUIAutomation = unsafe {
        CoCreateInstance(&CUIAutomation, None::<&windows::core::IUnknown>, CLSCTX_INPROC_SERVER)
    }
    .expect("UI Automation is unavailable");

    let element: IUIAutomationElement = match target {
        Some(hwnd) => unsafe { automation.ElementFromHandle(hwnd) }.expect("ElementFromHandle"),
        None => match unsafe { automation.GetFocusedElement() } {
            Ok(element) => element,
            Err(error) => {
                println!("err GetFocusedElement: {error}");
                return;
            }
        },
    };

    let mut current = element;
    for depth in 0..10 {
        let name = unsafe { current.CurrentName() }.map(|b| b.to_string()).unwrap_or_default();
        let class = unsafe { current.CurrentClassName() }
            .map(|b| b.to_string())
            .unwrap_or_default();
        let control = unsafe { current.CurrentControlType() }
            .map(|t| t.0)
            .unwrap_or_default();
        let password = unsafe { current.CurrentIsPassword() }
            .map(|b| b.as_bool())
            .unwrap_or(false);
        print!("[{depth}] class={class:?} controlType={control} password={password} name={name:?} ");

        match unsafe { current.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) } {
            Ok(pattern) => {
                let mut selected = String::new();
                let mut ranges = 0;
                if let Ok(collection) = unsafe { pattern.GetSelection() } {
                    if let Ok(count) = unsafe { collection.Length() } {
                        ranges = count;
                        for index in 0..count {
                            if let Ok(range) = unsafe { collection.GetElement(index) } {
                                if let Ok(text) = unsafe { range.GetText(-1) } {
                                    selected.push_str(text.to_string().trim_matches('\0'));
                                }
                            }
                        }
                    }
                }
                let document = unsafe { pattern.DocumentRange() }
                    .and_then(|range| unsafe { range.GetText(-1) })
                    .map(|text| text.to_string().trim_matches('\0').to_string())
                    .unwrap_or_default();
                let document = document.chars().take(60).collect::<String>();
                println!(
                    "TextPattern=yes ranges={ranges} text={selected:?} document={document:?}"
                );
            }
            Err(_) => println!("TextPattern=no"),
        }

        if target.is_some() {
            match unsafe { automation.ControlViewWalker() }
                .and_then(|walker| unsafe { walker.GetParentElement(&current) })
            {
                Ok(parent) => current = parent,
                Err(_) => break,
            }
        } else {
            break;
        }
    }
}

#[cfg(not(windows))]
fn main() {
    println!("selection-probe only runs on Windows");
}
