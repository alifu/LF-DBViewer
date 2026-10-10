import assert from "node:assert/strict";
import { test } from "node:test";

import { NO_CHANGES, addInsert, changeCount, editCell, editInsert, removeInsert, toChanges, toggleDeleted } from "./pending.js";

const columns = [{ name: "id" }, { name: "name" }, { name: "blob" }];
const binary = { kind: "binary", preview: "ff", size: 1 };
const rows = [
  ["1", "one", null],
  ["2", "two", binary],
];
const keyed = { rows, columns, primaryKey: ["id"] };

test("editing records the value and restoring the original removes it", () => {
  const edited = editCell(NO_CHANGES, 0, "name", "uno", "one");
  const restored = editCell(edited, 0, "name", "one", "one");

  assert.deepEqual(edited.edits, { 0: { name: "uno" } });
  assert.deepEqual(restored, NO_CHANGES);
  assert.deepEqual(NO_CHANGES.edits, {});
});

test("null is a value distinct from the empty string", () => {
  const edited = editCell(NO_CHANGES, 0, "name", null, "one");

  assert.deepEqual(toChanges(edited, keyed)[0].set, [{ column: "name", value: null }]);
});

test("counts each changed row once and ignores edits on deleted rows", () => {
  let pending = editCell(editCell(NO_CHANGES, 0, "name", "a", "one"), 0, "id", "9", "1");
  pending = toggleDeleted(editCell(pending, 1, "name", "b", "two"), 1);
  pending = addInsert(pending);

  assert.equal(changeCount(pending), 3);
  assert.deepEqual(toChanges(pending, keyed).map((change) => change.kind), ["delete", "update", "insert"]);
});

test("keyed rows are identified by their original primary key", () => {
  const pending = editCell(NO_CHANGES, 1, "id", "20", "2");

  assert.deepEqual(toChanges(pending, keyed), [
    { kind: "update", key: [{ column: "id", value: "2" }], set: [{ column: "id", value: "20" }] },
  ]);
});

test("keyless rows are identified by every plain column, skipping binary cells", () => {
  const pending = toggleDeleted(NO_CHANGES, 1);

  const [change] = toChanges(pending, { rows, columns, primaryKey: [] });

  assert.deepEqual(change.key, [
    { column: "id", value: "2" },
    { column: "name", value: "two" },
  ]);
});

test("a binary primary key cannot be edited", () => {
  const pending = toggleDeleted(NO_CHANGES, 1);

  assert.throws(() => toChanges(pending, { rows, columns, primaryKey: ["blob"] }), /binary/);
});

test("inserts carry only the columns that were filled in", () => {
  let pending = editInsert(addInsert(addInsert(NO_CHANGES)), 1, "name", "new");
  pending = removeInsert(pending, 0);

  assert.deepEqual(toChanges(pending, keyed), [{ kind: "insert", values: [{ column: "name", value: "new" }] }]);
});

test("toggling delete twice restores the row", () => {
  assert.deepEqual(toggleDeleted(toggleDeleted(NO_CHANGES, 0), 0), NO_CHANGES);
});
