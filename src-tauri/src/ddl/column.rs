//! SQL text for one column, index or foreign key.

use super::model::{
    ColumnDefault, ColumnDefinition, ForeignKeyDefinition, IndexColumn, IndexDefinition, IndexKind,
};
use crate::connection::model::Engine;
use crate::error::{AppError, AppResult};
use crate::sql::{is_balanced, is_plain_word, qualified, quote_identifier, quote_string};

pub const MAX_IDENTIFIER_CHARS: usize = 64;

pub fn check_identifier(name: &str, what: &str) -> AppResult<()> {
    if name.trim().is_empty() {
        return Err(AppError::Validation(format!("{what} name is required")));
    }
    if name.chars().count() > MAX_IDENTIFIER_CHARS {
        return Err(AppError::Validation(format!(
            "{what} name is longer than {MAX_IDENTIFIER_CHARS} characters"
        )));
    }
    if name.contains('\0') || name != name.trim_end() {
        return Err(AppError::Validation(format!(
            "{what} name \"{name}\" is not allowed"
        )));
    }
    Ok(())
}

fn check_balanced(text: &str, what: &str) -> AppResult<()> {
    if is_balanced(text) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "{what} has an unclosed quote or parenthesis"
        )))
    }
}

pub fn check_plain_word(text: &str, what: &str) -> AppResult<()> {
    if is_plain_word(text) {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "{what} \"{text}\" is not a valid name"
        )))
    }
}

/// `CURRENT_TIMESTAMP` (with optional precision) is written bare; any other expression needs parentheses.
fn default_expression_sql(expression: &str) -> String {
    let trimmed = expression.trim();
    let lower = trimmed.to_ascii_lowercase();
    let is_timestamp = ["current_timestamp", "now", "localtime", "localtimestamp"]
        .iter()
        .any(|name| {
            lower.strip_prefix(name).is_some_and(|rest| {
                rest.is_empty()
                    || (rest.starts_with('(')
                        && rest.ends_with(')')
                        && !rest[1..rest.len() - 1].contains('('))
            })
        });
    if is_timestamp || trimmed.starts_with('(') {
        trimmed.to_owned()
    } else {
        format!("({trimmed})")
    }
}

/// MariaDB's syntax for a generated column has no `NULL`/`NOT NULL`, MySQL's allows it.
pub fn column_sql(column: &ColumnDefinition, engine: Engine) -> AppResult<String> {
    check_identifier(&column.name, "Column")?;
    if column.column_type.trim().is_empty() {
        return Err(AppError::Validation(format!(
            "Column {} needs a type",
            column.name
        )));
    }
    check_balanced(&column.column_type, &format!("The type of {}", column.name))?;

    let mut parts = vec![
        quote_identifier(&column.name),
        column.column_type.trim().to_owned(),
    ];
    if let Some(collation) = &column.collation {
        check_plain_word(collation, "Collation")?;
        parts.push(format!("COLLATE {collation}"));
    }
    if let Some(generated) = &column.generated {
        check_balanced(
            &generated.expression,
            &format!("The expression of {}", column.name),
        )?;
        let storage = if generated.stored {
            "STORED"
        } else {
            "VIRTUAL"
        };
        parts.push(format!(
            "GENERATED ALWAYS AS ({}) {storage}",
            generated.expression.trim()
        ));
        if !column.nullable && engine == Engine::Mysql {
            parts.push("NOT NULL".into());
        }
    } else {
        parts.push(if column.nullable { "NULL" } else { "NOT NULL" }.into());
        match &column.default {
            ColumnDefault::None => {}
            ColumnDefault::Null => parts.push("DEFAULT NULL".into()),
            ColumnDefault::Value(value) => parts.push(format!("DEFAULT {}", quote_string(value))),
            ColumnDefault::Expression(expression) => {
                check_balanced(expression, &format!("The default of {}", column.name))?;
                parts.push(format!("DEFAULT {}", default_expression_sql(expression)));
            }
        }
        if column.auto_increment {
            parts.push("AUTO_INCREMENT".into());
        }
        if let Some(on_update) = &column.on_update {
            check_balanced(on_update, &format!("The ON UPDATE of {}", column.name))?;
            parts.push(format!("ON UPDATE {}", on_update.trim()));
        }
    }
    if !column.comment.is_empty() {
        parts.push(format!("COMMENT {}", quote_string(&column.comment)));
    }
    Ok(parts.join(" "))
}

fn index_column_sql(column: &IndexColumn) -> String {
    let length = column
        .length
        .map(|length| format!("({length})"))
        .unwrap_or_default();
    let order = if column.descending { " DESC" } else { "" };
    format!("{}{length}{order}", quote_identifier(&column.name))
}

pub fn index_sql(index: &IndexDefinition) -> AppResult<String> {
    if index.columns.is_empty() {
        return Err(AppError::Validation(format!(
            "Index {} has no columns",
            index.name
        )));
    }
    let columns: Vec<String> = index.columns.iter().map(index_column_sql).collect();
    let name = if index.name.is_empty() {
        String::new()
    } else {
        format!(" {}", quote_identifier(&index.name))
    };
    check_identifier_if_given(&index.name, "Index")?;
    Ok(match index.kind {
        IndexKind::Primary => format!("PRIMARY KEY ({})", columns.join(", ")),
        IndexKind::Unique => format!("UNIQUE KEY{name} ({})", columns.join(", ")),
        IndexKind::Index => format!("KEY{name} ({})", columns.join(", ")),
        IndexKind::Fulltext => format!("FULLTEXT KEY{name} ({})", columns.join(", ")),
        IndexKind::Spatial => format!("SPATIAL KEY{name} ({})", columns.join(", ")),
    })
}

fn check_identifier_if_given(name: &str, what: &str) -> AppResult<()> {
    if name.is_empty() {
        Ok(())
    } else {
        check_identifier(name, what)
    }
}

pub fn foreign_key_sql(table_database: &str, key: &ForeignKeyDefinition) -> AppResult<String> {
    if key.columns.is_empty() || key.columns.len() != key.ref_columns.len() {
        return Err(AppError::Validation(format!(
            "Foreign key {} needs the same number of local and referenced columns",
            key.name
        )));
    }
    check_identifier_if_given(&key.name, "Foreign key")?;
    check_identifier(&key.ref_table, "Referenced table")?;
    let list = |names: &[String]| {
        names
            .iter()
            .map(|name| quote_identifier(name))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let target = qualified(
        key.ref_database.as_deref().unwrap_or(table_database),
        &key.ref_table,
    );
    let constraint = if key.name.is_empty() {
        String::new()
    } else {
        format!("CONSTRAINT {} ", quote_identifier(&key.name))
    };
    Ok(format!(
        "{constraint}FOREIGN KEY ({}) REFERENCES {target} ({}) ON DELETE {} ON UPDATE {}",
        list(&key.columns),
        list(&key.ref_columns),
        key.on_delete.sql(),
        key.on_update.sql(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn col_sql(column: &ColumnDefinition) -> AppResult<String> {
        column_sql(column, Engine::Mysql)
    }
    use crate::ddl::model::samples::{column, index};
    use crate::ddl::model::{Generated, ReferentialAction};

    #[test]
    fn plain_column_is_name_type_and_nullability() {
        assert_eq!(col_sql(&column("id", "int")).unwrap(), "`id` int NOT NULL");
        let nullable = ColumnDefinition {
            nullable: true,
            ..column("note", "text")
        };
        assert_eq!(col_sql(&nullable).unwrap(), "`note` text NULL");
    }

    #[test]
    fn defaults_are_quoted_unless_they_are_expressions() {
        let with = |default| ColumnDefinition {
            default,
            ..column("c", "varchar(10)")
        };

        assert_eq!(
            col_sql(&with(ColumnDefault::Value("it's".into()))).unwrap(),
            "`c` varchar(10) NOT NULL DEFAULT 'it''s'"
        );
        assert_eq!(
            col_sql(&with(ColumnDefault::Null)).unwrap(),
            "`c` varchar(10) NOT NULL DEFAULT NULL"
        );
        assert_eq!(
            col_sql(&with(ColumnDefault::Expression(
                "CURRENT_TIMESTAMP(6)".into()
            )))
            .unwrap(),
            "`c` varchar(10) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)"
        );
        assert_eq!(
            col_sql(&with(ColumnDefault::Expression("uuid()".into()))).unwrap(),
            "`c` varchar(10) NOT NULL DEFAULT (uuid())"
        );
        assert_eq!(
            col_sql(&with(ColumnDefault::Expression("(1 + 1)".into()))).unwrap(),
            "`c` varchar(10) NOT NULL DEFAULT (1 + 1)"
        );
    }

    #[test]
    fn auto_increment_on_update_collation_and_comment_follow_in_order() {
        let definition = ColumnDefinition {
            auto_increment: true,
            on_update: Some("CURRENT_TIMESTAMP".into()),
            collation: Some("utf8mb4_bin".into()),
            comment: "who's this".into(),
            ..column("c", "varchar(10)")
        };

        assert_eq!(
            col_sql(&definition).unwrap(),
            "`c` varchar(10) COLLATE utf8mb4_bin NOT NULL AUTO_INCREMENT ON UPDATE CURRENT_TIMESTAMP COMMENT 'who''s this'"
        );
    }

    #[test]
    fn generated_columns_have_an_expression_and_no_default() {
        let definition = ColumnDefinition {
            generated: Some(Generated {
                expression: "price * 2".into(),
                stored: true,
            }),
            default: ColumnDefault::Value("ignored".into()),
            ..column("double", "int")
        };

        assert_eq!(
            col_sql(&definition).unwrap(),
            "`double` int GENERATED ALWAYS AS (price * 2) STORED NOT NULL"
        );
    }

    #[test]
    fn mariadb_generated_columns_leave_out_not_null() {
        let definition = ColumnDefinition {
            generated: Some(Generated {
                expression: "a + 1".into(),
                stored: false,
            }),
            ..column("b", "int")
        };

        assert_eq!(
            column_sql(&definition, Engine::Mariadb).unwrap(),
            "`b` int GENERATED ALWAYS AS (a + 1) VIRTUAL"
        );
        assert_eq!(
            column_sql(&definition, Engine::Mysql).unwrap(),
            "`b` int GENERATED ALWAYS AS (a + 1) VIRTUAL NOT NULL"
        );
    }

    #[test]
    fn rejects_unusable_names_types_and_unbalanced_text() {
        let bad = [
            column("", "int"),
            column("x", "  "),
            column("x", "varchar(10"),
            ColumnDefinition {
                collation: Some("x; DROP".into()),
                ..column("x", "text")
            },
            ColumnDefinition {
                default: ColumnDefault::Expression("a') --".into()),
                ..column("x", "text")
            },
            column(&"n".repeat(65), "int"),
        ];

        for definition in bad {
            assert!(
                matches!(col_sql(&definition), Err(AppError::Validation(_))),
                "{definition:?}"
            );
        }
    }

    #[test]
    fn identifiers_with_backticks_are_escaped_not_rejected() {
        assert_eq!(
            col_sql(&column("we`ird", "int")).unwrap(),
            "`we``ird` int NOT NULL"
        );
    }

    #[test]
    fn index_kinds_have_their_keywords_and_column_options() {
        let mut named = index(IndexKind::Unique, "uq", &["a"]);
        named.columns[0].length = Some(20);
        named.columns[0].descending = true;

        assert_eq!(
            index_sql(&index(IndexKind::Primary, "PRIMARY", &["id", "tenant"])).unwrap(),
            "PRIMARY KEY (`id`, `tenant`)"
        );
        assert_eq!(index_sql(&named).unwrap(), "UNIQUE KEY `uq` (`a`(20) DESC)");
        assert_eq!(
            index_sql(&index(IndexKind::Fulltext, "ft", &["body"])).unwrap(),
            "FULLTEXT KEY `ft` (`body`)"
        );
        assert_eq!(
            index_sql(&index(IndexKind::Index, "", &["a"])).unwrap(),
            "KEY (`a`)"
        );
        assert!(index_sql(&index(IndexKind::Index, "empty", &[])).is_err());
    }

    #[test]
    fn foreign_keys_reference_their_database_and_actions() {
        let key = ForeignKeyDefinition {
            original_name: None,
            name: "fk_customer".into(),
            columns: vec!["customer_id".into()],
            ref_database: None,
            ref_table: "customers".into(),
            ref_columns: vec!["id".into()],
            on_update: ReferentialAction::Cascade,
            on_delete: ReferentialAction::SetNull,
        };

        assert_eq!(
            foreign_key_sql("shop", &key).unwrap(),
            "CONSTRAINT `fk_customer` FOREIGN KEY (`customer_id`) REFERENCES `shop`.`customers` (`id`) ON DELETE SET NULL ON UPDATE CASCADE"
        );
        let other = ForeignKeyDefinition {
            ref_database: Some("crm".into()),
            name: String::new(),
            ..key.clone()
        };
        assert!(
            foreign_key_sql("shop", &other)
                .unwrap()
                .starts_with("FOREIGN KEY (`customer_id`) REFERENCES `crm`.`customers`")
        );
        let mismatched = ForeignKeyDefinition {
            ref_columns: vec![],
            ..key
        };
        assert!(foreign_key_sql("shop", &mismatched).is_err());
    }
}
