import { h } from "../../lib/dom.js";
import { schemaOf, tabsOf } from "../../state/app-state.js";
import { createSchemaTree } from "../schema/tree.js";

const ENGINE_LABELS = { mysql: "MySQL", mariadb: "MariaDB" };

function statusText(connection) {
  const { status } = connection;
  if (!status.connected) return status.error ?? "Disconnected";
  const latency = status.latencyMs === null ? "" : ` · ${status.latencyMs} ms`;
  return `Connected${latency}`;
}

function closeButton(label, onClose) {
  return h(
    "button",
    { type: "button", class: "ghost close", title: label, "aria-label": label, onClick: (event) => (event.stopPropagation(), onClose()) },
    "×",
  );
}

function connectionTab(connection, activeId, actions) {
  return h(
    "div",
    {
      class: connection.id === activeId ? "conn-tab active" : "conn-tab",
      onClick: () => actions.activate(connection.id),
      title: `${connection.user}@${connection.host}:${connection.port}${connection.viaSsh ? " via SSH" : ""}`,
    },
    h("span", { class: connection.status.connected ? "dot up" : "dot down" }),
    h("span", { class: "conn-name" }, connection.name),
    closeButton(`Disconnect ${connection.name}`, () => actions.close(connection.id)),
  );
}

function objectTab(tab, activeTabId, actions) {
  return h(
    "div",
    { class: tab.id === activeTabId ? "object-tab active" : "object-tab", title: `${tab.database}.${tab.name} (${tab.kind})`, onClick: () => actions.activateTab(tab.id) },
    h("span", {}, tab.name),
    h("span", { class: "muted" }, tab.database),
    closeButton(`Close ${tab.name}`, () => actions.closeTab(tab.id)),
  );
}

/**
 * Main window once connected: connection tabs and status on top, objects on the left,
 * object tabs in the middle. `createTabView(tab)` builds the content of a tab; each is
 * built once and kept until its tab closes, so switching tabs keeps scroll and edits.
 */
export function createWorkspace({ actions, logPanel, createTabView }) {
  const tabs = h("div", { class: "conn-tabs" });
  const status = h("div", { class: "conn-status" });
  const reload = h("button", { type: "button", title: "Reload objects and data; reconnects if the connection dropped" }, "Reload");
  const tree = createSchemaTree({ actions });
  const objectTabs = h("div", { class: "object-tabs" });
  const host = h("div", { class: "tab-host" });
  const views = new Map();
  let activeId = null;
  let shownTabId = null;
  reload.addEventListener("click", () => activeId && actions.reload(activeId, views.get(shownTabId) ?? null));

  const element = h(
    "div",
    { class: "workspace" },
    h(
      "header",
      { class: "topbar" },
      tabs,
      h("button", { type: "button", class: "ghost", title: "Open another connection", onClick: () => actions.newConnection() }, "+"),
      h("span", { class: "spacer" }),
      status,
      reload,
      h("button", { type: "button", class: "ghost", title: "Settings (⌘,)", onClick: () => actions.openSettings() }, "Settings"),
    ),
    h("div", { class: "workspace-body" }, h("aside", { class: "sidebar" }, tree.element), h("section", { class: "content" }, objectTabs, host)),
    logPanel.element,
  );

  function showTab(tab) {
    if (tab === null) {
      shownTabId = null;
      host.replaceChildren(h("p", { class: "muted empty" }, "Open a database on the left and pick a table to see its data."));
      return;
    }
    if (!views.has(tab.id)) views.set(tab.id, createTabView(tab));
    if (shownTabId === tab.id) return;
    shownTabId = tab.id;
    host.replaceChildren(views.get(tab.id).element);
  }

  return {
    element,
    /** Whether any open tab of the connection holds edits that were not applied. */
    hasUnsavedChanges: (tabIds) => tabIds.some((id) => views.get(id)?.hasUnsavedChanges()),
    render(state, previous) {
      const active = state.connections.find((connection) => connection.id === state.activeConnectionId) ?? null;
      activeId = active?.id ?? null;
      for (const id of [...views.keys()]) {
        if (!state.tabs.some((tab) => tab.id === id)) views.delete(id);
      }
      if (state.connections !== previous?.connections || state.activeConnectionId !== previous?.activeConnectionId) {
        tabs.replaceChildren(...state.connections.map((connection) => connectionTab(connection, activeId, actions)));
        reload.disabled = active === null;
        status.replaceChildren(
          ...(active === null
            ? []
            : [
                h("span", { class: active.status.connected ? "dot up" : "dot down" }),
                h("span", {}, `${ENGINE_LABELS[active.engine]} ${active.serverVersion}`),
                h("span", { class: active.status.connected ? "muted" : "error-text" }, statusText(active)),
              ]),
        );
      }
      if (active === null) return;

      const connectionTabs = tabsOf(state, active.id);
      const activeTabId = state.activeTabIds[active.id] ?? null;
      const activeTab = connectionTabs.find((tab) => tab.id === activeTabId) ?? null;
      const sameContext = state.activeConnectionId === previous?.activeConnectionId;
      if (!sameContext || state.schemas !== previous?.schemas || state.activeTabIds !== previous?.activeTabIds) {
        tree.render({ connectionId: active.id, schema: schemaOf(state, active.id), activeTab });
      }
      if (!sameContext || state.tabs !== previous?.tabs || state.activeTabIds !== previous?.activeTabIds) {
        objectTabs.replaceChildren(...connectionTabs.map((tab) => objectTab(tab, activeTabId, actions)));
        showTab(activeTab);
      }
    },
  };
}
