import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";

const source = readFileSync(new URL("../src/ocr/ocrState.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
});
const { emptyOcrState, ocrReducer } = await import(`data:text/javascript;base64,${Buffer.from(outputText).toString("base64")}`);
const output = { text: "recognized", lines: [], language: "en", confidence: null };
const result = { translatedResults: ["译文"], serviceName: "Google" };
const recognized = () => ocrReducer(
  ocrReducer(emptyOcrState, { type: "capture", id: "capture" }),
  { type: "recognized", id: "capture", output },
);

test("cancelled selection keeps the previous OCR text and completed translation", () => {
  const previous = ocrReducer(
    ocrReducer(recognized(), { type: "translate", id: "translated" }),
    { type: "translated", id: "translated", result },
  );
  const selecting = ocrReducer(previous, { type: "capture", id: "cancelled" });
  const state = ocrReducer(selecting, { type: "captureEnd", id: "cancelled" });
  assert.equal(state.text, previous.text);
  assert.equal(state.translation, previous.translation);
  assert.equal(state.recognitionId, null);
});

test("cancel ignores late recognition success and error without changing existing text", () => {
  const state = ocrReducer(recognized(), { type: "cancel" });
  assert.equal(ocrReducer(state, { type: "recognized", id: "capture", output: { ...output, text: "old" } }), state);
  assert.equal(ocrReducer(state, { type: "captureError", id: "capture", error: "old error" }), state);
});

test("editing the source invalidates its translation and OCR coordinates", () => {
  const translating = ocrReducer(recognized(), { type: "translate", id: "old" });
  const state = ocrReducer(translating, { type: "edit", text: "corrected source" });
  assert.equal(state.output, null);
  assert.equal(state.recognitionId, null);
  assert.equal(state.translationId, null);
  assert.equal(state.translation, null);
  assert.equal(ocrReducer(state, { type: "translated", id: "old", result }), state);
  assert.equal(ocrReducer(state, { type: "translationError", id: "old", error: "old error" }), state);
});

test("a new capture rejects the old translation and old capture completion", () => {
  const translating = ocrReducer(recognized(), { type: "translate", id: "old-translation" });
  const state = ocrReducer(translating, { type: "capture", id: "new-capture" });
  assert.equal(ocrReducer(state, { type: "translated", id: "old-translation", result }), state);
  assert.equal(ocrReducer(state, { type: "captureEnd", id: "capture" }), state);
  assert.equal(ocrReducer(state, { type: "recognized", id: "capture", output }), state);
  assert.equal(state.recognitionId, "new-capture");
});

test("cancel stops pending translation without accepting its late result", () => {
  const translating = ocrReducer(recognized(), { type: "translate", id: "translation" });
  const state = ocrReducer(translating, { type: "cancel" });
  assert.equal(state.translationId, null);
  assert.equal(ocrReducer(state, { type: "translated", id: "translation", result }), state);
});

test("only the current translation can clear the loading indicator", () => {
  const old = ocrReducer(recognized(), { type: "translate", id: "old" });
  const state = ocrReducer(old, { type: "translate", id: "new" });
  assert.equal(ocrReducer(state, { type: "translated", id: "old", result }), state);
  const completed = ocrReducer(state, { type: "translated", id: "new", result });
  assert.equal(completed.translation, "译文");
  assert.equal(completed.translationService, "Google");
  assert.equal(completed.translationId, null);
});

test("a translation error is not displayed as a successful translated result", () => {
  const translating = ocrReducer(recognized(), { type: "translate", id: "failed" });
  const state = ocrReducer(translating, {
    type: "translated", id: "failed", result: { serviceName: "Google", error: { message: "unavailable" } },
  });
  assert.equal(state.translation, null);
  assert.equal(state.translationService, null);
  assert.equal(state.error, "unavailable");
  assert.equal(state.translationId, null);
});
