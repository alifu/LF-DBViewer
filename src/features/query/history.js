export const MAX_HISTORY = 200;
const STORAGE_PREFIX = "lf-dbviewer.history.";

/** History is kept per server, so the same user and host share it across sessions. */
export function historyKey(connection) {
  return `${connection.user}@${connection.host}:${connection.port}`;
}

/** Newest first; running a statement again moves it to the top instead of duplicating it. */
export function addToHistory(history, statements, now = Date.now()) {
  const added = statements.map((sql) => sql.trim()).filter((sql) => sql !== "");
  const kept = history.filter((entry) => !added.includes(entry.sql));
  const entries = [...added].reverse().map((sql) => ({ sql, at: now }));
  return [...entries, ...kept].slice(0, MAX_HISTORY);
}

export function filterHistory(history, needle) {
  const text = needle.trim().toLowerCase();
  return text === "" ? history : history.filter((entry) => entry.sql.toLowerCase().includes(text));
}

function isEntry(value) {
  return value !== null && typeof value === "object" && typeof value.sql === "string" && typeof value.at === "number";
}

/** Storage can be missing, blocked or hold something unexpected; history then just starts empty. */
export function loadHistory(storage, key) {
  try {
    const parsed = JSON.parse(storage.getItem(STORAGE_PREFIX + key) ?? "[]");
    return Array.isArray(parsed) ? parsed.filter(isEntry).slice(0, MAX_HISTORY) : [];
  } catch {
    return [];
  }
}

export function saveHistory(storage, key, history) {
  try {
    storage.setItem(STORAGE_PREFIX + key, JSON.stringify(history));
  } catch {
    // History is a convenience; losing it must not interrupt a query.
  }
}
