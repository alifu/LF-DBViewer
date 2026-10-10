import assert from "node:assert/strict";
import { test } from "node:test";

import { createStore } from "./store.js";

test("notifies subscribers with the next and previous state", () => {
  const store = createStore({ count: 0 });
  const seen = [];
  store.subscribe((next, previous) => seen.push([next.count, previous.count]));

  store.update((state) => ({ ...state, count: state.count + 1 }));

  assert.deepEqual(seen, [[1, 0]]);
  assert.equal(store.getState().count, 1);
});

test("does not notify when the reducer returns the same state", () => {
  const store = createStore({ count: 0 });
  let calls = 0;
  store.subscribe(() => (calls += 1));

  store.update((state) => state);

  assert.equal(calls, 0);
});

test("stops notifying after unsubscribe", () => {
  const store = createStore({ count: 0 });
  let calls = 0;
  const unsubscribe = store.subscribe(() => (calls += 1));

  unsubscribe();
  store.update((state) => ({ ...state, count: 1 }));

  assert.equal(calls, 0);
});
