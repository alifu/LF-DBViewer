import { h } from "../../lib/dom.js";
import {
  DEFAULT_EXPORT_OPTIONS, IDLE_EXPORT, applyExportEvent, buildExportRequest, defaultFileName, exportProblem, failExport, finishExport,
  formatBytes, formatDuration, percent, withGzip,
} from "./transfer-state.js";

const OPTION_LABELS = [
  ["structure", "Structure (CREATE TABLE)"],
  ["data", "Data (INSERT statements)"],
  ["addDrop", "Add DROP before each table, so loading replaces what is there"],
  ["createDatabase", "Add CREATE DATABASE and USE, so the file restores on its own"],
  ["routines", "Procedures and functions"],
  ["triggers", "Triggers"],
  ["events", "Events"],
  ["keepDefiners", "Keep DEFINER (otherwise objects belong to whoever imports)"],
  ["consistentSnapshot", "Read all tables from one point in time (InnoDB)"],
];

const check = (checked, onChange, label) => h("label", { class: "check" }, h("input", { type: "checkbox", checked, onChange: (event) => onChange(event.target.checked) }), label);

/**
 * Export tab: choose tables and options, pick the file, then watch it write. `data`: `objects()`,
 * `pickPath(defaultName)`, `start(request, onEvent)` and `cancel(jobId)`.
 */
export function createExportView({ tab, data }) {
  const { tables: allTables, views: allViews } = data.objects();
  let selectedTables = new Set(tab.tables ?? allTables);
  let selectedViews = new Set(tab.tables ? [] : allViews);
  let options = { ...DEFAULT_EXPORT_OPTIONS };
  let path = "";
  /** Used while no path is chosen yet; afterwards the path's own `.gz` ending says whether the file is compressed. */
  let gzip = false;
  let progress = IDLE_EXPORT;
  let jobId = null;
  let drawScheduled = false;

  const body = h("div", { class: "transfer-body" });
  const element = h("div", { class: "transfer" }, h("h2", {}, `Export ${tab.database}`), body);

  function selection(title, names, selected, onChange) {
    const boxes = names.map((name) => check(selected.has(name), (value) => (value ? selected.add(name) : selected.delete(name)), name));
    return h(
      "fieldset",
      { class: "transfer-group" },
      h("legend", {}, `${title} (${names.length})`),
      h("div", { class: "transfer-links" }, h("button", { type: "button", class: "ghost", onClick: () => (onChange(new Set(names)), draw()) }, "All"), h("button", { type: "button", class: "ghost", onClick: () => (onChange(new Set()), draw()) }, "None")),
      h("div", { class: "transfer-list" }, ...(boxes.length > 0 ? boxes : [h("span", { class: "muted" }, "None")])),
    );
  }

  function drawForm(error) {
    return [
      h(
        "div",
        { class: "transfer-columns" },
        h("div", {}, selection("Tables", allTables, selectedTables, (next) => (selectedTables = next)), selection("Views", allViews, selectedViews, (next) => (selectedViews = next))),
        h("fieldset", { class: "transfer-group" }, h("legend", {}, "Options"), ...OPTION_LABELS.map(([key, label]) => check(options[key], (value) => (options = { ...options, [key]: value }), label))),
      ),
      check(/\.gz$/i.test(path) || (path === "" && gzip), (value) => ((gzip = value), (path = withGzip(path, value)), draw()), "Compress the file with gzip (.sql.gz); an import reads it directly"),
      h(
        "div",
        { class: "transfer-file" },
        h("input", { value: path, placeholder: "Where to save the .sql file", "aria-label": "File", spellcheck: false, onInput: (event) => (path = event.target.value) }),
        h("button", { type: "button", onClick: () => void choose() }, "Choose…"),
        h("button", { type: "button", class: "primary", onClick: () => void start() }, "Export"),
      ),
      error && h("p", { class: "error-text", role: "alert" }, error),
    ];
  }

  function drawProgress() {
    const running = progress.phase === "running";
    const rows = progress.tables.map((table) => {
      const total = table.estimatedRows;
      const text = table.done ? `${table.rows.toLocaleString()} rows` : `${table.rows.toLocaleString()}${total ? ` of about ${total.toLocaleString()}` : ""} rows…`;
      return h("div", { class: "transfer-row" }, h("span", {}, table.name), h("span", { class: table.done ? "muted" : "" }, text), !table.done && total ? h("progress", { max: 100, value: percent(table.rows, total) }) : null);
    });
    const summary = progress.summary;
    return [
      h("div", { class: "transfer-status" }, running ? `Writing ${progress.current ?? "…"} · ${formatBytes(progress.bytes)} so far` : statusLine(progress)),
      h("div", { class: "transfer-rows" }, ...rows, ...progress.notes.map((note) => h("div", { class: "transfer-row muted" }, note))),
      progress.error && h("p", { class: "error-text", role: "alert" }, progress.error),
      summary && !summary.cancelled && h("p", { class: "ok-text" }, `${summary.path} · ${formatBytes(summary.bytes)} · ${summary.rows.toLocaleString()} rows · ${formatDuration(summary.durationMs)}`),
      h("div", { class: "transfer-file" }, running ? h("button", { type: "button", class: "danger", onClick: () => void data.cancel(jobId) }, "Cancel") : h("button", { type: "button", onClick: () => ((progress = IDLE_EXPORT), draw()) }, "Export again")),
    ];
  }

  function statusLine(state) {
    return { done: "Finished.", cancelled: "Cancelled. Nothing was saved.", failed: "Failed.", idle: "" }[state.phase];
  }

  function draw(error = null) {
    body.replaceChildren(...(progress.phase === "idle" ? drawForm(error) : drawProgress()).filter(Boolean));
  }

  function scheduleDraw() {
    if (drawScheduled) return;
    drawScheduled = true;
    requestAnimationFrame(() => {
      drawScheduled = false;
      draw();
    });
  }

  async function choose() {
    const picked = await data.pickPath(defaultFileName(tab.database, new Date(), gzip || /\.gz$/i.test(path)));
    if (picked) {
      path = picked;
      draw();
    }
  }

  async function start() {
    const tables = allTables.filter((name) => selectedTables.has(name));
    const views = allViews.filter((name) => selectedViews.has(name));
    const problem = exportProblem({ path, tables, views, options });
    if (problem) return draw(problem);
    jobId = crypto.randomUUID();
    progress = { ...IDLE_EXPORT, phase: "running" };
    draw();
    try {
      const summary = await data.start(buildExportRequest({ jobId, database: tab.database, path, tables, views, options }), (event) => {
        progress = applyExportEvent(progress, event);
        scheduleDraw();
      });
      progress = finishExport(progress, summary);
    } catch (error) {
      progress = failExport(progress, error.message);
    }
    jobId = null;
    draw();
  }

  draw();

  return {
    element,
    hasUnsavedChanges: () => progress.phase === "running",
    destroy() {
      if (jobId !== null) void data.cancel(jobId);
    },
  };
}
