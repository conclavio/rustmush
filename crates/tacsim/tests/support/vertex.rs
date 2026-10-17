#![allow(dead_code)]
//! **The trade run's two-limit arithmetic, computed from data** (doc 18 cut IC8;
//! D-18-39; §4.3's integrality rule). No test stores a vertex: every load a test
//! needs at both of a hold's limits is solved here from the goods table and the
//! compartment, so a re-priced or re-measured good moves every test with it.
//!
//! A standalone file, included with `#[path = "support/vertex.rs"] mod vertex;`
//! (never declared in `support/mod.rs`, the heavy shipped-scale rig).

use mush_core::CanonicalId;
use tacsim::definitions::GoodsTable;
use tacsim::definitions::logistics::{MASS_KG, VOLUME_L};
use tacsim::port::capacity::Compartment;

fn limit(cargo: &Compartment, measure: &str) -> Option<u64> {
    cargo
        .limits
        .iter()
        .find(|(id, _)| id.as_str() == measure)
        .map(|(_, value)| *value)
}

fn per_unit(goods: &GoodsTable, good: &CanonicalId, measure: &str) -> Option<u64> {
    goods
        .good(good)?
        .measures
        .iter()
        .find(|(id, _)| id.as_str() == measure)
        .map(|(_, value)| *value)
}

/// **The load of `dense` and `bulky` that meets both the mass and the volume limit
/// exactly**, in whole units: with `M` the mass limit, `V` the volume limit and
/// `m`, `v` each good's per-unit figures, `bulky = (V·m_d − M·v_d) / (v_b·m_d − v_d·m_b)`
/// and `dense = (M − bulky·m_b) / m_d` (§4.3: `(V − M·v_dense)/(v_bulky − v_dense)` per
/// unit mass when both weigh a tonne a unit). `None` when the solution is not
/// integral or is negative.
pub fn both_limits_vertex(
    goods: &GoodsTable,
    cargo: &Compartment,
    dense: &CanonicalId,
    bulky: &CanonicalId,
) -> Option<(u64, u64)> {
    let big_m = i128::from(limit(cargo, MASS_KG)?);
    let big_v = i128::from(limit(cargo, VOLUME_L)?);
    let (md, vd) = (
        i128::from(per_unit(goods, dense, MASS_KG)?),
        i128::from(per_unit(goods, dense, VOLUME_L)?),
    );
    let (mb, vb) = (
        i128::from(per_unit(goods, bulky, MASS_KG)?),
        i128::from(per_unit(goods, bulky, VOLUME_L)?),
    );
    let denominator = vb * md - vd * mb;
    if denominator == 0 {
        return None;
    }
    let numerator = big_v * md - big_m * vd;
    if numerator % denominator != 0 {
        return None;
    }
    let b = numerator / denominator;
    let rest = big_m - b * mb;
    if md == 0 || rest % md != 0 {
        return None;
    }
    let d = rest / md;
    if b < 0 || d < 0 {
        return None;
    }
    Some((u64::try_from(d).ok()?, u64::try_from(b).ok()?))
}

/// **The best whole-unit load of the given goods** by exhaustive search (two goods:
/// at most 101² loads), maximizing Σ quantity × margin within every limit. Returns the
/// load (good → quantity, zeros included) and its value. Ties keep the first found.
pub fn best_integer_load(
    goods: &GoodsTable,
    cargo: &Compartment,
    margins: &[(CanonicalId, i64)],
) -> (Vec<(CanonicalId, u64)>, i64) {
    let ceilings: Vec<u64> = margins
        .iter()
        .map(|(good, _)| {
            cargo
                .limits
                .iter()
                .filter_map(|(measure, limit)| {
                    per_unit(goods, good, measure.as_str())
                        .filter(|per| *per > 0)
                        .map(|per| limit / per)
                })
                .min()
                .unwrap_or(0)
        })
        .collect();
    let mut best: (Vec<(CanonicalId, u64)>, i64) = (Vec::new(), i64::MIN);
    let mut counts = vec![0_u64; margins.len()];
    loop {
        let mut fits = true;
        for (measure, limit) in &cargo.limits {
            let total: u64 = margins
                .iter()
                .zip(&counts)
                .map(|((good, _), n)| per_unit(goods, good, measure.as_str()).unwrap_or(0) * n)
                .sum();
            if total > *limit {
                fits = false;
            }
        }
        if fits {
            let value: i64 = margins
                .iter()
                .zip(&counts)
                .map(|((_, margin), n)| margin * i64::try_from(*n).unwrap_or(i64::MAX))
                .sum();
            if value > best.1 {
                best = (
                    margins
                        .iter()
                        .zip(&counts)
                        .map(|((good, _), n)| (good.clone(), *n))
                        .collect(),
                    value,
                );
            }
        }
        // Next count vector (odometer).
        let mut i = 0;
        loop {
            if i == counts.len() {
                return best;
            }
            if counts[i] < ceilings[i] {
                counts[i] += 1;
                break;
            }
            counts[i] = 0;
            i += 1;
        }
    }
}
