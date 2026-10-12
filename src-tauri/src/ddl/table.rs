//! `CREATE TABLE` and the `ALTER TABLE` statements that turn one table definition into another.

use std::collections::{HashMap, HashSet};

use super::column::{check_identifier, check_plain_word, column_sql, foreign_key_sql, index_sql};
use super::model::{
    ColumnDefinition, ForeignKeyDefinition, IndexDefinition, IndexKind, TableDefinition,
    TableOptions,
};
use crate::connection::model::Engine;
use crate::error::{AppError, AppResult};
use crate::sql::{qualified, quote_identifier, quote_string};

fn invalid(message: String) -> AppError {
    AppError::Validation(message)
}

/// The problems a definition would run into, found before anything is sent to the server.
pub fn validate(table: &TableDefinition) -> AppResult<()> {
    check_identifier(&table.name, "Table")?;
    if table.columns.is_empty() {
        return Err(invalid("A table needs at least one column".into()));
    }
    let mut seen = HashSet::new();
    for column in &table.columns {
        if !seen.insert(column.name.to_lowercase()) {
            return Err(invalid(format!("Column {} appears twice", column.name)));
        }
    }
    if table
        .columns
        .iter()
        .filter(|column| column.auto_increment)
        .count()
        > 1
    {
        return Err(invalid("Only one column can be AUTO_INCREMENT".into()));
    }
    let known = |name: &str| table.columns.iter().any(|column| column.name == name);
    for index in &table.indexes {
        if let Some(missing) = index.columns.iter().find(|column| !known(&column.name)) {
            return Err(invalid(format!(
                "Index {} uses {}, which is not a column",
                index.name, missing.name
            )));
        }
    }
    if table
        .indexes
        .iter()
        .filter(|index| index.kind == IndexKind::Primary)
        .count()
        > 1
    {
        return Err(invalid("A table has only one primary key".into()));
    }
    let mut names = HashSet::new();
    for name in table
        .indexes
        .iter()
        .map(|index| &index.name)
        .filter(|name| !name.is_empty())
    {
        if !names.insert(name.to_lowercase()) {
            return Err(invalid(format!("Index name {name} is used twice")));
        }
    }
    for key in &table.foreign_keys {
        if let Some(missing) = key.columns.iter().find(|column| !known(column)) {
            return Err(invalid(format!(
                "Foreign key {} uses {missing}, which is not a column",
                key.name
            )));
        }
    }
    check_options(&table.options)
}

fn check_options(options: &TableOptions) -> AppResult<()> {
    for (value, what) in [
        (&options.engine, "Engine"),
        (&options.row_format, "Row format"),
        (&options.collation, "Collation"),
    ] {
        if let Some(word) = value {
            check_plain_word(word, what)?;
        }
    }
    Ok(())
}

fn option_clauses(options: &TableOptions) -> Vec<String> {
    let mut clauses = Vec::new();
    if let Some(engine) = &options.engine {
        clauses.push(format!("ENGINE={engine}"));
    }
    if let Some(row_format) = &options.row_format {
        clauses.push(format!("ROW_FORMAT={}", row_format.to_ascii_uppercase()));
    }
    if let Some(next) = options.auto_increment {
        clauses.push(format!("AUTO_INCREMENT={next}"));
    }
    if let Some(collation) = &options.collation {
        clauses.push(format!("DEFAULT COLLATE={collation}"));
    }
    if !options.comment.is_empty() {
        clauses.push(format!("COMMENT={}", quote_string(&options.comment)));
    }
    clauses
}

pub fn create_table_statement(
    database: &str,
    table: &TableDefinition,
    engine: Engine,
) -> AppResult<String> {
    check_identifier(database, "Database")?;
    validate(table)?;
    let mut lines = Vec::new();
    for column in &table.columns {
        lines.push(column_sql(column, engine)?);
    }
    // The primary key goes first, as in `SHOW CREATE TABLE`.
    let (primary, others): (Vec<&IndexDefinition>, Vec<&IndexDefinition>) = table
        .indexes
        .iter()
        .partition(|index| index.kind == IndexKind::Primary);
    for index in primary.into_iter().chain(others) {
        lines.push(index_sql(index)?);
    }
    for key in &table.foreign_keys {
        lines.push(foreign_key_sql(database, key)?);
    }
    let body = lines
        .iter()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let options = option_clauses(&table.options);
    let tail = if options.is_empty() {
        String::new()
    } else {
        format!(" {}", options.join(" "))
    };
    Ok(format!(
        "CREATE TABLE {} (\n{body}\n){tail}",
        qualified(database, &table.name)
    ))
}

// ---- alter -----------------------------------------------------------------

/// Everything about a column except which database column it came from.
fn column_changed(original: &ColumnDefinition, edited: &ColumnDefinition) -> bool {
    ColumnDefinition {
        original_name: None,
        ..original.clone()
    } != ColumnDefinition {
        original_name: None,
        ..edited.clone()
    }
}

fn position_clause(edited: &TableDefinition, position: usize) -> String {
    if position == 0 {
        "FIRST".into()
    } else {
        format!(
            "AFTER {}",
            quote_identifier(&edited.columns[position - 1].name)
        )
    }
}

/// `ADD`, `CHANGE` and `MODIFY` clauses, plus the `DROP COLUMN`s. A column is only mentioned when it changed
/// or has to move; the walk keeps a picture of the table's column order as it will be after each clause.
fn column_clauses(
    original: &TableDefinition,
    edited: &TableDefinition,
    engine: Engine,
) -> AppResult<Vec<String>> {
    let originals: HashMap<&str, &ColumnDefinition> = original
        .columns
        .iter()
        .map(|column| (column.name.as_str(), column))
        .collect();
    let kept: HashSet<&str> = edited
        .columns
        .iter()
        .filter_map(|column| column.original_name.as_deref())
        .collect();

    let mut clauses: Vec<String> = original
        .columns
        .iter()
        .filter(|column| !kept.contains(column.name.as_str()))
        .map(|column| format!("DROP COLUMN {}", quote_identifier(&column.name)))
        .collect();

    let mut order: Vec<String> = original
        .columns
        .iter()
        .filter(|column| kept.contains(column.name.as_str()))
        .map(|column| column.name.clone())
        .collect();
    for (position, column) in edited.columns.iter().enumerate() {
        let definition = column_sql(column, engine)?;
        match column.original_name.as_deref() {
            None => {
                let at_end = position == order.len();
                let place = if at_end {
                    String::new()
                } else {
                    format!(" {}", position_clause(edited, position))
                };
                clauses.push(format!("ADD COLUMN {definition}{place}"));
                order.insert(position, format!("\u{0}new{position}"));
            }
            Some(key) => {
                let original_column = originals.get(key).ok_or_else(|| {
                    invalid(format!("Column {key} no longer exists in the table"))
                })?;
                let in_place = order.get(position).is_some_and(|current| current == key);
                if in_place && !column_changed(original_column, column) {
                    continue;
                }
                let place = if in_place {
                    String::new()
                } else {
                    format!(" {}", position_clause(edited, position))
                };
                let verb = if column.name == key {
                    "MODIFY COLUMN".to_owned()
                } else {
                    format!("CHANGE COLUMN {}", quote_identifier(key))
                };
                clauses.push(format!("{verb} {definition}{place}"));
                if !in_place && let Some(from) = order.iter().position(|current| current == key) {
                    let moved = order.remove(from);
                    order.insert(position, moved);
                }
            }
        }
    }
    Ok(clauses)
}

fn renames(original: &TableDefinition, edited: &TableDefinition) -> HashMap<String, String> {
    let originals: HashSet<&str> = original
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect();
    edited
        .columns
        .iter()
        .filter_map(|column| {
            column
                .original_name
                .as_ref()
                .filter(|old| originals.contains(old.as_str()))
                .map(|old| (old.clone(), column.name.clone()))
        })
        .collect()
}

fn renamed<'a>(map: &'a HashMap<String, String>, name: &'a str) -> &'a str {
    map.get(name).map_or(name, String::as_str)
}

fn index_changed(
    original: &IndexDefinition,
    edited: &IndexDefinition,
    map: &HashMap<String, String>,
) -> bool {
    let mut expected = original.clone();
    for column in &mut expected.columns {
        column.name = renamed(map, &column.name).to_owned();
    }
    IndexDefinition {
        original_name: None,
        ..expected
    } != IndexDefinition {
        original_name: None,
        ..edited.clone()
    }
}

fn foreign_key_changed(
    original: &ForeignKeyDefinition,
    edited: &ForeignKeyDefinition,
    map: &HashMap<String, String>,
) -> bool {
    let mut expected = original.clone();
    expected.columns = original
        .columns
        .iter()
        .map(|column| renamed(map, column).to_owned())
        .collect();
    ForeignKeyDefinition {
        original_name: None,
        ..expected
    } != ForeignKeyDefinition {
        original_name: None,
        ..edited.clone()
    }
}

fn index_clauses(
    original: &TableDefinition,
    edited: &TableDefinition,
    map: &HashMap<String, String>,
) -> AppResult<(Vec<String>, Vec<String>)> {
    let (mut drops, mut adds) = (Vec::new(), Vec::new());
    for index in &original.indexes {
        let counterpart = edited
            .indexes
            .iter()
            .find(|candidate| candidate.original_name.as_deref() == Some(index.name.as_str()));
        if counterpart.is_none_or(|edited_index| index_changed(index, edited_index, map)) {
            drops.push(if index.kind == IndexKind::Primary {
                "DROP PRIMARY KEY".to_owned()
            } else {
                format!("DROP INDEX {}", quote_identifier(&index.name))
            });
        }
    }
    for index in &edited.indexes {
        let before = index.original_name.as_deref().and_then(|name| {
            original
                .indexes
                .iter()
                .find(|candidate| candidate.name == name)
        });
        if before.is_none_or(|original_index| index_changed(original_index, index, map)) {
            adds.push(format!("ADD {}", index_sql(index)?));
        }
    }
    Ok((drops, adds))
}

fn foreign_key_clauses(
    database: &str,
    original: &TableDefinition,
    edited: &TableDefinition,
    map: &HashMap<String, String>,
) -> AppResult<(Vec<String>, Vec<String>)> {
    let (mut drops, mut adds) = (Vec::new(), Vec::new());
    for key in &original.foreign_keys {
        let counterpart = edited
            .foreign_keys
            .iter()
            .find(|candidate| candidate.original_name.as_deref() == Some(key.name.as_str()));
        if counterpart.is_none_or(|edited_key| foreign_key_changed(key, edited_key, map)) {
            drops.push(format!("DROP FOREIGN KEY {}", quote_identifier(&key.name)));
        }
    }
    for key in &edited.foreign_keys {
        let before = key.original_name.as_deref().and_then(|name| {
            original
                .foreign_keys
                .iter()
                .find(|candidate| candidate.name == name)
        });
        if before.is_none_or(|original_key| foreign_key_changed(original_key, key, map)) {
            adds.push(format!("ADD {}", foreign_key_sql(database, key)?));
        }
    }
    Ok((drops, adds))
}

/// `CONVERT TO CHARACTER SET` needs the character set, which a collation name starts with.
pub fn charset_of_collation(collation: &str) -> &str {
    collation.split('_').next().unwrap_or(collation)
}

/// The statements, in the order they must run, or none when nothing changed.
/// Foreign keys are dropped first and added last, each in a statement of their own, because
/// the server refuses to change a column or index that a key still depends on.
pub fn alter_table_statements(
    database: &str,
    original: &TableDefinition,
    edited: &TableDefinition,
    convert_columns: bool,
    engine: Engine,
) -> AppResult<Vec<String>> {
    check_identifier(database, "Database")?;
    validate(edited)?;
    let target = qualified(database, &original.name);
    let map = renames(original, edited);
    let (key_drops, key_adds) = foreign_key_clauses(database, original, edited, &map)?;
    let (index_drops, index_adds) = index_clauses(original, edited, &map)?;

    let mut main = index_drops;
    main.extend(column_clauses(original, edited, engine)?);
    main.extend(index_adds);
    let (before, after) = (&original.options, &edited.options);
    let mut options = TableOptions::default();
    if before.engine != after.engine {
        options.engine = after.engine.clone();
    }
    if before.row_format != after.row_format {
        options.row_format = after.row_format.clone();
    }
    if before.auto_increment != after.auto_increment {
        options.auto_increment = after.auto_increment;
    }
    if before.comment != after.comment {
        options.comment = after.comment.clone();
    }
    let collation_changed = before.collation != after.collation && after.collation.is_some();
    if collation_changed && !convert_columns {
        options.collation = after.collation.clone();
    }
    main.extend(option_clauses(&options));
    if before.comment != after.comment && after.comment.is_empty() {
        main.push("COMMENT=''".into());
    }

    let mut statements = Vec::new();
    if !key_drops.is_empty() {
        statements.push(format!("ALTER TABLE {target} {}", key_drops.join(", ")));
    }
    if !main.is_empty() {
        statements.push(format!("ALTER TABLE {target} {}", main.join(",\n  ")));
    }
    if collation_changed
        && convert_columns
        && let Some(collation) = &after.collation
    {
        statements.push(format!(
            "ALTER TABLE {target} CONVERT TO CHARACTER SET {} COLLATE {collation}",
            charset_of_collation(collation)
        ));
    }
    if !key_adds.is_empty() {
        statements.push(format!("ALTER TABLE {target} {}", key_adds.join(", ")));
    }
    if edited.name != original.name {
        statements.push(format!(
            "RENAME TABLE {target} TO {}",
            qualified(database, &edited.name)
        ));
    }
    Ok(statements)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_table_statement(database: &str, table: &TableDefinition) -> AppResult<String> {
        super::create_table_statement(database, table, Engine::Mysql)
    }

    fn alter_table_statements(
        database: &str,
        original: &TableDefinition,
        edited: &TableDefinition,
        convert_columns: bool,
    ) -> AppResult<Vec<String>> {
        super::alter_table_statements(database, original, edited, convert_columns, Engine::Mysql)
    }
    use crate::ddl::model::samples::{column, index, new_column, orders};
    use crate::ddl::model::{ColumnDefault, ReferentialAction};

    fn alter(original: &TableDefinition, edited: &TableDefinition) -> Vec<String> {
        alter_table_statements("shop", original, edited, false).unwrap()
    }

    #[test]
    fn create_table_lists_columns_primary_key_then_other_keys_and_options() {
        let mut table = orders();
        table.original_name = None;
        table.options.collation = Some("utf8mb4_bin".into());
        table.options.comment = "Customer orders".into();
        table.foreign_keys.push(ForeignKeyDefinition {
            original_name: None,
            name: "fk_customer".into(),
            columns: vec!["customer_id".into()],
            ref_database: None,
            ref_table: "customers".into(),
            ref_columns: vec!["id".into()],
            on_update: ReferentialAction::Restrict,
            on_delete: ReferentialAction::Cascade,
        });

        let sql = create_table_statement("shop", &table).unwrap();

        assert_eq!(
            sql,
            "CREATE TABLE `shop`.`orders` (\n  `id` bigint unsigned NOT NULL AUTO_INCREMENT,\n  `customer_id` int NOT NULL,\n  `total` decimal(10,2) NULL,\n  PRIMARY KEY (`id`),\n  KEY `idx_customer` (`customer_id`),\n  CONSTRAINT `fk_customer` FOREIGN KEY (`customer_id`) REFERENCES `shop`.`customers` (`id`) ON DELETE CASCADE ON UPDATE RESTRICT\n) ENGINE=InnoDB DEFAULT COLLATE=utf8mb4_bin COMMENT='Customer orders'"
        );
    }

    #[test]
    fn invalid_tables_are_rejected_with_a_reason() {
        let mut no_columns = orders();
        no_columns.columns.clear();
        let mut twice = orders();
        twice.columns.push(new_column("ID", "int"));
        let mut two_auto = orders();
        two_auto.columns[1].auto_increment = true;
        let mut bad_index = orders();
        bad_index
            .indexes
            .push(index(IndexKind::Index, "broken", &["nope"]));
        let mut two_primary = orders();
        two_primary
            .indexes
            .push(index(IndexKind::Primary, "PRIMARY", &["id"]));
        let mut same_name = orders();
        same_name
            .indexes
            .push(index(IndexKind::Index, "IDX_customer", &["total"]));
        let mut bad_engine = orders();
        bad_engine.options.engine = Some("InnoDB; DROP".into());

        for table in [
            no_columns,
            twice,
            two_auto,
            bad_index,
            two_primary,
            same_name,
            bad_engine,
        ] {
            assert!(
                matches!(validate(&table), Err(AppError::Validation(_))),
                "{table:?}"
            );
        }
        assert!(validate(&orders()).is_ok());
    }

    #[test]
    fn an_unchanged_table_needs_no_statements() {
        assert!(alter(&orders(), &orders()).is_empty());
    }

    #[test]
    fn a_new_column_at_the_end_is_appended_and_in_the_middle_is_positioned() {
        let original = orders();
        let mut at_end = orders();
        at_end.columns.push(new_column("note", "text"));
        let mut in_middle = orders();
        in_middle
            .columns
            .insert(1, new_column("sku", "varchar(20)"));
        let mut first = orders();
        first.columns.insert(0, new_column("uuid", "char(36)"));

        assert_eq!(
            alter(&original, &at_end),
            ["ALTER TABLE `shop`.`orders` ADD COLUMN `note` text NOT NULL"]
        );
        assert_eq!(
            alter(&original, &in_middle),
            ["ALTER TABLE `shop`.`orders` ADD COLUMN `sku` varchar(20) NOT NULL AFTER `id`"]
        );
        assert_eq!(
            alter(&original, &first),
            ["ALTER TABLE `shop`.`orders` ADD COLUMN `uuid` char(36) NOT NULL FIRST"]
        );
    }

    #[test]
    fn changing_a_column_modifies_only_that_column() {
        let mut edited = orders();
        edited.columns[2].column_type = "decimal(12,4)".into();
        edited.columns[2].default = ColumnDefault::Value("0".into());

        assert_eq!(
            alter(&orders(), &edited),
            ["ALTER TABLE `shop`.`orders` MODIFY COLUMN `total` decimal(12,4) NULL DEFAULT '0'"]
        );
    }

    #[test]
    fn renaming_a_column_uses_change_and_keeps_its_index() {
        let mut edited = orders();
        edited.columns[1].name = "client_id".into();
        edited.indexes[1].columns[0].name = "client_id".into();

        assert_eq!(
            alter(&orders(), &edited),
            ["ALTER TABLE `shop`.`orders` CHANGE COLUMN `customer_id` `client_id` int NOT NULL"]
        );
    }

    #[test]
    fn renaming_a_column_without_updating_its_index_is_rejected_by_the_index_check() {
        let mut edited = orders();
        edited.columns[1].name = "client_id".into();

        assert!(matches!(
            alter_table_statements("shop", &orders(), &edited, false),
            Err(AppError::Validation(_))
        ));
    }

    #[test]
    fn dropping_a_column_drops_it_and_an_index_that_no_longer_matches() {
        let mut edited = orders();
        edited.columns.remove(1);
        edited.indexes.remove(1);

        assert_eq!(
            alter(&orders(), &edited),
            ["ALTER TABLE `shop`.`orders` DROP INDEX `idx_customer`,\n  DROP COLUMN `customer_id`"]
        );
    }

    #[test]
    fn moving_a_column_repositions_only_what_has_to_move() {
        let mut edited = orders();
        let total = edited.columns.remove(2);
        edited.columns.insert(0, total);

        assert_eq!(
            alter(&orders(), &edited),
            ["ALTER TABLE `shop`.`orders` MODIFY COLUMN `total` decimal(10,2) NULL FIRST"]
        );
    }

    #[test]
    fn indexes_are_replaced_when_their_columns_or_kind_change_and_added_when_new() {
        let mut edited = orders();
        edited.indexes[1] = IndexDefinition {
            kind: IndexKind::Unique,
            ..edited.indexes[1].clone()
        };
        edited.indexes.push(index(IndexKind::Index, "", &["total"]));
        edited.indexes.last_mut().unwrap().original_name = None;

        assert_eq!(
            alter(&orders(), &edited),
            [
                "ALTER TABLE `shop`.`orders` DROP INDEX `idx_customer`,\n  ADD UNIQUE KEY `idx_customer` (`customer_id`),\n  ADD KEY (`total`)"
            ]
        );
    }

    #[test]
    fn foreign_keys_are_dropped_first_and_added_last_in_their_own_statements() {
        let key = ForeignKeyDefinition {
            original_name: Some("fk_a".into()),
            name: "fk_a".into(),
            columns: vec!["customer_id".into()],
            ref_database: None,
            ref_table: "customers".into(),
            ref_columns: vec!["id".into()],
            on_update: ReferentialAction::Restrict,
            on_delete: ReferentialAction::Restrict,
        };
        let mut original = orders();
        original.foreign_keys.push(key.clone());
        let mut edited = original.clone();
        edited.foreign_keys[0].on_delete = ReferentialAction::Cascade;
        edited.columns[1].column_type = "bigint".into();

        let statements = alter(&original, &edited);

        assert_eq!(statements.len(), 3);
        assert_eq!(
            statements[0],
            "ALTER TABLE `shop`.`orders` DROP FOREIGN KEY `fk_a`"
        );
        assert!(statements[1].contains("MODIFY COLUMN `customer_id` bigint NOT NULL"));
        assert!(
            statements[2]
                .starts_with("ALTER TABLE `shop`.`orders` ADD CONSTRAINT `fk_a` FOREIGN KEY")
                && statements[2].contains("ON DELETE CASCADE")
        );
    }

    #[test]
    fn an_unchanged_foreign_key_is_left_alone_even_when_its_column_is_renamed_consistently() {
        let key = ForeignKeyDefinition {
            original_name: Some("fk_a".into()),
            name: "fk_a".into(),
            columns: vec!["customer_id".into()],
            ref_database: None,
            ref_table: "customers".into(),
            ref_columns: vec!["id".into()],
            on_update: ReferentialAction::Restrict,
            on_delete: ReferentialAction::Restrict,
        };
        let mut original = orders();
        original.foreign_keys.push(key);
        let mut edited = original.clone();
        edited.columns[1].name = "client_id".into();
        edited.indexes[1].columns[0].name = "client_id".into();
        edited.foreign_keys[0].columns[0] = "client_id".into();

        assert_eq!(
            alter(&original, &edited),
            ["ALTER TABLE `shop`.`orders` CHANGE COLUMN `customer_id` `client_id` int NOT NULL"]
        );
    }

    #[test]
    fn options_are_only_written_when_they_changed() {
        let mut edited = orders();
        edited.options.engine = Some("MyISAM".into());
        edited.options.row_format = Some("dynamic".into());
        edited.options.comment = "hello".into();

        assert_eq!(
            alter(&orders(), &edited),
            [
                "ALTER TABLE `shop`.`orders` ENGINE=MyISAM,\n  ROW_FORMAT=DYNAMIC,\n  COMMENT='hello'"
            ]
        );
        let mut cleared = edited.clone();
        cleared.options.comment = String::new();
        assert!(alter(&edited, &cleared)[0].ends_with("COMMENT=''"));
    }

    #[test]
    fn changing_the_collation_either_sets_the_default_or_converts_the_columns() {
        let mut original = orders();
        original.options.collation = Some("latin1_swedish_ci".into());
        let mut edited = original.clone();
        edited.options.collation = Some("utf8mb4_unicode_ci".into());

        assert_eq!(
            alter_table_statements("shop", &original, &edited, false).unwrap(),
            ["ALTER TABLE `shop`.`orders` DEFAULT COLLATE=utf8mb4_unicode_ci"]
        );
        assert_eq!(
            alter_table_statements("shop", &original, &edited, true).unwrap(),
            [
                "ALTER TABLE `shop`.`orders` CONVERT TO CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci"
            ]
        );
        assert_eq!(charset_of_collation("binary"), "binary");
    }

    #[test]
    fn renaming_the_table_is_the_last_statement_and_later_alters_use_the_old_name() {
        let mut edited = orders();
        edited.name = "sales".into();
        edited.columns.push(new_column("note", "text"));

        let statements = alter(&orders(), &edited);

        assert_eq!(statements.len(), 2);
        assert!(statements[0].starts_with("ALTER TABLE `shop`.`orders` ADD COLUMN"));
        assert_eq!(
            statements[1],
            "RENAME TABLE `shop`.`orders` TO `shop`.`sales`"
        );
    }

    #[test]
    fn a_column_swap_of_a_dropped_and_new_column_does_not_confuse_the_walk() {
        let mut edited = orders();
        edited.columns.remove(2);
        edited.columns.push(new_column("total", "double"));
        edited.columns.insert(1, column_named_new("tax"));

        let statements = alter(&orders(), &edited);

        assert_eq!(
            statements,
            [
                "ALTER TABLE `shop`.`orders` DROP COLUMN `total`,\n  ADD COLUMN `tax` int NOT NULL AFTER `id`,\n  ADD COLUMN `total` double NOT NULL"
            ]
        );
    }

    fn column_named_new(name: &str) -> ColumnDefinition {
        new_column(name, "int")
    }

    #[test]
    fn a_column_that_vanished_from_the_original_is_reported() {
        let mut edited = orders();
        edited.columns[1] = column("ghost", "int");

        assert!(matches!(
            alter_table_statements("shop", &orders(), &edited, false),
            Err(AppError::Validation(_))
        ));
    }
}
