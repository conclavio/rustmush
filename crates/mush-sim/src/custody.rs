//! The interior custody backend (E3c, doc 08b §5.1) — how an economy
//! [`Vault`](mush_economy::Vault) backed by a `sim.interior` space reaches the
//! placement graph.
//!
//! `mush-economy` owns the *policy* (who may deposit, what the vault is, the
//! journal entry that records it); this module owns the *physics* (a placed unit
//! moves only through a [`Transition`]). The seam is
//! [`mush_economy::CustodyProvider`], and the implementation lives here — with
//! the data it reads — exactly as E1c's faction-standing predicate lives in
//! `mush-faction`.
//!
//! ## The three rules this file exists to keep
//!
//! - **Sim owns the object's motion, so sim answers `claims`.** `claims` is the
//!   simulation's OWN movement veto — [`SpatialState::is_sim_bound`] (placed,
//!   quarantined, a domain/space/mount/station host). Since S2, the atlas owns
//!   the movement veto for its travelers. The simulation predicate makes the
//!   economy's `ContainerProvider` refuse to route a hull into a footlocker.
//! - **`None` means "sim is not wired", never "no".** [`SpatialState`] is
//!   installed at *registration*, so the slot exists from the moment
//!   [`SimulationModule`](crate::SimulationModule) is in the list; a definite
//!   `Some(false)` therefore requires the module's own boot to have run. The
//!   server wires `EconomyModule` **after** `SimulationModule` for exactly this
//!   reason (`mush-server/src/modules.rs`).
//! - **Validate, then mutate — never half-apply.** `mush_economy::post` runs a
//!   custody move BEFORE the store advances, and a handler `Err` discards
//!   deferred persists but not world writes. [`crate::transition::plan`] validates and
//!   [`crate::transition::apply`] mutates, so [`run`] is already all-or-nothing; the
//!   one thing that is not inside it — the destination of a withdrawal — is
//!   pre-validated here, before the first record changes.

use mush_core::output::display_name;
use mush_core::{Ctx, Flags, HookPoint, ObjId, World};
use mush_economy::{CustodyError, CustodyProvider, CustodyRef};

use crate::data::{MAX_SPATIAL_DEPTH, NS, SpaceAnchor, SpatialParent, SpatialState};
use crate::transition::{Transition, run};

/// The behavior [`Transition::Place`] requires. The provider **attaches
/// nothing**: making an object a simulation participant is an authoring act
/// (`@behavior/add`), not a side effect of a bank deposit.
const PARTICIPANT: &str = "sim.participant";

/// The `mush-sim` custody backend for [`CustodyRef::SpatialInterior`] backings:
/// a hangar, a bay, any room carrying `sim.interior` and holding placed units.
///
/// Wire it into the economy at boot:
/// `EconomyModule::with_custody(vec![Arc::new(InteriorCustodyProvider)])`.
#[derive(Debug, Default, Clone, Copy)]
pub struct InteriorCustodyProvider;

fn live(w: &World, o: ObjId) -> bool {
    w.get(o).is_some_and(|x| !x.flags.contains(Flags::GOING))
}

/// Why the simulation — not an ordinary container — owns `obj`'s motion, if it
/// does. The same predicate the `PlacementGate` movement veto is built from,
/// deliberately taken WHOLE: a space/mount/station host is not placed, yet may
/// not be picked up and dropped into a strongbox. (A traveler on `mush-atlas`'s
/// road is gated by the atlas's own `MoveAttempt` hook since S2.)
#[must_use]
fn claim_reason(w: &World, obj: ObjId) -> Option<String> {
    let st = w.module_state::<SpatialState>()?;
    st.is_sim_bound(obj)
        .then(|| format!("{} is bound into the simulation.", display_name(w, obj)))
}

/// Everything `space` holds through INTERIOR placement, transitively through
/// nested spaces (a bay riding a dropship parked in the hangar), depth-capped
/// at [`MAX_SPATIAL_DEPTH`].
///
/// Attached units are deliberately excluded: a mount is a structural fitting of
/// its host, not a thing the vault independently holds — `holds` and `contents`
/// agree on that, so a withdrawal can never name something `contents` never
/// listed.
fn interior_contents(st: &SpatialState, space: ObjId) -> Vec<ObjId> {
    let mut out: Vec<ObjId> = Vec::new();
    let mut frontier: Vec<ObjId> = vec![space];
    for _ in 0..MAX_SPATIAL_DEPTH {
        let mut next: Vec<ObjId> = Vec::new();
        for s in frontier.drain(..) {
            for &c in st.children_of(s) {
                if !matches!(st.parent_of(c), SpatialParent::Interior { space: p, .. } if p == s) {
                    continue;
                }
                if out.contains(&c) {
                    continue;
                }
                out.push(c);
                // A space riding `c` nests INSIDE this one: its occupants are
                // held here too (a container deposited whole is not a
                // laundering primitive — §4.3).
                if let Some(riding) = st.derived.anchored_spaces.get(&c) {
                    next.extend(riding.iter().copied());
                }
                if st.spaces.contains_key(&c) {
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

/// The inverse walk of [`interior_contents`]: whether `obj`'s interior parent
/// chain reaches `target`, stepping out of a nested space through its
/// `MovingObject` anchor. Bounded by [`MAX_SPATIAL_DEPTH`].
fn interior_holder(st: &SpatialState, obj: ObjId, target: ObjId) -> bool {
    let mut cur = obj;
    for _ in 0..MAX_SPATIAL_DEPTH {
        let SpatialParent::Interior { space, .. } = st.parent_of(cur) else {
            return false;
        };
        if space == target {
            return true;
        }
        match st.spaces.get(&space).map(|s| s.anchor.clone()) {
            Some(SpaceAnchor::MovingObject { host }) if host != space => cur = host,
            _ => return false,
        }
    }
    false
}

impl CustodyProvider for InteriorCustodyProvider {
    fn provider(&self) -> &str {
        NS
    }

    fn handles(&self, backing: &CustodyRef) -> bool {
        matches!(backing, CustodyRef::SpatialInterior(_))
    }

    fn claims(&self, w: &World, obj: ObjId) -> bool {
        claim_reason(w, obj).is_some()
    }

    /// `Some(true)` iff an [`InteriorSpaceRecord`](crate::InteriorSpaceRecord)
    /// exists for the space, its object is live (present, not `GOING`), and it
    /// is not quarantined. **`None` only when [`SpatialState`] is absent
    /// altogether** — i.e. `SimulationModule` is not registered at all. The slot
    /// exists (empty) from registration onward, so a `Some(false)` from a
    /// registered-but-unbooted module is a real risk: the server's module order
    /// (sim before economy) is what makes it impossible.
    fn backing_valid(&self, w: &World, backing: &CustodyRef) -> Option<bool> {
        let CustodyRef::SpatialInterior(space) = backing else {
            return None;
        };
        let st = w.module_state::<SpatialState>()?;
        Some(st.spaces.contains_key(space) && live(w, *space) && !st.is_quarantined(*space))
    }

    fn contents(&self, w: &World, backing: &CustodyRef) -> Option<Vec<ObjId>> {
        let CustodyRef::SpatialInterior(space) = backing else {
            return None;
        };
        let st = w.module_state::<SpatialState>()?;
        if !live(w, *space) || !st.spaces.contains_key(space) {
            return Some(Vec::new());
        }
        Some(interior_contents(st, *space))
    }

    fn holds(&self, w: &World, backing: &CustodyRef, obj: ObjId) -> Option<bool> {
        let CustodyRef::SpatialInterior(space) = backing else {
            return None;
        };
        let st = w.module_state::<SpatialState>()?;
        Some(interior_holder(st, obj, *space))
    }

    /// Place `obj` inside the space, slot-less. The object must ALREADY carry
    /// `sim.participant` — [`transition::plan`](crate::transition::plan)
    /// requires it and this provider attaches no behaviors, because a deposit
    /// is a custody act and "is this thing a simulation entity?" is an
    /// authoring one.
    ///
    /// [`run`] plans (pure, total) before it applies, so every refusal below
    /// leaves the world, the records, and the outbox untouched.
    fn move_into(
        &self,
        ctx: &mut Ctx<'_>,
        _actor: ObjId,
        obj: ObjId,
        backing: &CustodyRef,
    ) -> Result<(), CustodyError> {
        let CustodyRef::SpatialInterior(space) = backing else {
            return Err(CustodyError::NoProvider);
        };
        let participant = mush_core::BehaviorId::new(PARTICIPANT).expect("static behavior id");
        if !ctx.w.has_behavior(obj, &participant) {
            return Err(CustodyError::Refused(format!(
                "{} does not carry `{PARTICIPANT}`; it cannot be placed inside {} (@behavior/add first).",
                display_name(ctx.w, obj),
                display_name(ctx.w, *space)
            )));
        }
        run(
            ctx,
            &Transition::Place {
                obj,
                parent: SpatialParent::Interior {
                    space: *space,
                    slot: None,
                },
            },
        )
        .map_err(|e| CustodyError::Refused(e.to_string()))
    }

    /// Take `obj` out of the space and hand it to `to`, in three steps that
    /// cannot half-apply:
    ///
    /// 1. **Pre-validate `to`** — live, world-legal (`can_move_to`), and past
    ///    every OTHER module's `CarryAttempt` gate. Nothing has mutated yet.
    /// 2. `Unplace { keep: true }` — the object stops being a simulation
    ///    citizen. `keep: true` skips the recovery-room projection, so
    ///    `Obj.location` stays exactly where the placement put it (the space
    ///    object itself — `graph::project` maps `Interior { space }` to
    ///    `space`), and the placement record becomes `Unplaced` **without** the
    ///    `quarantined` latch, i.e. a plain object again.
    /// 3. The single `World::move_to(obj, to)`.
    ///
    /// The `keep: false` form would be two moves — recovery room, then `to` —
    /// and `plan` validates only the first of them (`transition.rs`
    /// `Unplace`), appending the recovery-log line before the second move ever
    /// runs. A refusal on that second hop would strand the object in the
    /// recovery room with an audit entry claiming a withdrawal that never
    /// happened. Hence: pre-validate, `keep: true`, one move.
    ///
    /// **Deviation from plan §5.1 (deliberate):** the carry-hook chain is
    /// consulted **minus this module's own hooks**. `PlacementGate` is
    /// registered on `CarryAttempt` and vetoes *any* move of a placed object —
    /// it would refuse every withdrawal by construction, since being placed is
    /// the precondition of withdrawing. Running the chain after the `Unplace`
    /// instead would let a foreign gate refuse too late to be atomic. Skipping
    /// sim's own veto (and only sim's) keeps both properties: a ruleset's
    /// no-take rule is still honoured, and the refusal still arrives before the
    /// first mutation.
    fn move_out(
        &self,
        ctx: &mut Ctx<'_>,
        actor: ObjId,
        obj: ObjId,
        backing: &CustodyRef,
        to: ObjId,
    ) -> Result<(), CustodyError> {
        let CustodyRef::SpatialInterior(space) = backing else {
            return Err(CustodyError::NoProvider);
        };
        // ---- 1. pre-validate the destination (no mutation past this point fails) ----
        if !live(ctx.w, obj) {
            return Err(CustodyError::Refused(format!(
                "#{} is not available to move.",
                obj.0
            )));
        }
        if !live(ctx.w, to) {
            return Err(CustodyError::Refused(format!(
                "#{} cannot hold anything.",
                to.0
            )));
        }
        ctx.w
            .can_move_to(obj, to)
            .map_err(|e| CustodyError::Refused(e.to_string()))?;
        for h in ctx.reg.hooks(HookPoint::CarryAttempt).to_vec() {
            if h.module == NS {
                continue; // our own placement veto — see the doc comment.
            }
            let hook_ctx = ctx.hook_read_context(&h.module);
            h.hook
                .carry_attempt(&hook_ctx, actor, obj, to)
                .map_err(|rt| CustodyError::Refused(rt.to_plain_text()))?;
        }
        // ---- 2. leave the simulation, keeping the projection where it is ----
        let reason = format!("withdrawn from {}", display_name(ctx.w, *space));
        run(
            ctx,
            &Transition::Unplace {
                obj,
                reason,
                by: Some(actor),
                keep: true,
            },
        )
        .map_err(|e| CustodyError::Refused(e.to_string()))?;
        // ---- 3. the ONE world move ----
        ctx.w
            .move_to(obj, to)
            .map_err(|e| CustodyError::Refused(e.to_string()))
    }
}
