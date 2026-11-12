//! **Setting a posture, as an order is given** (doc 45 unit AP.4; Q-45-19).
//!
//! A posture changes what a unit does when its player is not at its controls, so it is set
//! the way an order reaches a unit and no other way:
//!
//! 1. **From the unit's seat**: `MIND POSTURE <word>` at the station of a unit in an IC field
//!    (`crate::match_::cockpit`).
//! 2. **From aboard its carrier**, or from the rig's own cab on open ground:
//!    `+tv/<n> posture <word>` (`crate::port::tv`). The operator is with the unit, and the
//!    word is set at once.
//! 3. **Over the comms relay**: the same `+tv/<n> posture <word>` typed by an operator who is
//!    in the world but neither aboard the carrier nor in that rig's cab. The order travels to
//!    the unit (`crate::field::orders::posture_order`) and jamming stops it: an order that
//!    does not arrive changes nothing.
//!
//! **Never from an OOC room** ([`ooc`]): Limbo, the OOC area and the simulator centre inside
//! it. A posture in a practice match is doc 45 unit AP.10's.
//!
//! Every way ends in the record's one writer (`super::record::set`). A read (`MIND POSTURE`,
//! `+tv/<n> posture` with no word) is answered anywhere.

use mush_core::{CmdError, Ctx, LIMBO, ObjId, ObjKind, World};

use super::Posture;

/// The refusal from an OOC room.
pub const POSTURE_OOC: &str =
    "A posture is an order to a unit in the world. It is not given from an OOC room.";
/// The refusal for a word that is no posture.
pub const POSTURE_WORDS: &str = "A posture is RETURN, GUARD or HOLD.";
/// A read: `{unit}`, `{posture}`.
pub const POSTURE_READ: &str = "{unit} holds {posture}.";
/// A set made by an operator who is with the unit: `{unit}`, `{posture}`.
pub const POSTURE_SET: &str = "{unit} now holds {posture}.";
/// A set that changed nothing: `{unit}`, `{posture}`.
pub const POSTURE_KEPT: &str = "{unit} already holds {posture}.";
/// A set that went over the relay and arrived: `{unit}`, `{posture}`.
pub const POSTURE_RELAYED: &str =
    "Your order reaches {unit} over the relay: it now holds {posture}.";
/// A set the relay did not carry: `{unit}`, `{posture}` (what it still holds).
pub const POSTURE_JAMMED: &str =
    "Your order does not reach {unit}: the relay to it is jammed. It still holds {posture}.";
/// A set to a unit the sender cannot reach at all: `{unit}`.
pub const POSTURE_NO_PATH: &str = "Your order does not reach {unit}: you have no relay to where it is. Give it from aboard its carrier.";
/// What follows a `GUARD` or a `HOLD`, until the house mind flies them (doc 45 unit AP.6).
pub const POSTURE_NOT_FLOWN_YET: &str = "The house mind does not fly GUARD or HOLD yet and still takes the unit home; a mind you filed reads the word.";
/// The roster's and the status's line for a unit off the default: `{posture}`.
pub const POSTURE_LINE: &str = "  posture {posture}";

/// **Whether `actor` stands in an OOC room** (this module's head): Limbo, a room of the OOC
/// area's pack, or anything inside one (a simulator pod).
#[must_use]
pub fn ooc(w: &World, actor: ObjId) -> bool {
    let mut at = w.get(actor).and_then(|object| object.location);
    // Up from whatever holds the actor (a pod, a cab) to the room it is in.
    for _ in 0..8 {
        let Some(here) = at else {
            return false;
        };
        if here == LIMBO {
            return true;
        }
        let Some(object) = w.get(here) else {
            return false;
        };
        if object.kind == ObjKind::Room {
            return w
                .module_state::<mush_atlas::AtlasState>()
                .and_then(|atlas| atlas.records.roomgeo.get(&here))
                .is_some_and(|geo| {
                    geo.neighborhood.pack.as_str() == mush_atlas::starter::OOC_AREA_PACK
                });
        }
        at = object.location;
    }
    false
}

/// The posture `word` names.
///
/// # Errors
///
/// [`POSTURE_WORDS`].
pub fn word(word: &str) -> Result<Posture, CmdError> {
    Posture::parse(word).ok_or_else(|| CmdError::msg(POSTURE_WORDS))
}

/// One of this module's lines, filled.
#[must_use]
pub fn line(template: &str, unit: &str, posture: Posture) -> String {
    template
        .replace("{unit}", unit)
        .replace("{posture}", posture.label())
}

/// **Set `unit`'s posture for an operator who is with it** (this module's head, 1 and 2), and
/// answer the lines to say. `name` is how the answer names the unit.
///
/// # Errors
///
/// [`POSTURE_OOC`]; the record's own refusal.
pub fn set_present(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    name: &str,
    posture: Posture,
) -> Result<Vec<String>, CmdError> {
    if ooc(ctx.w, ctx.actor) {
        return Err(CmdError::msg(POSTURE_OOC));
    }
    let changed = super::record::set(ctx, unit, posture).map_err(CmdError::msg)?;
    Ok(said(
        if changed { POSTURE_SET } else { POSTURE_KEPT },
        name,
        posture,
    ))
}

/// The answer to a set: the line, and the note for a posture the house mind does not fly yet.
#[must_use]
pub fn said(template: &str, name: &str, posture: Posture) -> Vec<String> {
    let mut lines = vec![line(template, name, posture)];
    if !posture.is_return() {
        lines.push(POSTURE_NOT_FLOWN_YET.to_owned());
    }
    lines
}

/// **`+tv/<n> posture [word]`** (this module's head, 2 and 3): the rig's posture read, or set
/// at once by an operator aboard its carrier or in its cab, or sent to it over the relay by
/// one who is neither.
///
/// # Errors
///
/// [`POSTURE_WORDS`], [`POSTURE_OOC`], [`POSTURE_JAMMED`], [`POSTURE_NO_PATH`].
pub fn tv(
    ctx: &mut Ctx<'_>,
    rig: &crate::surface::act::RigAt,
    typed: Option<&str>,
) -> Result<Vec<String>, CmdError> {
    let name = format!("Rig {}", rig.slot);
    let Some(typed) = typed else {
        return Ok(vec![line(
            POSTURE_READ,
            &name,
            super::record::of(ctx.w, rig.tv),
        )]);
    };
    let posture = word(typed)?;
    if rig.operator_present(ctx.w, ctx.actor) {
        return set_present(ctx, rig.tv, &name, posture);
    }
    if ooc(ctx.w, ctx.actor) {
        return Err(CmdError::msg(POSTURE_OOC));
    }
    match crate::field::orders::posture_order(ctx, ctx.actor, rig.tv, posture) {
        crate::field::orders::PostureOrder::Arrived => Ok(said(POSTURE_RELAYED, &name, posture)),
        crate::field::orders::PostureOrder::Jammed => Err(CmdError::msg(line(
            POSTURE_JAMMED,
            &name,
            super::record::of(ctx.w, rig.tv),
        ))),
        crate::field::orders::PostureOrder::NoPath => {
            Err(CmdError::msg(line(POSTURE_NO_PATH, &name, posture)))
        }
    }
}

#[cfg(test)]
mod tests {
    use mush_core::{GOD, LIMBO, ObjKind, World};

    use super::{POSTURE_NOT_FLOWN_YET, POSTURE_READ, POSTURE_SET, line, ooc, said, word};
    use crate::posture::Posture;

    #[test]
    fn limbo_and_what_stands_in_it_are_ooc_and_a_plain_room_is_not() {
        let mut w = World::new_minimal();
        let player = w
            .create(ObjKind::Player, "Walker", GOD, Some(LIMBO))
            .expect("a player is made");
        assert!(ooc(&w, player), "Limbo is OOC");
        let pod = w
            .create(ObjKind::Thing, "Pod", GOD, Some(LIMBO))
            .expect("a thing is made");
        w.move_to(player, pod).expect("into the pod");
        assert!(ooc(&w, player), "inside a thing in an OOC room");
        // A room no atlas record places in the OOC area is the world's: a hull's interior.
        let bay = w
            .create(ObjKind::Room, "Bay", GOD, None)
            .expect("a room is made");
        w.move_to(player, bay).expect("into the bay");
        assert!(!ooc(&w, player));
        let cab = w
            .create(ObjKind::Thing, "Cab", GOD, Some(bay))
            .expect("a thing is made");
        w.move_to(player, cab).expect("into the cab");
        assert!(!ooc(&w, player), "a cab in the world is in the world");
    }

    /// The terminal floor is 120 columns: no line this module says is wider.
    #[test]
    fn every_line_fits_the_terminal_floor() {
        use super::{
            POSTURE_JAMMED, POSTURE_KEPT, POSTURE_LINE, POSTURE_NO_PATH, POSTURE_OOC,
            POSTURE_RELAYED, POSTURE_WORDS,
        };
        for template in [
            POSTURE_OOC,
            POSTURE_WORDS,
            POSTURE_READ,
            POSTURE_SET,
            POSTURE_KEPT,
            POSTURE_RELAYED,
            POSTURE_JAMMED,
            POSTURE_NO_PATH,
            POSTURE_NOT_FLOWN_YET,
            POSTURE_LINE,
        ] {
            let said = line(template, "Rig 4", Posture::Return);
            assert!(said.chars().count() < 120, "{said}");
        }
    }

    #[test]
    fn the_words_and_the_lines() {
        assert_eq!(word("guard").ok(), Some(Posture::Guard));
        assert_eq!(word("HOLD").ok(), Some(Posture::Hold));
        assert!(word("flee").is_err());
        assert_eq!(
            line(POSTURE_READ, "Rig 2", Posture::Return),
            "Rig 2 holds RETURN."
        );
        assert_eq!(
            said(POSTURE_SET, "Rig 2", Posture::Return),
            vec!["Rig 2 now holds RETURN.".to_owned()]
        );
        assert_eq!(
            said(POSTURE_SET, "Rig 2", Posture::Guard),
            vec![
                "Rig 2 now holds GUARD.".to_owned(),
                POSTURE_NOT_FLOWN_YET.to_owned()
            ]
        );
    }
}
