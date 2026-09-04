//! The world-facing hooks (E5b, 08d §4): access enforcement, the destroy gate, the
//! programmatic-destruction backstop, and the reconnect notice.
//!
//! ## Access is only ever this hook (D-E5-4)
//!
//! There is no lock, no exit rewrite and no `home` routing (owner answer 10). One
//! [`AccessHook`] is registered under **both** `MoveAttempt` and `CarryAttempt`,
//! because E5a's `may_take` *is* `may_enter` and registering one object under both
//! points is how that stays true at the module layer. The chain is first-veto-wins,
//! so this hook only ever needs to answer for rooms it owns and abstain everywhere
//! else.
//!
//! The decision, in order, for a move whose destination is room `r`:
//!
//! | question | answer |
//! |---|---|
//! | is `r` a room of a property this store knows? | no → **allow** (abstain) |
//! | is that property `Let`? | no → **allow** — a `Vacant` or `Withdrawn` residence is an ordinary room |
//! | does it have exactly one live tenancy? | no → **allow**: nobody lives there yet |
//! | [`crate::may_enter_with`] | `Refused` → **veto** with one neutral text; anything else → allow |
//!
//! A carry asks the same question **twice** — of the room the object is coming out
//! of and of the room it is going into — because "may not enter" has to mean both
//! "may not reach in" and "may not post things through the letterbox".
//!
//! One neutral refusal text for every case (§4.5's existence hiding, applied to
//! doors): "denied because a stranger lives there" and "denied because you are not on
//! the lease" must not be distinguishable, or the refusal itself becomes a census.

use mush_core::behavior::DestroyGate;
use mush_core::module::{EventHook, WorldEvent};
use mush_core::{Ctx, ObjId, Registry, World};
use mush_proto::RichText;
use mush_economy::{EscalationKind, EscalationSubject};
use mush_org::{OrgScopeId, PrincipalId};

use crate::access::Access;
use crate::module::{HousingStore, is_let, is_staff, room_of};
use crate::record::{Property, PropertyStatus};

/// The one refusal a stranger ever sees at a private door.
pub const REFUSED: &str = "You can't go that way.";
/// The carry half of the same neutral text.
pub const REFUSED_CARRY: &str = "You can't move that from there.";
/// What `@destroy` says to somebody who may not see the property.
pub const NO_DESTROY: &str = "You cannot destroy that right now.";

/// The `MoveAttempt` / `CarryAttempt` hook.
#[derive(Debug)]
pub struct AccessHook;

impl EventHook for AccessHook {
    fn move_attempt(
        &self,
        w: &mut World,
        actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        // `check_move_attempt` hands us the **moved object**, never the actor who
        // issued a `@teleport` (08d §1.1 scopes that limitation): a staffer
        // teleporting a stranger in is refused exactly like the stranger walking in,
        // which is the fail-closed direction. Add them as a guest first.
        match may_reach(w, actor, dest) {
            Access::Refused => Err(RichText::tag("error", REFUSED)),
            _ => Ok(()),
        }
    }

    fn carry_attempt(
        &self,
        w: &mut World,
        mover: ObjId,
        obj: ObjId,
        dest: ObjId,
    ) -> Result<(), RichText> {
        // Where it is coming FROM, and where it is going TO. Reaching into somebody's
        // home and dropping something into it are the same right.
        for room in [room_of(w, obj), room_of(w, dest)].into_iter().flatten() {
            if matches!(may_reach(w, mover, room), Access::Refused) {
                return Err(RichText::tag("error", REFUSED_CARRY));
            }
        }
        Ok(())
    }
}

/// The access answer for one `(who, room)` pair — the module-layer half of E5a's
/// predicate, and the **only** place the two are joined.
///
/// It abstains ([`Access::Staff`], the permissive answer) for every room this module
/// does not own, so a world with no properties behaves byte for byte as it did
/// before housing existed.
#[must_use]
pub fn may_reach(w: &World, who: ObjId, room: ObjId) -> Access {
    let Some(store) = w.module_state::<HousingStore>() else { return Access::Staff };
    let Some(property) = store.property_of_room(room) else { return Access::Staff };
    if !is_let(property) {
        return Access::Staff;
    }
    let Some(t) = store.live_tenancy_of(property.id) else { return Access::Staff };
    // The inspection term's authority is an org/world question E5a cannot answer, so
    // it is answered here and handed in (`may_enter_with`).
    let inspector = t
        .terms
        .inspection
        .as_ref()
        .is_some_and(|term| holds_principal(w, who, &term.who, property.landlord_scope));
    // A tenancy whose cadence clock has no record resolves to **no now at all**, and
    // `i64::MIN` is how that reaches the predicate: it makes `WhileDelinquent`'s
    // `now >= since` false, so a clock nobody can read never opens a door.
    let now = crate::module::cadence_now(w, t.cadence).unwrap_or(i64::MIN);
    crate::may_enter_with(t, &t.terms, who, inspector, is_staff(w, who), now)
}

/// Does `who` hold the authority `principal` names? (E5a's `may_enter_with` seam.)
///
/// An object principal is themself. An organization is whoever holds
/// `property.manage` on the compartment the property names — or, failing that, on the
/// org's root scope — because inspection is a **management** right, not a viewing
/// one. A system principal is held by **nobody, staff included**, exactly as
/// `mush_economy::WorldAuth::acts_for` decides: a standing right of entry belonging
/// to a faceless principal is one nobody can be held to having exercised.
#[must_use]
fn holds_principal(
    w: &World,
    who: ObjId,
    principal: &PrincipalId,
    hint: Option<OrgScopeId>,
) -> bool {
    match principal {
        PrincipalId::Object(o) => *o == who,
        PrincipalId::Organization(org) => {
            let Some(store) = w.module_state::<mush_org::OrgStore>() else { return false };
            let state = store.state();
            let scope = hint
                .filter(|s| state.scope(*s).is_some_and(|sc| sc.org == *org))
                .or_else(|| state.org(*org).map(|o| o.root_scope));
            scope.is_some_and(|s| crate::commands::holds(w, who, "property.manage", s))
        }
        PrincipalId::System(_) => false,
    }
}

// ---------------------------------------------------------------------------
// The destroy gate + the programmatic backstop
// ---------------------------------------------------------------------------

/// Refuses `@destroy` of any room of a `Let` property (08d §4, doc 08 §15.3).
///
/// Refusing **here** — before `World::destroy` marks the husk `GOING` and disperses
/// its contents — is the only point at which the room can still be saved. The
/// `ObjDestroyed` observer below is the backstop for destruction that never passes
/// this gate at all, and it can only tidy up after the fact.
#[derive(Debug)]
pub struct HousingDestroyGate;

impl DestroyGate for HousingDestroyGate {
    fn check(&self, w: &World, _reg: &Registry, actor: ObjId, target: ObjId) -> Result<(), String> {
        let Some(store) = w.module_state::<HousingStore>() else { return Ok(()) };
        let Some(p) = store.property_of_room(target).filter(|p| is_let(p)) else {
            return Ok(());
        };
        // Existence hiding, at the gate: only somebody who may already see the
        // property learns why. Everyone else gets the same sentence they would get
        // for a room that is merely locked down.
        if crate::commands::may_view_property(w, actor, p) {
            Err(format!(
                "#{} is a room of property #{} (\"{}\"), which is let. Withdraw the property \
                 first (@property/withdraw {}), or end the tenancy (@tenancy/resolve).",
                target.0, p.id, p.name, p.id
            ))
        } else {
            Err(NO_DESTROY.to_string())
        }
    }
}

/// `ObjDestroyed` observer: a property room died anyway (a programmatic destroy, a
/// gate that was not consulted).
///
/// Three things happen, and one deliberately does not:
///
/// - the property goes **`Withdrawn`** — the access hook and the destroy gate stop
///   answering for its remaining rooms, because a residence with a hole in it is out
///   of service;
/// - every tenancy on it is **left exactly as it is** (D-E5-1: a home does not end
///   because its rooms went away, and the tenants' `Obj.home` is untouched so their
///   own dispersal still works);
/// - a **level-2 page** carries the two commands that answer it.
///
/// A destroyed *tenant* is not handled here: `remove_tenant` writes a history entry
/// and the roster is a container document, so it belongs to the boot pass that can do
/// the whole reconciliation at once (`crate::boot`).
pub(crate) fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else { return };
    let obj = *obj;
    let hit: Option<Property> = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else { return };
        store
            .property_of_room(obj)
            .filter(|p| p.status != PropertyStatus::Withdrawn)
            .cloned()
    };
    let Some(mut p) = hit else { return };
    let (id, name) = (p.id, p.name.clone());
    let was_let = is_let(&p);
    p.status = PropertyStatus::Withdrawn;
    p.revision = p.revision.saturating_add(1);
    crate::module::put_property(ctx, p);
    tracing::error!(
        property = id.0,
        room = obj.0,
        was_let,
        "housing: a property room was destroyed; the property is withdrawn, tenancies untouched"
    );
    mush_economy::open_escalation(
        ctx,
        EscalationKind::BootReconciliation,
        EscalationSubject::HousingProperty(id.0),
        format!(
            "room #{} of property #{id} (\"{name}\") was destroyed{}. The property is withdrawn \
             and every tenancy on it is untouched — nothing ends a home automatically. Rebuild \
             the room and @property/rooms it back, or end the tenancy deliberately",
            obj.0,
            if was_let { " while it was LET" } else { "" }
        ),
        vec![format!("@property/show {id}"), format!("@tenancy/list property={id}")],
    );
    // Whoever lives there should hear it from the game rather than from a wall that
    // is not there any more.
    let tenants: Vec<ObjId> = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|s| s.live_tenancy_of(id).map(|t| t.tenants.iter().copied().collect()))
        .unwrap_or_default();
    for who in tenants {
        crate::notice::notify(
            ctx,
            who,
            format!(
                "HOME: part of {name} is gone and the property has been withdrawn. Your tenancy \
                 is unchanged; staff have been paged."
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// The reconnect notice
// ---------------------------------------------------------------------------

/// Renders whatever the tenant missed while they were away (the `mush-mail`
/// pattern).
#[derive(Debug)]
pub struct ReconnectNotice;

impl EventHook for ReconnectNotice {
    fn connected(&self, ctx: &mut Ctx<'_>, actor: ObjId) {
        crate::notice::render_pending(ctx, actor);
    }
}
