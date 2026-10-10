/**
 * Builds the table and column names the SQL editor offers, from what the tree already
 * loaded (tables and views per database) and the column names fetched for the selected
 * database. Names are qualified (`db.table`) so tables of other databases complete too,
 * and `defaultSchema` lets the selected database's tables complete without a prefix.
 */
export function buildCompletionSchema({ database, objectsByDatabase, columnsByTable }) {
  const schema = {};
  for (const [name, objects] of Object.entries(objectsByDatabase)) {
    const tables = [...objects.tables.map((table) => table.name), ...objects.views];
    for (const table of tables) {
      schema[`${name}.${table}`] = name === database ? (columnsByTable[table] ?? []) : [];
    }
  }
  return { schema, defaultSchema: database || undefined };
}
