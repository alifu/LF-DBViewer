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
