/**
 * State of one query run, built up from the backend's events. Every function returns a
 * new state, so the view can compare states and redraw only what changed.
 */

export const NO_RESULTS = Object.freeze({ statements: [], sets: [], running: false, summary: null, runError: null });

export function startRun(sqlList) {
  return {
    statements: sqlList.map((sql) => ({ sql, status: "pending" })),
    sets: [],
    running: true,
    summary: null,
    runError: null,
  };
}

function updateStatement(state, index, change) {
  return {
    ...state,
    statements: state.statements.map((statement, position) => (position === index ? { ...statement, ...change } : statement)),
  };
}

function updateSet(state, index, set, change) {
  return {
    ...state,
    sets: state.sets.map((candidate) => (candidate.statementIndex === index && candidate.set === set ? { ...candidate, ...change(candidate) } : candidate)),
  };
}

export function applyEvent(state, event) {
  switch (event.event) {
    case "statementStarted":
      return updateStatement(state, event.index, { status: "running" });
    case "resultSetStarted":
      return {
        ...state,
        sets: [
          ...state.sets,
          { id: `${event.index}:${event.set}`, statementIndex: event.index, set: event.set, columns: event.columns, rows: [], totalRows: 0, truncated: false, done: false },
        ],
      };
    case "rows":
      return updateSet(state, event.index, event.set, (current) => ({ rows: [...current.rows, ...event.rows] }));
    case "resultSetFinished":
      return updateSet(state, event.index, event.set, () => ({ totalRows: event.totalRows, truncated: event.truncated, done: true }));
    case "statementFinished":
      return updateStatement(state, event.index, {
        status: "done",
        affectedRows: event.affectedRows,
        lastInsertId: event.lastInsertId,
        warnings: event.warnings,
        info: event.info,
        durationMs: event.durationMs,
      });
    case "statementFailed":
      return updateStatement(state, event.index, {
        status: event.cancelled ? "cancelled" : "error",
        code: event.code,
        message: event.message,
        durationMs: event.durationMs,
      });
    default:
      return state;
  }
}

export function finishRun(state, summary) {
  return { ...state, running: false, summary };
}

/** The run itself could not happen (no session, connection gone); nothing was executed. */
export function failRun(state, message) {
  return { ...state, running: false, runError: message };
}

export function setLabel(sets, set) {
  return `Result ${sets.indexOf(set) + 1}`;
}

const numberFormat = new Intl.NumberFormat("en-US");

export function formatCount(count, noun) {
  return `${numberFormat.format(count)} ${noun}${count === 1 ? "" : "s"}`;
}

/** One line for the Messages tab. */
export function describeStatement(statement, resultSets) {
  const took = statement.durationMs === undefined ? "" : ` (${statement.durationMs} ms)`;
  switch (statement.status) {
    case "pending":
      return "Not run";
    case "running":
      return "Running…";
    case "cancelled":
      return `Cancelled${took}`;
    case "error":
      return `${statement.message}${took}`;
    default: {
      const parts = resultSets.map((set) => `${formatCount(set.totalRows, "row")}${set.truncated ? " (not all shown)" : ""}`);
      if (parts.length === 0 && statement.affectedRows !== null && statement.affectedRows !== undefined) {
        parts.push(`${formatCount(statement.affectedRows, "row")} affected`);
      }
      if (statement.lastInsertId) parts.push(`last insert id ${statement.lastInsertId}`);
      if (statement.warnings > 0) parts.push(formatCount(statement.warnings, "warning"));
      return `OK${parts.length > 0 ? `, ${parts.join(", ")}` : ""}${took}`;
    }
  }
}
