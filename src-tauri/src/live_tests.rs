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

// ---- query runs ----

mod run_tests {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use mysql_async::prelude::Queryable;

    use super::{connect, create_database};
    use crate::query::run::{QueryEvent, RunSummary, run_statements};
    use crate::query::session::SessionManager;

    type Events = Arc<Mutex<Vec<QueryEvent>>>;

    /// Runs for a very long time. (An interrupted `SELECT SLEEP()` returns 1 without an error, so it
    /// would not show the cancel path.)
    const HEAVY_QUERY: &str = "SELECT COUNT(*) FROM information_schema.COLUMNS a, information_schema.COLUMNS b, information_schema.COLUMNS c";

    fn collector() -> (Events, impl Fn(QueryEvent) + Send + Sync + 'static) {
        let events: Events = Arc::default();
        let sink = Arc::clone(&events);
        (events, move |event| sink.lock().unwrap().push(event))
    }

    fn statements(sql: &[&str]) -> Vec<String> {
        sql.iter().map(|text| (*text).to_owned()).collect()
    }

    fn row_counts(events: &Events) -> Vec<(usize, usize, u64, bool)> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|event| match event {
                QueryEvent::ResultSetFinished {
                    index,
                    set,
                    total_rows,
                    truncated,
                } => Some((*index, *set, *total_rows, *truncated)),
                _ => None,
            })
            .collect()
    }

    async fn run(
        manager: &crate::connection::manager::ConnectionManager,
        sessions: &SessionManager,
        connection_id: &str,
        database: Option<&str>,
        sql: &[&str],
        max_rows: u64,
    ) -> (RunSummary, Events) {
        let session_id = sessions.open(connection_id).await;
        let session = sessions.get(&session_id).await.unwrap();
        let active = manager.get(connection_id).await.unwrap();
        let (events, emit) = collector();
        let summary = run_statements(
            &session,
            active.open_session_conn(),
            database,
            &statements(sql),
            max_rows,
            &emit,
        )
        .await
        .unwrap();
        sessions.close(&session_id).await;
        (summary, events)
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn streams_each_result_set_including_every_one_a_call_returns() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = create_database(&mut conn).await;
        conn.query_drop(format!(
            "CREATE PROCEDURE `{database}`.`two_sets`() BEGIN SELECT id FROM items WHERE id <= 3; SELECT name FROM items WHERE id <= 5; END"
        ))
        .await
        .unwrap();
        let sessions = SessionManager::default();

        let (summary, events) = run(
            &manager,
            &sessions,
            &id,
            Some(&database),
            &[
                "SELECT id, name FROM items WHERE id <= 2",
                "CALL two_sets()",
                "UPDATE items SET name = name WHERE id <= 4",
            ],
            100,
        )
        .await;
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert_eq!(summary.statements_run, 3);
        assert!(!summary.failed && !summary.cancelled);
        assert_eq!(summary.current_database.as_deref(), Some(database.as_str()));
        assert_eq!(
            row_counts(&events),
            [(0, 0, 2, false), (1, 0, 3, false), (1, 1, 5, false)]
        );
        let events = events.lock().unwrap();
        let update = events.iter().find_map(|event| match event {
            QueryEvent::StatementFinished {
                index: 2,
                affected_rows,
                ..
            } => Some(*affected_rows),
            _ => None,
        });
        assert_eq!(update, Some(Some(4)));
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn stops_at_the_first_error_and_reports_it() {
        let (manager, id) = connect().await;
        let sessions = SessionManager::default();

        let (summary, events) = run(
            &manager,
            &sessions,
            &id,
            None,
            &[
                "SELECT 1",
                "SELECT * FROM lfdb_no_such_db.nothing",
                "SELECT 3",
            ],
            100,
        )
        .await;

        assert_eq!(summary.statements_run, 1);
        assert!(summary.failed && !summary.cancelled);
        let events = events.lock().unwrap();
        let failure = events.iter().find_map(|event| match event {
            QueryEvent::StatementFailed {
                index,
                code,
                message,
                ..
            } => Some((*index, *code, message.clone())),
            _ => None,
        });
        let (index, code, message) = failure.unwrap();
        assert_eq!((index, code), (1, Some(1049)));
        assert!(message.contains("lfdb_no_such_db"));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, QueryEvent::StatementStarted { index: 2 }))
        );
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_session_keeps_its_variables_and_transaction_between_runs() {
        let (manager, id) = connect().await;
        let sessions = SessionManager::default();
        let session_id = sessions.open(&id).await;
        let session = sessions.get(&session_id).await.unwrap();
        let active = manager.get(&id).await.unwrap();
        let (events, emit) = collector();

        run_statements(
            &session,
            active.open_session_conn(),
            None,
            &statements(&["SET @lfdb = 41"]),
            10,
            &emit,
        )
        .await
        .unwrap();
        run_statements(
            &session,
            active.open_session_conn(),
            None,
            &statements(&["SELECT @lfdb + 1"]),
            10,
            &emit,
        )
        .await
        .unwrap();
        sessions.close(&session_id).await;

        let answer = events.lock().unwrap().iter().find_map(|event| match event {
            QueryEvent::Rows { rows, .. } => serde_json::to_value(&rows[0][0]).ok(),
            _ => None,
        });
        assert_eq!(answer, Some(serde_json::json!("42")));
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn rows_past_the_limit_are_counted_but_not_sent() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = create_database(&mut conn).await;
        let sessions = SessionManager::default();

        let (_, events) = run(
            &manager,
            &sessions,
            &id,
            Some(&database),
            &["SELECT * FROM items"],
            5,
        )
        .await;
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert_eq!(row_counts(&events), [(0, 0, 25, true)]);
        let sent: usize = events
            .lock()
            .unwrap()
            .iter()
            .map(|event| match event {
                QueryEvent::Rows { rows, .. } => rows.len(),
                _ => 0,
            })
            .sum();
        assert_eq!(sent, 5);
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_running_statement_can_be_cancelled_and_the_session_stays_usable() {
        let (manager, id) = connect().await;
        let sessions = Arc::new(SessionManager::default());
        let session_id = sessions.open(&id).await;
        let session = sessions.get(&session_id).await.unwrap();
        let active = manager.get(&id).await.unwrap();
        let (events, emit) = collector();

        let running = {
            let (session, active) = (Arc::clone(&session), Arc::clone(&active));
            tokio::spawn(async move {
                let connect = active.open_session_conn();
                run_statements(
                    &session,
                    connect,
                    None,
                    &statements(&[HEAVY_QUERY, "SELECT 2"]),
                    10,
                    &emit,
                )
                .await
            })
        };
        let mut thread_id = 0;
        for _ in 0..100 {
            thread_id = session.thread_id.load(Ordering::SeqCst);
            if thread_id != 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_ne!(thread_id, 0, "the statement never started");
        tokio::time::sleep(Duration::from_millis(200)).await;
        session.cancel_requested.store(true, Ordering::SeqCst);
        active.kill_query(thread_id).await.unwrap();

        let summary = tokio::time::timeout(Duration::from_secs(10), running)
            .await
            .expect("cancel did not stop the statement")
            .unwrap()
            .unwrap();
        let (_, emit_again) = collector();
        let after = run_statements(
            &session,
            active.open_session_conn(),
            None,
            &statements(&["SELECT 1"]),
            10,
            &emit_again,
        )
        .await
        .unwrap();
        sessions.close(&session_id).await;

        assert!(summary.cancelled && !summary.failed);
        assert_eq!(summary.statements_run, 0);
        assert!(events.lock().unwrap().iter().any(|event| matches!(
            event,
            QueryEvent::StatementFailed {
                index: 0,
                cancelled: true,
                ..
            }
        )));
        assert!(
            !events
                .lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, QueryEvent::StatementStarted { index: 1 }))
        );
        assert_eq!(after.statements_run, 1);
    }
}

#[tokio::test]
#[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
async fn lists_column_names_for_autocomplete() {
    let (manager, id) = connect().await;
    let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
    let database = create_database(&mut conn).await;

    let tables = crate::schema::database_column_names(&mut conn, &database)
        .await
        .unwrap();
    conn.query_drop(format!("DROP DATABASE `{database}`"))
        .await
        .unwrap();

    assert_eq!(
        tables["items"],
        ["id", "name", "price", "made_on", "payload", "notes"]
    );
    assert_eq!(tables["cheap"], ["id", "name"]);
    assert_eq!(tables["loose"], ["a", "b"]);
}
