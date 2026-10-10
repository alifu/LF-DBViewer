//! Turns grid edits into parameterised statements and applies them in one transaction.

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, TxOpts, Value};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::schema::{ColumnInfo, table_columns};
use crate::sql::{placeholders, qualified, quote_identifier};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnValue {
    pub column: String,
    /// `None` is SQL NULL.
    pub value: Option<String>,
}

/// `key` identifies the row by the values it had when it was loaded: the primary key,
/// or every column when the table has none.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RowChange {
    Update {
        key: Vec<ColumnValue>,
        set: Vec<ColumnValue>,
    },
    Insert {
        values: Vec<ColumnValue>,
    },
    Delete {
        key: Vec<ColumnValue>,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    pub statements: Vec<String>,
    pub affected_rows: u64,
}

#[derive(Debug, PartialEq)]
struct Statement {
    sql: String,
    params: Vec<Value>,
    /// Updates and deletes must hit exactly the one row they were made for.
    must_match_one_row: bool,
}

fn to_param(value: &Option<String>) -> Value {
    value.as_deref().map_or(Value::NULL, Value::from)
}

fn check_columns(columns: &[ColumnInfo], values: &[ColumnValue], writing: bool) -> AppResult<()> {
    for value in values {
        let column = columns
            .iter()
            .find(|column| column.name == value.column)
            .ok_or_else(|| AppError::Validation(format!("Unknown column: {}", value.column)))?;
        if writing && column.generated {
            return Err(AppError::Validation(format!(
                "{} is a generated column and cannot be written",
                column.name
            )));
        }
    }
    Ok(())
}

/// `<=>` is the NULL-safe equality, so a key value of NULL still finds its row.
fn where_clause(key: &[ColumnValue]) -> AppResult<String> {
    if key.is_empty() {
        return Err(AppError::Validation(
            "A change is missing the row it applies to".into(),
        ));
    }
    Ok(key
        .iter()
        .map(|part| format!("{} <=> ?", quote_identifier(&part.column)))
        .collect::<Vec<_>>()
        .join(" AND "))
}

fn build_statement(
    target: &str,
    columns: &[ColumnInfo],
    change: &RowChange,
) -> AppResult<Statement> {
    match change {
        RowChange::Update { key, set } => {
            if set.is_empty() {
                return Err(AppError::Validation(
                    "An update has no changed columns".into(),
                ));
            }
            check_columns(columns, key, false)?;
            check_columns(columns, set, true)?;
            let assignments: Vec<String> = set
                .iter()
                .map(|part| format!("{} = ?", quote_identifier(&part.column)))
                .collect();
            Ok(Statement {
                sql: format!(
                    "UPDATE {target} SET {} WHERE {} LIMIT 1",
                    assignments.join(", "),
                    where_clause(key)?
                ),
                params: set
                    .iter()
                    .chain(key)
                    .map(|part| to_param(&part.value))
                    .collect(),
                must_match_one_row: true,
            })
        }
        RowChange::Insert { values } => {
            check_columns(columns, values, true)?;
            let names: Vec<String> = values
                .iter()
                .map(|part| quote_identifier(&part.column))
                .collect();
            Ok(Statement {
                sql: format!(
                    "INSERT INTO {target} ({}) VALUES ({})",
                    names.join(", "),
                    placeholders(values.len())
                ),
                params: values.iter().map(|part| to_param(&part.value)).collect(),
                must_match_one_row: false,
            })
        }
        RowChange::Delete { key } => {
            check_columns(columns, key, false)?;
            Ok(Statement {
                sql: format!("DELETE FROM {target} WHERE {} LIMIT 1", where_clause(key)?),
                params: key.iter().map(|part| to_param(&part.value)).collect(),
                must_match_one_row: true,
            })
        }
    }
}

/// Applies every change or none: any failure, or a row that can no longer be found, rolls back.
pub async fn apply_changes(
    conn: &mut Conn,
    database: &str,
    table: &str,
    changes: &[RowChange],
) -> AppResult<ApplyResult> {
    if changes.is_empty() {
        return Err(AppError::Validation("There are no changes to apply".into()));
    }
    let columns = table_columns(conn, database, table).await?;
    let target = qualified(database, table);
    let statements: Vec<Statement> = changes
        .iter()
        .map(|change| build_statement(&target, &columns, change))
        .collect::<AppResult<_>>()?;

    let mut transaction = conn.start_transaction(TxOpts::default()).await?;
    let mut affected_rows = 0;
    for statement in &statements {
        let outcome = transaction
            .exec_drop(statement.sql.as_str(), statement.params.clone())
            .await;
        let matched = transaction.affected_rows();
        let failure = match outcome {
            Err(error) => Some(AppError::from(error)),
            Ok(()) if statement.must_match_one_row && matched != 1 => Some(AppError::Validation(
                "A row could not be found; it was probably changed or deleted by someone else. \
                 Nothing was saved. Reload and try again."
                    .into(),
            )),
            Ok(()) => None,
        };
        if let Some(error) = failure {
            transaction.rollback().await?;
            return Err(error);
        }
        affected_rows += matched;
    }
    transaction.commit().await?;

    Ok(ApplyResult {
        statements: statements
            .into_iter()
            .map(|statement| statement.sql)
            .collect(),
        affected_rows,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::browse::sample_columns;

    const TARGET: &str = "`shop`.`orders`";

    fn value(column: &str, value: Option<&str>) -> ColumnValue {
        ColumnValue {
            column: column.into(),
            value: value.map(str::to_owned),
        }
    }

    fn build(change: RowChange) -> AppResult<Statement> {
        build_statement(
            TARGET,
            &sample_columns(&["id"], &["total", "note"]),
            &change,
        )
    }

    #[test]
    fn update_sets_changed_columns_for_one_keyed_row() {
        let statement = build(RowChange::Update {
            key: vec![value("id", Some("7"))],
            set: vec![value("total", Some("12.50")), value("note", None)],
        })
        .unwrap();

        assert_eq!(
            statement.sql,
            "UPDATE `shop`.`orders` SET `total` = ?, `note` = ? WHERE `id` <=> ? LIMIT 1"
        );
        assert_eq!(
            statement.params,
            vec![Value::from("12.50"), Value::NULL, Value::from("7")]
        );
        assert!(statement.must_match_one_row);
    }

    #[test]
    fn delete_matches_null_key_values_safely() {
        let statement = build(RowChange::Delete {
            key: vec![value("total", Some("3")), value("note", None)],
        })
        .unwrap();

        assert_eq!(
            statement.sql,
            "DELETE FROM `shop`.`orders` WHERE `total` <=> ? AND `note` <=> ? LIMIT 1"
        );
        assert_eq!(statement.params, vec![Value::from("3"), Value::NULL]);
    }

    #[test]
    fn insert_lists_only_the_given_columns_and_may_be_empty() {
        let filled = build(RowChange::Insert {
            values: vec![value("total", Some("1"))],
        })
        .unwrap();
        let defaults_only = build(RowChange::Insert { values: vec![] }).unwrap();

        assert_eq!(
            filled.sql,
            "INSERT INTO `shop`.`orders` (`total`) VALUES (?)"
        );
        assert_eq!(
            defaults_only.sql,
            "INSERT INTO `shop`.`orders` () VALUES ()"
        );
        assert!(!filled.must_match_one_row);
    }

    #[test]
    fn rejects_changes_without_a_key_or_without_columns() {
        let broken = [
            RowChange::Update {
                key: vec![],
                set: vec![value("total", Some("1"))],
            },
            RowChange::Update {
                key: vec![value("id", Some("1"))],
                set: vec![],
            },
            RowChange::Delete { key: vec![] },
            RowChange::Insert {
                values: vec![value("nope", Some("1"))],
            },
        ];

        for change in broken {
            assert!(matches!(build(change), Err(AppError::Validation(_))));
        }
    }

    #[test]
    fn rejects_writes_to_generated_columns_but_allows_them_in_keys() {
        let mut columns = sample_columns(&[], &["price", "price_with_tax"]);
        columns[1].generated = true;
        let write = RowChange::Insert {
            values: vec![value("price_with_tax", Some("1"))],
        };
        let key_only = RowChange::Delete {
            key: vec![value("price_with_tax", Some("1"))],
        };

        assert!(matches!(
            build_statement(TARGET, &columns, &write),
            Err(AppError::Validation(_))
        ));
        assert!(build_statement(TARGET, &columns, &key_only).is_ok());
    }

    #[test]
    fn changes_deserialize_from_tagged_json() {
        let json = r#"[{"kind":"update","key":[{"column":"id","value":"1"}],"set":[{"column":"note","value":null}]},
                       {"kind":"insert","values":[]},
                       {"kind":"delete","key":[{"column":"id","value":"2"}]}]"#;

        let changes: Vec<RowChange> = serde_json::from_str(json).unwrap();

        assert!(matches!(changes[0], RowChange::Update { .. }));
        assert!(matches!(changes[2], RowChange::Delete { .. }));
    }
}
