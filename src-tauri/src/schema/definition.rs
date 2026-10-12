//! Reads a table's full structure from `information_schema`, in the shape the designer edits.
//! `information_schema` is used rather than parsing `SHOW CREATE TABLE` because it reads the
//! same on MySQL and MariaDB, apart from how defaults are written, handled in `parse_default`.

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::Serialize;

use super::{optional, required, texts};
use crate::connection::model::Engine;
use crate::ddl::model::{
    ColumnDefault, ColumnDefinition, ForeignKeyDefinition, Generated, IndexColumn, IndexDefinition,
    IndexKind, ReferentialAction, TableDefinition, TableOptions,
};
use crate::error::{AppError, AppResult};

const PRIMARY_KEY_NAME: &str = "PRIMARY";

/// How the server reports a default differs: MySQL gives the bare value and flags expressions in
/// `EXTRA`; MariaDB writes strings quoted, the word `NULL` for `DEFAULT NULL`, and expressions bare.
pub fn parse_default(
    engine: Engine,
    raw: Option<&str>,
    extra: &str,
    nullable: bool,
    column_type: &str,
) -> ColumnDefault {
    match engine {
        Engine::Mysql => match raw {
            None if nullable => ColumnDefault::Null,
            None => ColumnDefault::None,
            Some(value) if extra.to_ascii_uppercase().contains("DEFAULT_GENERATED") => {
                ColumnDefault::Expression(value.to_owned())
            }
            Some(value)
                if column_type.to_ascii_lowercase().starts_with("bit")
                    && value.starts_with("b'") =>
            {
                ColumnDefault::Expression(value.to_owned())
            }
            Some(value) => ColumnDefault::Value(value.to_owned()),
        },
        Engine::Mariadb => match raw {
            None => ColumnDefault::None,
            Some("NULL") => ColumnDefault::Null,
            Some(value) if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') => {
                ColumnDefault::Value(
                    value[1..value.len() - 1]
                        .replace("\\'", "'")
                        .replace("''", "'"),
                )
            }
            Some(value) if value.parse::<f64>().is_ok() => ColumnDefault::Value(value.to_owned()),
            Some(value) => ColumnDefault::Expression(value.to_owned()),
        },
    }
}

/// The expression after `on update` in `EXTRA`, e.g. `CURRENT_TIMESTAMP(6)`.
pub fn parse_on_update(extra: &str) -> Option<String> {
    let lower = extra.to_ascii_lowercase();
    let start = lower.find("on update ")? + "on update ".len();
    extra[start..].split_whitespace().next().map(str::to_owned)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CollationInfo {
    pub name: String,
    pub charset: String,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerOptions {
    pub collations: Vec<CollationInfo>,
    pub engines: Vec<String>,
}

pub async fn server_options(conn: &mut Conn) -> AppResult<ServerOptions> {
    let collations: Vec<Row> = conn
        .query("SELECT COLLATION_NAME, CHARACTER_SET_NAME, IS_DEFAULT FROM information_schema.COLLATIONS ORDER BY CHARACTER_SET_NAME, COLLATION_NAME")
        .await?;
    let engines: Vec<Row> = conn
        .query("SELECT ENGINE FROM information_schema.ENGINES WHERE SUPPORT IN ('YES', 'DEFAULT') ORDER BY ENGINE")
        .await?;
    Ok(ServerOptions {
        collations: collations
            .into_iter()
            .map(texts)
            .map(|row| CollationInfo {
                name: required(row.first()),
                charset: required(row.get(1)),
                is_default: required(row.get(2)) == "Yes",
            })
            .collect(),
        engines: engines
            .into_iter()
            .map(|row| required(texts(row).first()))
            .collect(),
    })
}

async fn read_options(conn: &mut Conn, database: &str, table: &str) -> AppResult<TableOptions> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT ENGINE, ROW_FORMAT, AUTO_INCREMENT, TABLE_COLLATION, TABLE_COMMENT FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? AND TABLE_TYPE = 'BASE TABLE'",
            (database, table),
        )
        .await?;
    let row = rows
        .into_iter()
        .next()
        .map(texts)
        .ok_or_else(|| AppError::Validation(format!("{database}.{table} is not a table")))?;
    Ok(TableOptions {
        engine: optional(row.first()),
        row_format: optional(row.get(1)),
        auto_increment: optional(row.get(2)).and_then(|value| value.parse().ok()),
        collation: optional(row.get(3)),
        comment: required(row.get(4)),
    })
}

async fn read_columns(
    conn: &mut Conn,
    engine: Engine,
    database: &str,
    table: &str,
) -> AppResult<Vec<ColumnDefinition>> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_DEFAULT, EXTRA, COLLATION_NAME, COLUMN_COMMENT, GENERATION_EXPRESSION \
             FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
            (database, table),
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| {
            let name = required(row.first());
            let column_type = required(row.get(1));
            let nullable = required(row.get(2)) == "YES";
            let extra = required(row.get(4));
            let generated = optional(row.get(7)).map(|expression| Generated {
                expression,
                stored: extra.to_ascii_uppercase().contains("STORED"),
            });
            ColumnDefinition {
                original_name: Some(name.clone()),
                default: if generated.is_some() {
                    ColumnDefault::None
                } else {
                    parse_default(
                        engine,
                        row.get(3).and_then(Option::as_deref),
                        &extra,
                        nullable,
                        &column_type,
                    )
                },
                auto_increment: extra.to_ascii_lowercase().contains("auto_increment"),
                on_update: parse_on_update(&extra),
                collation: optional(row.get(5)),
                comment: required(row.get(6)),
                generated,
                name,
                column_type,
                nullable,
            }
        })
        .collect())
}

async fn read_indexes(
    conn: &mut Conn,
    database: &str,
    table: &str,
) -> AppResult<Vec<IndexDefinition>> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT INDEX_NAME, NON_UNIQUE, INDEX_TYPE, COLUMN_NAME, SUB_PART, COLLATION FROM information_schema.STATISTICS \
             WHERE TABLE_SCHEMA = ? AND TABLE_NAME = ? ORDER BY INDEX_NAME, SEQ_IN_INDEX",
            (database, table),
        )
        .await?;
    let mut indexes: Vec<IndexDefinition> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for row in rows.into_iter().map(texts) {
        let name = required(row.first());
        // Indexes on an expression have no column name; they cannot be edited here, so they are left out entirely.
        let Some(column) = optional(row.get(3)) else {
            skipped.push(name);
            continue;
        };
        let part = IndexColumn {
            name: column,
            length: optional(row.get(4)).and_then(|value| value.parse().ok()),
            descending: required(row.get(5)) == "D",
        };
        if let Some(existing) = indexes.iter_mut().find(|index| index.name == name) {
            existing.columns.push(part);
            continue;
        }
        let index_type = required(row.get(2)).to_ascii_uppercase();
        let kind = match (
            name.as_str(),
            index_type.as_str(),
            required(row.get(1)).as_str(),
        ) {
            (PRIMARY_KEY_NAME, _, _) => IndexKind::Primary,
            (_, "FULLTEXT", _) => IndexKind::Fulltext,
            (_, "SPATIAL", _) => IndexKind::Spatial,
            (_, _, "0") => IndexKind::Unique,
            _ => IndexKind::Index,
        };
        indexes.push(IndexDefinition {
            original_name: Some(name.clone()),
            name,
            kind,
            columns: vec![part],
        });
    }
    indexes.retain(|index| !skipped.contains(&index.name));
    indexes.sort_by_key(|index| (index.kind != IndexKind::Primary, index.name.to_lowercase()));
    Ok(indexes)
}

async fn read_foreign_keys(
    conn: &mut Conn,
    database: &str,
    table: &str,
) -> AppResult<Vec<ForeignKeyDefinition>> {
    let rows: Vec<Row> = conn
        .exec(
            "SELECT k.CONSTRAINT_NAME, k.COLUMN_NAME, k.REFERENCED_TABLE_SCHEMA, k.REFERENCED_TABLE_NAME, k.REFERENCED_COLUMN_NAME, r.UPDATE_RULE, r.DELETE_RULE \
             FROM information_schema.KEY_COLUMN_USAGE k \
             JOIN information_schema.REFERENTIAL_CONSTRAINTS r ON r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA AND r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.TABLE_NAME = k.TABLE_NAME \
             WHERE k.TABLE_SCHEMA = ? AND k.TABLE_NAME = ? AND k.REFERENCED_TABLE_NAME IS NOT NULL \
             ORDER BY k.CONSTRAINT_NAME, k.ORDINAL_POSITION",
            (database, table),
        )
        .await?;
    let mut keys: Vec<ForeignKeyDefinition> = Vec::new();
    for row in rows.into_iter().map(texts) {
        let name = required(row.first());
        let (column, referenced) = (required(row.get(1)), required(row.get(4)));
        if let Some(existing) = keys.iter_mut().find(|key| key.name == name) {
            existing.columns.push(column);
            existing.ref_columns.push(referenced);
            continue;
        }
        let ref_database = required(row.get(2));
        keys.push(ForeignKeyDefinition {
            original_name: Some(name.clone()),
            name,
            columns: vec![column],
            ref_database: (ref_database != database).then_some(ref_database),
            ref_table: required(row.get(3)),
            ref_columns: vec![referenced],
            on_update: ReferentialAction::from_rule(&required(row.get(5))),
            on_delete: ReferentialAction::from_rule(&required(row.get(6))),
        });
    }
    Ok(keys)
}

pub async fn read_table_definition(
    conn: &mut Conn,
    engine: Engine,
    database: &str,
    table: &str,
) -> AppResult<TableDefinition> {
    let options = read_options(conn, database, table).await?;
    Ok(TableDefinition {
        original_name: Some(table.to_owned()),
        name: table.to_owned(),
        columns: read_columns(conn, engine, database, table).await?,
        indexes: read_indexes(conn, database, table).await?,
        foreign_keys: read_foreign_keys(conn, database, table).await?,
        options,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mysql_defaults_are_values_unless_extra_says_expression() {
        let parse = |raw, extra, nullable, column_type| {
            parse_default(Engine::Mysql, raw, extra, nullable, column_type)
        };

        assert_eq!(parse(None, "", false, "int"), ColumnDefault::None);
        assert_eq!(parse(None, "", true, "int"), ColumnDefault::Null);
        assert_eq!(
            parse(Some("5"), "", false, "int"),
            ColumnDefault::Value("5".into())
        );
        assert_eq!(
            parse(Some(""), "", false, "varchar(5)"),
            ColumnDefault::Value(String::new())
        );
        assert_eq!(
            parse(
                Some("CURRENT_TIMESTAMP"),
                "DEFAULT_GENERATED on update CURRENT_TIMESTAMP",
                false,
                "timestamp"
            ),
            ColumnDefault::Expression("CURRENT_TIMESTAMP".into())
        );
        assert_eq!(
            parse(Some("uuid()"), "DEFAULT_GENERATED", false, "char(36)"),
            ColumnDefault::Expression("uuid()".into())
        );
        assert_eq!(
            parse(Some("b'0'"), "", false, "bit(1)"),
            ColumnDefault::Expression("b'0'".into())
        );
    }

    #[test]
    fn mariadb_defaults_are_read_from_their_quoting() {
        let parse = |raw| parse_default(Engine::Mariadb, raw, "", true, "varchar(10)");

        assert_eq!(parse(None), ColumnDefault::None);
        assert_eq!(parse(Some("NULL")), ColumnDefault::Null);
        assert_eq!(parse(Some("'abc'")), ColumnDefault::Value("abc".into()));
        assert_eq!(parse(Some("'it''s'")), ColumnDefault::Value("it's".into()));
        assert_eq!(parse(Some("''")), ColumnDefault::Value(String::new()));
        assert_eq!(parse(Some("42")), ColumnDefault::Value("42".into()));
        assert_eq!(parse(Some("-1.5")), ColumnDefault::Value("-1.5".into()));
        assert_eq!(
            parse(Some("current_timestamp()")),
            ColumnDefault::Expression("current_timestamp()".into())
        );
    }

    #[test]
    fn on_update_is_read_from_extra_on_both_engines() {
        assert_eq!(
            parse_on_update("DEFAULT_GENERATED on update CURRENT_TIMESTAMP(6)").as_deref(),
            Some("CURRENT_TIMESTAMP(6)")
        );
        assert_eq!(
            parse_on_update("on update current_timestamp()").as_deref(),
            Some("current_timestamp()")
        );
        assert_eq!(parse_on_update("auto_increment"), None);
    }
}
