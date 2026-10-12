use tauri::State;

use crate::error::AppResult;
use crate::query::browse::{self, CellValue, Filter, PageRequest, TablePage};
use crate::query::edit::{self, ApplyResult, ColumnValue, RowChange};
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

#[tauri::command]
pub async fn count_table_rows(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
    filters: Vec<Filter>,
) -> AppResult<u64> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    browse::count_rows(&mut conn, &database, &table, &filters).await
}

#[tauri::command]
pub async fn read_table_cell(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
    table: String,
    column: String,
    key: Vec<ColumnValue>,
) -> AppResult<CellValue> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    browse::read_cell(&mut conn, &database, &table, &column, &key).await
}
