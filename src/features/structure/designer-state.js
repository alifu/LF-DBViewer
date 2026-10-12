/**
 * The table designer's data and the edits made to it. A table is the same JSON the backend
 * sends and receives (`TableDefinition`); every function here returns a new table.
 */

export const COMMON_TYPES = Object.freeze([
  "int", "int unsigned", "bigint", "bigint unsigned", "smallint", "tinyint", "tinyint(1)", "decimal(10,2)", "float", "double",
  "varchar(255)", "char(36)", "text", "mediumtext", "longtext", "blob", "json", "boolean",
  "date", "datetime", "timestamp", "time", "year", "enum('a','b')",
]);

export const ROW_FORMATS = Object.freeze(["DYNAMIC", "COMPACT", "REDUNDANT", "COMPRESSED", "FIXED"]);
export const INDEX_KINDS = Object.freeze([
  { value: "index", label: "Index" },
  { value: "unique", label: "Unique" },
  { value: "fulltext", label: "Fulltext" },
  { value: "spatial", label: "Spatial" },
]);
export const REFERENTIAL_ACTIONS = Object.freeze([
  { value: "restrict", label: "RESTRICT" },
  { value: "cascade", label: "CASCADE" },
  { value: "setNull", label: "SET NULL" },
  { value: "noAction", label: "NO ACTION" },
  { value: "setDefault", label: "SET DEFAULT" },
]);

const TEXT_TYPE = /^(char|varchar|tinytext|text|mediumtext|longtext|enum|set)\b/i;

/** Only text columns have a collation. */
export function isTextType(columnType) {
  return TEXT_TYPE.test(columnType.trim());
}

export function uniqueName(taken, base) {
  const lower = new Set(taken.map((name) => name.toLowerCase()));
  if (!lower.has(base.toLowerCase())) return base;
  let counter = 2;
  while (lower.has(`${base}_${counter}`.toLowerCase())) counter += 1;
  return `${base}_${counter}`;
}

const blankColumn = (name) => ({
  originalName: null,
  name,
  columnType: "varchar(255)",
  nullable: true,
  default: { kind: "null" },
  autoIncrement: false,
  onUpdate: null,
  collation: null,
  comment: "",
  generated: null,
  invisible: false,
});

export function emptyTable(name) {
  return {
    originalName: null,
    name,
    columns: [{ ...blankColumn("id"), columnType: "bigint unsigned", nullable: false, default: { kind: "none" }, autoIncrement: true }],
    indexes: [{ originalName: null, name: "PRIMARY", kind: "primary", columns: [{ name: "id", length: null, descending: false }] }],
    foreignKeys: [],
    checks: [],
    options: { engine: "InnoDB", rowFormat: null, autoIncrement: null, collation: null, comment: "" },
  };
}

// ---- columns ----------------------------------------------------------------

export function addColumn(table) {
  const name = uniqueName(table.columns.map((column) => column.name), "column");
  return { ...table, columns: [...table.columns, blankColumn(name)] };
}

export function updateColumn(table, position, patch) {
  return { ...table, columns: table.columns.map((column, index) => (index === position ? { ...column, ...patch } : column)) };
}

/** Removing a column also removes it from indexes (dropping those left empty) and drops keys that used it. */
export function removeColumn(table, position) {
  const removed = table.columns[position].name;
  const indexes = table.indexes
    .map((index) => ({ ...index, columns: index.columns.filter((column) => column.name !== removed) }))
    .filter((index) => index.columns.length > 0);
  return {
    ...table,
    columns: table.columns.filter((_column, index) => index !== position),
    indexes,
    foreignKeys: table.foreignKeys.filter((key) => !key.columns.includes(removed)),
  };
}

export function moveColumn(table, position, step) {
  const target = position + step;
  if (target < 0 || target >= table.columns.length) return table;
  const columns = [...table.columns];
  [columns[position], columns[target]] = [columns[target], columns[position]];
  return { ...table, columns };
}

/** Renaming keeps indexes and foreign keys pointing at the column. */
export function renameColumn(table, position, newName) {
  const oldName = table.columns[position].name;
  if (oldName === newName) return table;
  const rename = (name) => (name === oldName ? newName : name);
  const selfReference = (key) => key.refTable === table.name && !key.refDatabase;
  return {
    ...table,
    columns: table.columns.map((column, index) => (index === position ? { ...column, name: newName } : column)),
    indexes: table.indexes.map((index) => ({ ...index, columns: index.columns.map((column) => ({ ...column, name: rename(column.name) })) })),
    foreignKeys: table.foreignKeys.map((key) => ({
      ...key,
      columns: key.columns.map(rename),
      refColumns: selfReference(key) ? key.refColumns.map(rename) : key.refColumns,
    })),
  };
}

export function primaryKeyColumns(table) {
  return table.indexes.find((index) => index.kind === "primary")?.columns.map((column) => column.name) ?? [];
}

/** A primary key column cannot be NULL, so adding one also makes the column NOT NULL. */
export function setPrimaryKey(table, columnName, enabled) {
  const current = primaryKeyColumns(table);
  const names = enabled ? [...current.filter((name) => name !== columnName), columnName] : current.filter((name) => name !== columnName);
  const others = table.indexes.filter((index) => index.kind !== "primary");
  const existing = table.indexes.find((index) => index.kind === "primary");
  const primary = names.length === 0 ? [] : [{
    originalName: existing?.originalName ?? null,
    name: "PRIMARY",
    kind: "primary",
    columns: names.map((name) => existing?.columns.find((column) => column.name === name) ?? { name, length: null, descending: false }),
  }];
  const columns = enabled
    ? table.columns.map((column) =>
        column.name === columnName ? { ...column, nullable: false, default: column.default.kind === "null" ? { kind: "none" } : column.default } : column,
      )
    : table.columns;
  return { ...table, columns, indexes: [...primary, ...others] };
}

export function setDefault(column, kind, value = "") {
  return { ...column, default: kind === "value" || kind === "expression" ? { kind, value } : { kind } };
}

// ---- indexes and foreign keys --------------------------------------------------

export function addIndex(table) {
  const name = uniqueName(table.indexes.map((index) => index.name), "idx");
  const first = table.columns[0]?.name;
  return { ...table, indexes: [...table.indexes, { originalName: null, name, kind: "index", columns: first ? [{ name: first, length: null, descending: false }] : [] }] };
}

export function updateIndex(table, position, patch) {
  return { ...table, indexes: table.indexes.map((index, at) => (at === position ? { ...index, ...patch } : index)) };
}

export function removeIndex(table, position) {
  return { ...table, indexes: table.indexes.filter((_index, at) => at !== position) };
}

export function addForeignKey(table) {
  const name = uniqueName(table.foreignKeys.map((key) => key.name), `fk_${table.name || "table"}`);
  const first = table.columns[0]?.name;
  return {
    ...table,
    foreignKeys: [
      ...table.foreignKeys,
      { originalName: null, name, columns: first ? [first] : [], refDatabase: null, refTable: "", refColumns: [], onUpdate: "restrict", onDelete: "restrict" },
    ],
  };
}

export function updateForeignKey(table, position, patch) {
  return { ...table, foreignKeys: table.foreignKeys.map((key, at) => (at === position ? { ...key, ...patch } : key)) };
}

export function removeForeignKey(table, position) {
  return { ...table, foreignKeys: table.foreignKeys.filter((_key, at) => at !== position) };
}

export function addCheck(table) {
  const taken = (table.checks ?? []).map((check) => check.name);
  return { ...table, checks: [...(table.checks ?? []), { originalName: null, name: uniqueName(taken, "chk"), expression: "" }] };
}

export function updateCheck(table, position, patch) {
  return { ...table, checks: table.checks.map((check, at) => (at === position ? { ...check, ...patch } : check)) };
}

export function removeCheck(table, position) {
  return { ...table, checks: table.checks.filter((_check, at) => at !== position) };
}

export function updateOptions(table, patch) {
  return { ...table, options: { ...table.options, ...patch } };
}

// ---- text forms of column lists -------------------------------------------------

export function parseNameList(text) {
  return text
    .split(",")
    .map((part) => part.trim().replace(/^`|`$/g, ""))
    .filter((part) => part !== "");
}

export function formatNameList(names) {
  return names.join(", ");
}

const INDEX_COLUMN = /^`?([^`(]+?)`?\s*(?:\((\d+)\))?\s*(asc|desc)?$/i;

/** `name`, `name(20)` for a prefix length, and `name DESC` for a descending column. */
export function parseIndexColumns(text) {
  return parseNameList(text).map((part) => {
    const match = INDEX_COLUMN.exec(part);
    if (!match) return { name: part, length: null, descending: false };
    return { name: match[1].trim(), length: match[2] ? Number(match[2]) : null, descending: match[3]?.toLowerCase() === "desc" };
  });
}

export function formatIndexColumns(columns) {
  return columns.map((column) => `${column.name}${column.length ? `(${column.length})` : ""}${column.descending ? " DESC" : ""}`).join(", ");
}

export function hasChanges(original, table) {
  return JSON.stringify(original) !== JSON.stringify(table);
}
