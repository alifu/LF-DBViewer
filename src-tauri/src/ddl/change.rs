//! Every structure change the interface can ask for, turned into the SQL that performs it.
//! The same function produces the preview and the statements that run.

use serde::Deserialize;

use super::column::{check_identifier, check_plain_word};
use super::model::TableDefinition;
use super::table::{alter_table_statements, create_table_statement};
use crate::connection::model::Engine;
use crate::error::{AppError, AppResult};
use crate::schema::ObjectKind;
use crate::sql::{qualified, quote_identifier};

const SYSTEM_DATABASES: [&str; 4] = ["mysql", "information_schema", "performance_schema", "sys"];

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum StructureChange {
    CreateDatabase {
        name: String,
        charset: Option<String>,
        collation: Option<String>,
    },
    AlterDatabase {
        name: String,
        charset: Option<String>,
        collation: Option<String>,
    },
    DropDatabase {
        name: String,
    },
    DropObject {
        database: String,
        object: ObjectKind,
        name: String,
    },
    TruncateTable {
        database: String,
        name: String,
    },
    CreateTable {
        database: String,
        table: Box<TableDefinition>,
    },
    AlterTable {
        database: String,
        original: Box<TableDefinition>,
        edited: Box<TableDefinition>,
        #[serde(default)]
        convert_columns: bool,
    },
}

fn check_not_system(name: &str) -> AppResult<()> {
    if SYSTEM_DATABASES.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(AppError::Validation(format!(
            "{name} is a system database and cannot be changed here"
        )));
    }
    Ok(())
}

fn database_options(charset: &Option<String>, collation: &Option<String>) -> AppResult<String> {
    let mut text = String::new();
    if let Some(charset) = charset {
        check_plain_word(charset, "Character set")?;
        text.push_str(&format!(" CHARACTER SET {charset}"));
    }
    if let Some(collation) = collation {
        check_plain_word(collation, "Collation")?;
        text.push_str(&format!(" COLLATE {collation}"));
    }
    Ok(text)
}

impl StructureChange {
    pub fn statements(&self, engine: Engine) -> AppResult<Vec<String>> {
        match self {
            Self::CreateDatabase {
                name,
                charset,
                collation,
            } => {
                check_identifier(name, "Database")?;
                Ok(vec![format!(
                    "CREATE DATABASE {}{}",
                    quote_identifier(name),
                    database_options(charset, collation)?
                )])
            }
            Self::AlterDatabase {
                name,
                charset,
                collation,
            } => {
                check_identifier(name, "Database")?;
                check_not_system(name)?;
                let options = database_options(charset, collation)?;
                if options.is_empty() {
                    return Err(AppError::Validation(
                        "Choose a character set or collation".into(),
                    ));
                }
                Ok(vec![format!(
                    "ALTER DATABASE {}{options}",
                    quote_identifier(name)
                )])
            }
            Self::DropDatabase { name } => {
                check_identifier(name, "Database")?;
                check_not_system(name)?;
                Ok(vec![format!("DROP DATABASE {}", quote_identifier(name))])
            }
            Self::DropObject {
                database,
                object,
                name,
            } => {
                check_identifier(database, "Database")?;
                check_identifier(name, "Object")?;
                check_not_system(database)?;
                Ok(vec![format!(
                    "DROP {} {}",
                    object.keyword(),
                    qualified(database, name)
                )])
            }
            Self::TruncateTable { database, name } => {
                check_identifier(database, "Database")?;
                check_identifier(name, "Table")?;
                check_not_system(database)?;
                Ok(vec![format!(
                    "TRUNCATE TABLE {}",
                    qualified(database, name)
                )])
            }
            Self::CreateTable { database, table } => {
                check_not_system(database)?;
                Ok(vec![create_table_statement(database, table, engine)?])
            }
            Self::AlterTable {
                database,
                original,
                edited,
                convert_columns,
            } => {
                check_not_system(database)?;
                alter_table_statements(database, original, edited, *convert_columns, engine)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddl::model::samples::{new_column, orders};

    fn statements(json: &str) -> AppResult<Vec<String>> {
        serde_json::from_str::<StructureChange>(json)
            .unwrap()
            .statements(Engine::Mysql)
    }

    #[test]
    fn databases_are_created_altered_and_dropped() {
        assert_eq!(
            statements(r#"{"kind":"createDatabase","name":"shop","charset":"utf8mb4","collation":"utf8mb4_unicode_ci"}"#).unwrap(),
            ["CREATE DATABASE `shop` CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"]
        );
        assert_eq!(
            statements(
                r#"{"kind":"createDatabase","name":"plain","charset":null,"collation":null}"#
            )
            .unwrap(),
            ["CREATE DATABASE `plain`"]
        );
        assert_eq!(
            statements(
                r#"{"kind":"alterDatabase","name":"shop","charset":null,"collation":"utf8mb4_bin"}"#
            )
            .unwrap(),
            ["ALTER DATABASE `shop` COLLATE utf8mb4_bin"]
        );
        assert_eq!(
            statements(r#"{"kind":"dropDatabase","name":"shop"}"#).unwrap(),
            ["DROP DATABASE `shop`"]
        );
    }

    #[test]
    fn system_databases_cannot_be_altered_dropped_or_filled() {
        for json in [
            r#"{"kind":"dropDatabase","name":"mysql"}"#,
            r#"{"kind":"dropDatabase","name":"Information_Schema"}"#,
            r#"{"kind":"alterDatabase","name":"sys","charset":"utf8mb4","collation":null}"#,
            r#"{"kind":"dropObject","database":"performance_schema","object":"table","name":"t"}"#,
            r#"{"kind":"truncateTable","database":"mysql","name":"user"}"#,
        ] {
            assert!(
                matches!(statements(json), Err(AppError::Validation(_))),
                "{json}"
            );
        }
    }

    #[test]
    fn alter_database_needs_something_to_change_and_options_cannot_inject_sql() {
        assert!(
            statements(r#"{"kind":"alterDatabase","name":"shop","charset":null,"collation":null}"#)
                .is_err()
        );
        assert!(statements(r#"{"kind":"createDatabase","name":"x","charset":"utf8; DROP DATABASE y","collation":null}"#).is_err());
        assert!(
            statements(r#"{"kind":"createDatabase","name":"  ","charset":null,"collation":null}"#)
                .is_err()
        );
    }

    #[test]
    fn objects_are_dropped_by_kind_and_tables_truncated() {
        assert_eq!(
            statements(
                r#"{"kind":"dropObject","database":"shop","object":"procedure","name":"p"}"#
            )
            .unwrap(),
            ["DROP PROCEDURE `shop`.`p`"]
        );
        assert_eq!(
            statements(
                r#"{"kind":"dropObject","database":"shop","object":"view","name":"we`ird"}"#
            )
            .unwrap(),
            ["DROP VIEW `shop`.`we``ird`"]
        );
        assert_eq!(
            statements(r#"{"kind":"truncateTable","database":"shop","name":"orders"}"#).unwrap(),
            ["TRUNCATE TABLE `shop`.`orders`"]
        );
    }

    #[test]
    fn table_changes_arrive_as_definitions_from_the_interface() {
        let original = orders();
        let mut edited = orders();
        edited.columns.push(new_column("note", "text"));
        let change = serde_json::json!({
            "kind": "alterTable", "database": "shop", "original": original, "edited": edited
        });

        let statements = serde_json::from_value::<StructureChange>(change)
            .unwrap()
            .statements(Engine::Mysql)
            .unwrap();

        assert_eq!(
            statements,
            ["ALTER TABLE `shop`.`orders` ADD COLUMN `note` text NOT NULL"]
        );
    }
}
