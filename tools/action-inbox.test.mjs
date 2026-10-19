import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { runInNewContext } from "node:vm";
import ts from "typescript";

const exports = {};
runInNewContext(ts.transpileModule(
  readFileSync(new URL("../src/hooks/actionInbox.ts", import.meta.url), "utf8"),
  { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS } },
).outputText, { exports });
const { createActionInbox } = exports;
const settle = () => new Promise(setImmediate);
function deferred() {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test("subscribes before first pull, drains FIFO, and stays idle without notifications", async () => {
  const ready = deferred();
  const queue = ["first", "second"].map((action) => ({ action }));
  const seen = [], errors = [];
  let pulls = 0, subscriptions = 0, wake;
  const subscribe = createActionInbox(
    async () => { pulls++; return queue.shift() ?? null; },
    (callback) => { subscriptions++; wake = callback; return ready.promise; },
    (error) => errors.push(error),
  );
  const detach = subscribe((value) => seen.push(value.action));
  await settle();
  assert.equal(pulls, 0);
  ready.resolve();
  await settle();
  assert.deepEqual(seen, ["first", "second"]);
  assert.equal(pulls, 3);
  await settle();
  assert.equal(pulls, 3);
  queue.push({ action: "third" });
  wake();
  await settle();
  assert.deepEqual(seen, ["first", "second", "third"]);
  detach();
  assert.equal(subscriptions, 1);
  assert.deepEqual(errors, []);
});

test("StrictMode cleanup holds an in-flight payload for the next ready consumer", async () => {
  const reply = deferred();
  let pulls = 0;
  const seen = [], errors = [];
  const subscribe = createActionInbox(
    () => ++pulls === 1 ? reply.promise : Promise.resolve(null),
    async () => {},
    (error) => errors.push(error),
  );
  const old = subscribe(() => assert.fail("disposed handler called"));
  await settle();
  old();
  reply.resolve({ action: "first-use" });
  await settle();
  assert.equal(pulls, 1, "no unbounded frontend prefetch while no consumer is ready");
  const current = subscribe((value) => seen.push(value.action));
  old(); // A delayed old cleanup must not remove the replacement.
  await settle();
  assert.deepEqual(seen, ["first-use"]);
  assert.equal(pulls, 2);
  current();
  assert.deepEqual(errors, []);
});

test("notification during an empty in-flight pull cannot lose the next action", async () => {
  const empty = deferred();
  let pulls = 0, wake;
  const seen = [];
  const subscribe = createActionInbox(
    () => ++pulls === 1 ? empty.promise : Promise.resolve(pulls === 2 ? { action: "raced" } : null),
    async (callback) => { wake = callback; },
    assert.fail,
  );
  subscribe((value) => seen.push(value.action));
  await settle();
  wake();
  empty.resolve(null);
  await settle();
  assert.deepEqual(seen, ["raced"]);
  assert.equal(pulls, 3);
});

test("failed subscription or pull reports an error and supports an explicit retry", async () => {
  const errors = [], seen = [];
  let listens = 0, pulls = 0, wake;
  const subscribe = createActionInbox(
    async () => {
      if (++pulls === 1) throw new Error("read failed");
      return pulls === 2 ? { action: "retained" } : null;
    },
    async (callback) => {
      if (++listens === 1) throw new Error("listen failed");
      wake = callback;
    },
    (error) => errors.push(error.message),
  );
  let detach = subscribe((value) => seen.push(value.action));
  await settle();
  assert.equal(pulls, 0);
  detach();
  detach = subscribe((value) => seen.push(value.action));
  await settle();
  assert.deepEqual(errors, ["listen failed", "read failed"]);
  wake();
  await settle();
  assert.deepEqual(seen, ["retained"]);
  detach();
});

test("unmounted before listener registration drains nothing, then reuses the listener", async () => {
  const ready = deferred();
  let pulls = 0, listens = 0;
  const subscribe = createActionInbox(
    async () => { pulls++; return null; },
    () => { listens++; return ready.promise; },
    assert.fail,
  );
  subscribe(assert.fail)();
  ready.resolve();
  await settle();
  assert.equal(pulls, 0);
  subscribe(assert.fail);
  await settle();
  assert.equal(pulls, 1);
  assert.equal(listens, 1);
});
