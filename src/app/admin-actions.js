import { openMaintenanceDialog } from "../features/admin/admin-dialogs.js";
import { reviewSql } from "../features/structure/review-dialog.js";
import * as transitions from "../state/app-state.js";

/** Backend calls for one connection's server tab, and the preview-confirm-apply routine for changes. */
export function createAdminActions({ api, store }) {
  const sqlLabel = (result) => result.statements.join(";\n");

  function forConnection(connectionId) {
    const read = (command, label, extra = {}) => api(command, { connectionId, ...extra }, { label });
    const quiet = (command, extra = {}) => api(command, { connectionId, ...extra }, { quiet: true });
    return {
      engine: () => store.getState().connections.find((connection) => connection.id === connectionId)?.engine,
      databaseNames: () => transitions.schemaOf(store.getState(), connectionId).databases.map((entry) => entry.name),
      tableNames: (database) => (transitions.schemaOf(store.getState(), connectionId).objects[database]?.tables ?? []).map((table) => table.name),
      databases: () => read("list_database_stats", "Read database sizes"),
      variables: () => read("list_server_variables", "Read server variables"),
      status: () => quiet("list_server_status"),
      processes: () => quiet("list_processes"),
      kill: (processId, queryOnly) => read("kill_process", `${queryOnly ? "Stop query of" : "Kill"} connection ${processId}`, { processId, queryOnly }),
      engines: () => read("list_engines", "Read storage engines"),
      charsets: () => read("list_charsets", "Read character sets"),
      users: () => read("list_users", "Read accounts"),
      plugins: () => read("list_plugins", "Read plugins"),
      replication: () => read("replication_status", "Read replication status"),
      binaryLogs: () => read("list_binary_logs", "Read binary logs"),
      binlogEvents: (file, limit) => read("list_binlog_events", `Read events of ${file}`, { file, limit }),
      tableStats: (database) => read("list_table_stats", `Read table sizes of ${database}`, { database }),
      grants: (user, host) => read("show_user_grants", `Read privileges of ${user}@${host}`, { user, host }),
      maintain: (operation, database, tables) => read("maintain_tables", `${operation} ${tables.length} table(s) of ${database}`, { operation, database, tables }),
      /** Resolves true when the change was applied. The SQL shown and logged has passwords masked. */
      async change(change, review = {}) {
        const statements = await api("preview_admin_change", { change }, { quiet: true });
        if (!(await reviewSql({ statements, confirmLabel: "Run", ...review }))) return false;
        await api("apply_admin_change", { connectionId, change }, { label: sqlLabel });
        return true;
      },
    };
  }

  /** Table maintenance for a database; `selected` limits the tables ticked at first. */
  async function openMaintenance(connectionId, database, selected = null) {
    const data = forConnection(connectionId);
    let tables = data.tableNames(database);
    if (tables.length === 0) {
      const objects = await api("list_objects", { connectionId, database }, { quiet: true });
      tables = objects.tables.map((table) => table.name);
    }
    openMaintenanceDialog({ database, tables, selected: selected ?? tables, run: (operation, names) => data.maintain(operation, database, names) });
  }

  return { forConnection, openMaintenance };
}
