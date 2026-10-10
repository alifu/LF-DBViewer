import assert from "node:assert/strict";
import { test } from "node:test";

import { visibleGroups } from "./tree.js";

const objects = {
  tables: [{ name: "orders" }, { name: "order_items" }, { name: "users" }],
  views: ["recent_orders"],
  procedures: [],
  functions: ["tax"],
  triggers: [],
  events: [],
};

test("empty groups are hidden", () => {
  assert.deepEqual(visibleGroups(objects, "").map((group) => group.key), ["tables", "views", "functions"]);
});

test("search matches any part of a name, ignoring case", () => {
  const groups = visibleGroups(objects, " ORDER ");

  assert.deepEqual(groups.map((group) => [group.key, group.items]), [
    ["tables", ["orders", "order_items"]],
    ["views", ["recent_orders"]],
  ]);
});

test("each group knows what kind of object it opens", () => {
  assert.equal(visibleGroups(objects, "tax")[0].kind, "function");
});
