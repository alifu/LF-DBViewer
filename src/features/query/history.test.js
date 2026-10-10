import assert from "node:assert/strict";
import { test } from "node:test";

import { MAX_HISTORY, addToHistory, filterHistory, historyKey, loadHistory, saveHistory } from "./history.js";

test("the newest statement comes first and a rerun moves to the top", () => {
  let history = addToHistory([], ["SELECT 1", "SELECT 2"], 100);
  history = addToHistory(history, ["SELECT 1"], 200);

  assert.deepEqual(history.map((entry) => entry.sql), ["SELECT 1", "SELECT 2"]);
  assert.equal(history[0].at, 200);
});

test("statements of one run keep their order, newest last-run first", () => {
  const history = addToHistory([], ["first", "second"], 1);

  assert.deepEqual(history.map((entry) => entry.sql), ["second", "first"]);
});

test("blank statements are not recorded and the list is capped", () => {
  const many = Array.from({ length: MAX_HISTORY + 20 }, (_, index) => `SELECT ${index}`);

  assert.equal(addToHistory([], ["  ", ""], 1).length, 0);
  assert.equal(addToHistory([], many, 1).length, MAX_HISTORY);
});

test("filtering ignores case and blank searches return everything", () => {
  const history = addToHistory([], ["SELECT * FROM Orders", "SHOW TABLES"], 1);

  assert.equal(filterHistory(history, "orders").length, 1);
  assert.equal(filterHistory(history, "  ").length, 2);
});

test("history is stored per server and survives a round trip", () => {
  const values = new Map();
  const storage = { getItem: (key) => values.get(key) ?? null, setItem: (key, value) => values.set(key, value) };
  const key = historyKey({ user: "root", host: "127.0.0.1", port: 3306 });

  saveHistory(storage, key, addToHistory([], ["SELECT 1"], 5));

  assert.equal(key, "root@127.0.0.1:3306");
  assert.equal(loadHistory(storage, key)[0].sql, "SELECT 1");
  assert.deepEqual(loadHistory(storage, "someone@else:1"), []);
});

test("corrupt or blocked storage gives an empty history without throwing", () => {
  const corrupt = { getItem: () => "{not json", setItem: () => {} };
  const odd = { getItem: () => JSON.stringify([{ sql: 1 }, "x", { sql: "ok", at: 1 }]), setItem: () => {} };
  const blocked = {
    getItem: () => {
      throw new Error("blocked");
    },
    setItem: () => {
      throw new Error("blocked");
    },
  };

  assert.deepEqual(loadHistory(corrupt, "k"), []);
  assert.deepEqual(loadHistory(odd, "k"), [{ sql: "ok", at: 1 }]);
  assert.deepEqual(loadHistory(blocked, "k"), []);
  assert.doesNotThrow(() => saveHistory(blocked, "k", []));
});
