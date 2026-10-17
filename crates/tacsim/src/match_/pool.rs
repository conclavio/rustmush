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
//! It is checked at the choice — `SIM UNIT` and `SIM MIND`
//! (`crate::match_::lobby::unit_type`, `crate::match_::lobby::mind`,
//! [`crate::match_::lobby::SimError::Pool`]) — and, by name, as the LAST gate
//! of check-out's every-refusal block (`crate::match_::start`,
//! [`crate::match_::StartError::Pool`], naming the refused hull since 17b cut
//! M5.1), so `BOARD` never fails because of a core and a refused roster writes
//! nothing. Both refuse with the arithmetic ([`PoolRefusal`], the owner's
//! OQ-11 wording since 17b cut M5.1): how much this tank would think a second,
//! its side's half, and what is spoken for.
//!
//! **Who loads what** (17d M6.2): every hull loads the class its actual seal
//! records, `min(rating, core, applicable cap)`, including unchosen defaults.
//! A claimed pilot uses the operator maximum; fill uses the lesser of that
//! maximum and its lead's selected cap (the configured fill class by default).
//! The governed-core over-class refusal precedes either cap; coreless and
//! Class 0 chassis retain the ROM exception. Every roster, chassis, mind,
//! class, mode and fill-size choice re-runs prospective admission. Checkout
//! recomputes from the completed seals before its first write. Slot release
//! remains unconditional; a roster made invalid by release must be repaired.
//!
//! **At the stock constants neither gate can refuse** (declared for doc 17
//! §24): `crate::config::MatchConfig::max_slots_per_side` is validated 1 to
//! 12 and `StartError::TooManyUnits` is the first gate, so a side is at most
//! twelve hulls; twelve Class III a side load 12 × 18,000 = 216,000 against a
//! half of 476,000 / 2 = 238,000 at 24 units — doc 17 §9's "a 12v12 of Class
//! III takes 432,000 and fits" — and a thirteenth is `TooManyUnits`. The
//! gates bite only under an operator's lowered pool
//! (`tests/match_lifecycle.rs`' over-subscribed fixture runs at
//! `pool_base_ops_s = 300_000`).
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
/// will run on — no lobby or check-out caller passes it since 17b cut M5.1,
/// [`admitted_class`] always answering a class), else the class's ceiling
/// ([`ClassTable::ceiling_per_s`]).
#[must_use]
pub fn load_of(classes: &ClassTable, mind: Option<CoreClass>) -> u64 {
    mind.map_or(0, |class| classes.ceiling_per_s(class))
}

/// **The class a fielded hull is admitted at** — `chosen` where a fixture or,
/// since 17b cut M5.1, a chosen mind named one, else the fill rule:
/// `fill_class` — `[tacsim.minds].fill_class`, 17b cut M4.3 — **seated or
/// not** (17b cut M5.1: a seated hull is sealed at `fill_class` and a release
/// hands it to a mind of that class, so it is admitted there; the `seated`
/// parameter that admitted it at nothing, M3.5's declared gap, is gone).
#[must_use]
pub const fn admitted_class(chosen: Option<CoreClass>, fill_class: CoreClass) -> Option<CoreClass> {
    match chosen {
        Some(class) => Some(class),
        None => Some(fill_class),
    }
}

/// **A hull the pool cannot carry** (D-UM-36): the arithmetic, every figure
/// with a thousands separator ([`thousands`]) — in the owner's OQ-11 wording
/// since 17b cut M5.1, said of the hull being admitted ("This tank would
/// think …"), where doc 17 §9's sentence said "This side" (its example is
/// corrected in the record). Under 120 columns at every figure a refusal can
/// print: `would` is at most a class ceiling the class table admits (18,000:
/// 4,500 ops every 250 ms), a refusal needs `spoken + would` past the half,
/// and a side is at most twelve hulls, so `spoken` is at most 216,000 and the
/// half under 234,000 — 118 columns at `18,000`, `233,999` and `216,000`
/// (119 with every figure at six digits).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error(
    "This tank would think {} ops a second; its side's half of the centre's pool is {} and {} are \
     spoken for.",
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
/// The first [`PoolRefusal`], with its load's index among `loads` (17b cut
/// M5.1: `crate::match_::StartError::Pool` names the hull it refused).
pub fn admit_side(
    pool: u64,
    loads: impl IntoIterator<Item = u64>,
) -> Result<(), (usize, PoolRefusal)> {
    let mut spoken: u64 = 0;
    for (at, would) in loads.into_iter().enumerate() {
        admit(pool, spoken, would).map_err(|refusal| (at, refusal))?;
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
    /// fill rule — a named class wins, every other hull loads
    /// `[tacsim.minds].fill_class` (17b cut M4.3; Class 0 until then).
    ///
    /// 17b cut M5.1: `admitted_class` lost its `seated` parameter — the
    /// pin `admitted_class(None, true, CoreClass::I) == None` (a seated hull
    /// loaded nothing, M3.5's declared gap) is now
    /// `m5_a_seated_hull_is_admitted_at_fill_class`'s `Some(CoreClass::I)`.
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
        assert_eq!(admitted_class(None, CoreClass::I), Some(CoreClass::I));
        assert_eq!(admitted_class(None, CoreClass::Zero), Some(CoreClass::Zero));
        assert_eq!(
            admitted_class(Some(CoreClass::III), CoreClass::I),
            Some(CoreClass::III)
        );
    }

    /// **17b cut M5.1** (the design note's M5.1 item 7; doc 17 §24's carried
    /// "seated hulls at the pool"): a hull that chose no mind is admitted at
    /// `[tacsim.minds].fill_class`, seated or not — there is no seat bit to
    /// ask any more — so a side of seated pilots that chose nothing loads
    /// what the same side of fill hulls loads (twelve at Class I: 18,000),
    /// and a hull whose mind was chosen loads that mind's class, below
    /// `fill_class` (Class 0 on a coreless chassis: 400) as well as at it.
    #[test]
    fn m5_a_seated_hull_is_admitted_at_fill_class() {
        let cfg = MindsConfig::default();
        let unchosen = load_of(&cfg.classes, admitted_class(None, cfg.fill_class));
        assert_eq!(unchosen, 1_500, "an unchosen hull, seated or not");
        assert_eq!(
            [unchosen; 12].iter().sum::<u64>(),
            18_000,
            "twelve seated pilots who chose nothing speak for the fill's 18,000"
        );
        assert_eq!(
            load_of(
                &cfg.classes,
                admitted_class(Some(CoreClass::Zero), cfg.fill_class)
            ),
            400,
            "a chosen mind on a coreless chassis seals at Class 0"
        );
        assert_eq!(
            admitted_class(None, CoreClass::III),
            Some(CoreClass::III),
            "an operator's fill_class is every unchosen hull's"
        );
        // Under a pool whose half is 1,900, a Class 0 hull beside a seated
        // Class I one fits, and a second Class I does not: the arithmetic the
        // lobby's choice refuses with.
        assert_eq!(admit_side(3_800, [1_500, 400]), Ok(()));
        assert_eq!(
            admit_side(3_800, [1_500, 1_500]),
            Err((
                1,
                PoolRefusal {
                    would: 1_500,
                    half: 1_900,
                    spoken: 1_500,
                }
            ))
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
        // First-come within a side; the refused load's index named (17b cut
        // M5.1: the eighth, index 7).
        assert_eq!(admit_side(276_000, [18_000; 7]), Ok(()));
        assert_eq!(
            admit_side(276_000, [18_000; 8]),
            Err((
                7,
                PoolRefusal {
                    would: 18_000,
                    half: 138_000,
                    spoken: 126_000,
                }
            ))
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

    /// **The pool's sentence, byte for byte**, and its figures' separator —
    /// doc 17 §9's until 17b cut M5.1, the owner's OQ-11 wording since:
    /// `This side would think 96,000 ops a second; its half of the centre's
    /// pool is 100,000 and 92,000 are spoken for.` was this pin at 17b cut
    /// M3.5; the sentence is said of the hull being admitted now (OQ-11),
    /// 17b cut M5.1. With every figure at six digits it is 119 columns.
    #[test]
    fn m3_the_refusal_is_doc_17_s_sentence() {
        assert_eq!(
            PoolRefusal {
                would: 96_000,
                half: 100_000,
                spoken: 92_000,
            }
            .to_string(),
            "This tank would think 96,000 ops a second; its side's half of the centre's pool is \
             100,000 and 92,000 are spoken for."
        );
        let widest = PoolRefusal {
            would: 999_999,
            half: 999_999,
            spoken: 999_999,
        }
        .to_string();
        assert_eq!(widest.len(), 119, "{widest}");
        assert!(widest.len() < 120, "every figure at six digits fits");
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
