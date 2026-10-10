import { invoke } from "@tauri-apps/api/core";

import { createDashboard } from "./features/dashboard/dashboard.js";
import { createLogPanel } from "./features/log/log-panel.js";
import { createWorkspace } from "./features/workspace/workspace.js";
import { createApi } from "./lib/api.js";
import { confirmDialog } from "./lib/dom.js";
import { createStore } from "./lib/store.js";
import * as transitions from "./state/app-state.js";

const PING_INTERVAL_MS = 15_000;
const SSH_UNKNOWN_HOST = "ssh_unknown_host";

const store = createStore(transitions.initialState);
const log = (entry) => store.update((state) => transitions.appendLog(state, { ...entry, time: new Date() }));
const api = createApi({ invoke, onLog: log });

async function refreshProfiles() {
  const profiles = await api("list_profiles", {}, { quiet: true });
  store.update((state) => transitions.setProfiles(state, profiles));
}

/** Runs a connect/test call; an unknown SSH host is retried once the user accepts its fingerprint. */
async function withHostKeyPrompt(command, request, label) {
  try {
    return await api(command, { request }, { label });
  } catch (error) {
    if (error.code !== SSH_UNKNOWN_HOST) throw error;
    const trusted = await confirmDialog({
      title: "Unknown SSH host",
      body: `${error.message}. Trust this host and add it to ~/.ssh/known_hosts?`,
      confirmLabel: "Trust and connect",
    });
    if (!trusted) throw error;
    return api(command, { request: { ...request, trustSshHost: true } }, { label });
  }
}

function toConnectRequest({ profile, password, sshSecret }) {
  return { profile, password, sshSecret };
}

async function checkConnection(id) {
  const status = await api("ping_connection", { connectionId: id }, { quiet: true });
  const previous = store.getState().connections.find((connection) => connection.id === id);
  if (previous && previous.status.connected !== status.connected) {
    log({ level: status.connected ? "info" : "error", message: `${previous.name}: ${status.connected ? "connection restored" : status.error}` });
  }
  store.update((state) => transitions.setConnectionStatus(state, id, status));
  return status;
}

const dashboardActions = {
  test: (request) => withHostKeyPrompt("test_connection", toConnectRequest(request), `Test ${request.profile.name}`),
  async save(request) {
    const saved = await api("save_profile", { request }, { label: `Save ${request.profile.name}` });
    await refreshProfiles();
    return saved;
  },
  async connect(request) {
    const info = await withHostKeyPrompt("open_connection", toConnectRequest(request), `Connect to ${request.profile.name}`);
    store.update((state) => transitions.addConnection(state, info));
    await checkConnection(info.id);
  },
  async remove(form) {
    const confirmed = await confirmDialog({
      title: "Delete saved connection",
      body: `Delete "${form.name}" and its saved passwords? The database itself is not touched.`,
      confirmLabel: "Delete",
      danger: true,
    });
    if (!confirmed) return false;
    await api("delete_profile", { id: form.id }, { label: `Delete ${form.name}` });
    await refreshProfiles();
    return true;
  },
  backToWorkspace: () => store.update((state) => transitions.activateConnection(state, state.activeConnectionId)),
};

const workspaceActions = {
  activate: (id) => store.update((state) => transitions.activateConnection(state, id)),
  newConnection: () => store.update(transitions.showDashboard),
  async close(id) {
    await api("close_connection", { connectionId: id }, { label: "Disconnect" }).catch(() => {});
    store.update((state) => transitions.removeConnection(state, id));
  },
  async reload(id) {
    const status = await checkConnection(id).catch(() => ({ connected: false }));
    if (status.connected) return;
    const info = await api("reconnect_connection", { connectionId: id }, { label: "Reconnect" }).catch(() => null);
    if (info === null) return;
    store.update((state) => transitions.replaceConnectionInfo(state, info));
    await checkConnection(id);
  },
};

const logPanel = createLogPanel({ onClear: () => store.update(transitions.clearLog) });
const dashboard = createDashboard({ actions: dashboardActions });
const workspace = createWorkspace({ actions: workspaceActions, logPanel });
const root = document.querySelector("#app");

function render(state, previous) {
  if (state.view !== previous?.view) {
    root.replaceChildren(state.view === transitions.VIEW.dashboard ? dashboard.element : workspace.element);
  }
  dashboard.render(state);
  workspace.render(state);
  if (state.log !== previous?.log) logPanel.render(state.log);
}

store.subscribe(render);
render(store.getState(), null);

setInterval(() => {
  // Failures are already in the log; one dead connection must not stop the others being checked.
  for (const connection of store.getState().connections) checkConnection(connection.id).catch(() => {});
}, PING_INTERVAL_MS);

refreshProfiles().catch(() => {});
api("detect_client_defaults", {}, { quiet: true })
  .then((defaults) => dashboard.setClientDefaults(defaults))
  .catch(() => {});
