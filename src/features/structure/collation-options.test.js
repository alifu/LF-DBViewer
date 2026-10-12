import assert from "node:assert/strict";
import { test } from "node:test";

import { charsetOfCollation, charsetsOf, collationsFor, defaultCollationFor } from "./collation-options.js";

const collations = [
  { name: "latin1_swedish_ci", charset: "latin1", isDefault: true },
  { name: "utf8mb4_bin", charset: "utf8mb4", isDefault: false },
  { name: "utf8mb4_0900_ai_ci", charset: "utf8mb4", isDefault: true },
  { name: "latin1_bin", charset: "latin1", isDefault: false },
];

test("charsets are unique and sorted", () => {
  assert.deepEqual(charsetsOf(collations), ["latin1", "utf8mb4"]);
});

test("collations are filtered by charset", () => {
  assert.deepEqual(collationsFor(collations, "latin1").map((collation) => collation.name), ["latin1_swedish_ci", "latin1_bin"]);
});

test("the default collation is the flagged one, else the first", () => {
  assert.equal(defaultCollationFor(collations, "utf8mb4"), "utf8mb4_0900_ai_ci");
  assert.equal(defaultCollationFor([{ name: "utf8mb4_bin", charset: "utf8mb4", isDefault: false }], "utf8mb4"), "utf8mb4_bin");
  assert.equal(defaultCollationFor(collations, "nope"), null);
});

test("the charset is the part of the name before the first underscore", () => {
  assert.equal(charsetOfCollation("utf8mb4_0900_ai_ci"), "utf8mb4");
  assert.equal(charsetOfCollation("binary"), "binary");
});
