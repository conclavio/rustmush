//! # Domain promotion and demotion — E7 cut 6 B5 (10b §3.6)
//!
//! *The governing rule: the system never runs tactically as a whole* (Q15A). A
//! domain **promotes** on one of four deterministic triggers and **demotes**
//! only after a quiet boundary and once a durable summary is written.
//!
//! ## What cut 6 owns, and what it deliberately does not
//!
//! Cut 6 owns the **records and the eligibility**, and nothing else:
//! `@domain/promote` writes a [`DomainActivation`] **request**, and cut 8's
//! authority is what applies it, adds `DomainClock::Pulse` and drives the tick.
//! So this module constructs **no** `Pulse`, registers **no**
//! `DomainClockProvider`, and **never mutates the `ClockProviderRegistry`** —
//! which is composition-frozen anyway (`Registrar::domain_clock_provider` is
//! compose-time only, 08i answer 4 / G-A). 10b §3.6's drafted
//! `DomainActivation { …, clock: ClockRef /* DomainTick */, … }` names cut 8's
//! clock and is **not** a field here (contradiction (iii)).
//!
//! Both records are **exempt `sim.domain`** workflow documents (§4.1):
//! bounded whole-document legacy post-images at `activation:<id>` /
//! `summary:<id>`, with strict [`TryFrom`] wires re-running every constructor
//! invariant, deterministic keys, and no part in authority or prepared
//! mechanics.
//!
//! ## Demotion changes cadence, never state
//!
//! The summary is written **before** the activation is cleared, and a summary
//! that will not construct leaves the domain **promoted** and pages — never
//! demoted-without-summary. Nothing about the domain's own record, its
//! placements or its subtree is touched by either direction, which is why state
//! survives promote → demote → promote byte-identically.

use std::collections::BTreeMap;

use mush_core::{ClockRef, ClockStamp, Outbox};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::data::{DomainId, push_op};

/// The hard limit on simultaneously promoted (tactical) domains (§7). At the
/// limit the next trigger **queues and re-evaluates**, never drops (§9 q1).
pub const MAX_ACTIVE_DOMAINS: usize = 8;
/// How long a domain must stand quiet before it may demote (§7), in seconds of
/// the world's IC clock.
pub const QUIET_BOUNDARY: i64 = 10;

/// Why a domain promoted — the four deterministic triggers (10b §3.6).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromotionTrigger {
    /// A player character is present in a contested context.
    PlayerPresent,
    /// A credible contact between opposed parties.
    CredibleContact,
    /// An interception along a journey leg.
    Interception,
    /// A declared operation was activated (cut 9).
    OperationActivated,
}

impl PromotionTrigger {
    /// Parse the staff-facing token. `None` for anything else — the caller
    /// refuses rather than guessing a trigger.
    #[must_use]
    pub fn parse(token: &str) -> Option<PromotionTrigger> {
        match token.trim().to_ascii_lowercase().as_str() {
            "player" | "playerpresent" | "player_present" => Some(PromotionTrigger::PlayerPresent),
            "contact" | "crediblecontact" | "credible_contact" => {
                Some(PromotionTrigger::CredibleContact)
            }
            "interception" | "intercept" => Some(PromotionTrigger::Interception),
            "operation" | "operationactivated" | "operation_activated" => {
                Some(PromotionTrigger::OperationActivated)
            }
            _ => None,
        }
    }

    /// The staff-facing token.
    #[must_use]
    pub fn token(self) -> &'static str {
        match self {
            PromotionTrigger::PlayerPresent => "player",
            PromotionTrigger::CredibleContact => "contact",
            PromotionTrigger::Interception => "interception",
            PromotionTrigger::OperationActivated => "operation",
        }
    }
}

/// Why a domain workflow record refused to be constructed or to re-enter.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DomainWorkflowError {
    /// A workflow record is stamped on the world IC timeline and nothing else.
    #[error("a domain workflow record must be stamped on ClockRef::WorldIc")]
    NotWorldIc,
    /// The two stamps name different clock generations, so the window they
    /// describe cannot be compared at all.
    #[error("a domain summary's window must lie on one clock generation")]
    ClockMismatch,
    /// The window runs backwards.
    #[error("a domain summary's window ends before it starts")]
    WindowBackwards,
}

/// A **request** to promote a domain to tactical cadence. Cut 8's authority
/// applies it; cut 6 only writes it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "DomainActivationWire")]
pub struct DomainActivation {
    domain: DomainId,
    trigger: PromotionTrigger,
    at: ClockStamp,
    units: u32,
    queued: bool,
}

/// The strict wire form of [`DomainActivation`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainActivationWire {
    /// The domain.
    pub domain: DomainId,
    /// Why it promoted.
    pub trigger: PromotionTrigger,
    /// The world-IC stamp of the request.
    pub at: ClockStamp,
    /// How many objects were local to the domain when it was requested.
    pub units: u32,
    /// Whether the request is **queued** behind [`MAX_ACTIVE_DOMAINS`] rather
    /// than live. A queued request is re-evaluated, never dropped (§9 q1).
    #[serde(default)]
    pub queued: bool,
}

impl DomainActivation {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`DomainWorkflowError::NotWorldIc`] for any clock but the world IC
    /// timeline.
    pub fn new(
        domain: DomainId,
        trigger: PromotionTrigger,
        at: ClockStamp,
        units: u32,
        queued: bool,
    ) -> Result<DomainActivation, DomainWorkflowError> {
        if !matches!(at.clock, ClockRef::WorldIc { .. }) {
            return Err(DomainWorkflowError::NotWorldIc);
        }
        Ok(DomainActivation {
            domain,
            trigger,
            at,
            units,
            queued,
        })
    }

    /// The domain.
    #[must_use]
    pub fn domain(&self) -> DomainId {
        self.domain
    }
    /// Why it promoted.
    #[must_use]
    pub fn trigger(&self) -> PromotionTrigger {
        self.trigger
    }
    /// The world-IC stamp of the request.
    #[must_use]
    pub fn at(&self) -> &ClockStamp {
        &self.at
    }
    /// How many objects were local when it was requested.
    #[must_use]
    pub fn units(&self) -> u32 {
        self.units
    }
    /// Whether the request is queued behind the limit rather than live.
    #[must_use]
    pub fn is_queued(&self) -> bool {
        self.queued
    }
    /// The same request, promoted off the queue.
    #[must_use]
    pub fn dequeued(mut self) -> DomainActivation {
        self.queued = false;
        self
    }
}

impl From<DomainActivation> for DomainActivationWire {
    fn from(value: DomainActivation) -> DomainActivationWire {
        DomainActivationWire {
            domain: value.domain,
            trigger: value.trigger,
            at: value.at,
            units: value.units,
            queued: value.queued,
        }
    }
}

impl TryFrom<DomainActivationWire> for DomainActivation {
    type Error = DomainWorkflowError;

    fn try_from(value: DomainActivationWire) -> Result<DomainActivation, DomainWorkflowError> {
        DomainActivation::new(
            value.domain,
            value.trigger,
            value.at,
            value.units,
            value.queued,
        )
    }
}

impl<'de> Deserialize<'de> for DomainActivation {
    fn deserialize<D>(deserializer: D) -> Result<DomainActivation, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DomainActivationWire::deserialize(deserializer)?;
        DomainActivation::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// The durable record a demotion must write **before** the cadence changes.
///
/// 10b §3.6 drafts `damage: Vec<UnitDamageLine>` and `facts: Vec<ExternalRef>`;
/// neither type exists before cuts 7–9 declare what damage and external facts
/// are, so cut 6 records the two counts it can actually observe and leaves the
/// lines to the cut that owns them (recorded as a deviation in §3.6 As built).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "DomainSummaryWire")]
pub struct DomainSummary {
    domain: DomainId,
    from: ClockStamp,
    to: ClockStamp,
    contacts: u32,
    units: u32,
}

/// The strict wire form of [`DomainSummary`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainSummaryWire {
    /// The domain.
    pub domain: DomainId,
    /// When the tactical window opened.
    pub from: ClockStamp,
    /// When it closed.
    pub to: ClockStamp,
    /// Contacts observed in the window (0 until cut 7 declares contacts).
    pub contacts: u32,
    /// Objects local to the domain when it closed.
    pub units: u32,
}

impl DomainSummary {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`DomainWorkflowError::NotWorldIc`] for any clock but the world IC
    /// timeline, [`DomainWorkflowError::ClockMismatch`] when the window's two
    /// ends lie on different generations (they could not be compared at all),
    /// and [`DomainWorkflowError::WindowBackwards`] when it ends before it
    /// starts.
    pub fn new(
        domain: DomainId,
        from: ClockStamp,
        to: ClockStamp,
        contacts: u32,
        units: u32,
    ) -> Result<DomainSummary, DomainWorkflowError> {
        if !matches!(from.clock, ClockRef::WorldIc { .. }) {
            return Err(DomainWorkflowError::NotWorldIc);
        }
        if from.clock != to.clock {
            return Err(DomainWorkflowError::ClockMismatch);
        }
        if to.at < from.at {
            return Err(DomainWorkflowError::WindowBackwards);
        }
        Ok(DomainSummary {
            domain,
            from,
            to,
            contacts,
            units,
        })
    }

    /// The domain.
    #[must_use]
    pub fn domain(&self) -> DomainId {
        self.domain
    }
    /// When the window opened.
    #[must_use]
    pub fn from(&self) -> &ClockStamp {
        &self.from
    }
    /// When it closed.
    #[must_use]
    pub fn to(&self) -> &ClockStamp {
        &self.to
    }
    /// Contacts observed.
    #[must_use]
    pub fn contacts(&self) -> u32 {
        self.contacts
    }
    /// Objects local when it closed.
    #[must_use]
    pub fn units(&self) -> u32 {
        self.units
    }
    /// The window's length on the one generation both ends share.
    #[must_use]
    pub fn elapsed(&self) -> i64 {
        self.to.at.saturating_sub(self.from.at)
    }
}

impl From<DomainSummary> for DomainSummaryWire {
    fn from(value: DomainSummary) -> DomainSummaryWire {
        DomainSummaryWire {
            domain: value.domain,
            from: value.from,
            to: value.to,
            contacts: value.contacts,
            units: value.units,
        }
    }
}

impl TryFrom<DomainSummaryWire> for DomainSummary {
    type Error = DomainWorkflowError;

    fn try_from(value: DomainSummaryWire) -> Result<DomainSummary, DomainWorkflowError> {
        DomainSummary::new(
            value.domain,
            value.from,
            value.to,
            value.contacts,
            value.units,
        )
    }
}

impl<'de> Deserialize<'de> for DomainSummary {
    fn deserialize<D>(deserializer: D) -> Result<DomainSummary, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = DomainSummaryWire::deserialize(deserializer)?;
        DomainSummary::try_from(wire).map_err(serde::de::Error::custom)
    }
}

// ----------------------------------------------------------------------
// The state slot
// ----------------------------------------------------------------------

/// The module's domain-workflow slot: the live and queued activation requests,
/// and the last summary each domain closed with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivationBook {
    /// Activation requests by domain (live and queued).
    pub activations: BTreeMap<DomainId, DomainActivation>,
    /// The last summary each domain closed with.
    pub summaries: BTreeMap<DomainId, DomainSummary>,
}

impl ActivationBook {
    /// How many domains are **live** (promoted, not queued).
    #[must_use]
    pub fn live(&self) -> usize {
        self.activations.values().filter(|a| !a.is_queued()).count()
    }

    /// Whether `domain` is already promoted (a fifth trigger on it must not
    /// re-promote).
    #[must_use]
    pub fn is_promoted(&self, domain: DomainId) -> bool {
        self.activations
            .get(&domain)
            .is_some_and(|a| !a.is_queued())
    }

    /// The lowest-id queued request, if any — the one a freed slot takes.
    #[must_use]
    pub fn next_queued(&self) -> Option<DomainId> {
        self.activations
            .iter()
            .find(|(_, a)| a.is_queued())
            .map(|(id, _)| *id)
    }
}

/// Re-evaluate the queue: while a slot is free, promote the lowest-id queued
/// request. Returns the domains that came off the queue, so the caller persists
/// exactly those. Bounded by [`MAX_ACTIVE_DOMAINS`] iterations — a queue can
/// never make one call unbounded.
pub(crate) fn drain_queue(book: &mut ActivationBook) -> Vec<DomainId> {
    let mut promoted = Vec::new();
    for _ in 0..MAX_ACTIVE_DOMAINS {
        if book.live() >= MAX_ACTIVE_DOMAINS {
            break;
        }
        let Some(id) = book.next_queued() else { break };
        let Some(request) = book.activations.remove(&id) else {
            break;
        };
        book.activations.insert(id, request.dequeued());
        promoted.push(id);
    }
    promoted
}

// ----------------------------------------------------------------------
// Persistence
// ----------------------------------------------------------------------

/// The doc key of an activation request.
#[must_use]
pub fn activation_key(domain: DomainId) -> SmolStr {
    SmolStr::new(format!("activation:{}", domain.0))
}

/// The doc key of a domain summary.
#[must_use]
pub fn summary_key(domain: DomainId) -> SmolStr {
    SmolStr::new(format!("summary:{}", domain.0))
}

pub(crate) fn persist_activation(out: &mut Outbox, a: &DomainActivation) {
    match serde_json::to_value(a) {
        Ok(value) => push_op(
            out,
            mush_core::ModuleWrite::JsonSet {
                key: activation_key(a.domain),
                value,
            },
        ),
        // E7 cut 6 fold, R12: this runs from a command and from boot, both in
        // the world task, so it pages instead of panicking.
        Err(error) => tracing::error!(
            domain = %a.domain,
            %error,
            "sim: a domain activation would not serialize; the document was NOT written"
        ),
    }
}

pub(crate) fn persist_activation_del(out: &mut Outbox, domain: DomainId) {
    push_op(
        out,
        mush_core::ModuleWrite::Del {
            key: activation_key(domain),
        },
    );
}

pub(crate) fn persist_summary(out: &mut Outbox, s: &DomainSummary) -> bool {
    match serde_json::to_value(s) {
        Ok(value) => {
            push_op(
                out,
                mush_core::ModuleWrite::JsonSet {
                    key: summary_key(s.domain),
                    value,
                },
            );
            true
        }
        // E7 cut 6 fold, R12. A summary that will not serialize must NOT be
        // followed by the activation delete: the caller reads this and leaves
        // the domain promoted rather than demoting it without a summary.
        Err(error) => {
            tracing::error!(
                domain = %s.domain,
                %error,
                "sim: a domain summary would not serialize; the domain STAYS PROMOTED"
            );
            false
        }
    }
}

// ----------------------------------------------------------------------
// Boot
// ----------------------------------------------------------------------

/// Pull `activation:*` / `summary:*` documents out of the boot kv and re-enter
/// each through its `TryFrom` wire. A corrupt document is skipped **loudly** and
/// its subject left un-promoted — never silently defaulted, and never a boot
/// failure (a bad workflow leaf must not brick a world).
pub(crate) fn extract_domain_workflow(data: &mut mush_core::ModuleBootData) -> ActivationBook {
    let mut book = ActivationBook::default();
    data.kv.retain(|(key, value)| {
        if key.starts_with("activation:") {
            match serde_json::from_value::<DomainActivation>(value.clone()) {
                Ok(a) if activation_key(a.domain) == *key => {
                    book.activations.insert(a.domain, a);
                }
                Ok(a) => {
                    tracing::error!(%key, domain = %a.domain, "sim: activation doc key/body mismatch dropped");
                }
                Err(error) => {
                    tracing::error!(%key, %error, "sim: unreadable activation doc dropped");
                }
            }
            false
        } else if key.starts_with("summary:") {
            match serde_json::from_value::<DomainSummary>(value.clone()) {
                Ok(s) if summary_key(s.domain) == *key => {
                    book.summaries.insert(s.domain, s);
                }
                Ok(s) => {
                    tracing::error!(%key, domain = %s.domain, "sim: summary doc key/body mismatch dropped");
                }
                Err(error) => {
                    tracing::error!(%key, %error, "sim: unreadable summary doc dropped");
                }
            }
            false
        } else {
            true
        }
    });
    book
}

/// Install the hydrated workflow records, drop the activations that can no
/// longer be live, and **re-evaluate the queue** (E7 cut 6 fold, R5/B3).
///
/// Two things free a tactical slot without any command running: an activation
/// document the wire refused (dropped by `extract_domain_workflow`), and an
/// activation for a domain that has since been retired, paused or deleted. Both
/// lower the live count below [`MAX_ACTIVE_DOMAINS`] — and before the fold
/// nothing drained the queue except `@domain/promote|demote`, so a world could
/// sit at seven live with a request queued indefinitely, breaking §9 q1's "a
/// queued trigger re-evaluates whenever a slot frees, never dropped".
pub(crate) fn install_domain_workflow(ctx: &mut mush_core::Ctx<'_>, mut book: ActivationBook) {
    let stale: Vec<DomainId> = {
        let live_domains = ctx.w.module_state::<crate::data::SpatialState>();
        book.activations
            .keys()
            .copied()
            .filter(|id| {
                live_domains.is_some_and(|st| {
                    st.domains
                        .get(id)
                        .is_none_or(|d| d.status != crate::data::DomainStatus::Active)
                })
            })
            .collect()
    };
    for id in stale {
        tracing::warn!(
            domain = %id,
            "sim: an activation for a domain that is not active was dropped at boot; its \
             tactical slot is released"
        );
        book.activations.remove(&id);
        persist_activation_del(ctx.out, id);
    }
    let promoted = drain_queue(&mut book);
    let records: Vec<DomainActivation> = promoted
        .iter()
        .filter_map(|id| book.activations.get(id).cloned())
        .collect();
    if let Some(slot) = ctx.w.module_state_mut::<ActivationBook>() {
        *slot = book;
    }
    for record in &records {
        persist_activation(ctx.out, record);
        tracing::info!(domain = %record.domain, "sim: a queued domain came off the queue at boot");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(at: i64) -> ClockStamp {
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at,
        }
    }

    fn request(domain: u64, queued: bool) -> DomainActivation {
        DomainActivation::new(
            DomainId(domain),
            PromotionTrigger::PlayerPresent,
            stamp(100),
            3,
            queued,
        )
        .expect("a coherent activation")
    }

    /// Both wires re-run every constructor invariant, so a stored workflow
    /// record that could never have been written is refused on load (§4 rule 1).
    #[test]
    fn e7c6_b5_the_workflow_wires_re_run_every_invariant() {
        let text = serde_json::to_string(&request(2, false)).expect("serializes");
        assert_eq!(
            serde_json::from_str::<DomainActivation>(&text).expect("re-enters"),
            request(2, false)
        );
        assert_eq!(
            DomainActivation::new(
                DomainId(0),
                PromotionTrigger::Interception,
                ClockStamp {
                    clock: ClockRef::Utc,
                    at: 0
                },
                0,
                false,
            ),
            Err(DomainWorkflowError::NotWorldIc)
        );
        let summary = DomainSummary::new(DomainId(1), stamp(10), stamp(40), 0, 2)
            .expect("a coherent summary");
        assert_eq!(summary.elapsed(), 30);
        let text = serde_json::to_string(&summary).expect("serializes");
        assert_eq!(
            serde_json::from_str::<DomainSummary>(&text).expect("re-enters"),
            summary
        );
        assert_eq!(
            DomainSummary::new(DomainId(1), stamp(40), stamp(10), 0, 0),
            Err(DomainWorkflowError::WindowBackwards)
        );
        assert_eq!(
            DomainSummary::new(
                DomainId(1),
                stamp(10),
                ClockStamp {
                    clock: ClockRef::WorldIc { generation: 2 },
                    at: 40
                },
                0,
                0,
            ),
            Err(DomainWorkflowError::ClockMismatch)
        );
        // An unknown field is refused rather than ignored.
        assert!(
            serde_json::from_str::<DomainActivation>(
                "{\"domain\":0,\"trigger\":\"PlayerPresent\",\"at\":{\"clock\":{\"WorldIc\":\
                 {\"generation\":1}},\"at\":0},\"units\":0,\"nonsense\":1}"
            )
            .is_err()
        );
    }

    /// At the limit a trigger QUEUES and re-evaluates; it is never dropped, and
    /// a freed slot takes the lowest-id queued request (§9 q1).
    #[test]
    fn e7c6_b5_the_queue_re_evaluates_and_never_drops() {
        let mut book = ActivationBook::default();
        for d in 0..MAX_ACTIVE_DOMAINS as u64 {
            book.activations.insert(DomainId(d), request(d, false));
        }
        assert_eq!(book.live(), MAX_ACTIVE_DOMAINS);
        // Two more arrive at the limit: both queue, neither is lost.
        book.activations.insert(DomainId(20), request(20, true));
        book.activations.insert(DomainId(21), request(21, true));
        assert_eq!(drain_queue(&mut book), Vec::<DomainId>::new());
        assert_eq!(book.activations.len(), MAX_ACTIVE_DOMAINS + 2);
        // A slot frees: the lowest-id queued request takes it, exactly one.
        book.activations.remove(&DomainId(0));
        assert_eq!(drain_queue(&mut book), vec![DomainId(20)]);
        assert!(book.is_promoted(DomainId(20)));
        assert!(!book.is_promoted(DomainId(21)));
        assert_eq!(book.next_queued(), Some(DomainId(21)));
    }
}
