import { Channel } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

const LOGGED_SQL_CHARS = 400;

const preview = (sql) => (sql.length > LOGGED_SQL_CHARS ? `${sql.slice(0, LOGGED_SQL_CHARS)}…` : sql);

/** Backend calls for the query editor, bound to one connection. */
export function createQueryActions({ api, log, connectionId }) {
  return {
    split: (sql) => api("split_statements", { sql }, { quiet: true }),
    openSession: () => api("open_query_session", { connectionId }, { quiet: true }),
    closeSession: (sessionId) => api("close_query_session", { sessionId }, { quiet: true }).catch(() => {}),
    columns: (database) => api("list_database_columns", { connectionId, database }, { label: `Load columns of ${database}` }),
    cancel: (sessionId) => api("cancel_query", { sessionId }, { label: "Cancel query" }),
    pickOpenPath: () => open({ multiple: false, directory: false, filters: [{ name: "SQL", extensions: ["sql", "txt"] }] }),
    pickSavePath: (defaultName) => save({ defaultPath: defaultName, filters: [{ name: "SQL", extensions: ["sql"] }] }),
    readFile: (path) => api("read_text_file", { path }, { label: `Open ${path}` }),
    writeFile: (path, text) => api("write_export_file", new TextEncoder().encode(text), { label: `Save ${path}`, headers: { path: encodeURIComponent(path) } }),
    /** Re-runs a statement and writes all its rows to a CSV or JSON file. */
    exportResult: (request, onEvent) => {
      const channel = new Channel();
      channel.onmessage = onEvent;
      return api("export_query_result", { connectionId, request, onEvent: channel }, { label: (summary) => `Export result → ${summary.path} (${summary.rows} rows${summary.cancelled ? ", cancelled" : ""})` });
    },
    pickResultPath: (defaultName, extension) => save({ defaultPath: defaultName, filters: [{ name: extension.toUpperCase(), extensions: [extension] }] }),
    cancelJob: (jobId) => api("cancel_transfer", { jobId }, { quiet: true }).catch(() => false),
    /** Streams the backend's events to `onEvent` and resolves to the run summary. */
    run({ sessionId, database, statements, onEvent }) {
      const channel = new Channel();
      channel.onmessage = (event) => {
        if (event.event === "statementFailed" && !event.cancelled) log({ level: "error", message: event.message, durationMs: event.durationMs });
        onEvent(event);
      };
      return api(
        "run_statements",
        { sessionId, database, statements, onEvent: channel },
        { label: () => statements.map(preview).join(";\n") },
      );
    },
  };
}
