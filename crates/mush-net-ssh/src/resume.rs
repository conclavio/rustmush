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
            grace: Some(grace),
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

pub fn encode_reply(reply: &Reply) -> io::Result<Zeroizing<Vec<u8>>> {
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
    let mut packet = Zeroizing::new(vec![0; 4 + MAX_FRAME]);
    let mut body = &mut packet[4..];
    serde_json::to_writer(&mut body, &wire).map_err(|_| bad_frame())?;
    let len = MAX_FRAME - body.len();
    packet.truncate(4 + len);
    packet[..4].copy_from_slice(&(len as u32).to_be_bytes());
    Ok(packet)
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
/// Presentation choices owned by the player. Capabilities and geometry are
/// always taken from the newly connected terminal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewPrefs {
    pub tui: bool,
    pub sounds: bool,
    pub encoder: mush_pixel::caps::EncoderChoice,
}
/// Character ID, rotated ticket, grace in seconds, and saved presentation.
pub type Redeemed = (u64, Ticket, u32, ViewPrefs);

impl Default for ViewPrefs {
    fn default() -> Self {
        Self {
            tui: false,
            sounds: true,
            encoder: Default::default(),
        }
    }
}
struct Lease {
    digest: [u8; 32],
    previous: Option<PreviousTicket>,
    character: u64,
    session: SessId,
    conn: u64,
    grace_seconds: u32,
    state: State,
    view: ViewPrefs,
}

impl Lease {
    fn cancel_candidate(&mut self, conn: u64) {
        if let State::Reserved {
            conn: candidate,
            deadline,
            ..
        } = self.state
            && candidate == conn
        {
            self.state = State::Detached(deadline);
        }
    }
}

#[derive(Clone, Copy)]
struct PreviousTicket {
    digest: [u8; 32],
    until: Instant,
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct PaneKey {
    account: String,
    pane: String,
}
impl PaneKey {
    fn new(account: &str, pane: &str) -> Self {
        Self {
            account: account.to_owned(),
            pane: pane.to_owned(),
        }
    }
}

type Leases = HashMap<PaneKey, Lease>;

#[derive(Clone, Default)]
pub struct Registry(Arc<Mutex<Leases>>);
impl Registry {
    fn lock(&self) -> MutexGuard<'_, Leases> {
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
    ) -> Result<(Ticket, u32), Failure> {
        let grace_seconds = grace.min(MAX_GRACE).as_secs() as u32;
        if !valid_pane(pane) || account.is_empty() || grace_seconds == 0 {
            return Err(Failure::Unavailable);
        }
        let ticket = Ticket::generate()?;
        let mut leases = self.lock();
        let now = Instant::now();
        leases.retain(|_, lease| !lease.state.is_expired(now));
        let key = PaneKey::new(account, pane);
        match leases.get(&key) {
            Some(lease) if !matches!(lease.state, State::Detached(_)) => return Err(Failure::Busy),
            None if leases.len() >= MAX_LEASES => return Err(Failure::Busy),
            _ => {}
        }
        if leases.iter().any(|(other, lease)| {
            (other != &key && lease.conn == conn)
                || matches!(lease.state, State::Reserved { conn: candidate, .. } if candidate == conn)
        }) {
            return Err(Failure::Busy);
        }
        leases.insert(
            key,
            Lease {
                digest: ticket.digest(),
                previous: None,
                character,
                session,
                conn,
                grace_seconds,
                state: State::Attached,
                view: ViewPrefs::default(),
            },
        );
        Ok((ticket, grace_seconds))
    }
    /// Ignore late input from a connection that has lost or transferred its lease.
    pub fn update_view(&self, conn: u64, account: &str, pane: &str, view: ViewPrefs) {
        if let Some(lease) = self.lock().get_mut(&PaneKey::new(account, pane))
            && lease.conn == conn
            && matches!(lease.state, State::Attached)
        {
            lease.view = view;
        }
    }

    /// Revoke an active owner or cancel its own unfinished admission. A late
    /// close from the old owner leaves a parked or newly reserved lease intact.
    pub fn close_attached(&self, conn: u64) {
        self.lock().retain(|_, lease| {
            lease.cancel_candidate(conn);
            lease.conn != conn || !matches!(lease.state, State::Attached)
        });
    }
    /// An orderly quit or revocation ends a ticket. An unexpected detach starts its grace clock.
    pub fn detach(&self, conn: u64, lost: bool) {
        let now = Instant::now();
        self.lock().retain(|_, lease| {
            if lease.state.is_expired(now) {
                return false;
            }
            lease.cancel_candidate(conn);
            if lease.conn != conn {
                return true;
            }
            if !lost {
                return false;
            }
            if matches!(lease.state, State::Attached) {
                lease.state =
                    State::Detached(now + Duration::from_secs(lease.grace_seconds.into()));
            }
            true
        });
    }
    /// A current credential may identify its attached connection for a gateway
    /// ownership check. The gateway must fence that exact session before redeeming.
    pub fn attached_owner(
        &self,
        pane: &str,
        account: &str,
        selected: Option<u64>,
        ticket: &Ticket,
    ) -> Result<Option<(u64, u64, SessId)>, Failure> {
        let mut leases = self.lock();
        let (lease, previous) = checked_lease(
            &mut leases,
            &PaneKey::new(account, pane),
            selected,
            ticket.digest(),
            Instant::now(),
        )?;
        Ok(
            (previous.is_none() && matches!(lease.state, State::Attached)).then_some((
                lease.conn,
                lease.character,
                lease.session,
            )),
        )
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
        let key = PaneKey::new(account, pane);
        let digest = ticket.digest();
        let now = Instant::now();
        let (lease, previous) = checked_lease(&mut leases, &key, selected, digest, now)?;
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
            key,
            conn,
            session: lease.session,
            character: lease.character,
            previous_conn: lease.conn,
            view: lease.view,
            replacement,
            previous: PreviousTicket {
                digest,
                until: previous.unwrap_or(now + Duration::from_secs(lease.grace_seconds.into())),
            },
        })
    }
}

/// The optional deadline belongs to an earlier credential whose replacement
/// reply may have been lost. Retrying with it never extends that deadline.
fn checked_lease<'a>(
    leases: &'a mut Leases,
    key: &PaneKey,
    selected: Option<u64>,
    digest: [u8; 32],
    now: Instant,
) -> Result<(&'a mut Lease, Option<Instant>), Failure> {
    if !leases.contains_key(key) {
        return Err(if leases.keys().any(|other| other.pane == key.pane) {
            Failure::Denied
        } else {
            Failure::Expired
        });
    }
    let lease = leases.get_mut(key).ok_or(Failure::Expired)?;
    if selected.is_some_and(|ch| ch != lease.character) {
        return Err(Failure::Denied);
    }
    lease.previous = lease.previous.filter(|prior| prior.until > now);
    let previous = if bool::from(lease.digest.ct_eq(&digest)) {
        None
    } else {
        Some(
            lease
                .previous
                .filter(|prior| bool::from(prior.digest.ct_eq(&digest)))
                .ok_or(Failure::Denied)?
                .until,
        )
    };
    if lease.state.is_expired(now) {
        return Err(Failure::Expired);
    }
    Ok((lease, previous))
}

/// No registry lock is held while the gateway checks its session and world admission.
pub struct Claim {
    view: ViewPrefs,
    registry: Registry,
    key: PaneKey,
    conn: u64,
    session: SessId,
    character: u64,
    previous_conn: u64,
    replacement: Ticket,
    previous: PreviousTicket,
}
impl Claim {
    pub fn view(&self) -> ViewPrefs {
        self.view
    }
    pub fn session(&self) -> SessId {
        self.session
    }
    pub fn character(&self) -> u64 {
        self.character
    }
    pub fn previous_conn(&self) -> u64 {
        self.previous_conn
    }
    pub fn commit(self) -> Result<(Ticket, u32), Failure> {
        let mut leases = self.registry.lock();
        let lease = leases.get_mut(&self.key).ok_or(Failure::Expired)?;
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
        let now = Instant::now();
        if deadline <= now {
            leases.remove(&self.key);
            return Err(Failure::Expired);
        }
        if self.previous.until <= now {
            return Err(Failure::Denied);
        }
        lease.state = State::Attached;
        lease.conn = self.conn;
        lease.digest = replacement;
        lease.previous = Some(self.previous);
        Ok((self.replacement.clone(), lease.grace_seconds))
    }
}
impl Drop for Claim {
    fn drop(&mut self) {
        if let Some(lease) = self.registry.lock().get_mut(&self.key)
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
    fn only_attached_owner_can_change_the_view() {
        let registry = Registry::default();
        let (ticket, _) = registry
            .issue(7, PANE, "pilot", 3, SessId(8), MAX_GRACE)
            .unwrap();
        let saved = ViewPrefs {
            tui: true,
            sounds: false,
            encoder: mush_pixel::caps::EncoderChoice::Off,
        };
        registry.update_view(7, "pilot", PANE, saved);
        registry.update_view(8, "pilot", PANE, ViewPrefs::default());
        registry.update_view(7, "other", PANE, ViewPrefs::default());
        registry.detach(7, true);
        registry.update_view(7, "pilot", PANE, ViewPrefs::default());
        let claim = registry
            .reserve(8, PANE, "pilot", Some(3), &ticket)
            .unwrap();
        assert_eq!(claim.view(), saved);
        let (ticket, _) = claim.commit().unwrap();
        registry.update_view(7, "pilot", PANE, ViewPrefs::default());
        registry.detach(8, true);
        assert_eq!(
            registry
                .reserve(9, PANE, "pilot", None, &ticket)
                .unwrap()
                .view(),
            saved
        );
    }

    #[test]
    fn reserve_requires_an_authenticated_owner_a_matching_character_and_a_detached_pane() {
        let registry = Registry::default();
        let (ticket, _) = registry
            .issue(70, PANE, "pilot", 3, SessId(80), MAX_GRACE)
            .expect("issue initial ticket");
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
        let claim = registry
            .reserve(71, PANE, "pilot", Some(3), &ticket)
            .expect("reserve detached pane");
        assert_eq!(
            (claim.character(), claim.session(), claim.previous_conn()),
            (3, SessId(80), 70)
        );
        assert!(matches!(
            registry.reserve(72, PANE, "pilot", None, &ticket),
            Err(Failure::Busy)
        ));
        drop(claim);
        let (rotated, _) = registry
            .reserve(72, PANE, "pilot", None, &ticket)
            .expect("reserve released claim")
            .commit()
            .expect("rotate ticket");
        assert!(matches!(
            registry.reserve(73, PANE, "pilot", None, &ticket),
            Err(Failure::Busy)
        ));
        registry.detach(72, true);
        let (latest, _) = registry
            .reserve(73, PANE, "pilot", None, &rotated)
            .expect("reserve using rotated ticket")
            .commit()
            .expect("commit reconnect");
        assert!(matches!(
            registry.reserve(74, PANE, "pilot", None, &ticket),
            Err(Failure::Denied)
        ));
        registry.detach(73, false);
        assert!(matches!(
            registry.reserve(74, PANE, "pilot", None, &latest),
            Err(Failure::Expired)
        ));
    }

    #[test]
    fn cancelled_claim_cannot_take_or_release_a_later_reservation_on_the_same_connection() {
        let registry = Registry::default();
        let (ticket, _) = registry
            .issue(1, PANE, "pilot", 3, SessId(8), MAX_GRACE)
            .expect("issue initial ticket");
        registry.detach(1, true);
        let first = registry
            .reserve(2, PANE, "pilot", None, &ticket)
            .expect("reserve first claim");
        registry.detach(2, true);
        let second = registry
            .reserve(2, PANE, "pilot", None, &ticket)
            .expect("reserve new claim");
        assert!(matches!(first.commit(), Err(Failure::Denied)));
        second.commit().expect("commit new claim");
    }

    #[test]
    fn fresh_login_replaces_only_the_accounts_own_detached_pane() {
        let registry = Registry::default();
        let (old, grace) = registry
            .issue(1, PANE, "pilot", 8, SessId(10), Duration::from_secs(3600))
            .expect("issue");
        assert_eq!(grace, 300);
        let (other, _) = registry
            .issue(2, PANE, "copilot", 9, SessId(20), MAX_GRACE)
            .expect("independent nonce");
        let issue_again =
            || registry.issue(3, PANE, "pilot", 7, SessId(30), Duration::from_millis(1900));
        assert!(matches!(issue_again(), Err(Failure::Busy)));
        registry.detach(1, true);
        let pending = registry
            .reserve(4, PANE, "pilot", None, &old)
            .expect("claim");
        assert!(matches!(issue_again(), Err(Failure::Busy)));
        drop(pending);
        let (fresh, grace) = issue_again().expect("issue after ordinary character admission");
        assert_eq!(grace, 1);
        registry.detach(1, false);
        assert!(matches!(
            registry.reserve(5, PANE, "pilot", None, &old),
            Err(Failure::Denied)
        ));
        assert!(matches!(
            registry.reserve(5, PANE, "pilot", None, &other),
            Err(Failure::Denied)
        ));
        assert_eq!(
            registry.attached_owner(PANE, "copilot", Some(9), &other),
            Ok(Some((2, 9, SessId(20))))
        );
        registry.detach(3, true);
        let claim = registry
            .reserve(5, PANE, "pilot", Some(7), &fresh)
            .expect("recover fresh session");
        assert_eq!(claim.session(), SessId(30));
        assert_eq!(claim.commit().expect("rotate").1, 1);
    }

    #[test]
    fn a_lost_rotation_reply_can_only_recover_a_detached_pane_within_one_grace() {
        let registry = Registry::default();
        let (first, _) = registry
            .issue(1, PANE, "pilot", 8, SessId(10), MAX_GRACE)
            .expect("issue");
        let owner = |ticket| registry.attached_owner(PANE, "pilot", Some(8), ticket);
        let reserve = |conn, ticket| registry.reserve(conn, PANE, "pilot", Some(8), ticket);
        assert_eq!(owner(&first), Ok(Some((1, 8, SessId(10)))));
        assert_eq!(
            registry.attached_owner(PANE, "other", Some(8), &first),
            Err(Failure::Denied)
        );
        assert_eq!(
            registry.attached_owner(PANE, "pilot", Some(9), &first),
            Err(Failure::Denied)
        );
        registry.detach(1, true);
        let (lost, _) = reserve(2, &first)
            .expect("first reclaim")
            .commit()
            .expect("first rotation");
        assert_eq!(owner(&first), Ok(None));
        assert_eq!(owner(&lost), Ok(Some((2, 8, SessId(10)))));
        assert!(matches!(reserve(3, &first), Err(Failure::Busy)));
        registry.detach(2, true);
        let key = PaneKey::new("pilot", PANE);
        let previous_deadline = || {
            registry
                .lock()
                .get(&key)
                .and_then(|l| l.previous)
                .expect("retry credential")
                .until
        };
        let deadline = previous_deadline();
        let (fresh, _) = reserve(3, &first)
            .expect("retry after reply loss")
            .commit()
            .expect("second rotation");
        assert_eq!(
            previous_deadline(),
            deadline,
            "retry must not renew the previous credential"
        );
        assert_eq!(owner(&lost), Err(Failure::Denied));
        registry.detach(3, true);
        let mut in_flight = reserve(4, &first).expect("reserve before retry deadline");
        in_flight.previous.until = Instant::now();
        assert!(matches!(in_flight.commit(), Err(Failure::Denied)));
        registry
            .lock()
            .get_mut(&key)
            .and_then(|l| l.previous.as_mut())
            .expect("retry credential")
            .until = Instant::now();
        assert!(matches!(reserve(4, &first), Err(Failure::Denied)));
        assert!(reserve(4, &fresh).is_ok());
    }

    #[test]
    fn a_late_close_only_revokes_the_exact_still_attached_owner() {
        let registry = Registry::default();
        let (ticket, _) = registry
            .issue(1, PANE, "pilot", 8, SessId(10), MAX_GRACE)
            .expect("issue");
        registry.detach(1, true);
        registry.close_attached(1);
        let abandoned = registry
            .reserve(3, PANE, "pilot", None, &ticket)
            .expect("candidate awaiting its world binding");
        registry.close_attached(3);
        assert!(abandoned.commit().is_err());
        let claim = registry
            .reserve(2, PANE, "pilot", None, &ticket)
            .expect("detached owner survived old close");
        registry.close_attached(1);
        let (replacement, _) = claim.commit().expect("reserved owner survived old close");
        registry.close_attached(1);
        assert_eq!(
            registry.attached_owner(PANE, "pilot", None, &replacement),
            Ok(Some((2, 8, SessId(10))))
        );
        registry.close_attached(2);
        assert_eq!(
            registry.attached_owner(PANE, "pilot", None, &replacement),
            Err(Failure::Expired)
        );
    }

    #[test]
    fn an_expired_lease_cannot_be_redeemed_before_the_next_sweep() {
        let registry = Registry::default();
        let (ticket, _) = registry
            .issue(1, PANE, "pilot", 3, SessId(8), MAX_GRACE)
            .expect("issue initial ticket");
        registry.detach(1, true);
        if let Some(lease) = registry.lock().get_mut(&PaneKey::new("pilot", PANE)) {
            lease.state = State::Detached(Instant::now());
        }
        assert!(matches!(
            registry.reserve(2, PANE, "pilot", None, &ticket),
            Err(Failure::Expired)
        ));
    }

    #[test]
    fn validates_the_bounded_request_and_prefixes_the_redacted_reply() {
        assert!(Hint::parse(&format!("redeem:{PANE}")).is_some());
        let ticket = Ticket::generate().expect("generate ticket");
        let secret = URL_SAFE_NO_PAD.encode(*ticket.0);
        let payload =
            serde_json::json!({"v":1,"op":"redeem","pane":PANE,"ticket":secret}).to_string();
        let request = decode_request(payload.as_bytes()).expect("decode request");
        assert_eq!(request.mode, Mode::Redeem);
        assert!(!format!("{request:?}").contains(&secret));
        let reply = Reply::issued(ticket, 300);
        assert!(!format!("{reply:?}").contains(&secret));
        let encoded = encode_reply(&reply).expect("encode reply");
        let (header, bytes) = encoded
            .split_first_chunk::<4>()
            .expect("reply length prefix");
        assert_eq!(u32::from_be_bytes(*header) as usize, bytes.len());
        let json: serde_json::Value = serde_json::from_slice(bytes).expect("decode reply");
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
        assert!(decode_request(&[]).is_err());
        let mut padded = payload.into_bytes();
        padded.resize(MAX_FRAME, b' ');
        assert!(decode_request(&padded).is_ok());
        padded.push(b' ');
        assert!(decode_request(&padded).is_err());
    }
}
