//! Writes a database (or some of its tables) to a SQL file that loads back without trouble.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_compression::tokio::write::GzipEncoder;
use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncWrite, AsyncWriteExt, BufWriter};

use super::text::{DEFAULT_STATEMENT_BYTES, FOOTER, InsertBatcher, header, section, strip_definer};
use super::value::{DumpColumn, row_literal};
use crate::error::{AppError, AppResult};
use crate::schema::{table_columns, texts};
use crate::sql::{qualified, quote_identifier, quote_string};

const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const CANCEL_CHECK_ROWS: u64 = 500;
const WRITE_BUFFER_BYTES: usize = 256 * 1024;
const NET_TIMEOUT_SECONDS: u32 = 3600;
const PARTIAL_SUFFIX: &str = ".partial";

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportRequest {
    pub job_id: String,
    pub database: String,
    pub path: String,
    /// Base tables to write.
    #[serde(default)]
    pub tables: Vec<String>,
    #[serde(default)]
    pub views: Vec<String>,
    #[serde(default = "yes")]
    pub structure: bool,
    #[serde(default = "yes")]
    pub data: bool,
    /// Put `DROP … IF EXISTS` before each table, so loading over an existing database replaces it.
    #[serde(default = "yes")]
    pub add_drop: bool,
    /// Start with `CREATE DATABASE IF NOT EXISTS` and `USE`, so the file restores by itself.
    #[serde(default)]
    pub create_database: bool,
    #[serde(default)]
    pub routines: bool,
    #[serde(default)]
    pub triggers: bool,
    #[serde(default)]
    pub events: bool,
    /// Keep `DEFINER=…` in views, routines, triggers and events.
    #[serde(default)]
    pub keep_definers: bool,
    /// Read every table from one point in time (InnoDB), so related tables agree with each other.
    #[serde(default = "yes")]
    pub consistent_snapshot: bool,
    #[serde(default)]
    pub max_statement_bytes: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ExportEvent {
    Started {
        tables: usize,
    },
    TableStarted {
        name: String,
        index: usize,
        estimated_rows: Option<u64>,
    },
    Progress {
        table: String,
        rows: u64,
        bytes: u64,
    },
    TableFinished {
        name: String,
        rows: u64,
    },
    Note {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportSummary {
    pub path: String,
    pub bytes: u64,
    pub tables: usize,
    pub rows: u64,
    pub duration_ms: u64,
    pub cancelled: bool,
}

pub type Emit<'a> = &'a (dyn Fn(ExportEvent) + Send + Sync);

/// Where the text goes: the file, compressed on the way when its name ends in `.gz`.
struct Output {
    file: std::pin::Pin<Box<dyn AsyncWrite + Send>>,
    /// Characters written so far, before compression.
    bytes: u64,
}

pub fn is_gzip_path(path: &str) -> bool {
    path.to_ascii_lowercase().ends_with(".gz")
}

impl Output {
    async fn write(&mut self, text: &str) -> AppResult<()> {
        self.file.write_all(text.as_bytes()).await?;
        self.bytes += text.len() as u64;
        Ok(())
    }
}

/// `2026-10-12 10:00:00 UTC` from seconds since the epoch.
pub fn utc_timestamp(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let (hour, minute, second) = (
        (seconds % 86_400) / 3600,
        (seconds % 3600) / 60,
        seconds % 60,
    );
    // Civil date from a day count (Howard Hinnant's algorithm).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

/// Runs a `SHOW CREATE …` and returns its columns by heading.
async fn show_create(
    conn: &mut Conn,
    sql: String,
) -> AppResult<std::collections::HashMap<String, String>> {
    let row: Option<Row> = conn.query_first(sql).await?;
    let row = row.ok_or_else(|| AppError::Database("the object no longer exists".into()))?;
    let names: Vec<String> = row
        .columns_ref()
        .iter()
        .map(|column| column.name_str().into_owned())
        .collect();
    Ok(names
        .into_iter()
        .zip(texts(row))
        .filter_map(|(name, value)| value.map(|value| (name, value)))
        .collect())
}

fn keep_or_strip(request: &ExportRequest, sql: &str) -> String {
    if request.keep_definers {
        sql.to_owned()
    } else {
        strip_definer(sql)
    }
}

/// Statements that must be sent whole, wrapped so a `;` inside a body does not end them.
fn delimited(statements: &[String]) -> String {
    let mut text = String::from("DELIMITER ;;\n");
    for statement in statements {
        text.push_str(statement);
        text.push_str(";;\n");
    }
    text.push_str("DELIMITER ;\n");
    text
}

fn with_sql_mode(sql_mode: Option<&String>, create: String) -> Vec<String> {
    match sql_mode {
        Some(mode) => vec![
            "SET @saved_sql_mode = @@sql_mode".to_owned(),
            format!("SET sql_mode = {}", quote_string(mode)),
            create,
            "SET sql_mode = @saved_sql_mode".to_owned(),
        ],
        None => vec![create],
    }
}

async fn setup_session(conn: &mut Conn, request: &ExportRequest) -> AppResult<()> {
    conn.query_drop("SET NAMES utf8mb4").await?;
    conn.query_drop("SET SESSION time_zone = '+00:00'").await?;
    conn.query_drop(format!(
        "SET SESSION net_write_timeout = {NET_TIMEOUT_SECONDS}"
    ))
    .await?;
    conn.query_drop(format!(
        "SET SESSION net_read_timeout = {NET_TIMEOUT_SECONDS}"
    ))
    .await?;
    if request.consistent_snapshot {
        conn.query_drop("SET SESSION TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .await?;
        conn.query_drop("START TRANSACTION WITH CONSISTENT SNAPSHOT")
            .await?;
    }
    Ok(())
}

async fn estimated_rows(conn: &mut Conn, database: &str, table: &str) -> Option<u64> {
    let rows: Vec<Row> = conn
        .exec("SELECT TABLE_ROWS FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ?", (database, table))
        .await
        .ok()?;
    rows.into_iter()
        .next()
        .and_then(|row| texts(row).into_iter().next().flatten())
        .and_then(|text| text.parse().ok())
}

struct TableDump<'a> {
    request: &'a ExportRequest,
    cancel: &'a AtomicBool,
    emit: Emit<'a>,
}

impl TableDump<'_> {
    async fn structure(&self, conn: &mut Conn, out: &mut Output, table: &str) -> AppResult<()> {
        out.write(&section(&format!(
            "Table structure for table {}",
            quote_identifier(table)
        )))
        .await?;
        if self.request.add_drop {
            out.write(&format!(
                "DROP TABLE IF EXISTS {};\n",
                quote_identifier(table)
            ))
            .await?;
        }
        let created = show_create(
            conn,
            format!(
                "SHOW CREATE TABLE {}",
                qualified(&self.request.database, table)
            ),
        )
        .await?;
        let sql = created
            .get("Create Table")
            .ok_or_else(|| AppError::Database(format!("no definition for {table}")))?;
        out.write(&format!("{sql};\n")).await
    }

    /// Streams the rows as batched inserts. Returns the number of rows, and whether it stopped early.
    async fn data(&self, conn: &mut Conn, out: &mut Output, table: &str) -> AppResult<(u64, bool)> {
        let columns: Vec<String> = table_columns(conn, &self.request.database, table)
            .await?
            .into_iter()
            .filter(|column| !column.generated)
            .map(|column| column.name)
            .collect();
        if columns.is_empty() {
            return Ok((0, false));
        }
        out.write(&section(&format!(
            "Dumping data for table {}",
            quote_identifier(table)
        )))
        .await?;
        let select = format!(
            "SELECT {} FROM {}",
            columns
                .iter()
                .map(|name| quote_identifier(name))
                .collect::<Vec<_>>()
                .join(","),
            qualified(&self.request.database, table)
        );
        let mut result = conn.query_iter(select).await?;
        let dump_columns: Vec<DumpColumn> = result
            .columns()
            .map(|columns| columns.iter().map(DumpColumn::of).collect())
            .unwrap_or_default();
        let mut batcher = InsertBatcher::new(
            table,
            &columns,
            self.request
                .max_statement_bytes
                .unwrap_or(DEFAULT_STATEMENT_BYTES),
        );
        let (mut rows, mut last_report) = (0u64, Instant::now());

        while let Some(row) = result.next().await? {
            rows += 1;
            if let Some(statement) = batcher.push(&row_literal(&row.unwrap(), &dump_columns)) {
                out.write(&statement).await?;
            }
            if rows % CANCEL_CHECK_ROWS == 0 {
                if self.cancel.load(Ordering::SeqCst) {
                    // The rest of the result must be read before the connection can be used again; the caller closes it.
                    return Ok((rows, true));
                }
                if last_report.elapsed() >= PROGRESS_EVERY {
                    (self.emit)(ExportEvent::Progress {
                        table: table.to_owned(),
                        rows,
                        bytes: out.bytes,
                    });
                    last_report = Instant::now();
                }
            }
        }
        if let Some(statement) = batcher.finish() {
            out.write(&statement).await?;
        }
        (self.emit)(ExportEvent::Progress {
            table: table.to_owned(),
            rows,
            bytes: out.bytes,
        });
        Ok((rows, false))
    }
}

/// A table with the view's columns, created first so views that use other views can be created in any order.
async fn view_stand_in(
    conn: &mut Conn,
    out: &mut Output,
    request: &ExportRequest,
    view: &str,
) -> AppResult<()> {
    let columns = table_columns(conn, &request.database, view).await?;
    let list: Vec<String> = columns
        .iter()
        .map(|column| format!("{} int", quote_identifier(&column.name)))
        .collect();
    out.write(&format!(
        "DROP VIEW IF EXISTS {0};\nDROP TABLE IF EXISTS {0};\nCREATE TABLE {0} ({1});\n",
        quote_identifier(view),
        list.join(", ")
    ))
    .await
}

async fn views(conn: &mut Conn, out: &mut Output, request: &ExportRequest) -> AppResult<()> {
    out.write(&section("Views")).await?;
    for view in &request.views {
        view_stand_in(conn, out, request, view).await?;
    }
    for view in &request.views {
        let created = show_create(
            conn,
            format!("SHOW CREATE VIEW {}", qualified(&request.database, view)),
        )
        .await?;
        let sql = created
            .get("Create View")
            .ok_or_else(|| AppError::Database(format!("no definition for view {view}")))?;
        out.write(&format!(
            "DROP TABLE IF EXISTS {};\n{};\n",
            quote_identifier(view),
            keep_or_strip(request, sql)
        ))
        .await?;
    }
    Ok(())
}

async fn names(conn: &mut Conn, sql: &str, database: &str) -> AppResult<Vec<String>> {
    let rows: Vec<Row> = conn.exec(sql, (database,)).await?;
    Ok(rows
        .into_iter()
        .map(|row| texts(row).into_iter().next().flatten().unwrap_or_default())
        .collect())
}

async fn routines(conn: &mut Conn, out: &mut Output, request: &ExportRequest) -> AppResult<()> {
    let database = &request.database;
    out.write(&section("Routines")).await?;
    for (keyword, query) in [
        (
            "PROCEDURE",
            "SELECT ROUTINE_NAME FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = 'PROCEDURE' ORDER BY ROUTINE_NAME",
        ),
        (
            "FUNCTION",
            "SELECT ROUTINE_NAME FROM information_schema.ROUTINES WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = 'FUNCTION' ORDER BY ROUTINE_NAME",
        ),
    ] {
        for name in names(conn, query, database).await? {
            let created = show_create(
                conn,
                format!("SHOW CREATE {keyword} {}", qualified(database, &name)),
            )
            .await?;
            let heading = if keyword == "PROCEDURE" {
                "Create Procedure"
            } else {
                "Create Function"
            };
            let Some(sql) = created.get(heading) else {
                continue;
            };
            let mut statements = Vec::new();
            if request.add_drop {
                statements.push(format!(
                    "DROP {keyword} IF EXISTS {}",
                    quote_identifier(&name)
                ));
            }
            statements.extend(with_sql_mode(
                created.get("sql_mode"),
                keep_or_strip(request, sql),
            ));
            out.write(&delimited(&statements)).await?;
        }
    }
    Ok(())
}

async fn triggers(conn: &mut Conn, out: &mut Output, request: &ExportRequest) -> AppResult<()> {
    let rows: Vec<Row> = conn
        .exec("SELECT TRIGGER_NAME, EVENT_OBJECT_TABLE FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = ? ORDER BY EVENT_OBJECT_TABLE, TRIGGER_NAME", (request.database.as_str(),))
        .await?;
    out.write(&section("Triggers")).await?;
    for row in rows.into_iter().map(texts) {
        let (name, table) = (
            row[0].clone().unwrap_or_default(),
            row[1].clone().unwrap_or_default(),
        );
        if !request.tables.contains(&table) {
            continue;
        }
        let created = show_create(
            conn,
            format!(
                "SHOW CREATE TRIGGER {}",
                qualified(&request.database, &name)
            ),
        )
        .await?;
        let Some(sql) = created.get("SQL Original Statement") else {
            continue;
        };
        let mut statements = Vec::new();
        if request.add_drop {
            statements.push(format!(
                "DROP TRIGGER IF EXISTS {}",
                quote_identifier(&name)
            ));
        }
        statements.extend(with_sql_mode(
            created.get("sql_mode"),
            keep_or_strip(request, sql),
        ));
        out.write(&delimited(&statements)).await?;
    }
    Ok(())
}

async fn events(conn: &mut Conn, out: &mut Output, request: &ExportRequest) -> AppResult<()> {
    out.write(&section("Events")).await?;
    for name in names(conn, "SELECT EVENT_NAME FROM information_schema.EVENTS WHERE EVENT_SCHEMA = ? ORDER BY EVENT_NAME", &request.database).await? {
        let created = show_create(conn, format!("SHOW CREATE EVENT {}", qualified(&request.database, &name))).await?;
        let Some(sql) = created.get("Create Event") else { continue };
        let mut statements = Vec::new();
        if request.add_drop {
            statements.push(format!("DROP EVENT IF EXISTS {}", quote_identifier(&name)));
        }
        statements.extend(with_sql_mode(created.get("sql_mode"), keep_or_strip(request, sql)));
        out.write(&delimited(&statements)).await?;
    }
    Ok(())
}

async fn database_header(
    conn: &mut Conn,
    out: &mut Output,
    request: &ExportRequest,
) -> AppResult<()> {
    let version: Option<String> = conn.query_first("SELECT VERSION()").await?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    out.write(&header(
        &version.unwrap_or_default(),
        &utc_timestamp(now),
        &request.database,
    ))
    .await?;
    if request.create_database {
        let rows: Vec<Row> = conn
            .exec("SELECT DEFAULT_CHARACTER_SET_NAME, DEFAULT_COLLATION_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = ?", (request.database.as_str(),))
            .await?;
        let found = rows.into_iter().next().map(texts).ok_or_else(|| {
            AppError::Validation(format!("Database {} does not exist", request.database))
        })?;
        let name = quote_identifier(&request.database);
        let options = match (found[0].as_deref(), found[1].as_deref()) {
            (Some(charset), Some(collation)) => {
                format!(" DEFAULT CHARACTER SET {charset} COLLATE {collation}")
            }
            _ => String::new(),
        };
        out.write(&format!(
            "CREATE DATABASE IF NOT EXISTS {name}{options};\nUSE {name};\n\n"
        ))
        .await?;
    }
    Ok(())
}

async fn write_everything(
    conn: &mut Conn,
    out: &mut Output,
    request: &ExportRequest,
    cancel: &AtomicBool,
    emit: Emit<'_>,
) -> AppResult<(u64, bool)> {
    setup_session(conn, request).await?;
    database_header(conn, out, request).await?;
    emit(ExportEvent::Started {
        tables: request.tables.len(),
    });
    let dump = TableDump {
        request,
        cancel,
        emit,
    };
    let mut total_rows = 0;

    for (index, table) in request.tables.iter().enumerate() {
        if cancel.load(Ordering::SeqCst) {
            return Ok((total_rows, true));
        }
        emit(ExportEvent::TableStarted {
            name: table.clone(),
            index,
            estimated_rows: estimated_rows(conn, &request.database, table).await,
        });
        if request.structure {
            dump.structure(conn, out, table).await?;
        }
        let mut rows = 0;
        if request.data {
            let (written, stopped) = dump.data(conn, out, table).await?;
            rows = written;
            total_rows += rows;
            if stopped {
                return Ok((total_rows, true));
            }
        }
        emit(ExportEvent::TableFinished {
            name: table.clone(),
            rows,
        });
    }

    if request.structure && !request.views.is_empty() {
        // Stand-ins now, real views after the routines they may call.
        emit(ExportEvent::Note {
            message: "Writing views".into(),
        });
    }
    if request.routines {
        emit(ExportEvent::Note {
            message: "Writing routines".into(),
        });
        routines(conn, out, request).await?;
    }
    if request.triggers && request.structure {
        emit(ExportEvent::Note {
            message: "Writing triggers".into(),
        });
        triggers(conn, out, request).await?;
    }
    if request.structure && !request.views.is_empty() {
        views(conn, out, request).await?;
    }
    if request.events {
        emit(ExportEvent::Note {
            message: "Writing events".into(),
        });
        events(conn, out, request).await?;
    }
    out.write(FOOTER).await?;
    if request.consistent_snapshot {
        conn.query_drop("COMMIT").await?;
    }
    Ok((total_rows, false))
}

fn partial_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(PARTIAL_SUFFIX);
    PathBuf::from(name)
}

fn validate(request: &ExportRequest) -> AppResult<()> {
    if request.database.trim().is_empty() {
        return Err(AppError::Validation("Choose a database to export".into()));
    }
    if request.path.trim().is_empty() {
        return Err(AppError::Validation("Choose where to save the file".into()));
    }
    let nothing = request.tables.is_empty()
        && request.views.is_empty()
        && !request.routines
        && !request.events;
    if nothing {
        return Err(AppError::Validation(
            "Select at least one table, view, routine or event".into(),
        ));
    }
    if !request.structure && !request.data {
        return Err(AppError::Validation(
            "Export the structure, the data, or both".into(),
        ));
    }
    Ok(())
}

/// The file is written under a temporary name and renamed when complete, so a failed or cancelled export
/// never leaves something that looks like a finished dump.
pub async fn export_database(
    conn: &mut Conn,
    request: &ExportRequest,
    cancel: &AtomicBool,
    emit: Emit<'_>,
) -> AppResult<ExportSummary> {
    validate(request)?;
    let started = Instant::now();
    let final_path = PathBuf::from(&request.path);
    let partial = partial_path(&final_path);
    let raw = BufWriter::with_capacity(WRITE_BUFFER_BYTES, File::create(&partial).await?);
    let mut out = Output {
        file: if is_gzip_path(&request.path) {
            Box::pin(GzipEncoder::new(raw))
        } else {
            Box::pin(raw)
        },
        bytes: 0,
    };

    let outcome = write_everything(conn, &mut out, request, cancel, emit).await;
    let flushed = match &outcome {
        // `shutdown` flushes, and for gzip also writes the closing block that makes the file readable.
        Ok((_, false)) => out.file.shutdown().await.map_err(AppError::from),
        _ => Ok(()),
    };
    drop(out.file);
    let finish = |bytes| ExportSummary {
        path: request.path.clone(),
        bytes,
        tables: request.tables.len(),
        rows: 0,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        cancelled: false,
    };

    match (outcome, flushed) {
        (Ok((rows, false)), Ok(())) => {
            tokio::fs::rename(&partial, &final_path).await?;
            // The size on disk, which for a compressed file is smaller than what was written.
            let on_disk = tokio::fs::metadata(&final_path)
                .await
                .map_or(out.bytes, |meta| meta.len());
            Ok(ExportSummary {
                rows,
                ..finish(on_disk)
            })
        }
        (Ok((rows, true)), _) => {
            let _ = tokio::fs::remove_file(&partial).await;
            Ok(ExportSummary {
                rows,
                cancelled: true,
                ..finish(out.bytes)
            })
        }
        (Err(error), _) | (_, Err(error)) => {
            let _ = tokio::fs::remove_file(&partial).await;
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_ending_in_gz_means_compressed() {
        assert!(is_gzip_path("/tmp/dump.sql.gz"));
        assert!(is_gzip_path("/tmp/DUMP.SQL.GZ"));
        assert!(!is_gzip_path("/tmp/dump.sql"));
    }

    #[test]
    fn timestamps_are_written_in_utc() {
        assert_eq!(utc_timestamp(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(utc_timestamp(951_782_400), "2000-02-29 00:00:00 UTC");
        assert_eq!(utc_timestamp(1_791_799_445), "2026-10-12 10:04:05 UTC");
    }

    #[test]
    fn routine_blocks_are_wrapped_in_a_delimiter_and_keep_the_sql_mode() {
        let statements = with_sql_mode(
            Some(&"STRICT_TRANS_TABLES".to_owned()),
            "CREATE PROCEDURE p() SELECT 1".to_owned(),
        );

        assert_eq!(statements.len(), 4);
        assert_eq!(statements[1], "SET sql_mode = 'STRICT_TRANS_TABLES'");
        assert_eq!(
            delimited(&statements[..1]),
            "DELIMITER ;;\nSET @saved_sql_mode = @@sql_mode;;\nDELIMITER ;\n"
        );
        assert_eq!(with_sql_mode(None, "X".into()), ["X"]);
    }

    #[test]
    fn requests_must_ask_for_something() {
        let base: ExportRequest =
            serde_json::from_str(r#"{"jobId":"j","database":"shop","path":"/tmp/x.sql"}"#).unwrap();

        assert!(validate(&base).is_err());
        assert!(
            validate(&ExportRequest {
                tables: vec!["t".into()],
                ..base.clone()
            })
            .is_ok()
        );
        assert!(
            validate(&ExportRequest {
                tables: vec!["t".into()],
                structure: false,
                data: false,
                ..base.clone()
            })
            .is_err()
        );
        assert!(
            validate(&ExportRequest {
                database: " ".into(),
                tables: vec!["t".into()],
                ..base.clone()
            })
            .is_err()
        );
        assert!(
            validate(&ExportRequest {
                tables: vec!["t".into()],
                path: String::new(),
                ..base
            })
            .is_err()
        );
    }

    #[test]
    fn a_request_has_safe_defaults() {
        let request: ExportRequest =
            serde_json::from_str(r#"{"jobId":"j","database":"shop","path":"/tmp/x.sql"}"#).unwrap();

        assert!(
            request.structure && request.data && request.add_drop && request.consistent_snapshot
        );
        assert!(!request.create_database && !request.keep_definers && !request.routines);
        assert_eq!(
            partial_path(Path::new("/tmp/x.sql")),
            PathBuf::from("/tmp/x.sql.partial")
        );
    }
}
