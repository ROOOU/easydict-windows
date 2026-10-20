/**
 * The query window — `main`, `mini` and `fixed`.
 *
 * This is the port of Easydict's `EZQueryWindowController` + `EZBaseQueryViewController`:
 * it owns the query lifecycle, the keyboard shortcuts, the tray/global-shortcut
 * actions, auto-copy, speech, and the auto-fit that grows the window with its
 * results. Everything else is presentational and lives in the sibling files.
 *
 * The `label` handed to the window commands is the Tauri window label, which is
 * also how `App.tsx` picked `kind`, so the three kinds map one-to-one onto the
 * `main`, `mini` and `fixed` windows the Rust side registers.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import * as api from "../api";
import { matchesShortcut } from "../shortcuts";
import { useFontRatio } from "../hooks/useAppearance";
import { useActions, useQuery } from "../hooks/useQuery";
import { loadSettings, patchSettings, useSettings } from "../hooks/useSettings";
import { showsInputField, showsLanguageBar } from "../hooks/windowSettings";
import type { ActionPayload, DeepPartial, Language, QueryResult, Settings } from "../types";
import { LanguageBar } from "./LanguageBar";
import { QueryInput } from "./QueryInput";
import { ResultList } from "./ResultList";
import { TitleBar } from "./TitleBar";
import type { SpeakOptions } from "./WordResultView";
import {
  appleDictionaryUrl,
  errorMessage,
  eudicUrl,
  googleSearchUrl,
  isEnglishWord,
  translatedTextOf,
} from "./resultUtils";
import "./windows.css";

/** Where the text being queried came from; drives the auto-copy rules. */
type QuerySource = "input" | "select" | "clipboard" | "ocr";

const AUTOFIT_DEBOUNCE_MS = 80;
const AUTO_QUERY_DEBOUNCE_MS = 400;
const TOAST_MS = 1800;

/**
 * The in-app bindings used when `settings.shortcuts` has no entry for an
 * action, mirroring `default_shortcuts()` in `src-tauri/src/config/mod.rs`.
 * Rust seeds every key, so this is only the safety net for a settings file
 * written before an action existed — without it the Settings tab would show a
 * shortcut that the window no longer reacts to.
 */
const DEFAULT_SHORTCUTS: Record<string, string> = {
  clearInput: "Ctrl+K",
  clearAll: "Ctrl+Shift+K",
  copy: "Ctrl+Shift+C",
  copyFirstResult: "Ctrl+Shift+J",
  focus: "Ctrl+I",
  play: "Ctrl+S",
  retry: "Ctrl+R",
  toggle: "Ctrl+T",
  pin: "Ctrl+P",
  hide: "Ctrl+Y",
  increaseFontSize: "Ctrl+=",
  decreaseFontSize: "Ctrl+-",
  google: "Ctrl+Enter",
  // Upstream's `.eudic` is `⌘⇧Return`; the Shift is what keeps it apart from
  // `.google`'s `⌘Return`.
  eudic: "Ctrl+Shift+Enter",
  appleDic: "Ctrl+Shift+D",
};

/**
 * The accelerator bound to an action: the user's binding when
 * `settings.shortcuts` carries one, the Rust default when the entry is
 * missing. A present-but-empty binding means "unbound" — that is what the
 * Shortcut tab writes when a row is cleared — so it stays empty.
 */
function shortcutBinding(settings: Settings | null, action: string): string {
  const bound: string | undefined = settings?.shortcuts?.[action];
  return bound === undefined ? (DEFAULT_SHORTCUTS[action] ?? "") : bound.trim();
}

const delay = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/** OCR output arrives with ragged lines; the macOS app normalises it too. */
function normaliseOcrText(text: string): string {
  return text
    .split("\n")
    .map((line) => line.replace(/\s+/g, " ").trim())
    .filter((line) => line.length > 0)
    .join("\n");
}

export function QueryWindow({ kind }: { kind: "main" | "mini" | "fixed" }) {
  const { settings } = useSettings();
  const { results, lastIdentifier, running, finished, run, stop, retry, clear } =
    useQuery(kind);
  const fontRatio = useFontRatio();

  const [text, setText] = useState("");
  const [composing, setComposing] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [busy, setBusy] = useState(false);
  const [toast, setToast] = useState<string | null>(null);

  /**
   * Outside Tauri (`vite dev` in a plain browser tab) there is no current
   * window; the panel still renders, it just cannot resize or hide itself.
   */
  const appWindow = useMemo(() => {
    try {
      return getCurrentWindow();
    } catch {
      return null;
    }
  }, []);
  const label = appWindow?.label ?? kind;

  const from = settings?.queryFromLanguage ?? "auto";
  const to = settings?.queryToLanguage ?? "Simplified-Chinese";
  const floating = kind !== "main";

  // `WindowConfigurationKey` — each query window owns whether it shows its
  // input field and its language row (`EZBaseQueryViewController.m:231/233`).
  // The head keeps its element so the auto-fit measurement still sees the
  // window's own offset when both parts are switched off.
  const showInput = showsInputField(settings, kind);
  const showLanguage = showsLanguageBar(settings, kind);

  // --- refs -----------------------------------------------------------------
  // The action handlers below are long-lived closures: they read the latest
  // values through these refs instead of being rebuilt on every render.
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const chromeRef = useRef<HTMLDivElement | null>(null);
  const headRef = useRef<HTMLDivElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);

  const settingsRef = useRef(settings);
  settingsRef.current = settings;
  const resultsRef = useRef(results);
  resultsRef.current = results;
  const textRef = useRef(text);
  textRef.current = text;
  const fromRef = useRef(from);
  fromRef.current = from;
  const toRef = useRef(to);
  toRef.current = to;
  const stopRef = useRef(stop);
  stopRef.current = stop;
  const retryRef = useRef(retry);
  retryRef.current = retry;
  const clearRef = useRef(clear);
  clearRef.current = clear;

  const lastAutoQueryRef = useRef("");
  const sourceRef = useRef<QuerySource>("input");
  const lastQueryRef = useRef("");
  const handledFinishRef = useRef(false);
  const pinAppliedRef = useRef(false);
  const toastTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const manualResizeRef = useRef(false);
  const fittingRef = useRef(false);
  const requestedSizeRef = useRef<{ width: number; height: number } | null>(null);
  const ocrRequestRef = useRef(0);
  const capturingScreenRef = useRef(false);
  const busyCountRef = useRef(0);

  const showToast = useCallback((message: string) => {
    setToast(message);
    if (toastTimerRef.current !== null) {
      clearTimeout(toastTimerRef.current);
    }
    toastTimerRef.current = setTimeout(() => setToast(null), TOAST_MS);
  }, []);

  useEffect(
    () => () => {
      ocrRequestRef.current += 1;
      if (toastTimerRef.current !== null) {
        clearTimeout(toastTimerRef.current);
      }
    },
    [],
  );

  // --- querying -------------------------------------------------------------

  const runQuery = useCallback(
    async (
      value: string,
      source: QuerySource = "input",
      override?: { from?: Language; to?: Language },
    ) => {
      const trimmed = value.trim();
      if (!trimmed) {
        return;
      }
      ocrRequestRef.current += 1;
      sourceRef.current = source;
      lastQueryRef.current = trimmed;
      lastAutoQueryRef.current = trimmed;
      handledFinishRef.current = false;
      try {
        await run({
          text: trimmed,
          from: override?.from ?? fromRef.current,
          to: override?.to ?? toRef.current,
          windowType: kind,
        });
      } catch (error) {
        showToast(errorMessage(error));
      }
    },
    [run, kind, showToast],
  );

  const runQueryRef = useRef(runQuery);
  runQueryRef.current = runQuery;

  const submit = useCallback(() => {
    const value = textRef.current.trim();
    if (!value) {
      return;
    }
    if (running) {
      return;
    }
    void runQuery(value, "input");
  }, [runQuery, running]);

  /**
   * Auto-query: while the setting is on, typing is enough — the query fires
   * `AUTO_QUERY_DEBOUNCE_MS` after the text settles. `lastAutoQueryRef` stops
   * the effect from re-running a query an action just started.
   */
  const autoQuery = settings?.autoQueryWhenTextChanged ?? false;
  useEffect(() => {
    if (!autoQuery || composing) {
      return;
    }
    const trimmed = text.trim();
    if (!trimmed || trimmed === lastAutoQueryRef.current) {
      return;
    }
    const timer = setTimeout(() => {
      void runQueryRef.current(trimmed, "input");
    }, AUTO_QUERY_DEBOUNCE_MS);
    return () => clearTimeout(timer);
  }, [text, autoQuery, composing]);

  // --- clipboard, speech, toast --------------------------------------------

  const copyToClipboard = useCallback(
    async (value: string, message: string) => {
      const trimmed = value.trim();
      if (!trimmed) {
        return;
      }
      try {
        await api.setClipboardText(trimmed);
        showToast(message);
      } catch (error) {
        showToast(errorMessage(error));
      }
    },
    [showToast],
  );

  const speak = useCallback(
    async (value: string, options?: SpeakOptions) => {
      const trimmed = value.trim();
      if (!trimmed) {
        return;
      }
      try {
        await api.playTts({
          text: trimmed,
          language: options?.language ?? fromRef.current,
          from: fromRef.current,
          to: toRef.current,
          isEnglishWord: isEnglishWord(trimmed),
          url: options?.url ?? null,
        });
      } catch (error) {
        showToast(errorMessage(error));
      }
    },
    [showToast],
  );

  /** A card's speaker: its own translation, preferring the URL it returned. */
  const playResult = useCallback(
    (result: QueryResult) => {
      const translated = translatedTextOf(result);
      if (translated) {
        void speak(translated, { language: result.to, url: result.toSpeakUrl });
        return;
      }
      void speak(result.queryText, { language: result.from, url: result.fromSpeakUrl });
    },
    [speak],
  );

  const copyFirstTranslated = useCallback(() => {
    const first = resultsRef.current.find((result) => translatedTextOf(result).length > 0);
    if (!first) {
      showToast("没有可复制的结果");
      return;
    }
    void copyToClipboard(translatedTextOf(first), "已复制");
  }, [copyToClipboard, showToast]);

  // --- window chrome --------------------------------------------------------

  const hideWindow = useCallback(async () => {
    ocrRequestRef.current += 1;
    // Leaving audio playing behind a hidden panel is the one thing the macOS
    // app always avoids, so the hide path stops speech first.
    await api.stopTts().catch(() => undefined);
    await api.hideWindow(label).catch(() => undefined);
  }, [label]);

  const togglePin = useCallback(async () => {
    const next = !pinned;
    setPinned(next);
    try {
      await api.pinWindow(label, next);
      showToast(next ? "已固定窗口" : "已取消固定");
    } catch (error) {
      setPinned(!next);
      showToast(errorMessage(error));
    }
  }, [label, pinned, showToast]);

  // `pinWindowWhenDisplayed`: applied once, the first time the settings land.
  useEffect(() => {
    if (!settings || pinAppliedRef.current) {
      return;
    }
    pinAppliedRef.current = true;
    if (settings.pinWindowWhenDisplayed) {
      setPinned(true);
      void api.pinWindow(label, true).catch(() => undefined);
    }
  }, [settings, label]);

  // The main window starts empty and focused; the floating ones take focus too,
  // because a shortcut-triggered panel is useless if the caret is elsewhere.
  useEffect(() => {
    textareaRef.current?.focus();
  }, []);

  // `selectQueryTextWhenWindowActivate`: reuse the existing text on refocus.
  useEffect(() => {
    if (!appWindow) {
      return;
    }
    let active = true;
    const subscription = appWindow
      .onFocusChanged(({ payload: focused }) => {
        if (active && focused && settingsRef.current?.selectQueryTextWhenWindowActivate) {
          textareaRef.current?.select();
        }
      });
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, [appWindow]);

  useEffect(() => {
    if (!appWindow) return;
    let active = true;
    const subscription = appWindow.onCloseRequested(() => {
      if (active) ocrRequestRef.current += 1;
    });
    return () => {
      active = false;
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, [appWindow]);

  // --- languages ------------------------------------------------------------

  const changeLanguage = useCallback(
    (next: { from?: Language; to?: Language }) => {
      const patch: DeepPartial<Settings> = {};
      if (next.from !== undefined) {
        patch.queryFromLanguage = next.from;
      }
      if (next.to !== undefined) {
        patch.queryToLanguage = next.to;
      }
      void patchSettings(patch)
        .then(() => {
          const value = textRef.current.trim();
          if (value) {
            return runQueryRef.current(value, "input", next);
          }
          return undefined;
        })
        .catch((error: unknown) => showToast(errorMessage(error)));
    },
    [showToast],
  );

  const swapLanguages = useCallback(async () => {
    try {
      const [nextFrom, nextTo] = await api.toggleLanguages();
      await loadSettings(true);
      const value = textRef.current.trim();
      if (value) {
        await runQueryRef.current(value, "input", { from: nextFrom, to: nextTo });
      }
    } catch (error) {
      showToast(errorMessage(error));
    }
  }, [showToast]);

  // --- auto-fit -------------------------------------------------------------

  /**
   * Grows the window to the height its content needs. The measured value is the
   * results column's content height plus the fixed HTML chrome above it. The
   * backend reserves the separate native frame in the work-area ceiling.
   *
   * Manual resizing disables it for the life of the window, and a maximised
   * window is left alone — both are the macOS behaviour.
   */
  useEffect(() => {
    if (!appWindow) {
      return;
    }
    if (label !== "main" && label !== "mini" && label !== "fixed") {
      // The settings and OCR windows own their own sizing.
      return;
    }

    let timer: ReturnType<typeof setTimeout> | null = null;
    let cancelled = false;
    let fitGeneration = 0;
    let activeFits = 0;

    const measure = () => {
      const chrome = chromeRef.current?.offsetHeight ?? 0;
      const head = headRef.current?.offsetHeight ?? 0;
      const content = contentRef.current?.scrollHeight ?? 0;
      return Math.ceil(chrome + head + content);
    };

    const fit = () => {
      if (cancelled || manualResizeRef.current) {
        return;
      }
      const height = measure();
      if (height < 120) {
        return;
      }
      const generation = ++fitGeneration;
      void appWindow
        .isMaximized()
        .then((maximised) => {
          if (cancelled || maximised || manualResizeRef.current || generation !== fitGeneration) {
            return;
          }
          activeFits += 1;
          fittingRef.current = true;
          return api.fitWindowToContent(label, height)
            .then((size) => {
              if (!cancelled && generation === fitGeneration) requestedSizeRef.current = size;
            })
            .finally(() => {
              // Use the applied size (possibly capped), not the content height.
              setTimeout(() => {
                activeFits -= 1;
                if (!cancelled) fittingRef.current = activeFits > 0;
              }, 120);
            });
        })
        .catch(() => undefined);
    };

    const schedule = () => {
      if (timer !== null) {
        clearTimeout(timer);
      }
      timer = setTimeout(fit, AUTOFIT_DEBOUNCE_MS);
    };

    const observer = new ResizeObserver(schedule);
    for (const element of [chromeRef.current, headRef.current, contentRef.current]) {
      if (element) {
        observer.observe(element);
      }
    }
    schedule();

    return () => {
      cancelled = true;
      if (timer !== null) {
        clearTimeout(timer);
      }
      observer.disconnect();
    };
  }, [appWindow, label]);

  // A resize we did not ask for is the user dragging the window edge. The size
  // we last fitted to identifies our own, asynchronously delivered, resize
  // events — a timing flag alone would eventually mis-classify one of them and
  // disable fitting for good.
  useEffect(() => {
    if (!appWindow) {
      return;
    }
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void appWindow
      .onResized(() => {
        if (fittingRef.current) {
          return;
        }
        void (async () => {
          const requested = requestedSizeRef.current;
          if (requested !== null) {
            try {
              const size = await appWindow.innerSize();
              const scale = await appWindow.scaleFactor();
              if (Math.abs(size.height / scale - requested.height) <= 4
                && Math.abs(size.width / scale - requested.width) <= 4) {
                return;
              }
            } catch {
              // Fall through: treat an unreadable size as a user resize.
            }
          }
          if (!cancelled && !fittingRef.current) manualResizeRef.current = true;
        })();
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [appWindow]);

  // --- finish handling: auto-copy ------------------------------------------

  useEffect(() => {
    if (!finished || handledFinishRef.current) {
      return;
    }
    handledFinishRef.current = true;
    const current = settingsRef.current;
    if (!current) {
      return;
    }
    if (current.autoPlayAudio && lastQueryRef.current && document.hasFocus()) {
      const sourceResult = resultsRef.current.find((result) => result.fromSpeakUrl) ?? resultsRef.current[0];
      void speak(lastQueryRef.current, {
        language: sourceResult?.from ?? fromRef.current,
        url: sourceResult?.fromSpeakUrl,
      });
    }
    const source = sourceRef.current;
    if (source === "select" && current.autoCopySelectedText && lastQueryRef.current) {
      void copyToClipboard(lastQueryRef.current, "已复制划词文本");
      return;
    }
    if (source === "ocr" && current.autoCopyOcrText && lastQueryRef.current) {
      void copyToClipboard(lastQueryRef.current, "已复制识别文本");
      return;
    }
    if (current.autoCopyFirstTranslatedText) {
      const first = resultsRef.current.find(
        (result) => translatedTextOf(result).length > 0,
      );
      if (first) {
        void copyToClipboard(translatedTextOf(first), "已复制译文");
      }
    }
  }, [finished, copyToClipboard, speak]);

  // --- actions --------------------------------------------------------------

  const runBusy = useCallback(
    async (work: () => Promise<void>) => {
      busyCountRef.current += 1;
      setBusy(true);
      try {
        await work();
      } catch (error) {
        showToast(errorMessage(error));
      } finally {
        busyCountRef.current -= 1;
        setBusy(busyCountRef.current > 0);
      }
    },
    [showToast],
  );

  const applySelectedText = useCallback(async (selected: string, autoQuery: boolean) => {
    const value = selected.trim();
    if (!value) return;
    const request = ++ocrRequestRef.current;
    clearRef.current();
    lastQueryRef.current = "";
    lastAutoQueryRef.current = value;
    setText(value);
    await api.showWindow(label);
    if (autoQuery && request === ocrRequestRef.current) await runQueryRef.current(value, "select");
  }, [label]);

  const selectTranslate = useCallback(async (target?: ActionPayload["selectionTarget"]) => {
    const request = ++ocrRequestRef.current;
    // Selection reads go to the focused window. If that is this panel, step out
    // of the way so the text the user actually selected gets read, then return.
    let selected: string;
    try {
      if (!target && document.hasFocus()) {
        await api.hideWindow(label);
        // The clipboard fallback needs the previous window to own the focus
        // again before it can send Ctrl+C there.
        await delay(240);
      }
      if (request !== ocrRequestRef.current) return;
      const value = await api.selectedText(target);
      if (value === null) return;
      selected = value.trim();
    } catch (error) {
      if (request !== ocrRequestRef.current) return;
      await api.showWindow(label).catch(() => undefined);
      if (request === ocrRequestRef.current) throw error;
      return;
    }
    if (request !== ocrRequestRef.current) return;
    if (!selected) {
      if (!settingsRef.current?.keepPrevResultWhenSelectedTextIsEmpty) {
        clearRef.current();
        setText("");
        lastAutoQueryRef.current = "";
      }
      showToast("没有获取到选中的文字，请先选中文本再按快捷键");
      await api.showWindow(label).catch(() => undefined);
      return;
    }
    await applySelectedText(selected, settingsRef.current?.autoQuerySelectedText ?? true);
  }, [applySelectedText, label, showToast]);

  const screenshotTranslate = useCallback(async () => {
    if (capturingScreenRef.current) return;
    capturingScreenRef.current = true;
    const request = ++ocrRequestRef.current;
    let showResult = false;
    try {
      showResult = await appWindow?.isVisible() ?? false;
      if (ocrRequestRef.current !== request) return;
      await api.hideWindow(label);
      await delay(220);
      if (ocrRequestRef.current !== request) return;
      const capture = await api.screenshotOcr();
      showResult = showResult && capture.restoreWindow;
      const output = capture.output;
      if (ocrRequestRef.current !== request || output === null) return;
      showResult = true;
      const recognised = normaliseOcrText(output.text);
      if (!recognised) {
        showToast("没有识别到文字");
        return;
      }
      await api.showWindow(label);
      if (ocrRequestRef.current !== request) return;
      setText(recognised);
      await runQueryRef.current(recognised, "ocr");
    } catch (error) {
      if (ocrRequestRef.current !== request) return;
      showResult = true;
      showToast(errorMessage(error));
    } finally {
      capturingScreenRef.current = false;
      if (ocrRequestRef.current === request && showResult) {
        await api.showWindow(label).catch(() => undefined);
      }
    }
  }, [appWindow, label, showToast]);

  const pasteboardTranslate = useCallback(async () => {
    await api.showWindow(label);
    const clipboard = (await api.clipboardText()).trim();
    if (!clipboard) {
      showToast("剪贴板中没有文字");
      return;
    }
    setText(clipboard);
    await runQueryRef.current(clipboard, "clipboard");
  }, [label, showToast]);

  const pasteboardOcr = useCallback(async () => {
    const request = ++ocrRequestRef.current;
    try {
      const output = await api.pasteboardOcr();
      if (ocrRequestRef.current !== request) return;
      const recognised = normaliseOcrText(output.text);
      if (!recognised) {
        showToast("剪贴板图片中没有识别到文字");
        return;
      }
      setText(recognised);
      await runQueryRef.current(recognised, "ocr");
    } catch (error) {
      if (ocrRequestRef.current === request) throw error;
    }
  }, [showToast]);

  const processAndReplace = useCallback(async (
    polish: boolean,
    target?: ActionPayload["selectionTarget"],
  ) => {
    const request = ++ocrRequestRef.current;
    try {
      if (!target) throw new Error("缺少原选区信息，请在原应用使用替换快捷键");
      if (request !== ocrRequestRef.current) return;
      const value = await api.selectedText(target);
      if (request !== ocrRequestRef.current || value === null) return;
      const selected = value.trim();
      if (!selected) throw new Error("没有获取到选中的文字");
      const result = await (polish ? api.polishText : api.translateText)({
        text: selected,
        from: polish ? "auto" : fromRef.current,
        to: toRef.current,
        windowType: kind,
      });
      if (request !== ocrRequestRef.current) return;
      const translated = translatedTextOf(result);
      if (!translated) throw new Error(polish ? "没有获取到润色结果" : "没有获取到译文");
      await api.replaceSelectedText(translated, target, value);
      if (request === ocrRequestRef.current) showToast(polish ? "已润色并替换" : "已替换");
    } catch (error) {
      if (request !== ocrRequestRef.current) return;
      // A toast alone is invisible in this no-window workflow.
      api.reportActionError(error);
      throw error;
    }
  }, [kind, showToast]);

  const toggleAutoSelectText = useCallback(async () => {
    const settings = await loadSettings();
    await patchSettings({ autoShowQueryIcon: !settings.autoShowQueryIcon });
  }, []);

  const applyInputText = useCallback(async (value: string) => {
    const request = ++ocrRequestRef.current;
    await api.showWindow(label);
    if (request !== ocrRequestRef.current) return;
    const next = value.trim();
    if (!next) {
      if (settingsRef.current?.clearQueryWhenInputTranslate) {
        clearRef.current();
        setText("");
        lastAutoQueryRef.current = "";
        lastQueryRef.current = "";
      }
      textareaRef.current?.focus();
      return;
    }
    if (settingsRef.current?.clearQueryWhenInputTranslate) {
      clearRef.current();
    }
    setText(next);
    await runQueryRef.current(next, "input");
  }, [label]);

  /** The tray opens the settings window before emitting these. */
  const getOutOfTheWay = useCallback(async () => {
    if (floating) {
      await api.hideWindow(label).catch(() => undefined);
    }
  }, [floating, label]);

  const handleAction = useCallback(
    async (payload: ActionPayload) => {
      switch (payload.action) {
        case "inputText":
          await runBusy(() => applyInputText(payload.text ?? ""));
          break;
        case "selectTranslate":
          await runBusy(() => selectTranslate(payload.selectionTarget));
          break;
        case "selectedText":
        case "selectedTextPreview":
          await runBusy(() => applySelectedText(payload.text ?? "", payload.action === "selectedText"));
          break;
        case "selectionError":
          await api.showWindow(label);
          showToast(payload.text ?? "无法更改划词设置");
          break;
        case "screenshotTranslate":
        case "snipTranslate":
          await runBusy(screenshotTranslate);
          break;
        case "pasteboardTranslate":
          await runBusy(pasteboardTranslate);
          break;
        case "pasteboardOCR":
          await runBusy(pasteboardOcr);
          break;
        case "translateAndReplace":
        case "polishAndReplace":
          await runBusy(() => processAndReplace(payload.action === "polishAndReplace", payload.selectionTarget));
          break;
        case "toggleAutoSelectText":
          await runBusy(toggleAutoSelectText);
          break;
        case "openSettings":
        case "favorites":
        case "history":
        case "about":
          await getOutOfTheWay();
          break;
        default:
          // OCR-window and global-shortcut actions this window does not own.
          break;
      }
    },
    [
      applyInputText,
      applySelectedText,
      label,
      showToast,
      getOutOfTheWay,
      pasteboardOcr,
      pasteboardTranslate,
      runBusy,
      screenshotTranslate,
      selectTranslate,
      toggleAutoSelectText,
      processAndReplace,
    ],
  );

  useActions(handleAction);

  // --- keyboard shortcuts ---------------------------------------------------

  const stepFontSize = useCallback(async (delta: number) => {
    const current = settingsRef.current;
    if (!current) {
      return;
    }
    const last = api.FONT_SIZE_RATIOS.length - 1;
    const next = Math.min(Math.max(current.fontSizeOptionIndex + delta, 0), last);
    if (next === current.fontSizeOptionIndex) {
      return;
    }
    try {
      await patchSettings({ fontSizeOptionIndex: next });
    } catch (error) {
      showToast(errorMessage(error));
    }
  }, [showToast]);

  /** `.copy` — `copyQueryText`, which puts the input text on the clipboard. */
  const copyQueryText = useCallback(() => {
    void copyToClipboard(textRef.current, "已复制");
  }, [copyToClipboard]);

  /**
   * The title-bar link buttons: `.google`, `.eudic` and `.appleDic` all open
   * the current input text through their scheme. `EZOpenLinkButton` encodes
   * the trimmed text into the link, and an empty input opens nothing.
   */
  const openQueryIn = useCallback(
    (url: (text: string) => string) => {
      const value = textRef.current.trim();
      if (!value) {
        return;
      }
      void api
        .openLink(url(value))
        .catch((error: unknown) => showToast(errorMessage(error)));
    },
    [showToast],
  );

  const clearInput = useCallback(() => {
    ocrRequestRef.current += 1;
    setText("");
    lastAutoQueryRef.current = "";
    textareaRef.current?.focus();
  }, []);

  const clearAll = useCallback(() => {
    ocrRequestRef.current += 1;
    setText("");
    lastAutoQueryRef.current = "";
    lastQueryRef.current = "";
    clearRef.current();
    textareaRef.current?.focus();
  }, []);

  const handleShortcut = useCallback(
    (event: KeyboardEvent): boolean => {
      // Escape is not bindable; the floating windows keep it as "close", the
      // macOS `closeWindowOrExitSreenshot`.
      if (event.key === "Escape" && floating && !event.ctrlKey && !event.metaKey) {
        void hideWindow();
        return true;
      }

      // Every in-app action of `ShortcutAction`, keyed by its settings id. The
      // binding decides which key runs it — `ShortcutManager`'s job on macOS —
      // and the order here breaks a tie between two equal bindings.
      const actions: Record<string, () => void> = {
        clearInput,
        clearAll,
        copy: copyQueryText,
        copyFirstResult: copyFirstTranslated,
        focus: () => textareaRef.current?.focus(),
        play: () => void speak(textRef.current),
        retry: () => void retryRef.current(),
        toggle: () => void swapLanguages(),
        pin: () => void togglePin(),
        hide: () => void hideWindow(),
        increaseFontSize: () => void stepFontSize(1),
        decreaseFontSize: () => void stepFontSize(-1),
        google: () => openQueryIn(googleSearchUrl),
        eudic: () => openQueryIn(eudicUrl),
        appleDic: () => openQueryIn(appleDictionaryUrl),
      };

      for (const [action, run] of Object.entries(actions)) {
        const binding = shortcutBinding(settingsRef.current, action);
        if (binding !== "" && matchesShortcut(event, binding)) {
          run();
          return true;
        }
      }
      return false;
    },
    [
      clearAll,
      clearInput,
      copyFirstTranslated,
      copyQueryText,
      floating,
      hideWindow,
      openQueryIn,
      speak,
      stepFontSize,
      swapLanguages,
      togglePin,
    ],
  );

  const shortcutRef = useRef(handleShortcut);
  shortcutRef.current = handleShortcut;

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.isComposing || event.keyCode === 229) return;
      if (shortcutRef.current(event)) {
        event.preventDefault();
        event.stopPropagation();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);

  // --- render ---------------------------------------------------------------

  const openSettings = useCallback((section = "general") => {
    void api.openSettings(section).catch((error: unknown) => showToast(errorMessage(error)));
  }, [showToast]);

  const placeholder = floating ? "输入翻译" : "输入要翻译的文字";

  return (
    <div
      className={`ez-window ez-query-window ez-query-${kind}${
        floating ? " ez-window-floating" : ""
      }`}
    >
      <div className="ez-chrome" ref={chromeRef}>
        <TitleBar
          kind={kind}
          pinned={pinned}
          busy={busy}
          onTogglePin={() => void togglePin()}
          onHide={() => void hideWindow()}
          onOpenSettings={() => openSettings()}
          onShowHistory={() => openSettings("history")}
          onShowFavorites={() => openSettings("favorites")}
          onShowAbout={() => openSettings("about")}
        />
      </div>

      <div
        className={`ez-query-head${showInput || showLanguage ? "" : " ez-query-head-empty"}`}
        ref={headRef}
      >
        {showLanguage ? (
          <LanguageBar
            from={from}
            to={to}
            onChange={changeLanguage}
            onSwap={() => void swapLanguages()}
          />
        ) : null}
        {showInput ? (
          <QueryInput
            value={text}
            onChange={(value) => {
              ocrRequestRef.current += 1;
              setText(value);
            }}
            onCompositionChange={setComposing}
            onPasteText={settings?.autoQueryPastedText
              ? (value) => void runQuery(value, "clipboard")
              : undefined}
            onSubmit={submit}
            onStop={() => void stopRef.current()}
            onClear={clearAll}
            onSpeakSource={() => void speak(textRef.current)}
            running={running}
            busy={busy}
            textareaRef={textareaRef}
            maxHeight={floating ? 120 : 180}
            placeholder={placeholder}
          />
        ) : null}
      </div>

      <ResultList
        results={results}
        queryText={lastQueryRef.current || text}
        fontRatio={fontRatio}
        markdownEnabled={settings?.enableMarkdownRendering ?? false}
        lastIdentifier={lastIdentifier}
        running={running}
        contentRef={contentRef}
        onCopy={(value, message) => void copyToClipboard(value, message ?? "已复制")}
        onToast={showToast}
        onStop={() => void stopRef.current()}
        onRetry={() => void retryRef.current()}
        onPlay={playResult}
        onSpeak={(value, options) => void speak(value, options)}
        onQueryWord={(value) => {
          setText(value);
          void runQueryRef.current(value, "input");
        }}
      />

      {toast !== null ? (
        <div className="ez-toast" role="status">
          {toast}
        </div>
      ) : null}
    </div>
  );
}
