//! Wiring-time extension points for the E1b module — the seam that lets an org
//! [`AccessPolicy`](crate::AccessPolicy) name a cross-module predicate (faction
//! standing, reputation, …) without `mush-org` depending on the provider crate.
//!
//! A [`PolicySubject::RegisteredPredicate`](crate::PolicySubject) carries a
//! `provider`/`predicate`/`arguments` triple. At authorization time the E1b world
//! seam ([`crate::authorize_in_world`]) builds a [`WorldPredicates`] over the
//! frozen provider table and hands it to the pure engine as a
//! [`PredicateEval`]. Each provider gets `&World` so it can
//! read its own module state; it returns only a boolean (never the underlying
//! value — the disclosure rule), and `None` when it cannot resolve, which the
//! engine treats fail-closed (suppresses an Allow, satisfies a Deny).
//!
//! The provider table is **frozen at wiring** ([`crate::OrgModule::with_predicates`]):
//! the set of providers is fixed when the server is assembled, exactly the
//! `MobilityVocabulary`/`UnitCatalog` freeze pattern. E1c supplies the first real
//! provider (faction standing); E1b ships none by default.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::{ObjId, World};

use crate::authorize::PredicateEval;
use crate::ids::{CapabilityId, OrgScopeId};

/// A world-aware policy-predicate provider, frozen into the module at wiring.
///
/// Implementors read live world state (their own module slot) to answer a policy
/// predicate. They must return only a boolean verdict — never the hidden value
/// the verdict derives from — and `None` when the predicate is unknown or
/// unresolvable (the engine then fails closed).
pub trait OrgPredicateProvider: Send + Sync + std::fmt::Debug {
    /// The `provider` token this resolves (e.g. `"faction"`). The bridge routes a
    /// predicate to the first provider whose token matches.
    fn provider(&self) -> &str;

    /// Resolve `predicate(arguments)` for `actor` against live world state.
    /// `Some(true)`/`Some(false)` when resolved; `None` when unknown/unresolvable.
    fn eval(
        &self,
        w: &World,
        predicate: &str,
        arguments: &serde_json::Value,
        actor: ObjId,
    ) -> Option<bool>;
}

/// The frozen provider table (an `Arc` so the module, the slot, and each command
/// closure share one immutable copy).
pub type PredicateProviders = Arc<Vec<Arc<dyn OrgPredicateProvider>>>;

/// Bridges the frozen [`OrgPredicateProvider`] table + a live `&World` to the pure
/// engine's [`PredicateEval`]. Constructed per authorization call; borrows the
/// world immutably alongside the `OrgState` read (both shared borrows coexist).
pub(crate) struct WorldPredicates<'a> {
    pub(crate) w: &'a World,
    pub(crate) providers: PredicateProviders,
}

impl PredicateEval for WorldPredicates<'_> {
    fn eval(
        &self,
        provider: &str,
        predicate: &str,
        arguments: &serde_json::Value,
        actor: ObjId,
    ) -> Option<bool> {
        // First provider whose token matches; an unmatched provider is unresolvable
        // (`None`) — the engine fails closed, never open.
        self.providers
            .iter()
            .find(|p| p.provider() == provider)?
            .eval(self.w, predicate, arguments, actor)
    }
}

// ---- the capability vocabulary (E3b2 §4.8.4) ------------------------------

/// The set of capability ids this server recognizes, frozen at wiring
/// ([`crate::OrgModule::with_capabilities`]).
///
/// **An EMPTY vocabulary accepts everything** — the E1 behavior, so a server that
/// wires none behaves exactly as before. When one is configured, `@org/grant`
/// refuses an unknown capability and `@org/show` flags a stored grant whose
/// capability has fallen outside it (a vocabulary can shrink between builds; an
/// existing grant is never silently deleted, only marked).
///
/// The vocabulary is a **typo catcher, not an authorization gate**: `authorize`
/// compares capability ids by equality, so an unknown capability already
/// authorizes nothing. What it prevents is a staff member granting
/// `economy.acount.post` and believing they granted anything at all.
pub type CapabilityVocabulary = Arc<BTreeSet<CapabilityId>>;

// ---- the scope-use seam (E3b2 §4.8.2) ------------------------------------

/// Answers *does anything still bind this scope?* for `@org/scope retire`.
///
/// `mush-org` sits **below** the economy in the dependency graph and cannot see an
/// account, asset, or vault — but retiring a compartment those records still point
/// at would strand them (an account whose scope confers no authority can no longer
/// be spent from by anyone but staff). So the binder answers, exactly as the
/// predicate table lets a policy ask faction about standing: the table is frozen at
/// wiring ([`crate::OrgModule::with_scope_use`]), and `mush-economy` implements it.
///
/// A binder that cannot answer returns `None` (nothing known to bind); the command
/// layer treats a present reason as a refusal.
pub trait ScopeUseProvider: Send + Sync + std::fmt::Debug {
    /// The binder's name (e.g. `"economy"`), for the refusal message.
    fn binder(&self) -> &str;
    /// A human-readable reason the scope is still in use, or `None`.
    fn scope_in_use(&self, w: &World, scope: OrgScopeId) -> Option<String>;
}

/// The frozen scope-use table (an `Arc`, shared like the predicate table).
pub type ScopeUseProviders = Arc<Vec<Arc<dyn ScopeUseProvider>>>;
