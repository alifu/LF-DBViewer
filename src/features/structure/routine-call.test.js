import assert from "node:assert/strict";
import { test } from "node:test";

import { buildCallSql, inputParameters, sqlLiteral } from "./routine-call.js";

const param = (position, mode, name, dataType = "int") => ({ position, mode, name, dataType });

test("literals are quoted, escaped, or NULL", () => {
  assert.equal(sqlLiteral("it's"), "'it''s'");
  assert.equal(sqlLiteral("a\\b"), "'a\\\\b'");
  assert.equal(sqlLiteral(null), "NULL");
  assert.equal(sqlLiteral(""), "''");
});

test("a procedure with only IN parameters is a plain CALL", () => {
  const sql = buildCallSql({ kind: "procedure", database: "shop", name: "greet", parameters: [param(1, "IN", "who")], values: { 1: "Ada" } });

  assert.equal(sql, "CALL `shop`.`greet`('Ada');");
});

test("OUT and INOUT parameters use session variables and are selected afterwards", () => {
  const parameters = [param(1, "IN", "who"), param(2, "INOUT", "counter"), param(3, "OUT", "answer")];

  const sql = buildCallSql({ kind: "procedure", database: "shop", name: "p", parameters, values: { 1: "x", 2: "5" } });

  assert.equal(
    sql,
    "SET @lf_counter = '5';\nCALL `shop`.`p`('x', @lf_counter, @lf_answer);\nSELECT @lf_counter AS `counter`, @lf_answer AS `answer`;",
  );
});

test("a missing value is sent as NULL", () => {
  const sql = buildCallSql({ kind: "procedure", database: "shop", name: "p", parameters: [param(1, "IN", "a")], values: { 1: null } });

  assert.equal(sql, "CALL `shop`.`p`(NULL);");
});

test("a function is called in a SELECT and its return value is skipped", () => {
  const parameters = [param(0, null, null, "int"), param(1, null, "a"), param(2, null, "b")];

  const sql = buildCallSql({ kind: "function", database: "shop", name: "add", parameters, values: { 1: "1", 2: "2" } });

  assert.equal(sql, "SELECT `shop`.`add`('1', '2') AS result;");
});

test("the dialog asks only for IN and INOUT parameters", () => {
  const parameters = [param(0, null, null), param(1, "IN", "a"), param(2, "OUT", "b"), param(3, "INOUT", "c")];

  assert.deepEqual(inputParameters(parameters).map((parameter) => parameter.name), ["a", "c"]);
});

test("names with backticks cannot break out of the identifier", () => {
  const sql = buildCallSql({ kind: "procedure", database: "we`ird", name: "p`x", parameters: [], values: {} });

  assert.equal(sql, "CALL `we``ird`.`p``x`();");
});
