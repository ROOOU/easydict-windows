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
  const calls = { run: [], show: [], clear: 0, selected: 0, translate: [], polish: [], replace: [], errors: [] };
  const states = [], refs = [], effects = [], timers = new Map();
  let stateIndex, refIndex, effectIndex, scheduled, handler, input, timerId = 0;
  let notifyLayout;
  const content = { scrollHeight: 1000 };
  const api = {
    showWindow: async (label) => { calls.show.push(label); },
    hideWindow: async () => {},
    stopTts: async () => {},
    selectedText: async () => { calls.selected++; return " shortcut "; },
    translateText: async (request) => { calls.translate.push(request); return { translatedResults: ["translated"] }; },
    polishText: async (request) => { calls.polish.push(request); return { translatedResults: ["polished"] }; },
    replaceSelectedText: async (text, target) => { calls.replace.push([text, target]); },
    reportActionError: (error) => { calls.errors.push(String(error)); },
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
    "./resultUtils": {
      errorMessage: (error) => String(error),
      translatedTextOf: (result) => result.translatedResults?.join("\n").trim() ?? "",
    },
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
    unmount: () => { for (const effect of effects) effect?.cleanup?.(); },
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

test("polish-and-replace uses only the polishing command and preserves the selection target", async () => {
  const view = panel({ queryFromLanguage: "French", queryToLanguage: "Simplified-Chinese" });
  view.action("polishAndReplace", null, [123, 456]);
  await view.settle();
  assert.equal(view.calls.translate.length, 0);
  assert.equal(view.calls.polish.length, 1);
  assert.equal(view.calls.polish[0].text, "shortcut");
  assert.equal(view.calls.polish[0].from, "auto", "polishing detects the actual source language");
  assert.deepEqual(view.calls.replace, [["polished", [123, 456]]]);
  assert.deepEqual(view.calls.show, [], "replacement must not steal focus");
});

test("translate-and-replace keeps its translation route", async () => {
  const view = panel({ queryFromLanguage: "English", queryToLanguage: "French" });
  view.action("translateAndReplace", null, [123, 456]);
  await view.settle();
  assert.equal(view.calls.polish.length, 0);
  assert.equal(view.calls.translate.length, 1);
  assert.equal(view.calls.translate[0].to, "French");
  assert.deepEqual(view.calls.replace, [["translated", [123, 456]]]);
});

test("the native inbox can await the entire replacement, including the paste", async () => {
  const pasted = deferred();
  const view = panel({}, { replaceSelectedText: () => pasted.promise });
  let finished = false;
  const work = view.action("polishAndReplace", null, [123, 456]);
  assert.equal(typeof work?.then, "function");
  void work.then(() => { finished = true; });
  await view.settle();
  assert.equal(finished, false);
  pasted.resolve();
  await work;
  assert.equal(finished, true);
});

test("replacement passes the exact original selection for a final native recheck", async () => {
  const original = " \r\nSelected text with whitespace \t";
  const replacements = [];
  const view = panel({}, {
    selectedText: async () => original,
    replaceSelectedText: async (...args) => { replacements.push(args); },
  });
  view.action("polishAndReplace", null, [123, 456]);
  await view.settle();
  assert.deepEqual(replacements, [["polished", [123, 456], original]]);
});

test("replacement without an original target stops before selection or network access", async () => {
  const view = panel();
  view.action("polishAndReplace");
  await view.settle();
  assert.equal(view.calls.selected, 0);
  assert.equal(view.calls.polish.length, 0);
  assert.equal(view.calls.replace.length, 0);
  assert.equal(view.calls.errors.length, 1);
});

test("the API wires action commands and the original selection to the native boundary", async () => {
  const calls = [], apiExports = {};
  let complete;
  runInNewContext(ts.transpileModule(
    readFileSync(new URL("../src/api.ts", import.meta.url), "utf8"),
    { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } },
  ).outputText, {
    exports: apiExports,
    require: (id) => ({
      "@tauri-apps/api/core": { invoke: async (command, args) => { calls.push([command, args]); } },
      "./hooks/actionInbox": { createActionInbox: (_next, _listen, _error, done) => { complete = done; } },
    }[id] ?? {}),
  });
  await apiExports.polishText({ text: "sample" });
  await complete();
  await apiExports.replaceSelectedText("replacement", [123, 456], " original ");
  assert.deepEqual(calls.map(([command]) => command), ["polish_text", "finish_action", "replace_selected_text"]);
  assert.equal(calls[2][1].expectedText, " original ");
  const native = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  for (const [command] of calls) assert.ok(native.includes(`commands::${command},`), command);
});

test("replacement rejects empty results and reports failure while hidden without falling back", async () => {
  for (const overrides of [
    { selectedText: async () => "" },
    { polishText: async () => ({ translatedResults: [" "] }) },
    { polishText: async () => { throw new Error("missing polishing key"); } },
  ]) {
    const view = panel({}, overrides);
    view.action("polishAndReplace", null, [123, 456]);
    await view.settle();
    assert.equal(view.calls.translate.length, 0);
    assert.equal(view.calls.replace.length, 0);
    assert.equal(view.calls.errors.length, 1);
    assert.deepEqual(view.calls.show, []);
  }
  const view = panel({}, { selectedText: async () => null });
  view.action("polishAndReplace", null, [123, 456]);
  await view.settle();
  assert.equal(view.calls.errors.length, 0, "a changed native target cancels silently");
  assert.equal(view.calls.polish.length, 0);
});

test("replacement discards a late selection or error after a newer action", async () => {
  for (const reject of [false, true]) {
    const reading = deferred();
    const view = panel({}, { selectedText: () => reading.promise });
    view.action("polishAndReplace", null, [123, 456]);
    view.action("selectedTextPreview", "new selection");
    if (reject) reading.reject(new Error("old read error"));
    else reading.resolve("old selection");
    await view.settle();
    assert.equal(view.calls.polish.length, 0);
    assert.equal(view.calls.translate.length, 0);
    assert.equal(view.calls.replace.length, 0);
    assert.equal(view.calls.errors.length, 0);
  }
});

test("both replacement actions discard late results after edit, clear, unmount or a newer selection", async () => {
  for (const action of ["polishAndReplace", "translateAndReplace"]) {
    for (const invalidate of [
      (view) => view.edit("new input"),
      (view) => view.clear(),
      (view) => view.unmount(),
      (view) => view.action("selectedTextPreview", "new selection"),
    ]) {
      const result = deferred();
      const view = panel({}, {
        polishText: () => result.promise,
        translateText: () => result.promise,
      });
      view.action(action, null, [123, 456]);
      await view.settle();
      invalidate(view);
      result.resolve({ translatedResults: ["stale output"] });
      await view.settle();
      assert.equal(view.calls.replace.length, 0, action);
    }
  }
});

test("a newer replacement wins even if an older service completes or fails last", async () => {
  for (const reject of [false, true]) {
    const first = deferred();
    let requests = 0;
    const view = panel({}, {
      polishText: () => ++requests === 1 ? first.promise : Promise.resolve({ translatedResults: ["new output"] }),
    });
    view.action("polishAndReplace", null, [123, 456]);
    await view.settle();
    view.action("polishAndReplace", null, [123, 456]);
    await view.settle();
    if (reject) first.reject(new Error("old service error"));
    else first.resolve({ translatedResults: ["old output"] });
    await view.settle();
    assert.deepEqual(view.calls.replace, [["new output", [123, 456]]]);
    assert.equal(view.calls.errors.length, 0);
  }
});

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
