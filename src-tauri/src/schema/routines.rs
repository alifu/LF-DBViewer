//! Routine parameters, and replacing the definition of a view, routine, trigger or event.
//! MySQL cannot alter these in place, so a replacement is drop-then-create; if the new
//! definition fails, the old one is created again so nothing is lost.

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::{Deserialize, Serialize};

use super::{ObjectKind, object_definition, optional, required, texts};
use crate::error::{AppError, AppResult};
use crate::sql::{qualified, quote_identifier};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoutineParameter {
    /// 0 is a function's return value.
    pub position: u32,
    /// `IN`, `OUT` or `INOUT`; none for a function's return value.
    pub mode: Option<String>,
    pub name: Option<String>,
    pub data_type: String,
}

pub async fn routine_parameters(
    conn: &mut Conn,
    database: &str,
    name: &str,
    kind: ObjectKind,
) -> AppResult<Vec<RoutineParameter>> {
    if !matches!(kind, ObjectKind::Procedure | ObjectKind::Function) {
        return Err(AppError::Validation(
            "Only procedures and functions have parameters".into(),
        ));
    }
    let rows: Vec<Row> = conn
        .exec(
            "SELECT ORDINAL_POSITION, PARAMETER_MODE, PARAMETER_NAME, DTD_IDENTIFIER FROM information_schema.PARAMETERS \
             WHERE SPECIFIC_SCHEMA = ? AND SPECIFIC_NAME = ? AND ROUTINE_TYPE = ? ORDER BY ORDINAL_POSITION",
            (database, name, kind.keyword()),
        )
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| RoutineParameter {
            position: required(row.first()).parse().unwrap_or(0),
            mode: optional(row.get(1)),
            name: optional(row.get(2)),
            data_type: required(row.get(3)),
        })
        .collect())
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionRequest {
    pub database: String,
    pub kind: ObjectKind,
    /// The object being edited; `None` when creating a new one.
    #[serde(default)]
    pub existing_name: Option<String>,
    pub sql: String,
}

fn words(sql: &str) -> impl Iterator<Item = &str> {
    sql.split_whitespace()
}

/// Makes the statement `CREATE OR REPLACE …`, which replaces a view in one step.
pub fn with_or_replace(sql: &str) -> AppResult<String> {
    let trimmed = sql.trim_start();
    let mut rest = words(trimmed);
    if !rest
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("create"))
    {
        return Err(AppError::Validation(
            "The definition must start with CREATE".into(),
        ));
    }
    let after_create = trimmed["create".len()..].trim_start();
    let mut following = words(after_create);
    let already = following
        .next()
        .is_some_and(|word| word.eq_ignore_ascii_case("or"))
        && following
            .next()
            .is_some_and(|word| word.eq_ignore_ascii_case("replace"));
    if already {
        return Ok(trimmed.to_owned());
    }
    Ok(format!("CREATE OR REPLACE {after_create}"))
}

fn drop_statement(request: &DefinitionRequest, name: &str) -> String {
    format!(
        "DROP {} IF EXISTS {}",
        request.kind.keyword(),
        qualified(&request.database, name)
    )
}

/// What will run: the view replacement, or for the other kinds a drop (when editing) followed by the create.
pub fn definition_statements(request: &DefinitionRequest) -> AppResult<Vec<String>> {
    if !request
        .sql
        .trim_start()
        .get(..6)
        .is_some_and(|start| start.eq_ignore_ascii_case("create"))
    {
        return Err(AppError::Validation(
            "The definition must start with CREATE".into(),
        ));
    }
    match (request.kind, &request.existing_name) {
        (ObjectKind::Table, _) => Err(AppError::Validation(
            "Tables are edited in the table designer".into(),
        )),
        (ObjectKind::View, _) => Ok(vec![with_or_replace(&request.sql)?]),
        (_, None) => Ok(vec![request.sql.trim().to_owned()]),
        (_, Some(name)) => Ok(vec![
            drop_statement(request, name),
            request.sql.trim().to_owned(),
        ]),
    }
}

pub async fn apply_definition(
    conn: &mut Conn,
    request: &DefinitionRequest,
) -> AppResult<Vec<String>> {
    let statements = definition_statements(request)?;
    conn.query_drop(format!("USE {}", quote_identifier(&request.database)))
        .await?;
    let replacing_without_alter = matches!(
        request.kind,
        ObjectKind::Procedure | ObjectKind::Function | ObjectKind::Trigger | ObjectKind::Event
    );
    let Some(name) = request
        .existing_name
        .as_ref()
        .filter(|_| replacing_without_alter)
    else {
        for statement in &statements {
            conn.query_drop(statement.as_str()).await?;
        }
        return Ok(statements);
    };

    let previous = object_definition(conn, &request.database, name, request.kind).await?;
    conn.query_drop(statements[0].as_str()).await?;
    if let Err(error) = conn.query_drop(statements[1].as_str()).await {
        return Err(match conn.query_drop(previous.as_str()).await {
            Ok(()) => AppError::Database(format!(
                "{error}. The previous definition of {name} was restored."
            )),
            Err(restore) => AppError::Database(format!(
                "{error}. Restoring {name} also failed ({restore}); its previous definition was:\n{previous}"
            )),
        });
    }
    Ok(statements)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: ObjectKind, existing: Option<&str>, sql: &str) -> DefinitionRequest {
        DefinitionRequest {
            database: "shop".into(),
            kind,
            existing_name: existing.map(str::to_owned),
            sql: sql.into(),
        }
    }

    #[test]
    fn views_are_replaced_in_one_statement() {
        let statements = definition_statements(&request(
            ObjectKind::View,
            Some("v"),
            "CREATE ALGORITHM=UNDEFINED VIEW `v` AS select 1",
        ))
        .unwrap();

        assert_eq!(
            statements,
            ["CREATE OR REPLACE ALGORITHM=UNDEFINED VIEW `v` AS select 1"]
        );
    }

    #[test]
    fn a_view_that_already_says_or_replace_is_left_alone() {
        assert_eq!(
            with_or_replace("  create or replace view v as select 1").unwrap(),
            "create or replace view v as select 1"
        );
    }

    #[test]
    fn editing_a_routine_drops_the_old_one_first_and_a_new_one_is_just_created() {
        let sql = "CREATE PROCEDURE p() SELECT 1";

        assert_eq!(
            definition_statements(&request(ObjectKind::Procedure, Some("p"), sql)).unwrap(),
            ["DROP PROCEDURE IF EXISTS `shop`.`p`", sql]
        );
        assert_eq!(
            definition_statements(&request(ObjectKind::Procedure, None, sql)).unwrap(),
            [sql]
        );
    }

    #[test]
    fn only_create_statements_are_accepted_and_tables_are_refused() {
        assert!(
            definition_statements(&request(ObjectKind::Function, None, "DROP TABLE x")).is_err()
        );
        assert!(definition_statements(&request(ObjectKind::Function, None, "")).is_err());
        assert!(
            definition_statements(&request(ObjectKind::Table, None, "CREATE TABLE t (a int)"))
                .is_err()
        );
        assert!(
            definition_statements(&request(
                ObjectKind::Event,
                None,
                "  create event e on schedule every 1 day do select 1"
            ))
            .is_ok()
        );
    }
}
