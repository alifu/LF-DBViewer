use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};
use crate::schema::{required, texts};
use crate::sql::qualified;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MaintenanceOp {
    Analyze,
    Optimize,
    Check,
    Repair,
    Checksum,
}

impl MaintenanceOp {
    fn keyword(self) -> &'static str {
        match self {
            Self::Analyze => "ANALYZE",
            Self::Optimize => "OPTIMIZE",
            Self::Check => "CHECK",
            Self::Repair => "REPAIR",
            Self::Checksum => "CHECKSUM",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaintenanceRow {
    pub table: String,
    /// `status`, `note`, `warning`, `error`, or `checksum`.
    pub kind: String,
    pub message: String,
}

pub fn maintenance_sql(op: MaintenanceOp, database: &str, tables: &[String]) -> AppResult<String> {
    if tables.is_empty() {
        return Err(AppError::Validation("Choose at least one table".into()));
    }
    let list: Vec<String> = tables
        .iter()
        .map(|table| qualified(database, table))
        .collect();
    Ok(format!("{} TABLE {}", op.keyword(), list.join(", ")))
}

/// The server answers with one row per message: `ANALYZE`, `OPTIMIZE`, `CHECK` and `REPAIR` give
/// `(Table, Op, Msg_type, Msg_text)`, `CHECKSUM` gives `(Table, Checksum)`.
pub async fn maintain(
    conn: &mut Conn,
    op: MaintenanceOp,
    database: &str,
    tables: &[String],
) -> AppResult<Vec<MaintenanceRow>> {
    let rows: Vec<Row> = conn.query(maintenance_sql(op, database, tables)?).await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| {
            if op == MaintenanceOp::Checksum {
                MaintenanceRow {
                    table: required(row.first()),
                    kind: "checksum".into(),
                    message: required(row.get(1)),
                }
            } else {
                MaintenanceRow {
                    table: required(row.first()),
                    kind: required(row.get(2)).to_ascii_lowercase(),
                    message: required(row.get(3)),
                }
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_statement_covers_every_chosen_table() {
        let tables = vec!["orders".to_owned(), "we`ird".to_owned()];

        assert_eq!(
            maintenance_sql(MaintenanceOp::Optimize, "shop", &tables).unwrap(),
            "OPTIMIZE TABLE `shop`.`orders`, `shop`.`we``ird`"
        );
        assert_eq!(
            maintenance_sql(MaintenanceOp::Checksum, "shop", &tables[..1]).unwrap(),
            "CHECKSUM TABLE `shop`.`orders`"
        );
        assert!(maintenance_sql(MaintenanceOp::Check, "shop", &[]).is_err());
    }

    #[test]
    fn operations_come_from_the_interface_by_name() {
        assert_eq!(
            serde_json::from_str::<MaintenanceOp>("\"repair\"").unwrap(),
            MaintenanceOp::Repair
        );
        assert!(serde_json::from_str::<MaintenanceOp>("\"drop\"").is_err());
    }
}
