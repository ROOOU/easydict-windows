import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { win32 } from "node:path";
import test from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";

// Exercise pure helpers without mounting React or opening native dialogs.
function helpers(file, names, dependencies = {}) {
  const source = readFileSync(new URL(`../src/settings/${file}`, import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.CommonJS,
      jsx: ts.JsxEmit.ReactJSX,
    },
  });
  const exports = {};
  runInNewContext(`${outputText}\nObject.assign(exports, { ${names.join(",")} });`, {
    exports,
    require: (id) => dependencies[id] ?? {},
  });
  return exports;
}

const { formatAccelerator, matchesShortcut } = helpers("../shortcuts.ts", ["formatAccelerator", "matchesShortcut"]);
const { transferPath } = helpers("AdvancedTab.tsx", ["transferPath"], {
  "@tauri-apps/api/path": {
    documentDir: async () => "C:\\Users\\test\\Documents",
    isAbsolute: async (path) => win32.isAbsolute(path),
    join: async (...parts) => win32.join(...parts),
  },
});

function key(code, value, modifiers = {}) {
  return { code, key: value, ctrlKey: false, altKey: false, shiftKey: false, metaKey: false, ...modifiers };
}

test("shortcut capture preserves physical keys, modifiers and numpad Enter", () => {
  assert.equal(formatAccelerator(key("Enter", "Enter", { ctrlKey: true })), "Ctrl+Enter");
  assert.equal(formatAccelerator(key("NumpadEnter", "Enter", { ctrlKey: true })), "Ctrl+NumpadEnter");
  assert.equal(formatAccelerator(key("Digit1", "!", { ctrlKey: true, shiftKey: true })), "Ctrl+Shift+1");
  assert.equal(formatAccelerator(key("KeyA", "a", { altKey: true })), "Alt+A");
  assert.equal(formatAccelerator(key("ControlLeft", "Control", { ctrlKey: true })), null);
  const numpad = key("NumpadEnter", "Enter", { ctrlKey: true });
  assert.equal(matchesShortcut(numpad, "Ctrl+NumpadEnter"), true);
  assert.equal(matchesShortcut(numpad, "Ctrl+Enter"), false);
  assert.equal(matchesShortcut(key("Enter", "Enter", { ctrlKey: true }), "Ctrl+NumpadEnter"), false);
});

test("captured shifted digits and punctuation match the same physical keys", () => {
  for (const [code, value] of [
    ["Digit1", "!"], ["Digit2", "@"], ["Digit0", ")"],
    ["Slash", "?"], ["Semicolon", ":"], ["BracketLeft", "{"],
    ["Quote", "\""], ["Backslash", "|"], ["Backquote", "~"],
  ]) {
    const event = key(code, value, { ctrlKey: true, shiftKey: true });
    assert.equal(matchesShortcut(event, formatAccelerator(event)), true, code);
    assert.equal(matchesShortcut({ ...event, shiftKey: false }, formatAccelerator(event)), false, code);
  }
  assert.equal(matchesShortcut(key("KeyQ", "a", { ctrlKey: true }), "Ctrl+Q"), true);
  assert.equal(matchesShortcut(key("KeyQ", "a", { ctrlKey: true }), "Ctrl+A"), false);
});

test("numpad keys never trigger the number row, navigation or punctuation bindings", () => {
  for (const [code, value, other] of [
    ["Numpad1", "1", "1"], ["Numpad1", "End", "End"],
    ["NumpadDecimal", ".", "."], ["NumpadAdd", "+", "="],
    ["NumpadSubtract", "-", "-"], ["NumpadDivide", "/", "/"],
  ]) {
    const event = key(code, value, { ctrlKey: true });
    assert.equal(matchesShortcut(event, `Ctrl+${other}`), false, code);
    assert.equal(formatAccelerator(event), `Ctrl+${code}`, code);
    assert.equal(matchesShortcut(event, formatAccelerator(event)), true, code);
  }
});

test("manual native aliases match and malformed or composing input does not", () => {
  for (const [event, binding] of [
    [key("ArrowUp", "ArrowUp", { altKey: true }), "Option+Up"],
    [key("Numpad1", "1", { ctrlKey: true }), "CommandOrControl+Num1"],
    [key("NumpadAdd", "+", { ctrlKey: true }), "CmdOrCtrl+NumPlus"],
    [key("Equal", "=", { ctrlKey: true }), "Control+Equal"],
    [key("KeyQ", "q", { metaKey: true }), "Command+KeyQ"],
  ]) {
    assert.equal(matchesShortcut(event, binding), true, binding);
  }
  const event = key("KeyK", "k", { ctrlKey: true });
  for (const binding of ["Ctrl++K", "Ctrl+K+Shift", "Ctrl+J+K", "Ctrl+", ""]) {
    assert.equal(matchesShortcut(event, binding), false, binding);
  }
  for (const composing of [{ isComposing: true }, { keyCode: 229 }]) {
    assert.equal(matchesShortcut({ ...event, ...composing }, "Ctrl+K"), false);
    assert.equal(formatAccelerator({ ...event, ...composing }), null);
  }
});

test("default modifiers, font-size aliases and unknown physical keys stay bounded", () => {
  const ctrl = { ctrlKey: true };
  const ctrlShift = { ctrlKey: true, shiftKey: true };
  assert.equal(matchesShortcut(key("KeyK", "k", ctrl), "Ctrl+K"), true);
  assert.equal(matchesShortcut(key("KeyK", "K", ctrlShift), "Ctrl+K"), false);
  assert.equal(matchesShortcut(key("Enter", "Enter", ctrlShift), "Ctrl+Enter"), false);
  assert.equal(matchesShortcut(key("Enter", "Enter", ctrlShift), "Ctrl+Shift+Enter"), true);
  assert.equal(matchesShortcut(key("Equal", "+", ctrlShift), "Ctrl+="), true);
  assert.equal(matchesShortcut(key("Minus", "_", ctrlShift), "Ctrl+-"), true);
  assert.equal(matchesShortcut(key("NumpadAdd", "+", ctrlShift), "Ctrl+="), false);
  assert.equal(matchesShortcut(key("NumpadSubtract", "_", ctrlShift), "Ctrl+-"), false);
  assert.equal(matchesShortcut(key("Unidentified", "k", ctrl), "Ctrl+K"), true);
  assert.equal(formatAccelerator(key("Unidentified", "k", ctrl)), "Ctrl+K");
  assert.equal(matchesShortcut(key("IntlRo", "k", ctrl), "Ctrl+K"), false);
  assert.equal(formatAccelerator(key("IntlRo", "k", ctrl)), null);
});

test("empty and relative transfer paths start in Documents", async () => {
  assert.equal(await transferPath("", "easydict-settings.json"), "C:\\Users\\test\\Documents\\easydict-settings.json");
  assert.equal(await transferPath("   "), "C:\\Users\\test\\Documents");
  assert.equal(await transferPath(" backup.json "), "C:\\Users\\test\\Documents\\backup.json");
});

test("absolute and network transfer paths keep the user's chosen location", async () => {
  assert.equal(await transferPath("D:\\backups\\settings.json"), "D:\\backups\\settings.json");
  assert.equal(await transferPath("\\\\server\\backups\\settings.json"), "\\\\server\\backups\\settings.json");
});
