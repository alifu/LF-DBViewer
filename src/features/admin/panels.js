import { dataTable, searchBox } from "../../lib/data-table.js";
import { confirmDialog, h } from "../../lib/dom.js";
import { formatBytes } from "../transfer/transfer-state.js";
import { computeRates, filterByName, formatSeconds, numbersOf, revokablePrivileges, summarizeStatus, visibleProcesses, describeScope } from "./admin-state.js";
import { openPrivilegeDialog, openUserDialog, openVariableDialog } from "./admin-dialogs.js";

const REFRESH_STATUS_MS = 5000;
const REFRESH_PROCESSES_MS = 3000;
const TOOLBAR = "admin-toolbar";

/** A panel is `{ element, load(), show(), hide() }`; `load` fetches, `show` and `hide` start and stop any timer. */
function panel(element, load, { show = () => {}, hide = () => {} } = {}) {
  return { element, load, show, hide };
}

const failure = (box, error) => box.replaceChildren(h("p", { class: "error-text", role: "alert" }, error.message));
const number = (value) => value.toLocaleString("en-US");

export function createDatabasesPanel({ data, onMaintain }) {
  const box = h("div", { class: "admin-box" });
  const element = h("div", {}, h("div", { class: TOOLBAR }, h("button", { type: "button", onClick: () => void load() }, "Refresh")), box);
  async function load() {
    try {
      const stats = await data.databases();
      const total = (key) => stats.reduce((sum, entry) => sum + entry[key], 0);
      box.replaceChildren(
        dataTable({
          columns: [
            { label: "Database", cell: (row) => row.name },
            { label: "Collation", cell: (row) => row.collation ?? "" },
            { label: "Tables", cell: (row) => number(row.tables), className: "num" },
            { label: "Rows (about)", cell: (row) => number(row.rows), className: "num" },
            { label: "Data", cell: (row) => formatBytes(row.dataBytes), className: "num" },
            { label: "Indexes", cell: (row) => formatBytes(row.indexBytes), className: "num" },
            { label: "Free", cell: (row) => formatBytes(row.freeBytes), className: "num" },
            { label: "", cell: (row) => (row.tables > 0 ? h("button", { type: "button", class: "ghost", onClick: () => onMaintain(row.name) }, "Maintain…") : "") },
          ],
          rows: stats,
        }),
        h("p", { class: "muted" }, `${stats.length} databases · ${formatBytes(total("dataBytes") + total("indexBytes"))} of data and indexes · row counts are estimates for InnoDB`),
      );
    } catch (error) {
      failure(box, error);
    }
  }
  return panel(element, load);
}

export function createVariablesPanel({ data, review }) {
  let variables = [];
  let filter = "";
  const box = h("div", { class: "admin-box" });
  const element = h("div", {}, h("div", { class: TOOLBAR }, searchBox("Filter variables", (text) => ((filter = text), draw())), h("button", { type: "button", onClick: () => void load() }, "Refresh")), box);
  function draw() {
    const shown = filterByName(variables, filter);
    box.replaceChildren(
      dataTable({
        columns: [
          { label: "Variable", cell: (row) => row.name },
          { label: "Value", cell: (row) => h("span", { class: "selectable" }, row.value) },
          { label: "", cell: (row) => h("button", { type: "button", class: "ghost", onClick: () => openVariableDialog({ name: row.name, value: row.value, canPersist: data.engine() === "mysql", submit: (change) => review(change, { title: `Set ${row.name}` }).then((applied) => applied && (void load(), true)) }) }, "Edit…") },
        ],
        rows: shown,
        empty: "No variable matches.",
      }),
      h("p", { class: "muted" }, `${shown.length} of ${variables.length} global variables`),
    );
  }
  async function load() {
    try {
      variables = await data.variables();
      draw();
    } catch (error) {
      failure(box, error);
    }
  }
  return panel(element, load);
}

export function createStatusPanel({ data }) {
  let previous = null;
  let rates = {};
  let entries = [];
  let filter = "";
  let timer = null;
  const tiles = h("div", { class: "status-tiles" });
  const box = h("div", { class: "admin-box" });
  const auto = h("input", { type: "checkbox", checked: true, onChange: () => schedule() });
  const element = h("div", {}, h("div", { class: TOOLBAR }, searchBox("Filter status", (text) => ((filter = text), draw())), h("label", { class: "check" }, auto, "Refresh every 5 s"), h("button", { type: "button", onClick: () => void load() }, "Refresh")), tiles, box);

  function draw() {
    const numbers = numbersOf(entries);
    tiles.replaceChildren(...summarizeStatus(numbers, rates).map((tile) => h("div", { class: "status-tile" }, h("div", { class: "muted" }, tile.label), h("strong", {}, tile.value), tile.detail && h("div", { class: "muted" }, tile.detail))));
    box.replaceChildren(
      dataTable({
        columns: [
          { label: "Status", cell: (row) => row.name },
          { label: "Value", cell: (row) => h("span", { class: "selectable" }, row.value) },
          { label: "Per second", cell: (row) => (rates[row.name] ? rates[row.name].toFixed(rates[row.name] >= 10 ? 0 : 2) : ""), className: "num" },
        ],
        rows: filterByName(entries, filter),
      }),
    );
  }
  async function load() {
    try {
      const now = performance.now();
      entries = await data.status();
      const numbers = numbersOf(entries);
      rates = previous === null ? {} : computeRates(previous.numbers, numbers, (now - previous.at) / 1000);
      previous = { numbers, at: now };
      draw();
    } catch (error) {
      failure(box, error);
    }
  }
  function schedule() {
    clearInterval(timer);
    timer = auto.checked ? setInterval(() => void load(), REFRESH_STATUS_MS) : null;
  }
  return panel(element, load, { show: schedule, hide: () => (clearInterval(timer), (timer = null)) });
}

export function createProcessesPanel({ data }) {
  let processes = [];
  let timer = null;
  const box = h("div", { class: "admin-box" });
  const hideSleeping = h("input", { type: "checkbox", checked: true, onChange: () => draw() });
  const auto = h("input", { type: "checkbox", checked: true, onChange: () => schedule() });
  const element = h("div", {}, h("div", { class: TOOLBAR }, h("label", { class: "check" }, hideSleeping, "Hide sleeping connections"), h("label", { class: "check" }, auto, "Refresh every 3 s"), h("button", { type: "button", onClick: () => void load() }, "Refresh")), box);

  async function stop(process, queryOnly) {
    const confirmed = await confirmDialog({
      title: queryOnly ? "Stop this query?" : "Close this connection?",
      body: queryOnly ? `The statement running on connection ${process.id} (${process.user}) is cancelled; the connection stays.` : `Connection ${process.id} (${process.user}@${process.host}) is closed. Whatever it was doing is rolled back.`,
      confirmLabel: queryOnly ? "Stop query" : "Close connection",
      danger: true,
    });
    if (!confirmed) return;
    try {
      await data.kill(process.id, queryOnly);
    } catch (error) {
      failure(box, error);
      return;
    }
    await load();
  }

  function draw() {
    const shown = visibleProcesses(processes, { hideSleeping: hideSleeping.checked });
    box.replaceChildren(
      dataTable({
        columns: [
          { label: "Id", cell: (row) => String(row.id), className: "num" },
          { label: "User", cell: (row) => row.user },
          { label: "Host", cell: (row) => row.host },
          { label: "Database", cell: (row) => row.database ?? "" },
          { label: "Command", cell: (row) => row.command },
          { label: "Time", cell: (row) => formatSeconds(row.seconds), className: "num" },
          { label: "State", cell: (row) => row.state ?? "" },
          { label: "Statement", cell: (row) => h("code", { class: "process-sql selectable", title: row.info ?? "" }, row.info ?? ""), className: "wide" },
          { label: "", cell: (row) => h("span", { class: "cell-actions" }, row.command === "Query" && h("button", { type: "button", class: "ghost", onClick: () => void stop(row, true) }, "Stop query"), h("button", { type: "button", class: "ghost danger", onClick: () => void stop(row, false) }, "Close")) },
        ],
        rows: shown,
        empty: "No connections to show.",
      }),
      h("p", { class: "muted" }, `${shown.length} shown of ${processes.length} connections`),
    );
  }
  async function load() {
    try {
      processes = await data.processes();
      draw();
    } catch (error) {
      failure(box, error);
    }
  }
  function schedule() {
    clearInterval(timer);
    timer = auto.checked ? setInterval(() => void load(), REFRESH_PROCESSES_MS) : null;
  }
  return panel(element, load, { show: schedule, hide: () => (clearInterval(timer), (timer = null)) });
}

export function createEnginesPanel({ data }) {
  const box = h("div", { class: "admin-box" });
  const element = h("div", {}, h("div", { class: TOOLBAR }, h("button", { type: "button", onClick: () => void load() }, "Refresh")), box);
  async function load() {
    try {
      const [engines, charsets] = await Promise.all([data.engines(), data.charsets()]);
      box.replaceChildren(
        h("h3", {}, "Storage engines"),
        dataTable({
          columns: [
            { label: "Engine", cell: (row) => row.name },
            { label: "Support", cell: (row) => row.support },
            { label: "Transactions", cell: (row) => row.transactions ?? "" },
            { label: "XA", cell: (row) => row.xa ?? "" },
            { label: "Savepoints", cell: (row) => row.savepoints ?? "" },
            { label: "Comment", cell: (row) => row.comment, className: "wide" },
          ],
          rows: engines,
        }),
        h("h3", {}, "Character sets"),
        dataTable({
          columns: [
            { label: "Character set", cell: (row) => row.name },
            { label: "Description", cell: (row) => row.description },
            { label: "Default collation", cell: (row) => row.defaultCollation },
            { label: "Bytes per character", cell: (row) => String(row.maxLength), className: "num" },
          ],
          rows: charsets,
        }),
      );
    } catch (error) {
      failure(box, error);
    }
  }
  return panel(element, load);
}

export function createUsersPanel({ data, review }) {
  let users = [];
  let selected = null;
  const list = h("div", { class: "user-list" });
  const detail = h("div", { class: "user-detail" });
  const element = h("div", { class: "users-layout" }, h("div", {}, h("div", { class: TOOLBAR }, h("button", { type: "button", onClick: () => openUserDialog({ mode: "create", submit: (change) => review(change, { title: "Create user" }).then((applied) => applied && (void load(), true)) }) }, "New user…"), h("button", { type: "button", onClick: () => void load() }, "Refresh")), list), detail);

  function drawList() {
    list.replaceChildren(
      ...(users.length === 0 ? [h("p", { class: "muted empty" }, "No accounts.")] : []),
      ...users.map((account) => {
        const isSelected = selected !== null && selected.user === account.user && selected.host === account.host;
        return h("div", { class: isSelected ? "user-row selected" : "user-row", onClick: () => void select(account) }, h("strong", {}, account.user === "" ? "(anonymous)" : account.user), h("span", { class: "muted" }, `@${account.host}`));
      }),
    );
  }

  async function select(account) {
    selected = account;
    drawList();
    detail.replaceChildren(h("p", { class: "muted" }, "Loading…"));
    try {
      const grants = await data.grants(account.user, account.host);
      drawDetail(grants);
    } catch (error) {
      failure(detail, error);
    }
  }

  const changed = async (change, title, extra = {}) => (await review(change, { title, ...extra })) && (void select(selected), true);

  function drawDetail(grants) {
    const { user, host } = selected;
    const rows = grants.map((entry) => {
      if (entry.kind === "raw") return h("div", { class: "grant-row" }, h("code", { class: "selectable" }, entry.text));
      const revoke = revokablePrivileges(entry);
      return h(
        "div",
        { class: "grant-row" },
        h("div", {}, h("strong", {}, describeScope(entry.scope)), h("div", { class: "selectable" }, entry.privileges.join(", ") + (entry.grantOption ? " · with grant option" : ""))),
        revoke.length > 0 && h("button", { type: "button", class: "ghost danger", onClick: () => void changed({ kind: "revoke", user, host, scope: entry.scope, privileges: revoke }, `Revoke from ${user}@${host}`, { confirmLabel: "Revoke", danger: true }) }, "Revoke…"),
      );
    });
    detail.replaceChildren(
      h("h3", {}, `${user === "" ? "(anonymous)" : user}@${host}`),
      h("div", { class: TOOLBAR },
        h("button", { type: "button", onClick: () => openPrivilegeDialog({ user, host, databases: data.databaseNames(), tablesOf: data.tableNames, submit: (change) => changed(change, `Grant to ${user}@${host}`) }) }, "Grant privileges…"),
        h("button", { type: "button", onClick: () => openUserDialog({ mode: "password", user, host, submit: (change) => changed(change, `New password for ${user}@${host}`) }) }, "Change password…"),
        h("button", { type: "button", class: "danger", onClick: () => void review({ kind: "dropUser", user, host }, { title: `Drop user ${user}@${host}`, confirmLabel: "Drop user", danger: true, note: "The account can no longer log in." }).then((dropped) => dropped && ((selected = null), detail.replaceChildren(), load())) }, "Drop user…")),
      h("h4", {}, "Privileges"),
      ...(rows.length > 0 ? rows : [h("p", { class: "muted" }, "None.")]),
      h("p", { class: "muted" }, `Plugin: ${selected.plugin ?? "unknown"}`),
    );
  }

  async function load() {
    try {
      users = await data.users();
      drawList();
    } catch (error) {
      failure(list, error);
    }
  }
  return panel(element, load);
}

