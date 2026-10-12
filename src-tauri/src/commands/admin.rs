use tauri::State;

use super::structure::ExecutedStatements;
use crate::admin::change::AdminChange;
use crate::admin::grants::GrantEntry;
use crate::admin::maintenance::{MaintenanceOp, MaintenanceRow, maintain};
use crate::admin::server::{self, CharsetInfo, DatabaseStats, EngineInfo, NameValue, ProcessInfo};
use crate::admin::users::{self, UserAccount};
use crate::error::AppResult;
use crate::state::AppState;

macro_rules! read {
    ($state:expr, $connection_id:expr, $call:path) => {{
        let mut conn = $state
            .connections
            .get(&$connection_id)
            .await?
            .conn()
            .await?;
        $call(&mut conn).await
    }};
}

#[tauri::command]
pub async fn list_server_variables(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<Vec<NameValue>> {
    read!(state, connection_id, server::variables)
}

#[tauri::command]
pub async fn list_server_status(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<Vec<NameValue>> {
    read!(state, connection_id, server::status)
}

#[tauri::command]
pub async fn list_processes(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<Vec<ProcessInfo>> {
    read!(state, connection_id, server::processes)
}

#[tauri::command]
pub async fn kill_process(
    state: State<'_, AppState>,
    connection_id: String,
    process_id: u64,
    query_only: bool,
) -> AppResult<()> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    server::kill(&mut conn, process_id, query_only).await
}

#[tauri::command]
pub async fn list_engines(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<Vec<EngineInfo>> {
    read!(state, connection_id, server::engines)
}

#[tauri::command]
pub async fn list_charsets(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<Vec<CharsetInfo>> {
    read!(state, connection_id, server::charsets)
}

#[tauri::command]
pub async fn list_database_stats(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<Vec<DatabaseStats>> {
    read!(state, connection_id, server::database_stats)
}

#[tauri::command]
pub async fn list_users(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<Vec<UserAccount>> {
    read!(state, connection_id, users::list_users)
}

#[tauri::command]
pub async fn show_user_grants(
    state: State<'_, AppState>,
    connection_id: String,
    user: String,
    host: String,
) -> AppResult<Vec<GrantEntry>> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    users::show_grants(&mut conn, &user, &host).await
}

/// The SQL a change would run, with passwords masked.
#[tauri::command]
pub async fn preview_admin_change(change: AdminChange) -> AppResult<Vec<String>> {
    change.statements(true)
}

/// Runs the change. What comes back (and so what the log shows) has passwords masked.
#[tauri::command]
pub async fn apply_admin_change(
    state: State<'_, AppState>,
    connection_id: String,
    change: AdminChange,
) -> AppResult<ExecutedStatements> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    crate::ddl::apply::run_statements(&mut conn, &change.statements(false)?).await?;
    Ok(ExecutedStatements {
        statements: change.statements(true)?,
    })
}

#[tauri::command]
pub async fn maintain_tables(
    state: State<'_, AppState>,
    connection_id: String,
    operation: MaintenanceOp,
    database: String,
    tables: Vec<String>,
) -> AppResult<Vec<MaintenanceRow>> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    maintain(&mut conn, operation, &database, &tables).await
}
