//! The external-fact seam (E4c, 08c §6.1) — how a contract's
//! [`TypedCondition::Fact`](crate::TypedCondition::Fact) reaches the module that
//! actually knows the answer.
//!
//! E4a shipped [`FactEval`], the pure engine's one-question
//! interface (`Option<bool>`, where `None` = unknown = unsatisfied). This is the
//! **world-side** half: a wiring-frozen table of [`FactProvider`]s, each owning one
//! `provider` namespace, and the [`WorldFacts`] bridge that turns an
//! [`ExternalRef`] into a provider call. The shape is E1c's provider table exactly
//! (`OrgModule::with_predicates`, `EconomyModule::with_custody`): frozen at wiring,
//! never mutated at runtime, and consulted live at every validation.
//!
//! ## Shaped for doc 10 §15, not for jobs
//!
//! `mush-jobs` is the first implementor, but the seam is deliberately not
//! job-shaped. Doc 10 §15 (T2 boundary facts) needs the same seam to carry
//! `mush-sim`/`mush-tactics` facts — sortie, journey, unit state, delivery,
//! docking, salvage — so:
//!
//! - **`kind` is a namespaced string**, not an enum. `jobs.resolved` today;
//!   `sim.journey.arrived`, `sim.station.served`, `tactics.unit.destroyed`,
//!   `tactics.salvage.claimed` when those crates register. Widening an enum in
//!   `mush-economy` every time a domain learns a new fact would put the economy in
//!   the path of every simulation change; a string keyed by the provider that owns
//!   it does not.
//! - **`id` is the provider's own stable id**, opaque here. Doc 10 §15's
//!   "a stable id space per domain journal" is the provider's contract, not this
//!   crate's: the economy's causal reference points *at* the mechanical journal and
//!   never duplicates it.
//! - **`revision` is passed through.** [`ExternalRef`] has carried one since E2, and
//!   a provider may use it to refuse a stale reference (a journey record amended
//!   after the contract named it) by answering `None` rather than a fact about a
//!   different version of the world.
//!
//! ## Unknown is unsatisfied, everywhere
//!
//! An unregistered provider, an unknown `kind`, a missing record and a refused
//! revision all answer `None`, and `None` never settles money. That is the whole
//! fail-closed argument: a `Fact`-gated contract in a world where the owning module
//! is not loaded waits forever rather than paying out on a question nobody asked.

pub mod scoped;

use std::sync::Arc;

use mush_core::{ObjId, World};

use crate::engine::FactEval;
use crate::record::ExternalRef;

/// What a provider knows about one external fact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FactState {
    /// Whether the fact holds **now**. Re-resolved at settlement like every other
    /// condition, so a fact that stops holding stops settling.
    pub satisfied: bool,
    /// Who brought it about, when the provider can say (the approving staffer, the
    /// arriving pilot). Recorded on the attestation for audit; never authority.
    pub actor: Option<ObjId>,
    /// When it became true (unix seconds), or `0` when the provider does not know.
    pub at: i64,
}

/// A module that can answer external facts in one namespace.
///
/// Implementors live **with the data** (E1c's direction): `mush-jobs` owns
/// `jobs.*`, `mush-sim` will own `sim.*`, `mush-tactics` `tactics.*`. The economy
/// never depends on any of them — the composition root hands the table in through
/// [`crate::EconomyModule::with_facts`].
pub trait FactProvider: Send + Sync + std::fmt::Debug {
    /// The `ExternalRef::provider` namespace this table entry answers for
    /// (`"jobs"`, `"sim"`, `"tactics"`). The first entry claiming a provider wins,
    /// exactly like the custody table's routing.
    fn provider(&self) -> &str;

    /// Resolve one fact, or `None` for *unknown* — an unrecognized `kind`, a record
    /// that does not exist, or a `revision` the provider considers stale.
    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState>;

    /// Opt in per reference. The unscoped bridge refuses such references even if
    /// an implementation accidentally answers them in `resolve`.
    fn requires_scope(&self, _fact: &ExternalRef) -> bool {
        false
    }

    /// Authorize attachment, disclosure or use of this exact reference against
    /// actual dispatch and contract scope. Opt-in defaults to refusal. Event
    /// attribution and arbitrary party membership are never subject authority.
    fn authorize_scope(&self, _ctx: &scoped::ScopedFactContext<'_>, _fact: &ExternalRef) -> bool {
        false
    }

    /// Called only after scope authorization. This grants no mutation authority.
    fn resolve_scoped(
        &self,
        ctx: &scoped::ScopedFactContext<'_>,
        fact: &ExternalRef,
    ) -> Option<FactState> {
        self.resolve(
            ctx.world(),
            fact.kind.as_str(),
            fact.id.as_str(),
            fact.revision,
        )
    }

    /// Request immutable owner services under the actual Economy registrar.
    /// Other consumers must obtain their own registered handles; a public API
    /// call does not change its dispatch's owner identity.
    fn register_scoped(
        &self,
        _reg: &mut mush_core::module::Registrar<'_>,
    ) -> Result<(), mush_core::module::ModError> {
        Ok(())
    }
}

/// The wiring-frozen provider table (an `Arc` so the store can hand out cheap
/// clones while the world is borrowed).
pub type FactTable = Arc<Vec<Arc<dyn FactProvider>>>;

/// The live bridge from the pure [`FactEval`] question to the frozen table.
///
/// Built per validation, exactly like `WorldCustody`, so a provider registered at
/// wiring is consulted against the world as it is *now* rather than as it was when
/// the contract was drafted.
pub struct WorldFacts<'a> {
    /// The world every provider reads.
    pub w: &'a World,
    /// The frozen table.
    pub providers: FactTable,
}

impl WorldFacts<'_> {
    /// Resolve a reference to its provider's full answer (the attestation-shaped
    /// form the canon/settlement paths want), or `None` for unknown.
    #[must_use]
    pub fn state(&self, fact: &ExternalRef) -> Option<FactState> {
        let provider = self
            .providers
            .iter()
            .find(|p| p.provider() == fact.provider.as_str())?;
        if provider.requires_scope(fact) {
            return None;
        }
        provider.resolve(self.w, fact.kind.as_str(), fact.id.as_str(), fact.revision)
    }
}

/// Whether the wiring registered a fact provider for `namespace` (doc 18 §4.3, cut
/// IC7): `false` with no economy. A module that gates money on its own fact checks
/// this before it takes the money — the market refuses a drawn sale before any
/// escrow when its `market` namespace is not composed, because a `Fact` nobody
/// answers would leave the contract waiting and refunding instead of selling.
#[must_use]
pub fn has_fact_provider(w: &World, namespace: &str) -> bool {
    w.module_state::<crate::module::EconomyStore>()
        .is_some_and(|s| s.facts().iter().any(|p| p.provider() == namespace))
}

impl FactEval for WorldFacts<'_> {
    fn resolve(&self, fact: &ExternalRef) -> Option<bool> {
        self.state(fact).map(|s| s.satisfied)
    }
}
