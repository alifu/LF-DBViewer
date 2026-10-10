mod commands;
mod connection;
mod error;
#[cfg(test)]
mod live_tests;
mod query;
mod schema;
mod sql;
mod sql_split;
mod state;
mod storage;

use tauri::Manager;

use commands::{
    connection as connection_commands, data as data_commands, profile as profile_commands,
    query as query_commands, schema as schema_commands,
};
use state::AppState;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
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
            data_commands::browse_table,
            data_commands::apply_row_changes,
            query_commands::split_statements,
            query_commands::open_query_session,
            query_commands::close_query_session,
            query_commands::run_statements,
            query_commands::cancel_query,
            query_commands::list_database_columns,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
