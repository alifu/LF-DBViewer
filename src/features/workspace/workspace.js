import { h } from "../../lib/dom.js";

const ENGINE_LABELS = { mysql: "MySQL", mariadb: "MariaDB" };

function statusText(connection) {
  const { status } = connection;
  if (!status.connected) return status.error ?? "Disconnected";
  const latency = status.latencyMs === null ? "" : ` · ${status.latencyMs} ms`;
  return `Connected${latency}`;
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
    h(
      "button",
      {
        type: "button",
        class: "ghost close",
        title: "Disconnect",
        "aria-label": `Disconnect ${connection.name}`,
        onClick: (event) => (event.stopPropagation(), actions.close(connection.id)),
      },
      "×",
    ),
  );
}

function detail(label, value) {
  return [h("dt", {}, label), h("dd", {}, value)];
}

/** Main window once connected: connection tabs and status on top, objects on the left. */
export function createWorkspace({ actions, logPanel }) {
  const tabs = h("div", { class: "conn-tabs" });
  const status = h("div", { class: "conn-status" });
  const reload = h("button", { type: "button", title: "Reload (reconnects if the connection dropped)" }, "Reload");
  const sidebar = h("aside", { class: "sidebar" });
  const content = h("section", { class: "content" });
  let activeId = null;
  reload.addEventListener("click", () => activeId && actions.reload(activeId));

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
    ),
    h("div", { class: "workspace-body" }, sidebar, content),
    logPanel.element,
  );

  return {
    element,
    render(state) {
      const active = state.connections.find((connection) => connection.id === state.activeConnectionId) ?? null;
      activeId = active?.id ?? null;
      tabs.replaceChildren(...state.connections.map((connection) => connectionTab(connection, activeId, actions)));
      reload.disabled = active === null;
      if (active === null) {
        status.replaceChildren();
        sidebar.replaceChildren();
        content.replaceChildren();
        return;
      }
      status.replaceChildren(
        h("span", { class: active.status.connected ? "dot up" : "dot down" }),
        h("span", {}, `${ENGINE_LABELS[active.engine]} ${active.serverVersion}`),
        h("span", { class: active.status.connected ? "muted" : "error-text" }, statusText(active)),
      );
      sidebar.replaceChildren(
        h("div", { class: "panel-header" }, h("strong", {}, "Connection")),
        h(
          "dl",
          { class: "details" },
          detail("Server", `${active.host}:${active.port}`),
          detail("User", active.user),
          detail("Database", active.database ?? "(none selected)"),
          detail("Route", active.viaSsh ? "SSH tunnel" : "Direct"),
        ),
      );
      content.replaceChildren(h("p", { class: "muted empty" }, "Connected. Tables, data and the query editor are the next build steps."));
    },
  };
}
