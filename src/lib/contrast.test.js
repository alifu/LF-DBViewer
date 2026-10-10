import assert from "node:assert/strict";
import { test } from "node:test";

import { contrastRatio } from "./contrast.js";

test("black on white is the maximum ratio", () => {
  assert.equal(contrastRatio("#000000", "#ffffff").toFixed(1), "21.0");
});

test("the order of the colours does not matter", () => {
  assert.equal(contrastRatio("#1f845a", "#ffffff"), contrastRatio("#ffffff", "#1f845a"));
});

test("identical colours have a ratio of 1", () => {
  assert.equal(contrastRatio("#777777", "#777777"), 1);
});
