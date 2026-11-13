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
//! it. In a practice match `MIND POSTURE` sets a practice posture instead, which is the
//! match's and never a unit's record (doc 45 unit AP.10; `crate::match_::practice`): none
//! of the three ways above is taken, and [`set_present`] is not called.
//!
//! Every way ends in the record's one writer (`super::record::set`). A read (`MIND POSTURE`,
//! `+tv/<n> posture` with no word) is answered anywhere, and to a held character too.
//!
//! **Who may give it.** The seat that holds the hull's helm, for `MIND POSTURE`. For `+tv`,
//! whoever `+tv` selects the rig for: its lessee or title holder at the helm hull, at once
//! when aboard the carrier or in the rig's cab and over the relay otherwise; and a hired
//! crew's operator, from aboard the rig only.

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
/// A hired crew's operator who is not aboard the rig.
pub const POSTURE_CREW: &str = "A crew sets its rig's posture from aboard the rig.";
/// A roster order's refusal for a slot a hired crew flies (doc 45 unit AP.4a).
pub const POSTURE_ORDER_CREW: &str =
    "A posture is ordered for a player's unit. A hired crew's unit takes none.";
/// The unit's player answering a leader's `POSTURE` order themselves.
pub const POSTURE_ORDER_ANSWERED: &str =
    "A POSTURE order is answered by the unit itself. Set its posture yourself to change it.";
/// A roster order's usage.
pub const POSTURE_ORDER_USAGE: &str = "Use <slot> POSTURE RETURN, GUARD or HOLD.";
/// The unit's player, when a leader's roster order sets its posture: `{leader}`, `{unit}`,
/// `{posture}`.
pub const POSTURE_ORDERED: &str = "{leader}, leading your detachment, ordered #{unit} to {posture}: it holds that word until you set another.";
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
    // Deeper than anything the world nests: not somewhere an order is given from.
    true
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

/// The answer to a set: the one line. Every posture is flown by whichever mind holds the
/// unit (doc 45 unit AP.6), so a set says what was set and nothing more.
#[must_use]
pub fn said(template: &str, name: &str, posture: Posture) -> Vec<String> {
    vec![line(template, name, posture)]
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
    // A hired crew works its rig from aboard it and by no other right: the relay is for the
    // rig's own operator (the lessee or the title holder at the helm hull).
    if rig.crew.is_some() {
        return Err(CmdError::msg(POSTURE_CREW));
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

    use super::{POSTURE_READ, POSTURE_SET, line, ooc, said, word};
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
            POSTURE_LINE,
            super::POSTURE_ORDER_CREW,
            super::POSTURE_ORDER_USAGE,
            super::POSTURE_ORDER_ANSWERED,
            // With a leader's name of twenty characters in place of the placeholder.
            &super::POSTURE_ORDERED.replace("{leader}", "Twenty Characters Xx"),
            super::POSTURE_CREW,
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
            vec!["Rig 2 now holds GUARD.".to_owned()],
            "a posture the house mind flies needs no note"
        );
    }
}
