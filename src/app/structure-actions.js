import { openDatabaseDialog } from "../features/structure/database-dialog.js";
import { showMenu } from "../features/structure/menu.js";
import { reviewSql } from "../features/structure/review-dialog.js";
import { buildCallSql } from "../features/structure/routine-call.js";
import { openRunDialog } from "../features/structure/run-dialog.js";
import { field, formDialog, input } from "../features/admin/admin-dialogs.js";
import { alertDialog, h } from "../lib/dom.js";
import * as transitions from "../state/app-state.js";

const OBJECT_LABELS = { table: "table", view: "view", procedure: "procedure", function: "function", trigger: "trigger", event: "event" };
/** Tabs that show an object, by the kind of object, so a dropped object takes its tabs with it. */
const TABS_OF_KIND = { table: ["table", "structure"], view: ["view", "viewdef"] };
const NEW_GROUP_KINDS = { tables: "table", views: "view", procedures: "procedure", functions: "function", triggers: "trigger", events: "event" };

/** Everything that changes a database's structure: dialogs, previews, confirmations and the follow-up refresh. */
export function createStructureActions({ api, store, schema, admin }) {
  const serverOptions = (connectionId) => api("server_options", { connectionId }, { quiet: true });
  const sqlLabel = (result) => result.statements.join(";\n");

  async function refreshTree(connectionId) {
    await schema.loadDatabases(connectionId).catch(() => {});
  }

  /** Previews the change, shows the SQL for confirmation, applies it. Resolves true when applied. */
  async function runChange(connectionId, change, review = {}) {
    const statements = await api("preview_structure_change", { connectionId, change }, { quiet: true });
    if (statements.length === 0) {
      await alertDialog({ title: "Nothing to change", body: "The definition matches what is already in the database." });
      return false;
    }
    if (!(await reviewSql({ statements, confirmLabel: "Run", ...review }))) return false;
    await api("apply_structure_change", { connectionId, change }, { label: sqlLabel });
    await refreshTree(connectionId);
    return true;
  }

  async function runDefinition(connectionId, request, review = {}) {
    const statements = await api("preview_definition", { request }, { quiet: true });
    if (!(await reviewSql({ statements, confirmLabel: "Save", ...review }))) return false;
    await api("apply_definition", { connectionId, request }, { label: sqlLabel });
    await refreshTree(connectionId);
    return true;
  }

  /** Errors from a menu action go to a dialog; the log panel has them too. */
  const guarded = (title, work) => async () => {
    try {
      await work();
    } catch (error) {
      await alertDialog({ title, body: error.message });
    }
  };

  const closeObjectTabs = (connectionId, database, kind, name) =>
    store.update((state) =>
      transitions.closeTabsWhere(
        state,
        (tab) => tab.connectionId === connectionId && tab.database === database && tab.name === name && (TABS_OF_KIND[kind] ?? [kind]).includes(tab.kind),
      ),
    );

  const databaseInfo = (connectionId, database) => transitions.schemaOf(store.getState(), connectionId).databases.find((entry) => entry.name === database);

  // ---- databases ----------------------------------------------------------------

  async function newDatabase(connectionId) {
    openDatabaseDialog({
      mode: "create",
      options: await serverOptions(connectionId),
      submit: (change) => runChange(connectionId, change, { title: "Create database", confirmLabel: "Create" }),
    });
  }

  async function alterDatabase(connectionId, database) {
    const info = databaseInfo(connectionId, database);
    openDatabaseDialog({
      mode: "alter",
      name: database,
      charset: info?.charset ?? null,
      collation: info?.collation ?? null,
      options: await serverOptions(connectionId),
      submit: (change) => runChange(connectionId, change, { title: `Alter database ${database}`, confirmLabel: "Alter" }),
    });
  }

  async function dropDatabase(connectionId, database) {
    const dropped = await runChange(connectionId, { kind: "dropDatabase", name: database }, {
      title: `Drop database ${database}`,
      note: "This deletes the database with all its tables and data. It cannot be undone.",
      confirmLabel: "Drop database",
      danger: true,
      confirmText: database,
    });
    if (dropped) store.update((state) => transitions.closeTabsWhere(state, (tab) => tab.connectionId === connectionId && tab.database === database));
  }

  // ---- objects ----------------------------------------------------------------------

  async function dropObject(connectionId, database, kind, name) {
    const dropped = await runChange(connectionId, { kind: "dropObject", database, object: kind, name }, {
      title: `Drop ${OBJECT_LABELS[kind]} ${name}`,
      note: kind === "table" ? "This deletes the table and all its rows. It cannot be undone." : "",
      confirmLabel: `Drop ${OBJECT_LABELS[kind]}`,
      danger: true,
    });
    if (dropped) closeObjectTabs(connectionId, database, kind, name);
  }

  const truncateTable = (connectionId, database, name) =>
    runChange(connectionId, { kind: "truncateTable", database, name }, {
      title: `Truncate table ${name}`,
      note: "This deletes every row and resets AUTO_INCREMENT. It cannot be undone.",
      confirmLabel: "Truncate",
      danger: true,
    });

  async function runRoutine(connectionId, database, name, kind) {
    const parameters = await api("routine_parameters", { connectionId, database, name, kind }, { label: `Read parameters of ${name}` });
    openRunDialog({
      kind,
      name,
      parameters,
      run: (values) => schema.newQuery(connectionId, database, { initialSql: buildCallSql({ kind, database, name, parameters, values }), autorun: true }),
    });
  }

  /** Replaces the tab of a new or renamed object by the tab of the saved one. */
  function reopen(tab, kind, name) {
    store.update((state) => transitions.closeTab(state, tab.id));
    schema.openObject({ connectionId: tab.connectionId, database: tab.database, name, kind });
  }

  // ---- data for tab views --------------------------------------------------------------

  function designerData(tab) {
    const { connectionId, database } = tab;
    const objects = () => transitions.schemaOf(store.getState(), connectionId).objects[database];
    return {
      load: () => api("table_definition", { connectionId, database, table: tab.name }, { label: `Read structure of ${tab.name}` }),
      serverOptions: () => serverOptions(connectionId),
      tableNames: async () => (objects()?.tables ?? []).map((table) => table.name),
      review: (change) => runChange(connectionId, { ...change, database }, { title: tab.isNew ? "Create table" : `Review changes to ${tab.name}`, confirmLabel: tab.isNew ? "Create" : "Apply" }),
      onCreated: (name) => reopen(tab, "structure", name),
      onRenamed: (name) => reopen(tab, "structure", name),
    };
  }

  function objectEditorData(tab) {
    const { connectionId, database } = tab;
    const kind = tab.kind === "viewdef" ? "view" : tab.kind;
    return {
      load: (name) => schema.definition({ connectionId, database, name, kind }),
      review: ({ sql, existingName }) =>
        runDefinition(connectionId, { database, kind, existingName, sql }, {
          title: existingName === null ? `Create ${kind}` : `Save ${kind} ${existingName}`,
          note: existingName !== null && kind !== "view" ? "MySQL cannot alter this in place: it is dropped and created again. If the new definition fails, the old one is restored." : "",
        }),
      run: () => runRoutine(connectionId, database, tab.name, kind),
      onSaved: (name, previousName) => {
        if (name !== null && name !== previousName) reopen(tab, tab.kind, name);
      },
    };
  }

  // ---- context menus -----------------------------------------------------------------------

  function menuItems({ connectionId, type, database, group, name, kind }) {
    const act = (title, work) => guarded(title, work);
    const open = (tabKind) => () => schema.openObject({ connectionId, database, name, kind: tabKind });
    const newObject = (objectKind) => () => schema.openNewObject(connectionId, database, objectKind);
    if (type === "database") {
      return [
        { label: "New table…", onSelect: newObject("table") },
        { label: "New query here", onSelect: () => schema.newQuery(connectionId, database) },
        { label: "Refresh", onSelect: () => refreshTree(connectionId) },
        "-",
        { label: "Maintain tables…", onSelect: act("Maintain tables", () => admin.openMaintenance(connectionId, database)) },
        { label: "Diagram…", onSelect: () => schema.openDiagram(connectionId, database) },
        { label: "Export database…", onSelect: () => schema.openTransfer(connectionId, database, "export") },
        { label: "Import SQL file…", onSelect: () => schema.openTransfer(connectionId, database, "import") },
        "-",
        { label: "Alter database…", onSelect: act("Alter database", () => alterDatabase(connectionId, database)) },
        { label: "Drop database…", danger: true, onSelect: act("Drop database", () => dropDatabase(connectionId, database)) },
      ];
    }
    if (type === "group") {
      const objectKind = NEW_GROUP_KINDS[group];
      return [{ label: `New ${OBJECT_LABELS[objectKind]}…`, onSelect: newObject(objectKind) }];
    }
    const drop = { label: `Drop ${OBJECT_LABELS[kind]}…`, danger: true, onSelect: act(`Drop ${kind}`, () => dropObject(connectionId, database, kind, name)) };
    switch (kind) {
      case "table":
        return [
          { label: "Open data", onSelect: open("table") },
          { label: "Structure…", onSelect: open("structure") },
          { label: "Duplicate…", onSelect: act("Duplicate table", () => duplicateTable(connectionId, database, name)) },
          { label: "Maintenance…", onSelect: act("Maintain table", () => admin.openMaintenance(connectionId, database, [name])) },
          { label: "Import CSV…", onSelect: () => schema.openTransfer(connectionId, database, "csvimport", { table: name }) },
          { label: "Export table…", onSelect: () => schema.openTransfer(connectionId, database, "export", { tables: [name] }) },
          "-",
          { label: "Truncate…", danger: true, onSelect: act("Truncate table", () => truncateTable(connectionId, database, name)) },
          drop,
        ];
      case "view":
        return [{ label: "Open data", onSelect: open("view") }, { label: "Edit definition…", onSelect: open("viewdef") }, "-", drop];
      case "procedure":
      case "function":
        return [{ label: "Edit…", onSelect: open(kind) }, { label: "Run…", onSelect: act(`Run ${kind}`, () => runRoutine(connectionId, database, name, kind)) }, "-", drop];
      default:
        return [{ label: "Edit…", onSelect: open(kind) }, "-", drop];
    }
  }

  /** Duplicate a table: ask for the new name, then review the SQL. Foreign keys are not copied. */
  async function duplicateTable(connectionId, database, source) {
    const definition = await api("table_definition", { connectionId, database, table: source }, { label: `Read structure of ${source}` });
    const columns = definition.columns.filter((column) => column.generated === null).map((column) => column.name);
    const name = input({ value: `${source}_copy`, "aria-label": "Name of the copy", required: true });
    const withData = h("input", { type: "checkbox", checked: true, "aria-label": "Copy the rows too" });
    const dialog = formDialog({
      title: `Duplicate ${source}`,
      fields: [field("Name of the copy", name), h("label", { class: "check" }, withData, "Copy the rows too"), h("p", { class: "muted" }, "Columns and indexes are copied. Foreign keys and triggers are not.")],
      submitLabel: "Review SQL…",
      submit: () => runChange(connectionId, { kind: "copyTable", database, source, target: name.value.trim(), withData: withData.checked, columns }, { title: `Duplicate ${source}`, confirmLabel: "Create copy" }),
    });
    name.focus();
    name.select();
    return dialog;
  }

  /** Reads a table's definition, lets `mutate` change a copy, and runs the difference after review. */
  async function editTable(connectionId, database, table, mutate, review = {}) {
    const original = await api("table_definition", { connectionId, database, table }, { label: `Read structure of ${table}` });
    return runChange(connectionId, { kind: "alterTable", database, original, edited: mutate(structuredClone(original)) }, review);
  }

  return {
    editTable,
    duplicateTable,
    newDatabase: (connectionId) => guarded("New database", () => newDatabase(connectionId))(),
    showMenu: (event, target) => showMenu(event, menuItems(target)),
    designerData,
    objectEditorData,
  };
}
