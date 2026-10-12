import { h } from "../../lib/dom.js";
import { DELIMITERS, IDLE_CSV, applyCsvEvent, autoMapping, buildCsvRequest, csvProblem, duplicateTargets, failCsv, finishCsv } from "./csv-state.js";
import { formatBytes, formatDuration, percent } from "./transfer-state.js";

const SAMPLE_VALUES = 3;
const check = (checked, onChange, label) => h("label", { class: "check" }, h("input", { type: "checkbox", checked, onChange: (event) => onChange(event.target.checked) }), label);

/**
 * Import tab for one table: choose a CSV file, see its first rows, say which CSV column goes into which table column,
 * then load it in one transaction (a refused row cancels the whole import and is named).
 * `data`: `tableColumns()`, `pickFile()`, `preview(path, delimiter, hasHeader)`, `start(request, onEvent)`, `cancel(jobId)`.
 */
export function createCsvImportView({ tab, data }) {
  let path = "";
  let delimiter = "auto";
  let hasHeader = true;
  let nullText = "";
  let emptyAsNull = true;
  let truncateFirst = false;
  let tableColumns = [];
  let preview = null;
  let mapping = [];
  let progress = IDLE_CSV;
  let jobId = null;
  let drawScheduled = false;
  let notice = null;

  const body = h("div", { class: "transfer-body" });
  const element = h("div", { class: "transfer" }, h("h2", {}, `Import CSV into ${tab.database}.${tab.table}`), body);

  function mappingTable() {
    if (preview === null) return null;
    if (preview.headers.length === 0) return h("p", { class: "muted" }, "The file is empty.");
    const rows = preview.headers.map((header, index) =>
      h(
        "tr",
        {},
        h("td", {}, h("strong", {}, header)),
        h("td", { class: "muted csv-sample" }, preview.rows.slice(0, SAMPLE_VALUES).map((row) => row[index] ?? "").join("  ·  ")),
        h(
          "td",
          {},
          h(
            "select",
            { "aria-label": `Table column for ${header}`, onChange: (event) => ((mapping = mapping.map((value, at) => (at === index ? event.target.value || null : value))), draw()) },
            h("option", { value: "" }, "(skip this column)"),
            ...tableColumns.map((name) => h("option", { value: name, selected: name === mapping[index] }, name)),
          ),
        ),
      ),
    );
    const repeated = duplicateTargets(mapping);
    return h(
      "div",
      {},
      h("p", { class: "muted" }, `Delimiter: ${preview.delimiter === "\\t" ? "tab" : preview.delimiter} · ${formatBytes(preview.size)}`),
      h("table", { class: "designer-table" }, h("thead", {}, h("tr", {}, ...["CSV column", "First values", "Goes into"].map((label) => h("th", {}, label)))), h("tbody", {}, ...rows)),
      repeated.length > 0 && h("p", { class: "error-text" }, `${repeated.join(", ")} is chosen more than once.`),
    );
  }

  function drawForm() {
    return [
      h(
        "div",
        { class: "transfer-file" },
        h("input", { value: path, placeholder: "The .csv file", "aria-label": "File", spellcheck: false, onChange: (event) => ((path = event.target.value), void loadPreview()) }),
        h("button", { type: "button", onClick: () => void choose() }, "Choose…"),
      ),
      h(
        "fieldset",
        { class: "transfer-group" },
        h("legend", {}, "Options"),
        h("label", { class: "field" }, h("span", {}, "Delimiter"), h("select", { "aria-label": "Delimiter", onChange: (event) => ((delimiter = event.target.value), void loadPreview()) }, ...DELIMITERS.map((entry) => h("option", { value: entry.value, selected: entry.value === delimiter }, entry.label)))),
        check(hasHeader, (value) => ((hasHeader = value), void loadPreview()), "The first row holds the column names"),
        h("label", { class: "field" }, h("span", {}, "Text that means NULL (optional)"), h("input", { value: nullText, placeholder: "NULL or \\N", "aria-label": "NULL text", onInput: (event) => (nullText = event.target.value) })),
        check(emptyAsNull, (value) => (emptyAsNull = value), "An empty field is NULL"),
        check(truncateFirst, (value) => (truncateFirst = value), "Empty the table first (TRUNCATE; this cannot be undone)"),
      ),
      mappingTable(),
      h("div", { class: "transfer-file" }, h("button", { type: "button", class: "primary", onClick: () => void start() }, "Import")),
      notice && h("p", { class: "error-text", role: "alert" }, notice),
    ];
  }

  function drawProgress() {
    const running = progress.phase === "running";
    const status = { done: "Finished.", cancelled: "Cancelled. Nothing was imported.", failed: "Nothing was imported.", running: `${progress.rows.toLocaleString()} rows · ${percent(progress.bytesRead, progress.totalBytes)}%` }[progress.phase];
    return [
      h("div", { class: "transfer-status" }, status),
      running && h("progress", { class: "wide", max: 100, value: percent(progress.bytesRead, progress.totalBytes) }),
      progress.error && h("p", { class: "error-text", role: "alert" }, progress.error),
      progress.summary && !progress.summary.cancelled && h("p", { class: "ok-text" }, `${progress.summary.rows.toLocaleString()} rows loaded in ${formatDuration(progress.summary.durationMs)}`),
      h("div", { class: "transfer-file" }, running ? h("button", { type: "button", class: "danger", onClick: () => void data.cancel(jobId) }, "Cancel") : h("button", { type: "button", onClick: () => ((progress = IDLE_CSV), draw()) }, "Import another file")),
    ];
  }

  function draw() {
    body.replaceChildren(...(progress.phase === "idle" ? drawForm() : drawProgress()).filter(Boolean));
  }

  function scheduleDraw() {
    if (drawScheduled) return;
    drawScheduled = true;
    requestAnimationFrame(() => {
      drawScheduled = false;
      draw();
    });
  }

  async function loadPreview() {
    notice = null;
    if (path.trim() === "") return draw();
    try {
      preview = await data.preview(path, delimiter, hasHeader);
      mapping = autoMapping(preview.headers, tableColumns);
    } catch (error) {
      preview = null;
      notice = error.message;
    }
    draw();
  }

  async function choose() {
    const picked = await data.pickFile();
    if (typeof picked === "string" && picked !== "") {
      path = picked;
      await loadPreview();
    }
  }

  async function start() {
    const problem = csvProblem({ path, mapping }) ?? (duplicateTargets(mapping).length > 0 ? "A table column is chosen more than once." : null);
    if (problem) {
      notice = problem;
      return draw();
    }
    if (truncateFirst && !window.confirm(`Empty ${tab.table} first? Its rows are deleted and cannot be brought back.`)) return;
    jobId = crypto.randomUUID();
    progress = { ...IDLE_CSV, phase: "running" };
    draw();
    const request = buildCsvRequest({ jobId, path, database: tab.database, table: tab.table, delimiter: preview?.delimiter ?? delimiter, hasHeader, mapping, nullText, emptyAsNull, truncateFirst });
    try {
      const summary = await data.start(request, (event) => {
        progress = applyCsvEvent(progress, event);
        scheduleDraw();
      });
      progress = finishCsv(progress, summary);
    } catch (error) {
      progress = failCsv(progress, error.message);
    }
    jobId = null;
    draw();
  }

  data.tableColumns().then(
    (names) => {
      tableColumns = names;
      draw();
    },
    (error) => {
      notice = error.message;
      draw();
    },
  );
  draw();

  return {
    element,
    hasUnsavedChanges: () => progress.phase === "running",
    destroy() {
      if (jobId !== null) void data.cancel(jobId);
    },
  };
}
