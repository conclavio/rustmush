//! **Postures in the simulator** (doc 45 unit AP.10; Q-45-25, with doc 44's Q-44-19).
//!
//! A player may set a posture on the hull they fly in a practice match and watch its mind fly
//! it, so they can see what `GUARD` does before trusting a rig to it. Three forms of the
//! seat's `MIND` verb, typed in the pod:
//!
//! - `MIND POSTURE [RETURN|GUARD|HOLD]` reads the hull's practice posture, or sets it. The
//!   word is kept on the running match's mirror; the arena is told nothing until the pilot
//!   lets go, so a posture set and never left changes nothing the match does.
//! - `MIND AWAY` lets go of the controls. The pilot stays in the pod and goes on being sent
//!   the cockpit; the hull is handed to its sealed mind **as an absent player's**: the arena
//!   is told so by the two words the IC field host tells it (`ArenaMsg::Posture`,
//!   `ArenaMsg::Absent`), so the hull is held to the one rule a rig left in the world is
//!   (`crate::arena::absent::permits`) and flown on its posture by the same kernel
//!   (`Arena::lay_standing`, `Arena::stand_posture`).
//! - `MIND BACK` takes the controls again where the hull stands.
//!
//! **A practice and a drop are told apart by who said it.** A practice is an act typed in the
//! seat by a pilot who is connected and stays in the pod; its hand-over reaches the arena as
//! a release that names no pilot. A drop is a fact about the pilot's seat that the world
//! tells the arena: a lost link, the grace running out, a `QUIT`, an `EXIT`, a boot's
//! resume. Every one of those is a control that **names the pilot**
//! ([`crate::arena_bridge::send_control`]), and in a match the arena ends that pilot's
//! practice on such a control before it applies it (`Arena::end_practice`): the rule is
//! lifted, the arena's copy of the posture goes back to `RETURN`, and the hull is the
//! pilot's again. What the control then does, it does to a hull its pilot holds, exactly as
//! for a pilot who never practised. So a dropped pilot's hull goes to its sealed mind seeded
//! as every drop seeds it, and fights, bound by nothing (Q-44-19): "a unit that walks home
//! loses its side the match". `MIND BACK` is the same control from the seat, a `Live`. The
//! hull is stopped when a practice ends, so it is handed back standing.
//!
//! **Nothing here is in character.** The practice posture is kept on the running match's
//! mirror ([`crate::arena_bridge::MatchMirror::postures`]) and, while the pilot is away, in
//! that match's arena, and nowhere else. No unit's own posture record
//! (`crate::posture::record`) is read or written: the simulator's hulls keep whatever
//! record they had (none), a practice match leaves no word behind it, and the next match
//! opens on `RETURN`.
//!
//! **Whose the other units are.** `GUARD` answers for "another unit of the same player". A
//! pilot in the simulator flies one hull; the side's machines are fielded by the side's lead
//! (`SIM AI`, `SIM AI MIND`: the fill's warrant principal,
//! `crate::match_::lobby::lead_character_of`). So the lead's own units are their hull and
//! the side's fill, and every other pilot's are their one hull: a lead's guard answers for
//! the fill, and for any other pilot the fill's quarrel is a stranger's, as another
//! player's rigs are in the world. The arena is told whose each is at `MIND AWAY` (and at a
//! posture set while away); it keeps a fire record on those hulls from then on
//! (`Arena::note_fired_on`), so "has fired on it" counts from the first time the pilot let
//! go in the match.
//!
//! **`RETURN` in a match.** There is no carrier to go home to. The go-home order sends the
//! hull to its side's home on the arena's ground (its entry zone), where it stays; it
//! answers only a unit that has fired on it.
//!
//! **Which matches take a posture.** Any match with a pilot in a pod: a skirmish, a hold
//! match and a range. A range's targets never fire, so a hull left on `GUARD` or `HOLD`
//! there stands and fires nothing. A match of machines only (a staff `@arena/start`, a
//! proving run) has no seat to set one from.

use mush_core::{Ctx, ObjId, World};

use crate::arena::intents::ControlState;
use crate::arena::state::Channel;
use crate::arena_bridge::{ArenaRegistry, IntentRefusal};
use crate::arena_task::{ArenaMsg, MatchId};
use crate::match_::cockpit::CockpitError;
use crate::posture::Posture;
use crate::posture::order::{POSTURE_KEPT, POSTURE_READ, line};

/// A set from a seat that holds the controls: `{unit}`, `{posture}`.
pub const PRACTICE_SET: &str = "{unit} now holds {posture} for this match. MIND AWAY lets its mind fly it on that word while you watch.";
/// A set from a seat whose pilot has let go: `{unit}`, `{posture}`.
pub const PRACTICE_SET_AWAY: &str =
    "{unit} now holds {posture}: its mind flies it on that word from its next think.";
/// A read from a seat whose pilot has let go: `{unit}`, `{posture}`.
pub const PRACTICE_READ_AWAY: &str = "{unit} holds {posture}, and its mind is flying it on that word. MIND BACK takes the controls again.";
/// `MIND AWAY`: `{unit}`, `{posture}`.
pub const PRACTICE_AWAY: &str = "You let go: {unit}'s mind flies it on {posture} as if you were away. MIND BACK takes the controls again.";
/// `MIND AWAY` from a seat that has let go already: `{unit}`.
pub const PRACTICE_ALREADY_AWAY: &str =
    "{unit}'s mind already has the controls. MIND BACK takes them again.";
/// `MIND BACK`: `{unit}`.
pub const PRACTICE_BACK: &str = "You take the controls again: {unit} is yours where it stands.";
/// `MIND BACK` from a seat that holds the controls: `{unit}`.
pub const PRACTICE_NOT_AWAY: &str = "You already hold {unit}'s controls.";
/// `MIND AWAY` or `MIND BACK` in an IC field.
pub const PRACTICE_ONLY: &str = "MIND AWAY and MIND BACK are the simulator's. In the world a unit's mind takes it when you leave its controls.";
/// Bare `MIND`'s channel line while the seat's pilot has let go.
pub const PRACTICE_CHANNELS: &str = "Channels: the mind holds every one and flies the hull on its posture; MIND BACK takes them again.";
/// The seat's `STATUS` line while its pilot has let go.
pub const PRACTICE_STATUS_AWAY: &str = "  its mind has the controls; MIND BACK takes them again";
/// What an order typed in the seat is refused with while its pilot has let go
/// (`crate::arena_bridge::IntentRefusal::Away`), after the cockpit's `Refused: `.
pub const PRACTICE_SEAT_REFUSED: &str =
    "the seat gives no order while the mind flies the hull; MIND BACK takes the controls";

/// **The practice match that holds `unit`**: a running match of the arena registry that is
/// not an IC field's. `None` for a unit in a field and for one no arena holds.
#[must_use]
pub fn match_of(w: &World, unit: ObjId) -> Option<MatchId> {
    if crate::field::field_of_unit(w, unit).is_some() {
        return None;
    }
    w.module_state::<ArenaRegistry>()?.match_of_unit(unit)
}

/// **The posture practised on `unit`** (this module's head), `RETURN` when none was set; and
/// whether its pilot has let go of the controls. `None` for a unit no practice match holds.
#[must_use]
pub fn held(w: &World, unit: ObjId) -> Option<(Posture, bool)> {
    match_of(w, unit)?;
    let registry = w.module_state::<ArenaRegistry>()?;
    Some((
        registry.practice_posture(unit),
        registry.standing_back(unit),
    ))
}

/// **The other units of `pilot`** in the practice match `match_id` (this module's head,
/// "whose the other units are"): the side's fill when `pilot` leads the side, and nothing
/// otherwise.
fn other_units(w: &World, match_id: MatchId, pilot: ObjId) -> Vec<ObjId> {
    let Some(book) = w.module_state::<crate::match_::MatchBook>() else {
        return Vec::new();
    };
    let centre = book.centre();
    if centre.match_id != Some(match_id) {
        return Vec::new();
    }
    let Some(slot) = centre.slot_of_pilot(pilot) else {
        return Vec::new();
    };
    if !centre.leads_side(slot.team, slot.account) {
        return Vec::new();
    }
    book.get(match_id)
        .and_then(|record| record.teams.iter().find(|roster| roster.team == slot.team))
        .map(|roster| {
            roster
                .units
                .iter()
                .copied()
                .filter(|unit| roster.pilots.iter().all(|(_, flown)| flown != unit))
                .collect()
        })
        .unwrap_or_default()
}

/// **Tell the arena `unit`'s practice posture and whose it and the pilot's other units
/// are** (this module's head). The hull's own word first: a refusal of that one is the
/// command's, and nothing was told. A word for another unit that the task will not take is
/// told again at the next posture set while away, or at the next `MIND AWAY`.
fn tell(
    ctx: &mut Ctx<'_>,
    match_id: MatchId,
    pilot: ObjId,
    unit: ObjId,
    posture: Posture,
) -> Result<(), IntentRefusal> {
    let others = other_units(ctx.w, match_id, pilot);
    let registry = ctx
        .w
        .module_state::<ArenaRegistry>()
        .ok_or(IntentRefusal::Unavailable)?;
    let tasks = registry.tasks().clone();
    let own = ArenaMsg::Posture {
        unit,
        posture,
        player: Some(pilot),
    };
    if tasks.offer(match_id, own).is_err() {
        return Err(if tasks.is_live(match_id) {
            IntentRefusal::Busy
        } else {
            IntentRefusal::Gone
        });
    }
    for other in others.into_iter().filter(|other| *other != unit) {
        // A machine holds no practice posture: the word names whose it is.
        let _ = tasks.offer(
            match_id,
            ArenaMsg::Posture {
                unit: other,
                posture: Posture::Return,
                player: Some(pilot),
            },
        );
    }
    Ok(())
}

/// The seat's practice match, and that the seat holds the hull's helm: the check a helm
/// order takes (`crate::arena_bridge::authorise_seat`), so a second berth or a gunner
/// neither sets the hull's word nor lets go of, or takes, controls it does not hold.
fn helm(ctx: &mut Ctx<'_>, driver: ObjId, unit: ObjId) -> Result<MatchId, CockpitError> {
    Ok(crate::arena_bridge::authorise_seat(
        ctx,
        driver,
        unit,
        Channel::Helm,
    )?)
}

/// **`MIND POSTURE` with no word, in a practice match**: what the hull holds.
#[must_use]
pub fn read(w: &World, unit: ObjId, name: &str) -> String {
    let (posture, away) = held(w, unit).unwrap_or_default();
    line(
        if away {
            PRACTICE_READ_AWAY
        } else {
            POSTURE_READ
        },
        name,
        posture,
    )
}

/// **`MIND POSTURE <word>` in a practice match** (this module's head): the word is kept on
/// the match's mirror, and told to the match's arena when the pilot is away, so the mind
/// flying the hull reads it at its next think. No unit's record is written.
///
/// # Errors
///
/// The seat's refusal for a seat that does not hold the helm; while away, `Busy` when the
/// arena would not take the word, and nothing changed.
pub fn set(
    ctx: &mut Ctx<'_>,
    driver: ObjId,
    unit: ObjId,
    name: &str,
    posture: Posture,
) -> Result<String, CockpitError> {
    let match_id = helm(ctx, driver, unit)?;
    let (was, away) = held(ctx.w, unit).unwrap_or_default();
    if away {
        tell(ctx, match_id, driver, unit, posture)?;
    }
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.note_practice_posture(unit, posture);
    }
    let template = match (was == posture, away) {
        (true, _) => POSTURE_KEPT,
        (false, true) => PRACTICE_SET_AWAY,
        (false, false) => PRACTICE_SET,
    };
    Ok(line(template, name, posture))
}

/// **`MIND AWAY`** (this module's head): the pilot lets go of the controls and stays in the
/// pod. The arena is told the hull's posture and whose it is, then that it is flown for an
/// absent player, then that its seat is released **naming no pilot**, which hands the hull
/// to its mind and leaves the pilot on the arena's frame roster, bound in the registry and
/// seated on the board. The words are offered in that order and the task takes them in it,
/// so the release never reaches a pulse ahead of the rule.
///
/// # Errors
///
/// The seat's refusal; [`PRACTICE_ALREADY_AWAY`]; `Busy` when the arena would not take the
/// words, and nothing changed.
pub fn away(
    ctx: &mut Ctx<'_>,
    driver: ObjId,
    unit: ObjId,
    name: &str,
) -> Result<String, CockpitError> {
    let match_id = helm(ctx, driver, unit)?;
    crate::match_::cockpit::require_alive(ctx.w, driver)?;
    let (posture, was_away) = held(ctx.w, unit).unwrap_or_default();
    if was_away {
        return Err(CockpitError::Refused(line(
            PRACTICE_ALREADY_AWAY,
            name,
            posture,
        )));
    }
    tell(ctx, match_id, driver, unit, posture)?;
    let tasks = ctx
        .w
        .module_state::<ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .ok_or(CockpitError::Unavailable)?;
    if tasks
        .offer(match_id, ArenaMsg::Absent { unit, absent: true })
        .is_err()
    {
        return Err(IntentRefusal::Busy.into());
    }
    if let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() {
        registry.note_standing_back(unit, Some(driver));
    }
    // The debrief's `Your mind` block: the mind flies the tank from here.
    crate::match_::lobby::practice_span(ctx, driver, true);
    for channel in Channel::ALL {
        if let Err(error) =
            crate::arena_bridge::send_control(ctx, unit, channel, ControlState::Released, None)
        {
            tracing::debug!(%unit, channel = channel.as_str(), %error, "a practice hand-over was not delivered");
        }
    }
    Ok(line(PRACTICE_AWAY, name, posture))
}

/// **`MIND BACK`** (this module's head): the pilot takes the controls again. One `Live` a
/// channel naming the pilot, at a new epoch: what a reclaim sends
/// (`crate::arena_bridge::reseat`). The arena ends the practice on it
/// (`Arena::end_practice`) and the world's half follows from the same control
/// (`ended_by_seat`).
///
/// # Errors
///
/// The seat's refusal for a seat that does not hold the helm; [`PRACTICE_NOT_AWAY`].
pub fn back(
    ctx: &mut Ctx<'_>,
    driver: ObjId,
    unit: ObjId,
    name: &str,
) -> Result<String, CockpitError> {
    helm(ctx, driver, unit)?;
    let practising = ctx
        .w
        .module_state::<ArenaRegistry>()
        .and_then(|registry| registry.practising(unit));
    if practising != Some(driver) {
        return Err(CockpitError::Refused(line(
            PRACTICE_NOT_AWAY,
            name,
            Posture::Return,
        )));
    }
    crate::arena_bridge::reseat(ctx, unit, driver);
    Ok(line(PRACTICE_BACK, name, Posture::Return))
}

/// **The world's half of a practice's end** (this module's head): called by
/// [`crate::arena_bridge::send_control`] for every control that names a pilot, ahead of the
/// control. When a pilot has let go of `unit`'s controls, the match's mirror forgets it and
/// that pilot's debrief span closes. Nothing is sent: the arena ends the practice on the
/// control itself (`Arena::end_practice`), whichever control it is, whichever seat of the
/// hull it names and however the task has buffered it. Answers whether a practice was
/// ended.
pub(crate) fn ended_by_seat(ctx: &mut Ctx<'_>, unit: ObjId) -> bool {
    let Some(registry) = ctx.w.module_state_mut::<ArenaRegistry>() else {
        return false;
    };
    let Some(practising) = registry.practising(unit) else {
        return false;
    };
    registry.note_standing_back(unit, None);
    // The debrief's span closes here; a drop that follows opens its own.
    crate::match_::lobby::practice_span(ctx, practising, false);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The terminal floor is 120 columns: no line this module says is wider, with a hull's
    /// name of twenty characters and the longest word.
    #[test]
    fn ap10_every_line_fits_the_terminal_floor() {
        for template in [
            PRACTICE_SET,
            PRACTICE_SET_AWAY,
            PRACTICE_READ_AWAY,
            PRACTICE_AWAY,
            PRACTICE_ALREADY_AWAY,
            PRACTICE_BACK,
            PRACTICE_NOT_AWAY,
            PRACTICE_ONLY,
            PRACTICE_STATUS_AWAY,
            PRACTICE_CHANNELS,
        ] {
            let said = line(template, "Twenty Characters Xx", Posture::Return);
            assert!(said.chars().count() < 120, "{said}");
        }
        assert!(
            format!("Refused: {}.", IntentRefusal::Away).chars().count() < 120,
            "the seat's refusal"
        );
    }

    #[test]
    fn ap10_the_lines() {
        assert_eq!(
            line(PRACTICE_SET, "Red Tank 1", Posture::Guard),
            "Red Tank 1 now holds GUARD for this match. MIND AWAY lets its mind fly it on that word while you watch."
        );
        assert_eq!(
            line(PRACTICE_AWAY, "Red Tank 1", Posture::Hold),
            "You let go: Red Tank 1's mind flies it on HOLD as if you were away. MIND BACK takes the controls again."
        );
        assert_eq!(
            format!("Refused: {}.", IntentRefusal::Away),
            "Refused: the seat gives no order while the mind flies the hull; MIND BACK takes the controls."
        );
    }
}
