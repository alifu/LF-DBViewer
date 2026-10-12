import { Channel } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";

import * as transitions from "../state/app-state.js";

const SQL_FILTER = [{ name: "SQL", extensions: ["sql"] }];
const DUMP_FILTER = [{ name: "SQL or gzip-compressed SQL", extensions: ["sql", "gz"] }];
const CSV_FILTER = [{ name: "CSV or text", extensions: ["csv", "tsv", "txt"] }];

/** Exports and imports for one connection: file choosers, the long-running calls with their progress, and cancel. */
export function createTransferActions({ api, store, schema }) {
  const objectsOf = (connectionId, database) => transitions.schemaOf(store.getState(), connectionId).objects[database];

  async function run(command, connectionId, request, onEvent, label) {
    const channel = new Channel();
    channel.onmessage = onEvent;
    return api(command, { connectionId, request, onEvent: channel }, { label });
  }

  const cancel = (jobId) => api("cancel_transfer", { jobId }, { quiet: true }).catch(() => false);

  function exportData(tab) {
    const { connectionId, database } = tab;
    return {
      objects: () => {
        const objects = objectsOf(connectionId, database);
        return { tables: (objects?.tables ?? []).map((table) => table.name), views: objects?.views ?? [] };
      },
      pickPath: (defaultName) => save({ defaultPath: defaultName, filters: DUMP_FILTER }),
      start: (request, onEvent) =>
        run("export_database", connectionId, request, onEvent, (summary) => `Export ${database} → ${summary.path} (${summary.rows} rows${summary.cancelled ? ", cancelled" : ""})`),
      cancel,
    };
  }

  function importData(tab) {
    const { connectionId } = tab;
    return {
      databases: () => transitions.schemaOf(store.getState(), connectionId).databases.map((entry) => entry.name),
      pickFile: () => open({ multiple: false, directory: false, filters: [...DUMP_FILTER, { name: "All files", extensions: ["*"] }] }),
      start: async (request, onEvent) => {
        try {
          return await run("import_sql_file", connectionId, request, onEvent, (summary) => `Import ${request.path} (${summary.statements} statements${summary.failed ? ", with errors" : ""})`);
        } finally {
          // New or changed tables show up in the tree whether or not the import went all the way.
          await schema.loadDatabases(connectionId).catch(() => {});
        }
      },
      cancel,
    };
  }

  /** Loading a CSV file into one table. */
  function csvImportData(tab) {
    const { connectionId, database, table } = tab;
    return {
      tableColumns: async () => {
        const definition = await api("table_definition", { connectionId, database, table }, { label: `Read structure of ${table}` });
        return definition.columns.filter((column) => column.generated === null).map((column) => column.name);
      },
      pickFile: () => open({ multiple: false, directory: false, filters: [...CSV_FILTER, { name: "All files", extensions: ["*"] }] }),
      preview: (path, delimiter, hasHeader) => api("preview_csv_file", { path, delimiter, hasHeader }, { quiet: true }),
      start: (request, onEvent) => run("import_csv_file", connectionId, request, onEvent, (summary) => `Import ${request.path} into ${table} (${summary.rows} rows${summary.cancelled ? ", cancelled" : ""})`),
      cancel,
    };
  }

  return { exportData, importData, csvImportData, run, cancel };
}
