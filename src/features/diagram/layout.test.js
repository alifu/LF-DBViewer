import assert from "node:assert/strict";
import { test } from "node:test";

import { autoLayout, mergeLayout, overlaps } from "./layout.js";

const box = (id, width = 180, height = 120) => ({ id, width, height });
const model = (ids, links, sizes = {}) => {
  const boxes = ids.map((id) => ({ ...box(id), ...(sizes[id] ?? {}) }));
  return { boxes, byId: new Map(boxes.map((entry) => [entry.id, entry])), edges: links.map(([from, to]) => ({ id: `${from}-${to}`, from, to })) };
};
const rectangles = (m, positions) => m.boxes.map((entry) => ({ id: entry.id, ...positions[entry.id], width: entry.width, height: entry.height }));

function assertNoOverlap(m, positions) {
  const all = rectangles(m, positions);
  for (let a = 0; a < all.length; a += 1) {
    for (let b = a + 1; b < all.length; b += 1) assert.ok(!overlaps(all[a], all[b]), `${all[a].id} overlaps ${all[b].id}`);
  }
}

test("a chain runs left to right, one column per step", () => {
  const m = model(["a", "b", "c"], [["a", "b"], ["b", "c"]]);

  const positions = autoLayout(m);

  assert.ok(Math.abs(positions.a.x - positions.b.x) > 0 && Math.abs(positions.b.x - positions.c.x) > 0);
  assert.equal(new Set([positions.a.x, positions.b.x, positions.c.x]).size, 3);
  assertNoOverlap(m, positions);
});

test("the best-connected table is the hub and its neighbours share a column", () => {
  const m = model(["hub", "x", "y", "z"], [["x", "hub"], ["y", "hub"], ["z", "hub"]]);

  const positions = autoLayout(m);

  assert.equal(positions.x.x, positions.y.x);
  assert.equal(positions.y.x, positions.z.x);
  assert.notEqual(positions.hub.x, positions.x.x);
  assertNoOverlap(m, positions);
});

test("tables with no relations go in a grid below the connected ones", () => {
  const m = model(["a", "b", "l1", "l2", "l3"], [["a", "b"]]);

  const positions = autoLayout(m);
  const connectedBottom = Math.max(positions.a.y + 120, positions.b.y + 120);

  for (const id of ["l1", "l2", "l3"]) assert.ok(positions[id].y >= connectedBottom, id);
  assertNoOverlap(m, positions);
});

test("a table that refers to itself does not break the layout", () => {
  const m = model(["tree", "leaf"], [["tree", "tree"], ["leaf", "tree"]]);

  assertNoOverlap(m, autoLayout(m));
});

test("an empty model has an empty layout", () => {
  assert.deepEqual(autoLayout(model([], [])), {});
});

test("the same model always gives the same layout", () => {
  const m = model(["a", "b", "c", "d", "e"], [["a", "b"], ["c", "b"], ["d", "e"]]);

  assert.deepEqual(autoLayout(m), autoLayout(m));
});

test("random schemas never overlap, however they are wired or sized", () => {
  let seed = 12345;
  const random = () => ((seed = (seed * 1103515245 + 12345) % 2147483648) / 2147483648);
  for (let round = 0; round < 40; round += 1) {
    const count = 1 + Math.floor(random() * 60);
    const ids = Array.from({ length: count }, (_, index) => `t${index}`);
    const links = Array.from({ length: Math.floor(random() * count * 1.2) }, () => [ids[Math.floor(random() * count)], ids[Math.floor(random() * count)]]);
    const sizes = Object.fromEntries(ids.map((id) => [id, { width: 170 + Math.floor(random() * 230), height: 60 + Math.floor(random() * 400) }]));
    const m = model(ids, links, sizes);

    const positions = autoLayout(m);

    assert.equal(Object.keys(positions).length, count, `round ${round}`);
    assertNoOverlap(m, positions);
  }
});

test("saved positions win and new tables go under everything that is placed", () => {
  const m = model(["a", "b", "c"], [["a", "b"]]);
  const automatic = autoLayout(m);

  const merged = mergeLayout(automatic, { a: { x: 500, y: 40 }, b: { x: 900, y: 40 }, gone: { x: 1, y: 1 } }, m);

  assert.deepEqual(merged.a, { x: 500, y: 40 });
  assert.deepEqual(merged.b, { x: 900, y: 40 });
  assert.ok(merged.c.y >= 40 + 120, "the new table is below");
  assert.equal(merged.gone, undefined);
  assertNoOverlap(m, merged);
});

test("with nothing saved the automatic layout is used, and bad saved values are ignored", () => {
  const m = model(["a", "b"], [["a", "b"]]);
  const automatic = autoLayout(m);

  assert.deepEqual(mergeLayout(automatic, {}, m), automatic);
  assert.deepEqual(mergeLayout(automatic, { a: { x: "left", y: null }, b: undefined }, m), automatic);
});
