use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::{AppError, AppResult};

/// Running exports and imports, each with a flag the user can raise to stop it.
#[derive(Default)]
pub struct JobRegistry {
    jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl JobRegistry {
    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>> {
        self.jobs
            .lock()
            .map_err(|_| AppError::Storage("job registry lock is poisoned".into()))
    }

    pub fn start(&self, id: &str) -> AppResult<Arc<AtomicBool>> {
        let mut jobs = self.lock()?;
        if jobs.contains_key(id) {
            return Err(AppError::Validation(format!("Job {id} is already running")));
        }
        let flag = Arc::new(AtomicBool::new(false));
        jobs.insert(id.to_owned(), Arc::clone(&flag));
        Ok(flag)
    }

    /// True when a running job was asked to stop.
    pub fn cancel(&self, id: &str) -> AppResult<bool> {
        Ok(self
            .lock()?
            .get(id)
            .map(|flag| flag.store(true, Ordering::SeqCst))
            .is_some())
    }

    pub fn finish(&self, id: &str) {
        if let Ok(mut jobs) = self.lock() {
            jobs.remove(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_job_can_be_started_cancelled_and_finished() {
        let registry = JobRegistry::default();

        let flag = registry.start("a").unwrap();
        assert!(!flag.load(Ordering::SeqCst));
        assert!(registry.cancel("a").unwrap());
        assert!(flag.load(Ordering::SeqCst));
        registry.finish("a");

        assert!(!registry.cancel("a").unwrap());
        assert!(registry.start("a").is_ok());
    }

    #[test]
    fn the_same_job_cannot_run_twice() {
        let registry = JobRegistry::default();
        registry.start("a").unwrap();

        assert!(matches!(registry.start("a"), Err(AppError::Validation(_))));
    }
}
