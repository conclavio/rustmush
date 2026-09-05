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
//! | does that property have exactly one live tenancy? | no → **allow**: nobody lives there |
//! | [`crate::may_enter_with`] | `Refused` → **veto** with one neutral text; anything else → allow |
//!
//! **The `Let` latch is not part of that decision** (verify fold, H1). It used to be
//! the first question, and it made `Withdrawn` a master key: withdrawing a property —
//! or a single room dying, which withdraws it automatically — cancelled every access
//! rule on a home somebody still lived in, while D-E5-1 guarantees the tenancy itself
//! is untouched. Withdrawal is a *service* decision, so it still lifts the destroy
//! gate (that is what it is for), and it still stops nothing about who may walk in.
//! The question the hook asks is the only one that means anything at a door: **does
//! somebody live here?**
//!
//! A carry asks the same question **twice** — of the room the object is coming out
//! of and of the room it is going into — because "may not enter" has to mean both
//! "may not reach in" and "may not post things through the letterbox".
//!
//! One neutral refusal text for every case (§4.5's existence hiding, applied to
//! doors): "denied because a stranger lives there" and "denied because you are not on
//! the lease" must not be distinguishable, or the refusal itself becomes a census.

use mush_core::behavior::DestroyGate;
use mush_core::module::{EventHook, HookReadContext, WorldEvent};
use mush_core::{Ctx, ObjId, Registry, World};
use mush_economy::{EscalationKind, EscalationSubject};
use mush_org::{OrgScopeId, PrincipalId};
use mush_proto::RichText;

use crate::access::Access;
use crate::module::{HousingStore, LiveEntry, RoomEntry, is_let, is_staff, room_of};
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
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        _exit: Option<ObjId>,
        dest: ObjId,
    ) -> Result<(), RichText> {
        let w = ctx.world();
        // `check_move_attempt` hands us the **moved object**, never the actor who
        // issued a `@teleport` (08d §1.1 scopes that limitation): a staffer
        // teleporting a stranger in is refused exactly like the stranger walking in,
        // which is the fail-closed direction. Add them as a guest first.
        if matches!(may_reach(w, actor, dest), Access::Refused) {
            return Err(RichText::tag("error", REFUSED));
        }
        // **The source half, for things** (verify fold, H2). A move only ever asked
        // about the destination, so `@teleport #lamp=#yard` walked an object straight
        // out of a home nobody was allowed to reach into — the exact hole
        // `carry_attempt` closes for `get`, reopened by a different verb.
        //
        // The hook cannot see who issued the teleport (08d §1.1's scoped limitation,
        // and the reason the destination check is already phrased as "the moved
        // object's own standing"), so the source check is the same shape and fails
        // the same way: a thing has no standing of its own, so a thing standing in an
        // occupied home cannot be teleported out of it **by anybody**. A tenant moves
        // their own belongings with `get`/`drop`, which is `carry_attempt` and does
        // ask about the mover.
        //
        // **People are exempt on purpose.** A person is never trapped by this hook:
        // an ex-tenant standing in the house they were just evicted from must always
        // be able to walk out, and a staffer must always be able to pull somebody
        // out. It is the same asymmetry `to_unhome_on_end` has — the door is about
        // reaching *in*.
        if !crate::module::is_character(w, actor)
            && let Some(from) = room_of(w, actor)
            && matches!(may_reach(w, actor, from), Access::Refused)
        {
            return Err(RichText::tag("error", REFUSED_CARRY));
        }
        Ok(())
    }

    fn carry_attempt(
        &self,
        ctx: &HookReadContext<'_>,
        mover: ObjId,
        obj: ObjId,
        dest: ObjId,
    ) -> Result<(), RichText> {
        let w = ctx.world();
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
///
/// **A conflict fails closed** (look-back fold, D-LBK-1/4a): a room two properties
/// hold, a property two live tenancies claim, or any room of a property a quarantine
/// involves is `Refused` for everyone but staff. The projections' `Conflict` entries
/// are the source of truth; the quarantine is their consequence.
#[must_use]
pub fn may_reach(w: &World, who: ObjId, room: ObjId) -> Access {
    let Some(store) = w.module_state::<HousingStore>() else {
        return Access::Staff;
    };
    let closed = || {
        if is_staff(w, who) {
            Access::Staff
        } else {
            Access::Refused
        }
    };
    let property = match store.room_entry(room) {
        None => return Access::Staff,
        Some(RoomEntry::Conflict(_)) => return closed(),
        Some(RoomEntry::Unique(id)) => *id,
    };
    if store.is_quarantined_property(property) {
        return closed();
    }
    let Some(property) = store.property(property) else {
        return Access::Staff;
    };
    // **The live tenancy alone, never the `Let` latch** (fold H1). See the module
    // doc: `Withdrawn` is a service decision about the destroy gate, and a home
    // somebody still lives in does not stop being theirs because the property was
    // taken out of service or a spare room fell down.
    let t = match store.live_entry(property.id) {
        Some(LiveEntry::Live(id)) => store.tenancy(*id),
        Some(LiveEntry::Conflict(_)) => return closed(),
        Some(LiveEntry::Vacant) | None => None,
    };
    let Some(t) = t else { return Access::Staff };
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
        // **One authority path** (fold A4): the compartment is resolved by the same
        // helper `may_view_property` uses — the hinted one when it really belongs to
        // this org, else the org's root scope — so a property created without
        // `scope=` is not a different question here than it is there. The
        // *capability* deliberately stays `property.manage` alone: inspection is a
        // management right, and a read grant must not open a door.
        PrincipalId::Organization(_) => crate::commands::officer_scope(w, principal, hint)
            .is_some_and(|s| crate::commands::holds(w, who, "property.manage", s)),
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
        let Some(store) = w.module_state::<HousingStore>() else {
            return Ok(());
        };
        // Look-back fold, D-LBK-1: a room under review is refused to everyone. A
        // viewer of any property involved is told so; everyone else gets the
        // neutral sentence.
        let review = match store.room_entry(target) {
            None => return Ok(()),
            Some(RoomEntry::Conflict(props)) => Some(props.clone()),
            Some(RoomEntry::Unique(id)) if store.is_quarantined_property(*id) => {
                Some(std::collections::BTreeSet::from([*id]))
            }
            Some(RoomEntry::Unique(_)) => None,
        };
        if let Some(props) = review {
            let may_view = props
                .iter()
                .filter_map(|p| store.property(*p))
                .any(|p| crate::commands::may_view_property(w, actor, p));
            return Err(if may_view {
                format!(
                    "#{} is a room of a property under housing review (@housing/quarantine show); \
                     repair the conflict first.",
                    target.0
                )
            } else {
                NO_DESTROY.to_string()
            });
        }
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
///
/// **The latch is not the condition** (verify fold, H6). It used to be: a property
/// that was already `Withdrawn` — because the first room died, or because staff
/// withdrew it deliberately to clear the way for a demolition — was skipped entirely,
/// so the *second* room of somebody's home fell down in total silence, and so did the
/// third, and so did the anchor. What matters is whether anybody still lives there,
/// and the economy's ladder dedupes by subject, so paging every time costs one record
/// rather than one per room.
pub(crate) fn on_obj_destroyed(ctx: &mut Ctx<'_>, ev: &WorldEvent) {
    let WorldEvent::ObjDestroyed { obj } = ev else {
        return;
    };
    let obj = *obj;
    // Every property that held the room — one, or (a quarantined overlap) several;
    // each is withdrawn and paged on its own record (look-back fold, D-LBK-4a).
    let hits: Vec<(Property, bool)> = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else {
            return;
        };
        let ids: Vec<crate::ids::PropertyId> = match store.room_entry(obj) {
            None => Vec::new(),
            Some(RoomEntry::Unique(id)) => vec![*id],
            Some(RoomEntry::Conflict(set)) => set.iter().copied().collect(),
        };
        ids.into_iter()
            .filter_map(|id| {
                store
                    .property(id)
                    .map(|p| (p.clone(), store.has_live_tenancy(p.id)))
            })
            .collect()
    };
    for (p, live) in hits {
        withdraw_destroyed(ctx, obj, p, live);
    }
}

fn withdraw_destroyed(ctx: &mut Ctx<'_>, obj: ObjId, p: Property, live: bool) {
    if p.status == PropertyStatus::Withdrawn && !live {
        // Already out of service and nobody's home: there is no decision left to
        // hand a staffer, so this is the one case that stays quiet.
        return;
    }
    let (id, name) = (p.id, p.name.clone());
    let was_let = is_let(&p);
    if p.status != PropertyStatus::Withdrawn {
        let mut next = p.clone();
        next.status = PropertyStatus::Withdrawn;
        next.revision = next.revision.saturating_add(1);
        crate::module::put_property(ctx, next);
    }
    tracing::error!(
        property = id.0,
        room = obj.0,
        was_let,
        live,
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
        vec![
            format!("@property/show {id}"),
            format!("@tenancy/list property={id}"),
        ],
    );
    // Whoever lives there should hear it from the game rather than from a wall that
    // is not there any more.
    let tenants: Vec<ObjId> = ctx
        .w
        .module_state::<HousingStore>()
        .and_then(|s| {
            s.live_tenancy_of(id)
                .map(|t| t.tenants.iter().copied().collect())
        })
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
