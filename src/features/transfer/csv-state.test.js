import assert from "node:assert/strict";
import { test } from "node:test";

import { IDLE_CSV, applyCsvEvent, autoMapping, buildCsvRequest, csvProblem, duplicateTargets, failCsv, finishCsv } from "./csv-state.js";

test("headers are matched to table columns ignoring case, spaces and underscores", () => {
  const mapping = autoMapping(["ID", "First Name", "e-mail", "ignored", "Total"], ["id", "first_name", "email", "total", "note"]);

  assert.deepEqual(mapping, ["id", "first_name", "email", null, "total"]);
});

test("a table column is only used once and blank headers are skipped", () => {
  assert.deepEqual(autoMapping(["name", "Name", ""], ["name"]), ["name", null, null]);
});

test("the form needs a file and at least one mapped column", () => {
  assert.match(csvProblem({ path: " ", mapping: ["a"] }), /Choose a CSV file/);
  assert.match(csvProblem({ path: "/x.csv", mapping: [null, null] }), /at least one/);
  assert.equal(csvProblem({ path: "/x.csv", mapping: [null, "a"] }), null);
});

test("a column chosen twice is reported", () => {
  assert.deepEqual(duplicateTargets(["a", "b", "a", null, null, "b"]), ["a", "b"]);
  assert.deepEqual(duplicateTargets(["a", null]), []);
});

test("the request sends no NULL text when none was typed", () => {
  const base = { jobId: "j", path: "/x.csv", database: "d", table: "t", delimiter: ",", hasHeader: true, mapping: ["a"], emptyAsNull: false, truncateFirst: false };

  assert.equal(buildCsvRequest({ ...base, nullText: "" }).nullText, null);
  assert.equal(buildCsvRequest({ ...base, nullText: "NULL" }).nullText, "NULL");
});

test("progress follows the events and ends with the summary", () => {
  let state = applyCsvEvent(IDLE_CSV, { event: "started", totalBytes: 1000 });
  state = applyCsvEvent(state, { event: "progress", rows: 300, bytesRead: 400, totalBytes: 1000 });

  assert.deepEqual([state.phase, state.rows, state.bytesRead], ["running", 300, 400]);
  assert.equal(finishCsv(state, { rows: 800, cancelled: false }).phase, "done");
  assert.equal(finishCsv(state, { rows: 0, cancelled: true }).phase, "cancelled");
  assert.equal(failCsv(state, "Row 3 was refused").error, "Row 3 was refused");
});
