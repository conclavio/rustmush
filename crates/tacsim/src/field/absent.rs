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
//! 2. **The hand-over** (`hand_over`). Five wall minutes after the link was lost
//!    ([`FIELD_LINKDEAD_GRACE_WALL_S`]), at once on a clean QUIT (Q-44-46) and at once when the
//!    pilot is incapacitated while connected (Q-44-43), the unit passes to its mind as an absent
//!    player's unit: the seat is released, and the arena is told the unit is flown for an
//!    absent player (`crate::arena::absent`), which lays the go-home order every mind reads and
//!    binds whatever mind flies it to the absent player's rule. `hand_over` is the one door,
//!    and link loss is only one way through it. **Admission** comes through it too (doc 45 cut
//!    AP.5; Q-45-1, Q-45-2; `super::admit::seat_absent`): a field that takes in a unit whose
//!    seated pilot is not connected, or not fit, hands it over in the same world turn, and one
//!    whose pilot's link was lost less than five minutes before keeps the seat `Linkdead` for
//!    what is left of them (`admitted_link_lost`).
//! 3. **Going home, or staying** (`tick`; doc 45 unit AP.6, Q-45-7). The unit's posture
//!    decides, and the kernel reads it (`crate::arena::absent`, "flying a posture"). Once a
//!    world second the host asks the field to let the unit out at its carrier's ramp
//!    (`super::home::ask_out`), as it does for an NPC under a WITHDRAW, and the kernel decides
//!    on the unit's own pose and on the posture it is flown on: a unit on `RETURN` is let go
//!    once it stands beside the ramp, one on `GUARD` is kept until it can no longer defend
//!    itself, and one on `HOLD` is never asked. No mind may board (D-UM-11), so the host
//!    does: out, the unit's recover runs as its pilot, the rig's recover act or the DT
//!    recover, and is tried again every [`RECOVER_RETRY_WALL_S`] until the unit is off the
//!    ground. A unit whose party has no carrier on the site has no ramp to be asked out at and
//!    holds where it stands (Q-44-42).
//! 4. **The way back** (`take_back`). A pilot who reconnects, at any time while the field
//!    runs and the unit is in it, is bound and seated again, and the arena is told the player
//!    is back: the mind's pending think and its order are dropped by the seat's own `Live`
//!    control, and a ramp exit the host asked for and the kernel has not yet applied is
//!    recalled (Q-44-37). What the mind already did stands: a unit already let out at the ramp
//!    is out, and its pilot is told where it is.
//! 5. **A restart** (`at_resume`). A unit the mind held is the mind's again. A seated pilot
//!    who is not connected is not handed the controls: the seat is `Linkdead` and the clock
//!    runs from the boot, or from the loss the record already carries.
//!
//! **The escort that leaves** (doc 35 §11.4's open point). A unit that goes home leaves
//! whoever it was with. It goes, and they are told: the leader of its own party and the
//! leaders of the parties that hold it a friend hear one line ([`GONE_HOME`]), queued for one
//! who is not connected. For a unit left on `GUARD` or `HOLD` they hear the word it is
//! flown on instead ([`STAYS_ON`]).
//!
//! **A unit nobody sits in** (doc 45 unit AP.7; Q-45-1, Q-45-10, Q-45-11): a rig worked from
//! its carrier or left on a fill plan, and a carrier on its pad. In a field the player who
//! counts is the seated pilot, and such a unit has none, so it is an absent player's unit
//! from the turn the field takes it in, whether or not its operator is connected
//! (`unattended`). Its player is whoever `super::posture::player_of` names: the operator
//! who launched the rig, the carrier's commander. A unit that is nobody's (a raider's fixture
//! carrier, a hull a hired crew launched) is left as it was. There is no seat to release and
//! none to take back, so the record is written handed, the kernel is told, and the rest is
//! the pass above:
//!
//! - **a rig** flies its posture. On `RETURN` its mind drives it to its carrier's ramp, the
//!   kernel lets it out and the host runs its recover as its operator, into the bay slot it
//!   launched from. The surface ends the plan it was working once it is aboard
//!   (`crate::surface::plan::PlanEnd::Field`). On `HOLD` it stays, and its plan, which the
//!   field paused, runs on when the site demotes. A rig that carries nothing to fire cannot
//!   guard: set to `GUARD` it is flown as `HOLD` (`crate::arena::absent`).
//! - **a carrier** is home where it stands. The host tells its arena `HOLD` for a carrier
//!   whose record says `RETURN` (`super::posture`), so its mind stays, turns to and answers
//!   only a unit that has fired on it, and this pass never asks it out: **no mind lifts a
//!   carrier**. A lift leaves its rigs behind, and that stays the live leave-behind
//!   confirmation alone (doc 27 Q-H10; `super::exit`, `Then::Lift`). Its rigs come aboard
//!   by their own recovers, above.
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
/// **How many times it tries** before it leaves the unit beside the ramp for its pilot: five
/// minutes of tries.
pub const RECOVER_TRIES: u32 = 30;

/// The pilot, when the mind takes a unit whose pilot is still connected: `{unit}`.
pub const MIND_HAS_IT: &str = "You cannot fly #{unit}: its mind has the controls and is taking it home. It fires only on a unit that fires on it.";
/// The pilot, queued, when the mind takes the unit of a pilot who is away: `{unit}`.
pub const TAKEN_HOME: &str = "While you were away #{unit} passed to its mind, which makes for its carrier's ramp and fires only on a unit that fires on it.";
/// The pilot, back in the seat: `{unit}`, `{station}`.
pub const BACK_IN_SEAT: &str = "You are back at the controls of #{unit}; its mind stands down and what it had pending is dropped. +focus #{unit}/{station} takes your cockpit back.";
/// The pilot, back after the unit left the field: `{unit}`.
pub const WENT_HOME: &str =
    "While you were away its mind took #{unit} out of the field at its carrier's ramp.";
/// The pilot, queued, when the host gives up putting the unit aboard: `{unit}`.
pub const LEFT_AT_RAMP: &str = "While you were away its mind took #{unit} out of the field, but it could not be put aboard: it stands beside your carrier's ramp. Recover it yourself.";
/// The pilot, back but not fit to fly: `{unit}`.
pub const NOT_FIT: &str = "You are in no state to fly #{unit}: its mind keeps the controls.";
/// The pilot, queued, when a field opened around a unit they sat in while they were away (doc 45
/// cut AP.5): `{unit}`.
pub const ADMITTED_AWAY: &str = "A field opened around #{unit} while you were away: its mind has the controls, makes for its carrier's ramp and fires only on a unit that fires on it.";
/// Those the unit was with, when it goes home: `{unit}`, `{why}`.
pub const GONE_HOME: &str =
    "#{unit} breaks off for its carrier's ramp under its mind: {why}. It will not fire first.";
/// [`MIND_HAS_IT`] for a unit left on `GUARD` or `HOLD` (doc 45 unit AP.6): `{unit}`,
/// `{posture}`.
pub const MIND_HAS_IT_ON: &str = "You cannot fly #{unit}: its mind has the controls and keeps it on {posture}. It will not fire first.";
/// [`TAKEN_HOME`] for a unit left on `GUARD` or `HOLD`: `{unit}`, `{posture}`.
pub const TAKEN_ON: &str = "While you were away #{unit} passed to its mind, which flies it on {posture} and will not fire first.";
/// [`ADMITTED_AWAY`] for a unit left on `GUARD` or `HOLD`: `{unit}`, `{posture}`.
pub const ADMITTED_ON: &str = "A field opened around #{unit} while you were away: its mind has the controls and flies it on {posture}. It will not fire first.";
/// [`GONE_HOME`] for a unit left on `GUARD` or `HOLD`: `{unit}`, `{posture}`, `{why}`. It
/// names the word and not what the unit does: only the kernel knows whether a guard can
/// still defend itself, and one that cannot goes home.
pub const STAYS_ON: &str =
    "#{unit} passes to its mind, which flies it on {posture}: {why}. It will not fire first.";

/// The operator, when a field takes in a rig of theirs that nobody sits in (doc 45 unit
/// AP.7), said to one who is connected and queued for one who is not: `{unit}`.
pub const UNATTENDED: &str = "A field opened around #{unit} with nobody in its seat: its mind has the controls, makes for its carrier's ramp and fires only on a unit that fires on it. A plan it was working ends once it is aboard.";
/// [`UNATTENDED`] for a rig left on `GUARD` or `HOLD`: `{unit}`, `{posture}`.
pub const UNATTENDED_ON: &str = "A field opened around #{unit} with nobody in its seat: its mind has the controls and flies it on {posture}. It will not fire first. A plan it was working runs on once the field has closed.";

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
    /// The pilot sat in the unit and was not connected when the field took it in (doc 45 cut
    /// AP.5; Q-45-1, Q-45-2).
    AwayAtAdmission,
    /// Nobody sits in the unit (doc 45 unit AP.7; Q-45-1): a rig worked from its carrier or
    /// on a fill plan, a carrier on its pad.
    Unattended,
}

impl Why {
    /// The reason as the clause [`GONE_HOME`] carries.
    #[must_use]
    pub const fn clause(self) -> &'static str {
        match self {
            Why::LinkLost => "its pilot's link is lost",
            Why::Quit => "its pilot has left",
            Why::Incapacitated => "its pilot is down",
            Why::AwayAtAdmission => "its pilot was away when the field opened",
            Why::Unattended => "nobody is in its seat",
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
    /// Its pilot, as whom the recover runs; for a unit nobody sits in, its player.
    pilot: ObjId,
    /// Whether `pilot` sits in it: a seated pilot who comes back takes it where it stands
    /// ([`on_present`]); a unit nobody sits in is put aboard whoever is connected.
    seated: bool,
    /// The recover.
    then: Then,
    /// The wall second before which it is not tried again.
    retry_at_wall_s: i64,
    /// Why its player was away when it left.
    why: Why,
    /// How many times the recover has been tried again.
    tries: u32,
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

/// One unit nobody sits in, in a running field (doc 45 unit AP.7).
#[derive(Clone, Debug)]
struct Bare {
    site: CanonicalId,
    task: MatchId,
    unit: ObjId,
    away: Option<Away>,
}

/// Every unit of every running field that nobody sits in, with its presence record.
fn bare_rows(w: &World) -> Vec<Bare> {
    let Some(book) = w.module_state::<FieldBook>() else {
        return Vec::new();
    };
    book.live
        .iter()
        .flat_map(|(site, hosted)| {
            hosted
                .live
                .units
                .iter()
                .filter(|row| row.pilot.is_none())
                .map(move |row| Bare {
                    site: site.clone(),
                    task: hosted.task,
                    unit: row.unit,
                    away: row.away,
                })
        })
        .collect()
}

/// `unit`'s row, when a running field holds it with nobody in its seat.
fn bare_of(w: &World, unit: ObjId) -> Option<Bare> {
    bare_rows(w).into_iter().find(|row| row.unit == unit)
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
    let site = super::field_of_unit(w, unit)?;
    w.module_state::<FieldBook>()?
        .hosted(&site)?
        .live
        .units
        .iter()
        .find(|row| row.unit == unit)?
        .away
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

/// **A field took in a unit whose pilot's link was already lost** (doc 45 cut AP.5; Q-45-1;
/// `super::admit::seat_absent`): the seat is `Linkdead` from the unit's first pulse and the one
/// clock runs from the loss, `since_wall_s`, not from the admission. The five minutes are the
/// pilot's, and a field opening inside them neither shortens nor restarts them. Nothing when
/// the row already carries a record.
pub(crate) fn admitted_link_lost(ctx: &mut Ctx<'_>, unit: ObjId, pilot: ObjId, since_wall_s: i64) {
    let Some(row) = row_of(ctx.w, unit).filter(|row| row.pilot == pilot) else {
        return;
    };
    if row.away.is_some() || super::orders::is_npc(ctx.w, pilot) {
        return;
    }
    let away = Away {
        why: Why::LinkLost,
        since_wall_s,
        handed: false,
    };
    set_away(ctx, &row.site, unit, Some(away));
    crate::arena_bridge::control_all_channels(ctx, unit, ControlState::Linkdead, pilot);
}

/// **The kernel has just taken `unit` into a running field** (doc 45 cut AP.5; a hull put back
/// in after its carrier was lost at the ramp, a launch): the host's word on an absent player is
/// owed again, so [`tick`] tells the task again. Before doc 45 unit AP.7 a word offered
/// ahead of the join was dropped by the arena; it now waits there for the unit, and telling
/// again is harmless.
pub(crate) fn tell_again(ctx: &mut Ctx<'_>, unit: ObjId) {
    if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
        minding.told.remove(&unit);
    }
}

/// **The kernel refused `unit`'s join** (`super::carrier::on_joined`; doc 45 unit AP.7): a
/// word that set the absent flag ahead of the join waits in the task for the unit, and would
/// bind a later join of the same hull whoever then sits in it. The host takes it back: a
/// clearing word never waits, and replaces the kept one.
pub(crate) fn join_refused(ctx: &mut Ctx<'_>, task: MatchId, unit: ObjId) {
    if let Some(tasks) = ctx
        .w
        .module_state::<crate::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
    {
        let _ = tasks.offer(
            task,
            ArenaMsg::Absent {
                unit,
                absent: false,
            },
        );
    }
    if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
        minding.told.remove(&unit);
    }
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
                .filter(|(_, owed)| owed.seated && owed.pilot == pilot)
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
    // The pilot: said to one who is connected and down, queued for one who is away. A unit
    // left on `GUARD` or `HOLD` is told so (doc 45 unit AP.6): its mind does not take it home.
    let posture = crate::posture::record::of(ctx.w, unit);
    let line = match (on_watch(ctx.w, pilot), posture.is_return()) {
        (true, true) => MIND_HAS_IT,
        (true, false) => MIND_HAS_IT_ON,
        (false, true) if why == Why::AwayAtAdmission => ADMITTED_AWAY,
        (false, false) if why == Why::AwayAtAdmission => ADMITTED_ON,
        (false, true) => TAKEN_HOME,
        (false, false) => TAKEN_ON,
    };
    notify_budget(
        ctx,
        pilot,
        &line
            .replace("{unit}", &unit.0.to_string())
            .replace("{posture}", posture.label()),
        NoticeLevel::Trip,
    );
    // Those it was with (this module's head, "the escort that leaves").
    let line = if posture.is_return() {
        GONE_HOME
    } else {
        STAYS_ON
    }
    .replace("{unit}", &unit.0.to_string())
    .replace("{posture}", posture.label())
    .replace("{why}", why.clause());
    for lead in leads(ctx.w, &row) {
        notify_budget(ctx, lead, &line, NoticeLevel::Trip);
    }
    tracing::info!(site = %row.site, unit = unit.0, pilot = pilot.0, ?why, "tacsim: a field unit passed to its mind for an absent player");
    true
}

/// **The units nobody sits in pass to their minds** (this module's head, "a unit nobody sits
/// in"; doc 45 unit AP.7): each unit of `site`'s running field, or `only` the one named, that
/// the live record holds with no pilot, that is some player's
/// ([`super::posture::player_of`]) and whose row carries no record yet. The record is written
/// handed, since there is no seat to wait five minutes for, and the kernel is told. A unit
/// whose row already says its mind has it is told to the kernel again.
///
/// Called at admission in the turn the task opens (`only` is `None`: the rig's operator is
/// told, and those it was with), at a join's answer, and by [`tick`] for whatever neither
/// reached (a resumed field, a hull whose player was named after it came in), which tells
/// nobody. The kernel keeps a word for a unit whose join it has not applied yet
/// (`crate::arena_task::ArenaMsg::Absent`), so the word may be offered ahead of the answer.
pub(crate) fn unattended(ctx: &mut Ctx<'_>, site: &CanonicalId, only: Option<ObjId>) {
    let bare: Vec<Bare> = bare_rows(ctx.w)
        .into_iter()
        .filter(|row| row.site == *site && only.is_none_or(|unit| unit == row.unit))
        .collect();
    for row in bare {
        if let Some(away) = row.away {
            if away.handed {
                tell_again(ctx, row.unit);
            }
            continue;
        }
        let Some(player) = super::posture::player_of(ctx, row.unit, None) else {
            continue;
        };
        let away = Away {
            why: Why::Unattended,
            since_wall_s: ctx.w.now(),
            handed: true,
        };
        set_away(ctx, &row.site, row.unit, Some(away));
        tell_bare(ctx, &row);
        tracing::info!(site = %row.site, unit = row.unit.0, player = player.0, "tacsim: a field unit nobody sits in passed to its mind as an absent player's");
        // A carrier stays where it is and was never anybody's to fly: nobody is told. A
        // join is not a field opening: the rig's operator was told when it opened.
        if only.is_some() || super::carrier::is_carrier(ctx.w, row.unit) {
            continue;
        }
        let posture = crate::posture::record::of(ctx.w, row.unit);
        let line = if posture.is_return() {
            UNATTENDED
        } else {
            UNATTENDED_ON
        };
        notify_budget(
            ctx,
            player,
            &line
                .replace("{unit}", &row.unit.0.to_string())
                .replace("{posture}", posture.label()),
            NoticeLevel::Trip,
        );
        let line = if posture.is_return() {
            GONE_HOME
        } else {
            STAYS_ON
        }
        .replace("{unit}", &row.unit.0.to_string())
        .replace("{posture}", posture.label())
        .replace("{why}", Why::Unattended.clause());
        for lead in leads_of(ctx.w, &row.site, row.unit, player) {
            notify_budget(ctx, lead, &line, NoticeLevel::Trip);
        }
    }
}

/// **Tell the arena `row`'s unit is flown for an absent player** ([`tell_arena`], for a unit
/// nobody sits in): the host's word alone, since there is no seat to release. Nothing when
/// the field will not take the word now; [`tick`] offers again.
fn tell_bare(ctx: &mut Ctx<'_>, row: &Bare) {
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
        .is_ok()
        && let Some(minding) = ctx.w.module_state_mut::<Minding>()
    {
        minding.told.insert(row.unit, row.task);
    }
}

/// **Who is told a unit has gone home**: its own party's leader and the leader of every party
/// that stands a friend of its party, less its own pilot.
fn leads(w: &World, row: &Row) -> BTreeSet<ObjId> {
    leads_of(w, &row.site, row.unit, row.pilot)
}

/// [`leads`], for `unit` on `site`, less `its` player.
fn leads_of(w: &World, site: &CanonicalId, unit: ObjId, its: ObjId) -> BTreeSet<ObjId> {
    let Some(live) = super::live_on(w, site) else {
        return BTreeSet::new();
    };
    let Some(own) = live.party_of(unit) else {
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
        .filter(|lead| *lead != its && !super::orders::is_npc(w, *lead))
        .collect()
}

/// **Tell the arena `row`'s unit is flown for an absent player**: the host's word first, so
/// the unit is bound by the absent player's rule from the pulse that hands it over, then the
/// seat's release. Nothing when the field will not take the word now; [`tick`] offers again,
/// and it offers again too when the task took the word and not every channel's release (doc 45
/// cut AP.5's review): a seat left half released is a unit nobody flies.
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
    let mut released = true;
    for channel in crate::arena::state::Channel::ALL {
        if let Err(error) = crate::arena_bridge::send_control(
            ctx,
            row.unit,
            channel,
            ControlState::Released,
            Some(row.pilot),
        ) {
            tracing::debug!(unit = row.unit.0, channel = channel.as_str(), %error, "tacsim: an absent player's seat was not released; the host's pass offers again");
            released = false;
        }
    }
    if released && let Some(minding) = ctx.w.module_state_mut::<Minding>() {
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
/// again. `then` is what the exit runs, and whether the host asked for it quietly.
///
/// **A unit nobody sits in** (doc 45 unit AP.7) is the host's to put aboard only when the host
/// itself asked it out. Its record stands for the whole field while its player may be
/// connected and typing: an exit that player asked for is theirs, told to them and never tried
/// again behind them. A carrier is never the host's: the only exit it has is its commander's
/// live lift (doc 27 Q-H10), and the host files no departure.
pub(crate) fn on_out(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    unit: ObjId,
    then: Option<(&Then, bool)>,
) {
    let handed = |away: Option<Away>| away.filter(|away| away.handed);
    // Its pilot; or, for a unit nobody sits in, its player.
    let (pilot, seated, away) = match row_of(ctx.w, unit) {
        Some(row) => (Some(row.pilot), true, handed(row.away)),
        None => match bare_of(ctx.w, unit) {
            Some(row) => {
                let hosts = then.is_some_and(|(_, quiet)| quiet)
                    && !super::carrier::is_carrier(ctx.w, unit);
                if !hosts {
                    return;
                }
                (
                    super::posture::player_of(ctx, unit, None),
                    false,
                    handed(row.away),
                )
            }
            None => return,
        },
    };
    let (Some(pilot), Some(away)) = (pilot, away) else {
        return;
    };
    let Some(then) = then.map(|(then, _)| then.clone()) else {
        return;
    };
    let now = ctx.w.now();
    if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
        minding.told.remove(&unit);
        minding.homebound.insert(
            unit,
            Homebound {
                site: site.clone(),
                pilot,
                seated,
                then,
                retry_at_wall_s: now.saturating_add(RECOVER_RETRY_WALL_S),
                why: away.why,
                tries: 0,
            },
        );
    }
}

/// **Why `unit`'s player was away when its mind took it out at the ramp**, while the host
/// still owes its recover ([`on_out`]): what a hull put back into the field is handed over
/// under again (doc 45 cut AP.5).
#[must_use]
pub(crate) fn owed_why(w: &World, unit: ObjId) -> Option<Why> {
    w.module_state::<Minding>()
        .and_then(|minding| minding.homebound.get(&unit))
        .map(|owed| owed.why)
        // A hull that left with nobody in it and comes back with a pilot is that pilot's
        // absence, not an empty seat's (doc 45 unit AP.7).
        .filter(|why| *why != Why::Unattended)
}

/// **Whether the host owes `unit` its recover** ([`on_out`]): its refusals are tried again,
/// never said.
#[must_use]
pub(crate) fn homebound(w: &World, unit: ObjId) -> bool {
    w.module_state::<Minding>()
        .is_some_and(|minding| minding.homebound.contains_key(&unit))
}

/// **Whether the host owes `unit` its recover as `who`** ([`on_out`]): the player the host
/// runs that recover as, and nobody else. The recover's own gate reads it
/// (`crate::surface::act::recover_plan`; doc 45 unit AP.11): the host's recover is not asked
/// where that player is.
#[must_use]
pub(crate) fn homebound_as(w: &World, unit: ObjId, who: ObjId) -> bool {
    w.module_state::<Minding>()
        .and_then(|minding| minding.homebound.get(&unit))
        .is_some_and(|owed| owed.pilot == who)
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
        // Doc 45 unit AP.6 (Q-45-7): a unit on `HOLD` stays, and is never asked out. One on
        // `GUARD` is asked like one on `RETURN`, and the kernel keeps it while it can still
        // defend itself (`crate::arena::withdraw::WithdrawRefusal::Standing`): only the
        // kernel holds the unit's live damage and its rounds.
        if crate::posture::record::of(ctx.w, row.unit) == crate::posture::Posture::Hold
            || in_tow(ctx, &row.site, row.unit)
        {
            continue;
        }
        if let Some(then) = recover_of(ctx, row.unit) {
            super::home::ask_out(ctx, row.unit, row.pilot, then);
        }
    }
    // Doc 45 unit AP.7: and the units nobody sits in. Nobody takes such a unit back, so
    // there is no presence to read: the record is opened where no hook opened it, the task is
    // told, and a rig is asked out at its ramp exactly as above.
    for row in bare_rows(ctx.w) {
        let Some(away) = row.away else {
            // Most units with no pilot are nobody's (a fixture, a casualty, a hired crew's
            // launch): asked once here, and handed over only when some player's.
            if super::posture::player_of(ctx, row.unit, None).is_some() {
                unattended(ctx, &row.site, Some(row.unit));
            }
            continue;
        };
        if !away.handed {
            continue;
        }
        let told = ctx
            .w
            .module_state::<Minding>()
            .is_some_and(|minding| minding.told.get(&row.unit) == Some(&row.task));
        if !told {
            tell_bare(ctx, &row);
            continue;
        }
        // **No mind lifts a carrier** (Q-45-11; doc 27 Q-H10): it is home where it stands,
        // and the one way it leaves its rigs behind is its commander's live lift.
        if super::carrier::is_carrier(ctx.w, row.unit)
            || crate::posture::record::of(ctx.w, row.unit) == crate::posture::Posture::Hold
            || in_tow(ctx, &row.site, row.unit)
        {
            continue;
        }
        let Some(player) = super::posture::player_of(ctx, row.unit, None) else {
            continue;
        };
        if let Some(then) = recover_of(ctx, row.unit) {
            super::home::ask_out(ctx, row.unit, player, then);
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

/// **Whether `unit` is half of a tow on `site`'s record** (doc 36 cut GC.4; doc 40 cut FR.5,
/// Q-40-25): a casualty in tow, or the tug that tows one. Neither is the host's to ask out.
/// The kernel judges a casualty in tow by its tug, so an ask for the casualty alone, made
/// while its tug stands beside the ramp, takes it out from behind the tug; and a tug in tow
/// leaves only with its casualty, by its player's own load (`crate::surface::tow`). Found by
/// doc 45 unit AP.7, whose units nobody sits in are the first absent players' units that are
/// commonly in tow; it holds for a seated pilot's unit too.
fn in_tow(ctx: &Ctx<'_>, site: &CanonicalId, unit: ObjId) -> bool {
    super::close::site_record(ctx, site).is_some_and(|record| {
        record.tows.towing(unit).is_some() || record.tows.towed(unit).is_some()
    })
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
        // Back in a field: its carrier was lost at the ramp and the host put it back
        // (`super::boarding`), with a fresh row and its pilot's seat. Its pilot is still away
        // (their return clears this list), so the unit passes to its mind again.
        //
        // Doc 45 cut AP.5 (its review): not before the kernel has answered the join. A word
        // offered ahead of it named a unit the arena did not hold yet and was dropped there
        // (since doc 45 unit AP.7 it waits for the unit), while the release that follows it
        // lands after the join, and would leave the hull a mind under no rule. The seat's
        // release still must not run ahead of the join, so the join's answer hands it over
        // (`super::admit::seat_absent`), and until then it is a seat nobody types at.
        if row_of(ctx.w, unit).is_some() {
            if super::carrier::rejoining_unit(ctx.w, unit) {
                continue;
            }
            if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
                minding.homebound.remove(&unit);
            }
            hand_over(ctx, unit, owed.pilot, owed.why);
            continue;
        }
        // Doc 45 unit AP.7: and a unit nobody sits in, which the join's answer or this
        // pass hands to its mind again ([`unattended`]).
        if let Some(row) = bare_of(ctx.w, unit) {
            if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
                minding.homebound.remove(&unit);
            }
            unattended(ctx, &row.site, Some(unit));
            continue;
        }
        // A recover that never lands is given up: the carrier lifted, or the unit can no
        // longer come aboard. It stands where it is, and its pilot reads why at login.
        if owed.tries >= RECOVER_TRIES {
            if let Some(minding) = ctx.w.module_state_mut::<Minding>() {
                minding.homebound.remove(&unit);
            }
            notify_budget(
                ctx,
                owed.pilot,
                &LEFT_AT_RAMP.replace("{unit}", &unit.0.to_string()),
                NoticeLevel::Trip,
            );
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
            row.tries = row.tries.saturating_add(1);
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

    /// **Doc 45 unit AP.7: a unit nobody sits in has a record of its own reason**, written
    /// handed from the start, and its lines name the unit and promise what its posture does.
    #[test]
    fn ap7_a_unit_nobody_sits_in_is_recorded_as_its_minds_from_the_start() {
        let bare = Away {
            why: Why::Unattended,
            since_wall_s: 40,
            handed: true,
        };
        let text = serde_json::to_string(&bare).expect("it encodes");
        assert_eq!(
            text,
            r#"{"why":"unattended","since_wall_s":40,"handed":true}"#
        );
        assert_eq!(serde_json::from_str::<Away>(&text).ok(), Some(bare));
        assert_eq!(Why::Unattended.clause(), "nobody is in its seat");
        for line in [UNATTENDED, UNATTENDED_ON] {
            assert!(
                line.contains("#{unit}")
                    && line.contains("will not fire first")
                        != line.contains("fires only on a unit that fires on it")
            );
        }
        assert!(UNATTENDED.contains("ends once it is aboard") && !UNATTENDED.contains("{posture}"));
        assert!(UNATTENDED_ON.contains("{posture}") && UNATTENDED_ON.contains("runs on"));
    }
}
