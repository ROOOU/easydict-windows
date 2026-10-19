import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { runInNewContext } from "node:vm";

// Execute the real settings components and their callbacks without native APIs.
function render(file, name, props, api = {}) {
  const nodes = [];
  const jsx = (type, props) => {
    const node = { type, props };
    nodes.push(node);
    return node;
  };
  const components = new Proxy({
    QUERY_TYPE_TRANSLATION: 1, QUERY_TYPE_DICTIONARY: 2, QUERY_TYPE_SENTENCE: 4,
    sortByServiceOrder: (services) => services,
    serviceName: () => "Test", serviceMonogram: () => "T", errorMessage: String,
  }, { get: (target, key) => target[key] ?? key });
  const dependencies = {
    react: {
      useState: (value) => [value, () => {}],
      useEffect: () => {},
      useMemo: (fn) => fn(),
      useCallback: (fn) => fn,
    },
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "./components": components,
    "../api": api,
    "../hooks/useLanguages": { useLanguages: () => ({}) },
    "../hooks/windowSettings": { windowConfigurationValue: () => true },
  };
  const { outputText } = ts.transpileModule(
    readFileSync(new URL(`../src/settings/${file}.tsx`, import.meta.url), "utf8"),
    { compilerOptions: {
      target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX,
    } },
  );
  const exports = {};
  runInNewContext(outputText, { exports, require: (id) => dependencies[id] ?? {} });
  exports[name](props);
  return nodes.filter((node) => node.type === "Toggle").map((node) => node.props);
}

test("each window toggle writes only its own intelligent mode with missing values off", async () => {
  const patches = [];
  const toggles = render("AdvancedTab", "AdvancedTab", {
    settings: { intelligentQueryMode: { mini: true }, serviceOrder: [] },
    services: [], patch: async (patch) => patches.push(JSON.parse(JSON.stringify(patch))),
  }).filter((toggle) => toggle.label.endsWith("智能查询"));
  assert.deepEqual(toggles.map((toggle) => toggle.checked), [false, true, false]);
  for (const toggle of toggles) await toggle.onChange(!toggle.checked);
  assert.deepEqual(patches, [
    { intelligentQueryMode: { main: true } },
    { intelligentQueryMode: { mini: false } },
    { intelligentQueryMode: { fixed: true } },
  ]);
});

test("service participation toggles preserve other bits and never write output modes", async () => {
  const writes = [];
  let reloads = 0;
  const service = {
    identifier: "CustomOpenAI#test", serviceType: "CustomOpenAI",
    configurableIntelligentQuery: true, supportedQueryType: 1,
    config: {
      intelligentQueryTextType: 5, validModels: [], supportedModels: [], model: "",
      endpoint: "", translation: true, dictionary: false, sentence: false,
    },
  };
  const props = { service, settings: {}, onChanged: async () => { reloads++; } };
  const api = { updateServiceConfig: async (id, patch) => {
    writes.push([id, JSON.parse(JSON.stringify(patch))]);
  } };
  const toggles = render("ServiceDetail", "ServiceDetail", props, api)
    .filter((toggle) => toggle.label.startsWith("智能查询"));
  assert.deepEqual(toggles.map((toggle) => toggle.checked), [true, false, true]);
  await toggles[1].onChange(true);
  await toggles[0].onChange(false);
  assert.deepEqual(writes, [
    ["CustomOpenAI#test", { intelligentQueryTextType: 7 }],
    ["CustomOpenAI#test", { intelligentQueryTextType: 4 }],
  ]);
  assert.equal(reloads, 2);
  service.configurableIntelligentQuery = false;
  assert.equal(render("ServiceDetail", "ServiceDetail", props, api)
    .filter((toggle) => toggle.label.startsWith("智能查询")).length, 0);
});
