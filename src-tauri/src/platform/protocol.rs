//! The `easydict://` URL scheme: per-user registration and command-line parsing.
//!
//! macOS gets the scheme for free: `CFBundleURLTypes` in `Info.plist` tells the
//! system that the app answers `easydict://`, and AppKit delivers matching links
//! through `kAEGetURL`. Windows has the same concept with a different mechanism:
//! a `URL Protocol` key under `HKEY_CURRENT_USER\Software\Classes`, whose
//! `shell\open\command` value names the executable to launch, with `%1` standing
//! for the URL. Windows then starts that command with the URL as an argument.
//!
//! That single mechanism covers both cases this port has to handle, because the
//! link always arrives on a command line:
//!
//! * **cold start** — the app was not running, so Windows starts it with the
//!   link as `argv[1]`; `lib.rs` routes it once the settings are loaded;
//! * **already running** — Windows starts the same command anyway, and
//!   `tauri-plugin-single-instance` forwards that command line to the first
//!   process, which routes the link from its `argv`.
//!
//! The registration is per user, so it needs no elevation, and it is written on
//! every start: a moved (portable) executable repairs its own registration
//! instead of leaving `easydict://` pointing at a path that no longer exists.
//! [`unregister`] removes it again — the uninstaller deletes the same key, and
//! `app.exe --unregister-url-scheme` does it by hand.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
    RegCreateKeyExW, RegDeleteTreeW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::PCWSTR;

/// The scheme name, spelled exactly as upstream's `CFBundleURLSchemes` entry.
pub const SCHEME: &str = "easydict";

const PREFIX: &str = "easydict://";

/// The part of a deep link after `easydict://`, or `None` when the argument is
/// not a link at all.
///
/// A command line carries the executable path first and unrelated switches
/// after it, so every argument has to be examined rather than assumed. The
/// scheme is matched case-insensitively (browsers lowercase it, a hand-typed
/// link need not) and surrounding quotes are dropped, which is what a shell
/// quoting a URL that contains spaces would leave behind.
pub fn parse_link(argument: &str) -> Option<&str> {
    let argument = argument.trim().trim_matches('"');
    let prefix = argument.get(..PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    Some(&argument[PREFIX.len()..])
}

/// The `shell\open\command` value: this executable, then the URL.
///
/// The path is quoted because it routinely contains spaces, while `%1` stays
/// unquoted so that Windows substitutes the URL verbatim — quoting it here as
/// well is what makes a registered handler receive a stray pair of quotes.
pub fn command_line(executable: &Path) -> String {
    format!("\"{}\" \"%1\"", executable.display())
}

fn key_path(scheme: &str) -> String {
    format!("Software\\Classes\\{scheme}")
}

fn command_key_path(scheme: &str) -> String {
    format!("Software\\Classes\\{scheme}\\shell\\open\\command")
}

/// A `REG_SZ` value as the registry stores it: UTF-16LE with a terminator.
fn value_bytes(value: &str) -> Vec<u8> {
    value
        .encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// A registry path or value name, NUL-terminated for the `W` APIs.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn failure(action: &str, code: WIN32_ERROR) -> String {
    format!(
        "{action}: {}",
        std::io::Error::from_raw_os_error(code.0 as i32)
    )
}

/// Owns an open key so that every early return closes it.
struct Key(HKEY);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

fn create(path: &str) -> Result<Key, String> {
    let path = wide(path);
    let mut key = HKEY::default();
    let code = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(path.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_WRITE,
            None,
            &mut key,
            None,
        )
    };
    if code != ERROR_SUCCESS {
        return Err(failure("could not create the registry key", code));
    }
    Ok(Key(key))
}

/// Writes a `REG_SZ` value; `name` of `None` is the key's own `(Default)` value.
fn set_value(key: &Key, name: Option<&str>, value: &str) -> Result<(), String> {
    let name = name.map(wide);
    let data = value_bytes(value);
    let code = unsafe {
        RegSetValueExW(
            key.0,
            name.as_ref().map_or(PCWSTR::null(), |name| {
                PCWSTR(name.as_ptr())
            }),
            None,
            REG_SZ,
            Some(&data),
        )
    };
    if code != ERROR_SUCCESS {
        return Err(failure("could not write the registry value", code));
    }
    Ok(())
}

/// Registers `scheme://…` for the current user, pointing at `command`.
///
/// Existing values are replaced, so this is safe to run on every start.
pub fn register(scheme: &str, command: &str) -> Result<(), String> {
    let protocol = create(&key_path(scheme))?;
    // The default value is the display name Windows shows when it asks which
    // application should open the link.
    set_value(&protocol, None, &format!("URL:{scheme} protocol"))?;
    // The presence of this value is what marks the key as a protocol handler.
    set_value(&protocol, Some("URL Protocol"), "")?;
    let handler = create(&command_key_path(scheme))?;
    set_value(&handler, None, command)
}

/// Removes the registration; `Ok(false)` means there was nothing to remove.
pub fn unregister(scheme: &str) -> Result<bool, String> {
    let path = wide(&key_path(scheme));
    let code = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr())) };
    match code {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        _ => Err(failure("could not remove the registry key", code)),
    }
}

/// Reads back the command Windows would run for `scheme://…`.
pub fn registered_command(scheme: &str) -> Result<Option<String>, String> {
    let path = wide(&command_key_path(scheme));
    let mut key = HKEY::default();
    let code =
        unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key) };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if code != ERROR_SUCCESS {
        return Err(failure("could not open the registry key", code));
    }
    let key = Key(key);

    let mut kind = REG_SZ;
    let mut size = 0u32;
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR::null(),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if code != ERROR_SUCCESS {
        return Err(failure("could not read the registry value", code));
    }
    if size == 0 {
        return Ok(Some(String::new()));
    }
    let mut buffer = vec![0u8; size as usize];
    let code = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR::null(),
            None,
            None,
            Some(buffer.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if code != ERROR_SUCCESS {
        return Err(failure("could not read the registry value", code));
    }
    buffer.truncate(size as usize);
    let units: Vec<u16> = buffer
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    Ok(Some(
        String::from_utf16_lossy(&units)
            .trim_end_matches('\0')
            .to_string(),
    ))
}

/// Registers this executable for `easydict://…` and reports what was written.
pub fn register_self() -> Result<String, String> {
    let executable = std::env::current_exe()
        .map_err(|error| format!("could not locate this executable: {error}"))?;
    let command = command_line(&executable);
    register(SCHEME, &command)?;
    Ok(command)
}

/// Removes this user's `easydict://…` registration.
pub fn unregister_self() -> Result<bool, String> {
    unregister(SCHEME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_arguments_that_are_links_are_parsed() {
        assert_eq!(parse_link("easydict://query?text=hi"), Some("query?text=hi"));
        assert_eq!(parse_link("EASYDICT://ocr"), Some("ocr"));
        assert_eq!(parse_link("  easydict://ocr  "), Some("ocr"));
        // A window title or a path can contain the scheme without being a link.
        assert_eq!(parse_link("easydict://"), Some(""));
        assert_eq!(parse_link("https://easydict.example"), None);
        assert_eq!(parse_link(r"C:\Program Files\Easydict\app.exe"), None);
        assert_eq!(parse_link("--unregister-url-scheme"), None);
        assert_eq!(parse_link("easydict:/query"), None);
        assert_eq!(parse_link(""), None);
        // Quotes survive a shell that added them, and the URL may contain spaces.
        assert_eq!(
            parse_link("\"easydict://query?text=hello world\""),
            Some("query?text=hello world")
        );
        // The prefix check must not split a multi-byte character.
        assert_eq!(parse_link("易dict://ocr"), None);
        assert_eq!(parse_link("easydict"), None);
    }

    #[test]
    fn a_command_line_names_this_executable_and_passes_the_link_through() {
        assert_eq!(
            command_line(Path::new(r"C:\Program Files\Easydict\app.exe")),
            r#""C:\Program Files\Easydict\app.exe" "%1""#
        );
    }

    /// The registration is what the whole feature rests on, so it is exercised
    /// against the real registry — under a scratch scheme name, so a test run
    /// cannot take over `easydict://` from an installed application.
    #[test]
    fn registering_and_removing_a_scheme_round_trips_through_the_registry() {
        let scheme = format!("easydict-selftest-{}", uuid::Uuid::new_v4().simple());
        let command = command_line(Path::new(r"C:\scratch\app.exe"));

        assert_eq!(registered_command(&scheme).unwrap(), None);
        assert!(!unregister(&scheme).unwrap(), "nothing was registered yet");

        register(&scheme, &command).unwrap();
        assert_eq!(registered_command(&scheme).unwrap().as_deref(), Some(command.as_str()));

        // Registering again replaces the previous command instead of failing.
        let moved = command_line(Path::new(r"D:\moved\app.exe"));
        register(&scheme, &moved).unwrap();
        assert_eq!(registered_command(&scheme).unwrap().as_deref(), Some(moved.as_str()));

        assert!(unregister(&scheme).unwrap());
        assert_eq!(registered_command(&scheme).unwrap(), None);
        assert!(!unregister(&scheme).unwrap());
    }
}
