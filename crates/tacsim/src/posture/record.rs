//! **The posture record** (doc 45 unit AP.3; Q-45-23): one attribute on the unit's own
//! object, [`POSTURE_ATTR`], owned by `GOD`.
//!
//! **Why the unit's object and not its tactical record.** A unit's physical facts are a
//! `mush_tactics::unit::TacticalUnit`, and that record is closed while a host holds the
//! unit: a deployed unit refuses every rewrite but its check-in, and a check-in that voids
//! restores the snapshot taken at check-out. A posture is set exactly while a host holds the
//! unit (from its seat in a field, over the relay), so there it would be refused or lost. The
//! object's attribute is outside custody: a field, a bubble or a practice match checks the
//! unit out and in around it and never touches it. The same choice was made for a raider's
//! standing withdrawal (`crate::field::raiders::RAIDER_ATTR`), for the same reason.
//!
//! **Its shape.** `{"posture": "guard"}` or `{"posture": "hold"}`. A unit on `RETURN` has no
//! attribute at all, so every unit that existed before this record reads `RETURN` and no
//! stored format moves. An attribute `GOD` does not own is not the record: a player who
//! writes one on a hull they own has written nothing, and one who removes it has set
//! `RETURN`, which is the default they could always have.
//!
//! **One writer.** [`set`] is the only function that writes it. It also tells a field that
//! holds the unit (`crate::field::posture`), so the mind flying it reads the new word on its
//! next think. The verbs that call it are doc 45 unit AP.4's.

use mush_core::{Attr, Ctx, GOD, ObjId, World};

use super::Posture;

/// **The attribute that holds a unit's posture** (the module head).
pub const POSTURE_ATTR: &str = "TACSIM.POSTURE";

/// **The posture `unit` holds**: its record's, or `RETURN` for a unit with none.
#[must_use]
pub fn of(w: &World, unit: ObjId) -> Posture {
    w.attr(unit, POSTURE_ATTR)
        .filter(|attr| attr.owner == GOD)
        .and_then(|attr| attr.as_json())
        .and_then(|value| value.get("posture"))
        .and_then(serde_json::Value::as_str)
        .and_then(Posture::parse)
        .unwrap_or_default()
}

/// **Set `unit`'s posture** (the module head, "one writer"), and tell the field that holds
/// it. Answers whether the record changed.
///
/// # Errors
///
/// The unit is not an object of the world.
pub fn set(ctx: &mut Ctx<'_>, unit: ObjId, posture: Posture) -> Result<bool, String> {
    let changed = write(ctx.w, unit, posture)?;
    // Told whether or not the record moved: a field that missed the last word hears this one.
    crate::field::posture::sync(ctx);
    Ok(changed)
}

/// Write the record and nothing else: `RETURN` removes the attribute.
fn write(w: &mut World, unit: ObjId, posture: Posture) -> Result<bool, String> {
    if w.get(unit).is_none() {
        return Err(format!("#{} is not a unit", unit.0));
    }
    if of(w, unit) == posture {
        return Ok(false);
    }
    if posture.is_return() {
        w.remove_attr(unit, POSTURE_ATTR)
            .map_err(|error| error.to_string())?;
    } else {
        w.set_attr(
            unit,
            POSTURE_ATTR,
            Attr::json(serde_json::json!({ "posture": posture.as_str() }), GOD),
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use mush_core::{Attr, GOD, ObjId, ObjKind, World};

    use super::{POSTURE_ATTR, of, write};
    use crate::posture::Posture;

    fn world_with_a_unit() -> (World, ObjId) {
        let mut w = World::new_minimal();
        let unit = w
            .create(ObjKind::Thing, "Rig", GOD, None)
            .expect("a thing is made");
        (w, unit)
    }

    #[test]
    fn a_unit_with_no_record_holds_return_and_a_set_one_reads_back() {
        let (mut w, unit) = world_with_a_unit();
        assert_eq!(of(&w, unit), Posture::Return);
        assert!(w.attr(unit, POSTURE_ATTR).is_none());
        for posture in [Posture::Guard, Posture::Hold] {
            assert_eq!(write(&mut w, unit, posture), Ok(true));
            assert_eq!(of(&w, unit), posture);
            assert_eq!(write(&mut w, unit, posture), Ok(false), "nothing to change");
        }
        // The default spends no attribute: a unit set back to it is as it was made.
        assert_eq!(write(&mut w, unit, Posture::Return), Ok(true));
        assert_eq!(of(&w, unit), Posture::Return);
        assert!(w.attr(unit, POSTURE_ATTR).is_none());
        assert!(write(&mut w, ObjId(9_999), Posture::Hold).is_err());
    }

    #[test]
    fn an_attribute_god_does_not_own_or_cannot_read_is_not_the_record() {
        let (mut w, unit) = world_with_a_unit();
        let player = w
            .create(ObjKind::Player, "Forger", GOD, None)
            .expect("a player is made");
        w.set_attr(
            unit,
            POSTURE_ATTR,
            Attr::json(serde_json::json!({ "posture": "hold" }), player),
        )
        .expect("the attribute is written");
        assert_eq!(of(&w, unit), Posture::Return, "a forged record is none");
        for junk in [
            serde_json::json!({ "posture": "flee" }),
            serde_json::json!({ "posture": 2 }),
            serde_json::json!("hold"),
        ] {
            w.set_attr(unit, POSTURE_ATTR, Attr::json(junk, GOD))
                .expect("the attribute is written");
            assert_eq!(of(&w, unit), Posture::Return);
        }
        w.set_attr(unit, POSTURE_ATTR, Attr::text("hold", GOD))
            .expect("the attribute is written");
        assert_eq!(of(&w, unit), Posture::Return, "text is not the record");
    }
}
