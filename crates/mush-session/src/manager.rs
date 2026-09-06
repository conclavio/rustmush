//! [`SessionManager`]: character → connections fan-out, envelope stamping +
//! resume ring buffers, attach/detach, linkdead, WHO, flood control.

use std::collections::VecDeque;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use mush_proto::{ClientCaps, Envelope, OutputEvent, ProjectedCommitToken};

use crate::conn::{ConnId, ConnectionHandle};
use crate::flood::{FloodBucket, FloodVerdict};

/// The wall-clock source for [`Envelope::ts`] stamps: unix **milliseconds**.
/// Injected at construction so tests drive a manual clock; production uses
/// [`system_unix_ms`].
pub type EnvelopeClock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// The default [`EnvelopeClock`]: `SystemTime::now()` as unix milliseconds.
#[must_use]
pub fn system_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

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
    /// Capacity of the per-session resume ring buffer (stamped envelopes
    /// retained for web `Resume { last_seq }` replay). Recording is
    /// **always on** for every session (chosen for simplicity over
    /// per-transport gating): worst-case memory is
    /// `resume_ring × (8-byte seq + Arc<Envelope>)` per session, where each
    /// retained [`Envelope`] owns a full clone of its event — ~512 small
    /// events per session, an acceptable cost at MUSH populations.
    pub resume_ring: usize,
    /// Maximum projected authoritative receipts retained until a structured
    /// client acknowledges their commit token. This is deliberately separate
    /// from the transport resume ring: a socket may advance its sequence and
    /// disappear before the application accepts the semantic result.
    pub receipt_retention: usize,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            linkdead_grace: Duration::from_secs(300),
            flood_rate: 10.0,
            flood_burst: 30.0,
            resume_ring: 512,
            receipt_retention: 128,
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

/// Why [`SessionManager::replay_since`] could not replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplayError {
    /// The gap exceeds the ring: at least one event after `last_seq` has
    /// already been evicted. The client must re-hydrate (fresh `Room` +
    /// panel snapshots) instead of replaying.
    TooOld,
    /// No such session (closed, reaped, or never existed).
    UnknownSession,
}

/// A page of [`SessionManager::backlog_matching`] — a bounded, newest-first
/// slice of a session's resume ring (interface plan I12).
#[derive(Clone, Debug, Default)]
pub struct NarrativeBacklog {
    /// The matched envelopes, newest-first, at most the requested `limit`.
    pub lines: Vec<Arc<Envelope>>,
    /// A further match exists below this page (the client may page again).
    pub more: bool,
    /// Older history may have rolled off the ring (conservative — the ring
    /// has evicted at least one envelope and this page reached its oldest
    /// retained match).
    pub evicted: bool,
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
    /// Highest envelope seq stamped for this session (0 = none yet). Lives
    /// on the session, not the connection, so seqs stay monotonic across
    /// reconnects and multi-connect.
    last_seq: u64,
    /// Resume ring: the newest [`SessionConfig::resume_ring`] session-scoped
    /// stamped envelopes, oldest first. Conn-scoped sends
    /// ([`SessionManager::send_to_conn`]) consume seqs but are **not**
    /// recorded — they must not replay to other/future connections; the
    /// resulting seq gaps are harmless (replay filters `seq > last_seq`).
    ring: VecDeque<(u64, Arc<Envelope>)>,
    /// Highest seq evicted from the ring (0 = nothing evicted yet) — the
    /// [`ReplayError::TooOld`] boundary.
    evicted_seq: u64,
    /// Structured presentation awaiting exact client acknowledgement,
    /// oldest first. Durable action status remains the long-horizon recovery
    /// source if this bounded cache rolls over.
    unacked_receipts: VecDeque<(ProjectedCommitToken, Arc<OutputEvent>)>,
}

struct ConnEntry {
    handle: Arc<ConnectionHandle>,
    sess: SessId,
    bucket: Mutex<FloodBucket>,
}

/// The invariant the interface plan I12 narrative backlog leans on: a
/// `Narrative` reaches the resume ring only through the M11-gated resolved-Char
/// fan-out (`send_to_char` / `send_event_to_char`), never a session- or
/// broadcast-scoped writer (whose producers all emit `Sys`). Enforced by
/// `debug_assert` on those writers so the leak proof's hand-audit becomes a
/// tripwire, mirroring the SP1 rule-1 allowlist precedent.
const NARRATIVE_RING_INVARIANT: &str = "Narrative must reach the resume ring only via the M11-gated Char fan-out, \
     never a session/broadcast writer (interface plan I12 leak invariant)";

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
    /// Wall-clock source for envelope `ts` stamps (injected; tests use a
    /// manual clock).
    clock: EnvelopeClock,
    sessions: DashMap<SessId, SessEntry>,
    conns: DashMap<ConnId, ConnEntry>,
    by_char: DashMap<CharKey, SessId>,
    /// Characters whose input currently bypasses the flood throttle (the
    /// world task marks actors sitting in an input context — multi-line
    /// pastes into mail compose must not lose body lines).
    flood_exempt: DashMap<CharKey, ()>,
    next_sess: AtomicU64,
    /// Called synchronously from [`SessionManager::detach`] when a detach
    /// leaves a session **linkdead** (its last connection dropped uncleanly).
    /// The one place every transport's drop passes through, so a driver that
    /// wants to react — the Discord bridge's auto-follow (D-D-4) — hooks
    /// here rather than into each bridge. Must not block; a driver spawns.
    linkdead_hook: Mutex<Option<LinkdeadHook>>,
}

/// A driver's linkdead observer: `(character, grace)` — see
/// [`SessionManager::set_linkdead_hook`].
pub type LinkdeadHook = Arc<dyn Fn(CharKey, Duration) + Send + Sync>;

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManager {
    /// Manager with default config (300 s grace, 10 lines/s, burst 30,
    /// 512-envelope resume rings) and the system clock.
    pub fn new() -> Self {
        Self::with_config(SessionConfig::default())
    }

    /// Manager with explicit tunables and the system clock.
    pub fn with_config(config: SessionConfig) -> Self {
        Self::with_config_and_clock(config, system_unix_ms)
    }

    /// Manager with explicit tunables and an injected [`EnvelopeClock`]
    /// (unix milliseconds) — deterministic `ts` stamps for tests.
    pub fn with_config_and_clock(
        config: SessionConfig,
        clock: impl Fn() -> i64 + Send + Sync + 'static,
    ) -> Self {
        Self {
            config,
            clock: Arc::new(clock),
            sessions: DashMap::new(),
            conns: DashMap::new(),
            by_char: DashMap::new(),
            flood_exempt: DashMap::new(),
            next_sess: AtomicU64::new(1),
            linkdead_hook: Mutex::new(None),
        }
    }

    /// The active configuration.
    pub fn config(&self) -> SessionConfig {
        self.config
    }

    /// Install (or replace) the linkdead observer: called synchronously,
    /// inside [`SessionManager::detach`], with the character whose last
    /// connection just dropped uncleanly and the grace before it is reaped.
    /// It must return promptly — a driver that wants to do I/O spawns.
    pub fn set_linkdead_hook(&self, hook: LinkdeadHook) {
        *self.linkdead_hook.lock().expect("linkdead hook poisoned") = Some(hook);
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
                    // Interface plan I12: a rejoin can read the whole resume
                    // ring (the narrative backlog), so a second principal must
                    // never attach to another account's session. `mushd`
                    // guarantees this by construction (append-only accounts,
                    // no character transfer); assert it so a future transfer
                    // path re-checks the entitlement at this seam.
                    debug_assert_eq!(
                        s.account, account,
                        "attach: rejoin by a different account (I12 ring entitlement)"
                    );
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
                last_seq: 0,
                ring: VecDeque::new(),
                evicted_seq: 0,
                unacked_receipts: VecDeque::new(),
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
        let mut linkdead_char: Option<CharKey> = None;
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
                    linkdead_char = Some(s.character);
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
        // Observe AFTER every map borrow above is released: the hook may
        // call back into this manager (a driver attaching a replacement
        // connection) without deadlocking on a shard.
        if let (DetachOutcome::Linkdead { grace }, Some(character)) = (outcome, linkdead_char) {
            let hook = self
                .linkdead_hook
                .lock()
                .expect("linkdead hook poisoned")
                .clone();
            if let Some(hook) = hook {
                hook(character, grace);
            }
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
    //
    // Every session-scoped send funnels through `deliver_to_session`, the
    // single stamping path: it allocates the session's next seq, stamps an
    // Envelope (seq + ts from the injected clock + a clone of the event),
    // records it in the resume ring, and fans out — the plain
    // Arc<OutputEvent> to ANSI connections, the Arc<Envelope> to structured
    // ones. One semantic event, two renders (treaty ruling 1); the ring is
    // recorded for every session, live connections or not, so output during
    // web linkdead replays on resume.

    /// Fan an event out to every connection of `character`'s session (treaty
    /// ruling 2: the world resolves room → characters, this resolves
    /// character → connections). The `Arc` is cloned per connection; sends
    /// never block. A character with no session reports zero deliveries; a
    /// linkdead session delivers to nobody but still records the resume
    /// ring.
    pub fn send_to_char(&self, character: CharKey, ev: Arc<OutputEvent>) -> SendReport {
        match self.by_char.get(&character).map(|r| *r) {
            Some(sess) => self.deliver_to_session(sess, &ev),
            None => SendReport::default(),
        }
    }

    /// Fan an event out to every connection of one session.
    pub fn send_to_session(&self, sess: SessId, ev: Arc<OutputEvent>) -> SendReport {
        debug_assert!(
            !matches!(*ev, OutputEvent::Narrative { .. }),
            "{NARRATIVE_RING_INVARIANT}"
        );
        self.deliver_to_session(sess, &ev)
    }

    /// Owned-event convenience over [`send_to_char`](Self::send_to_char):
    /// the world task's primary send path. Stamps seq/ts into an
    /// [`Envelope`] for the ring + structured connections and hands the
    /// plain [`OutputEvent`] to ANSI-rendering connections.
    pub fn send_event_to_char(&self, character: CharKey, event: OutputEvent) -> SendReport {
        self.send_to_char(character, Arc::new(event))
    }

    /// Owned-event convenience over
    /// [`send_to_session`](Self::send_to_session).
    pub fn send_event_to_session(&self, sess: SessId, event: OutputEvent) -> SendReport {
        debug_assert!(
            !matches!(event, OutputEvent::Narrative { .. }),
            "{NARRATIVE_RING_INVARIANT}"
        );
        self.deliver_to_session(sess, &Arc::new(event))
    }

    /// Stamp + ring + fan out one session-scoped event (see the section
    /// note above). Unknown sessions report zero deliveries.
    fn deliver_to_session(&self, sess: SessId, ev: &Arc<OutputEvent>) -> SendReport {
        // Stamp under the session shard lock, then release it before
        // touching `conns` (one map at a time — see the struct docs).
        let stamped = {
            let Some(mut s) = self.sessions.get_mut(&sess) else {
                return SendReport::default();
            };
            s.last_seq += 1;
            let seq = s.last_seq;
            let env = Arc::new(Envelope::new(seq, (self.clock)(), (**ev).clone()));
            s.ring.push_back((seq, Arc::clone(&env)));
            while s.ring.len() > self.config.resume_ring {
                if let Some((evicted, _)) = s.ring.pop_front() {
                    s.evicted_seq = evicted;
                }
            }
            (env, s.conns.clone())
        };
        let (env, conn_ids) = stamped;
        let mut report = SendReport::default();
        for id in conn_ids {
            if let Some(e) = self.conns.get(&id) {
                let sent = if e.handle.is_structured() {
                    e.handle.try_send_envelope(Arc::clone(&env))
                } else {
                    e.handle.try_send(Arc::clone(ev))
                };
                if sent {
                    report.delivered += 1;
                } else {
                    report.dropped.push(id);
                }
            }
        }
        report
    }

    /// Send to one specific connection (prompts, login banners, echoes).
    ///
    /// A structured connection still receives a stamped [`Envelope`] (its
    /// wire carries nothing else), but conn-scoped envelopes are **not**
    /// recorded in the resume ring — a prompt addressed to one connection
    /// must not replay to the session's other or future connections. The
    /// seq gap this leaves in the ring is harmless.
    pub fn send_to_conn(&self, conn: ConnId, ev: Arc<OutputEvent>) -> SendToConn {
        let Some(e) = self.conns.get(&conn) else {
            return SendToConn::Unknown;
        };
        if !e.handle.is_structured() {
            return if e.handle.try_send(ev) {
                SendToConn::Delivered
            } else {
                SendToConn::Dropped
            };
        }
        let sess = e.sess;
        let handle = Arc::clone(&e.handle);
        // Release `conns` before touching `sessions` (one map at a time).
        drop(e);
        let Some(env) = self.stamp_conn_scoped(sess, &ev) else {
            // Session vanished mid-teardown; the connection is going away
            // with it.
            return SendToConn::Unknown;
        };
        if handle.try_send_envelope(env) {
            SendToConn::Delivered
        } else {
            SendToConn::Dropped
        }
    }

    /// Deliver one authoritative result to a connection and, for a
    /// structured sink, retain it by exact commit token until acknowledged.
    ///
    /// Duplicate status lookup or replay consumes no additional retention
    /// slot. ANSI connections use the ordinary best-effort path and retain
    /// nothing because their protocol has no ACK.
    pub fn send_acknowledgeable_to_conn(
        &self,
        conn: ConnId,
        commit: ProjectedCommitToken,
        ev: Arc<OutputEvent>,
    ) -> SendToConn {
        let Some(entry) = self.conns.get(&conn) else {
            return SendToConn::Unknown;
        };
        if !entry.handle.is_structured() {
            return if entry.handle.try_send(ev) {
                SendToConn::Delivered
            } else {
                SendToConn::Dropped
            };
        }
        let sess = entry.sess;
        let handle = Arc::clone(&entry.handle);
        drop(entry);

        let env = {
            let Some(mut session) = self.sessions.get_mut(&sess) else {
                return SendToConn::Unknown;
            };
            Self::retain_receipt(
                &mut session,
                sess,
                self.config.receipt_retention,
                commit,
                Arc::clone(&ev),
            );
            session.last_seq += 1;
            Arc::new(Envelope::new(
                session.last_seq,
                (self.clock)(),
                (*ev).clone(),
            ))
        };
        if handle.try_send_envelope(env) {
            SendToConn::Delivered
        } else {
            SendToConn::Dropped
        }
    }

    /// Retain a projected receipt for a character whose originating
    /// connection has already disappeared. Returns `false` when the session
    /// was closed/reaped; durable `action/status` is then the recovery path.
    pub fn retain_acknowledgeable_for_char(
        &self,
        character: CharKey,
        commit: ProjectedCommitToken,
        ev: Arc<OutputEvent>,
    ) -> bool {
        let Some(sess) = self.by_char.get(&character).map(|entry| *entry) else {
            return false;
        };
        let Some(mut session) = self.sessions.get_mut(&sess) else {
            return false;
        };
        Self::retain_receipt(
            &mut session,
            sess,
            self.config.receipt_retention,
            commit,
            ev,
        );
        true
    }

    fn retain_receipt(
        session: &mut SessEntry,
        sess: SessId,
        capacity: usize,
        commit: ProjectedCommitToken,
        ev: Arc<OutputEvent>,
    ) {
        if capacity == 0
            || session
                .unacked_receipts
                .iter()
                .any(|(retained, _)| retained == &commit)
        {
            return;
        }
        while session.unacked_receipts.len() >= capacity {
            if let Some((evicted, _)) = session.unacked_receipts.pop_front() {
                tracing::warn!(
                    session = sess.0,
                    commit = %evicted,
                    "unacknowledged receipt retention full; durable action/status is required"
                );
            }
        }
        session.unacked_receipts.push_back((commit, ev));
    }

    /// Retire one exact structured presentation receipt for `conn`'s
    /// session. Unknown, duplicate, and cross-session ACKs are harmless
    /// no-ops and reveal no receipt existence.
    pub fn acknowledge_receipt(&self, conn: ConnId, commit: &ProjectedCommitToken) -> bool {
        let Some(entry) = self.conns.get(&conn) else {
            return false;
        };
        if !entry.handle.is_structured() {
            return false;
        }
        let sess = entry.sess;
        drop(entry);
        let Some(mut session) = self.sessions.get_mut(&sess) else {
            return false;
        };
        let before = session.unacked_receipts.len();
        session
            .unacked_receipts
            .retain(|(retained, _)| retained != commit);
        session.unacked_receipts.len() != before
    }

    /// Re-emit every still-unacknowledged semantic receipt to one structured
    /// connection with fresh monotonically increasing envelope sequences.
    /// Frames are not added to the ordinary resume ring: this ACK-gated cache
    /// is their replay authority. A full sink stops the bounded pass and
    /// leaves every receipt retained for the next reconnect.
    pub fn redeliver_unacknowledged(&self, conn: ConnId) -> SendReport {
        let Some(entry) = self.conns.get(&conn) else {
            return SendReport::default();
        };
        if !entry.handle.is_structured() {
            return SendReport::default();
        }
        let sess = entry.sess;
        let handle = Arc::clone(&entry.handle);
        drop(entry);
        let envelopes = {
            let Some(mut session) = self.sessions.get_mut(&sess) else {
                return SendReport::default();
            };
            let events: Vec<Arc<OutputEvent>> = session
                .unacked_receipts
                .iter()
                .map(|(_, event)| Arc::clone(event))
                .collect();
            events
                .into_iter()
                .map(|event| {
                    session.last_seq += 1;
                    Arc::new(Envelope::new(
                        session.last_seq,
                        (self.clock)(),
                        (*event).clone(),
                    ))
                })
                .collect::<Vec<_>>()
        };
        let mut report = SendReport::default();
        for envelope in envelopes {
            if handle.try_send_envelope(envelope) {
                report.delivered += 1;
            } else {
                report.dropped.push(conn);
                break;
            }
        }
        report
    }

    /// Number of projected receipts still awaiting an ACK in `sess`.
    #[must_use]
    pub fn unacknowledged_receipt_count(&self, sess: SessId) -> Option<usize> {
        self.sessions
            .get(&sess)
            .map(|session| session.unacked_receipts.len())
    }

    /// Allocate the session's next seq and stamp an envelope **without**
    /// ring recording (see [`send_to_conn`](Self::send_to_conn)).
    fn stamp_conn_scoped(&self, sess: SessId, ev: &Arc<OutputEvent>) -> Option<Arc<Envelope>> {
        let mut s = self.sessions.get_mut(&sess)?;
        s.last_seq += 1;
        Some(Arc::new(Envelope::new(
            s.last_seq,
            (self.clock)(),
            (**ev).clone(),
        )))
    }

    /// Send to every attached connection (`@wall`, shutdown notices).
    /// Stamped + ring-recorded per session like any session-scoped send.
    pub fn broadcast_all(&self, ev: Arc<OutputEvent>) -> SendReport {
        debug_assert!(
            !matches!(*ev, OutputEvent::Narrative { .. }),
            "{NARRATIVE_RING_INVARIANT}"
        );
        // Collect ids first: `deliver_to_session` takes the same shards
        // `iter()` would hold.
        let sessions: Vec<SessId> = self.sessions.iter().map(|s| *s.key()).collect();
        let mut report = SendReport::default();
        for sess in sessions {
            let r = self.deliver_to_session(sess, &ev);
            report.delivered += r.delivered;
            report.dropped.extend(r.dropped);
        }
        report
    }

    // ---- resume replay --------------------------------------------------

    /// The stamped envelopes a reconnecting structured client is missing:
    /// every **ring-recorded** envelope with `seq > last_seq`, oldest
    /// first. `Err(TooOld)` when the gap exceeds the ring (an event after
    /// `last_seq` was already evicted) — the client must re-hydrate. A
    /// `last_seq` exactly at the newest stamp replays nothing.
    ///
    /// Also `Err(TooOld)` when `last_seq` is **higher** than anything this
    /// session ever stamped (HIGH finding): per-session seqs restart at 0,
    /// so a client resuming with a seq remembered from a *prior*
    /// incarnation of this character's session (that session was reaped
    /// and a fresh one now exists under the same character) would
    /// otherwise pass the `evicted_seq` check and get `Ok(empty)` — the
    /// client reads that as "already caught up" and silently loses every
    /// event the new session ever stamps. A legitimate client can never
    /// exceed `last_seq`: conn-scoped sends
    /// ([`SessionManager::send_to_conn`]) advance it without ring-recording,
    /// but they still advance it.
    pub fn replay_since(
        &self,
        sess: SessId,
        last_seq: u64,
    ) -> Result<Vec<Arc<Envelope>>, ReplayError> {
        let s = self
            .sessions
            .get(&sess)
            .ok_or(ReplayError::UnknownSession)?;
        if last_seq < s.evicted_seq || last_seq > s.last_seq {
            return Err(ReplayError::TooOld);
        }
        Ok(s.ring
            .iter()
            .filter(|(seq, _)| *seq > last_seq)
            .map(|(_, env)| Arc::clone(env))
            .collect())
    }

    /// A bounded, newest-first slice of this session's own resume ring,
    /// filtered by `keep` (interface plan I12 — the narrative backlog reuses
    /// the per-viewer, already-gated resume ring: every ring envelope arrived
    /// as a `SendTarget::Char` the world had already run through
    /// exclude/@ignore/perceive, so no re-gating is needed). Read-only: never
    /// touches `last_seq`/`evicted_seq`.
    ///
    /// Pages by the per-session envelope seq: entries with `seq < before_seq`
    /// (or all, when `None`), matching `keep`, newest-first, at most `limit`.
    /// `more` is set when a further match exists below the returned page;
    /// `evicted` is a conservative "older history may have rolled off" flag
    /// (true when the ring has evicted anything and this page reached the
    /// oldest retained match) — monotonic `evicted_seq`, so never a false
    /// negative. An unknown/reaped session yields `Err(UnknownSession)`; the
    /// gateway answers that reap race with an empty page (re-hydration safe).
    /// A connection with no session at all never reaches here — the bridge
    /// drops the query, as it does every unattached query.
    pub fn backlog_matching(
        &self,
        sess: SessId,
        before_seq: Option<u64>,
        limit: usize,
        keep: impl Fn(&OutputEvent) -> bool,
    ) -> Result<NarrativeBacklog, ReplayError> {
        let s = self
            .sessions
            .get(&sess)
            .ok_or(ReplayError::UnknownSession)?;
        // A zero limit would return `lines = []` with `more = true` — a
        // spurious empty terminal page. The bridge clamps to 1..=200; guard
        // here too for any future caller.
        let limit = limit.max(1);
        let mut lines: Vec<Arc<Envelope>> = Vec::new();
        for (seq, env) in s.ring.iter().rev() {
            if before_seq.is_some_and(|b| *seq >= b) {
                continue;
            }
            if keep(&env.event) {
                if lines.len() == limit {
                    // A further match exists below this page.
                    return Ok(NarrativeBacklog {
                        lines,
                        more: true,
                        evicted: false,
                    });
                }
                lines.push(Arc::clone(env));
            }
        }
        Ok(NarrativeBacklog {
            lines,
            more: false,
            evicted: s.evicted_seq > 0,
        })
    }

    /// Highest envelope seq stamped for a session so far (0 = none) — what
    /// a fresh structured connect reports as its resume baseline.
    pub fn last_seq(&self, sess: SessId) -> Option<u64> {
        self.sessions.get(&sess).map(|s| s.last_seq)
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
    /// Connections of a [flood-exempt](SessionManager::set_flood_exempt)
    /// character pass without charging their bucket.
    pub fn check_input_at(&self, conn: ConnId, now: Instant) -> FloodVerdict {
        // Lock discipline: never hold `conns` while touching `sessions` —
        // each map is acquired and released one at a time here.
        if !self.flood_exempt.is_empty() {
            let sess = self.conns.get(&conn).map(|e| e.sess);
            let character = sess.and_then(|s| self.sessions.get(&s).map(|s| s.character));
            if character.is_some_and(|c| self.flood_exempt.contains_key(&c)) {
                return FloodVerdict::Ok;
            }
        }
        match self.conns.get(&conn) {
            Some(e) => e.bucket.lock().expect("flood bucket poisoned").check(now),
            None => FloodVerdict::Ok,
        }
    }

    /// [`check_query_flood`](Self::check_query_flood) at an explicit instant,
    /// so the strict bucket is testable with a manual clock like
    /// [`check_input_at`](Self::check_input_at).
    pub fn check_query_flood_at(&self, conn: ConnId, now: Instant) -> FloodVerdict {
        match self.conns.get(&conn) {
            Some(e) => e.bucket.lock().expect("flood bucket poisoned").check(now),
            None => FloodVerdict::Ok,
        }
    }

    /// Like [`check_input`](Self::check_input) but **does not honour
    /// flood-exempt**: a flood-exempt character (mail compose, chargen) still
    /// has its bucket charged. Used by locally-answered gateway queries
    /// (interface plan I12 narrative backlog) that have no world-side
    /// backpressure — the exempt exists to protect composed input *lines*, not
    /// background panel pulls, so a mail-composing connection must not be able
    /// to fire unthrottled ring scans.
    pub fn check_query_flood(&self, conn: ConnId) -> FloodVerdict {
        match self.conns.get(&conn) {
            Some(e) => e
                .bucket
                .lock()
                .expect("flood bucket poisoned")
                .check(Instant::now()),
            None => FloodVerdict::Ok,
        }
    }

    /// Whether `character` is currently [flood-exempt](Self::set_flood_exempt)
    /// (sitting in an input context — mail compose, chargen). The split
    /// gateway's M18 input-shed consults this so an input-context paste is never
    /// shed, mirroring how [`check_input_at`](Self::check_input_at) lets it
    /// bypass the throttle.
    #[must_use]
    pub fn is_char_flood_exempt(&self, character: CharKey) -> bool {
        !self.flood_exempt.is_empty() && self.flood_exempt.contains_key(&character)
    }

    /// Mark (or clear) a character as flood-exempt: their input lines
    /// bypass the per-connection throttle while set. The world task sets
    /// this while the character sits in an input context (mail compose,
    /// chargen) — multi-line pastes are exactly what those contexts exist
    /// for, and a throttled line would silently vanish from the composed
    /// body. The flag is keyed by character, so it survives reconnects the
    /// way the input-context stack itself does.
    pub fn set_flood_exempt(&self, character: CharKey, exempt: bool) {
        if exempt {
            self.flood_exempt.insert(character, ());
        } else {
            self.flood_exempt.remove(&character);
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

    /// The live connection handles attached to a session, in attach order;
    /// empty if the session is unknown (or linkdead — no connections).
    /// Lets drivers act on a *subset* of a session's connections (e.g.
    /// `@newpassword` boots the structured/web ones and leaves SSH alone)
    /// where [`SessionManager::close_session`] would take all of them.
    #[must_use]
    pub fn conns_of_session(&self, sess: SessId) -> Vec<Arc<ConnectionHandle>> {
        // Snapshot the id list first, then resolve handles — never hold a
        // `sessions` ref while touching `conns` (see the struct docs on
        // lock order).
        let ids: Vec<ConnId> = match self.sessions.get(&sess) {
            Some(s) => s.conns.clone(),
            None => return Vec::new(),
        };
        ids.into_iter()
            .filter_map(|c| self.conns.get(&c).map(|e| Arc::clone(&e.handle)))
            .collect()
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
