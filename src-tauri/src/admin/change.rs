//! Changes to the server itself: variables, accounts and privileges. Like table changes, they are turned into SQL in
//! one place, so the preview and what runs are the same. Passwords are masked in the text that is shown or logged.

use serde::Deserialize;

use super::grants::PrivilegeScope;
use super::server::{check_variable_name, variable_literal};
use crate::error::{AppError, AppResult};
use crate::sql::quote_string;

const MAX_ACCOUNT_CHARS: usize = 80;
const MASK: &str = "********";
const ANY_HOST: &str = "%";

const TABLE_PRIVILEGES: [&str; 12] = [
    "SELECT",
    "INSERT",
    "UPDATE",
    "DELETE",
    "CREATE",
    "DROP",
    "ALTER",
    "INDEX",
    "REFERENCES",
    "CREATE VIEW",
    "SHOW VIEW",
    "TRIGGER",
];
const DATABASE_ONLY_PRIVILEGES: [&str; 6] = [
    "CREATE TEMPORARY TABLES",
    "LOCK TABLES",
    "EXECUTE",
    "CREATE ROUTINE",
    "ALTER ROUTINE",
    "EVENT",
];
const GLOBAL_ONLY_PRIVILEGES: [&str; 10] = [
    "FILE",
    "PROCESS",
    "RELOAD",
    "SHUTDOWN",
    "SUPER",
    "SHOW DATABASES",
    "CREATE USER",
    "REPLICATION CLIENT",
    "REPLICATION SLAVE",
    "CREATE TABLESPACE",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VariableScope {
    Global,
    /// MySQL 8: also written to `mysqld-auto.cnf`, so it survives a restart.
    Persist,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum AdminChange {
    SetVariable {
        name: String,
        value: String,
        scope: VariableScope,
    },
    CreateUser {
        user: String,
        host: String,
        password: String,
    },
    SetPassword {
        user: String,
        host: String,
        password: String,
    },
    DropUser {
        user: String,
        host: String,
    },
    Grant {
        user: String,
        host: String,
        scope: PrivilegeScope,
        privileges: Vec<String>,
        #[serde(default)]
        grant_option: bool,
    },
    Revoke {
        user: String,
        host: String,
        scope: PrivilegeScope,
        privileges: Vec<String>,
    },
}

fn account(user: &str, host: &str) -> AppResult<String> {
    let host = if host.trim().is_empty() {
        ANY_HOST
    } else {
        host.trim()
    };
    for (value, what) in [(user, "User"), (host, "Host")] {
        if value.contains('\0') || value.chars().count() > MAX_ACCOUNT_CHARS {
            return Err(AppError::Validation(format!("{what} name is not allowed")));
        }
    }
    if user.trim().is_empty() {
        return Err(AppError::Validation("User name is required".into()));
    }
    Ok(format!("{}@{}", quote_string(user), quote_string(host)))
}

fn password_clause(password: &str, mask: bool) -> AppResult<String> {
    if password.is_empty() {
        return Err(AppError::Validation("A password is required".into()));
    }
    Ok(format!(
        "IDENTIFIED BY {}",
        quote_string(if mask { MASK } else { password })
    ))
}

fn allowed(scope: &PrivilegeScope) -> Vec<&'static str> {
    let mut names: Vec<&str> = TABLE_PRIVILEGES.to_vec();
    if !matches!(scope, PrivilegeScope::Table { .. }) {
        names.extend(DATABASE_ONLY_PRIVILEGES);
    }
    if matches!(scope, PrivilegeScope::Global) {
        names.extend(GLOBAL_ONLY_PRIVILEGES);
    }
    names.extend(["ALL PRIVILEGES", "GRANT OPTION"]);
    names
}

/// Upper-cased, single-spaced, and checked against what the scope can have, so a name can never carry extra SQL.
fn privilege_list(
    scope: &PrivilegeScope,
    privileges: &[String],
    allow_grant_option: bool,
) -> AppResult<String> {
    let allowed = allowed(scope);
    let mut chosen: Vec<String> = Vec::new();
    for privilege in privileges {
        let normal = privilege
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_ascii_uppercase();
        let normal = if normal == "ALL" {
            "ALL PRIVILEGES".to_owned()
        } else {
            normal
        };
        if !allowed.contains(&normal.as_str()) || (normal == "GRANT OPTION" && !allow_grant_option)
        {
            return Err(AppError::Validation(format!(
                "\"{privilege}\" is not a privilege that can be set here"
            )));
        }
        if !chosen.contains(&normal) {
            chosen.push(normal);
        }
    }
    if chosen.is_empty() {
        return Err(AppError::Validation("Choose at least one privilege".into()));
    }
    Ok(chosen.join(", "))
}

fn check_scope(scope: &PrivilegeScope) -> AppResult<()> {
    let names: Vec<&String> = match scope {
        PrivilegeScope::Global => vec![],
        PrivilegeScope::Database { database } => vec![database],
        PrivilegeScope::Table { database, table } => vec![database, table],
    };
    if names
        .iter()
        .any(|name| name.trim().is_empty() || name.contains('\0'))
    {
        return Err(AppError::Validation(
            "The database or table name is missing".into(),
        ));
    }
    Ok(())
}

impl AdminChange {
    /// With `mask`, passwords are replaced by asterisks: that is the text for the review dialog and the log.
    pub fn statements(&self, mask: bool) -> AppResult<Vec<String>> {
        match self {
            Self::SetVariable { name, value, scope } => {
                check_variable_name(name)?;
                let keyword = if *scope == VariableScope::Persist {
                    "PERSIST"
                } else {
                    "GLOBAL"
                };
                Ok(vec![format!(
                    "SET {keyword} {name} = {}",
                    variable_literal(value)
                )])
            }
            Self::CreateUser {
                user,
                host,
                password,
            } => Ok(vec![format!(
                "CREATE USER {} {}",
                account(user, host)?,
                password_clause(password, mask)?
            )]),
            Self::SetPassword {
                user,
                host,
                password,
            } => Ok(vec![format!(
                "ALTER USER {} {}",
                account(user, host)?,
                password_clause(password, mask)?
            )]),
            Self::DropUser { user, host } => {
                Ok(vec![format!("DROP USER {}", account(user, host)?)])
            }
            Self::Grant {
                user,
                host,
                scope,
                privileges,
                grant_option,
            } => {
                check_scope(scope)?;
                let list = privilege_list(scope, privileges, false)?;
                let with = if *grant_option {
                    " WITH GRANT OPTION"
                } else {
                    ""
                };
                Ok(vec![format!(
                    "GRANT {list} ON {} TO {}{with}",
                    scope.target_sql(),
                    account(user, host)?
                )])
            }
            Self::Revoke {
                user,
                host,
                scope,
                privileges,
            } => {
                check_scope(scope)?;
                let list = privilege_list(scope, privileges, true)?;
                Ok(vec![format!(
                    "REVOKE {list} ON {} FROM {}",
                    scope.target_sql(),
                    account(user, host)?
                )])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(json: &str) -> AdminChange {
        serde_json::from_str(json).unwrap()
    }

    fn sql(json: &str) -> AppResult<Vec<String>> {
        change(json).statements(false)
    }

    #[test]
    fn variables_are_set_globally_or_persistently_with_the_right_literal() {
        assert_eq!(
            sql(
                r#"{"kind":"setVariable","name":"max_connections","value":"300","scope":"global"}"#
            )
            .unwrap(),
            ["SET GLOBAL max_connections = 300"]
        );
        assert_eq!(
            sql(r#"{"kind":"setVariable","name":"slow_query_log","value":"on","scope":"persist"}"#)
                .unwrap(),
            ["SET PERSIST slow_query_log = ON"]
        );
        assert_eq!(sql(r#"{"kind":"setVariable","name":"sql_mode","value":"STRICT_ALL_TABLES","scope":"global"}"#).unwrap(), ["SET GLOBAL sql_mode = 'STRICT_ALL_TABLES'"]);
        assert!(sql(r#"{"kind":"setVariable","name":"x = 1; DROP DATABASE y; --","value":"1","scope":"global"}"#).is_err());
    }

    #[test]
    fn users_are_created_changed_and_dropped_with_quoted_accounts() {
        let create =
            change(r#"{"kind":"createUser","user":"app","host":"10.0.%","password":"s3cr'et"}"#);

        assert_eq!(
            create.statements(false).unwrap(),
            ["CREATE USER 'app'@'10.0.%' IDENTIFIED BY 's3cr''et'"]
        );
        assert_eq!(
            create.statements(true).unwrap(),
            ["CREATE USER 'app'@'10.0.%' IDENTIFIED BY '********'"]
        );
        assert_eq!(
            sql(r#"{"kind":"setPassword","user":"app","host":"","password":"new"}"#).unwrap(),
            ["ALTER USER 'app'@'%' IDENTIFIED BY 'new'"]
        );
        assert_eq!(
            sql(r#"{"kind":"dropUser","user":"o'brien","host":"localhost"}"#).unwrap(),
            ["DROP USER 'o''brien'@'localhost'"]
        );
    }

    #[test]
    fn an_account_needs_a_user_and_a_password_when_one_is_set() {
        assert!(sql(r#"{"kind":"createUser","user":"  ","host":"%","password":"x"}"#).is_err());
        assert!(sql(r#"{"kind":"createUser","user":"app","host":"%","password":""}"#).is_err());
        assert!(sql(r#"{"kind":"dropUser","user":"a\u0000b","host":"%"}"#).is_err());
    }

    #[test]
    fn grants_name_their_scope_and_can_pass_on_the_grant_option() {
        assert_eq!(
            sql(r#"{"kind":"grant","user":"app","host":"%","scope":{"kind":"database","database":"shop"},"privileges":["select","Create View"],"grantOption":true}"#).unwrap(),
            ["GRANT SELECT, CREATE VIEW ON `shop`.* TO 'app'@'%' WITH GRANT OPTION"]
        );
        assert_eq!(
            sql(r#"{"kind":"grant","user":"app","host":"%","scope":{"kind":"table","database":"shop","table":"orders"},"privileges":["SELECT","SELECT","UPDATE"]}"#).unwrap(),
            ["GRANT SELECT, UPDATE ON `shop`.`orders` TO 'app'@'%'"]
        );
        assert_eq!(sql(r#"{"kind":"grant","user":"dba","host":"localhost","scope":{"kind":"global"},"privileges":["all"]}"#).unwrap(), ["GRANT ALL PRIVILEGES ON *.* TO 'dba'@'localhost'"]);
    }

    #[test]
    fn revoke_may_include_the_grant_option() {
        assert_eq!(
            sql(r#"{"kind":"revoke","user":"app","host":"%","scope":{"kind":"database","database":"shop"},"privileges":["INSERT","GRANT OPTION"]}"#).unwrap(),
            ["REVOKE INSERT, GRANT OPTION ON `shop`.* FROM 'app'@'%'"]
        );
    }

    #[test]
    fn only_privileges_the_scope_can_have_are_accepted_and_nothing_can_be_smuggled_in() {
        for json in [
            r#"{"kind":"grant","user":"a","host":"%","scope":{"kind":"table","database":"d","table":"t"},"privileges":["SHUTDOWN"]}"#,
            r#"{"kind":"grant","user":"a","host":"%","scope":{"kind":"database","database":"d"},"privileges":["SUPER"]}"#,
            r#"{"kind":"grant","user":"a","host":"%","scope":{"kind":"global"},"privileges":["SELECT ON *.* TO evil@% --"]}"#,
            r#"{"kind":"grant","user":"a","host":"%","scope":{"kind":"global"},"privileges":["GRANT OPTION"]}"#,
            r#"{"kind":"grant","user":"a","host":"%","scope":{"kind":"global"},"privileges":[]}"#,
            r#"{"kind":"revoke","user":"a","host":"%","scope":{"kind":"database","database":" "},"privileges":["SELECT"]}"#,
        ] {
            assert!(matches!(sql(json), Err(AppError::Validation(_))), "{json}");
        }
    }
}
