use serde::Serialize;
use tauri::State;

use crate::ddl::apply::run_statements;
use crate::ddl::change::StructureChange;
use crate::ddl::model::TableDefinition;
use crate::error::AppResult;
use crate::schema::ObjectKind;
use crate::schema::definition::{
    ServerOptions, read_table_definition, server_options as read_server_options,
};
use crate::schema::routines::{self, DefinitionRequest, RoutineParameter};
use crate::state::AppState;

#[derive(Serialize)]
pub struct ExecutedStatements {
    pub statements: Vec<String>,
}

#[tauri::command]
pub async fn server_options(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<ServerOptions> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    read_server_options(&mut conn).await
}

#[tauri::command]
pub async fn table_definition(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
) -> AppResult<TableDefinition> {
    let active = state.connections.get(&connection_id).await?;
    let mut conn = active.conn().await?;
    read_table_definition(&mut conn, active.engine(), &database, &table).await
}

/// The SQL a change would run, without running it. The connection says which engine's syntax to use.
#[tauri::command]
pub async fn preview_structure_change(
    state: State<'_, AppState>,
    connection_id: String,
    change: StructureChange,
) -> AppResult<Vec<String>> {
    change.statements(state.connections.get(&connection_id).await?.engine())
}

#[tauri::command]
pub async fn apply_structure_change(
    state: State<'_, AppState>,
    connection_id: String,
    change: StructureChange,
) -> AppResult<ExecutedStatements> {
    let active = state.connections.get(&connection_id).await?;
    let statements = change.statements(active.engine())?;
    let mut conn = active.conn().await?;
    run_statements(&mut conn, &statements).await?;
    Ok(ExecutedStatements { statements })
}

#[tauri::command]
pub async fn routine_parameters(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    name: String,
    kind: ObjectKind,
) -> AppResult<Vec<RoutineParameter>> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    routines::routine_parameters(&mut conn, &database, &name, kind).await
}

#[tauri::command]
pub async fn preview_definition(request: DefinitionRequest) -> AppResult<Vec<String>> {
    routines::definition_statements(&request)
}

#[tauri::command]
pub async fn apply_definition(
    state: State<'_, AppState>,
    connection_id: String,
    request: DefinitionRequest,
) -> AppResult<ExecutedStatements> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    let statements = routines::apply_definition(&mut conn, &request).await?;
    Ok(ExecutedStatements { statements })
}
