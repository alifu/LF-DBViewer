use mysql_async::Conn;
use mysql_async::prelude::Queryable;

use crate::error::{AppError, AppResult};

fn first_line(sql: &str) -> &str {
    sql.lines().next().unwrap_or(sql)
}

/// Runs the statements in order and stops at the first failure. DDL cannot be rolled back, so the
/// error says how many statements had already run.
pub async fn run_statements(conn: &mut Conn, statements: &[String]) -> AppResult<()> {
    for (position, statement) in statements.iter().enumerate() {
        if let Err(error) = conn.query_drop(statement.as_str()).await {
            let earlier = match position {
                0 => String::new(),
                1 => " The 1 statement before it already ran.".to_owned(),
                count => format!(" The {count} statements before it already ran."),
            };
            return Err(AppError::Database(format!(
                "{} failed: {error}.{earlier}",
                first_line(statement)
            )));
        }
    }
    Ok(())
}
