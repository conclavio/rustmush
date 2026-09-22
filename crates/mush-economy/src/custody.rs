//! The custody seam (E3b1, doc 08b §4.3) — how a [`Vault`] reaches the world.
//!
//! `mush-economy` is world-free at its core: the pure engine asks a
//! [`CustodyEval`] the four world questions a custody op needs (is the backing
//! usable, does the vault hold this, does the actor have standing, is the
//! destination legal). This module supplies the live answers, and adds the
//! *mutation* half the engine never touches: a [`CustodyProvider`] table that
//! actually moves objects.
//!
//! ## Rules that keep custody from half-applying
//!
//! - **Exactly one provider resolves a whole op** — the first whose
//!   [`CustodyProvider::handles`] accepts the backing. The op is refused if any
//!   *other* registered provider [`claims`](CustodyProvider::claims) the object
//!   (a sim-placed hull is not a thing you drop into a footlocker).
//! - **`None` from any query is fail-closed.** A provider that cannot answer
//!   (its own state is not loaded yet, the backing is not its kind) never
//!   accidentally authorizes; boot treats `None` as "unknown", never as drift.
//! - **Validate-then-mutate, never half-apply.** [`CustodyProvider::move_into`]
//!   and [`CustodyProvider::move_out`] prove every precondition before the first
//!   world write, because [`crate::post`] runs them BEFORE the store advances
//!   (§4.2) and a handler `Err` discards deferred persists but not world writes.
//! - The built-in [`ContainerProvider`] is always appended **last**, so a wired
//!   provider always wins the routing race for its own backings.

use std::sync::Arc;

use mush_core::{Ctx, Flags, MAX_CONTAINMENT_DEPTH, ObjId, ObjKind, World, perm};
use mush_org::PrincipalId;

use crate::engine::CustodyEval;
use crate::module::EconomyStore;
use crate::record::{CustodyRef, Vault};

/// The wiring-frozen provider table (the `OrgModule::with_predicates` shape).
pub type CustodyTable = Arc<Vec<Arc<dyn CustodyProvider>>>;

/// Why a custody move was refused. Never a partial application: a provider that
/// returns this has mutated nothing.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CustodyError {
    /// No registered provider handles that backing (`Registered{..}` backings
    /// have none in E3; `SpatialInterior` waits for E3c).
    #[error("no custody provider handles that backing")]
    NoProvider,
    /// Another provider owns the object's placement — routing it through this
    /// one would strand it (a sim-placed or traveling object).
    #[error("object #{obj} is held by the {provider} system and cannot move that way")]
    Claimed {
        /// The object another provider claims.
        obj: ObjId,
        /// Which provider claims it.
        provider: String,
    },
    /// The provider validated and refused; nothing moved.
    #[error("{0}")]
    Refused(String),
}

/// A custody backend: the party that knows how a class of backing physically
/// holds things, and the only party that moves objects in or out of one.
///
/// Implementors live with the data they read (`mush-sim` owns interiors, E3c);
/// `mush-economy` ships only [`ContainerProvider`].
pub trait CustodyProvider: Send + Sync + std::fmt::Debug {
    /// A short stable name for messages/logs (`"container"`, `"sim"`).
    fn provider(&self) -> &str;
    /// Whether this provider resolves ops against `backing`.
    fn handles(&self, backing: &CustodyRef) -> bool;
    /// `true` if this provider owns the object's placement/motion, so no OTHER
    /// provider may move it. `ContainerProvider` claims nothing.
    fn claims(&self, w: &World, obj: ObjId) -> bool;
    /// Whether the backing is a live, usable custody container. `None` = cannot
    /// answer (fail closed at validate; "unknown", never drift, at boot).
    fn backing_valid(&self, w: &World, backing: &CustodyRef) -> Option<bool>;
    /// Everything the backing holds, transitively, depth-capped at
    /// [`MAX_CONTAINMENT_DEPTH`] — a container deposited whole is not a
    /// laundering primitive: its contents stay visible and individually
    /// withdrawable.
    fn contents(&self, w: &World, backing: &CustodyRef) -> Option<Vec<ObjId>>;
    /// Whether the backing (transitively) holds `obj`.
    fn holds(&self, w: &World, backing: &CustodyRef, obj: ObjId) -> Option<bool>;
    /// Move `obj` into the backing. **Validate first; never half-apply.**
    fn move_into(
        &self,
        ctx: &mut Ctx<'_>,
        actor: ObjId,
        obj: ObjId,
        backing: &CustodyRef,
    ) -> Result<(), CustodyError>;
    /// Move `obj` out of the backing to `to`. **Validate first; never half-apply.**
    fn move_out(
        &self,
        ctx: &mut Ctx<'_>,
        actor: ObjId,
        obj: ObjId,
        backing: &CustodyRef,
        to: ObjId,
    ) -> Result<(), CustodyError>;
}

/// Resolve the ONE provider for an op: the first that `handles(backing)`, refused
/// if any *other* provider `claims(obj)`. Pass `obj: None` for a pure query about
/// the backing itself (no object is being routed).
///
/// `@vault/create` calls this with the backing object itself, which is exactly
/// the "never a backing another system owns" guard: the provider that will run
/// the vault's ops is exempt (an interior vault's backing IS a space `mush-sim`
/// owns), while a sim-placed hull offered as a *container* backing is refused,
/// because the container provider is the one that would have to move things into
/// it.
pub(crate) fn resolve<'t>(
    providers: &'t [Arc<dyn CustodyProvider>],
    w: &World,
    backing: &CustodyRef,
    obj: Option<ObjId>,
) -> Result<&'t Arc<dyn CustodyProvider>, CustodyError> {
    let idx = providers
        .iter()
        .position(|p| p.handles(backing))
        .ok_or(CustodyError::NoProvider)?;
    if let Some(obj) = obj {
        for (i, p) in providers.iter().enumerate() {
            if i != idx && p.claims(w, obj) {
                return Err(CustodyError::Claimed {
                    obj,
                    provider: p.provider().to_string(),
                });
            }
        }
    }
    Ok(&providers[idx])
}

// ---- the built-in container provider --------------------------------------

/// The ordinary-container backend: any live world object can hold things.
///
/// Moves go through `World::can_move_to` **and** the `CarryAttempt` hook chain
/// (`World::move_to` itself consults no hook, so skipping the chain would make
/// `mush-sim`'s placement gate blind to a vault deposit) **and** a
/// [`MAX_CONTAINMENT_DEPTH`] check on the resulting chain (`move_to` does not
/// enforce it, and `containing_room` returns `None` past it) — all before the
/// single `move_to`.
#[derive(Debug, Default, Clone, Copy)]
pub struct ContainerProvider;

impl CustodyProvider for ContainerProvider {
    fn provider(&self) -> &str {
        "container"
    }
    fn handles(&self, backing: &CustodyRef) -> bool {
        matches!(backing, CustodyRef::Container(_))
    }
    fn claims(&self, _w: &World, _obj: ObjId) -> bool {
        false
    }
    fn backing_valid(&self, w: &World, backing: &CustodyRef) -> Option<bool> {
        let CustodyRef::Container(o) = backing else {
            return None;
        };
        Some(live(w, *o))
    }
    fn contents(&self, w: &World, backing: &CustodyRef) -> Option<Vec<ObjId>> {
        let CustodyRef::Container(o) = backing else {
            return None;
        };
        if !live(w, *o) {
            return Some(Vec::new());
        }
        Some(transitive_contents(w, *o))
    }
    fn holds(&self, w: &World, backing: &CustodyRef, obj: ObjId) -> Option<bool> {
        self.contents(w, backing).map(|v| v.contains(&obj))
    }
    fn move_into(
        &self,
        ctx: &mut Ctx<'_>,
        actor: ObjId,
        obj: ObjId,
        backing: &CustodyRef,
    ) -> Result<(), CustodyError> {
        let CustodyRef::Container(dest) = backing else {
            return Err(CustodyError::NoProvider);
        };
        container_move(ctx, actor, obj, *dest)
    }
    fn move_out(
        &self,
        ctx: &mut Ctx<'_>,
        actor: ObjId,
        obj: ObjId,
        backing: &CustodyRef,
        to: ObjId,
    ) -> Result<(), CustodyError> {
        if !self.handles(backing) {
            return Err(CustodyError::NoProvider);
        }
        container_move(ctx, actor, obj, to)
    }
}

/// The one mutating primitive: prove every precondition, then exactly one
/// `move_to`. Every early return leaves the world untouched.
fn container_move(
    ctx: &mut Ctx<'_>,
    actor: ObjId,
    obj: ObjId,
    dest: ObjId,
) -> Result<(), CustodyError> {
    if !live(ctx.w, obj) {
        return Err(CustodyError::Refused(format!(
            "#{} is not available to move.",
            obj.0
        )));
    }
    // Topology is not custody. `World::move_to` files an `Exit` into the
    // destination's `exits` vector, which NO custody query walks (`contents_of` is
    // contents only) — so a deposited exit would be journaled, then be invisible to
    // `holds`/`contents`, unwithdrawable, let `@vault/close` close an "empty" vault
    // that still held it, and boot as a custody mismatch. Refuse every kind
    // `move_to` would not file into `contents`.
    if !custody_movable(ctx.w, obj) {
        return Err(CustodyError::Refused(format!(
            "#{} is not a thing custody can hold.",
            obj.0
        )));
    }
    if !live(ctx.w, dest) {
        return Err(CustodyError::Refused(format!(
            "#{} cannot hold anything.",
            dest.0
        )));
    }
    ctx.w
        .can_move_to(obj, dest)
        .map_err(|e| CustodyError::Refused(e.to_string()))?;
    if resulting_depth(ctx.w, obj, dest) > MAX_CONTAINMENT_DEPTH {
        return Err(CustodyError::Refused(
            "That would nest containers too deeply.".to_string(),
        ));
    }
    // The only thing that makes a registered `CarryAttempt` gate (sim placement,
    // a ruleset's no-take rule) see a vault move at all.
    ctx.check_carry_attempt(actor, obj, dest)
        .map_err(|rt| CustodyError::Refused(rt.to_plain_text()))?;
    ctx.w
        .move_to(obj, dest)
        .map_err(|e| CustodyError::Refused(e.to_string()))
}

fn live(w: &World, o: ObjId) -> bool {
    w.get(o).is_some_and(|x| !x.flags.contains(Flags::GOING))
}

/// Whether custody can hold this object at all: `World::move_to` must file it into
/// the destination's **`contents`**, which is where every custody query looks. An
/// [`ObjKind::Exit`] is filed into `exits` instead — it is world topology, not a
/// thing — and a `Room` has no location to move. Both are refused before any move.
fn custody_movable(w: &World, o: ObjId) -> bool {
    w.get(o)
        .is_some_and(|x| !matches!(x.kind, ObjKind::Exit | ObjKind::Room))
}

/// Everything under `root`, transitively, capped at [`MAX_CONTAINMENT_DEPTH`]
/// levels (so a cyclic or absurdly deep world cannot make this unbounded).
fn transitive_contents(w: &World, root: ObjId) -> Vec<ObjId> {
    let mut out: Vec<ObjId> = Vec::new();
    let mut frontier: Vec<ObjId> = vec![root];
    for _ in 0..MAX_CONTAINMENT_DEPTH {
        let mut next: Vec<ObjId> = Vec::new();
        for id in frontier.drain(..) {
            for &c in w.contents_of(id) {
                if !out.contains(&c) {
                    out.push(c);
                    next.push(c);
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }
    out
}

/// The containment-chain length that moving `obj` into `dest` would produce:
/// the chain above `dest` (counting `dest`), the object itself, and the deepest
/// thing the object carries.
fn resulting_depth(w: &World, obj: ObjId, dest: ObjId) -> usize {
    depth_above(w, dest) + 1 + subtree_depth(w, obj, MAX_CONTAINMENT_DEPTH)
}

fn depth_above(w: &World, start: ObjId) -> usize {
    let mut n = 1usize;
    let mut cur = start;
    for _ in 0..=(MAX_CONTAINMENT_DEPTH * 2) {
        match w.get(cur).and_then(|o| o.location) {
            Some(loc) => {
                n += 1;
                cur = loc;
            }
            None => break,
        }
    }
    n
}

fn subtree_depth(w: &World, obj: ObjId, budget: usize) -> usize {
    if budget == 0 {
        return 0;
    }
    w.contents_of(obj)
        .iter()
        .map(|c| 1 + subtree_depth(w, *c, budget - 1))
        .max()
        .unwrap_or(0)
}

// ---- the CustodyEval bridge -----------------------------------------------

/// Bridges the provider table to the pure engine's [`CustodyEval`], exactly as
/// `WorldAuth` bridges `AuthEval`. Every unanswerable query is `false`.
pub(crate) struct WorldCustody<'a> {
    pub(crate) w: &'a World,
    pub(crate) providers: CustodyTable,
}

impl CustodyEval for WorldCustody<'_> {
    fn backing_valid(&self, vault: &Vault) -> bool {
        resolve(&self.providers, self.w, &vault.backing, None)
            .ok()
            .and_then(|p| p.backing_valid(self.w, &vault.backing))
            .unwrap_or(false)
    }
    fn holds(&self, vault: &Vault, obj: ObjId) -> bool {
        // Resolving WITH the object is also the claims check: an object another
        // provider owns is never "in" a container vault.
        let Ok(p) = resolve(&self.providers, self.w, &vault.backing, Some(obj)) else {
            return false;
        };
        p.holds(self.w, &vault.backing, obj).unwrap_or(false)
    }
    /// Invariant 15's standing rule: **carried exactly** (`Obj.location ==
    /// Some(actor)`, never an ancestor walk), **or** co-located with the actor
    /// and the actor is the title owner / assignee / MUSH controller / staff.
    fn may_deposit(&self, obj: ObjId, actor: &PrincipalId) -> bool {
        let PrincipalId::Object(a) = actor else {
            return false;
        };
        let a = *a;
        if !live(self.w, obj) || !live(self.w, a) {
            return false;
        }
        // A person is not vault contents. `obj == actor` would otherwise sail
        // through every clause below — co-located with itself, and
        // `perm::controls(a, a)` is true — so an actor could file THEMSELF into a
        // vault; the same reasoning bars depositing any other actor (a custody
        // record is a claim about a thing, and imprisonment is not a custody op).
        if obj == a || self.w.get(obj).is_some_and(|o| o.kind.is_actor()) {
            return false;
        }
        // Topology is not custody either (see `custody_movable`): an exit filed
        // into `exits` would be journaled and then invisible to every query.
        if !custody_movable(self.w, obj) {
            return false;
        }
        let obj_loc = self.w.get(obj).and_then(|o| o.location);
        if obj_loc == Some(a) {
            return true;
        }
        let actor_loc = self.w.get(a).and_then(|o| o.location);
        if obj_loc.is_none() || obj_loc != actor_loc {
            return false;
        }
        if crate::api::is_staff(self.w, a) || perm::controls(self.w, a, obj) {
            return true;
        }
        let principal = PrincipalId::Object(a);
        self.w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().asset(obj))
            .is_some_and(|asset| {
                asset.title_owner == principal
                    || asset
                        .assignment
                        .as_ref()
                        .is_some_and(|x| x.assigned_to == principal)
            })
    }
    /// Invariant 15's destination rule: the actor or the actor's current location,
    /// live, not `GOING` — and **not inside any vault's backing**.
    ///
    /// The engine owns the direct check (`is_vault_backing(to)`), but only the world
    /// can see the containment CHAIN above `to`: standing inside vault B's backing,
    /// `@vault/withdraw x=A` (destination `to = me`) would otherwise land the object
    /// in vault B with no Deposit authorization, no standing check, and no custody
    /// effect recording it — laundering by geography. Entering a vault is a
    /// `Deposit`, at any depth.
    fn destination_ok(&self, actor: &PrincipalId, to: ObjId) -> bool {
        let PrincipalId::Object(a) = actor else {
            return false;
        };
        let a = *a;
        let reachable = to == a || self.w.get(a).and_then(|o| o.location) == Some(to);
        if !reachable || !live(self.w, to) {
            return false;
        }
        let Some(store) = self.w.module_state::<EconomyStore>() else {
            return false;
        };
        let snapshot = store.snapshot();
        let mut cur = Some(to);
        for _ in 0..=MAX_CONTAINMENT_DEPTH {
            let Some(id) = cur else { return true };
            if snapshot.is_vault_backing(id) {
                return false;
            }
            cur = self.w.get(id).and_then(|o| o.location);
        }
        // A chain deeper than the cap is a world we cannot prove safe: fail closed.
        false
    }
}

// ---- E7c2: the quantity-endpoint state seam (10b §3.2, OPEN 1) ------------

/// What a quantity endpoint (`mush-inventory`'s `EndpointRef(VaultId)`) may do
/// right now.
///
/// E7 cut 2 binds every balance to an **E3 custody endpoint** and adds no second
/// custody model: this is the one question it asks, answered here so the rules
/// stay E3's — the vault's own lifecycle, the frozen provider table's
/// `backing_valid`, and `resolve`'s exactly-one-claiming-provider rule.
/// Nothing here mutates, and nothing here is a permission check: who may *see*
/// or *use* an endpoint is the caller's own disclosure question.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EndpointState {
    /// No such vault.
    Missing,
    /// The vault exists but is `Frozen` or `Closed`.
    Inactive(crate::record::VaultStatus),
    /// No registered provider handles the backing, or the provider cannot
    /// confirm it is a live usable container (`None` is fail-closed).
    BackingUnavailable,
    /// Another registered provider claims the backing object, so routing
    /// anything through this endpoint would strand it (E3's `resolve`).
    Contested {
        /// The provider that claims it.
        provider: String,
    },
    /// Live: active, backed, and uncontested.
    Live,
}

impl EndpointState {
    /// Whether quantity may move at this endpoint at all.
    #[must_use]
    pub const fn is_live(&self) -> bool {
        matches!(self, Self::Live)
    }
}

/// Prose, not Rust struct syntax. The audience for an endpoint's state is
/// already entitled to the vault (E7c2's B11 was refuted as a leak precisely
/// because the view gate sits above the render), so this is presentation polish
/// rather than a disclosure change — but `Contested { provider: "sim" }` is not
/// a sentence.
impl std::fmt::Display for EndpointState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(f, "no such vault"),
            Self::Inactive(status) => write!(f, "{status:?}"),
            Self::BackingUnavailable => write!(f, "backing unavailable"),
            Self::Contested { provider } => write!(f, "contested by {provider}"),
            Self::Live => write!(f, "live"),
        }
    }
}

/// Whether `actor` holds E3's **withdraw** standing over that vault (§4.4's
/// vault policy, `CustodyOp::Withdraw`): staff, the vault's own owner — itself,
/// or whoever MUSH-controls it — or `vault.withdraw` on the vault's compartment
/// under `scope_coherent`.
///
/// Exported (E7c2 fold C8, owner decision D4; 08i answer 16's owner-exported
/// `pub fn` seam) because a quantity endpoint's **take** gate and its **read**
/// gate are different questions and cut 2 had one predicate answering both. The
/// difference is not cosmetic: `acts_for`'s `Organization` arm answers true on
/// `economy.account.post`, so "may post the org's money" was being read as "may
/// walk the org's physical stock out". Vault ops never consult title (D5), and
/// this adds no capability to E3's vocabulary — `vault.withdraw` is the one
/// `authorize_vault` already asks for.
///
/// The per-deposit `ContractAuth` arm of §4.4 is deliberately not reachable
/// here: it authorizes one named object into one named vault, never a pool take.
#[must_use]
pub fn may_take_from_vault(w: &World, actor: ObjId, vault: crate::ids::VaultId) -> bool {
    let Some(store) = w.module_state::<EconomyStore>() else {
        return false;
    };
    let Some(record) = store.snapshot().vault(vault) else {
        return false;
    };
    if crate::api::is_staff(w, actor) {
        return true;
    }
    if matches!(record.owner, PrincipalId::Object(_)) && crate::acts_for(w, actor, &record.owner) {
        return true;
    }
    let Some(scope) = record.scope else {
        return false;
    };
    crate::api::scope_coherent(w, Some(scope), &record.owner)
        && crate::api::holds_capability(w, actor, "vault.withdraw", scope)
}

/// Resolve one quantity endpoint's state, asking E3's own rules and rebuilding
/// none of them.
///
/// The economy is the only crate that may read the wiring-frozen provider table,
/// so this is the seam a neutral quantity authority calls instead of growing a
/// custody model of its own (10b §3.2's rules; D-E7-3).
#[must_use]
pub fn endpoint_state(w: &World, vault: crate::ids::VaultId) -> EndpointState {
    let Some(store) = w.module_state::<EconomyStore>() else {
        return EndpointState::BackingUnavailable;
    };
    let Some(record) = store.snapshot().vault(vault) else {
        return EndpointState::Missing;
    };
    if record.status != crate::record::VaultStatus::Active {
        return EndpointState::Inactive(record.status);
    }
    let providers = store.providers();
    match resolve(&providers, w, &record.backing, record.backing.object()) {
        Ok(provider) => {
            if provider.backing_valid(w, &record.backing) == Some(true) {
                EndpointState::Live
            } else {
                EndpointState::BackingUnavailable
            }
        }
        Err(CustodyError::Claimed { provider, .. }) => EndpointState::Contested { provider },
        Err(_) => EndpointState::BackingUnavailable,
    }
}
