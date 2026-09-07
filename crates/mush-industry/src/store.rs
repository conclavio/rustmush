//! E7c3 B7 — boot hydration, the deposit quarantine, and the fact mirror.
//!
//! # The document table, and the direction of every loss
//!
//! Cut 3 owns **one** conventional document — the survey disclosure map — and
//! five authoritative typed families. The kernel's hydration rule is the
//! stricter one and applies to the five: `ActionStateStore::hydrate` is atomic,
//! and a record that fails path, envelope, schema, bounds, typed validation or
//! canonical decode→serialize equality **refuses the boot**.
//!
//! | family / document | class | corrupt → | direction of the loss |
//! |---|---|---|---|
//! | `industry.deposit` (`OreDeposit`) | authoritative | **boot refuses** | the curve and `accessibility` decide a cycle's yield *and* whether a cycle is legal; a body nobody can decode would either stop producing silently or produce on a default |
//! | `industry.claim` (`ClaimRecord`) | authoritative | **boot refuses** | the fail-open shape here is *"the missing claim was the one that had been seized"* — losing it would let a seized holder extract again |
//! | `industry.run` (`ExtractionRun`) | authoritative | **boot refuses** | losing `next_cycle` reuses a deterministic key, so a replayed cycle would read as fresh |
//! | `industry.applied` (`AppliedMarker`) | authoritative | **boot refuses** | a marker is what a settlement points at; unreadable must read as neither "happened" nor "did not" |
//! | `industry.place_census` / `industry.claim_census` | authoritative | **boot refuses** | a lost census is a cap that stops being enforced, silently |
//! | `industry:survey:<deposit>` (`SurveyDoc`) | **exempt**, legacy-post-image | **skipped loudly, deposit marked** | the loss is *disclosure only*: no legality check reads a band, so a dropped row costs a surveyor their knowledge and costs nobody their authority. It is refused rather than truncated past `MAX_SURVEY_BANDS`, because a silently dropped row is a principal who was told something the record then denies |
//!
//! # What this module adds on top
//!
//! The kernel decodes each family independently and cannot see across them, or
//! across crates. Three cross-cutting shapes are checked here, and all three
//! **quarantine the deposit** — every `@extract` and every `@claim` mutation
//! against it refuses until it is reconciled, and its pool never rises:
//!
//! * the economy hydrated **no vault** for the endpoint the deposit names
//!   (`Quarantine::VaultMissing`);
//! * the endpoint's pool is **half-present** — cut 2's own quarantine, asked
//!   through `InventoryStore` (`Quarantine::EndpointQuarantined`);
//! * the survey document for the deposit failed its wire
//!   (`Quarantine::SurveyLost`).
//!
//! A **fully-absent** pool pair at a still-hydrated vault (both `inventory.pool`
//! and `inventory.book` gone, which the kernel does not refuse at boot because
//! *missing* is not *corrupt*) is **not** a distinct quarantine shape: it
//! degrades to `Depleted` by design (E7c3 verify fold R2). That is fail-closed —
//! `remaining()` reads 0, `next_yield` is 0, and extraction refuses — and it is
//! why cut 3 constructs no `NoPool` variant. A fully-absent pool at a hydrated
//! vault is unreachable through this crate's own verbs (`open_deposit` seeds the
//! pool in the same bundle as the record), so the shape can only arise from
//! external corruption, and the fail-closed degradation is the honest answer.
//!
//! The load-bearing rule is that **a skipped record must not raise
//! authorization or availability**. A quarantined deposit authorizes nobody and
//! yields nothing; it does not quietly become an unclaimed one.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{ObjId, Outbox, World};
use mush_economy::{EconomyStore, InstrumentId, VaultId};
use mush_inventory::{EndpointRef, InventoryStore};
use smol_str::SmolStr;

use crate::facility::{FacilityRecord, facility_of_key};
use crate::record::{
    AppliedMarker, MAX_APPLIED_MARKERS, MAX_MARKER_ID_BYTES, NS, OreDeposit, OreDepositId,
    SurveyDoc,
};
use crate::workorder::WorkOrderMarker;

/// The document key one deposit's disclosure map lives at.
#[must_use]
pub fn survey_key(deposit: OreDepositId) -> SmolStr {
    SmolStr::new(format!("survey:{:016x}", deposit.0))
}

/// The deposit a survey document key names, if it is one.
#[must_use]
pub fn survey_of_key(key: &str) -> Option<OreDepositId> {
    let body = key.strip_prefix("survey:")?;
    (body.len() == 16)
        .then(|| u64::from_str_radix(body, 16).ok())
        .flatten()
        .map(OreDepositId)
}

/// Why one ore body is quarantined, and the reconciliation a staffer runs.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Quarantine {
    /// The economy has no vault record for the endpoint the deposit names.
    VaultMissing,
    /// Cut 2 quarantined the endpoint the deposit pools at.
    EndpointQuarantined,
    /// The survey document for this deposit failed its wire and was skipped.
    SurveyLost,
}

/// Why one facility is quarantined (R4, independent verify fold), and the
/// reconciliation a staffer runs. A quarantined facility's availability reads 0
/// and every submit against it refuses — the fail-closed direction §3.4's
/// document table calls the only one forbidden to reverse.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FacilityQuarantine {
    /// A live (`Running`/`Interrupted`/`AwaitingDelivery`/`Ready`) work order
    /// holds a slot at this facility, but its authoritative capacity book
    /// (`industry.facility_slots` **and** `industry.facility_book`) is **wholly
    /// absent** — the "missing is not corrupt" shape the kernel does not refuse at
    /// boot. Reading `maximum_slots` here would let availability RISE with a live
    /// reservation gone; instead the facility reads 0 and refuses until reconciled.
    CapacityLost,
}

impl FacilityQuarantine {
    /// The one line a staff surface prints.
    #[must_use]
    pub const fn runbook(self) -> &'static str {
        match self {
            Self::CapacityLost => {
                "a live work order holds a slot here but this facility's capacity book is gone; \
                 availability reads 0 and every submit refuses. Restore the capacity records or \
                 cancel the stranded orders, then re-run the sweep"
            }
        }
    }
}

impl Quarantine {
    /// The one line a staff surface prints, with the order filled in. Staff
    /// intervention is the last resort: this names a decision, never a chore
    /// (08c §4.6, and never park state on "wait for staff").
    #[must_use]
    pub const fn runbook(self) -> &'static str {
        match self {
            Self::VaultMissing => {
                "the economy hydrated no vault for this body's endpoint, so its reserve is \
                 unreachable; restore the vault record (`@vault/show <id>`), then \
                 `@deposit/show <id>`"
            }
            Self::EndpointQuarantined => {
                "cut 2 quarantined this body's endpoint; run `@inv/quarantine`, reconcile the \
                 endpoint it names, then `@deposit/show <id>`"
            }
            Self::SurveyLost => {
                "this body's survey disclosure document failed its wire and was skipped; the \
                 bands every surveyor was told are lost. Restore the document or accept the \
                 loss with `@deposit/show <id>` — no authority depends on it"
            }
        }
    }
}

/// The module's RAM projection: the one conventional document it owns, what
/// boot derived, and what a `&World`-only reader (the fact provider) may see.
#[derive(Clone, Debug, Default)]
pub struct IndustryStore {
    /// Deposits whose cross-family state disagreed, and why.
    quarantined: BTreeMap<OreDepositId, Quarantine>,
    /// E7c4 R4: facilities whose live work orders reference a capacity book that
    /// is wholly absent — the fail-open both-absent shape, fenced fail-closed.
    facility_quarantined: BTreeMap<ObjId, FacilityQuarantine>,
    /// The one conventional document: per-deposit disclosure.
    surveys: BTreeMap<OreDepositId, SurveyDoc>,
    /// E7c4 B1: the facility definition documents (`industry:facility:<obj>`),
    /// exempt/legacy-post-image, hydrated at boot and read at draft/offer time.
    facilities: BTreeMap<ObjId, FacilityRecord>,
    /// The extraction applied-marker mirror the `industry.*` fact provider answers
    /// from.
    applied: BTreeMap<SmolStr, AppliedMarker>,
    /// E7c4 B7: the work-order completion-marker mirror the SAME provider's second
    /// arm (`industry.workorder.completed`) answers from (DECIDED (f)).
    wo_applied: BTreeMap<SmolStr, WorkOrderMarker>,
    /// Whether the last reconciliation completed inside its page budget **and**
    /// held the whole marker family in the RAM mirror. A truncated walk — the
    /// page budget tripped, or the mirror pruned past `MAX_APPLIED_MARKERS`
    /// (E7c3 verify fold R1) — leaves the store **fail-closed**: it does not
    /// claim the mirror is complete, and `facts::resolve` answers nothing at
    /// all, for the newest fact as much as the oldest. Never a non-monotone
    /// flip. The durable family is unbounded until cut 4 prunes it, so past the
    /// cap facts go dark honestly and loudly rather than answering some.
    complete: bool,
    /// How many reconciliations have run — the pin that keeps the mirror off
    /// the command path (cut 2's fold B4).
    reconciliations: u64,
    /// The extraction-licence instrument this deployment names. `None` refuses
    /// every extraction, loudly and by design: a server that has not said what
    /// a licence *is* must not let anyone extract without one.
    licence: Option<InstrumentId>,
    /// IC second the last mutating submission was stamped with, and its ordinal
    /// inside that second (cut 2's fold R1(b)). Two deliberately retyped
    /// intents are two requests; one submission driven twice is still one.
    attempt_at: i64,
    attempt_seq: u32,
    /// **E7r1 R1** — the budget sweep's rotation cursor: the encoded work-order key
    /// the next fire resumes *after*. `None` restarts at the head, which is both the
    /// boot state and what running off the end means, so no order is ever
    /// permanently past the cursor. It is RAM-only on purpose: the arm is a no-op
    /// past the applied point (the hold's own status is the marker), so a reboot
    /// re-running it from the head *is* the reconcile.
    ///
    /// **Coverage, recorded beside correctness (independent verify fold, D9).** That
    /// argument is exactly right about the *correctness* of repeats and says nothing
    /// about *coverage*. Terminal orders are never pruned from the family,
    /// `MAX_WO_VISITED_PER_FIRE` is 64 and `SWEEP_PERIOD_SECS` is 60, so a world with
    /// several hundred orders that restarts more often than a full rotation only ever
    /// settles the first `MAX_WO_SETTLE_PER_FIRE` unsettled orders per boot session.
    /// It **converges** rather than starving — a settled order is a no-op on the next
    /// pass, so each boot session advances by the settle budget — which is why this is
    /// a recorded deferral (§15.3) and not a fix. Promoting it means persisting the
    /// cursor in the conventional document beside `licence`.
    settle_cursor: Option<String>,
    /// **E7r1 R1** — orders seen once holding a budget reservation they should not
    /// still have (a `Draft` whose submission was refused after the place). The
    /// compensating release runs on the **second** consecutive sighting, never the
    /// first: a hold placed by the command and a submission still in the same drain
    /// are indistinguishable to a walk, and two Coarse fires apart they are not.
    settle_pending: BTreeSet<crate::workorder::WorkOrderId>,
    /// How many notes were dropped at [`crate::money::MAX_SETTLE_PENDING`] since
    /// boot — counted rather than silent (D9).
    settle_pending_dropped: u64,
}

impl IndustryStore {
    /// Name the extraction-licence instrument.
    pub fn set_licence(&mut self, instrument: Option<InstrumentId>) {
        self.licence = instrument;
    }

    /// The extraction-licence instrument, if this deployment names one.
    #[must_use]
    pub const fn licence(&self) -> Option<&InstrumentId> {
        self.licence.as_ref()
    }

    /// **E7r1 R1** — where the budget sweep resumes.
    #[must_use]
    pub fn settle_cursor(&self) -> Option<&str> {
        self.settle_cursor.as_deref()
    }

    /// Advance the budget sweep past the last work order it **visited**.
    pub fn set_settle_cursor(&mut self, cursor: Option<String>) {
        self.settle_cursor = cursor;
    }

    /// Note one order as carrying an unexpected reservation; `true` once it has been
    /// seen on **two** consecutive fires and the compensating release is due.
    ///
    /// Bounded by [`crate::money::MAX_SETTLE_PENDING`] (independent verify fold, D9):
    /// the set had no declared cap and no constant ceiling, only requesters ×
    /// `MAX_WORK_ORDERS_PER_REQUESTER`. Past the cap a **fresh** note is dropped and
    /// counted rather than inserted — an order already noted still promotes — so the
    /// only cost is that one orphan waits another rotation, which the attempt-scoped
    /// settle key makes safe.
    pub fn note_settle_pending(&mut self, order: crate::workorder::WorkOrderId) -> bool {
        if self.settle_pending.contains(&order) {
            return true;
        }
        if self.settle_pending.len() >= crate::money::MAX_SETTLE_PENDING {
            self.settle_pending_dropped = self.settle_pending_dropped.saturating_add(1);
            return false;
        }
        self.settle_pending.insert(order);
        false
    }

    /// How many settlement notes were dropped at the cap since boot (D9).
    #[must_use]
    pub const fn settle_pending_dropped(&self) -> u64 {
        self.settle_pending_dropped
    }

    /// How many orders currently carry a settlement note (D9's observable).
    #[must_use]
    pub fn settle_pending_len(&self) -> usize {
        self.settle_pending.len()
    }

    /// Forget one order's pending note (its state moved on, or the release landed).
    pub fn clear_settle_pending(&mut self, order: crate::workorder::WorkOrderId) {
        self.settle_pending.remove(&order);
    }

    /// Whether one body is quarantined.
    #[must_use]
    pub fn is_quarantined(&self, deposit: OreDepositId) -> bool {
        self.quarantined.contains_key(&deposit)
    }

    /// Why one body is quarantined.
    #[must_use]
    pub fn quarantine(&self, deposit: OreDepositId) -> Option<Quarantine> {
        self.quarantined.get(&deposit).copied()
    }

    /// Every quarantined body, in canonical order.
    #[must_use]
    pub fn quarantined(&self) -> Vec<(OreDepositId, Quarantine)> {
        self.quarantined
            .iter()
            .map(|(id, why)| (*id, *why))
            .collect()
    }

    /// Whether one facility is quarantined (R4): its capacity book is gone while a
    /// live work order still holds a slot there.
    #[must_use]
    pub fn is_facility_quarantined(&self, object: ObjId) -> bool {
        self.facility_quarantined.contains_key(&object)
    }

    /// Why one facility is quarantined.
    #[must_use]
    pub fn facility_quarantine(&self, object: ObjId) -> Option<FacilityQuarantine> {
        self.facility_quarantined.get(&object).copied()
    }

    /// Every quarantined facility, in canonical order.
    #[must_use]
    pub fn facilities_quarantined(&self) -> Vec<(ObjId, FacilityQuarantine)> {
        self.facility_quarantined
            .iter()
            .map(|(id, why)| (*id, *why))
            .collect()
    }

    /// One body's disclosure document.
    #[must_use]
    pub fn survey(&self, deposit: OreDepositId) -> Option<&SurveyDoc> {
        self.surveys.get(&deposit)
    }

    /// Install one disclosure document (the single RAM write path).
    pub fn put_survey(&mut self, doc: SurveyDoc) {
        self.surveys.insert(doc.deposit(), doc);
    }

    /// Whether the marker mirror is complete enough to answer a fact.
    #[must_use]
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    /// How many reconciliations have run.
    #[must_use]
    pub const fn reconciliations(&self) -> u64 {
        self.reconciliations
    }

    /// One extraction applied marker, if the mirror has it.
    #[must_use]
    pub fn marker(&self, id: &str) -> Option<&AppliedMarker> {
        self.applied.get(id)
    }

    /// One work-order completion marker, if the mirror has it.
    #[must_use]
    pub fn wo_marker(&self, id: &str) -> Option<&WorkOrderMarker> {
        self.wo_applied.get(id)
    }

    /// One facility definition document, if hydrated.
    #[must_use]
    pub fn facility(&self, object: ObjId) -> Option<&FacilityRecord> {
        self.facilities.get(&object)
    }

    /// Install one facility definition document (the single RAM write path).
    pub fn put_facility(&mut self, facility: FacilityRecord) {
        self.facilities.insert(facility.object(), facility);
    }

    /// Every hydrated facility, in canonical order.
    #[must_use]
    pub fn facilities(&self) -> Vec<&FacilityRecord> {
        self.facilities.values().collect()
    }

    /// Every mirrored marker of one kind, newest last.
    #[must_use]
    pub fn markers_of(&self, kind: &str) -> Vec<&AppliedMarker> {
        self.applied
            .values()
            .filter(|marker| marker.kind() == kind)
            .collect()
    }

    /// The next ordinal inside one IC second, for a submission discriminator.
    pub fn next_attempt(&mut self, at: i64) -> u32 {
        if self.attempt_at == at {
            self.attempt_seq = self.attempt_seq.saturating_add(1);
        } else {
            self.attempt_at = at;
            self.attempt_seq = 0;
        }
        self.attempt_seq
    }
}

/// Load the one conventional document family, skipping a corrupt one **loudly**
/// and marking its subject.
///
/// # Panics
/// Never: every arm is total.
pub fn hydrate(store: &mut IndustryStore, kv: &[(SmolStr, serde_json::Value)]) {
    let mut lost = BTreeSet::new();
    for (key, value) in kv {
        // E7c4 B1: facility definition documents — skipped LOUDLY on a wire
        // failure, exactly as a survey is; the loss is availability (a facility
        // whose document is gone offers no slots), the fail-closed direction.
        if let Some(object) = facility_of_key(key.as_str()) {
            match serde_json::from_value::<FacilityRecord>(value.clone()) {
                Ok(facility) if facility.object() == object => store.put_facility(facility),
                Ok(_) | Err(_) => {
                    tracing::error!(
                        target: "industry",
                        key = %key,
                        "industry: a facility document failed its wire or disagrees with its \
                         key; skipped"
                    );
                }
            }
            continue;
        }
        let Some(deposit) = survey_of_key(key.as_str()) else {
            // A key this crate does not own. Not an error: the namespace may
            // grow, and a boot must not refuse on a document it will simply
            // never read.
            continue;
        };
        match serde_json::from_value::<SurveyDoc>(value.clone()) {
            Ok(doc) if doc.deposit() == deposit => {
                store.put_survey(doc);
            }
            Ok(doc) => {
                // The key and the document disagree: a key is not proof of what
                // it names (cut 2's fold C10, same rule).
                tracing::error!(
                    target: "industry",
                    key = %key,
                    named = %doc.deposit(),
                    "industry: a survey document disagrees with its own key; skipped"
                );
                lost.insert(deposit);
            }
            Err(error) => {
                tracing::error!(
                    target: "industry",
                    key = %key,
                    %error,
                    "industry: a survey document failed its wire; skipped, and its body is marked"
                );
                lost.insert(deposit);
            }
        }
    }
    for deposit in lost {
        store.quarantined.insert(deposit, Quarantine::SurveyLost);
    }
}

/// Queue one facility definition document write (E7c4 B1, legacy-post-image).
pub fn persist_facility(out: &mut Outbox, facility: &FacilityRecord) {
    let Ok(value) = serde_json::to_value(facility) else {
        tracing::error!(
            target: "industry",
            object = facility.object().0,
            "industry: a facility document would not serialize; not written"
        );
        return;
    };
    out.defer(mush_core::Deferred::Persist(mush_core::ModuleOp {
        ns: SmolStr::new_static(NS),
        op: mush_core::ModuleWrite::JsonSet {
            key: crate::facility::facility_key(facility.object()),
            value,
        },
    }));
}

/// Queue one survey document write.
pub fn persist_survey(out: &mut Outbox, doc: &SurveyDoc) {
    let Ok(value) = serde_json::to_value(doc) else {
        tracing::error!(
            target: "industry",
            deposit = %doc.deposit(),
            "industry: a survey document would not serialize; not written"
        );
        return;
    };
    out.defer(mush_core::Deferred::Persist(mush_core::ModuleOp {
        ns: SmolStr::new_static(NS),
        op: mush_core::ModuleWrite::JsonSet {
            key: survey_key(doc.deposit()),
            value,
        },
    }));
}

/// The cross-family reconciliation the kernel cannot make, plus the marker
/// mirror refresh.
///
/// Bounded on records **visited**, never on records matched (D-LBK-5), and
/// deliberately **off the command path** — boot and the coarse sweep keep the
/// mirror, which is exactly what 08i answer 15 grants.
pub fn reconcile(ctx: &mut mush_core::ModuleCtx<'_>, families: &crate::IndustryFamilies) {
    let mut quarantined: BTreeMap<OreDepositId, Quarantine> = BTreeMap::new();
    let mut applied: BTreeMap<SmolStr, AppliedMarker> = BTreeMap::new();
    let mut complete = true;

    // ---- the deposits, and the three cross-cutting shapes ----
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        if pages >= crate::record::MAX_LIST_PAGES {
            complete = false;
            break;
        }
        pages += 1;
        let page = match ctx.scan_action_state(
            &families.deposits,
            cursor.as_deref(),
            crate::record::INDUSTRY_PAGE,
        ) {
            Ok(page) => page,
            Err(error) => {
                tracing::error!(
                    target: "industry",
                    %error,
                    "industry: the deposit family is unreadable at boot"
                );
                complete = false;
                break;
            }
        };
        let mut rows: Vec<OreDeposit> = Vec::new();
        for entry in page.entries() {
            rows.push(entry.value().clone());
        }
        let next = page.next_after().map(ToString::to_string);
        for deposit in rows {
            if let Some(why) = deposit_quarantine(ctx.w, &deposit) {
                quarantined.insert(deposit.id(), why);
            }
        }
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    // ---- the marker mirror the facts answer from ----
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    // E7c3 verify fold R1: cut 2 carries this flag and cut 3 dropped it. The RAM
    // mirror keeps only the newest `MAX_APPLIED_MARKERS`, but `is_complete()`
    // stayed true after a `pop_first`, so `facts::resolve` answered `None`
    // (unsatisfied) for the older-but-durable, committed markers while the newest
    // answered `Some` — a silent, non-monotone flip a reboot re-derived. A pruned
    // mirror is truncated, and a truncated mirror is fail-closed for the WHOLE
    // family, exactly as cut 2 arranges (same 4096 cap / 256-page budget).
    let mut truncated = false;
    loop {
        if pages >= crate::record::MAX_MARKER_PAGES {
            tracing::error!(
                target: "industry",
                "industry: the applied-marker walk hit its page budget; the fact mirror is \
                 incomplete and answers nothing"
            );
            complete = false;
            break;
        }
        pages += 1;
        let page = match ctx.scan_action_state(
            &families.applied,
            cursor.as_deref(),
            crate::record::INDUSTRY_PAGE,
        ) {
            Ok(page) => page,
            Err(error) => {
                tracing::error!(
                    target: "industry",
                    %error,
                    "industry: the applied-marker family is unreadable at boot"
                );
                complete = false;
                break;
            }
        };
        let mut rows: Vec<(SmolStr, AppliedMarker)> = Vec::new();
        for entry in page.entries() {
            if entry.encoded_key().len() <= MAX_MARKER_ID_BYTES {
                rows.push((SmolStr::new(entry.encoded_key()), entry.value().clone()));
            }
        }
        let next = page.next_after().map(ToString::to_string);
        for (key, marker) in rows {
            applied.insert(key, marker);
            // Canonical key order IS time order (the id carries the IC second
            // first), so keeping the newest is a `pop_first`, never a sort.
            while applied.len() > MAX_APPLIED_MARKERS {
                applied.pop_first();
                truncated = true;
            }
        }
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    if truncated {
        // A pruned mirror is not "behind by one sweep" — it is a mirror that
        // silently drops committed markers, so it answers NOTHING for the whole
        // family until the durable family is pruned (cut 4's seam). An operator
        // event, not a note.
        tracing::error!(
            target: "industry",
            markers = applied.len(),
            "industry: the applied-marker mirror is truncated; every industry fact answers \
             unsatisfied until the durable marker family is pruned (cut 4)"
        );
        complete = false;
    }

    // ---- E7c4 B7: the work-order completion-marker mirror ----
    let mut wo_applied: BTreeMap<SmolStr, WorkOrderMarker> = BTreeMap::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        if pages >= crate::record::MAX_MARKER_PAGES {
            complete = false;
            break;
        }
        pages += 1;
        let page = match ctx.scan_action_state(
            &families.wo_applied,
            cursor.as_deref(),
            crate::record::INDUSTRY_PAGE,
        ) {
            Ok(page) => page,
            Err(_) => {
                complete = false;
                break;
            }
        };
        let mut rows: Vec<(SmolStr, WorkOrderMarker)> = Vec::new();
        for entry in page.entries() {
            if entry.encoded_key().len() <= MAX_MARKER_ID_BYTES {
                rows.push((SmolStr::new(entry.encoded_key()), entry.value().clone()));
            }
        }
        let next = page.next_after().map(ToString::to_string);
        for (key, marker) in rows {
            wo_applied.insert(key, marker);
            while wo_applied.len() > MAX_APPLIED_MARKERS {
                wo_applied.pop_first();
                complete = false;
            }
        }
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }

    // ---- E7c4 R4: the work-order → capacity cross-family check ----
    // The kernel refuses a boot on a *corrupt* authoritative record, but "missing
    // is not corrupt": if a facility's capacity book (`industry.facility_slots`
    // AND `industry.facility_book`) is wholly absent while a live work order still
    // holds a slot there, `available_slots` would read `maximum_slots` — the one
    // fail-open direction §3.4 forbids. Quarantine such a facility so its
    // availability reads 0 and every submit refuses (its reservations stand — the
    // slot the vanished book held is not released). A corrupt (not absent)
    // authoritative work order still refuses the whole boot by design.
    let mut facility_quarantined: BTreeMap<ObjId, FacilityQuarantine> = BTreeMap::new();
    let mut cursor: Option<String> = None;
    let mut pages = 0;
    loop {
        if pages >= crate::record::MAX_LIST_PAGES {
            complete = false;
            break;
        }
        pages += 1;
        let page = match ctx.scan_action_state(
            &families.work_orders,
            cursor.as_deref(),
            crate::record::INDUSTRY_PAGE,
        ) {
            Ok(page) => page,
            Err(_) => {
                complete = false;
                break;
            }
        };
        let mut rows: Vec<crate::workorder::WorkOrder> = Vec::new();
        for entry in page.entries() {
            rows.push(entry.value().clone());
        }
        let next = page.next_after().map(ToString::to_string);
        for order in rows {
            if !order.state().holds_resources() {
                continue;
            }
            let facility = order.facility();
            if facility_quarantined.contains_key(&facility) {
                continue;
            }
            let Ok(key) = crate::facility::facility_capacity_key(facility) else {
                continue;
            };
            let book_present = ctx
                .action_state(&families.facility_books, &key)
                .ok()
                .flatten()
                .is_some();
            let scalar_present = ctx
                .action_state(&families.facility_slots, &key)
                .ok()
                .flatten()
                .is_some();
            if !book_present && !scalar_present {
                facility_quarantined.insert(facility, FacilityQuarantine::CapacityLost);
            }
        }
        match next {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
    for (facility, why) in &facility_quarantined {
        tracing::error!(
            target: "industry",
            facility = facility.0,
            runbook = why.runbook(),
            "industry: a facility is quarantined — a live work order's capacity book is gone"
        );
    }

    let Some(store) = ctx.w.module_state_mut::<IndustryStore>() else {
        return;
    };
    // A survey document that failed its wire at boot already marked its body;
    // keep those marks, because the document walk does not run again here.
    for (deposit, why) in &store.quarantined {
        if *why == Quarantine::SurveyLost {
            quarantined
                .entry(*deposit)
                .or_insert(Quarantine::SurveyLost);
        }
    }
    for (deposit, why) in &quarantined {
        tracing::error!(
            target: "industry",
            deposit = %deposit,
            runbook = why.runbook(),
            "industry: an ore body is quarantined"
        );
    }
    store.quarantined = quarantined;
    store.facility_quarantined = facility_quarantined;
    store.applied = applied;
    store.wo_applied = wo_applied;
    store.complete = complete;
    store.reconciliations = store.reconciliations.saturating_add(1);
}

/// The three cross-cutting shapes, asked of one deposit.
fn deposit_quarantine(world: &World, deposit: &OreDeposit) -> Option<Quarantine> {
    let endpoint = deposit.endpoint();
    let vault: VaultId = endpoint.vault();
    let known = world
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().vault(vault).cloned())
        .is_some();
    if !known {
        return Some(Quarantine::VaultMissing);
    }
    if world
        .module_state::<InventoryStore>()
        .is_some_and(|store| store.is_quarantined(EndpointRef(vault)))
    {
        return Some(Quarantine::EndpointQuarantined);
    }
    None
}

/// Whether one ore body is currently quarantined, for a command's own
/// pre-check.
#[must_use]
pub fn is_quarantined(world: &World, deposit: OreDepositId) -> bool {
    world
        .module_state::<IndustryStore>()
        .is_some_and(|store| store.is_quarantined(deposit))
}

/// Whether the actor is staff, re-derived locally because
/// `mush_economy::api::is_staff` is `pub(crate)` (the `mush-market` precedent).
#[must_use]
pub fn is_staff(world: &World, actor: ObjId) -> bool {
    mush_core::perm::has_power(world, actor, mush_core::Powers::SIM_ADMIN)
        || mush_core::perm::has_role(world, actor, mush_core::Role::Royalty)
}
