import assert from "node:assert/strict";
import { test } from "node:test";

import { describeSpecial, displayCell, isSpecial } from "./cell-format.js";

test("NULL is shown dimmed and the empty string is shown as empty", () => {
  assert.deepEqual(displayCell(null), { text: "NULL", className: "muted" });
  assert.deepEqual(displayCell(""), { text: "", className: "" });
});

test("binary previews are hex with the size", () => {
  assert.equal(describeSpecial({ kind: "binary", preview: "cafe", size: 2 }), "0xcafe… (2 bytes)");
});

test("truncated text shows its beginning and the full size", () => {
  assert.equal(describeSpecial({ kind: "truncated", preview: "abc", size: 12345 }), "abc… (12,345 bytes)");
});

test("only objects are special", () => {
  assert.equal(isSpecial({ kind: "binary" }), true);
  assert.equal(isSpecial(null), false);
  assert.equal(isSpecial("text"), false);
});
