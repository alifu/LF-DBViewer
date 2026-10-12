//! Writes the rows a query returns to a CSV or JSON file, straight from the server and without the display limits.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mysql_async::Conn;
use mysql_async::prelude::Queryable;
use serde::{Deserialize, Serialize};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};

use super::csv_import::parse_delimiter;
use super::export::{Emit, ExportEvent};
use super::value::{DumpColumn, json_value, plain_text};
use crate::error::{AppError, AppResult};
use crate::sql::quote_identifier;

const PROGRESS_EVERY: Duration = Duration::from_millis(250);
const CANCEL_CHECK_ROWS: u64 = 500;
const BYTE_ORDER_MARK: &str = "\u{feff}";
const ALLOWED_EXTENSIONS: [&str; 4] = ["csv", "tsv", "json", "txt"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ResultFormat {
    Csv,
    Json,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultExportRequest {
    pub job_id: String,
    pub sql: String,
    #[serde(default)]
    pub database: Option<String>,
    pub path: String,
    pub format: ResultFormat,
    #[serde(default = "default_delimiter")]
    pub delimiter: String,
    #[serde(default = "yes")]
    pub header: bool,
    /// What a NULL looks like in a CSV file.
    #[serde(default)]
    pub null_text: String,
    /// A UTF-8 byte order mark, which makes Excel read the file as UTF-8.
    #[serde(default)]
    pub byte_order_mark: bool,
}

fn default_delimiter() -> String {
    ",".into()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultExportSummary {
    pub path: String,
    pub rows: u64,
    pub bytes: u64,
    pub duration_ms: u64,
    pub cancelled: bool,
}

fn checked_path(raw: &str) -> AppResult<PathBuf> {
    let path = PathBuf::from(raw);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    if !path.is_absolute()
        || !extension.is_some_and(|extension| ALLOWED_EXTENSIONS.contains(&extension.as_str()))
    {
        return Err(AppError::Validation(
            "Choose a file ending in .csv, .tsv or .json".into(),
        ));
    }
    if !path.parent().is_some_and(std::path::Path::is_dir) {
        return Err(AppError::Validation("The folder does not exist".into()));
    }
    Ok(path)
}

/// Adds the size of a chunk to the running total and hands the chunk back, ready to write.
fn counted(total: &mut u64, chunk: Vec<u8>) -> Vec<u8> {
    *total += chunk.len() as u64;
    chunk
}

fn csv_line(fields: &[Option<String>], delimiter: u8, null_text: &str) -> AppResult<Vec<u8>> {
    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_writer(Vec::new());
    writer
        .write_record(
            fields
                .iter()
                .map(|field| field.as_deref().unwrap_or(null_text)),
        )
        .map_err(|error| AppError::Storage(error.to_string()))?;
    writer
        .into_inner()
        .map_err(|error| AppError::Storage(error.to_string()))
}

async fn write_rows(
    conn: &mut Conn,
    request: &ResultExportRequest,
    out: &mut BufWriter<File>,
    cancel: &AtomicBool,
    emit: Emit<'_>,
) -> AppResult<(u64, u64, bool)> {
    let delimiter = parse_delimiter(&request.delimiter)?;
    if let Some(database) = request.database.as_deref().filter(|name| !name.is_empty()) {
        conn.query_drop(format!("USE {}", quote_identifier(database)))
            .await?;
    }
    let mut result = conn.query_iter(request.sql.as_str()).await?;
    let Some(columns) = result.columns().filter(|columns| !columns.is_empty()) else {
        return Err(AppError::Validation(
            "The statement did not return rows, so there is nothing to export".into(),
        ));
    };
    let names: Vec<String> = columns
        .iter()
        .map(|column| column.name_str().into_owned())
        .collect();
    let dump_columns: Vec<DumpColumn> = columns.iter().map(DumpColumn::of).collect();

    let mut bytes = 0u64;
    if request.byte_order_mark && request.format == ResultFormat::Csv {
        out.write_all(&counted(&mut bytes, BYTE_ORDER_MARK.as_bytes().to_vec()))
            .await?;
    }
    match request.format {
        ResultFormat::Csv if request.header => {
            let header: Vec<Option<String>> = names.iter().cloned().map(Some).collect();
            out.write_all(&counted(&mut bytes, csv_line(&header, delimiter, "")?))
                .await?;
        }
        ResultFormat::Json => out.write_all(&counted(&mut bytes, b"[".to_vec())).await?,
        ResultFormat::Csv => {}
    }

    let (mut rows, mut last_report) = (0u64, Instant::now());
    while let Some(row) = result.next().await? {
        let values = row.unwrap();
        let chunk = match request.format {
            ResultFormat::Csv => {
                let fields: Vec<Option<String>> = values
                    .iter()
                    .zip(&dump_columns)
                    .map(|(value, column)| plain_text(value, *column))
                    .collect();
                csv_line(&fields, delimiter, &request.null_text)?
            }
            ResultFormat::Json => {
                let object: serde_json::Map<String, serde_json::Value> = names
                    .iter()
                    .cloned()
                    .zip(
                        values
                            .iter()
                            .zip(&dump_columns)
                            .map(|(value, column)| json_value(value, *column)),
                    )
                    .collect();
                let separator = if rows == 0 { "\n  " } else { ",\n  " };
                format!("{separator}{}", serde_json::Value::Object(object)).into_bytes()
            }
        };
        out.write_all(&counted(&mut bytes, chunk)).await?;
        rows += 1;
        if rows % CANCEL_CHECK_ROWS == 0 {
            if cancel.load(Ordering::SeqCst) {
                return Ok((rows, bytes, true));
            }
            if last_report.elapsed() >= PROGRESS_EVERY {
                last_report = Instant::now();
                emit(ExportEvent::Progress {
                    table: "result".into(),
                    rows,
                    bytes,
                });
            }
        }
    }
    if request.format == ResultFormat::Json {
        out.write_all(&counted(&mut bytes, b"\n]\n".to_vec()))
            .await?;
    }
    emit(ExportEvent::Progress {
        table: "result".into(),
        rows,
        bytes,
    });
    Ok((rows, bytes, false))
}

/// Written under a temporary name and renamed when complete, like a database export.
pub async fn export_result(
    conn: &mut Conn,
    request: &ResultExportRequest,
    cancel: &AtomicBool,
    emit: Emit<'_>,
) -> AppResult<ResultExportSummary> {
    if request.sql.trim().is_empty() {
        return Err(AppError::Validation(
            "There is no statement to export".into(),
        ));
    }
    let started = Instant::now();
    let final_path = checked_path(&request.path)?;
    let mut partial = final_path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let mut out = BufWriter::new(File::create(&partial).await?);

    let outcome = write_rows(conn, request, &mut out, cancel, emit).await;
    let flushed = if matches!(outcome, Ok((_, _, false))) {
        out.flush().await.map_err(AppError::from)
    } else {
        Ok(())
    };
    drop(out);
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match (outcome, flushed) {
        (Ok((rows, bytes, false)), Ok(())) => {
            tokio::fs::rename(&partial, &final_path).await?;
            Ok(ResultExportSummary {
                path: request.path.clone(),
                rows,
                bytes,
                duration_ms,
                cancelled: false,
            })
        }
        (Ok((rows, bytes, true)), _) => {
            let _ = tokio::fs::remove_file(&partial).await;
            Ok(ResultExportSummary {
                path: request.path.clone(),
                rows,
                bytes,
                duration_ms,
                cancelled: true,
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
    fn csv_lines_quote_what_needs_it_and_write_null_as_the_chosen_text() {
        let line = csv_line(
            &[
                Some("a,b".into()),
                Some("say \"hi\"".into()),
                Some("two\nlines".into()),
                None,
                Some("plain".into()),
            ],
            b',',
            "\\N",
        )
        .unwrap();

        assert_eq!(
            String::from_utf8(line).unwrap(),
            "\"a,b\",\"say \"\"hi\"\"\",\"two\nlines\",\\N,plain\n"
        );
        assert_eq!(
            String::from_utf8(csv_line(&[Some("a;b".into()), None], b';', "").unwrap()).unwrap(),
            "\"a;b\";\n"
        );
        assert_eq!(
            String::from_utf8(csv_line(&[Some("a\tb".into())], b'\t', "").unwrap()).unwrap(),
            "\"a\tb\"\n"
        );
    }

    #[test]
    fn only_data_file_extensions_in_an_existing_folder_are_accepted() {
        let folder = std::env::temp_dir();

        assert!(checked_path(&format!("{}/out.csv", folder.display())).is_ok());
        assert!(checked_path(&format!("{}/OUT.JSON", folder.display())).is_ok());
        for bad in [
            "out.csv",
            "/tmp/out.sh",
            "/tmp/out",
            "/no/such/folder/out.csv",
            "",
        ] {
            assert!(checked_path(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn requests_have_sensible_defaults() {
        let request: ResultExportRequest = serde_json::from_str(
            r#"{"jobId":"j","sql":"SELECT 1","path":"/tmp/x.csv","format":"csv"}"#,
        )
        .unwrap();

        assert_eq!(
            (
                request.delimiter.as_str(),
                request.header,
                request.byte_order_mark
            ),
            (",", true, false)
        );
        assert!(
            serde_json::from_str::<ResultExportRequest>(
                r#"{"jobId":"j","sql":"x","path":"/x","format":"xml"}"#
            )
            .is_err()
        );
    }
}
