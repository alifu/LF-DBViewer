use std::sync::Arc;

use serde::Deserialize;
use tauri::State;

use super::profile::on_keychain;
use crate::connection::model::{
    ConnectionInfo, ConnectionProfile, ConnectionSecrets, ConnectionStatus,
};
use crate::error::AppResult;
use crate::state::AppState;
use crate::storage::secrets::{SecretKind, SecretStore, secret_key};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectRequest {
    pub profile: ConnectionProfile,
    /// `None` means "use the saved one"; an empty string means "no password".
    pub password: Option<String>,
    pub ssh_secret: Option<String>,
    /// Set after the user has confirmed an unknown SSH host's fingerprint.
    #[serde(default)]
    pub trust_ssh_host: bool,
}

fn saved_or(
    secrets: &dyn SecretStore,
    id: &str,
    kind: SecretKind,
    typed: Option<String>,
) -> AppResult<Option<String>> {
    if typed.is_some() || id.is_empty() {
        return Ok(typed);
    }
    secrets.get(&secret_key(id, kind))
}

async fn resolve_secrets(
    state: &AppState,
    request: &ConnectRequest,
) -> AppResult<ConnectionSecrets> {
    let id = request.profile.id.clone();
    let (password, ssh_secret) = (request.password.clone(), request.ssh_secret.clone());
    on_keychain(Arc::clone(&state.secrets), move |secrets| {
        Ok(ConnectionSecrets {
            password: saved_or(secrets, &id, SecretKind::DatabasePassword, password)?,
            ssh_secret: saved_or(secrets, &id, SecretKind::SshSecret, ssh_secret)?,
        })
    })
    .await
}

#[tauri::command]
pub async fn open_connection(
    state: State<'_, AppState>,
    request: ConnectRequest,
) -> AppResult<ConnectionInfo> {
    let secrets = resolve_secrets(&state, &request).await?;
    state
        .connections
        .open(request.profile, secrets, request.trust_ssh_host)
        .await
}

#[tauri::command]
pub async fn test_connection(
    state: State<'_, AppState>,
    request: ConnectRequest,
) -> AppResult<ConnectionInfo> {
    let secrets = resolve_secrets(&state, &request).await?;
    state
        .connections
        .test(request.profile, secrets, request.trust_ssh_host)
        .await
}

#[tauri::command]
pub async fn close_connection(state: State<'_, AppState>, connection_id: String) -> AppResult<()> {
    state.connections.close(&connection_id).await
}

#[tauri::command]
pub async fn ping_connection(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<ConnectionStatus> {
    state.connections.ping(&connection_id).await
}

#[tauri::command]
pub async fn reconnect_connection(
    state: State<'_, AppState>,
    connection_id: String,
) -> AppResult<ConnectionInfo> {
    state.connections.reconnect(&connection_id).await
}

#[tauri::command]
pub async fn list_connections(state: State<'_, AppState>) -> AppResult<Vec<ConnectionInfo>> {
    Ok(state.connections.list().await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::secrets::testing::MemoryStore;

    #[test]
    fn typed_secret_wins_and_saved_one_is_the_fallback() {
        let store = MemoryStore::default();
        store.set("p1:db", "saved").unwrap();
        let kind = SecretKind::DatabasePassword;

        assert_eq!(
            saved_or(&store, "p1", kind, Some("typed".into()))
                .unwrap()
                .as_deref(),
            Some("typed")
        );
        assert_eq!(
            saved_or(&store, "p1", kind, Some(String::new()))
                .unwrap()
                .as_deref(),
            Some("")
        );
        assert_eq!(
            saved_or(&store, "p1", kind, None).unwrap().as_deref(),
            Some("saved")
        );
        assert_eq!(saved_or(&store, "", kind, None).unwrap(), None);
    }

    #[test]
    fn connect_request_deserializes_from_what_the_interface_sends() {
        let json = r#"{"profile":{"id":"","name":"Local","engine":"mysql","host":"127.0.0.1","port":3306,
                       "user":"root","database":null,"socket":null,"sslMode":"disabled","sslCa":null,
                       "ssh":{"host":"bastion","port":22,"user":"me","auth":{"method":"keyFile","path":"~/.ssh/id"}}},
                       "password":"","sshSecret":null,"trustSshHost":true}"#;

        let request: ConnectRequest = serde_json::from_str(json).unwrap();

        assert_eq!(request.password.as_deref(), Some(""));
        assert_eq!(request.ssh_secret, None);
        assert!(request.trust_ssh_host);
        assert!(request.profile.ssh.is_some());
    }
}
