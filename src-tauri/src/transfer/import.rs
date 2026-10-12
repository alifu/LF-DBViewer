//! Runs a SQL file against a database, reading it in pieces so its size does not matter.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mysql_async::Conn;
use mysql_async::prelude::Queryable;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::AsyncReadExt;

use crate::error::{AppError, AppResult};
use crate::sql::quote_identifier;
use crate::sql_stream::{PendingStatement, StatementStream, Utf8Decoder};

const READ_CHUNK_BYTES: usize = 1024 * 1024;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const MAX_RECORDED_ERRORS: usize = 100;
const SQL_PREVIEW_CHARS: usize = 300;

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportRequest {
    pub job_id: String,
    pub path: String,
    /// Load into this database. `None` leaves it to the file (`CREATE DATABASE` / `USE` inside it).
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default = "yes")]
    pub create_database: bool,
    #[serde(default = "yes")]
    pub stop_on_error: bool,
    #[serde(default = "yes")]
    pub disable_foreign_key_checks: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportError {
    pub statement: u64,
    pub line: usize,
    pub message: String,
    pub sql: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ImportEvent {
    Started {
        total_bytes: u64,
    },
    Progress {
        bytes_read: u64,
        total_bytes: u64,
        statements: u64,
        line: usize,
    },
    StatementFailed {
        error: ImportError,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportSummary {
    pub statements: u64,
    pub bytes: u64,
    pub duration_ms: u64,
    pub cancelled: bool,
    pub failed: bool,
    pub errors: Vec<ImportError>,
}

pub type Emit<'a> = &'a (dyn Fn(ImportEvent) + Send + Sync);

fn preview(sql: &str) -> String {
    match sql.char_indices().nth(SQL_PREVIEW_CHARS) {
        Some((cut, _)) => format!("{}…", &sql[..cut]),
        None => sql.to_owned(),
    }
}

struct Run<'a> {
    request: &'a ImportRequest,
    cancel: &'a AtomicBool,
    emit: Emit<'a>,
    total_bytes: u64,
    bytes_read: u64,
    statements: u64,
    errors: Vec<ImportError>,
    failed: bool,
    cancelled: bool,
    last_report: Instant,
    line: usize,
}

impl Run<'_> {
    fn stopped(&self) -> bool {
        self.cancelled || (self.failed && self.request.stop_on_error)
    }

    fn report(&mut self, force: bool) {
        if force || self.last_report.elapsed() >= PROGRESS_EVERY {
            self.last_report = Instant::now();
            (self.emit)(ImportEvent::Progress {
                bytes_read: self.bytes_read,
                total_bytes: self.total_bytes,
                statements: self.statements,
                line: self.line,
            });
        }
    }

    async fn execute(&mut self, conn: &mut Conn, pending: Vec<PendingStatement>) {
        for statement in pending {
            if self.stopped() {
                return;
            }
            if self.cancel.load(Ordering::SeqCst) {
                self.cancelled = true;
                return;
            }
            self.line = statement.line;
            self.statements += 1;
            if let Err(error) = conn.query_drop(statement.text.as_str()).await {
                let failure = ImportError {
                    statement: self.statements,
                    line: statement.line,
                    message: error.to_string(),
                    sql: preview(&statement.text),
                };
                (self.emit)(ImportEvent::StatementFailed {
                    error: failure.clone(),
                });
                self.failed = true;
                if self.errors.len() < MAX_RECORDED_ERRORS {
                    self.errors.push(failure);
                }
                // A lost connection ends the run whatever the setting says.
                if error.is_fatal() {
                    self.request_stop();
                    return;
                }
            }
            self.report(false);
        }
    }

    fn request_stop(&mut self) {
        self.cancelled = true;
    }
}

async fn prepare(conn: &mut Conn, request: &ImportRequest) -> AppResult<()> {
    conn.query_drop("SET NAMES utf8mb4").await?;
    if request.disable_foreign_key_checks {
        conn.query_drop("SET FOREIGN_KEY_CHECKS = 0").await?;
    }
    if let Some(database) = request
        .database
        .as_deref()
        .filter(|name| !name.trim().is_empty())
    {
        let name = quote_identifier(database);
        if request.create_database {
            conn.query_drop(format!("CREATE DATABASE IF NOT EXISTS {name}"))
                .await?;
        }
        conn.query_drop(format!("USE {name}")).await?;
    }
    Ok(())
}

pub async fn import_file(
    conn: &mut Conn,
    request: &ImportRequest,
    cancel: &AtomicBool,
    emit: Emit<'_>,
) -> AppResult<ImportSummary> {
    if request.path.trim().is_empty() {
        return Err(AppError::Validation("Choose a SQL file to import".into()));
    }
    let started = Instant::now();
    let mut file = File::open(&request.path)
        .await
        .map_err(|error| AppError::Validation(format!("Cannot open {}: {error}", request.path)))?;
    let total_bytes = file.metadata().await?.len();
    prepare(conn, request).await?;
    emit(ImportEvent::Started { total_bytes });

    let mut run = Run {
        request,
        cancel,
        emit,
        total_bytes,
        bytes_read: 0,
        statements: 0,
        errors: Vec::new(),
        failed: false,
        cancelled: false,
        last_report: Instant::now(),
        line: 1,
    };
    let (mut stream, mut decoder) = (StatementStream::default(), Utf8Decoder::default());
    let mut buffer = vec![0u8; READ_CHUNK_BYTES];

    while !run.stopped() {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        run.bytes_read += read as u64;
        let pending = stream.push(&decoder.push(&buffer[..read]));
        run.execute(conn, pending).await;
        run.report(false);
    }
    if !run.stopped() {
        let mut pending = stream.push(&decoder.finish());
        pending.extend(stream.finish());
        run.execute(conn, pending).await;
    }
    run.report(true);

    Ok(ImportSummary {
        statements: run.statements,
        bytes: run.bytes_read,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        cancelled: run.cancelled && !run.failed,
        failed: run.failed,
        errors: run.errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_statements_are_shortened_for_the_error_report_on_a_character_boundary() {
        let long = "é".repeat(SQL_PREVIEW_CHARS + 50);

        let shown = preview(&long);

        assert_eq!(shown.chars().count(), SQL_PREVIEW_CHARS + 1);
        assert!(shown.ends_with('…'));
        assert_eq!(preview("SELECT 1"), "SELECT 1");
    }

    #[test]
    fn a_request_has_safe_defaults() {
        let request: ImportRequest =
            serde_json::from_str(r#"{"jobId":"j","path":"/tmp/x.sql"}"#).unwrap();

        assert!(
            request.stop_on_error && request.disable_foreign_key_checks && request.create_database
        );
        assert_eq!(request.database, None);
    }
}
