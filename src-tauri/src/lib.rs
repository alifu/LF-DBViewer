mod commands;
mod connection;
mod error;
mod state;
mod storage;

use tauri::Manager;

use commands::{connection as connection_commands, profile as profile_commands};
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
