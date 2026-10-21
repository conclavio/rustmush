//! **The capacity function** (doc 18 cut IC2; doc 18 §4.3 "The capacity
//! function", §4.2 "Compartments"; D-18-28 as replaced by R7).
//!
//! Pure: no `World`, no `Ctx`, checked `u64` throughout. A compartment is doc 06
//! D3's measure map — `si.mass_kg` and `si.volume_l` on the Dray's cargo hold,
//! mass alone on the tank — and a load is the sum, per **limited** measure, of
//! each good's quantity times its per-unit figure from the goods table
//! ([`crate::definitions::GoodsTable`]). Mass and packed volume stay separate:
//! a volume-bound load and a mass-bound load are different refusals, and
//! nothing here collapses them into one "fullness".
//!
//! [`admits`] answers in **one enum** — admitted, refused, or an error — so no
//! caller can `?` past a refusal: a refusal is an answer the caller renders
//! (cut IC8 writes the sentences from [`CapacityRefusal`]'s fields), an error is
//! a malformed question (a good the table does not know, one the compartment
//! does not accept, one that declares no figure for a limited measure, or an
//! overflow). Player text says "packed volume" or "m³", never "volume" alone;
//! [`format_m3`] truncates, never rounds up.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::CanonicalId;

use crate::definitions::{GoodsTable, StowageDef};

/// **One stowage compartment**, as the capacity function reads it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Compartment {
    /// The compartment id (`cargo`).
    pub id: CanonicalId,
    /// The stowage tags it accepts.
    pub accepts: BTreeSet<CanonicalId>,
    /// Measure → limit.
    pub limits: BTreeMap<CanonicalId, u64>,
}

impl Compartment {
    /// A compartment from a hull definition's stowage block.
    #[must_use]
    pub fn from_stowage(id: &CanonicalId, def: &StowageDef) -> Compartment {
        Compartment {
            id: id.clone(),
            accepts: def.accepts.clone(),
            limits: def.limits.clone(),
        }
    }
}

/// Measure → sum, over the compartment's limited measures.
pub type Load = BTreeMap<CanonicalId, u64>;

/// **A malformed question** — never a refusal (doc 18 §4.3).
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum CapacityError {
    /// A good the goods table does not know.
    #[error("`{good}` is not a good the goods table knows")]
    UnknownGood {
        /// The good.
        good: CanonicalId,
    },
    /// A good whose stowage tag the compartment does not accept.
    #[error("`{good}` does not stow in `{compartment}`")]
    NotAccepted {
        /// The good.
        good: CanonicalId,
        /// The compartment.
        compartment: CanonicalId,
    },
    /// A good that declares no figure for a measure the compartment limits.
    #[error("`{good}` declares no `{measure}`, which the compartment limits")]
    MissingMeasure {
        /// The good.
        good: CanonicalId,
        /// The measure.
        measure: CanonicalId,
    },
    /// A load past `u64`.
    #[error("the load overflows")]
    Overflow,
}

/// **Why an addition does not fit**, for the caller to render (cut IC8).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapacityRefusal {
    /// The compartment.
    pub compartment: CanonicalId,
    /// Every limited measure the load after the addition exceeds, in the
    /// limit map's order (mass before volume).
    pub bound: Vec<CanonicalId>,
    /// How many units of the good would fit ([`max_more`]).
    pub fits: u64,
    /// Limit − load per limited measure, before the addition (saturating at 0).
    pub room: Load,
}

/// **The one answer** [`admits`] gives: one enum, so no `?` (doc 18 §4.3).
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Admission {
    /// It fits; the load after the addition.
    Admitted(Load),
    /// It does not fit, and why.
    Refused(CapacityRefusal),
    /// The question was malformed.
    Error(CapacityError),
}

/// The good's per-unit figures for every measure the compartment limits,
/// refusing an unknown, unaccepted or under-declared good.
fn figures(
    goods: &GoodsTable,
    compartment: &Compartment,
    good: &CanonicalId,
) -> Result<Vec<(CanonicalId, u64)>, CapacityError> {
    let Some(def) = goods.good(good) else {
        return Err(CapacityError::UnknownGood { good: good.clone() });
    };
    if !compartment.accepts.contains(&def.stowage) {
        return Err(CapacityError::NotAccepted {
            good: good.clone(),
            compartment: compartment.id.clone(),
        });
    }
    let mut out = Vec::with_capacity(compartment.limits.len());
    for measure in compartment.limits.keys() {
        let Some(per_unit) = def.per_unit(measure) else {
            return Err(CapacityError::MissingMeasure {
                good: good.clone(),
                measure: measure.clone(),
            });
        };
        out.push((measure.clone(), per_unit));
    }
    Ok(out)
}

/// Add `qty` units at `figures` into `load`, checked.
fn add_into(
    load: &mut Load,
    figures: &[(CanonicalId, u64)],
    qty: u64,
) -> Result<(), CapacityError> {
    for (measure, per_unit) in figures {
        let add = qty.checked_mul(*per_unit).ok_or(CapacityError::Overflow)?;
        let slot = load.entry(measure.clone()).or_insert(0);
        *slot = slot.checked_add(add).ok_or(CapacityError::Overflow)?;
    }
    Ok(())
}

/// **The compartment's load**: Σ quantity × per-unit for each limited measure
/// (an unlimited measure is ignored), every limited measure present.
///
/// # Errors
///
/// [`CapacityError`] for a good the table does not know, the compartment does
/// not accept or that declares no figure for a limited measure, or overflow.
pub fn load(
    goods: &GoodsTable,
    compartment: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
) -> Result<Load, CapacityError> {
    let mut out: Load = compartment
        .limits
        .keys()
        .map(|measure| (measure.clone(), 0))
        .collect();
    for (good, qty) in contents {
        let figures = figures(goods, compartment, good)?;
        add_into(&mut out, &figures, *qty)?;
    }
    Ok(out)
}

/// Limit − load per limited measure, saturating at zero.
fn room_of(compartment: &Compartment, load: &Load) -> Load {
    compartment
        .limits
        .iter()
        .map(|(measure, limit)| {
            let used = load.get(measure).copied().unwrap_or(0);
            (measure.clone(), limit.saturating_sub(used))
        })
        .collect()
}

/// The minimum over limited measures of `room / per_unit`, and every measure
/// that attains it. A measure `room` does not name has no room (`0` units): every
/// caller here builds `figures` from the compartment's own limits. Doc 20 cut SM.7:
/// mining's fit (`crate::surface::mine::fit`) reads the same rule rather than
/// restating it, passing only the measures its room names (the SM.7 lead's
/// pre-commit round, item 6).
pub(crate) fn fits_in(room: &Load, figures: &[(CanonicalId, u64)]) -> (u64, Vec<CanonicalId>) {
    let mut best: Option<u64> = None;
    let mut bound: Vec<CanonicalId> = Vec::new();
    for (measure, per_unit) in figures {
        let left = room.get(measure).copied().unwrap_or(0);
        let units = left.checked_div(*per_unit).unwrap_or(0);
        match best {
            Some(current) if units > current => {}
            Some(current) if units == current => bound.push(measure.clone()),
            _ => {
                best = Some(units);
                bound = vec![measure.clone()];
            }
        }
    }
    // A compartment that limits nothing bounds nothing.
    (best.unwrap_or(u64::MAX), bound)
}

/// **Whether `add` fits** on top of `contents` (doc 18 §4.3).
///
/// Quantity 0 is admitted with the current load. Otherwise the load after the
/// addition is computed; every limited measure it exceeds is `bound`, in the
/// limit map's order; none → [`Admission::Admitted`] with the after-load, else
/// [`Admission::Refused`] with how many units fit and the room before.
#[must_use]
pub fn admits(
    goods: &GoodsTable,
    compartment: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
    add: (&CanonicalId, u64),
) -> Admission {
    let (good, qty) = add;
    let current = match load(goods, compartment, contents) {
        Ok(current) => current,
        Err(error) => return Admission::Error(error),
    };
    let figures = match figures(goods, compartment, good) {
        Ok(figures) => figures,
        Err(error) => return Admission::Error(error),
    };
    if qty == 0 {
        return Admission::Admitted(current);
    }
    let mut after = current.clone();
    if let Err(error) = add_into(&mut after, &figures, qty) {
        return Admission::Error(error);
    }
    let bound: Vec<CanonicalId> = compartment
        .limits
        .iter()
        .filter(|(measure, limit)| after.get(*measure).copied().unwrap_or(0) > **limit)
        .map(|(measure, _)| measure.clone())
        .collect();
    if bound.is_empty() {
        return Admission::Admitted(after);
    }
    let room = room_of(compartment, &current);
    let (fits, _) = fits_in(&room, &figures);
    Admission::Refused(CapacityRefusal {
        compartment: compartment.id.clone(),
        bound,
        fits,
        room,
    })
}

/// **How many more units of `good` fit**, and every limited measure that
/// binds at that count: the minimum over the compartment's limits of
/// `(limit − load) / per_unit`, naming **every** measure that attains it.
///
/// # Errors
///
/// As [`load`], for the contents and for `good`.
pub fn max_more(
    goods: &GoodsTable,
    compartment: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
    good: &CanonicalId,
) -> Result<(u64, Vec<CanonicalId>), CapacityError> {
    let current = load(goods, compartment, contents)?;
    let figures = figures(goods, compartment, good)?;
    Ok(fits_in(&room_of(compartment, &current), &figures))
}

/// **Litres as cubic metres to a tenth, truncated** (doc 18 §4.3 UI rule):
/// `120_400` → `"120.4"`, `99` → `"0.0"`. Never rounded up, so a hold reading
/// `120.0` is never over its 120 m³.
#[must_use]
pub fn format_m3(litres: u64) -> String {
    let tenths = litres / 100;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// **Kilograms as whole tonnes, truncated**: `100_999` → `"100"`.
#[must_use]
pub fn format_tonnes(kg: u64) -> String {
    (kg / 1_000).to_string()
}

/// **The one compartment a good stows in** (doc 18 §4.3 "The capacity function"; cut
/// IC8): the compartment whose `accepts` holds the good's `stowage` tag. `None` for a
/// good the table does not know, or when no compartment — or more than one — accepts
/// it (an ambiguous hull is refused, never guessed at).
#[must_use]
pub fn compartment_for(
    goods: &GoodsTable,
    compartments: &[Compartment],
    good: &CanonicalId,
) -> Option<Compartment> {
    let def = goods.good(good)?;
    let mut matching = compartments
        .iter()
        .filter(|compartment| compartment.accepts.contains(&def.stowage));
    let only = matching.next()?;
    matching.next().is_none().then(|| only.clone())
}
