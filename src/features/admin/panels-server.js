import { dataTable } from "../../lib/data-table.js";
import { h } from "../../lib/dom.js";
import { formatBytes } from "../transfer/transfer-state.js";
import { formDialog } from "./admin-dialogs.js";
import { replicationSummary } from "./admin-state.js";

const TOOLBAR = "admin-toolbar";
const BINLOG_EVENT_LIMIT = 200;

const failure = (box, error) => box.replaceChildren(h("p", { class: "error-text", role: "alert" }, error.message));
const pairs = (entries) => dataTable({ columns: [{ label: "Name", cell: (row) => row.name }, { label: "Value", cell: (row) => h("span", { class: "selectable" }, row.value) }], rows: entries, empty: "Nothing reported." });

function simplePanel(toolbarExtra, draw) {
  const box = h("div", { class: "admin-box" });
  const element = h("div", {}, h("div", { class: TOOLBAR }, ...toolbarExtra, h("button", { type: "button", onClick: () => void load() }, "Refresh")), box);
  async function load() {
    try {
      box.replaceChildren(...(await draw()));
    } catch (error) {
      failure(box, error);
    }
  }
  return { element, load, show: () => {}, hide: () => {} };
}

export function createReplicationPanel({ data }) {
  return simplePanel([], async () => {
    const info = await data.replication();
    return [
      h("p", {}, h("strong", {}, replicationSummary(info))),
      h("h3", {}, "As a replica"),
      pairs(info.replica),
      h("h3", {}, "As a source (this server's binary log)"),
      pairs(info.source),
    ];
  });
}

export function createPluginsPanel({ data }) {
  return simplePanel([], async () => {
    const plugins = await data.plugins();
    return [
      dataTable({
        columns: [
          { label: "Plugin", cell: (row) => row.name },
          { label: "Status", cell: (row) => row.status },
          { label: "Type", cell: (row) => row.kind },
          { label: "Library", cell: (row) => row.library ?? "built in" },
          { label: "Description", cell: (row) => row.description ?? "", className: "wide" },
        ],
        rows: plugins,
      }),
      h("p", { class: "muted" }, `${plugins.length} plugins`),
    ];
  });
}

function openEventsDialog(file, events) {
  const dialog = h(
    "dialog",
    { class: "dialog wide" },
    h("h2", {}, file),
    dataTable({
      columns: [
        { label: "Position", cell: (row) => String(row.position), className: "num" },
        { label: "Event", cell: (row) => row.kind },
        { label: "Server", cell: (row) => String(row.serverId), className: "num" },
        { label: "End", cell: (row) => String(row.endPosition), className: "num" },
        { label: "Info", cell: (row) => h("code", { class: "process-sql selectable" }, row.info), className: "wide" },
      ],
      rows: events,
    }),
    h("p", { class: "muted" }, `The first ${events.length} events (at most ${BINLOG_EVENT_LIMIT}).`),
    h("div", { class: "dialog-actions" }, h("button", { type: "button", onClick: () => dialog.close() }, "Close")),
  );
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
}

export function createBinaryLogsPanel({ data }) {
  const panel = simplePanel([], async () => {
    const logs = await data.binaryLogs();
    return [
      dataTable({
        columns: [
          { label: "Log file", cell: (row) => row.name },
          { label: "Size", cell: (row) => formatBytes(row.bytes), className: "num" },
          { label: "", cell: (row) => h("button", { type: "button", class: "ghost", onClick: () => void showEvents(row.name, panel) }, "Events…") },
        ],
        rows: logs,
        empty: "Binary logging is off, or this account may not read it.",
      }),
      h("p", { class: "muted" }, `${logs.length} log files · ${formatBytes(logs.reduce((sum, log) => sum + log.bytes, 0))}`),
    ];
  });
  async function showEvents(file, owner) {
    try {
      openEventsDialog(file, await data.binlogEvents(file, BINLOG_EVENT_LIMIT));
    } catch (error) {
      owner.element.append(h("p", { class: "error-text", role: "alert" }, error.message));
    }
  }
  return panel;
}

/** The tables of one database, largest first. */
export async function openTableStatsDialog({ database, load }) {
  const rows = await load();
  const dialog = formDialog({
    title: `Tables of ${database}`,
    className: "wide",
    submitLabel: "Close",
    submit: async () => true,
    fields: [
      dataTable({
        columns: [
          { label: "Table", cell: (row) => row.name },
          { label: "Engine", cell: (row) => row.engine ?? "" },
          { label: "Rows (about)", cell: (row) => row.rows.toLocaleString("en-US"), className: "num" },
          { label: "Data", cell: (row) => formatBytes(row.dataBytes), className: "num" },
          { label: "Indexes", cell: (row) => formatBytes(row.indexBytes), className: "num" },
          { label: "Free", cell: (row) => formatBytes(row.freeBytes), className: "num" },
        ],
        rows,
      }),
    ],
  });
  return dialog;
}
