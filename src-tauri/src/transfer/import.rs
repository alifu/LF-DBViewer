//! Runs a SQL file against a database, reading it in pieces so its size does not matter.

use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use async_compression::tokio::bufread::GzipDecoder;
use mysql_async::Conn;
use mysql_async::prelude::Queryable;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncSeekExt, BufReader};

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

/// The first two bytes of a gzip file.
const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

pub fn is_gzip(start: &[u8]) -> bool {
    start.starts_with(&GZIP_MAGIC)
}

/// Counts the bytes read from the file itself, so progress is right for a compressed file too.
struct CountingReader<R> {
    inner: R,
    count: Arc<AtomicU64>,
}

impl<R: AsyncRead + Unpin> AsyncRead for CountingReader<R> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let before = buffer.filled().len();
        let poll = Pin::new(&mut self.inner).poll_read(context, buffer);
        if let Poll::Ready(Ok(())) = &poll {
            self.count
                .fetch_add((buffer.filled().len() - before) as u64, Ordering::Relaxed);
        }
        poll
    }
}

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
    let mut magic = [0u8; 2];
    let compressed = file.read(&mut magic).await? == 2 && is_gzip(&magic);
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let position = Arc::new(AtomicU64::new(0));
    let counted = BufReader::new(CountingReader {
        inner: file,
        count: Arc::clone(&position),
    });
    let mut reader: Pin<Box<dyn AsyncRead + Send>> = if compressed {
        let mut decoder = GzipDecoder::new(counted);
        decoder.multiple_members(true);
        Box::pin(decoder)
    } else {
        Box::pin(counted)
    };

    while !run.stopped() {
        let read = reader.read(&mut buffer).await?;
        run.bytes_read = position.load(Ordering::Relaxed);
        if read == 0 {
            break;
        }
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

    #[tokio::test]
    async fn gzip_files_are_recognised_decoded_and_counted_by_their_compressed_size() {
        use async_compression::tokio::write::GzipEncoder;
        use tokio::io::AsyncWriteExt;

        let text = "INSERT INTO t VALUES (1);\n".repeat(500);
        let mut encoder = GzipEncoder::new(Vec::new());
        encoder.write_all(text.as_bytes()).await.unwrap();
        encoder.shutdown().await.unwrap();
        let compressed = encoder.into_inner();

        let count = Arc::new(AtomicU64::new(0));
        let source = BufReader::new(CountingReader {
            inner: std::io::Cursor::new(compressed.clone()),
            count: Arc::clone(&count),
        });
        let mut decoder = GzipDecoder::new(source);
        let mut decoded = String::new();
        decoder.read_to_string(&mut decoded).await.unwrap();

        assert!(is_gzip(&compressed) && !is_gzip(b"IN") && !is_gzip(b""));
        assert_eq!(decoded, text);
        assert_eq!(count.load(Ordering::Relaxed), compressed.len() as u64);
        assert!(compressed.len() < text.len() / 10);
    }
}
