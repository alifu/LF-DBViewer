use std::path::Path;
use std::sync::Arc;

use crate::connection::manager::ConnectionManager;
use crate::query::session::SessionManager;
use crate::storage::profiles::ProfileStore;
use crate::storage::secrets::{KeychainStore, SecretStore};

pub struct AppState {
    pub connections: ConnectionManager,
    pub sessions: SessionManager,
    pub profiles: ProfileStore,
    pub secrets: Arc<dyn SecretStore>,
}

impl AppState {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            connections: ConnectionManager::default(),
            sessions: SessionManager::default(),
            profiles: ProfileStore::new(data_dir),
            secrets: Arc::new(KeychainStore),
        }
    }
}
