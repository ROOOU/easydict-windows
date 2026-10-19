import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { runInNewContext } from "node:vm";

// The settings window keeps one scroller for every tab, so a navigation that
// does not reset it opens the next page halfway down. These tests execute the
// real modules: `scroll.ts` directly, and `ServiceTab` with a minimal React so
// the selection change can be driven through its real effect.

function transpile(relative) {
  return ts.transpileModule(readFileSync(new URL(relative, import.meta.url), "utf8"), {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.CommonJS,
      jsx: ts.JsxEmit.ReactJSX,
    },
  }).outputText;
}

function loadScroll() {
  const exports = {};
  runInNewContext(transpile("../src/settings/scroll.ts"), {
    exports,
    require: () => ({}),
  });
  return exports;
}

const scroll = loadScroll();

test("a detached pane is safe to reset and a pane is reset to the top", () => {
  scroll.attachScrollPane(null);
  assert.doesNotThrow(() => scroll.resetScroll());

  const pane = { scrollTop: 640 };
  scroll.attachScrollPane(pane);
  scroll.resetScroll();
  assert.equal(pane.scrollTop, 0);

  // Unmounting detaches it, so a late reset cannot touch a dead node.
  scroll.attachScrollPane(null);
  pane.scrollTop = 120;
  scroll.resetScroll();
  assert.equal(pane.scrollTop, 120);
});

test("opening a service and returning to the list both reset the scroll offset", () => {
  const resets = [];
  const states = [];
  const effects = [];
  let stateIndex = 0;
  let effectIndex = 0;
  let pending = [];
  const nodes = [];
  const jsx = (type, props) => {
    const node = { type, props };
    nodes.push(node);
    return node;
  };
  const components = new Proxy(
    {
      QUERY_TYPE_TRANSLATION: 1,
      QUERY_TYPE_DICTIONARY: 2,
      QUERY_TYPE_SENTENCE: 4,
      sortByServiceOrder: (services) => services,
      serviceName: () => "Test",
      serviceMonogram: () => "T",
      errorMessage: String,
    },
    { get: (target, key) => target[key] ?? key },
  );
  const dependencies = {
    react: {
      useState: (initial) => {
        const index = stateIndex++;
        if (!(index in states)) states[index] = initial;
        return [
          states[index],
          (value) => {
            states[index] = typeof value === "function" ? value(states[index]) : value;
          },
        ];
      },
      useEffect: (callback, deps) => {
        const index = effectIndex++;
        const previous = effects[index];
        if (!previous || deps.some((value, i) => !Object.is(value, previous.deps[i]))) {
          pending.push(() => {
            effects[index] = { deps, cleanup: callback() };
          });
        }
      },
      // The reset runs before paint, so the component uses a layout effect.
      useLayoutEffect: (callback, deps) => {
        const index = effectIndex++;
        const previous = effects[index];
        if (!previous || deps.some((value, i) => !Object.is(value, previous.deps[i]))) {
          pending.push(() => {
            effects[index] = { deps, cleanup: callback() };
          });
        }
      },
      useMemo: (fn) => fn(),
    },
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "./components": components,
    "./scroll": {
      resetScroll: () => resets.push(Date.now()),
    },
    "./ServiceDetail": { ServiceDetail: "ServiceDetail" },
    "../api": {},
  };
  const exports = {};
  runInNewContext(transpile("../src/settings/ServiceTab.tsx"), {
    exports,
    require: (id) => dependencies[id] ?? {},
  });

  const service = {
    identifier: "Google",
    serviceType: "Google",
    nameKey: "service.google.name",
    available: true,
    isStream: false,
    requiresApiKey: false,
    hasApiKey: true,
    allowsMultipleInstances: false,
    config: { enabled: true },
    supportedQueryType: 1,
  };

  function render() {
    stateIndex = effectIndex = 0;
    pending = [];
    nodes.length = 0;
    exports.ServiceTab({
      settings: { serviceOrder: [] },
      patch: async () => {},
      services: [service],
      reloadServices: async () => {},
    });
    for (const effect of pending) effect();
  }

  render();
  assert.equal(resets.length, 1, "the list resets when the tab mounts");

  const row = nodes.find((node) => node.props?.role === "button");
  assert.ok(row, "the service row is rendered");
  row.props.onClick();
  render();
  assert.equal(resets.length, 2, "opening a service resets the offset");

  const back = nodes.find((node) => node.type === "ServiceDetail");
  assert.ok(back, "the detail panel replaces the list");
  back.props.onBack();
  render();
  assert.equal(resets.length, 3, "going back resets the offset again");
  assert.ok(nodes.some((node) => node.props?.role === "button"), "the list is back");
});
