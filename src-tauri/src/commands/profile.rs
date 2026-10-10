use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;

use crate::connection::client_config::{self, ClientDefaults};
use crate::connection::model::ConnectionProfile;
use crate::error::{AppError, AppResult};
use crate::state::AppState;
use crate::storage::secrets::{SecretKind, SecretStore, secret_key};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveProfileRequest {
    pub profile: ConnectionProfile,
    /// `None` leaves the stored password as it is.
    pub password: Option<String>,
    pub ssh_secret: Option<String>,
    pub save_secrets: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedProfile {
    #[serde(flatten)]
    pub profile: ConnectionProfile,
    pub has_saved_password: bool,
}

/// Keychain calls block (and may show a system prompt), so keep them off the async workers.
pub async fn on_keychain<T: Send + 'static>(
    secrets: Arc<dyn SecretStore>,
    work: impl FnOnce(&dyn SecretStore) -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(move || work(secrets.as_ref()))
        .await
        .map_err(|error| AppError::Secret(error.to_string()))?
}

fn store_secrets(
    secrets: &dyn SecretStore,
    request: &SaveProfileRequest,
    id: &str,
) -> AppResult<()> {
    let entries = [
        (SecretKind::DatabasePassword, &request.password),
        (SecretKind::SshSecret, &request.ssh_secret),
    ];
    for (kind, value) in entries {
        let key = secret_key(id, kind);
        match (request.save_secrets, value) {
            (false, _) => secrets.delete(&key)?,
            (true, Some(value)) if value.is_empty() => secrets.delete(&key)?,
            (true, Some(value)) => secrets.set(&key, value)?,
            (true, None) => {}
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> AppResult<Vec<SavedProfile>> {
    let profiles = state.profiles.list()?;
    on_keychain(Arc::clone(&state.secrets), move |secrets| {
        profiles
            .into_iter()
            .map(|profile| {
                let key = secret_key(&profile.id, SecretKind::DatabasePassword);
                Ok(SavedProfile {
                    has_saved_password: secrets.get(&key)?.is_some(),
                    profile,
                })
            })
            .collect()
    })
    .await
}

#[tauri::command]
pub async fn save_profile(
    state: State<'_, AppState>,
    request: SaveProfileRequest,
) -> AppResult<ConnectionProfile> {
    let validated = request.profile.validated()?;
    let profile = if validated.id.is_empty() {
        ConnectionProfile {
            id: uuid::Uuid::new_v4().to_string(),
            ..validated
        }
    } else {
        validated
    };

    state.profiles.upsert(&profile)?;
    let id = profile.id.clone();
    on_keychain(Arc::clone(&state.secrets), move |secrets| {
        store_secrets(secrets, &request, &id)
    })
    .await?;
    Ok(profile)
}

#[tauri::command]
pub async fn delete_profile(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state.profiles.remove(&id)?;
    on_keychain(Arc::clone(&state.secrets), move |secrets| {
        secrets.delete(&secret_key(&id, SecretKind::DatabasePassword))?;
        secrets.delete(&secret_key(&id, SecretKind::SshSecret))
    })
    .await
}

/// What a terminal `mysql` would connect with, or all-empty when nothing is configured.
#[tauri::command]
pub async fn detect_client_defaults() -> AppResult<ClientDefaults> {
    tokio::task::spawn_blocking(client_config::detect)
        .await
        .map_err(|error| AppError::Storage(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::model::sample_profile;
    use crate::storage::secrets::testing::MemoryStore;

    fn request(password: Option<&str>, save_secrets: bool) -> SaveProfileRequest {
        SaveProfileRequest {
            profile: sample_profile(),
            password: password.map(str::to_owned),
            ssh_secret: None,
            save_secrets,
        }
    }

    #[test]
    fn saves_a_provided_password_and_keeps_it_when_none_is_sent() {
        let store = MemoryStore::default();

        store_secrets(&store, &request(Some("hunter2"), true), "p1").unwrap();
        store_secrets(&store, &request(None, true), "p1").unwrap();

        assert_eq!(store.get("p1:db").unwrap().as_deref(), Some("hunter2"));
    }

    #[test]
    fn turning_saving_off_or_sending_blank_removes_the_password() {
        let store = MemoryStore::default();
        store.set("p1:db", "hunter2").unwrap();
        store.set("p1:ssh", "passphrase").unwrap();

        store_secrets(&store, &request(Some(""), true), "p1").unwrap();
        assert_eq!(store.get("p1:db").unwrap(), None);
        assert_eq!(store.get("p1:ssh").unwrap().as_deref(), Some("passphrase"));

        store_secrets(&store, &request(None, false), "p1").unwrap();
        assert_eq!(store.get("p1:ssh").unwrap(), None);
    }
}
