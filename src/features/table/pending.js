/**
 * Unsaved grid edits, kept apart from the loaded rows so they can be shown, discarded
 * or sent as one batch. All functions return a new value; nothing is mutated.
 *
 * Cells are `null`, a string, or `{ kind, preview, size }` for binary and oversized
 * values, which are shown but cannot be edited here.
 */

export const NO_CHANGES = Object.freeze({ edits: {}, deleted: {}, inserts: [] });

export function isSpecial(cell) {
  return cell !== null && typeof cell === "object";
}

function without(object, key) {
  const { [key]: _removed, ...rest } = object;
  return rest;
}

/** Records a new value for a loaded row; setting it back to the original drops the edit. */
export function editCell(pending, rowIndex, column, value, original) {
  const rowEdits = pending.edits[rowIndex] ?? {};
  const nextRow = value === original ? without(rowEdits, column) : { ...rowEdits, [column]: value };
  const edits = Object.keys(nextRow).length === 0 ? without(pending.edits, rowIndex) : { ...pending.edits, [rowIndex]: nextRow };
  return { ...pending, edits };
}

export function toggleDeleted(pending, rowIndex) {
  const deleted = pending.deleted[rowIndex] ? without(pending.deleted, rowIndex) : { ...pending.deleted, [rowIndex]: true };
  return { ...pending, deleted };
}

export function addInsert(pending) {
  return { ...pending, inserts: [...pending.inserts, {}] };
}

export function editInsert(pending, insertIndex, column, value) {
  return {
    ...pending,
    inserts: pending.inserts.map((values, index) => (index === insertIndex ? { ...values, [column]: value } : values)),
  };
}

export function removeInsert(pending, insertIndex) {
  return { ...pending, inserts: pending.inserts.filter((_values, index) => index !== insertIndex) };
}

export function changeCount(pending) {
  const edited = Object.keys(pending.edits).filter((rowIndex) => !pending.deleted[rowIndex]).length;
  return edited + Object.keys(pending.deleted).length + pending.inserts.length;
}

/**
 * Identifies a loaded row for the backend: its primary key, or every plain column when
 * the table has none. Binary and oversized cells cannot be compared, so a primary key
 * containing one is an error, while a keyless row simply leaves them out.
 */
function rowKey(row, columns, primaryKey) {
  const keyed = primaryKey.length > 0;
  const names = keyed ? primaryKey : columns.map((column) => column.name);
  const parts = names.map((name) => ({ column: name, value: row[columns.findIndex((column) => column.name === name)] }));
  if (keyed && parts.some((part) => isSpecial(part.value))) {
    throw new Error("This row's primary key holds binary data, so it cannot be edited here.");
  }
  const usable = parts.filter((part) => !isSpecial(part.value));
  if (usable.length === 0) throw new Error("This row has no plain values to identify it by.");
  return usable;
}

const toValues = (values) => Object.entries(values).map(([column, value]) => ({ column, value }));

/** The batch to send: deletes first, then updates, then inserts. A deleted row's edits are dropped. */
export function toChanges(pending, { rows, columns, primaryKey }) {
  const key = (rowIndex) => rowKey(rows[rowIndex], columns, primaryKey);
  const deletes = Object.keys(pending.deleted).map((rowIndex) => ({ kind: "delete", key: key(rowIndex) }));
  const updates = Object.entries(pending.edits)
    .filter(([rowIndex]) => !pending.deleted[rowIndex])
    .map(([rowIndex, values]) => ({ kind: "update", key: key(rowIndex), set: toValues(values) }));
  const inserts = pending.inserts.map((values) => ({ kind: "insert", values: toValues(values) }));
  return [...deletes, ...updates, ...inserts];
}
