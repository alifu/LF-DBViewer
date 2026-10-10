use std::collections::BTreeMap;
use std::sync::atomic::Ordering;

use tauri::State;
use tauri::ipc::Channel;

use crate::error::AppResult;
use crate::query::run::{self, MAX_RESULT_ROWS, QueryEvent, RunSummary};
use crate::schema;
use crate::sql_split::{Statement, split_statements as split};
use crate::state::AppState;

#[tauri::command]
pub async fn split_statements(sql: String) -> AppResult<Vec<Statement>> {
    Ok(split(&sql))
}

#[tauri::command]
pub async fn open_query_session(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<String> {
    // Fail now, with a clear message, if the connection is not open.
    state.connections.get(&connection_id).await?;
    Ok(state.sessions.open(&connection_id).await)
}

#[tauri::command]
pub async fn close_query_session(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    state.sessions.close(&session_id).await;
    Ok(())
}

#[tauri::command]
pub async fn run_statements(
    state: State<'_, AppState>,
    session_id: String,
    database: Option<String>,
    statements: Vec<String>,
    on_event: Channel<QueryEvent>,
) -> AppResult<RunSummary> {
    let session = state.sessions.get(&session_id).await?;
    let active = state.connections.get(&session.connection_id).await?;
    // If the window is gone nobody is listening, and the run then simply finishes.
    let emit = move |event: QueryEvent| {
        let _ = on_event.send(event);
    };
    run::run_statements(
        &session,
        active.open_session_conn(),
        database.as_deref(),
        &statements,
        MAX_RESULT_ROWS,
        &emit,
    )
    .await
}

#[tauri::command]
pub async fn cancel_query(state: State<'_, AppState>, session_id: String) -> AppResult<()> {
    let session = state.sessions.get(&session_id).await?;
    session.cancel_requested.store(true, Ordering::SeqCst);
    let thread_id = session.thread_id.load(Ordering::SeqCst);
    if thread_id == 0 {
        return Ok(());
    }
    state
        .connections
        .get(&session.connection_id)
        .await?
        .kill_query(thread_id)
        .await
}

#[tauri::command]
pub async fn list_database_columns(
    state: State<'_, AppState>,
    connection_id: String,
    database: String,
) -> AppResult<BTreeMap<String, Vec<String>>> {
    let mut conn = state.connections.get(&connection_id).await?.conn().await?;
    schema::database_column_names(&mut conn, &database).await
}
