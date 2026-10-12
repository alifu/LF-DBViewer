/** Pure helpers for the server tab: status maths, privilege lists, and filtering. */

const SECONDS_PER = { minute: 60, hour: 3600, day: 86_400 };

export function formatUptime(totalSeconds) {
  const seconds = Math.max(0, Math.floor(totalSeconds));
  const days = Math.floor(seconds / SECONDS_PER.day);
  const hours = Math.floor((seconds % SECONDS_PER.day) / SECONDS_PER.hour);
  const minutes = Math.floor((seconds % SECONDS_PER.hour) / SECONDS_PER.minute);
  if (days > 0) return `${days} d ${hours} h`;
  if (hours > 0) return `${hours} h ${String(minutes).padStart(2, "0")} min`;
  if (minutes > 0) return `${minutes} min ${String(seconds % SECONDS_PER.minute).padStart(2, "0")} s`;
  return `${seconds} s`;
}

/** `[{ name, value }]` as a lookup of numbers; values that are not numbers are left out. */
export function numbersOf(pairs) {
  const numbers = {};
  for (const { name, value } of pairs) {
    const number = Number(value);
    if (value !== "" && Number.isFinite(number)) numbers[name] = number;
  }
  return numbers;
}

/** Change per second of every counter that grew between two readings. A restart (a value that fell) is skipped. */
export function computeRates(previous, next, seconds) {
  if (!(seconds > 0)) return {};
  const rates = {};
  for (const [name, value] of Object.entries(next)) {
    const before = previous[name];
    if (before !== undefined && value >= before) rates[name] = (value - before) / seconds;
  }
  return rates;
}

function perSecond(rate, unit) {
  if (rate === undefined) return null;
  return rate >= 100 ? `${Math.round(rate)} ${unit}` : `${rate.toFixed(1)} ${unit}`;
}

const KILOBYTE = 1024;

function bytesPerSecond(rate) {
  if (rate === undefined) return null;
  if (rate >= KILOBYTE * KILOBYTE) return `${(rate / KILOBYTE / KILOBYTE).toFixed(1)} MB/s`;
  if (rate >= KILOBYTE) return `${(rate / KILOBYTE).toFixed(1)} KB/s`;
  return `${Math.round(rate)} B/s`;
}

/** Headline figures: always the ones the server reports, and the per-second ones once two readings exist. */
export function summarizeStatus(numbers, rates) {
  const tiles = [];
  const add = (label, value, detail = null) => value !== null && value !== undefined && tiles.push({ label, value, detail });
  if (numbers.Uptime !== undefined) add("Uptime", formatUptime(numbers.Uptime));
  if (numbers.Threads_connected !== undefined) add("Connections", String(numbers.Threads_connected), numbers.Threads_running === undefined ? null : `${numbers.Threads_running} running`);
  if (numbers.Questions !== undefined) {
    const total = numbers.Questions.toLocaleString("en-US");
    const rate = perSecond(rates.Questions, "/s");
    add("Queries", rate ?? total, rate === null ? "total since start" : `${total} total`);
  }
  add("Traffic in", bytesPerSecond(rates.Bytes_received));
  add("Traffic out", bytesPerSecond(rates.Bytes_sent));
  if (numbers.Slow_queries !== undefined) add("Slow queries", numbers.Slow_queries.toLocaleString("en-US"));
  const requests = numbers.Innodb_buffer_pool_read_requests;
  if (requests > 0 && numbers.Innodb_buffer_pool_reads !== undefined) {
    add("Buffer pool hits", `${(100 * (1 - numbers.Innodb_buffer_pool_reads / requests)).toFixed(2)}%`);
  }
  if (numbers.Open_tables !== undefined) add("Open tables", String(numbers.Open_tables));
  return tiles;
}

export function filterByName(items, text, key = (item) => item.name) {
  const needle = text.trim().toLowerCase();
  return needle === "" ? items : items.filter((item) => key(item).toLowerCase().includes(needle));
}

// ---- privileges -----------------------------------------------------------------------------

const TABLE_PRIVILEGES = ["SELECT", "INSERT", "UPDATE", "DELETE", "CREATE", "DROP", "ALTER", "INDEX", "REFERENCES", "CREATE VIEW", "SHOW VIEW", "TRIGGER"];
const DATABASE_PRIVILEGES = [...TABLE_PRIVILEGES, "CREATE TEMPORARY TABLES", "LOCK TABLES", "EXECUTE", "CREATE ROUTINE", "ALTER ROUTINE", "EVENT"];
const GLOBAL_PRIVILEGES = [...DATABASE_PRIVILEGES, "FILE", "PROCESS", "RELOAD", "SHUTDOWN", "SUPER", "SHOW DATABASES", "CREATE USER", "REPLICATION CLIENT", "REPLICATION SLAVE", "CREATE TABLESPACE"];

/** The privileges a scope can have, matching what the backend accepts. */
export function privilegesFor(scopeKind) {
  return { global: GLOBAL_PRIVILEGES, database: DATABASE_PRIVILEGES, table: TABLE_PRIVILEGES }[scopeKind] ?? [];
}

export function describeScope(scope) {
  if (scope.kind === "global") return "All databases (*.*)";
  if (scope.kind === "database") return `Database ${scope.database}`;
  return `Table ${scope.database}.${scope.table}`;
}

/** The privileges to revoke to undo a grant entry, or none when it holds nothing but USAGE. */
export function revokablePrivileges(entry) {
  const real = entry.privileges.filter((privilege) => privilege !== "USAGE");
  if (real.length === 0) return [];
  return entry.grantOption && !real.includes("ALL PRIVILEGES") ? [...real, "GRANT OPTION"] : real.concat(entry.grantOption ? ["GRANT OPTION"] : []);
}

export function scopeFromForm({ kind, database, table }) {
  if (kind === "global") return { kind: "global" };
  if (kind === "database") return { kind: "database", database };
  return { kind: "table", database, table };
}

// ---- processes ------------------------------------------------------------------------------

export function visibleProcesses(processes, { hideSleeping }) {
  return hideSleeping ? processes.filter((process) => process.command !== "Sleep") : processes;
}

export function formatSeconds(seconds) {
  return seconds >= SECONDS_PER.hour ? formatUptime(seconds) : `${seconds} s`;
}
