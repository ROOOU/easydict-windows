import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

// The helper imports its types only, so the transpiled module has no runtime
// dependencies: no React, no Tauri bridge.
function loadModule(relative) {
  const source = readFileSync(new URL(relative, import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS },
  });
  const exports = {};
  new Function("exports", "require", outputText)(exports, () => ({}));
  return exports;
}

const { windowConfigurationValue, showsInputField, showsLanguageBar } = loadModule(
  "../src/hooks/windowSettings.ts",
);

/** Every field `true`, so a per-window override is what the assertions see. */
const settings = (overrides = {}) => ({
  inputFieldCellVisibleMain: true,
  inputFieldCellVisibleMini: true,
  inputFieldCellVisibleFixed: true,
  selectLanguageCellVisibleMain: true,
  selectLanguageCellVisibleMini: true,
  selectLanguageCellVisibleFixed: true,
  ...overrides,
});

test("each window kind reads its own stored keys", () => {
  assert.equal(showsInputField(settings({ inputFieldCellVisibleMini: false }), "mini"), false);
  assert.equal(showsInputField(settings({ inputFieldCellVisibleMini: false }), "main"), true);
  assert.equal(showsInputField(settings({ inputFieldCellVisibleMini: false }), "fixed"), true);

  assert.equal(showsLanguageBar(settings({ selectLanguageCellVisibleFixed: false }), "fixed"), false);
  assert.equal(showsLanguageBar(settings({ selectLanguageCellVisibleFixed: false }), "main"), true);
  assert.equal(
    windowConfigurationValue(settings({ inputFieldCellVisibleMain: false }), "main", "inputField"),
    false,
  );
  assert.equal(
    windowConfigurationValue(settings({ inputFieldCellVisibleMain: false }), "main", "selectLanguage"),
    true,
  );
});

test("missing or malformed settings keep the upstream default of visible", () => {
  assert.equal(showsInputField(null, "main"), true);
  assert.equal(showsLanguageBar(undefined, "mini"), true);
  assert.equal(showsInputField({}, "fixed"), true);
  // A corrupt document must not hide the input field: only a real boolean off
  // turns it off, which is what the settings switch writes.
  for (const value of [0, 1, "", "false", null, undefined, {}]) {
    assert.equal(
      showsInputField(settings({ inputFieldCellVisibleMain: value }), "main"),
      true,
      String(value),
    );
  }
  assert.equal(showsInputField(settings({ inputFieldCellVisibleMain: false }), "main"), false);
});

test("the query window renders its head parts from the per-window switches", () => {
  const source = readFileSync(new URL("../src/windows/QueryWindow.tsx", import.meta.url), "utf8");
  assert.match(source, /const showInput = showsInputField\(settings, kind\);/);
  assert.match(source, /const showLanguage = showsLanguageBar\(settings, kind\);/);
  assert.match(source, /\{showLanguage \? \(/);
  assert.match(source, /\{showInput \? \(/);
  // The empty head must not keep a padding band and a second separator.
  assert.match(source, /ez-query-head-empty/);
  const css = readFileSync(new URL("../src/windows/windows.css", import.meta.url), "utf8");
  assert.match(css, /\.ez-query-head-empty \{[^}]*padding: 0;[^}]*border-bottom: none;/s);
});

test("the settings tab exposes both switches for all three query windows", () => {
  const source = readFileSync(new URL("../src/settings/AdvancedTab.tsx", import.meta.url), "utf8");
  for (const windowType of ['kind: "main"', 'kind: "mini"', 'kind: "fixed"']) {
    assert.ok(source.includes(windowType), windowType);
  }
  for (const field of [
    "inputFieldCellVisibleMain",
    "inputFieldCellVisibleMini",
    "inputFieldCellVisibleFixed",
    "selectLanguageCellVisibleMain",
    "selectLanguageCellVisibleMini",
    "selectLanguageCellVisibleFixed",
  ]) {
    assert.ok(source.includes(`"${field}"`), field);
  }
  // Both toggles are wired to the same reader the query window uses.
  assert.match(source, /windowConfigurationValue\(settings, kind, "inputField"\)/);
  assert.match(source, /windowConfigurationValue\(settings, kind, "selectLanguage"\)/);
  assert.match(source, /patch\(\{ \[inputField\]: value \}\)/);
  assert.match(source, /patch\(\{ \[selectLanguage\]: value \}\)/);
  assert.match(source, /checked=\{settings\.hideMainWindow\}/);
  assert.match(source, /patch\(\{ hideMainWindow: value \}\)/);
});

test("the settings document carries camelCase keys the Rust config serialises", () => {
  const types = readFileSync(new URL("../src/types.ts", import.meta.url), "utf8");
  for (const field of [
    "inputFieldCellVisibleMain",
    "inputFieldCellVisibleMini",
    "inputFieldCellVisibleFixed",
    "selectLanguageCellVisibleMain",
    "selectLanguageCellVisibleMini",
    "selectLanguageCellVisibleFixed",
  ]) {
    assert.match(types, new RegExp(`\\b${field}: boolean;`), field);
  }

  const config = readFileSync(
    new URL("../src-tauri/src/config/mod.rs", import.meta.url),
    "utf8",
  );
  for (const field of [
    "input_field_cell_visible_main",
    "input_field_cell_visible_mini",
    "input_field_cell_visible_fixed",
    "select_language_cell_visible_main",
    "select_language_cell_visible_mini",
    "select_language_cell_visible_fixed",
  ]) {
    assert.match(config, new RegExp(`pub ${field}: bool`), field);
  }
  assert.match(config, /rename_all = "camelCase"/);
});
