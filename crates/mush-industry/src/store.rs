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

use crate::record::{
    AppliedMarker, MAX_APPLIED_MARKERS, MAX_MARKER_ID_BYTES, NS, OreDeposit, OreDepositId,
    SurveyDoc,
};

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
    /// The one conventional document: per-deposit disclosure.
    surveys: BTreeMap<OreDepositId, SurveyDoc>,
    /// The applied-marker mirror the `industry.*` fact provider answers from.
    applied: BTreeMap<SmolStr, AppliedMarker>,
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

    /// One applied marker, if the mirror has it.
    #[must_use]
    pub fn marker(&self, id: &str) -> Option<&AppliedMarker> {
        self.applied.get(id)
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
    store.applied = applied;
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
