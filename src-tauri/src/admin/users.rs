use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Row};
use serde::Serialize;

use super::grants::{GrantEntry, parse_grant};
use crate::connection::model::Engine;
use crate::error::{AppError, AppResult};
use crate::schema::{optional, required, texts};
use crate::sql::quote_string;

#[derive(Debug, Clone, Serialize)]
pub struct UserAccount {
    pub user: String,
    pub host: String,
    pub plugin: Option<String>,
    /// `None` when the server does not say.
    pub locked: Option<bool>,
    pub password_expired: Option<bool>,
}

fn flag(value: Option<&Option<String>>) -> Option<bool> {
    optional(value).map(|text| matches!(text.as_str(), "Y" | "y" | "true" | "1"))
}

/// MySQL keeps the lock and expiry flags in `mysql.user`; MariaDB 10.4+ keeps them inside a JSON column of
/// `mysql.global_priv`. If the richer query is not possible (older server, fewer rights), the plain list is used.
const LIST_MYSQL: &str = "SELECT User, Host, plugin, account_locked, password_expired FROM mysql.user ORDER BY User, Host";
const LIST_MARIADB: &str = "SELECT User, Host, JSON_VALUE(Priv, '$.plugin'), JSON_VALUE(Priv, '$.account_locked'), IF(JSON_VALUE(Priv, '$.password_last_changed') = 0, 'true', 'false') FROM mysql.global_priv ORDER BY User, Host";
const LIST_PLAIN: &str =
    "SELECT User, Host, plugin, NULL, NULL FROM mysql.user ORDER BY User, Host";

/// Needs the right to read `mysql.user`; without it the server's own message says so.
pub async fn list_users(conn: &mut Conn, engine: Engine) -> AppResult<Vec<UserAccount>> {
    let rich = if engine == Engine::Mariadb {
        LIST_MARIADB
    } else {
        LIST_MYSQL
    };
    let rows: Vec<Row> = match conn.query(rich).await {
        Ok(rows) => rows,
        Err(_) => conn.query(LIST_PLAIN).await?,
    };
    Ok(rows
        .into_iter()
        .map(texts)
        .map(|row| UserAccount {
            user: required(row.first()),
            host: required(row.get(1)),
            plugin: optional(row.get(2)),
            locked: flag(row.get(3)),
            password_expired: flag(row.get(4)),
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
