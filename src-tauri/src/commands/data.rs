use tauri::State;

use crate::error::AppResult;
use crate::query::browse::{self, PageRequest, TablePage};
use crate::query::edit::{self, ApplyResult, RowChange};
use crate::state::AppState;

#[tauri::command]
pub async fn browse_table(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
    page: PageRequest,
) -> AppResult<TablePage> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    browse::browse_table(&mut conn, &database, &table, &page).await
}

#[tauri::command]
pub async fn apply_row_changes(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
    changes: Vec<RowChange>,
) -> AppResult<ApplyResult> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    edit::apply_changes(&mut conn, &database, &table, &changes).await
}
