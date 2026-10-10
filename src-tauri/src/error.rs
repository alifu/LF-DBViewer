use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

pub type AppResult<T> = Result<T, AppError>;

/// Every failure that can cross the command boundary. Serialized as
/// `{ code, message }` so the UI can branch on `code` and show `message`.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Validation(String),
    #[error("Connection is not open: {0}")]
    ConnectionNotFound(String),
    #[error("{0}")]
    Database(String),
    #[error("Timed out after {seconds}s while {action}")]
    Timeout { action: String, seconds: u64 },
    #[error("SSH: {0}")]
    Ssh(String),
    #[error("SSH host {host}:{port} is not in known_hosts (fingerprint {fingerprint})")]
    SshUnknownHost {
        host: String,
        port: u16,
        fingerprint: String,
    },
    #[error(
        "SSH host key for {host}:{port} does not match known_hosts line {line}. \
         Refusing to connect; this can mean someone is intercepting the connection."
    )]
    SshHostKeyChanged {
        host: String,
        port: u16,
        line: usize,
    },
    #[error("Keychain: {0}")]
    Secret(String),
    #[error("Storage: {0}")]
    Storage(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

impl AppError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) => "validation",
            Self::ConnectionNotFound(_) => "connection_not_found",
            Self::Database(_) => "database",
            Self::Timeout { .. } => "timeout",
            Self::Ssh(_) => "ssh",
            Self::SshUnknownHost { .. } => "ssh_unknown_host",
            Self::SshHostKeyChanged { .. } => "ssh_host_key_changed",
            Self::Secret(_) => "secret",
            Self::Storage(_) => "storage",
            Self::Io(_) => "io",
        }
    }
}

impl From<mysql_async::Error> for AppError {
    fn from(error: mysql_async::Error) -> Self {
        Self::Database(error.to_string())
    }
}

impl From<russh::Error> for AppError {
    fn from(error: russh::Error) -> Self {
        Self::Ssh(error.to_string())
    }
}

impl From<russh::keys::Error> for AppError {
    fn from(error: russh::keys::Error) -> Self {
        Self::Ssh(error.to_string())
    }
}

impl From<serde_json::Error> for AppError {
    fn from(error: serde_json::Error) -> Self {
        Self::Storage(error.to_string())
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", self.code())?;
        state.serialize_field("message", &self.to_string())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_as_code_and_message() {
        let error = AppError::ConnectionNotFound("abc".into());

        let json = serde_json::to_value(&error).unwrap();

        assert_eq!(json["code"], "connection_not_found");
        assert_eq!(json["message"], "Connection is not open: abc");
    }

    #[test]
    fn unknown_ssh_host_carries_fingerprint_in_message() {
        let error = AppError::SshUnknownHost {
            host: "bastion".into(),
            port: 22,
            fingerprint: "SHA256:xyz".into(),
        };

        let json = serde_json::to_value(&error).unwrap();

        assert_eq!(json["code"], "ssh_unknown_host");
        assert!(json["message"].as_str().unwrap().contains("SHA256:xyz"));
    }
}
