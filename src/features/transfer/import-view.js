import { h } from "../../lib/dom.js";
import { IDLE_IMPORT, applyImportEvent, buildImportRequest, failImport, finishImport, formatBytes, formatDuration, importProblem, percent } from "./transfer-state.js";

const AS_IN_FILE = "";
const check = (checked, onChange, label) => h("label", { class: "check" }, h("input", { type: "checkbox", checked, onChange: (event) => onChange(event.target.checked) }), label);

/**
 * Import tab: choose a .sql file and a target, then watch it run. `data`: `databases()`, `pickFile()`,
 * `start(request, onEvent)` and `cancel(jobId)`.
 */
export function createImportView({ tab, data }) {
  let path = "";
  let target = tab.database ?? AS_IN_FILE;
  let createDatabase = true;
  let stopOnError = true;
  let disableForeignKeyChecks = true;
  let progress = IDLE_IMPORT;
  let jobId = null;
  let drawScheduled = false;

  const body = h("div", { class: "transfer-body" });
  const element = h("div", { class: "transfer" }, h("h2", {}, "Import a SQL file"), body);

  function drawForm(error) {
    const names = data.databases();
    const targets = target !== AS_IN_FILE && !names.includes(target) ? [target, ...names] : names;
    return [
      h(
        "div",
        { class: "transfer-file" },
        h("input", { value: path, placeholder: "The .sql file to import", "aria-label": "File", spellcheck: false, onInput: (event) => (path = event.target.value) }),
        h("button", { type: "button", onClick: () => void choose() }, "Choose…"),
      ),
      h(
        "fieldset",
        { class: "transfer-group" },
        h("legend", {}, "Options"),
        h("label", { class: "field" }, h("span", {}, "Import into"), h("select", { "aria-label": "Target database", onChange: (event) => ((target = event.target.value), draw()) }, h("option", { value: AS_IN_FILE, selected: target === AS_IN_FILE }, "(as the file says: CREATE DATABASE / USE inside it)"), ...targets.map((name) => h("option", { value: name, selected: name === target }, name)))),
        target !== AS_IN_FILE && check(createDatabase, (value) => (createDatabase = value), "Create the database if it does not exist"),
        check(stopOnError, (value) => (stopOnError = value), "Stop at the first error (otherwise carry on and list every error)"),
        check(disableForeignKeyChecks, (value) => (disableForeignKeyChecks = value), "Turn off foreign key checks while loading"),
      ),
      h("p", { class: "muted" }, "Statements already in the file, such as DROP TABLE, run as written. Import into a database you can restore."),
      h("div", { class: "transfer-file" }, h("button", { type: "button", class: "primary", onClick: () => void start() }, "Import")),
      error && h("p", { class: "error-text", role: "alert" }, error),
    ];
  }

  function drawProgress() {
    const running = progress.phase === "running";
    const failures = progress.errors.map((failure) =>
      h("div", { class: "import-error" }, h("strong", {}, `Statement ${failure.statement}, line ${failure.line}`), h("div", { class: "error-text" }, failure.message), h("pre", {}, failure.sql)),
    );
    const summary = progress.summary;
    return [
      h("div", { class: "transfer-status" }, running ? `${formatBytes(progress.bytesRead)} of ${formatBytes(progress.totalBytes)} · ${progress.statements.toLocaleString()} statements · line ${progress.line.toLocaleString()}` : statusLine(progress)),
      running && h("progress", { class: "wide", max: 100, value: percent(progress.bytesRead, progress.totalBytes) }),
      progress.error && h("p", { class: "error-text", role: "alert" }, progress.error),
      summary && h("p", { class: summary.failed ? "error-text" : "ok-text" }, `${summary.statements.toLocaleString()} statements · ${formatBytes(summary.bytes)} · ${formatDuration(summary.durationMs)}${summary.errors.length > 0 ? ` · ${summary.errors.length} error${summary.errors.length === 1 ? "" : "s"}` : ""}`),
      h("div", { class: "import-errors" }, ...failures),
      h("div", { class: "transfer-file" }, running ? h("button", { type: "button", class: "danger", onClick: () => void data.cancel(jobId) }, "Cancel") : h("button", { type: "button", onClick: () => ((progress = IDLE_IMPORT), draw()) }, "Import another")),
    ];
  }

  function statusLine(state) {
    return { done: "Finished.", cancelled: "Cancelled. Statements that already ran stay in the database.", failed: "Stopped with errors.", idle: "" }[state.phase];
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
    const picked = await data.pickFile();
    if (typeof picked === "string" && picked !== "") {
      path = picked;
      draw();
    }
  }

  async function start() {
    const problem = importProblem({ path });
    if (problem) return draw(problem);
    jobId = crypto.randomUUID();
    progress = { ...IDLE_IMPORT, phase: "running" };
    draw();
    try {
      const summary = await data.start(buildImportRequest({ jobId, path, database: target, createDatabase, stopOnError, disableForeignKeyChecks }), (event) => {
        progress = applyImportEvent(progress, event);
        scheduleDraw();
      });
      progress = finishImport(progress, summary);
    } catch (error) {
      progress = failImport(progress, error.message);
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
