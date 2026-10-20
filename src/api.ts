/**
 * Typed wrappers over the Tauri command surface.
 *
 * Every function here maps to one `#[tauri::command]` in
 * `src-tauri/src/commands.rs`. Nothing else in the front-end should call
 * `invoke` directly — keeping the boundary in one file is what makes the Rust
 * and TypeScript types checkable against each other.
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { createActionInbox } from "./hooks/actionInbox";

import type {
  ActionPayload,
  AppInfo,
  DeepPartial,
  DesktopStatus,
  Language,
  LanguageInfo,
  OcrOutput,
  ScreenshotOutput,
  QueryEventPayload,
  QueryFinishedPayload,
  QueryRecord,
  QueryRequest,
  QueryResult,
  Rect,
  ServiceConfig,
  ServiceDescriptor,
  ServiceValidation,
  Settings,
  ShortcutActionDef,
  TtsRequest,
} from "./types";

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

export const getSettings = () => invoke<Settings>("get_settings");

export const getDesktopStatus = () => invoke<DesktopStatus>("get_desktop_status");

export const updateSettings = (patch: DeepPartial<Settings>) =>
  invoke<Settings>("update_settings", { patch });

export const resetSettings = () => invoke<Settings>("reset_settings");

export const exportSettings = (path: string, includeSecrets = false) =>
  invoke<void>("export_settings", { path, includeSecrets });

export const importSettings = (path: string) =>
  invoke<Settings>("import_settings", { path });

// ---------------------------------------------------------------------------
// Services
// ---------------------------------------------------------------------------

export const listServices = () => invoke<ServiceDescriptor[]>("list_services");

export const getServiceConfig = (identifier: string) =>
  invoke<ServiceConfig>("get_service_config", { identifier });

export const updateServiceConfig = (
  identifier: string,
  patch: DeepPartial<ServiceConfig>,
) => invoke<ServiceConfig>("update_service_config", { identifier, patch });

export const validateService = (identifier: string) =>
  invoke<ServiceValidation>("validate_service", { identifier });

export const fetchServiceModels = (identifier: string) =>
  invoke<string[]>("fetch_service_models", { identifier });

export const serviceLanguages = (identifier: string) =>
  invoke<LanguageInfo[]>("service_languages", { identifier });

export const serviceOrder = () => invoke<string[]>("service_order");

export const setServiceOrder = (order: string[]) =>
  invoke<void>("set_service_order", { order });

export const showServiceResult = (identifier: string, windowType: QueryRequest["windowType"]) =>
  invoke<void>("show_service_result", { identifier, windowType });

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

/**
 * Starts a fan-out query. Results arrive on the `query-update` event, not in
 * the promise — the promise resolves as soon as the run is dispatched.
 */
export const query = (request: QueryRequest) => invoke<void>("query", { request });

export const stopQuery = (windowType: QueryRequest["windowType"], requestId?: string) =>
  invoke<void>("stop_query", { windowType, requestId });

export const retryQuery = (request: QueryRequest) =>
  invoke<void>("retry_query", { request });

export const querySingle = (identifier: string, request: QueryRequest) =>
  invoke<QueryResult>("query_single", { identifier, request });

/** Translates through the first service that answers, without touching the UI. */
export const translateText = (request: QueryRequest) =>
  invoke<QueryResult>("translate_text", { request });

/** Uses the dedicated polishing service, never a translation fallback. */
export const polishText = (request: QueryRequest) =>
  invoke<QueryResult>("polish_text", { request });

export const onQueryUpdate = (handler: (payload: QueryEventPayload) => void) =>
  getCurrentWebviewWindow().listen<QueryEventPayload>("query-update", (event) => handler(event.payload));

export const onQueryFinished = (handler: (payload: QueryFinishedPayload) => void) =>
  getCurrentWebviewWindow().listen<QueryFinishedPayload>("query-finished", (event) => handler(event.payload));

export const reportActionError = (error: unknown) => {
  console.error("Window action failed", error);
  void invoke<void>("report_action_error", { error: String(error) }).catch(console.error);
};

export const onAction = createActionInbox(
  () => invoke<ActionPayload | null>("next_action"),
  (wake) => getCurrentWebviewWindow().listen("action", wake),
  reportActionError,
  () => invoke<void>("finish_action"),
);

export const onDesktopStatusChanged = (handler: () => void) =>
  getCurrentWebviewWindow().listen("desktop-status-changed", handler);

export const onSettingsChanged = (handler: (settings: Settings) => void) =>
  listen<Settings>("settings-changed", (event) => handler(event.payload));

export { type UnlistenFn };

// ---------------------------------------------------------------------------
// Clipboard, selection and text actions
// ---------------------------------------------------------------------------

export const clipboardText = () => invoke<string>("clipboard_text");

export const setClipboardText = (text: string) =>
  invoke<void>("set_clipboard_text", { text });

export const selectedText = (target?: ActionPayload["selectionTarget"]) =>
  invoke<string | null>("selected_text", { target: target ?? null });

export const replaceSelectedText = (
  text: string,
  target?: ActionPayload["selectionTarget"],
  expectedText?: string,
) => invoke<void>("replace_selected_text", { text, target: target ?? null, expectedText: expectedText ?? null });

// ---------------------------------------------------------------------------
// OCR
// ---------------------------------------------------------------------------

/** `image` is base64 without a data-URL prefix. */
export const ocrImage = (image: string) =>
  invoke<OcrOutput>("ocr_image", { image });

export const screenshotOcr = (region?: Rect) =>
  invoke<ScreenshotOutput>("screenshot_ocr", { region: region ?? null });

export const pasteboardOcr = () => invoke<OcrOutput>("pasteboard_ocr");

// ---------------------------------------------------------------------------
// Speech
// ---------------------------------------------------------------------------

export const playTts = (request: TtsRequest) => invoke<void>("play_tts", { request });

export const stopTts = () => invoke<void>("stop_tts");

// ---------------------------------------------------------------------------
// Languages
// ---------------------------------------------------------------------------

export const detectLanguage = (text: string) =>
  invoke<Language>("detect_language", { text });

export const supportedLanguages = () =>
  invoke<LanguageInfo[]>("supported_languages");

export const systemLanguages = () => invoke<string[]>("system_languages");

/** Swaps the from/to pickers and returns the new pair. */
export const toggleLanguages = () =>
  invoke<[Language, Language]>("toggle_languages");

// ---------------------------------------------------------------------------
// History and favourites
// ---------------------------------------------------------------------------

export const history = (limit: number, offset = 0) =>
  invoke<QueryRecord[]>("history", { limit, offset });

export const searchHistory = (needle: string, limit: number) =>
  invoke<QueryRecord[]>("search_history", { needle, limit });

export const deleteHistory = (record: QueryRecord) =>
  invoke<void>("delete_history", { record });

export const clearHistory = () => invoke<void>("clear_history");

export const historyCount = () => invoke<number>("history_count");

export const favorites = () => invoke<QueryRecord[]>("favorites");

export const addFavorite = (record: QueryRecord) =>
  invoke<void>("add_favorite", { record });

export const removeFavorite = (record: QueryRecord) =>
  invoke<void>("remove_favorite", { record });

export const isFavorite = (record: QueryRecord) =>
  invoke<boolean>("is_favorite", { record });

// ---------------------------------------------------------------------------
// Windows and app
// ---------------------------------------------------------------------------

export const showWindow = (label: string, position?: string) =>
  invoke<void>("show_window", { label, position: position ?? null });

export const openSettings = (section = "general") =>
  invoke<void>("open_settings", { section });

export const hideWindow = (label: string) => invoke<void>("hide_window", { label });

export const pinWindow = (label: string, pinned: boolean) =>
  invoke<void>("pin_window", { label, pinned });

export const setWindowSize = (
  label: string,
  width: number,
  height: number,
  animate = true,
) => invoke<void>("set_window_size", { label, width, height, animate });

export const fitWindowToContent = (label: string, contentHeight: number) =>
  invoke<{ width: number; height: number }>("fit_window_to_content", { label, contentHeight });

export const openLink = (url: string) => invoke<void>("open_link", { url });

export const appInfo = () => invoke<AppInfo>("app_info");

// ---------------------------------------------------------------------------
// Static tables mirrored from Rust
// ---------------------------------------------------------------------------

/**
 * Mirrors `SHORTCUT_ACTIONS` in `src-tauri/src/config/mod.rs`, in the order the
 * Shortcut tab lists them. Kept here rather than fetched so the settings window
 * can render before the first round-trip.
 */
export const SHORTCUT_ACTIONS: ShortcutActionDef[] = [
  { id: "inputTranslate", titleKey: "menu_input_translate", title: "输入翻译", global: true },
  { id: "snipTranslate", titleKey: "menu_screenshot_Translate", title: "截图翻译", global: true },
  { id: "selectTranslate", titleKey: "menu_selectWord_Translate", title: "划词翻译", global: true },
  { id: "toggleAutoSelectText", titleKey: "shortcut_toggle_auto_select_text", title: "开关自动划词", global: true },
  { id: "showMiniWindow", titleKey: "menu_show_mini_window", title: "显示迷你窗口", global: true },
  { id: "pasteboardTranslate", titleKey: "menu_pasteboard_translate", title: "剪贴板翻译", global: true },
  { id: "polishAndReplace", titleKey: "menu_polish_and_replace", title: "润色并替换", global: true },
  { id: "translateAndReplace", titleKey: "menu_translate_and_replace", title: "翻译并替换", global: true },
  { id: "silentScreenshotOCR", titleKey: "menu_silent_screenshot_OCR", title: "静默截图 OCR", global: true },
  { id: "screenshotOCR", titleKey: "menu_screenshot_OCR", title: "截图 OCR", global: true },
  { id: "pasteboardOCR", titleKey: "menu_pasteboard_OCR", title: "剪贴板 OCR", global: true },
  { id: "showOCRWindow", titleKey: "menu_show_ocr_window", title: "显示 OCR 窗口", global: true },
  { id: "clearInput", titleKey: "shortcut_clear_input", title: "清空输入", global: false },
  { id: "clearAll", titleKey: "shortcut_clear_all", title: "清空全部", global: false },
  { id: "copy", titleKey: "shortcut_copy", title: "复制", global: false },
  { id: "copyFirstResult", titleKey: "shortcut_copy_first_translated_text", title: "复制第一个翻译结果", global: false },
  { id: "focus", titleKey: "shortcut_focus", title: "聚焦输入框", global: false },
  { id: "play", titleKey: "shortcut_play", title: "朗读", global: false },
  { id: "retry", titleKey: "retry", title: "重试", global: false },
  { id: "toggle", titleKey: "toggle_languages", title: "交换语言", global: false },
  { id: "pin", titleKey: "pin", title: "固定窗口", global: false },
  { id: "hide", titleKey: "hide", title: "隐藏窗口", global: false },
  { id: "increaseFontSize", titleKey: "shortcut_increase_font", title: "增大字号", global: false },
  { id: "decreaseFontSize", titleKey: "shortcut_decrease_font", title: "减小字号", global: false },
  { id: "google", titleKey: "open_in_google", title: "在 Google 中打开", global: false },
  { id: "eudic", titleKey: "open_in_eudic", title: "在 Eudic 中打开", global: false },
  { id: "appleDic", titleKey: "open_in_apple_dictionary", title: "在 Apple 词典中打开", global: false },
];

/** The five font-size steps, mirroring `Settings::font_size_ratio`. */
export const FONT_SIZE_RATIOS = [1.0, 1.1, 1.2, 1.3, 1.4];
