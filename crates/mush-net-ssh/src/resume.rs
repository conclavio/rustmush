//! The optional cockpit resume subsystem. Tickets stay off the terminal channel.
//! The instance gateway owns one registry shared by its direct and trusted SSH listeners.
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use mush_session::SessId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fmt, io,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use zeroize::Zeroizing;

pub const MAX_FRAME: usize = 1024;
pub const SUBSYSTEM: &str = "rustmush-resume-v1";
pub const ENV_NAME: &str = "RUSTMUSH_RESUME_V1";
const MAX_LEASES: usize = 4096;
const MAX_GRACE: Duration = Duration::from_secs(300);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Issue,
    Redeem,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hint {
    pub mode: Mode,
    pub pane: String,
}

fn valid_pane(pane: &str) -> bool {
    pane.len() == 32
        && pane
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Hint {
    pub fn parse(value: &str) -> Option<Self> {
        let (mode, pane) = value.split_once(':')?;
        let mode = match mode {
            "issue" => Mode::Issue,
            "redeem" => Mode::Redeem,
            _ => return None,
        };
        valid_pane(pane).then(|| Self {
            mode,
            pane: pane.to_owned(),
        })
    }
}

/// Random 256-bit value issued only by the instance. Registry entries retain its digest.
#[derive(Clone)]
pub struct Ticket(Zeroizing<[u8; 32]>);

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ticket([redacted])")
    }
}
impl Ticket {
    fn generate() -> Result<Self, Failure> {
        let mut bytes = Zeroizing::new([0; 32]);
        getrandom::fill(&mut *bytes).map_err(|_| Failure::Unavailable)?;
        Ok(Self(bytes))
    }
    fn digest(&self) -> [u8; 32] {
        Sha256::digest(self.0.as_slice()).into()
    }
    fn parse(encoded: &str) -> Option<Self> {
        if encoded.len() != 43 {
            return None;
        }
        let mut bytes = Zeroizing::new([0; 32]);
        (URL_SAFE_NO_PAD.decode_slice(encoded, &mut *bytes).ok()? == 32).then_some(Self(bytes))
    }
}

#[derive(Debug)]
pub struct Request {
    pub mode: Mode,
    pub pane: String,
    pub ticket: Option<Ticket>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    Denied,
    Expired,
    Unavailable,
    Busy,
}
impl Failure {
    fn status(self) -> &'static str {
        match self {
            Self::Denied => "denied",
            Self::Expired => "expired",
            Self::Unavailable => "unavailable",
            Self::Busy => "busy",
        }
    }
}

#[derive(Debug)]
pub struct Reply {
    status: &'static str,
    ticket: Option<Ticket>,
    grace: Option<u32>,
}
impl Reply {
    pub fn issued(ticket: Ticket, grace_seconds: u32) -> Self {
        Self::success("issued", ticket, grace_seconds)
    }
    pub fn redeemed(ticket: Ticket, grace_seconds: u32) -> Self {
        Self::success("redeemed", ticket, grace_seconds)
    }
    fn success(status: &'static str, ticket: Ticket, grace: u32) -> Self {
        Self {
            status,
            ticket: Some(ticket),
            grace: Some(grace.clamp(1, 300)),
        }
    }
    pub fn auth_required() -> Self {
        Self {
            status: "auth_required",
            ticket: None,
            grace: None,
        }
    }
    pub fn failure(error: Failure) -> Self {
        Self {
            status: error.status(),
            ticket: None,
            grace: None,
        }
    }
}

fn bad_frame() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid cockpit resume frame")
}

pub fn decode_request(payload: &[u8]) -> io::Result<Request> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Wire<'a> {
        v: u8,
        op: Mode,
        pane: &'a str,
        ticket: Option<&'a str>,
    }
    if payload.is_empty() || payload.len() > MAX_FRAME {
        return Err(bad_frame());
    }
    let wire: Wire<'_> = serde_json::from_slice(payload).map_err(|_| bad_frame())?;
    if wire.v != 1 || !valid_pane(wire.pane) {
        return Err(bad_frame());
    }
    let ticket = match (wire.op, wire.ticket) {
        (Mode::Issue, None) => None,
        (Mode::Redeem, Some(raw)) => Some(Ticket::parse(raw).ok_or_else(bad_frame)?),
        _ => return Err(bad_frame()),
    };
    Ok(Request {
        mode: wire.op,
        pane: wire.pane.to_owned(),
        ticket,
    })
}

pub async fn read_request(reader: &mut (impl AsyncRead + Unpin)) -> io::Result<Request> {
    let len = reader.read_u32().await? as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(bad_frame());
    }
    let mut bytes = Zeroizing::new(vec![0; len]);
    reader.read_exact(&mut bytes).await?;
    decode_request(&bytes)
}

pub fn encode_reply(reply: &Reply) -> io::Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Wire<'a> {
        v: u8,
        status: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        ticket: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        grace_seconds: Option<u32>,
    }
    let secret = reply
        .ticket
        .as_ref()
        .map(|t| Zeroizing::new(URL_SAFE_NO_PAD.encode(t.0.as_slice())));
    let wire = Wire {
        v: 1,
        status: reply.status,
        ticket: secret.as_ref().map(|s| s.as_str()),
        grace_seconds: reply.grace,
    };
    let mut packet = vec![0; 4];
    serde_json::to_writer(&mut packet, &wire).map_err(|_| bad_frame())?;
    let len = packet.len() - 4;
    if len > MAX_FRAME {
        return Err(bad_frame());
    }
    packet[..4].copy_from_slice(&(len as u32).to_be_bytes());
    Ok(packet)
}

pub async fn write_reply(writer: &mut (impl AsyncWrite + Unpin), reply: &Reply) -> io::Result<()> {
    writer
        .write_all(&Zeroizing::new(encode_reply(reply)?))
        .await?;
    writer.flush().await
}

#[derive(Clone, Copy)]
enum State {
    Attached,
    Detached(Instant),
    Reserved {
        conn: u64,
        deadline: Instant,
        replacement: [u8; 32],
    },
}
impl State {
    fn is_expired(self, now: Instant) -> bool {
        match self {
            Self::Attached => false,
            Self::Detached(deadline) | Self::Reserved { deadline, .. } => deadline <= now,
        }
    }
}
struct Lease {
    digest: [u8; 32],
    account: String,
    character: u64,
    session: SessId,
    conn: u64,
    grace: Duration,
    state: State,
}

#[derive(Clone, Default)]
pub struct Registry(Arc<Mutex<HashMap<String, Lease>>>);
impl Registry {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Lease>> {
        self.0.lock().unwrap_or_else(|poison| poison.into_inner())
    }
    /// Call only after the gateway has established the authenticated actor and session.
    pub fn issue(
        &self,
        conn: u64,
        pane: &str,
        account: &str,
        character: u64,
        session: SessId,
        grace: Duration,
    ) -> Result<Ticket, Failure> {
        if !valid_pane(pane) || account.is_empty() || grace.as_secs() == 0 {
            return Err(Failure::Unavailable);
        }
        let ticket = Ticket::generate()?;
        let mut leases = self.lock();
        let now = Instant::now();
        leases.retain(|_, lease| !lease.state.is_expired(now));
        if leases.len() >= MAX_LEASES
            || leases.contains_key(pane)
            || leases.values().any(|l| l.conn == conn)
        {
            return Err(Failure::Busy);
        }
        leases.insert(
            pane.to_owned(),
            Lease {
                digest: ticket.digest(),
                account: account.to_owned(),
                character,
                session,
                conn,
                grace: grace.min(MAX_GRACE),
                state: State::Attached,
            },
        );
        Ok(ticket)
    }
    /// An orderly quit or revocation ends a ticket. An unexpected detach starts its grace clock.
    pub fn detach(&self, conn: u64, lost: bool) {
        let now = Instant::now();
        self.lock().retain(|_, lease| {
            if lease.state.is_expired(now) {
                return false;
            }
            if let State::Reserved {
                conn: candidate,
                deadline,
                ..
            } = lease.state
                && candidate == conn
            {
                lease.state = State::Detached(deadline);
            }
            if lease.conn != conn {
                return true;
            }
            if !lost {
                return false;
            }
            if matches!(lease.state, State::Attached) {
                lease.state = State::Detached(now + lease.grace);
            }
            true
        });
    }
    /// Account and selected character come from the authenticated gateway, never the sideband request.
    pub fn reserve(
        &self,
        conn: u64,
        pane: &str,
        account: &str,
        selected: Option<u64>,
        ticket: &Ticket,
    ) -> Result<Claim, Failure> {
        let mut leases = self.lock();
        let lease = leases.get_mut(pane).ok_or(Failure::Expired)?;
        if !bool::from(lease.digest.ct_eq(&ticket.digest()))
            || lease.account != account
            || selected.is_some_and(|c| c != lease.character)
        {
            return Err(Failure::Denied);
        }
        let now = Instant::now();
        if lease.state.is_expired(now) {
            leases.remove(pane);
            return Err(Failure::Expired);
        }
        let State::Detached(deadline) = lease.state else {
            return Err(Failure::Busy);
        };
        let replacement = Ticket::generate()?;
        lease.state = State::Reserved {
            conn,
            deadline,
            replacement: replacement.digest(),
        };
        Ok(Claim {
            registry: self.clone(),
            pane: pane.to_owned(),
            conn,
            session: lease.session,
            character: lease.character,
            previous_conn: lease.conn,
            replacement,
        })
    }
}

/// No registry lock is held while the gateway checks its session and world admission.
pub struct Claim {
    registry: Registry,
    pane: String,
    conn: u64,
    session: SessId,
    character: u64,
    previous_conn: u64,
    replacement: Ticket,
}
impl Claim {
    pub fn session(&self) -> SessId {
        self.session
    }
    pub fn character(&self) -> u64 {
        self.character
    }
    pub fn previous_conn(&self) -> u64 {
        self.previous_conn
    }
    pub fn commit(self) -> Result<Ticket, Failure> {
        let mut leases = self.registry.lock();
        let lease = leases.get_mut(&self.pane).ok_or(Failure::Expired)?;
        let State::Reserved {
            conn,
            deadline,
            replacement,
        } = lease.state
        else {
            return Err(Failure::Denied);
        };
        if conn != self.conn
            || lease.session != self.session
            || replacement != self.replacement.digest()
        {
            return Err(Failure::Denied);
        }
        if deadline <= Instant::now() {
            leases.remove(&self.pane);
            return Err(Failure::Expired);
        }
        lease.state = State::Attached;
        lease.conn = self.conn;
        lease.digest = replacement;
        Ok(self.replacement.clone())
    }
}
impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(lease) = self.registry.lock().get_mut(&self.pane)
            && let State::Reserved {
                conn,
                deadline,
                replacement,
            } = lease.state
            && conn == self.conn
            && lease.session == self.session
            && replacement == self.replacement.digest()
        {
            lease.state = State::Detached(deadline);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const PANE: &str = "00112233445566778899aabbccddeeff";

    #[test]
    fn reserve_requires_an_authenticated_owner_a_matching_character_and_a_detached_pane()
    -> Result<(), Failure> {
        let registry = Registry::default();
        let ticket = registry.issue(70, PANE, "pilot", 3, SessId(80), MAX_GRACE)?;
        assert!(matches!(
            registry.reserve(71, PANE, "pilot", Some(3), &ticket),
            Err(Failure::Busy)
        ));
        registry.detach(70, true);
        assert!(matches!(
            registry.reserve(71, PANE, "intruder", Some(3), &ticket),
            Err(Failure::Denied)
        ));
        assert!(matches!(
            registry.reserve(71, PANE, "pilot", Some(4), &ticket),
            Err(Failure::Denied)
        ));
        let claim = registry.reserve(71, PANE, "pilot", Some(3), &ticket)?;
        assert_eq!(
            (claim.character(), claim.session(), claim.previous_conn()),
            (3, SessId(80), 70)
        );
        assert!(matches!(
            registry.reserve(72, PANE, "pilot", None, &ticket),
            Err(Failure::Busy)
        ));
        drop(claim);
        let rotated = registry
            .reserve(72, PANE, "pilot", None, &ticket)?
            .commit()?;
        registry.detach(72, true);
        assert!(matches!(
            registry.reserve(73, PANE, "pilot", None, &ticket),
            Err(Failure::Denied)
        ));
        let latest = registry
            .reserve(73, PANE, "pilot", None, &rotated)?
            .commit()?;
        registry.detach(73, false);
        assert!(matches!(
            registry.reserve(74, PANE, "pilot", None, &latest),
            Err(Failure::Expired)
        ));
        Ok(())
    }

    #[test]
    fn cancelled_claim_cannot_take_or_release_a_later_reservation_on_the_same_connection()
    -> Result<(), Failure> {
        let registry = Registry::default();
        let ticket = registry.issue(1, PANE, "pilot", 3, SessId(8), MAX_GRACE)?;
        registry.detach(1, true);
        let first = registry.reserve(2, PANE, "pilot", None, &ticket)?;
        registry.detach(2, true);
        let second = registry.reserve(2, PANE, "pilot", None, &ticket)?;
        assert!(matches!(first.commit(), Err(Failure::Denied)));
        second.commit()?;
        Ok(())
    }

    #[test]
    fn an_expired_lease_cannot_be_redeemed_before_the_next_sweep() -> Result<(), Failure> {
        let registry = Registry::default();
        let ticket = registry.issue(1, PANE, "pilot", 3, SessId(8), MAX_GRACE)?;
        registry.detach(1, true);
        if let Some(lease) = registry.lock().get_mut(PANE) {
            lease.state = State::Detached(Instant::now());
        }
        assert!(matches!(
            registry.reserve(2, PANE, "pilot", None, &ticket),
            Err(Failure::Expired)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn reads_one_bounded_versioned_frame_and_redacts_tickets() -> io::Result<()> {
        assert!(Hint::parse(&format!("redeem:{PANE}")).is_some());
        let ticket = Ticket::generate().map_err(|_| bad_frame())?;
        let secret = URL_SAFE_NO_PAD.encode(*ticket.0);
        let payload =
            serde_json::json!({"v":1,"op":"redeem","pane":PANE,"ticket":secret}).to_string();
        let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(payload.as_bytes());
        let request = read_request(&mut frame.as_slice()).await?;
        assert_eq!(request.mode, Mode::Redeem);
        assert!(!format!("{request:?}").contains(&secret));
        let reply = Reply::issued(ticket, 300);
        assert!(!format!("{reply:?}").contains(&secret));
        let encoded = encode_reply(&reply)?;
        let mut bytes = encoded.as_slice();
        assert_eq!(bytes.read_u32().await? as usize, bytes.len());
        let json: serde_json::Value = serde_json::from_slice(bytes)?;
        assert_eq!(json["ticket"], secret);
        for invalid in [
            serde_json::json!({"v":1,"op":"issue","pane":PANE,"ticket":secret}),
            serde_json::json!({"v":1,"op":"redeem","pane":PANE}),
            serde_json::json!({"v":1,"op":"redeem","pane":PANE,"ticket":"short"}),
            serde_json::json!({"v":2,"op":"issue","pane":PANE}),
            serde_json::json!({"v":1,"op":"issue","pane":"short"}),
            serde_json::json!({"v":1,"op":"issue","pane":PANE,"account":"other"}),
        ] {
            assert!(decode_request(invalid.to_string().as_bytes()).is_err());
        }
        assert!(
            read_request(&mut (MAX_FRAME as u32 + 1).to_be_bytes().as_slice())
                .await
                .is_err()
        );
        Ok(())
    }
}
