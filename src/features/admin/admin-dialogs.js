import { h } from "../../lib/dom.js";
import { dataTable } from "../../lib/data-table.js";
import { privilegesFor, scopeFromForm } from "./admin-state.js";

/** A small modal form. `submit()` resolves true to close it; a rejection is shown under the fields. */
export function formDialog({ title, fields, submitLabel, submit, className = "" }) {
  const message = h("p", { class: "form-message error", role: "alert" });
  const submitButton = h("button", { type: "submit", class: "primary" }, submitLabel);
  const dialog = h(
    "dialog",
    { class: `dialog ${className}` },
    h(
      "form",
      {
        onSubmit: async (event) => {
          event.preventDefault();
          message.textContent = "";
          submitButton.disabled = true;
          try {
            if (await submit()) dialog.close();
          } catch (error) {
            message.textContent = error.message;
          } finally {
            submitButton.disabled = false;
          }
        },
      },
      h("h2", {}, title),
      ...fields,
      message,
      h("div", { class: "dialog-actions" }, h("button", { type: "button", onClick: () => dialog.close() }, "Cancel"), submitButton),
    ),
  );
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  return dialog;
}

export const field = (label, control) => h("label", { class: "field", style: "margin-bottom:10px" }, h("span", {}, label), control);
export const input = (props = {}) => h("input", { spellcheck: false, autocomplete: "off", ...props });

export function openVariableDialog({ name, value, canPersist, submit }) {
  const valueInput = input({ value, "aria-label": name });
  const scope = h(
    "select",
    { "aria-label": "How long the change lasts" },
    h("option", { value: "global" }, "Until the server restarts"),
    canPersist && h("option", { value: "persist" }, "Keep after restart (SET PERSIST)"),
  );
  const dialog = formDialog({
    title: `Set ${name}`,
    fields: [h("p", { class: "muted" }, "Changes the global value for new connections."), field("Value", valueInput), field("Applies", scope)],
    submitLabel: "Review SQL…",
    submit: () => submit({ kind: "setVariable", name, value: valueInput.value, scope: scope.value }),
  });
  valueInput.focus();
  valueInput.select();
  return dialog;
}

/** `mode` is `create` or `password`. */
export function openUserDialog({ mode, user = "", host = "%", submit }) {
  const creating = mode === "create";
  const userInput = input({ value: user, readOnly: !creating, required: true, "aria-label": "User" });
  const hostInput = input({ value: host, readOnly: !creating, "aria-label": "Host" });
  const password = input({ type: "password", "aria-label": "Password" });
  const again = input({ type: "password", "aria-label": "Repeat password" });
  const dialog = formDialog({
    title: creating ? "New user" : `New password for ${user}@${host}`,
    fields: [
      creating && field("User", userInput),
      creating && field("Host (% means any)", hostInput),
      field("Password", password),
      field("Repeat password", again),
    ].filter(Boolean),
    submitLabel: "Review SQL…",
    submit: () => {
      if (password.value !== again.value) throw new Error("The two passwords are not the same.");
      return submit({ kind: creating ? "createUser" : "setPassword", user: userInput.value, host: hostInput.value || "%", password: password.value });
    },
  });
  (creating ? userInput : password).focus();
  return dialog;
}

/** Choose a scope and privileges to grant. `tablesOf(database)` fills the table suggestions. */
export function openPrivilegeDialog({ user, host, databases, tablesOf, submit }) {
  const kind = h("select", { "aria-label": "Scope" }, h("option", { value: "database" }, "One database"), h("option", { value: "table" }, "One table"), h("option", { value: "global" }, "Everything on the server"));
  const database = h("select", { "aria-label": "Database" }, ...databases.map((name) => h("option", { value: name }, name)));
  const table = input({ list: "lf-priv-tables", "aria-label": "Table", placeholder: "table name" });
  const tableNames = h("datalist", { id: "lf-priv-tables" });
  const grantOption = h("input", { type: "checkbox", "aria-label": "With grant option" });
  const boxes = h("div", { class: "privilege-grid" });
  const chosen = new Set();

  const drawPrivileges = () => {
    chosen.clear();
    boxes.replaceChildren(
      ...privilegesFor(kind.value).map((privilege) =>
        h("label", { class: "check" }, h("input", { type: "checkbox", onChange: (event) => (event.target.checked ? chosen.add(privilege) : chosen.delete(privilege)) }), privilege),
      ),
    );
  };
  const drawScope = () => {
    database.parentElement.hidden = kind.value === "global";
    table.parentElement.hidden = kind.value !== "table";
    tableNames.replaceChildren(...tablesOf(database.value).map((name) => h("option", { value: name })));
    drawPrivileges();
  };
  kind.addEventListener("change", drawScope);
  database.addEventListener("change", drawScope);

  const dialog = formDialog({
    title: `Grant privileges to ${user}@${host}`,
    className: "privilege-dialog",
    fields: [
      field("Where", kind),
      field("Database", database),
      field("Table", table),
      tableNames,
      h("div", { class: "field" }, h("span", {}, "Privileges"), boxes),
      h("label", { class: "check" }, grantOption, "Allow this user to pass these privileges on (WITH GRANT OPTION)"),
    ],
    submitLabel: "Review SQL…",
    submit: () =>
      submit({
        kind: "grant",
        user,
        host,
        scope: scopeFromForm({ kind: kind.value, database: database.value, table: table.value }),
        privileges: [...chosen],
        grantOption: grantOption.checked,
      }),
  });
  drawScope();
  return dialog;
}

const OPERATIONS = [
  { value: "analyze", label: "Analyze", help: "Refreshes the statistics the query planner uses." },
  { value: "optimize", label: "Optimize", help: "Rebuilds the table and its indexes and reclaims unused space. Locks the table for a while on big ones." },
  { value: "check", label: "Check", help: "Looks for errors in the table." },
  { value: "repair", label: "Repair", help: "Tries to fix a damaged table (MyISAM, Aria, Archive and CSV only)." },
  { value: "checksum", label: "Checksum", help: "A number that identifies the table's contents, to compare two copies." },
];

/** Run table maintenance on chosen tables of a database. `run(operation, tables)` resolves to the result rows. */
export function openMaintenanceDialog({ database, tables, selected, run }) {
  let operation = "analyze";
  const chosen = new Set(selected);
  const help = h("p", { class: "muted" }, OPERATIONS[0].help);
  const results = h("div", { class: "maintenance-results" });
  const message = h("p", { class: "form-message error", role: "alert" });
  const runButton = h("button", { type: "button", class: "primary", onClick: () => void go() }, "Run");
  const operationButtons = OPERATIONS.map((entry) =>
    h("label", { class: "check" }, h("input", { type: "radio", name: "operation", checked: entry.value === operation, onChange: () => ((operation = entry.value), (help.textContent = entry.help)) }), entry.label),
  );
  const tableBoxes = tables.map((name) => h("label", { class: "check" }, h("input", { type: "checkbox", checked: chosen.has(name), onChange: (event) => (event.target.checked ? chosen.add(name) : chosen.delete(name)) }), name));

  async function go() {
    message.textContent = "";
    if (chosen.size === 0) return void (message.textContent = "Choose at least one table.");
    runButton.disabled = true;
    try {
      const rows = await run(operation, tables.filter((name) => chosen.has(name)));
      results.replaceChildren(
        dataTable({
          columns: [
            { label: "Table", cell: (row) => row.table },
            { label: "Result", cell: (row) => h("span", { class: row.kind === "error" ? "error-text" : row.kind === "warning" ? "warn-text" : "" }, row.kind) },
            { label: "Message", cell: (row) => row.message },
          ],
          rows,
        }),
      );
    } catch (error) {
      message.textContent = error.message;
    } finally {
      runButton.disabled = false;
    }
  }

  const dialog = h(
    "dialog",
    { class: "dialog maintenance-dialog" },
    h("h2", {}, `Maintain tables of ${database}`),
    h("div", { class: "transfer-columns" }, h("fieldset", { class: "transfer-group" }, h("legend", {}, "Operation"), ...operationButtons, help), h("fieldset", { class: "transfer-group" }, h("legend", {}, `Tables (${tables.length})`), h("div", { class: "transfer-list" }, ...tableBoxes))),
    message,
    results,
    h("div", { class: "dialog-actions" }, h("button", { type: "button", onClick: () => dialog.close() }, "Close"), runButton),
  );
  dialog.addEventListener("close", () => dialog.remove());
  document.body.append(dialog);
  dialog.showModal();
  return dialog;
}
