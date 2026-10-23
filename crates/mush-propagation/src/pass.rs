//! E7 cut 7 B6 — **the two passes cut 8's pulse driver calls** (the B lens).
//!
//! Cut 7 builds **no** `DomainTick` driver, no `DomainClock::Pulse` and no
//! domain-tick schedule source: those are cut 8's, and 10b §3.8 owns them. What
//! cut 7 hands over is exactly these two functions plus the
//! `PerceivedRef::Contact` entries tactical intents target. Their settled
//! signatures, after the independent verify fold, are
//! `detection_pass(store, state, world, ledger, now, domain, arm_bound)
//! -> DetectionReport` and
//! `delivery_pass(store, state, world, ledger, now, domain) -> DeliveryReport`
//! — one commit per domain per tick is §3.8's shape, so the domain scope is a
//! parameter rather than a whole-world scan the pulse would repeat per domain.
//!
//! Both are **pure over the store and the placement graph**: they take an
//! immutable world and a mutable book, and every state change they make is a
//! whole-document post-image (or a named drop) the caller persists. Both are
//! **bounded on records visited** (D-LBK-5), both **resume where they stopped**
//! through a durable cursor, and both **page rather than log** when the bound
//! binds (E4c V6): the report says what was left AND where to continue, so
//! running again makes progress rather than repeating the same work.
//!
//! Neither pass panics after its first mutation: there is no `?`, no `unwrap`,
//! no `expect` and no panicking index anywhere below, and every arithmetic term
//! saturates or is checked (E5a S2).
//!
//! **The detection pass is a world unit listing** in doc 16 §3.6's sense (16b
//! cut U.4, 16c `T-68`): *while an arena holds custody, its domain is excluded
//! from … every world unit listing, for every viewer including staff*. So a
//! unit an arena holds is never the observer of a sighting, never its subject,
//! never the platform a `Refresh`, a `Loss` or a `Reacquire` acts for and never
//! either end of a `Share` — each of the pass's paths asks
//! [`mush_sim::deployed::excluded`] (through `held_by_an_arena`) before it
//! acts. The arena's own contacts are its in-RAM shadow (doc 16 §3.6); the
//! world-side book must not mint, refresh or share a second picture of a unit
//! whose pose the world no longer owns. On check-in the marker is cleared and
//! the unit is an ordinary platform again on the next pass.

use mush_core::{ClockStamp, ContactLedgerState, ObjId, World};
use mush_sim::deployed::Shown;
use mush_sim::{DomainId, SpatialState};

use crate::context::{PropagationContext, domain_of_place, resolve_delivery, resolve_detection};
use crate::record::{
    ENVELOPE_TTL_SECS, Envelope, EnvelopeState, MAX_ENVELOPE_HOPS, PropagationStore, Sighting,
};

/// Envelopes visited in one delivery pass (D-LBK-5).
pub const MAX_ENVELOPES_PER_PASS: usize = 64;
/// Observers materialised in one detection pass.
pub const MAX_OBSERVERS_PER_PASS: usize = 32;
/// Subjects examined per observer.
pub const MAX_SUBJECTS_PER_OBSERVER: usize = 32;
/// `(observer, subject)` pairs one detection pass may VISIT (D-LBK-5).
///
/// The pass resumes from a durable cursor rather than restarting at the lowest
/// `ObjId`, so this bounds one pass's work without making the 33rd placed object
/// permanently invisible — which is what the two nested truncations did before
/// the independent verify fold (K7(a)).
pub const MAX_DETECTION_VISITS: usize = MAX_OBSERVERS_PER_PASS * MAX_SUBJECTS_PER_OBSERVER;
/// Records one pass's reaper may drop, per kind, per fire (fold D7).
pub const MAX_REAP_PER_PASS: usize = 64;
/// The most encoded bytes one arm's intent document may carry.
///
/// **K's cap, cited not redefined**: an offered text parameter is bounded by
/// `mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES`, which is exactly what
/// `mush_tactics::MAX_SENSOR_ARM_BYTES` is. Naming it over the `mush-core` edge
/// is how the pass sizes an arm by bytes without a `mush-tactics` edge (fold B9).
pub const MAX_ARM_DOCUMENT_BYTES: usize = mush_core::MAX_PROJECTED_ACTION_TEXT_BYTES;
/// The signature every platform emits, until a signature model lands.
///
/// **One constant for everything, deliberately.** A per-object figure would be
/// a second truth about a unit with no owner, and cut 7 has no unit model of its
/// own (cut 8 migrates `TacticalUnit`). Recorded as a deviation.
///
/// **It is not the fog model, and it must not become one again.** Detection is
/// bounded by domain adjacency inside `PropagationContext::build` (D1) and a
/// closed space occludes in both directions (K9), so both gates are structural:
/// `vf_dev_c_the_signature_constant_is_not_the_fog_model` pins that raising this
/// figure to `u32::MAX` re-opens neither.
pub const DEFAULT_SIGNATURE: u32 = 12;
/// How stale an observation may get before the policy refreshes it, in IC
/// seconds. Deliberately well below [`CONTACT_TTL_SECS`], so the refresh band
/// and the loss band are two distinct arms and neither is unreachable.
pub const REFRESH_AFTER_SECS: i64 = 10;
/// `CONTACT_TTL` (30 ticks, 10b §7) rendered in IC seconds at one tick a second
/// — the figure past which a contact is called lost rather than refreshed.
pub const CONTACT_TTL_SECS: i64 = mush_core::CONTACT_TTL as i64;
/// The `Loss` reason a contact stranded by a world rate change is retired under.
pub const CLOCK_GENERATION_REASON: &str = "clock_generation";

/// What one pass's reaper dropped. Every entry is a document key the caller
/// **deletes**, beside the post-images it writes (fold D7 / B1).
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Reaped {
    /// Terminal envelopes past [`ENVELOPE_TTL_SECS`].
    pub envelopes: Vec<u64>,
    /// Provenance rows whose contact has left the ledger.
    pub provenance: Vec<mush_core::ContactId>,
    /// Expired phantoms whose holders hold no active row any more.
    pub phantoms: Vec<mush_core::IdempotencyKeyText>,
    /// Sighting book keys aged out.
    pub sightings: Vec<String>,
}

impl Reaped {
    /// Whether anything was dropped.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.envelopes.is_empty()
            && self.provenance.is_empty()
            && self.phantoms.is_empty()
            && self.sightings.is_empty()
    }

    /// How many records in total.
    #[must_use]
    pub fn len(&self) -> usize {
        self.envelopes
            .len()
            .saturating_add(self.provenance.len())
            .saturating_add(self.phantoms.len())
            .saturating_add(self.sightings.len())
    }
}

/// What one delivery pass did. Counts and ids only: an envelope's PAYLOAD never
/// enters a report, because a report is read by staff and by a tick, not by the
/// recipient.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeliveryReport {
    /// Persisted itinerary or queue transitions, including unchanged public state.
    pub routed: Vec<u64>,
    /// Envelopes that took a relay hop.
    pub relayed: Vec<u64>,
    /// Envelopes that arrived.
    pub delivered: Vec<u64>,
    /// Originals an acknowledgement closed.
    pub acknowledged: Vec<u64>,
    /// Envelopes that ran out of time, hops or recipient.
    pub expired: Vec<u64>,
    /// Envelopes a newer revision displaced.
    pub superseded: Vec<u64>,
    /// Envelopes held because the link is jammed — waiting, never lost.
    pub jammed: Vec<u64>,
    /// How many records the pass visited.
    pub visited: usize,
    /// Whether the visit bound bound. **Paged, not logged** (E4c V6).
    pub bound_hit: bool,
    /// Where the next pass resumes. The page line names it, and because the
    /// cursor is durable, running again makes progress (fold B3).
    pub resume: Option<u64>,
    /// Finished traffic this fire compacted (fold D7).
    pub reaped: Reaped,
    /// A book that would not take a post-image. The pass stops only that
    /// envelope and says so, with the command a person would run.
    pub anomalies: Vec<String>,
}

impl DeliveryReport {
    /// Whether anything at all changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.relayed.is_empty()
            && self.routed.is_empty()
            && self.delivered.is_empty()
            && self.acknowledged.is_empty()
            && self.expired.is_empty()
            && self.superseded.is_empty()
            && self.reaped.is_empty()
    }

    /// Every envelope id whose document must be persisted, in canonical order.
    #[must_use]
    pub fn touched(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .relayed
            .iter()
            .chain(&self.routed)
            .chain(&self.delivered)
            .chain(&self.acknowledged)
            .chain(&self.expired)
            .chain(&self.superseded)
            .copied()
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.retain(|id| !self.reaped.envelopes.contains(id));
        ids
    }
}

/// What one detection pass did (fold D4(a)).
///
/// `DeliveryReport`'s shape: what was emitted, whether the bound bound, where to
/// resume, and what was refused. The pass **books exactly what it emits**, so
/// the page it prints is true — before the fold it booked a sighting for every
/// pair it detected, armed the first sixteen, and the remainder was
/// unreachable for ever because the book already answered `known`.
#[derive(Clone, Debug, Default)]
pub struct DetectionReport {
    /// The intents to arm, in canonical order.
    pub intents: Vec<mush_core::ObservationIntent>,
    /// Whether the arm bound or the visit bound bound.
    pub bound_hit: bool,
    /// The `(observer, subject)` pair the next pass resumes at.
    pub resume: Option<(ObjId, ObjId)>,
    /// Work the pass deliberately did **not** emit, each with its reason.
    pub refused: Vec<String>,
    /// Sighting keys whose document the caller must persist.
    pub sightings: Vec<String>,
    /// Sighting keys the caller must delete.
    pub dropped_sightings: Vec<String>,
    /// Contacts on an incomparable clock generation that the pass retired.
    pub stranded: usize,
}

impl DetectionReport {
    /// Whether anything at all is due.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.intents.is_empty()
    }

    /// How many intents are due.
    #[must_use]
    pub fn len(&self) -> usize {
        self.intents.len()
    }
}

/// Advance every live envelope one step (the O and B lenses).
///
/// The rules, in order, per envelope:
/// 1. **Age.** Past `ENVELOPE_TTL` it `Expire`s.
/// 2. **A recipient that went away.** It `Expire`s and is **attributed** —
///    never retried forever (the review's own addition).
/// 3. **Delivery.** Jammed keeps it exactly where it is (store-and-forward: the
///    recipient keeps its last delivered order and its fallback doctrine);
///    reachable relays it one hop, and the hop that reaches the recipient
///    delivers it. `MAX_ENVELOPE_HOPS` is what terminates a relay CYCLE.
/// 4. **Arrival.** An acknowledgement closes its original (`Acknowledged`); an
///    order supersedes older delivered revisions **from the same issuer** to the
///    same recipient, and an older revision arriving after a newer one is
///    **`Superseded` on arrival and never becomes active**.
///
/// The scan **starts at the store's own cursor and wraps**, so a permanently
/// jammed head no longer starves everything behind it (fold B3), and a bounded
/// reaper compacts finished traffic so the census caps stop being caps on
/// lifetime totals (fold D7 / B1).
pub fn delivery_pass(
    store: &mut PropagationStore,
    state: &SpatialState,
    world: &World,
    ledger: &ContactLedgerState,
    now: &ClockStamp,
    domain: Option<DomainId>,
) -> DeliveryReport {
    delivery_pass_with_metric(store, state, world, ledger, now, domain, None)
}

/// The shipping command/tick path supplies its owner's live metric projection.
#[allow(clippy::too_many_arguments)]
pub fn delivery_pass_with_metric(
    store: &mut PropagationStore,
    state: &SpatialState,
    world: &World,
    ledger: &ContactLedgerState,
    now: &ClockStamp,
    domain: Option<DomainId>,
    metric: Option<Box<dyn crate::routing::Metric>>,
) -> DeliveryReport {
    let mut report = DeliveryReport::default();
    reap(store, ledger, now, &mut report);
    // The declared-interference view is read once, before the book is mutated,
    // so every envelope in one pass sees the same physical picture. Only the
    // jamming map is snapshotted — a full store clone was up to three deep
    // copies of every book per fire (fold B16).
    let jamming_view = store.jamming_map();
    let mut router = crate::routing::Pass::new(world, state, store, now, metric);
    let cursor = store.envelope_scan_cursor();
    let mut ordered: Vec<u64> = store
        .envelopes()
        .filter(|envelope| !envelope.state().is_terminal())
        .filter(|envelope| in_domain(state, world, domain, envelope.recipient()))
        .map(Envelope::id)
        .collect();
    // Rotate to the cursor: everything at or after it, then everything before.
    let split = ordered.partition_point(|id| *id < cursor);
    ordered.rotate_left(split);
    let mut next_cursor = 0_u64;
    for id in ordered {
        if report.visited >= MAX_ENVELOPES_PER_PASS {
            report.bound_hit = true;
            report.resume = Some(id);
            next_cursor = id;
            break;
        }
        report.visited = report.visited.saturating_add(1);
        next_cursor = id.saturating_add(1);
        let Some(envelope) = store.envelope(id).cloned() else {
            continue;
        };
        // Re-check terminality against the LIVE book: an arrival earlier in this
        // same pass may have settled this envelope (an acknowledgement closes
        // its original), and re-stepping a terminal record would move it back.
        if envelope.state().is_terminal() {
            continue;
        }
        let next = if matches!(envelope.state(), EnvelopeState::Delivered)
            && envelope.age(now) < ENVELOPE_TTL_SECS
        {
            // Arrival is a one-time transition, not repeated every delivery fire.
            None
        } else if envelope.age(now) < ENVELOPE_TTL_SECS
            && !is_gone(world, envelope.recipient())
            && (router.is_some() || envelope.transit().is_some())
        {
            let next = router.as_mut().and_then(|r| r.step(store, &envelope, now));
            if let Some(next) = &next {
                report.routed.push(envelope.id());
                if next.path().len() > envelope.path().len() {
                    report.relayed.push(envelope.id());
                }
                if next.state() == EnvelopeState::Delivered {
                    report.delivered.push(envelope.id());
                }
                if next.state() == EnvelopeState::Expired {
                    report.expired.push(envelope.id());
                }
            }
            next
        } else {
            step_envelope(&envelope, &jamming_view, state, world, now, &mut report)
        };
        let Some(next) = next else { continue };
        if let Err(error) = store.put_envelope(next.clone()) {
            report.anomalies.push(format!(
                "envelope {id} could not be advanced ({error}); run @prop/envelopes #{} to inspect it",
                next.recipient().0
            ));
            continue;
        }
        if next.state() == EnvelopeState::Delivered {
            settle_arrival(store, &next, now, &mut report);
        }
    }
    store.set_envelope_scan_cursor(next_cursor);
    report
}

/// Whether an object sits in the domain a pass is scoped to. `None` means the
/// whole world, which is what the staff verb and cut 7's own tick pass.
fn in_domain(state: &SpatialState, world: &World, domain: Option<DomainId>, obj: ObjId) -> bool {
    let Some(wanted) = domain else { return true };
    crate::context::place_of(state, world, obj)
        .and_then(|place| domain_of_place(state, world, place))
        .is_some_and(|actual| actual == wanted)
}

/// The bounded per-fire compaction (fold D7 / B1).
///
/// Nothing in the crate removed a record before the fold: `MAX_ENVELOPES` and
/// its siblings were caps on **lifetime totals**, so once 4_096 envelopes had
/// ever been issued `+order` refused for every actor for ever, with no verb that
/// pruned. Each kind is capped per fire, and each drop is named in the report so
/// the caller emits a `ModuleWrite::Del` beside its post-images.
fn reap(
    store: &mut PropagationStore,
    ledger: &ContactLedgerState,
    now: &ClockStamp,
    report: &mut DeliveryReport,
) {
    // Terminal envelopes past their TTL. An envelope whose stamp is incomparable
    // can never reach the TTL by arithmetic, so a terminal one is reaped anyway.
    let dead: Vec<u64> = store
        .envelopes()
        .filter(|envelope| {
            envelope.state().is_terminal()
                && envelope
                    .checked_age(now)
                    .is_none_or(|age| age >= ENVELOPE_TTL_SECS)
        })
        .map(Envelope::id)
        .take(MAX_REAP_PER_PASS)
        .collect();
    for id in dead {
        if store.drop_envelope(id).is_some() {
            report.reaped.envelopes.push(id);
        }
    }
    // Provenance rows whose contact has left the ledger entirely.
    let orphans: Vec<mush_core::ContactId> = store
        .provenance()
        .map(crate::record::ContactProvenance::contact)
        .filter(|contact| !ledger.rows().iter().any(|row| row.contact == *contact))
        .take(MAX_REAP_PER_PASS)
        .collect();
    for contact in orphans {
        if store.drop_provenance(contact).is_some() {
            report.reaped.provenance.push(contact);
        }
    }
    // Expired phantoms whose holders hold no ACTIVE row naming them any more:
    // the Losses have been armed and applied, so the record has nothing left to
    // say. A phantom with a live row stays, because that row still needs its
    // expiry Loss.
    let spent: Vec<mush_core::IdempotencyKeyText> = store
        .phantoms()
        .filter(|phantom| phantom.is_expired(now))
        .filter(|phantom| {
            !ledger
                .rows()
                .iter()
                .any(|row| row.active && row.phantom.as_ref() == Some(phantom.id()))
        })
        .map(|phantom| phantom.id().clone())
        .take(MAX_REAP_PER_PASS)
        .collect();
    for id in spent {
        if store.drop_phantom(&id).is_some() {
            report.reaped.phantoms.push(id);
        }
    }
}

/// The context between two ends, with the **declared interference at the far
/// end** applied. Jamming is delivery-affecting only: `resolve_detection` never
/// reads it, which is R4's "a jammed unit still takes orders".
#[must_use]
pub fn context_between(
    store: &PropagationStore,
    state: &SpatialState,
    world: &World,
    from: ObjId,
    to: ObjId,
    at: &ClockStamp,
) -> Option<PropagationContext> {
    let jamming = store.jamming_map();
    context_between_view(&jamming, state, world, from, to, at)
}

fn context_between_view(
    jamming: &std::collections::BTreeMap<ObjId, u32>,
    state: &SpatialState,
    world: &World,
    from: ObjId,
    to: ObjId,
    at: &ClockStamp,
) -> Option<PropagationContext> {
    let context = PropagationContext::build(state, world, from, to, at.clone())?;
    let place = crate::context::place_of(state, world, to)?;
    Some(context.jammed(jamming.get(&place).copied().unwrap_or(0)))
}

/// One envelope's step, or `None` when nothing about it changed.
fn step_envelope(
    envelope: &Envelope,
    jamming: &std::collections::BTreeMap<ObjId, u32>,
    state: &SpatialState,
    world: &World,
    now: &ClockStamp,
    report: &mut DeliveryReport,
) -> Option<Envelope> {
    if envelope.age(now) >= ENVELOPE_TTL_SECS {
        report.expired.push(envelope.id());
        return Some(envelope.in_state(EnvelopeState::Expired, now.clone()));
    }
    // A recipient that was destroyed mid-flight: expire and attribute, never
    // retry forever. `GOING` counts as gone, because the object is on its way
    // out and a delivery to it would arrive nowhere.
    if is_gone(world, envelope.recipient()) {
        report.expired.push(envelope.id());
        report.anomalies.push(format!(
            "envelope {} expired: its recipient #{} is gone",
            envelope.id(),
            envelope.recipient().0
        ));
        return Some(envelope.in_state(EnvelopeState::Expired, now.clone()));
    }
    let from = envelope
        .path()
        .last()
        .map_or(envelope.issuer(), |hop| hop.relay);
    let Some(context) =
        context_between_view(jamming, state, world, from, envelope.recipient(), now)
    else {
        // Neither end is placed, or they are in different domains: the envelope
        // waits. It is not lost, and it is not delivered — exactly what
        // store-and-forward means, and the accepted consequence of D2 for a ship
        // that is currently under way.
        report.jammed.push(envelope.id());
        return None;
    };
    let outcome = resolve_delivery(&context);
    if !outcome.is_reachable() {
        report.jammed.push(envelope.id());
        return None;
    }
    let taken = u8::try_from(envelope.path().len()).unwrap_or(u8::MAX);
    if taken.saturating_add(1) >= outcome.hops() {
        report.delivered.push(envelope.id());
        return Some(envelope.in_state(EnvelopeState::Delivered, now.clone()));
    }
    if taken >= MAX_ENVELOPE_HOPS {
        report.expired.push(envelope.id());
        return Some(envelope.in_state(EnvelopeState::Expired, now.clone()));
    }
    match envelope.relayed(from, now.clone()) {
        Ok(relayed) => {
            report.relayed.push(envelope.id());
            Some(relayed)
        }
        Err(_) => {
            // The hop bound: a relay cycle terminates here rather than growing
            // age without bound.
            report.expired.push(envelope.id());
            Some(envelope.in_state(EnvelopeState::Expired, now.clone()))
        }
    }
}

/// Whether an object is gone, or on its way out. The same check `step_envelope`
/// makes for a recipient, so a transition is never armed for a holder that
/// cannot possibly commit it (fold K4(c)).
fn is_gone(world: &World, obj: ObjId) -> bool {
    world
        .get(obj)
        .is_none_or(|object| object.flags.contains(mush_core::Flags::GOING))
}

/// What an arrival settles: an ack closes its original; an order supersedes and
/// may itself be superseded on arrival.
fn settle_arrival(
    store: &mut PropagationStore,
    arrived: &Envelope,
    now: &ClockStamp,
    report: &mut DeliveryReport,
) {
    if let Some(original) = arrived.ack_of() {
        // **An acknowledgement is itself a delivery** (Q4A): only now does the
        // issuer learn anything, which is exactly why a lost ack leaves the
        // order `Delivered` and the issuer uninformed.
        if let Some(record) = store.envelope(original).cloned() {
            let closed = record.in_state(EnvelopeState::Acknowledged, now.clone());
            if store.put_envelope(closed).is_ok() {
                report.acknowledged.push(original);
            }
        }
        return;
    }
    if arrived.kind() != crate::record::DispatchKind::Order {
        return;
    }
    // Newest-valid-revision-wins, in both directions, **per issuer** — the same
    // scope `active_order` reads and `next_revision` allocates in, so the three
    // agree by construction (fold K8).
    let peers: Vec<Envelope> = store
        .envelopes()
        .filter(|peer| {
            peer.id() != arrived.id()
                && peer.issuer() == arrived.issuer()
                && peer.recipient() == arrived.recipient()
                && peer.kind() == crate::record::DispatchKind::Order
                && matches!(
                    peer.state(),
                    EnvelopeState::Delivered | EnvelopeState::Acknowledged
                )
        })
        .cloned()
        .collect();
    if peers
        .iter()
        .any(|peer| peer.revision() > arrived.revision())
    {
        // Revision 2 arriving after revision 3: superseded on arrival, and it
        // never becomes the active order at any point.
        let superseded = arrived.in_state(EnvelopeState::Superseded, now.clone());
        if store.put_envelope(superseded).is_ok() {
            report.superseded.push(arrived.id());
            report.delivered.retain(|id| *id != arrived.id());
        }
        return;
    }
    for peer in peers {
        if peer.revision() < arrived.revision() {
            let superseded = peer.in_state(EnvelopeState::Superseded, now.clone());
            let id = peer.id();
            if store.put_envelope(superseded).is_ok() {
                report.superseded.push(id);
            }
        }
    }
}

/// Every standing order a recipient holds: **the newest valid revision from
/// each issuer it has actually received** (T15, fold K8).
///
/// Per issuer, because supersession is per issuer. Selecting across issuers on a
/// bare revision comparison let any actor's revision 1 displace a commander's
/// standing order while the commander's envelope stayed `Delivered` and
/// unsuperseded — two live orders, and a bare integer deciding which one the
/// recipient was told to obey. Orders stay *messages*; who a unit **obeys** is
/// cut 9's command relation.
#[must_use]
pub fn standing_orders(store: &PropagationStore, recipient: ObjId) -> Vec<&Envelope> {
    let mut issuers: Vec<ObjId> = store
        .envelopes()
        .filter(|envelope| is_standing(envelope, recipient))
        .map(Envelope::issuer)
        .collect();
    issuers.sort_unstable();
    issuers.dedup();
    issuers
        .into_iter()
        .filter_map(|issuer| active_order_from(store, recipient, issuer))
        .collect()
}

fn is_standing(envelope: &Envelope, recipient: ObjId) -> bool {
    envelope.recipient() == recipient
        && envelope.kind() == crate::record::DispatchKind::Order
        && matches!(
            envelope.state(),
            EnvelopeState::Delivered | EnvelopeState::Acknowledged
        )
}

/// One issuer's standing order to `recipient`.
#[must_use]
pub fn active_order_from(
    store: &PropagationStore,
    recipient: ObjId,
    issuer: ObjId,
) -> Option<&Envelope> {
    store
        .envelopes()
        .filter(|envelope| is_standing(envelope, recipient) && envelope.issuer() == issuer)
        .max_by(|left, right| {
            left.revision()
                .cmp(&right.revision())
                .then_with(|| left.id().cmp(&right.id()))
        })
}

/// The standing order the recipient received **most recently**, across issuers.
///
/// Retained for callers that want one answer; `standing_orders` is the honest
/// one, because a recipient may hold a standing order from more than one sender
/// and cut 7 records no authority relation that ranks them.
#[must_use]
pub fn active_order(store: &PropagationStore, recipient: ObjId) -> Option<&Envelope> {
    standing_orders(store, recipient)
        .into_iter()
        .max_by_key(|envelope| envelope.id())
}

/// Every observer the detection pass walks: a placed platform that hosts at
/// least one control station.
///
/// A platform with a crew station is a thing somebody can see FROM; a loose
/// crate is not. Bounded, canonical order, so two passes agree.
///
/// **The unfiltered primitive** (16b cut U.4): with no world it cannot ask
/// [`mush_sim::deployed::excluded`], so a unit an arena holds is on it. The
/// pass itself never calls it — `detect_new` walks [`observers_from`] with the
/// world, which is filtered — and neither does anything else in the tree today;
/// it is kept as the placement graph's own answer to "who has a station".
#[must_use]
pub fn observers(state: &SpatialState) -> Vec<ObjId> {
    observers_from(state, None, None, None)
}

/// **Whether an arena holds `unit`** (16b cut U.4, 16c `T-68`): the one question
/// every path of the detection pass asks before it acts for or on a platform —
/// doc 16 §3.6's world-side exclusion, read through
/// [`mush_sim::deployed::excluded`] like every other world unit listing.
fn held_by_an_arena(world: &World, unit: ObjId) -> bool {
    mush_sim::deployed::excluded(world, Shown::Unit(unit))
}

/// The roster a detection pass walks, scoped to one domain and resumed from one
/// observer. At most [`MAX_OBSERVERS_PER_PASS`] entries are materialised per
/// pass, and the cursor is what makes the 33rd host reachable on the next one.
///
/// **With a world, a unit an arena holds is not on it** (16b cut U.4, 16c
/// `T-68`; doc 16 §3.6), whatever `domain` says: a deployed platform's placement
/// is frozen at muster while the arena moves it, so nothing the world could
/// compute from it would be true. Without one it is [`observers`]' unfiltered
/// primitive.
#[must_use]
pub fn observers_from(
    state: &SpatialState,
    world: Option<&World>,
    domain: Option<DomainId>,
    from: Option<ObjId>,
) -> Vec<ObjId> {
    let mut roster: Vec<ObjId> = state
        .stations
        .keys()
        .copied()
        .filter(|host| state.placements.contains_key(host))
        .filter(|host| from.is_none_or(|start| *host >= start))
        .filter(|host| world.is_none_or(|world| !held_by_an_arena(world, *host)))
        .filter(|host| match (world, domain) {
            (Some(world), Some(_)) => in_domain(state, world, domain, *host),
            _ => true,
        })
        .collect();
    roster.sort_unstable();
    roster.truncate(MAX_OBSERVERS_PER_PASS);
    roster
}

/// One detection pass's intents, in canonical order (the K lens).
///
/// It produces:
/// - an **`Observe`** for each newly detected `(observer, subject)` pair the
///   sighting book has not already acted on — so two passes over one unchanged
///   world produce the **same ordered set**, and running it twice arms nothing
///   twice — and one for each live phantom its holder has not yet been credited
///   with (fold O7, which is what gives the phantom→contact path a producer at
///   all);
/// - a **`Loss`** for every holder of an **expired phantom**, read from the
///   leak-safe ledger's own `phantom` column (hazard axis 3);
/// - a **`Loss`** for an active contact whose last observation is older than
///   `CONTACT_TTL`, a **`Refresh`** for one merely going stale, and a **`Loss`**
///   with reason `clock_generation` for one stranded on an older clock
///   generation by a world rate change (fold D5);
/// - a **`Reacquire`** for a lost phantom-backed contact whose phantom is still
///   live (fold K6);
/// - a **`Share`** for each **`Delivered`** report envelope — and for no
///   `Transmitted` one, which is the whole "sharing is a delivery" rule.
///
/// The store is taken mutably **only** to record and age sightings, which is
/// what makes the pass idempotent; nothing here writes a contact, and this crate
/// holds no handle with which it could.
///
/// **It books exactly what it emits.** `arm_bound` is the caller's own per-arm
/// bound; the pass stops there, records where it stopped, and the sighting book
/// only remembers pairs whose intent actually went out — so the page the caller
/// prints is true and the remainder really is still due next run.
pub fn detection_pass(
    store: &mut PropagationStore,
    state: &SpatialState,
    world: &World,
    ledger: &ContactLedgerState,
    now: &ClockStamp,
    domain: Option<DomainId>,
    arm_bound: usize,
) -> DetectionReport {
    let mut report = DetectionReport::default();
    let bound = arm_bound.clamp(1, mush_core::MAX_OBSERVATIONS_PER_PASS);
    age_sightings(store, now, &mut report);
    detect_new(store, state, world, now, domain, bound, &mut report);
    expire_phantoms(store, ledger, world, now, &mut report);
    age_contacts(ledger, world, now, &mut report);
    share_delivered(store, ledger, world, now, &mut report);
    report
        .intents
        .sort_by(|left, right| left.key().as_str().cmp(right.key().as_str()));
    report
        .intents
        .dedup_by(|left, right| left.key() == right.key());
    // One transition per (kind, contact) per pass: the ledger emits one row per
    // (contact, holder), so a shared contact used to arm one Refresh per holder
    // against the SAME expected_revision — the first committed, every other was
    // refused `Stale`, and its cursor stood for ever (fold O9).
    let mut seen: Vec<(mush_core::ObservationKind, mush_core::ContactId)> = Vec::new();
    report.intents.retain(|intent| {
        let Some(contact) = intent.op().contact() else {
            return true;
        };
        let entry = (intent.kind(), contact);
        if seen.contains(&entry) {
            return false;
        }
        seen.push(entry);
        true
    });
    trim_to_arm(store, &mut report, bound);
    match mush_core::bounded_pass(std::mem::take(&mut report.intents)) {
        Ok(intents) => report.intents = intents,
        Err(error) => {
            // A refusal that names the numbers, never a silent truncation.
            report.refused.push(error.to_string());
            report.bound_hit = true;
        }
    }
    report
}

/// Fill the arm to the caller's COUNT bound **and the offer document's BYTE
/// bound**, un-booking anything that does not go out (fold B9 + D4(a)).
///
/// 08i answer 15's rule is to measure the bundle, not assume it fits: 16
/// ordinary `Observe` intents already encode to 3,525 of 4,096 bytes (86 %), and
/// the phantom arm O7 added — a `SubjectKey::Scoped` truth with a holder
/// audience — takes 16 of them to **5,665**, which would refuse the whole batch
/// at `SensorArmProvider::intents` with nothing armed and nothing said. So the
/// pass fills by bytes and reports the true remainder.
///
/// Anything trimmed has its **sighting un-booked**, because a pass that books
/// what it does not emit is exactly the K7 defect this fold closed: the book
/// would answer `known` on the next run and the observation would be lost for
/// good.
fn trim_to_arm(store: &mut PropagationStore, report: &mut DetectionReport, bound: usize) {
    let mut budget = MAX_ARM_DOCUMENT_BYTES.saturating_sub(2); // the `[` and `]`
    let mut kept = 0_usize;
    let mut dropped: Vec<mush_core::ObservationIntent> = Vec::new();
    report.intents.retain(|intent| {
        if kept >= bound {
            dropped.push(intent.clone());
            return false;
        }
        let cost = serde_json::to_string(intent)
            .map(|text| text.len().saturating_add(1))
            .unwrap_or(usize::MAX);
        if cost > budget {
            dropped.push(intent.clone());
            return false;
        }
        budget = budget.saturating_sub(cost);
        kept = kept.saturating_add(1);
        true
    });
    if dropped.is_empty() {
        return;
    }
    report.bound_hit = true;
    for intent in dropped {
        let mush_core::ObservationOp::Observe { truth, .. } = intent.op() else {
            continue;
        };
        let observer = intent.observer().platform();
        let key = match truth {
            mush_core::SubjectKey::Object { id } => Sighting::key_of(observer, Some(*id), None),
            mush_core::SubjectKey::Scoped { id, .. } => Sighting::key_of(observer, None, Some(id)),
        };
        if !report.sightings.contains(&key) {
            continue;
        }
        store.drop_sighting(&key);
        report.sightings.retain(|written| *written != key);
    }
}

/// Age the sighting book (fold D4(c)).
///
/// A sighting older than `CONTACT_TTL` is dropped, so a legitimate
/// re-observation is never blocked for ever by a stale entry; one whose stamp is
/// incomparable is dropped too, because a rate change would otherwise freeze it
/// permanently. A sighting still inside the refresh band is left exactly as it
/// is, so an unchanged world writes no sighting document at all (fold B12).
fn age_sightings(store: &mut PropagationStore, now: &ClockStamp, report: &mut DetectionReport) {
    let stale: Vec<String> = store
        .sightings()
        .filter(|sighting| {
            sighting
                .staleness(now)
                .is_none_or(|age| age >= CONTACT_TTL_SECS)
        })
        .map(Sighting::key)
        .take(MAX_REAP_PER_PASS)
        .collect();
    for key in stale {
        if store.drop_sighting(&key).is_some() {
            report.dropped_sightings.push(key);
        }
    }
}

/// Step (1): new sightings, bounded, resumable and booked only when armed.
fn detect_new(
    store: &mut PropagationStore,
    state: &SpatialState,
    world: &World,
    now: &ClockStamp,
    domain: Option<DomainId>,
    bound: usize,
    report: &mut DetectionReport,
) {
    let cursor = store.detection_cursor();
    // 16b cut U.4 (16c `T-68`): a unit an arena holds is sighted by nobody on
    // the world side (doc 16 §3.6); the observers are filtered the same way,
    // inside `observers_from`.
    let subjects: Vec<ObjId> = state
        .placements
        .keys()
        .copied()
        .filter(|subject| !held_by_an_arena(world, *subject))
        .collect();
    let mut visited = 0_usize;
    let mut stopped: Option<(ObjId, ObjId)> = None;
    'observers: for observer in
        observers_from(state, Some(world), domain, cursor.map(|(host, _)| host))
    {
        if is_gone(world, observer) {
            continue;
        }
        let observer_ref = mush_core::ObserverRef::Platform { id: observer };
        for subject in subjects.iter().copied() {
            if let Some((resume_host, resume_subject)) = cursor
                && observer == resume_host
                && subject < resume_subject
            {
                continue;
            }
            if subject == observer {
                continue;
            }
            if report.intents.len() >= bound || visited >= MAX_DETECTION_VISITS {
                stopped = Some((observer, subject));
                report.bound_hit = true;
                report.resume = stopped;
                break 'observers;
            }
            visited = visited.saturating_add(1);
            let Some(context) =
                PropagationContext::build(state, world, observer, subject, now.clone())
            else {
                continue;
            };
            // Constant work: the outcome is computed either way and the sighting
            // book is consulted either way, so a miss costs a hit's reads.
            let outcome = resolve_detection(&context, DEFAULT_SIGNATURE);
            let key = Sighting::key_of(observer, Some(subject), None);
            let known = store.sighting(&key).cloned();
            if !outcome.is_detected() {
                continue;
            }
            if let Some(existing) = known {
                // Re-detection re-stamps only once it has drifted into the
                // refresh band, so a still-current pair costs no write at all.
                if existing
                    .staleness(now)
                    .is_none_or(|age| age >= REFRESH_AFTER_SECS)
                    && let Ok(refreshed) = existing.seen_again(now)
                    && store.put_sighting(refreshed).is_ok()
                {
                    report.sightings.push(key);
                }
                continue;
            }
            let Ok(intent_key) = mush_core::ObservationIntent::key_for(
                mush_core::ObservationKind::Observe,
                &observer_ref,
                &format!("o{}", subject.0),
                now.at,
            ) else {
                continue;
            };
            let Ok(intent) = mush_core::ObservationIntent::new(
                observer_ref.clone(),
                mush_core::ObservationOp::Observe {
                    truth: mush_core::SubjectKey::Object { id: subject },
                    audience: vec![observer_ref.principal()],
                },
                now.clone(),
                intent_key,
            ) else {
                continue;
            };
            let Ok(sighting) =
                Sighting::new(observer, Some(subject), None, now.clone(), now.clone())
            else {
                continue;
            };
            // **The census failure is never swallowed.** An intent whose
            // idempotency record could not be written is NOT emitted, and the
            // refusal is named: before the fold the book refused, the intent went
            // out anyway, and the same observation re-armed under a fresh key on
            // every pass for ever.
            if let Err(error) = store.put_sighting(sighting) {
                report
                    .refused
                    .push(format!("an observation was not armed: {error}"));
                continue;
            }
            report.sightings.push(key);
            report.intents.push(intent);
        }
    }
    // `None` restarts the rotation, which is what a pass that finished the whole
    // roster should do.
    store.set_detection_cursor(stopped);
}

/// Step (2): an expired phantom issues `Loss` to every holder, and a live one
/// its holder has not been credited with issues an `Observe` (fold O7).
fn expire_phantoms(
    store: &mut PropagationStore,
    ledger: &ContactLedgerState,
    world: &World,
    now: &ClockStamp,
    report: &mut DetectionReport,
) {
    let expired: Vec<mush_core::IdempotencyKeyText> = store
        .phantoms()
        .filter(|phantom| phantom.is_expired(now))
        .map(|phantom| phantom.id().clone())
        .collect();
    for row in ledger.rows() {
        let Some(phantom) = row.phantom.as_ref() else {
            continue;
        };
        if !expired.iter().any(|id| id == phantom) || !row.active {
            continue;
        }
        let mush_core::PrincipalId::Object { id: holder } = row.holder else {
            continue;
        };
        // 16b cut U.4: the expiry's `Loss` acts for the holder as `age_contacts`'
        // does, so a holder an arena holds waits for its check-in here too.
        if is_gone(world, holder) || held_by_an_arena(world, holder) {
            continue;
        }
        push_contact_intent(
            &mut report.intents,
            mush_core::ObserverRef::Platform { id: holder },
            mush_core::ObservationKind::Loss,
            row,
            now,
            Some("expired"),
        );
    }
    // The producer the phantom→contact path never had: a live phantom whose
    // holder holds no ledger row for it is observed, with the phantom's own
    // scoped subject as truth. `@prop/phantom` minted a document nothing
    // consumed, so OPEN 5's "Loss to every holder on expiry" was reachable only
    // through a test-installed ledger.
    let live: Vec<(
        mush_core::IdempotencyKeyText,
        mush_core::SubjectKey,
        Vec<mush_core::PrincipalId>,
    )> = store
        .phantoms()
        .filter(|phantom| !phantom.is_expired(now))
        .map(|phantom| {
            (
                phantom.id().clone(),
                phantom.subject(),
                phantom.holders().to_vec(),
            )
        })
        .collect();
    for (id, subject, holders) in live {
        for holder in &holders {
            let mush_core::PrincipalId::Object { id: platform } = holder else {
                continue;
            };
            // 16b cut U.4: nor is a holder an arena holds credited with one.
            if is_gone(world, *platform) || held_by_an_arena(world, *platform) {
                continue;
            }
            if ledger
                .rows()
                .iter()
                .any(|row| row.holder == *holder && row.phantom.as_ref() == Some(&id))
            {
                continue;
            }
            let key = Sighting::key_of(*platform, None, Some(&id));
            if store.sighting(&key).is_some() {
                continue;
            }
            let observer = mush_core::ObserverRef::Platform { id: *platform };
            let Ok(intent_key) = mush_core::ObservationIntent::key_for(
                mush_core::ObservationKind::Observe,
                &observer,
                &format!("f{}", id.as_str()),
                now.at,
            ) else {
                continue;
            };
            let Ok(intent) = mush_core::ObservationIntent::new(
                observer.clone(),
                mush_core::ObservationOp::Observe {
                    truth: subject.clone(),
                    audience: holders.clone(),
                },
                now.clone(),
                intent_key,
            ) else {
                continue;
            };
            let Ok(sighting) =
                Sighting::new(*platform, None, Some(id.clone()), now.clone(), now.clone())
            else {
                continue;
            };
            if let Err(error) = store.put_sighting(sighting) {
                report
                    .refused
                    .push(format!("a phantom was not credited: {error}"));
                continue;
            }
            report.sightings.push(key);
            report.intents.push(intent);
        }
    }
}

/// Step (3): staleness, the clock-generation retirement, and reacquisition.
fn age_contacts(
    ledger: &ContactLedgerState,
    world: &World,
    now: &ClockStamp,
    report: &mut DetectionReport,
) {
    for row in ledger.rows() {
        let mush_core::PrincipalId::Object { id: holder } = row.holder else {
            continue;
        };
        // A transition whose holder is gone can never commit, and arming it
        // leaves a durable cursor standing for ever (fold K4(c)).
        if is_gone(world, holder) {
            continue;
        }
        // 16b cut U.4 (16c `T-68`): nor does the world age a contact whose
        // holder an arena holds — every arm below acts with the holder as the
        // platform, and the arena is that platform's single writer (doc 16
        // §3.6). The row waits, untouched, for the check-in.
        if held_by_an_arena(world, holder) {
            continue;
        }
        let observer = mush_core::ObserverRef::Platform { id: holder };
        if row.observed_at.clock != now.clock {
            // **A world rate change no longer freezes knowledge** (fold D5). The
            // stamps are incomparable, so nothing can be refreshed against them;
            // the contact is RETIRED instead, which the kernel now admits
            // because `Loss` is terminal and orders nothing.
            if row.active {
                report.stranded = report.stranded.saturating_add(1);
                push_contact_intent(
                    &mut report.intents,
                    observer,
                    mush_core::ObservationKind::Loss,
                    row,
                    now,
                    Some(CLOCK_GENERATION_REASON),
                );
            }
            continue;
        }
        if !row.active {
            // Reacquisition. Propagation can only resolve a contact back to its
            // subject when the subject is its OWN mint — a phantom — because the
            // leak-safe ledger deliberately carries no object id. A lost contact
            // on a real unit is re-observed instead, through the aged sighting
            // book, which mints a fresh identity rather than reviving one.
            if row.phantom.is_some() {
                push_contact_intent(
                    &mut report.intents,
                    observer,
                    mush_core::ObservationKind::Reacquire,
                    row,
                    now,
                    None,
                );
            }
            continue;
        }
        let age = now.at.saturating_sub(row.observed_at.at);
        if age >= CONTACT_TTL_SECS {
            push_contact_intent(
                &mut report.intents,
                observer,
                mush_core::ObservationKind::Loss,
                row,
                now,
                Some("stale"),
            );
        } else if age >= REFRESH_AFTER_SECS {
            push_contact_intent(
                &mut report.intents,
                observer,
                mush_core::ObservationKind::Refresh,
                row,
                now,
                None,
            );
        }
    }
}

/// Step (4): a DELIVERED report shares its contact; a transmitted one shares
/// nothing at all.
fn share_delivered(
    store: &PropagationStore,
    ledger: &ContactLedgerState,
    world: &World,
    now: &ClockStamp,
    report: &mut DetectionReport,
) {
    for envelope in store.envelopes() {
        let Some(contact) = envelope.shares() else {
            continue;
        };
        if !matches!(
            envelope.state(),
            EnvelopeState::Delivered | EnvelopeState::Acknowledged
        ) {
            continue;
        }
        if is_gone(world, envelope.issuer()) || is_gone(world, envelope.recipient()) {
            continue;
        }
        // 16b cut U.4 (16c `T-68`): a `Share` is armed FROM the issuer and names
        // the recipient, so a unit an arena holds is neither end of one (doc 16
        // §3.6). The envelope stays `Delivered`, and a later pass shares it once
        // both ends are back in the world.
        if held_by_an_arena(world, envelope.issuer())
            || held_by_an_arena(world, envelope.recipient())
        {
            continue;
        }
        let issuer_holder = mush_core::PrincipalId::Object {
            id: envelope.issuer(),
        };
        let recipient_holder = mush_core::PrincipalId::Object {
            id: envelope.recipient(),
        };
        let Some(row) = ledger
            .rows()
            .iter()
            .find(|row| row.contact == contact && row.holder == issuer_holder)
        else {
            continue;
        };
        if ledger
            .rows()
            .iter()
            .any(|row| row.contact == contact && row.holder == recipient_holder)
        {
            continue;
        }
        let observer = mush_core::ObserverRef::Platform {
            id: envelope.issuer(),
        };
        let Ok(key) = mush_core::ObservationIntent::key_for_state(
            mush_core::ObservationKind::Share,
            &observer,
            contact,
        ) else {
            continue;
        };
        let Ok(intent) = mush_core::ObservationIntent::new(
            observer,
            mush_core::ObservationOp::Share {
                contact,
                expected_revision: row.revision,
                to: mush_core::ObserverRef::Platform {
                    id: envelope.recipient(),
                },
            },
            now.clone(),
            key,
        ) else {
            continue;
        };
        report.intents.push(intent);
    }
}

fn push_contact_intent(
    intents: &mut Vec<mush_core::ObservationIntent>,
    observer: mush_core::ObserverRef,
    kind: mush_core::ObservationKind,
    row: &mush_core::ContactLedgerRow,
    now: &ClockStamp,
    reason: Option<&str>,
) {
    // The key carries no tick and the WHOLE contact id (fold D3, O10): the
    // identity of a transition is `(kind, observer, contact)`, so a still-due
    // transition one IC second later REPLACES its standing cursor rather than
    // stacking a second one beside it, and two contacts sharing a 32-bit prefix
    // no longer collide into one silently dropped intent.
    let Ok(key) = mush_core::ObservationIntent::key_for_state(kind, &observer, row.contact) else {
        return;
    };
    let op = match kind {
        mush_core::ObservationKind::Loss => {
            let Ok(reason) = mush_core::CanonicalId::new(reason.unwrap_or("stale")) else {
                return;
            };
            mush_core::ObservationOp::Loss {
                contact: row.contact,
                expected_revision: row.revision,
                reason,
            }
        }
        mush_core::ObservationKind::Reacquire => mush_core::ObservationOp::Reacquire {
            contact: row.contact,
            expected_revision: row.revision,
        },
        _ => mush_core::ObservationOp::Refresh {
            contact: row.contact,
            expected_revision: row.revision,
        },
    };
    if let Ok(intent) = mush_core::ObservationIntent::new(observer, op, now.clone(), key) {
        intents.push(intent);
    }
}
