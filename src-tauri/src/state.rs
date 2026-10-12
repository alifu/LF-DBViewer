use std::path::Path;
use std::sync::Arc;

use crate::connection::manager::ConnectionManager;
use crate::query::session::SessionManager;
use crate::storage::profiles::ProfileStore;
use crate::storage::secrets::{KeychainStore, SecretStore};
use crate::transfer::jobs::JobRegistry;

pub struct AppState {
    pub connections: ConnectionManager,
    pub sessions: SessionManager,
    pub jobs: JobRegistry,
    pub profiles: ProfileStore,
    pub secrets: Arc<dyn SecretStore>,
}

impl AppState {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            connections: ConnectionManager::default(),
            sessions: SessionManager::default(),
            jobs: JobRegistry::default(),
            profiles: ProfileStore::new(data_dir),
            secrets: Arc::new(KeychainStore),
        }
    }
}
