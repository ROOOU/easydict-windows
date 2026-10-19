/**
 * Window chrome.
 *
 * `mini` and `fixed` are undecorated floating panels, so they draw their own
 * title bar: a drag region in the middle, the pin toggle and the hide button.
 * `main` has native decorations, so it keeps only a compact toolbar row with
 * the same app-level actions — the title bar itself is supplied by Windows.
 */

import type { ReactNode } from "react";

import { ClockIcon, CloseIcon, GearIcon, InfoIcon, PinIcon, StarIcon } from "./icons";

export interface TitleBarProps {
  kind: "main" | "mini" | "fixed";
  pinned: boolean;
  /** An action-driven job (OCR, selection) is in flight. */
  busy?: boolean;
  onTogglePin: () => void;
  onHide: () => void;
  onOpenSettings: () => void;
  onShowHistory: () => void;
  onShowFavorites: () => void;
  onShowAbout: () => void;
}

const KIND_TITLES: Record<"main" | "mini" | "fixed", string> = {
  main: "Easydict",
  mini: "迷你窗口",
  fixed: "固定窗口",
};

export function TitleBar({
  kind,
  pinned,
  busy = false,
  onTogglePin,
  onHide,
  onOpenSettings,
  onShowHistory,
  onShowFavorites,
  onShowAbout,
}: TitleBarProps) {
  const pinButton = (
    <button
      type="button"
      className="ez-icon-btn"
      aria-pressed={pinned}
      title={pinned ? "取消固定" : "固定窗口"}
      aria-label={pinned ? "取消固定" : "固定窗口"}
      onClick={onTogglePin}
    >
      <PinIcon size={14} filled={pinned} />
    </button>
  );

  if (kind === "main") {
    return (
      <div className="ez-query-toolbar">
        <span className="ez-toolbar-title">
          Easydict
          {busy ? <span className="ez-spinner" aria-hidden="true" /> : null}
        </span>
        <div className="ez-toolbar-drag" data-tauri-drag-region />
        <ToolbarButton title="设置" onClick={onOpenSettings}>
          <GearIcon size={14} />
        </ToolbarButton>
        <ToolbarButton title="历史记录" onClick={onShowHistory}>
          <ClockIcon size={14} />
        </ToolbarButton>
        <ToolbarButton title="收藏" onClick={onShowFavorites}>
          <StarIcon size={14} />
        </ToolbarButton>
        <ToolbarButton title="关于 Easydict" onClick={onShowAbout}>
          <InfoIcon size={14} />
        </ToolbarButton>
        {pinButton}
      </div>
    );
  }

  return (
    <div className="ez-titlebar">
      {pinButton}
      <span className="ez-badge ez-titlebar-badge">{KIND_TITLES[kind]}</span>
      <div className="ez-drag" data-tauri-drag-region />
      {busy ? <span className="ez-spinner" aria-hidden="true" /> : null}
      <button
        type="button"
        className="ez-icon-btn"
        title="隐藏窗口"
        aria-label="隐藏窗口"
        onClick={onHide}
      >
        <CloseIcon size={14} />
      </button>
    </div>
  );
}

function ToolbarButton({
  title,
  onClick,
  children,
}: {
  title: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button type="button" className="ez-icon-btn" title={title} aria-label={title} onClick={onClick}>
      {children}
    </button>
  );
}
