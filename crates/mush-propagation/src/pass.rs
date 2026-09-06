//! E7 cut 7 B6 — **the two passes cut 8's pulse driver calls** (the B lens).
//!
//! Cut 7 builds **no** `DomainTick` driver, no `DomainClock::Pulse` and no
//! domain-tick schedule source: those are cut 8's, and 10b §3.8 owns them. What
//! cut 7 hands over is exactly these two functions plus the
//! `PerceivedRef::Contact` entries tactical intents target.
//!
//! Both are **pure over the store and the placement graph**: they take an
//! immutable world and a mutable book, and every state change they make is a
//! whole-document post-image the caller persists. Both are **bounded on records
//! visited** (D-LBK-5), and both **page rather than log** when the bound binds
//! (E4c V6): the report says what was left, so a caller can run again or a staff
//! verb can print it, and neither ever truncates silently.
//!
//! Neither pass panics after its first mutation: there is no `?`, no `unwrap`,
//! no `expect` and no panicking index anywhere below, and every arithmetic term
//! saturates or is checked (E5a S2).

use mush_core::{ClockStamp, ContactLedgerState, ObjId, World};
use mush_sim::SpatialState;

use crate::context::{PropagationContext, resolve_delivery, resolve_detection};
use crate::record::{
    ENVELOPE_TTL_SECS, Envelope, EnvelopeState, MAX_ENVELOPE_HOPS, PropagationStore, Sighting,
};

/// Envelopes visited in one delivery pass (D-LBK-5).
pub const MAX_ENVELOPES_PER_PASS: usize = 64;
/// Observers visited in one detection pass.
pub const MAX_OBSERVERS_PER_PASS: usize = 32;
/// Subjects examined per observer.
pub const MAX_SUBJECTS_PER_OBSERVER: usize = 32;
/// The signature every platform emits, until a signature model lands.
///
/// **One constant for everything, deliberately.** A per-object figure would be
/// a second truth about a unit with no owner, and cut 7 has no unit model of its
/// own (cut 8 migrates `TacticalUnit`). Recorded as a deviation.
pub const DEFAULT_SIGNATURE: u32 = 12;
/// How stale an observation may get before the policy refreshes it, in IC
/// seconds. Deliberately well below [`CONTACT_TTL_SECS`], so the refresh band
/// and the loss band are two distinct arms and neither is unreachable.
pub const REFRESH_AFTER_SECS: i64 = 10;
/// `CONTACT_TTL` (30 ticks, 10b §7) rendered in IC seconds at one tick a second
/// — the figure past which a contact is called lost rather than refreshed.
pub const CONTACT_TTL_SECS: i64 = mush_core::CONTACT_TTL as i64;

/// What one delivery pass did. Counts and ids only: an envelope's PAYLOAD never
/// enters a report, because a report is read by staff and by a tick, not by the
/// recipient.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DeliveryReport {
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
    /// A book that would not take a post-image. The pass stops only that
    /// envelope and says so, with the command a person would run.
    pub anomalies: Vec<String>,
}

impl DeliveryReport {
    /// Whether anything at all changed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.relayed.is_empty()
            && self.delivered.is_empty()
            && self.acknowledged.is_empty()
            && self.expired.is_empty()
            && self.superseded.is_empty()
    }

    /// Every envelope id whose document must be persisted, in canonical order.
    #[must_use]
    pub fn touched(&self) -> Vec<u64> {
        let mut ids: Vec<u64> = self
            .relayed
            .iter()
            .chain(&self.delivered)
            .chain(&self.acknowledged)
            .chain(&self.expired)
            .chain(&self.superseded)
            .copied()
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids
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
///    order supersedes older delivered revisions to the same recipient, and an
///    older revision arriving after a newer one is **`Superseded` on arrival and
///    never becomes active**.
pub fn delivery_pass(
    store: &mut PropagationStore,
    state: &SpatialState,
    world: &World,
    now: &ClockStamp,
) -> DeliveryReport {
    let mut report = DeliveryReport::default();
    // The declared-interference view is read once, before the book is mutated,
    // so every envelope in one pass sees the same physical picture.
    let jamming_view = store.clone();
    let live: Vec<u64> = store
        .envelopes()
        .filter(|envelope| !envelope.state().is_terminal())
        .map(Envelope::id)
        .collect();
    for id in live {
        if report.visited >= MAX_ENVELOPES_PER_PASS {
            report.bound_hit = true;
            break;
        }
        report.visited = report.visited.saturating_add(1);
        let Some(envelope) = store.envelope(id).cloned() else {
            continue;
        };
        let next = step_envelope(&envelope, &jamming_view, state, world, now, &mut report);
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
    report
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
    let context = PropagationContext::build(state, world, from, to, at.clone())?;
    let place = crate::context::place_of(state, world, to)?;
    Some(context.jammed(store.jamming_at(place)))
}

/// One envelope's step, or `None` when nothing about it changed.
fn step_envelope(
    envelope: &Envelope,
    store_jamming: &PropagationStore,
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
    if world
        .get(envelope.recipient())
        .is_none_or(|object| object.flags.contains(mush_core::Flags::GOING))
    {
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
        context_between(store_jamming, state, world, from, envelope.recipient(), now)
    else {
        // Neither end is placed: the envelope waits. It is not lost, and it is
        // not delivered — exactly what store-and-forward means.
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
    // Newest-valid-revision-wins, in both directions.
    let peers: Vec<Envelope> = store
        .envelopes()
        .filter(|peer| {
            peer.id() != arrived.id()
                && peer.issuer() == arrived.issuer()
                && peer.recipient() == arrived.recipient()
                && peer.ack_of().is_none()
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

/// The order a recipient is actually obeying: **the newest valid revision it
/// has actually received** (T15).
///
/// An `Acknowledged` order is still the active one — acknowledging is telling
/// the issuer, not finishing. A `Transmitted` one is not: it has not arrived.
#[must_use]
pub fn active_order(store: &PropagationStore, recipient: ObjId) -> Option<&Envelope> {
    store
        .envelopes()
        .filter(|envelope| {
            envelope.recipient() == recipient
                && envelope.ack_of().is_none()
                && matches!(
                    envelope.state(),
                    EnvelopeState::Delivered | EnvelopeState::Acknowledged
                )
        })
        .max_by(|left, right| {
            left.revision()
                .cmp(&right.revision())
                .then_with(|| left.id().cmp(&right.id()))
        })
}

/// Every observer the detection pass walks: a placed platform that hosts at
/// least one control station.
///
/// A platform with a crew station is a thing somebody can see FROM; a loose
/// crate is not. Bounded, canonical order, so two passes agree.
#[must_use]
pub fn observers(state: &SpatialState) -> Vec<ObjId> {
    let mut roster: Vec<ObjId> = state
        .stations
        .keys()
        .copied()
        .filter(|host| state.placements.contains_key(host))
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
///   twice;
/// - a **`Loss`** for every holder of an **expired phantom**, read from the
///   leak-safe ledger's own `phantom` column (hazard axis 3);
/// - a **`Loss`** for an active contact whose last observation is older than
///   `CONTACT_TTL`, and a **`Refresh`** for one merely going stale;
/// - a **`Share`** for each **`Delivered`** report envelope — and for no
///   `Transmitted` one, which is the whole "sharing is a delivery" rule.
///
/// The store is taken mutably **only** to record sightings, which is what makes
/// the pass idempotent; nothing here writes a contact, and this crate holds no
/// handle with which it could.
pub fn detection_pass(
    store: &mut PropagationStore,
    state: &SpatialState,
    world: &World,
    ledger: &ContactLedgerState,
    now: &ClockStamp,
) -> Vec<mush_core::ObservationIntent> {
    let mut intents: Vec<mush_core::ObservationIntent> = Vec::new();
    // (1) new sightings.
    for observer in observers(state) {
        let observer_ref = mush_core::ObserverRef::Platform { id: observer };
        let mut examined = 0_usize;
        for subject in state.placements.keys().copied() {
            if examined >= MAX_SUBJECTS_PER_OBSERVER {
                break;
            }
            if subject == observer {
                continue;
            }
            examined = examined.saturating_add(1);
            let Some(context) =
                PropagationContext::build(state, world, observer, subject, now.clone())
            else {
                continue;
            };
            // Constant work: the outcome is computed either way and the sighting
            // book is consulted either way, so a miss costs a hit's reads.
            let outcome = resolve_detection(&context, DEFAULT_SIGNATURE);
            let key = Sighting::key_of(observer, Some(subject), None);
            let known = store.sighting(&key).is_some();
            if !outcome.is_detected() || known {
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
            let _ = store.put_sighting(Sighting {
                observer,
                subject: Some(subject),
                phantom: None,
                first: now.clone(),
                last: now.clone(),
            });
            intents.push(intent);
        }
    }
    // (2) expiring phantoms: Loss to EVERY holder.
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
        push_contact_intent(
            &mut intents,
            mush_core::ObserverRef::Platform { id: holder },
            mush_core::ObservationKind::Loss,
            row,
            now,
            Some("expired"),
        );
    }
    // (3) staleness: refresh what is merely old, lose what is past the TTL.
    for row in ledger.rows() {
        if !row.active || row.observed_at.clock != now.clock {
            continue;
        }
        if row.phantom.as_ref().is_some_and(|id| expired.contains(id)) {
            continue;
        }
        let age = now.at.saturating_sub(row.observed_at.at);
        let mush_core::PrincipalId::Object { id: holder } = row.holder else {
            continue;
        };
        let observer = mush_core::ObserverRef::Platform { id: holder };
        if age >= CONTACT_TTL_SECS {
            push_contact_intent(
                &mut intents,
                observer,
                mush_core::ObservationKind::Loss,
                row,
                now,
                Some("stale"),
            );
        } else if age >= REFRESH_AFTER_SECS {
            push_contact_intent(
                &mut intents,
                observer,
                mush_core::ObservationKind::Refresh,
                row,
                now,
                None,
            );
        }
    }
    // (4) a DELIVERED report shares its contact; a transmitted one shares
    // nothing at all.
    for envelope in store.envelopes() {
        let Some(contact) = envelope.shares() else {
            continue;
        };
        if envelope.state() != EnvelopeState::Delivered {
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
        let Ok(key) = mush_core::ObservationIntent::key_for(
            mush_core::ObservationKind::Share,
            &observer,
            &format!("c{}", &contact.to_hex()[..8]),
            now.at,
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
        intents.push(intent);
    }
    intents.sort_by(|left, right| left.key().as_str().cmp(right.key().as_str()));
    intents.dedup_by(|left, right| left.key() == right.key());
    intents.truncate(mush_core::MAX_OBSERVATIONS_PER_PASS);
    intents
}

fn push_contact_intent(
    intents: &mut Vec<mush_core::ObservationIntent>,
    observer: mush_core::ObserverRef,
    kind: mush_core::ObservationKind,
    row: &mush_core::ContactLedgerRow,
    now: &ClockStamp,
    reason: Option<&str>,
) {
    let Ok(key) = mush_core::ObservationIntent::key_for(
        kind,
        &observer,
        &format!("c{}", &row.contact.to_hex()[..8]),
        now.at,
    ) else {
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
        _ => mush_core::ObservationOp::Refresh {
            contact: row.contact,
            expected_revision: row.revision,
        },
    };
    if let Ok(intent) = mush_core::ObservationIntent::new(observer, op, now.clone(), key) {
        intents.push(intent);
    }
}
