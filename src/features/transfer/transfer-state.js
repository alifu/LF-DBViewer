/** Pure helpers for the export and import tabs: formatting, building requests, and progress built from events. */

const UNITS = ["B", "KB", "MB", "GB", "TB"];
const KILO = 1024;

export function formatBytes(bytes) {
  let value = bytes;
  let unit = 0;
  while (value >= KILO && unit < UNITS.length - 1) {
    value /= KILO;
    unit += 1;
  }
  return `${unit === 0 ? value : value.toFixed(value >= 100 ? 0 : 1)} ${UNITS[unit]}`;
}

export function formatDuration(milliseconds) {
  if (milliseconds < 1000) return `${Math.round(milliseconds)} ms`;
  const seconds = milliseconds / 1000;
  if (seconds < 60) return `${seconds.toFixed(1)} s`;
  const whole = Math.round(seconds);
  return `${Math.floor(whole / 60)} min ${String(whole % 60).padStart(2, "0")} s`;
}

export function percent(done, total) {
  if (!total || total <= 0) return 0;
  return Math.max(0, Math.min(100, Math.floor((done / total) * 100)));
}

const two = (number) => String(number).padStart(2, "0");

/** `shop_20261012_1004.sql` (or `.sql.gz`), in local time. */
export function defaultFileName(database, date, gzip = false) {
  const day = `${date.getFullYear()}${two(date.getMonth() + 1)}${two(date.getDate())}`;
  return `${database}_${day}_${two(date.getHours())}${two(date.getMinutes())}.sql${gzip ? ".gz" : ""}`;
}

/** The path with `.gz` added or taken off, so the checkbox and the file name agree. */
export function withGzip(path, enabled) {
  const plain = path.replace(/\.gz$/i, "");
  return enabled && plain !== "" ? `${plain}.gz` : plain;
}

export const DEFAULT_EXPORT_OPTIONS = Object.freeze({
  structure: true,
  data: true,
  addDrop: true,
  createDatabase: true,
  routines: true,
  triggers: true,
  events: true,
  keepDefiners: false,
  consistentSnapshot: true,
});

/** The problem with the form, or null when it can be run. */
export function exportProblem({ path, tables, views, options }) {
  if (path.trim() === "") return "Choose where to save the file.";
  if (!options.structure && !options.data) return "Export the structure, the data, or both.";
  if (tables.length === 0 && views.length === 0 && !options.routines && !options.events) return "Select at least one table, view, routine or event.";
  return null;
}

export function buildExportRequest({ jobId, database, path, tables, views, options }) {
  return { jobId, database, path, tables, views, ...options };
}

export function importProblem({ path }) {
  return path.trim() === "" ? "Choose a SQL file to import." : null;
}

export function buildImportRequest({ jobId, path, database, createDatabase, stopOnError, disableForeignKeyChecks }) {
  return { jobId, path, database: database === "" ? null : database, createDatabase, stopOnError, disableForeignKeyChecks };
}

// ---- progress ----------------------------------------------------------------------------

export const IDLE_EXPORT = Object.freeze({ phase: "idle", tables: [], current: null, bytes: 0, notes: [], summary: null, error: null });

const withTable = (tables, name, change) => tables.map((table) => (table.name === name ? { ...table, ...change } : table));

export function applyExportEvent(state, event) {
  switch (event.event) {
    case "started":
      return { ...IDLE_EXPORT, phase: "running" };
    case "tableStarted":
      return { ...state, current: event.name, tables: [...state.tables, { name: event.name, rows: 0, estimatedRows: event.estimatedRows, done: false }] };
    case "progress":
      return { ...state, bytes: event.bytes, tables: withTable(state.tables, event.table, { rows: event.rows }) };
    case "tableFinished":
      return { ...state, tables: withTable(state.tables, event.name, { rows: event.rows, done: true }) };
    case "note":
      return { ...state, notes: [...state.notes, event.message] };
    default:
      return state;
  }
}

export function finishExport(state, summary) {
  return { ...state, phase: summary.cancelled ? "cancelled" : "done", summary, current: null, bytes: summary.bytes };
}

export function failExport(state, message) {
  return { ...state, phase: "failed", error: message, current: null };
}

export const IDLE_IMPORT = Object.freeze({ phase: "idle", bytesRead: 0, totalBytes: 0, statements: 0, line: 0, errors: [], summary: null, error: null });

export function applyImportEvent(state, event) {
  switch (event.event) {
    case "started":
      return { ...IDLE_IMPORT, phase: "running", totalBytes: event.totalBytes };
    case "progress":
      return { ...state, bytesRead: event.bytesRead, totalBytes: event.totalBytes, statements: event.statements, line: event.line };
    case "statementFailed":
      return { ...state, errors: [...state.errors, event.error] };
    default:
      return state;
  }
}

export function finishImport(state, summary) {
  const phase = summary.cancelled ? "cancelled" : summary.failed ? "failed" : "done";
  return { ...state, phase, summary, errors: summary.errors, statements: summary.statements, bytesRead: summary.bytes };
}

export function failImport(state, message) {
  return { ...state, phase: "failed", error: message };
}
