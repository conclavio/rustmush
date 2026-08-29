//! Derived-stat computation — the finalize step the SR5 schema documents.
//!
//! SR5's Limits and Condition Monitors need `ceil(x/2)` / `ceil(x/3)`, which
//! [`mush_ruleset::DerivedExpr`] cannot express. These pure helpers do the
//! arithmetic; [`apply_derived`] writes the results into the sheet's stored
//! Number fields ([`PHYS_LIMIT`](crate::schema::PHYS_LIMIT), …) and resizes the
//! two Condition-Monitor tracks to match the recomputed box counts.
//!
//! **The engine must call [`apply_derived`] after any change to an attribute
//! or Essence** (chargen finalize, `@sheet/set`, `+harm` box changes never
//! resize, but attribute buys do). It preserves existing damage marks up to
//! the smaller of the old/new box counts. The sheet stays valid at all times:
//! a Track's length always equals its box-count field, whatever the
//! attributes say, because the box-count field is what the Track's
//! `TrackSize::Derived` reads.

use mush_ruleset::SheetDoc;

use crate::schema::{
    CONDITION_PHYSICAL, CONDITION_STUN, ESSENCE, INITIATIVE_BASE, MENTAL_LIMIT, PHYS_CM_MAX,
    PHYS_LIMIT, SOCIAL_LIMIT, STUN_CM_MAX,
};

/// Integer ceiling of `a / b` for `b > 0`, `a >= 0`.
#[must_use]
pub fn ceil_div(a: i64, b: i64) -> i64 {
    if b <= 0 {
        return 0;
    }
    (a + b - 1) / b
}

/// Physical Limit = `ceil((STR·2 + BOD + REA) / 3)`.
#[must_use]
pub fn physical_limit(body: i64, reaction: i64, strength: i64) -> i64 {
    ceil_div(strength * 2 + body + reaction, 3)
}

/// Mental Limit = `ceil((LOG·2 + INT + WIL) / 3)`.
#[must_use]
pub fn mental_limit(logic: i64, intuition: i64, willpower: i64) -> i64 {
    ceil_div(logic * 2 + intuition + willpower, 3)
}

/// Social Limit = `ceil((CHA·2 + WIL + ⌊ESS⌉) / 3)`.
#[must_use]
pub fn social_limit(charisma: i64, willpower: i64, essence_rounded: i64) -> i64 {
    ceil_div(charisma * 2 + willpower + essence_rounded, 3)
}

/// Physical Condition Monitor box count = `8 + ceil(BOD / 2)`.
#[must_use]
pub fn physical_cm(body: i64) -> i64 {
    8 + ceil_div(body, 2)
}

/// Stun Condition Monitor box count = `8 + ceil(WIL / 2)`.
#[must_use]
pub fn stun_cm(willpower: i64) -> i64 {
    8 + ceil_div(willpower, 2)
}

/// Initiative base = Reaction + Intuition (physical initiative; the 1D6+
/// initiative dice ride the separate `initiative_dice` field).
#[must_use]
pub fn initiative_base(reaction: i64, intuition: i64) -> i64 {
    reaction + intuition
}

/// Essence in whole points (hundredths → float), e.g. `545 → 5.45`.
#[must_use]
pub fn essence_points(essence_hundredths: i64) -> f64 {
    essence_hundredths as f64 / 100.0
}

/// Essence rounded to the nearest whole point (half up), for the Social Limit.
#[must_use]
pub fn essence_rounded(essence_hundredths: i64) -> i64 {
    (essence_hundredths + 50) / 100
}

/// A read attribute (defaulting to 1 for a fresh core attribute).
fn attr(doc: &SheetDoc, id: &str) -> i64 {
    doc.number(id).unwrap_or(1)
}

/// A snapshot of the three Limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Physical Limit.
    pub physical: i64,
    /// Mental Limit.
    pub mental: i64,
    /// Social Limit.
    pub social: i64,
}

/// Compute the three Limits from a sheet's current attributes + Essence.
#[must_use]
pub fn limits(doc: &SheetDoc) -> Limits {
    let ess = essence_rounded(doc.number(ESSENCE).unwrap_or(600));
    Limits {
        physical: physical_limit(
            attr(doc, "body"),
            attr(doc, "reaction"),
            attr(doc, "strength"),
        ),
        mental: mental_limit(
            attr(doc, "logic"),
            attr(doc, "intuition"),
            attr(doc, "willpower"),
        ),
        social: social_limit(attr(doc, "charisma"), attr(doc, "willpower"), ess),
    }
}

/// A snapshot of both Condition Monitors: box counts, filled boxes, and the
/// total wound modifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConditionMonitors {
    /// Physical CM box count.
    pub physical_max: i64,
    /// Stun CM box count.
    pub stun_max: i64,
    /// Filled physical boxes.
    pub physical_filled: i64,
    /// Filled stun boxes.
    pub stun_filled: i64,
    /// The combined wound modifier (see [`wound_modifier`]).
    pub wound_modifier: i64,
}

/// Filled (non-`null`) boxes on a track field.
fn filled_boxes(doc: &SheetDoc, field: &str) -> i64 {
    doc.track(field)
        .map(|marks| marks.iter().filter(|m| m.is_some()).count() as i64)
        .unwrap_or(0)
}

/// The SR5 wound modifier applied to dice pools: `−⌊phys/3⌋ − ⌊stun/3⌋`, the
/// sum of the injury modifiers from both Condition Monitors (SR5: −1 per three
/// boxes on *either* track, cumulative). Returns a value `<= 0`.
#[must_use]
pub fn wound_modifier(doc: &SheetDoc) -> i64 {
    let phys = filled_boxes(doc, CONDITION_PHYSICAL);
    let stun = filled_boxes(doc, CONDITION_STUN);
    -(phys / 3) - (stun / 3)
}

/// Compute the Condition-Monitor snapshot from the sheet's current state.
#[must_use]
pub fn condition_monitors(doc: &SheetDoc) -> ConditionMonitors {
    let physical_filled = filled_boxes(doc, CONDITION_PHYSICAL);
    let stun_filled = filled_boxes(doc, CONDITION_STUN);
    ConditionMonitors {
        physical_max: physical_cm(attr(doc, "body")),
        stun_max: stun_cm(attr(doc, "willpower")),
        physical_filled,
        stun_filled,
        wound_modifier: -(physical_filled / 3) - (stun_filled / 3),
    }
}

/// Resize a track field to `new_len`, preserving existing marks up to
/// `min(old, new)` and padding with empty (`null`) boxes.
fn resize_track(doc: &mut SheetDoc, field: &str, new_len: usize) {
    let mut arr = doc
        .get(field)
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    if arr.len() > new_len {
        arr.truncate(new_len);
    } else {
        arr.resize(new_len, serde_json::Value::Null);
    }
    doc.set(field, serde_json::Value::Array(arr));
}

/// Recompute every attribute-derived stat and store it: the three Limits,
/// Initiative base, the two Condition-Monitor box counts, and (resizing the
/// tracks) the CM box arrays. Idempotent; safe to call after any sheet edit.
///
/// Leaves damage marks, Essence, Magic, Power Points, and Edge untouched —
/// those are the engine's / chargen's to set. Call this **before** persisting
/// a sheet whose attributes changed so the stored derived fields and CM track
/// sizes stay consistent.
pub fn apply_derived(doc: &mut SheetDoc) {
    let lim = limits(doc);
    doc.set_number(PHYS_LIMIT, lim.physical);
    doc.set_number(MENTAL_LIMIT, lim.mental);
    doc.set_number(SOCIAL_LIMIT, lim.social);

    doc.set_number(
        INITIATIVE_BASE,
        initiative_base(attr(doc, "reaction"), attr(doc, "intuition")),
    );

    let phys_cm = physical_cm(attr(doc, "body"));
    let stun_cm_n = stun_cm(attr(doc, "willpower"));
    doc.set_number(PHYS_CM_MAX, phys_cm);
    doc.set_number(STUN_CM_MAX, stun_cm_n);
    resize_track(doc, CONDITION_PHYSICAL, phys_cm.max(0) as usize);
    resize_track(doc, CONDITION_STUN, stun_cm_n.max(0) as usize);
}

/// Total Essence spent by augmentations (sum of the `augmentations` map
/// values, in hundredths), and the resulting Essence (`600 − spent`, floored
/// at 0). A convenience for the engine's Essence accounting; does not mutate.
#[must_use]
pub fn essence_from_augmentations(doc: &SheetDoc) -> (i64, i64) {
    let spent: i64 = doc
        .map(crate::schema::AUGMENTATIONS_FIELD)
        .map(|m| m.values().filter_map(serde_json::Value::as_i64).sum())
        .unwrap_or(0);
    (spent, (600 - spent).max(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{CONDITION_PHYSICAL, build_schema};
    use mush_ruleset::{SheetDoc, validate};
    use smol_str::SmolStr;

    fn sheet() -> SheetDoc {
        SheetDoc::new_default(&build_schema("shadowrun"))
    }

    #[test]
    fn ceil_and_limit_formulas() {
        assert_eq!(ceil_div(4, 3), 2);
        assert_eq!(ceil_div(9, 3), 3);
        assert_eq!(ceil_div(1, 2), 1);
        // A sample street samurai: STR 5, BOD 6, REA 5 → ceil((10+6+5)/3)=7.
        assert_eq!(physical_limit(6, 5, 5), 7);
        // LOG 3, INT 4, WIL 5 → ceil((6+4+5)/3)=5.
        assert_eq!(mental_limit(3, 4, 5), 5);
        // CHA 4, WIL 5, ESS 6 → ceil((8+5+6)/3)=ceil(19/3)=7.
        assert_eq!(social_limit(4, 5, 6), 7);
    }

    #[test]
    fn condition_monitor_box_counts() {
        // BOD 6 → 8 + ceil(6/2) = 11; WIL 5 → 8 + ceil(5/2) = 11.
        assert_eq!(physical_cm(6), 11);
        assert_eq!(stun_cm(5), 11);
        // BOD 1 → 9 (the default).
        assert_eq!(physical_cm(1), 9);
    }

    #[test]
    fn essence_encoding() {
        assert_eq!(essence_rounded(600), 6);
        assert_eq!(essence_rounded(545), 5);
        assert_eq!(essence_rounded(550), 6);
        assert!((essence_points(545) - 5.45).abs() < 1e-9);
    }

    #[test]
    fn apply_derived_resizes_tracks_and_keeps_valid() {
        let schema = build_schema("shadowrun");
        let mut doc = SheetDoc::new_default(&schema);
        doc.set_number("body", 6);
        doc.set_number("willpower", 5);
        doc.set_number("reaction", 5);
        doc.set_number("intuition", 4);
        doc.set_number("strength", 5);
        doc.set_number("charisma", 4);
        doc.set_number("logic", 3);
        apply_derived(&mut doc);

        assert_eq!(doc.number(PHYS_CM_MAX), Some(11));
        assert_eq!(doc.track(CONDITION_PHYSICAL).unwrap().len(), 11);
        assert_eq!(doc.number(PHYS_LIMIT), Some(7));
        assert_eq!(doc.number(MENTAL_LIMIT), Some(5));
        assert_eq!(doc.number(SOCIAL_LIMIT), Some(7));
        assert_eq!(doc.number(INITIATIVE_BASE), Some(9));
        // Still schema-valid after the resize.
        assert!(validate(&schema, &doc.data).is_ok());
    }

    #[test]
    fn apply_derived_preserves_existing_damage() {
        let mut doc = sheet();
        // Mark 3 physical boxes, then grow the track.
        doc.set_track(
            CONDITION_PHYSICAL,
            &[
                Some(SmolStr::new("physical")),
                Some(SmolStr::new("physical")),
                Some(SmolStr::new("physical")),
                None,
                None,
                None,
                None,
                None,
                None,
            ],
        );
        doc.set_number("body", 8); // 8 + ceil(8/2) = 12 boxes
        apply_derived(&mut doc);
        let marks = doc.track(CONDITION_PHYSICAL).unwrap();
        assert_eq!(marks.len(), 12);
        assert_eq!(marks.iter().filter(|m| m.is_some()).count(), 3);
        // −1 per 3 boxes: 3 filled → −1.
        assert_eq!(wound_modifier(&doc), -1);
    }

    /// A track of `len` boxes with the first `filled` marked.
    fn marks(filled: usize, len: usize) -> Vec<Option<SmolStr>> {
        (0..len)
            .map(|i| (i < filled).then(|| SmolStr::new("x")))
            .collect()
    }

    #[test]
    fn wound_modifier_sums_both_tracks() {
        let mut doc = sheet();
        doc.set_track(CONDITION_PHYSICAL, &marks(3, 9));
        doc.set_track(CONDITION_STUN, &marks(6, 9));
        // −⌊3/3⌋ − ⌊6/3⌋ = −1 − 2 = −3.
        assert_eq!(wound_modifier(&doc), -3);
    }
}
