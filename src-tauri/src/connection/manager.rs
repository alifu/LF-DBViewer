use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mysql_async::prelude::Queryable;
use mysql_async::{Conn, Opts, OptsBuilder, Pool, PoolConstraints, PoolOpts, SslOpts};
use tokio::sync::RwLock;

use super::model::{
    ConnectionInfo, ConnectionProfile, ConnectionSecrets, ConnectionStatus, Engine, SslMode,
};
use super::ssh_tunnel::SshTunnel;
use crate::error::{AppError, AppResult};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const PING_TIMEOUT: Duration = Duration::from_secs(5);
const POOL_MIN_CONNECTIONS: usize = 0;
const POOL_MAX_CONNECTIONS: usize = 8;
const LOOPBACK: &str = "127.0.0.1";

/// One open server connection: a small pool, plus the SSH tunnel it runs through, if any.
pub struct ActiveConnection {
    info: ConnectionInfo,
    profile: ConnectionProfile,
    secrets: ConnectionSecrets,
    opts: Opts,
    pool: Pool,
    tunnel: Option<SshTunnel>,
}

impl ActiveConnection {
    pub async fn conn(&self) -> AppResult<Conn> {
        Ok(self.pool.get_conn().await?)
    }

    pub fn engine(&self) -> Engine {
        self.info.engine
    }

    /// A connection of its own, outside the pool, for a query tab's session.
    pub async fn open_session_conn(&self) -> AppResult<Conn> {
        let connect = async { Ok(Conn::new(self.opts.clone()).await?) };
        within(CONNECT_TIMEOUT, "opening a query session", connect).await
    }

    /// Stops the statement a session is running, from a different connection.
    pub async fn kill_query(&self, thread_id: u32) -> AppResult<()> {
        let mut conn = self.conn().await?;
        conn.query_drop(format!("KILL QUERY {thread_id}")).await?;
        Ok(())
    }

    async fn shut_down(&self) {
        // Best effort: the server drops the sessions anyway once the sockets close.
        let _ = self.pool.clone().disconnect().await;
        if let Some(tunnel) = &self.tunnel {
            tunnel.close().await;
        }
    }
}

fn ssl_opts(profile: &ConnectionProfile) -> Option<SslOpts> {
    let base = SslOpts::default();
    let with_ca = match &profile.ssl_ca {
        Some(path) => base.with_root_certs(vec![PathBuf::from(path).into()]),
        None => base,
    };
    match profile.ssl_mode {
        SslMode::Disabled => None,
        SslMode::Required => Some(
            with_ca
                .with_danger_accept_invalid_certs(true)
                .with_danger_skip_domain_validation(true),
        ),
        SslMode::VerifyCa => Some(with_ca.with_danger_skip_domain_validation(true)),
        SslMode::VerifyIdentity => Some(with_ca),
    }
}

fn pool_opts() -> AppResult<PoolOpts> {
    let constraints = PoolConstraints::new(POOL_MIN_CONNECTIONS, POOL_MAX_CONNECTIONS)
        .ok_or_else(|| AppError::Validation("invalid connection pool size".into()))?;
    Ok(PoolOpts::default().with_constraints(constraints))
}

fn driver_opts(
    profile: &ConnectionProfile,
    password: Option<&str>,
    tunnel_port: Option<u16>,
) -> AppResult<OptsBuilder> {
    let (host, port) = match tunnel_port {
        Some(local_port) => (LOOPBACK, local_port),
        None => (profile.host.as_str(), profile.port),
    };
    Ok(OptsBuilder::default()
        .ip_or_hostname(host)
        .tcp_port(port)
        .user(Some(profile.user.as_str()))
        .pass(password)
        .db_name(profile.database.as_deref())
        .socket(profile.socket.as_deref())
        .prefer_socket(profile.socket.is_some())
        .ssl_opts(ssl_opts(profile))
        // Report rows matched rather than rows changed, so an edit that sets a value to
        // what it already is still counts as having found its row.
        .client_found_rows(true)
        .pool_opts(pool_opts()?))
}

async fn within<T>(
    limit: Duration,
    action: &str,
    work: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    tokio::time::timeout(limit, work).await.unwrap_or_else(|_| {
        Err(AppError::Timeout {
            action: action.to_owned(),
            seconds: limit.as_secs(),
        })
    })
}

async fn server_version(pool: &Pool) -> AppResult<String> {
    let mut conn = pool.get_conn().await?;
    let version: Option<String> = conn.query_first("SELECT VERSION()").await?;
    Ok(version.unwrap_or_default())
}

async fn establish(
    id: String,
    profile: ConnectionProfile,
    secrets: ConnectionSecrets,
    trust_ssh_host: bool,
) -> AppResult<ActiveConnection> {
    let tunnel = match &profile.ssh {
        Some(ssh) => {
            let target = (profile.host.clone(), profile.port);
            let open = SshTunnel::open(ssh, secrets.ssh_secret.as_deref(), target, trust_ssh_host);
            Some(within(CONNECT_TIMEOUT, "opening the SSH tunnel", open).await?)
        }
        None => None,
    };

    let tunnel_port = tunnel.as_ref().map(SshTunnel::local_port);
    let opts: Opts = driver_opts(&profile, secrets.password.as_deref(), tunnel_port)?.into();
    let pool = Pool::new(opts.clone());
    let version = match within(
        CONNECT_TIMEOUT,
        "connecting to the database",
        server_version(&pool),
    )
    .await
    {
        Ok(version) => version,
        Err(error) => {
            let _ = pool.disconnect().await;
            if let Some(tunnel) = &tunnel {
                tunnel.close().await;
            }
            return Err(error);
        }
    };

    let info = ConnectionInfo {
        id,
        profile_id: profile.id.clone(),
        name: profile.name.clone(),
        engine: ConnectionProfile::engine_for_version(&version),
        server_version: version,
        host: profile.host.clone(),
        port: profile.port,
        user: profile.user.clone(),
        database: profile.database.clone(),
        via_ssh: profile.ssh.is_some(),
    };
    Ok(ActiveConnection {
        info,
        profile,
        secrets,
        opts,
        pool,
        tunnel,
    })
}

#[derive(Default)]
pub struct ConnectionManager {
    connections: RwLock<HashMap<String, Arc<ActiveConnection>>>,
}

impl ConnectionManager {
    pub async fn open(
        &self,
        profile: ConnectionProfile,
        secrets: ConnectionSecrets,
        trust_ssh_host: bool,
    ) -> AppResult<ConnectionInfo> {
        let id = uuid::Uuid::new_v4().to_string();
        let connection =
            establish(id.clone(), profile.validated()?, secrets, trust_ssh_host).await?;
        let info = connection.info.clone();
        self.connections
            .write()
            .await
            .insert(id, Arc::new(connection));
        Ok(info)
    }

    /// Connects and immediately disconnects, returning what the server reported.
    pub async fn test(
        &self,
        profile: ConnectionProfile,
        secrets: ConnectionSecrets,
        trust_ssh_host: bool,
    ) -> AppResult<ConnectionInfo> {
        let connection =
            establish(String::new(), profile.validated()?, secrets, trust_ssh_host).await?;
        connection.shut_down().await;
        Ok(connection.info.clone())
    }

    pub async fn get(&self, id: &str) -> AppResult<Arc<ActiveConnection>> {
        self.connections
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| AppError::ConnectionNotFound(id.to_owned()))
    }

    pub async fn list(&self) -> Vec<ConnectionInfo> {
        self.connections
            .read()
            .await
            .values()
            .map(|connection| connection.info.clone())
            .collect()
    }

    pub async fn close(&self, id: &str) -> AppResult<()> {
        let removed = self.connections.write().await.remove(id);
        let connection = removed.ok_or_else(|| AppError::ConnectionNotFound(id.to_owned()))?;
        connection.shut_down().await;
        Ok(())
    }

    /// Never fails for a known connection: a dead server is a status, not an error.
    pub async fn ping(&self, id: &str) -> AppResult<ConnectionStatus> {
        let connection = self.get(id).await?;
        if connection
            .tunnel
            .as_ref()
            .is_some_and(|tunnel| !tunnel.is_alive())
        {
            return Ok(ConnectionStatus {
                connected: false,
                latency_ms: None,
                error: Some("the SSH tunnel is closed".into()),
            });
        }

        let started = Instant::now();
        let ping = async {
            let mut conn = connection.conn().await?;
            conn.ping().await?;
            Ok(())
        };
        Ok(
            match within(PING_TIMEOUT, "pinging the server", ping).await {
                Ok(()) => ConnectionStatus {
                    connected: true,
                    latency_ms: Some(
                        u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    ),
                    error: None,
                },
                Err(error) => ConnectionStatus {
                    connected: false,
                    latency_ms: None,
                    error: Some(error.to_string()),
                },
            },
        )
    }

    /// Rebuilds the tunnel and pool under the same connection id, so open tabs keep working.
    pub async fn reconnect(&self, id: &str) -> AppResult<ConnectionInfo> {
        let previous = self.get(id).await?;
        let replacement = establish(
            id.to_owned(),
            previous.profile.clone(),
            previous.secrets.clone(),
            false,
        )
        .await?;
        let info = replacement.info.clone();
        self.connections
            .write()
            .await
            .insert(id.to_owned(), Arc::new(replacement));
        previous.shut_down().await;
        Ok(info)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::model::sample_profile;

    #[test]
    fn driver_targets_the_tunnel_port_when_there_is_one() {
        let direct = driver_opts(&sample_profile(), None, None).unwrap();
        let tunnelled = driver_opts(&sample_profile(), None, Some(54321)).unwrap();

        assert_eq!(mysql_async::Opts::from(direct).tcp_port(), 3306);
        let tunnelled = mysql_async::Opts::from(tunnelled);
        assert_eq!(tunnelled.tcp_port(), 54321);
        assert_eq!(tunnelled.ip_or_hostname(), LOOPBACK);
    }

    #[test]
    fn ssl_is_off_only_when_disabled() {
        let required = ConnectionProfile {
            ssl_mode: SslMode::Required,
            ..sample_profile()
        };

        assert!(ssl_opts(&sample_profile()).is_none());
        assert!(ssl_opts(&required).is_some());
    }

    #[tokio::test]
    async fn unknown_connection_ids_are_reported() {
        let manager = ConnectionManager::default();

        assert!(matches!(
            manager.get("nope").await,
            Err(AppError::ConnectionNotFound(_))
        ));
        assert!(matches!(
            manager.ping("nope").await,
            Err(AppError::ConnectionNotFound(_))
        ));
        assert!(matches!(
            manager.close("nope").await,
            Err(AppError::ConnectionNotFound(_))
        ));
    }

    #[tokio::test]
    async fn invalid_profile_is_rejected_before_any_network_use() {
        let manager = ConnectionManager::default();
        let profile = ConnectionProfile {
            user: "".into(),
            ..sample_profile()
        };

        let result = manager
            .open(profile, ConnectionSecrets::default(), false)
            .await;

        assert!(matches!(result, Err(AppError::Validation(_))));
        assert!(manager.list().await.is_empty());
    }

    /// Runs only when a server is supplied, e.g.
    /// `LFDB_TEST_HOST=127.0.0.1 LFDB_TEST_USER=root cargo test -- --ignored`.
    #[tokio::test]
    #[ignore = "needs a live MySQL/MariaDB server (LFDB_TEST_* variables)"]
    async fn opens_pings_reconnects_and_closes_against_a_live_server() {
        let env = |name: &str| std::env::var(name).ok();
        let profile = ConnectionProfile {
            host: env("LFDB_TEST_HOST").unwrap_or_else(|| LOOPBACK.into()),
            port: env("LFDB_TEST_PORT")
                .and_then(|port| port.parse().ok())
                .unwrap_or(3306),
            user: env("LFDB_TEST_USER").unwrap_or_else(|| "root".into()),
            ..sample_profile()
        };
        let secrets = ConnectionSecrets {
            password: env("LFDB_TEST_PASSWORD"),
            ssh_secret: None,
        };
        let manager = ConnectionManager::default();

        let opened = manager.open(profile, secrets, false).await.unwrap();
        let status = manager.ping(&opened.id).await.unwrap();
        let reconnected = manager.reconnect(&opened.id).await.unwrap();
        manager.close(&opened.id).await.unwrap();

        assert!(!opened.server_version.is_empty());
        assert!(status.connected);
        assert_eq!(reconnected.id, opened.id);
        assert!(manager.list().await.is_empty());
    }

    #[tokio::test]
    async fn refused_database_port_is_a_database_error() {
        let listener = tokio::net::TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let closed_port = listener.local_addr().unwrap().port();
        drop(listener);
        let manager = ConnectionManager::default();
        let profile = ConnectionProfile {
            port: closed_port,
            ..sample_profile()
        };

        let result = manager
            .test(profile, ConnectionSecrets::default(), false)
            .await;

        assert!(matches!(result, Err(AppError::Database(_))));
    }
}
