import { invoke } from "@tauri-apps/api/core";

import { createConnectionActions } from "./app/connection-actions.js";
import { createSchemaActions } from "./app/schema-actions.js";
import { createDashboard } from "./features/dashboard/dashboard.js";
import { createLogPanel } from "./features/log/log-panel.js";
import { createDefinitionView } from "./features/schema/definition-view.js";
import { databaseKey } from "./features/schema/tree.js";
import { createTableView } from "./features/table/table-view.js";
import { createWorkspace } from "./features/workspace/workspace.js";
import { createApi } from "./lib/api.js";
import { confirmDialog } from "./lib/dom.js";
import { createStore } from "./lib/store.js";
import * as transitions from "./state/app-state.js";

const PING_INTERVAL_MS = 15_000;
const DATA_KINDS = ["table", "view"];

const store = createStore(transitions.initialState);
const log = (entry) => store.update((state) => transitions.appendLog(state, { ...entry, time: new Date() }));
const api = createApi({ invoke, onLog: log });
const schema = createSchemaActions({ api, store });

/** A new connection shows its databases, with the profile's default database already open. */
async function showSchema(info) {
  await schema.loadDatabases(info.id).catch(() => {});
  const isClosed = !transitions.schemaOf(store.getState(), info.id).expanded[databaseKey(info.database)];
  if (info.database && isClosed) await schema.toggleDatabase(info.id, info.database);
}

const connections = createConnectionActions({ api, store, log, onConnected: showSchema });

async function confirmLosingEdits(tabIds, what) {
  if (!workspace.hasUnsavedChanges(tabIds)) return true;
  return confirmDialog({
    title: "Discard unsaved changes?",
    body: `${what} has row changes that were not applied.`,
    confirmLabel: "Discard",
    danger: true,
  });
}

const workspaceActions = {
  ...schema,
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

function createTabView(tab) {
  if (DATA_KINDS.includes(tab.kind)) return createTableView({ tab, data: schema.tableData(tab) });
  return createDefinitionView({ load: () => schema.definition(tab) });
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
