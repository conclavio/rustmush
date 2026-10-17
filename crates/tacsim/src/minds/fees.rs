//! **The kernels' fees** (17b cut M2.3; doc 17 §7 "The budget"; D-UM-26; 17b
//! §1 (iv)).
//!
//! A kernel does in one native call what a script would spend many operations
//! on, so each charges a fee in operations when it is called
//! (`mush_mind::budget::charge`), and the think's budget is operations plus
//! fees against its ceiling ([`super::STOCK_OPS_PER_THINK`] for the stock
//! class). Calibrated once on the blessing host at about 42 ns per operation
//! (doc 17 §7: 1/24 ops per µs); the trip lands at the next operation, or at
//! the sandbox's re-check after `think` returns for a fee the last call
//! charged. The getters, the context functions, the integer helpers and the
//! station functions are free: each is a field read or a push.
//!
//! | kernel | fee |
//! |---|---|
//! | every think, before its first operation | [`FEE_THINK`] 20 |
//! | `best_hostile(…)` | [`FEE_HOSTILE_BASE`] 4 + [`FEE_HOSTILE_ROW`] 3 × the view's rows |
//! | `Contact.odds(w)`, `Contact.aspect`, `contact(v, id)`, `is_dry(v)`, `would_overheat(v, w)`, `fight_limit_cm(v)` | [`FEE_READ`] 1 |
//! | `holds_a_hostile(v)` | [`FEE_READ`] 1 + [`FEE_HOLDS_ROW`] 1 × the view's rows |
//! | `health_permille(v)` | [`FEE_HEALTH`] 2 |
//! | `ranges(v, band, obey)` | [`FEE_RANGES`] 2 |
//! | `line_to(v, id)` | [`FEE_LINE_TO`] 20, at most [`super::STOCK_LINE_TO_PER_THINK`] a think |
//! | `entry_cell`, `far_entry_cell`, `stand_off_cell`, `patrol_step`, `fall_back_to_waypoint` | [`FEE_NAV`] 30 |
//! | `terrain_at(cell)` | [`FEE_TERRAIN`] 4, at most 16 local reads per think |
//! | `spot_cell(letter)` | [`FEE_SPOT`] 4, fixed public key-spot lookup |
//! | `rand(k)` | [`FEE_RAND`] 2 |

use crate::arena::mind::MindView;

/// **The fixed fee every think pays** for its scope's setup (doc 17 §7:
/// measured 0.99–1.34 µs at zero contacts) — the runner's `fee_per_think`.
pub const FEE_THINK: u64 = 20;

/// **One read of the plot** (doc 17 §7: an odds or aspect read 1): `odds`,
/// `aspect`, `contact`, `is_dry`, `would_overheat`, `fight_limit_cm`, and the
/// base of `holds_a_hostile`.
pub const FEE_READ: u64 = 1;

/// **`health_permille`**: every section's armour and internals summed.
pub const FEE_HEALTH: u64 = 2;

/// **`ranges`**: the loaded weapon's bands, the sight and the order's cap.
pub const FEE_RANGES: u64 = 2;

/// **`best_hostile`'s base** (doc 17 §7: 4 + 3 per row).
pub const FEE_HOSTILE_BASE: u64 = 4;

/// **`best_hostile`'s price per row of the view** (doc 17 §7).
pub const FEE_HOSTILE_ROW: u64 = 3;

/// **`holds_a_hostile`'s price per row of the view**.
pub const FEE_HOLDS_ROW: u64 = 1;

/// **`line_to`** (doc 17 §7: 20, capped per think by tier because it is a
/// trace).
pub const FEE_LINE_TO: u64 = 20;

/// One bounded public terrain-cell lookup, including refused/unavailable attempts.
pub const FEE_TERRAIN: u64 = 4;

/// One public map-letter lookup, including invalid or absent labels.
pub const FEE_SPOT: u64 = 4;

/// **Each nav kernel** (doc 17 §7: 30) — the motor's pickers.
pub const FEE_NAV: u64 = 30;

/// **Each `rand`** (doc 17 §7: 2).
pub const FEE_RAND: u64 = 2;

/// **Charge a kernel's fee** to the running think (`mush_mind::budget`).
pub(crate) fn charge(fee: u64) {
    mush_mind::budget::charge(fee);
}

/// **The view's rows**, as a fee multiplier.
pub(crate) fn rows(view: &MindView) -> u64 {
    u64::try_from(view.contacts.len()).unwrap_or(u64::MAX)
}

/// `base + per_row × rows`, saturating.
pub(crate) fn per_row(base: u64, per_row: u64, view: &MindView) -> u64 {
    base.saturating_add(per_row.saturating_mul(rows(view)))
}
