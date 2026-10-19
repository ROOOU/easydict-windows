/**
 * The controls every settings tab is built from.
 *
 * Nothing here reaches into the backend: the tabs own their writes and these
 * components only render and report. The one exception is `useDebouncedText`,
 * which exists so a text field can hand back a settled value instead of one
 * call per keystroke — the Rust side round-trips the whole settings document on
 * every write, so typing an API key through `patch` directly would be wasteful.
 */

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";

import type { DeepPartial, ServiceDescriptor, Settings } from "../types";

// ---------------------------------------------------------------------------
// Shared prop surface
// ---------------------------------------------------------------------------

/** A tab that reads the settings document and writes it back through `patch`. */
export interface SettingsTabProps {
  settings: Settings;
  patch: (patch: DeepPartial<Settings>) => Promise<void>;
}

/** A tab that additionally needs a forced re-read (import, reset, service writes). */
export interface TabProps extends SettingsTabProps {
  reload: () => Promise<void>;
}

/** The service registry lives in `SettingsApp`; these tabs render it. */
export interface ServiceListProps {
  services: ServiceDescriptor[];
  reloadServices: () => Promise<void>;
}

export type ServiceTabProps = TabProps & ServiceListProps;

export type AdvancedTabProps = TabProps & ServiceListProps;

// ---------------------------------------------------------------------------
// Icons — one stroke path each, drawn in `currentColor`
// ---------------------------------------------------------------------------

export type IconName =
  | "back"
  | "chevron"
  | "check"
  | "close"
  | "warning"
  | "trash"
  | "refresh"
  | "copy"
  | "eye"
  | "eye-off"
  | "plus"
  | "link"
  | "external"
  | "screenshot"
  | "clipboard"
  | "scan"
  | "search"
  | "folder"
  | "grip";

const ICON_PATHS: Record<IconName, string> = {
  back: "M15 5l-7 7 7 7",
  chevron: "M9 5l7 7-7 7",
  check: "M5 13l4 4L19 7",
  close: "M6 6l12 12M18 6L6 18",
  warning: "M12 4l9 16H3zM12 10v4M12 17.4h.01",
  trash: "M4 7h16M9 7V5h6v2M6 7l1 13h10l1-13",
  refresh: "M20 12a8 8 0 1 1-2.3-5.6M20 4v4.5h-4.5",
  copy: "M8 8h11v11H8zM5 16V5h11",
  eye: "M2 12s3.6-6 10-6 10 6 10 6-3.6 6-10 6-10-6-10-6zM12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6z",
  "eye-off": "M4 4l16 16M10.6 6.3A10 10 0 0 1 12 6c6.4 0 10 6 10 6a17 17 0 0 1-3.4 4M6.2 8.2A17 17 0 0 0 2 12s3.6 6 10 6a10 10 0 0 0 3-.5",
  plus: "M12 5v14M5 12h14",
  link: "M10 14a4 4 0 0 0 5.7 0l3-3A4 4 0 0 0 13 5.3l-1 1M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 11 18.7l1-1",
  external: "M14 4h6v6M20 4l-8 8M18 14v5a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1V7a1 1 0 0 1 1-1h5",
  screenshot: "M3 8V5a2 2 0 0 1 2-2h3M16 3h3a2 2 0 0 1 2 2v3M21 16v3a2 2 0 0 1-2 2h-3M8 21H5a2 2 0 0 1-2-2v-3M3 12h18",
  clipboard: "M9 4h6v3H9zM8 6H6a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V7a1 1 0 0 0-1-1h-2",
  scan: "M4 8V5a1 1 0 0 1 1-1h3M20 8V5a1 1 0 0 0-1-1h-3M4 16v3a1 1 0 0 0 1 1h3M20 16v3a1 1 0 0 1-1 1h-3M8 12h8",
  search: "M11 19a8 8 0 1 0 0-16 8 8 0 0 0 0 16zM21 21l-4.3-4.3",
  folder: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z",
  grip: "M9 6h.01M9 12h.01M9 18h.01M15 6h.01M15 12h.01M15 18h.01",
};

export function Icon({
  name,
  size = 14,
  className,
}: {
  name: IconName;
  size?: number;
  className?: string;
}) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      <path d={ICON_PATHS[name]} />
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

export function Section({
  title,
  description,
  actions,
  children,
}: {
  title?: ReactNode;
  description?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="ezy-section">
      {(title !== undefined || actions !== undefined) && (
        <header className="ezy-section-header">
          <div className="ezy-section-heading">
            {title !== undefined && <h2 className="ezy-section-title">{title}</h2>}
            {description !== undefined && (
              <p className="ezy-section-description">{description}</p>
            )}
          </div>
          {actions !== undefined && <div className="ezy-section-actions">{actions}</div>}
        </header>
      )}
      <div className="ezy-section-body">{children}</div>
    </section>
  );
}

/** Label on the left, control on the right — the macOS preferences row. */
export function Row({
  label,
  description,
  children,
  disabled,
  stacked,
}: {
  label?: ReactNode;
  description?: ReactNode;
  children?: ReactNode;
  disabled?: boolean;
  /** Puts the control under the label, for wide controls such as text areas. */
  stacked?: boolean;
}) {
  return (
    <div
      className="ezy-row"
      data-disabled={disabled === true ? "true" : undefined}
      data-stacked={stacked === true ? "true" : undefined}
    >
      {(label !== undefined || description !== undefined) && (
        <div className="ezy-row-label">
          {label !== undefined && <span className="ezy-row-title">{label}</span>}
          {description !== undefined && (
            <span className="ezy-row-description">{description}</span>
          )}
        </div>
      )}
      {children !== undefined && <div className="ezy-row-control">{children}</div>}
    </div>
  );
}

export function Banner({
  kind,
  children,
  onDismiss,
}: {
  kind: "info" | "success" | "error" | "warning";
  children: ReactNode;
  onDismiss?: () => void;
}) {
  return (
    <div className="ezy-banner" data-kind={kind} role={kind === "error" ? "alert" : undefined}>
      <span className="ezy-banner-icon">
        {kind === "success" ? (
          <Icon name="check" />
        ) : kind === "info" ? (
          <Icon name="clipboard" />
        ) : (
          <Icon name="warning" />
        )}
      </span>
      <span className="ezy-banner-text ez-selectable">{children}</span>
      {onDismiss !== undefined && (
        <button type="button" className="ez-icon-btn" onClick={onDismiss} aria-label="关闭">
          <Icon name="close" size={12} />
        </button>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Controls
// ---------------------------------------------------------------------------

export function Toggle({
  checked,
  onChange,
  disabled,
  label,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      className="ezy-switch"
      disabled={disabled === true}
      onClick={() => onChange(!checked)}
    />
  );
}

export interface SelectOption<T extends string> {
  value: T;
  label: string;
  disabled?: boolean;
}

export function Select<T extends string>({
  value,
  options,
  onChange,
  disabled,
  label,
  width,
}: {
  value: T;
  options: readonly SelectOption<T>[];
  onChange: (next: T) => void;
  disabled?: boolean;
  label?: string;
  width?: number;
}) {
  return (
    <select
      className="ezy-select"
      value={value}
      disabled={disabled === true}
      aria-label={label}
      style={width === undefined ? undefined : { width }}
      // The option list is the only source of values, so the cast is sound.
      onChange={(event) => onChange(event.target.value as T)}
    >
      {options.map((option) => (
        <option key={option.value} value={option.value} disabled={option.disabled === true}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  disabled,
  label,
}: {
  value: T;
  options: readonly SelectOption<T>[];
  onChange: (next: T) => void;
  disabled?: boolean;
  label?: string;
}) {
  return (
    <div className="ezy-segmented" role="radiogroup" aria-label={label}>
      {options.map((option) => (
        <button
          key={option.value}
          type="button"
          role="radio"
          aria-checked={value === option.value}
          disabled={disabled === true || option.disabled === true}
          className="ezy-segment"
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function Slider({
  value,
  min,
  max,
  step,
  onChange,
  disabled,
  format,
  label,
}: {
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (next: number) => void;
  disabled?: boolean;
  format?: (value: number) => string;
  label?: string;
}) {
  return (
    <div className="ezy-slider">
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled === true}
        aria-label={label}
        onChange={(event) => onChange(Number(event.target.value))}
      />
      <span className="ezy-slider-value">{format ? format(value) : String(value)}</span>
    </div>
  );
}

/**
 * A text field that reports its value 300 ms after the last keystroke.
 *
 * The draft lives here so the input stays responsive while the write is in
 * flight; it resynchronises with `value` whenever the field is not dirty, which
 * is what makes an external change (import, reset) visible.
 */
export function useDebouncedText(
  value: string,
  commit: (next: string) => void,
  delay = 300,
): {
  draft: string;
  change: (next: string) => void;
  flush: () => void;
} {
  const [draft, setDraft] = useState(value);
  const pending = useRef<string | null>(null);
  const timer = useRef<number | null>(null);
  const commitRef = useRef(commit);

  useEffect(() => {
    commitRef.current = commit;
  }, [commit]);

  useEffect(() => {
    if (pending.current === null) {
      setDraft(value);
    }
  }, [value]);

  const flush = useCallback(() => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    const next = pending.current;
    pending.current = null;
    if (next !== null) {
      commitRef.current(next);
    }
  }, []);

  const change = useCallback(
    (next: string) => {
      pending.current = next;
      setDraft(next);
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
      }
      timer.current = window.setTimeout(() => {
        timer.current = null;
        const settled = pending.current;
        pending.current = null;
        if (settled !== null) {
          commitRef.current(settled);
        }
      }, delay);
    },
    [delay],
  );

  // Leaving the tab must not swallow the last edit.
  useEffect(
    () => () => {
      if (timer.current !== null) {
        window.clearTimeout(timer.current);
        timer.current = null;
      }
      const next = pending.current;
      pending.current = null;
      if (next !== null) {
        commitRef.current(next);
      }
    },
    [],
  );

  return { draft, change, flush };
}

export interface TextFieldProps {
  value: string;
  onChange: (next: string) => void;
  placeholder?: string;
  disabled?: boolean;
  /** Renders a textarea instead of an input. */
  multiline?: boolean;
  rows?: number;
  label?: string;
  monospace?: boolean;
  /** Milliseconds before the value is reported; `0` writes on every keystroke. */
  delay?: number;
  width?: number;
  type?: "text" | "password";
}

export function TextField({
  value,
  onChange,
  placeholder,
  disabled,
  multiline,
  rows = 3,
  label,
  monospace,
  delay = 300,
  width,
  type = "text",
}: TextFieldProps) {
  const { draft, change, flush } = useDebouncedText(value, onChange, delay);
  const className = monospace === true ? "ezy-input ezy-mono" : "ezy-input";
  const style = width === undefined ? undefined : { width };

  if (multiline === true) {
    return (
      <textarea
        className={`${className} ezy-textarea`}
        value={draft}
        rows={rows}
        placeholder={placeholder}
        disabled={disabled === true}
        aria-label={label}
        style={style}
        onChange={(event) => change(event.target.value)}
        onBlur={flush}
      />
    );
  }

  return (
    <input
      type={type}
      className={className}
      value={draft}
      placeholder={placeholder}
      disabled={disabled === true}
      aria-label={label}
      style={style}
      spellCheck={false}
      onChange={(event) => change(event.target.value)}
      onBlur={flush}
      onKeyDown={(event) => {
        if (event.key === "Enter") {
          flush();
        }
      }}
    />
  );
}

/** A `TextField` that masks its contents, with a reveal button. */
export function SecretField(props: TextFieldProps) {
  const [revealed, setRevealed] = useState(false);
  return (
    <div className="ezy-secret">
      <TextField {...props} type={revealed ? "text" : "password"} />
      <button
        type="button"
        className="ez-icon-btn"
        aria-label={revealed ? "隐藏" : "显示"}
        title={revealed ? "隐藏" : "显示"}
        disabled={props.disabled === true}
        onClick={() => setRevealed((previous) => !previous)}
      >
        <Icon name={revealed ? "eye-off" : "eye"} />
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Small helpers shared by the tabs
// ---------------------------------------------------------------------------

/**
 * The zh-Hans name for a service, keyed by the Rust `nameKey`.
 *
 * The backend falls back to the English `default_name()`; mirroring the macOS
 * app's localisation here keeps the list Chinese without a round-trip.
 */
const SERVICE_NAMES: Record<string, string> = {
  apple_dictionary: "Apple 词典",
  "service.mdict.name": "MDict 词典",
  youdao_dict: "有道词典",
  openai_translate: "OpenAI",
  deepseek_translate: "DeepSeek",
  groq_translate: "Groq",
  zhipu_translate: "智谱 AI",
  minimax_translate: "MiniMax",
  github_models: "GitHub Models",
  built_in_ai: "内置 AI",
  "service.claude_code.name": "Claude Code",
  "service.codex_cli.name": "Codex CLI",
  "service.github_copilot.name": "GitHub Copilot",
  gemini_translate: "Gemini",
  claude_translate: "Claude",
  ollama_translate: "Ollama",
  polishing_service: "润色",
  summary_service: "总结",
  custom_openai: "自定义 OpenAI",
  deepL_translate: "DeepL",
  google_translate: "Google 翻译",
  apple_translate: "Apple 翻译",
  baidu_translate: "百度翻译",
  bing_translate: "必应翻译",
  volcano_translate: "火山翻译",
  niuTrans_translate: "小牛翻译",
  caiyun_translate: "彩云小译",
  tencent_translate: "腾讯翻译",
  ali_translate: "阿里翻译",
  doubao_translate: "豆包翻译",
  system_speech: "系统朗读",
};

/** A service's display name: the user's override, then zh-Hans, then English. */
export function serviceName(service: ServiceDescriptor): string {
  const custom = service.config.name.trim();
  if (custom) {
    return custom;
  }
  return SERVICE_NAMES[service.nameKey] ?? service.name;
}

/** The single character drawn in a service's icon tile. */
export function serviceMonogram(service: ServiceDescriptor): string {
  const name = serviceName(service);
  return name.slice(0, 1).toUpperCase();
}

/** Sorts descriptors the way the result list stacks them. */
export function sortByServiceOrder(
  services: ServiceDescriptor[],
  order: string[],
): ServiceDescriptor[] {
  const rank = new Map<string, number>();
  order.forEach((identifier, index) => rank.set(identifier, index));
  return [...services].sort((left, right) => {
    const a = rank.get(left.identifier) ?? Number.MAX_SAFE_INTEGER;
    const b = rank.get(right.identifier) ?? Number.MAX_SAFE_INTEGER;
    return a === b ? 0 : a - b;
  });
}

/** `2026-10-16 15:47` — the history/favourites time column. */
export function formatTimestamp(milliseconds: number): string {
  const date = new Date(milliseconds);
  if (Number.isNaN(date.getTime())) {
    return "";
  }
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())} ${pad(
    date.getHours(),
  )}:${pad(date.getMinutes())}`;
}

/** The query-type bits the Rust side reports in `supportedQueryType`. */
export const QUERY_TYPE_TRANSLATION = 1;
export const QUERY_TYPE_DICTIONARY = 2;
export const QUERY_TYPE_SENTENCE = 4;

/** Tauri rejects with a plain string; `Error` instances come from the bridge. */
export function errorMessage(error: unknown): string {
  if (typeof error === "string") {
    return error;
  }
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}
