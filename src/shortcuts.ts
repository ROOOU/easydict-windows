/** Shared physical-key spelling for capture and in-app dispatch. */
const SYMBOLS: Record<string, string> = {
  Backquote: "`", Backslash: "\\", BracketLeft: "[", BracketRight: "]",
  Comma: ",", Equal: "=", Minus: "-", Period: ".", Quote: "'",
  Semicolon: ";", Slash: "/",
};

const NAMED_KEYS = new Set([
  "Space", "Enter", "Tab", "Backspace", "Delete", "Escape", "CapsLock",
  "ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight", "Home", "End",
  "PageUp", "PageDown", "Insert", "Pause", "PrintScreen", "ScrollLock", "NumLock",
  "NumpadAdd", "NumpadDecimal", "NumpadDivide", "NumpadEnter", "NumpadEqual",
  "NumpadMultiply", "NumpadSubtract", "AudioVolumeDown", "AudioVolumeUp",
  "AudioVolumeMute", "MediaPlay", "MediaPause", "MediaPlayPause", "MediaStop",
  "MediaTrackNext", "MediaTrackPrevious",
]);

// Aliases accepted by the Rust global-hotkey parser, normalized to DOM codes.
const KEY_CODES: Record<string, string> = {
  ...Object.fromEntries(Object.entries(SYMBOLS).map(([code, symbol]) => [symbol, code.toLowerCase()])),
  ...Object.fromEntries([...Object.keys(SYMBOLS), ...NAMED_KEYS].map((code) => [code.toLowerCase(), code.toLowerCase()])),
  up: "arrowup", down: "arrowdown", left: "arrowleft", right: "arrowright",
  esc: "escape", pausebreak: "pause", numpadplus: "numpadadd",
  volumedown: "audiovolumedown", volumeup: "audiovolumeup", volumemute: "audiovolumemute",
  mediatrackprev: "mediatrackprevious",
};

function keyCode(name: string): string | null {
  let key = name.toLowerCase();
  if (/^(key)?[a-z]$/.test(key)) return `key${key.slice(-1)}`;
  if (/^(digit)?[0-9]$/.test(key)) return `digit${key.slice(-1)}`;
  if (/^num(?!pad)/.test(key) && key !== "numlock") {
    key = key.replace(/^num/, "numpad");
  }
  if (/^numpad[0-9]$/.test(key) || /^f([1-9]|1[0-9]|2[0-4])$/.test(key)) return key;
  return KEY_CODES[key] ?? null;
}

export function acceleratorKeyName(event: KeyboardEvent): string | null {
  const code = event.code;
  if (/^Key[A-Z]$/.test(code)) return code.slice(3);
  if (/^Digit[0-9]$/.test(code)) return code.slice(5);
  if (/^Numpad[0-9]$/.test(code) || /^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return code;
  if (SYMBOLS[code]) return SYMBOLS[code];
  if (NAMED_KEYS.has(code)) return code;
  if ((!code || code === "Unidentified") && /^[a-z0-9]$/i.test(event.key)) {
    return event.key.toUpperCase();
  }
  return null;
}

export function formatAccelerator(event: KeyboardEvent): string | null {
  if (event.isComposing || event.keyCode === 229) return null;
  const key = acceleratorKeyName(event);
  if (key === null) return null;
  return [
    ...(event.ctrlKey ? ["Ctrl"] : []),
    ...(event.altKey ? ["Alt"] : []),
    ...(event.shiftKey ? ["Shift"] : []),
    ...(event.metaKey ? ["Super"] : []),
    key,
  ].join("+");
}

export function matchesShortcut(event: KeyboardEvent, binding: string): boolean {
  if (event.isComposing || event.keyCode === 229) return false;
  const parts = binding.trim().split("+").map((part) => part.trim());
  const wanted = keyCode(parts.pop() ?? "");
  if (wanted === null) return false;

  let ctrl = false;
  let alt = false;
  let shift = false;
  let superKey = false;
  for (const part of parts) {
    switch (part.toLowerCase()) {
      case "ctrl":
      case "control":
      case "commandorcontrol":
      case "commandorctrl":
      case "cmdorcontrol":
      case "cmdorctrl":
        ctrl = true;
        break;
      case "alt":
      case "option":
        alt = true;
        break;
      case "shift":
        shift = true;
        break;
      case "super":
      case "command":
      case "cmd":
        superKey = true;
        break;
      default:
        return false;
    }
  }
  if (event.ctrlKey !== ctrl || event.altKey !== alt || event.metaKey !== superKey) return false;
  // Never use the printed character when a physical code is known: NumLock,
  // Shift and keyboard layouts can otherwise invoke a different binding.
  const pressed = event.code && event.code !== "Unidentified"
    ? event.code
    : event.key === " " ? "Space" : event.key;
  if (keyCode(pressed) !== wanted) return false;
  if (event.shiftKey === shift) return true;
  // Keep the existing Ctrl+plus / Ctrl+underscore font-size convenience.
  return !shift && event.shiftKey &&
    ((wanted === "equal" && event.key === "+") || (wanted === "minus" && event.key === "_"));
}
