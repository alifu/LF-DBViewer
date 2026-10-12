import assert from "node:assert/strict";
import { test } from "node:test";

import { defaultConstraintName, withForeignKey, withoutForeignKey } from "./fk-edit.js";

const definition = { name: "orders", foreignKeys: [{ originalName: "fk_a", name: "fk_a", columns: ["a"], refTable: "t", refColumns: ["id"] }], columns: [] };

test("the default name says what the key is and avoids names in use", () => {
  assert.equal(defaultConstraintName("orders", ["customer_id"], []), "fk_orders_customer_id");
  assert.equal(defaultConstraintName("orders", ["customer_id"], ["fk_orders_customer_id"]), "fk_orders_customer_id_2");
  assert.ok(defaultConstraintName("t".repeat(70), ["c"], []).length <= 64);
});

test("adding a key keeps the others and marks the new one as new", () => {
  const added = withForeignKey(definition, { name: "fk_b", columns: ["b"], refDatabase: undefined, refTable: "customers", refColumns: ["id"], onUpdate: "cascade", onDelete: "restrict" });

  assert.deepEqual(added.foreignKeys.map((key) => key.name), ["fk_a", "fk_b"]);
  assert.equal(added.foreignKeys[1].originalName, null);
  assert.equal(added.foreignKeys[1].refDatabase, null);
  assert.equal(definition.foreignKeys.length, 1, "the original is not changed");
});

test("removing a key leaves the rest alone", () => {
  assert.deepEqual(withoutForeignKey(definition, "fk_a").foreignKeys, []);
  assert.equal(withoutForeignKey(definition, "nope").foreignKeys.length, 1);
});
