/**
 * Window router.
 *
 * All five windows load the same bundle; the window's own label decides which
 * root component mounts. This keeps a single Vite build and a single set of
 * shared hooks, and mirrors how the macOS app shares one `EZWindowManager`
 * across `EZWindowType`s.
 */

import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { OcrApp } from "./ocr/OcrApp";
import { SettingsApp } from "./settings/SettingsApp";
import { useAppearance } from "./hooks/useAppearance";
import { useSettings } from "./hooks/useSettings";
import { reportActionError } from "./api";
import { QueryWindow } from "./windows/QueryWindow";

type WindowKind = "main" | "mini" | "fixed" | "settings" | "ocr";

function App() {
  const [kind, setKind] = useState<WindowKind | null>(null);
  const { settings, error, reload } = useSettings();

  useEffect(() => {
    setKind(getCurrentWindow().label as WindowKind);
  }, []);

  // Theme lives in the window root so it applies before any window renders.
  useAppearance();

  useEffect(() => {
    if (error) reportActionError(`无法读取窗口设置：${error}`);
  }, [error]);

  // Mount no action consumers with fallback language/auto-query preferences.
  // Failed reads keep native actions queued and allow a user-triggered retry.
  if (!settings) {
    return <div className="ezy-loading" role={error ? "alert" : "status"}>
      {error ? `设置加载失败：${error}` : "正在加载设置…"}
      {error && <button className="ez-btn" onClick={() => void reload().catch(reportActionError)}>重试</button>}
    </div>;
  }

  if (kind === null) {
    return null;
  }

  switch (kind) {
    case "settings":
      return <SettingsApp />;
    case "ocr":
      return <OcrApp />;
    case "mini":
      return <QueryWindow kind="mini" />;
    case "fixed":
      return <QueryWindow kind="fixed" />;
    default:
      return <QueryWindow kind="main" />;
  }
}

export { App };
