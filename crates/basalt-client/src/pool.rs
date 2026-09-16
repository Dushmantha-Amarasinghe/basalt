//! A small pool of connections to one host.
//!
//! The reason this exists rather than a single shared connection: the protocol
//! is strictly request-then-response, so a 2 GB download would own the
//! connection for a minute and a half and the folder list would sit there
//! waiting. Opening a second connection costs nothing — Phase 0 measured 1
//! stream at 20.8 MB/s and 16 at 24.1, essentially flat — so the fix for
//! head-of-line blocking is simply not to share the line.
//!
//! Four is the cap: one for browsing, one for the transfer in progress, and
//! two spare for the media player's range requests, which want to seek while
//! something else is running.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use crate::session::Session;
use crate::{ClientError, Result};

const MAX_IDLE: usize = 4;

struct Inner {
    addr: SocketAddr,
    host_id: String,
    token: String,
    device_name: String,
    idle: Mutex<Vec<Session>>,
}

/// Connections to one paired host.
#[derive(Clone)]
pub struct Pool {
    inner: Arc<Inner>,
}

impl Pool {
    pub fn new(addr: SocketAddr, host_id: &str, token: &str, device_name: &str) -> Self {
        Self {
            inner: Arc::new(Inner {
                addr,
                host_id: host_id.to_string(),
                token: token.to_string(),
                device_name: device_name.to_string(),
                idle: Mutex::new(Vec::new()),
            }),
        }
    }

    /// Adopts a session that already exists, such as the one pairing opened.
    pub fn with_session(
        addr: SocketAddr,
        host_id: &str,
        token: &str,
        device_name: &str,
        session: Session,
    ) -> Self {
        let pool = Self::new(addr, host_id, token, device_name);
        pool.inner.idle.lock().expect("idle lock").push(session);
        pool
    }

    pub fn address(&self) -> SocketAddr {
        self.inner.addr
    }

    pub fn host_id(&self) -> &str {
        &self.inner.host_id
    }

    pub fn idle_count(&self) -> usize {
        self.inner.idle.lock().expect("idle lock").len()
    }

    /// Takes a connection, opening one if none is idle.
    pub async fn acquire(&self) -> Result<Lease> {
        // Never hold the lock across an await: `acquire` is called from every
        // task in the app and blocking the executor on a mutex here would stall
        // everything, including the connection being waited on.
        let pooled = self.inner.idle.lock().expect("idle lock").pop();

        let session = match pooled {
            Some(session) => session,
            None => {
                Session::connect(
                    self.inner.addr,
                    &self.inner.host_id,
                    &self.inner.token,
                    &self.inner.device_name,
                )
                .await?
            }
        };

        Ok(Lease {
            session: Some(session),
            inner: Arc::clone(&self.inner),
            healthy: true,
        })
    }

    /// Drops every idle connection, for example after the host goes away.
    pub fn clear(&self) {
        self.inner.idle.lock().expect("idle lock").clear();
    }
}

/// A borrowed connection, returned to the pool when it goes out of scope.
pub struct Lease {
    session: Option<Session>,
    inner: Arc<Inner>,
    healthy: bool,
}

impl Lease {
    /// Marks this connection as not worth reusing.
    ///
    /// Called after any transport failure. Returning a connection that has
    /// already failed would hand the next caller a broken one and turn a single
    /// dropped Wi-Fi packet into a cascade of unrelated errors.
    pub fn discard(&mut self) {
        self.healthy = false;
    }

    /// Runs an operation, discarding the connection if the transport failed.
    ///
    /// The distinction matters: "the file is not there" leaves a perfectly good
    /// connection, while "the socket reset" does not.
    pub fn check<T>(&mut self, result: Result<T>) -> Result<T> {
        if let Err(e) = &result
            && e.is_transient()
        {
            self.discard();
        }
        result
    }
}

impl std::ops::Deref for Lease {
    type Target = Session;
    fn deref(&self) -> &Session {
        self.session
            .as_ref()
            .expect("a lease always holds a session")
    }
}

impl std::ops::DerefMut for Lease {
    fn deref_mut(&mut self) -> &mut Session {
        self.session
            .as_mut()
            .expect("a lease always holds a session")
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        if !self.healthy {
            return;
        }
        let mut idle = self.inner.idle.lock().expect("idle lock");
        if idle.len() < MAX_IDLE {
            idle.push(session);
        }
        // Over the cap the session is simply dropped, which closes it.
    }
}

impl ClientError {
    /// Whether a fresh connection could plausibly succeed where this failed.
    pub fn is_transient(&self) -> bool {
        match self {
            ClientError::Io(_) => true,
            ClientError::Net(e) => e.is_transient(),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> SocketAddr {
        "127.0.0.1:1".parse().unwrap()
    }

    #[test]
    fn a_new_pool_holds_nothing() {
        let pool = Pool::new(addr(), "aa", "token", "Laptop A");
        assert_eq!(pool.idle_count(), 0);
        assert_eq!(pool.host_id(), "aa");
        assert_eq!(pool.address(), addr());
    }

    #[tokio::test]
    async fn acquiring_against_a_dead_host_fails_rather_than_hanging() {
        // Port 1 has nothing on it, so this is a connection refusal.
        let pool = Pool::new(addr(), "aa", "token", "Laptop A");
        let Err(err) = pool.acquire().await else {
            panic!("nothing is listening on port 1");
        };
        assert!(err.is_transient(), "a refused connection is worth retrying");
        assert_eq!(pool.idle_count(), 0);
    }

    #[test]
    fn clearing_empties_the_pool() {
        let pool = Pool::new(addr(), "aa", "token", "Laptop A");
        pool.clear();
        assert_eq!(pool.idle_count(), 0);
    }

    #[test]
    fn transport_failures_are_transient_and_refusals_are_not() {
        assert!(
            ClientError::Io(std::io::Error::from(std::io::ErrorKind::ConnectionReset))
                .is_transient()
        );
        assert!(
            !ClientError::PairingClosed.is_transient(),
            "a closed pairing window will still be closed on a new connection"
        );
        assert!(!ClientError::BadPin("nope".into()).is_transient());
        assert!(
            !ClientError::Net(basalt_net::NetError::Remote(basalt_proto::WireError::new(
                basalt_proto::ErrorCode::NotFound,
                "gone"
            )))
            .is_transient()
        );
    }
}
