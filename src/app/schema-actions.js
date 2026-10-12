import { databaseKey, groupKey } from "../features/schema/tree.js";
import { newObjectName } from "../features/structure/object-templates.js";
import * as transitions from "../state/app-state.js";

const DEFAULT_OPEN_GROUP = "tables";

/** Loading and navigating the object tree, and the backend calls a tab's content needs. */
export function createSchemaActions({ api, store }) {
  const schema = (connectionId) => transitions.schemaOf(store.getState(), connectionId);

  async function loadObjects(connectionId, database) {
    const objects = await api("list_objects", { connectionId, database }, { label: `Load objects of ${database}` });
    store.update((state) => transitions.setObjects(state, connectionId, database, objects));
  }

  /** Reloads the database list, then the objects of every database that is open in the tree. */
  async function loadDatabases(connectionId) {
    const databases = await api("list_databases", { connectionId }, { label: "Load databases" });
    store.update((state) => transitions.setDatabases(state, connectionId, databases));
    const open = databases.filter((database) => schema(connectionId).expanded[databaseKey(database.name)]);
    await Promise.all(open.map((database) => loadObjects(connectionId, database.name)));
  }

  async function toggleDatabase(connectionId, database) {
    const key = databaseKey(database);
    const opening = !schema(connectionId).expanded[key];
    const firstTime = schema(connectionId).objects[database] === undefined;
    store.update((state) => transitions.toggleExpanded(state, connectionId, key));
    if (!opening || !firstTime) return;
    const tablesKey = groupKey(database, DEFAULT_OPEN_GROUP);
    if (!schema(connectionId).expanded[tablesKey]) {
      store.update((state) => transitions.toggleExpanded(state, connectionId, tablesKey));
    }
    // A failure is already in the log; close the node again so it can be retried.
    await loadObjects(connectionId, database).catch(() => store.update((state) => transitions.toggleExpanded(state, connectionId, key)));
  }

  return {
    loadDatabases,
    toggleDatabase,
    toggleGroup: (connectionId, database, group) =>
      store.update((state) => transitions.toggleExpanded(state, connectionId, groupKey(database, group))),
    /** `extra` can carry `initialSql` to put in the editor and `autorun` to run it at once. */
    newQuery(connectionId, database, extra = {}) {
      const state = store.getState();
      const used = transitions.tabsOf(state, connectionId).filter((tab) => tab.kind === "query").map((tab) => Number(tab.name.replace("Query ", "")));
      const name = `Query ${Math.max(0, ...used) + 1}`;
      store.update((current) => transitions.openTab(current, { connectionId, database, name, kind: "query", ...extra }));
    },
    openDiagram: (connectionId, database) => store.update((state) => transitions.openTab(state, { connectionId, database, name: `Diagram ${database}`, kind: "diagram" })),
    /** The server tab: one per connection. */
    openServer: (connectionId) => store.update((state) => transitions.openTab(state, { connectionId, database: "", name: "Server", kind: "admin" })),
    /** An export or import tab; `extra` can carry `tables` to export only those. */
    openTransfer(connectionId, database, kind, extra = {}) {
      const name = kind === "import" ? `Import into ${database}` : `Export ${extra.tables?.length === 1 ? extra.tables[0] : database}`;
      store.update((current) => transitions.openTab(current, { connectionId, database, name, kind, ...extra }));
    },
    /** A tab for a table, view, routine, trigger or event that does not exist yet. */
    openNewObject(connectionId, database, kind) {
      const tabKind = { table: "newtable", view: "viewdef" }[kind] ?? kind;
      const taken = transitions.tabsOf(store.getState(), connectionId).filter((tab) => tab.isNew && tab.kind === tabKind).map((tab) => tab.name);
      const name = newObjectName(kind, taken);
      store.update((current) => transitions.openTab(current, { connectionId, database, name, kind: tabKind, isNew: true }));
    },
    openObject: (target) => store.update((state) => transitions.openTab(state, target)),
    activateTab: (id) => store.update((state) => transitions.activateTab(state, id)),
    closeTab: (id) => store.update((state) => transitions.closeTab(state, id)),
    /** Backend calls bound to one table tab; each logs the SQL that ran. */
    tableData: ({ connectionId, database, name }) => ({
      openStructure: () => store.update((state) => transitions.openTab(state, { connectionId, database, name, kind: "structure" })),
      count: (filters) => api("count_table_rows", { connectionId, database, table: name, filters }, { label: `Count rows of ${name}` }),
      readCell: ({ column, key }) => api("read_table_cell", { connectionId, database, table: name, column, key }, { label: `Read ${name}.${column}` }),
      browse: (page) => api("browse_table", { connectionId, database, table: name, page }, { label: (result) => result.sql }),
      apply: (changes) =>
        api("apply_row_changes", { connectionId, database, table: name, changes }, { label: (result) => result.statements.join(";\n") }),
    }),
    definition: ({ connectionId, database, name, kind }) =>
      api("object_definition", { connectionId, database, name, kind }, { label: `SHOW CREATE ${kind.toUpperCase()} ${database}.${name}` }),
  };
}
