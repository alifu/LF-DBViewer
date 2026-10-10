import assert from "node:assert/strict";
import { test } from "node:test";

import { describeFilter, makeFilter, nextSort } from "./filters.js";

test("value operators need a value and null checks do not", () => {
  assert.equal(makeFilter("name", "contains", ""), null);
  assert.deepEqual(makeFilter("name", "contains", "ali"), { column: "name", operator: "contains", value: "ali" });
  assert.deepEqual(makeFilter("name", "isNull", "ignored"), { column: "name", operator: "isNull", value: null });
});

test("unknown operators and missing columns give no filter", () => {
  assert.equal(makeFilter("name", "regex", "x"), null);
  assert.equal(makeFilter("", "equals", "x"), null);
});

test("filters read like a condition", () => {
  assert.equal(describeFilter({ column: "total", operator: "greaterOrEqual", value: "5" }), "total ≥ 5");
  assert.equal(describeFilter({ column: "note", operator: "isNull", value: null }), "note is NULL");
});

test("sorting cycles ascending, descending, none, and restarts on another column", () => {
  const ascending = nextSort(null, "name");
  const descending = nextSort(ascending, "name");

  assert.deepEqual(ascending, { column: "name", descending: false });
  assert.deepEqual(descending, { column: "name", descending: true });
  assert.equal(nextSort(descending, "name"), null);
  assert.deepEqual(nextSort(descending, "total"), { column: "total", descending: false });
});
