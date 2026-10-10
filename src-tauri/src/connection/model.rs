use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

pub const DEFAULT_MYSQL_PORT: u16 = 3306;
pub const DEFAULT_SSH_PORT: u16 = 22;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Mysql,
    Mariadb,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SslMode {
    #[default]
    Disabled,
    /// Encrypt, but do not verify the server certificate.
    Required,
    /// Encrypt and verify the certificate chain.
    VerifyCa,
    /// Encrypt, verify the chain and the host name.
    VerifyIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", rename_all = "camelCase")]
pub enum SshAuth {
    Password,
    KeyFile { path: String },
    Agent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshConfig {
    pub host: String,
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
}

/// Everything about a connection except its secrets, which live in the Keychain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionProfile {
    #[serde(default)]
    pub id: String,
    pub name: String,
    pub engine: Engine,
    #[serde(default)]
    pub host: String,
    #[serde(default = "default_mysql_port")]
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub database: Option<String>,
    #[serde(default)]
    pub socket: Option<String>,
    #[serde(default)]
    pub ssl_mode: SslMode,
    #[serde(default)]
    pub ssl_ca: Option<String>,
    #[serde(default)]
    pub ssh: Option<SshConfig>,
}

/// Secrets for one connection attempt. Deliberately not `Debug` or `Serialize`.
#[derive(Clone, Default)]
pub struct ConnectionSecrets {
    pub password: Option<String>,
    pub ssh_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub id: String,
    pub profile_id: String,
    pub name: String,
    pub engine: Engine,
    pub server_version: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub database: Option<String>,
    pub via_ssh: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionStatus {
    pub connected: bool,
    pub latency_ms: Option<u64>,
    pub error: Option<String>,
}

fn default_mysql_port() -> u16 {
    DEFAULT_MYSQL_PORT
}

fn default_ssh_port() -> u16 {
    DEFAULT_SSH_PORT
}

fn blank_to_none(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

impl ConnectionProfile {
    /// Returns a trimmed copy, or the first problem found.
    pub fn validated(&self) -> AppResult<Self> {
        let profile = Self {
            name: self.name.trim().to_owned(),
            host: self.host.trim().to_owned(),
            user: self.user.trim().to_owned(),
            database: blank_to_none(&self.database),
            socket: blank_to_none(&self.socket),
            ssl_ca: blank_to_none(&self.ssl_ca),
            ssh: self.ssh.as_ref().map(SshConfig::trimmed),
            ..self.clone()
        };

        if profile.name.is_empty() {
            return Err(AppError::Validation("Connection name is required".into()));
        }
        if profile.user.is_empty() {
            return Err(AppError::Validation("User is required".into()));
        }
        if profile.socket.is_none() && profile.host.is_empty() {
            return Err(AppError::Validation("Host or socket is required".into()));
        }
        if profile.socket.is_none() && profile.port == 0 {
            return Err(AppError::Validation(
                "Port must be between 1 and 65535".into(),
            ));
        }
        if profile.socket.is_some() && profile.ssh.is_some() {
            return Err(AppError::Validation(
                "A socket connection cannot go through SSH".into(),
            ));
        }
        if profile.ssl_mode == SslMode::VerifyIdentity && profile.ssh.is_some() {
            return Err(AppError::Validation(
                "Host name verification is not available through an SSH tunnel; use Verify CA"
                    .into(),
            ));
        }
        if let Some(ssh) = &profile.ssh {
            ssh.validate()?;
        }
        Ok(profile)
    }

    /// The engine the server actually reports, which wins over what the user picked.
    pub fn engine_for_version(version: &str) -> Engine {
        if version.to_ascii_lowercase().contains("mariadb") {
            Engine::Mariadb
        } else {
            Engine::Mysql
        }
    }
}

impl SshConfig {
    fn trimmed(&self) -> Self {
        let auth = match &self.auth {
            SshAuth::KeyFile { path } => SshAuth::KeyFile {
                path: path.trim().to_owned(),
            },
            other => other.clone(),
        };
        Self {
            host: self.host.trim().to_owned(),
            user: self.user.trim().to_owned(),
            port: self.port,
            auth,
        }
    }

    fn validate(&self) -> AppResult<()> {
        if self.host.is_empty() {
            return Err(AppError::Validation("SSH host is required".into()));
        }
        if self.user.is_empty() {
            return Err(AppError::Validation("SSH user is required".into()));
        }
        if self.port == 0 {
            return Err(AppError::Validation(
                "SSH port must be between 1 and 65535".into(),
            ));
        }
        if matches!(&self.auth, SshAuth::KeyFile { path } if path.is_empty()) {
            return Err(AppError::Validation("SSH key file is required".into()));
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) fn sample_profile() -> ConnectionProfile {
    ConnectionProfile {
        id: "p1".into(),
        name: "Local".into(),
        engine: Engine::Mysql,
        host: "127.0.0.1".into(),
        port: DEFAULT_MYSQL_PORT,
        user: "root".into(),
        database: None,
        socket: None,
        ssl_mode: SslMode::Disabled,
        ssl_ca: None,
        ssh: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(auth: SshAuth) -> SshConfig {
        SshConfig {
            host: "bastion".into(),
            port: DEFAULT_SSH_PORT,
            user: "deploy".into(),
            auth,
        }
    }

    #[test]
    fn trims_fields_and_turns_blank_optionals_into_none() {
        let profile = ConnectionProfile {
            name: "  Local ".into(),
            database: Some("   ".into()),
            ..sample_profile()
        };

        let validated = profile.validated().unwrap();

        assert_eq!(validated.name, "Local");
        assert_eq!(validated.database, None);
    }

    #[test]
    fn rejects_missing_name_user_and_host() {
        for broken in [
            ConnectionProfile {
                name: " ".into(),
                ..sample_profile()
            },
            ConnectionProfile {
                user: "".into(),
                ..sample_profile()
            },
            ConnectionProfile {
                host: "".into(),
                ..sample_profile()
            },
            ConnectionProfile {
                port: 0,
                ..sample_profile()
            },
        ] {
            assert!(matches!(broken.validated(), Err(AppError::Validation(_))));
        }
    }

    #[test]
    fn allows_socket_without_host() {
        let profile = ConnectionProfile {
            host: "".into(),
            socket: Some("/tmp/mysql.sock".into()),
            ..sample_profile()
        };

        assert!(profile.validated().is_ok());
    }

    #[test]
    fn rejects_socket_combined_with_ssh() {
        let profile = ConnectionProfile {
            socket: Some("/tmp/mysql.sock".into()),
            ssh: Some(ssh(SshAuth::Agent)),
            ..sample_profile()
        };

        assert!(matches!(profile.validated(), Err(AppError::Validation(_))));
    }

    #[test]
    fn rejects_ssh_key_auth_without_a_path() {
        let profile = ConnectionProfile {
            ssh: Some(ssh(SshAuth::KeyFile { path: "  ".into() })),
            ..sample_profile()
        };

        assert!(matches!(profile.validated(), Err(AppError::Validation(_))));
    }

    #[test]
    fn rejects_host_name_verification_through_ssh() {
        let profile = ConnectionProfile {
            ssl_mode: SslMode::VerifyIdentity,
            ssh: Some(ssh(SshAuth::Agent)),
            ..sample_profile()
        };

        assert!(matches!(profile.validated(), Err(AppError::Validation(_))));
    }

    #[test]
    fn detects_engine_from_version_string() {
        assert_eq!(
            ConnectionProfile::engine_for_version("11.4.2-MariaDB-ubu2404"),
            Engine::Mariadb
        );
        assert_eq!(
            ConnectionProfile::engine_for_version("9.3.0"),
            Engine::Mysql
        );
    }

    #[test]
    fn profile_json_uses_camel_case_and_tagged_ssh_auth() {
        let profile = ConnectionProfile {
            ssh: Some(ssh(SshAuth::KeyFile {
                path: "~/.ssh/id_ed25519".into(),
            })),
            ..sample_profile()
        };

        let json = serde_json::to_value(&profile).unwrap();

        assert_eq!(json["sslMode"], "disabled");
        assert_eq!(json["ssh"]["auth"]["method"], "keyFile");
        assert_eq!(
            serde_json::from_value::<ConnectionProfile>(json).unwrap(),
            profile
        );
    }
}
