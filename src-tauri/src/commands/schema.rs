use tauri::State;

use crate::error::AppResult;
use crate::schema::diagram::{Diagram, read_diagram};
use crate::schema::{self, DatabaseInfo, ObjectKind, SchemaObjects};
use crate::state::AppState;

#[tauri::command]
pub async fn list_databases(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<Vec<DatabaseInfo>> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    schema::list_databases(&mut conn).await
}

#[tauri::command]
pub async fn list_objects(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> AppResult<SchemaObjects> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    schema::list_objects(&mut conn, &database).await
}

#[tauri::command]
pub async fn object_definition(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    name: String,
    kind: ObjectKind,
) -> AppResult<String> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    schema::object_definition(&mut conn, &database, &name, kind).await
}

#[tauri::command]
pub async fn database_diagram(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> AppResult<Diagram> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    read_diagram(&mut conn, &database).await
}
