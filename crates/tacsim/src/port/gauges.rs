//! **The ship's gauges** (doc 18 cut IC6; doc 18 §4.2 "Compartments", 18a §3,
//! D-18-39; lead ruling L11 of the IC6 brief).
//!
//! Pure renders: no `World`, no `Ctx`. The inputs are the goods table
//! ([`GoodsTable`]), a stowage compartment ([`Compartment::from_stowage`]) and
//! its contents (good id → ledger units, read by the caller from the hold
//! vault's pools at look time), and every figure goes through the one capacity
//! function ([`crate::port::capacity`]). **No pack text carries a load**
//! (D-18-39): a room's description says where the gauge is, and these lines say
//! what it reads, so no two hulls and no two days read alike.
//!
//! The conventions (the IC6 brief's item 9):
//!
//! - a **multi-good** compartment accepts more than one good of the table (the
//!   cargo hold); a **single-good** one accepts exactly one (the tank, the
//!   stores locker). The goods a compartment accepts are the table's goods whose
//!   `stowage` it accepts, **in table order**;
//! - `si.mass_kg` renders as whole tonnes (truncated) and `si.volume_l` as
//!   cubic metres to a tenth (truncated, never rounded up, doc 18 §4.3's UI
//!   rule) for a load, and as whole units for a limit (`100 t`, `120 m³`); any
//!   other measure as `{n} {unit}` from the table's [`crate::definitions::MeasureDef`];
//! - `FULL` names every limited measure whose load has reached its limit, by the
//!   table's `word` (`mass`, `packed volume`);
//! - the instrument and compartment words are two small template-id tables in
//!   code ([`compartment_words`], [`room_instrument`]): template ids, never pack
//!   text (L11). The burn and reserve lines are cut IC9's.
//!
//! An error is a [`CapacityError`]; the caller renders `{Instrument}: dark.`
//! ([`DARK`]) and logs it.

use std::collections::BTreeMap;

use mush_core::CanonicalId;

use crate::definitions::logistics::{MASS_KG, VOLUME_L};
use crate::definitions::{GoodDef, GoodsTable};
use crate::port::capacity::{CapacityError, Compartment, format_m3, format_tonnes, load, max_more};

/// The word a failed read renders (`Load cells: dark.`).
pub const DARK: &str = "dark";

/// **One compartment's reading**: its stowage id, the compartment and its
/// contents (good id → ledger units).
pub type Reading = (CanonicalId, Compartment, BTreeMap<CanonicalId, u64>);

/// **The compartment words** (`COMPARTMENT_WORDS`, L11): the lower-case word
/// `+ship`'s hold line uses and the upper-case one the summary displays use.
/// `cargo → (cargo, HOLD)`, `propellant → (tank, TANKAGE)`, `provisions →
/// (locker, STORES)`; any other compartment id reads as itself.
#[must_use]
pub fn compartment_words(id: &CanonicalId) -> (String, String) {
    match id.as_str() {
        "cargo" => ("cargo".to_owned(), "HOLD".to_owned()),
        "propellant" => ("tank".to_owned(), "TANKAGE".to_owned()),
        "provisions" => ("locker".to_owned(), "STORES".to_owned()),
        other => (other.to_owned(), other.to_ascii_uppercase()),
    }
}

/// **The room instruments** (`ROOM_INSTRUMENTS`, L11): the label a hull room's
/// gauge lines stand under, by the room's template id (18a §3: the load cells
/// in the hold, the manifest slate in the passage, the displays on the bridge,
/// the locker tally in the crew compartment, the tank gauges in engineering).
#[must_use]
pub fn room_instrument(room: &CanonicalId) -> &'static str {
    match room.as_str() {
        "hold" => "Load cells",
        "passage" => "Manifest slate",
        "bridge" => "Displays",
        "crew" => "Locker tally",
        "engineering" => "Tank gauges",
        _ => "Gauges",
    }
}

/// The goods `comp` accepts, in table order.
fn accepted<'a>(goods: &'a GoodsTable, comp: &Compartment) -> Vec<&'a GoodDef> {
    goods
        .goods
        .iter()
        .filter(|good| comp.accepts.contains(&good.stowage))
        .collect()
}

/// Whether `comp` holds exactly one good of the table (the tank, the locker).
fn single_good<'a>(goods: &'a GoodsTable, comp: &Compartment) -> Option<&'a GoodDef> {
    match accepted(goods, comp).as_slice() {
        [only] => Some(*only),
        _ => None,
    }
}

/// The word a quantity of `good` is counted in: `t`, `crew-days`, else the
/// ledger unit.
fn unit_word(good: &GoodDef) -> String {
    match good.ledger_unit.as_str() {
        "t" => "t".to_owned(),
        "crew_day" => "crew-days".to_owned(),
        other => other.to_owned(),
    }
}

/// The unit an unrecognised measure prints in, from the table's registry.
fn measure_unit(goods: &GoodsTable, measure: &CanonicalId) -> String {
    goods
        .measures
        .iter()
        .find(|m| &m.id == measure)
        .map_or_else(|| measure.to_string(), |m| m.unit.to_string())
}

/// The word `FULL` names a measure by.
fn measure_word(goods: &GoodsTable, measure: &CanonicalId) -> String {
    goods
        .measures
        .iter()
        .find(|m| &m.id == measure)
        .map_or_else(|| measure.to_string(), |m| m.word.clone())
}

/// A load in `measure`: `20 t`, `8.0 m³`, else `{n} {unit}`.
fn render_load(goods: &GoodsTable, measure: &CanonicalId, value: u64) -> String {
    match measure.as_str() {
        MASS_KG => format!("{} t", format_tonnes(value)),
        VOLUME_L => format!("{} m³", format_m3(value)),
        _ => format!("{value} {}", measure_unit(goods, measure)),
    }
}

/// A limit in `measure`: whole tonnes, whole cubic metres, else `{n} {unit}`.
fn render_limit(goods: &GoodsTable, measure: &CanonicalId, value: u64) -> String {
    match measure.as_str() {
        MASS_KG => format!("{} t", value / 1_000),
        VOLUME_L => format!("{} m³", value / 1_000),
        _ => format!("{value} {}", measure_unit(goods, measure)),
    }
}

/// `{load}/{limit} {unit}` for one measure, the unit written once.
fn render_pair(goods: &GoodsTable, measure: &CanonicalId, load: u64, limit: u64) -> String {
    match measure.as_str() {
        MASS_KG => format!("{}/{} t", format_tonnes(load), limit / 1_000),
        VOLUME_L => format!("{}/{} m³", format_m3(load), limit / 1_000),
        _ => format!("{load}/{limit} {}", measure_unit(goods, measure)),
    }
}

/// `FULL: mass and packed volume` when some limited measure is at its limit.
fn full_text(
    goods: &GoodsTable,
    comp: &Compartment,
    loaded: &BTreeMap<CanonicalId, u64>,
) -> Option<String> {
    let full: Vec<String> = comp
        .limits
        .iter()
        .filter(|(measure, limit)| loaded.get(*measure).copied().unwrap_or(0) >= **limit)
        .map(|(measure, _)| measure_word(goods, measure))
        .collect();
    (!full.is_empty()).then(|| format!("FULL: {}", full.join(" and ")))
}

/// **`{load}/{limit}` per limited measure**, joined ` · ` in the limit map's
/// order (mass before volume): `100/100 t · 120.0/120 m³`. With `with_full`
/// and a measure at its limit, ` · FULL: mass and packed volume` follows.
///
/// # Errors
///
/// As [`load`].
pub fn measures_line(
    goods: &GoodsTable,
    comp: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
    with_full: bool,
) -> Result<String, CapacityError> {
    let loaded = load(goods, comp, contents)?;
    let mut parts: Vec<String> = comp
        .limits
        .iter()
        .map(|(measure, limit)| {
            render_pair(
                goods,
                measure,
                loaded.get(measure).copied().unwrap_or(0),
                *limit,
            )
        })
        .collect();
    if with_full {
        parts.extend(full_text(goods, comp, &loaded));
    }
    Ok(parts.join(" · "))
}

/// **The non-zero goods**, table order, `{qty} {unit} {label}` joined `, `
/// (`20 t metal concentrate, 80 t water ice`); `empty` when none.
#[must_use]
pub fn goods_list(
    goods: &GoodsTable,
    comp: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
) -> String {
    let listed: Vec<String> = accepted(goods, comp)
        .into_iter()
        .filter_map(|good| {
            let qty = contents.get(&good.id).copied().unwrap_or(0);
            (qty > 0).then(|| format!("{qty} {} {}", unit_word(good), good.label))
        })
        .collect();
    if listed.is_empty() {
        "empty".to_owned()
    } else {
        listed.join(", ")
    }
}

/// A single-good compartment's good, quantity and capacity (qty +
/// [`max_more`]). `None` for a compartment that is not single-good.
fn single_reading<'a>(
    goods: &'a GoodsTable,
    comp: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
) -> Result<Option<(&'a GoodDef, u64, u64)>, CapacityError> {
    let Some(good) = single_good(goods, comp) else {
        return Ok(None);
    };
    let qty = contents.get(&good.id).copied().unwrap_or(0);
    let (more, _) = max_more(goods, comp, contents, &good.id)?;
    let cap = qty.checked_add(more).ok_or(CapacityError::Overflow)?;
    Ok(Some((good, qty, cap)))
}

/// **A single-good compartment against its capacity**: `{qty}/{cap} {unit}`
/// (`42/80 t`, `21/120 crew-days`). A compartment that is not single-good reads
/// its measures instead ([`measures_line`]).
///
/// # Errors
///
/// As [`max_more`].
pub fn single_summary(
    goods: &GoodsTable,
    comp: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
) -> Result<String, CapacityError> {
    match single_reading(goods, comp, contents)? {
        Some((good, qty, cap)) => Ok(format!("{qty}/{cap} {}", unit_word(good))),
        None => measures_line(goods, comp, contents, false),
    }
}

/// One compartment's summary: the measures for a multi-good one, the
/// capacity reading for a single-good one.
fn summary(
    goods: &GoodsTable,
    comp: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
) -> Result<String, CapacityError> {
    if single_good(goods, comp).is_some() {
        single_summary(goods, comp, contents)
    } else {
        measures_line(goods, comp, contents, false)
    }
}

/// **`+ship`'s hold line** (doc 18 §2 step 5): `Hold: ` + per compartment, in
/// stowage order, `{word} {summary}` joined ` · ` + `.` — `Hold: cargo 0/100 t ·
/// 0.0/120 m³ · tank 0/80 t · locker 0/120 crew-days.`
///
/// # Errors
///
/// As [`load`] and [`max_more`], for any compartment.
pub fn ship_hold_line(goods: &GoodsTable, readings: &[Reading]) -> Result<String, CapacityError> {
    let mut parts: Vec<String> = Vec::with_capacity(readings.len());
    for (id, comp, contents) in readings {
        let (word, _) = compartment_words(id);
        parts.push(format!("{word} {}", summary(goods, comp, contents)?));
    }
    Ok(format!("Hold: {}.", parts.join(" · ")))
}

/// The load-cell body of a multi-good compartment: one line per accepted good
/// (zeros included — 18a: "one line per good the hold accepts"), TOTAL, LIMIT,
/// and FULL when a measure is at its limit; each line prefixed `indent`.
fn load_cell_lines(
    goods: &GoodsTable,
    comp: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
    indent: &str,
) -> Result<Vec<String>, CapacityError> {
    let loaded = load(goods, comp, contents)?;
    let mut out: Vec<String> = Vec::new();
    for good in accepted(goods, comp) {
        let qty = contents.get(&good.id).copied().unwrap_or(0);
        let mut parts: Vec<String> = Vec::with_capacity(comp.limits.len());
        for measure in comp.limits.keys() {
            let per_unit = good
                .per_unit(measure)
                .ok_or_else(|| CapacityError::MissingMeasure {
                    good: good.id.clone(),
                    measure: measure.clone(),
                })?;
            let value = qty.checked_mul(per_unit).ok_or(CapacityError::Overflow)?;
            parts.push(render_load(goods, measure, value));
        }
        out.push(format!("{indent}{}: {}", good.label, parts.join(" · ")));
    }
    let total: Vec<String> = comp
        .limits
        .keys()
        .map(|measure| render_load(goods, measure, loaded.get(measure).copied().unwrap_or(0)))
        .collect();
    out.push(format!("{indent}TOTAL: {}", total.join(" · ")));
    let limit: Vec<String> = comp
        .limits
        .iter()
        .map(|(measure, limit)| render_limit(goods, measure, *limit))
        .collect();
    out.push(format!("{indent}LIMIT: {}", limit.join(" · ")));
    if let Some(full) = full_text(goods, comp, &loaded) {
        out.push(format!("{indent}{full}"));
    }
    Ok(out)
}

/// **One hull room's gauge lines** (18a §3; doc 18 §4.2 "Compartments").
/// `shown` is the compartments `interior.gauges` lists for this room, in the
/// gauges map's order; none → no lines. With the room's instrument label `L`:
///
/// - one multi-good compartment (the Cargo Hold): `L:` and the load-cell body;
/// - one single-good compartment: `L: {good label}, {qty} of {cap} {unit}.`;
/// - several (the passage, the bridge): `L:` then per compartment `  {WORD}
///   {goods list}; {measures with FULL}` (multi-good) or `  {WORD} {qty}/{cap}
///   {unit}` (single-good).
///
/// # Errors
///
/// As [`load`] and [`max_more`], for any compartment.
pub fn room_lines(
    room: &CanonicalId,
    goods: &GoodsTable,
    shown: &[Reading],
) -> Result<Vec<String>, CapacityError> {
    let label = room_instrument(room);
    match shown {
        [] => Ok(Vec::new()),
        [(_, comp, contents)] => match single_reading(goods, comp, contents)? {
            Some((good, qty, cap)) => Ok(vec![format!(
                "{label}: {}, {qty} of {cap} {}.",
                good.label,
                unit_word(good)
            )]),
            None => {
                let mut out = vec![format!("{label}:")];
                out.extend(load_cell_lines(goods, comp, contents, "  ")?);
                Ok(out)
            }
        },
        several => {
            let mut out = vec![format!("{label}:")];
            for (id, comp, contents) in several {
                let (_, upper) = compartment_words(id);
                if single_good(goods, comp).is_some() {
                    out.push(format!(
                        "  {upper} {}",
                        single_summary(goods, comp, contents)?
                    ));
                } else {
                    out.push(format!(
                        "  {upper} {}; {}",
                        goods_list(goods, comp, contents),
                        measures_line(goods, comp, contents, true)?
                    ));
                }
            }
            Ok(out)
        }
    }
}

/// **`+ship/hold`'s lines**: `Hold of the {name} ({mark}):`, then per
/// compartment in stowage order a multi-good one as `  {WORD}` over its
/// load-cell body indented four spaces, a single-good one as `  {WORD} {good
/// label}, {qty} of {cap} {unit}`.
///
/// # Errors
///
/// As [`load`] and [`max_more`], for any compartment.
pub fn hold_lines(
    name: &str,
    mark: &str,
    goods: &GoodsTable,
    readings: &[Reading],
) -> Result<Vec<String>, CapacityError> {
    let mut out = vec![format!("Hold of the {name} ({mark}):")];
    for (id, comp, contents) in readings {
        let (_, upper) = compartment_words(id);
        match single_reading(goods, comp, contents)? {
            Some((good, qty, cap)) => out.push(format!(
                "  {upper} {}, {qty} of {cap} {}",
                good.label,
                unit_word(good)
            )),
            None => {
                out.push(format!("  {upper}"));
                out.extend(load_cell_lines(goods, comp, contents, "    ")?);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::StowageDef;

    fn cid(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("id")
    }

    fn table() -> GoodsTable {
        crate::definitions::goods::goods_table().expect("the compiled-in goods table")
    }

    /// The Dray's three compartments, as `dray.json` authors them (the
    /// integration tests read the shipped definition itself).
    fn dray() -> Vec<(CanonicalId, Compartment)> {
        let mk = |accepts: &[&str], limits: &[(&str, u64)]| StowageDef {
            accepts: accepts.iter().map(|a| cid(a)).collect(),
            limits: limits.iter().map(|(m, l)| (cid(m), *l)).collect(),
        };
        vec![
            (
                cid("cargo"),
                Compartment::from_stowage(
                    &cid("cargo"),
                    &mk(&["stow.bulk"], &[(MASS_KG, 100_000), (VOLUME_L, 120_000)]),
                ),
            ),
            (
                cid("propellant"),
                Compartment::from_stowage(
                    &cid("propellant"),
                    &mk(&["stow.propellant"], &[(MASS_KG, 80_000)]),
                ),
            ),
            (
                cid("provisions"),
                Compartment::from_stowage(
                    &cid("provisions"),
                    &mk(&["stow.provisions"], &[(MASS_KG, 480), (VOLUME_L, 1_200)]),
                ),
            ),
        ]
    }

    fn comp(id: &str) -> Compartment {
        dray()
            .into_iter()
            .find(|(cid_, _)| cid_.as_str() == id)
            .map(|(_, c)| c)
            .expect("compartment")
    }

    fn contents(rows: &[(&str, u64)]) -> BTreeMap<CanonicalId, u64> {
        rows.iter().map(|(g, q)| (cid(g), *q)).collect()
    }

    fn reading(id: &str, rows: &[(&str, u64)]) -> Reading {
        (cid(id), comp(id), contents(rows))
    }

    #[test]
    fn ic6_the_load_cells_list_every_good_with_both_limits() {
        let goods = table();
        let full = reading("cargo", &[("metals", 20), ("volatiles", 80)]);
        assert_eq!(
            room_lines(&cid("hold"), &goods, std::slice::from_ref(&full)).expect("reads"),
            vec![
                "Load cells:",
                "  metal concentrate: 20 t · 8.0 m³",
                "  water ice: 80 t · 112.0 m³",
                "  structural alloy stock: 0 t · 0.0 m³",
                "  yard fittings, crated: 0 t · 0.0 m³",
                "  TOTAL: 100 t · 120.0 m³",
                "  LIMIT: 100 t · 120 m³",
                "  FULL: mass and packed volume",
            ]
        );
        let ice = reading("cargo", &[("volatiles", 85)]);
        assert_eq!(
            room_lines(&cid("hold"), &goods, &[ice]).expect("reads"),
            vec![
                "Load cells:",
                "  metal concentrate: 0 t · 0.0 m³",
                "  water ice: 85 t · 119.0 m³",
                "  structural alloy stock: 0 t · 0.0 m³",
                "  yard fittings, crated: 0 t · 0.0 m³",
                "  TOTAL: 85 t · 119.0 m³",
                "  LIMIT: 100 t · 120 m³",
            ]
        );
    }

    #[test]
    fn ic6_full_names_the_binding_measure() {
        let goods = table();
        let cargo = comp("cargo");
        // 100 t of alloys is 30 m³: the mass binds.
        assert_eq!(
            measures_line(
                &goods,
                &cargo,
                &contents(&[("structural_alloys", 100)]),
                true
            )
            .expect("reads"),
            "100/100 t · 30.0/120 m³ · FULL: mass"
        );
        // 40 t of fittings and 20 t of ice is exactly 120 m³ at 60 t: the packed
        // volume binds. (52 t of fittings alone is 119.6 m³, short of the limit.)
        assert_eq!(
            measures_line(
                &goods,
                &cargo,
                &contents(&[("industrial_parts", 40), ("volatiles", 20)]),
                true
            )
            .expect("reads"),
            "60/100 t · 120.0/120 m³ · FULL: packed volume"
        );
        assert_eq!(
            measures_line(&goods, &cargo, &contents(&[("industrial_parts", 52)]), true)
                .expect("reads"),
            "52/100 t · 119.6/120 m³"
        );
        // Without `with_full`, no FULL.
        assert_eq!(
            measures_line(
                &goods,
                &cargo,
                &contents(&[("structural_alloys", 100)]),
                false
            )
            .expect("reads"),
            "100/100 t · 30.0/120 m³"
        );
    }

    #[test]
    fn ic6_single_good_compartments_read_against_their_capacity() {
        let goods = table();
        assert_eq!(
            single_summary(
                &goods,
                &comp("propellant"),
                &contents(&[("propellant", 42)])
            )
            .expect("reads"),
            "42/80 t"
        );
        assert_eq!(
            single_summary(
                &goods,
                &comp("provisions"),
                &contents(&[("provisions", 21)])
            )
            .expect("reads"),
            "21/120 crew-days"
        );
        assert_eq!(
            room_lines(
                &cid("engineering"),
                &goods,
                &[reading("propellant", &[("propellant", 42)])]
            )
            .expect("reads"),
            vec!["Tank gauges: reaction mass, 42 of 80 t."]
        );
        assert_eq!(
            room_lines(
                &cid("crew"),
                &goods,
                &[reading("provisions", &[("provisions", 21)])]
            )
            .expect("reads"),
            vec!["Locker tally: voyage provisions, 21 of 120 crew-days."]
        );
    }

    #[test]
    fn ic6_the_summary_rooms_render_every_compartment() {
        let goods = table();
        let shown = [
            reading("cargo", &[("metals", 20), ("volatiles", 80)]),
            reading("propellant", &[("propellant", 42)]),
            reading("provisions", &[("provisions", 21)]),
        ];
        let body = [
            "  HOLD 20 t metal concentrate, 80 t water ice; 100/100 t · 120.0/120 m³ · FULL: mass \
             and packed volume",
            "  TANKAGE 42/80 t",
            "  STORES 21/120 crew-days",
        ];
        let mut passage = vec!["Manifest slate:"];
        passage.extend(body);
        assert_eq!(
            room_lines(&cid("passage"), &goods, &shown).expect("reads"),
            passage
        );
        let mut bridge = vec!["Displays:"];
        bridge.extend(body);
        assert_eq!(
            room_lines(&cid("bridge"), &goods, &shown).expect("reads"),
            bridge
        );
        // Empty: the zero readings.
        let empty = [
            reading("cargo", &[]),
            reading("propellant", &[]),
            reading("provisions", &[]),
        ];
        assert_eq!(
            room_lines(&cid("bridge"), &goods, &empty).expect("reads"),
            vec![
                "Displays:",
                "  HOLD empty; 0/100 t · 0.0/120 m³",
                "  TANKAGE 0/80 t",
                "  STORES 0/120 crew-days",
            ]
        );
        // A room no gauge lists reads nothing.
        assert!(
            room_lines(&cid("airlock"), &goods, &[])
                .expect("reads")
                .is_empty()
        );
    }

    #[test]
    fn ic6_the_ship_hold_line_matches_doc_18_step_5() {
        let goods = table();
        let empty = [
            reading("cargo", &[]),
            reading("propellant", &[]),
            reading("provisions", &[]),
        ];
        assert_eq!(
            ship_hold_line(&goods, &empty).expect("reads"),
            "Hold: cargo 0/100 t · 0.0/120 m³ · tank 0/80 t · locker 0/120 crew-days."
        );
        let outbound = [
            reading("cargo", &[("metals", 20), ("volatiles", 80)]),
            reading("propellant", &[("propellant", 42)]),
            reading("provisions", &[("provisions", 21)]),
        ];
        assert_eq!(
            ship_hold_line(&goods, &outbound).expect("reads"),
            "Hold: cargo 100/100 t · 120.0/120 m³ · tank 42/80 t · locker 21/120 crew-days."
        );
        assert_eq!(
            hold_lines("Salt Moth", "FPC-D-0401", &goods, &outbound).expect("reads"),
            vec![
                "Hold of the Salt Moth (FPC-D-0401):",
                "  HOLD",
                "    metal concentrate: 20 t · 8.0 m³",
                "    water ice: 80 t · 112.0 m³",
                "    structural alloy stock: 0 t · 0.0 m³",
                "    yard fittings, crated: 0 t · 0.0 m³",
                "    TOTAL: 100 t · 120.0 m³",
                "    LIMIT: 100 t · 120 m³",
                "    FULL: mass and packed volume",
                "  TANKAGE reaction mass, 42 of 80 t",
                "  STORES voyage provisions, 21 of 120 crew-days",
            ]
        );
    }

    #[test]
    fn ic6_m3_truncates_in_every_gauge() {
        let goods = table();
        let cargo = comp("cargo");
        // No good of the table packs to 99 L, so the renders are pinned directly.
        assert_eq!(
            render_pair(&goods, &cid(VOLUME_L), 99, 120_000),
            "0.0/120 m³"
        );
        assert_eq!(render_load(&goods, &cid(VOLUME_L), 99), "0.0 m³");
        assert_eq!(render_load(&goods, &cid(VOLUME_L), 119_999), "119.9 m³");
        // And through a compartment: 1 t of metals is 400 L = 0.4 m³.
        assert_eq!(
            measures_line(&goods, &cargo, &contents(&[("metals", 1)]), true).expect("reads"),
            "1/100 t · 0.4/120 m³"
        );
        // An unknown good is an error the caller renders dark.
        assert!(measures_line(&goods, &cargo, &contents(&[("nonesuch", 1)]), true).is_err());
    }
}
