import assert from "node:assert/strict";
import { test } from "node:test";

import { placeMenu } from "./menu.js";

const window = { viewportWidth: 800, viewportHeight: 600, width: 200, height: 150 };

test("a menu that fits opens at the pointer", () => {
  assert.deepEqual(placeMenu({ ...window, x: 100, y: 120 }), { left: 100, top: 120 });
});

test("near the right and bottom edges it opens inwards", () => {
  assert.deepEqual(placeMenu({ ...window, x: 790, y: 590 }), { left: 594, top: 444 });
});

test("it never leaves the top left corner", () => {
  assert.deepEqual(placeMenu({ ...window, x: -50, y: -50 }), { left: 6, top: 6 });
});
