//! Desktop registrations, with rollback when a settings change cannot commit.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

use crate::config::{Settings, SHORTCUT_ACTIONS};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DesktopStatus {
    pub autostart_enabled: Option<bool>,
    pub autostart_error: Option<String>,
    pub shortcut_errors: BTreeMap<String, String>,
    pub auto_selection: crate::platform::auto_selection::Status,
}

fn parse_binding(binding: &str, global: bool) -> Result<Shortcut, String> {
    let shortcut = binding
        .trim()
        .parse::<Shortcut>()
        .map_err(|_| format!("无法识别快捷键「{binding}」"))?;
    if global {
        if shortcut.key == Code::F12 {
            return Err("F12 为 Windows 调试器保留，请使用其他按键".into());
        }
        let function_key = matches!(
            shortcut.key,
            Code::F1
                | Code::F2
                | Code::F3
                | Code::F4
                | Code::F5
                | Code::F6
                | Code::F7
                | Code::F8
                | Code::F9
                | Code::F10
                | Code::F11
                | Code::F13
                | Code::F14
                | Code::F15
                | Code::F16
                | Code::F17
                | Code::F18
                | Code::F19
                | Code::F20
                | Code::F21
                | Code::F22
                | Code::F23
                | Code::F24
        );
        if !function_key
            && !shortcut
                .mods
                .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
        {
            return Err("全局快捷键请包含 Ctrl、Alt 或 Windows 修饰键，避免拦截正常输入".into());
        }
    }
    Ok(shortcut)
}

fn binding_errors(settings: &Settings) -> BTreeMap<String, String> {
    let mut errors = BTreeMap::new();
    let mut owners = HashMap::<u32, &str>::new();
    for action in SHORTCUT_ACTIONS {
        let binding = settings
            .shortcuts
            .get(action.id)
            .map(String::as_str)
            .unwrap_or("")
            .trim();
        if binding.is_empty() {
            continue;
        }
        match parse_binding(binding, action.global) {
            Ok(shortcut) => {
                if let Some(owner) = owners.insert(shortcut.id(), action.id) {
                    let message = format!("快捷键「{binding}」重复，请为不同操作设置不同组合");
                    errors.insert(owner.to_string(), message.clone());
                    errors.insert(action.id.to_string(), message);
                }
            }
            Err(error) => {
                errors.insert(action.id.to_string(), error);
            }
        }
    }
    errors
}

fn global_bindings(settings: &Settings) -> BTreeMap<u32, Shortcut> {
    let errors = binding_errors(settings);
    SHORTCUT_ACTIONS
        .iter()
        .filter(|action| action.global && !errors.contains_key(action.id))
        .filter_map(|action| {
            settings
                .shortcuts
                .get(action.id)?
                .trim()
                .parse::<Shortcut>()
                .ok()
                .map(|shortcut| (shortcut.id(), shortcut))
        })
        .collect()
}

pub fn status(app: &AppHandle, settings: &Settings) -> DesktopStatus {
    let mut shortcut_errors = binding_errors(settings);
    let manager = app.global_shortcut();
    for action in SHORTCUT_ACTIONS.iter().filter(|action| action.global) {
        let binding = settings
            .shortcuts
            .get(action.id)
            .map(String::as_str)
            .unwrap_or("")
            .trim();
        if !binding.is_empty()
            && !shortcut_errors.contains_key(action.id)
            && !manager.is_registered(binding)
        {
            shortcut_errors.insert(
                action.id.to_string(),
                "未注册成功，可能已被其他程序占用；请更换组合".into(),
            );
        }
    }
    let (autostart_enabled, autostart_error) = match app.autolaunch().is_enabled() {
        Ok(enabled) => (Some(enabled), None),
        Err(error) => (None, Some(format!("无法读取 Windows 自启动状态：{error}"))),
    };
    DesktopStatus {
        autostart_enabled,
        autostart_error,
        shortcut_errors,
        auto_selection: app.try_state::<crate::platform::auto_selection::AutoSelection>()
            .map(|selection| selection.status()).unwrap_or_default(),
    }
}

pub fn initialize_shortcuts(app: &AppHandle, settings: &Settings) {
    let errors = binding_errors(settings);
    for action in SHORTCUT_ACTIONS.iter().filter(|action| action.global) {
        if let Some(error) = errors.get(action.id) {
            log::warn!("{}: {error}", action.id);
            continue;
        }
        if let Some(shortcut) = settings
            .shortcuts
            .get(action.id)
            .and_then(|binding| binding.trim().parse::<Shortcut>().ok())
        {
            if let Err(error) = execute(app, Action::Shortcut(shortcut, true)) {
                log::warn!("{error}");
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Action {
    Shortcut(Shortcut, bool),
    Autostart(bool),
}

#[derive(Clone, Copy, Debug)]
struct Change {
    apply: Action,
    undo: Action,
}

fn shortcut_changes(
    before: &Settings,
    after: &Settings,
    registered: impl Fn(Shortcut) -> bool,
) -> Result<Vec<Change>, String> {
    if before.shortcuts == after.shortcuts {
        return Ok(Vec::new());
    }
    let previous_errors = binding_errors(before);
    for (action, error) in binding_errors(after) {
        // Old invalid bindings must be repairable one row at a time.
        if before.shortcuts.get(&action) != after.shortcuts.get(&action)
            || !previous_errors.contains_key(&action)
        {
            return Err(error);
        }
    }
    let previous = global_bindings(before);
    let desired = global_bindings(after);
    let mut changes = Vec::new();
    // Reserve changed combinations first. An unrelated startup conflict must
    // not prevent users from repairing other bindings one row at a time.
    for action in SHORTCUT_ACTIONS.iter().filter(|action| action.global) {
        if before.shortcuts.get(action.id) == after.shortcuts.get(action.id) {
            continue;
        }
        let Some(shortcut) = after
            .shortcuts
            .get(action.id)
            .and_then(|binding| binding.trim().parse::<Shortcut>().ok())
            .filter(|shortcut| desired.contains_key(&shortcut.id()) && !registered(*shortcut))
        else {
            continue;
        };
        changes.push(Change {
            apply: Action::Shortcut(shortcut, true),
            undo: Action::Shortcut(shortcut, false),
        });
    }
    for (id, shortcut) in previous {
        if !desired.contains_key(&id) && registered(shortcut) {
            changes.push(Change {
                apply: Action::Shortcut(shortcut, false),
                undo: Action::Shortcut(shortcut, true),
            });
        }
    }
    Ok(changes)
}

fn execute(app: &AppHandle, action: Action) -> Result<(), String> {
    match action {
        Action::Shortcut(shortcut, enabled) => {
            let manager = app.global_shortcut();
            if manager.is_registered(shortcut) == enabled {
                return Ok(());
            }
            let result = if enabled {
                manager.register(shortcut)
            } else {
                manager.unregister(shortcut)
            };
            result.map_err(|error| {
                format!(
                    "快捷键「{shortcut}」{}失败，可能存在占用：{error}",
                    if enabled { "注册" } else { "释放" }
                )
            })
        }
        Action::Autostart(enabled) => {
            let manager = app.autolaunch();
            let result = if enabled {
                manager.enable()
            } else {
                manager.disable()
            };
            result.map_err(|error| format!("更改 Windows 自启动失败：{error}"))?;
            let actual = manager
                .is_enabled()
                .map_err(|error| format!("无法确认 Windows 自启动状态：{error}"))?;
            if actual != enabled {
                return Err("Windows 未应用自启动更改，请检查任务管理器或系统策略".into());
            }
            Ok(())
        }
    }
}

pub fn commit(
    app: &AppHandle,
    before: &Settings,
    after: &Settings,
    apply_autostart: bool,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let mut changes = shortcut_changes(before, after, |shortcut| {
        app.global_shortcut().is_registered(shortcut)
    })?;
    if let Some(change) = autostart_change(before, after, apply_autostart, || {
        app.autolaunch()
            .is_enabled()
            .map_err(|error| format!("无法读取 Windows 自启动状态：{error}"))
    })? {
        changes.push(change);
    }
    commit_changes(&changes, |action| execute(app, action), persist)
}

fn autostart_change(
    before: &Settings,
    after: &Settings,
    explicitly_requested: bool,
    read_actual: impl FnOnce() -> Result<bool, String>,
) -> Result<Option<Change>, String> {
    if !explicitly_requested && before.launch_at_startup == after.launch_at_startup {
        return Ok(None);
    }
    // Restore the actual OS value, not a possibly stale saved preference.
    let actual = read_actual()?;
    Ok((actual != after.launch_at_startup).then_some(Change {
        apply: Action::Autostart(after.launch_at_startup),
        undo: Action::Autostart(actual),
    }))
}

fn commit_changes(
    changes: &[Change],
    mut execute: impl FnMut(Action) -> Result<(), String>,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let mut attempted = 0;
    let result = (|| {
        for change in changes {
            attempted += 1;
            execute(change.apply)?;
        }
        persist()
    })();
    if let Err(error) = result {
        let mut rollback_errors = Vec::new();
        // Include the failing action: native APIs may partially apply a change.
        for change in changes[..attempted].iter().rev() {
            if let Err(error) = execute(change.undo) {
                rollback_errors.push(error);
            }
        }
        return Err(if rollback_errors.is_empty() {
            error
        } else {
            format!(
                "{error}；系统状态回滚失败，请检查快捷键和自启动状态：{}",
                rollback_errors.join("；")
            )
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::HashSet;

    #[test]
    fn default_bindings_are_valid_and_alias_collisions_are_rejected() {
        let mut settings = Settings::default();
        assert!(binding_errors(&settings).is_empty());
        settings
            .shortcuts
            .insert("inputTranslate".into(), "Control+Shift+A".into());
        settings
            .shortcuts
            .insert("showMiniWindow".into(), "shift+ctrl+KeyA".into());
        let errors = binding_errors(&settings);
        assert!(errors.contains_key("inputTranslate"));
        assert!(errors.contains_key("showMiniWindow"));
        settings
            .shortcuts
            .insert("showMiniWindow".into(), "Ctrl+Shift+B".into());
        assert!(binding_errors(&settings).is_empty());
        assert!(parse_binding("Ctrl+F12", true).is_err());
        assert!(parse_binding("Shift+A", true).is_err());
        assert!(parse_binding("F8", true).is_ok());
        assert!(parse_binding("Ctrl+", true).is_err());
        assert!(parse_binding("Ctrl+NumpadEnter", true).is_ok());
        assert!(parse_binding("F12", false).is_ok());
    }

    #[test]
    fn unchanged_and_reassigned_keys_are_not_unregistered() {
        let before = Settings::default();
        let mut after = before.clone();
        after
            .shortcuts
            .insert("inputTranslate".into(), "Alt+F".into());
        after
            .shortcuts
            .insert("showMiniWindow".into(), "Alt+A".into());
        assert!(shortcut_changes(&before, &after, |_| true)
            .unwrap()
            .is_empty());
        assert!(shortcut_changes(&before, &before, |_| false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn invalid_legacy_bindings_can_be_repaired_individually() {
        let mut before = Settings::default();
        before
            .shortcuts
            .insert("inputTranslate".into(), "F12".into());
        before
            .shortcuts
            .insert("showMiniWindow".into(), "F12".into());
        let mut after = before.clone();
        after
            .shortcuts
            .insert("inputTranslate".into(), "Alt+A".into());
        let changes = shortcut_changes(&before, &after, |_| false).unwrap();
        assert!(!changes.is_empty());
        assert!(global_bindings(&after)
            .values()
            .all(|key| key.key != Code::F12));
        after.shortcuts.insert("inputTranslate".into(), "A".into());
        assert!(shortcut_changes(&before, &after, |_| false).is_err());
    }

    #[test]
    fn autostart_respects_system_changes_until_explicitly_requested() {
        let mut saved = Settings::default();
        saved.launch_at_startup = true;
        assert!(
            autostart_change(&saved, &saved, false, || panic!("unrelated edit"))
                .unwrap()
                .is_none()
        );
        let change = autostart_change(&saved, &saved, true, || Ok(false))
            .unwrap()
            .unwrap();
        assert!(matches!(change.apply, Action::Autostart(true)));
        assert!(matches!(change.undo, Action::Autostart(false)));
        assert!(autostart_change(&saved, &saved, true, || Ok(true))
            .unwrap()
            .is_none());
        assert!(autostart_change(&saved, &saved, true, || Err("denied".into())).is_err());
    }

    #[test]
    fn unrelated_startup_conflicts_do_not_block_editing_other_bindings() {
        let before = Settings::default();
        let mut after = before.clone();
        after
            .shortcuts
            .insert("inputTranslate".into(), "Ctrl+Alt+F9".into());
        let changes = shortcut_changes(&before, &after, |_| false).unwrap();
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].apply, Action::Shortcut(key, true) if key.key == Code::F9));
        after = before.clone();
        after.shortcuts.insert("clearInput".into(), "Ctrl+J".into());
        assert!(shortcut_changes(&before, &after, |_| false)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn native_conflicts_leave_old_bindings_and_do_not_persist() {
        let before = Settings::default();
        let mut after = before.clone();
        after
            .shortcuts
            .insert("inputTranslate".into(), "Ctrl+Alt+F9".into());
        let original: HashSet<_> = global_bindings(&before).into_keys().collect();
        let mut registered = original.clone();
        let changes =
            shortcut_changes(&before, &after, |key| registered.contains(&key.id())).unwrap();
        let persisted = Cell::new(false);
        let result = commit_changes(
            &changes,
            |action| {
                if let Action::Shortcut(key, enabled) = action {
                    if enabled {
                        return Err("occupied".into());
                    }
                    registered.remove(&key.id());
                }
                Ok(())
            },
            || {
                persisted.set(true);
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!persisted.get());
        assert_eq!(registered, original);
    }

    #[test]
    fn failed_persistence_restores_native_state_in_reverse_order() {
        let before = Settings::default();
        let mut after = before.clone();
        after
            .shortcuts
            .insert("inputTranslate".into(), "Ctrl+Alt+F9".into());
        let original: HashSet<_> = global_bindings(&before).into_keys().collect();
        let mut registered = original.clone();
        let mut changes =
            shortcut_changes(&before, &after, |key| registered.contains(&key.id())).unwrap();
        changes.push(Change {
            apply: Action::Autostart(true),
            undo: Action::Autostart(false),
        });
        let mut autostart = false;
        assert!(commit_changes(
            &changes,
            |action| {
                match action {
                    Action::Shortcut(key, true) => {
                        registered.insert(key.id());
                    }
                    Action::Shortcut(key, false) => {
                        registered.remove(&key.id());
                    }
                    Action::Autostart(value) => {
                        autostart = value;
                    }
                }
                Ok(())
            },
            || Err("disk full".into())
        )
        .is_err());
        assert_eq!(registered, original);
        assert!(!autostart);
    }

    #[test]
    fn partial_native_failures_are_rolled_back_and_rollback_errors_are_reported() {
        let changes = [Change {
            apply: Action::Autostart(true),
            undo: Action::Autostart(false),
        }];
        let mut actual = false;
        assert!(commit_changes(
            &changes,
            |action| {
                let Action::Autostart(value) = action else {
                    unreachable!()
                };
                actual = value;
                if value {
                    Err("partial failure".into())
                } else {
                    Ok(())
                }
            },
            || panic!("must not persist")
        )
        .is_err());
        assert!(!actual);
        let error = commit_changes(&changes, |_| Err("denied".into()), || Ok(())).unwrap_err();
        assert!(error.contains("回滚失败"));
    }
}
