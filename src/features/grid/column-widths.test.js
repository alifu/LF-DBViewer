import assert from "node:assert/strict";
import { test } from "node:test";

import { DEFAULT_COLUMN_WIDTH, MAX_COLUMN_WIDTH, MIN_COLUMN_WIDTH, clampWidth, gridTemplate, totalWidth, widthOf, withWidth } from "./column-widths.js";

test("widths stay within limits and are whole pixels", () => {
  assert.equal(clampWidth(10), MIN_COLUMN_WIDTH);
  assert.equal(clampWidth(99999), MAX_COLUMN_WIDTH);
  assert.equal(clampWidth(200.6), 201);
});

test("a column nobody resized has the default width", () => {
  assert.equal(widthOf({ id: 90 }, "id"), 90);
  assert.equal(widthOf({ id: 90 }, "name"), DEFAULT_COLUMN_WIDTH);
});

test("the template and total follow each column's width", () => {
  const widths = { id: 80, notes: 400 };

  assert.equal(gridTemplate(60, ["id", "name", "notes"], widths), "60px 80px 180px 400px");
  assert.equal(totalWidth(60, ["id", "name", "notes"], widths), 720);
});

test("resizing returns a new object and clamps", () => {
  const before = { id: 80 };

  const after = withWidth(before, "name", 5);

  assert.deepEqual(after, { id: 80, name: MIN_COLUMN_WIDTH });
  assert.deepEqual(before, { id: 80 });
});
