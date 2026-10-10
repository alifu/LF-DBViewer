import assert from "node:assert/strict";
import { test } from "node:test";

import { buildCompletionSchema } from "./completion-schema.js";

const objectsByDatabase = {
  shop: { tables: [{ name: "orders" }, { name: "items" }], views: ["cheap"] },
  crm: { tables: [{ name: "contacts" }], views: [] },
};

test("tables and views are qualified and the selected database gets its columns", () => {
  const result = buildCompletionSchema({ database: "shop", objectsByDatabase, columnsByTable: { orders: ["id", "total"], cheap: ["id"] } });

  assert.deepEqual(result.schema, {
    "shop.orders": ["id", "total"],
    "shop.items": [],
    "shop.cheap": ["id"],
    "crm.contacts": [],
  });
  assert.equal(result.defaultSchema, "shop");
});

test("columns are never taken from a different database", () => {
  const result = buildCompletionSchema({ database: "crm", objectsByDatabase, columnsByTable: { orders: ["id"] } });

  assert.deepEqual(result.schema["shop.orders"], []);
});

test("with no database selected there is no default schema", () => {
  assert.equal(buildCompletionSchema({ database: "", objectsByDatabase: {}, columnsByTable: {} }).defaultSchema, undefined);
});
