import assert from "node:assert/strict";
import { test } from "node:test";

import {
  DEFAULT_EXPORT_OPTIONS, IDLE_EXPORT, IDLE_IMPORT, applyExportEvent, applyImportEvent, buildExportRequest, buildImportRequest, defaultFileName, withGzip,
  exportProblem, failExport, failImport, finishExport, finishImport, formatBytes, formatDuration, importProblem, percent,
} from "./transfer-state.js";

test("sizes read in the largest sensible unit", () => {
  assert.equal(formatBytes(0), "0 B");
  assert.equal(formatBytes(1023), "1023 B");
  assert.equal(formatBytes(1536), "1.5 KB");
  assert.equal(formatBytes(5 * 1024 * 1024), "5.0 MB");
  assert.equal(formatBytes(150 * 1024 ** 3), "150 GB");
});

test("durations read as milliseconds, seconds or minutes", () => {
  assert.equal(formatDuration(850), "850 ms");
  assert.equal(formatDuration(12_340), "12.3 s");
  assert.equal(formatDuration(125_000), "2 min 05 s");
});

test("percent is whole, clamped, and zero for an unknown total", () => {
  assert.equal(percent(50, 200), 25);
  assert.equal(percent(5, 0), 0);
  assert.equal(percent(300, 200), 100);
  assert.equal(percent(-1, 10), 0);
});

test("the default file name has the database and the local date and time", () => {
  assert.equal(defaultFileName("shop", new Date(2026, 9, 12, 9, 5)), "shop_20261012_0905.sql");
});

test("the export form must name a file, an output and something to export", () => {
  const form = { path: "/tmp/x.sql", tables: ["t"], views: [], options: DEFAULT_EXPORT_OPTIONS };

  assert.equal(exportProblem(form), null);
  assert.match(exportProblem({ ...form, path: " " }), /where to save/);
  assert.match(exportProblem({ ...form, options: { ...DEFAULT_EXPORT_OPTIONS, structure: false, data: false } }), /structure, the data/);
  assert.match(exportProblem({ ...form, tables: [], options: { ...DEFAULT_EXPORT_OPTIONS, routines: false, events: false } }), /at least one/);
  assert.equal(exportProblem({ ...form, tables: [], options: DEFAULT_EXPORT_OPTIONS }), null);
});

test("the export request flattens the options next to the objects", () => {
  const request = buildExportRequest({ jobId: "j", database: "shop", path: "/x.sql", tables: ["a"], views: ["v"], options: DEFAULT_EXPORT_OPTIONS });

  assert.equal(request.database, "shop");
  assert.equal(request.consistentSnapshot, true);
  assert.deepEqual(request.tables, ["a"]);
});

test("an import with no target database leaves it to the file", () => {
  assert.equal(importProblem({ path: "" }), "Choose a SQL file to import.");
  assert.equal(buildImportRequest({ jobId: "j", path: "/x.sql", database: "", createDatabase: true, stopOnError: true, disableForeignKeyChecks: true }).database, null);
  assert.equal(buildImportRequest({ jobId: "j", path: "/x.sql", database: "copy", createDatabase: true, stopOnError: true, disableForeignKeyChecks: true }).database, "copy");
});

test("export progress follows table by table and ends with the summary", () => {
  let state = IDLE_EXPORT;
  for (const event of [
    { event: "started", tables: 2 },
    { event: "tableStarted", name: "a", index: 0, estimatedRows: 100 },
    { event: "progress", table: "a", rows: 60, bytes: 4000 },
    { event: "tableFinished", name: "a", rows: 100 },
    { event: "tableStarted", name: "b", index: 1, estimatedRows: null },
    { event: "note", message: "Writing routines" },
  ]) {
    state = applyExportEvent(state, event);
  }

  assert.equal(state.phase, "running");
  assert.deepEqual(state.tables.map((table) => [table.name, table.rows, table.done]), [["a", 100, true], ["b", 0, false]]);
  assert.equal(state.current, "b");
  assert.equal(state.bytes, 4000);
  assert.deepEqual(state.notes, ["Writing routines"]);
  assert.equal(finishExport(state, { cancelled: false, bytes: 9000, rows: 100 }).phase, "done");
  assert.equal(finishExport(state, { cancelled: true, bytes: 10 }).phase, "cancelled");
  assert.equal(failExport(state, "disk full").error, "disk full");
});

test("events never change the state they were applied to", () => {
  const before = applyExportEvent(IDLE_EXPORT, { event: "started" });

  applyExportEvent(before, { event: "tableStarted", name: "a", index: 0, estimatedRows: 1 });

  assert.equal(before.tables.length, 0);
  assert.equal(IDLE_EXPORT.phase, "idle");
});

test("import progress counts statements and collects failures", () => {
  let state = applyImportEvent(IDLE_IMPORT, { event: "started", totalBytes: 1000 });
  state = applyImportEvent(state, { event: "progress", bytesRead: 400, totalBytes: 1000, statements: 12, line: 30 });
  state = applyImportEvent(state, { event: "statementFailed", error: { statement: 13, line: 31, message: "boom", sql: "INSERT" } });

  assert.deepEqual([state.bytesRead, state.statements, state.errors.length], [400, 12, 1]);
  assert.equal(finishImport(state, { cancelled: false, failed: true, errors: state.errors, statements: 13, bytes: 1000 }).phase, "failed");
  assert.equal(finishImport(state, { cancelled: false, failed: false, errors: [], statements: 13, bytes: 1000 }).phase, "done");
  assert.equal(finishImport(state, { cancelled: true, failed: false, errors: [], statements: 5, bytes: 100 }).phase, "cancelled");
  assert.equal(failImport(state, "no such file").phase, "failed");
});

test("a compressed dump has .gz after .sql, and the checkbox keeps the path in step", () => {
  assert.equal(defaultFileName("shop", new Date(2026, 9, 12, 9, 5), true), "shop_20261012_0905.sql.gz");
  assert.equal(withGzip("/tmp/a.sql", true), "/tmp/a.sql.gz");
  assert.equal(withGzip("/tmp/a.sql.gz", true), "/tmp/a.sql.gz");
  assert.equal(withGzip("/tmp/a.sql.GZ", false), "/tmp/a.sql");
  assert.equal(withGzip("", true), "");
});
