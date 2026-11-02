//! **Recovery as exit** (doc 32 cut CF.5; Q-CF-18, Q-CF-36; the rule in doc 32 §8).
//!
//! A unit leaves a running field the way it would leave the surface: up its own carrier's ramp,
//! under its own power, by `+tv/<n> recover` or `+sortie/recover`. Both verbs land here when the
//! unit is under a field's mark, and the exit runs in three steps:
//!
//! 1. **The kernel lets it go** ([`request`]): the host sends `ArenaMsg::Withdraw` with the ramp's
//!    site position, and the arena decides at its next pulse on its own live facts — the unit
//!    within one cell of the ramp and able to return under its own power
//!    (`crate::arena::withdraw`). A refusal is told to whoever asked; the unit fights on.
//! 2. **The host checks it in** ([`on_withdrawn`]): the unit's row goes home through the IC
//!    condition plan (`super::close::demote_units`, the demotion's own path: only the custody
//!    vocabulary, no blanket wreck stamp, never the check-out snapshot), stands at its field pose
//!    beside the ramp, leaves the live record, and its pilot's seat leaves the task. The crew's
//!    state is derived as at any check-in (`super::crew`). The record is owed an `exit`
//!    (`tacsim_field op=exit`, the host's own write): the unit is named on the open promotion.
//! 3. **The recover runs** ([`tick`]): once the record names the exit, the verb that asked is
//!    submitted again as its player — the rig's recover act from beside the ramp, or the DT
//!    recover — and CF.1's refusal of surface work on a promoted site passes that one act for
//!    that one unit (`crate::surface::site_record::SiteRecord::exited`). The field carries on.
//!
//! **A carrier lifting out of a running field** (doc 32 F-6, ruled 2026-10-31; cut CF.7) takes
//! the same three steps with no ramp: a live `+ship/lift` or `+ship/undock` whose leave-behind
//! confirmation names every unit the carrier has out (`crate::port::voyage::undock_cmd`) asks the
//! kernel to remove the carrier outright, the host checks it in with the souls aboard it, and the
//! departure is submitted again as its pilot once the record names the exit. The units it left
//! stay in the field. **A raider's fixture carrier** (doc 36 cut GC.0) lifts the same way, asked
//! by the host as its leader once its hulls are aboard or down (`super::raiders::lift_check`); its
//! continuation takes it off the site's ground.
//!
//! **Across a restart** the check-in, the live record and the exit are each durable, and the owed
//! exit is durable from the check-in's own batch (the live record's `owed_exits`, doc 32 §15): a
//! boot owes it again, so a crash before the K3 write never leaves the unit refused its recover
//! until the field closes. The RAM continuation is not durable: a unit whose exit landed and
//! whose recover had not been submitted stands beside the ramp out of the field, and the player's
//! own recover passes (the record names it).
//! A resumed field drops every unit its live record no longer holds from the snapshot it resumes
//! from (`super::boot`).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, PerceivedRef};
use serde_json::json;

use super::FieldBook;
use crate::arena::damage::UnitReturn;
use crate::arena::withdraw::WithdrawRefusal;
use crate::arena_task::MatchId;

/// **What runs once the unit is out**: the verb that asked, as its player asked it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Then {
    /// `+tv/<n> recover`: the rig's recover act from beside the ramp.
    Rig {
        /// The bay slot the player named.
        slot: u8,
        /// The act's `quiet` flag.
        quiet: bool,
    },
    /// `+sortie/recover`: the DT recover of the slot's unit.
    Sortie {
        /// The operation.
        operation: crate::detachment::sortie::OperationId,
        /// Its roster slot.
        slot: u16,
    },
    /// `+ship/lift` or `+ship/undock` (doc 32 F-6): the carrier's live departure.
    Lift {
        /// The command's arguments, as the pilot typed them.
        raw: String,
        /// `+ship/lift` (else `+ship/undock`).
        lift: bool,
    },
    /// A raider's hull under its leader's WITHDRAW (doc 32 cut CF.6b): aboard its fixture
    /// carrier again (`super::raiders::board`).
    Raider,
    /// A raider's fixture carrier whose hulls are all aboard or down (doc 36 cut GC.0; doc 32
    /// F-6's departure, run by the host for the NPC carrier): off the site's ground
    /// (`super::raiders::lift`).
    RaiderLift,
}

/// **An exit asked for** (RAM).
#[derive(Clone, Debug)]
pub(crate) struct Exiting {
    /// Who asked.
    pub requester: ObjId,
    /// The site.
    pub site: CanonicalId,
    /// What runs once the unit is out.
    pub then: Then,
    /// The kernel let it go and the host checked it in.
    pub out: bool,
}

/// **The exits in flight**, by unit (module state).
#[derive(Clone, Debug, Default)]
pub(crate) struct ExitBook(pub BTreeMap<ObjId, Exiting>);

/// **The `exit` writes the host owes the record**: by site, each unit and who submits it.
#[derive(Clone, Debug, Default)]
pub(crate) struct OwedExits(pub BTreeMap<CanonicalId, BTreeMap<ObjId, ObjId>>);

/// The line a player hears when the arena takes the unit.
pub const EXIT_ASKED: &str = "#{unit} makes for the ramp: it leaves field {ordinal} at the next pulse if it stands beside the ramp and can move under its own power.";
/// The line once it is out.
pub const EXIT_OUT: &str =
    "#{unit} is up the ramp and out of field {ordinal}; its recover into the bay follows.";
/// The line a pilot hears when the arena takes a departing carrier (doc 32 F-6).
pub const LIFT_ASKED: &str = "The {name} breaks off: it leaves field {ordinal} at the next pulse, and its departure files once it is clear. The units it leaves stay in the field.";
/// The line once it is out.
pub const LIFT_OUT: &str = "The {name} is out of field {ordinal}; its departure files now.";

/// **Ask the field holding `unit` to let it go at the ramp standing at `ramp` (cm)** (this
/// module's head, step 1); `None` for a carrier lifting off its pad (doc 32 F-6).
///
/// # Errors
///
/// The sentence for the player: the unit is in no field, or already on its way out, or the
/// field is busy.
pub(crate) fn request(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    ramp: Option<(i64, i64)>,
    then: Then,
) -> Result<(), CmdError> {
    let site = super::field_of_unit(ctx.w, unit)
        .ok_or_else(|| CmdError::msg(WithdrawRefusal::NotHere.sentence()))?;
    let (task, ordinal) = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(&site))
        .filter(|hosted| hosted.live.units.iter().any(|row| row.unit == unit))
        .map(|hosted| (hosted.task, hosted.live.field.ordinal))
        .ok_or_else(|| {
            CmdError::msg("That field is not running yet; recover once it has opened.")
        })?;
    if ctx
        .w
        .module_state::<ExitBook>()
        .is_some_and(|book| book.0.contains_key(&unit))
    {
        return Err(CmdError::msg("That unit is already on its way out."));
    }
    let tasks = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .ok_or_else(|| CmdError::msg("The field host is unavailable."))?;
    tasks
        .offer(task, crate::arena_task::ArenaMsg::Withdraw { unit, ramp })
        .map_err(|_| CmdError::msg("The field is busy; try the recover again."))?;
    let requester = ctx.actor;
    let line = match &then {
        Then::Lift { .. } | Then::RaiderLift => LIFT_ASKED
            .replace("{name}", &crate::port::coop::name_of(ctx.w, unit))
            .replace("{ordinal}", &ordinal.to_string()),
        Then::Rig { .. } | Then::Sortie { .. } | Then::Raider => EXIT_ASKED
            .replace("{unit}", &unit.0.to_string())
            .replace("{ordinal}", &ordinal.to_string()),
    };
    if let Some(book) = ctx.w.module_state_mut::<ExitBook>() {
        book.0.insert(
            unit,
            Exiting {
                requester,
                site,
                then,
                out: false,
            },
        );
    }
    ctx.out.sys_to_actor(requester, "tacsim.field", line);
    Ok(())
}

/// **The arena's answer** (the bridge's branch; this module's head, step 2).
pub(crate) fn on_withdrawn(
    ctx: &mut Ctx<'_>,
    task: MatchId,
    unit: ObjId,
    outcome: Result<Box<UnitReturn>, WithdrawRefusal>,
) {
    let site = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.site_of_task(task).cloned());
    let asked = ctx
        .w
        .module_state::<ExitBook>()
        .and_then(|book| book.0.get(&unit).cloned());
    let back = match outcome {
        Ok(back) => *back,
        Err(refusal) => {
            if let Some(book) = ctx.w.module_state_mut::<ExitBook>() {
                book.0.remove(&unit);
            }
            if let Some(asked) = asked {
                ctx.out.sys_to_actor(
                    asked.requester,
                    "tacsim.field",
                    refusal.sentence().to_owned(),
                );
            }
            return;
        }
    };
    let Some(site) = site else {
        tracing::error!(
            unit = unit.0,
            "tacsim: a unit left a field the host no longer holds; it checks in as its mark stands"
        );
        return;
    };
    let (pilot, aboard, ordinal, promoter) = ctx
        .w
        .module_state::<FieldBook>()
        .and_then(|book| book.hosted(&site))
        .map(|hosted| {
            let row = hosted.live.units.iter().find(|row| row.unit == unit);
            (
                row.and_then(|row| row.pilot.as_ref().map(|(pilot, _)| *pilot)),
                row.map(|row| row.aboard.clone()).unwrap_or_default(),
                hosted.live.field.ordinal,
                hosted.live.parties.iter().find_map(|party| party.leader),
            )
        })
        .unwrap_or((None, Vec::new(), 0, None));
    // The check-in, at the field pose beside the ramp (Q-CF-36): a mobile unit leaves nothing on
    // the record's unit index. A departing carrier (F-6) checks its souls in aboard it, as its
    // demotion would.
    let _ = super::close::demote_units(ctx, &site, &[unit], std::slice::from_ref(&back));
    super::crew::check_in(ctx, &site, unit, pilot);
    super::crew::check_in_aboard(ctx, &site, unit, &aboard);
    if let Some(pilot) = pilot {
        if let Some(registry) = ctx
            .w
            .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
        {
            registry.unbind_pilot(pilot);
        }
        crate::arena_bridge::clear_cockpit(ctx, pilot);
    }
    // The record's `exit`, submitted by whoever asked (else the field's first leader), owed on
    // the live record in this batch, beside the check-in (doc 32 §15, the review's CF-1): a crash
    // before the K3 write lands owes it again at boot.
    let actor = asked
        .as_ref()
        .map(|asked| asked.requester)
        .or(pilot)
        .or(promoter);
    if let Some(actor) = actor
        && let Some(owed) = ctx.w.module_state_mut::<OwedExits>()
    {
        owed.0.entry(site.clone()).or_default().insert(unit, actor);
    }
    let live = ctx.w.module_state_mut::<FieldBook>().and_then(|book| {
        let hosted = book.live.get_mut(&site)?;
        hosted.live.units.retain(|row| row.unit != unit);
        if let Some(actor) = actor {
            hosted.live.owed_exits.retain(|owed| owed.unit != unit);
            hosted
                .live
                .owed_exits
                .push(super::record::OwedExit { unit, actor });
        }
        Some(hosted.live.clone())
    });
    if let Some(live) = live {
        super::record::write_live(ctx, &live);
    }
    if let Some(book) = ctx.w.module_state_mut::<ExitBook>()
        && let Some(asked) = book.0.get_mut(&unit)
    {
        asked.out = true;
    }
    let line = match asked.as_ref().map(|asked| &asked.then) {
        Some(Then::Lift { .. } | Then::RaiderLift) => LIFT_OUT
            .replace("{name}", &crate::port::coop::name_of(ctx.w, unit))
            .replace("{ordinal}", &ordinal.to_string()),
        _ => EXIT_OUT
            .replace("{unit}", &unit.0.to_string())
            .replace("{ordinal}", &ordinal.to_string()),
    };
    let mut told: BTreeSet<ObjId> = BTreeSet::new();
    for who in asked.map(|asked| asked.requester).into_iter().chain(pilot) {
        if told.insert(who) {
            ctx.out.sys_to_actor(who, "tacsim.field", line.clone());
        }
    }
    submit_owed(ctx);
    tracing::info!(site = %site, unit = unit.0, ordinal, "tacsim: a unit left an IC field at its carrier's ramp");
}

/// **Submit every owed `exit`** whose actor has nothing in flight, at most one an actor (the
/// engine stages one authoritative action per actor at a time); answers who submitted.
fn submit_owed(ctx: &mut Ctx<'_>) -> BTreeSet<ObjId> {
    let mut busy = BTreeSet::new();
    let Some(action) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|handles| handles.field.clone())
    else {
        return busy;
    };
    let owed: Vec<(CanonicalId, ObjId, ObjId)> = ctx
        .w
        .module_state::<OwedExits>()
        .map(|owed| {
            owed.0
                .iter()
                .flat_map(|(site, units)| {
                    units
                        .iter()
                        .map(move |(unit, actor)| (site.clone(), *unit, *actor))
                })
                .collect()
        })
        .unwrap_or_default();
    let now = ctx.w.world_ic_now().at;
    for (site, unit, actor) in owed {
        if busy.contains(&actor) || ctx.authoritative_pending(actor) {
            continue;
        }
        let Some(key) = super::provider::field_key(actor, &site, "exit", unit.0, now) else {
            continue;
        };
        let saved = ctx.actor;
        ctx.actor = actor;
        ctx.submit_authoritative(
            action.clone(),
            PerceivedRef::SelfActor,
            Vec::new(),
            json!({ "site": site.to_string(), "op": "exit", "unit": unit.0 }),
            key,
        );
        ctx.actor = saved;
        busy.insert(actor);
    }
    busy
}

/// **The host's tick, for exits** (this module's head, step 3): the owed writes, then each exit
/// the record now names runs its recover as its player.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let sites: BTreeSet<CanonicalId> = ctx
        .w
        .module_state::<OwedExits>()
        .map(|owed| owed.0.keys().cloned().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(
            ctx.w
                .module_state::<ExitBook>()
                .map(|book| {
                    book.0
                        .values()
                        .map(|asked| asked.site.clone())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        )
        .collect();
    let mut ready: Vec<(ObjId, Exiting)> = Vec::new();
    for site in sites {
        let record = super::close::site_record(ctx, &site);
        let landed = |unit: ObjId| {
            record
                .as_ref()
                .is_none_or(|record| record.promoted.is_none() || record.exited(unit))
        };
        if let Some(owed) = ctx.w.module_state_mut::<OwedExits>()
            && let Some(units) = owed.0.get_mut(&site)
        {
            units.retain(|unit, _| !landed(*unit));
            if units.is_empty() {
                owed.0.remove(&site);
            }
        }
        // The live record forgets each landed exit too.
        let live = ctx.w.module_state_mut::<FieldBook>().and_then(|book| {
            let hosted = book.live.get_mut(&site)?;
            let before = hosted.live.owed_exits.len();
            hosted.live.owed_exits.retain(|owed| !landed(owed.unit));
            (hosted.live.owed_exits.len() != before).then(|| hosted.live.clone())
        });
        if let Some(live) = live {
            super::record::write_live(ctx, &live);
        }
        if let Some(book) = ctx.w.module_state::<ExitBook>() {
            ready.extend(
                book.0
                    .iter()
                    .filter(|(unit, asked)| asked.site == site && asked.out && landed(**unit))
                    .map(|(unit, asked)| (*unit, asked.clone())),
            );
        }
    }
    // The owed writes after the prune, so a landed exit is never written twice; then each
    // recover, one an actor a pass, behind whatever that actor still has in flight.
    let mut busy = submit_owed(ctx);
    for (unit, asked) in ready {
        if busy.contains(&asked.requester) || ctx.authoritative_pending(asked.requester) {
            continue;
        }
        busy.insert(asked.requester);
        if let Some(book) = ctx.w.module_state_mut::<ExitBook>() {
            book.0.remove(&unit);
        }
        let saved = ctx.actor;
        ctx.actor = asked.requester;
        let ran = continue_recover(ctx, unit, &asked.then, &asked.site);
        ctx.actor = saved;
        if let Err(error) = ran {
            let what = match asked.then {
                Then::Lift { .. } => "departure did not file",
                Then::RaiderLift => "lift did not finish",
                Then::Rig { .. } | Then::Sortie { .. } | Then::Raider => "recover did not start",
            };
            ctx.out.sys_to_actor(
                asked.requester,
                "tacsim.field",
                format!("#{} is out of the field, but its {what}: {error}", unit.0),
            );
        }
    }
}

/// The recover the player asked for, as the player (`ctx.actor`).
fn continue_recover(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    then: &Then,
    site: &CanonicalId,
) -> Result<(), String> {
    match then {
        Then::Rig { slot, quiet } => {
            let rig = crate::port::tv::select_rig(
                ctx,
                Some(*slot),
                crate::surface::excursion::ActKind::Recover,
            )?;
            if rig.tv != unit {
                return Err("that slot holds another rig now".to_owned());
            }
            crate::surface::act::act_cmd(
                ctx,
                &rig,
                crate::surface::excursion::ActKind::Recover,
                *quiet,
                true,
                None,
            )
            .map_err(|error| error.to_string())
        }
        Then::Sortie { operation, slot } => {
            crate::detachment::sortie::recover_after_exit(ctx, *operation, *slot, unit)
        }
        Then::Lift { raw, lift } => {
            crate::port::voyage::undock_as(ctx, raw, *lift).map_err(|error| error.to_string())
        }
        Then::Raider => super::raiders::board(ctx, unit, site),
        Then::RaiderLift => super::raiders::lift(ctx, unit, site),
    }
}

/// **The ramp a rig's carrier stands with on its pad**, as a site position (cm).
#[must_use]
pub(crate) fn rig_ramp(w: &mush_core::World, hull: ObjId) -> Option<(i64, i64)> {
    let view = crate::port::ship::hull_view(w, hull)?;
    let berth = crate::surface::act::site_berth(w, &view).ok()?;
    Some((berth.ramp.x_cm, berth.ramp.y_cm))
}
