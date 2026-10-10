import { confirmDialog } from "../lib/dom.js";
import * as transitions from "../state/app-state.js";

const SSH_UNKNOWN_HOST = "ssh_unknown_host";

/** Everything the login screen and the connection tabs can do. */
export function createConnectionActions({ api, store, log, onConnected }) {
  async function refreshProfiles() {
    const profiles = await api("list_profiles", {}, { quiet: true });
    store.update((state) => transitions.setProfiles(state, profiles));
  }

  /** Runs a connect or test call; an unknown SSH host is retried once the user accepts its fingerprint. */
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

  const toConnectRequest = ({ profile, password, sshSecret }) => ({ profile, password, sshSecret });

  async function checkConnection(id) {
    const status = await api("ping_connection", { connectionId: id }, { quiet: true });
    const previous = store.getState().connections.find((connection) => connection.id === id);
    if (previous && previous.status.connected !== status.connected) {
      log({ level: status.connected ? "info" : "error", message: `${previous.name}: ${status.connected ? "connection restored" : status.error}` });
    }
    store.update((state) => transitions.setConnectionStatus(state, id, status));
    return status;
  }

  /** Pings, and reconnects if the connection dropped. Resolves to whether it is usable. */
  async function ensureConnected(id) {
    const status = await checkConnection(id).catch(() => ({ connected: false }));
    if (status.connected) return true;
    const info = await api("reconnect_connection", { connectionId: id }, { label: "Reconnect" }).catch(() => null);
    if (info === null) return false;
    store.update((state) => transitions.replaceConnectionInfo(state, info));
    return (await checkConnection(id)).connected;
  }

  return {
    refreshProfiles,
    checkConnection,
    ensureConnected,
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
      await onConnected(info);
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
    activate: (id) => store.update((state) => transitions.activateConnection(state, id)),
    newConnection: () => store.update(transitions.showDashboard),
    async disconnect(id) {
      // Even if the server is already gone, the tab should close.
      await api("close_connection", { connectionId: id }, { label: "Disconnect" }).catch(() => {});
      store.update((state) => transitions.removeConnection(state, id));
    },
  };
}
