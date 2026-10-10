import assert from "node:assert/strict";
import { test } from "node:test";

import { chooseToRun, explainTexts, statementAt } from "./statement-at.js";

// "SELECT 1; SELECT 2;\n\nSELECT 3"
const spans = [
  { text: "SELECT 1", start: 0, end: 8 },
  { text: "SELECT 2", start: 10, end: 18 },
  { text: "SELECT 3", start: 21, end: 29 },
];

test("the statement containing the cursor is chosen", () => {
  assert.equal(statementAt(spans, 12).text, "SELECT 2");
  assert.equal(statementAt(spans, 21).text, "SELECT 3");
  assert.equal(statementAt(spans, 29).text, "SELECT 3");
});

test("a cursor right after the semicolon still belongs to that statement", () => {
  assert.equal(statementAt(spans, 9).text, "SELECT 1");
});

test("in the gap between statements the previous one is used", () => {
  assert.equal(statementAt(spans, 20).text, "SELECT 2");
});

test("before the first statement the first one is used, and an empty script has none", () => {
  assert.equal(statementAt([{ text: "SELECT 1", start: 4, end: 12 }], 0).text, "SELECT 1");
  assert.equal(statementAt([], 5), null);
});

test("current mode runs one statement and all mode runs every one", () => {
  assert.deepEqual(chooseToRun("current", spans, 12).map((span) => span.text), ["SELECT 2"]);
  assert.equal(chooseToRun("all", spans, 12).length, 3);
  assert.deepEqual(chooseToRun("current", [], 0), []);
});

test("explain prefixes only statements MySQL can explain", () => {
  const texts = explainTexts([
    { text: "select * from t" },
    { text: "WITH x AS (SELECT 1) SELECT * FROM x" },
    { text: "SHOW TABLES" },
    { text: "  update t set a = 1" },
  ]);

  assert.deepEqual(texts, ["EXPLAIN select * from t", "EXPLAIN WITH x AS (SELECT 1) SELECT * FROM x", "SHOW TABLES", "EXPLAIN   update t set a = 1"]);
});
