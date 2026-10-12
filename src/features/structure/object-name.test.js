import assert from "node:assert/strict";
import { test } from "node:test";

import { objectNameFromSql } from "./object-name.js";

test("plain and quoted names are found", () => {
  assert.equal(objectNameFromSql("CREATE PROCEDURE greet(IN a INT) SELECT 1"), "greet");
  assert.equal(objectNameFromSql("create function `my fn`() returns int return 1"), "my fn");
  assert.equal(objectNameFromSql("CREATE EVENT IF NOT EXISTS nightly ON SCHEDULE EVERY 1 DAY DO SELECT 1"), "nightly");
});

test("a database qualifier is skipped", () => {
  assert.equal(objectNameFromSql("CREATE TRIGGER shop.audit BEFORE INSERT ON t FOR EACH ROW SET @a = 1"), "audit");
  assert.equal(objectNameFromSql("CREATE VIEW `shop`.`v` AS SELECT 1"), "v");
});

test("what SHOW CREATE prints (algorithm, definer, security) does not hide the name", () => {
  const sql = "CREATE ALGORITHM=UNDEFINED DEFINER=`root`@`localhost` SQL SECURITY DEFINER VIEW `cheap` AS select 1";

  assert.equal(objectNameFromSql(sql), "cheap");
  assert.equal(objectNameFromSql("CREATE OR REPLACE VIEW v AS SELECT 1"), "v");
  assert.equal(objectNameFromSql("CREATE DEFINER=root@localhost PROCEDURE p() SELECT 1"), "p");
});

test("doubled backticks inside a name are unescaped", () => {
  assert.equal(objectNameFromSql("CREATE VIEW `we``ird` AS SELECT 1"), "we`ird");
});

test("other statements have no name", () => {
  assert.equal(objectNameFromSql("SELECT 1"), null);
  assert.equal(objectNameFromSql("CREATE TABLE t (a int)"), null);
  assert.equal(objectNameFromSql(""), null);
});
