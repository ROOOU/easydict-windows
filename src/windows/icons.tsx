/**
 * The icon set.
 *
 * The macOS app ships SF Symbols; there is no icon font on Windows here, so
 * every glyph is a tiny inline SVG drawn on a 24×24 grid with `currentColor`
 * strokes. That keeps icon colouring in CSS (`color:`), which is how the
 * shared `.ez-icon-btn` styles drive their hover state.
 */

import type { ReactNode } from "react";

export interface IconProps {
  size?: number;
  className?: string;
}

function Glyph({
  size = 14,
  className,
  children,
}: IconProps & { children: ReactNode }) {
  return (
    <svg
      className={className}
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
    >
      {children}
    </svg>
  );
}

export function CopyIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <rect x="9" y="9" width="11" height="11" rx="2" />
      <path d="M5 15V6a2 2 0 0 1 2-2h9" />
    </Glyph>
  );
}

export function SpeakerIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M4 9.5h3l4-3.5v12l-4-3.5H4z" />
      <path d="M15.5 9a4 4 0 0 1 0 6" />
      <path d="M18 6.5a7.5 7.5 0 0 1 0 11" />
    </Glyph>
  );
}

export function StopIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <rect x="7" y="7" width="10" height="10" rx="1.6" />
    </Glyph>
  );
}

export function RetryIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M20 12a8 8 0 1 1-2.6-5.9" />
      <path d="M20 4v4.5h-4.5" />
    </Glyph>
  );
}

export function PinIcon({ filled = false, ...props }: IconProps & { filled?: boolean }) {
  return (
    <Glyph {...props}>
      <path
        d="M9 3h6l-1 5 3 3v2H7v-2l3-3z"
        fill={filled ? "currentColor" : "none"}
      />
      <path d="M12 13v8" />
    </Glyph>
  );
}

export function CloseIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M6 6l12 12M18 6L6 18" />
    </Glyph>
  );
}

export function ChevronDownIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M6 9.5l6 6 6-6" />
    </Glyph>
  );
}

export function SwapIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M4 8h13l-3.2-3.2" />
      <path d="M20 16H7l3.2 3.2" />
    </Glyph>
  );
}

export function SearchIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="11" cy="11" r="6" />
      <path d="M15.5 15.5L20 20" />
    </Glyph>
  );
}

export function GearIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="12" cy="12" r="3" />
      <path d="M12 3.5v2M12 18.5v2M3.5 12h2M18.5 12h2M6 6l1.4 1.4M16.6 16.6L18 18M18 6l-1.4 1.4M7.4 16.6L6 18" />
    </Glyph>
  );
}

export function ClockIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 7.5V12l3 2" />
    </Glyph>
  );
}

export function StarIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M12 4l2.5 5.2 5.5.8-4 3.9 1 5.6-5-2.8-5 2.8 1-5.6-4-3.9 5.5-.8z" />
    </Glyph>
  );
}

export function InfoIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 11v5M12 8.2v.2" />
    </Glyph>
  );
}

export function LinkIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M13.5 6.5l1.8-1.8a3.8 3.8 0 0 1 5.4 5.4l-2.6 2.6" />
      <path d="M10.5 17.5l-1.8 1.8a3.8 3.8 0 0 1-5.4-5.4l2.6-2.6" />
      <path d="M9.5 14.5l5-5" />
    </Glyph>
  );
}

export function WarningIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M12 4.5l8 14H4z" />
      <path d="M12 10v4M12 16.5v.2" />
    </Glyph>
  );
}

export function ErrorIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <circle cx="12" cy="12" r="8" />
      <path d="M12 8v5M12 15.5v.2" />
    </Glyph>
  );
}

export function ExternalIcon(props: IconProps) {
  return (
    <Glyph {...props}>
      <path d="M14 5h5v5" />
      <path d="M19 5l-7 7" />
      <path d="M18 14v4a1.5 1.5 0 0 1-1.5 1.5h-10A1.5 1.5 0 0 1 5 18V8a1.5 1.5 0 0 1 1.5-1.5H10" />
    </Glyph>
  );
}
