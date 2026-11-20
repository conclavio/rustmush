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

/// **Make a region from content, or find it** (doc 50 cut KC.0, Q-50-3): the region named
/// `name`, made when missing. Answers its id and whether this call made it, so a seed writes
/// a region's world-start holders once and never over a later flip. Persists the region and
/// the id allocator when it makes one.
///
/// # Errors
///
/// The faction module is not loaded, or the name is not a usable region name (the
/// `@region/create` rule: 1 to [`MAX_NAME_LEN`](crate::MAX_NAME_LEN) letters, digits, space
/// and `- _ ' .`).
pub fn ensure_region(ctx: &mut Ctx<'_>, name: &str) -> Result<(RegionId, bool), String> {
    let name = name.trim();
    if !crate::commands::valid_name(name) {
        return Err(format!("{name:?} is not a usable region name."));
    }
    let st = ctx
        .w
        .module_state_mut::<FactionState>()
        .ok_or("Factions unavailable.")?;
    if let Some(r) = st.region_by_name(name) {
        return Ok((r.id, false));
    }
    let id = st.alloc_region_id();
    let r = crate::data::Region {
        id,
        name: smol_str::SmolStr::new(name),
        note: String::new(),
        zones: Vec::new(),
        atlas: None,
    };
    st.regions.insert(id, r.clone());
    let (next_f, next_r) = (st.next_faction_id, st.next_region_id);
    crate::data::persist_region(ctx.out, &r);
    crate::data::persist_meta(ctx.out, next_f, next_r);
    Ok((id, true))
}

/// **Make a faction from content, or find it** (doc 50 cut KC.0, Q-50-2): the faction named
/// `name`, made when missing with no backing organization and no principal. A polity is one.
///
/// # Errors
///
/// The faction module is not loaded, the name is not a usable faction name (the
/// `@faction/create` rule), or the registry is full.
pub fn ensure_faction(ctx: &mut Ctx<'_>, name: &str) -> Result<FactionId, String> {
    let name = name.trim();
    if !crate::commands::valid_name(name) {
        return Err(format!("{name:?} is not a usable faction name."));
    }
    let st = ctx
        .w
        .module_state_mut::<FactionState>()
        .ok_or("Factions unavailable.")?;
    if let Some(f) = st.faction_by_name(name) {
        return Ok(f.id);
    }
    if st.factions.len() >= 4096 {
        return Err("Faction registry is full.".into());
    }
    let id = st.alloc_faction_id();
    let f = crate::data::Faction::new(id, name);
    st.factions.insert(id, f.clone());
    let (next_f, next_r) = (st.next_faction_id, st.next_region_id);
    crate::data::persist_faction(ctx.out, &f);
    crate::data::persist_meta(ctx.out, next_f, next_r);
    Ok(id)
}

/// **Mark a hold contested** (doc 50 cut KC.0; doc 11 §5's "Republic pickets contest"):
/// `challenger` contests `holder`'s `kind` hold on `region`, as `@faction/contest` does but
/// never toggling it off. Answers whether this call added the contest.
///
/// # Errors
///
/// The faction module is not loaded, the hold does not exist, the challenger is no faction,
/// or a faction would contest its own hold.
pub fn contest_control(
    ctx: &mut Ctx<'_>,
    region: RegionId,
    holder: FactionId,
    kind: ControlKind,
    challenger: FactionId,
) -> Result<bool, String> {
    let st = ctx
        .w
        .module_state_mut::<FactionState>()
        .ok_or("Factions unavailable.")?;
    if challenger == holder || !st.factions.contains_key(&challenger) {
        return Err("A contest names another faction.".into());
    }
    let recs = st.control.get_mut(&region).ok_or("No such hold.")?;
    let rec = recs
        .iter_mut()
        .find(|r| r.faction == holder && r.kind == kind)
        .ok_or("No such hold.")?;
    if rec.contested_by.contains(&challenger) {
        return Ok(false);
    }
    rec.contested_by.push(challenger);
    let after = recs.clone();
    persist_control(ctx.out, region, &after);
    Ok(true)
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
