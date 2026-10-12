import assert from "node:assert/strict";
import { test } from "node:test";

import { accountBadges, computeRates, filterByName, formatSeconds, formatUptime, numbersOf, privilegesFor, pushHistory, replicationSummary, revokablePrivileges, scopeFromForm, sparklinePoints, summarizeStatus, visibleProcesses } from "./admin-state.js";

test("uptime reads in the two largest useful units", () => {
  assert.equal(formatUptime(45), "45 s");
  assert.equal(formatUptime(125), "2 min 05 s");
  assert.equal(formatUptime(3 * 3600 + 5 * 60), "3 h 05 min");
  assert.equal(formatUptime(2 * 86_400 + 4 * 3600 + 59), "2 d 4 h");
  assert.equal(formatUptime(-5), "0 s");
});

test("only numeric status values are kept", () => {
  assert.deepEqual(numbersOf([{ name: "Uptime", value: "120" }, { name: "Ssl_cipher", value: "" }, { name: "Version", value: "9.3.0" }, { name: "Rate", value: "1.5" }]), { Uptime: 120, Rate: 1.5 });
});

test("rates are the growth per second, skipping counters that restarted or are new", () => {
  const rates = computeRates({ Questions: 100, Bytes_sent: 5000, Gone: 1 }, { Questions: 160, Bytes_sent: 10, New: 5, Uptime: 9 }, 30);

  assert.deepEqual(rates, { Questions: 2 });
  assert.deepEqual(computeRates({ a: 1 }, { a: 2 }, 0), {});
});

test("the summary shows what is known and adds rates once there are two readings", () => {
  const numbers = { Uptime: 7200, Threads_connected: 12, Threads_running: 3, Questions: 1_500_000, Slow_queries: 4, Innodb_buffer_pool_read_requests: 1000, Innodb_buffer_pool_reads: 5, Open_tables: 300 };

  const first = summarizeStatus(numbers, {});
  const later = summarizeStatus(numbers, { Questions: 250.4, Bytes_received: 2048, Bytes_sent: 3 * 1024 * 1024 });
  const byLabel = (tiles, label) => tiles.find((tile) => tile.label === label);

  assert.equal(byLabel(first, "Uptime").value, "2 h 00 min");
  assert.deepEqual(byLabel(first, "Connections"), { label: "Connections", value: "12", detail: "3 running" });
  assert.deepEqual(byLabel(first, "Queries"), { label: "Queries", value: "1,500,000", detail: "total since start" });
  assert.equal(byLabel(first, "Buffer pool hits").value, "99.50%");
  assert.deepEqual(byLabel(later, "Queries"), { label: "Queries", value: "250 /s", detail: "1,500,000 total" });
  assert.equal(byLabel(later, "Traffic in").value, "2.0 KB/s");
  assert.equal(byLabel(later, "Traffic out").value, "3.0 MB/s");
});

test("tiles for missing figures are left out rather than shown empty", () => {
  assert.deepEqual(summarizeStatus({}, {}), []);
});

test("filtering ignores case and blanks", () => {
  const items = [{ name: "max_connections" }, { name: "Innodb_buffer_pool_size" }];

  assert.equal(filterByName(items, "POOL").length, 1);
  assert.equal(filterByName(items, "  ").length, 2);
  assert.equal(filterByName([{ user: "App" }], "app", (item) => item.user).length, 1);
});

test("each scope offers only the privileges it can have", () => {
  assert.ok(privilegesFor("global").includes("SUPER"));
  assert.ok(!privilegesFor("database").includes("SUPER"));
  assert.ok(privilegesFor("database").includes("EXECUTE"));
  assert.ok(!privilegesFor("table").includes("EXECUTE"));
  assert.deepEqual(privilegesFor("nothing"), []);
});

test("revoking a grant entry undoes what it holds, including the grant option", () => {
  assert.deepEqual(revokablePrivileges({ privileges: ["SELECT", "INSERT"], grantOption: false }), ["SELECT", "INSERT"]);
  assert.deepEqual(revokablePrivileges({ privileges: ["SELECT"], grantOption: true }), ["SELECT", "GRANT OPTION"]);
  assert.deepEqual(revokablePrivileges({ privileges: ["ALL PRIVILEGES"], grantOption: true }), ["ALL PRIVILEGES", "GRANT OPTION"]);
  assert.deepEqual(revokablePrivileges({ privileges: ["USAGE"], grantOption: false }), []);
});

test("the scope sent to the backend carries only what its kind needs", () => {
  assert.deepEqual(scopeFromForm({ kind: "global", database: "x", table: "y" }), { kind: "global" });
  assert.deepEqual(scopeFromForm({ kind: "database", database: "x", table: "y" }), { kind: "database", database: "x" });
  assert.deepEqual(scopeFromForm({ kind: "table", database: "x", table: "y" }), { kind: "table", database: "x", table: "y" });
});

test("sleeping connections can be hidden", () => {
  const processes = [{ command: "Sleep" }, { command: "Query" }];

  assert.equal(visibleProcesses(processes, { hideSleeping: true }).length, 1);
  assert.equal(visibleProcesses(processes, { hideSleeping: false }).length, 2);
  assert.equal(formatSeconds(30), "30 s");
  assert.equal(formatSeconds(7200), "2 h 00 min");
});

test("history keeps the newest readings without changing the old list", () => {
  const old = [1, 2, 3];

  assert.deepEqual(pushHistory(old, 4, 3), [2, 3, 4]);
  assert.deepEqual(old, [1, 2, 3]);
});

test("a sparkline scales to its box and needs two readings", () => {
  assert.equal(sparklinePoints([5], 100, 20), "");
  assert.equal(sparklinePoints([0, 10], 100, 20), "0.0,20.0 100.0,0.0");
  assert.equal(sparklinePoints([3, 3, 3], 100, 20), "0.0,20.0 50.0,20.0 100.0,20.0");
});

test("account badges name the locked and expired states", () => {
  assert.deepEqual(accountBadges({ locked: true, passwordExpired: true }), ["locked", "password expired"]);
  assert.deepEqual(accountBadges({ locked: null, passwordExpired: false }), []);
});

test("the replication summary tells replica, source and idle servers apart", () => {
  const pair = (name, value) => ({ name, value });

  assert.match(replicationSummary({ replica: [], source: [] }), /binary logging is off/);
  assert.match(replicationSummary({ replica: [], source: [pair("File", "b.1")] }), /writes a binary log/);
  assert.equal(replicationSummary({ replica: [pair("Replica_IO_Running", "Yes"), pair("Replica_SQL_Running", "Yes"), pair("Seconds_Behind_Source", "3")], source: [] }), "Replicating · 3 s behind");
  assert.match(replicationSummary({ replica: [pair("Slave_IO_Running", "No"), pair("Slave_SQL_Running", "Yes")], source: [] }), /not running/);
});
