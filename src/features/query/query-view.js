import { h } from "../../lib/dom.js";
import { displayCell } from "../grid/cell-format.js";
import { createGrid } from "../grid/grid.js";
import { buildCompletionSchema } from "./completion-schema.js";
import { createSqlEditor } from "./editor.js";
import { addToHistory, filterHistory, historyKey, loadHistory, saveHistory } from "./history.js";
import { NO_RESULTS, applyEvent, describeStatement, failRun, finishRun, formatCount, setLabel, startRun } from "./results.js";
import { chooseToRun, explainTexts } from "./statement-at.js";

const MESSAGES_TAB = "messages";
const NO_DATABASE = "";
const MIN_PANE_PX = 90;

function statusIcon(status) {
  return { done: "✓", error: "✕", cancelled: "■", running: "…", pending: "·" }[status];
}

/**
 * A query tab: toolbar, SQL editor and results. Each tab has its own server session, so
 * `USE`, `SET @x` and open transactions carry over between runs.
 * `data` is the query actions of the connection; `storage` holds the history.
 */
export function createQueryView({ tab, connection, data, storage }) {
  const historyStorageKey = historyKey(connection);
  let sessionPromise = data.openSession();
  let database = tab.database ?? NO_DATABASE;
  let databaseNames = [];
  let objectsByDatabase = {};
  let columnsByTable = {};
  let results = NO_RESULTS;
  let activeResultId = MESSAGES_TAB;
  let lastRunText = null;
  let drawScheduled = false;
  let destroyed = false;

  const grid = createGrid({ onSort: () => {}, onEdit: () => {}, onSelect: () => {}, onNearEnd: () => {} });
  const databaseSelect = h("select", { "aria-label": "Database for this query", onChange: () => changeDatabase(databaseSelect.value) });
  const runButton = h("button", { type: "button", class: "primary", title: "Run the selection or the current statement (⌘↵)", onClick: () => void run("current") }, "Run");
  const runAllButton = h("button", { type: "button", title: "Run every statement (⇧⌘↵)", onClick: () => void run("all") }, "Run all");
  const explainButton = h("button", { type: "button", title: "EXPLAIN the current statement", onClick: () => void run("current", { explain: true }) }, "Explain");
  const cancelButton = h("button", { type: "button", class: "danger", title: "Stop the running statement (⌘.)", onClick: () => void cancel() }, "Cancel");
  const historyButton = h("button", { type: "button", onClick: openHistory }, "History");
  const runState = h("span", { class: "muted query-state" });
  const editorHost = h("div", { class: "query-editor" });
  const divider = h("div", { class: "query-divider", role: "separator", "aria-orientation": "horizontal", title: "Drag to resize" });
  const resultTabs = h("div", { class: "result-tabs" });
  const resultBody = h("div", { class: "result-body" });
  const resultFooter = h("div", { class: "result-footer muted" });
  const element = h(
    "div",
    { class: "query-view" },
    h("div", { class: "query-toolbar" }, runButton, runAllButton, explainButton, cancelButton, h("span", { class: "spacer" }), runState, databaseSelect, historyButton),
    editorHost,
    divider,
    h("div", { class: "query-results" }, resultTabs, resultBody, resultFooter),
  );

  const editor = createSqlEditor({
    parent: editorHost,
    engine: connection.engine,
    onRun: (mode) => void run(mode),
    onCancel: () => void cancel(),
    onChange: () => {},
  });

  // ---- layout -------------------------------------------------------------

  divider.addEventListener("pointerdown", (event) => {
    divider.setPointerCapture(event.pointerId);
    const move = (moveEvent) => {
      const bounds = element.getBoundingClientRect();
      const toolbarHeight = element.querySelector(".query-toolbar").offsetHeight;
      const editorHeight = moveEvent.clientY - bounds.top - toolbarHeight;
      const clamped = Math.min(Math.max(editorHeight, MIN_PANE_PX), bounds.height - toolbarHeight - MIN_PANE_PX);
      element.style.setProperty("--editor-height", `${clamped}px`);
    };
    const stop = () => {
      divider.removeEventListener("pointermove", move);
      divider.removeEventListener("pointerup", stop);
    };
    divider.addEventListener("pointermove", move);
    divider.addEventListener("pointerup", stop);
  });

  // ---- database and completion --------------------------------------------

  function drawDatabases() {
    const names = database !== NO_DATABASE && !databaseNames.includes(database) ? [database, ...databaseNames] : databaseNames;
    databaseSelect.replaceChildren(
      h("option", { value: NO_DATABASE, selected: database === NO_DATABASE }, "(no database)"),
      ...names.map((name) => h("option", { value: name, selected: name === database }, name)),
    );
  }

  function refreshCompletion() {
    editor.setCompletion(buildCompletionSchema({ database, objectsByDatabase, columnsByTable }));
  }

  async function loadColumns() {
    columnsByTable = {};
    if (database !== NO_DATABASE) {
      // Autocomplete is a convenience: if columns cannot be loaded the failure is already in the log.
      columnsByTable = await data.columns(database).catch(() => ({}));
    }
    if (!destroyed) refreshCompletion();
  }

  function changeDatabase(next) {
    database = next;
    refreshCompletion();
    void loadColumns();
  }

  // ---- results ------------------------------------------------------------

  function scheduleDraw() {
    if (drawScheduled || destroyed) return;
    drawScheduled = true;
    requestAnimationFrame(() => {
      drawScheduled = false;
      if (!destroyed) draw();
    });
  }

  function activeSet() {
    return results.sets.find((set) => set.id === activeResultId) ?? null;
  }

  function showSet(set) {
    grid.setModel(
      {
        columns: set.columns.map((column) => ({ name: column.name, detail: column.dataType, sort: null })),
        rowCount: set.rows.length,
        cellAt: (row, col) => displayCell(set.rows[row][col]),
        canEdit: () => false,
        editText: () => "",
        rowClass: () => "",
      },
      { keepScroll: true },
    );
  }

  function messagesPanel() {
    if (results.runError !== null) return h("div", { class: "messages" }, h("p", { class: "error-text" }, results.runError));
    if (results.statements.length === 0) {
      return h("div", { class: "messages" }, h("p", { class: "muted empty" }, "Write a statement and press ⌘↵ to run it, or ⇧⌘↵ to run everything."));
    }
    return h(
      "div",
      { class: "messages" },
      ...results.statements.map((statement, index) =>
        h(
          "div",
          { class: `message message-${statement.status}` },
          h("span", { class: "message-icon" }, statusIcon(statement.status)),
          h(
            "div",
            {},
            h("div", { class: "message-sql" }, statement.sql),
            h("div", { class: "message-text" }, describeStatement(statement, results.sets.filter((set) => set.statementIndex === index))),
          ),
        ),
      ),
    );
  }

  function resultTabLabel(set) {
    const shown = formatCount(set.rows.length, "row");
    return `${setLabel(results.sets, set)} · ${shown}${set.truncated ? "+" : ""}`;
  }

  function draw() {
    const selected = activeSet();
    if (selected === null && activeResultId !== MESSAGES_TAB) activeResultId = MESSAGES_TAB;
    resultTabs.replaceChildren(
      ...results.sets.map((set) =>
        h("button", { type: "button", class: set.id === activeResultId ? "result-tab active" : "result-tab", title: results.statements[set.statementIndex].sql, onClick: () => select(set.id) }, resultTabLabel(set)),
      ),
      h("button", { type: "button", class: activeResultId === MESSAGES_TAB ? "result-tab active" : "result-tab", onClick: () => select(MESSAGES_TAB) }, "Messages"),
    );
    if (selected === null) {
      resultBody.replaceChildren(messagesPanel());
    } else {
      if (resultBody.firstChild !== grid.element) resultBody.replaceChildren(grid.element);
      showSet(selected);
    }
    drawToolbar(selected);
  }

  function drawToolbar(selected) {
    const busy = results.running;
    for (const button of [runButton, runAllButton, explainButton, databaseSelect]) button.disabled = busy;
    cancelButton.hidden = !busy;
    runState.textContent = busy ? "Running…" : "";
    if (selected === null) {
      resultFooter.textContent = "";
      return;
    }
    const cut = selected.truncated ? ` · showing the first ${formatCount(selected.rows.length, "row")} of ${formatCount(selected.totalRows, "row")}` : "";
    resultFooter.textContent = `${formatCount(selected.done ? selected.totalRows : selected.rows.length, "row")}${cut}${selected.done ? "" : " · loading…"}`;
  }

  function select(id) {
    activeResultId = id;
    draw();
  }

  // ---- running ------------------------------------------------------------

  /** Selected text wins over the cursor: it is split and everything in it runs. */
  async function statementsToRun(mode) {
    const selection = editor.getSelectedText();
    if (selection.trim() !== "") return data.split(selection);
    const spans = await data.split(editor.getText());
    return chooseToRun(mode, spans, editor.getCursor());
  }

  async function run(mode, { explain = false } = {}) {
    if (results.running) return;
    let spans;
    try {
      spans = await statementsToRun(explain ? "current" : mode);
    } catch (error) {
      results = failRun(startRun([]), error.message);
      draw();
      return;
    }
    if (spans.length === 0) {
      runState.textContent = "Nothing to run";
      return;
    }
    const texts = explain ? explainTexts(spans) : spans.map((span) => span.text);
    results = startRun(texts);
    activeResultId = MESSAGES_TAB;
    lastRunText = editor.getText();
    draw();

    try {
      const sessionId = await sessionPromise;
      const summary = await data.run({
        sessionId,
        database: database === NO_DATABASE ? null : database,
        statements: texts,
        onEvent: (event) => {
          results = applyEvent(results, event);
          // Show the first result as soon as it starts arriving.
          if (event.event === "resultSetStarted" && activeResultId === MESSAGES_TAB && results.sets.length === 1) activeResultId = results.sets[0].id;
          scheduleDraw();
        },
      });
      results = finishRun(results, summary);
      // A failed or cancelled run explains itself on the Messages tab.
      if (summary.failed || summary.cancelled) activeResultId = MESSAGES_TAB;
      if (summary.currentDatabase !== null && summary.currentDatabase !== database) {
        database = summary.currentDatabase;
        drawDatabases();
        void loadColumns();
      }
      if (summary.statementsRun > 0) {
        const entries = addToHistory(loadHistory(storage, historyStorageKey), texts.slice(0, summary.statementsRun));
        saveHistory(storage, historyStorageKey, entries);
      }
    } catch (error) {
      results = failRun(results, error.message);
      // A lost connection ends the session; the next run opens a new one.
      sessionPromise = data.openSession();
    }
    if (!destroyed) draw();
  }

  async function cancel() {
    if (!results.running) return;
    runState.textContent = "Cancelling…";
    try {
      await data.cancel(await sessionPromise);
    } catch (error) {
      runState.textContent = error.message;
    }
  }

  // ---- history ------------------------------------------------------------

  function openHistory() {
    const entries = loadHistory(storage, historyStorageKey);
    const search = h("input", { placeholder: "Search history", spellcheck: false });
    const list = h("div", { class: "history-list" });
    const dialog = h(
      "dialog",
      { class: "dialog history-dialog" },
      h("h2", {}, "History"),
      search,
      list,
      h("div", { class: "dialog-actions" }, h("button", { type: "button", onClick: () => dialog.close() }, "Close")),
    );
    const drawList = () => {
      const shown = filterHistory(entries, search.value);
      list.replaceChildren(
        ...(shown.length === 0
          ? [h("p", { class: "muted empty" }, entries.length === 0 ? "Statements you run appear here." : "No matches.")]
          : shown.map((entry) =>
              h(
                "button",
                { type: "button", class: "history-entry", title: "Insert into the editor", onClick: () => (dialog.close(), editor.insert(entry.sql)) },
                h("span", { class: "history-sql" }, entry.sql),
                h("span", { class: "muted" }, new Date(entry.at).toLocaleString()),
              ),
            )),
      );
    };
    search.addEventListener("input", drawList);
    dialog.addEventListener("close", () => dialog.remove());
    drawList();
    document.body.append(dialog);
    dialog.showModal();
    search.focus();
  }

  // ---- lifecycle ----------------------------------------------------------

  drawDatabases();
  draw();
  void loadColumns();

  return {
    element,
    /** The tree's databases and loaded objects, for the selector and autocomplete. */
    update({ databases, objects }) {
      databaseNames = databases;
      objectsByDatabase = objects;
      drawDatabases();
      refreshCompletion();
    },
    /** Reload button: refresh what autocomplete knows. */
    async reload() {
      await loadColumns();
      return true;
    },
    focus: () => editor.focus(),
    /** True for text that was written but never run, or changed since the last run. */
    hasUnsavedChanges: () => editor.getText().trim() !== "" && editor.getText() !== lastRunText,
    destroy() {
      destroyed = true;
      editor.destroy();
      void sessionPromise.then((sessionId) => data.closeSession(sessionId)).catch(() => {});
    },
  };
}
