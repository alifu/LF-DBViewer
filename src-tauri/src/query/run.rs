//! Runs statements on a session connection and streams what comes back as events:
//! every result set (a `CALL` can return several), its rows in batches, and the outcome.

use std::sync::atomic::Ordering;
use std::time::Instant;

use mysql_async::prelude::Queryable;
use mysql_async::{Column, Conn};
use serde::Serialize;

use super::cell::{Cell, ColumnShape, cell_from_value};
use super::session::QuerySession;
use crate::error::{AppError, AppResult};
use crate::sql::quote_identifier;

/// Rows kept per result set. Rows past this are counted but not sent, so a runaway
/// `SELECT` cannot exhaust memory; the result says it was cut.
pub const MAX_RESULT_ROWS: u64 = 100_000;
const ROW_BATCH: usize = 1_000;
const SERVER_ERROR_INTERRUPTED: u16 = 1317;
const SERVER_ERROR_MAX_EXECUTION_TIME: u16 = 3024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum QueryEvent {
    StatementStarted {
        index: usize,
    },
    ResultSetStarted {
        index: usize,
        set: usize,
        columns: Vec<ResultColumn>,
    },
    Rows {
        index: usize,
        set: usize,
        rows: Vec<Vec<Cell>>,
    },
    ResultSetFinished {
        index: usize,
        set: usize,
        total_rows: u64,
        truncated: bool,
    },
    StatementFinished {
        index: usize,
        affected_rows: Option<u64>,
        last_insert_id: Option<u64>,
        warnings: u16,
        info: String,
        duration_ms: u64,
    },
    StatementFailed {
        index: usize,
        code: Option<u16>,
        message: String,
        duration_ms: u64,
        cancelled: bool,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunSummary {
    pub statements_run: usize,
    pub failed: bool,
    pub cancelled: bool,
    pub current_database: Option<String>,
}

pub type Emit<'a> = &'a (dyn Fn(QueryEvent) + Send + Sync);

#[derive(Default)]
struct Outcome {
    affected_rows: Option<u64>,
    last_insert_id: Option<u64>,
    warnings: u16,
    info: String,
}

/// `MYSQL_TYPE_VAR_STRING` becomes `var_string`.
fn data_type_name(column: &Column) -> String {
    let debug = format!("{:?}", column.column_type());
    debug.trim_start_matches("MYSQL_TYPE_").to_ascii_lowercase()
}

fn describe(column: &Column) -> ResultColumn {
    ResultColumn {
        name: column.name_str().into_owned(),
        data_type: data_type_name(column),
    }
}

fn milliseconds(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

async fn run_statement(
    conn: &mut Conn,
    index: usize,
    sql: &str,
    max_rows: u64,
    emit: Emit<'_>,
) -> mysql_async::Result<Outcome> {
    let mut result = conn.query_iter(sql).await?;
    let mut outcome = Outcome::default();
    let mut set = 0;

    // The driver moves on to the next result set by itself once one is read to its end.
    while let Some(columns) = result.columns() {
        if columns.is_empty() {
            outcome = Outcome {
                affected_rows: Some(result.affected_rows()),
                last_insert_id: result.last_insert_id().filter(|id| *id > 0),
                warnings: 0,
                info: result.info().into_owned(),
            };
            result.next().await?;
            continue;
        }

        let shapes: Vec<ColumnShape> = columns.iter().map(ColumnShape::of).collect();
        emit(QueryEvent::ResultSetStarted {
            index,
            set,
            columns: columns.iter().map(describe).collect(),
        });
        let (mut total, mut batch) = (0u64, Vec::new());
        while let Some(row) = result.next().await? {
            total += 1;
            if total > max_rows {
                continue;
            }
            batch.push(
                row.unwrap()
                    .into_iter()
                    .zip(&shapes)
                    .map(|(value, shape)| cell_from_value(value, *shape))
                    .collect(),
            );
            if batch.len() >= ROW_BATCH {
                emit(QueryEvent::Rows {
                    index,
                    set,
                    rows: std::mem::take(&mut batch),
                });
            }
        }
        if !batch.is_empty() {
            emit(QueryEvent::Rows {
                index,
                set,
                rows: batch,
            });
        }
        emit(QueryEvent::ResultSetFinished {
            index,
            set,
            total_rows: total,
            truncated: total > max_rows,
        });
        set += 1;
    }
    outcome.warnings = result.warnings();
    Ok(outcome)
}

fn failure_event(
    index: usize,
    error: &mysql_async::Error,
    cancel_requested: bool,
    started: Instant,
) -> QueryEvent {
    let server_code = match error {
        mysql_async::Error::Server(server) => Some(server.code),
        _ => None,
    };
    let interrupted = server_code == Some(SERVER_ERROR_INTERRUPTED);
    let message = match server_code {
        Some(SERVER_ERROR_MAX_EXECUTION_TIME) => error.to_string(),
        _ if interrupted || cancel_requested => "Cancelled".to_owned(),
        Some(code) => match error {
            mysql_async::Error::Server(server) => {
                format!("ERROR {code} ({}): {}", server.state, server.message)
            }
            _ => error.to_string(),
        },
        None => error.to_string(),
    };
    QueryEvent::StatementFailed {
        index,
        code: server_code,
        message,
        duration_ms: milliseconds(started),
        cancelled: interrupted || cancel_requested,
    }
}

/// Stops at the first failing statement, as most SQL clients do.
/// `connection` supplies a fresh connection when the session has none.
pub async fn run_statements(
    session: &QuerySession,
    connect: impl std::future::Future<Output = AppResult<Conn>>,
    database: Option<&str>,
    statements: &[String],
    max_rows: u64,
    emit: Emit<'_>,
) -> AppResult<RunSummary> {
    let mut guard = session.conn.lock().await;
    session.cancel_requested.store(false, Ordering::SeqCst);
    if guard.is_none() {
        *guard = Some(connect.await?);
    }
    let conn = guard
        .as_mut()
        .ok_or_else(|| AppError::Database("no connection".into()))?;
    session.thread_id.store(conn.id(), Ordering::SeqCst);

    let mut summary = RunSummary {
        statements_run: 0,
        failed: false,
        cancelled: false,
        current_database: None,
    };
    let mut connection_lost = false;

    let selected = match database {
        Some(name) => conn
            .query_drop(format!("USE {}", quote_identifier(name)))
            .await
            .map_err(AppError::from),
        None => Ok(()),
    };
    if let Err(error) = selected {
        session.thread_id.store(0, Ordering::SeqCst);
        return Err(error);
    }

    for (index, sql) in statements.iter().enumerate() {
        if session.cancel_requested.load(Ordering::SeqCst) {
            summary.cancelled = true;
            break;
        }
        emit(QueryEvent::StatementStarted { index });
        let started = Instant::now();
        match run_statement(conn, index, sql, max_rows, emit).await {
            Ok(outcome) => {
                summary.statements_run += 1;
                emit(QueryEvent::StatementFinished {
                    index,
                    affected_rows: outcome.affected_rows,
                    last_insert_id: outcome.last_insert_id,
                    warnings: outcome.warnings,
                    info: outcome.info,
                    duration_ms: milliseconds(started),
                });
            }
            Err(error) => {
                let requested = session.cancel_requested.load(Ordering::SeqCst);
                let event = failure_event(index, &error, requested, started);
                summary.cancelled = matches!(
                    event,
                    QueryEvent::StatementFailed {
                        cancelled: true,
                        ..
                    }
                );
                summary.failed = !summary.cancelled;
                connection_lost = error.is_fatal();
                emit(event);
                break;
            }
        }
    }

    session.thread_id.store(0, Ordering::SeqCst);
    if connection_lost {
        *guard = None;
    } else if let Some(conn) = guard.as_mut() {
        // DATABASE() is NULL when none is selected, hence the nested Option.
        let selected: Option<Option<String>> =
            conn.query_first("SELECT DATABASE()").await.unwrap_or(None);
        summary.current_database = selected.flatten();
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_serialize_with_camel_case_names() {
        let event = QueryEvent::ResultSetFinished {
            index: 2,
            set: 0,
            total_rows: 7,
            truncated: false,
        };

        let json = serde_json::to_value(&event).unwrap();

        assert_eq!(json["event"], "resultSetFinished");
        assert_eq!(json["totalRows"], 7);
        assert_eq!(json["index"], 2);
    }

    #[test]
    fn a_cancelled_failure_is_reported_as_cancelled_not_as_a_server_error() {
        let error = mysql_async::Error::Server(mysql_async::ServerError {
            code: SERVER_ERROR_INTERRUPTED,
            message: "Query execution was interrupted".into(),
            state: "70100".into(),
        });

        let event = failure_event(0, &error, false, Instant::now());

        let QueryEvent::StatementFailed {
            cancelled, message, ..
        } = event
        else {
            panic!("expected a failure")
        };
        assert!(cancelled);
        assert_eq!(message, "Cancelled");
    }

    #[test]
    fn server_errors_keep_their_code_state_and_message() {
        let error = mysql_async::Error::Server(mysql_async::ServerError {
            code: 1146,
            message: "Table 'x.y' doesn't exist".into(),
            state: "42S02".into(),
        });

        let event = failure_event(3, &error, false, Instant::now());

        let QueryEvent::StatementFailed {
            code,
            message,
            cancelled,
            index,
            ..
        } = event
        else {
            panic!("expected a failure")
        };
        assert_eq!((code, cancelled, index), (Some(1146), false, 3));
        assert_eq!(message, "ERROR 1146 (42S02): Table 'x.y' doesn't exist");
    }
}
