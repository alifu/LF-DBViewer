import assert from "node:assert/strict";
import { test } from "node:test";

import {
  HEADER_HEIGHT, MAX_BOX_WIDTH, MAX_SCALE, MIN_BOX_WIDTH, MIN_SCALE, ROW_HEIGHT, anchor, boundsOf, boxHeight, boxWidth, chooseSides, edgePath, ellipsize, fitView, panBy, rowCenter, toDiagram, zoomAt,
} from "./geometry.js";

test("long names are cut with an ellipsis", () => {
  assert.equal(ellipsize("short", 10), "short");
  assert.equal(ellipsize("abcdefghij", 5), "abcd…");
});

test("a box is as wide as its longest row but within limits", () => {
  const narrow = boxWidth("t", [{ name: "id", type: "int" }]);
  const wide = boxWidth("orders", [{ name: "a_rather_long_column_name_for_testing", type: "varchar(255)" }]);
  const huge = boxWidth("x".repeat(200), [{ name: "y".repeat(200), type: "z".repeat(200) }]);

  assert.equal(narrow, MIN_BOX_WIDTH);
  assert.ok(wide > narrow && wide <= MAX_BOX_WIDTH);
  assert.equal(huge, MAX_BOX_WIDTH);
});

test("height grows with the rows and rows are centred below the header", () => {
  assert.equal(boxHeight(3) - boxHeight(2), ROW_HEIGHT);
  assert.equal(rowCenter(0), HEADER_HEIGHT + ROW_HEIGHT / 2);
  assert.equal(rowCenter(2) - rowCenter(1), ROW_HEIGHT);
});

test("anchors sit on the chosen side at the row's height", () => {
  const box = { width: 200 };

  assert.deepEqual(anchor(box, { x: 100, y: 50 }, 1, "right"), { x: 300, y: 50 + rowCenter(1), side: "right" });
  assert.deepEqual(anchor(box, { x: 100, y: 50 }, 0, "left"), { x: 100, y: 50 + rowCenter(0), side: "left" });
  assert.equal(anchor(box, { x: 0, y: 0 }, null, "left").y, HEADER_HEIGHT / 2);
});

test("lines leave the side facing the other box, and loop out on the right otherwise", () => {
  const box = { width: 100 };

  assert.deepEqual(chooseSides(box, { x: 0, y: 0 }, { width: 100 }, { x: 300, y: 0 }), ["right", "left"]);
  assert.deepEqual(chooseSides({ width: 100 }, { x: 300, y: 0 }, box, { x: 0, y: 0 }), ["left", "right"]);
  assert.deepEqual(chooseSides(box, { x: 0, y: 0 }, { width: 100 }, { x: 50, y: 200 }), ["right", "right"]);
  assert.deepEqual(chooseSides(box, { x: 0, y: 0 }, box, { x: 0, y: 0 }), ["right", "right"]);
});

test("a curve starts and ends at its anchors with horizontal tangents", () => {
  const path = edgePath({ x: 0, y: 10, side: "right" }, { x: 200, y: 90, side: "left" });

  assert.equal(path, "M 0 10 C 100 10, 100 90, 200 90");
  assert.match(edgePath({ x: 0, y: 0, side: "right" }, { x: 0, y: 40, side: "right" }), /^M 0 0 C 46 0, 46 40, 0 40$/);
});

test("screen and diagram points convert both ways around the view", () => {
  const view = { x: 100, y: 50, scale: 2 };

  assert.deepEqual(toDiagram(view, { x: 300, y: 250 }), { x: 100, y: 100 });
});

test("zooming keeps the point under the cursor still and respects the limits", () => {
  const view = { x: 40, y: 20, scale: 1 };
  const point = { x: 300, y: 200 };

  const zoomed = zoomAt(view, 2, point);

  assert.deepEqual(toDiagram(zoomed, point), toDiagram(view, point));
  assert.equal(zoomed.scale, 2);
  assert.equal(zoomAt(view, 1000, point).scale, MAX_SCALE);
  assert.equal(zoomAt(view, 0.00001, point).scale, MIN_SCALE);
});

test("panning moves the offset and keeps the scale", () => {
  assert.deepEqual(panBy({ x: 1, y: 2, scale: 3 }, 10, -5), { x: 11, y: -3, scale: 3 });
});

test("bounds cover every rectangle", () => {
  assert.deepEqual(boundsOf([{ x: 10, y: 20, width: 100, height: 50 }, { x: -30, y: 60, width: 20, height: 100 }]), { x: -30, y: 20, width: 140, height: 140 });
  assert.deepEqual(boundsOf([]), { x: 0, y: 0, width: 0, height: 0 });
});

test("fit shows everything centred and never zooms in past the limit", () => {
  const bounds = { x: 100, y: 100, width: 800, height: 400 };
  const view = fitView(bounds, { width: 1000, height: 600 }, { padding: 50 });
  const topLeft = { x: bounds.x * view.scale + view.x, y: bounds.y * view.scale + view.y };
  const bottomRight = { x: (bounds.x + bounds.width) * view.scale + view.x, y: (bounds.y + bounds.height) * view.scale + view.y };

  assert.equal(view.scale, 1);
  assert.ok(topLeft.x >= 0 && topLeft.y >= 0 && bottomRight.x <= 1000 && bottomRight.y <= 600);
  assert.equal(topLeft.x + bottomRight.x, 1000);
  assert.ok(fitView({ x: 0, y: 0, width: 8000, height: 4000 }, { width: 1000, height: 600 }).scale < 0.2);
  assert.deepEqual(fitView({ x: 0, y: 0, width: 0, height: 0 }, { width: 10, height: 10 }), { x: 0, y: 0, scale: 1 });
});
