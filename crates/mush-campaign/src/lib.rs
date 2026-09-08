//! # mush-campaign (E7 cut 9)
//!
//! **Declared operations, formations, the funded bond and idempotent terminal
//! resolution** (10b §3.9; doc 10 §13 T6, §19 T8, §12 item 7A).
//!
//! ## The two binding constraints
//!
//! **A campaign is FUNDED, never infinite, and this crate owns no mint path.** The
//! bond is an E4 money `Hold` on the attacker's own account, placed from the
//! **command** through `mush_economy::post` (a `prepare_authoritative` provider has
//! no world and cannot place one — cut 4's As-built deviation (a)). The
//! never-negative gate is the engine's own `EconomyError::InsufficientAvailable`;
//! this crate writes no `Amount` arithmetic, no `create_system_obligation`, no
//! credit line and no faucet. Funds exhausted ⇒ the operation **pauses at its next
//! phase boundary and pages** (level 2, command filled in, subject-only stop).
//!
//! **Every reaction to durable state is a `Coarse` pull-tick.** A
//! `schedule_source`-fired action has no `&World`, `Ctx` or `Outbox` and cannot emit
//! a `WorldEvent`, and there is no world-side seam on a scheduled authoritative
//! commit (the kernel fact cut 6 established). So [`tick`] is a
//! `reg.tick(.., TickCadence::Coarse(..), ..)` on `crates/mush-housing/src/tick.rs`'s
//! exact shape — **never** an observer on a family change, **never** a
//! `schedule_source`, **never** a `WorldEvent` subscription.
//!
//! ## What it registers, and what it deliberately does not
//!
//! One `reg.state`, one `reg.tick`, three commands in three classified groups, and
//! **exactly one `FactProvider` namespace** — `campaign.*`, D-E7-30's fifth and last
//! crate. It registers **no** `schedule_source`, no `clock_provider` or
//! `domain_clock_provider`, no `state_family`, no `AuthorityGuardKind` validator, no
//! `revision_validator`, no `WorldEvent` variant and no second fact provider, and it
//! performs **no `Ctx`-level authoritative submit**: it writes exempt workflow
//! documents and posts to the economy from the command.
//!
//! It builds no second clock or pulse (cut 8's), no placement or journey mechanism
//! (cut 6's), no station control (cut 5's), no contact family (cut 7's) and no
//! minting path (cut 11's). Its edge set is `{mush-core, mush-economy, mush-canon,
//! mush-org}` and there is deliberately **no `mush-faction` edge**: the front
//! consequence is discharged by the ONE root canon record, which `mush-faction`'s
//! own `on_canon_recorded` observer already turns into a `Clock::advance` and an
//! `on_complete_control` flip.

pub mod bond;
pub mod commands;
pub mod facts;
pub mod formation;
pub mod ids;
pub mod record;
pub mod recovery;
pub mod recoverycmd;
pub mod render;
pub mod seat;
pub mod seatcmd;
pub mod seatstate;
pub mod settle;
pub mod store;
pub mod terms;
pub mod tick;

#[cfg(test)]
mod testfix;

use std::sync::Arc;

use mush_core::module::{GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, TickCadence};

pub use commands::{CAPABILITIES, OUTCOME_VOCABULARY};
pub use facts::CampaignFactProvider;
pub use formation::{
    CategoryId, Composition, FormationAuthority, FormationError, FormationKind, FormationNode,
    MAX_ALLOWED_CATEGORIES, MAX_COMMAND_ROLES, MAX_FORMATION_DEPTH, MAX_FORMATION_MEMBERS,
    MAX_FORMATIONS_PER_OPERATION, Qualification, RoleSpec, Strictness,
};
pub use ids::{FormationId, OperationId, formation_key, operation_key};
pub use record::{
    AudienceSpec, Bond, Consequence, ConsumerId, MAX_CONSUMER_ATTEMPTS, MAX_LABEL_BYTES,
    MAX_LEGAL_TARGETS, MAX_OBJECTIVE_BYTES, MAX_OPERATION_PARTICIPANTS, MAX_OUTCOME_NAME_BYTES,
    MAX_SIDES, MAX_STAKES, OPERATION_NOTICE_MAX, OPERATION_NOTICE_MIN, ObjectiveKind,
    ObjectiveSpec, OperationDoc, OperationRuntime, OperationState, OperationTerms, Participation,
    PauseReason, RootFact, RuntimeError, SideSpec, TaskForce, TermsError, UnitRef,
};
pub use recovery::{
    ApplicationState, MAX_RECOVERY_APPLICATIONS_PER_PRINCIPAL, MAX_RECOVERY_LOSS_VISIT,
    MAX_RECOVERY_PER_FIRE, MAX_RECOVERY_QUEUE, MAX_RETAINED_RECOVERY_RECORDS,
    RECOVERY_ACCEPT_TTL_SECS, RECOVERY_COOLDOWN_SECS, RECOVERY_QUEUE_KEY, RecoveryApplication,
    RecoveryConfig, RecoveryGrant, RecoveryPolicy, RecoveryQueue, RecoveryRecordError,
    RecoveryRefusal, accessible_operational_assets, cooldown_until, eligible,
    is_accessible_operational, live_lease_on, live_recovery_lease, loss_is_covered, next_loaner,
    recovery_application_key, recovery_grant_key,
};
pub use recoverycmd::{GrantOutcome, NO_SUCH_APPLICANT, NOT_ENABLED, grant_key};
pub use render::{
    Lens, NO_SUCH_FORMATION, NO_SUCH_OPERATION, Verbosity, lens_for, visible_operation,
};
pub use seat::{
    CrewComplement, DriverToken, EligibilitySpec, FallbackSpec, GrantState, MAX_CREW_NAMED,
    MAX_MUSTER_SLOTS, MAX_RETAINED_MUSTERS, MAX_SEAT_CHANNELS, MAX_SEAT_GRANTS_PER_OPERATION,
    MAX_SEAT_LABEL_BYTES, MAX_SEAT_OFFERS_PER_OPERATION, MusterReport, MusterSlot, RevocationTerms,
    SEAT_OFFER_TTL_SECS, SeatGrant, SeatOffer, SeatOfferId, SeatRecordError, SeatScope, SlotFill,
    crew_key, muster_key, seat_consent_hash, seat_grant_key, seat_offer_key,
};
pub use seatcmd::{NO_SUCH_CREW, NO_SUCH_SEAT};
pub use seatstate::SeatAuditHolder;
pub use settle::{
    KIND_OPERATION_ABANDONED, KIND_OPERATION_RESOLVED, abandon, advance_one, canon_source,
    enter_resolving, outcome_abandons, outcome_forfeits, scope_for,
};
pub use store::{
    CampaignStore, HeldDoc, MAX_FORMATION_NODES, MAX_HELD_DOCUMENTS, MAX_OPERATIONS,
    MAX_OPERATIONS_PER_ISSUER, META_KEY, NS, TickStats,
};
pub use terms::operation_consent_hash;
pub use tick::{
    MAX_SEATS_PER_FIRE, MAX_VISITED_PER_FIRE, MAX_WORKED_PER_FIRE, TICK_PERIOD_SECS, TICK_TAG,
};

/// The campaign module. Hand one to the composition root.
#[derive(Default)]
pub struct CampaignModule {
    formation_kinds: Option<Arc<dyn FormationAuthority>>,
    /// E7c10 B3 (10b §3.10 OPEN 4): the server-side, audit-only seat
    /// attribution log. `None` leaves it unwired and every consumer fails
    /// closed — a seat cannot be taken un-attributed, and a grant with no row
    /// refuses activation and pages.
    seat_audit: Option<Arc<dyn mush_core::SeatAudit>>,
    /// **E7r1 R4** — the composed `[tacsim.recovery]` block. `None` is the disabled
    /// default: `@recovery` still registers and answers "not enabled on this server"
    /// (E5c deviation 1), which is also what keeps the rule-1 allowlist tripwire able
    /// to see the verb in a default composition.
    recovery: Option<recovery::RecoveryConfig>,
}

impl CampaignModule {
    /// A fresh module with no formation authority wired — which makes
    /// `@force/create` refuse every kind, the fail-closed default.
    #[must_use]
    pub fn new() -> CampaignModule {
        CampaignModule::default()
    }

    /// **The composition seam formation kinds arrive through** (OPEN 4), on
    /// `IndustryModule::with_recipes`' shape: the implementation lives at the
    /// composition root and reads the ruleset's own store, so `mush-campaign`
    /// gains no ruleset edge and `ruleset-tacsim` gains no campaign edge.
    /// **The composition seam the seat audit arrives through** (E7c10 B3), on
    /// `with_formation_kinds`' / `SimulationModule::with_accounts`' shape: the
    /// implementation lives at the composition root beside `AccountIdentity`'s,
    /// the row never becomes a world document, and unwired is fail-closed.
    #[must_use]
    pub fn with_seat_audit(mut self, audit: Arc<dyn mush_core::SeatAudit>) -> CampaignModule {
        self.seat_audit = Some(audit);
        self
    }

    /// Hand in the composed `[tacsim.recovery]` policy (**E7r1 R4**).
    ///
    /// The composition root maps `ruleset_tacsim::TacsimRecoveryConfig` into this
    /// crate's own type, exactly as `IndustryModule::with_recipes` and
    /// `with_formation_kinds` do — `mush-campaign` takes no ruleset edge, and
    /// `cargo tree -p mush-campaign` is the test.
    #[must_use]
    pub fn with_recovery(mut self, cfg: recovery::RecoveryConfig) -> CampaignModule {
        self.recovery = Some(cfg);
        self
    }

    #[must_use]
    pub fn with_formation_kinds(
        mut self,
        authority: Arc<dyn FormationAuthority>,
    ) -> CampaignModule {
        self.formation_kinds = Some(authority);
        self
    }
}

impl GameModule for CampaignModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        // K8.5: classify BEFORE the first `reg.command`. `commands::register`
        // opens with its own `classify_commands` per group and restores
        // `LegacyPostImage` last; this one covers the module's own first call and
        // makes the requirement local rather than remembered.
        reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
        let mut store = CampaignStore::default();
        store.set_authority(self.formation_kinds.clone());
        reg.state(store);
        // ONE Coarse pull-tick, on housing's shape. No `schedule_source`, no
        // observer, no `WorldEvent` subscription — see the crate header.
        reg.tick(
            TICK_TAG,
            TickCadence::Coarse(std::time::Duration::from_secs(TICK_PERIOD_SECS)),
            Arc::new(tick::tick),
        )?;
        // E7c10 B3: the audit holder, carried in world state so the seat
        // commands reach it synchronously. Unwired = fail closed.
        // E7r1 R4/R3: the composed `[tacsim.recovery]` block, carried in world state
        // so `@recovery` and the tick arm reach it synchronously. Unwired is the
        // disabled default — **fail closed**: an unwired provider grants nothing and
        // says so out loud.
        reg.state(recovery::RecoveryPolicy::new(
            self.recovery.clone().unwrap_or_default(),
        ));
        reg.state(match self.seat_audit.clone() {
            Some(audit) => seatstate::SeatAuditHolder::new(audit),
            None => seatstate::SeatAuditHolder::unwired(),
        });
        // E7r1 R3: `@recovery`, in its own classified group (K8.5), and the module
        // default restored after it.
        recoverycmd::register(reg)?;
        commands::register(reg)?;
        // E7c10 B8: cut 10's five verbs, in two classified groups, with
        // `LegacyPostImage` restored last (K8.5).
        seatcmd::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut store = store::load(data.kv);
        store.set_authority(self.formation_kinds.clone());
        store.hydrate_kinds(ctx.w);
        let held: Vec<store::HeldDoc> = store.held().to_vec();
        let held_overflow = store.held_overflow();
        let unknown: Vec<(OperationId, Vec<String>)> = store
            .operations()
            .filter(|doc| !doc.runtime.unknown_consumers.is_empty())
            .map(|doc| {
                (
                    doc.id(),
                    doc.runtime
                        .unknown_consumers
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                )
            })
            .collect();
        {
            let slot = ctx
                .w
                .module_state_mut::<CampaignStore>()
                .ok_or_else(|| ModError::msg("campaign: state slot missing"))?;
            *slot = store;
        }
        // The world still boots. A document that could not be decoded is HELD, not
        // deleted — it may be the only record of a live money hold's purpose — and
        // its subject is marked with a level-2 page carrying the command.
        for doc in held {
            tracing::error!(
                target: "campaign",
                subject = %doc.key,
                runbook = %format!(
                    "the campaign document {} could not be decoded and is held, not deleted; \
                     inspect it in the store and repair or remove it, then reboot",
                    doc.key
                ),
                error = %doc.error,
                "campaign: a document was refused on load and is held (level 2)"
            );
        }
        // B14: past `MAX_HELD_DOCUMENTS` the list stops growing, and cut 9 dropped
        // the 65th refused document with no counter and no log — so staff were told
        // about 64 of them and silently not about the rest, which is exactly the
        // failure mode that can make a bond-bearing document invisible. The overflow
        // is counted and paged once.
        if held_overflow > 0 {
            tracing::error!(
                target: "campaign",
                subject = "campaign:held-overflow",
                runbook = %format!(
                    "{held_overflow} further campaign document(s) were refused on load beyond \
                     the {} this build lists; none was deleted and every allocator is still \
                     floored past them. Repair the listed ones and reboot to see the next batch",
                    store::MAX_HELD_DOCUMENTS
                ),
                "campaign: more documents were refused than the held list carries (level 2)"
            );
        }
        // A stored consumer id this build does not know fails CLOSED: the
        // operation keeps it, can never reach `Resolved`, and pages once.
        for (id, ids) in unknown {
            tracing::error!(
                target: "campaign",
                subject = %format!("op:{}", id.0),
                runbook = %format!(
                    "operation {id} stores consumer id(s) {} that this build does not know; run \
                     the build that knows them, or abandon it with `@op/resolve {id}=abort`",
                    ids.join(", ")
                ),
                "campaign: an operation stores an unknown consumer id (level 2)"
            );
        }
        Ok(())
    }
}
