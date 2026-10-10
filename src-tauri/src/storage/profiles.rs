use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::connection::model::ConnectionProfile;
use crate::error::{AppError, AppResult};

const PROFILES_FILE: &str = "connections.json";

/// Saved connections as a JSON file in the app data directory. No secrets in here.
pub struct ProfileStore {
    path: PathBuf,
    write_lock: Mutex<()>,
}

/// A copy of `profiles` with `profile` replacing the entry with the same id, or appended.
fn with_upserted(
    profiles: &[ConnectionProfile],
    profile: &ConnectionProfile,
) -> Vec<ConnectionProfile> {
    if profiles.iter().any(|existing| existing.id == profile.id) {
        return profiles
            .iter()
            .map(|existing| {
                if existing.id == profile.id {
                    profile
                } else {
                    existing
                }
            })
            .cloned()
            .collect();
    }
    profiles
        .iter()
        .chain(std::iter::once(profile))
        .cloned()
        .collect()
}

impl ProfileStore {
    pub fn new(data_dir: &Path) -> Self {
        Self {
            path: data_dir.join(PROFILES_FILE),
            write_lock: Mutex::new(()),
        }
    }

    pub fn list(&self) -> AppResult<Vec<ConnectionProfile>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn upsert(&self, profile: &ConnectionProfile) -> AppResult<()> {
        let _guard = self.lock()?;
        self.write(&with_upserted(&self.list()?, profile))
    }

    pub fn remove(&self, id: &str) -> AppResult<()> {
        let _guard = self.lock()?;
        let remaining: Vec<_> = self
            .list()?
            .into_iter()
            .filter(|profile| profile.id != id)
            .collect();
        self.write(&remaining)
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, ()>> {
        self.write_lock
            .lock()
            .map_err(|_| AppError::Storage("profile store lock is poisoned".into()))
    }

    /// Writes to a sibling file and renames, so a crash never leaves a half-written list.
    fn write(&self, profiles: &[ConnectionProfile]) -> AppResult<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = self.path.with_extension("json.tmp");
        std::fs::write(&temporary, serde_json::to_vec_pretty(profiles)?)?;
        std::fs::rename(&temporary, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::model::sample_profile;

    fn store() -> (tempfile::TempDir, ProfileStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn lists_nothing_before_anything_is_saved() {
        let (_dir, store) = store();

        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn upsert_appends_new_and_replaces_existing_by_id() {
        let (_dir, store) = store();
        let first = sample_profile();
        let second = ConnectionProfile {
            id: "p2".into(),
            ..sample_profile()
        };
        let renamed = ConnectionProfile {
            name: "Renamed".into(),
            ..sample_profile()
        };

        store.upsert(&first).unwrap();
        store.upsert(&second).unwrap();
        store.upsert(&renamed).unwrap();

        assert_eq!(store.list().unwrap(), vec![renamed, second]);
    }

    #[test]
    fn remove_deletes_only_the_matching_profile() {
        let (_dir, store) = store();
        let kept = ConnectionProfile {
            id: "p2".into(),
            ..sample_profile()
        };
        store.upsert(&sample_profile()).unwrap();
        store.upsert(&kept).unwrap();

        store.remove("p1").unwrap();

        assert_eq!(store.list().unwrap(), vec![kept]);
    }

    #[test]
    fn corrupt_file_is_an_error_not_an_empty_list() {
        let (dir, store) = store();
        std::fs::write(dir.path().join(PROFILES_FILE), b"{not json").unwrap();

        assert!(matches!(store.list(), Err(AppError::Storage(_))));
    }
}
