import { save } from "@tauri-apps/plugin-dialog";

import { defaultConstraintName, withForeignKey, withoutForeignKey } from "../features/diagram/fk-edit.js";
import { layoutKey, loadLayout, saveLayout } from "../features/diagram/layout-store.js";
import { historyKey } from "../features/query/history.js";

const IMAGE_FILTERS = { svg: [{ name: "SVG image", extensions: ["svg"] }], png: [{ name: "PNG image", extensions: ["png"] }] };

/** Backend calls and file saving for one diagram tab. */
export function createDiagramActions({ api, store, schema, structure, storage }) {
  function forTab(tab) {
    const { connectionId, database } = tab;
    const connection = () => store.getState().connections.find((candidate) => candidate.id === connectionId);
    const key = () => layoutKey(historyKey(connection()), database);
    const reviewTitle = (what) => ({ title: what, confirmLabel: "Run" });
    return {
      load: () => api("database_diagram", { connectionId, database }, { label: `Read diagram of ${database}` }),
      loadLayout: () => loadLayout(storage, key()),
      saveLayout: (layout) => saveLayout(storage, key(), layout),
      openTable: (name) => schema.openObject({ connectionId, database, name, kind: "table" }),
      openStructure: (name) => schema.openObject({ connectionId, database, name, kind: "structure" }),
      /** Adds a foreign key to `table`. Resolves true when it was applied. */
      addForeignKey: ({ table, columns, refDatabase, refTable, refColumns, name, onUpdate, onDelete }) =>
        structure.editTable(connectionId, database, table, (definition) => withForeignKey(definition, { name, columns, refDatabase, refTable, refColumns, onUpdate, onDelete }), reviewTitle(`Add foreign key to ${table}`)),
      dropForeignKey: ({ table, name }) =>
        structure.editTable(connectionId, database, table, (definition) => withoutForeignKey(definition, name), { title: `Drop foreign key ${name}`, confirmLabel: "Drop foreign key", danger: true }),
      /** A name for a new key that no other key of the table uses. */
      suggestName: async (table, columns) => {
        const definition = await api("table_definition", { connectionId, database, table }, { quiet: true });
        return defaultConstraintName(table, columns, definition.foreignKeys.map((existing) => existing.name));
      },
      /** Asks where to save, then writes the bytes. Resolves to the path, or null when cancelled. */
      async saveImage(kind, bytes) {
        const path = await save({ defaultPath: `${database}.${kind}`, filters: IMAGE_FILTERS[kind] });
        if (!path) return null;
        await api("write_export_file", bytes, { label: `Save ${path}`, headers: { path: encodeURIComponent(path) } });
        return path;
      },
    };
  }
  return { forTab };
}
