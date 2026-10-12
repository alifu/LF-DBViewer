/** Pure helpers for importing a CSV file into a table. */

export const DELIMITERS = Object.freeze([
  { value: "auto", label: "Detect" },
  { value: ",", label: "Comma  ," },
  { value: ";", label: "Semicolon  ;" },
  { value: "\\t", label: "Tab" },
  { value: "|", label: "Pipe  |" },
]);

const normalise = (name) => name.toLowerCase().replace(/[^a-z0-9]/g, "");

/**
 * The table column for each CSV column: the one with the same name, ignoring case, spaces and underscores
 * (`First Name` goes to `first_name`). A table column is used at most once; the rest are skipped (null).
 */
export function autoMapping(headers, tableColumns) {
  const free = new Map(tableColumns.map((name) => [normalise(name), name]));
  return headers.map((header) => {
    const key = normalise(header);
    const found = key === "" ? undefined : free.get(key);
    if (found === undefined) return null;
    free.delete(key);
    return found;
  });
}

export function csvProblem({ path, mapping }) {
  if (path.trim() === "") return "Choose a CSV file.";
  if (!mapping.some((column) => column !== null)) return "Choose at least one table column to fill.";
  return null;
}

/** A table column picked for two CSV columns, which the server would refuse. */
export function duplicateTargets(mapping) {
  const seen = new Set();
  const repeated = new Set();
  for (const column of mapping) {
    if (column === null) continue;
    if (seen.has(column)) repeated.add(column);
    seen.add(column);
  }
  return [...repeated];
}

export function buildCsvRequest({ jobId, path, database, table, delimiter, hasHeader, mapping, nullText, emptyAsNull, truncateFirst }) {
  return { jobId, path, database, table, delimiter, hasHeader, mapping, nullText: nullText === "" ? null : nullText, emptyAsNull, truncateFirst };
}

export const IDLE_CSV = Object.freeze({ phase: "idle", rows: 0, bytesRead: 0, totalBytes: 0, summary: null, error: null });

export function applyCsvEvent(state, event) {
  if (event.event === "started") return { ...IDLE_CSV, phase: "running", totalBytes: event.totalBytes };
  if (event.event === "progress") return { ...state, rows: event.rows, bytesRead: event.bytesRead, totalBytes: event.totalBytes };
  return state;
}

export const finishCsv = (state, summary) => ({ ...state, phase: summary.cancelled ? "cancelled" : "done", summary, rows: summary.rows });
export const failCsv = (state, message) => ({ ...state, phase: "failed", error: message });
