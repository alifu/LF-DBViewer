//! Local port forward through an SSH server: the MySQL driver connects to a
//! loopback port and every accepted socket is piped over a `direct-tcpip` channel.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::Disconnect;
use russh::client::{self, AuthResult, Handle};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::AgentClient;
use russh::keys::{self, HashAlg, PrivateKeyWithHashAlg, PublicKey};
use tokio::net::TcpListener;
use tokio::task::JoinHandle;

use super::model::{SshAuth, SshConfig};
use crate::error::{AppError, AppResult};

const LOOPBACK: &str = "127.0.0.1";
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);

/// Why the host key check refused the server, kept so the caller can explain it.
enum HostKeyRejection {
    Unknown { fingerprint: String },
    Changed { line: usize },
    Unsupported,
}

struct HostKeyVerifier {
    host: String,
    port: u16,
    trust_unknown: bool,
    rejection: Arc<Mutex<Option<HostKeyRejection>>>,
}

impl HostKeyVerifier {
    fn reject(&self, rejection: HostKeyRejection) -> Result<bool, russh::Error> {
        if let Ok(mut slot) = self.rejection.lock() {
            *slot = Some(rejection);
        }
        Ok(false)
    }

    fn verify(&self, key: &PublicKey) -> Result<bool, russh::Error> {
        match keys::check_known_hosts(&self.host, self.port, key) {
            Ok(true) => Ok(true),
            Err(keys::Error::KeyChanged { line }) => {
                self.reject(HostKeyRejection::Changed { line })
            }
            // Not listed, or there is no known_hosts file yet.
            Ok(false) | Err(_) if self.trust_unknown => {
                keys::known_hosts::learn_known_hosts(&self.host, self.port, key)?;
                Ok(true)
            }
            Ok(false) | Err(_) => self.reject(HostKeyRejection::Unknown {
                fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
            }),
        }
    }
}

impl client::Handler for HostKeyVerifier {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        match server_public_key {
            keys::PublicKeyOrCertificate::PublicKey { key, .. } => self.verify(key),
            keys::PublicKeyOrCertificate::Certificate(_) => {
                self.reject(HostKeyRejection::Unsupported)
            }
        }
    }
}

pub struct SshTunnel {
    local_port: u16,
    session: Arc<Handle<HostKeyVerifier>>,
    forwarder: JoinHandle<()>,
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

fn require_success(result: AuthResult, method: &str) -> AppResult<()> {
    match result {
        AuthResult::Success => Ok(()),
        AuthResult::Failure { .. } => Err(AppError::Ssh(format!(
            "the server rejected {method} authentication"
        ))),
    }
}

async fn authenticate_with_agent(
    session: &mut Handle<HostKeyVerifier>,
    user: &str,
    hash: Option<HashAlg>,
) -> AppResult<()> {
    let mut agent = AgentClient::connect_env().await?;
    let identities = agent.request_identities().await?;
    if identities.is_empty() {
        return Err(AppError::Ssh("the SSH agent has no keys loaded".into()));
    }
    for identity in identities {
        let AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        let result = session
            .authenticate_publickey_with(user, key, hash, &mut agent)
            .await
            .map_err(|error| AppError::Ssh(error.to_string()))?;
        if matches!(result, AuthResult::Success) {
            return Ok(());
        }
    }
    Err(AppError::Ssh(
        "the server accepted none of the SSH agent's keys".into(),
    ))
}

async fn authenticate(
    session: &mut Handle<HostKeyVerifier>,
    config: &SshConfig,
    secret: Option<&str>,
) -> AppResult<()> {
    match &config.auth {
        SshAuth::Password => {
            let password =
                secret.ok_or_else(|| AppError::Validation("SSH password is required".into()))?;
            let result = session
                .authenticate_password(&config.user, password)
                .await?;
            require_success(result, "password")
        }
        SshAuth::KeyFile { path } => {
            let key = keys::load_secret_key(expand_home(path), secret.filter(|s| !s.is_empty()))?;
            let hash = session.best_supported_rsa_hash().await?.flatten();
            let result = session
                .authenticate_publickey(
                    &config.user,
                    PrivateKeyWithHashAlg::new(Arc::new(key), hash),
                )
                .await?;
            require_success(result, "key")
        }
        SshAuth::Agent => {
            let hash = session.best_supported_rsa_hash().await?.flatten();
            authenticate_with_agent(session, &config.user, hash).await
        }
    }
}

/// Accepts loopback connections and pipes each one to `target` over the SSH session.
/// A failed channel just drops the socket; the database driver reports that as a
/// connection error to whoever was using it.
async fn forward(
    listener: TcpListener,
    session: Arc<Handle<HostKeyVerifier>>,
    target: (String, u16),
) {
    while let Ok((mut socket, _)) = listener.accept().await {
        let session = Arc::clone(&session);
        let (host, port) = target.clone();
        tokio::spawn(async move {
            let Ok(channel) = session
                .channel_open_direct_tcpip(host, u32::from(port), LOOPBACK, 0)
                .await
            else {
                return;
            };
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
        });
    }
}

impl SshTunnel {
    /// Connects, checks the host key against `~/.ssh/known_hosts`, authenticates and
    /// starts forwarding. An unknown host is refused unless `trust_unknown_host` is
    /// set, in which case its key is recorded in known_hosts.
    pub async fn open(
        config: &SshConfig,
        secret: Option<&str>,
        target: (String, u16),
        trust_unknown_host: bool,
    ) -> AppResult<Self> {
        let rejection = Arc::new(Mutex::new(None));
        let verifier = HostKeyVerifier {
            host: config.host.clone(),
            port: config.port,
            trust_unknown: trust_unknown_host,
            rejection: Arc::clone(&rejection),
        };
        let client_config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE_INTERVAL),
            ..client::Config::default()
        });

        let connected =
            client::connect(client_config, (config.host.as_str(), config.port), verifier).await;
        let mut session = match connected {
            Ok(session) => session,
            Err(error) => {
                let rejected = rejection.lock().ok().and_then(|mut slot| slot.take());
                return Err(Self::connect_error(config, rejected, error));
            }
        };

        if let Err(error) = authenticate(&mut session, config, secret).await {
            let _ = session
                .disconnect(Disconnect::ByApplication, "", "en")
                .await;
            return Err(error);
        }

        let listener = TcpListener::bind((LOOPBACK, 0)).await?;
        let local_port = listener.local_addr()?.port();
        let session = Arc::new(session);
        let forwarder = tokio::spawn(forward(listener, Arc::clone(&session), target));

        Ok(Self {
            local_port,
            session,
            forwarder,
        })
    }

    fn connect_error(
        config: &SshConfig,
        rejected: Option<HostKeyRejection>,
        error: russh::Error,
    ) -> AppError {
        let (host, port) = (config.host.clone(), config.port);
        match rejected {
            Some(HostKeyRejection::Unknown { fingerprint }) => AppError::SshUnknownHost {
                host,
                port,
                fingerprint,
            },
            Some(HostKeyRejection::Changed { line }) => {
                AppError::SshHostKeyChanged { host, port, line }
            }
            Some(HostKeyRejection::Unsupported) => {
                AppError::Ssh("host certificates are not supported yet".into())
            }
            None => AppError::Ssh(format!("could not connect to {host}:{port}: {error}")),
        }
    }

    pub fn local_port(&self) -> u16 {
        self.local_port
    }

    pub fn is_alive(&self) -> bool {
        !self.session.is_closed() && !self.forwarder.is_finished()
    }

    pub async fn close(&self) {
        self.forwarder.abort();
        let _ = self
            .session
            .disconnect(Disconnect::ByApplication, "", "en")
            .await;
    }
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.forwarder.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_a_leading_tilde_only() {
        let home = std::env::var("HOME").unwrap();

        assert_eq!(
            expand_home("~/.ssh/id_ed25519"),
            PathBuf::from(&home).join(".ssh/id_ed25519")
        );
        assert_eq!(expand_home("/keys/id_rsa"), PathBuf::from("/keys/id_rsa"));
    }

    #[test]
    fn rejected_host_keys_become_specific_errors() {
        let config = SshConfig {
            host: "bastion".into(),
            port: 22,
            user: "deploy".into(),
            auth: SshAuth::Agent,
        };

        let unknown = SshTunnel::connect_error(
            &config,
            Some(HostKeyRejection::Unknown {
                fingerprint: "SHA256:abc".into(),
            }),
            russh::Error::UnknownKey,
        );
        let changed = SshTunnel::connect_error(
            &config,
            Some(HostKeyRejection::Changed { line: 7 }),
            russh::Error::UnknownKey,
        );

        assert_eq!(unknown.code(), "ssh_unknown_host");
        assert_eq!(changed.code(), "ssh_host_key_changed");
    }

    #[tokio::test]
    async fn refused_ssh_port_is_reported_as_an_ssh_error() {
        let listener = TcpListener::bind((LOOPBACK, 0)).await.unwrap();
        let closed_port = listener.local_addr().unwrap().port();
        drop(listener);
        let config = SshConfig {
            host: LOOPBACK.into(),
            port: closed_port,
            user: "nobody".into(),
            auth: SshAuth::Agent,
        };

        let result = SshTunnel::open(&config, None, ("db".into(), 3306), false).await;

        assert!(matches!(result, Err(AppError::Ssh(_))));
    }
}
