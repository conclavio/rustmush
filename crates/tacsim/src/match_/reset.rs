//! **The stock tanks, put back** (16b cut F.1; doc 16 §3.3 "The stock tanks are
//! persistent objects", §3.1 "Custody", §3.8 "Lifecycle").
//!
//! Doc 16 §3.3: *eight tank objects, one per pod, live in the arena's domain and
//! are created once by the centre's content. At check-in and at every `Idle`
//! transition each is reset to its `UnitSpec` by one `UnitMutationPlan` (cells to
//! initial values, conditions to `intact`, capabilities restored). No objects are
//! created or destroyed per match.* Three comments in the tree and two rows of the
//! 16b ledger said this had been built. **Nothing reset anything.** The readiness
//! review of 2026-09-18 fought three matches by hand and found the loser's tank
//! still reading `section hull: wreck` after `@arena/reset`, starting up in match
//! two, driving, and absorbing fourteen rounds without dying — because
//! `Arena::plan_hit` fires a section's consequences only on the transition to zero
//! internals, and that transition had already happened in match one.
//!
//! # Where the reset happens, and where it deliberately does not
//!
//! Doc 16 says *at check-in **and** at every `Idle` transition*. This cut builds
//! only the second, and that is a recorded deviation:
//!
//! **Check-in keeps committing the arena's final cells and its final pose**,
//! because that state is the match's **evidence**. Doc 16 §1's own acceptance has
//! staff read the wreck with `@unit/show` after the match; the result document
//! points at a tank that is a wreck; the terminal centre carries the outcome until
//! somebody clears it. A check-in that reset the tank in the same breath would
//! destroy the record of the match it was closing, and every reader after it would
//! see eight pristine tanks and a result document nobody could corroborate.
//!
//! So the reset runs on the way **into** `Idle` — [`crate::match_::lobby::reset`],
//! which is the one funnel for that transition since cut F.1 — and at **boot**,
//! where the centre is `Idle` by construction because the match book is RAM. A
//! player's `SIM TEAM` on a terminal centre goes through the same funnel, so two
//! players rematch with no staff and still get every tank at spec.
//!
//! The boot call is made **inside the boot pass** (`crate::boot_matches`), not on
//! a later tick, and that is load-bearing rather than tidy: this pass rewrites
//! every roster tank's record *and its placement*, so a copy of it running a
//! second after the front door opened would undo whatever a staffer had just
//! done — the arena story's own opening is a `@spatial/place` of the two slot-1
//! tanks nose to nose, and a deferred reset put them back 2.4 km apart. Nothing
//! here needs the content latch: the roster document and the definitions are this
//! module's own state, and `mush-sim` and `mush-tactics` are composed before it,
//! so every fact is in hand while `on_world_loaded` is still running.
//!
//! The boot's **pod sweep** does need the latch — a pod is resolved through it,
//! and the content module that fills it is composed after this one — so it runs
//! a step later, and **still inside the boot**: in this module's `on_all_loaded`
//! (16b cut U.4, 16c `B-37`), which the kernel calls once every module's
//! `on_world_loaded` has run. It used to wait for the module's first 1 s tick for
//! want of that hook; now only its hatch lines wait for a tick, because boot
//! output reaches no session — and each is said on the first tick that finds
//! its body's session attached (`crate::match_::MatchBook::hatch_notices`,
//! `crate::match_::HATCH_HOLD_S`).
//!
//! # What it touches, and what it will not
//!
//! Only the **centre's roster tanks**: the objects the `tacsim:arena:<entry>`
//! roster document names, which is the same ladder [`crate::match_::tank_of`]
//! resolves a pod through. Never a unit staff fielded through `@arena/start`
//! (those are somebody's own objects and a silent repair of them would be a
//! surprise), never any other unit in the world, and never a unit carrying a
//! `deployed` marker — the reset only runs when the centre is not live, so a
//! roster tank that is still deployed is a fault, and the honest answer is to skip
//! it, log it and report it rather than to fight the arena for the record.
//!
//! # One plan, not a re-creation
//!
//! Doc 16 says *by one `UnitMutationPlan`*, and that is what this builds: the
//! difference between the unit the world holds and a fresh
//! [`mush_tactics::TacticalUnit::from_spec`] of the very `DefinitionRef` the held
//! unit points at, as `SetCell`, `SetSectionCondition`, `SetComponentCondition`,
//! `GrantCapability` and `RevokeCapability` ops. M32's closed verb set expresses
//! all of it — `GrantCapability` is what restores what a wreck revoked — so no
//! verb was added and the object, its stations, its billets and its roster entry
//! are never touched. A re-creation (`@unit/remove` then `@unit/create`) would
//! have been the other route and was not taken: it destroys and remakes the
//! record a pod's station lives on.
//!
//! Compare first, always. A tank already equal to its spec and already standing on
//! its authored cell is **not written**, so a boot on a clean world writes nothing
//! and a second `@arena/reset` still says nothing changed.
//!
//! # And the type, before the plan (16b cut Q.8)
//!
//! `SIM UNIT` lets a pilot field a biped or a laser tank out of one of these
//! pods, which means a roster tank can be standing here as something its
//! pack did not author — and a plan cannot turn a biped back into a tank, for
//! the reason above. So `reset_one` asks one question before the diff: is the
//! held unit's schema the **authored** one? If it is not, the tank is
//! re-instantiated through [`mush_tactics::module::refit_unit`] — the one seam
//! that can — and the compare-first plan then runs exactly as before and finds
//! nothing to do. `ResetReport::refitted` counts them.
//!
//! This is what makes a chosen type a property of one match rather than of a
//! pod: `Slot::unit_type` goes with the slot at every release, and the tank
//! itself comes back here.

use mush_core::{Ctx, ObjId};
use smol_str::SmolStr;

use crate::definitions::unit::to_tactical_unit;
use crate::match_::centre::CENTRE_ARENA;
use crate::store::TacsimStore;

/// The word a reset's journal entry carries, beside `arena:checkin`'s.
pub const RESET_CAUSE: &str = "arena:reset";

/// **What one reset did** (16b cut F.1).
///
/// Four lists rather than four counts: the caller that reports to a staffer wants
/// to name what it found, and every entry that is not `restored` is something
/// somebody may have to look at.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ResetReport {
    /// The tanks whose unit document was rewritten back to its spec.
    pub restored: Vec<ObjId>,
    /// The tanks put back on their authored cell.
    pub remustered: Vec<ObjId>,
    /// **How many tanks were put back to their authored TYPE** (16b cut Q.8) —
    /// a re-instantiation through [`mush_tactics::module::refit_unit`], not a
    /// plan, and therefore not one of [`ResetReport::restored`]'s rows.
    ///
    /// A count and not a list, because it is a fact about the pass and not
    /// something anybody has to go and look at: a refit that refused is already
    /// in [`ResetReport::failed`] with its reason, and a refit that worked left
    /// the tank at spec like every other row.
    pub refitted: usize,
    /// The roster tanks skipped because they still carry a `deployed` marker.
    pub deployed: Vec<ObjId>,
    /// The roster tanks that could not be reset, with the reason.
    pub failed: Vec<(ObjId, String)>,
}

impl ResetReport {
    /// Whether anything was written.
    #[must_use]
    pub fn changed(&self) -> bool {
        !self.restored.is_empty() || !self.remustered.is_empty() || self.refitted > 0
    }

    /// Whether the reset found nothing to say at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        !self.changed() && self.deployed.is_empty() && self.failed.is_empty()
    }
}

/// **Put every stock tank of the centre's arena back** (doc 16 §3.3).
///
/// See the module head for where this is called from and what it refuses to
/// touch. Idempotent and quiet: a tank already at its spec and already at its
/// authored cell is compared and left alone.
///
/// **16b cut AF.3**: `keep` is the tanks of the matches the boot is resuming.
/// Each is skipped **quietly** — no `warn` line, not listed under
/// [`ResetReport::deployed`] — because it is deployed on purpose and stays so;
/// every other deployed roster tank is still the fault it always was. Every
/// caller but the boot passes an empty set.
pub fn reset_stock_tanks(
    ctx: &mut Ctx<'_>,
    keep: &std::collections::BTreeSet<ObjId>,
) -> ResetReport {
    let mut report = ResetReport::default();
    let Ok(arena) = mush_core::CanonicalId::new(CENTRE_ARENA) else {
        return report;
    };
    // The authored slots, and what this world's applier made of them. Both come
    // out of the module's own state in one borrow, because everything below needs
    // `ctx` mutably.
    let slots: Vec<(SmolStr, ObjId, mush_sim::data::Pose)> = {
        let Some(store) = ctx.w.module_state::<TacsimStore>() else {
            return report;
        };
        let Some(def) = store.definitions().arenas.get(&arena) else {
            return report;
        };
        let Some(roster) = store.arena_roster(&arena) else {
            return report;
        };
        let half = i64::from(def.cell_cm / 2);
        let cell = i64::from(def.cell_cm);
        def.tanks
            .iter()
            .filter_map(|tank| {
                let object = roster.unit(tank.id.as_str())?;
                // The same arithmetic the arena pack's applier does
                // (`crate::content::ArenaApplier::put`), so "the authored slot
                // placement" is literally the placement the applier gave it.
                let pose = mush_sim::data::Pose {
                    x_cm: i64::from(tank.x) * cell + half,
                    y_cm: i64::from(tank.y) * cell + half,
                    z_cm: 0,
                    heading_mdeg: tank.heading_mdeg,
                    posture: mush_sim::data::passive_rules_value(serde_json::json!({})).ok()?,
                };
                Some((SmolStr::new(tank.unit.as_str()), object, pose))
            })
            .collect()
    };
    for (schema, object, authored) in slots {
        if ctx.w.get(object).is_none() || keep.contains(&object) {
            continue;
        }
        // Never a deployed unit. The reset runs on the way into `Idle`, so this
        // cannot happen on a healthy server; when it does, the arena is the
        // single writer for that record and the honest answer is to say so.
        if mush_sim::deployed::is_custodied(ctx.w, object) {
            tracing::warn!(
                unit = %object,
                "tacsim: a roster tank is still deployed at an Idle transition; it is not reset"
            );
            report.deployed.push(object);
            continue;
        }
        match reset_one(ctx, object, &schema) {
            Ok((refitted, rewritten)) => {
                if refitted {
                    report.refitted = report.refitted.saturating_add(1);
                }
                if rewritten {
                    report.restored.push(object);
                }
            }
            Err(why) => {
                tracing::error!(unit = %object, why, "tacsim: a stock tank could not be reset");
                report.failed.push((object, why));
                continue;
            }
        }
        match remuster(ctx, object, &authored) {
            Ok(true) => report.remustered.push(object),
            Ok(false) => {}
            Err(why) => {
                tracing::error!(unit = %object, why, "tacsim: a stock tank could not be re-placed");
                report.failed.push((object, why));
            }
        }
    }
    report
}

/// Put one tank's record back to its spec. Answers
/// `(was it refitted, was a plan written)`.
fn reset_one(ctx: &mut Ctx<'_>, object: ObjId, schema: &SmolStr) -> Result<(bool, bool), String> {
    let Some(held) = mush_tactics::module::unit_of(ctx.w, object).cloned() else {
        // A roster object that is not a tactical unit at all: content the applier
        // never finished, or a staffer's `@unit/remove`. Not this pass's business.
        return Ok((false, false));
    };
    let (spec, binding) = {
        let entry = mush_core::CanonicalId::new(schema.as_str())
            .map_err(|error| format!("`{schema}` is not a canonical id: {error}"))?;
        let catalog = ctx
            .w
            .module_state::<TacsimStore>()
            .map(|store| store.definitions())
            .ok_or_else(|| "the TacSim layer is not composed".to_owned())?;
        let spec = catalog
            .specs
            .get(&entry)
            .cloned()
            .ok_or_else(|| format!("no unit schema `{schema}` is loaded"))?;
        let binding = catalog
            .bindings
            .get(&entry)
            .cloned()
            .ok_or_else(|| format!("unit schema `{schema}` has no binding"))?;
        (spec, binding)
    };
    // **A tank that is not the type its pod authors is put back to it first**
    // (16b cut Q.8). A plan can move a cell and cannot invent one, so a unit
    // built from another schema — a `SIM UNIT` refit, or a version of this one
    // that is no longer loaded — cannot be reset by the diff below at all: every
    // op would name a cell the held unit does not have. Until this cut that was
    // the refusal `re-apply the arena pack`, which named a repair no verb
    // performs; now it is a re-instantiation through the one seam that can make
    // one, and the plan that follows then finds nothing left to do.
    //
    // The write is the reset's own: it is why `SIM UNIT`'s choice does not have
    // to survive a match, and why a half-refitted `SIM START` that refused
    // leaves nothing behind.
    let mut refitted = false;
    let mut held = held;
    if binding.spec.definition != *held.schema() {
        mush_tactics::module::refit_unit(ctx, object, &binding.spec).map_err(|why| {
            format!(
                "its record points at {} and the loaded `{schema}` is {}; it could not be \
                 refitted: {why}",
                held.schema(),
                binding.spec.definition
            )
        })?;
        refitted = true;
        held = mush_tactics::module::unit_of(ctx.w, object)
            .cloned()
            .ok_or_else(|| format!("`{schema}` was refitted and then found no unit"))?;
    }
    // `to_tactical_unit` is `TacticalUnit::from_spec` over the binding's exact
    // registry identity — the same call the arena pack's applier makes when it
    // creates one of these stock tanks, which is what "equal to a fresh instantiation"
    // means here, and which is why this function's own doc comment has named cut
    // K's reset as a caller since C.2 and had none until now.
    let pristine = to_tactical_unit(&spec, &binding, object)
        .map_err(|error| format!("`{schema}` will not instantiate: {error}"))?;
    let Some(plan) = spec_plan(&held, &pristine) else {
        return Ok((refitted, false));
    };
    mush_tactics::module::apply_plan(ctx, object, &plan)?;
    Ok((refitted, true))
}

/// **The plan that turns `held` back into `pristine`** (doc 16 §3.3: *cells to
/// initial values, conditions to `intact`, capabilities restored*), or `None`
/// when the two already agree.
///
/// The mirror of [`crate::match_::returned_plan`], which is the check-in's diff in
/// the other direction — and it carries the one op kind that one deliberately
/// cannot: `GrantCapability`. A wreck is `hull` condition `wreck` plus the
/// revocation of `tacsim.power`, `tactics.mobility` and every `tacsim.weapon.*`
/// (doc 16 §3.3), so a reset that only moved cells and conditions would hand back
/// an intact hull that still could not be started, driven or fired.
///
/// Structure is never touched: a cell, a section or a component the held unit has
/// and the spec does not is left exactly where it is, because a plan cannot remove
/// one and a reset that could restructure a persisted unit is a reset that could
/// restructure it wrongly.
#[must_use]
pub fn spec_plan(
    held: &mush_tactics::TacticalUnit,
    pristine: &mush_tactics::TacticalUnit,
) -> Option<mush_tactics::UnitMutationPlan> {
    let mut ops: Vec<mush_tactics::UnitMutationOp> = Vec::new();
    for (addr, cell) in pristine.cells() {
        if held
            .cell(addr)
            .is_some_and(|before| before.current != cell.current)
        {
            ops.push(mush_tactics::UnitMutationOp::SetCell {
                addr: addr.clone(),
                value: cell.current,
            });
        }
    }
    for (id, section) in pristine.sections() {
        if held
            .section(id)
            .is_some_and(|before| before.condition != section.condition)
        {
            ops.push(mush_tactics::UnitMutationOp::SetSectionCondition {
                section: id.clone(),
                condition: section.condition.clone(),
            });
        }
    }
    for (id, component) in pristine.components() {
        if held
            .component(id)
            .is_some_and(|before| before.condition != component.condition)
        {
            ops.push(mush_tactics::UnitMutationOp::SetComponentCondition {
                component: id.clone(),
                condition: component.condition.clone(),
            });
        }
    }
    for (key, rules) in pristine
        .capabilities()
        .filter_map(|key| pristine.capability(key).map(|rules| (key, rules)))
    {
        if held.capability(key) != Some(rules) {
            ops.push(mush_tactics::UnitMutationOp::GrantCapability {
                key: key.clone(),
                rules: rules.clone(),
            });
        }
    }
    for key in held.capabilities() {
        if pristine.capability(key).is_none() {
            ops.push(mush_tactics::UnitMutationOp::RevokeCapability { key: key.clone() });
        }
    }
    if ops.is_empty() {
        return None;
    }
    match mush_tactics::UnitMutationPlan::new(ops, None, Some(SmolStr::new_static(RESET_CAUSE))) {
        Ok(plan) => Some(plan),
        Err(error) => {
            // M32's `MAX_UNIT_MUTATION_OPERATIONS`. The stock tank is nowhere near
            // it; a unit with enough cells to reach it is left as it stands rather
            // than half-reset, which is the same fail-closed answer a refused
            // check-in gets.
            tracing::error!(
                %error,
                "tacsim: a stock tank's reset would not fit one plan; it is left as it stands"
            );
            None
        }
    }
}

/// Put one tank back on the cell its pack authors. Answers whether it moved.
///
/// The domain is the one the tank is standing in, which is the arena's own
/// passive domain — the applier opened it on the muster room and placed every tank
/// there, and nothing since moves a tank between domains. A tank that is not
/// placed at all is placed back on the roster's own muster domain, which is how a
/// unit whose placement a fault dropped comes home.
fn remuster(
    ctx: &mut Ctx<'_>,
    object: ObjId,
    authored: &mush_sim::data::Pose,
) -> Result<bool, String> {
    let standing = mush_sim::pose_of(ctx.w, object);
    let domain = match standing.as_ref() {
        Some((domain, _)) => *domain,
        None => muster_domain(ctx).ok_or_else(|| "its arena has no muster domain".to_owned())?,
    };
    if let Some((at, held)) = standing.as_ref()
        && *at == domain
        && held.x_cm == authored.x_cm
        && held.y_cm == authored.y_cm
        && held.z_cm == authored.z_cm
        && held.heading_mdeg == authored.heading_mdeg
    {
        return Ok(false);
    }
    // The posture the unit already carries, never the authored one: the arena has
    // no posture and must not be read as having invented one, and the same is true
    // of a reset.
    let pose = mush_sim::data::Pose {
        posture: standing
            .map(|(_, held)| held.posture)
            .unwrap_or_else(|| authored.posture.clone()),
        ..authored.clone()
    };
    mush_sim::place(ctx, object, domain, pose).map_err(|error| error.to_string())?;
    Ok(true)
}

/// The domain the arena's muster room hosts, out of the roster document.
fn muster_domain(ctx: &Ctx<'_>) -> Option<mush_sim::DomainId> {
    let arena = mush_core::CanonicalId::new(CENTRE_ARENA).ok()?;
    let room = ctx
        .w
        .module_state::<TacsimStore>()?
        .arena_roster(&arena)
        .map(|roster| ObjId(roster.room))?;
    ctx.w
        .module_state::<mush_sim::data::SpatialState>()?
        .domain_of_host(room)
        .map(|record| record.id)
}
