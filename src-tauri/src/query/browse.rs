//! Reads one page of a table, with server-side filtering, sorting and paging.

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row, Value};
use serde::{Deserialize, Serialize};

use super::cell::{Cell, ColumnShape, cell_from_value, value_text};
use super::edit::ColumnValue;
use crate::error::{AppError, AppResult};
use crate::schema::{ColumnInfo, table_columns};
use crate::sql::{placeholders, qualified, quote_identifier};

pub const MAX_PAGE_ROWS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FilterOperator {
    Equals,
    NotEquals,
    Less,
    LessOrEqual,
    Greater,
    GreaterOrEqual,
    Contains,
    Like,
    NotLike,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    pub column: String,
    pub operator: FilterOperator,
    #[serde(default)]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SortKey {
    pub column: String,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRequest {
    pub limit: usize,
    #[serde(default)]
    pub offset: u64,
    /// Primary key of the last row already loaded. Used instead of `offset` when the
    /// table is read in primary key order, so deep pages stay as fast as the first.
    #[serde(default)]
    pub after: Option<Vec<String>>,
    #[serde(default)]
    pub sort: Vec<SortKey>,
    #[serde(default)]
    pub filters: Vec<Filter>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TablePage {
    pub columns: Vec<ColumnInfo>,
    pub primary_key: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
    pub has_more: bool,
    /// True when the next page can be requested with `after`.
    pub keyset: bool,
    pub sql: String,
}

#[derive(Debug, PartialEq, Eq)]
struct SelectStatement {
    sql: String,
    params: Vec<String>,
    keyset: bool,
}

fn known_column<'a>(columns: &'a [ColumnInfo], name: &str) -> AppResult<&'a ColumnInfo> {
    columns
        .iter()
        .find(|column| column.name == name)
        .ok_or_else(|| AppError::Validation(format!("Unknown column: {name}")))
}

fn filter_sql(filter: &Filter) -> AppResult<(String, Option<String>)> {
    let column = quote_identifier(&filter.column);
    let comparison = match filter.operator {
        FilterOperator::IsNull => return Ok((format!("{column} IS NULL"), None)),
        FilterOperator::IsNotNull => return Ok((format!("{column} IS NOT NULL"), None)),
        FilterOperator::Equals => "= ?",
        FilterOperator::NotEquals => "<> ?",
        FilterOperator::Less => "< ?",
        FilterOperator::LessOrEqual => "<= ?",
        FilterOperator::Greater => "> ?",
        FilterOperator::GreaterOrEqual => ">= ?",
        FilterOperator::Contains => "LIKE CONCAT('%', ?, '%')",
        FilterOperator::Like => "LIKE ?",
        FilterOperator::NotLike => "NOT LIKE ?",
    };
    let value = filter.value.clone().ok_or_else(|| {
        AppError::Validation(format!("Filter on {} needs a value", filter.column))
    })?;
    Ok((format!("{column} {comparison}"), Some(value)))
}

/// The `WHERE` conditions and their parameters for a list of filters on known columns.
fn filter_conditions(
    columns: &[ColumnInfo],
    filters: &[Filter],
) -> AppResult<(Vec<String>, Vec<String>)> {
    let mut conditions = Vec::new();
    let mut params = Vec::new();
    for filter in filters {
        known_column(columns, &filter.column)?;
        let (condition, value) = filter_sql(filter)?;
        conditions.push(condition);
        params.extend(value);
    }
    Ok((conditions, params))
}

fn build_count(
    database: &str,
    table: &str,
    columns: &[ColumnInfo],
    filters: &[Filter],
) -> AppResult<(String, Vec<String>)> {
    let (conditions, params) = filter_conditions(columns, filters)?;
    let mut sql = format!("SELECT COUNT(*) FROM {}", qualified(database, table));
    if !conditions.is_empty() {
        sql.push_str(&format!(" WHERE {}", conditions.join(" AND ")));
    }
    Ok((sql, params))
}

/// The exact number of rows that match the filters. Slow on a big table, which is why it is asked for, not shown by default.
pub async fn count_rows(
    conn: &mut Conn,
    database: &str,
    table: &str,
    filters: &[Filter],
) -> AppResult<u64> {
    let columns = table_columns(conn, database, table).await?;
    let (sql, params) = build_count(database, table, &columns, filters)?;
    let count: Option<u64> = conn.exec_first(sql, params).await?;
    Ok(count.unwrap_or(0))
}

/// Text longer than this is cut when a single cell is opened; the cell dialog says so.
pub const MAX_CELL_CHARS: u64 = 2_000_000;
/// Binary values are shown as hex up to this many bytes.
pub const MAX_CELL_BINARY_BYTES: usize = 64 * 1024;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CellValue {
    pub binary: bool,
    /// The text, or the bytes as hex when `binary`.
    pub text: String,
    /// Size of the whole value in bytes.
    pub size: u64,
    pub truncated: bool,
}

/// One cell in full, found by the key it had when the row was loaded (the primary key, or every plain column).
pub async fn read_cell(
    conn: &mut Conn,
    database: &str,
    table: &str,
    column: &str,
    key: &[ColumnValue],
) -> AppResult<CellValue> {
    let columns = table_columns(conn, database, table).await?;
    known_column(&columns, column)?;
    if key.is_empty() {
        return Err(AppError::Validation("The row cannot be identified".into()));
    }
    for part in key {
        known_column(&columns, &part.column)?;
    }
    let target = quote_identifier(column);
    let conditions: Vec<String> = key
        .iter()
        .map(|part| format!("{} <=> ?", quote_identifier(&part.column)))
        .collect();
    let sql = format!(
        "SELECT OCTET_LENGTH({target}), SUBSTRING({target}, 1, {MAX_CELL_CHARS}) FROM {} WHERE {} LIMIT 2",
        qualified(database, table),
        conditions.join(" AND ")
    );
    let params: Vec<Value> = key
        .iter()
        .map(|part| part.value.as_deref().map_or(Value::NULL, Value::from))
        .collect();

    let mut result = conn.exec_iter(sql, params).await?;
    let binary = result
        .columns()
        .is_some_and(|found| found.get(1).is_some_and(|c| ColumnShape::of(c).is_binary));
    let rows: Vec<Row> = result.collect().await?;
    let [row] = <[Row; 1]>::try_from(rows).map_err(|rows| {
        AppError::Validation(
            if rows.is_empty() {
                "The row no longer exists. Reload and try again."
            } else {
                "More than one row matches; the cell cannot be told apart."
            }
            .into(),
        )
    })?;
    let mut values = row.unwrap();
    let content = values.pop().unwrap_or(Value::NULL);
    let size = values
        .pop()
        .and_then(|value| value_text(&value))
        .and_then(|text| text.parse().ok())
        .unwrap_or(0);
    Ok(match content {
        Value::Bytes(bytes) if binary => {
            let shown = bytes.len().min(MAX_CELL_BINARY_BYTES);
            CellValue {
                binary: true,
                text: bytes[..shown]
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
                size,
                truncated: shown < size as usize,
            }
        }
        Value::Bytes(bytes) => CellValue {
            binary: false,
            text: String::from_utf8_lossy(&bytes).into_owned(),
            size,
            truncated: size > bytes.len() as u64,
        },
        other => CellValue {
            binary: false,
            text: value_text(&other).unwrap_or_default(),
            size,
            truncated: false,
        },
    })
}

fn build_select(
    database: &str,
    table: &str,
    columns: &[ColumnInfo],
    request: &PageRequest,
) -> AppResult<SelectStatement> {
    let limit = request.limit.clamp(1, MAX_PAGE_ROWS);
    let primary_key: Vec<String> = columns
        .iter()
        .filter(|column| column.primary_key)
        .map(|column| quote_identifier(&column.name))
        .collect();
    let keyset = request.sort.is_empty() && !primary_key.is_empty();

    let (mut conditions, mut params) = filter_conditions(columns, &request.filters)?;

    let cursor = request.after.as_ref().filter(|_| keyset);
    if let Some(after) = cursor {
        if after.len() != primary_key.len() {
            return Err(AppError::Validation(
                "Paging cursor does not match the primary key".into(),
            ));
        }
        conditions.push(format!(
            "({}) > ({})",
            primary_key.join(", "),
            placeholders(after.len())
        ));
        params.extend(after.iter().cloned());
    }

    let order: Vec<String> = if keyset {
        primary_key
    } else {
        request
            .sort
            .iter()
            .map(|key| {
                known_column(columns, &key.column)?;
                let direction = if key.descending { " DESC" } else { "" };
                Ok(format!("{}{direction}", quote_identifier(&key.column)))
            })
            .collect::<AppResult<_>>()?
    };

    let mut sql = format!("SELECT * FROM {}", qualified(database, table));
    if !conditions.is_empty() {
        sql.push_str(&format!(" WHERE {}", conditions.join(" AND ")));
    }
    if !order.is_empty() {
        sql.push_str(&format!(" ORDER BY {}", order.join(", ")));
    }
    // One extra row tells us whether another page exists without a COUNT(*).
    sql.push_str(&format!(" LIMIT {}", limit + 1));
    if cursor.is_none() && request.offset > 0 {
        sql.push_str(&format!(" OFFSET {}", request.offset));
    }
    Ok(SelectStatement {
        sql,
        params,
        keyset,
    })
}

pub async fn browse_table(
    conn: &mut Conn,
    database: &str,
    table: &str,
    request: &PageRequest,
) -> AppResult<TablePage> {
    let columns = table_columns(conn, database, table).await?;
    let statement = build_select(database, table, &columns, request)?;
    let limit = request.limit.clamp(1, MAX_PAGE_ROWS);

    let mut result = conn
        .exec_iter(statement.sql.as_str(), statement.params)
        .await?;
    let shapes: Vec<ColumnShape> = result.columns_ref().iter().map(ColumnShape::of).collect();
    let fetched: Vec<Row> = result.collect().await?;
    drop(result);

    let has_more = fetched.len() > limit;
    let rows = fetched
        .into_iter()
        .take(limit)
        .map(|row| {
            row.unwrap()
                .into_iter()
                .zip(&shapes)
                .map(|(value, shape)| cell_from_value(value, *shape))
                .collect()
        })
        .collect();

    Ok(TablePage {
        primary_key: columns
            .iter()
            .filter(|column| column.primary_key)
            .map(|column| column.name.clone())
            .collect(),
        columns,
        rows,
        has_more,
        keyset: statement.keyset,
        sql: statement.sql,
    })
}

#[cfg(test)]
pub(crate) fn sample_columns(primary_key: &[&str], others: &[&str]) -> Vec<ColumnInfo> {
    let column = |name: &&str, primary_key: bool| ColumnInfo {
        name: (*name).to_owned(),
        column_type: "int".into(),
        nullable: !primary_key,
        primary_key,
        default: None,
        extra: String::new(),
        collation: None,
        generated: false,
    };
    primary_key
        .iter()
        .map(|name| column(name, true))
        .chain(others.iter().map(|name| column(name, false)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn select(columns: &[ColumnInfo], request: PageRequest) -> SelectStatement {
        build_select("shop", "orders", columns, &request).unwrap()
    }

    #[test]
    fn first_page_of_a_keyed_table_is_in_primary_key_order() {
        let statement = select(
            &sample_columns(&["id"], &["total"]),
            PageRequest {
                limit: 100,
                ..Default::default()
            },
        );

        assert_eq!(
            statement.sql,
            "SELECT * FROM `shop`.`orders` ORDER BY `id` LIMIT 101"
        );
        assert!(statement.keyset);
        assert!(statement.params.is_empty());
    }

    #[test]
    fn later_pages_continue_after_the_last_key_instead_of_using_offset() {
        let request = PageRequest {
            limit: 2,
            offset: 500,
            after: Some(vec!["7".into(), "b".into()]),
            ..Default::default()
        };

        let statement = select(&sample_columns(&["tenant", "code"], &[]), request);

        assert_eq!(
            statement.sql,
            "SELECT * FROM `shop`.`orders` WHERE (`tenant`, `code`) > (?, ?) ORDER BY `tenant`, `code` LIMIT 3"
        );
        assert_eq!(statement.params, vec!["7", "b"]);
    }

    #[test]
    fn custom_sort_uses_offset_paging() {
        let request = PageRequest {
            limit: 50,
            offset: 100,
            after: Some(vec!["9".into()]),
            sort: vec![SortKey {
                column: "total".into(),
                descending: true,
            }],
            ..Default::default()
        };

        let statement = select(&sample_columns(&["id"], &["total"]), request);

        assert_eq!(
            statement.sql,
            "SELECT * FROM `shop`.`orders` ORDER BY `total` DESC LIMIT 51 OFFSET 100"
        );
        assert!(!statement.keyset);
    }

    #[test]
    fn filters_are_parameterised_and_combined_with_the_cursor() {
        let request = PageRequest {
            limit: 10,
            after: Some(vec!["40".into()]),
            filters: vec![
                Filter {
                    column: "total".into(),
                    operator: FilterOperator::GreaterOrEqual,
                    value: Some("5".into()),
                },
                Filter {
                    column: "note".into(),
                    operator: FilterOperator::IsNull,
                    value: None,
                },
                Filter {
                    column: "note".into(),
                    operator: FilterOperator::Contains,
                    value: Some("x' OR 1=1".into()),
                },
            ],
            ..Default::default()
        };

        let statement = select(&sample_columns(&["id"], &["total", "note"]), request);

        assert_eq!(
            statement.sql,
            "SELECT * FROM `shop`.`orders` WHERE `total` >= ? AND `note` IS NULL \
             AND `note` LIKE CONCAT('%', ?, '%') AND (`id`) > (?) ORDER BY `id` LIMIT 11"
        );
        assert_eq!(statement.params, vec!["5", "x' OR 1=1", "40"]);
    }

    #[test]
    fn table_without_a_primary_key_has_no_order_and_no_keyset() {
        let statement = select(
            &sample_columns(&[], &["a"]),
            PageRequest {
                limit: 10,
                offset: 20,
                ..Default::default()
            },
        );

        assert_eq!(
            statement.sql,
            "SELECT * FROM `shop`.`orders` LIMIT 11 OFFSET 20"
        );
        assert!(!statement.keyset);
    }

    #[test]
    fn page_size_is_clamped() {
        let columns = sample_columns(&[], &["a"]);

        assert!(
            select(
                &columns,
                PageRequest {
                    limit: 0,
                    ..Default::default()
                }
            )
            .sql
            .ends_with("LIMIT 2")
        );
        assert!(
            select(
                &columns,
                PageRequest {
                    limit: 9_999_999,
                    ..Default::default()
                }
            )
            .sql
            .ends_with("LIMIT 1001")
        );
    }

    #[test]
    fn rejects_unknown_columns_missing_values_and_bad_cursors() {
        let columns = sample_columns(&["id"], &["total"]);
        let bad_requests = [
            PageRequest {
                limit: 1,
                sort: vec![SortKey {
                    column: "nope".into(),
                    descending: false,
                }],
                ..Default::default()
            },
            PageRequest {
                limit: 1,
                filters: vec![Filter {
                    column: "nope".into(),
                    operator: FilterOperator::IsNull,
                    value: None,
                }],
                ..Default::default()
            },
            PageRequest {
                limit: 1,
                filters: vec![Filter {
                    column: "total".into(),
                    operator: FilterOperator::Equals,
                    value: None,
                }],
                ..Default::default()
            },
            PageRequest {
                limit: 1,
                after: Some(vec!["1".into(), "2".into()]),
                ..Default::default()
            },
        ];

        for request in bad_requests {
            assert!(matches!(
                build_select("shop", "orders", &columns, &request),
                Err(AppError::Validation(_))
            ));
        }
    }

    #[test]
    fn page_request_deserializes_from_what_the_interface_sends() {
        let json = r#"{"limit":500,"sort":[{"column":"total","descending":true}],
                       "filters":[{"column":"note","operator":"isNull","value":null},
                                  {"column":"total","operator":"greaterOrEqual","value":"5"}],
                       "after":["41"]}"#;

        let request: PageRequest = serde_json::from_str(json).unwrap();

        assert_eq!(request.limit, 500);
        assert_eq!(request.offset, 0);
        assert!(request.sort[0].descending);
        assert_eq!(request.filters[1].operator, FilterOperator::GreaterOrEqual);
        assert_eq!(request.after, Some(vec!["41".to_owned()]));
    }
}
