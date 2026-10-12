use tauri::State;
use tauri::ipc::Channel;

use crate::error::AppResult;
use crate::state::AppState;
use crate::transfer::csv_import::{
    CsvImportEvent, CsvImportRequest, CsvImportSummary, CsvPreview, import_csv, preview_csv,
};
use crate::transfer::export::{
    ExportEvent, ExportRequest, ExportSummary, export_database as run_export,
};
use crate::transfer::import::{ImportEvent, ImportRequest, ImportSummary, import_file};
use crate::transfer::result_export::{ResultExportRequest, ResultExportSummary, export_result};

/// Closing the connection is best effort: the server drops it anyway when the socket closes.
async fn close(conn: mysql_async::Conn) {
    let _ = conn.disconnect().await;
}

#[tauri::command]
pub async fn export_database(
    state: State<'_, AppState>,
    connection_id: String,
    request: ExportRequest,
    on_event: Channel<ExportEvent>,
) -> AppResult<ExportSummary> {
    let active = state.connections.get(&connection_id).await?;
    let cancel = state.jobs.start(&request.job_id)?;
    let emit = move |event: ExportEvent| {
        // If the window is gone nobody is watching; the export itself carries on.
        let _ = on_event.send(event);
    };
    let outcome = async {
        let mut conn = active.open_session_conn().await?;
        let result = run_export(&mut conn, &request, &cancel, &emit).await;
        close(conn).await;
        result
    }
    .await;
    state.jobs.finish(&request.job_id);
    outcome
}

#[tauri::command]
pub async fn import_sql_file(
    state: State<'_, AppState>,
    connection_id: String,
    request: ImportRequest,
    on_event: Channel<ImportEvent>,
) -> AppResult<ImportSummary> {
    let active = state.connections.get(&connection_id).await?;
    let cancel = state.jobs.start(&request.job_id)?;
    let emit = move |event: ImportEvent| {
        let _ = on_event.send(event);
    };
    let outcome = async {
        let mut conn = active.open_session_conn().await?;
        let result = import_file(&mut conn, &request, &cancel, &emit).await;
        close(conn).await;
        result
    }
    .await;
    state.jobs.finish(&request.job_id);
    outcome
}

/// Asks a running export or import to stop. Returns false when no such job is running.
#[tauri::command]
pub async fn cancel_transfer(state: State<'_, AppState>, job_id: String) -> AppResult<bool> {
    state.jobs.cancel(&job_id)
}

#[tauri::command]
pub async fn preview_csv_file(
    path: String,
    delimiter: String,
    has_header: bool,
) -> AppResult<CsvPreview> {
    // Reading a file is blocking work.
    tokio::task::spawn_blocking(move || preview_csv(&path, &delimiter, has_header))
        .await
        .map_err(|error| crate::error::AppError::Storage(error.to_string()))?
}

#[tauri::command]
pub async fn import_csv_file(
    state: State<'_, AppState>,
    connection_id: String,
    request: CsvImportRequest,
    on_event: Channel<CsvImportEvent>,
) -> AppResult<CsvImportSummary> {
    let active = state.connections.get(&connection_id).await?;
    let cancel = state.jobs.start(&request.job_id)?;
    let emit = move |event: CsvImportEvent| {
        let _ = on_event.send(event);
    };
    let outcome = async {
        let mut conn = active.open_session_conn().await?;
        let result = import_csv(&mut conn, &request, &cancel, &emit).await;
        close(conn).await;
        result
    }
    .await;
    state.jobs.finish(&request.job_id);
    outcome
}

/// Runs a query again and writes everything it returns to a CSV or JSON file.
#[tauri::command]
pub async fn export_query_result(
    state: State<'_, AppState>,
    connection_id: String,
    request: ResultExportRequest,
    on_event: Channel<ExportEvent>,
) -> AppResult<ResultExportSummary> {
    let active = state.connections.get(&connection_id).await?;
    let cancel = state.jobs.start(&request.job_id)?;
    let emit = move |event: ExportEvent| {
        let _ = on_event.send(event);
    };
    let outcome = async {
        let mut conn = active.open_session_conn().await?;
        let result = export_result(&mut conn, &request, &cancel, &emit).await;
        close(conn).await;
        result
    }
    .await;
    state.jobs.finish(&request.job_id);
    outcome
}
