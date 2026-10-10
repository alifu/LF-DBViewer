//! Reads the MySQL client settings a terminal `mysql` would use (option files
//! and environment variables) so the login form can be pre-filled from them.

use std::path::PathBuf;

use serde::Serialize;

const CLIENT_SECTIONS: [&str; 2] = ["client", "mysql"];
const SYSTEM_OPTION_FILES: [&str; 4] = [
    "/etc/my.cnf",
    "/etc/mysql/my.cnf",
    "/usr/local/etc/my.cnf",
    "/opt/homebrew/etc/my.cnf",
];
const USER_OPTION_FILE: &str = ".my.cnf";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientDefaults {
    pub host: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub password: Option<String>,
    pub socket: Option<String>,
    pub database: Option<String>,
    /// Where the values came from, for display.
    pub sources: Vec<String>,
}

impl ClientDefaults {
    pub fn is_empty(&self) -> bool {
        self.host.is_none()
            && self.port.is_none()
            && self.user.is_none()
            && self.password.is_none()
            && self.socket.is_none()
            && self.database.is_none()
    }

    /// A copy where every value set in `later` wins.
    fn overridden_by(&self, later: &Self) -> Self {
        Self {
            host: later.host.clone().or_else(|| self.host.clone()),
            port: later.port.or(self.port),
            user: later.user.clone().or_else(|| self.user.clone()),
            password: later.password.clone().or_else(|| self.password.clone()),
            socket: later.socket.clone().or_else(|| self.socket.clone()),
            database: later.database.clone().or_else(|| self.database.clone()),
            sources: self.sources.iter().chain(&later.sources).cloned().collect(),
        }
    }
}

fn unquoted(value: &str) -> String {
    let value = value.trim();
    for quote in ['"', '\''] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            return value[1..value.len() - 1].to_owned();
        }
    }
    // An unquoted value ends at a trailing comment.
    value.split(" #").next().unwrap_or(value).trim().to_owned()
}

/// Parses the `[client]` and `[mysql]` sections of an option file.
pub fn parse_option_file(text: &str) -> ClientDefaults {
    let mut defaults = ClientDefaults::default();
    let mut in_client_section = false;

    for line in text.lines().map(str::trim) {
        if line.is_empty()
            || line.starts_with('#')
            || line.starts_with(';')
            || line.starts_with('!')
        {
            continue;
        }
        if let Some(section) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_client_section =
                CLIENT_SECTIONS.contains(&section.trim().to_ascii_lowercase().as_str());
            continue;
        }
        if !in_client_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = unquoted(value);
        if value.is_empty() {
            continue;
        }
        match key.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "host" => defaults.host = Some(value),
            "port" => defaults.port = value.parse().ok().or(defaults.port),
            "user" => defaults.user = Some(value),
            "password" => defaults.password = Some(value),
            "socket" => defaults.socket = Some(value),
            "database" => defaults.database = Some(value),
            _ => {}
        }
    }
    defaults
}

/// Reads the `MYSQL_*` variables the command-line client honours.
pub fn from_environment(lookup: impl Fn(&str) -> Option<String>) -> ClientDefaults {
    let non_empty = |name: &str| lookup(name).filter(|value| !value.is_empty());
    let defaults = ClientDefaults {
        host: non_empty("MYSQL_HOST"),
        port: non_empty("MYSQL_TCP_PORT").and_then(|port| port.parse().ok()),
        password: non_empty("MYSQL_PWD"),
        socket: non_empty("MYSQL_UNIX_PORT"),
        ..ClientDefaults::default()
    };
    if defaults.is_empty() {
        return defaults;
    }
    ClientDefaults {
        sources: vec!["environment".into()],
        ..defaults
    }
}

fn option_file_paths() -> Vec<PathBuf> {
    let system = SYSTEM_OPTION_FILES.iter().map(PathBuf::from);
    let user = std::env::var_os("HOME").map(|home| PathBuf::from(home).join(USER_OPTION_FILE));
    system.chain(user).collect()
}

/// Merges option files (later files win, as in the mysql client) and then the environment.
/// Unreadable or missing files are skipped: this is a convenience, not a requirement.
pub fn detect() -> ClientDefaults {
    let from_files = option_file_paths()
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let parsed = parse_option_file(&text);
            (!parsed.is_empty()).then(|| ClientDefaults {
                sources: vec![path.display().to_string()],
                ..parsed
            })
        })
        .fold(ClientDefaults::default(), |merged, next| {
            merged.overridden_by(&next)
        });

    from_files.overridden_by(&from_environment(|name| std::env::var(name).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_client_and_mysql_sections_only() {
        let text = "
            [mysqld]
            port = 3307
            bind-address = 127.0.0.1

            [client]
            host = db.internal
            port = 3310
            user = alif
            password = \"s3cr#t\"

            [mysql]
            database = shop
        ";

        let defaults = parse_option_file(text);

        assert_eq!(defaults.host.as_deref(), Some("db.internal"));
        assert_eq!(defaults.port, Some(3310));
        assert_eq!(defaults.user.as_deref(), Some("alif"));
        assert_eq!(defaults.password.as_deref(), Some("s3cr#t"));
        assert_eq!(defaults.database.as_deref(), Some("shop"));
    }

    #[test]
    fn server_only_file_yields_nothing() {
        let defaults = parse_option_file("[mysqld]\nbind-address = 127.0.0.1\nport=3306\n");

        assert!(defaults.is_empty());
    }

    #[test]
    fn ignores_comments_blank_values_and_bad_ports() {
        let text = "[client]\n# host = nope\nuser = root # admin\nport = abc\nsocket =\n";

        let defaults = parse_option_file(text);

        assert_eq!(defaults.user.as_deref(), Some("root"));
        assert_eq!(defaults.host, None);
        assert_eq!(defaults.port, None);
        assert_eq!(defaults.socket, None);
    }

    #[test]
    fn environment_values_are_read_and_labelled() {
        let defaults = from_environment(|name| match name {
            "MYSQL_HOST" => Some("envhost".into()),
            "MYSQL_TCP_PORT" => Some("3399".into()),
            _ => None,
        });

        assert_eq!(defaults.host.as_deref(), Some("envhost"));
        assert_eq!(defaults.port, Some(3399));
        assert_eq!(defaults.sources, vec!["environment".to_owned()]);
    }

    #[test]
    fn empty_environment_yields_nothing() {
        assert!(from_environment(|_| None).is_empty());
        assert!(from_environment(|_| None).sources.is_empty());
    }

    #[test]
    fn later_source_overrides_earlier_but_keeps_unset_values() {
        let file = ClientDefaults {
            host: Some("filehost".into()),
            user: Some("fileuser".into()),
            sources: vec!["~/.my.cnf".into()],
            ..ClientDefaults::default()
        };
        let environment = ClientDefaults {
            host: Some("envhost".into()),
            sources: vec!["environment".into()],
            ..ClientDefaults::default()
        };

        let merged = file.overridden_by(&environment);

        assert_eq!(merged.host.as_deref(), Some("envhost"));
        assert_eq!(merged.user.as_deref(), Some("fileuser"));
        assert_eq!(merged.sources.len(), 2);
    }
}
