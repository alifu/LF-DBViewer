import { h } from "../../lib/dom.js";
import { formatBytes, formatDuration } from "../transfer/transfer-state.js";

const FORMATS = Object.freeze([
  { value: "csv", label: "CSV (opens in Excel and Numbers)" },
  { value: "json", label: "JSON (an array of objects)" },
]);
const DELIMITERS = Object.freeze([
  { value: ",", label: "Comma" },
  { value: ";", label: "Semicolon" },
  { value: "\\t", label: "Tab" },
]);

/**
 * Writes everything a statement returns to a file. The statement is run again on a connection of its own, so the
 * file has every row in full, not just what the grid kept. `data`: `pickPath(name, extension)`, `start(request, onEvent)`, `cancel(jobId)`.
 */
export function openResultExportDialog({ sql, database, data }) {
  let format = "csv";
  let delimiter = ",";
  let header = true;
  let nullText = "";
  let byteOrderMark = false;
  let path = "";
  let jobId = null;
  let running = false;

  const message = h("p", { class: "form-message", role: "status" });
  const fields = h("div", {});
  const file = h("input", { value: "", placeholder: "Where to save the file", "aria-label": "File", spellcheck: false, onInput: (event) => (path = event.target.value) });
  const exportButton = h("button", { type: "button", class: "primary", onClick: () => void run() }, "Export");
  const closeButton = h("button", { type: "button", onClick: () => dialog.close() }, "Close");
  const cancelButton = h("button", { type: "button", class: "danger", hidden: true, onClick: () => void data.cancel(jobId) }, "Cancel export");
  const check = (checked, onChange, label) => h("label", { class: "check" }, h("input", { type: "checkbox", checked, onChange: (event) => onChange(event.target.checked) }), label);
  const select = (label, options, value, onChange) => h("label", { class: "field" }, h("span", {}, label), h("select", { "aria-label": label, onChange: (event) => onChange(event.target.value) }, ...options.map((option) => h("option", { value: option.value, selected: option.value === value }, option.label))));

  function drawFields() {
    fields.replaceChildren(
      select("Format", FORMATS, format, (value) => ((format = value), (path = path.replace(/\.(csv|json)$/i, `.${value}`)), (file.value = path), drawFields())),
      ...(format === "csv"
        ? [
            select("Separator", DELIMITERS, delimiter, (value) => (delimiter = value)),
            check(header, (value) => (header = value), "First row holds the column names"),
            h("label", { class: "field" }, h("span", {}, "Write NULL as (leave empty for nothing)"), h("input", { value: nullText, "aria-label": "NULL text", placeholder: "NULL or \\N", onInput: (event) => (nullText = event.target.value) })),
            check(byteOrderMark, (value) => (byteOrderMark = value), "Mark the file as UTF-8 (Excel needs this for accents and symbols)"),
          ]
        : []),
    );
  }

  async function choose() {
    const picked = await data.pickPath(`result.${format}`, format);
    if (picked) {
      path = picked;
      file.value = picked;
    }
  }

  async function run() {
    if (path.trim() === "") return void (message.textContent = "Choose where to save the file.");
    running = true;
    jobId = crypto.randomUUID();
    exportButton.disabled = true;
    cancelButton.hidden = false;
    message.className = "form-message";
    message.textContent = "Exporting…";
    const request = { jobId, sql, database: database || null, path, format, delimiter, header, nullText, byteOrderMark };
    try {
      const summary = await data.start(request, (event) => {
        if (event.event === "progress") message.textContent = `${event.rows.toLocaleString()} rows · ${formatBytes(event.bytes)}`;
      });
      message.className = summary.cancelled ? "form-message" : "form-message ok";
      message.textContent = summary.cancelled ? "Cancelled. No file was saved." : `${summary.rows.toLocaleString()} rows · ${formatBytes(summary.bytes)} · ${formatDuration(summary.durationMs)} → ${summary.path}`;
    } catch (error) {
      message.className = "form-message error";
      message.textContent = error.message;
    } finally {
      running = false;
      jobId = null;
      exportButton.disabled = false;
      cancelButton.hidden = true;
    }
  }

  const shown = sql.length > 300 ? `${sql.slice(0, 300)}…` : sql;
  const dialog = h(
    "dialog",
    { class: "dialog result-export-dialog" },
    h("h2", {}, "Export result"),
    h("p", { class: "muted" }, "The statement below is run again, and every row it returns is written to the file."),
    h("pre", { class: "review-sql" }, shown),
    fields,
    h("div", { class: "transfer-file" }, file, h("button", { type: "button", onClick: () => void choose() }, "Choose…")),
    message,
    h("div", { class: "dialog-actions" }, cancelButton, closeButton, exportButton),
  );
  dialog.addEventListener("close", () => {
    if (running && jobId !== null) void data.cancel(jobId);
    dialog.remove();
  });
  drawFields();
  document.body.append(dialog);
  dialog.showModal();
  return dialog;
}
