//! What the server reports about itself: variables, status, running statements, engines, character sets, sizes.

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::Serialize;

use crate::error::{AppError, AppResult};
use crate::schema::{optional, required, texts};
use crate::sql::{is_plain_word, quote_string};

const MAX_INFO_CHARS: usize = 2_000;

#[derive(Debug, Clone, Serialize)]
pub struct NameValue {
    pub name: String,
    pub value: String,
}

async fn pairs(conn: &mut Conn, sql: &str) -> AppResult<Vec<NameValue>> {
    let rows: Vec<Row> = conn.query(sql).await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| NameValue {
            name: required(row.first()),
            value: required(row.get(1)),
        })
        .collect())
}

pub async fn variables(conn: &mut Conn) -> AppResult<Vec<NameValue>> {
    pairs(conn, "SHOW GLOBAL VARIABLES").await
}

pub async fn status(conn: &mut Conn) -> AppResult<Vec<NameValue>> {
    pairs(conn, "SHOW GLOBAL STATUS").await
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessInfo {
    pub id: u64,
    pub user: String,
    pub host: String,
    pub database: Option<String>,
    pub command: String,
    pub seconds: u64,
    pub state: Option<String>,
    /// The statement being run, cut to a sensible length.
    pub info: Option<String>,
}

fn shorten(text: String) -> String {
    match text.char_indices().nth(MAX_INFO_CHARS) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text,
    }
}

pub async fn processes(conn: &mut Conn) -> AppResult<Vec<ProcessInfo>> {
    let rows: Vec<Row> = conn.query("SHOW FULL PROCESSLIST").await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| ProcessInfo {
            id: required(row.first()).parse().unwrap_or(0),
            user: required(row.get(1)),
            host: required(row.get(2)),
            database: optional(row.get(3)),
            command: required(row.get(4)),
            seconds: required(row.get(5)).parse().unwrap_or(0),
            state: optional(row.get(6)),
            info: row.get(7).cloned().flatten().map(shorten),
        })
        .collect())
}

/// `KILL QUERY` stops the statement and keeps the connection; `KILL` closes the connection.
pub async fn kill(conn: &mut Conn, id: u64, query_only: bool) -> AppResult<()> {
    if id == 0 {
        return Err(AppError::Validation("Not a connection id".into()));
    }
    conn.query_drop(format!(
        "KILL {}{id}",
        if query_only { "QUERY " } else { "" }
    ))
    .await?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineInfo {
    pub name: String,
    pub support: String,
    pub comment: String,
    pub transactions: Option<String>,
    pub xa: Option<String>,
    pub savepoints: Option<String>,
}

pub async fn engines(conn: &mut Conn) -> AppResult<Vec<EngineInfo>> {
    let rows: Vec<Row> = conn.query("SELECT ENGINE, SUPPORT, COMMENT, TRANSACTIONS, XA, SAVEPOINTS FROM information_schema.ENGINES ORDER BY ENGINE").await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| EngineInfo {
            name: required(row.first()),
            support: required(row.get(1)),
            comment: required(row.get(2)),
            transactions: optional(row.get(3)),
            xa: optional(row.get(4)),
            savepoints: optional(row.get(5)),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharsetInfo {
    pub name: String,
    pub description: String,
    pub default_collation: String,
    pub max_length: u32,
}

pub async fn charsets(conn: &mut Conn) -> AppResult<Vec<CharsetInfo>> {
    let rows: Vec<Row> = conn
        .query("SELECT CHARACTER_SET_NAME, DESCRIPTION, DEFAULT_COLLATE_NAME, MAXLEN FROM information_schema.CHARACTER_SETS ORDER BY CHARACTER_SET_NAME")
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| CharsetInfo {
            name: required(row.first()),
            description: required(row.get(1)),
            default_collation: required(row.get(2)),
            max_length: required(row.get(3)).parse().unwrap_or(1),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseStats {
    pub name: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub tables: u64,
    /// The storage engine's estimate; exact for MyISAM.
    pub rows: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub free_bytes: u64,
}

/// `SUM()` comes back as a decimal string such as `12345`.
fn count(text: Option<&String>) -> u64 {
    text.and_then(|value| {
        value
            .parse::<u64>()
            .ok()
            .or_else(|| value.parse::<f64>().ok().map(|number| number as u64))
    })
    .unwrap_or(0)
}

pub async fn database_stats(conn: &mut Conn) -> AppResult<Vec<DatabaseStats>> {
    let rows: Vec<Row> = conn
        .query(
            "SELECT s.SCHEMA_NAME, s.DEFAULT_CHARACTER_SET_NAME, s.DEFAULT_COLLATION_NAME, COUNT(t.TABLE_NAME), \
             COALESCE(SUM(t.TABLE_ROWS), 0), COALESCE(SUM(t.DATA_LENGTH), 0), COALESCE(SUM(t.INDEX_LENGTH), 0), COALESCE(SUM(t.DATA_FREE), 0) \
             FROM information_schema.SCHEMATA s \
             LEFT JOIN information_schema.TABLES t ON t.TABLE_SCHEMA = s.SCHEMA_NAME AND t.TABLE_TYPE = 'BASE TABLE' \
             GROUP BY s.SCHEMA_NAME, s.DEFAULT_CHARACTER_SET_NAME, s.DEFAULT_COLLATION_NAME ORDER BY s.SCHEMA_NAME",
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| DatabaseStats {
            name: required(row.first()),
            charset: optional(row.get(1)),
            collation: optional(row.get(2)),
            tables: count(row.get(3).and_then(Option::as_ref)),
            rows: count(row.get(4).and_then(Option::as_ref)),
            data_bytes: count(row.get(5).and_then(Option::as_ref)),
            index_bytes: count(row.get(6).and_then(Option::as_ref)),
            free_bytes: count(row.get(7).and_then(Option::as_ref)),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub name: String,
    pub status: String,
    pub kind: String,
    pub library: Option<String>,
    pub description: Option<String>,
}

pub async fn plugins(conn: &mut Conn) -> AppResult<Vec<PluginInfo>> {
    let rows: Vec<Row> = conn
        .query("SELECT PLUGIN_NAME, PLUGIN_STATUS, PLUGIN_TYPE, PLUGIN_LIBRARY, PLUGIN_DESCRIPTION FROM information_schema.PLUGINS ORDER BY PLUGIN_TYPE, PLUGIN_NAME")
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| PluginInfo {
            name: required(row.first()),
            status: required(row.get(1)),
            kind: required(row.get(2)),
            library: optional(row.get(3)),
            description: optional(row.get(4)),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplicationInfo {
    /// What this server reports about the source it copies from; empty when it is not a replica.
    pub replica: Vec<NameValue>,
    /// This server's own binary log position, as a source; empty when binary logging is off.
    pub source: Vec<NameValue>,
}

/// The first row of the first statement the server accepts, as name and value pairs. Newer servers renamed
/// `SLAVE`, `MASTER` and `BINARY LOG STATUS`, so several spellings are tried. Empty when the server returns no row.
async fn first_row(conn: &mut Conn, statements: &[&str]) -> AppResult<Vec<NameValue>> {
    let mut last_error = None;
    for statement in statements {
        match conn.query::<Row, _>(*statement).await {
            Ok(rows) => {
                return Ok(rows
                    .into_iter()
                    .next()
                    .map(|row| {
                        let names: Vec<String> = row
                            .columns_ref()
                            .iter()
                            .map(|column| column.name_str().into_owned())
                            .collect();
                        names
                            .into_iter()
                            .zip(texts(row))
                            .filter_map(|(name, value)| {
                                value
                                    .filter(|text| !text.is_empty())
                                    .map(|value| NameValue { name, value })
                            })
                            .collect()
                    })
                    .unwrap_or_default());
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.map_or_else(
        || AppError::Validation("No statement to try".into()),
        AppError::from,
    ))
}

pub async fn replication(conn: &mut Conn) -> AppResult<ReplicationInfo> {
    let replica = first_row(conn, &["SHOW REPLICA STATUS", "SHOW SLAVE STATUS"]).await?;
    // Binary logging may be off, which is an answer ("nothing to show"), not a failure.
    let source = first_row(conn, &["SHOW BINARY LOG STATUS", "SHOW MASTER STATUS"])
        .await
        .unwrap_or_default();
    Ok(ReplicationInfo { replica, source })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinaryLog {
    pub name: String,
    pub bytes: u64,
}

pub async fn binary_logs(conn: &mut Conn) -> AppResult<Vec<BinaryLog>> {
    let rows: Vec<Row> = conn.query("SHOW BINARY LOGS").await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| BinaryLog {
            name: required(row.first()),
            bytes: count(row.get(1).and_then(Option::as_ref)),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BinlogEvent {
    pub position: u64,
    pub kind: String,
    pub server_id: u64,
    pub end_position: u64,
    pub info: String,
}

pub const MAX_BINLOG_EVENTS: u32 = 500;

/// The first events of one binary log. The file name is checked so it cannot carry anything but a name.
pub async fn binlog_events(conn: &mut Conn, file: &str, limit: u32) -> AppResult<Vec<BinlogEvent>> {
    if file.is_empty()
        || !file
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    {
        return Err(AppError::Validation(format!(
            "\"{file}\" is not a binary log name"
        )));
    }
    let rows: Vec<Row> = conn
        .query(format!(
            "SHOW BINLOG EVENTS IN {} LIMIT {}",
            quote_string(file),
            limit.clamp(1, MAX_BINLOG_EVENTS)
        ))
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| BinlogEvent {
            position: count(row.get(1).and_then(Option::as_ref)),
            kind: required(row.get(2)),
            server_id: count(row.get(3).and_then(Option::as_ref)),
            end_position: count(row.get(4).and_then(Option::as_ref)),
            info: shorten(required(row.get(5))),
        })
        .collect())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableStats {
    pub name: String,
    pub engine: Option<String>,
    pub rows: u64,
    pub data_bytes: u64,
    pub index_bytes: u64,
    pub free_bytes: u64,
    pub collation: Option<String>,
}

pub async fn table_stats(conn: &mut Conn, database: &str) -> AppResult<Vec<TableStats>> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT TABLE_NAME, ENGINE, TABLE_ROWS, DATA_LENGTH, INDEX_LENGTH, DATA_FREE, TABLE_COLLATION FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE' ORDER BY (DATA_LENGTH + INDEX_LENGTH) DESC, TABLE_NAME",
            (database,),
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| TableStats {
            name: required(row.first()),
            engine: optional(row.get(1)),
            rows: count(row.get(2).and_then(Option::as_ref)),
            data_bytes: count(row.get(3).and_then(Option::as_ref)),
            index_bytes: count(row.get(4).and_then(Option::as_ref)),
            free_bytes: count(row.get(5).and_then(Option::as_ref)),
            collation: optional(row.get(6)),
        })
        .collect())
}

/// How a value is written in `SET GLOBAL name = …`: numbers and the words ON/OFF/TRUE/FALSE/DEFAULT as they are
/// (a quoted number is refused for numeric variables), everything else as a string.
pub fn variable_literal(value: &str) -> String {
    let trimmed = value.trim();
    let digits = trimmed.strip_prefix('-').unwrap_or(trimmed);
    let mut parts = digits.splitn(2, '.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    let numeric = !whole.is_empty()
        && whole.chars().all(|c| c.is_ascii_digit())
        && fraction.is_none_or(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    if numeric {
        return trimmed.to_owned();
    }
    if ["ON", "OFF", "TRUE", "FALSE", "DEFAULT"]
        .iter()
        .any(|word| trimmed.eq_ignore_ascii_case(word))
    {
        return trimmed.to_ascii_uppercase();
    }
    quote_string(value)
}

pub fn check_variable_name(name: &str) -> AppResult<()> {
    if is_plain_word(name) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "\"{name}\" is not a variable name"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_and_keywords_are_bare_and_everything_else_is_a_string() {
        assert_eq!(variable_literal("200"), "200");
        assert_eq!(variable_literal(" -1 "), "-1");
        assert_eq!(variable_literal("0.5"), "0.5");
        assert_eq!(variable_literal("on"), "ON");
        assert_eq!(variable_literal("Default"), "DEFAULT");
        assert_eq!(
            variable_literal("STRICT_TRANS_TABLES,NO_ZERO_DATE"),
            "'STRICT_TRANS_TABLES,NO_ZERO_DATE'"
        );
        assert_eq!(variable_literal("utf8mb4"), "'utf8mb4'");
        assert_eq!(variable_literal("it's"), "'it''s'");
    }

    #[test]
    fn things_that_only_look_like_numbers_are_strings() {
        for text in ["inf", "NaN", "1e5", "12abc", "1.", ".5", "--1", "", "0x10"] {
            assert!(variable_literal(text).starts_with('\''), "{text}");
        }
    }

    #[test]
    fn variable_names_are_plain_words() {
        assert!(check_variable_name("max_connections").is_ok());
        assert!(check_variable_name("x = 1; DROP").is_err());
        assert!(check_variable_name("").is_err());
    }

    #[test]
    fn long_statements_are_cut_on_a_character_boundary() {
        let long = "é".repeat(MAX_INFO_CHARS + 5);

        assert_eq!(shorten(long).chars().count(), MAX_INFO_CHARS + 1);
        assert_eq!(shorten("SELECT 1".to_owned()), "SELECT 1");
    }

    #[test]
    fn sums_parse_whole_and_decimal_text() {
        assert_eq!(count(Some(&"12345".to_owned())), 12345);
        assert_eq!(count(Some(&"12345.0".to_owned())), 12345);
        assert_eq!(count(None), 0);
    }
}
