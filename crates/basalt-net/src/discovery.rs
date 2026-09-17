//! Finding hosts on the local network.
//!
//! The point of this module is that **an address is never something a person
//! types, and never something that is remembered.** What a client remembers is
//! the host's public key. Reconnecting means asking the network who is there
//! and connecting to whichever machine presents the key that was pinned. The
//! router can hand out a different address every day and nothing notices.
//!
//! That also settles how much this protocol has to be trusted, which is: not at
//! all. Anything on the network can send a reply claiming to be a Basalt host,
//! and it will fail the TLS pin check a moment later. Discovery is a hint about
//! **where to look**, never a statement about **who to trust**. Nothing secret
//! travels here — the host id is a public key hash, published deliberately.
//!
//! ```text
//! query   [magic "BSLTd"][version u8][nonce u64 LE]
//! reply   [magic "BSLTd"][version u8][nonce u64 LE][JSON]
//! ```
//!
//! Hosts answer queries *and* announce themselves unprompted every few seconds.
//! A machine with virtual adapters — VMware, Hyper-V, WSL — does not always
//! route a broadcast the way you would hope, so the client both asks and
//! listens.
//!
//! Written rather than using mDNS: mDNS means a dependency, a responder
//! competing with the one Windows already runs, and a great deal of protocol
//! for the question "who is out there". This is the same reasoning that
//! produced the file protocol.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::net::UdpSocket;

use crate::{NetError, Result};

/// The discovery port. One above the file protocol's.
pub const DISCOVERY_PORT: u16 = 7743;

const MAGIC: &[u8; 5] = b"BSLTd";
const VERSION: u8 = 1;
const HEADER_BYTES: usize = 5 + 1 + 8;

/// Largest datagram accepted. A reply is a few hundred bytes of JSON.
const MAX_DATAGRAM: usize = 2048;

/// How often an idle host announces itself.
pub const ANNOUNCE_INTERVAL: Duration = Duration::from_secs(3);

/// How long a scan listens before giving up.
pub const SCAN_WINDOW: Duration = Duration::from_millis(900);

/// What a host says about itself.
///
/// Chosen to be exactly what a person needs in order to pick one from a list,
/// and nothing more.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Beacon {
    /// Hex SHA-256 of the host's SPKI. Its permanent identity, and the thing
    /// the client pins.
    pub host_id: String,
    /// The machine's name, for the list.
    pub host_name: String,
    /// What the drive is called.
    pub vault: String,
    /// The file protocol's port on this host.
    pub port: u16,
    /// Whether pairing will ask for a PIN.
    pub requires_pin: bool,
    /// False when the host is running but has not been given a drive yet.
    #[serde(default)]
    pub has_vault: bool,
}

/// A host that answered, and where it answered from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub beacon: Beacon,
    /// Where to connect. Learned fresh every scan, never stored.
    pub address: SocketAddr,
}

impl Found {
    /// The address to dial: where the datagram came from, with the port the
    /// host advertised.
    ///
    /// Taking the address from the payload instead would let a reply redirect
    /// the client to a machine it never heard from.
    fn from_reply(beacon: Beacon, from: SocketAddr) -> Self {
        let port = beacon.port;
        Self {
            beacon,
            address: SocketAddr::new(from.ip(), port),
        }
    }
}

fn encode(nonce: u64, body: Option<&Beacon>) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(HEADER_BYTES + 256);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&nonce.to_le_bytes());
    if let Some(beacon) = body {
        serde_json::to_writer(&mut out, beacon)
            .map_err(|e| NetError::Protocol(format!("encoding a beacon: {e}")))?;
    }
    Ok(out)
}

/// Splits a datagram into its nonce and body, rejecting anything foreign.
///
/// Returns `None` rather than an error for traffic that is simply not ours — a
/// broadcast port hears a great deal of unrelated noise, and none of it is
/// worth a log line.
fn decode(datagram: &[u8]) -> Option<(u64, &[u8])> {
    if datagram.len() < HEADER_BYTES {
        return None;
    }
    if &datagram[..5] != MAGIC || datagram[5] != VERSION {
        return None;
    }
    let nonce = u64::from_le_bytes(datagram[6..HEADER_BYTES].try_into().ok()?);
    Some((nonce, &datagram[HEADER_BYTES..]))
}

// ---------------------------------------------------------------------------
// Host side
// ---------------------------------------------------------------------------

/// Answers discovery queries and announces itself, until dropped.
///
/// The beacon is read through a closure rather than captured once, because the
/// vault can be renamed and the PIN requirement toggled while the host runs,
/// and an announcement carrying stale details would show the wrong thing in
/// somebody's list.
pub async fn respond<F>(describe: F) -> Result<()>
where
    F: Fn() -> Beacon + Send + 'static,
{
    let socket = bind_responder().await?;
    socket.set_broadcast(true).ok();

    let mut buf = vec![0u8; MAX_DATAGRAM];
    let mut announce = tokio::time::interval(ANNOUNCE_INTERVAL);

    loop {
        tokio::select! {
            received = socket.recv_from(&mut buf) => {
                let Ok((len, from)) = received else { continue };
                let Some((nonce, body)) = decode(&buf[..len]) else { continue };
                // Only queries get an answer. Without this a host would reply
                // to its own announcements echoing back off the network.
                if !body.is_empty() {
                    continue;
                }
                if let Ok(reply) = encode(nonce, Some(&describe())) {
                    let _ = socket.send_to(&reply, from).await;
                }
            }
            _ = announce.tick() => {
                // Unprompted, for the clients whose query never arrived.
                if let Ok(packet) = encode(0, Some(&describe())) {
                    for target in broadcast_targets() {
                        let _ = socket.send_to(&packet, target).await;
                    }
                }
            }
        }
    }
}

/// Binds the discovery port, tolerating another process already on it.
async fn bind_responder() -> Result<UdpSocket> {
    // `SO_REUSEADDR` so a host and a client can coexist on one machine, which
    // is exactly the situation during development and testing.
    let socket = socket2::Socket::new(
        socket2::Domain::IPV4,
        socket2::Type::DGRAM,
        Some(socket2::Protocol::UDP),
    )
    .map_err(NetError::Io)?;
    socket.set_reuse_address(true).map_err(NetError::Io)?;
    socket.set_nonblocking(true).map_err(NetError::Io)?;
    socket
        .bind(&SocketAddr::from((Ipv4Addr::UNSPECIFIED, DISCOVERY_PORT)).into())
        .map_err(NetError::Io)?;

    UdpSocket::from_std(socket.into()).map_err(NetError::Io)
}

// ---------------------------------------------------------------------------
// Client side
// ---------------------------------------------------------------------------

/// Asks who is out there, and collects the answers.
///
/// Always waits the full window rather than returning at the first reply: on a
/// network with two hosts, stopping early would show whichever answered fastest
/// and hide the other.
pub async fn scan(window: Duration) -> Result<Vec<Found>> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .await
        .map_err(NetError::Io)?;
    socket.set_broadcast(true).ok();

    let nonce = rand_nonce();
    let query = encode(nonce, None)?;
    for target in broadcast_targets() {
        let _ = socket.send_to(&query, target).await;
    }

    // Keyed by host id, so a host that both answers and announces inside the
    // window appears once.
    let mut found: HashMap<String, Found> = HashMap::new();
    let deadline = tokio::time::Instant::now() + window;
    let mut buf = vec![0u8; MAX_DATAGRAM];

    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        let Ok(Ok((len, from))) = tokio::time::timeout(remaining, socket.recv_from(&mut buf)).await
        else {
            break;
        };

        let Some((reply_nonce, body)) = decode(&buf[..len]) else {
            continue;
        };
        // Zero is an unprompted announcement; anything else must match the
        // query this scan sent, so a reply to somebody else's scan is ignored.
        if reply_nonce != 0 && reply_nonce != nonce {
            continue;
        }
        let Ok(beacon) = serde_json::from_slice::<Beacon>(body) else {
            continue;
        };
        found
            .entry(beacon.host_id.clone())
            .or_insert_with(|| Found::from_reply(beacon, from));
    }

    let mut hosts: Vec<Found> = found.into_values().collect();
    // Stable order, so a list on screen does not reshuffle between scans.
    hosts.sort_by(|a, b| a.beacon.host_name.cmp(&b.beacon.host_name));
    Ok(hosts)
}

/// Scans until a host with this id appears, or the deadline passes.
///
/// This is how reconnection works: the client knows the key it pinned, asks the
/// network where that key is today, and dials whatever answers. An address is
/// never stored and never goes stale.
pub async fn find_host(host_id: &str, timeout: Duration) -> Result<Option<Found>> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        let window = remaining.min(SCAN_WINDOW);
        if let Some(found) = scan(window)
            .await?
            .into_iter()
            .find(|f| f.beacon.host_id == host_id)
        {
            return Ok(Some(found));
        }
    }
}

// ---------------------------------------------------------------------------
// Shared
// ---------------------------------------------------------------------------

/// Every address worth broadcasting a query to.
///
/// The limited broadcast alone is not enough. A machine with a virtual adapter
/// — VMware, Hyper-V, WSL, all common — may route `255.255.255.255` out of the
/// wrong one, and the query never reaches the network the host is on. Sending
/// to each interface's own broadcast address as well costs a few datagrams and
/// removes the whole class of problem.
fn broadcast_targets() -> Vec<SocketAddr> {
    let mut targets = vec![SocketAddr::V4(SocketAddrV4::new(
        Ipv4Addr::BROADCAST,
        DISCOVERY_PORT,
    ))];

    if let Ok(interfaces) = if_addrs::get_if_addrs() {
        for interface in interfaces {
            if interface.is_loopback() {
                continue;
            }
            let if_addrs::IfAddr::V4(v4) = interface.addr else {
                continue;
            };
            let Some(broadcast) = v4.broadcast else {
                continue;
            };
            let target = SocketAddr::V4(SocketAddrV4::new(broadcast, DISCOVERY_PORT));
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
    }
    targets
}

/// This machine's non-loopback IPv4 addresses, for the host to display.
pub fn local_addresses() -> Vec<IpAddr> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    interfaces
        .into_iter()
        .filter(|i| !i.is_loopback())
        .filter_map(|i| match i.addr {
            if_addrs::IfAddr::V4(v4) => Some(IpAddr::V4(v4.ip)),
            _ => None,
        })
        // A link-local address means the adapter never got a lease; showing one
        // would send somebody chasing an address that cannot work.
        .filter(|ip| !matches!(ip, IpAddr::V4(v4) if v4.is_link_local()))
        .collect()
}

fn rand_nonce() -> u64 {
    // Never zero: zero is reserved to mean "unprompted announcement".
    loop {
        if let Ok(bytes) = crate::pairing::random_bytes(8) {
            let value = u64::from_le_bytes(bytes.try_into().expect("asked for 8 bytes"));
            if value != 0 {
                return value;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn beacon() -> Beacon {
        Beacon {
            host_id: "aabbccdd".into(),
            host_name: "laptop-b".into(),
            vault: "My Drive".into(),
            port: 7742,
            requires_pin: true,
            has_vault: true,
        }
    }

    #[test]
    fn a_query_round_trips() {
        let packet = encode(12345, None).unwrap();
        let (nonce, body) = decode(&packet).expect("our own packet must decode");
        assert_eq!(nonce, 12345);
        assert!(body.is_empty(), "a query carries no body");
    }

    #[test]
    fn a_reply_round_trips() {
        let packet = encode(999, Some(&beacon())).unwrap();
        let (nonce, body) = decode(&packet).unwrap();
        assert_eq!(nonce, 999);
        assert_eq!(serde_json::from_slice::<Beacon>(body).unwrap(), beacon());
    }

    // A broadcast port hears a great deal that is not ours.
    #[test]
    fn foreign_traffic_is_ignored_rather_than_misread() {
        assert!(decode(b"").is_none());
        assert!(decode(b"short").is_none());
        assert!(
            decode(b"NOPEx\x01\0\0\0\0\0\0\0\0").is_none(),
            "wrong magic"
        );

        let mut wrong_version = encode(1, None).unwrap();
        wrong_version[5] = 99;
        assert!(decode(&wrong_version).is_none());
    }

    /// A beacon is broadcast in the clear to anything listening, so every field
    /// on it is published to the whole network.
    ///
    /// Pinned as an exact list rather than a search for suspicious words —
    /// which was the first attempt, and it failed immediately because
    /// `requiresPin` contains "pin". Naming the fields means adding one is a
    /// deliberate act with a test to update, which is the point.
    #[test]
    fn a_beacon_carries_exactly_these_public_fields_and_no_others() {
        let json = serde_json::to_value(beacon()).unwrap();
        let mut keys: Vec<&str> = json
            .as_object()
            .expect("a beacon is an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();

        assert_eq!(
            keys,
            [
                "hasVault",
                "hostId",
                "hostName",
                "port",
                "requiresPin",
                "vault",
            ],
            "a field was added to something broadcast in the clear — is it public?"
        );
    }

    #[test]
    fn a_beacon_says_whether_a_pin_is_needed_without_carrying_one() {
        // The flag is public and useful: the client shows a lock against hosts
        // that will ask. The PIN itself never leaves the host.
        let json = serde_json::to_value(beacon()).unwrap();
        assert_eq!(
            json.get("requiresPin"),
            Some(&serde_json::Value::Bool(true))
        );
        assert!(json.get("pin").is_none());
    }

    #[test]
    fn an_older_beacon_without_the_vault_flag_still_parses() {
        let value = serde_json::json!({
            "hostId": "aa", "hostName": "b", "vault": "v",
            "port": 7742, "requiresPin": false,
        });
        let parsed: Beacon = serde_json::from_value(value).unwrap();
        assert!(!parsed.has_vault);
    }

    #[test]
    fn the_dial_address_comes_from_the_sender_not_the_payload() {
        let from: SocketAddr = "192.168.1.11:54321".parse().unwrap();
        let found = Found::from_reply(beacon(), from);
        assert_eq!(found.address.ip(), from.ip());
        assert_eq!(found.address.port(), 7742, "the advertised service port");
    }

    #[test]
    fn nonces_are_never_zero_because_zero_means_announcement() {
        for _ in 0..200 {
            assert_ne!(rand_nonce(), 0);
        }
    }

    #[test]
    fn nonces_differ_between_scans() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..50 {
            seen.insert(rand_nonce());
        }
        assert!(seen.len() > 45, "nonces look degenerate");
    }

    #[test]
    fn the_limited_broadcast_is_always_a_target() {
        let targets = broadcast_targets();
        assert!(targets.contains(&SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::BROADCAST,
            DISCOVERY_PORT
        ))));
        assert!(targets.iter().all(|t| t.port() == DISCOVERY_PORT));
    }

    #[test]
    fn broadcast_targets_are_not_duplicated() {
        let targets = broadcast_targets();
        let unique: std::collections::HashSet<_> = targets.iter().collect();
        assert_eq!(unique.len(), targets.len());
    }

    /// The whole thing, over real sockets.
    #[tokio::test]
    async fn a_host_answers_a_scan() {
        let responder = tokio::spawn(respond(beacon));
        // Let the responder bind before anything is sent to it.
        tokio::time::sleep(Duration::from_millis(150)).await;

        let found = scan(Duration::from_millis(700)).await.unwrap();
        responder.abort();

        let ours = found.iter().find(|f| f.beacon.host_id == "aabbccdd");
        assert!(
            ours.is_some(),
            "the host should have answered; found {found:?}"
        );
        let ours = ours.unwrap();
        assert_eq!(ours.beacon.vault, "My Drive");
        assert_eq!(ours.address.port(), 7742);
    }

    #[tokio::test]
    async fn looking_for_a_specific_key_finds_it() {
        let responder = tokio::spawn(respond(beacon));
        tokio::time::sleep(Duration::from_millis(150)).await;

        let found = find_host("aabbccdd", Duration::from_millis(2000))
            .await
            .unwrap();
        responder.abort();
        assert!(found.is_some(), "the pinned key should have been located");
    }

    #[tokio::test]
    async fn looking_for_a_key_that_is_not_there_gives_up() {
        let found = find_host("not-a-real-host-id", Duration::from_millis(400))
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn a_scan_with_nobody_listening_returns_a_list_rather_than_failing() {
        let found = scan(Duration::from_millis(250)).await.unwrap();
        // Another test's responder may be alive; what matters is that this
        // completes and returns a list rather than erroring or hanging.
        assert!(found.len() < 50);
    }
}
