import { Channel } from "@tauri-apps/api/core";

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
