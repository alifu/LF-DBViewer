//! Tests that need a real server. Run with `make test-live` (see the Makefile).

use mysql_async::Conn;
use mysql_async::prelude::Queryable;

use crate::connection::manager::ConnectionManager;
use crate::connection::model::{ConnectionProfile, ConnectionSecrets, sample_profile};
use crate::error::AppError;
use crate::query::browse::{Filter, FilterOperator, PageRequest, SortKey, browse_table};
use crate::query::cell::{Cell, SpecialKind};
use crate::query::edit::{ColumnValue, RowChange, apply_changes};
use crate::schema::{ObjectKind, list_databases, list_objects, object_definition};

const ROW_COUNT: usize = 25;

async fn connect() -> (ConnectionManager, String) {
    let env = |name: &str| std::env::var(name).ok();
    let profile = ConnectionProfile {
        host: env("LFDB_TEST_HOST").unwrap_or_else(|| "127.0.0.1".into()),
        port: env("LFDB_TEST_PORT")
            .and_then(|port| port.parse().ok())
            .unwrap_or(3306),
        user: env("LFDB_TEST_USER").unwrap_or_else(|| "root".into()),
        ..sample_profile()
    };
    let secrets = ConnectionSecrets {
        password: env("LFDB_TEST_PASSWORD"),
        ssh_secret: None,
    };
    let manager = ConnectionManager::default();
    let id = manager.open(profile, secrets, false).await.unwrap().id;
    (manager, id)
}

/// A fresh database per test, so tests can run side by side and leave nothing behind.
async fn create_database(conn: &mut Conn) -> String {
    let database = format!("lfdb_test_{}", uuid::Uuid::new_v4().simple());
    conn.query_drop(format!("CREATE DATABASE `{database}`"))
        .await
        .unwrap();
    conn.query_drop(format!(
        "CREATE TABLE `{database}`.`items` (
            id BIGINT UNSIGNED PRIMARY KEY AUTO_INCREMENT,
            name VARCHAR(50) NOT NULL,
            price DECIMAL(30,10) NULL,
            made_on DATE NULL,
            payload VARBINARY(16) NULL,
            notes TEXT NULL
        )"
    ))
    .await
    .unwrap();
    for index in 1..=ROW_COUNT {
        conn.exec_drop(
            format!("INSERT INTO `{database}`.`items` (name, price, made_on, payload) VALUES (?, ?, ?, ?)"),
            (format!("item {index:02}"), "12345678901234567890.0123456789", "2026-01-31", vec![0xca_u8, 0xfe]),
        )
        .await
        .unwrap();
    }
    conn.query_drop(format!(
        "CREATE TABLE `{database}`.`loose` (a INT NULL, b VARCHAR(10) NULL)"
    ))
    .await
    .unwrap();
    conn.query_drop(format!(
        "INSERT INTO `{database}`.`loose` VALUES (1, 'x'), (NULL, 'y')"
    ))
    .await
    .unwrap();
    conn.query_drop(format!(
        "CREATE VIEW `{database}`.`cheap` AS SELECT id, name FROM `{database}`.`items`"
    ))
    .await
    .unwrap();
    conn.query_drop(format!(
        "CREATE PROCEDURE `{database}`.`count_items`() SELECT COUNT(*) FROM items"
    ))
    .await
    .unwrap();
    database
}

fn value(column: &str, value: Option<&str>) -> ColumnValue {
    ColumnValue {
        column: column.into(),
        value: value.map(str::to_owned),
    }
}

fn text(cell: &Cell) -> &str {
    match cell {
        Cell::Text(text) => text,
        other => panic!("expected text, got {other:?}"),
    }
}

#[tokio::test]
#[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
async fn lists_databases_objects_and_definitions() {
    let (manager, id) = connect().await;
    let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
    let database = create_database(&mut conn).await;

    let databases = list_databases(&mut conn).await.unwrap();
    let objects = list_objects(&mut conn, &database).await.unwrap();
    let table_sql = object_definition(&mut conn, &database, "items", ObjectKind::Table)
        .await
        .unwrap();
    let procedure_sql =
        object_definition(&mut conn, &database, "count_items", ObjectKind::Procedure)
            .await
            .unwrap();
    conn.query_drop(format!("DROP DATABASE `{database}`"))
        .await
        .unwrap();

    assert!(
        databases
            .iter()
            .any(|found| found.name == database && found.collation.is_some())
    );
    let tables: Vec<&str> = objects
        .tables
        .iter()
        .map(|table| table.name.as_str())
        .collect();
    assert_eq!(tables, ["items", "loose"]);
    assert_eq!(objects.tables[0].engine.as_deref(), Some("InnoDB"));
    assert_eq!(objects.views, ["cheap"]);
    assert_eq!(objects.procedures, ["count_items"]);
    assert!(table_sql.starts_with("CREATE TABLE `items`"));
    assert!(procedure_sql.contains("PROCEDURE `count_items`"));
}

#[tokio::test]
#[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
async fn pages_through_a_table_by_key_and_converts_values() {
    let (manager, id) = connect().await;
    let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
    let database = create_database(&mut conn).await;

    let first = browse_table(
        &mut conn,
        &database,
        "items",
        &PageRequest {
            limit: 10,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let cursor = vec![text(&first.rows[9][0]).to_owned()];
    let second = browse_table(
        &mut conn,
        &database,
        "items",
        &PageRequest {
            limit: 10,
            after: Some(cursor),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let last_cursor = vec![text(&second.rows[9][0]).to_owned()];
    let third = browse_table(
        &mut conn,
        &database,
        "items",
        &PageRequest {
            limit: 10,
            after: Some(last_cursor),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let filtered = browse_table(
        &mut conn,
        &database,
        "items",
        &PageRequest {
            limit: 10,
            sort: vec![SortKey {
                column: "name".into(),
                descending: true,
            }],
            filters: vec![Filter {
                column: "name".into(),
                operator: FilterOperator::Contains,
                value: Some("item 0".into()),
            }],
            ..Default::default()
        },
    )
    .await
    .unwrap();
    conn.query_drop(format!("DROP DATABASE `{database}`"))
        .await
        .unwrap();

    assert_eq!(first.primary_key, ["id"]);
    assert!(first.keyset && first.has_more);
    assert_eq!(text(&first.rows[0][1]), "item 01");
    assert_eq!(text(&second.rows[0][1]), "item 11");
    assert_eq!(third.rows.len(), ROW_COUNT - 20);
    assert!(!third.has_more);
    assert_eq!(text(&first.rows[0][2]), "12345678901234567890.0123456789");
    assert_eq!(text(&first.rows[0][3]), "2026-01-31");
    assert_eq!(
        first.rows[0][4],
        Cell::Special {
            kind: SpecialKind::Binary,
            preview: "cafe".into(),
            size: 2
        }
    );
    assert_eq!(first.rows[0][5], Cell::Null);
    assert_eq!(filtered.rows.len(), 9);
    assert_eq!(text(&filtered.rows[0][1]), "item 09");
    assert!(!filtered.keyset);
}

#[tokio::test]
#[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
async fn applies_edits_atomically() {
    let (manager, id) = connect().await;
    let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
    let database = create_database(&mut conn).await;

    let applied = apply_changes(
        &mut conn,
        &database,
        "items",
        &[
            RowChange::Update {
                key: vec![value("id", Some("1"))],
                set: vec![value("name", Some("renamed")), value("price", None)],
            },
            // Setting a value to what it already is must still count as finding the row.
            RowChange::Update {
                key: vec![value("id", Some("2"))],
                set: vec![value("name", Some("item 02"))],
            },
            RowChange::Delete {
                key: vec![value("id", Some("3"))],
            },
            RowChange::Insert {
                values: vec![value("name", Some("new"))],
            },
        ],
    )
    .await
    .unwrap();
    let keyless = apply_changes(
        &mut conn,
        &database,
        "loose",
        &[RowChange::Delete {
            key: vec![value("a", None), value("b", Some("y"))],
        }],
    )
    .await
    .unwrap();
    let rolled_back = apply_changes(
        &mut conn,
        &database,
        "items",
        &[
            RowChange::Update {
                key: vec![value("id", Some("4"))],
                set: vec![value("name", Some("must not stick"))],
            },
            RowChange::Delete {
                key: vec![value("id", Some("999999"))],
            },
        ],
    )
    .await;

    let names: Vec<String> = conn
        .query(format!(
            "SELECT name FROM `{database}`.`items` WHERE id IN (1, 4) ORDER BY id"
        ))
        .await
        .unwrap();
    let count: Option<u64> = conn
        .query_first(format!("SELECT COUNT(*) FROM `{database}`.`items`"))
        .await
        .unwrap();
    let loose: Option<u64> = conn
        .query_first(format!("SELECT COUNT(*) FROM `{database}`.`loose`"))
        .await
        .unwrap();
    conn.query_drop(format!("DROP DATABASE `{database}`"))
        .await
        .unwrap();

    assert_eq!(applied.affected_rows, 4);
    assert_eq!(applied.statements.len(), 4);
    assert_eq!(keyless.affected_rows, 1);
    assert!(matches!(rolled_back, Err(AppError::Validation(_))));
    assert_eq!(names, ["renamed", "item 04"]);
    assert_eq!(count, Some(ROW_COUNT as u64));
    assert_eq!(loose, Some(1));
}
