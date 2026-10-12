import assert from "node:assert/strict";
import { test } from "node:test";

import {
  addColumn, addForeignKey, addIndex, emptyTable, formatIndexColumns, hasChanges, isTextType, moveColumn, parseIndexColumns,
  parseNameList, primaryKeyColumns, removeColumn, renameColumn, setDefault, setPrimaryKey, uniqueName, updateColumn,
} from "./designer-state.js";

const withIndexAndKey = () => {
  let table = addColumn(emptyTable("orders")); // id, column
  table = renameColumn(table, 1, "customer_id");
  table = addIndex(table);
  table = { ...table, indexes: table.indexes.map((index) => (index.kind === "index" ? { ...index, columns: [{ name: "customer_id", length: null, descending: false }] } : index)) };
  return { ...addForeignKey(table), foreignKeys: [{ ...addForeignKey(table).foreignKeys[0], columns: ["customer_id"], refTable: "customers", refColumns: ["id"] }] };
};

test("a new table has an auto-increment primary key column", () => {
  const table = emptyTable("t");

  assert.equal(table.columns[0].autoIncrement, true);
  assert.deepEqual(primaryKeyColumns(table), ["id"]);
  assert.equal(table.options.engine, "InnoDB");
});

test("new names avoid existing ones, ignoring case", () => {
  assert.equal(uniqueName(["column", "Column_2"], "column"), "column_3");
  assert.equal(uniqueName([], "x"), "x");
});

test("added columns are nullable text and get distinct names", () => {
  const table = addColumn(addColumn(emptyTable("t")));

  assert.deepEqual(table.columns.map((column) => column.name), ["id", "column", "column_2"]);
  assert.equal(table.columns[1].nullable, true);
});

test("renaming a column updates its indexes and foreign keys", () => {
  const table = renameColumn(withIndexAndKey(), 1, "client_id");

  assert.equal(table.indexes.find((index) => index.kind === "index").columns[0].name, "client_id");
  assert.deepEqual(table.foreignKeys[0].columns, ["client_id"]);
  assert.deepEqual(table.foreignKeys[0].refColumns, ["id"]);
});

test("a self-referencing key follows a renamed referenced column", () => {
  let table = emptyTable("tree");
  table = { ...addColumn(table), foreignKeys: [{ name: "fk", columns: ["column"], refDatabase: null, refTable: "tree", refColumns: ["id"], onUpdate: "restrict", onDelete: "restrict" }] };

  table = renameColumn(table, 0, "node_id");

  assert.deepEqual(table.foreignKeys[0].refColumns, ["node_id"]);
});

test("removing a column removes it from indexes, drops emptied indexes and keys that used it", () => {
  const table = removeColumn(withIndexAndKey(), 1);

  assert.deepEqual(table.columns.map((column) => column.name), ["id"]);
  assert.deepEqual(table.indexes.map((index) => index.kind), ["primary"]);
  assert.equal(table.foreignKeys.length, 0);
});

test("columns move up and down but not past the ends", () => {
  const table = addColumn(emptyTable("t"));

  assert.deepEqual(moveColumn(table, 1, -1).columns.map((column) => column.name), ["column", "id"]);
  assert.equal(moveColumn(table, 0, -1), table);
  assert.equal(moveColumn(table, 1, 1), table);
});

test("making a column part of the primary key makes it NOT NULL and clears a NULL default", () => {
  const table = setPrimaryKey(addColumn(emptyTable("t")), "column", true);

  assert.deepEqual(primaryKeyColumns(table), ["id", "column"]);
  const column = table.columns[1];
  assert.equal(column.nullable, false);
  assert.deepEqual(column.default, { kind: "none" });
});

test("removing the last primary key column removes the primary key", () => {
  const table = setPrimaryKey(emptyTable("t"), "id", false);

  assert.deepEqual(primaryKeyColumns(table), []);
  assert.equal(table.indexes.length, 0);
});

test("the primary key keeps the database name it was loaded with so the diff can replace it", () => {
  const loaded = { ...emptyTable("t"), indexes: [{ originalName: "PRIMARY", name: "PRIMARY", kind: "primary", columns: [{ name: "id", length: null, descending: false }] }] };

  assert.equal(setPrimaryKey(addColumn(loaded), "column", true).indexes[0].originalName, "PRIMARY");
});

test("defaults carry a value only for value and expression kinds", () => {
  const column = emptyTable("t").columns[0];

  assert.deepEqual(setDefault(column, "value", "5").default, { kind: "value", value: "5" });
  assert.deepEqual(setDefault(column, "expression", "now()").default, { kind: "expression", value: "now()" });
  assert.deepEqual(setDefault(column, "null").default, { kind: "null" });
});

test("only text types get a collation", () => {
  assert.equal(isTextType("varchar(20)"), true);
  assert.equal(isTextType("ENUM('a')"), true);
  assert.equal(isTextType("int"), false);
  assert.equal(isTextType("blob"), false);
});

test("name lists and index column lists parse and format both ways", () => {
  assert.deepEqual(parseNameList(" a, `b` ,, c "), ["a", "b", "c"]);
  assert.deepEqual(parseIndexColumns("name(20), created DESC, id"), [
    { name: "name", length: 20, descending: false },
    { name: "created", length: null, descending: true },
    { name: "id", length: null, descending: false },
  ]);
  assert.equal(formatIndexColumns(parseIndexColumns("name(20), created DESC")), "name(20), created DESC");
});

test("edits are detected by comparing with the original", () => {
  const original = emptyTable("t");

  assert.equal(hasChanges(original, original), false);
  assert.equal(hasChanges(original, updateColumn(original, 0, { comment: "x" })), true);
});

test("editing never changes the table it started from", () => {
  const original = emptyTable("t");

  addColumn(original);
  renameColumn(original, 0, "other");

  assert.equal(original.columns.length, 1);
  assert.equal(original.columns[0].name, "id");
});
