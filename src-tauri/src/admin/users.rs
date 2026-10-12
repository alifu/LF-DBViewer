use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::Serialize;

use super::grants::{GrantEntry, parse_grant};
use crate::error::{AppError, AppResult};
use crate::schema::{optional, required, texts};
use crate::sql::quote_string;

#[derive(Debug, Clone, Serialize)]
pub struct UserAccount {
    pub user: String,
    pub host: String,
    pub plugin: Option<String>,
}

/// Needs the right to read `mysql.user`; without it the server's own message says so.
pub async fn list_users(conn: &mut Conn) -> AppResult<Vec<UserAccount>> {
    let rows: Vec<Row> = conn
        .query("SELECT User, Host, plugin FROM mysql.user ORDER BY User, Host")
        .await?;
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| UserAccount {
            user: required(row.first()),
            host: required(row.get(1)),
            plugin: optional(row.get(2)),
        })
        .collect())
}

pub async fn show_grants(conn: &mut Conn, user: &str, host: &str) -> AppResult<Vec<GrantEntry>> {
    if user.contains('\0') || host.contains('\0') {
        return Err(AppError::Validation("User name is not allowed".into()));
    }
    let rows: Vec<Row> = conn
        .query(format!(
            "SHOW GRANTS FOR {}@{}",
            quote_string(user),
            quote_string(host)
        ))
        .await?;
    Ok(rows
        .into_iter()
        .filter_map(|row| texts(row).into_iter().next().flatten())
        .map(|text| parse_grant(&text))
        .collect())
}
