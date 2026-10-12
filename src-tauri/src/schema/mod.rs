//! Reads what exists on the server: databases, their objects, and table columns.

use std::collections::BTreeMap;

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::query::cell::value_text;
use crate::sql::qualified;

pub mod definition;
pub mod diagram;
pub mod routines;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseInfo {
    pub name: String,
    pub charset: Option<String>,
    pub collation: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TableInfo {
    pub name: String,
    pub engine: Option<String>,
    /// The storage engine's estimate; exact for MyISAM, approximate for InnoDB.
    pub estimated_rows: Option<u64>,
    pub collation: Option<String>,
    pub comment: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaObjects {
    pub tables: Vec<TableInfo>,
    pub views: Vec<String>,
    pub procedures: Vec<String>,
    pub functions: Vec<String>,
    pub triggers: Vec<String>,
    pub events: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnInfo {
    pub name: String,
    /// Full type as declared, e.g. `varchar(255)` or `int unsigned`.
    pub column_type: String,
    pub nullable: bool,
    pub primary_key: bool,
    pub default: Option<String>,
    pub extra: String,
    pub collation: Option<String>,
    /// Generated columns cannot be written to.
    pub generated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectKind {
    Table,
    View,
    Procedure,
    Function,
    Trigger,
    Event,
}

impl ObjectKind {
    pub fn keyword(self) -> &'static str {
        match self {
            Self::Table => "TABLE",
            Self::View => "VIEW",
            Self::Procedure => "PROCEDURE",
            Self::Function => "FUNCTION",
            Self::Trigger => "TRIGGER",
            Self::Event => "EVENT",
        }
    }
}

pub(crate) fn texts(row: Row) -> Vec<Option<String>> {
    row.unwrap().iter().map(value_text).collect()
}

pub(crate) fn required(value: Option<&Option<String>>) -> String {
    value.cloned().flatten().unwrap_or_default()
}

pub(crate) fn optional(value: Option<&Option<String>>) -> Option<String> {
    value.cloned().flatten().filter(|text| !text.is_empty())
}

async fn names(conn: &mut Conn, sql: &str, database: &str) -> AppResult<Vec<String>> {
    let rows: Vec<Row> = conn.exec(sql, (database,)).await?;
    Ok(rows
        .into_iter()
        .map(|row| required(texts(row).first()))
        .collect())
}

pub async fn list_databases(conn: &mut Conn) -> AppResult<Vec<DatabaseInfo>> {
    let rows: Vec<Row> = conn
        .query(
            "SELECT SCHEMA_NAME, DEFAULT_CHARACTER_SET_NAME, DEFAULT_COLLATION_NAME \
             FROM information_schema.SCHEMATA ORDER BY SCHEMA_NAME",
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| DatabaseInfo {
            name: required(row.first()),
            charset: optional(row.get(1)),
            collation: optional(row.get(2)),
        })
        .collect())
}

async fn list_tables(conn: &mut Conn, database: &str) -> AppResult<Vec<TableInfo>> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT TABLE_NAME, ENGINE, TABLE_ROWS, TABLE_COLLATION, TABLE_COMMENT \
             FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_TYPE <> 'VIEW' ORDER BY TABLE_NAME",
            (database,),
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| TableInfo {
            name: required(row.first()),
            engine: optional(row.get(1)),
            estimated_rows: optional(row.get(2)).and_then(|rows| rows.parse().ok()),
            collation: optional(row.get(3)),
            comment: optional(row.get(4)),
        })
        .collect())
}

pub async fn list_objects(conn: &mut Conn, database: &str) -> AppResult<SchemaObjects> {
    const ROUTINES: &str = "SELECT ROUTINE_NAME FROM information_schema.ROUTINES \
                            WHERE ROUTINE_SCHEMA = ? AND ROUTINE_TYPE = ";
    Ok(SchemaObjects {
        tables: list_tables(conn, database).await?,
        views: names(
            conn,
            "SELECT TABLE_NAME FROM information_schema.VIEWS WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME",
            database,
        )
        .await?,
        procedures: names(conn, &format!("{ROUTINES}'PROCEDURE' ORDER BY ROUTINE_NAME"), database).await?,
        functions: names(conn, &format!("{ROUTINES}'FUNCTION' ORDER BY ROUTINE_NAME"), database).await?,
        triggers: names(
            conn,
            "SELECT TRIGGER_NAME FROM information_schema.TRIGGERS WHERE TRIGGER_SCHEMA = ? ORDER BY TRIGGER_NAME",
            database,
        )
        .await?,
        events: names(
            conn,
            "SELECT EVENT_NAME FROM information_schema.EVENTS WHERE EVENT_SCHEMA = ? ORDER BY EVENT_NAME",
            database,
        )
        .await?,
    })
}

fn column_from_row(row: Vec<Option<String>>) -> ColumnInfo {
    let extra = required(row.get(5));
    ColumnInfo {
        name: required(row.first()),
        column_type: required(row.get(1)),
        nullable: required(row.get(2)) == "YES",
        primary_key: required(row.get(3)) == "PRI",
        default: row.get(4).cloned().flatten(),
        generated: extra.to_ascii_uppercase().contains("GENERATED"),
        extra,
        collation: optional(row.get(6)),
    }
}

/// Columns in table order. An unknown table yields an error rather than an empty list.
pub async fn table_columns(
    conn: &mut Conn,
    database: &str,
    table: &str,
) -> AppResult<Vec<ColumnInfo>> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, EXTRA, COLLATION_NAME \
             FROM information_schema.COLUMNS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
            (database, table),
        )
        .await?;
    if rows.is_empty() {
        return Err(AppError::Validation(format!(
            "Table {database}.{table} does not exist or has no visible columns"
        )));
    }
    Ok(rows.into_iter().map(texts).map(column_from_row).collect())
}

/// Column names of every table and view in a database, for autocomplete. Capped so a
/// huge schema cannot make the editor slow.
pub const MAX_AUTOCOMPLETE_COLUMNS: usize = 20_000;

pub async fn database_column_names(
    conn: &mut Conn,
    database: &str,
) -> AppResult<BTreeMap<String, Vec<String>>> {
    let rows: Vec<Row> = conn
        .exec(
            format!(
                "SELECT TABLE_NAME, COLUMN_NAME FROM information_schema.COLUMNS \
                 WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION LIMIT {MAX_AUTOCOMPLETE_COLUMNS}"
            ),
            (database,),
        )
        .await?;
    let mut tables: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in rows {
        let values = texts(row);
        tables
            .entry(required(values.first()))
            .or_default()
            .push(required(values.get(1)));
    }
    Ok(tables)
}

/// The `CREATE …` statement for an object, as the server reports it.
pub async fn object_definition(
    conn: &mut Conn,
    database: &str,
    name: &str,
    kind: ObjectKind,
) -> AppResult<String> {
    let sql = format!(
        "SHOW CREATE {} {}",
        kind.keyword(),
        qualified(database, name)
    );
    let row: Option<Row> = conn.query_first(sql).await?;
    let row = row.ok_or_else(|| AppError::Validation(format!("{name} was not found")))?;
    // The statement sits in a different column for each kind, so find it by its heading.
    let position = row
        .columns_ref()
        .iter()
        .position(|column| {
            let heading = column.name_str();
            heading.starts_with("Create ") || heading == "SQL Original Statement"
        })
        .ok_or_else(|| AppError::Database("the server returned no definition".into()))?;
    texts(row)
        .into_iter()
        .nth(position)
        .flatten()
        .ok_or_else(|| AppError::Database(format!("you may lack the privilege to view {name}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(values: [Option<&str>; 7]) -> Vec<Option<String>> {
        values
            .iter()
            .map(|value| value.map(str::to_owned))
            .collect()
    }

    #[test]
    fn reads_a_primary_key_column() {
        let column = column_from_row(row([
            Some("id"),
            Some("bigint unsigned"),
            Some("NO"),
            Some("PRI"),
            None,
            Some("auto_increment"),
            None,
        ]));

        assert!(column.primary_key);
        assert!(!column.nullable);
        assert!(!column.generated);
        assert_eq!(column.default, None);
    }

    #[test]
    fn keeps_an_empty_string_default_and_flags_generated_columns() {
        let column = column_from_row(row([
            Some("full_name"),
            Some("varchar(90)"),
            Some("YES"),
            Some(""),
            Some(""),
            Some("VIRTUAL GENERATED"),
            Some("utf8mb4_0900_ai_ci"),
        ]));

        assert_eq!(column.default.as_deref(), Some(""));
        assert!(column.generated);
        assert!(column.nullable);
        assert_eq!(column.collation.as_deref(), Some("utf8mb4_0900_ai_ci"));
    }
}
