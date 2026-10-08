//! **The centre's compute pool** (17b cut M3.5; doc 17 §9 "The pool", §12;
//! D-UM-36, D-UM-37).
//!
//! Each sim task has a deterministic compute allotment, the engine's
//! guarantee that no purchase can break the lens: [`pool_for`] is
//! `max(floor, base − per_unit × units)` over `[tacsim.minds]`'s three pool
//! keys (stock 500,000, 1,000 and 120,000 ops a sim second: 200,000 at 300
//! units, 476,000 at 24, 120,000 from 380 units up). **Admission counts class
//! ceilings, never measured use**: a hull's load is its mind's class's ceiling
//! per sim second ([`load_of`], `crate::arena::core::ClassTable::ceiling_per_s`
//! — 400, 1,500, 6,000 and 18,000 on the stock table), and a hull no mind will
//! run on loads nothing.
//!
//! **In the arena** the pool is split evenly between the two sides
//! ([`admit`]'s `half`) and spoken for first-come within a side, in slot order.
//! It is checked at the choice — `SIM UNIT` (`crate::match_::lobby::unit_type`,
//! [`crate::match_::lobby::SimError::Pool`]) — and, by name, as the LAST gate
//! of check-out's every-refusal block (`crate::match_::start`,
//! [`crate::match_::StartError::Pool`]), so `BOARD` never fails because of a
//! core and a refused roster writes nothing. Both refuse with the arithmetic
//! ([`PoolRefusal`]): how much the side would think a second, its half, and
//! what is spoken for.
//!
//! **Who loads what before 17b cut M5** (the lead's instruction): the AI
//! fill's hull at `[tacsim.minds].fill_class` — Class I, 1,500 each, from 17b
//! cut M4.3 (Class 0's 400 before it) — until M5 lets
//! the lead set the fill's class (D-UM-37); a seated pilot's hull nothing,
//! until M5's `SIM MIND` names a mind for it; a fixture (or M5) may name a
//! class on the fielded unit (`crate::match_::Fielded::mind_class`).
//!
//! **At the stock constants the arena's check-out gate cannot refuse**
//! (declared for doc 17 §24): `crate::config::MatchConfig::max_slots_per_side`
//! is validated 1 to 12 and `StartError::TooManyUnits` is the first gate, so a
//! side is at most twelve hulls; twelve Class III a side load 12 × 18,000 =
//! 216,000 against a half of 476,000 / 2 = 238,000 at 24 units — doc 17 §9's
//! "a 12v12 of Class III takes 432,000 and fits" — and a thirteenth is
//! `TooManyUnits`. The gate bites only under an operator's lowered pool
//! (`tests/match_lifecycle.rs`' over-subscribed fixture runs at
//! `pool_base_ops_s = 300_000`). Nor can `SIM UNIT` refuse before 17b cut M5:
//! every hull a pilot chooses for is seated, and loads nothing — and since
//! 17b cut M4.3 the side's unclaimed pods under the AI fill are spoken for at
//! `fill_class` there too, a choice that adds nothing never refused.
//!
//! **Doc 17 §9's recount of the harness's 300 machines** (all at Class 0
//! take 120,000 of a 200,000 pool) is a per-TASK sum over the hulls the
//! harness fields through `crate::match_::custody_unit`, never through
//! `crate::match_::start`: the harness has no sides and no lobby, and nothing
//! here admits it.

use crate::arena::core::{ClassTable, CoreClass};
use crate::config::MindsConfig;

/// **A task's allotment, ops a sim second** (D-UM-36):
/// `max(floor, base − per_unit × units)`, saturating — 200,000 at 300 units,
/// 476,000 at 24 and 120,000 at 500 on the stock keys.
#[must_use]
pub fn pool_for(cfg: &MindsConfig, units: usize) -> u64 {
    let units = u64::try_from(units).unwrap_or(u64::MAX);
    cfg.pool_base_ops_s
        .saturating_sub(cfg.pool_per_unit_ops_s.saturating_mul(units))
        .max(cfg.pool_floor_ops_s)
}

/// **One hull's load, ops a sim second**: nothing for `None` (a hull no mind
/// will run on — a seated pilot's, until 17b cut M5's `SIM MIND` names a
/// mind), else the class's ceiling ([`ClassTable::ceiling_per_s`]).
#[must_use]
pub fn load_of(classes: &ClassTable, mind: Option<CoreClass>) -> u64 {
    mind.map_or(0, |class| classes.ceiling_per_s(class))
}

/// **The class a fielded hull is admitted at** — `chosen` where a fixture (or
/// 17b cut M5) named one, else the fill rule: nothing on a seated hull (M5's
/// gap, declared: a seated hull is sealed at `fill_class` too, so a release
/// adds no unadmitted load), and `fill_class` — `[tacsim.minds].fill_class`,
/// 17b cut M4.3 — on an unseated one, which the fill flies.
#[must_use]
pub const fn admitted_class(
    chosen: Option<CoreClass>,
    seated: bool,
    fill_class: CoreClass,
) -> Option<CoreClass> {
    match chosen {
        Some(class) => Some(class),
        None if seated => None,
        None => Some(fill_class),
    }
}

/// **A side the pool cannot carry** (D-UM-36): the arithmetic, every figure
/// with a thousands separator ([`thousands`]) — doc 17 §9's sentence.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "This side would think {} ops a second; its half of the centre's pool is {} and {} are spoken \
     for.",
    thousands(*.would),
    thousands(*.half),
    thousands(*.spoken)
)]
pub struct PoolRefusal {
    /// What the hull being admitted would think, ops a sim second.
    pub would: u64,
    /// The side's half of the pool.
    pub half: u64,
    /// What the side's hulls before it already speak for.
    pub spoken: u64,
}

/// **Admit one hull's `would` against a side's `spoken`** under `pool`
/// (D-UM-36): the side's half is `pool / 2`, and the hull is refused when
/// `spoken + would` passes it — a side that reaches its half exactly is
/// admitted.
///
/// # Errors
///
/// [`PoolRefusal`] with the three figures.
pub const fn admit(pool: u64, spoken: u64, would: u64) -> Result<(), PoolRefusal> {
    let half = pool / 2;
    if spoken.saturating_add(would) > half {
        return Err(PoolRefusal {
            would,
            half,
            spoken,
        });
    }
    Ok(())
}

/// **One side, first-come** (D-UM-36): every load in slot order admitted
/// against what the loads before it speak for, under `pool`.
///
/// # Errors
///
/// The first [`PoolRefusal`].
pub fn admit_side(pool: u64, loads: impl IntoIterator<Item = u64>) -> Result<(), PoolRefusal> {
    let mut spoken: u64 = 0;
    for would in loads {
        admit(pool, spoken, would)?;
        spoken = spoken.saturating_add(would);
    }
    Ok(())
}

/// **`n` with a comma every three digits** (`1,500`, `18,000`): how a player
/// line writes ops — the pool's sentence and the tripped sentence
/// (`crate::arena_bridge`) alike.
#[must_use]
pub fn thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len().saturating_add(digits.len() / 3));
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && digits.len().saturating_sub(at) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **17b cut M3.5** (doc 17 §9, D-UM-36): the allotment at doc 17's three
    /// counts — 200,000 at 300 units, 476,000 at 24, the floor's 120,000 at
    /// 500 — and saturating past every count.
    #[test]
    fn m3_the_pool_is_doc_17_s_figure_at_300_24_and_500() {
        let cfg = MindsConfig::default();
        assert_eq!(pool_for(&cfg, 300), 200_000);
        assert_eq!(pool_for(&cfg, 24), 476_000);
        assert_eq!(pool_for(&cfg, 500), 120_000);
        assert_eq!(pool_for(&cfg, 0), 500_000);
        assert_eq!(pool_for(&cfg, usize::MAX), 120_000);
        let lowered = MindsConfig {
            pool_base_ops_s: 300_000,
            ..MindsConfig::default()
        };
        assert_eq!(pool_for(&lowered, 24), 276_000, "the gate fixture's pool");
    }

    /// **The loads are the class ceilings** (doc 17 §9): 400, 1,500, 6,000
    /// and 18,000 a second on the stock table; nothing for no mind; and the
    /// fill rule — a named class wins, a seated hull loads nothing, an
    /// unseated one `[tacsim.minds].fill_class` (17b cut M4.3; Class 0 until
    /// then).
    #[test]
    fn m3_the_loads_are_the_class_ceilings_and_the_fill_is_fill_class() {
        let classes = ClassTable::STOCK;
        let loads: Vec<u64> = CoreClass::ALL
            .iter()
            .map(|class| load_of(&classes, Some(*class)))
            .collect();
        assert_eq!(loads, [400, 1_500, 6_000, 18_000]);
        assert_eq!(load_of(&classes, None), 0);
        assert_eq!(MindsConfig::default().fill_class, CoreClass::I);
        assert_eq!(admitted_class(None, true, CoreClass::I), None);
        assert_eq!(
            admitted_class(None, false, CoreClass::I),
            Some(CoreClass::I)
        );
        assert_eq!(
            admitted_class(None, false, CoreClass::Zero),
            Some(CoreClass::Zero)
        );
        assert_eq!(
            admitted_class(Some(CoreClass::III), true, CoreClass::I),
            Some(CoreClass::III)
        );
    }

    /// **The stock fill's load** (17b cut M4.3; doc 17 §9): a 24-unit match
    /// at the fill's Class I speaks for 36,000 of 476,000 — twelve a side at
    /// 1,500 against a half of 238,000 — admitted with room for every class.
    #[test]
    fn m4_3_twenty_four_at_class_i_speak_for_36_000_of_476_000() {
        let cfg = MindsConfig::default();
        let pool = pool_for(&cfg, 24);
        let load = load_of(&cfg.classes, Some(cfg.fill_class));
        assert_eq!(pool, 476_000);
        assert_eq!(load, 1_500);
        assert_eq!(12 * load, 18_000);
        assert_eq!(24 * load, 36_000);
        assert_eq!(admit_side(pool, [load; 12]), Ok(()));
    }

    /// **The edge** (D-UM-36): `spoken + would == half` is admitted, one more
    /// op is refused with the three figures.
    #[test]
    fn m3_admission_holds_at_the_half_and_refuses_one_past_it() {
        assert_eq!(admit(276_000, 120_000, 18_000), Ok(()));
        assert_eq!(
            admit(276_000, 120_001, 18_000),
            Err(PoolRefusal {
                would: 18_000,
                half: 138_000,
                spoken: 120_001,
            })
        );
        assert_eq!(admit(3, 0, 1), Ok(()), "an odd pool's half rounds down");
        assert!(admit(3, 1, 1).is_err());
        assert_eq!(admit(0, u64::MAX, u64::MAX).map_err(|r| r.half), Err(0));
        // First-come within a side.
        assert_eq!(admit_side(276_000, [18_000; 7]), Ok(()));
        assert_eq!(
            admit_side(276_000, [18_000; 8]),
            Err(PoolRefusal {
                would: 18_000,
                half: 138_000,
                spoken: 126_000,
            })
        );
    }

    /// **The stock fact** (declared, doc 17 §24): at the stock keys, a full
    /// arena side of twelve Class III — `max_slots_per_side`'s ceiling — is
    /// admitted, both sides of a 24-unit match.
    #[test]
    fn m3_twelve_class_iii_a_side_fit_the_stock_pool() {
        let cfg = MindsConfig::default();
        let pool = pool_for(&cfg, 24);
        let load = load_of(&cfg.classes, Some(CoreClass::III));
        assert_eq!(12 * load, 216_000);
        assert_eq!(pool / 2, 238_000);
        assert_eq!(admit_side(pool, [load; 12]), Ok(()));
    }

    /// **Doc 17 §9's sentence, byte for byte**, and its figures' separator.
    #[test]
    fn m3_the_refusal_is_doc_17_s_sentence() {
        assert_eq!(
            PoolRefusal {
                would: 96_000,
                half: 100_000,
                spoken: 92_000,
            }
            .to_string(),
            "This side would think 96,000 ops a second; its half of the centre's pool is 100,000 \
             and 92,000 are spoken for."
        );
        for (n, written) in [
            (0, "0"),
            (7, "7"),
            (400, "400"),
            (1_500, "1,500"),
            (18_000, "18,000"),
            (138_000, "138,000"),
            (1_000_000, "1,000,000"),
            (u64::MAX, "18,446,744,073,709,551,615"),
        ] {
            assert_eq!(thousands(n), written);
        }
    }
}
