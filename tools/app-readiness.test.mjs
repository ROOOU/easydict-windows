import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";

test("all action-consuming roots wait for real settings; failed loading exposes retry", () => {
  let settings = null, error = null, kind;
  const exports = {};
  const modules = {
    react: { useState: () => [kind, () => {}], useEffect() {} },
    "react/jsx-runtime": { jsx: (type, props) => ({ type, props }), jsxs: (type, props) => ({ type, props }) },
    "./hooks/useSettings": { useSettings: () => ({ settings, error, reload: async () => {} }) },
    "./hooks/useAppearance": { useAppearance() {} },
    "./windows/QueryWindow": { QueryWindow: "query" },
    "./settings/SettingsApp": { SettingsApp: "settings" },
    "./ocr/OcrApp": { OcrApp: "ocr" },
  };
  runInNewContext(ts.transpileModule(
    readFileSync(new URL("../src/App.tsx", import.meta.url), "utf8"),
    { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX } },
  ).outputText, { exports, require: (id) => modules[id] ?? {} });
  for (kind of ["main", "mini", "fixed", "settings", "ocr"]) {
    settings = null;
    error = null;
    assert.equal(exports.App().props.role, "status");
    error = "read failed";
    assert.equal(exports.App().props.role, "alert");
    assert.equal(exports.App().props.children[1].type, "button");
    settings = { queryToLanguage: "Japanese", autoQuerySelectedText: false };
    assert.equal(exports.App().type, ["settings", "ocr"].includes(kind) ? kind : "query");
  }
});
