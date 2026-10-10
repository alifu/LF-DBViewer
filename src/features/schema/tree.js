import { h } from "../../lib/dom.js";

export const GROUPS = Object.freeze([
  { key: "tables", label: "Tables", kind: "table" },
  { key: "views", label: "Views", kind: "view" },
  { key: "procedures", label: "Procedures", kind: "procedure" },
  { key: "functions", label: "Functions", kind: "function" },
  { key: "triggers", label: "Triggers", kind: "trigger" },
  { key: "events", label: "Events", kind: "event" },
]);

export const databaseKey = (database) => `db:${database}`;
export const groupKey = (database, group) => `group:${database}:${group}`;

const nameOf = (item) => (typeof item === "string" ? item : item.name);

/** The groups of one database that have something to show for the search text. */
export function visibleGroups(objects, search) {
  const needle = search.trim().toLowerCase();
  return GROUPS.map((group) => ({
    ...group,
    items: (objects[group.key] ?? []).map(nameOf).filter((name) => name.toLowerCase().includes(needle)),
  })).filter((group) => group.items.length > 0);
}

function twisty(open) {
  return h("span", { class: "twisty" }, open ? "▾" : "▸");
}

/** Left-hand list of databases and their objects; objects load when a database is opened. */
export function createSchemaTree({ actions }) {
  const search = h("input", { type: "search", placeholder: "Filter objects", "aria-label": "Filter objects", spellcheck: false });
  const list = h("div", { class: "tree", role: "tree" });
  const element = h("div", { class: "schema-tree" }, h("div", { class: "tree-search" }, search), list);
  let last = null;
  search.addEventListener("input", () => last && draw(last));

  function drawDatabase(connectionId, schema, database, activeTab) {
    const open = Boolean(schema.expanded[databaseKey(database.name)]);
    const objects = schema.objects[database.name];
    const node = h(
      "div",
      { class: "tree-node database", title: database.collation ?? "", onClick: () => actions.toggleDatabase(connectionId, database.name) },
      twisty(open),
      h("strong", {}, database.name),
    );
    if (!open) return [node];
    if (objects === undefined) return [node, h("div", { class: "tree-node leaf muted" }, "Loading…")];
    const groups = visibleGroups(objects, search.value);
    if (groups.length === 0) return [node, h("div", { class: "tree-node leaf muted" }, search.value ? "No matches" : "Empty")];

    return [
      node,
      ...groups.flatMap((group) => {
        // While searching, every group with a match is shown open.
        const groupOpen = search.value !== "" || Boolean(schema.expanded[groupKey(database.name, group.key)]);
        const heading = h(
          "div",
          { class: "tree-node group", onClick: () => actions.toggleGroup(connectionId, database.name, group.key) },
          twisty(groupOpen),
          `${group.label} `,
          h("span", { class: "muted" }, String(group.items.length)),
        );
        if (!groupOpen) return [heading];
        return [
          heading,
          ...group.items.map((name) => {
            const isActive = activeTab !== null && activeTab.database === database.name && activeTab.name === name && activeTab.kind === group.kind;
            return h(
              "div",
              {
                class: isActive ? "tree-node leaf active" : "tree-node leaf",
                title: name,
                onClick: () => actions.openObject({ connectionId, database: database.name, name, kind: group.kind }),
              },
              name,
            );
          }),
        ];
      }),
    ];
  }

  function draw({ connectionId, schema, activeTab }) {
    last = { connectionId, schema, activeTab };
    const nodes = schema.databases.flatMap((database) => drawDatabase(connectionId, schema, database, activeTab));
    list.replaceChildren(...(nodes.length > 0 ? nodes : [h("div", { class: "tree-node leaf muted" }, "No databases")]));
  }

  return { element, render: draw };
}
