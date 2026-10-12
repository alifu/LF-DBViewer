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
        assert_eq!(index, 1);
        // MySQL reports an unknown database (1049), MariaDB an unknown table (1146).
        assert!(matches!(code, Some(1049 | 1146)), "{code:?}");
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

// ---- structure changes ----

mod structure_tests {
    use mysql_async::Conn;
    use mysql_async::prelude::Queryable;

    use super::connect;
    use crate::connection::model::Engine;
    use crate::ddl::apply::run_statements;
    use crate::ddl::change::StructureChange;
    use crate::ddl::model::{
        ColumnDefault, ColumnDefinition, ForeignKeyDefinition, Generated, IndexColumn,
        IndexDefinition, IndexKind, ReferentialAction, TableDefinition, TableOptions,
    };
    use crate::schema::definition::{read_table_definition, server_options};
    use crate::schema::routines::{DefinitionRequest, apply_definition, routine_parameters};
    use crate::schema::{ObjectKind, object_definition};

    async fn engine_of(conn: &mut Conn) -> Engine {
        let version: Option<String> = conn.query_first("SELECT VERSION()").await.unwrap();
        crate::connection::model::ConnectionProfile::engine_for_version(&version.unwrap())
    }

    async fn scratch_database(conn: &mut Conn) -> String {
        let name = format!("lfdb_ddl_{}", uuid::Uuid::new_v4().simple());
        conn.query_drop(format!("CREATE DATABASE `{name}`"))
            .await
            .unwrap();
        name
    }

    async fn apply(conn: &mut Conn, change: &StructureChange) -> Vec<String> {
        let engine = engine_of(conn).await;
        let statements = change.statements(engine).unwrap();
        run_statements(conn, &statements)
            .await
            .unwrap_or_else(|error| panic!("{error}\n{}", statements.join(";\n")));
        statements
    }

    fn column(name: &str, column_type: &str) -> ColumnDefinition {
        ColumnDefinition {
            original_name: None,
            name: name.into(),
            column_type: column_type.into(),
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

    fn key(name: &str, kind: IndexKind, columns: &[&str]) -> IndexDefinition {
        IndexDefinition {
            original_name: None,
            name: name.into(),
            kind,
            columns: columns
                .iter()
                .map(|column| IndexColumn {
                    name: (*column).into(),
                    length: None,
                    descending: false,
                })
                .collect(),
        }
    }

    fn customers() -> TableDefinition {
        TableDefinition {
            original_name: None,
            name: "customers".into(),
            columns: vec![
                ColumnDefinition {
                    auto_increment: true,
                    ..column("id", "int unsigned")
                },
                column("name", "varchar(80)"),
            ],
            indexes: vec![key("PRIMARY", IndexKind::Primary, &["id"])],
            foreign_keys: vec![],
            checks: vec![],
            options: TableOptions {
                engine: Some("InnoDB".into()),
                ..TableOptions::default()
            },
        }
    }

    /// A table using most column features, with a foreign key to `customers`.
    fn orders() -> TableDefinition {
        TableDefinition {
            original_name: None,
            name: "orders".into(),
            columns: vec![
                ColumnDefinition {
                    auto_increment: true,
                    ..column("id", "bigint unsigned")
                },
                column("customer_id", "int unsigned"),
                ColumnDefinition {
                    default: ColumnDefault::Value("pending".into()),
                    comment: "state".into(),
                    ..column("status", "varchar(20)")
                },
                ColumnDefinition {
                    nullable: true,
                    default: ColumnDefault::Null,
                    ..column("note", "text")
                },
                ColumnDefinition {
                    default: ColumnDefault::Value("9.50".into()),
                    ..column("total", "decimal(10,2)")
                },
                ColumnDefinition {
                    default: ColumnDefault::Expression("CURRENT_TIMESTAMP".into()),
                    ..column("created_at", "timestamp")
                },
                ColumnDefinition {
                    default: ColumnDefault::Expression("CURRENT_TIMESTAMP".into()),
                    on_update: Some("CURRENT_TIMESTAMP".into()),
                    ..column("updated_at", "timestamp")
                },
                ColumnDefinition {
                    generated: Some(Generated {
                        expression: "total * 2".into(),
                        stored: true,
                    }),
                    ..column("double_total", "decimal(12,2)")
                },
                column("kind", "enum('web','shop')"),
            ],
            indexes: vec![
                key("PRIMARY", IndexKind::Primary, &["id"]),
                key(
                    "idx_customer",
                    IndexKind::Index,
                    &["customer_id", "created_at"],
                ),
                key("uq_status_kind", IndexKind::Unique, &["status", "kind"]),
            ],
            foreign_keys: vec![ForeignKeyDefinition {
                original_name: None,
                name: "fk_customer".into(),
                columns: vec!["customer_id".into()],
                ref_database: None,
                ref_table: "customers".into(),
                ref_columns: vec!["id".into()],
                on_update: ReferentialAction::Cascade,
                on_delete: ReferentialAction::Restrict,
            }],
            checks: vec![],
            options: TableOptions {
                engine: Some("InnoDB".into()),
                comment: "Orders".into(),
                ..TableOptions::default()
            },
        }
    }

    async fn read(conn: &mut Conn, engine: Engine, database: &str, table: &str) -> TableDefinition {
        read_table_definition(conn, engine, database, table)
            .await
            .unwrap()
    }

    /// What a definition looks like once created, minus what the server fills in or rewrites:
    /// collations, row format, the auto-increment counter and the spelling of generated expressions.
    fn shape(table: &TableDefinition) -> TableDefinition {
        let mut shaped = table.clone();
        shaped.original_name = None;
        for column in &mut shaped.columns {
            column.original_name = None;
            column.collation = None;
            // The server rewrites the expression text, and reports a nullable column without a default as DEFAULT NULL.
            if let Some(generated) = &mut column.generated {
                generated.expression.clear();
                // MariaDB has no NOT NULL for generated columns, so only the generation itself is compared.
                column.nullable = true;
                column.default = ColumnDefault::None;
            }
            if column.nullable && column.default == ColumnDefault::None {
                column.default = ColumnDefault::Null;
            }
            // MariaDB spells integer widths and `current_timestamp()` differently from MySQL 8.
            column.column_type = strip_integer_width(&column.column_type);
            if let ColumnDefault::Expression(expression) = &mut column.default {
                *expression = expression.to_ascii_lowercase().replace("()", "");
            }
            column.on_update = column
                .on_update
                .take()
                .map(|text| text.to_ascii_lowercase().replace("()", ""));
        }
        for index in &mut shaped.indexes {
            index.original_name = None;
        }
        for key in &mut shaped.foreign_keys {
            key.original_name = None;
        }
        shaped.options.auto_increment = None;
        shaped.options.collation = None;
        shaped.options.row_format = None;
        shaped
    }

    fn strip_integer_width(column_type: &str) -> String {
        let lower = column_type.to_ascii_lowercase();
        let is_integer = ["tinyint", "smallint", "mediumint", "int", "bigint"]
            .iter()
            .any(|name| lower.starts_with(name));
        match (is_integer, lower.find('('), lower.find(')')) {
            (true, Some(open), Some(close)) => format!("{}{}", &lower[..open], &lower[close + 1..]),
            _ => lower,
        }
    }

    async fn create_orders(conn: &mut Conn, database: &str) -> Engine {
        let engine = engine_of(conn).await;
        apply(
            conn,
            &StructureChange::CreateTable {
                database: database.into(),
                table: Box::new(customers()),
            },
        )
        .await;
        apply(
            conn,
            &StructureChange::CreateTable {
                database: database.into(),
                table: Box::new(orders()),
            },
        )
        .await;
        engine
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_created_table_reads_back_as_it_was_defined_and_an_unedited_alter_is_empty() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = scratch_database(&mut conn).await;
        let engine = create_orders(&mut conn, &database).await;

        let loaded = read(&mut conn, engine, &database, "orders").await;
        let unchanged =
            crate::ddl::table::alter_table_statements(&database, &loaded, &loaded, false, engine)
                .unwrap();
        let mut expected = orders();
        expected.options.comment = "Orders".into();
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert!(unchanged.is_empty(), "{unchanged:?}");
        for (got, wanted) in shape(&loaded).columns.iter().zip(&shape(&expected).columns) {
            assert_eq!(got, wanted, "column {}", wanted.name);
        }
        assert_eq!(shape(&loaded).indexes, shape(&expected).indexes);
        assert_eq!(shape(&loaded).foreign_keys, shape(&expected).foreign_keys);
        assert_eq!(loaded.options.engine.as_deref(), Some("InnoDB"));
        assert_eq!(loaded.options.comment, "Orders");
        assert!(
            loaded
                .columns
                .iter()
                .find(|column| column.name == "double_total")
                .unwrap()
                .generated
                .is_some()
        );
        assert!(
            loaded
                .columns
                .iter()
                .find(|column| column.name == "status")
                .unwrap()
                .collation
                .is_some()
        );
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn alters_rename_reorder_add_and_drop_in_one_pass_and_the_result_matches_the_edit() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = scratch_database(&mut conn).await;
        let engine = create_orders(&mut conn, &database).await;
        let original = read(&mut conn, engine, &database, "orders").await;

        let mut edited = original.clone();
        edited.columns[1].name = "client_id".into(); // rename, referenced by an index and the foreign key
        edited.indexes[1].columns[0].name = "client_id".into();
        edited.foreign_keys[0].columns[0] = "client_id".into();
        edited.columns.insert(
            2,
            ColumnDefinition {
                original_name: None,
                ..column("source", "tinyint")
            },
        ); // new, right after the renamed column
        let total = edited.columns.remove(5); // `total` moves to the front
        edited.columns.insert(0, total);
        edited.columns.retain(|column| column.name != "note"); // dropped
        edited.columns.push(ColumnDefinition {
            original_name: None,
            nullable: true,
            ..column("ref_code", "int")
        });
        edited.options.comment = "Renamed things".into();
        let statements = apply(
            &mut conn,
            &StructureChange::AlterTable {
                database: database.clone(),
                original: Box::new(original),
                edited: Box::new(edited.clone()),
                convert_columns: false,
            },
        )
        .await;

        let after = read(&mut conn, engine, &database, "orders").await;
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        let names = |table: &TableDefinition| {
            table
                .columns
                .iter()
                .map(|column| column.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&after), names(&edited), "{statements:#?}");
        assert_eq!(
            shape(&after).columns,
            shape(&edited).columns,
            "{statements:#?}"
        );
        assert_eq!(shape(&after).indexes, shape(&edited).indexes);
        assert_eq!(shape(&after).foreign_keys, shape(&edited).foreign_keys);
        assert_eq!(after.options.comment, "Renamed things");
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_foreign_key_and_its_column_change_together_and_a_table_is_renamed_and_converted() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = scratch_database(&mut conn).await;
        let engine = create_orders(&mut conn, &database).await;
        let original = read(&mut conn, engine, &database, "orders").await;

        let mut edited = original.clone();
        // A foreign key that sets NULL needs a nullable column: the column and the key change together.
        edited.foreign_keys[0].on_delete = ReferentialAction::SetNull;
        edited.columns[1].nullable = true;
        edited
            .indexes
            .retain(|index| index.name != "uq_status_kind");
        edited.name = "sales".into();
        edited.options.collation = Some("utf8mb4_bin".into());
        apply(
            &mut conn,
            &StructureChange::AlterTable {
                database: database.clone(),
                original: Box::new(original),
                edited: Box::new(edited.clone()),
                convert_columns: true,
            },
        )
        .await;

        let after = read(&mut conn, engine, &database, "sales").await;
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert!(after.columns[1].nullable);
        assert_eq!(after.foreign_keys[0].on_delete, ReferentialAction::SetNull);
        assert!(
            after
                .indexes
                .iter()
                .all(|index| index.name != "uq_status_kind")
        );
        assert_eq!(after.options.collation.as_deref(), Some("utf8mb4_bin"));
        assert_eq!(after.columns[2].collation.as_deref(), Some("utf8mb4_bin"));
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn checks_and_invisible_columns_are_written_read_back_changed_and_a_table_can_be_copied()
    {
        use crate::ddl::model::CheckConstraint;
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = scratch_database(&mut conn).await;
        let engine = engine_of(&mut conn).await;
        let mut table = customers();
        table.columns.push(ColumnDefinition {
            invisible: true,
            nullable: true,
            default: ColumnDefault::Null,
            ..column("secret", "int")
        });
        table.checks = vec![CheckConstraint {
            original_name: None,
            name: "name_not_empty".into(),
            expression: "char_length(`name`) > 0".into(),
        }];
        apply(
            &mut conn,
            &StructureChange::CreateTable {
                database: database.clone(),
                table: Box::new(table),
            },
        )
        .await;
        conn.query_drop(format!(
            "INSERT INTO `{database}`.`customers` (name, secret) VALUES ('a', 7), ('b', 8)"
        ))
        .await
        .unwrap();

        let loaded = read(&mut conn, engine, &database, "customers").await;
        let refused = conn
            .query_drop(format!(
                "INSERT INTO `{database}`.`customers` (name) VALUES ('')"
            ))
            .await;
        let mut edited = loaded.clone();
        edited.checks.clear();
        edited.checks.push(CheckConstraint {
            original_name: None,
            name: "name_short".into(),
            expression: "char_length(`name`) < 20".into(),
        });
        apply(
            &mut conn,
            &StructureChange::AlterTable {
                database: database.clone(),
                original: Box::new(loaded.clone()),
                edited: Box::new(edited),
                convert_columns: false,
            },
        )
        .await;
        let changed = read(&mut conn, engine, &database, "customers").await;
        let allowed_now = conn
            .query_drop(format!(
                "INSERT INTO `{database}`.`customers` (name) VALUES ('')"
            ))
            .await;
        let columns: Vec<String> = loaded
            .columns
            .iter()
            .filter(|column| column.generated.is_none())
            .map(|column| column.name.clone())
            .collect();
        apply(
            &mut conn,
            &StructureChange::CopyTable {
                database: database.clone(),
                source: "customers".into(),
                target: "customers_copy".into(),
                with_data: true,
                columns,
            },
        )
        .await;
        let copied: Option<u64> = conn
            .query_first(format!(
                "SELECT COUNT(*) FROM `{database}`.`customers_copy`"
            ))
            .await
            .unwrap();
        let copied_secret: Option<i64> = conn
            .query_first(format!(
                "SELECT secret FROM `{database}`.`customers_copy` WHERE name = 'b'"
            ))
            .await
            .unwrap();
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert!(
            loaded
                .columns
                .iter()
                .find(|column| column.name == "secret")
                .unwrap()
                .invisible,
            "the invisible flag is read back"
        );
        assert!(
            !loaded
                .columns
                .iter()
                .find(|column| column.name == "name")
                .unwrap()
                .invisible
        );
        assert_eq!(
            loaded
                .checks
                .iter()
                .map(|check| check.name.as_str())
                .collect::<Vec<_>>(),
            ["name_not_empty"]
        );
        assert!(refused.is_err(), "the check rejects an empty name");
        assert_eq!(
            changed
                .checks
                .iter()
                .map(|check| check.name.as_str())
                .collect::<Vec<_>>(),
            ["name_short"]
        );
        assert!(allowed_now.is_ok(), "the old check is gone");
        assert_eq!((copied, copied_secret), (Some(3), Some(8)));
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn databases_and_objects_are_created_altered_truncated_and_dropped() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let name = format!("lfdb_ddl_{}", uuid::Uuid::new_v4().simple());
        let options = server_options(&mut conn).await.unwrap();

        apply(
            &mut conn,
            &StructureChange::CreateDatabase {
                name: name.clone(),
                charset: Some("latin1".into()),
                collation: None,
            },
        )
        .await;
        apply(
            &mut conn,
            &StructureChange::AlterDatabase {
                name: name.clone(),
                charset: None,
                collation: Some("utf8mb4_bin".into()),
            },
        )
        .await;
        apply(
            &mut conn,
            &StructureChange::CreateTable {
                database: name.clone(),
                table: Box::new(customers()),
            },
        )
        .await;
        conn.query_drop(format!(
            "INSERT INTO `{name}`.`customers` (name) VALUES ('a'), ('b')"
        ))
        .await
        .unwrap();
        apply(
            &mut conn,
            &StructureChange::TruncateTable {
                database: name.clone(),
                name: "customers".into(),
            },
        )
        .await;
        let rows: Option<u64> = conn
            .query_first(format!("SELECT COUNT(*) FROM `{name}`.`customers`"))
            .await
            .unwrap();
        let collation: Option<String> = conn
            .query_first(format!("SELECT DEFAULT_COLLATION_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = '{name}'"))
            .await
            .unwrap();
        apply(
            &mut conn,
            &StructureChange::DropObject {
                database: name.clone(),
                object: ObjectKind::Table,
                name: "customers".into(),
            },
        )
        .await;
        apply(
            &mut conn,
            &StructureChange::DropDatabase { name: name.clone() },
        )
        .await;
        let remaining: Option<String> = conn
            .query_first(format!(
                "SELECT SCHEMA_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = '{name}'"
            ))
            .await
            .unwrap();

        assert_eq!(rows, Some(0));
        assert_eq!(collation.as_deref(), Some("utf8mb4_bin"));
        assert_eq!(remaining, None);
        assert!(
            options
                .engines
                .iter()
                .any(|engine| engine.eq_ignore_ascii_case("InnoDB"))
        );
        assert!(
            options
                .collations
                .iter()
                .any(|collation| collation.charset == "utf8mb4" && collation.name == "utf8mb4_bin")
        );
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn replacing_a_routine_keeps_the_old_one_when_the_new_definition_is_broken() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = scratch_database(&mut conn).await;
        let request = |existing: Option<&str>, kind, sql: &str| DefinitionRequest {
            database: database.clone(),
            kind,
            existing_name: existing.map(str::to_owned),
            sql: sql.into(),
        };

        apply_definition(&mut conn, &request(None, ObjectKind::Procedure, "CREATE PROCEDURE greet(IN who VARCHAR(20), OUT answer VARCHAR(40)) BEGIN SET answer = CONCAT('hi ', who); END")).await.unwrap();
        let parameters = routine_parameters(&mut conn, &database, "greet", ObjectKind::Procedure)
            .await
            .unwrap();
        apply_definition(&mut conn, &request(Some("greet"), ObjectKind::Procedure, "CREATE PROCEDURE greet(IN who VARCHAR(20), OUT answer VARCHAR(40)) BEGIN SET answer = CONCAT('hello ', who); END")).await.unwrap();
        let replaced = object_definition(&mut conn, &database, "greet", ObjectKind::Procedure)
            .await
            .unwrap();
        let broken = apply_definition(
            &mut conn,
            &request(
                Some("greet"),
                ObjectKind::Procedure,
                "CREATE PROCEDURE greet(IN who VARCHAR(20)) BEGIN THIS IS NOT SQL; END",
            ),
        )
        .await;
        let restored = object_definition(&mut conn, &database, "greet", ObjectKind::Procedure)
            .await
            .unwrap();
        conn.query_drop(format!("CREATE VIEW `{database}`.`v` AS SELECT 1 AS one"))
            .await
            .unwrap();
        apply_definition(
            &mut conn,
            &request(
                Some("v"),
                ObjectKind::View,
                "CREATE VIEW `v` AS SELECT 2 AS two",
            ),
        )
        .await
        .unwrap();
        let view_column: Option<String> = conn.query_first(format!("SELECT COLUMN_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = '{database}' AND TABLE_NAME = 'v'")).await.unwrap();
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert_eq!(
            parameters
                .iter()
                .map(|parameter| (parameter.mode.clone(), parameter.name.clone()))
                .collect::<Vec<_>>(),
            [
                (Some("IN".into()), Some("who".into())),
                (Some("OUT".into()), Some("answer".into()))
            ]
        );
        assert!(replaced.contains("hello "));
        let message = broken.unwrap_err().to_string();
        assert!(message.contains("restored"), "{message}");
        assert!(
            restored.contains("hello "),
            "the previous definition was not restored: {restored}"
        );
        assert_eq!(view_column.as_deref(), Some("two"));
    }
}

// ---- export and import ----

mod transfer_tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use mysql_async::prelude::Queryable;
    use mysql_async::{Conn, Row};

    use super::connect;
    use crate::schema::texts;
    use crate::transfer::export::{ExportEvent, ExportRequest, export_database};
    use crate::transfer::import::{ImportEvent, ImportRequest, import_file};

    async fn run(conn: &mut Conn, sql: &str) {
        conn.query_drop(sql)
            .await
            .unwrap_or_else(|error| panic!("{error}\n{sql}"));
    }

    fn unique(prefix: &str) -> String {
        format!("{prefix}_{}", uuid::Uuid::new_v4().simple())
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{}_{name}", unique("lfdb")))
    }

    /// Everything the dump has to carry: awkward values, keys that point forwards, views of views, a routine,
    /// a trigger and an event.
    async fn seed(conn: &mut Conn, database: &str) {
        run(
            conn,
            &format!("CREATE DATABASE `{database}` DEFAULT CHARACTER SET utf8mb4"),
        )
        .await;
        run(conn, &format!("USE `{database}`")).await;
        run(conn, "SET SESSION sql_mode = ''").await;
        run(conn, "SET SESSION time_zone = '+09:00'").await;
        run(conn, "SET SESSION foreign_key_checks = 0").await;
        // `a_child` sorts before the table it references, so loading must tolerate the order.
        run(conn, "CREATE TABLE a_child (id INT PRIMARY KEY, parent_id INT NOT NULL, note VARCHAR(40), CONSTRAINT fk_parent FOREIGN KEY (parent_id) REFERENCES z_parent (id) ON DELETE CASCADE) ENGINE=InnoDB").await;
        run(conn, "CREATE TABLE z_parent (id INT PRIMARY KEY AUTO_INCREMENT, name VARCHAR(60)) ENGINE=InnoDB").await;
        run(
            conn,
            "CREATE TABLE kinds (
                id BIGINT UNSIGNED PRIMARY KEY AUTO_INCREMENT,
                tiny TINYINT, big BIGINT UNSIGNED, dec_col DECIMAL(30,10), flt FLOAT, dbl DOUBLE,
                txt TEXT, chr CHAR(5), bin VARBINARY(64), blb BLOB, bits BIT(10), flag ENUM('a','b','c'), tags SET('x','y','z'),
                d DATE, dt DATETIME(3), ts TIMESTAMP NULL, tm TIME(3), yr YEAR, js JSON,
                price DECIMAL(10,2), qty INT, total DECIMAL(12,2) AS (price * qty) STORED, label VARCHAR(80) AS (CONCAT('#', qty)) VIRTUAL
            ) ENGINE=InnoDB",
        )
        .await;
        run(
            conn,
            "INSERT INTO z_parent (name) VALUES ('first'), ('second'), (NULL)",
        )
        .await;
        run(
            conn,
            "INSERT INTO a_child VALUES (1, 1, 'ok'), (2, 2, NULL)",
        )
        .await;
        let rows: &[&str] = &[
            "(1, -128, 18446744073709551615, 12345678901234567890.0123456789, 0.1, 0.30000000000000004, 'plain', 'abc', 0x00ff10, 0xdeadbeef, b'1010101010', 'b', 'x,z', '2026-01-31', '2026-01-31 23:59:59.123', '2026-06-01 12:00:00', '-26:03:04.500', 2026, '{\"a\": [1, 2, {\"b\": null}]}', 9.99, 3)",
            "(2, 127, 0, -0.0000000001, 1e30, 1e300, 'it''s \"quoted\" \\\\ back\\nslash\ttab\rcr', '', '', '', b'0', 'a', '', '0000-00-00', '0000-00-00 00:00:00.000', NULL, '838:59:59', 0000, 'null', 0.00, 0)",
            "(3, NULL, NULL, NULL, NULL, NULL, '😀 é ñ 日本語 \u{1a}', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL, '1970-01-02 03:04:05', NULL, NULL, NULL, NULL, NULL)",
        ];
        for row in rows {
            run(conn, &format!("INSERT INTO kinds (id, tiny, big, dec_col, flt, dbl, txt, chr, bin, blb, bits, flag, tags, d, dt, ts, tm, yr, js, price, qty) VALUES {row}")).await;
        }
        // Enough rows, and long enough text, to need many insert statements.
        run(
            conn,
            "CREATE TABLE bulk (id INT PRIMARY KEY, body TEXT) ENGINE=InnoDB",
        )
        .await;
        let mut values = Vec::new();
        for id in 1..=600 {
            values.push(format!(
                "({id}, REPEAT(CONCAT('row ', {id}, ' ; \\' -- '), 8))"
            ));
        }
        run(
            conn,
            &format!("INSERT INTO bulk VALUES {}", values.join(",")),
        )
        .await;
        run(
            conn,
            "CREATE VIEW v_base AS SELECT id, name FROM z_parent WHERE name IS NOT NULL",
        )
        .await;
        run(
            conn,
            "CREATE VIEW v_top AS SELECT id, UPPER(name) AS shout FROM v_base",
        )
        .await;
        run(
            conn,
            "CREATE FUNCTION double_it(x INT) RETURNS INT DETERMINISTIC RETURN x * 2",
        )
        .await;
        run(conn, "CREATE PROCEDURE count_parents(OUT n INT) BEGIN SELECT COUNT(*) INTO n FROM z_parent; SELECT 'done' AS msg; END").await;
        run(conn, "CREATE TABLE audit (id INT AUTO_INCREMENT PRIMARY KEY, what VARCHAR(40)) ENGINE=InnoDB").await;
        run(conn, "CREATE TRIGGER z_parent_ai AFTER INSERT ON z_parent FOR EACH ROW INSERT INTO audit (what) VALUES (CONCAT('added ', NEW.id))").await;
        // This insert fires the trigger once: one audit row that a restore must not duplicate.
        run(conn, "INSERT INTO z_parent (name) VALUES ('late')").await;
        run(
            conn,
            "CREATE EVENT yearly ON SCHEDULE EVERY 1 YEAR STARTS '2099-01-01 00:00:00' DO SELECT 1",
        )
        .await;
        run(conn, "SET SESSION time_zone = SYSTEM").await;
        run(conn, "SET SESSION foreign_key_checks = 1").await;
    }

    /// What must be identical after a restore: row counts, a checksum of every table, and the definitions.
    async fn fingerprint(conn: &mut Conn, database: &str) -> Vec<String> {
        // `SHOW CREATE VIEW` leaves out the database name only while that database is selected.
        run(conn, &format!("USE `{database}`")).await;
        let mut parts = Vec::new();
        let tables: Vec<Row> = conn.exec("SELECT TABLE_NAME FROM information_schema.TABLES WHERE TABLE_SCHEMA = ? AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME", (database,)).await.unwrap();
        for table in tables.into_iter().map(|row| texts(row)[0].clone().unwrap()) {
            let checksum: Vec<Row> = conn
                .query(format!("CHECKSUM TABLE `{database}`.`{table}`"))
                .await
                .unwrap();
            let count: Option<u64> = conn
                .query_first(format!("SELECT COUNT(*) FROM `{database}`.`{table}`"))
                .await
                .unwrap();
            let create: Row = conn
                .query_first(format!("SHOW CREATE TABLE `{database}`.`{table}`"))
                .await
                .unwrap()
                .unwrap();
            parts.push(format!(
                "table {table}: rows={count:?} checksum={:?} create={:?}",
                texts(checksum.into_iter().next().unwrap())[1],
                texts(create)[1]
            ));
        }
        for (kind, name) in [
            ("VIEW", "v_base"),
            ("VIEW", "v_top"),
            ("FUNCTION", "double_it"),
            ("PROCEDURE", "count_parents"),
            ("TRIGGER", "z_parent_ai"),
            ("EVENT", "yearly"),
        ] {
            let row: Row = conn
                .query_first(format!("SHOW CREATE {kind} `{database}`.`{name}`"))
                .await
                .unwrap()
                .unwrap_or_else(|| panic!("{kind} {name} is missing"));
            let definition = texts(row)
                .into_iter()
                .flatten()
                .find(|text| text.to_ascii_uppercase().contains("CREATE "))
                .unwrap_or_default();
            parts.push(format!("{kind} {name}: {definition}"));
        }
        let view_rows: Vec<Row> = conn
            .query(format!(
                "SELECT shout FROM `{database}`.`v_top` ORDER BY id"
            ))
            .await
            .unwrap();
        parts.push(format!(
            "v_top rows: {:?}",
            view_rows
                .into_iter()
                .map(|row| texts(row)[0].clone())
                .collect::<Vec<_>>()
        ));
        parts
    }

    fn request(database: &str, path: &PathBuf) -> ExportRequest {
        serde_json::from_value(serde_json::json!({
            "jobId": unique("job"), "database": database, "path": path,
            "tables": ["a_child", "z_parent", "kinds", "bulk", "audit"], "views": ["v_base", "v_top"],
            "routines": true, "triggers": true, "events": true, "createDatabase": true, "maxStatementBytes": 4000
        }))
        .unwrap()
    }

    /// Run by hand (`BIG_ROWS=2000000`): checks that a big table streams through export and import quickly
    /// and without holding it in memory. Peak memory is read from the outside (`/usr/bin/time -l`).
    #[tokio::test]
    #[ignore = "slow; needs a live server and BIG_ROWS"]
    async fn a_big_table_round_trips_with_bounded_memory() {
        let wanted: u64 = std::env::var("BIG_ROWS")
            .ok()
            .and_then(|rows| rows.parse().ok())
            .unwrap_or(1_000_000);
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.open_session_conn().await.unwrap();
        let database = unique("lfdb_big");
        run(&mut conn, &format!("CREATE DATABASE `{database}`")).await;
        run(&mut conn, &format!("USE `{database}`")).await;
        run(&mut conn, "CREATE TABLE big (id BIGINT AUTO_INCREMENT PRIMARY KEY, n INT, price DECIMAL(10,2), note VARCHAR(80), created DATETIME) ENGINE=InnoDB").await;
        run(&mut conn, "INSERT INTO big (n, price, note, created) VALUES (1, 9.99, 'a note with some text; and \\'quotes\\'', '2026-01-01 00:00:00')").await;
        let mut rows = 1;
        while rows < wanted {
            run(&mut conn, "INSERT INTO big (n, price, note, created) SELECT n + 1, price + 0.01, note, created FROM big").await;
            rows *= 2;
        }
        let before: Vec<Row> = conn.query("CHECKSUM TABLE big").await.unwrap();
        let path = temp_file("big.sql");
        let mut big = request(&database, &path);
        big.tables = vec!["big".into()];
        big.views = vec![];
        big.routines = false;
        big.triggers = false;
        big.events = false;
        big.max_statement_bytes = None;

        let started = std::time::Instant::now();
        let exported = export_database(&mut conn, &big, &AtomicBool::new(false), &|_| {})
            .await
            .unwrap();
        let export_time = started.elapsed();
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let mut loader = active.open_session_conn().await.unwrap();
        let import_request: ImportRequest =
            serde_json::from_value(serde_json::json!({ "jobId": unique("job"), "path": path }))
                .unwrap();
        let started = std::time::Instant::now();
        let imported = import_file(
            &mut loader,
            &import_request,
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let import_time = started.elapsed();
        run(&mut conn, &format!("USE `{database}`")).await;
        let after: Vec<Row> = conn.query("CHECKSUM TABLE big").await.unwrap();
        let (before, after) = (
            texts(before.into_iter().next().unwrap())[1].clone(),
            texts(after.into_iter().next().unwrap())[1].clone(),
        );
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let megabytes = exported.bytes as f64 / 1_048_576.0;
        let _ = std::fs::remove_file(&path);

        println!(
            "BIG rows={} file={megabytes:.1} MiB statements={} export={export_time:.1?} ({:.0} MiB/s) import={import_time:.1?} ({:.0} MiB/s)",
            exported.rows,
            imported.statements,
            megabytes / export_time.as_secs_f64(),
            megabytes / import_time.as_secs_f64()
        );
        assert_eq!(exported.rows, rows);
        assert!(imported.errors.is_empty(), "{:?}", imported.errors);
        assert_eq!(before, after);
    }

    type Events = Arc<Mutex<Vec<ExportEvent>>>;

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_dumped_database_loads_back_identical_after_it_is_dropped() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.open_session_conn().await.unwrap();
        let database = unique("lfdb_dump");
        seed(&mut conn, &database).await;
        let before = fingerprint(&mut conn, &database).await;
        let path = temp_file("dump.sql");
        let events: Events = Arc::default();
        let sink = Arc::clone(&events);

        let exported = export_database(
            &mut conn,
            &request(&database, &path),
            &AtomicBool::new(false),
            &move |event| sink.lock().unwrap().push(event),
        )
        .await
        .unwrap();
        let file = std::fs::read_to_string(&path).unwrap();
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let mut loader = active.open_session_conn().await.unwrap();
        let import_request: ImportRequest =
            serde_json::from_value(serde_json::json!({ "jobId": unique("job"), "path": path }))
                .unwrap();
        let imported = import_file(
            &mut loader,
            &import_request,
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let after = fingerprint(&mut conn, &database).await;
        let triggered: Option<u64> = conn
            .query_first(format!("SELECT COUNT(*) FROM `{database}`.`audit`"))
            .await
            .unwrap();
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let _ = std::fs::remove_file(&path);

        assert!(
            !exported.cancelled && exported.rows >= 600 + 4,
            "{exported:?}"
        );
        assert!(
            imported.errors.is_empty() && !imported.failed,
            "{:?}",
            imported.errors
        );
        assert_eq!(before, after);
        // Triggers come after the data in the file, so loading does not fire them again: only the dumped audit row exists.
        assert_eq!(triggered, Some(1));
        assert!(
            file.matches("INSERT INTO `bulk`").count() > 5,
            "the bulk table should span many statements"
        );
        assert!(
            file.contains("DELIMITER ;;")
                && file.contains("TIME_ZONE='+00:00'")
                && !file.contains("DEFINER=")
        );
        assert!(!std::path::Path::new(&format!("{}.partial", path.display())).exists());
        let started = events
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, ExportEvent::Started { tables: 5 }));
        assert!(started);
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_dump_can_be_loaded_into_another_database() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.open_session_conn().await.unwrap();
        let (source, copy) = (unique("lfdb_src"), unique("lfdb_copy"));
        seed(&mut conn, &source).await;
        let path = temp_file("copy.sql");
        let mut plain = request(&source, &path);
        plain.create_database = false;
        plain.views = vec![];
        plain.routines = false;
        plain.events = false;
        plain.triggers = false;

        export_database(&mut conn, &plain, &AtomicBool::new(false), &|_| {})
            .await
            .unwrap();
        let mut loader = active.open_session_conn().await.unwrap();
        let import_request: ImportRequest = serde_json::from_value(
            serde_json::json!({ "jobId": unique("job"), "path": path, "database": copy }),
        )
        .unwrap();
        let imported = import_file(
            &mut loader,
            &import_request,
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let mut sums = Vec::new();
        for database in [&source, &copy] {
            for table in ["kinds", "bulk", "z_parent", "a_child"] {
                let row: Vec<Row> = conn
                    .query(format!("CHECKSUM TABLE `{database}`.`{table}`"))
                    .await
                    .unwrap();
                sums.push(texts(row.into_iter().next().unwrap())[1].clone());
            }
        }
        run(&mut conn, &format!("DROP DATABASE `{source}`")).await;
        run(&mut conn, &format!("DROP DATABASE `{copy}`")).await;
        let _ = std::fs::remove_file(&path);

        assert!(imported.errors.is_empty(), "{:?}", imported.errors);
        assert_eq!(sums[..4], sums[4..]);
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_cancelled_export_leaves_no_file_behind() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.open_session_conn().await.unwrap();
        let database = unique("lfdb_cancel");
        seed(&mut conn, &database).await;
        let path = temp_file("cancelled.sql");
        let cancel = AtomicBool::new(false);
        cancel.store(true, Ordering::SeqCst);

        let summary = export_database(&mut conn, &request(&database, &path), &cancel, &|_| {})
            .await
            .unwrap();
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;

        assert!(summary.cancelled);
        assert!(
            !path.exists()
                && !std::path::Path::new(&format!("{}.partial", path.display())).exists()
        );
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn an_import_stops_at_the_first_error_or_carries_on_and_reports_every_one() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.open_session_conn().await.unwrap();
        let database = unique("lfdb_errors");
        run(&mut conn, &format!("CREATE DATABASE `{database}`")).await;
        let path = temp_file("errors.sql");
        std::fs::write(&path, "CREATE TABLE t (a INT);\nINSERT INTO t VALUES (1);\nINSERT INTO missing_table VALUES (2);\nINSERT INTO t VALUES (3);\nINSERT INTO also_missing VALUES (4);\n").unwrap();
        let make = |stop: bool| -> ImportRequest {
            serde_json::from_value(serde_json::json!({ "jobId": unique("job"), "path": path, "database": database, "stopOnError": stop })).unwrap()
        };
        let seen: Arc<Mutex<Vec<ImportEvent>>> = Arc::default();
        let sink = Arc::clone(&seen);

        let mut stopping = active.open_session_conn().await.unwrap();
        let stopped = import_file(
            &mut stopping,
            &make(true),
            &AtomicBool::new(false),
            &move |event| sink.lock().unwrap().push(event),
        )
        .await
        .unwrap();
        let after_stop: Option<u64> = conn
            .query_first(format!("SELECT COUNT(*) FROM `{database}`.`t`"))
            .await
            .unwrap();
        run(&mut conn, &format!("DROP TABLE `{database}`.`t`")).await;
        let mut carrying = active.open_session_conn().await.unwrap();
        let carried = import_file(
            &mut carrying,
            &make(false),
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let after_carry: Option<u64> = conn
            .query_first(format!("SELECT COUNT(*) FROM `{database}`.`t`"))
            .await
            .unwrap();
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let _ = std::fs::remove_file(&path);

        assert!(stopped.failed && stopped.errors.len() == 1 && stopped.statements == 3);
        assert_eq!(
            (stopped.errors[0].statement, stopped.errors[0].line),
            (3, 3)
        );
        assert!(
            stopped.errors[0]
                .message
                .to_lowercase()
                .contains("missing_table")
        );
        assert_eq!(after_stop, Some(1));
        assert_eq!(
            carried
                .errors
                .iter()
                .map(|error| error.line)
                .collect::<Vec<_>>(),
            [3, 5]
        );
        assert_eq!(carried.statements, 5);
        assert_eq!(after_carry, Some(2));
        assert!(
            seen.lock()
                .unwrap()
                .iter()
                .any(|event| matches!(event, ImportEvent::StatementFailed { .. }))
        );
    }
}

// ---- server administration ----

mod admin_tests {
    use mysql_async::prelude::Queryable;
    use mysql_async::{Conn, OptsBuilder};

    use super::connect;
    use crate::admin::change::AdminChange;
    use crate::admin::grants::{GrantEntry, PrivilegeScope};
    use crate::admin::maintenance::{MaintenanceOp, maintain};
    use crate::admin::server;
    use crate::admin::users::{list_users, show_grants};
    use crate::ddl::apply::run_statements;

    fn unique(prefix: &str) -> String {
        format!(
            "{prefix}_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        )
    }

    async fn apply(conn: &mut Conn, json: serde_json::Value) {
        let change: AdminChange = serde_json::from_value(json).unwrap();
        run_statements(conn, &change.statements(false).unwrap())
            .await
            .unwrap_or_else(|error| panic!("{error}\n{:?}", change.statements(true)));
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn variables_status_engines_charsets_and_sizes_are_listed() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = unique("lfdb_adm");
        conn.query_drop(format!("CREATE DATABASE `{database}`"))
            .await
            .unwrap();
        conn.query_drop(format!(
            "CREATE TABLE `{database}`.`t` (id INT PRIMARY KEY, body TEXT) ENGINE=InnoDB"
        ))
        .await
        .unwrap();
        conn.query_drop(format!(
            "INSERT INTO `{database}`.`t` VALUES (1, REPEAT('x', 1000)), (2, REPEAT('y', 1000))"
        ))
        .await
        .unwrap();

        let variables = server::variables(&mut conn).await.unwrap();
        let status = server::status(&mut conn).await.unwrap();
        let engines = server::engines(&mut conn).await.unwrap();
        let charsets = server::charsets(&mut conn).await.unwrap();
        let stats = server::database_stats(&mut conn).await.unwrap();
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert!(
            variables
                .iter()
                .any(|variable| variable.name == "max_connections"
                    && variable.value.parse::<u64>().is_ok())
        );
        assert!(
            status
                .iter()
                .any(|entry| entry.name == "Uptime" && entry.value.parse::<u64>().unwrap() > 0)
        );
        assert!(engines.iter().any(|engine| engine.name == "InnoDB"
            && matches!(engine.support.as_str(), "DEFAULT" | "YES")));
        assert!(
            charsets
                .iter()
                .any(|charset| charset.name == "utf8mb4" && charset.max_length == 4)
        );
        let mine = stats
            .iter()
            .find(|entry| entry.name == database)
            .expect("the new database is listed");
        assert_eq!(mine.tables, 1);
        assert!(mine.data_bytes > 0);
        assert!(stats.iter().any(|entry| entry.name == "information_schema"));
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_global_variable_can_be_changed_and_put_back() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let read = |variables: Vec<server::NameValue>| {
            variables
                .into_iter()
                .find(|variable| variable.name == "max_connections")
                .unwrap()
                .value
        };
        let original = read(server::variables(&mut conn).await.unwrap());
        let changed = (original.parse::<u64>().unwrap() + 7).to_string();

        apply(&mut conn, serde_json::json!({"kind": "setVariable", "name": "max_connections", "value": changed, "scope": "global"})).await;
        let after = read(server::variables(&mut conn).await.unwrap());
        apply(&mut conn, serde_json::json!({"kind": "setVariable", "name": "max_connections", "value": original, "scope": "global"})).await;
        let restored = read(server::variables(&mut conn).await.unwrap());
        let read_only = run_statements(
            &mut conn,
            &AdminChange::SetVariable {
                name: "version".into(),
                value: "x".into(),
                scope: crate::admin::change::VariableScope::Global,
            }
            .statements(false)
            .unwrap(),
        )
        .await;

        assert_eq!(after, changed);
        assert_eq!(restored, original);
        assert!(
            read_only.is_err(),
            "a read-only variable must be refused by the server"
        );
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn running_statements_are_listed_and_a_connection_can_be_killed() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.conn().await.unwrap();
        let mut victim = active.open_session_conn().await.unwrap();
        let victim_id = victim.id();

        let listed = server::processes(&mut conn).await.unwrap();
        server::kill(&mut conn, u64::from(victim_id), false)
            .await
            .unwrap();
        let gone = victim.query_drop("SELECT 1").await;
        let invalid = server::kill(&mut conn, 0, true).await;

        assert!(
            listed
                .iter()
                .any(|process| process.id == u64::from(victim_id))
        );
        assert!(listed.iter().any(|process| {
            process.command == "Query"
                && process
                    .info
                    .as_deref()
                    .is_some_and(|info| info.contains("PROCESSLIST"))
        }));
        assert!(gone.is_err(), "a killed connection cannot run anything");
        assert!(invalid.is_err());
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn accounts_and_privileges_are_created_enforced_changed_and_removed() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.conn().await.unwrap();
        let database = unique("lfdb_priv");
        let user = unique("lfdb_user");
        conn.query_drop(format!("CREATE DATABASE `{database}`"))
            .await
            .unwrap();
        conn.query_drop(format!(
            "CREATE TABLE `{database}`.`t` (id INT PRIMARY KEY)"
        ))
        .await
        .unwrap();
        conn.query_drop(format!("INSERT INTO `{database}`.`t` VALUES (1)"))
            .await
            .unwrap();
        let on_database = serde_json::json!({"kind": "database", "database": database});

        apply(&mut conn, serde_json::json!({"kind": "createUser", "user": user, "host": "%", "password": "p@ss'word"})).await;
        apply(&mut conn, serde_json::json!({"kind": "grant", "user": user, "host": "%", "scope": on_database, "privileges": ["select", "insert"]})).await;
        let listed = list_users(&mut conn).await.unwrap();
        let grants = show_grants(&mut conn, &user, "%").await.unwrap();

        // Log in as the new account: the password works, SELECT and INSERT are allowed, DELETE is not.
        let as_user = |password: &str| {
            OptsBuilder::from_opts(active_opts(&active))
                .user(Some(user.as_str()))
                .pass(Some(password))
                .db_name(Some(database.as_str()))
        };
        let mut limited = Conn::new(as_user("p@ss'word"))
            .await
            .expect("the new account can log in");
        let selected: Option<u64> = limited.query_first("SELECT COUNT(*) FROM t").await.unwrap();
        let inserted = limited.query_drop("INSERT INTO t VALUES (2)").await;
        let deleted = limited.query_drop("DELETE FROM t WHERE id = 2").await;
        let wrong = Conn::new(as_user("not the password")).await;

        apply(&mut conn, serde_json::json!({"kind": "revoke", "user": user, "host": "%", "scope": on_database, "privileges": ["INSERT"]})).await;
        apply(&mut conn, serde_json::json!({"kind": "grant", "user": user, "host": "%", "scope": {"kind": "table", "database": database, "table": "t"}, "privileges": ["DELETE"], "grantOption": false})).await;
        let after = show_grants(&mut conn, &user, "%").await.unwrap();
        apply(&mut conn, serde_json::json!({"kind": "setPassword", "user": user, "host": "%", "password": "changed"})).await;
        let relogin = Conn::new(as_user("changed")).await;
        apply(
            &mut conn,
            serde_json::json!({"kind": "dropUser", "user": user, "host": "%"}),
        )
        .await;
        let remaining = list_users(&mut conn).await.unwrap();
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert!(
            listed
                .iter()
                .any(|account| account.user == user && account.host == "%")
        );
        let database_grant = grants.iter().find_map(|entry| match entry {
            GrantEntry::Privileges {
                scope: PrivilegeScope::Database { database: name },
                privileges,
                ..
            } if *name == database => Some(privileges.clone()),
            _ => None,
        });
        let mut granted = database_grant.expect("a database-level grant is listed");
        granted.sort();
        assert_eq!(granted, ["INSERT", "SELECT"]);
        assert_eq!(selected, Some(1));
        assert!(inserted.is_ok());
        assert!(deleted.is_err(), "DELETE was never granted");
        assert!(wrong.is_err());
        let names_after: Vec<String> = after
            .iter()
            .filter_map(|entry| match entry {
                GrantEntry::Privileges {
                    scope, privileges, ..
                } => Some(format!("{}:{}", scope.target_sql(), privileges.join(","))),
                GrantEntry::Raw { .. } => None,
            })
            .collect();
        assert!(
            names_after.iter().any(|text| text.ends_with(".*:SELECT")),
            "{names_after:?}"
        );
        assert!(
            names_after.iter().any(|text| text.ends_with("`t`:DELETE")),
            "{names_after:?}"
        );
        assert!(relogin.is_ok());
        assert!(remaining.iter().all(|account| account.user != user));
    }

    fn active_opts(active: &crate::connection::manager::ActiveConnection) -> mysql_async::Opts {
        active.driver_options()
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn tables_can_be_analysed_checked_optimised_and_checksummed() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = unique("lfdb_maint");
        conn.query_drop(format!("CREATE DATABASE `{database}`"))
            .await
            .unwrap();
        conn.query_drop(format!(
            "CREATE TABLE `{database}`.`a` (id INT PRIMARY KEY)"
        ))
        .await
        .unwrap();
        conn.query_drop(format!(
            "CREATE TABLE `{database}`.`b` (id INT PRIMARY KEY)"
        ))
        .await
        .unwrap();
        let tables = vec!["a".to_owned(), "b".to_owned()];

        let analyse = maintain(&mut conn, MaintenanceOp::Analyze, &database, &tables)
            .await
            .unwrap();
        let check = maintain(&mut conn, MaintenanceOp::Check, &database, &tables)
            .await
            .unwrap();
        let optimise = maintain(&mut conn, MaintenanceOp::Optimize, &database, &tables[..1])
            .await
            .unwrap();
        let checksum = maintain(&mut conn, MaintenanceOp::Checksum, &database, &tables)
            .await
            .unwrap();
        let missing = maintain(
            &mut conn,
            MaintenanceOp::Check,
            &database,
            &["nope".to_owned()],
        )
        .await
        .unwrap();
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert!(
            analyse.iter().any(|row| row.table.ends_with(".a")
                && row.kind == "status"
                && row.message.eq_ignore_ascii_case("OK")),
            "{analyse:?}"
        );
        assert_eq!(
            check
                .iter()
                .filter(|row| row.kind == "status" && row.message.eq_ignore_ascii_case("OK"))
                .count(),
            2,
            "{check:?}"
        );
        assert!(!optimise.is_empty());
        assert_eq!(checksum.len(), 2);
        assert!(checksum.iter().all(|row| row.kind == "checksum"));
        assert!(missing.iter().any(|row| row.kind == "error"), "{missing:?}");
    }
}

// ---- relationship diagram ----

mod diagram_tests {
    use mysql_async::prelude::Queryable;

    use super::connect;
    use crate::ddl::model::ReferentialAction;
    use crate::schema::diagram::read_diagram;

    fn unique(prefix: &str) -> String {
        format!(
            "{prefix}_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        )
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn tables_columns_and_every_kind_of_foreign_key_are_read() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let (database, other) = (unique("lfdb_dia"), unique("lfdb_other"));
        for sql in [
            format!("CREATE DATABASE `{other}`"),
            format!("CREATE TABLE `{other}`.`regions` (code CHAR(2) PRIMARY KEY) ENGINE=InnoDB"),
            format!("CREATE DATABASE `{database}`"),
            format!(
                "CREATE TABLE `{database}`.`customers` (id INT PRIMARY KEY, name VARCHAR(40) NOT NULL, region CHAR(2), boss_id INT NULL, INDEX (region), CONSTRAINT fk_boss FOREIGN KEY (boss_id) REFERENCES customers (id) ON DELETE SET NULL, CONSTRAINT fk_region FOREIGN KEY (region) REFERENCES `{other}`.`regions` (code)) ENGINE=InnoDB"
            ),
            format!(
                "CREATE TABLE `{database}`.`order_lines` (order_id INT, line_no INT, customer_id INT NOT NULL, PRIMARY KEY (order_id, line_no), INDEX (customer_id), CONSTRAINT fk_customer FOREIGN KEY (customer_id) REFERENCES customers (id) ON UPDATE CASCADE ON DELETE CASCADE) ENGINE=InnoDB"
            ),
            format!(
                "CREATE TABLE `{database}`.`line_notes` (order_id INT, line_no INT, note TEXT, CONSTRAINT fk_line FOREIGN KEY (order_id, line_no) REFERENCES order_lines (order_id, line_no)) ENGINE=InnoDB"
            ),
            format!("CREATE VIEW `{database}`.`v` AS SELECT id FROM `{database}`.`customers`"),
        ] {
            conn.query_drop(sql.as_str())
                .await
                .unwrap_or_else(|error| panic!("{error}\n{sql}"));
        }

        let diagram = read_diagram(&mut conn, &database).await.unwrap();
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();
        conn.query_drop(format!("DROP DATABASE `{other}`"))
            .await
            .unwrap();

        let names: Vec<&str> = diagram
            .tables
            .iter()
            .map(|table| table.name.as_str())
            .collect();
        assert_eq!(
            names,
            ["customers", "line_notes", "order_lines"],
            "views are not tables"
        );
        assert!(!diagram.truncated);
        let lines = diagram
            .tables
            .iter()
            .find(|table| table.name == "order_lines")
            .unwrap();
        assert_eq!(
            lines
                .columns
                .iter()
                .map(|column| (column.name.as_str(), column.primary_key))
                .collect::<Vec<_>>(),
            [
                ("order_id", true),
                ("line_no", true),
                ("customer_id", false)
            ]
        );
        let customers = diagram
            .tables
            .iter()
            .find(|table| table.name == "customers")
            .unwrap();
        assert!(
            customers
                .columns
                .iter()
                .find(|column| column.name == "boss_id")
                .unwrap()
                .nullable
        );
        assert_eq!(diagram.relations.len(), 4);
        let relation = |name: &str| {
            diagram
                .relations
                .iter()
                .find(|relation| relation.name == name)
                .unwrap_or_else(|| panic!("{name} missing"))
        };
        let composite = relation("fk_line");
        assert_eq!(
            (composite.columns.clone(), composite.ref_columns.clone()),
            (
                vec!["order_id".to_owned(), "line_no".to_owned()],
                vec!["order_id".to_owned(), "line_no".to_owned()]
            )
        );
        assert_eq!(relation("fk_boss").ref_table, "customers");
        assert_eq!(relation("fk_boss").on_delete, ReferentialAction::SetNull);
        assert_eq!(relation("fk_region").ref_database, other);
        assert_eq!(
            (
                relation("fk_customer").on_update,
                relation("fk_customer").on_delete
            ),
            (ReferentialAction::Cascade, ReferentialAction::Cascade)
        );
    }
}

// ---- SSH tunnel against a real OpenSSH server ----
//
// Needs `LFDB_SSH_PORT` (and optionally LFDB_SSH_HOST, LFDB_SSH_USER, LFDB_SSH_PASSWORD, LFDB_SSH_DIR with the key pair
// `id_test` (passphrase `keypass`) and `id_plain`), and a database the SSH server can reach, named by
// LFDB_SSH_TARGET_HOST and LFDB_SSH_TARGET_PORT. See the Makefile target `ssh-up`.

mod ssh_tests {
    use std::path::PathBuf;

    use mysql_async::prelude::Queryable;
    use mysql_async::{Conn, OptsBuilder};

    use crate::connection::model::{SshAuth, SshConfig};
    use crate::connection::ssh_tunnel::SshTunnel;
    use crate::error::AppError;

    fn env(name: &str, default: &str) -> String {
        std::env::var(name).unwrap_or_else(|_| default.to_owned())
    }

    fn config(auth: SshAuth) -> SshConfig {
        SshConfig {
            host: env("LFDB_SSH_HOST", "127.0.0.1"),
            port: env("LFDB_SSH_PORT", "33322").parse().unwrap(),
            user: env("LFDB_SSH_USER", "tunnel"),
            auth,
        }
    }

    fn target() -> (String, u16) {
        (
            env("LFDB_SSH_TARGET_HOST", "lfdb-mariadb"),
            env("LFDB_SSH_TARGET_PORT", "3306").parse().unwrap(),
        )
    }

    fn key(name: &str) -> String {
        PathBuf::from(env("LFDB_SSH_DIR", "/nonexistent"))
            .join(name)
            .display()
            .to_string()
    }

    fn known_hosts() -> PathBuf {
        std::env::temp_dir().join(format!(
            "lfdb_known_hosts_{}",
            uuid::Uuid::new_v4().simple()
        ))
    }

    async fn version_through(tunnel: &SshTunnel) -> String {
        let options = OptsBuilder::default()
            .ip_or_hostname("127.0.0.1")
            .tcp_port(tunnel.local_port())
            .user(Some("root"))
            .pass(None::<String>);
        let mut conn = Conn::new(options)
            .await
            .expect("the database is reachable through the tunnel");
        let version: Option<String> = conn.query_first("SELECT VERSION()").await.unwrap();
        version.unwrap()
    }

    #[tokio::test]
    #[ignore = "needs an OpenSSH server that refuses forwarding (make ssh-up)"]
    async fn a_server_that_refuses_forwarding_is_reported_as_such() {
        let tunnel = SshTunnel::open(
            &config(SshAuth::Password),
            Some(&env("LFDB_SSH_PASSWORD", "tunnelpass")),
            target(),
            true,
            Some(known_hosts()),
        )
        .await
        .unwrap();
        let options = OptsBuilder::default()
            .ip_or_hostname("127.0.0.1")
            .tcp_port(tunnel.local_port())
            .user(Some("root"));

        let attempt = Conn::new(options).await;
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert!(
            attempt.is_err(),
            "nothing can be reached when forwarding is refused"
        );
        let reason = tunnel.last_error().expect("the refusal was recorded");
        assert!(
            reason.to_lowercase().contains("refus")
                || reason.to_lowercase().contains("administratively")
                || reason.to_lowercase().contains("prohibit")
                || reason.to_lowercase().contains("denied"),
            "{reason}"
        );
    }

    #[tokio::test]
    #[ignore = "needs an OpenSSH server that allows forwarding (make ssh-up)"]
    async fn password_login_reaches_the_database_and_the_host_key_is_checked_and_remembered() {
        let known = known_hosts();
        let secret = env("LFDB_SSH_PASSWORD", "tunnelpass");

        let unknown = SshTunnel::open(
            &config(SshAuth::Password),
            Some(&secret),
            target(),
            false,
            Some(known.clone()),
        )
        .await;
        let tunnel = SshTunnel::open(
            &config(SshAuth::Password),
            Some(&secret),
            target(),
            true,
            Some(known.clone()),
        )
        .await
        .unwrap();
        let version = version_through(&tunnel).await;
        let remembered = SshTunnel::open(
            &config(SshAuth::Password),
            Some(&secret),
            target(),
            false,
            Some(known.clone()),
        )
        .await;
        let written = std::fs::read_to_string(&known).unwrap();
        let wrong = SshTunnel::open(
            &config(SshAuth::Password),
            Some("not the password"),
            target(),
            true,
            Some(known.clone()),
        )
        .await;

        assert!(
            matches!(unknown, Err(AppError::SshUnknownHost { ref fingerprint, .. }) if fingerprint.starts_with("SHA256:")),
            "{:?}",
            unknown.err()
        );
        assert!(!version.is_empty());
        assert!(
            remembered.is_ok(),
            "a host that was trusted once is accepted without asking again"
        );
        assert!(
            written.contains("ssh-ed25519")
                || written.contains("ssh-rsa")
                || written.contains("ecdsa"),
            "{written}"
        );
        assert!(matches!(wrong, Err(AppError::Ssh(_))), "{:?}", wrong.err());
        tunnel.close().await;
        let _ = std::fs::remove_file(&known);
    }

    #[tokio::test]
    #[ignore = "needs an OpenSSH server that allows forwarding (make ssh-up)"]
    async fn key_files_with_and_without_a_passphrase_work_and_a_wrong_passphrase_does_not() {
        let known = known_hosts();

        let with_passphrase = SshTunnel::open(
            &config(SshAuth::KeyFile {
                path: key("id_test"),
            }),
            Some("keypass"),
            target(),
            true,
            Some(known.clone()),
        )
        .await
        .unwrap();
        let version = version_through(&with_passphrase).await;
        let wrong = SshTunnel::open(
            &config(SshAuth::KeyFile {
                path: key("id_test"),
            }),
            Some("nope"),
            target(),
            true,
            Some(known.clone()),
        )
        .await;
        let unauthorised = SshTunnel::open(
            &config(SshAuth::KeyFile {
                path: key("id_unknown"),
            }),
            None,
            target(),
            true,
            Some(known.clone()),
        )
        .await;
        let missing = SshTunnel::open(
            &config(SshAuth::KeyFile {
                path: key("does_not_exist"),
            }),
            None,
            target(),
            true,
            Some(known.clone()),
        )
        .await;

        assert!(!version.is_empty());
        assert!(matches!(wrong, Err(AppError::Ssh(_))), "{:?}", wrong.err());
        assert!(
            matches!(unauthorised, Err(AppError::Ssh(_))),
            "a key the server does not know is refused: {:?}",
            unauthorised.err()
        );
        assert!(
            matches!(missing, Err(AppError::Ssh(_) | AppError::Io(_))),
            "{:?}",
            missing.err()
        );
        with_passphrase.close().await;
        let _ = std::fs::remove_file(&known);
    }

    #[tokio::test]
    #[ignore = "needs an OpenSSH server and a running ssh-agent holding the key id_plain (SSH_AUTH_SOCK)"]
    async fn the_ssh_agent_can_sign_in() {
        let known = known_hosts();

        let tunnel = SshTunnel::open(
            &config(SshAuth::Agent),
            None,
            target(),
            true,
            Some(known.clone()),
        )
        .await
        .unwrap();
        let version = version_through(&tunnel).await;
        tunnel.close().await;
        let _ = std::fs::remove_file(&known);

        assert!(!version.is_empty());
    }

    #[tokio::test]
    #[ignore = "needs an OpenSSH server (make ssh-up)"]
    async fn a_changed_host_key_is_refused_even_when_unknown_hosts_would_be_trusted() {
        let known = known_hosts();
        let secret = env("LFDB_SSH_PASSWORD", "tunnelpass");
        let other_key = std::fs::read_to_string(format!("{}.pub", key("id_plain"))).unwrap();
        let mut parts = other_key.split_whitespace();
        let (kind, data) = (parts.next().unwrap(), parts.next().unwrap());
        let c = config(SshAuth::Password);
        std::fs::write(&known, format!("[{}]:{} {kind} {data}\n", c.host, c.port)).unwrap();

        let result = SshTunnel::open(&c, Some(&secret), target(), true, Some(known.clone())).await;

        assert!(
            matches!(result, Err(AppError::SshHostKeyChanged { line: 1, .. })),
            "{:?}",
            result.err()
        );
        let _ = std::fs::remove_file(&known);
    }
}

// ---- single cells and exact counts ----

mod cell_tests {
    use mysql_async::prelude::Queryable;

    use super::connect;
    use crate::query::browse::{
        Filter, FilterOperator, MAX_CELL_BINARY_BYTES, count_rows, read_cell,
    };
    use crate::query::edit::ColumnValue;

    fn key(column: &str, value: &str) -> ColumnValue {
        ColumnValue {
            column: column.into(),
            value: Some(value.into()),
        }
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_whole_cell_is_read_even_when_the_grid_only_shows_a_preview_and_rows_are_counted() {
        let (manager, id) = connect().await;
        let mut conn = manager.get(&id).await.unwrap().conn().await.unwrap();
        let database = format!(
            "lfdb_cell_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        );
        conn.query_drop(format!("CREATE DATABASE `{database}`"))
            .await
            .unwrap();
        conn.query_drop(format!("CREATE TABLE `{database}`.`t` (id INT PRIMARY KEY, body LONGTEXT, raw LONGBLOB, tag VARCHAR(10))")).await.unwrap();
        conn.query_drop(format!("INSERT INTO `{database}`.`t` VALUES (1, REPEAT('é', 5000), REPEAT(0xAB, 100000), 'a'), (2, 'short', 0x00ff, 'b'), (3, NULL, NULL, 'a')")).await.unwrap();

        let long = read_cell(&mut conn, &database, "t", "body", &[key("id", "1")])
            .await
            .unwrap();
        let small_binary = read_cell(&mut conn, &database, "t", "raw", &[key("id", "2")])
            .await
            .unwrap();
        let big_binary = read_cell(&mut conn, &database, "t", "raw", &[key("id", "1")])
            .await
            .unwrap();
        let missing = read_cell(&mut conn, &database, "t", "body", &[key("id", "99")]).await;
        let ambiguous = read_cell(&mut conn, &database, "t", "body", &[key("tag", "a")]).await;
        let bad_column = read_cell(&mut conn, &database, "t", "nope", &[key("id", "1")]).await;
        let all = count_rows(&mut conn, &database, "t", &[]).await.unwrap();
        let filtered = count_rows(
            &mut conn,
            &database,
            "t",
            &[Filter {
                column: "tag".into(),
                operator: FilterOperator::Equals,
                value: Some("a".into()),
            }],
        )
        .await
        .unwrap();
        conn.query_drop(format!("DROP DATABASE `{database}`"))
            .await
            .unwrap();

        assert_eq!(
            (
                long.binary,
                long.text.chars().count(),
                long.size,
                long.truncated
            ),
            (false, 5000, 10_000, false)
        );
        assert_eq!(
            (
                small_binary.binary,
                small_binary.text.as_str(),
                small_binary.size,
                small_binary.truncated
            ),
            (true, "00ff", 2, false)
        );
        assert_eq!(
            (big_binary.text.len(), big_binary.size, big_binary.truncated),
            (MAX_CELL_BINARY_BYTES * 2, 100_000, true)
        );
        assert!(missing.is_err() && ambiguous.is_err() && bad_column.is_err());
        assert_eq!((all, filtered), (3, 2));
    }
}

// ---- gzip dumps, CSV import and result export ----

mod file_transfer_tests {
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    use mysql_async::prelude::Queryable;
    use mysql_async::{Conn, Row};

    use super::connect;
    use crate::schema::texts;
    use crate::transfer::csv_import::{CsvImportRequest, import_csv};
    use crate::transfer::export::{ExportRequest, export_database};
    use crate::transfer::import::{ImportRequest, import_file};
    use crate::transfer::result_export::{ResultExportRequest, export_result};

    fn unique(prefix: &str) -> String {
        format!(
            "{prefix}_{}",
            &uuid::Uuid::new_v4().simple().to_string()[..12]
        )
    }

    fn temp_file(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("{}_{name}", unique("lfdb")))
    }

    async fn run(conn: &mut Conn, sql: &str) {
        conn.query_drop(sql)
            .await
            .unwrap_or_else(|error| panic!("{error}\n{sql}"));
    }

    async fn checksum(conn: &mut Conn, database: &str, table: &str) -> String {
        let rows: Vec<Row> = conn
            .query(format!("CHECKSUM TABLE `{database}`.`{table}`"))
            .await
            .unwrap();
        texts(rows.into_iter().next().unwrap())[1].clone().unwrap()
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_gzip_dump_is_smaller_and_loads_back_identical() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.open_session_conn().await.unwrap();
        let database = unique("lfdb_gz");
        run(&mut conn, &format!("CREATE DATABASE `{database}`")).await;
        run(&mut conn, &format!("USE `{database}`")).await;
        run(
            &mut conn,
            "CREATE TABLE t (id INT PRIMARY KEY, body TEXT) ENGINE=InnoDB",
        )
        .await;
        let values: Vec<String> = (1..=800)
            .map(|id| format!("({id}, REPEAT('compressible text {id} ', 12))"))
            .collect();
        run(
            &mut conn,
            &format!("INSERT INTO t VALUES {}", values.join(",")),
        )
        .await;
        let before = checksum(&mut conn, &database, "t").await;
        let (plain, packed) = (temp_file("dump.sql"), temp_file("dump.sql.gz"));
        let request = |path: &PathBuf| -> ExportRequest {
            serde_json::from_value(serde_json::json!({ "jobId": unique("job"), "database": database, "path": path, "tables": ["t"], "createDatabase": true, "maxStatementBytes": 4000 })).unwrap()
        };

        let plain_summary = export_database(
            &mut conn,
            &request(&plain),
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let packed_summary = export_database(
            &mut conn,
            &request(&packed),
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let magic = std::fs::read(&packed).unwrap();
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let mut loader = active.open_session_conn().await.unwrap();
        let import_request: ImportRequest =
            serde_json::from_value(serde_json::json!({ "jobId": unique("job"), "path": packed }))
                .unwrap();
        let imported = import_file(
            &mut loader,
            &import_request,
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let after = checksum(&mut conn, &database, "t").await;
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let _ = (std::fs::remove_file(&plain), std::fs::remove_file(&packed));

        assert_eq!(&magic[..2], &[0x1f, 0x8b], "the file really is gzip");
        assert!(
            packed_summary.bytes * 5 < plain_summary.bytes,
            "{} compressed vs {} plain",
            packed_summary.bytes,
            plain_summary.bytes
        );
        assert!(imported.errors.is_empty(), "{:?}", imported.errors);
        assert_eq!(before, after);
        assert_eq!(
            imported.bytes, packed_summary.bytes,
            "progress counts the compressed bytes that were read"
        );
    }

    fn csv_request(path: &PathBuf, database: &str, mapping: serde_json::Value) -> CsvImportRequest {
        serde_json::from_value(serde_json::json!({
            "jobId": unique("job"), "path": path, "database": database, "table": "people", "delimiter": ",", "hasHeader": true,
            "mapping": mapping, "nullText": "NULL", "emptyAsNull": true
        }))
        .unwrap()
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn csv_rows_load_with_mapping_nulls_and_awkward_text_and_a_bad_row_changes_nothing() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.open_session_conn().await.unwrap();
        let database = unique("lfdb_csv");
        run(&mut conn, &format!("CREATE DATABASE `{database}`")).await;
        run(&mut conn, &format!("CREATE TABLE `{database}`.`people` (id INT PRIMARY KEY, name VARCHAR(60), age INT NULL, note TEXT NULL, extra INT NULL DEFAULT 5)")).await;
        let good = temp_file("good.csv");
        std::fs::write(&good, "\u{feff}id,name,skip,age,note\n1,\"Ann, \"\"the boss\"\"\",x,34,\"two\nlines\"\n2,Zoë 日本,x,NULL,\n3,Bob,x,,plain\n").unwrap();
        let bad = temp_file("bad.csv");
        let mut bad_text = String::from("id,name,skip,age,note\n");
        for number in 10..2010 {
            bad_text.push_str(&format!("{number},row {number},x,{number},fine\n"));
        }
        bad_text.push_str("2010,broken,x,not a number,oops\n2011,after,x,1,ok\n");
        std::fs::write(&bad, &bad_text).unwrap();
        let mapping = serde_json::json!(["id", "name", null, "age", "note"]);

        let loaded = import_csv(
            &mut conn,
            &csv_request(&good, &database, mapping.clone()),
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let rows: Vec<Row> = conn
            .query(format!(
                "SELECT id, name, age, note, extra FROM `{database}`.`people` ORDER BY id"
            ))
            .await
            .unwrap();
        let rows: Vec<Vec<Option<String>>> = rows.into_iter().map(texts).collect();
        let refused = import_csv(
            &mut conn,
            &csv_request(&bad, &database, mapping.clone()),
            &AtomicBool::new(false),
            &|_| {},
        )
        .await;
        let after_refusal: Option<u64> = conn
            .query_first(format!("SELECT COUNT(*) FROM `{database}`.`people`"))
            .await
            .unwrap();
        let mut replace = csv_request(&good, &database, mapping.clone());
        replace.truncate_first = true;
        let replaced = import_csv(&mut conn, &replace, &AtomicBool::new(false), &|_| {})
            .await
            .unwrap();
        let total_after_replace: Option<u64> = conn
            .query_first(format!("SELECT COUNT(*) FROM `{database}`.`people`"))
            .await
            .unwrap();
        let wrong_column = import_csv(
            &mut conn,
            &csv_request(&good, &database, serde_json::json!(["id", "nope"])),
            &AtomicBool::new(false),
            &|_| {},
        )
        .await;
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let _ = (std::fs::remove_file(&good), std::fs::remove_file(&bad));

        assert_eq!(loaded.rows, 3);
        assert_eq!(
            rows[0],
            [
                Some("1".into()),
                Some("Ann, \"the boss\"".into()),
                Some("34".into()),
                Some("two\nlines".into()),
                Some("5".into())
            ]
        );
        assert_eq!(rows[1][1].as_deref(), Some("Zoë 日本"));
        assert_eq!(rows[1][2], None, "NULL text became NULL");
        assert_eq!(rows[1][3], None, "an empty field became NULL");
        assert_eq!(rows[2][2], None);
        let message = refused.unwrap_err().to_string();
        assert!(message.contains("Row 2001"), "{message}");
        assert_eq!(
            after_refusal,
            Some(3),
            "the 2000 good rows before the bad one were rolled back too"
        );
        assert_eq!(replaced.rows, 3);
        assert_eq!(
            total_after_replace,
            Some(3),
            "truncate first replaced the rows instead of adding to them"
        );
        assert!(wrong_column.is_err());
    }

    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn a_result_is_exported_in_full_as_csv_and_json_with_no_display_limits() {
        let (manager, id) = connect().await;
        let active = manager.get(&id).await.unwrap();
        let mut conn = active.open_session_conn().await.unwrap();
        let database = unique("lfdb_res");
        run(&mut conn, &format!("CREATE DATABASE `{database}`")).await;
        run(&mut conn, &format!("CREATE TABLE `{database}`.`t` (id INT PRIMARY KEY, body LONGTEXT, price DECIMAL(20,4), raw VARBINARY(8), seen DATETIME NULL)")).await;
        run(&mut conn, &format!("INSERT INTO `{database}`.`t` VALUES (1, REPEAT('long ', 2000), 12345678901234.5678, 0xCAFE, '2026-01-31 10:00:00'), (2, 'say \"hi\", ok', 0.5000, NULL, NULL)")).await;
        let (csv, json) = (temp_file("out.csv"), temp_file("out.json"));
        let make = |path: &PathBuf, format: &str| -> ResultExportRequest {
            serde_json::from_value(serde_json::json!({ "jobId": unique("job"), "sql": "SELECT * FROM t ORDER BY id", "database": database, "path": path, "format": format, "nullText": "NULL", "byteOrderMark": true })).unwrap()
        };

        let csv_summary = export_result(
            &mut conn,
            &make(&csv, "csv"),
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let json_summary = export_result(
            &mut conn,
            &make(&json, "json"),
            &AtomicBool::new(false),
            &|_| {},
        )
        .await
        .unwrap();
        let no_rows = export_result(
            &mut conn,
            &ResultExportRequest {
                sql: "SET @x = 1".into(),
                path: temp_file("none.csv").display().to_string(),
                ..make(&csv, "csv")
            },
            &AtomicBool::new(false),
            &|_| {},
        )
        .await;
        let cancel = AtomicBool::new(true);
        let cancelled_path = temp_file("cancelled.csv");
        let many = ExportCancelProbe::run(&mut conn, &database, &cancelled_path, &cancel).await;
        let csv_text = std::fs::read_to_string(&csv).unwrap();
        let json_text = std::fs::read_to_string(&json).unwrap();
        run(&mut conn, &format!("DROP DATABASE `{database}`")).await;
        let _ = (std::fs::remove_file(&csv), std::fs::remove_file(&json));

        assert_eq!((csv_summary.rows, json_summary.rows), (2, 2));
        assert!(csv_text.starts_with('\u{feff}'));
        let records: Vec<csv::StringRecord> =
            csv::Reader::from_reader(csv_text.trim_start_matches('\u{feff}').as_bytes())
                .records()
                .map(Result::unwrap)
                .collect();
        assert_eq!(
            records[0].get(1).unwrap().len(),
            10_000,
            "a long text is not cut"
        );
        assert_eq!(
            (records[0].get(2), records[0].get(3), records[0].get(4)),
            (
                Some("12345678901234.5678"),
                Some("0xcafe"),
                Some("2026-01-31 10:00:00")
            )
        );
        assert_eq!(
            (records[1].get(1), records[1].get(3), records[1].get(4)),
            (Some("say \"hi\", ok"), Some("NULL"), Some("NULL"))
        );
        let parsed: serde_json::Value = serde_json::from_str(&json_text).unwrap();
        assert_eq!(parsed[0]["id"], 1);
        assert_eq!(parsed[0]["price"], "12345678901234.5678");
        assert_eq!(parsed[1]["raw"], serde_json::Value::Null);
        assert_eq!(parsed[0]["body"].as_str().unwrap().len(), 10_000);
        assert!(no_rows.is_err());
        assert!(many.cancelled && !cancelled_path.exists());
    }

    /// Exports enough rows that a raised cancel flag is noticed.
    struct ExportCancelProbe;

    impl ExportCancelProbe {
        async fn run(
            conn: &mut Conn,
            database: &str,
            path: &PathBuf,
            cancel: &AtomicBool,
        ) -> crate::transfer::result_export::ResultExportSummary {
            run(
                conn,
                &format!("CREATE TABLE `{database}`.`many` (n INT PRIMARY KEY)"),
            )
            .await;
            let values: Vec<String> = (1..=3000).map(|n| format!("({n})")).collect();
            run(
                conn,
                &format!(
                    "INSERT INTO `{database}`.`many` VALUES {}",
                    values.join(",")
                ),
            )
            .await;
            let request: ResultExportRequest = serde_json::from_value(serde_json::json!({ "jobId": unique("job"), "sql": "SELECT * FROM many", "database": database, "path": path, "format": "csv" })).unwrap();
            export_result(conn, &request, cancel, &|_| {})
                .await
                .unwrap()
        }
    }
}
