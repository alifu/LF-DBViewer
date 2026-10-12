import { h } from "../../lib/dom.js";
import { createDatabasesPanel, createEnginesPanel, createProcessesPanel, createStatusPanel, createUsersPanel, createVariablesPanel } from "./panels.js";

const SECTIONS = [
  ["databases", "Databases"],
  ["status", "Status"],
  ["processes", "Processes"],
  ["variables", "Variables"],
  ["users", "Users"],
  ["engines", "Engines & character sets"],
];

/**
 * The server tab: what the server is doing and how it is set up, with a section list on the left.
 * `data` is the connection's admin actions; `onMaintain(database)` opens table maintenance.
 */
export function createAdminView({ data, onMaintain }) {
  const review = (change, options) => data.change(change, options);
  const panels = {
    databases: createDatabasesPanel({ data, onMaintain }),
    status: createStatusPanel({ data }),
    processes: createProcessesPanel({ data }),
    variables: createVariablesPanel({ data, review }),
    users: createUsersPanel({ data, review }),
    engines: createEnginesPanel({ data }),
  };
  const loaded = new Set();
  let current = null;
  const nav = h("nav", { class: "admin-nav" });
  const content = h("div", { class: "admin-content" });
  const element = h("div", { class: "admin" }, nav, content);

  function select(key) {
    if (current !== null) panels[current].hide();
    current = key;
    nav.replaceChildren(...SECTIONS.map(([id, label]) => h("button", { type: "button", class: id === key ? "admin-tab active" : "admin-tab", onClick: () => select(id) }, label)));
    content.replaceChildren(panels[key].element);
    if (!loaded.has(key)) {
      loaded.add(key);
      void panels[key].load();
    }
    panels[key].show();
  }

  select("databases");

  return {
    element,
    reload: async () => {
      await panels[current].load();
      return true;
    },
    hasUnsavedChanges: () => false,
    destroy: () => Object.values(panels).forEach((panel) => panel.hide()),
  };
}
