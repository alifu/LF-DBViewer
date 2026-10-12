//! Reading `SHOW GRANTS` output into something the interface can show and revoke.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum PrivilegeScope {
    Global,
    Database { database: String },
    Table { database: String, table: String },
}

impl PrivilegeScope {
    /// `*.*`, `` `db`.* `` or `` `db`.`table` ``.
    pub fn target_sql(&self) -> String {
        use crate::sql::quote_identifier;
        match self {
            Self::Global => "*.*".to_owned(),
            Self::Database { database } => format!("{}.*", quote_identifier(database)),
            Self::Table { database, table } => {
                format!("{}.{}", quote_identifier(database), quote_identifier(table))
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum GrantEntry {
    Privileges {
        scope: PrivilegeScope,
        privileges: Vec<String>,
        grant_option: bool,
    },
    /// Anything that is not a plain privilege grant: roles, proxies, column or routine privileges.
    Raw { text: String },
}

/// Splits on commas that are not inside parentheses or backticks (`SELECT (a, b), INSERT`).
fn split_privileges(text: &str) -> Vec<String> {
    let (mut parts, mut current, mut depth, mut quoted) = (Vec::new(), String::new(), 0, false);
    for character in text.chars() {
        match character {
            '`' => {
                quoted = !quoted;
                current.push(character);
            }
            '(' if !quoted => {
                depth += 1;
                current.push(character);
            }
            ')' if !quoted => {
                depth -= 1;
                current.push(character);
            }
            ',' if !quoted && depth == 0 => parts.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    parts.push(current);
    parts
        .into_iter()
        .map(|part| part.trim().to_owned())
        .filter(|part| !part.is_empty())
        .collect()
}

/// One name: `` `quoted` `` (with `` `` `` for a backtick), a bare word, or `*`. Returns it and what follows.
fn read_name(text: &str) -> Option<(String, &str)> {
    if let Some(rest) = text.strip_prefix('`') {
        let mut name = String::new();
        let mut chars = rest.char_indices().peekable();
        while let Some((index, character)) = chars.next() {
            if character == '`' {
                if chars.peek().is_some_and(|(_, next)| *next == '`') {
                    name.push('`');
                    chars.next();
                } else {
                    return Some((name, &rest[index + 1..]));
                }
            } else {
                name.push(character);
            }
        }
        return None;
    }
    let end = text
        .find(|c: char| c == '.' || c.is_whitespace())
        .unwrap_or(text.len());
    (end > 0).then(|| (text[..end].to_owned(), &text[end..]))
}

fn parse_scope(target: &str) -> Option<(PrivilegeScope, &str)> {
    let (first, rest) = read_name(target)?;
    let rest = rest.strip_prefix('.')?;
    let (second, rest) = read_name(rest)?;
    let scope = match (first.as_str(), second.as_str()) {
        ("*", "*") => PrivilegeScope::Global,
        (_, "*") => PrivilegeScope::Database { database: first },
        _ => PrivilegeScope::Table {
            database: first,
            table: second,
        },
    };
    Some((scope, rest))
}

pub fn parse_grant(text: &str) -> GrantEntry {
    let raw = || GrantEntry::Raw {
        text: text.to_owned(),
    };
    let Some(after_grant) = text.strip_prefix("GRANT ") else {
        return raw();
    };
    // The privilege list ends at the first ` ON ` outside parentheses; a list cannot contain backticked text with it,
    // except in column privileges, which are shown as they are.
    let Some(on) = after_grant.find(" ON ") else {
        return raw();
    };
    let list = &after_grant[..on];
    if list.contains('(') {
        return raw();
    }
    let target = after_grant[on + " ON ".len()..].trim_start();
    if ["TABLE ", "FUNCTION ", "PROCEDURE "]
        .iter()
        .any(|kind| target.starts_with(kind))
    {
        return raw();
    }
    let Some((scope, rest)) = parse_scope(target) else {
        return raw();
    };
    if !rest.trim_start().starts_with("TO ") {
        return raw();
    }
    GrantEntry::Privileges {
        scope,
        privileges: split_privileges(list),
        grant_option: text.trim_end().ends_with("WITH GRANT OPTION"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn privileges(text: &str) -> (PrivilegeScope, Vec<String>, bool) {
        match parse_grant(text) {
            GrantEntry::Privileges {
                scope,
                privileges,
                grant_option,
            } => (scope, privileges, grant_option),
            other => panic!("expected privileges, got {other:?}"),
        }
    }

    #[test]
    fn global_database_and_table_grants_are_read_with_their_privileges() {
        assert_eq!(
            privileges("GRANT SELECT, INSERT, UPDATE ON *.* TO `app`@`%`"),
            (
                PrivilegeScope::Global,
                vec!["SELECT".into(), "INSERT".into(), "UPDATE".into()],
                false
            )
        );
        assert_eq!(
            privileges("GRANT SELECT, CREATE VIEW ON `shop`.* TO `app`@`%`"),
            (
                PrivilegeScope::Database {
                    database: "shop".into()
                },
                vec!["SELECT".into(), "CREATE VIEW".into()],
                false
            )
        );
        assert_eq!(
            privileges("GRANT DELETE ON `shop`.`orders` TO `app`@`%`"),
            (
                PrivilegeScope::Table {
                    database: "shop".into(),
                    table: "orders".into()
                },
                vec!["DELETE".into()],
                false
            )
        );
    }

    #[test]
    fn grant_option_usage_and_all_privileges_are_recognised() {
        assert_eq!(
            privileges("GRANT ALL PRIVILEGES ON *.* TO `root`@`localhost` WITH GRANT OPTION"),
            (PrivilegeScope::Global, vec!["ALL PRIVILEGES".into()], true)
        );
        assert_eq!(
            privileges("GRANT USAGE ON *.* TO `nobody`@`%`").1,
            ["USAGE"]
        );
    }

    #[test]
    fn mariadb_grants_with_authentication_clauses_are_read_too() {
        let text = "GRANT ALL PRIVILEGES ON *.* TO `root`@`%` IDENTIFIED VIA mysql_native_password USING '*ABC' OR unix_socket WITH GRANT OPTION";

        assert_eq!(
            privileges(text),
            (PrivilegeScope::Global, vec!["ALL PRIVILEGES".into()], true)
        );
    }

    #[test]
    fn names_with_dots_spaces_escapes_and_backticks_survive() {
        assert_eq!(
            privileges("GRANT SELECT ON `my.db`.`we``ird table` TO `u`@`h`"),
            (
                PrivilegeScope::Table {
                    database: "my.db".into(),
                    table: "we`ird table".into()
                },
                vec!["SELECT".into()],
                false
            )
        );
        assert_eq!(
            privileges("GRANT SELECT ON `my\\_db`.* TO `u`@`h`").0,
            PrivilegeScope::Database {
                database: "my\\_db".into()
            }
        );
    }

    #[test]
    fn column_routine_role_and_proxy_grants_are_left_as_text() {
        for text in [
            "GRANT SELECT (`a`, `b`) ON `shop`.`orders` TO `u`@`h`",
            "GRANT EXECUTE ON PROCEDURE `shop`.`p` TO `u`@`h`",
            "GRANT `reader` TO `u`@`h`",
            "GRANT PROXY ON ''@'' TO 'root'@'localhost' WITH GRANT OPTION",
            "something else",
        ] {
            assert_eq!(
                parse_grant(text),
                GrantEntry::Raw {
                    text: text.to_owned()
                },
                "{text}"
            );
        }
    }

    #[test]
    fn scopes_turn_back_into_grant_targets() {
        assert_eq!(PrivilegeScope::Global.target_sql(), "*.*");
        assert_eq!(
            PrivilegeScope::Database {
                database: "we`ird".into()
            }
            .target_sql(),
            "`we``ird`.*"
        );
        assert_eq!(
            PrivilegeScope::Table {
                database: "a".into(),
                table: "b".into()
            }
            .target_sql(),
            "`a`.`b`"
        );
    }

    #[test]
    fn privilege_lists_split_on_top_level_commas_only() {
        assert_eq!(
            split_privileges("SELECT, INSERT (a, b), `x,y`"),
            ["SELECT", "INSERT (a, b)", "`x,y`"]
        );
    }
}
