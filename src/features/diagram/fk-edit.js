import { uniqueName } from "../structure/designer-state.js";

const MAX_NAME_CHARS = 64;

/** `fk_orders_customer_id`, made unique among the table's existing constraint names and short enough for MySQL. */
export function defaultConstraintName(table, columns, taken) {
  const base = `fk_${table}_${columns.join("_")}`.slice(0, MAX_NAME_CHARS - 4);
  return uniqueName(taken, base);
}

/** The table's definition with one more foreign key, as the designer would add it. */
export function withForeignKey(definition, { name, columns, refDatabase, refTable, refColumns, onUpdate, onDelete }) {
  return {
    ...definition,
    foreignKeys: [...definition.foreignKeys, { originalName: null, name, columns, refDatabase: refDatabase ?? null, refTable, refColumns, onUpdate, onDelete }],
  };
}

export function withoutForeignKey(definition, name) {
  return { ...definition, foreignKeys: definition.foreignKeys.filter((key) => key.name !== name) };
}
