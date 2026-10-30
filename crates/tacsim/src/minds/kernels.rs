//! **The kernels** (17b cut M2.3; doc 17 §3 "The map is public", §4 "The
//! motor", §7 "The engine" and "The budget"; D-UM-24, D-UM-26, D-UM-29; 17b §1
//! (iv)).
//!
//! **The rule (the programme lead's): every kernel is a thin registered
//! wrapper over the ONE free function the native stock mind (T1) calls, never
//! a second body** — so a Rhai port that calls a kernel where T1 calls the
//! function answers what T1 answers (Gate A), and a fix to the function is a
//! fix to both. `entry_interior_cell` also exposes the shared native geometry
//! used by the Conduct observer; the stock `entry_cell` contract is unchanged.
//! Each wrapper converts its arguments (refusing a wrong shape
//! `type` through the context's `refused`), charges its fee
//! ([`super::fees`]) and calls:
//!
//! * `best_hostile` → `stock_mind::best_hostile` over [`Weights`], with the
//!   `admit` T1's `stand` builds (a ceiling and an optional screen sector);
//! * `health_permille`, `is_dry`, `holds_a_hostile`, `would_overheat`,
//!   `fight_limit_cm` → the `stock_mind` functions of those names;
//! * `contact` → `stock_mind::contact`, the row lookup — a port uses it and
//!   never a script-side scan over `v.contacts`, so its cost stays flat in
//!   the row count (doc 17 §18);
//! * `ranges` → `stock_mind::view_ranges` over the band per mille, the
//!   order passed only when the SCRIPT says it obeys one (`obey_order`, off
//!   its own obeyed-order local after the early ends — as T1 passes
//!   `obeyed(view, mem)` and never `view.order`: 17b §5's feas F6);
//! * `line_to` → `stock_mind::line_between` (a `()` cell on either end is a
//!   line, with no trace), capped at the row's `line_to` a think (17b cut
//!   M3.1; Class I's [`super::STOCK_LINE_TO_PER_THINK`]);
//! * `entry_cell`, `entry_interior_cell`, `far_entry_cell`, `patrol_step`,
//!   `fall_back_to_waypoint`
//!   → the motor's pickers (`crate::arena::motor`), and `stand_off_cell` →
//!   `stock_mind::stand_off_of`, every one through the context's map and as
//!   a tracked hull (`CostRow::Tracked`, `stock_mind`'s residual: the view
//!   carries no cost row);
//! * the integer helpers, free: `sin_m`/`cos_m` → `motion::sin_fx`/`cos_fx`,
//!   `bearing_to` → `perception::bearing_of` from the view's pose to a cell's
//!   centre, `range_to` → `index::range_between` likewise, `arc` →
//!   `doctrine::arc_between`, `stand_off_want` → `doctrine::stand_off_want`;
//! * `rand(k)` → `rng::draw` under `DrawLabel::Mind` (D-UM-29).
//!
//! `hold_arrived`, `hold_contains`, and `hold_advance_arrived` read public
//! objective geometry and the own hull only, using the same pure native
//! helpers. Each charges `FEE_READ`; no capture state is exposed.
//!
//! A kernel's required INT arguments are typed INT; a call that no registered
//! signature matches (a FLOAT weight) is rhai's own error and rhai's own `type`
//! fault. The arguments that may be `()` are taken last and as `Dynamic`, so
//! rhai's wildcard search for them stays short (it walks the `Dynamic`
//! positions from the last argument up).

use mush_mind::rhai::{Array, Dynamic, EvalAltResult, INT, ImmutableString};

use super::RAND_MAX_K;
use super::Registrar;
use super::api::{
    Contact, View, bounded, cell, cell_arg, contact_int, contact_of, int, opt, opt_cell_arg,
    opt_int_arg,
};
use super::context::{REFUSED_LIMIT, REFUSED_TYPE, REFUSED_UNKNOWN, refuse, refused, with};
use super::fees::{
    FEE_HEALTH, FEE_HOLDS_ROW, FEE_HOSTILE_BASE, FEE_HOSTILE_ROW, FEE_LINE_TO, FEE_NAV, FEE_RAND,
    FEE_RANGES, FEE_READ, FEE_SPOT, FEE_TERRAIN, charge, per_row,
};
use crate::arena::doctrine::{self, arc_between};
use crate::arena::index::range_between;
use crate::arena::mind::{MindView, ViewContact};
use crate::arena::motion::{cos_fx, sin_fx};
use crate::arena::motor::{self, WalkStep};
use crate::arena::perception::bearing_of;
use crate::arena::rng::{DrawLabel, draw};
use crate::arena::state::{CostRow, NO_SUCH_CONTACT};
use crate::arena::stock_mind::{
    self, Weights, best_hostile as hostile_of, cell_point, line_between, own_point, row_range_cm,
    stand_off_of, view_ranges,
};

/// A full turn, millidegrees: an angle helper reads its argument modulo one.
const TURN_MDEG: INT = 360_000;

/// The refusal a weapon id the view does not carry answers.
const NO_SUCH_WEAPON: &str = "No such weapon on this hull.";

/// **What `ranges` answers** (`Ranges`): T1's [`doctrine::Ranges`] —
/// `engage_cm`, `release_cm`, `stand_cm`, `limit_cm`, centimetres, saturating
/// at `INT::MAX`.
#[derive(Clone, Copy, Debug)]
pub struct Ranges(pub doctrine::Ranges);

/// **What `patrol_step` and `fall_back_to_waypoint` answer** (`Walk`): the
/// motor's [`WalkStep`] — `waypoint` and `from_waypoint` (`()` or an INT
/// node), `goal` (`()` or a cell) and `clear_route` (always `false` from the
/// fallback).
#[derive(Clone, Copy, Debug)]
pub struct Walk(pub WalkStep);

/// A degree figure in millidegrees, reduced modulo a turn.
fn turn(mdeg: INT) -> u32 {
    u32::try_from(mdeg.rem_euclid(TURN_MDEG)).unwrap_or(0)
}

/// **`best_hostile(v, w_range, w_facet, w_health, health_permille, prefer,
/// ceiling_cm, facing_mdeg, half_mdeg)`** — `stock_mind::best_hostile` over
/// the script's [`Weights`], its health figure and its `prefer` (`()` or a
/// contact INT), with `admit` built as T1's `stand` builds it: a row's held
/// range at or under `ceiling_cm` (`()` admits every range: T1's `|_| true`
/// on the hostile edges and the withdraw stand), and, with `facing_mdeg` and
/// `half_mdeg` both INTs, the row's whole-degree bearing within `half_mdeg`
/// of `facing_mdeg` (the screen's sector; both `()` for none). T1's stand
/// passes `ranges.release_cm.max(1)` as the ceiling; the script passes it the
/// same. Answers the contact INT or `()`. Fee 4 + 3 × the view's rows.
#[allow(clippy::too_many_arguments)]
fn best_hostile(
    v: &mut View,
    w_range: INT,
    w_facet: INT,
    w_health: INT,
    health_permille: INT,
    prefer: Dynamic,
    ceiling_cm: Dynamic,
    facing_mdeg: Dynamic,
    half_mdeg: Dynamic,
) -> Result<Dynamic, Box<EvalAltResult>> {
    const TEXT: &str = "best_hostile takes weights and a health from 0, then a contact, a ceiling and a sector or ()";
    let weights = Weights {
        range: bounded(w_range, TEXT)?,
        facet: bounded(w_facet, TEXT)?,
        health: bounded(w_health, TEXT)?,
    };
    let health: u32 = bounded(health_permille, TEXT)?;
    let prefer = opt_int_arg(&prefer, TEXT)?.map(contact_of);
    let ceiling: Option<u64> = match opt_int_arg(&ceiling_cm, TEXT)? {
        Some(ceiling) => Some(bounded(ceiling, TEXT)?),
        None => None,
    };
    let sector: Option<(u32, u32)> = match (
        opt_int_arg(&facing_mdeg, TEXT)?,
        opt_int_arg(&half_mdeg, TEXT)?,
    ) {
        (None, None) => None,
        (Some(facing), Some(half)) => Some((bounded(facing, TEXT)?, bounded(half, TEXT)?)),
        _ => return Err(refused(REFUSED_TYPE, TEXT)),
    };
    let view = &v.0;
    charge(per_row(FEE_HOSTILE_BASE, FEE_HOSTILE_ROW, view));
    let found = hostile_of(view, &weights, health, prefer, |row| {
        ceiling.is_none_or(|ceiling| row_range_cm(row) <= ceiling)
            && sector.is_none_or(|(facing, half)| {
                arc_between(row.bearing_deg.saturating_mul(1_000), facing) <= half
            })
    });
    Ok(opt(found, |id| Dynamic::from_int(contact_int(id))))
}

/// The row `id` names in `view`, by `stock_mind::contact`, with the slot it
/// sits in (a handle's index), or `None` — no refusal built: `contact`'s
/// answer for an unheld id is `()`, and a caller that refuses builds its own.
fn held(view: &MindView, id: INT) -> Option<(u32, &ViewContact)> {
    let row = stock_mind::contact(view, contact_of(id))?;
    // The lookup is the one function's; the handle needs the slot it found.
    let index = view
        .contacts
        .iter()
        .position(|held| std::ptr::eq(held, row))?;
    Some((u32::try_from(index).ok()?, row))
}

/// **`ranges(v, engage_band_permille, obey_order)`** — fee 2.
fn ranges(v: &mut View, band: INT, obey_order: bool) -> Result<Ranges, Box<EvalAltResult>> {
    let band: u32 = bounded(band, "ranges takes a band per mille from 0 and a bool")?;
    charge(FEE_RANGES);
    let view = &v.0;
    let order = if obey_order {
        view.order.as_ref()
    } else {
        None
    };
    Ok(Ranges(view_ranges(view, band, order)))
}

/// **The refusal a `line_to` past the cap raises** (17b cut M3.1): Class I's
/// sentence byte for byte (`twice`), and the cap spelled out for another row.
fn line_to_cap_text(max: u32) -> String {
    match max {
        1 => "line_to is traced at most once a think".to_owned(),
        2 => "line_to is traced at most twice a think".to_owned(),
        n => format!("line_to is traced at most {n} times a think"),
    }
}

/// **`line_to(v, id)`** — fee 20; a call past the think's cap refused `limit`
/// (17b cut M3.1: the row's `line_to`, `ThinkContext::line_to_max` — Class I's
/// two, so the third); an id the view does not hold `unknown`.
fn line_to(v: &mut View, id: INT) -> Result<bool, Box<EvalAltResult>> {
    let view = &v.0;
    with(|ctx| {
        ctx.line_to_calls = ctx.line_to_calls.saturating_add(1);
        if ctx.line_to_calls > ctx.line_to_max {
            let text = line_to_cap_text(ctx.line_to_max);
            return Err(refuse(ctx, REFUSED_LIMIT, &text));
        }
        charge(FEE_LINE_TO);
        let Some(row) = stock_mind::contact(view, contact_of(id)) else {
            return Err(refuse(ctx, REFUSED_UNKNOWN, NO_SUCH_CONTACT));
        };
        Ok(line_between(&ctx.map, view.cell, row.cell))
    })?
}

/// Public map read for the local opening/ground checks. Tuple positions are
/// [kind: public snake-case terrain label, tracked_passable: bool]. The latter
/// is the authored tracked locomotion row, never occupancy, slope clearance,
/// this hull's current capability, or permission to enter an enemy entry zone.
/// At most sixteen attempts, including malformed/unavailable reads, per think.
/// Off-map, no own cell, and beyond 320 m Chebyshev distance all answer ().
fn terrain_at(at: Dynamic) -> Result<Dynamic, Box<EvalAltResult>> {
    with(|ctx| {
        charge(FEE_TERRAIN);
        ctx.terrain_calls = ctx.terrain_calls.saturating_add(1);
        if ctx.terrain_calls > 16 {
            return Err(refuse(
                ctx,
                REFUSED_LIMIT,
                "terrain_at is read at most sixteen times a think",
            ));
        }
        Ok(())
    })??;
    const TEXT: &str = "terrain_at takes one [x, y] cell";
    let values = at
        .try_cast::<Array>()
        .ok_or_else(|| refused(REFUSED_TYPE, TEXT))?;
    let [x, y] = values.as_slice() else {
        return Err(refused(REFUSED_TYPE, TEXT));
    };
    let x = x.as_int().map_err(|_| refused(REFUSED_TYPE, TEXT))?;
    let y = y.as_int().map_err(|_| refused(REFUSED_TYPE, TEXT))?;
    let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
        return Ok(Dynamic::UNIT);
    };
    let at = (x, y);
    with(|ctx| {
        let Some(here) = ctx.view.cell else {
            return Dynamic::UNIT;
        };
        let far = u64::from(here.0.abs_diff(at.0).max(here.1.abs_diff(at.1)))
            .saturating_mul(u64::from(ctx.map.terrain.cell_cm()));
        if far > 32_000 {
            return Dynamic::UNIT;
        }
        match ctx.map.terrain.get(at.0, at.1) {
            Some(cell) => Dynamic::from_array(vec![
                Dynamic::from(ImmutableString::from(cell.class.label())),
                Dynamic::from_bool(cell.passable()),
            ]),
            None => Dynamic::UNIT,
        }
    })
}

/// One fixed public map-letter lookup, with no entity or observation inputs.
/// The same alphabet and authored ordering drive the cockpit's public map.
fn spot_cell(letter: Dynamic) -> Result<Dynamic, Box<EvalAltResult>> {
    charge(FEE_SPOT);
    let Some(letter) = letter.try_cast::<ImmutableString>() else {
        return Ok(Dynamic::UNIT);
    };
    let [letter] = letter.as_bytes() else {
        return Ok(Dynamic::UNIT);
    };
    let letter = char::from(letter.to_ascii_uppercase());
    let Some(index) = crate::arena::state::SPOT_LETTERS
        .iter()
        .position(|candidate| *candidate == letter)
    else {
        return Ok(Dynamic::UNIT);
    };
    with(|ctx| cell(ctx.map.key_spots.get(index).map(|spot| (spot.x, spot.y))))
}

/// **`stand_off_cell(v, id, want_cm)`** — fee 30; `()` or a cell.
fn stand_off_cell(v: &mut View, id: INT, want_cm: INT) -> Result<Dynamic, Box<EvalAltResult>> {
    let want: u64 = bounded(want_cm, "stand_off_cell takes a contact and a range from 0")?;
    let view = &v.0;
    let (_, row) = held(view, id).ok_or_else(|| refused(REFUSED_UNKNOWN, NO_SUCH_CONTACT))?;
    charge(FEE_NAV);
    with(|ctx| cell(stand_off_of(&ctx.map, view, row, want)))
}

/// A walk's node argument: `()` or a node number.
fn node_arg(value: &Dynamic, text: &str) -> Result<Option<u16>, Box<EvalAltResult>> {
    match opt_int_arg(value, text)? {
        Some(node) => Ok(Some(bounded(node, text)?)),
        None => Ok(None),
    }
}

/// **`patrol_step(v, arrived, waypoint, from_waypoint, goal)`** — fee 30.
fn patrol_step(
    v: &mut View,
    arrived: bool,
    waypoint: Dynamic,
    from_waypoint: Dynamic,
    goal: Dynamic,
) -> Result<Walk, Box<EvalAltResult>> {
    const TEXT: &str = "patrol_step takes a bool, two nodes or (), and a cell or ()";
    let walk = motor::Walk {
        waypoint: node_arg(&waypoint, TEXT)?,
        from_waypoint: node_arg(&from_waypoint, TEXT)?,
        goal: opt_cell_arg(&goal, TEXT)?,
    };
    let view = &v.0;
    charge(FEE_NAV);
    with(|ctx| {
        Walk(motor::patrol_step(
            &ctx.map, view.unit, view.cell, arrived, walk,
        ))
    })
}

/// **`fall_back_to_waypoint(v, failed, waypoint, from_waypoint)`** — fee 30:
/// the walk after the motor's `unreachable` fallback from the view's cell
/// (`motor::fallback_waypoint`); off the window, the walk kept and no goal,
/// as T1 keeps it.
fn fall_back_to_waypoint(
    v: &mut View,
    failed: Dynamic,
    waypoint: Dynamic,
    from_waypoint: Dynamic,
) -> Result<Walk, Box<EvalAltResult>> {
    const TEXT: &str = "fall_back_to_waypoint takes a cell or () and two nodes or ()";
    let failed = opt_cell_arg(&failed, TEXT)?;
    let waypoint = node_arg(&waypoint, TEXT)?;
    let from_waypoint = node_arg(&from_waypoint, TEXT)?;
    let view = &v.0;
    charge(FEE_NAV);
    with(|ctx| {
        let walk = match view.cell {
            Some(here) => {
                let (waypoint, from_waypoint, goal) = motor::fallback_waypoint(
                    &ctx.map,
                    here,
                    failed,
                    waypoint,
                    from_waypoint,
                    view.unit,
                );
                motor::Walk {
                    waypoint,
                    from_waypoint,
                    goal,
                }
            }
            None => motor::Walk {
                waypoint,
                from_waypoint,
                goal: None,
            },
        };
        Walk(WalkStep {
            walk,
            clear_route: false,
        })
    })
}

/// A point, `(x_cm, y_cm, z_cm)` — `stock_mind`'s shape.
type Point = (i64, i64, i32);

/// **`bearing_to(v, [x, y])`** and **`range_to(v, [x, y])`**' shared read:
/// the view's own pose, and the cell's centre at the ground's height there
/// (`stock_mind::cell_point`).
fn toward(v: &View, at: &Dynamic, text: &str) -> Result<(Point, Point), Box<EvalAltResult>> {
    let at = cell_arg(at, text)?;
    let point = with(|ctx| cell_point(&ctx.map, at))?;
    Ok((own_point(&v.0), point))
}

/// **`would_overheat(v, weapon)`** — `stock_mind::would_overheat` on the
/// named row; fee 1; a weapon the hull does not carry `unknown`.
fn would_overheat(v: &mut View, weapon: ImmutableString) -> Result<bool, Box<EvalAltResult>> {
    charge(FEE_READ);
    let row =
        v.0.weapons
            .iter()
            .find(|row| row.id.as_str() == weapon.as_str())
            .ok_or_else(|| refused(REFUSED_UNKNOWN, NO_SUCH_WEAPON))?;
    Ok(stock_mind::would_overheat(&v.0, row))
}

/// **`entry_cell(v)`** — `motor::entry_cell` from the view's cell for its
/// side; fee 30; `()` off the window or with no zone.
fn entry_cell(v: &mut View) -> Result<Dynamic, Box<EvalAltResult>> {
    charge(FEE_NAV);
    let view = &v.0;
    with(|ctx| {
        cell(
            view.cell
                .and_then(|here| motor::entry_cell(&ctx.map, view.team, CostRow::Tracked, here)),
        )
    })
}

/// **`entry_interior_cell(v)`** — the own-entry goal whose entire native
/// arrival square is inside the zone; fee 30. `()` off the window, without
/// an own zone, or without a passable interior found within the bounded scan.
fn entry_interior_cell(v: &mut View) -> Result<Dynamic, Box<EvalAltResult>> {
    charge(FEE_NAV);
    let view = &v.0;
    with(|ctx| {
        cell(view.cell.and_then(|here| {
            motor::entry_interior_cell(&ctx.map, view.team, CostRow::Tracked, here)
        }))
    })
}

/// **`far_entry_cell(v)`** — `motor::far_entry_cell` from the view's cell,
/// with no `taken` cells (the motor fans the landing: the M1a plan's
/// departure (d)); fee 30.
fn far_entry_cell(v: &mut View) -> Result<Dynamic, Box<EvalAltResult>> {
    charge(FEE_NAV);
    let view = &v.0;
    with(|ctx| {
        cell(view.cell.and_then(|here| {
            motor::far_entry_cell(&ctx.map, view.team, CostRow::Tracked, here, &[])
        }))
    })
}

/// **`bearing_to(v, [x, y])`** — millidegrees from the view's pose to the
/// cell's centre (`perception::bearing_of`); free.
fn bearing_to(v: &mut View, at: Array) -> Result<INT, Box<EvalAltResult>> {
    let (own, point) = toward(
        v,
        &Dynamic::from_array(at),
        "bearing_to takes a cell [x, y]",
    )?;
    Ok(INT::from(bearing_of(
        point.0.saturating_sub(own.0),
        point.1.saturating_sub(own.1),
    )))
}

/// **`range_to(v, [x, y])`** — centimetres from the view's pose to the
/// cell's centre at the ground's height there (`index::range_between`); free.
fn range_to(v: &mut View, at: Array) -> Result<INT, Box<EvalAltResult>> {
    let (own, point) = toward(v, &Dynamic::from_array(at), "range_to takes a cell [x, y]")?;
    let range = range_between(own.0, own.1, own.2, point.0, point.1, point.2);
    Ok(INT::try_from(range).unwrap_or(INT::MAX))
}

/// **`stand_off_want(engage_cm, cell_cm, range_cm)`** —
/// `doctrine::stand_off_want`; free.
fn stand_off_want(engage_cm: INT, cell_cm: INT, range_cm: INT) -> Result<INT, Box<EvalAltResult>> {
    const TEXT: &str = "stand_off_want takes three distances from 0";
    let want = doctrine::stand_off_want(
        bounded(engage_cm, TEXT)?,
        bounded(cell_cm, TEXT)?,
        bounded(range_cm, TEXT)?,
    );
    Ok(INT::try_from(want).unwrap_or(INT::MAX))
}

/// **`rand(k)`** (D-UM-29): `draw(seed, Mind { unit, think_seq, n }) % k`,
/// `n` the think's draw count; fee 2. `k` outside `1..=RAND_MAX_K` refused
/// `type`.
fn rand(k: INT) -> Result<INT, Box<EvalAltResult>> {
    if !(1..=RAND_MAX_K).contains(&k) {
        return Err(refused(
            REFUSED_TYPE,
            "rand takes a whole number from 1 to 1000",
        ));
    }
    let k = u64::try_from(k).unwrap_or(1);
    let value = with(|ctx| {
        let label = DrawLabel::Mind {
            unit: ctx.unit.0,
            think_seq: ctx.think_seq,
            n: ctx.draws,
        };
        ctx.draws = ctx.draws.saturating_add(1);
        draw(ctx.seed, &label) % k
    })?;
    charge(FEE_RAND);
    Ok(INT::try_from(value).unwrap_or(0))
}

/// **The kernels, their result types and the integer helpers** (the file
/// head).
#[allow(clippy::too_many_lines)]
pub(crate) fn register(api: &mut Registrar<'_>) {
    custom_type!(api, Ranges, "Ranges");
    getter!(api, "Ranges", "engage_cm", |r: &mut Ranges| {
        int(r.0.engage_cm)
    });
    getter!(api, "Ranges", "release_cm", |r: &mut Ranges| {
        int(r.0.release_cm)
    });
    getter!(api, "Ranges", "stand_cm", |r: &mut Ranges| {
        int(r.0.stand_cm)
    });
    getter!(api, "Ranges", "limit_cm", |r: &mut Ranges| {
        int(r.0.limit_cm)
    });
    custom_type!(api, Walk, "Walk");
    getter!(api, "Walk", "waypoint", |w: &mut Walk| {
        opt(w.0.walk.waypoint, int)
    });
    getter!(api, "Walk", "from_waypoint", |w: &mut Walk| {
        opt(w.0.walk.from_waypoint, int)
    });
    getter!(api, "Walk", "goal", |w: &mut Walk| { cell(w.0.walk.goal) });
    getter!(api, "Walk", "clear_route", |w: &mut Walk| {
        Dynamic::from_bool(w.0.clear_route)
    });

    function!(api, "best_hostile", best_hostile);
    function!(api, "health_permille", |v: &mut View| -> INT {
        charge(FEE_HEALTH);
        INT::from(stock_mind::health_permille(&v.0))
    });
    function!(api, "is_dry", |v: &mut View| -> bool {
        charge(FEE_READ);
        stock_mind::is_dry(&v.0)
    });
    function!(api, "holds_a_hostile", |v: &mut View| -> bool {
        charge(per_row(FEE_READ, FEE_HOLDS_ROW, &v.0));
        stock_mind::holds_a_hostile(&v.0)
    });
    function!(api, "would_overheat", would_overheat);
    function!(api, "fight_limit_cm", |v: &mut View| -> Dynamic {
        charge(FEE_READ);
        int(stock_mind::fight_limit_cm(&v.0))
    });
    function!(api, "contact", |v: &mut View, id: INT| -> Dynamic {
        charge(FEE_READ);
        // An id the view does not hold is `()` here, not a refusal: this is
        // the row lookup, and "not held" is its answer.
        opt(held(&v.0, id), |(index, _)| {
            Dynamic::from(Contact(std::sync::Arc::clone(&v.0), index))
        })
    });
    function!(api, "ranges", ranges);
    function!(api, "line_to", line_to);
    function!(api, "terrain_at", terrain_at);
    function!(api, "spot_cell", spot_cell);
    function!(api, "entry_cell", entry_cell);
    function!(api, "entry_interior_cell", entry_interior_cell);
    function!(api, "far_entry_cell", far_entry_cell);
    function!(api, "stand_off_cell", stand_off_cell);
    function!(api, "patrol_step", patrol_step);
    function!(api, "hold_arrived", |v: &mut View| -> bool {
        charge(FEE_READ);
        v.0.objective
            .as_ref()
            .is_some_and(|objective| motor::hold_arrived(objective, v.0.immobile, v.0.cell))
    });
    function!(api, "hold_contains", |v: &mut View,
                                     at: Dynamic|
     -> Result<
        bool,
        Box<EvalAltResult>,
    > {
        let at = opt_cell_arg(&at, "hold_contains takes a cell or ()")?;
        charge(FEE_READ);
        Ok(v.0
            .objective
            .as_ref()
            .is_some_and(|objective| at.is_some_and(|cell| objective.square_contains(cell))))
    });
    function!(api, "hold_advance_arrived", |v: &mut View,
                                            to: Dynamic|
     -> Result<
        bool,
        Box<EvalAltResult>,
    > {
        let to = cell_arg(&to, "hold_advance_arrived takes a cell")?;
        charge(FEE_READ);
        Ok(v.0.objective.as_ref().is_some_and(|objective| {
            stock_mind::advance_reached_the_hold(objective, to, v.0.cell, v.0.motor.as_ref())
        }))
    });
    function!(api, "fall_back_to_waypoint", fall_back_to_waypoint);

    // The integer helpers, free (doc 17 §7). `sin_m`/`cos_m` answer
    // `motion::sin_fx`/`cos_fx` in its own fixed point, `SIN_ONE` = 1 << 20
    // = 1,048,576 for 1.0, of a heading in millidegrees taken modulo a turn.
    function!(api, "sin_m", |mdeg: INT| -> INT { sin_fx(turn(mdeg)) });
    function!(api, "cos_m", |mdeg: INT| -> INT { cos_fx(turn(mdeg)) });
    function!(api, "bearing_to", bearing_to);
    function!(api, "range_to", range_to);
    function!(api, "arc", |a: INT, b: INT| -> INT {
        INT::from(arc_between(turn(a), turn(b)))
    });
    function!(api, "stand_off_want", stand_off_want);
    function!(api, "rand", rand);
}

#[cfg(test)]
mod tests {
    use mush_mind::Fault;
    use std::sync::Arc;

    use super::super::fees::FEE_THINK;
    use super::super::fixture::{
        self, BLIP, HOSTILE, context_on, eval, eval_in, shown, think, think_with, view,
    };
    use super::*;
    use crate::arena::perception::Aspect;
    use crate::arena::state::{ArenaTerrain, Classification, ContactId, PartyId};
    use crate::arena::step::FrameHeat;
    use crate::arena::stock_mind::{fight_limit_cm, health_permille, holds_a_hostile, is_dry};
    use crate::deliberation::order::RangeBand;

    /// `expr` evaluated on `seen` (the full view's map), rendered.
    fn on(seen: &MindView, expr: &str) -> String {
        shown(
            &eval_in(context_on(seen.clone()), expr)
                .unwrap_or_else(|error| panic!("{expr}: {error}")),
        )
    }

    #[test]
    fn interior_entry_api_matches_native_geometry_and_refusals() {
        let mut seen = view();
        seen.cell = Some((7, 4));
        let mut map = fixture::map();
        map.entry_zones
            .iter_mut()
            .find(|zone| zone.team == seen.team)
            .expect("own zone")
            .x1 = 3;
        let native = motor::entry_interior_cell(&map, seen.team, CostRow::Tracked, (7, 4));
        assert_eq!(native, Some((2, 4)));
        let mut ctx = context_on(seen.clone());
        ctx.map = Arc::new(map.clone());
        assert_eq!(
            shown(&eval_in(ctx, "entry_interior_cell(v)").expect("registered helper")),
            shown(&cell(native))
        );
        seen.cell = None;
        let mut ctx = context_on(seen);
        ctx.map = Arc::new(map);
        assert_eq!(
            shown(&eval_in(ctx, "entry_interior_cell(v)").expect("off-window")),
            "()"
        );
        assert_eq!(
            on(&view(), "entry_interior_cell(v)"),
            "()",
            "unchanged narrow fixture zone"
        );
        assert!(eval("entry_interior_cell(42)").is_err(), "typed view only");
    }

    /// A rendered `Option<ContactId>` as the kernel answers it.
    fn id_shown(id: Option<ContactId>) -> String {
        shown(&opt(id, |id| Dynamic::from_int(contact_int(id))))
    }

    #[test]
    fn spot_cells_are_exactly_the_already_public_map_markers() {
        let mut definition = fixture::map();
        definition.key_spots = crate::arena::state::SPOT_LETTERS
            .iter()
            .enumerate()
            .map(|(i, _)| crate::arena::state::KeySpot {
                name: format!("Public {i}").into(),
                x: i as u16,
                y: 2,
            })
            .collect();
        let public = crate::match_::cockpit::map_frame(
            crate::arena_task::MatchId(1),
            "test",
            &definition.terrain,
            &definition.key_spots,
        );
        for marker in public["spots"].as_array().unwrap() {
            let letter = marker["letter"].as_str().unwrap();
            let mut seen = view();
            seen.cell = None;
            seen.contacts.clear();
            seen.sections.clear();
            let mut ctx = context_on(seen);
            ctx.map = Arc::new(definition.clone());
            let result = eval_in(ctx, &format!("spot_cell(\"{letter}\")")).unwrap();
            assert_eq!(
                shown(&result),
                format!("[{}, {}]", marker["cell"][0], marker["cell"][1])
            );
        }
        for word in ["\"\"", "\"AA\"", "\"D\"", "\"?\"", "123", "()"] {
            assert_eq!(on(&view(), &format!("spot_cell({word})")), "()");
        }
        // The fixture only has A; an absent valid letter is the same unavailable
        // value as malformed input, with the same four-operation charge.
        for word in ["\"B\"", "123"] {
            let result = think(&format!("fn think(v,c) {{ let x=spot_cell({word}); }}"));
            assert_eq!(result.result.unwrap().fee, FEE_THINK + FEE_SPOT);
        }
    }

    #[test]
    fn terrain_reads_only_authored_tracked_ground_in_a_bounded_neighborhood() {
        use crate::arena::state::TerrainCell;
        use mush_atlas::TerrainClass;

        let mut seen = view();
        seen.cell = Some((40, 40));
        let mut definition = fixture::map();
        definition.terrain = ArenaTerrain::uniform(
            100,
            100,
            1_000,
            TerrainCell {
                class: TerrainClass::WoodsLight,
                move_cost: 150,
                elev_cm: 12,
            },
        )
        .unwrap();
        let read = |seen: MindView, at: &str| {
            let mut ctx = context_on(seen);
            ctx.map = Arc::new(definition.clone());
            shown(&eval_in(ctx, &format!("terrain_at({at})")).unwrap())
        };
        assert_eq!(read(seen.clone(), "[72, 72]"), "[\"woods_light\", true]");
        assert_eq!(read(seen.clone(), "[73, 40]"), "()");
        assert_eq!(read(seen.clone(), "[100, 40]"), "()");
        assert_eq!(read(seen.clone(), "[-1, 40]"), "()");
        assert_eq!(read(seen.clone(), "[65536, 40]"), "()");
        // Own hardware, visible entities and contact truth cannot change this
        // public authored-map answer: none is an operand of the kernel.
        let mut other = seen.clone();
        other.contacts.clear();
        other.weapons.clear();
        other.sections.clear();
        other.immobile = true;
        other.lock = None;
        assert_eq!(read(other, "[40, 40]"), read(seen.clone(), "[40, 40]"));
        seen.cell = None;
        assert_eq!(read(seen, "[40, 40]"), "()");
        definition.terrain = ArenaTerrain::uniform(
            100,
            100,
            1_000,
            TerrainCell {
                class: TerrainClass::WaterDeep,
                move_cost: 0,
                elev_cm: 0,
            },
        )
        .unwrap();
        let mut ctx = context_on(view());
        ctx.map = Arc::new(definition);
        assert_eq!(
            shown(&eval_in(ctx, "terrain_at(v.cell)").unwrap()),
            "[\"water_deep\", false]"
        );
    }

    #[test]
    fn terrain_attempts_are_charged_bounded_and_reset_for_each_think() {
        let source = "fn think(v, c) { for n in 0..16 { let x = terrain_at([65535, 65535]); } }";
        for _ in 0..2 {
            let ran = think(source);
            assert_eq!(
                ran.result.expect("sixteen unavailable reads").fee,
                FEE_THINK + 16 * FEE_TERRAIN
            );
            assert_eq!(ran.ctx.terrain_calls, 16);
            assert_eq!(ran.ctx.refused, None);
        }
        let ran = think("fn think(v,c) { for n in 0..17 { let x = terrain_at([65535, 65535]); } }");
        assert!(matches!(ran.result, Err(Fault::Type(_))));
        assert_eq!(ran.ctx.terrain_calls, 17);
        assert_eq!(ran.ctx.refused, Some(REFUSED_LIMIT));
        let ran = think("fn think(v,c) { let x = terrain_at([true, 0]); }");
        assert!(matches!(ran.result, Err(Fault::Type(_))));
        assert_eq!(
            ran.ctx.terrain_calls, 1,
            "malformed attempts consume the same allowance"
        );
        assert_eq!(ran.ctx.refused, Some(REFUSED_TYPE));
    }

    /// One call of every fee-charging kernel, and the fee each charges on the
    /// full view (three rows).
    fn calls() -> Vec<(&'static str, u64)> {
        vec![
            (
                "best_hostile(v, 1, 1, 1, 1000, (), (), (), ())",
                FEE_HOSTILE_BASE + FEE_HOSTILE_ROW * 3,
            ),
            ("health_permille(v)", FEE_HEALTH),
            ("is_dry(v)", FEE_READ),
            ("hold_arrived(v)", FEE_READ),
            ("hold_contains(v, [6, 4])", FEE_READ),
            ("hold_advance_arrived(v, [6, 4])", FEE_READ),
            ("holds_a_hostile(v)", FEE_READ + FEE_HOLDS_ROW * 3),
            ("would_overheat(v, \"cannon\")", FEE_READ),
            ("fight_limit_cm(v)", FEE_READ),
            ("contact(v, v.lock)", FEE_READ),
            ("ranges(v, 800, true)", FEE_RANGES),
            ("line_to(v, v.lock)", FEE_LINE_TO),
            ("terrain_at(v.cell)", FEE_TERRAIN),
            ("spot_cell(\"A\")", FEE_SPOT),
            ("entry_cell(v)", FEE_NAV),
            ("entry_interior_cell(v)", FEE_NAV),
            ("far_entry_cell(v)", FEE_NAV),
            ("stand_off_cell(v, v.lock, 500)", FEE_NAV),
            ("patrol_step(v, false, (), (), ())", FEE_NAV),
            ("fall_back_to_waypoint(v, [6, 4], (), ())", FEE_NAV),
            ("rand(10)", FEE_RAND),
            ("v.contacts[0].aspect", FEE_READ),
            ("v.contacts[0].odds(\"cannon\")", FEE_READ),
        ]
    }

    /// 17b §2's M2.3 test 4: **the fees are the table** — a think that calls
    /// every kernel once spends exactly [`FEE_THINK`] plus the table's sum in
    /// fees; each kernel alone spends its row; `best_hostile`'s fee grows by 3
    /// a row; a third `line_to` is refused `limit`; the free helpers charge
    /// nothing.
    #[test]
    fn the_fees_are_the_table() {
        // D-UM-26's table, literally: a fee is part of the API's contract (a
        // change is an `API_VERSION` bump), so the figures are pinned here
        // and not only read back through the constants the code charges.
        assert_eq!(
            [
                FEE_THINK,
                FEE_READ,
                FEE_HEALTH,
                FEE_RANGES,
                FEE_HOSTILE_BASE,
                FEE_HOSTILE_ROW,
                FEE_HOLDS_ROW,
                FEE_LINE_TO,
                FEE_NAV,
                FEE_RAND,
            ],
            [20, 1, 2, 2, 4, 3, 1, 20, 30, 2],
            "D-UM-26's fee table moved"
        );
        let all: String = calls()
            .iter()
            .map(|(call, _)| format!("let x = {call};"))
            .collect();
        let ran = think(&format!("fn think(v, c) {{ {all} }}"));
        let outcome = ran.result.expect("a clean think");
        let sum: u64 = calls().iter().map(|(_, fee)| fee).sum();
        assert_eq!(outcome.fee, FEE_THINK + sum);
        assert_eq!(ran.ctx.refused, None);

        for (call, fee) in calls() {
            let ran = think(&format!("fn think(v, c) {{ let x = {call}; }}"));
            let outcome = ran
                .result
                .unwrap_or_else(|fault| panic!("{call}: {fault:?}"));
            assert_eq!(outcome.fee, FEE_THINK + fee, "{call}");
        }

        // The free ones.
        let free = "fn think(v, c) {
            let a = sin_m(90000); let b = cos_m(-90000); let d = bearing_to(v, [4, 4]);
            let e = range_to(v, [4, 4]); let f = arc(350000, 10000);
            let g = stand_off_want(4000, 1000, 300); let h = v.unit; let i = c.stance;
            let j = t_ms(); let r = repath_ms(); let k = abs(-1); let l = clamp(5, 0, 3); let m = isqrt(17);
        }";
        assert_eq!(think(free).result.expect("clean").fee, FEE_THINK);

        // best_hostile grows by 3 a row, from 4 on no rows.
        let source = "fn think(v, c) { let x = best_hostile(v, 1, 1, 1, 1000, (), (), (), ()); }";
        let fees: Vec<u64> = (0..=3_usize)
            .map(|rows| {
                let mut seen = view();
                seen.contacts.truncate(rows);
                let ran = think_with(source, context_on(seen), fixture::card(), &[]);
                ran.result.expect("clean").fee
            })
            .collect();
        assert_eq!(fees, vec![20 + 4, 20 + 4 + 3, 20 + 4 + 6, 20 + 4 + 9]);
        for pair in fees.windows(2) {
            if let [less, more] = pair {
                assert_eq!(more - less, 3, "best_hostile's fee a row");
            }
        }

        // The third line_to.
        let ran = think(
            "fn think(v, c) { let a = line_to(v, v.lock); let b = line_to(v, v.lock); let d = line_to(v, v.lock); }",
        );
        assert!(
            matches!(ran.result, Err(Fault::Type(_))),
            "{:?}",
            ran.result
        );
        assert_eq!(ran.ctx.refused, Some(REFUSED_LIMIT));
        assert_eq!(ran.ctx.line_to_calls, 3);
    }

    /// 17b §2's M2.3 test 6: **`rand` is the sixth label and `k` is bounded**
    /// — three draws of `rand(1000)` are `draw(seed, Mind { unit, think_seq,
    /// n }) % 1000` for `n` 0, 1, 2; `rand(1001)` and `rand(0)` refuse `type`.
    #[test]
    fn rand_is_the_sixth_label_and_k_is_bounded() {
        let ran = think("fn think(v, c) { this.r = [rand(1000), rand(1000), rand(1000)]; }");
        let outcome = ran.result.expect("a clean think");
        let memory = mush_mind::memory::decode(&outcome.memory).expect("canonical");
        let ctx = context_on(view());
        let want: Vec<i64> = (0..3)
            .map(|n| {
                let label = DrawLabel::Mind {
                    unit: ctx.unit.0,
                    think_seq: ctx.think_seq,
                    n,
                };
                i64::try_from(draw(ctx.seed, &label) % 1_000).unwrap()
            })
            .collect();
        assert_eq!(memory.get("r"), Some(&mush_mind::memory::Value::Ints(want)));
        assert_eq!(ran.ctx.draws, 3);
        assert_eq!(
            DrawLabel::Mind {
                unit: 4,
                think_seq: 7,
                n: 0
            }
            .kind(),
            "mind"
        );
        for k in ["1001", "0", "-1"] {
            let ran = think(&format!("fn think(v, c) {{ let x = rand({k}); }}"));
            assert!(matches!(ran.result, Err(Fault::Type(_))), "rand({k})");
            assert_eq!(ran.ctx.refused, Some(REFUSED_TYPE), "rand({k})");
            assert_eq!(ran.ctx.draws, 0, "rand({k}) drew");
        }
        // rand(1) is always 0, and still a draw.
        let ran = think("fn think(v, c) { this.r = rand(1); }");
        assert!(ran.result.is_ok());
        assert_eq!(ran.ctx.draws, 1);
    }

    /// **Each kernel answers its one free function** on the full view: the
    /// value through the script equals the function called directly.
    #[test]
    fn each_kernel_answers_its_free_function() {
        let seen = view();
        let map = fixture::map();
        let cfg = crate::config::ReflexConfig::default();
        let health = health_permille(&seen);
        let best = hostile_of(&seen, &Weights::of(&cfg), health, None, |_| true);
        let expr = format!(
            "best_hostile(v, {}, {}, {}, {health}, (), (), (), ())",
            cfg.weight_range, cfg.weight_facet, cfg.weight_health
        );
        assert_eq!(
            shown(&eval(&expr).expect("best_hostile")),
            shown(&opt(best, |id| Dynamic::from_int(contact_int(id))))
        );
        assert_eq!(
            shown(&eval("health_permille(v)").expect("health")),
            health.to_string()
        );
        assert_eq!(
            shown(&eval("holds_a_hostile(v)").expect("holds")),
            holds_a_hostile(&seen).to_string()
        );
        let ranged = view_ranges(&seen, 800, seen.order.as_ref());
        assert_eq!(
            shown(&eval("ranges(v, 800, true).release_cm").expect("ranges")),
            ranged.release_cm.to_string()
        );
        let unordered = view_ranges(&seen, 800, None);
        assert_eq!(
            shown(&eval("ranges(v, 800, false).release_cm").expect("ranges")),
            unordered.release_cm.to_string()
        );
        let entry = seen
            .cell
            .and_then(|here| motor::entry_cell(&map, seen.team, CostRow::Tracked, here));
        assert_eq!(
            shown(&eval("entry_cell(v)").expect("entry")),
            shown(&cell(entry))
        );
        let interior = seen
            .cell
            .and_then(|here| motor::entry_interior_cell(&map, seen.team, CostRow::Tracked, here));
        assert_eq!(
            shown(&eval("entry_interior_cell(v)").expect("interior entry")),
            shown(&cell(interior))
        );
        let far = seen
            .cell
            .and_then(|here| motor::far_entry_cell(&map, seen.team, CostRow::Tracked, here, &[]));
        assert_eq!(
            shown(&eval("far_entry_cell(v)").expect("far")),
            shown(&cell(far))
        );
        let row = stock_mind::contact(&seen, HOSTILE).expect("held");
        assert_eq!(
            shown(
                &eval(&format!("stand_off_cell(v, {}, 500)", contact_int(HOSTILE)))
                    .expect("stand off")
            ),
            shown(&cell(stand_off_of(&map, &seen, row, 500)))
        );
        let step = motor::patrol_step(
            &map,
            seen.unit,
            seen.cell,
            false,
            motor::Walk {
                waypoint: None,
                from_waypoint: None,
                goal: None,
            },
        );
        assert_eq!(
            shown(&eval("patrol_step(v, false, (), (), ()).goal").expect("walk")),
            shown(&cell(step.walk.goal))
        );
        assert_eq!(
            shown(&eval("patrol_step(v, false, (), (), ()).clear_route").expect("walk")),
            step.clear_route.to_string()
        );
        let (waypoint, _, goal) =
            motor::fallback_waypoint(&map, (1, 4), Some((1, 4)), None, None, seen.unit);
        assert_eq!(
            shown(&eval("fall_back_to_waypoint(v, [1, 4], (), ()).goal").expect("fallback")),
            shown(&cell(goal))
        );
        assert_eq!(
            shown(&eval("fall_back_to_waypoint(v, [1, 4], (), ()).waypoint").expect("fallback")),
            shown(&opt(waypoint, int))
        );
        assert_eq!(
            shown(&eval("sin_m(90000)").expect("sin")),
            sin_fx(90_000).to_string()
        );
        assert_eq!(
            shown(&eval("cos_m(-90000)").expect("cos")),
            cos_fx(270_000).to_string()
        );
        assert_eq!(shown(&eval("arc(350000, 10000)").expect("arc")), "20000");
        assert_eq!(
            shown(&eval("stand_off_want(4000, 1000, 300)").expect("want")),
            doctrine::stand_off_want(4_000, 1_000, 300).to_string()
        );
        let (x, y, z) = cell_point(&map, (4, 4));
        assert_eq!(
            shown(&eval("range_to(v, [4, 4])").expect("range")),
            range_between(seen.x_cm, seen.y_cm, 0, x, y, z).to_string()
        );
        assert_eq!(
            shown(&eval("bearing_to(v, [4, 4])").expect("bearing")),
            bearing_of(x - seen.x_cm, y - seen.y_cm).to_string()
        );
        assert_eq!(
            shown(&eval(&format!("line_to(v, {})", contact_int(HOSTILE))).expect("line")),
            line_between(&map, seen.cell, row.cell).to_string()
        );
        // An unplotted row is a line, with no trace (T1's `line_between`).
        assert_eq!(
            shown(&eval(&format!("line_to(v, {})", contact_int(BLIP))).expect("line")),
            "true"
        );
        assert_eq!(shown(&eval("contact(v, 999)").expect("lookup")), "()");
        assert_eq!(
            shown(&eval("contact(v, v.lock).designator").expect("lookup")),
            "\"BB\""
        );
    }

    /// **`ranges` passes the order only when the script obeys one** (17b
    /// §5's feas F6): on a view whose order caps the fight at the `close`
    /// band (a third of the reach, under the cannon's inner band), every
    /// field of `ranges(v, band, true)` is `view_ranges` with the order and
    /// every field of `ranges(v, band, false)` is `view_ranges` with none —
    /// and the two differ, so a wrapper that always (or never) passes the
    /// view's order answers one of them wrong.
    #[test]
    fn ranges_passes_the_order_only_when_the_script_obeys_one() {
        let mut seen = view();
        if let Some(order) = seen.order.as_mut() {
            order.max_range_band = Some(RangeBand::Close);
        }
        let obeyed = view_ranges(&seen, 800, seen.order.as_ref());
        let free = view_ranges(&seen, 800, None);
        assert_ne!(obeyed, free, "the order must move the ranges here");
        assert!(obeyed.release_cm < free.release_cm);
        for (flag, want) in [("true", obeyed), ("false", free)] {
            for (field, value) in [
                ("engage_cm", want.engage_cm),
                ("release_cm", want.release_cm),
                ("stand_cm", want.stand_cm),
                ("limit_cm", want.limit_cm),
            ] {
                assert_eq!(
                    on(&seen, &format!("ranges(v, 800, {flag}).{field}")),
                    value.to_string(),
                    "ranges(v, 800, {flag}).{field}"
                );
            }
        }
    }

    /// **`best_hostile` answers `stock_mind::best_hostile` over the script's
    /// weights, prefer, ceiling and sector** — on a view of three classified
    /// hostiles (near on the flank, middling face-on, far rear-on) where the
    /// weights, the prefer, the ceiling and the sector each pick a different
    /// row: every answer equal to the one function under the `admit` T1's
    /// `stand` builds, and to the row named here.
    #[test]
    #[allow(clippy::type_complexity)]
    fn best_hostile_answers_its_weights_prefer_ceiling_and_sector() {
        const REAR: ContactId = ContactId(0x44);
        let mut seen = view();
        // The unidentified return, classified hostile: 600 m face-on at 135°.
        for row in &mut seen.contacts {
            if row.id == BLIP {
                row.class = Classification::Classified { party: PartyId::BLUE };
                row.reads_dead = Some(false);
            }
        }
        // A third hostile: 700 m, rear-on, at 200°.
        let mut rear = seen.contacts.first().cloned().expect("the hostile");
        rear.id = REAR;
        rear.alias = None;
        rear.range_m = 700;
        rear.bearing_deg = 200;
        rear.aspect = Aspect::Rear;
        rear.cell = Some((6, 7));
        seen.contacts.push(rear);

        // T1's stand's admit: a ceiling, then a whole-degree sector.
        let t1 = |weights: (u32, u32, u32),
                  prefer: Option<ContactId>,
                  ceiling: Option<u64>,
                  sector: Option<(u32, u32)>| {
            let weights = Weights {
                range: weights.0,
                facet: weights.1,
                health: weights.2,
            };
            hostile_of(&seen, &weights, 1_000, prefer, |row| {
                ceiling.is_none_or(|ceiling| row_range_cm(row) <= ceiling)
                    && sector.is_none_or(|(facing, half)| {
                        arc_between(row.bearing_deg.saturating_mul(1_000), facing) <= half
                    })
            })
        };
        let arg = |value: Option<i64>| value.map_or_else(|| "()".to_owned(), |v| v.to_string());
        let cases: [(
            (u32, u32, u32),
            Option<ContactId>,
            Option<u64>,
            Option<(u32, u32)>,
            Option<ContactId>,
        ); 9] = [
            // Range alone: the nearest.
            ((1, 0, 0), None, None, None, Some(HOSTILE)),
            // Facet alone: the rear-on row.
            ((0, 1, 0), None, None, None, Some(REAR)),
            // Health is one addend for every row, so the facet still decides
            // (a health weight read into the range slot picks the nearest).
            ((0, 1, 5), None, None, None, Some(REAR)),
            // A prefer scores nought: the worst row by range wins.
            ((1, 0, 0), Some(REAR), None, None, Some(REAR)),
            // A ceiling under the rear-on row's 700 m: the flank one.
            ((0, 1, 0), None, Some(65_000), None, Some(HOSTILE)),
            // A ceiling under every row: none.
            ((1, 0, 0), None, Some(20_000), None, None),
            // A sector on 135° ± 10°: the face-on row only.
            ((1, 0, 0), None, None, Some((135_000, 10_000)), Some(BLIP)),
            // A sector on 200° ± 30°: the rear-on row only.
            ((1, 0, 0), None, None, Some((200_000, 30_000)), Some(REAR)),
            // A sector on 90° ± 0°: the flank row, on its whole degree.
            ((0, 1, 0), None, None, Some((90_000, 0)), Some(HOSTILE)),
        ];
        for (weights, prefer, ceiling, sector, want) in cases {
            let expr = format!(
                "best_hostile(v, {}, {}, {}, 1000, {}, {}, {}, {})",
                weights.0,
                weights.1,
                weights.2,
                arg(prefer.map(contact_int)),
                arg(ceiling.map(|c| i64::try_from(c).unwrap())),
                arg(sector.map(|(facing, _)| i64::from(facing))),
                arg(sector.map(|(_, half)| i64::from(half))),
            );
            assert_eq!(t1(weights, prefer, ceiling, sector), want, "{expr}");
            assert_eq!(on(&seen, &expr), id_shown(want), "{expr}");
        }
    }

    /// **`is_dry`, `would_overheat` and `fight_limit_cm` answer their
    /// functions** — on the full view, and on views where each answer flips:
    /// every round spent (dry, and nothing loaded to fight at), and the heat
    /// at its ceiling (the cannon, which makes heat, would overheat; the
    /// coax, which makes none, would not).
    #[test]
    fn the_reads_answer_their_functions_and_flip_with_the_view() {
        let full = view();
        let mut dry = view();
        for row in &mut dry.weapons {
            row.rounds = 0;
        }
        let mut hot = view();
        hot.heat = Some(FrameHeat {
            heat: 100,
            max: 100,
        });
        for seen in [&full, &dry, &hot] {
            assert_eq!(on(seen, "is_dry(v)"), is_dry(seen).to_string());
            assert_eq!(
                on(seen, "fight_limit_cm(v)"),
                fight_limit_cm(seen).to_string()
            );
            for weapon in &seen.weapons {
                assert_eq!(
                    on(
                        seen,
                        &format!("would_overheat(v, {:?})", weapon.id.as_str())
                    ),
                    stock_mind::would_overheat(seen, weapon).to_string(),
                    "{}",
                    weapon.id
                );
            }
        }
        // The answers themselves, so a constant wrapper cannot pass.
        assert_eq!(on(&full, "is_dry(v)"), "false");
        assert_eq!(on(&dry, "is_dry(v)"), "true");
        assert_eq!(on(&full, "fight_limit_cm(v)"), "80000");
        assert_eq!(on(&dry, "fight_limit_cm(v)"), "0");
        assert_eq!(on(&full, r#"would_overheat(v, "cannon")"#), "false");
        assert_eq!(on(&hot, r#"would_overheat(v, "cannon")"#), "true");
        assert_eq!(on(&hot, r#"would_overheat(v, "coax")"#), "false");
    }

    /// **`line_to` traces the ground** — on a window with a ridge at (2, 4)
    /// between the hull at (1, 4) and the hostile's plot at (4, 4), the
    /// kernel answers `false` with `line_between`, and a row off the ridge's
    /// line (the friend at (1, 0)) still answers `true`.
    #[test]
    fn line_to_answers_false_across_a_ridge() {
        let mut ctx = context_on(view());
        let mut cells = vec![crate::arena::test_support::ground(); 64];
        if let Some(ridge) = cells.get_mut(4 * 8 + 2) {
            ridge.elev_cm = 10_000;
        }
        std::sync::Arc::make_mut(&mut ctx.map).terrain =
            ArenaTerrain::new(8, 8, 1_000, cells).expect("the ridge window");
        let map = std::sync::Arc::clone(&ctx.map);
        let seen = std::sync::Arc::clone(&ctx.view);
        let hostile = stock_mind::contact(&seen, HOSTILE).expect("held");
        let friend = stock_mind::contact(&seen, fixture::FRIEND).expect("held");
        assert!(!line_between(&map, seen.cell, hostile.cell), "the ridge");
        assert!(line_between(&map, seen.cell, friend.cell), "off the ridge");
        let expr = format!(
            "[line_to(v, {}), line_to(v, {})]",
            contact_int(HOSTILE),
            contact_int(fixture::FRIEND)
        );
        let answer = eval_in(ctx, &expr).expect("two lines");
        assert_eq!(shown(&answer), "[false, true]");
    }

    /// The kernels refuse a wrong shape `type` and an unheld id `unknown`,
    /// through the context.
    #[test]
    fn the_kernels_refuse_by_word() {
        for (call, word) in [
            (
                "best_hostile(v, -1, 1, 1, 1000, (), (), (), ())",
                REFUSED_TYPE,
            ),
            (
                "best_hostile(v, 1, 1, 1, 1000, (), (), 9000, ())",
                REFUSED_TYPE,
            ),
            (
                "best_hostile(v, 1, 1, 1, 1000, 1.5, (), (), ())",
                REFUSED_TYPE,
            ),
            ("would_overheat(v, \"laser\")", REFUSED_UNKNOWN),
            ("line_to(v, 999)", REFUSED_UNKNOWN),
            ("stand_off_cell(v, 999, 500)", REFUSED_UNKNOWN),
            ("stand_off_cell(v, v.lock, -5)", REFUSED_TYPE),
            ("patrol_step(v, false, -1, (), ())", REFUSED_TYPE),
            ("fall_back_to_waypoint(v, [1], (), ())", REFUSED_TYPE),
            ("bearing_to(v, [1.5, 2])", REFUSED_TYPE),
            ("ranges(v, -1, false)", REFUSED_TYPE),
        ] {
            let ran = think(&format!("fn think(v, c) {{ let x = {call}; }}"));
            assert!(
                matches!(ran.result, Err(Fault::Type(_))),
                "{call}: {:?}",
                ran.result
            );
            assert_eq!(ran.ctx.refused, Some(word), "{call}");
        }
    }
}
