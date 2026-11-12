//! **A field unit whose player is not at its controls** (doc 44 cut MW.7; Q-44-16 to Q-44-19,
//! Q-44-22, Q-44-35 to Q-44-38, Q-44-42, Q-44-43, Q-44-46; doc 35 §11.4, cuts `FB.1` to `FB.3`).
//!
//! Every unit always has a mind, and presence decides who holds the controls (Q-44-22). In a
//! field the player who counts is the unit's seated pilot (Q-44-35), conscious and at liberty
//! (Q-44-43). This module is the field host's half of that sentence:
//!
//! 1. **The record** ([`Away`], on the unit's row of the field's live record, so a restart
//!    keeps it). It is opened when the pilot's link is lost and carries the wall second it was
//!    lost at: **one clock** (`FB.1`), cleared when the pilot is back. While it runs the seat is
//!    `Linkdead` as before: the unit keeps its continuous orders and fires nothing.
//! 2. **The hand-over** ([`hand_over`]). Five wall minutes after the link was lost
//!    ([`FIELD_LINKDEAD_GRACE_WALL_S`]), at once on a clean QUIT (Q-44-46) and at once when the
//!    pilot is incapacitated while connected (Q-44-43), the unit passes to its mind as an absent
//!    player's unit: the seat is released, and the arena is told the unit is flown for an
//!    absent player (`crate::arena::absent`), which lays the go-home order every mind reads and
//!    binds whatever mind flies it to the absent player's rule. [`hand_over`] is the one door,
//!    and link loss is only one way through it: admission of a pilot who is not there (doc 45
//!    AP.5) and a posture (AP.3) call the same function with a [`Why`] of their own.
//! 3. **Going home** ([`tick`]). Once a world second the host asks the field to let the unit
//!    out at its carrier's ramp (`super::home::ask_out`), as it does for an NPC under a
//!    WITHDRAW, and the kernel decides on the unit's own pose. No mind may board (D-UM-11), so
//!    the host does: out, the unit's recover runs as its pilot, the rig's recover act or the
//!    DT recover, and is tried again every [`RECOVER_RETRY_WALL_S`] until the unit is off the
//!    ground. A unit whose party has no carrier on the site has no ramp to be asked out at and
//!    holds where it stands (Q-44-42).
//! 4. **The way back** ([`take_back`]). A pilot who reconnects, at any time while the field
//!    runs and the unit is in it, is bound and seated again, and the arena is told the player
//!    is back: the mind's pending think and its order are dropped by the seat's own `Live`
//!    control, and a ramp exit the host asked for and the kernel has not yet applied is
//!    recalled (Q-44-37). What the mind already did stands: a unit already let out at the ramp
//!    is out, and its pilot is told where it is.
//! 5. **A restart** ([`at_resume`]). A unit the mind held is the mind's again. A seated pilot
//!    who is not connected is not handed the controls: the seat is `Linkdead` and the clock
//!    runs from the boot, or from the loss the record already carries.
//!
//! **The escort that leaves** (doc 35 §11.4's open point). A unit that goes home leaves
//! whoever it was with. It goes, and they are told: the leader of its own party and the
//! leaders of the parties that hold it a friend hear one line ([`GONE_HOME`]), queued for one
//! who is not connected.
//!
//! **The simulator is unchanged** (Q-44-19): nothing here acts on a unit no field hosts.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, ObjId, World};
use serde::{Deserialize, Serialize};

use super::FieldBook;
use super::exit::Then;
use crate::arena::intents::ControlState;
use crate::arena_task::{ArenaMsg, MatchId};
use crate::port_tick::{NoticeLevel, notify_budget, on_watch};

/// **How long a field unit waits for its pilot's link to come back**, wall seconds (the
/// owner's ruling, 2026-11-11: five minutes, in the field and in space alike; space's is
/// `crate::space::SPACE_LINKDEAD_GRACE_WALL_S`).
pub const FIELD_LINKDEAD_GRACE_WALL_S: i64 = 300;
/// **How often the host tries a home unit's recover again**, wall seconds.
pub const RECOVER_RETRY_WALL_S: i64 = 10;

/// The pilot, when the mind takes a unit whose pilot is still connected: `{unit}`.
pub const MIND_HAS_IT: &str = "You cannot fly #{unit}: its mind has the controls and is taking it home. It fires only on a unit that fires on it.";
/// The pilot, queued, when the mind takes the unit of a pilot who is away: `{unit}`.
pub const TAKEN_HOME: &str = "While you were away #{unit} passed to its mind, which makes for its carrier's ramp and fires only on a unit that fires on it.";
/// The pilot, back in the seat: `{unit}`, `{station}`.
pub const BACK_IN_SEAT: &str = "You are back at the controls of #{unit}; its mind stands down and what it had pending is dropped. +focus #{unit}/{station} takes your cockpit back.";
/// The pilot, back after the unit left the field: `{unit}`.
pub const WENT_HOME: &str =
    "While you were away its mind took #{unit} out of the field at its carrier's ramp.";
/// The pilot, back but not fit to fly: `{unit}`.
pub const NOT_FIT: &str = "You are in no state to fly #{unit}: its mind keeps the controls.";
/// Those the unit was with, when it goes home: `{unit}`, `{why}`.
pub const GONE_HOME: &str =
    "#{unit} breaks off for its carrier's ramp under its mind: {why}. It will not fire first.";

/// **Why a unit's player is not at its controls.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Why {
    /// The pilot's link was lost.
    LinkLost,
    /// The pilot quit.
    Quit,
    /// The pilot is incapacitated or held (Q-44-43).
    Incapacitated,
}

impl Why {
    /// The reason as the clause [`GONE_HOME`] carries.
    #[must_use]
    pub const fn clause(self) -> &'static str {
        match self {
            Why::LinkLost => "its pilot's link is lost",
            Why::Quit => "its pilot has left",
            Why::Incapacitated => "its pilot is down",
        }
    }
}

/// **A field unit's presence record** (this module's head, 1): on [`super::FieldUnit`], written
/// only while the unit's player is not at its controls.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Away {
    /// Why.
    pub why: Why,
    /// The wall second (`World::now`) the player was last at the controls.
    pub since_wall_s: i64,
    /// The unit's mind holds the controls: the hand-over is done.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub handed: bool,
}

/// **A unit the mind took out at its ramp, whose recover the host still owes** (RAM).
#[derive(Clone, Debug)]
struct Homebound {
    /// The site it left.
    site: CanonicalId,
    /// Its pilot, as whom the recover runs.
    pilot: ObjId,
    /// The recover.
    then: Then,
    /// The wall second before which it is not tried again.
    retry_at_wall_s: i64,
}

/// **What this boot's host has done for its absent players' units** (RAM, module state): the
/// units whose task has been told, so a restart tells the resumed task again, and the units
/// out at a ramp whose recover is owed.
#[derive(Clone, Debug, Default)]
pub(crate) struct Minding {
    told: BTreeMap<ObjId, MatchId>,
    homebound: BTreeMap<ObjId, Homebound>,
}

/// One absent player's unit in a running field.
#[derive(Clone, Debug)]
struct Row {
    site: CanonicalId,
    task: MatchId,
    unit: ObjId,
    pilot: ObjId,
    station: smol_str::SmolStr,
    away: Option<Away>,
}

/// Every piloted unit of every running field, with its presence record.
fn rows(w: &World) -> Vec<Row> {
    let Some(book) = w.module_state::<FieldBook>() else {
        return Vec::new();
    };
    book.live
        .iter()
        .flat_map(|(site, hosted)| {
            hosted.live.units.iter().filter_map(move |row| {
                let (pilot, station) = row.pilot.clone()?;
                Some(Row {
                    site: site.clone(),
                    task: hosted.task,
                    unit: row.unit,
                    pilot,
                    station,
                    away: row.away,
                })
            })
        })
        .collect()
}

/// `unit`'s row, when a running field holds it with a pilot.
fn row_of(w: &World, unit: ObjId) -> Option<Row> {
    rows(w).into_iter().find(|row| row.unit == unit)
}

/// Write `away` on `unit`'s row of `site`'s live record.
fn set_away(ctx: &mut Ctx<'_>, site: &CanonicalId, unit: ObjId, away: Option<Away>) {
    let live = ctx.w.module_state_mut::<FieldBook>().and_then(|book| {
        let hosted = book.live.get_mut(site)?;
        let row = hosted.live.units.iter_mut().find(|row| row.unit == unit)?;
        if row.away == away {
            return None;
        }
        row.away = away;
        Some(hosted.live.clone())
    });
    if let Some(live) = live {
        super::record::write_live(ctx, &live);
    }
}

/// **Whether `who` may hold a unit's controls** (Q-44-43): conscious and at liberty. A
/// character the field engine never touched has no crew record and is fit.
#[must_use]
pub(crate) fn fit(w: &World, who: ObjId) -> bool {
    super::crew::crew_of(w, who).is_none_or(|record| {
        record.condition != super::crew::CrewCondition::Incapacitated && record.hold().is_none()
    })
}

/// **Whether a mind flies `unit` for an absent player** in a running field (the hand-over is
/// done and the player is not back).
#[must_use]
pub fn minded(w: &World, unit: ObjId) -> bool {
    away_of(w, unit).is_some_and(|away| away.handed)
}

/// **`unit`'s presence record**, when a running field holds it and its player is not at its
/// controls.
#[must_use]
pub fn away_of(w: &World, unit: ObjId) -> Option<Away> {
    row_of(w, unit).and_then(|row| row.away)
}

/// **The pilot's link was lost** (the seat's `Linkdead` hook): the clock starts, once.
pub(crate) fn on_link_lost(ctx: &mut Ctx<'_>, unit: ObjId, pilot: ObjId) {
    let Some(row) = row_of(ctx.w, unit).filter(|row| row.pilot == pilot) else {
        return;
    };
    if row.away.is_some() || super::orders::is_npc(ctx.w, pilot) {
        return;
    }
    let away = Away {
        why: Why::LinkLost,
        since_wall_s: ctx.w.now(),
        handed: false,
    };
    set_away(ctx, &row.site, unit, Some(away));
}

/// **The pilot's session ended** (the seat's `Disconnected` hook): `true` when a field holds
/// the unit, and the seat is then this module's to release. A session that ends with the clock
/// already running was a lost link whose server grace ran out: the field's own clock decides
/// (one clock, `FB.1`). One that ends with no clock running is a clean QUIT, and hands over at
/// once (Q-44-46).
pub(crate) fn on_session_gone(ctx: &mut Ctx<'_>, unit: ObjId, pilot: ObjId) -> bool {
    let Some(row) = row_of(ctx.w, unit).filter(|row| row.pilot == pilot) else {
        return false;
    };
    if row.away.is_none() {
        hand_over(ctx, unit, pilot, Why::Quit);
    }
    true
}

/// **The pilot is here again** (the seat's `Connected` and `Revived` hooks, and [`tick`]):
/// `true` when a field unit of theirs was waiting for them. Inside the grace the record is
/// cleared and the seat's own `Live` control does the rest; past it, [`take_back`].
pub(crate) fn on_present(ctx: &mut Ctx<'_>, pilot: ObjId) -> bool {
    let waiting: Vec<Row> = rows(ctx.w)
        .into_iter()
        .filter(|row| row.pilot == pilot && row.away.is_some())
        .collect();
    let mut any = false;
    for row in waiting {
        any = true;
        if row.away.is_some_and(|away| away.handed) {
            take_back(ctx, &row);
        } else {
            set_away(ctx, &row.site, row.unit, None);
            seat(ctx, &row);
        }
    }
    // A unit the mind already took out at its ramp is the pilot's again where it stands.
    let home: Vec<ObjId> = ctx
        .w
        .module_state::<Minding>()
        .map(|minding| {
            minding
                .homebound
                .iter()
                .filter(|(_, owed)| owed.pilot == pilot)
                .map(|(unit, _)| *unit)
                .collect()
        })
        .unwrap_or_default();
    for unit in home {
        any = true;
        if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
            minding.homebound.remove(&unit);
        }
        ctx.out.sys_to_actor(
            pilot,
            "tacsim.field",
            WENT_HOME.replace("{unit}", &unit.0.to_string()),
        );
    }
    any
}

/// Put `row`'s pilot at its controls: bound to the task and `Live` on every channel.
fn seat(ctx: &mut Ctx<'_>, row: &Row) {
    if let Some(registry) = ctx
        .w
        .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
    {
        registry.unbind_watcher(row.pilot);
        registry.bind_pilot(row.pilot, row.task, row.unit);
    }
    crate::arena_bridge::reseat(ctx, row.unit, row.pilot);
}

/// **The hand-over** (this module's head, 2): `unit` passes to its mind as an absent player's
/// unit. The one door: link loss, a quit and an incapacitated pilot come through it today, and
/// admission of an absent pilot and a posture will. `false` when no running field holds the
/// unit with that pilot, or its mind already has it.
pub(crate) fn hand_over(ctx: &mut Ctx<'_>, unit: ObjId, pilot: ObjId, why: Why) -> bool {
    let Some(row) = row_of(ctx.w, unit).filter(|row| row.pilot == pilot) else {
        return false;
    };
    if row.away.is_some_and(|away| away.handed) || super::orders::is_npc(ctx.w, pilot) {
        return false;
    }
    let away = Away {
        why,
        since_wall_s: row
            .away
            .map_or_else(|| ctx.w.now(), |away| away.since_wall_s),
        handed: true,
    };
    set_away(ctx, &row.site, unit, Some(away));
    tell_arena(ctx, &row);
    // The pilot: said to one who is connected and down, queued for one who is away.
    let line = if on_watch(ctx.w, pilot) {
        MIND_HAS_IT
    } else {
        TAKEN_HOME
    };
    notify_budget(
        ctx,
        pilot,
        &line.replace("{unit}", &unit.0.to_string()),
        NoticeLevel::Trip,
    );
    // Those it was with (this module's head, "the escort that leaves").
    let line = GONE_HOME
        .replace("{unit}", &unit.0.to_string())
        .replace("{why}", why.clause());
    for lead in leads(ctx.w, &row) {
        notify_budget(ctx, lead, &line, NoticeLevel::Trip);
    }
    tracing::info!(site = %row.site, unit = unit.0, pilot = pilot.0, ?why, "tacsim: a field unit passed to its mind for an absent player");
    true
}

/// **Who is told a unit has gone home**: its own party's leader and the leader of every party
/// that stands a friend of its party, less its own pilot.
fn leads(w: &World, row: &Row) -> BTreeSet<ObjId> {
    let Some(live) = super::live_on(w, &row.site) else {
        return BTreeSet::new();
    };
    let Some(own) = live.party_of(row.unit) else {
        return BTreeSet::new();
    };
    let friends = super::parties::friends_of(w, &live.parties);
    live.parties
        .iter()
        .filter(|party| {
            party.party == own
                || friends
                    .iter()
                    .any(|(a, b)| (*a, *b) == (own, party.party) || (*b, *a) == (own, party.party))
        })
        .filter_map(|party| party.leader)
        .filter(|lead| *lead != row.pilot && !super::orders::is_npc(w, *lead))
        .collect()
}

/// **Tell the arena `row`'s unit is flown for an absent player**: the host's word first, so
/// the unit is bound by the absent player's rule from the pulse that hands it over, then the
/// seat's release. Nothing when the field will not take the word now; [`tick`] offers again.
fn tell_arena(ctx: &mut Ctx<'_>, row: &Row) {
    let Some(tasks) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
    else {
        return;
    };
    if tasks
        .offer(
            row.task,
            ArenaMsg::Absent {
                unit: row.unit,
                absent: true,
            },
        )
        .is_err()
    {
        return;
    }
    crate::arena_bridge::control_all_channels(ctx, row.unit, ControlState::Released, row.pilot);
    if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
        minding.told.insert(row.unit, row.task);
    }
}

/// **The way back** (this module's head, 4): `row`'s pilot takes the controls from the mind.
/// `false` when they may not hold them (Q-44-43), no longer sit in the unit, or the field will
/// not take the word now ([`tick`] tries again).
fn take_back(ctx: &mut Ctx<'_>, row: &Row) -> bool {
    if !fit(ctx.w, row.pilot) {
        ctx.out.sys_to_actor(
            row.pilot,
            "tacsim.field",
            NOT_FIT.replace("{unit}", &row.unit.0.to_string()),
        );
        return false;
    }
    if super::parties::seated(ctx.w, row.unit).is_none_or(|(sat, _)| sat != row.pilot) {
        return false;
    }
    let Some(tasks) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
    else {
        return false;
    };
    // The player is back: the arena lifts the rule and recalls a ramp exit it has not applied.
    if tasks
        .offer(
            row.task,
            ArenaMsg::Absent {
                unit: row.unit,
                absent: false,
            },
        )
        .is_err()
    {
        return false;
    }
    set_away(ctx, &row.site, row.unit, None);
    if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
        minding.told.remove(&row.unit);
    }
    seat(ctx, row);
    ctx.out.sys_to_actor(
        row.pilot,
        "tacsim.field",
        BACK_IN_SEAT
            .replace("{unit}", &row.unit.0.to_string())
            .replace("{station}", &row.station),
    );
    tracing::info!(site = %row.site, unit = row.unit.0, pilot = row.pilot.0, "tacsim: a returning pilot took a field unit back from its mind");
    true
}

/// **A resumed field's seats** (this module's head, 5; `super::boot`): `true` when `pilot`
/// was handed `unit`'s controls again, which only a connected pilot is. A unit its mind held is
/// left to [`tick`], which tells the resumed task. A seated pilot who is not connected is bound
/// for their return, the seat `Linkdead`, the clock running from the boot unless the record
/// already carries an earlier loss.
pub(crate) fn at_resume(ctx: &mut Ctx<'_>, task: MatchId, unit: ObjId, pilot: ObjId) -> bool {
    let Some(row) = row_of(ctx.w, unit).filter(|row| row.pilot == pilot && row.task == task) else {
        return false;
    };
    if row.away.is_some_and(|away| away.handed) {
        return false;
    }
    if let Some(registry) = ctx
        .w
        .module_state_mut::<crate::arena_bridge::ArenaRegistry>()
    {
        registry.bind_pilot(pilot, task, unit);
    }
    if on_watch(ctx.w, pilot) && fit(ctx.w, pilot) {
        set_away(ctx, &row.site, unit, None);
        crate::arena_bridge::reseat(ctx, unit, pilot);
        return true;
    }
    if row.away.is_none() {
        let away = Away {
            why: Why::LinkLost,
            since_wall_s: ctx.w.now(),
            handed: false,
        };
        set_away(ctx, &row.site, unit, Some(away));
    }
    crate::arena_bridge::control_all_channels(ctx, unit, ControlState::Linkdead, pilot);
    false
}

/// **The kernel let an absent player's unit out at its ramp** (`super::exit::on_withdrawn`,
/// called before the row leaves the live record): the recover is the host's to run and to try
/// again.
pub(crate) fn on_out(ctx: &mut Ctx<'_>, site: &CanonicalId, unit: ObjId, then: Option<&Then>) {
    let Some(row) = row_of(ctx.w, unit).filter(|row| row.away.is_some_and(|away| away.handed))
    else {
        return;
    };
    let Some(then) = then.cloned() else {
        return;
    };
    let now = ctx.w.now();
    if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
        minding.told.remove(&unit);
        minding.homebound.insert(
            unit,
            Homebound {
                site: site.clone(),
                pilot: row.pilot,
                then,
                retry_at_wall_s: now.saturating_add(RECOVER_RETRY_WALL_S),
            },
        );
    }
}

/// **Whether the host owes `unit` its recover** ([`on_out`]): its refusals are tried again,
/// never said.
#[must_use]
pub(crate) fn homebound(w: &World, unit: ObjId) -> bool {
    w.module_state::<Minding>()
        .is_some_and(|minding| minding.homebound.contains_key(&unit))
}

/// The recover a unit of a running field goes home by: the DT recover of a roster's unit, else
/// the rig's own recover act into the bay slot it launched from.
fn recover_of(ctx: &Ctx<'_>, unit: ObjId) -> Option<Then> {
    if let Some((operation, slot)) = crate::detachment::sortie::field_flight(ctx, unit) {
        return Some(Then::Sortie { operation, slot });
    }
    let slot = super::parties::rig_record(ctx, unit)?.excursion?.slot;
    Some(Then::Rig { slot, quiet: true })
}

/// **The host's pass for absent players' units** (this module's head; `super::home::tick`
/// calls it once a world second).
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let now = ctx.w.now();
    for row in rows(ctx.w) {
        let Some(away) = row.away else {
            continue;
        };
        // A pilot who is here and no hook said so (a restart's re-attached connection).
        if on_watch(ctx.w, row.pilot) && fit(ctx.w, row.pilot) {
            if away.handed {
                take_back(ctx, &row);
            } else {
                set_away(ctx, &row.site, row.unit, None);
                seat(ctx, &row);
            }
            continue;
        }
        if !away.handed {
            if away.why == Why::LinkLost
                && now.saturating_sub(away.since_wall_s) >= FIELD_LINKDEAD_GRACE_WALL_S
            {
                hand_over(ctx, row.unit, row.pilot, Why::LinkLost);
            }
            continue;
        }
        // The mind has it: the task is told (again after a restart, or when it was busy), and
        // the unit is asked out at its ramp until the kernel lets it go there.
        let told = ctx
            .w
            .module_state::<Minding>()
            .is_some_and(|minding| minding.told.get(&row.unit) == Some(&row.task));
        if !told {
            tell_arena(ctx, &row);
            continue;
        }
        if let Some(then) = recover_of(ctx, row.unit) {
            super::home::ask_out(ctx, row.unit, row.pilot, then);
        }
    }
    retry_recovers(ctx, now);
    // Forget what no running field holds any more.
    let stale: Vec<ObjId> = ctx
        .w
        .module_state::<Minding>()
        .map(|minding| minding.told.keys().copied().collect())
        .unwrap_or_default();
    for unit in stale {
        if super::field_of_unit(ctx.w, unit).is_none()
            && let Some(minding) = ctx.w.module_state_mut::<Minding>()
        {
            minding.told.remove(&unit);
        }
    }
}

/// **A recover that was refused is tried again** (this module's head, 3): each unit out at a
/// ramp is recovered as its pilot until it is off the site's ground.
fn retry_recovers(ctx: &mut Ctx<'_>, now: i64) {
    let owed: Vec<(ObjId, Homebound)> = ctx
        .w
        .module_state::<Minding>()
        .map(|minding| {
            minding
                .homebound
                .iter()
                .map(|(unit, owed)| (*unit, owed.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (unit, owed) in owed {
        // Aboard: off the ground. Gone: nothing left to recover.
        let aboard = mush_sim::pose_of(ctx.w, unit).is_none();
        if aboard || ctx.w.get(unit).is_none() {
            if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
                minding.homebound.remove(&unit);
            }
            if aboard {
                tracing::info!(site = %owed.site, unit = unit.0, "tacsim: an absent player's unit is aboard its carrier");
            }
            continue;
        }
        // Still on its way out (the exit's own first try), or not yet due.
        if now < owed.retry_at_wall_s
            || super::exit::leaving(ctx.w, &owed.site, unit)
            || ctx.authoritative_pending(owed.pilot)
        {
            continue;
        }
        if let Some(minding) = ctx.w.module_state_mut::<Minding>()
            && let Some(row) = minding.homebound.get_mut(&unit)
        {
            row.retry_at_wall_s = now.saturating_add(RECOVER_RETRY_WALL_S);
        }
        let saved = ctx.actor;
        ctx.actor = owed.pilot;
        let ran = super::exit::continue_recover(ctx, unit, &owed.then, &owed.site);
        ctx.actor = saved;
        if let Err(error) = ran {
            tracing::debug!(site = %owed.site, unit = unit.0, %error, "tacsim: an absent player's unit is not aboard yet; its recover is tried again");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The record is written only while it says something, and reads back.
    #[test]
    fn mw7_the_presence_record_round_trips() {
        let away = Away {
            why: Why::LinkLost,
            since_wall_s: 1_700,
            handed: false,
        };
        let text = serde_json::to_string(&away).expect("it encodes");
        assert_eq!(text, r#"{"why":"link_lost","since_wall_s":1700}"#);
        assert_eq!(serde_json::from_str::<Away>(&text).ok(), Some(away));
        let handed = Away {
            why: Why::Incapacitated,
            since_wall_s: 9,
            handed: true,
        };
        let text = serde_json::to_string(&handed).expect("it encodes");
        assert!(text.contains(r#""handed":true"#), "{text}");
        assert_eq!(serde_json::from_str::<Away>(&text).ok(), Some(handed));
    }
}
