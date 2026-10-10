import assert from "node:assert/strict";
import { test } from "node:test";

import { NO_RESULTS, applyEvent, describeStatement, failRun, finishRun, formatCount, setLabel, startRun } from "./results.js";

const feed = (state, events) => events.reduce(applyEvent, state);

test("rows are appended across batches and the set is closed with its totals", () => {
  const state = feed(startRun(["SELECT 1"]), [
    { event: "statementStarted", index: 0 },
    { event: "resultSetStarted", index: 0, set: 0, columns: [{ name: "a", dataType: "long" }] },
    { event: "rows", index: 0, set: 0, rows: [["1"], ["2"]] },
    { event: "rows", index: 0, set: 0, rows: [["3"]] },
    { event: "resultSetFinished", index: 0, set: 0, totalRows: 3, truncated: false },
  ]);

  assert.deepEqual(state.sets[0].rows, [["1"], ["2"], ["3"]]);
  assert.equal(state.sets[0].done, true);
  assert.equal(state.statements[0].status, "running");
});

test("a call that returns two result sets keeps them apart", () => {
  const state = feed(startRun(["CALL p()"]), [
    { event: "resultSetStarted", index: 0, set: 0, columns: [] },
    { event: "rows", index: 0, set: 0, rows: [["a"]] },
    { event: "resultSetStarted", index: 0, set: 1, columns: [] },
    { event: "rows", index: 0, set: 1, rows: [["b"], ["c"]] },
  ]);

  assert.deepEqual(state.sets.map((set) => set.rows.length), [1, 2]);
  assert.deepEqual(state.sets.map((set) => setLabel(state.sets, set)), ["Result 1", "Result 2"]);
});

test("events never change the state they were applied to", () => {
  const before = startRun(["SELECT 1"]);

  applyEvent(before, { event: "statementStarted", index: 0 });

  assert.equal(before.statements[0].status, "pending");
  assert.equal(NO_RESULTS.running, false);
});

test("a finished statement records its outcome and a failed one its message", () => {
  const state = feed(startRun(["UPDATE t SET a = 1", "BAD"]), [
    { event: "statementFinished", index: 0, affectedRows: 4, lastInsertId: null, warnings: 1, info: "Rows matched: 4", durationMs: 7 },
    { event: "statementFailed", index: 1, code: 1064, message: "ERROR 1064 (42000): syntax", durationMs: 1, cancelled: false },
  ]);

  assert.equal(describeStatement(state.statements[0], []), "OK, 4 rows affected, 1 warning (7 ms)");
  assert.equal(describeStatement(state.statements[1], []), "ERROR 1064 (42000): syntax (1 ms)");
  assert.equal(state.statements[1].status, "error");
});

test("a cancelled statement is not shown as an error", () => {
  const state = feed(startRun(["SELECT SLEEP(9)"]), [{ event: "statementFailed", index: 0, code: 1317, message: "Cancelled", durationMs: 300, cancelled: true }]);

  assert.equal(state.statements[0].status, "cancelled");
  assert.equal(describeStatement(state.statements[0], []), "Cancelled (300 ms)");
});

test("a select reports its row counts and says when not all rows are shown", () => {
  const statement = { status: "done", affectedRows: null, warnings: 0, durationMs: 12 };

  assert.equal(describeStatement(statement, [{ totalRows: 1, truncated: false }]), "OK, 1 row (12 ms)");
  assert.equal(describeStatement(statement, [{ totalRows: 250000, truncated: true }]), "OK, 250,000 rows (not all shown) (12 ms)");
});

test("finishing and failing a run both stop it", () => {
  const running = startRun(["SELECT 1"]);

  assert.equal(finishRun(running, { failed: false }).running, false);
  assert.equal(failRun(running, "no connection").runError, "no connection");
  assert.equal(failRun(running, "no connection").running, false);
  assert.equal(formatCount(2, "row"), "2 rows");
});

test("unknown events are ignored", () => {
  const state = startRun(["SELECT 1"]);

  assert.equal(applyEvent(state, { event: "somethingNew" }), state);
});
