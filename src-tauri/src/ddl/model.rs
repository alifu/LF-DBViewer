//! A table described as data: what the designer edits and what the SQL generator reads.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum ColumnDefault {
    /// No `DEFAULT` clause.
    None,
    Null,
    /// A literal value, written quoted.
    Value(String),
    /// An expression such as `CURRENT_TIMESTAMP` or `uuid()`, written as is.
    Expression(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Generated {
    pub expression: String,
    pub stored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDefinition {
    /// The name the column has in the database; `None` for a column added in the designer.
    #[serde(default)]
    pub original_name: Option<String>,
    pub name: String,
    /// The full type as declared: `varchar(255)`, `int unsigned`, `enum('a','b')`.
    pub column_type: String,
    #[serde(default)]
    pub nullable: bool,
    pub default: ColumnDefault,
    #[serde(default)]
    pub auto_increment: bool,
    /// For example `CURRENT_TIMESTAMP`.
    #[serde(default)]
    pub on_update: Option<String>,
    /// Only for text columns. Always written out when present, because `MODIFY` would
    /// otherwise reset the column to the table's default collation.
    #[serde(default)]
    pub collation: Option<String>,
    #[serde(default)]
    pub comment: String,
    #[serde(default)]
    pub generated: Option<Generated>,
    /// Left out of `SELECT *` and plain `INSERT`s (MySQL 8.0.23+, MariaDB 10.3+).
    #[serde(default)]
    pub invisible: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum IndexKind {
    Primary,
    Unique,
    Index,
    Fulltext,
    Spatial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexColumn {
    pub name: String,
    /// Prefix length for text and blob columns.
    #[serde(default)]
    pub length: Option<u32>,
    #[serde(default)]
    pub descending: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexDefinition {
    #[serde(default)]
    pub original_name: Option<String>,
    /// Empty lets the server choose one. The primary key is always `PRIMARY`.
    #[serde(default)]
    pub name: String,
    pub kind: IndexKind,
    pub columns: Vec<IndexColumn>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ReferentialAction {
    Restrict,
    Cascade,
    SetNull,
    NoAction,
    SetDefault,
}

impl ReferentialAction {
    pub fn sql(self) -> &'static str {
        match self {
            Self::Restrict => "RESTRICT",
            Self::Cascade => "CASCADE",
            Self::SetNull => "SET NULL",
            Self::NoAction => "NO ACTION",
            Self::SetDefault => "SET DEFAULT",
        }
    }

    pub fn from_rule(rule: &str) -> Self {
        match rule.to_ascii_uppercase().as_str() {
            "CASCADE" => Self::Cascade,
            "SET NULL" => Self::SetNull,
            "NO ACTION" => Self::NoAction,
            "SET DEFAULT" => Self::SetDefault,
            _ => Self::Restrict,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignKeyDefinition {
    #[serde(default)]
    pub original_name: Option<String>,
    #[serde(default)]
    pub name: String,
    pub columns: Vec<String>,
    /// `None` means the table's own database.
    #[serde(default)]
    pub ref_database: Option<String>,
    pub ref_table: String,
    pub ref_columns: Vec<String>,
    pub on_update: ReferentialAction,
    pub on_delete: ReferentialAction,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableOptions {
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub row_format: Option<String>,
    #[serde(default)]
    pub auto_increment: Option<u64>,
    #[serde(default)]
    pub collation: Option<String>,
    #[serde(default)]
    pub comment: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckConstraint {
    #[serde(default)]
    pub original_name: Option<String>,
    /// Empty lets the server name it.
    #[serde(default)]
    pub name: String,
    /// The condition, without the surrounding `CHECK ( )`.
    pub expression: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableDefinition {
    #[serde(default)]
    pub original_name: Option<String>,
    pub name: String,
    pub columns: Vec<ColumnDefinition>,
    #[serde(default)]
    pub indexes: Vec<IndexDefinition>,
    #[serde(default)]
    pub foreign_keys: Vec<ForeignKeyDefinition>,
    #[serde(default)]
    pub checks: Vec<CheckConstraint>,
    #[serde(default)]
    pub options: TableOptions,
}

#[cfg(test)]
pub(crate) mod samples {
    use super::*;

    pub fn column(name: &str, column_type: &str) -> ColumnDefinition {
        ColumnDefinition {
            original_name: Some(name.to_owned()),
            name: name.to_owned(),
            column_type: column_type.to_owned(),
            nullable: false,
            default: ColumnDefault::None,
            auto_increment: false,
            on_update: None,
            collation: None,
            comment: String::new(),
            generated: None,
            invisible: false,
        }
    }

    pub fn new_column(name: &str, column_type: &str) -> ColumnDefinition {
        ColumnDefinition {
            original_name: None,
            ..column(name, column_type)
        }
    }

    pub fn index(kind: IndexKind, name: &str, columns: &[&str]) -> IndexDefinition {
        IndexDefinition {
            original_name: Some(name.to_owned()),
            name: name.to_owned(),
            kind,
            columns: columns
                .iter()
                .map(|column| IndexColumn {
                    name: (*column).to_owned(),
                    length: None,
                    descending: false,
                })
                .collect(),
        }
    }

    /// `orders(id PK auto_increment, customer_id, total)` with a key on customer_id.
    pub fn orders() -> TableDefinition {
        TableDefinition {
            original_name: Some("orders".into()),
            name: "orders".into(),
            columns: vec![
                ColumnDefinition {
                    auto_increment: true,
                    ..column("id", "bigint unsigned")
                },
                column("customer_id", "int"),
                ColumnDefinition {
                    nullable: true,
                    ..column("total", "decimal(10,2)")
                },
            ],
            indexes: vec![
                index(IndexKind::Primary, "PRIMARY", &["id"]),
                index(IndexKind::Index, "idx_customer", &["customer_id"]),
            ],
            foreign_keys: vec![],
            checks: vec![],
            options: TableOptions {
                engine: Some("InnoDB".into()),
                ..TableOptions::default()
            },
        }
    }
}
