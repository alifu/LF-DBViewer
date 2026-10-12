import assert from "node:assert/strict";
import { test } from "node:test";

import { OBJECT_KINDS, newObjectName, templateFor } from "./object-templates.js";

test("every template is a CREATE statement for its own kind", () => {
  for (const kind of OBJECT_KINDS) {
    const text = templateFor(kind);
    assert.match(text, new RegExp(`^CREATE ${kind.toUpperCase()}\\b`), kind);
  }
});

test("an unknown kind has no template", () => {
  assert.throws(() => templateFor("table"), /No template/);
});

test("new object tab names count up", () => {
  assert.equal(newObjectName("procedure", []), "New procedure");
  assert.equal(newObjectName("procedure", ["New procedure", "New procedure 2"]), "New procedure 3");
});
