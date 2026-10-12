import { boxHeight, boxWidth } from "./geometry.js";

/**
 * Turns what the backend reports into boxes and edges. Local tables have all their columns (or only the key ones);
 * a table in another database appears as a small box with just the columns that are referenced.
 */

const externalId = (database, table) => `${database}.${table}`;

function describeColumns(table, relations, localKeyColumns, keyColumnsOnly) {
  const foreignColumns = new Set(relations.filter((relation) => relation.table === table.name).flatMap((relation) => relation.columns));
  const referenced = localKeyColumns.get(table.name) ?? new Set();
  return table.columns
    .map((column) => ({
      name: column.name,
      type: column.columnType,
      primaryKey: column.primaryKey,
      foreignKey: foreignColumns.has(column.name),
      referenced: referenced.has(column.name),
      nullable: column.nullable,
    }))
    .filter((column) => !keyColumnsOnly || column.primaryKey || column.foreignKey || column.referenced);
}

export function buildModel(diagram, { keyColumnsOnly = false } = {}) {
  const local = new Set(diagram.tables.map((table) => table.name));
  const referencedLocally = new Map();
  const externalColumns = new Map();
  for (const relation of diagram.relations) {
    const isLocal = relation.refDatabase === diagram.database && local.has(relation.refTable);
    const target = isLocal ? referencedLocally : externalColumns;
    const key = isLocal ? relation.refTable : externalId(relation.refDatabase, relation.refTable);
    target.set(key, new Set([...(target.get(key) ?? []), ...relation.refColumns]));
  }

  const boxes = diagram.tables.map((table) => {
    const rows = describeColumns(table, diagram.relations, referencedLocally, keyColumnsOnly);
    return { id: table.name, title: table.name, external: false, estimatedRows: table.estimatedRows, rows, totalColumns: table.columns.length, width: boxWidth(table.name, rows), height: boxHeight(rows.length) };
  });
  for (const [id, names] of externalColumns) {
    const rows = [...names].map((name) => ({ name, type: "", primaryKey: false, foreignKey: false, referenced: true, nullable: false }));
    boxes.push({ id, title: id, external: true, estimatedRows: null, rows, totalColumns: rows.length, width: boxWidth(id, rows), height: boxHeight(rows.length) });
  }

  const byId = new Map(boxes.map((box) => [box.id, box]));
  const edges = diagram.relations.map((relation) => {
    const parentId = relation.refDatabase === diagram.database && local.has(relation.refTable) ? relation.refTable : externalId(relation.refDatabase, relation.refTable);
    const child = byId.get(relation.table);
    const parent = byId.get(parentId);
    return {
      id: `${relation.table}.${relation.name}`,
      name: relation.name,
      from: relation.table,
      to: parentId,
      fromRow: child.rows.findIndex((row) => row.name === relation.columns[0]),
      toRow: parent.rows.findIndex((row) => row.name === relation.refColumns[0]),
      columns: relation.columns,
      refColumns: relation.refColumns,
      onUpdate: relation.onUpdate,
      onDelete: relation.onDelete,
    };
  });
  return { boxes, edges, byId };
}

/** The edges that touch a box, either way. */
export function edgesOf(model, boxId) {
  return model.edges.filter((edge) => edge.from === boxId || edge.to === boxId);
}

export function describeEdge(edge) {
  const list = (names) => (names.length === 1 ? names[0] : `(${names.join(", ")})`);
  return `${edge.from}.${list(edge.columns)} → ${edge.to}.${list(edge.refColumns)}`;
}
