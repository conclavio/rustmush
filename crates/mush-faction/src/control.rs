//! **Writing control** (doc 50 cut KC.0a, Q-50-50): the one function that seeds and flips a
//! region's control, for the lane's seed, for TacSim's typed calls, for a filled clock's
//! `on_complete_control`, and for staff `@faction/control`.
//!
//! The rule it holds: **de facto control is exclusive.** A de facto write moves the region's
//! prior de facto holder out, so a region has at most one. De jure, covert and economic stay
//! plural (doc 11 §5 uses them that way), one record a `(faction, kind)`.
//!
//! The record carries the IC date its hold began (`since_ic`), read from the world's clock
//! seam (`World::game_now_unix`), never the OS clock. A write that finds the same faction
//! already holding the same kind keeps that date: the hold goes on, it does not begin again.

use mush_core::Ctx;

use crate::data::{
    ControlKind, ControlRecord, FactionId, FactionState, RegionId, Visibility, persist_control,
};

/// One control write: `faction` holds `region` in `kind`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ControlWrite {
    /// The region.
    pub region: RegionId,
    /// The holder.
    pub faction: FactionId,
    /// The kind of hold.
    pub kind: ControlKind,
    /// The strength to set; `None` keeps an existing hold's, or 1 for a new one.
    pub strength: Option<u8>,
    /// The visibility to set; `None` keeps an existing hold's, or public for a new one.
    pub visibility: Option<Visibility>,
}

impl ControlWrite {
    /// A write with no strength or visibility given.
    #[must_use]
    pub fn new(region: RegionId, faction: FactionId, kind: ControlKind) -> ControlWrite {
        ControlWrite {
            region,
            faction,
            kind,
            strength: None,
            visibility: None,
        }
    }
}

/// What a control write did.
#[derive(Clone, Debug, PartialEq)]
pub struct ControlChange {
    /// The record as it now stands.
    pub record: ControlRecord,
    /// Whether the hold began at this write (`since_ic` is now). False when the faction already
    /// held the region in this kind.
    pub began: bool,
    /// The prior de facto holders this write moved out (empty unless `kind` is de facto; at
    /// most one on a world written only through this function).
    pub displaced: Vec<FactionId>,
}

/// Apply `w` to the registry at IC time `ic_now`. Answers the change and the region's records
/// after it, for the caller to persist.
pub(crate) fn apply(
    st: &mut FactionState,
    w: ControlWrite,
    ic_now: i64,
) -> Result<(ControlChange, Vec<ControlRecord>), String> {
    if !st.regions.contains_key(&w.region) {
        return Err(format!("No region #{}.", w.region));
    }
    if !st.factions.contains_key(&w.faction) {
        return Err(format!("No faction #{}.", w.faction));
    }
    let recs = st.control.entry(w.region).or_default();
    let mut displaced = Vec::new();
    if w.kind == ControlKind::DeFacto {
        recs.retain(|r| {
            let out = r.kind == ControlKind::DeFacto && r.faction != w.faction;
            if out {
                displaced.push(r.faction);
            }
            !out
        });
    }
    let (record, began) = if let Some(existing) = recs
        .iter_mut()
        .find(|r| r.faction == w.faction && r.kind == w.kind)
    {
        if let Some(s) = w.strength {
            existing.strength = s;
        }
        if let Some(v) = w.visibility {
            existing.visibility = v;
        }
        (existing.clone(), false)
    } else {
        let record = ControlRecord {
            region: w.region,
            faction: w.faction,
            kind: w.kind,
            strength: w.strength.unwrap_or(1),
            contested_by: Vec::new(),
            visibility: w.visibility.unwrap_or_default(),
            since_ic: ic_now,
        };
        recs.push(record.clone());
        (record, true)
    };
    let after = recs.clone();
    Ok((
        ControlChange {
            record,
            began,
            displaced,
        },
        after,
    ))
}

/// **Seed or flip control** (doc 50 cut KC.0a, Q-50-50): `w.faction` holds `w.region` in
/// `w.kind`, stamped with the IC date from the world's clock. A de facto write moves the prior
/// de facto holder out. Persists the region's control document. Writes no canon: a caller that
/// owes the world a line commits its own.
///
/// # Errors
///
/// The faction module is not loaded, or the region or faction does not exist. Nothing is
/// written then.
pub fn write_control(ctx: &mut Ctx<'_>, w: ControlWrite) -> Result<ControlChange, String> {
    let ic_now = ctx.w.game_now_unix();
    let st = ctx
        .w
        .module_state_mut::<FactionState>()
        .ok_or("Factions unavailable.")?;
    let (change, after) = apply(st, w, ic_now)?;
    persist_control(ctx.out, w.region, &after);
    Ok(change)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Faction, Region};

    fn state() -> FactionState {
        let mut st = FactionState::default();
        for (i, n) in ["Freeport", "Republic", "Co-op"].iter().enumerate() {
            let id = FactionId(i as u64);
            st.factions.insert(id, Faction::new(id, n));
        }
        st.regions.insert(
            RegionId(0),
            Region {
                id: RegionId(0),
                name: "Kirkwood approach".into(),
                note: String::new(),
                zones: Vec::new(),
                atlas: None,
            },
        );
        st
    }

    #[test]
    fn a_de_facto_flip_moves_the_prior_holder_out_and_dates_the_hold() {
        let mut st = state();
        let r = RegionId(0);
        let (a, _) = apply(
            &mut st,
            ControlWrite::new(r, FactionId(0), ControlKind::DeFacto),
            100,
        )
        .unwrap();
        assert!(a.began && a.displaced.is_empty());
        let (b, after) = apply(
            &mut st,
            ControlWrite::new(r, FactionId(1), ControlKind::DeFacto),
            200,
        )
        .unwrap();
        assert_eq!(b.displaced, vec![FactionId(0)]);
        assert_eq!(after.len(), 1, "one de facto holder a region");
        let holder = st.de_facto(r).unwrap();
        assert_eq!((holder.faction, holder.since_ic), (FactionId(1), 200));
    }

    #[test]
    fn de_jure_covert_and_economic_stay_plural() {
        let mut st = state();
        let r = RegionId(0);
        for kind in [
            ControlKind::DeJure,
            ControlKind::Covert,
            ControlKind::Economic,
        ] {
            for f in [FactionId(0), FactionId(1)] {
                let (c, _) = apply(&mut st, ControlWrite::new(r, f, kind), 5).unwrap();
                assert!(c.displaced.is_empty());
            }
        }
        // A de facto flip touches none of them.
        apply(
            &mut st,
            ControlWrite::new(r, FactionId(2), ControlKind::DeFacto),
            9,
        )
        .unwrap();
        assert_eq!(st.control[&r].len(), 7);
    }

    #[test]
    fn the_same_holder_keeps_its_date_and_takes_new_terms() {
        let mut st = state();
        let r = RegionId(0);
        apply(
            &mut st,
            ControlWrite::new(r, FactionId(1), ControlKind::DeFacto),
            100,
        )
        .unwrap();
        let w = ControlWrite {
            strength: Some(4),
            visibility: Some(Visibility::Covert),
            ..ControlWrite::new(r, FactionId(1), ControlKind::DeFacto)
        };
        let (c, _) = apply(&mut st, w, 900).unwrap();
        assert!(!c.began);
        assert_eq!(
            (c.record.since_ic, c.record.strength, c.record.visibility),
            (100, 4, Visibility::Covert)
        );
    }

    #[test]
    fn an_unknown_region_or_faction_writes_nothing() {
        let mut st = state();
        let bad_region = ControlWrite::new(RegionId(9), FactionId(0), ControlKind::DeFacto);
        assert!(apply(&mut st, bad_region, 1).is_err());
        let bad_faction = ControlWrite::new(RegionId(0), FactionId(9), ControlKind::DeFacto);
        assert!(apply(&mut st, bad_faction, 1).is_err());
        assert!(st.control.is_empty());
    }
}
