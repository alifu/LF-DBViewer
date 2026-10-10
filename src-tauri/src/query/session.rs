//! A query tab's own database connection, so `USE`, `SET @x` and open transactions
//! carry from one run to the next.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use mysql_async::Conn;
use tokio::sync::{Mutex, RwLock};

use crate::error::{AppError, AppResult};

pub struct QuerySession {
    pub connection_id: String,
    /// `None` until the first run, and again after the connection broke.
    pub conn: Mutex<Option<Conn>>,
    /// Server thread running the current statement, 0 when idle. Readable while `conn` is locked.
    pub thread_id: AtomicU32,
    pub cancel_requested: AtomicBool,
}

impl QuerySession {
    fn new(connection_id: &str) -> Self {
        Self {
            connection_id: connection_id.to_owned(),
            conn: Mutex::new(None),
            thread_id: AtomicU32::new(0),
            cancel_requested: AtomicBool::new(false),
        }
    }
}

#[derive(Default)]
pub struct SessionManager {
    sessions: RwLock<HashMap<String, Arc<QuerySession>>>,
}

impl SessionManager {
    pub async fn open(&self, connection_id: &str) -> String {
        let id = uuid::Uuid::new_v4().to_string();
        self.sessions
            .write()
            .await
            .insert(id.clone(), Arc::new(QuerySession::new(connection_id)));
        id
    }

    pub async fn get(&self, id: &str) -> AppResult<Arc<QuerySession>> {
        self.sessions
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::Validation(format!("Query session is not open: {id}")))
    }

    pub async fn close(&self, id: &str) {
        let removed = self.sessions.write().await.remove(id);
        if let Some(session) = removed {
            Self::shut_down(&session).await;
        }
    }

    pub async fn close_for_connection(&self, connection_id: &str) {
        let doomed: Vec<Arc<QuerySession>> = {
            let mut sessions = self.sessions.write().await;
            let ids: Vec<String> = sessions
                .iter()
                .filter(|(_, session)| session.connection_id == connection_id)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter().filter_map(|id| sessions.remove(id)).collect()
        };
        for session in doomed {
            Self::shut_down(&session).await;
        }
    }

    /// A running statement keeps its connection until it ends; dropping it closes the socket then.
    async fn shut_down(session: &QuerySession) {
        session.cancel_requested.store(true, Ordering::SeqCst);
        if let Ok(mut guard) = session.conn.try_lock()
            && let Some(conn) = guard.take()
        {
            // The server drops the session when the socket closes, so a failed goodbye is harmless.
            let _ = conn.disconnect().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sessions_can_be_opened_found_and_closed() {
        let manager = SessionManager::default();

        let id = manager.open("c1").await;
        let found = manager.get(&id).await.unwrap();
        manager.close(&id).await;

        assert_eq!(found.connection_id, "c1");
        assert!(matches!(
            manager.get(&id).await,
            Err(AppError::Validation(_))
        ));
    }

    #[tokio::test]
    async fn closing_a_connection_closes_only_its_sessions() {
        let manager = SessionManager::default();
        let mine = manager.open("c1").await;
        let other = manager.open("c2").await;

        manager.close_for_connection("c1").await;

        assert!(manager.get(&mine).await.is_err());
        assert!(manager.get(&other).await.is_ok());
    }

    #[tokio::test]
    async fn closing_a_session_asks_a_running_statement_to_stop() {
        let manager = SessionManager::default();
        let id = manager.open("c1").await;
        let session = manager.get(&id).await.unwrap();

        manager.close(&id).await;

        assert!(session.cancel_requested.load(Ordering::SeqCst));
    }
}
