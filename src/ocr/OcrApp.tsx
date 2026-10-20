/**
 * The OCR window.
 *
 * Left pane: the recognised text, editable — editing it is how the user fixes a
 * misread before translating. Right pane: the translation, produced by
 * `translateText` either automatically (when 自动翻译 is on) or on demand.
 */

import { useCallback, useEffect, useReducer, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

import * as api from "../api";
import { useLanguages } from "../hooks/useLanguages";
import { useSettings } from "../hooks/useSettings";
import { useActions } from "../hooks/useQuery";
import { emptyOcrState, ocrReducer } from "./ocrState";
import { Banner, Icon, Toggle, errorMessage } from "../settings/components";
// The re-used controls (`.ezy-switch`, `.ezy-banner`) live in the settings
// stylesheet; importing it here keeps this window self-contained.
import "../settings/settings.css";
import "./ocr.css";

/** Where the current image came from — 识别 re-runs the same capture. */
type CaptureSource = "screenshot" | "clipboard";

const SOURCE_LABELS: Record<CaptureSource, string> = {
  screenshot: "截图",
  clipboard: "剪贴板",
};

export function OcrApp() {
  const { settings, patch } = useSettings();
  const languages = useLanguages();

  const [state, dispatch] = useReducer(ocrReducer, emptyOcrState);
  const { output, text, translation, translationService, error } = state;
  const recognising = state.recognitionId !== null;
  const translating = state.translationId !== null;
  const [copied, setCopied] = useState<"text" | "translation" | null>(null);
  const [showLines, setShowLines] = useState(false);
  const [lastSource, setLastSource] = useState<CaptureSource | null>(null);
  const captureRequest = useRef<string | null>(null);
  const translationRequest = useRef<string | null>(null);

  const autoTranslate = settings?.autoQueryOcrText ?? false;
  const autoTranslateRef = useRef(autoTranslate);
  autoTranslateRef.current = autoTranslate;

  const cancel = useCallback(() => {
    captureRequest.current = null;
    translationRequest.current = null;
    dispatch({ type: "cancel" });
  }, []);

  useEffect(() => {
    let active = true;
    const subscription = getCurrentWindow().onCloseRequested(() => {
      if (active) cancel();
    });
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !event.isComposing) cancel();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => {
      active = false;
      captureRequest.current = null;
      translationRequest.current = null;
      window.removeEventListener("keydown", onKeyDown);
      void subscription.then((unlisten) => unlisten()).catch(() => undefined);
    };
  }, [cancel]);

  const translate = useCallback(async (value: string) => {
    const query = value.trim();
    if (query === "") return;
    const id = crypto.randomUUID();
    translationRequest.current = id;
    dispatch({ type: "translate", id });
    try {
      const result = await api.translateText({ text: query });
      if (translationRequest.current !== id) return;
      dispatch({ type: "translated", id, result });
    } catch (cause) {
      if (translationRequest.current !== id) return;
      dispatch({ type: "translationError", id, error: `翻译失败：${errorMessage(cause)}` });
    } finally {
      if (translationRequest.current === id) translationRequest.current = null;
    }
  }, []);

  const capture = async (source: CaptureSource, silent = false) => {
    if (captureRequest.current !== null) return;
    const id = crypto.randomUUID();
    captureRequest.current = id;
    translationRequest.current = null;
    dispatch({ type: "capture", id });
    let showResult = false;
    try {
      showResult = await getCurrentWindow().isVisible();
      if (captureRequest.current !== id) return;
      if (source === "screenshot") {
        await api.hideWindow("ocr");
        await new Promise<void>((resolve) => setTimeout(resolve, 220));
      }
      if (captureRequest.current !== id) return;
      const next = source === "screenshot"
        ? await api.screenshotOcr().then((capture) => {
          showResult = showResult && capture.restoreWindow;
          return capture.output;
        })
        : await api.pasteboardOcr();
      if (captureRequest.current !== id || next === null) return;
      showResult = !silent;
      setLastSource(source);
      setCopied(null);
      dispatch({ type: "recognized", id, output: next });
      if (!silent && autoTranslateRef.current) void translate(next.text);
      if (silent && next.text.trim()) await api.setClipboardText(next.text);
    } catch (cause) {
      if (captureRequest.current !== id) return;
      showResult = true;
      dispatch({ type: "captureError", id, error: `${SOURCE_LABELS[source]}失败：${errorMessage(cause)}` });
    } finally {
      if (captureRequest.current === id) {
        captureRequest.current = null;
        dispatch({ type: "captureEnd", id });
        if (showResult) await api.showWindow("ocr").catch(() => undefined);
      }
    }
  };

  useActions(({ action }) => {
    if (action === "screenshotOCR") return capture("screenshot");
    else if (action === "silentScreenshotOCR") return capture("screenshot", true);
    else if (action === "pasteboardOCR") return capture("clipboard");
  });

  const copy = async (kind: "text" | "translation") => {
    const value = kind === "text" ? text : (translation ?? "");
    if (value === "") {
      return;
    }
    try {
      await api.setClipboardText(value);
      setCopied(kind);
      window.setTimeout(() => setCopied(null), 1500);
    } catch (cause) {
      dispatch({ type: "error", error: `复制失败：${errorMessage(cause)}` });
    }
  };

  const confidence = output?.confidence ?? null;
  const lines = output?.lines ?? [];

  return (
    <div className="ezo-root">
      <div className="ezo-toolbar">
        <button
          type="button"
          className="ez-btn"
          disabled={recognising}
          title="拖动框选识别区域，Esc 或右键取消"
          onClick={() => void capture("screenshot")}
        >
          <Icon name="screenshot" size={13} />
          截图
        </button>
        <button
          type="button"
          className="ez-btn"
          disabled={recognising}
          onClick={() => void capture("clipboard")}
        >
          <Icon name="clipboard" size={13} />
          从剪贴板
        </button>
        <button
          type="button"
          className="ez-btn"
          disabled={recognising || lastSource === null}
          title={
            lastSource === null
              ? "先截图或从剪贴板识别"
              : lastSource === "screenshot" ? "重新选择截图区域" : "重新读取剪贴板图片"
          }
          onClick={() => {
            if (lastSource !== null) {
              void capture(lastSource);
            }
          }}
        >
          {recognising ? <span className="ez-spinner" /> : <Icon name="scan" size={13} />}
          识别
        </button>

        {(recognising || translating) && (
          <button type="button" className="ez-btn" onClick={cancel}>
            取消
          </button>
        )}

        <span className="ezo-toolbar-spacer" />

        <label className="ezo-toggle">
          <Toggle
            checked={autoTranslate}
            label="自动翻译"
            onChange={(value) => {
              void patch({ autoQueryOcrText: value });
              if (value && !recognising && text.trim() !== "" && translation === null) {
                void translate(text);
              }
            }}
          />
          自动翻译
        </label>
        <button
          type="button"
          className="ez-btn"
          disabled={text.trim() === ""}
          onClick={() => void copy("text")}
        >
          <Icon name="copy" size={13} />
          {copied === "text" ? "已复制" : "复制"}
        </button>
      </div>

      {error !== null && (
        <div className="ezo-banner">
          <Banner kind="error" onDismiss={() => dispatch({ type: "error", error: null })}>
            {error}
          </Banner>
        </div>
      )}

      <div className="ezo-panes">
        <section className="ezo-pane">
          <header className="ezo-pane-header">
            <span className="ezo-pane-title">识别文本</span>
            {lines.length > 0 && (
              <button
                type="button"
                className="ez-btn"
                aria-pressed={showLines}
                onClick={() => setShowLines((previous) => !previous)}
              >
                {showLines ? "隐藏行" : `显示行（${lines.length}）`}
              </button>
            )}
          </header>
          <div className="ezo-pane-body">
            <textarea
              className="ezo-text ez-scroll"
              value={text}
              placeholder="点击「截图」或「从剪贴板」，识别结果会出现在这里；也可以直接输入文本。"
              spellCheck={false}
              aria-label="识别文本"
              onChange={(event) => {
                cancel();
                setCopied(null);
                dispatch({ type: "edit", text: event.target.value });
              }}
            />
          </div>
          {showLines && lines.length > 0 && (
            <div className="ezo-lines ez-scroll">
              {lines.map((line, index) => (
                <div key={`${index}-${line.text}`} className="ezo-line">
                  <span className="ezo-line-box">
                    {line.boundingBox.map((value) => Math.round(value)).join(",")}
                  </span>
                  <span className="ez-selectable">{line.text}</span>
                </div>
              ))}
            </div>
          )}
        </section>

        <section className="ezo-pane">
          <header className="ezo-pane-header">
            <span className="ezo-pane-title">翻译</span>
            {translationService !== null && (
              <span className="ez-badge">{translationService}</span>
            )}
            <button
              type="button"
              className="ez-btn"
              disabled={recognising || translating || text.trim() === ""}
              onClick={() => void translate(text)}
            >
              {translating ? <span className="ez-spinner" /> : <Icon name="refresh" size={12} />}
              翻译
            </button>
            <button
              type="button"
              className="ez-icon-btn"
              aria-label="复制译文"
              title="复制译文"
              disabled={(translation ?? "") === ""}
              onClick={() => void copy("translation")}
            >
              <Icon name={copied === "translation" ? "check" : "copy"} size={13} />
            </button>
          </header>
          <div className="ezo-pane-body">
            <div className="ezo-result ez-scroll ez-selectable">
              {translation ?? (
                <span className="ezo-placeholder">
                  {autoTranslate ? "识别完成后会自动翻译。" : "点击「翻译」生成译文。"}
                </span>
              )}
            </div>
          </div>
        </section>
      </div>

      <div className="ezo-status">
        {lastSource !== null && <span>来源：{SOURCE_LABELS[lastSource]}</span>}
        {output !== null && (
          <span>字符：{text.trim() === "" ? 0 : text.length}</span>
        )}
        {output !== null && lines.length > 0 && <span>行数：{lines.length}</span>}
        {output?.language !== null && output?.language !== undefined && (
          <span>语言：{languages.name(output.language)}</span>
        )}
        {confidence !== null && (
          // Vision reports 0…1; Windows.Media.Ocr reports nothing at all, so
          // this only appears if the backend ever fills the field in.
          <span>置信度：{Math.round(confidence * 100)}%</span>
        )}
        <span className="ezo-status-spacer" />
        {translating && <span>正在翻译…</span>}
        {recognising && <span>正在识别…</span>}
      </div>
    </div>
  );
}
