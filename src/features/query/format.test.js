import assert from "node:assert/strict";
import { test } from "node:test";

import { formatSql } from "./format.js";

test("keywords are capitalised and clauses go on their own lines", () => {
  const result = formatSql("select a,b from t where x=1 and y>2 order by a", "mysql");

  assert.match(result, /^SELECT\n/);
  assert.ok(result.includes("\nFROM\n") || result.includes("\nFROM "));
  assert.ok(result.includes("WHERE"));
  assert.ok(result.includes("ORDER BY"));
});

test("several statements stay separate and comments and strings survive", () => {
  const result = formatSql("-- note\nselect 'a;b' as s; select 2", "mariadb");

  assert.ok(result.includes("-- note"));
  assert.ok(result.includes("'a;b'"));
  assert.equal(result.match(/;/g).length >= 2, true);
});

test("formatting twice changes nothing more", () => {
  const once = formatSql("select * from a join b on a.id=b.id", "mysql");

  assert.equal(formatSql(once, "mysql"), once);
});

test("text that is not SQL is reported rather than mangled", () => {
  assert.throws(() => formatSql("select ( from", "mysql"));
});
