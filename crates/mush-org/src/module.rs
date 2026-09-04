//! The `OrgModule` (E1b) — the cut that makes the pure E1a library LIVE: attached
//! to the world, persisted per-record, surviving reboot, authorizing through a
//! world seam, and audited.
//!
//! ## Contracts (from the E1 design review)
//! - **The in-RAM [`OrgStore`] slot is the single source of truth.** Every actor
//!   mutation flows through [`crate::submit`], which validates on a clone (E1a's
//!   refuse-whole apply), installs the post-image, and persists exactly the
//!   touched docs + an audit entry + the allocator `meta` on one write batch.
//! - **Authority is a pure projection of live records** (F3): a destroyed member
//!   grants nothing because [`crate::OrgState::scrub_object`] deletes its records;
//!   the `ObjDestroyed` observer persists those deletions.
//! - **Boot fails closed with effect-based polarity.** Org and scope docs are
//!   *containers* — a corrupt one FAILS the boot (proceeding would orphan-purge an
//!   org's whole subtree). Membership/role/grant/delegation/policy docs are
//!   *leaves* — a corrupt one is skipped loudly; an orphan (its org gone, or a
//!   membership whose object was destroyed) is dropped and its doc corrected.
//! - **The predicate table is frozen at wiring** ([`OrgModule::with_predicates`]),
//!   the `MobilityVocabulary`/`UnitCatalog` pattern; E1b ships none.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::module::{
    GameModule, ModError, ModuleBootData, ModuleCtx, Registrar, WorldEvent, WorldEventKind,
};
use mush_core::{Ctx, Ledger};

use crate::api;
use crate::boot;
use crate::ids::{CapabilityId, DelegationId, GrantId, OrgId, OrgScopeId, PolicyId, RoleId};
use crate::record::AccessPolicy;
use crate::state::OrgState;
use crate::wiring::{
    CapabilityVocabulary, OrgPredicateProvider, PredicateProviders, ScopeUseProvider,
    ScopeUseProviders,
};

/// The module id and persistence namespace.
pub const NS: &str = api::NS;

/// The in-RAM org slot — the single source of truth. Holds the authoritative
/// [`OrgState`], the append-only audit [`Ledger`] (open tail in RAM), the
/// monotonic id allocators (resumed from the `meta` doc at boot), and the frozen
/// predicate table injected at wiring.
#[derive(Clone, Debug, Default)]
pub struct OrgStore {
    state: OrgState,
    /// The org-action audit trail (append-only; open tail in RAM).
    pub audit: Ledger,
    next_org: u64,
    next_scope: u64,
    next_role: u64,
    next_grant: u64,
    next_delegation: u64,
    next_policy: u64,
    providers: PredicateProviders,
    /// The wiring-frozen capability vocabulary (E3b2 §4.8.4); empty accepts all.
    capabilities: CapabilityVocabulary,
    /// The wiring-frozen scope-use binders (E3b2 §4.8.2).
    scope_use: ScopeUseProviders,
}

impl OrgStore {
    /// Assemble a store from boot-loaded parts (used by [`crate::boot`]). Providers
    /// are empty here — the module injects the frozen table after boot.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_parts(
        state: OrgState,
        audit: Ledger,
        next_org: u64,
        next_scope: u64,
        next_role: u64,
        next_grant: u64,
        next_delegation: u64,
        next_policy: u64,
    ) -> OrgStore {
        OrgStore {
            state,
            audit,
            next_org,
            next_scope,
            next_role,
            next_grant,
            next_delegation,
            next_policy,
            providers: PredicateProviders::default(),
            capabilities: CapabilityVocabulary::default(),
            scope_use: ScopeUseProviders::default(),
        }
    }

    /// The authoritative state (read seam for [`crate::authorize_in_world`]).
    #[must_use]
    pub fn state(&self) -> &OrgState {
        &self.state
    }
    /// The frozen predicate table (an `Arc` clone — cheap).
    #[must_use]
    pub(crate) fn providers(&self) -> PredicateProviders {
        self.providers.clone()
    }
    /// The frozen capability vocabulary. **Empty means "accept everything"** (the
    /// E1 behavior for a server that wires none).
    #[must_use]
    pub fn capabilities(&self) -> &BTreeSet<CapabilityId> {
        &self.capabilities
    }
    /// Whether `capability` is inside the configured vocabulary (always `true` when
    /// no vocabulary is configured).
    #[must_use]
    pub fn knows_capability(&self, capability: &CapabilityId) -> bool {
        self.capabilities.is_empty() || self.capabilities.contains(capability)
    }
    /// The frozen scope-use binder table (an `Arc` clone — cheap).
    #[must_use]
    pub(crate) fn scope_use(&self) -> ScopeUseProviders {
        self.scope_use.clone()
    }
    /// Install a new post-image state ([`crate::submit`] only).
    pub(crate) fn install_state(&mut self, state: OrgState) {
        self.state = state;
    }

    // ---- id allocators (the public id-minting seam; resumed from `meta`) ----
    //
    // A caller in another crate (E1c faction, E2 economy) mints the ids it needs,
    // builds an [`OrgMutationPlan`](crate::OrgMutationPlan) with them, and applies
    // it through [`crate::submit`] (which persists the refreshed `meta`). Minting
    // without a matching record just advances the never-reused counter — a harmless
    // id gap, never a collision.

    /// Mint the next organization id.
    pub fn alloc_org(&mut self) -> OrgId {
        let id = OrgId(self.next_org);
        self.next_org += 1;
        id
    }
    /// Mint the next scope id.
    pub fn alloc_scope(&mut self) -> OrgScopeId {
        let id = OrgScopeId(self.next_scope);
        self.next_scope += 1;
        id
    }
    /// Mint the next role-template id.
    pub fn alloc_role(&mut self) -> RoleId {
        let id = RoleId(self.next_role);
        self.next_role += 1;
        id
    }
    /// Mint the next capability-grant id.
    pub fn alloc_grant(&mut self) -> GrantId {
        let id = GrantId(self.next_grant);
        self.next_grant += 1;
        id
    }
    /// Mint the next delegation id.
    pub fn alloc_delegation(&mut self) -> DelegationId {
        let id = DelegationId(self.next_delegation);
        self.next_delegation += 1;
        id
    }
    /// Mint the next access-policy id.
    pub fn alloc_policy(&mut self) -> PolicyId {
        let id = PolicyId(self.next_policy);
        self.next_policy += 1;
        id
    }

    /// The allocator resume document mirrored on every mutation.
    pub(crate) fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_org_id": self.next_org,
            "next_scope_id": self.next_scope,
            "next_role_id": self.next_role,
            "next_grant_id": self.next_grant,
            "next_delegation_id": self.next_delegation,
            "next_policy_id": self.next_policy,
        })
    }
}

/// The organization + scoped-authorization game module. Construct with a frozen
/// predicate table (empty by default). Hand one to `Engine::with_modules`.
#[derive(Debug, Default)]
pub struct OrgModule {
    providers: PredicateProviders,
    capabilities: CapabilityVocabulary,
    scope_use: ScopeUseProviders,
}

impl OrgModule {
    /// A module with no predicate providers.
    #[must_use]
    pub fn new() -> OrgModule {
        OrgModule::default()
    }
    /// A module with a frozen predicate table (a ruleset / server wiring supplies
    /// it; e.g. E1c's faction-standing provider).
    #[must_use]
    pub fn with_predicates(providers: Vec<Arc<dyn OrgPredicateProvider>>) -> OrgModule {
        OrgModule {
            providers: Arc::new(providers),
            ..OrgModule::default()
        }
    }
    /// Freeze the capability vocabulary this server recognizes (E3b2 §4.8.4). An
    /// **empty** set (the default) accepts every capability — the E1 behavior.
    /// Chained after [`OrgModule::with_predicates`] at wiring; the server passes
    /// `mush_economy::CAPABILITIES ∪ {org.role.manage, org.grant.delegate}`.
    #[must_use]
    pub fn with_capabilities(mut self, capabilities: BTreeSet<CapabilityId>) -> OrgModule {
        self.capabilities = Arc::new(capabilities);
        self
    }
    /// Freeze the table of modules that bind records to scopes (E3b2 §4.8.2), so
    /// `@org/scope retire` can refuse a compartment something still points at.
    #[must_use]
    pub fn with_scope_use(mut self, binders: Vec<Arc<dyn ScopeUseProvider>>) -> OrgModule {
        self.scope_use = Arc::new(binders);
        self
    }
}

impl GameModule for OrgModule {
    fn name(&self) -> &'static str {
        NS
    }

    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.state(OrgStore::default());
        reg.subscribe(WorldEventKind::ObjDestroyed, Arc::new(on_obj_destroyed));
        crate::commands::register(reg)?;
        Ok(())
    }

    fn on_world_loaded(
        &mut self,
        ctx: &mut ModuleCtx<'_>,
        data: ModuleBootData,
    ) -> Result<(), ModError> {
        let mut store = boot::load_boot(ctx, data)?;
        // Inject the wiring-frozen tables into the freshly booted slot.
        store.providers = self.providers.clone();
        store.capabilities = self.capabilities.clone();
        store.scope_use = self.scope_use.clone();
        *ctx.w
            .module_state_mut::<OrgStore>()
            .expect("org store slot installed at registration") = store;
        Ok(())
    }
}

/// `ObjDestroyed` observer: scrub every org binding of the destroyed object (the
/// primary authority guard) and persist exactly the deletions + the policies whose
/// rule lists shrank. No audit entry — this is a world event, not an actor action.
fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;
    let (report, modified) = {
        let Some(store) = ctx.w.module_state_mut::<OrgStore>() else {
            return;
        };
        let report = store.state.scrub_object(obj);
        let modified: Vec<AccessPolicy> = report
            .modified_policies
            .iter()
            .filter_map(|id| store.state.policy(*id).cloned())
            .collect();
        (report, modified)
    };
    if report.is_empty() {
        return;
    }
    for org in &report.memberships {
        api::persist_del(ctx.out, api::member_key(*org, obj));
    }
    for g in &report.grants {
        api::persist_del(ctx.out, api::grant_key(*g));
    }
    for d in &report.delegations {
        api::persist_del(ctx.out, api::delegation_key(*d));
    }
    for p in &report.deleted_policies {
        api::persist_del(ctx.out, api::policy_key(*p));
    }
    for p in &modified {
        api::persist_policy(ctx.out, p);
    }
}
