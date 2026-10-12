import { invoke } from "@tauri-apps/api/core";

import { createAdminActions } from "./app/admin-actions.js";
import { createConnectionActions } from "./app/connection-actions.js";
import { createSchemaActions } from "./app/schema-actions.js";
import { createAdminView } from "./features/admin/admin-view.js";
import { createDashboard } from "./features/dashboard/dashboard.js";
import { createLogPanel } from "./features/log/log-panel.js";
import { createQueryActions } from "./app/query-actions.js";
import { createStructureActions } from "./app/structure-actions.js";
import { createTransferActions } from "./app/transfer-actions.js";
import { createQueryView } from "./features/query/query-view.js";
import { databaseKey } from "./features/schema/tree.js";
import { createExportView } from "./features/transfer/export-view.js";
import { createImportView } from "./features/transfer/import-view.js";
import { createDesignerView } from "./features/structure/designer-view.js";
import { createObjectEditor } from "./features/structure/object-editor.js";
import { openSettings } from "./features/settings/settings-dialog.js";
import { browserStorage, createThemeController } from "./features/settings/theme.js";
import { createTableView } from "./features/table/table-view.js";
import { createWorkspace } from "./features/workspace/workspace.js";
import { createApi } from "./lib/api.js";
import { confirmDialog } from "./lib/dom.js";
import { createStore } from "./lib/store.js";
import * as transitions from "./state/app-state.js";

const PING_INTERVAL_MS = 15_000;
const DATA_KINDS = ["table", "view"];
const DESIGNER_KINDS = ["structure", "newtable"];

// Applied before anything renders, so the first paint already has the right colours.
const theme = createThemeController({
  root: document.documentElement,
  storage: browserStorage(window),
  matchMedia: (query) => window.matchMedia(query),
});
const showSettings = () => openSettings({ theme });

const store = createStore(transitions.initialState);
const log = (entry) => store.update((state) => transitions.appendLog(state, { ...entry, time: new Date() }));
const api = createApi({ invoke, onLog: log });
const schema = createSchemaActions({ api, store });
const admin = createAdminActions({ api, store });
const structure = createStructureActions({ api, store, schema, admin });
const transfer = createTransferActions({ api, store, schema });

/** A new connection shows its databases, with the profile's default database already open. */
async function showSchema(info) {
  await schema.loadDatabases(info.id).catch(() => {});
  const isClosed = !transitions.schemaOf(store.getState(), info.id).expanded[databaseKey(info.database)];
  if (info.database && isClosed) await schema.toggleDatabase(info.id, info.database);
}

const connections = { ...createConnectionActions({ api, store, log, onConnected: showSchema }), openSettings: showSettings };

async function confirmLosingEdits(tabIds, what) {
  if (!workspace.hasUnsavedChanges(tabIds)) return true;
  return confirmDialog({
    title: "Discard unsaved changes?",
    body: `${what} has unsaved changes or a job in progress: edited rows, query text that was not run, or a running export or import.`,
    confirmLabel: "Discard",
    danger: true,
  });
}

const workspaceActions = {
  ...schema,
  openSettings: showSettings,
  newQuery,
  newDatabase: structure.newDatabase,
  showMenu: structure.showMenu,
  activate: connections.activate,
  newConnection: connections.newConnection,
  async close(id) {
    const tabIds = transitions.tabsOf(store.getState(), id).map((tab) => tab.id);
    if (await confirmLosingEdits(tabIds, "This connection")) await connections.disconnect(id);
  },
  async closeTab(id) {
    if (await confirmLosingEdits([id], "This tab")) schema.closeTab(id);
  },
  /** Reload button: make sure the connection is up, then refresh the tree and the open tab. */
  async reload(id, activeView) {
    if (!(await connections.ensureConnected(id))) return;
    await schema.loadDatabases(id).catch(() => {});
    await activeView?.reload();
  },
};

/** The database a new query starts in: the open tab's, else the connection's, else any open one in the tree. */
function defaultDatabase(state, connectionId) {
  const openTab = state.tabs.find((tab) => tab.id === state.activeTabIds[connectionId]);
  const connection = state.connections.find((candidate) => candidate.id === connectionId);
  const expanded = Object.keys(transitions.schemaOf(state, connectionId).expanded).find((key) => key.startsWith("db:"));
  return openTab?.database ?? connection?.database ?? expanded?.slice("db:".length) ?? "";
}

function newQuery(connectionId) {
  schema.newQuery(connectionId, defaultDatabase(store.getState(), connectionId));
}

function createTabView(tab) {
  if (tab.kind === "query") {
    const connection = store.getState().connections.find((candidate) => candidate.id === tab.connectionId);
    return createQueryView({
      tab,
      connection,
      data: createQueryActions({ api, log, connectionId: tab.connectionId }),
      storage: browserStorage(window),
    });
  }
  if (DATA_KINDS.includes(tab.kind)) return createTableView({ tab, data: schema.tableData(tab) });
  if (tab.kind === "admin") return createAdminView({ data: admin.forConnection(tab.connectionId), onMaintain: (database) => void admin.openMaintenance(tab.connectionId, database) });
  if (tab.kind === "export") return createExportView({ tab, data: transfer.exportData(tab) });
  if (tab.kind === "import") return createImportView({ tab, data: transfer.importData(tab) });
  if (DESIGNER_KINDS.includes(tab.kind)) return createDesignerView({ tab, data: structure.designerData(tab) });
  const connection = store.getState().connections.find((candidate) => candidate.id === tab.connectionId);
  return createObjectEditor({ tab, data: structure.objectEditorData(tab), engine: connection.engine });
}

const logPanel = createLogPanel({ onClear: () => store.update(transitions.clearLog) });
const dashboard = createDashboard({ actions: connections });
const workspace = createWorkspace({ actions: workspaceActions, logPanel, createTabView });
const root = document.querySelector("#app");

function render(state, previous) {
  if (state.view !== previous?.view) {
    root.replaceChildren(state.view === transitions.VIEW.dashboard ? dashboard.element : workspace.element);
  }
  dashboard.render(state);
  workspace.render(state, previous);
  if (state.log !== previous?.log) logPanel.render(state.log);
}

window.addEventListener("keydown", (event) => {
  if (!event.metaKey) return;
  const state = store.getState();
  if (event.key === ",") {
    event.preventDefault();
    showSettings();
  } else if (event.key === "t" && state.view === transitions.VIEW.workspace && state.activeConnectionId !== null) {
    event.preventDefault();
    newQuery(state.activeConnectionId);
  }
});

store.subscribe(render);
render(store.getState(), null);

setInterval(() => {
  // Failures are already in the log; one dead connection must not stop the others being checked.
  for (const connection of store.getState().connections) connections.checkConnection(connection.id).catch(() => {});
}, PING_INTERVAL_MS);

connections.refreshProfiles().catch(() => {});
api("detect_client_defaults", {}, { quiet: true })
  .then((defaults) => dashboard.setClientDefaults(defaults))
  .catch(() => {});
