//! **The port audit** (doc 50 unit LW.3; Q-50-36 and Q-50-65 as ruled, Q-50-69's chances, doc
//! 50a A-6a, D-4, A-4, B-1).
//!
//! At a port whose law restricts a good (the de facto holder's terms, `crate::port::law`), a
//! docked hull's hold is audited once a docking, by chance. Holding a restricted good without its
//! paper there is itself an offense, and an audit is the only way it is found (Q-50-65).
//!
//! - **When**: on the port tick after the hull docks, because the arrival hook cannot read a
//!   hold. A docking is the hull, its port and the IC second the tick first sees her there
//!   ([`AuditBook`]).
//! - **The chance** ([`chance_permille`], Q-50-69, provisional): by the band of the hull's
//!   answerer's standing with the port's holder: never when trusted, one in eight when friendly,
//!   one in four when neutral, one in two when unfriendly, always when hostile. An organization
//!   has no personal standing and is audited at the neutral chance.
//! - **The roll** ([`roll_permille`]): a keyed hash of the hull, the port and the docking, as the
//!   survey deposits are keyed, so a docking rolls once and a story can pin it.
//! - **The reading**: CR's one cargo reading (`crate::port::ship::hold_reading`), lots as well as
//!   the pool.
//! - **The finding**: an [`OffenseKind::UnlicensedPossession`] under the port's holder, against
//!   the hull's answerer (`crate::offense::answerer_for_hull`), fined at once
//!   (`crate::surface::security::fine_now`) by the tonne. The goods stay aboard.
//! - **Staff** may order an audit of a docked hull at once (`+law/audit #<hull>`), whatever the
//!   roll.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ObjId, World};

use crate::offense::{Evidence, Offender, OffenseKey, OffenseKind, Proof, ProvenOffense};

/// **The audit chance by standing band**, per mille (Q-50-69, provisional).
#[must_use]
pub fn chance_permille(band: &str) -> u16 {
    match band {
        "trusted" => 0,
        "friendly" => 125,
        "unfriendly" => 500,
        "hostile" => 1_000,
        _ => 250,
    }
}

/// **A docking's roll**, `0..1000`: a keyed hash of the hull, the port and the docking's IC
/// second.
#[must_use]
pub fn roll_permille(hull: ObjId, entry: &CanonicalId, since_ic: i64) -> u16 {
    let Ok(hash) = mush_core::CanonicalHash::for_canonical_value(
        "tacsim.law.audit.v1",
        1,
        &serde_json::json!({ "hull": hull.0, "port": entry.as_str(), "since": since_ic }),
        256,
    ) else {
        return 999;
    };
    let b = hash.as_bytes();
    u16::from_le_bytes([b[0], b[1]]) % 1_000
}

/// **The dockings the tick has seen**: each docked hull's port and the IC second it was first
/// seen there. RAM only: a restart sees each docked hull afresh, and rolls her again.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditBook {
    /// Hull → (port entry, first seen IC).
    pub seen: BTreeMap<ObjId, (CanonicalId, i64)>,
}

/// **What an audit found**: the goods and units held without their paper, and whether an offense
/// was recorded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuditFinding {
    /// `(good, units)` held without the paper.
    pub found: Vec<(CanonicalId, u64)>,
    /// Whether an offense was recorded (a new one; an audit of the same docking records none).
    pub recorded: bool,
}

/// The chance of an audit of `hull` at `entry`, per mille: by her answerer's standing with the
/// port's holder.
fn chance_for(w: &World, hull: ObjId, holder: mush_faction::FactionId) -> u16 {
    let band = match crate::offense::answerer_for_hull(w, hull) {
        Some(Offender::Character(who)) => {
            mush_faction::public_standing(w, holder, who).map(|v| v.band)
        }
        _ => None,
    };
    chance_permille(band.unwrap_or("neutral"))
}

/// **The port tick's audits** (Q-50-36): each hull newly seen docked at a port whose law
/// restricts a good rolls once, and is audited on a roll under her chance.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let now = ctx.w.world_ic_now().at;
    let mut docked: BTreeMap<ObjId, CanonicalId> = BTreeMap::new();
    for hull in crate::port::berth::hulls(ctx.w) {
        if let crate::port::ship::Berth::Docked { map, def } =
            crate::port::ship::berth_of(ctx.w, hull)
            && def.site.is_none()
        {
            docked.insert(hull, map.entry.clone());
        }
    }
    let mut due = Vec::new();
    if let Some(store) = ctx.w.module_state_mut::<crate::store::TacsimStore>() {
        store
            .audits
            .seen
            .retain(|hull, (entry, _)| docked.get(hull) == Some(entry));
        for (hull, entry) in &docked {
            if !store.audits.seen.contains_key(hull) {
                store.audits.seen.insert(*hull, (entry.clone(), now));
                due.push((*hull, entry.clone(), now));
            }
        }
    }
    for (hull, entry, since) in due {
        let Some(terms) = crate::polity::terms_at(ctx.w, &entry) else {
            continue;
        };
        let (Some(holder), Some(row)) = (terms.holder, terms.row) else {
            continue;
        };
        if row.restricted.is_empty() {
            continue;
        }
        let chance = chance_for(ctx.w, hull, mush_faction::FactionId(holder));
        if roll_permille(hull, &entry, since) < chance {
            let _ = audit(ctx, hull, &entry, since);
        }
    }
}

/// **Audit `hull`, docked at `entry`** (Q-50-65): every good the port's law restricts that her
/// hold holds and her answerer has no paper for is found; one offense a good is recorded under
/// the port's holder and fined at once. `docking` keys the finding, so one docking's audit records
/// a finding once.
pub(crate) fn audit(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    entry: &CanonicalId,
    docking: i64,
) -> AuditFinding {
    let mut out = AuditFinding::default();
    let Some(terms) = crate::polity::terms_at(ctx.w, entry) else {
        return out;
    };
    let (Some(holder), Some(row)) = (terms.holder, terms.row) else {
        return out;
    };
    let Some(offender) = crate::offense::answerer_for_hull(ctx.w, hull) else {
        return out;
    };
    let now = ctx.w.world_ic_now().at;
    let clear = |w: &World, paper: &CanonicalId| match offender {
        Offender::Character(who) => {
            paper.as_str() == crate::port::law::PICKET_PAPERS
                && crate::port::law::papers_clear(w, who, now)
        }
        Offender::Organization(_) | Offender::Account(_) => false,
    };
    let key = OffenseKey {
        offender,
        polity: mush_faction::FactionId(holder),
    };
    for restricted in &row.restricted {
        let units = crate::port::ship::hold_reading(ctx, hull, &restricted.good).unwrap_or(0);
        if units == 0 || clear(ctx.w, &restricted.paper) {
            continue;
        }
        out.found.push((restricted.good.clone(), units));
        let offense = ProvenOffense {
            kind: OffenseKind::UnlicensedPossession,
            evidence: Evidence::Audit {
                hull,
                entry: entry.clone(),
                good: restricted.good.clone(),
                units,
                at_ic: docking,
            },
            proof: Proof::Audit,
            proven_ic: now,
            debt: None,
            payout: None,
        };
        out.recorded |=
            crate::offense::record_offense(ctx, key, offense, |_, row| row.debt.is_some());
    }
    if out.recorded {
        let told = match offender {
            Offender::Character(who) => Some(who),
            Offender::Organization(_) | Offender::Account(_) => {
                crate::port::coop::helm_holder(ctx.w, hull)
            }
        };
        crate::surface::security::fine_now(ctx, &key, entry, told);
        // Doc 50 unit LW.4 (Q-50-69): nothing when paid at once, else a loss.
        crate::port::record::lose_standing(
            ctx,
            &key,
            |row| matches!(&row.evidence, Evidence::Audit { hull: h, at_ic, .. } if *h == hull && *at_ic == docking),
        );
    }
    out
}

/// **`+law/audit #<hull>`** (staff): audit a docked hull now, whatever the roll; the answer.
pub(crate) fn audit_now(ctx: &mut Ctx<'_>, hull: ObjId) -> String {
    let crate::port::ship::Berth::Docked { map, .. } = crate::port::ship::berth_of(ctx.w, hull)
    else {
        return format!("#{} is not docked at a port.", hull.0);
    };
    let entry = map.entry.clone();
    let docking = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|s| s.audits.seen.get(&hull).cloned())
        .filter(|(e, _)| *e == entry)
        .map_or_else(|| ctx.w.world_ic_now().at, |(_, since)| since);
    let finding = audit(ctx, hull, &entry, docking);
    if finding.found.is_empty() {
        format!(
            "The audit of #{} finds nothing restricted held without its papers.",
            hull.0
        )
    } else {
        let goods: Vec<String> = finding
            .found
            .iter()
            .map(|(g, n)| format!("{n} {g}"))
            .collect();
        format!(
            "The audit of #{} finds {} held without its papers{}.",
            hull.0,
            goods.join(", "),
            if finding.recorded {
                "; it is recorded and fined"
            } else {
                "; it was already recorded"
            }
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 50 unit LW.3: the chance and the roll** (Q-50-36, Q-50-69): the bands' chances,
    /// and a docking's roll is fixed by the docking and moves with it.
    #[test]
    fn lw3_the_audit_chance_by_band_and_a_keyed_roll() {
        assert_eq!(
            ["trusted", "friendly", "neutral", "unfriendly", "hostile"].map(chance_permille),
            [0, 125, 250, 500, 1_000]
        );
        let port = CanonicalId::new("ceres_port").expect("id");
        let a = roll_permille(ObjId(7), &port, 100);
        assert!(a < 1_000);
        assert_eq!(
            a,
            roll_permille(ObjId(7), &port, 100),
            "one docking rolls once"
        );
        let rolls: std::collections::BTreeSet<u16> =
            (0..64).map(|t| roll_permille(ObjId(7), &port, t)).collect();
        assert!(rolls.len() > 32, "the roll moves with the docking");
    }
}
