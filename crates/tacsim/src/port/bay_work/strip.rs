//! **Strip and scrap** (doc 36 cut GC.8b; Q-36-37, Q-36-39) through `mush_tactics`' workshop
//! engine, the matter laid into the carrier's hold as lots by [`super::act`]'s mint.
//!
//! - **`+bay/strip <hull>=<part>`**: a surviving part (intact, damaged or jammed) comes off a
//!   hull berthed in a workshop (`WorkshopBook::strip_component`); its slot reads `Missing`
//!   (GC.2) and the part becomes a one-unit lot of `tacsim.part.<id>` — the refit bay's own
//!   catalogue — carrying its provenance: the parent hull, the slot, its condition and the
//!   claim it came off under (Q-F6). Title and a `maintain_refit` grant may strip.
//! - **`+bay/scrap <hull>`**: the title holder breaks a berthed hull up
//!   (`WorkshopBook::scrap_hull`): its structural mass becomes `structural_alloys`, a tonne per
//!   1,000 kg, and each salvageable part a lot with its provenance. The hull is left an
//!   irrecoverable shell in the workshop, its berth freed.
//! - **When the hull changes** (doc 36a §5.1): a strip or a scrap is checked against the hull
//!   when it is ordered, but written to it only when its mint has landed ([`land_mint`], from
//!   the port tick's settle), so a mint given up destroys nothing.
//! - **Hot parts** (Q-F6): [`vacate_claim`] flags a stripped part hot when the claim it came off
//!   under is vacated — the seam GC.5/GC.6's contest calls, a staff fixture until then. The bay
//!   refuses to fit a hot part (`+bay/fit`).
//! - **Remains** (Q-36-39): scrapped where they lie by their title holder or a winning claimant —
//!   built by doc 36 cut GC.6 as `+tv/<n> scrap` on the site ([`crate::surface::claim`]), which
//!   mints the alloys at [`SCRAP_KG_PER_T`] in the same bundle that takes the remains off the
//!   site's record.

use mush_core::{CanonicalId, Ctx, ObjId};
use mush_inventory::LotId;
use mush_tactics::field_ops::{
    ComponentCondition, CustodyTitleStatus, ProvenanceLegalityStatus, StripAuthorization,
    WorkshopBook, assess_tactical_unit,
};
use smol_str::SmolStr;

use super::act::{self, LotMint, OwedAct, OwedKind, PartProvenance};
use super::{Act, BayHull, FacilityClass, NO_RIGHT, SETTLING, permits, record_of, right_of};
use crate::store::put_bay_hull;

/// The bulk good scrap's structural mass becomes (Q-36-39).
pub const SCRAP_GOOD: &str = "structural_alloys";
/// Kilograms of structural scrap to a tonne of `structural_alloys`. Provisional.
pub const SCRAP_KG_PER_T: u64 = 1_000;
/// The grade scrapped alloy goes to the hold at (the lane's stock grade). Provisional.
pub const SCRAP_GRADE: u8 = 50;

/// The record, a berthed workshop hull with no open order, no owed act and no scrapping, and
/// the actor standing in its workshop with `act` in their right.
fn ready(ctx: &Ctx<'_>, hull: ObjId, act: Act) -> Result<(BayHull, super::Facility), String> {
    let record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    if record.open().is_some() {
        return Err(super::HELD_OPEN.to_owned());
    }
    if record.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    if record.scrapped_ic.is_some() {
        return Err("That hull has been scrapped.".to_owned());
    }
    let facility = super::facility_of(ctx.w, hull)?;
    if facility.class != FacilityClass::Workshop {
        return Err(
            "Parts come off and hulls are broken up in a workshop; berth her first.".into(),
        );
    }
    if ctx.w.get(ctx.actor).and_then(|o| o.location) != Some(facility.room) {
        return Err("That is done in the workshop the hull stands in.".to_owned());
    }
    let right = right_of(ctx, hull).ok_or(NO_RIGHT)?;
    if !permits(right, act) {
        return Err(match act {
            Act::Scrap => "Only the hull's title holder may scrap it.".to_owned(),
            _ => "Your right over that hull does not reach taking parts off it.".to_owned(),
        });
    }
    Ok((record, facility))
}

/// Owe `lots` minted into `vault` for `why`, as the actor, and settle it at once.
fn owe_mint(
    ctx: &mut Ctx<'_>,
    mut record: BayHull,
    act: u32,
    vault: u64,
    why: &str,
    lots: Vec<LotMint>,
) {
    record.acts = act;
    record.owed = Some(Box::new(OwedAct {
        act,
        by: ctx.actor,
        vault: Some(vault),
        kind: OwedKind::Mint {
            why: why.to_owned(),
            lots,
        },
        tries: 0,
    }));
    let hull = record.hull;
    put_bay_hull(ctx, &record);
    super::settle(ctx, hull);
}

/// A part's provenance as it comes off `hull`.
#[allow(clippy::too_many_arguments)]
fn provenance(
    lot: LotId,
    unit: &mush_tactics::TacticalUnit,
    slot: &mush_tactics::unit::ComponentId,
    definition: mush_core::DefinitionRef,
    condition: ComponentCondition,
    by: ObjId,
    at_ic: i64,
    scrapped: bool,
) -> PartProvenance {
    PartProvenance {
        lot,
        parent_hull: unit.object(),
        parent_schema: unit.schema().clone(),
        slot: slot.clone(),
        definition,
        condition,
        // A title holder or a grant holder strips here; a salvor's claim reference rides in
        // once GC.5/GC.6 record their custody (`super::salvor_custody`).
        claim: None,
        legality: ProvenanceLegalityStatus::CleanTitle,
        stripped_by: by,
        stripped_at_ic: at_ic,
        scrapped,
        fitted_into: None,
    }
}

/// **`+bay/strip`** (Q-36-37): take `part` off a berthed hull. Returns the part's lot.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
pub fn strip(ctx: &mut Ctx<'_>, hull: ObjId, part: &str) -> Result<LotId, String> {
    let (record, facility) = ready(ctx, hull, Act::Strip)?;
    let vault = super::source_hold(ctx.w, hull, &facility)?;
    let (before, _, anatomy) = super::unit_parts(ctx.w, hull)?;
    let part = part.trim();
    let slot = before
        .components()
        .map(|(id, _)| id.clone())
        .find(|id| id.as_str() == part || id.as_str().ends_with(&format!(".{part}")))
        .ok_or("That hull has no such part.")?;
    let definition = before
        .component(&slot)
        .map(|c| c.definition.clone())
        .ok_or("That hull has no such part.")?;
    let commodity = crate::hardware::bay::part_commodity(&definition)
        .ok_or("That part is not one the bay stocks; it does not come off.")?;
    let at_ic = ctx.w.world_ic_now().at;
    let (after, _, stripped) = WorkshopBook::new()
        .strip_component(
            &before,
            &anatomy,
            &slot,
            ctx.actor,
            &StripAuthorization::BayOrWorkshop {
                custody: CustodyTitleStatus::ResolvedTitle { owner: ctx.actor },
                has_explicit_owner_permission: true,
            },
            None,
            at_ic,
        )
        .map_err(|e| e.to_string())?;
    // The change is checked here and written when the mint lands (`land_mint`).
    mush_tactics::field_ops::ic_condition_plan(
        &before,
        &after,
        None,
        Some(SmolStr::new_static("tacsim.bay.strip")),
    )
    .map_err(|e| e.to_string())?
    .ok_or("Nothing came off.")?;
    let act_no = super::next_act(ctx, &record);
    let lot = act::lot_id(hull, act_no, 0)?;
    let condition = stripped.condition;
    let row = LotMint {
        lot,
        commodity,
        quantity: 1,
        grade: crate::hardware::bay::part_grade(condition == ComponentCondition::Intact),
        part: Some(provenance(
            lot, &before, &slot, definition, condition, ctx.actor, at_ic, false,
        )),
    };
    owe_mint(ctx, record, act_no, vault.0, "strip", vec![row]);
    Ok(lot)
}

/// What a scrapping yields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scrapped {
    /// Tonnes of `structural_alloys`.
    pub alloys_t: u64,
    /// The parts' lots.
    pub parts: Vec<LotId>,
}

/// **`+bay/scrap`** (Q-36-39): the title holder breaks a berthed hull up.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
pub fn scrap(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<Scrapped, String> {
    let (mut record, facility) = ready(ctx, hull, Act::Scrap)?;
    let vault = super::source_hold(ctx.w, hull, &facility)?;
    let (before, _, anatomy) = super::unit_parts(ctx.w, hull)?;
    let at_ic = ctx.w.world_ic_now().at;
    let assessment = assess_tactical_unit(&before, &anatomy);
    let (after, report) = WorkshopBook::new()
        .scrap_hull(
            &before,
            &anatomy,
            ctx.actor,
            &CustodyTitleStatus::ResolvedTitle { owner: ctx.actor },
            false,
            at_ic,
        )
        .map_err(|e| e.to_string())?;
    let act_no = super::next_act(ctx, &record);
    let mut lots = Vec::new();
    let alloys_t = report.structural_scrap_kg / SCRAP_KG_PER_T;
    if alloys_t > 0 {
        lots.push(LotMint {
            lot: act::lot_id(hull, act_no, 0)?,
            commodity: CanonicalId::new(SCRAP_GOOD).map_err(|e| e.to_string())?,
            quantity: alloys_t,
            grade: SCRAP_GRADE,
            part: None,
        });
    }
    for (slot, condition) in &assessment.component_conditions {
        if !condition.is_salvageable() || lots.len() >= act::MAX_ACT_ROWS {
            continue;
        }
        let Some(definition) = before.component(slot).map(|c| c.definition.clone()) else {
            continue;
        };
        let Some(commodity) = crate::hardware::bay::part_commodity(&definition) else {
            continue;
        };
        let lot = act::lot_id(hull, act_no, lots.len().saturating_add(1))?;
        lots.push(LotMint {
            lot,
            commodity,
            quantity: 1,
            grade: crate::hardware::bay::part_grade(*condition == ComponentCondition::Intact),
            part: Some(provenance(
                lot, &before, slot, definition, *condition, ctx.actor, at_ic, true,
            )),
        });
    }
    let plan = mush_tactics::field_ops::ic_condition_plan(
        &before,
        &after,
        None,
        Some(SmolStr::new_static("tacsim.bay.scrap")),
    )
    .map_err(|e| e.to_string())?;
    let parts = lots
        .iter()
        .filter(|l| l.part.is_some())
        .map(|l| l.lot)
        .collect();
    if lots.is_empty() {
        // Nothing to lay in the hold: the hull is broken up now.
        if let Some(plan) = plan {
            mush_tactics::module::apply_plan(ctx, hull, &plan).map_err(|e| e.to_string())?;
        }
        record.scrapped_ic = Some(at_ic);
        record.berth = None;
        put_bay_hull(ctx, &record);
    } else {
        // The breaking-up is written when the mint lands (`land_mint`); the hull stands
        // berthed and whole until then.
        owe_mint(ctx, record, act_no, vault.0, "scrap", lots);
    }
    Ok(Scrapped { alloys_t, parts })
}

/// **A mint landed** (doc 36a §5.1): the change a strip or a scrap owes the hull is written
/// now that its lots stand in the hold — derived again from the hull as it stands (the strip
/// of the slot the lot's provenance names, or the breaking-up of the hull), so a mint given up
/// destroys nothing. A hull that will not take the change any more is paged for staff; its
/// lots stand. A scrap marks the record scrapped and frees its berth either way.
pub(crate) fn land_mint(ctx: &mut Ctx<'_>, hull: ObjId, by: ObjId, why: &str, lots: &[LotMint]) {
    let at_ic = ctx.w.world_ic_now().at;
    let scrap = why == "scrap";
    let custody = CustodyTitleStatus::ResolvedTitle { owner: by };
    let written = (|| -> Result<(), String> {
        let (before, _, anatomy) = super::unit_parts(ctx.w, hull)?;
        let (after, cause) = if scrap {
            let (after, _) = WorkshopBook::new()
                .scrap_hull(&before, &anatomy, by, &custody, false, at_ic)
                .map_err(|e| e.to_string())?;
            (after, SmolStr::new_static("tacsim.bay.scrap"))
        } else {
            let slot = lots
                .iter()
                .find_map(|l| l.part.as_ref())
                .map(|p| p.slot.clone())
                .ok_or("the strip names no slot")?;
            let (after, _, _) = WorkshopBook::new()
                .strip_component(
                    &before,
                    &anatomy,
                    &slot,
                    by,
                    &StripAuthorization::BayOrWorkshop {
                        custody: custody.clone(),
                        has_explicit_owner_permission: true,
                    },
                    None,
                    at_ic,
                )
                .map_err(|e| e.to_string())?;
            (after, SmolStr::new_static("tacsim.bay.strip"))
        };
        let plan = mush_tactics::field_ops::ic_condition_plan(&before, &after, None, Some(cause))
            .map_err(|e| e.to_string())?;
        if let Some(plan) = plan {
            mush_tactics::module::apply_plan(ctx, hull, &plan).map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Err(error) = written {
        tracing::error!(
            hull = hull.0, %error, op = why,
            "tacsim: a bay mint landed but the hull would not take its change; staff reconcile \
             the hull against the lots in the hold"
        );
        ctx.out.sys_to_actor(
            by,
            "tacsim",
            format!(
                "The {why} of #{} is laid into the hold, but the hull would not take the change; \
                 staff are paged.",
                hull.0
            ),
        );
    }
    if scrap && let Ok(mut record) = record_of(ctx.w, hull) {
        record.scrapped_ic = Some(at_ic);
        record.berth = None;
        put_bay_hull(ctx, &record);
    }
}

/// **A stripped part's claim is vacated** (Q-S13, Q-F6): the provenance of `lot`, a part that
/// came off `hull`, is flagged hot, `rightful_owner` the one who may clear it. The seam
/// GC.5/GC.6's salvage contest calls when a claim is vacated. A salvor's custody reaches no
/// strip in GC.8b's rights table, so no part comes off under a claim yet and GC.6's vacate has
/// nothing to flag: only a staff fixture calls it. `by` submits the flag (a character, never GOD); the act refuses a lot that did not
/// come off `hull`.
///
/// # Errors
///
/// A zero claim or owner, or the hull's record busy or unreadable.
pub fn vacate_claim(
    ctx: &mut Ctx<'_>,
    by: ObjId,
    hull: ObjId,
    lot: LotId,
    claim: u64,
    rightful_owner: ObjId,
) -> Result<(), String> {
    if claim == 0 || rightful_owner.0 == 0 || by.0 == 0 {
        return Err("A vacated claim names its claim, its rightful owner and its hand.".to_owned());
    }
    let mut record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    if record.owed.is_some() {
        return Err(SETTLING.to_owned());
    }
    let act_no = super::next_act(ctx, &record);
    record.acts = act_no;
    record.owed = Some(Box::new(OwedAct {
        act: act_no,
        by,
        vault: None,
        kind: OwedKind::Hot {
            lot,
            claim,
            rightful_owner,
        },
        tries: 0,
    }));
    put_bay_hull(ctx, &record);
    super::settle(ctx, hull);
    Ok(())
}
