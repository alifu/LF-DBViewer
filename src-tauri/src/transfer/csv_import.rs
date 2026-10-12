//! Loads a CSV (or TSV) file into an existing table: a preview to map columns, then batched inserts in one transaction.

use std::fs::File;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Value};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::schema::table_columns;
use crate::sql::{placeholders, qualified, quote_identifier};

const PREVIEW_ROWS: usize = 8;
const MAX_BATCH_ROWS: usize = 1000;
/// The server allows at most 65,535 placeholders in one statement.
const MAX_PLACEHOLDERS: usize = 60_000;
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

fn yes() -> bool {
    true
}

/// `,` `;` `|` and a tab (written `\t` or `tab`), or any single character.
pub fn parse_delimiter(text: &str) -> AppResult<u8> {
    let normal = text.to_ascii_lowercase();
    if normal == "\\t" || normal == "tab" || text == "\t" {
        return Ok(b'\t');
    }
    match text.as_bytes() {
        [byte] if byte.is_ascii() && !matches!(byte, b'"' | b'\n' | b'\r') => Ok(*byte),
        _ => Err(AppError::Validation(
            "The delimiter must be one character, or tab".into(),
        )),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvPreview {
    /// The header row, or `column 1`, `column 2`… when the file has none.
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub size: u64,
}

fn open_reader(path: &str, delimiter: u8) -> AppResult<csv::Reader<File>> {
    let file = File::open(path)
        .map_err(|error| AppError::Validation(format!("Cannot open {path}: {error}")))?;
    Ok(csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .has_headers(false)
        .flexible(true)
        .from_reader(file))
}

fn csv_error(error: &csv::Error) -> AppError {
    AppError::Validation(format!("The file is not valid CSV: {error}"))
}

pub fn preview_csv(path: &str, delimiter: &str, has_header: bool) -> AppResult<CsvPreview> {
    let mut reader = open_reader(path, parse_delimiter(delimiter)?)?;
    let size = std::fs::metadata(Path::new(path))?.len();
    let mut records = reader.records();
    let take = |records: &mut csv::StringRecordsIter<'_, File>| -> AppResult<Option<Vec<String>>> {
        records
            .next()
            .transpose()
            .map_err(|error| csv_error(&error))
            .map(|record| record.map(|fields| fields.iter().map(str::to_owned).collect()))
    };
    let first = take(&mut records)?;
    let mut rows = Vec::new();
    let headers = match (has_header, first) {
        (_, None) => {
            return Ok(CsvPreview {
                headers: Vec::new(),
                rows,
                size,
            });
        }
        (true, Some(header)) => header,
        (false, Some(row)) => {
            let names = (1..=row.len())
                .map(|number| format!("column {number}"))
                .collect();
            rows.push(row);
            names
        }
    };
    while rows.len() < PREVIEW_ROWS {
        match take(&mut records)? {
            Some(row) => rows.push(row),
            None => break,
        }
    }
    Ok(CsvPreview {
        headers,
        rows,
        size,
    })
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportRequest {
    pub job_id: String,
    pub path: String,
    pub database: String,
    pub table: String,
    pub delimiter: String,
    #[serde(default = "yes")]
    pub has_header: bool,
    /// For each CSV column, in order, the table column it goes into, or `None` to skip it.
    pub mapping: Vec<Option<String>>,
    /// Text that means NULL, for example `NULL` or `\N`.
    #[serde(default)]
    pub null_text: Option<String>,
    #[serde(default)]
    pub empty_as_null: bool,
    /// Empty the table first (`TRUNCATE`, which cannot be undone).
    #[serde(default)]
    pub truncate_first: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CsvImportEvent {
    Started {
        total_bytes: u64,
    },
    Progress {
        rows: u64,
        bytes_read: u64,
        total_bytes: u64,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CsvImportSummary {
    pub rows: u64,
    pub duration_ms: u64,
    pub cancelled: bool,
}

pub type Emit<'a> = &'a (dyn Fn(CsvImportEvent) + Send + Sync);

/// The columns to insert into, checked: each exists, is not generated, and is used once.
pub fn target_columns(
    mapping: &[Option<String>],
    known: &[crate::schema::ColumnInfo],
) -> AppResult<Vec<(usize, String)>> {
    let mut chosen: Vec<(usize, String)> = Vec::new();
    for (position, name) in mapping.iter().enumerate() {
        let Some(name) = name else { continue };
        let column = known
            .iter()
            .find(|column| &column.name == name)
            .ok_or_else(|| AppError::Validation(format!("The table has no column {name}")))?;
        if column.generated {
            return Err(AppError::Validation(format!(
                "{name} is a generated column and cannot be filled"
            )));
        }
        if chosen.iter().any(|(_, existing)| existing == name) {
            return Err(AppError::Validation(format!(
                "{name} is chosen for two CSV columns"
            )));
        }
        chosen.push((position, name.clone()));
    }
    if chosen.is_empty() {
        return Err(AppError::Validation(
            "Choose at least one table column to fill".into(),
        ));
    }
    Ok(chosen)
}

fn cell(field: Option<&str>, request: &CsvImportRequest) -> Value {
    match field {
        None => Value::NULL,
        Some(text)
            if request
                .null_text
                .as_deref()
                .is_some_and(|null| !null.is_empty() && null == text) =>
        {
            Value::NULL
        }
        Some("") if request.empty_as_null => Value::NULL,
        Some(text) => Value::from(text),
    }
}

pub fn batch_rows(columns: usize) -> usize {
    (MAX_PLACEHOLDERS / columns.max(1)).clamp(1, MAX_BATCH_ROWS)
}

fn insert_sql(request: &CsvImportRequest, columns: &[(usize, String)], rows: usize) -> String {
    let names: Vec<String> = columns
        .iter()
        .map(|(_, name)| quote_identifier(name))
        .collect();
    let row = format!("({})", placeholders(columns.len()));
    format!(
        "INSERT INTO {} ({}) VALUES {}",
        qualified(&request.database, &request.table),
        names.join(", "),
        vec![row; rows].join(", ")
    )
}

/// Inserts the rows one by one to say which of them the server refused.
async fn find_bad_row(
    conn: &mut Conn,
    request: &CsvImportRequest,
    columns: &[(usize, String)],
    batch: &[Vec<Value>],
    first_row: u64,
) -> AppError {
    let sql = insert_sql(request, columns, 1);
    for (offset, row) in batch.iter().enumerate() {
        if let Err(error) = conn.exec_drop(sql.as_str(), row.clone()).await {
            return AppError::Database(format!(
                "Row {} of the file was refused: {error}. Nothing was imported.",
                first_row + offset as u64
            ));
        }
    }
    AppError::Database("The server refused a batch of rows. Nothing was imported.".into())
}

pub async fn import_csv(
    conn: &mut Conn,
    request: &CsvImportRequest,
    cancel: &AtomicBool,
    emit: Emit<'_>,
) -> AppResult<CsvImportSummary> {
    let started = Instant::now();
    let delimiter = parse_delimiter(&request.delimiter)?;
    let known = table_columns(conn, &request.database, &request.table).await?;
    let columns = target_columns(&request.mapping, &known)?;
    let total_bytes = std::fs::metadata(&request.path)
        .map_err(|error| AppError::Validation(format!("Cannot open {}: {error}", request.path)))?
        .len();
    let mut reader = open_reader(&request.path, delimiter)?;
    let mut records = reader.records();
    if request.has_header {
        records
            .next()
            .transpose()
            .map_err(|error| csv_error(&error))?;
    }
    if request.truncate_first {
        conn.query_drop(format!(
            "TRUNCATE TABLE {}",
            qualified(&request.database, &request.table)
        ))
        .await?;
    }
    emit(CsvImportEvent::Started { total_bytes });

    conn.query_drop("START TRANSACTION").await?;
    let per_batch = batch_rows(columns.len());
    let (mut rows_done, mut batch, mut batch_first_row) = (0u64, Vec::<Vec<Value>>::new(), 1u64);
    let mut last_report = Instant::now();
    let mut finished = false;

    while !finished {
        if cancel.load(Ordering::SeqCst) {
            conn.query_drop("ROLLBACK").await?;
            return Ok(CsvImportSummary {
                rows: 0,
                duration_ms: elapsed(started),
                cancelled: true,
            });
        }
        match records.next() {
            None => finished = true,
            Some(Err(error)) => {
                conn.query_drop("ROLLBACK").await?;
                return Err(csv_error(&error));
            }
            Some(Ok(record)) => {
                if batch.is_empty() {
                    batch_first_row = rows_done + 1;
                }
                batch.push(
                    columns
                        .iter()
                        .map(|(position, _)| cell(record.get(*position), request))
                        .collect(),
                );
                rows_done += 1;
            }
        }
        if batch.len() >= per_batch || (finished && !batch.is_empty()) {
            let sql = insert_sql(request, &columns, batch.len());
            let params: Vec<Value> = batch.iter().flatten().cloned().collect();
            if conn.exec_drop(sql, params).await.is_err() {
                let error = find_bad_row(conn, request, &columns, &batch, batch_first_row).await;
                conn.query_drop("ROLLBACK").await?;
                return Err(error);
            }
            batch.clear();
        }
        if last_report.elapsed() >= PROGRESS_EVERY {
            last_report = Instant::now();
            emit(CsvImportEvent::Progress {
                rows: rows_done,
                bytes_read: records_position(&records),
                total_bytes,
            });
        }
    }
    conn.query_drop("COMMIT").await?;
    emit(CsvImportEvent::Progress {
        rows: rows_done,
        bytes_read: total_bytes,
        total_bytes,
    });
    Ok(CsvImportSummary {
        rows: rows_done,
        duration_ms: elapsed(started),
        cancelled: false,
    })
}

fn records_position(records: &csv::StringRecordsIter<'_, File>) -> u64 {
    records.reader().position().byte()
}

fn elapsed(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::ColumnInfo;

    fn temp_file(contents: &[u8]) -> std::path::PathBuf {
        let path =
            std::env::temp_dir().join(format!("lfdb_csv_{}.csv", uuid::Uuid::new_v4().simple()));
        std::fs::write(&path, contents).unwrap();
        path
    }

    fn column(name: &str, generated: bool) -> ColumnInfo {
        ColumnInfo {
            name: name.into(),
            column_type: "int".into(),
            nullable: true,
            primary_key: false,
            default: None,
            extra: String::new(),
            collation: None,
            generated,
        }
    }

    #[test]
    fn delimiters_are_single_characters_or_tab() {
        assert_eq!(parse_delimiter(",").unwrap(), b',');
        assert_eq!(parse_delimiter(";").unwrap(), b';');
        assert_eq!(parse_delimiter("\\t").unwrap(), b'\t');
        assert_eq!(parse_delimiter("TAB").unwrap(), b'\t');
        assert_eq!(parse_delimiter("\t").unwrap(), b'\t');
        for bad in ["", ",,", "\"", "é"] {
            assert!(parse_delimiter(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_preview_reads_headers_quoted_fields_and_a_byte_order_mark() {
        let path = temp_file(
            "\u{feff}id,name,note\n1,\"Ann, the \"\"boss\"\"\",\"line one\nline two\"\n2,Bob,\n"
                .as_bytes(),
        );

        let preview = preview_csv(path.to_str().unwrap(), ",", true).unwrap();
        let without_header = preview_csv(path.to_str().unwrap(), ",", false).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(preview.headers, ["id", "name", "note"]);
        assert_eq!(
            preview.rows[0],
            ["1", "Ann, the \"boss\"", "line one\nline two"]
        );
        assert_eq!(preview.rows[1], ["2", "Bob", ""]);
        assert_eq!(without_header.headers, ["column 1", "column 2", "column 3"]);
        assert_eq!(without_header.rows.len(), 3);
    }

    #[test]
    fn an_empty_file_and_a_missing_file_are_handled() {
        let empty = temp_file(b"");

        assert!(
            preview_csv(empty.to_str().unwrap(), ",", true)
                .unwrap()
                .headers
                .is_empty()
        );
        assert!(matches!(
            preview_csv("/no/such/file.csv", ",", true),
            Err(AppError::Validation(_))
        ));
        std::fs::remove_file(&empty).unwrap();
    }

    #[test]
    fn the_preview_stops_after_a_few_rows() {
        let many: String = (0..50).map(|n| format!("{n},x\n")).collect();
        let path = temp_file(many.as_bytes());

        let preview = preview_csv(path.to_str().unwrap(), ",", false).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert_eq!(preview.rows.len(), PREVIEW_ROWS);
    }

    #[test]
    fn mappings_must_name_real_ungenerated_columns_once() {
        let known = vec![
            column("id", false),
            column("name", false),
            column("total", true),
        ];
        let map = |names: &[Option<&str>]| {
            names
                .iter()
                .map(|name| name.map(str::to_owned))
                .collect::<Vec<_>>()
        };

        assert_eq!(
            target_columns(&map(&[Some("name"), None, Some("id")]), &known).unwrap(),
            [(0, "name".to_owned()), (2, "id".to_owned())]
        );
        for bad in [
            map(&[None, None]),
            map(&[Some("nope")]),
            map(&[Some("total")]),
            map(&[Some("id"), Some("id")]),
        ] {
            assert!(
                matches!(target_columns(&bad, &known), Err(AppError::Validation(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn null_text_and_empty_fields_become_null_only_when_asked() {
        let request = |null_text: Option<&str>, empty_as_null| CsvImportRequest {
            job_id: String::new(),
            path: String::new(),
            database: "d".into(),
            table: "t".into(),
            delimiter: ",".into(),
            has_header: true,
            mapping: vec![],
            null_text: null_text.map(str::to_owned),
            empty_as_null,
            truncate_first: false,
        };

        assert_eq!(
            cell(Some("NULL"), &request(Some("NULL"), false)),
            Value::NULL
        );
        assert_eq!(
            cell(Some("null"), &request(Some("NULL"), false)),
            Value::from("null")
        );
        assert_eq!(cell(Some(""), &request(None, true)), Value::NULL);
        assert_eq!(cell(Some(""), &request(None, false)), Value::from(""));
        assert_eq!(cell(Some(""), &request(Some(""), false)), Value::from(""));
        assert_eq!(cell(None, &request(None, false)), Value::NULL);
    }

    #[test]
    fn batches_stay_under_the_placeholder_limit() {
        assert_eq!(batch_rows(3), MAX_BATCH_ROWS);
        assert_eq!(batch_rows(100), 600);
        assert_eq!(batch_rows(60_000), 1);
        assert_eq!(batch_rows(0), MAX_BATCH_ROWS);
        assert!(batch_rows(70) * 70 <= 65_535);
    }
}
