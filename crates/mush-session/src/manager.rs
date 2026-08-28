//! [`SessionManager`]: character → connections fan-out, attach/detach,
//! linkdead, WHO, flood control.

use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use mush_proto::{ClientCaps, OutputEvent};

use crate::conn::{ConnId, ConnectionHandle};
use crate::flood::{FloodBucket, FloodVerdict};

/// Identifies one session (one account playing one character, over 1..N
/// connections).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SessId(pub u64);

/// Opaque character key. `mushd` maps `mush-core::ObjId` (a `u32`) to and
/// from this by widening (`CharKey(u64::from(id.0))`); the session layer
/// never interprets the value — it is purely a fan-out routing key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CharKey(pub u64);

/// Tunables for the session layer.
#[derive(Clone, Copy, Debug)]
pub struct SessionConfig {
    /// How long a session stays linkdead after its last connection drops
    /// uncleanly before [`SessionManager::take_expired`] will reap it.
    pub linkdead_grace: Duration,
    /// Sustained input lines per second per connection.
    pub flood_rate: f64,
    /// Input burst allowance per connection (token-bucket capacity).
    pub flood_burst: f64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            linkdead_grace: Duration::from_secs(300),
            flood_rate: 10.0,
            flood_burst: 30.0,
        }
    }
}

/// What [`SessionManager::attach`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachOutcome {
    /// First connection for this character: a fresh session.
    New,
    /// The character already had a session; this connection joined it.
    Rejoined {
        /// How many *other* live connections the session already had
        /// (0 when this is a reconnect out of linkdead).
        others: usize,
        /// `true` when the session was linkdead and this attach revived it
        /// (drivers announce "has reconnected" vs "has connected again").
        from_linkdead: bool,
    },
}

/// Why a connection is being detached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetachReason {
    /// Player typed QUIT: clean close, no linkdead grace.
    Quit,
    /// Staff/@boot or server shutdown: clean close, no linkdead grace.
    Booted,
    /// Transport dropped unexpectedly: linkdead grace applies.
    Lost,
    /// Server dropped a slow client (output buffer overflow): linkdead
    /// grace applies (their link is bad, not their intent).
    SlowClient,
}

impl DetachReason {
    /// Clean reasons close the session immediately; unclean ones leave it
    /// linkdead for the grace period.
    fn is_clean(self) -> bool {
        matches!(self, DetachReason::Quit | DetachReason::Booted)
    }
}

/// What [`SessionManager::detach`] did with the session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetachOutcome {
    /// Other connections remain attached; the session is unaffected.
    OthersRemain,
    /// That was the last connection and the reason was unclean: the session
    /// is now linkdead. The driver should announce it and arrange to call
    /// [`SessionManager::take_expired`] after `grace` (see the crate docs —
    /// this crate runs no timers of its own).
    Linkdead {
        /// Configured grace period before the session may be reaped.
        grace: Duration,
    },
    /// The session closed (clean last-connection detach, or it was already
    /// gone).
    Closed,
}

/// A linkdead session reaped by [`SessionManager::take_expired`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpiredSession {
    /// The reaped session.
    pub sess: SessId,
    /// Its character (the driver tells the world to sweep it).
    pub character: CharKey,
    /// Its account name.
    pub account: String,
}

/// Result of a fan-out send ([`SessionManager::send_to_char`],
/// [`SessionManager::send_to_session`], [`SessionManager::broadcast_all`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SendReport {
    /// Connections the event was queued to.
    pub delivered: usize,
    /// Connections whose buffer was full (or transport gone): the event was
    /// dropped for them and their dropped flag is set. The caller may
    /// disconnect them (`detach(conn, DetachReason::SlowClient)`).
    pub dropped: Vec<ConnId>,
}

/// Result of a single-connection send ([`SessionManager::send_to_conn`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SendToConn {
    /// Queued to the connection.
    Delivered,
    /// Buffer full or transport gone; dropped flag set on the connection.
    Dropped,
    /// No such connection.
    Unknown,
}

/// One row of [`SessionManager::who`].
#[derive(Clone, Debug)]
pub struct SessionMeta {
    /// Session id.
    pub sess: SessId,
    /// Character being played.
    pub character: CharKey,
    /// Account name.
    pub account: String,
    /// Peer address (primary — earliest-attached — live connection; for a
    /// linkdead session, the last known address).
    pub addr: IpAddr,
    /// When the session's first connection attached.
    pub connected_at: Instant,
    /// Most recent input across all connections (idle = now − this).
    pub last_input: Instant,
    /// Number of live connections (0 while linkdead).
    pub conns: usize,
    /// Whether the session is currently linkdead.
    pub linkdead: bool,
}

#[derive(Clone, Copy, Debug)]
enum SessState {
    Online,
    Linkdead { since: Instant },
}

struct SessEntry {
    character: CharKey,
    account: String,
    /// Live connections in attach order; the first is the "primary" whose
    /// address WHO reports.
    conns: Vec<ConnId>,
    state: SessState,
    first_connected_at: Instant,
    /// Address of the most recently detached/attached connection — WHO's
    /// fallback while linkdead.
    last_addr: IpAddr,
    /// High-water input mark carried over from detached connections.
    last_input_snapshot: Instant,
}

struct ConnEntry {
    handle: Arc<ConnectionHandle>,
    sess: SessId,
    bucket: Mutex<FloodBucket>,
}

/// The session registry. Fully `Send + Sync`; every method is non-blocking
/// and none requires a tokio runtime (delivery uses `mpsc::Sender::try_send`
/// only). See the crate docs for the linkdead and slow-client contracts.
///
/// Lock discipline (DashMap shards): the only method that ever holds two
/// maps at once is `attach`, and acquisition order is always
/// `by_char` → `sessions` → `conns`; everything else holds at most one
/// shard at a time. Do not add code that acquires in a different order.
pub struct SessionManager {
    config: SessionConfig,
    sessions: DashMap<SessId, SessEntry>,
    conns: DashMap<ConnId, ConnEntry>,
    by_char: DashMap<CharKey, SessId>,
    next_sess: AtomicU64,
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    /// Manager with default config (300 s grace, 10 lines/s, burst 30).
    pub fn new() -> Self {
        Self::with_config(SessionConfig::default())
    }

    /// Manager with explicit tunables.
    pub fn with_config(config: SessionConfig) -> Self {
        Self {
            config,
            sessions: DashMap::new(),
            conns: DashMap::new(),
            by_char: DashMap::new(),
            next_sess: AtomicU64::new(1),
        }
    }

    /// The active configuration.
    pub fn config(&self) -> SessionConfig {
        self.config
    }

    // ---- attach / detach ------------------------------------------------

    /// Register an authenticated connection as playing `character` under
    /// `account`. Called by the transport after auth + character selection.
    ///
    /// If the character already has a session (multi-connect, or a linkdead
    /// reconnect) the connection joins it and the outcome says so; a
    /// linkdead session is revived. `mushd` guarantees a given `character`
    /// always arrives with the same `account`, so a rejoin keeps the
    /// session's existing account string.
    pub fn attach(
        &self,
        conn: ConnectionHandle,
        character: CharKey,
        account: String,
    ) -> (SessId, AttachOutcome) {
        let conn_id = conn.id;
        let addr = conn.addr;
        let attached_at = conn.connected_at;
        debug_assert!(
            !self.conns.contains_key(&conn_id),
            "attach: duplicate {conn_id}"
        );
        let handle = Arc::new(conn);
        let bucket = Mutex::new(FloodBucket::new(
            self.config.flood_rate,
            self.config.flood_burst,
            attached_at,
        ));

        // Lock order: by_char entry held across sessions/conns access — the
        // one sanctioned two-map hold (see the struct docs).
        match self.by_char.entry(character) {
            Entry::Occupied(mut occ) => {
                let sess = *occ.get();
                let joined = self.sessions.get_mut(&sess).map(|mut s| {
                    let from_linkdead = matches!(s.state, SessState::Linkdead { .. });
                    s.state = SessState::Online;
                    let others = s.conns.len();
                    s.conns.push(conn_id);
                    s.last_addr = addr;
                    (others, from_linkdead)
                });
                match joined {
                    Some((others, from_linkdead)) => {
                        self.conns.insert(
                            conn_id,
                            ConnEntry {
                                handle,
                                sess,
                                bucket,
                            },
                        );
                        (
                            sess,
                            AttachOutcome::Rejoined {
                                others,
                                from_linkdead,
                            },
                        )
                    }
                    None => {
                        // Stale index: the session was reaped between the
                        // by_char lookup and here. Start fresh in place.
                        let sess =
                            self.insert_session(character, account, addr, attached_at, conn_id);
                        occ.insert(sess);
                        self.conns.insert(
                            conn_id,
                            ConnEntry {
                                handle,
                                sess,
                                bucket,
                            },
                        );
                        (sess, AttachOutcome::New)
                    }
                }
            }
            Entry::Vacant(vac) => {
                let sess = self.insert_session(character, account, addr, attached_at, conn_id);
                vac.insert(sess);
                self.conns.insert(
                    conn_id,
                    ConnEntry {
                        handle,
                        sess,
                        bucket,
                    },
                );
                (sess, AttachOutcome::New)
            }
        }
    }

    fn insert_session(
        &self,
        character: CharKey,
        account: String,
        addr: IpAddr,
        attached_at: Instant,
        first_conn: ConnId,
    ) -> SessId {
        let sess = SessId(self.next_sess.fetch_add(1, Ordering::Relaxed));
        self.sessions.insert(
            sess,
            SessEntry {
                character,
                account,
                conns: vec![first_conn],
                state: SessState::Online,
                first_connected_at: attached_at,
                last_addr: addr,
                last_input_snapshot: attached_at,
            },
        );
        sess
    }

    /// Detach a connection "now". See [`SessionManager::detach_at`].
    pub fn detach(&self, conn: ConnId, reason: DetachReason) -> Option<DetachOutcome> {
        self.detach_at(conn, reason, Instant::now())
    }

    /// Detach a connection at an explicit instant (`now` becomes the
    /// linkdead epoch when this was the last connection). Returns `None` if
    /// the connection was not attached.
    pub fn detach_at(
        &self,
        conn: ConnId,
        reason: DetachReason,
        now: Instant,
    ) -> Option<DetachOutcome> {
        let (_, entry) = self.conns.remove(&conn)?;
        let sess = entry.sess;
        let mut close_char: Option<CharKey> = None;
        let outcome = match self.sessions.get_mut(&sess) {
            // Session already gone (concurrent close): nothing left to do.
            None => return Some(DetachOutcome::Closed),
            Some(mut s) => {
                s.conns.retain(|c| *c != conn);
                s.last_addr = entry.handle.addr;
                let li = entry.handle.last_input_at();
                if li > s.last_input_snapshot {
                    s.last_input_snapshot = li;
                }
                if !s.conns.is_empty() {
                    DetachOutcome::OthersRemain
                } else if reason.is_clean() {
                    close_char = Some(s.character);
                    DetachOutcome::Closed
                } else {
                    s.state = SessState::Linkdead { since: now };
                    DetachOutcome::Linkdead {
                        grace: self.config.linkdead_grace,
                    }
                }
            }
        };
        if let Some(character) = close_char {
            self.sessions.remove(&sess);
            self.by_char.remove_if(&character, |_, v| *v == sess);
        }
        Some(outcome)
    }

    /// Force-close a session: drop it (and its character mapping) and detach
    /// all its connections, returning their handles so the driver can close
    /// the transports. Empty if the session was unknown.
    pub fn close_session(&self, sess: SessId) -> Vec<Arc<ConnectionHandle>> {
        let Some((_, s)) = self.sessions.remove(&sess) else {
            return Vec::new();
        };
        self.by_char.remove_if(&s.character, |_, v| *v == sess);
        s.conns
            .iter()
            .filter_map(|c| self.conns.remove(c).map(|(_, e)| e.handle))
            .collect()
    }

    // ---- linkdead reaping (driver-polled; no internal timers) -----------

    /// Remove and return every linkdead session whose grace period has
    /// elapsed as of `now`. The driver calls this from its own clock (poll
    /// or a timer armed from [`SessionManager::next_linkdead_expiry`]) and
    /// then tells the world to sweep each returned character. A session that
    /// reconnected in the meantime is not returned — reattach cancels
    /// linkdead.
    pub fn take_expired(&self, now: Instant) -> Vec<ExpiredSession> {
        let grace = self.config.linkdead_grace;
        let expired = |state: SessState| match state {
            SessState::Linkdead { since } => now.saturating_duration_since(since) >= grace,
            SessState::Online => false,
        };
        let candidates: Vec<SessId> = self
            .sessions
            .iter()
            .filter(|r| expired(r.state))
            .map(|r| *r.key())
            .collect();
        let mut out = Vec::new();
        for sess in candidates {
            // Re-check under the removal lock: an attach may have revived it.
            let Some((_, s)) = self.sessions.remove_if(&sess, |_, e| expired(e.state)) else {
                continue;
            };
            self.by_char.remove_if(&s.character, |_, v| *v == sess);
            // Linkdead sessions hold no live connections; sweep defensively.
            for c in &s.conns {
                self.conns.remove(c);
            }
            out.push(ExpiredSession {
                sess,
                character: s.character,
                account: s.account,
            });
        }
        out
    }

    /// Earliest instant at which some linkdead session will expire, if any —
    /// lets the driver arm one timer instead of polling blind.
    pub fn next_linkdead_expiry(&self) -> Option<Instant> {
        let grace = self.config.linkdead_grace;
        self.sessions
            .iter()
            .filter_map(|r| match r.state {
                SessState::Linkdead { since } => Some(since + grace),
                SessState::Online => None,
            })
            .min()
    }

    // ---- delivery -------------------------------------------------------

    /// Fan an event out to every connection of `character`'s session (treaty
    /// ruling 2: the world resolves room → characters, this resolves
    /// character → connections). The `Arc` is cloned per connection; sends
    /// never block. A character with no session (or linkdead) reports zero
    /// deliveries.
    pub fn send_to_char(&self, character: CharKey, ev: Arc<OutputEvent>) -> SendReport {
        match self.by_char.get(&character).map(|r| *r) {
            Some(sess) => self.send_to_session(sess, ev),
            None => SendReport::default(),
        }
    }

    /// Fan an event out to every connection of one session.
    pub fn send_to_session(&self, sess: SessId, ev: Arc<OutputEvent>) -> SendReport {
        let conn_ids = self
            .sessions
            .get(&sess)
            .map(|s| s.conns.clone())
            .unwrap_or_default();
        let mut report = SendReport::default();
        for id in conn_ids {
            if let Some(e) = self.conns.get(&id) {
                if e.handle.try_send(Arc::clone(&ev)) {
                    report.delivered += 1;
                } else {
                    report.dropped.push(id);
                }
            }
        }
        report
    }

    /// Send to one specific connection (prompts, login banners, echoes).
    pub fn send_to_conn(&self, conn: ConnId, ev: Arc<OutputEvent>) -> SendToConn {
        match self.conns.get(&conn) {
            None => SendToConn::Unknown,
            Some(e) => {
                if e.handle.try_send(ev) {
                    SendToConn::Delivered
                } else {
                    SendToConn::Dropped
                }
            }
        }
    }

    /// Send to every attached connection (`@wall`, shutdown notices).
    pub fn broadcast_all(&self, ev: Arc<OutputEvent>) -> SendReport {
        let mut report = SendReport::default();
        for e in self.conns.iter() {
            if e.handle.try_send(Arc::clone(&ev)) {
                report.delivered += 1;
            } else {
                report.dropped.push(e.handle.id);
            }
        }
        report
    }

    // ---- idle + flood bookkeeping ---------------------------------------

    /// Record an input line on `conn` "now" (WHO idle times).
    pub fn record_input(&self, conn: ConnId) {
        self.record_input_at(conn, Instant::now());
    }

    /// Record an input line at an explicit instant.
    pub fn record_input_at(&self, conn: ConnId, now: Instant) {
        if let Some(e) = self.conns.get(&conn) {
            e.handle.record_input_at(now);
        }
    }

    /// Charge one input line against `conn`'s flood bucket "now".
    pub fn check_input(&self, conn: ConnId) -> FloodVerdict {
        self.check_input_at(conn, Instant::now())
    }

    /// Charge one input line at an explicit instant. Unknown connections are
    /// not throttled (there is nothing to protect; treat as a caller bug).
    pub fn check_input_at(&self, conn: ConnId, now: Instant) -> FloodVerdict {
        match self.conns.get(&conn) {
            Some(e) => e.bucket.lock().expect("flood bucket poisoned").check(now),
            None => FloodVerdict::Ok,
        }
    }

    // ---- queries --------------------------------------------------------

    /// One row per session (linkdead sessions included, with `conns == 0`),
    /// ordered by connect time. Feeds the WHO command; idle time is
    /// `now − last_input` at the caller.
    pub fn who(&self) -> Vec<SessionMeta> {
        let mut rows: Vec<SessionMeta> = self
            .sessions
            .iter()
            .map(|s| {
                let mut last_input = s.last_input_snapshot;
                let mut addr = s.last_addr;
                for (i, c) in s.conns.iter().enumerate() {
                    if let Some(e) = self.conns.get(c) {
                        let li = e.handle.last_input_at();
                        if li > last_input {
                            last_input = li;
                        }
                        if i == 0 {
                            addr = e.handle.addr;
                        }
                    }
                }
                SessionMeta {
                    sess: *s.key(),
                    character: s.character,
                    account: s.account.clone(),
                    addr,
                    connected_at: s.first_connected_at,
                    last_input,
                    conns: s.conns.len(),
                    linkdead: matches!(s.state, SessState::Linkdead { .. }),
                }
            })
            .collect();
        rows.sort_by_key(|m| (m.connected_at, m.sess.0));
        rows
    }

    /// The session a connection belongs to.
    pub fn session_of_conn(&self, conn: ConnId) -> Option<SessId> {
        self.conns.get(&conn).map(|e| e.sess)
    }

    /// The live (or linkdead) session for a character.
    pub fn session_of_char(&self, character: CharKey) -> Option<SessId> {
        self.by_char.get(&character).map(|r| *r)
    }

    /// The character a session is playing.
    pub fn character_of(&self, sess: SessId) -> Option<CharKey> {
        self.sessions.get(&sess).map(|s| s.character)
    }

    /// The shared handle for a connection.
    pub fn conn_handle(&self, conn: ConnId) -> Option<Arc<ConnectionHandle>> {
        self.conns.get(&conn).map(|e| Arc::clone(&e.handle))
    }

    /// Whether a session is currently linkdead.
    pub fn is_linkdead(&self, sess: SessId) -> bool {
        self.sessions
            .get(&sess)
            .is_some_and(|s| matches!(s.state, SessState::Linkdead { .. }))
    }

    /// Current capabilities of a connection.
    pub fn caps(&self, conn: ConnId) -> Option<ClientCaps> {
        self.conns.get(&conn).map(|e| e.handle.caps())
    }

    /// Replace a connection's capabilities (terminal resize). Returns
    /// whether the connection was known.
    pub fn update_caps(&self, conn: ConnId, caps: ClientCaps) -> bool {
        match self.conns.get(&conn) {
            Some(e) => {
                e.handle.set_caps(caps);
                true
            }
            None => false,
        }
    }

    /// Number of attached connections.
    pub fn connection_count(&self) -> usize {
        self.conns.len()
    }

    /// Number of sessions (linkdead included).
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}
