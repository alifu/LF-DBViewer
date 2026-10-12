import assert from "node:assert/strict";
import { test } from "node:test";

import { emptyLayout, layoutKey, loadLayout, saveLayout } from "./layout-store.js";

const memory = () => {
  const values = new Map();
  return { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
};

test("a layout survives a round trip, per server and database", () => {
  const storage = memory();
  const layout = { positions: { orders: { x: 10, y: 20 } }, view: { x: 5, y: 6, scale: 0.5 }, keyColumnsOnly: true };

  saveLayout(storage, layoutKey("root@127.0.0.1:3306", "shop"), layout);

  assert.deepEqual(loadLayout(storage, layoutKey("root@127.0.0.1:3306", "shop")), layout);
  assert.deepEqual(loadLayout(storage, layoutKey("root@127.0.0.1:3306", "crm")), emptyLayout());
});

test("anything unusable is dropped rather than trusted", () => {
  const storage = memory();
  storage.setItem("k", JSON.stringify({ positions: { ok: { x: 1, y: 2 }, bad: { x: "1", y: 2 }, worse: null }, view: { x: 1, y: 1, scale: 0 }, keyColumnsOnly: "yes" }));

  assert.deepEqual(loadLayout(storage, "k"), { positions: { ok: { x: 1, y: 2 } }, view: null, keyColumnsOnly: false });
});

test("missing, corrupt or blocked storage gives an empty layout without throwing", () => {
  const blocked = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };
  const corrupt = { getItem: () => "{nope", setItem: () => {} };

  assert.deepEqual(loadLayout(memory(), "missing"), emptyLayout());
  assert.deepEqual(loadLayout(corrupt, "k"), emptyLayout());
  assert.deepEqual(loadLayout(blocked, "k"), emptyLayout());
  assert.doesNotThrow(() => saveLayout(blocked, "k", emptyLayout()));
});
