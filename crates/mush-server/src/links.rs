//! The server's links to its gateways (doc 33 §4.3, cut GG.2).
//!
//! Until GG.2 the server accepted ONE gateway and lived as long as that link:
//! a gateway EOF was the server's cue to shut the world down. Now the accept
//! is a loop, every accepted gateway is a [`Link`] with its own reader, writer,
//! Hello, negotiated version and bounded outbox, and the server outlives any
//! of them: a link's end detaches that link's connections (each as if it had
//! sent `ConnDown { reason: Lost }`) and nothing else. The operator stops the
//! server with `systemctl stop` or the in-game shutdown.
//!
//! The world keeps its connection map keyed by connection id alone; THIS
//! module keeps the link layer's two maps beside it — link → the connections
//! it owns, connection → the link that owns it (and the actor it plays) — so
//! that
//!
//! - a connection-addressed frame (`Deliver` to [`SendTarget::Conn`],
//!   `SetOutputBrackets`, `Detach`) goes to the owning link only, while a
//!   character- or world-addressed one goes to every link (the gateway that
//!   holds no connection of the character ignores it);
//! - a link's EOF knows exactly which connections to detach;
//! - a connection id announced on two live links at once is refused on the
//!   second (§4.3: closed on the newer link, logged with both links, counted),
//!   because the world could not tell the two apart.
//!
//! The registry orders lifecycle bookkeeping against the world: every claim,
//! release and link close takes the registry's [`lifecycle`](LinkRegistry::lifecycle)
//! lock around BOTH its table update and the world send it implies, so a link
//! that ends while another re-announces the same connection cannot interleave
//! a `ConnUp` and a `ConnDown` for it in the wrong order.
//!
//! [`SendTarget::Conn`]: mush_ipc::SendTarget::Conn

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use mush_config::metrics::Metrics;
use mush_core::ObjId;
use mush_ipc::{LinkVersion, ToGateway};

use crate::gateway_sink::LinkSink;

/// A link's number: unique for the life of this server process, allocated
/// from 1 in accept order. Appears in the logs beside the connection ids.
pub type LinkId = u64;

/// The fixed marker an operator (or an alert rule) greps the server log for
/// when two live links announced the same connection id (doc 33 §4.3): two
/// gateways of one generation, or a `--generation` left off one of them. The
/// refusal line carries both link numbers and the id. Do not reword it.
pub const IPC_DUPLICATE_CONN_ID_MARKER: &str = "IPC-DUPLICATE-CONN-ID";

/// What [`LinkRegistry::claim`] decided for a connection a link announced.
#[derive(Debug, PartialEq, Eq)]
pub enum Claim {
    /// The connection is this link's (newly, or already).
    Owned,
    /// Another live link owns the id: the announce is refused, and the caller
    /// closes the connection on the announcing link.
    Duplicate {
        /// The link that owns the id.
        owner: LinkId,
    },
}

/// What [`LinkRegistry::release`] found for a connection a link let go of.
#[derive(Debug, PartialEq, Eq)]
pub enum Release {
    /// The connection was this link's; it is nobody's now.
    Owned,
    /// Another link owns the id: the release is NOT the owner's and must not
    /// reach the world as one.
    OtherLink(LinkId),
    /// No link owns the id (a connection the server never saw come up).
    Unknown,
}

/// What a closed link leaves behind, for the caller to tell the world.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Detached {
    /// Every connection the link still owned, with the actor each played:
    /// each one goes down as `Lost`.
    pub conns: Vec<(u64, ObjId)>,
    /// Every actor the link owed a `SessionGone` for (one of its connections
    /// went down `Lost` and its gateway's grace timer had not fired) plus the
    /// actors of `conns`: the server reaps each after the linkdead grace,
    /// since the gateway that would have is gone. The world ignores a stale
    /// one (the actor reconnected elsewhere).
    pub actors: Vec<ObjId>,
}

/// One link's entry in the table.
struct LinkEntry {
    sink: LinkSink,
    version: LinkVersion,
    conns: HashSet<u64>,
    /// Actors whose connection on this link went down `Lost`, until the link
    /// sends their `SessionGone` or a new `ConnUp` for them.
    linkdead: HashSet<ObjId>,
}

/// Who owns a connection id.
#[derive(Clone, Copy)]
struct Owner {
    link: LinkId,
    actor: ObjId,
}

#[derive(Default)]
struct Table {
    links: HashMap<LinkId, LinkEntry>,
    owners: HashMap<u64, Owner>,
}

/// The server's table of live links and the connections each owns. One per
/// server process, shared by the accept loop, every link's reader and the
/// world's [`SocketSink`](crate::gateway_sink::SocketSink).
pub struct LinkRegistry {
    table: Mutex<Table>,
    /// Orders lifecycle bookkeeping against the world (module docs). An async
    /// lock: it is held across the world send that follows the table update.
    lifecycle: tokio::sync::Mutex<()>,
    metrics: Arc<Metrics>,
    next_link: AtomicU64,
}

impl LinkRegistry {
    /// An empty registry over the process's metrics registry (the link gauge
    /// and the duplicate counter land there).
    #[must_use]
    pub fn new(metrics: Arc<Metrics>) -> LinkRegistry {
        LinkRegistry {
            table: Mutex::new(Table::default()),
            lifecycle: tokio::sync::Mutex::new(()),
            metrics,
            next_link: AtomicU64::new(1),
        }
    }

    fn table(&self) -> MutexGuard<'_, Table> {
        self.table.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The lock that orders claims, releases and link closes against the
    /// world sends they imply. Held by the reader around a lifecycle frame's
    /// bookkeeping AND its forward to the world, and by the link supervisor
    /// around a close and the `ConnDown`s it sends.
    pub fn lifecycle(&self) -> &tokio::sync::Mutex<()> {
        &self.lifecycle
    }

    /// Register a freshly accepted gateway: its outbox and its version cell.
    /// The returned [`Link`] is what the link's reader carries.
    pub fn open_link(self: &Arc<Self>, sink: LinkSink, version: LinkVersion) -> Link {
        let id = self.next_link.fetch_add(1, Ordering::Relaxed);
        self.table().links.insert(
            id,
            LinkEntry {
                sink: sink.clone(),
                version: version.clone(),
                conns: HashSet::new(),
                linkdead: HashSet::new(),
            },
        );
        self.metrics.ipc_link_opened();
        tracing::info!(link = id, links = self.links(), "gateway link opened");
        Link {
            id,
            registry: Arc::clone(self),
            sink,
            version,
        }
    }

    /// A link announced `conn` playing `actor` (`ConnUp`, `Attach`, or one
    /// record of a `ReAnnounce`). The same link announcing a connection it
    /// already owns re-points it (the world re-attaches a reused id the same
    /// way); another live link owning it is the duplicate §4.3 refuses.
    pub fn claim(&self, link: LinkId, conn: u64, actor: ObjId) -> Claim {
        let mut t = self.table();
        if let Some(owner) = t.owners.get(&conn)
            && owner.link != link
            && t.links.contains_key(&owner.link)
        {
            let owner = owner.link;
            self.metrics.ipc_duplicate_conn_id();
            return Claim::Duplicate { owner };
        }
        t.owners.insert(conn, Owner { link, actor });
        if let Some(entry) = t.links.get_mut(&link) {
            entry.conns.insert(conn);
            entry.linkdead.remove(&actor);
        }
        Claim::Owned
    }

    /// A link let go of `conn` (`ConnDown`, `Detach`).
    pub fn release(&self, link: LinkId, conn: u64) -> Release {
        let mut t = self.table();
        match t.owners.get(&conn).copied() {
            None => Release::Unknown,
            Some(owner) if owner.link != link => Release::OtherLink(owner.link),
            Some(_) => {
                t.owners.remove(&conn);
                if let Some(entry) = t.links.get_mut(&link) {
                    entry.conns.remove(&conn);
                }
                Release::Owned
            }
        }
    }

    /// A link's connection of `actor` went down `Lost`: the link's gateway
    /// owes the world a `SessionGone` after its grace, which the server sends
    /// in its stead should the link end first.
    pub fn note_linkdead(&self, link: LinkId, actor: ObjId) {
        if let Some(entry) = self.table().links.get_mut(&link) {
            entry.linkdead.insert(actor);
        }
    }

    /// The link's gateway sent the `SessionGone` it owed for `actor`.
    pub fn clear_linkdead(&self, link: LinkId, actor: ObjId) {
        if let Some(entry) = self.table().links.get_mut(&link) {
            entry.linkdead.remove(&actor);
        }
    }

    /// A link ended: forget it and release everything it owned. `None` for a
    /// link already closed (the supervisor's two ends can both arrive).
    pub fn close(&self, link: LinkId) -> Option<Detached> {
        let mut t = self.table();
        let entry = t.links.remove(&link)?;
        let mut detached = Detached::default();
        let mut actors: HashSet<ObjId> = entry.linkdead;
        for conn in entry.conns {
            // Only connections this link still owns: a claim that moved the
            // id to another link (impossible while both live, see `claim`)
            // is not this link's to detach.
            let Some(owner) = t.owners.get(&conn).copied() else {
                continue;
            };
            if owner.link != link {
                continue;
            }
            t.owners.remove(&conn);
            detached.conns.push((conn, owner.actor));
            actors.insert(owner.actor);
        }
        drop(t);
        detached.conns.sort_unstable();
        detached.actors = actors.into_iter().collect();
        detached.actors.sort_unstable();
        self.metrics.ipc_link_closed(detached.conns.len() as u64);
        tracing::info!(
            link,
            detached = detached.conns.len(),
            links = self.links(),
            "gateway link closed"
        );
        Some(detached)
    }

    /// How many links are open.
    #[must_use]
    pub fn links(&self) -> usize {
        self.table().links.len()
    }

    /// The link that owns `conn`, if any live one does.
    #[must_use]
    pub fn owner_of(&self, conn: u64) -> Option<LinkId> {
        let t = self.table();
        t.owners
            .get(&conn)
            .map(|o| o.link)
            .filter(|l| t.links.contains_key(l))
    }

    /// The version each open link negotiated, by link number.
    #[must_use]
    pub fn versions(&self) -> Vec<(LinkId, Option<u32>)> {
        let t = self.table();
        let mut v: Vec<_> = t
            .links
            .iter()
            .map(|(id, e)| (*id, e.version.get()))
            .collect();
        v.sort_unstable();
        v
    }

    /// Queue `msg` on the link that owns `conn`. `false` when no live link
    /// does (the frame is dropped: there is no connection to deliver it to).
    pub fn send_to_conn(&self, conn: u64, msg: ToGateway) -> bool {
        let sink = {
            let t = self.table();
            t.owners
                .get(&conn)
                .and_then(|o| t.links.get(&o.link))
                .map(|e| e.sink.clone())
        };
        match sink {
            Some(sink) => {
                sink.send(msg);
                true
            }
            None => false,
        }
    }

    /// Queue `msg` on one link. `false` when the link is gone.
    pub fn send_to_link(&self, link: LinkId, msg: ToGateway) -> bool {
        let sink = self.table().links.get(&link).map(|e| e.sink.clone());
        match sink {
            Some(sink) => {
                sink.send(msg);
                true
            }
            None => false,
        }
    }

    /// Queue `msg` on every open link. Returns how many took it.
    pub fn broadcast(&self, msg: &ToGateway) -> usize {
        let sinks: Vec<LinkSink> = self
            .table()
            .links
            .values()
            .map(|e| e.sink.clone())
            .collect();
        for sink in &sinks {
            sink.send(msg.clone());
        }
        sinks.len()
    }

    /// Forget every link without detaching anything: the shutdown tail. Each
    /// link's outbox sender drops with its entry, so every writer sees its
    /// channel close once the world's sink is gone too, sends `Bye`, and ends.
    pub fn drain(&self) {
        let mut t = self.table();
        let n = t.links.len();
        t.links.clear();
        t.owners.clear();
        drop(t);
        for _ in 0..n {
            self.metrics.ipc_link_closed(0);
        }
    }
}

/// One gateway's link, as its reader carries it: the link's number, its
/// bounded outbox (the reply path for correlated requests and dispositions,
/// so a reply goes back on the link that asked), its version cell (GG.0's
/// per-link gate) and the registry the bookkeeping goes to.
#[derive(Clone)]
pub struct Link {
    id: LinkId,
    registry: Arc<LinkRegistry>,
    sink: LinkSink,
    version: LinkVersion,
}

impl Link {
    /// The link's number.
    #[must_use]
    pub fn id(&self) -> LinkId {
        self.id
    }

    /// The link's outbox: frames queued here reach THIS gateway only.
    #[must_use]
    pub fn sink(&self) -> &LinkSink {
        &self.sink
    }

    /// The link's negotiated-version cell.
    #[must_use]
    pub fn version(&self) -> &LinkVersion {
        &self.version
    }

    /// The registry this link is entered in.
    #[must_use]
    pub fn registry(&self) -> &Arc<LinkRegistry> {
        &self.registry
    }

    /// [`LinkRegistry::claim`] for this link.
    pub fn claim(&self, conn: u64, actor: ObjId) -> Claim {
        self.registry.claim(self.id, conn, actor)
    }

    /// [`LinkRegistry::release`] for this link.
    pub fn release(&self, conn: u64) -> Release {
        self.registry.release(self.id, conn)
    }

    /// [`LinkRegistry::note_linkdead`] for this link.
    pub fn note_linkdead(&self, actor: ObjId) {
        self.registry.note_linkdead(self.id, actor);
    }

    /// [`LinkRegistry::clear_linkdead`] for this link.
    pub fn clear_linkdead(&self, actor: ObjId) {
        self.registry.clear_linkdead(self.id, actor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::Notify;
    use tokio::sync::mpsc;

    fn registry() -> Arc<LinkRegistry> {
        Arc::new(LinkRegistry::new(Arc::new(Metrics::new())))
    }

    fn open(reg: &Arc<LinkRegistry>) -> (Link, mpsc::Receiver<ToGateway>) {
        let (tx, rx) = mpsc::channel::<ToGateway>(16);
        let sink = LinkSink::new(
            tx,
            Arc::new(Metrics::new()),
            Arc::new(Notify::new()),
            16,
            80,
        );
        (reg.open_link(sink, LinkVersion::unknown()), rx)
    }

    fn exempt(char: u64) -> ToGateway {
        ToGateway::SetFloodExempt { char, exempt: true }
    }

    #[test]
    fn links_are_numbered_in_accept_order_and_counted() {
        let reg = registry();
        let (a, _ra) = open(&reg);
        let (b, _rb) = open(&reg);
        assert_eq!((a.id(), b.id()), (1, 2));
        assert_eq!(reg.links(), 2);
        assert_eq!(reg.metrics.snapshot().ipc_links, 2);
        assert_eq!(reg.metrics.snapshot().ipc_links_total, 2);
        assert!(reg.close(a.id()).is_some());
        assert_eq!(reg.links(), 1);
        assert_eq!(reg.metrics.snapshot().ipc_links, 1);
        // The second end of the same link finds nothing.
        assert!(reg.close(a.id()).is_none());
    }

    /// A connection id announced on two live links is the §4.3 duplicate: the
    /// second link is refused (and the refusal counted), the first keeps it.
    #[test]
    fn a_duplicate_across_live_links_is_refused_on_the_newer_link() {
        let reg = registry();
        let (a, _ra) = open(&reg);
        let (b, _rb) = open(&reg);
        assert_eq!(a.claim(7, ObjId(1)), Claim::Owned);
        assert_eq!(b.claim(7, ObjId(2)), Claim::Duplicate { owner: a.id() });
        assert_eq!(reg.owner_of(7), Some(a.id()));
        assert_eq!(reg.metrics.snapshot().ipc_duplicate_conn_ids_total, 1);
        // The owner re-announcing its own connection is not a duplicate.
        assert_eq!(a.claim(7, ObjId(1)), Claim::Owned);
        assert_eq!(reg.metrics.snapshot().ipc_duplicate_conn_ids_total, 1);
    }

    /// Once the owning link is gone the id is free again: the next link that
    /// announces it (a gateway reconnecting with its held sessions) owns it.
    #[test]
    fn a_closed_links_ids_are_free_for_the_next_link() {
        let reg = registry();
        let (a, _ra) = open(&reg);
        assert_eq!(a.claim(7, ObjId(1)), Claim::Owned);
        let detached = reg.close(a.id()).expect("first close");
        assert_eq!(detached.conns, vec![(7, ObjId(1))]);
        assert_eq!(detached.actors, vec![ObjId(1)]);
        let (b, _rb) = open(&reg);
        assert_eq!(b.claim(7, ObjId(1)), Claim::Owned);
        assert_eq!(reg.owner_of(7), Some(b.id()));
    }

    /// A release names its finding: the owner's, another link's, nobody's.
    #[test]
    fn release_tells_the_owner_from_a_stranger() {
        let reg = registry();
        let (a, _ra) = open(&reg);
        let (b, _rb) = open(&reg);
        assert_eq!(a.claim(7, ObjId(1)), Claim::Owned);
        assert_eq!(b.release(7), Release::OtherLink(a.id()));
        assert_eq!(
            reg.owner_of(7),
            Some(a.id()),
            "a stranger's release changes nothing"
        );
        assert_eq!(a.release(7), Release::Owned);
        assert_eq!(a.release(7), Release::Unknown);
        assert_eq!(reg.owner_of(7), None);
    }

    /// A link that ends with connections live leaves each of them to be
    /// detached and every actor to be reaped; a `Lost` connection whose
    /// `SessionGone` the link still owed is reaped too, one whose `SessionGone`
    /// arrived is not.
    #[test]
    fn close_lists_the_live_connections_and_the_owed_actors() {
        let reg = registry();
        let (a, _ra) = open(&reg);
        assert_eq!(a.claim(1, ObjId(10)), Claim::Owned);
        assert_eq!(a.claim(2, ObjId(20)), Claim::Owned);
        assert_eq!(a.claim(3, ObjId(30)), Claim::Owned);
        // 2 went Lost (owed), 3 went Lost and its SessionGone arrived.
        assert_eq!(a.release(2), Release::Owned);
        a.note_linkdead(ObjId(20));
        assert_eq!(a.release(3), Release::Owned);
        a.note_linkdead(ObjId(30));
        a.clear_linkdead(ObjId(30));
        let detached = reg.close(a.id()).expect("close");
        assert_eq!(detached.conns, vec![(1, ObjId(10))]);
        assert_eq!(detached.actors, vec![ObjId(10), ObjId(20)]);
        assert_eq!(reg.metrics.snapshot().ipc_link_detached_conns_total, 1);
    }

    /// A reconnect clears the owed `SessionGone`: the actor is live again on
    /// this link, and its connection is what the close detaches.
    #[test]
    fn a_new_conn_up_clears_the_owed_session_gone() {
        let reg = registry();
        let (a, _ra) = open(&reg);
        assert_eq!(a.claim(1, ObjId(10)), Claim::Owned);
        assert_eq!(a.release(1), Release::Owned);
        a.note_linkdead(ObjId(10));
        assert_eq!(a.claim(5, ObjId(10)), Claim::Owned);
        let detached = reg.close(a.id()).expect("close");
        assert_eq!(detached.conns, vec![(5, ObjId(10))]);
        assert_eq!(detached.actors, vec![ObjId(10)]);
    }

    /// Routing: a connection-addressed frame reaches the owning link only; a
    /// broadcast reaches every link; an unowned connection's frame is dropped
    /// and said so.
    #[tokio::test]
    async fn conn_frames_go_to_the_owner_and_broadcasts_to_all() {
        let reg = registry();
        let (a, mut ra) = open(&reg);
        let (b, mut rb) = open(&reg);
        assert_eq!(a.claim(7, ObjId(1)), Claim::Owned);
        assert!(reg.send_to_conn(7, exempt(1)));
        assert!(ra.try_recv().is_ok());
        assert!(rb.try_recv().is_err(), "b owns no connection 7");
        assert!(!reg.send_to_conn(8, exempt(1)), "nobody owns 8");
        assert_eq!(reg.broadcast(&exempt(2)), 2);
        assert!(ra.try_recv().is_ok());
        assert!(rb.try_recv().is_ok());
        assert!(reg.send_to_link(b.id(), exempt(3)));
        assert!(rb.try_recv().is_ok());
        assert!(ra.try_recv().is_err());
        reg.close(b.id());
        assert!(!reg.send_to_link(b.id(), exempt(3)), "b is gone");
    }

    /// The shutdown tail drops every link's sender without detaching anything
    /// into a world that is already stopping.
    #[tokio::test]
    async fn drain_forgets_every_link_and_closes_their_outboxes() {
        let reg = registry();
        let (a, mut ra) = open(&reg);
        let (b, mut rb) = open(&reg);
        assert_eq!(a.claim(7, ObjId(1)), Claim::Owned);
        drop((a, b));
        reg.drain();
        assert_eq!(reg.links(), 0);
        assert_eq!(reg.owner_of(7), None);
        assert_eq!(reg.metrics.snapshot().ipc_links, 0);
        // With the registry's clones gone and the `Link`s dropped, the
        // receivers see their channels close: that is the writer's `Bye` cue.
        assert!(ra.recv().await.is_none());
        assert!(rb.recv().await.is_none());
    }
}
