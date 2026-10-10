import assert from "node:assert/strict";
import { test } from "node:test";

import { formatDuration, formatTime } from "./format.js";

test("durations under a second are whole milliseconds", () => {
  assert.equal(formatDuration(12.4), "12 ms");
});

test("longer durations are seconds with two decimals", () => {
  assert.equal(formatDuration(1234), "1.23 s");
});

test("a missing duration is blank", () => {
  assert.equal(formatDuration(undefined), "");
});

test("times are zero padded", () => {
  assert.equal(formatTime(new Date(2026, 0, 1, 9, 5, 3)), "09:05:03");
});
