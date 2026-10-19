//! Small Windows system integrations that do not belong to a single feature:
//! the user's locale and synthesised keystrokes.
//!
//! The macOS originals are `Locale.preferredLanguages` and
//! `CGEvent.post(tap:)`.

use crate::model::Language;

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Globalization::GetUserDefaultLocaleName;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_CONTROL, VK_C, VK_V,
    };

    /// `LOCALE_NAME_MAX_LENGTH` from `winnls.h`. The `windows` crate does not
    /// re-export it, and it has been 85 since Windows Vista.
    const LOCALE_NAME_MAX_LENGTH: usize = 85;

    /// The user's locale as a BCP-47 tag, e.g. `zh-CN`.
    ///
    /// `GetUserDefaultLocaleName` reports only the preferred UI locale, where
    /// macOS exposes an ordered list. The app only ever needs the first entry —
    /// it decides which of the two auto-translate languages is primary — so the
    /// shorter answer is enough.
    pub fn system_languages() -> Vec<String> {
        let mut buffer = [0u16; LOCALE_NAME_MAX_LENGTH as usize];
        let len = unsafe { GetUserDefaultLocaleName(&mut buffer) };
        if len <= 1 {
            return Vec::new();
        }
        // `len` includes the terminating NUL.
        let name = String::from_utf16_lossy(&buffer[..(len as usize - 1)]);
        vec![name]
    }

    fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn send(events: &[INPUT]) -> Result<(), String> {
        let sent = unsafe { SendInput(events, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize == events.len() {
            Ok(())
        } else {
            Err(format!(
                "SendInput delivered {sent} of {} events",
                events.len()
            ))
        }
    }

    /// Whether a physical or synthetic key is currently held down.
    fn is_down(vk: VIRTUAL_KEY) -> bool {
        (unsafe { GetAsyncKeyState(vk.0 as i32) } as u16) & 0x8000 != 0
    }

    /// Synthesises `Ctrl+C` into whatever window has focus.
    ///
    /// Used by the clipboard fallback of the selection reader. When the user is
    /// already holding Ctrl (a `Ctrl+…` shortcut), only `C` is sent, because a
    /// synthesised Ctrl key-up would release the key the user still holds.
    pub fn send_copy_keystroke() -> Result<(), String> {
        let ctrl_held = is_down(VK_CONTROL);
        let mut events = Vec::with_capacity(4);
        if !ctrl_held {
            events.push(key(VK_CONTROL, false));
        }
        events.push(key(VK_C, false));
        events.push(key(VK_C, true));
        if !ctrl_held {
            events.push(key(VK_CONTROL, true));
        }
        send(&events)
    }

    /// Synthesises `Ctrl+V` into whatever window has focus.
    ///
    /// Used by "translate and replace": the caller puts the new text on the
    /// clipboard, calls this, then restores the old clipboard once the target
    /// application has had a moment to read it.
    pub fn send_paste_keystroke() -> Result<(), String> {
        send(&[
            key(VK_CONTROL, false),
            key(VK_V, false),
            key(VK_V, true),
            key(VK_CONTROL, true),
        ])
    }

    /// The first entry of [`system_languages`] resolved to a [`Language`],
    /// falling back to English.
    pub fn system_language() -> Language {
        system_languages()
            .first()
            .map(|code| Language::from_code(code))
            .filter(|l| *l != Language::Auto)
            .unwrap_or(Language::English)
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn system_languages() -> Vec<String> {
        std::env::var("LANG")
            .ok()
            .map(|v| vec![v])
            .unwrap_or_default()
    }

    pub fn send_paste_keystroke() -> Result<(), String> {
        Err("synthesised keystrokes are only implemented on Windows".to_string())
    }

    pub fn system_language() -> Language {
        Language::English
    }
}

pub use imp::{send_paste_keystroke, system_language, system_languages};
#[cfg(windows)]
pub use imp::send_copy_keystroke;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_language_is_never_auto() {
        // The app relies on this to pick which auto-translate language leads.
        assert_ne!(system_language(), Language::Auto);
    }
}
