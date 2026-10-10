import assert from "node:assert/strict";
import { test } from "node:test";

import { visibleRange } from "./window.js";

const base = { viewportHeight: 240, rowHeight: 24, rowCount: 1000 };

test("draws the visible rows plus overscan", () => {
  assert.deepEqual(visibleRange({ ...base, scrollTop: 2400, overscan: 5 }), { start: 95, end: 116 });
});

test("clamps at the top and bottom", () => {
  assert.deepEqual(visibleRange({ ...base, scrollTop: 0, overscan: 5 }), { start: 0, end: 16 });
  assert.deepEqual(visibleRange({ ...base, scrollTop: 24 * 995, overscan: 5 }), { start: 990, end: 1000 });
});

test("an empty list draws nothing", () => {
  assert.deepEqual(visibleRange({ ...base, rowCount: 0, scrollTop: 100 }), { start: 0, end: 0 });
});

test("a scroll position past the end never yields a negative range", () => {
  const range = visibleRange({ ...base, rowCount: 3, scrollTop: 5000, overscan: 2 });

  assert.ok(range.start <= range.end);
  assert.equal(range.end, 3);
});
