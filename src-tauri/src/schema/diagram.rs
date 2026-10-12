//! Everything the relationship diagram needs about a database in one read: its tables with their columns,
//! and the foreign keys between them.

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::Serialize;

use super::{optional, required, texts};
use crate::ddl::model::ReferentialAction;
use crate::error::AppResult;

/// More tables than this make a diagram nobody can read; the rest are left out and the result says so.
pub const MAX_DIAGRAM_TABLES: usize = 400;
const MAX_COLUMNS_PER_TABLE: usize = 500;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramColumn {
    pub name: String,
    pub column_type: String,
    pub primary_key: bool,
    pub nullable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramTable {
    pub name: String,
    pub estimated_rows: Option<u64>,
    pub columns: Vec<DiagramColumn>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagramRelation {
    pub name: String,
    pub table: String,
    pub columns: Vec<String>,
    pub ref_database: String,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_update: ReferentialAction,
    pub on_delete: ReferentialAction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diagram {
    pub database: String,
    pub tables: Vec<DiagramTable>,
    pub relations: Vec<DiagramRelation>,
    /// True when the database has more tables than `MAX_DIAGRAM_TABLES`.
    pub truncated: bool,
}

async fn read_tables(conn: &mut Conn, database: &str) -> AppResult<(Vec<DiagramTable>, bool)> {
    let rows: Vec<Row> = conn
        .exec(
            format!(
                "SELECT TABLE_NAME, TABLE_ROWS FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME LIMIT {}",
                MAX_DIAGRAM_TABLES + 1
            ),
            (database,),
        )
        .await?;
    let truncated = rows.len() > MAX_DIAGRAM_TABLES;
    let tables = rows
        .into_iter()
        .take(MAX_DIAGRAM_TABLES)
        .map(texts)
        .map(|row| DiagramTable {
            name: required(row.first()),
            estimated_rows: optional(row.get(1)).and_then(|rows| rows.parse().ok()),
            columns: Vec::new(),
        })
        .collect();
    Ok((tables, truncated))
}

async fn read_columns(
    conn: &mut Conn,
    database: &str,
    tables: &mut [DiagramTable],
) -> AppResult<()> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT TABLE_NAME, COLUMN_NAME, COLUMN_TYPE, COLUMN_KEY, IS_NULLABLE FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION",
            (database,),
        )
        .await?;
    for row in rows.into_iter().map(texts) {
        let table = required(row.first());
        let Some(target) = tables.iter_mut().find(|candidate| candidate.name == table) else {
            continue;
        };
        if target.columns.len() < MAX_COLUMNS_PER_TABLE {
            target.columns.push(DiagramColumn {
                name: required(row.get(1)),
                column_type: required(row.get(2)),
                primary_key: required(row.get(3)) == "PRI",
                nullable: required(row.get(4)) == "YES",
            });
        }
    }
    Ok(())
}

async fn read_relations(
    conn: &mut Conn,
    database: &str,
    tables: &[DiagramTable],
) -> AppResult<Vec<DiagramRelation>> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT k.TABLE_NAME, k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.UPDATE_RULE, r.DELETE_RULE \
             FROM information_schema.KEY_COLUMN_USAGE k \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS r ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
             WHERE k.TABLE_SCHEMA = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY k.TABLE_NAME, k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
            (database,),
        )
        .await?;
    let mut relations: Vec<DiagramRelation> = Vec::new();
    for row in rows.into_iter().map(texts) {
        let (table, name) = (required(row.first()), required(row.get(1)));
        if !tables.iter().any(|candidate| candidate.name == table) {
            continue;
        }
        let (column, referenced) = (required(row.get(2)), required(row.get(5)));
        if let Some(existing) = relations
            .iter_mut()
            .find(|relation| relation.table == table && relation.name == name)
        {
            existing.columns.push(column);
            existing.ref_columns.push(referenced);
            continue;
        }
        relations.push(DiagramRelation {
            name,
            table,
            columns: vec![column],
            ref_database: required(row.get(3)),
            ref_table: required(row.get(4)),
            ref_columns: vec![referenced],
            on_update: ReferentialAction::from_rule(&required(row.get(6))),
            on_delete: ReferentialAction::from_rule(&required(row.get(7))),
        });
    }
    Ok(relations)
}

pub async fn read_diagram(conn: &mut Conn, database: &str) -> AppResult<Diagram> {
    let (mut tables, truncated) = read_tables(conn, database).await?;
    read_columns(conn, database, &mut tables).await?;
    let relations = read_relations(conn, database, &tables).await?;
    Ok(Diagram {
        database: database.to_owned(),
        tables,
        relations,
        truncated,
    })
}
