/**
 * 快捷键 — one row per `SHORTCUT_ACTIONS` entry, split into global and in-app.
 *
 * A binding is stored as the accelerator string the Rust side parses:
 * `Ctrl+Alt+Shift+Key`, joined with `+` — the format of the built-in defaults
 * (`Alt+A`, `Ctrl+=`, `Ctrl+Shift+D`). Capture is a window-level key listener
 * so it sees the combination before the window's own shortcuts do.
 */

import { useEffect, useState } from "react";

import { SHORTCUT_ACTIONS } from "../api";
import { formatAccelerator } from "../shortcuts";
import {
  Icon,
  Row,
  Section,
  type SettingsTabProps,
} from "./components";
import type { DesktopStatus, ShortcutActionDef } from "../types";

export function ShortcutTab({ settings, patch, desktop }: SettingsTabProps & { desktop: DesktopStatus | null }) {
  const [capturing, setCapturing] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ id: string; value: string } | null>(null);

  const bindings = settings.shortcuts;

  useEffect(() => {
    if (capturing === null) {
      return;
    }
    const action = capturing;

    const commit = async (binding: string) => {
      setCapturing(null);
      await patch({ shortcuts: { [action]: binding } });
    };

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) {
        return;
      }
      event.preventDefault();
      event.stopPropagation();

      if (event.key === "Escape") {
        setCapturing(null);
        return;
      }
      if (event.key === "Backspace" || event.key === "Delete") {
        void commit("");
        return;
      }
      const accelerator = formatAccelerator(event);
      if (accelerator === null) {
        // Modifier-only press: keep waiting for the real key.
        return;
      }
      void commit(accelerator);
    };

    window.addEventListener("keydown", onKeyDown, true);
    return () => window.removeEventListener("keydown", onKeyDown, true);
  }, [capturing, patch]);

  const renderRow = (action: ShortcutActionDef) => {
    const binding = (bindings[action.id] ?? "").trim();
    const isCapturing = capturing === action.id;
    const error = desktop?.shortcutErrors[action.id];

    return (
      <Row
        key={action.id}
        label={action.title}
        description={error ?? (action.global ? "全局快捷键" : "应用内快捷键")}
      >
        <div className="ezy-inline">
          {error && (
            <span
              className="ezy-caution"
              title={error}
              aria-label={error}
            >
              <Icon name="warning" />
            </span>
          )}
          {editing?.id === action.id ? (
            <form className="ezy-inline" onSubmit={(event) => {
              event.preventDefault();
              const value = editing.value.trim();
              setEditing(null);
              void patch({ shortcuts: { [action.id]: value } });
            }}>
              <input
                className="ezy-input"
                style={{ width: 180 }}
                aria-label={`${action.title}快捷键组合`}
                placeholder="Ctrl+Alt+F9"
                value={editing.value}
                autoFocus
                onChange={(event) => setEditing({ id: action.id, value: event.target.value })}
                onKeyDown={(event) => {
                  event.stopPropagation();
                  if (event.key === "Escape") {
                    event.preventDefault();
                    setEditing(null);
                  }
                }}
              />
              <button type="submit" className="ez-icon-btn" aria-label={`保存 ${action.title}快捷键`} title="保存">
                <Icon name="check" />
              </button>
              <button type="button" className="ez-icon-btn" aria-label="取消输入快捷键" title="取消" onClick={() => setEditing(null)}>
                <Icon name="close" />
              </button>
            </form>
          ) : <><button
            type="button"
            className="ezy-keycap"
            data-capturing={isCapturing ? "true" : undefined}
            data-empty={binding === "" ? "true" : undefined}
            onClick={() => {
              setEditing(null);
              setCapturing(isCapturing ? null : action.id);
            }}
            onBlur={() => {
              if (isCapturing) {
                setCapturing(null);
              }
            }}
          >
            {isCapturing ? "请按下快捷键…" : binding === "" ? "未设置" : binding}
          </button>
          <button
            type="button"
            className="ez-btn"
            aria-label={`手动输入 ${action.title}快捷键`}
            title="按键被系统拦截时，可手动输入组合"
            onClick={() => {
              setCapturing(null);
              setEditing({ id: action.id, value: binding });
            }}
          >
            输入
          </button>
          <button
            type="button"
            className="ez-icon-btn"
            aria-label={`清除 ${action.title}`}
            title="清除"
            disabled={binding === ""}
            onClick={() => {
              setCapturing(null);
              void patch({ shortcuts: { [action.id]: "" } });
            }}
          >
            <Icon name="close" size={12} />
          </button>
          </>}
        </div>
      </Row>
    );
  };

  const globalActions = SHORTCUT_ACTIONS.filter((action) => action.global);
  const inAppActions = SHORTCUT_ACTIONS.filter((action) => !action.global);

  return (
    <>
      <Section
        title="全局快捷键"
        description="点击按键区域录制，Esc 取消，Backspace 清除。组合被系统拦截而无法录制时，点击「输入」手动填写，例如 Ctrl+Alt+F9；保存时检查占用。"
      >
        {globalActions.map(renderRow)}
      </Section>

      <Section title="应用内快捷键" description="仅在 Easydict 窗口获得焦点时生效。">
        {inAppActions.map(renderRow)}
      </Section>
    </>
  );
}
