/**
 * The query textarea.
 *
 * Grow-with-content up to a cap (the floating windows get a shorter one), Enter
 * to translate, Shift + Enter for a newline, and the two inline affordances the
 * macOS app keeps in the corner of the field: clear, speak and translate/stop.
 */

import { useEffect, type RefObject } from "react";

import { CloseIcon, SpeakerIcon, StopIcon } from "./icons";

export interface QueryInputProps {
  value: string;
  onChange: (value: string) => void;
  onSubmit: () => void;
  onStop: () => void;
  onClear: () => void;
  onSpeakSource: () => void;
  onPasteText?: (value: string) => void;
  onCompositionChange: (composing: boolean) => void;
  running: boolean;
  /** An action-driven job (OCR, selection, replace) is in flight. */
  busy: boolean;
  textareaRef: RefObject<HTMLTextAreaElement | null>;
  /** Cap for the grown textarea, in pixels. */
  maxHeight?: number;
  placeholder?: string;
}

export function QueryInput({
  value,
  onChange,
  onSubmit,
  onStop,
  onClear,
  onSpeakSource,
  onPasteText,
  onCompositionChange,
  running,
  busy,
  textareaRef,
  maxHeight = 160,
  placeholder = "输入翻译",
}: QueryInputProps) {
  // Re-measure whenever the text changes: the field grows to fit, then scrolls.
  useEffect(() => {
    const element = textareaRef.current;
    if (!element) {
      return;
    }
    element.style.height = "auto";
    const next = Math.min(element.scrollHeight, maxHeight);
    element.style.height = `${next}px`;
    element.style.overflowY = element.scrollHeight > maxHeight ? "auto" : "hidden";
  }, [value, maxHeight, textareaRef]);

  const empty = value.trim().length === 0;
  const working = running || busy;

  return (
    <div className="ez-input-wrap">
      <textarea
        ref={textareaRef}
        className="ez-textarea ez-selectable"
        value={value}
        rows={1}
        spellCheck={false}
        placeholder={placeholder}
        aria-label={placeholder}
        onChange={(event) => onChange(event.target.value)}
        onCompositionStart={() => onCompositionChange(true)}
        onCompositionEnd={() => onCompositionChange(false)}
        onPaste={(event) => {
          if (!onPasteText) return;
          const pasted = event.clipboardData.getData("text/plain");
          if (!pasted) return;
          event.preventDefault();
          const field = event.currentTarget;
          const start = field.selectionStart;
          const next = value.slice(0, start) + pasted + value.slice(field.selectionEnd);
          onChange(next);
          onPasteText(next);
          requestAnimationFrame(() => field.setSelectionRange(start + pasted.length, start + pasted.length));
        }}
        onKeyDown={(event) => {
          // Enter during an IME composition commits the candidate, not the query.
          if (event.nativeEvent.isComposing || event.keyCode === 229) {
            return;
          }
          if (
            event.key === "Enter" &&
            !event.shiftKey &&
            !event.ctrlKey &&
            !event.metaKey &&
            !event.altKey
          ) {
            event.preventDefault();
            onSubmit();
          }
        }}
      />

      <div className="ez-input-actions">
        {empty ? null : (
          <button
            type="button"
            className="ez-icon-btn"
            title="清空输入"
            aria-label="清空输入"
            onClick={onClear}
          >
            <CloseIcon size={13} />
          </button>
        )}

        <button
          type="button"
          className="ez-icon-btn"
          title="发音"
          aria-label="发音"
          disabled={empty}
          onClick={onSpeakSource}
        >
          <SpeakerIcon size={14} />
        </button>

        {working ? (
          <button
            type="button"
            className="ez-btn ez-translate-btn"
            title="停止"
            onClick={onStop}
          >
            <StopIcon size={13} />
            停止
          </button>
        ) : (
          <button
            type="button"
            className="ez-btn ez-btn-primary ez-translate-btn"
            title="翻译"
            disabled={empty}
            onClick={onSubmit}
          >
            翻译
          </button>
        )}
      </div>
    </div>
  );
}
