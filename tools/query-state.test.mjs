import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const source = readFileSync(new URL("../src/hooks/queryState.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
});
const { emptyQueryState, queryReducer } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString("base64")}`);
const start = (requestId) => queryReducer(emptyQueryState, { type: "start", requestId });
const update = (requestId, identifier = "Google", generation = 1) => ({
  type: "update",
  payload: { requestId, identifier, serviceType: identifier, result: {
    queryText: requestId, generation, isLoading: true, isShowing: true, isStreamFinished: false,
  } },
});

test("old translation frames cannot overwrite a newer request", () => {
  const state = start("new");
  assert.equal(queryReducer(state, update("old")), state);
});

test("an old completion cannot stop a newer query", () => {
  const state = start("new");
  assert.equal(queryReducer(state, { type: "finish", payload: { requestId: "old", cancelled: false } }), state);
  assert.equal(state.running, true);
});

test("clearing input invalidates both queued frames and completion", () => {
  let state = queryReducer(start("old"), update("old"));
  state = queryReducer(state, { type: "clear" });
  assert.equal(queryReducer(state, update("old")), emptyQueryState);
  assert.equal(queryReducer(state, { type: "finish", payload: { requestId: "old", cancelled: false } }), emptyQueryState);
});

test("stop keeps partial text but removes loading indicators", () => {
  const state = queryReducer(queryReducer(start("active"), update("active")), { type: "stop" });
  assert.equal(state.requestId, null);
  assert.equal(state.running, false);
  assert.equal(state.finished, false);
  assert.equal(state.byIdentifier.Google.queryText, "active");
  assert.equal(state.byIdentifier.Google.isLoading, false);
  assert.equal(queryReducer(state, update("active")), state);
});

test("cancellation is not a successful finish for auto-copy or audio", () => {
  const state = queryReducer(queryReducer(start("active"), update("active")), {
    type: "finish", payload: { requestId: "active", cancelled: true },
  });
  assert.equal(state.finished, false);
  assert.equal(state.running, false);
  assert.equal(state.byIdentifier.Google.isLoading, false);
});

test("new queries discard cards from disabled or previous services", () => {
  const old = queryReducer(start("old"), update("old", "Disabled"));
  const state = queryReducer(old, { type: "start", requestId: "new" });
  assert.deepEqual(state.byIdentifier, {});
  assert.equal(state.lastIdentifier, null);
});

test("a successful finish enables post-query actions", () => {
  const state = queryReducer(start("active"), { type: "finish", payload: { requestId: "active", cancelled: false } });
  assert.equal(state.running, false);
  assert.equal(state.finished, true);
});

test("an older service generation cannot replace a newer card", () => {
  const state = queryReducer(start("active"), update("active", "Google", 2));
  assert.equal(queryReducer(state, update("active", "Google", 1)), state);
  const revealed = queryReducer(state, { type: "reveal", identifier: "Google" });
  assert.equal(revealed.byIdentifier.Google.manualShow, true);
});
