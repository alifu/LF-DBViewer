use crate::error::{AppError, AppResult};

const KEYCHAIN_SERVICE: &str = "com.alifu.lf-dbviewer";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretKind {
    DatabasePassword,
    SshSecret,
}

pub fn secret_key(profile_id: &str, kind: SecretKind) -> String {
    let suffix = match kind {
        SecretKind::DatabasePassword => "db",
        SecretKind::SshSecret => "ssh",
    };
    format!("{profile_id}:{suffix}")
}

/// Where saved passwords live. The real store is the macOS Keychain, so the
/// operating system does the encryption and nothing secret is written to our files.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> AppResult<Option<String>>;
    fn set(&self, key: &str, value: &str) -> AppResult<()>;
    fn delete(&self, key: &str) -> AppResult<()>;
}

pub struct KeychainStore;

impl KeychainStore {
    fn entry(key: &str) -> AppResult<keyring::Entry> {
        keyring::Entry::new(KEYCHAIN_SERVICE, key).map_err(to_secret_error)
    }
}

fn to_secret_error(error: keyring::Error) -> AppError {
    AppError::Secret(error.to_string())
}

impl SecretStore for KeychainStore {
    fn get(&self, key: &str) -> AppResult<Option<String>> {
        match Self::entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(to_secret_error(error)),
        }
    }

    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        Self::entry(key)?
            .set_password(value)
            .map_err(to_secret_error)
    }

    fn delete(&self, key: &str) -> AppResult<()> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(to_secret_error(error)),
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct MemoryStore {
        values: Mutex<HashMap<String, String>>,
    }

    impl SecretStore for MemoryStore {
        fn get(&self, key: &str) -> AppResult<Option<String>> {
            Ok(self.values.lock().unwrap().get(key).cloned())
        }

        fn set(&self, key: &str, value: &str) -> AppResult<()> {
            self.values.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }

        fn delete(&self, key: &str) -> AppResult<()> {
            self.values.lock().unwrap().remove(key);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::MemoryStore;
    use super::*;

    #[test]
    fn keys_are_distinct_per_profile_and_kind() {
        assert_eq!(secret_key("p1", SecretKind::DatabasePassword), "p1:db");
        assert_eq!(secret_key("p1", SecretKind::SshSecret), "p1:ssh");
    }

    #[test]
    fn memory_store_round_trips_and_deleting_twice_is_fine() {
        let store = MemoryStore::default();

        store.set("p1:db", "hunter2").unwrap();
        assert_eq!(store.get("p1:db").unwrap().as_deref(), Some("hunter2"));

        store.delete("p1:db").unwrap();
        store.delete("p1:db").unwrap();
        assert_eq!(store.get("p1:db").unwrap(), None);
    }
}
