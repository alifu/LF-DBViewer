mod admin;
mod commands;
mod connection;
mod ddl;
mod error;
#[cfg(test)]
mod live_tests;
mod query;
mod schema;
mod sql;
mod sql_split;
mod sql_stream;
mod state;
mod storage;
mod transfer;

use tauri::Manager;

use commands::{
    admin as admin_commands, connection as connection_commands, data as data_commands,
    files as files_commands, profile as profile_commands, query as query_commands,
    schema as schema_commands, structure as structure_commands, transfer as transfer_commands,
};
use state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            app.manage(AppState::new(&data_dir));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            profile_commands::list_profiles,
            profile_commands::save_profile,
            profile_commands::delete_profile,
            profile_commands::detect_client_defaults,
            connection_commands::open_connection,
            connection_commands::test_connection,
            connection_commands::close_connection,
            connection_commands::ping_connection,
            connection_commands::reconnect_connection,
            connection_commands::list_connections,
            schema_commands::list_databases,
            schema_commands::list_objects,
            schema_commands::object_definition,
            schema_commands::database_diagram,
            files_commands::write_export_file,
            data_commands::browse_table,
            data_commands::apply_row_changes,
            query_commands::split_statements,
            query_commands::open_query_session,
            query_commands::close_query_session,
            query_commands::run_statements,
            query_commands::cancel_query,
            query_commands::list_database_columns,
            structure_commands::server_options,
            structure_commands::table_definition,
            structure_commands::preview_structure_change,
            structure_commands::apply_structure_change,
            structure_commands::routine_parameters,
            structure_commands::preview_definition,
            structure_commands::apply_definition,
            transfer_commands::export_database,
            transfer_commands::import_sql_file,
            transfer_commands::cancel_transfer,
            admin_commands::list_server_variables,
            admin_commands::list_server_status,
            admin_commands::list_processes,
            admin_commands::kill_process,
            admin_commands::list_engines,
            admin_commands::list_charsets,
            admin_commands::list_database_stats,
            admin_commands::list_users,
            admin_commands::show_user_grants,
            admin_commands::preview_admin_change,
            admin_commands::apply_admin_change,
            admin_commands::maintain_tables,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
