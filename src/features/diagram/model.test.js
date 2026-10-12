import assert from "node:assert/strict";
import { test } from "node:test";

import { buildModel, describeEdge, edgesOf } from "./model.js";

const column = (name, columnType = "int", primaryKey = false) => ({ name, columnType, primaryKey, nullable: !primaryKey });
const diagram = {
  database: "shop",
  truncated: false,
  tables: [
    { name: "customers", estimatedRows: 10, columns: [column("id", "int", true), column("name", "varchar(40)"), column("region"), column("boss_id")] },
    { name: "orders", estimatedRows: 99, columns: [column("id", "int", true), column("customer_id"), column("total", "decimal(10,2)")] },
    { name: "lonely", estimatedRows: null, columns: [column("id", "int", true)] },
  ],
  relations: [
    { name: "fk_customer", table: "orders", columns: ["customer_id"], refDatabase: "shop", refTable: "customers", refColumns: ["id"], onUpdate: "cascade", onDelete: "restrict" },
    { name: "fk_boss", table: "customers", columns: ["boss_id"], refDatabase: "shop", refTable: "customers", refColumns: ["id"], onUpdate: "restrict", onDelete: "setNull" },
    { name: "fk_region", table: "customers", columns: ["region"], refDatabase: "geo", refTable: "regions", refColumns: ["code"], onUpdate: "restrict", onDelete: "restrict" },
  ],
};

test("every table is a box and a table in another database is a small external box", () => {
  const model = buildModel(diagram);

  assert.deepEqual(model.boxes.map((box) => [box.id, box.external]), [["customers", false], ["orders", false], ["lonely", false], ["geo.regions", true]]);
  assert.deepEqual(model.byId.get("geo.regions").rows.map((row) => row.name), ["code"]);
});

test("columns are flagged as primary, foreign and referenced", () => {
  const rows = buildModel(diagram).byId.get("orders").rows;

  assert.deepEqual(rows.map((row) => [row.name, row.primaryKey, row.foreignKey]), [["id", true, false], ["customer_id", false, true], ["total", false, false]]);
  assert.equal(buildModel(diagram).byId.get("customers").rows[0].referenced, true);
});

test("key columns only hides the others but keeps the ones a line needs", () => {
  const model = buildModel(diagram, { keyColumnsOnly: true });

  assert.deepEqual(model.byId.get("customers").rows.map((row) => row.name), ["id", "region", "boss_id"]);
  assert.equal(model.byId.get("customers").totalColumns, 4);
  assert.ok(model.byId.get("customers").height < buildModel(diagram).byId.get("customers").height);
});

test("edges point from the table that holds the key to the table it refers to, at the right rows", () => {
  const model = buildModel(diagram);
  const customer = model.edges.find((edge) => edge.name === "fk_customer");

  assert.deepEqual([customer.from, customer.to, customer.fromRow, customer.toRow], ["orders", "customers", 1, 0]);
  assert.equal(model.edges.find((edge) => edge.name === "fk_boss").to, "customers");
  assert.equal(model.edges.find((edge) => edge.name === "fk_region").to, "geo.regions");
  assert.ok(model.edges.every((edge) => edge.fromRow >= 0 && edge.toRow >= 0));
});

test("the edges of a box are those it starts or ends", () => {
  const model = buildModel(diagram);

  assert.deepEqual(edgesOf(model, "customers").map((edge) => edge.name).sort(), ["fk_boss", "fk_customer", "fk_region"]);
  assert.deepEqual(edgesOf(model, "lonely"), []);
});

test("a composite key reads as a list", () => {
  assert.equal(describeEdge({ from: "a", columns: ["x", "y"], to: "b", refColumns: ["p", "q"] }), "a.(x, y) → b.(p, q)");
  assert.equal(describeEdge({ from: "orders", columns: ["customer_id"], to: "customers", refColumns: ["id"] }), "orders.customer_id → customers.id");
});
