//! **Strip and scrap** (doc 36 cut GC.8b; Q-36-37, Q-36-39) through `mush_tactics`' workshop
//! engine, the matter laid into the carrier's hold as lots by [`super::act`]'s mint.
//!
//! - **`+bay/strip <hull>=<part>`**: a surviving part (intact, damaged or jammed) comes off a
//!   hull berthed in a workshop (`WorkshopBook::strip_component`); its slot reads `Missing`
//!   (GC.2) and the part becomes a one-unit lot of `tacsim.part.<id>` — the refit bay's own
//!   catalogue — carrying its provenance: the parent hull, the slot, its condition and the
//!   claim it came off under (Q-F6). Title and a `maintain_refit` grant may strip.
//! - **`+bay/scrap <hull>`**: the title holder breaks a berthed hull up
//!   (`WorkshopBook::scrap_hull`) and the workshop's operator buys what comes off (doc 39 cut
//!   IN.8, §4.6, [`super::scrap`]): the structure's yield at the design's own figures into the
//!   operator's stock in whole tonnes of `structural_alloys`, each salvageable part a lot with
//!   its provenance into its stores, the fraction and the hull's bale into the shop bale; the
//!   holder is credited a fifth of the scrappable value for the structure (at most the alloys'
//!   worth), the parts' used value less their removal, and the bale at the references. The hull
//!   is left an irrecoverable shell in the workshop, its berth freed.
//! - **`+bay/return <hull>`** (doc 39 cut IN.8, §4.10): the holder sells the hull's opened bale
//!   back to the workshop at the references, kilogram for kilogram, into its shop bale, which
//!   mints a tonne into the operator's stock each time it passes 1,000 kg.
//! - **When the hull changes** (doc 36a §5.1): a strip or a scrap is checked against the hull
//!   when it is ordered, but written to it only when its mint has landed (`land_mint`, from
//!   the port tick's settle), so a mint given up destroys nothing. The command writes the owed
//!   act and nothing else; the tick submits it (doc 36a §5.4).
//! - **Room** (doc 36a P2-4): the bulk a strip or a scrap lays in the hold (`structural_alloys`;
//!   a part is a lot of its own kind and takes no governed room) must fit the hold as it stands
//!   by [`crate::port::market::PortCapacity`]'s rule, read before any plan is owed
//!   (`hold_takes`) and again in the mint's prepare from the plan the offer carries.
//! - **Hot parts** (Q-F6): [`vacate_claim`] flags a stripped part hot when the claim it came off
//!   under is vacated — the seam GC.5/GC.6's contest calls, a staff fixture until then. The bay
//!   refuses to fit a hot part (`+bay/fit`).
//! - **Remains** (Q-36-39): scrapped where they lie by their title holder or a winning claimant —
//!   built by doc 36 cut GC.6 as `+tv/<n> scrap` on the site ([`crate::surface::claim`]), which
//!   mints the capped yield (doc 39 cut IN.8, §4.6) at [`SCRAP_KG_PER_T`] in the same bundle
//!   that takes the remains off the site's record.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, Ctx, ObjId};
use mush_economy::VaultId;
use mush_inventory::LotId;
use mush_inventory::record::EndpointRef;
use mush_market::{CapacityProvider, CapacityReadError, CapacityRefusal};
use mush_tactics::field_ops::{
    ComponentCondition, CustodyTitleStatus, ProvenanceLegalityStatus, StripAuthorization,
    WorkshopBook, assess_tactical_unit,
};
use smol_str::SmolStr;

use super::act::{self, LotMint, OwedAct, OwedKind, PartProvenance};
use super::{Act, BayHull, FacilityClass, NO_RIGHT, SETTLING, permits, record_of, right_of};
use crate::port::market::PortCapacity;
use crate::store::put_bay_hull;

/// The bulk good scrap's structural mass becomes (Q-36-39).
pub const SCRAP_GOOD: &str = "structural_alloys";
/// Kilograms of structural scrap to a tonne of `structural_alloys`. Provisional.
pub const SCRAP_KG_PER_T: u64 = 1_000;
/// The grade scrapped alloy goes to the hold at (the lane's stock grade). Provisional.
pub const SCRAP_GRADE: u8 = 50;

/// The record, a berthed workshop hull with no open order, no owed act and no scrapping, and
/// the actor standing in its workshop with `act` in their right.
fn ready(ctx: &mut Ctx<'_>, hull: ObjId, act: Act) -> Result<(BayHull, super::Facility), String> {
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
    // Doc 36a P1-2: the lots go into the hold of the carrier she came off, which must still
    // serve her — never a salvor's hold after the claim is vacated.
    super::berth_stands(ctx, ctx.actor, &record)?;
    Ok((record, facility))
}

/// **Whether the hold takes what `lots` lay in it** (doc 36a P2-4; doc 39 cut IN.1): every lot,
/// a part by its catalogue mass, summed by commodity, against [`PortCapacity`]'s plan for the
/// hold as it stands (the parts already aboard included). An ungoverned hold takes anything; an
/// unreadable plan or pool refuses.
///
/// # Errors
///
/// The refusal, as a sentence carrying the hold's own line.
fn hold_takes(ctx: &Ctx<'_>, vault: VaultId, lots: &[LotMint]) -> Result<(), String> {
    let endpoint = EndpointRef(vault);
    let Some(plan) = PortCapacity.plan(ctx.w, endpoint) else {
        return Ok(());
    };
    let mut bulk: BTreeMap<CanonicalId, u64> = BTreeMap::new();
    for lot in lots {
        let slot = bulk.entry(lot.commodity.clone()).or_insert(0);
        *slot = slot.saturating_add(lot.quantity);
    }
    for (commodity, quantity) in &bulk {
        let mut read = |good: &CanonicalId| {
            crate::port_tick::read_pool(ctx, endpoint, good).map_err(|_| CapacityReadError)
        };
        match PortCapacity.check(&plan, commodity, *quantity, &mut read) {
            Ok(()) => {}
            Err(CapacityRefusal::Refused { line, .. }) => {
                return Err(format!(
                    "Her carrier's hold will not take what comes off her: {line}"
                ));
            }
            Err(CapacityRefusal::Unreadable) => {
                return Err("Her carrier's hold cannot be read; nothing comes off her.".to_owned());
            }
        }
    }
    Ok(())
}

/// Owe `lots` minted into `vault` for `why`, as the actor. The port tick submits it (doc 36a
/// §5.4: no command submits inside its handler) and writes the hull when it lands.
fn owe_mint(
    ctx: &mut Ctx<'_>,
    mut record: BayHull,
    act: u32,
    vault: u64,
    why: &str,
    (lots, settle): (Vec<LotMint>, Option<super::scrap::Settlement>),
) {
    record.acts = act;
    record.owed = Some(Box::new(OwedAct {
        act,
        by: ctx.actor,
        vault: Some(vault),
        kind: OwedKind::Mint {
            why: why.to_owned(),
            lots,
            settle: settle.map(Box::new),
        },
        tries: 0,
    }));
    put_bay_hull(ctx, &record);
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
        definition,
        condition,
        legality: ProvenanceLegalityStatus::CleanTitle,
        fitted_into: None,
        origin: act::PartOrigin::Stripped(act::Stripped {
            parent_hull: unit.object(),
            parent_schema: unit.schema().clone(),
            slot: slot.clone(),
            // A title holder or a grant holder strips here; a salvor's claim reference rides
            // in once GC.5/GC.6 record their custody (`super::salvor_custody`).
            claim: None,
            stripped_by: by,
            stripped_at_ic: at_ic,
            scrapped,
        }),
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
    let part_row = crate::hardware::bay::part_of(ctx.w, &definition)
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
        commodity: part_row.commodity,
        quantity: 1,
        grade: crate::hardware::bay::part_grade(condition == ComponentCondition::Intact),
        part: Some(provenance(
            lot,
            &before,
            &slot,
            part_row.row,
            condition,
            ctx.actor,
            at_ic,
            false,
        )),
    };
    hold_takes(ctx, vault, std::slice::from_ref(&row))?;
    let mut record = record;
    // Doc 41 §4.6: a lot that comes off under a pending claim is listed under it, so the claim's
    // vacate flags it. Dormant on the ruled rights table (a salvor never strips, Q-36-33): no
    // path strips under a claim yet.
    if let Some(contract) =
        super::salvor_custody(ctx, ctx.actor, hull).and_then(|row| match row.claim {
            crate::surface::salvage::ClaimSeam::Filed { contract } => Some(contract),
            crate::surface::salvage::ClaimSeam::Owed => None,
        })
        && record.claim_lots.len() < super::MAX_CLAIM_LOTS
    {
        record.claim_lots.push(super::ClaimLot {
            lot: lot.0,
            claim: contract,
            vacated: None,
        });
    }
    owe_mint(ctx, record, act_no, vault.0, "strip", (vec![row], None));
    Ok(lot)
}

/// What a scrapping yields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scrapped {
    /// Tonnes of `structural_alloys` into the operator's stock.
    pub alloys_t: u64,
    /// The yield, kilograms, before the tonnes are taken out.
    pub yield_kg: u64,
    /// The parts' lots, into its stores.
    pub parts: Vec<LotId>,
    /// The holder's credit, minor units.
    pub credit_minor: u64,
}

/// What a return comes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Returned {
    /// The bale bought back.
    pub bale: super::Bale,
    /// The holder's credit, minor units.
    pub credit_minor: u64,
    /// Tonnes the shop bale mints of `structural_alloys` and `industrial_parts`.
    pub minted: (u64, u64),
}

/// The operator's storefront, the org and account that pay, and its shop bale.
type Buyer = (
    VaultId,
    (mush_org::OrgId, mush_economy::AccountId),
    super::ShopBale,
);

/// **The workshop's side of a scrap or a return** (doc 39 cut IN.8): the operator's storefront
/// (its stores and stock), the org and account that pay, and its shop bale — read free of
/// another hull's pending mint.
fn buyer(w: &mush_core::World, facility: &super::Facility) -> Result<Buyer, String> {
    let (Some(port), Some(operator), Some(row)) = (
        facility.port.as_ref(),
        facility.operator,
        super::stores_of(w, facility),
    ) else {
        return Err("That workshop keeps no stores to buy what comes off her.".to_owned());
    };
    let shop = w
        .module_state::<crate::store::TacsimStore>()
        .ok_or("TacSim is not loaded.")?
        .shop_bale(port)
        .ok_or("The workshop's books cannot be read; staff repair them first.")?;
    if shop.pending.is_some() {
        return Err(
            "The workshop is still taking another hull's matter into its stores; try again in a \
             moment."
                .to_owned(),
        );
    }
    Ok((VaultId(row.vault), operator, shop))
}

/// The holder's credit account.
fn holder_account(w: &mush_core::World, holder: ObjId) -> Result<mush_economy::AccountId, String> {
    crate::port::credit::own_account(w, holder)
        .map(|account| account.id)
        .ok_or_else(|| "You hold no credit account for the yard to pay into.".to_owned())
}

/// The bulk lots a settlement mints into the operator's stock, rows `0` and `1` of the act.
fn bulk_lots(
    hull: ObjId,
    act_no: u32,
    terms: &super::scrap::Terms,
) -> Result<Vec<LotMint>, String> {
    let mut lots = Vec::new();
    for (row, good, t) in [
        (0, SCRAP_GOOD, terms.alloys_t),
        (1, super::PARTS_GOOD, terms.fittings_t),
    ] {
        if t > 0 {
            lots.push(LotMint {
                lot: act::lot_id(hull, act_no, row)?,
                commodity: CanonicalId::new(good).map_err(|e| e.to_string())?,
                quantity: t,
                grade: SCRAP_GRADE,
                part: None,
            });
        }
    }
    Ok(lots)
}

/// The references a scrap or a return reads (§4.6, §4.10), the hull's at `hull_minor`.
fn references(w: &mush_core::World, hull_minor: u64) -> super::scrap::References {
    super::scrap::References {
        hull_minor,
        alloys_minor_t: crate::definitions::mass::good_reference_minor(w, SCRAP_GOOD),
        fittings_minor_t: crate::definitions::mass::good_reference_minor(w, super::PARTS_GOOD),
    }
}

/// **Settle or owe** a scrap's or a return's matter and money: with nothing to mint it lands
/// now; otherwise the shop bale is held for this act and the mint is owed with its settlement.
#[allow(clippy::too_many_arguments)]
fn settle_or_owe(
    ctx: &mut Ctx<'_>,
    record: BayHull,
    (act_no, why): (u32, &str),
    (stores, shop): (VaultId, super::ShopBale),
    lots: Vec<LotMint>,
    settle: super::scrap::Settlement,
) {
    let hull = record.hull;
    if lots.is_empty() {
        super::scrap::land(ctx, hull, act_no, ctx.actor, &settle);
        return;
    }
    crate::store::put_shop_bale(
        ctx,
        &super::ShopBale {
            pending: Some((hull, act_no)),
            ..shop
        },
    );
    owe_mint(ctx, record, act_no, stores.0, why, (lots, Some(settle)));
}

/// **`+bay/scrap`** (Q-36-39; doc 39 cut IN.8, §4.6): the title holder breaks a berthed hull up
/// and the workshop's operator buys what comes off.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
#[allow(clippy::too_many_lines)]
pub fn scrap(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<Scrapped, String> {
    let (mut record, facility) = ready(ctx, hull, Act::Scrap)?;
    let (stores, (operator, operator_account), shop) = buyer(ctx.w, &facility)?;
    let holder = holder_account(ctx.w, ctx.actor)?;
    let (before, _, anatomy) = super::unit_parts(ctx.w, hull)?;
    let at_ic = ctx.w.world_ic_now().at;
    let assessment = assess_tactical_unit(&before, &anatomy);
    let (after, _) = WorkshopBook::new()
        .scrap_hull(
            &before,
            &anatomy,
            ctx.actor,
            &CustodyTitleStatus::ResolvedTitle { owner: ctx.actor },
            false,
            at_ic,
        )
        .map_err(|e| e.to_string())?;
    // Doc 39 cut IN.8 (§4.6): the yield at the design's own figures, over its structural cells.
    let budget = crate::definitions::mass::hull_budget(ctx.w, hull);
    let yield_kg = budget.as_ref().map_or(0, |(_, budget)| {
        budget.scrap_yield_kg(super::scrap::surviving_points(&before))
    });
    let act_no = super::next_act(ctx, &record);
    let mut parts = Vec::new();
    let mut part_lots = Vec::new();
    for (slot, condition) in &assessment.component_conditions {
        if !condition.is_salvageable() || part_lots.len() >= act::MAX_ACT_ROWS - 2 {
            continue;
        }
        let Some(definition) = before.component(slot).map(|c| c.definition.clone()) else {
            continue;
        };
        let Some(part_row) = crate::hardware::bay::part_of(ctx.w, &definition) else {
            continue;
        };
        let intact = *condition == ComponentCondition::Intact;
        parts.push(super::scrap::PartIn {
            reference_minor: super::part_price(ctx.w, hull, slot).map_or(0, |(_, _, price)| price),
            intact,
        });
        // Rows 0 and 1 are the bulk lots'.
        let lot = act::lot_id(hull, act_no, part_lots.len().saturating_add(2))?;
        part_lots.push(LotMint {
            lot,
            commodity: part_row.commodity,
            quantity: 1,
            grade: crate::hardware::bay::part_grade(intact),
            part: Some(provenance(
                lot,
                &before,
                slot,
                part_row.row,
                *condition,
                ctx.actor,
                at_ic,
                true,
            )),
        });
    }
    let hull_minor = budget.as_ref().map_or(0, |(design, budget)| {
        crate::definitions::mass::reference_price_minor(ctx.w, design, budget)
    });
    // The removal is the tariff's replacement labour at the operator's multiplier (§4.6: the
    // strip's four IC hours).
    let removal = crate::definitions::workshop::workshop_tariff()
        .map_err(|e| e.to_string())?
        .with_multiplier_permille(facility.multiplier_permille)
        .map_err(|e| e.to_string())?
        .labor_cost_replace_component;
    let terms = super::scrap::terms(
        references(ctx.w, hull_minor),
        yield_kg,
        &parts,
        removal,
        record.bale,
        shop.kg,
    );
    if !super::scrap::operator_covers(ctx.w, operator_account, terms.credit_minor()) {
        return Err("The yard cannot pay for her just now; nothing is scrapped.".to_owned());
    }
    let mut lots = bulk_lots(hull, act_no, &terms)?;
    lots.extend(part_lots);
    let plan = mush_tactics::field_ops::ic_condition_plan(
        &before,
        &after,
        None,
        Some(SmolStr::new_static("tacsim.bay.scrap")),
    )
    .map_err(|e| e.to_string())?;
    hold_takes(ctx, stores, &lots)?;
    // Doc 41 cut RS.9a: what the scrap mints is a source in the matter census, counted when its
    // lots land (`scrap::land`).
    let goods = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| store.goods());
    let part_kg = lots
        .iter()
        .filter(|l| l.part.is_some())
        .map(|l| {
            goods
                .and_then(|goods| goods.good(&l.commodity))
                .and_then(|def| def.mass_kg_per_unit())
                .map_or(0, |per| per.saturating_mul(l.quantity))
        })
        .fold(0_u64, u64::saturating_add);
    let settle = super::scrap::Settlement {
        credit_minor: terms.credit_minor(),
        operator: operator.0,
        operator_account: operator_account.0,
        holder_account: holder.0,
        shop: super::ShopBale {
            kg: terms.shop_after,
            pending: None,
            ..shop.clone()
        },
        clear_bale: record.bale.is_some(),
        minted_kg: yield_kg.saturating_add(part_kg),
    };
    let scrapped = Scrapped {
        alloys_t: terms.alloys_t,
        yield_kg,
        parts: lots
            .iter()
            .filter(|l| l.part.is_some())
            .map(|l| l.lot)
            .collect(),
        credit_minor: terms.credit_minor(),
    };
    if lots.is_empty() {
        // Nothing to lay in the stores: the hull is broken up now.
        if let Some(plan) = plan {
            mush_tactics::module::apply_plan(ctx, hull, &plan).map_err(|e| e.to_string())?;
        }
        record.scrapped_ic = Some(at_ic);
        record.berth = None;
        put_bay_hull(ctx, &record);
    }
    // Otherwise the breaking-up and the credit land with the mint (`land_mint`); the hull
    // stands berthed and whole, and the shop bale held, until then.
    settle_or_owe(ctx, record, (act_no, "scrap"), (stores, shop), lots, settle);
    Ok(scrapped)
}

/// **`+bay/return`** (doc 39 cut IN.8, §4.10): the title holder sells the hull's opened bale back
/// to the workshop at the references, raw matter with no fifth applied.
///
/// # Errors
///
/// The refusal, as a sentence; nothing is written on any.
pub fn return_bale(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<Returned, String> {
    let (record, facility) = ready(ctx, hull, Act::Scrap)?;
    let Some(bale) = record.bale.filter(|bale| *bale != super::Bale::default()) else {
        return Err("Her bale holds nothing to return.".to_owned());
    };
    let (stores, (operator, operator_account), shop) = buyer(ctx.w, &facility)?;
    let holder = holder_account(ctx.w, ctx.actor)?;
    let terms = super::scrap::terms(references(ctx.w, 0), 0, &[], 0, Some(bale), shop.kg);
    if !super::scrap::operator_covers(ctx.w, operator_account, terms.credit_minor()) {
        return Err("The yard cannot pay for her bale just now.".to_owned());
    }
    let act_no = super::next_act(ctx, &record);
    let lots = bulk_lots(hull, act_no, &terms)?;
    hold_takes(ctx, stores, &lots)?;
    let settle = super::scrap::Settlement {
        credit_minor: terms.credit_minor(),
        operator: operator.0,
        operator_account: operator_account.0,
        holder_account: holder.0,
        shop: super::ShopBale {
            kg: terms.shop_after,
            pending: None,
            ..shop.clone()
        },
        clear_bale: true,
        minted_kg: 0,
    };
    settle_or_owe(
        ctx,
        record,
        (act_no, "return"),
        (stores, shop),
        lots,
        settle,
    );
    Ok(Returned {
        bale,
        credit_minor: terms.credit_minor(),
        minted: (terms.alloys_t, terms.fittings_t),
    })
}

/// **A mint landed** (doc 36a §5.1): the change a strip or a scrap owes the hull is written
/// now that its lots stand in the hold — derived again from the hull as it stands (the strip
/// of the slot the lot's provenance names, or the breaking-up of the hull), so a mint given up
/// destroys nothing. A hull that will not take the change any more is paged for staff; its
/// lots stand. A scrap marks the record scrapped and frees its berth either way.
pub(crate) fn land_mint(ctx: &mut Ctx<'_>, hull: ObjId, by: ObjId, why: &str, lots: &[LotMint]) {
    // Doc 39 cut IN.8: a return changes nothing on the hull but its bale (its settlement's).
    if why == "return" {
        return;
    }
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
                .and_then(|p| p.stripped().map(|s| s.slot.clone()))
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
/// came off `hull`, is flagged hot, `rightful_owner` the one who may clear it. The claims' tick
/// calls it (`flag_next_vacated`, doc 41 cut RS.3) for each lot the hull's record lists under
/// a vacated claim, and the lot leaves the list in the same write; the mush-tactics RAM-book
/// twin is retired. A salvor's custody reaches no strip in GC.8b's rights table, so no part
/// comes off under a claim yet and the list stays empty in play. `by` submits the flag (a
/// character, never GOD); the act refuses a lot that did not come off `hull`.
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
    record.claim_lots.retain(|listed| listed.lot != lot.0);
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
    Ok(())
}

/// **List a stripped lot under a pending claim** (doc 39 cut IN.7; doc 41 §4.6): staff's seam
/// for the strip under a claim, which no ruled path makes yet (a salvor never strips, Q-36-33),
/// [`vacate_claim`]'s pair. `lot` came off `hull`; until `claim` is vacated or its contract
/// ends, the part is disputed ([`super::disputed`]).
///
/// # Errors
///
/// A zero claim, the record unreadable, or its claim list full.
pub fn list_under_claim(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    lot: LotId,
    claim: u64,
) -> Result<(), String> {
    if claim == 0 {
        return Err("A claim names its contract.".to_owned());
    }
    let mut record = record_of(ctx.w, hull).map_err(str::to_owned)?;
    if record.claim_lots.len() >= super::MAX_CLAIM_LOTS {
        return Err("That hull's claim list is full.".to_owned());
    }
    record.claim_lots.retain(|listed| listed.lot != lot.0);
    record.claim_lots.push(super::ClaimLot {
        lot: lot.0,
        claim,
        vacated: None,
    });
    put_bay_hull(ctx, &record);
    Ok(())
}

/// **A claim on `hull` was vacated** (doc 41 §4.6): every lot its record lists under `claim` is
/// marked for flagging, `rightful_owner` the title holder and `by` the hand that submits each
/// flag. Answers whether any lot is now waiting.
pub(crate) fn claim_vacated(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    claim: u64,
    (rightful_owner, by): (ObjId, ObjId),
) -> bool {
    let Ok(mut record) = record_of(ctx.w, hull) else {
        return false;
    };
    let mut marked = false;
    for listed in record
        .claim_lots
        .iter_mut()
        .filter(|listed| listed.claim == claim && listed.vacated.is_none())
    {
        listed.vacated = Some(super::VacatedLot { rightful_owner, by });
        marked = true;
    }
    if marked {
        put_bay_hull(ctx, &record);
    }
    marked
        || record
            .claim_lots
            .iter()
            .any(|listed| listed.vacated.is_some())
}

/// **Flag the next lot `hull`'s record lists under a vacated claim** (doc 41 §4.6): one
/// [`vacate_claim`] while no act is owed on the hull, so the tick works the list down one act at
/// a time. Answers whether one was flagged.
pub(crate) fn flag_next_vacated(ctx: &mut Ctx<'_>, hull: ObjId) -> bool {
    let Ok(record) = record_of(ctx.w, hull) else {
        return false;
    };
    if record.owed.is_some() {
        return false;
    }
    let Some((listed, vacated)) = record
        .claim_lots
        .iter()
        .find_map(|listed| listed.vacated.map(|vacated| (*listed, vacated)))
    else {
        return false;
    };
    match vacate_claim(
        ctx,
        vacated.by,
        hull,
        LotId(listed.lot),
        listed.claim,
        vacated.rightful_owner,
    ) {
        Ok(()) => true,
        Err(error) => {
            tracing::warn!(hull = hull.0, lot = listed.lot, %error, "tacsim: a vacated claim's lot was not flagged; retried next tick");
            false
        }
    }
}
