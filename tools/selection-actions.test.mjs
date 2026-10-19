import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";

const { outputText } = ts.transpileModule(
  readFileSync(new URL("../src/windows/QueryWindow.tsx", import.meta.url), "utf8"),
  { compilerOptions: {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.CommonJS,
    jsx: ts.JsxEmit.ReactJSX,
  } },
);

// Run the actual action handlers and effects; native APIs and child views stay inert.
function panel(options = {}, overrides = {}, native = null) {
  const settings = { autoQuerySelectedText: true, autoQueryWhenTextChanged: true, ...options };
  const calls = { run: [], show: [], clear: 0, selected: 0 };
  const states = [], refs = [], effects = [], timers = new Map();
  let stateIndex, refIndex, effectIndex, scheduled, handler, input, timerId = 0;
  let notifyLayout;
  const content = { scrollHeight: 1000 };
  const api = {
    showWindow: async (label) => { calls.show.push(label); },
    hideWindow: async () => {},
    stopTts: async () => {},
    selectedText: async () => { calls.selected++; return " shortcut "; },
    ...overrides,
  };
  const jsx = (type, props) => {
    if (type === "QueryInput") input = props;
    if (type === "ResultList") props.contentRef.current = content;
    if (type === "div" && props.ref) props.ref.current = { offsetHeight: 60 };
    return { type, props };
  };
  const dependencies = {
    react: {
      useCallback: (callback) => callback,
      useMemo: (callback) => callback(),
      useState: (initial) => {
        const index = stateIndex++;
        if (!(index in states)) states[index] = initial;
        return [states[index], (value) => {
          states[index] = typeof value === "function" ? value(states[index]) : value;
        }];
      },
      useRef: (value) => refs[refIndex++] ??= { current: value },
      useEffect: (callback, deps) => {
        const index = effectIndex++;
        const previous = effects[index];
        if (!previous || deps.some((value, i) => !Object.is(value, previous.deps[i]))) {
          scheduled.push(() => {
            previous?.cleanup?.();
            effects[index] = { deps, cleanup: callback() };
          });
        }
      },
    },
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "@tauri-apps/api/window": { getCurrentWindow: () => native },
    "../api": api,
    "../hooks/useSettings": { useSettings: () => ({ settings }) },
    "../hooks/useAppearance": { useFontRatio: () => 1 },
    // Same rule as `src/hooks/windowSettings.ts`: only a real boolean off hides
    // a part, so the default here is "visible".
    "../hooks/windowSettings": {
      showsInputField: (settings, kind) => settings?.[`inputFieldCellVisible${kind[0].toUpperCase()}${kind.slice(1)}`] !== false,
      showsLanguageBar: (settings, kind) => settings?.[`selectLanguageCellVisible${kind[0].toUpperCase()}${kind.slice(1)}`] !== false,
    },
    "../hooks/useQuery": {
      useActions: (callback) => { handler = callback; },
      useQuery: () => ({
        results: [], running: false, finished: false,
        clear: () => { calls.clear++; },
        run: async (request) => { calls.run.push(request); },
      }),
    },
    "./QueryInput": { QueryInput: "QueryInput" },
    "./ResultList": { ResultList: "ResultList" },
    "./resultUtils": { errorMessage: (error) => String(error) },
  };
  const exports = {};
  runInNewContext(outputText, {
    exports,
    require: (id) => dependencies[id] ?? {},
    document: { hasFocus: () => false },
    window: { addEventListener() {}, removeEventListener() {} },
    ResizeObserver: class {
      constructor(callback) { notifyLayout = callback; }
      observe() {}
      disconnect() {}
    },
    setTimeout: (callback) => { timers.set(++timerId, callback); return timerId; },
    clearTimeout: (id) => timers.delete(id),
  });
  function render() {
    stateIndex = refIndex = effectIndex = 0;
    scheduled = [];
    exports.QueryWindow({ kind: "fixed" });
    for (const effect of scheduled) effect();
  }
  render();
  return {
    calls,
    action: (action, text, selectionTarget) => handler({ action, text, selectionTarget }),
    edit: (value) => input.onChange(value),
    clear: () => input.onClear(),
    layout: (height) => { content.scrollHeight = height; notifyLayout(); },
    get text() { return states[0]; },
    async settle() {
      await new Promise(setImmediate);
      render();
    },
    async tick() {
      const pending = [...timers.values()];
      timers.clear();
      for (const callback of pending) callback();
      await this.settle();
    },
  };
}

function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test("lazy shortcut passes the original foreground token and a changed target cancels silently", async () => {
  const tokens = [];
  const view = panel({}, { selectedText: async (target) => { tokens.push(target); return null; } });
  view.action("selectTranslate", null, [123, 456]);
  await view.settle();
  assert.deepEqual(tokens, [[123, 456]]);
  assert.equal(view.calls.clear, 0);
  assert.equal(view.calls.show.length, 0);
  assert.equal(view.calls.run.length, 0);
});

test("input shortcut opens an empty lazy window and does not query", async () => {
  const view = panel();
  view.action("inputText");
  await view.settle();
  assert.deepEqual(view.calls.show, ["fixed"]);
  assert.equal(view.calls.run.length, 0);
});

test("URL text waiting for first show cannot replace a newer selection", async () => {
  const opened = deferred();
  const view = panel({}, { showWindow: () => opened.promise });
  view.action("inputText", "old URL");
  view.action("selectedTextPreview", "new selection");
  opened.resolve();
  await view.settle();
  assert.equal(view.text, "new selection");
  assert.equal(view.calls.run.length, 0);
});

test("captured selection preview clears old results without querying or rereading selection", async () => {
  const view = panel();
  view.action("selectedTextPreview", "  preview text  ");
  await view.settle();
  await view.tick();
  assert.equal(view.text, "preview text");
  assert.equal(view.calls.clear, 1);
  assert.deepEqual(view.calls.show, ["fixed"]);
  assert.equal(view.calls.selected, 0);
  assert.equal(view.calls.run.length, 0, "text-change auto-query must not defeat preview");
});

test("captured selection queries once with the receiving window and never reads again", async () => {
  const view = panel();
  view.action("selectedText", " captured ");
  await view.settle();
  await view.tick();
  assert.equal(view.calls.run.length, 1);
  assert.equal(view.calls.run[0].text, "captured");
  assert.equal(view.calls.run[0].windowType, "fixed");
  assert.equal(view.calls.selected, 0);
});

test("new preview supersedes a selection waiting for its window to show", async () => {
  const opening = deferred();
  const view = panel({}, { showWindow: () => opening.promise });
  view.action("selectedText", "old");
  view.action("selectedTextPreview", "new");
  opening.resolve();
  await view.settle();
  await view.tick();
  assert.equal(view.text, "new");
  assert.equal(view.calls.run.length, 0);
});

test("shortcut selection honors preview mode even when text-change querying is enabled", async () => {
  const view = panel({ autoQuerySelectedText: false });
  view.action("selectTranslate");
  await view.settle();
  await view.tick();
  assert.equal(view.calls.selected, 1);
  assert.equal(view.text, "shortcut");
  assert.equal(view.calls.run.length, 0);
});

test("late shortcut selection cannot replace a newer mouse preview", async () => {
  const reading = deferred();
  const view = panel({}, { selectedText: () => reading.promise });
  view.action("selectTranslate");
  view.action("selectedTextPreview", "new");
  await view.settle();
  const shows = view.calls.show.length;
  reading.resolve("old");
  await view.settle();
  await view.tick();
  assert.equal(view.text, "new");
  assert.equal(view.calls.run.length, 0);
  assert.equal(view.calls.show.length, shows, "stale completion must not refocus the window");
});

test("late empty selection cannot clear a newer preview", async () => {
  const reading = deferred();
  const view = panel({}, { selectedText: () => reading.promise });
  view.action("selectTranslate");
  view.action("selectedTextPreview", "new");
  await view.settle();
  reading.resolve("");
  await view.settle();
  assert.equal(view.text, "new");
  assert.equal(view.calls.clear, 1);
});

test("late selection errors do not reopen a cleared window", async () => {
  const reading = deferred();
  const view = panel({}, { selectedText: () => reading.promise });
  view.action("selectTranslate");
  view.clear();
  reading.reject(new Error("stale UIA error"));
  await view.settle();
  assert.equal(view.text, "");
  assert.equal(view.calls.show.length, 0);
  assert.equal(view.calls.run.length, 0);
});

test("editing or clearing while a captured selection opens prevents its old query", async () => {
  for (const clear of [false, true]) {
    const opening = deferred();
    const view = panel({ autoQueryWhenTextChanged: false }, { showWindow: () => opening.promise });
    view.action("selectedText", "old");
    if (clear) view.clear();
    else view.edit("edited");
    opening.resolve();
    await view.settle();
    assert.equal(view.text, clear ? "" : "edited");
    assert.equal(view.calls.run.length, 0);
  }
});

test("empty captured payload does not clear, show, query or read", async () => {
  const view = panel();
  view.action("selectedText", " \n ");
  await view.settle();
  assert.equal(view.calls.clear, 0);
  assert.equal(view.calls.show.length, 0);
  assert.equal(view.calls.run.length, 0);
  assert.equal(view.calls.selected, 0);
});

function nativeLayout() {
  let resized;
  return {
    label: "fixed",
    size: { width: 800, height: 600 },
    isMaximized: async () => false,
    innerSize: async function () { return this.size; },
    scaleFactor: async () => 2,
    onFocusChanged: async () => () => {},
    onCloseRequested: async () => () => {},
    onResized: async (callback) => { resized = callback; return () => {}; },
    resized: () => resized(),
  };
}

test("late resize events for a capped client size do not disable content fitting", async () => {
  const native = nativeLayout();
  let fits = 0;
  const view = panel({}, {
    fitWindowToContent: async () => { fits++; return { width: 400, height: 300 }; },
  }, native);
  await view.tick();
  await view.tick();
  native.resized();
  await view.settle();
  view.layout(900);
  await view.tick();
  assert.equal(fits, 2, "compare against applied size, not the taller content");
});

test("a width-only manual resize stops automatic fitting", async () => {
  const native = nativeLayout();
  let fits = 0;
  const view = panel({}, {
    fitWindowToContent: async () => { fits++; return { width: 400, height: 300 }; },
  }, native);
  await view.tick();
  await view.tick();
  native.size = { width: 900, height: 600 };
  native.resized();
  await view.settle();
  view.layout(900);
  await view.tick();
  assert.equal(fits, 1);
});

test("an older fit completion cannot clear a newer fit's resize guard", async () => {
  const native = nativeLayout();
  const first = deferred(), second = deferred();
  let fits = 0;
  const view = panel({}, {
    fitWindowToContent: () => ++fits === 1 ? first.promise : second.promise,
  }, native);
  await view.tick();
  view.layout(900);
  await view.tick();
  first.resolve({ width: 400, height: 300 });
  await view.settle();
  await view.tick();
  native.resized();
  await view.settle();
  second.resolve({ width: 400, height: 300 });
  await view.settle();
  await view.tick();
  view.layout(800);
  await view.tick();
  assert.equal(fits, 3);
});
