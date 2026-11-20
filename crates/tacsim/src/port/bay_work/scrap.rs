//! **The scrap ceiling and the bale's return** (doc 39 cut IN.8; §4.6, §4.10, Q-39-16,
//! Q-39-17, Q-39-25, Q-39-26), **sold on the board** (doc 48 cut EX.3; Q-48-9, doc 48a A-3).
//!
//! The workshop's operator buys what comes off a hull, and the title holder is paid:
//!
//! ```text
//! parts_new         = Σ reference of each salvageable part
//! scrappable        = reference_price − parts_new
//! structural_credit = min(yield_kg × alloy_reference / 1,000, scrappable / 5)
//! part_credit       = Σ (reference × used − removal)        (used 0.75 intact, 0.40 worn)
//! bale_credit       = the hull bale's kilograms at their references (raw matter: no fifth)
//! ```
//!
//! The yield, the bale and the workshop's shop bale ([`super::ShopBale`]) sum per good. **Scrap is
//! paid by the whole tonne**: the whole tonnes and the parts' lots mint into the holder's own
//! scrap vault at the workshop ([`scrap_vault`]: one a holder a venue, uncapped), and what is
//! under a tonne stays in the shop bale, the operator's, as before. `+bay/return` is the bale's
//! half alone. When the mint lands the holder sells it to the operator on the board
//! ([`sale_rows`], `sell_step`): the port's buy-back desk (`desk`) posts a private bid a row,
//! funded by the operator, and the holder sells into it from the scrap vault, as themself,
//! offline or an NPC alike. The market escrows the bid before the matter moves; a leg that
//! cannot be staged refunds the escrow to the desk's purse, and the matter stays in the holder's
//! vault. Nothing is paid off the board: a scrap or a return with nothing whole pays nothing
//! ([`NOTHING_WHOLE`]).

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{
    AccountId, Amount, EconomicCause, EconomyPlan, EconomyStore, IdempotencyKey, PostOutcome,
    RequestedLine,
};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::{Bale, KG_PER_T, ShopBale};
use crate::store::put_bay_hull;

/// **A part's used value, per mille of its reference** (§4.6): intact.
pub const USED_INTACT_PERMILLE: u64 = 750;
/// A worn part's (grade 3: damaged or jammed).
pub const USED_WORN_PERMILLE: u64 = 400;
/// **The fifth** (Q-38-5): the structure is worth at most the scrappable value over this.
pub const CEILING_DIVISOR: u64 = 5;

/// The idempotency namespace a scrap's or a return's credit posts under.
pub const CREDIT_NAMESPACE: &str = "tacsim.bay_scrap";

/// One salvageable part a scrap takes: its reference and whether it is intact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PartIn {
    /// Its catalogue reference, minor units.
    pub reference_minor: u64,
    /// Intact, or worn.
    pub intact: bool,
}

/// **What a scrap or a return comes to** (§4.6, §4.10), minor units and kilograms.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Terms {
    /// The structure's credit: the yield at the alloys' reference, at most the fifth.
    pub structural_minor: u64,
    /// The parts' used value less their removal.
    pub parts_minor: u64,
    /// The hull bale's kilograms bought back at the references.
    pub bale_minor: u64,
    /// Whole tonnes of `structural_alloys` into the operator's pool.
    pub alloys_t: u64,
    /// Whole tonnes of `industrial_parts`.
    pub fittings_t: u64,
    /// The shop bale after: what stays under a tonne.
    pub shop_after: Bale,
}

impl Terms {
    /// The holder's whole credit.
    #[must_use]
    pub const fn credit_minor(&self) -> u64 {
        self.structural_minor
            .saturating_add(self.parts_minor)
            .saturating_add(self.bale_minor)
    }
}

/// **The references a scrap reads**, minor units: the hull's, and a tonne of each bulk good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct References {
    /// The hull's reference price (§4.6: its bill, or a programme hull's buyout).
    pub hull_minor: u64,
    /// A tonne of `structural_alloys`.
    pub alloys_minor_t: u64,
    /// A tonne of `industrial_parts`.
    pub fittings_minor_t: u64,
}

/// **The terms** of a scrap yielding `yield_kg` of alloys with `parts` salvageable, each removed
/// at `removal_minor`, the hull's `bale` bought back, into a workshop whose shop bale holds
/// `shop` (a return is a scrap with no yield, no parts and no hull reference).
#[must_use]
pub fn terms(
    references: References,
    yield_kg: u64,
    parts: &[PartIn],
    removal_minor: u64,
    bale: Option<Bale>,
    shop: Bale,
) -> Terms {
    let parts_new = parts
        .iter()
        .fold(0u64, |sum, part| sum.saturating_add(part.reference_minor));
    let scrappable = references.hull_minor.saturating_sub(parts_new);
    let structural_minor =
        at_reference(yield_kg, references.alloys_minor_t).min(scrappable / CEILING_DIVISOR);
    let parts_minor = parts.iter().fold(0u64, |sum, part| {
        let used = if part.intact {
            USED_INTACT_PERMILLE
        } else {
            USED_WORN_PERMILLE
        };
        sum.saturating_add(
            (part.reference_minor.saturating_mul(used) / 1_000).saturating_sub(removal_minor),
        )
    });
    let bale = bale.unwrap_or_default();
    let bale_minor = at_reference(bale.alloys_kg, references.alloys_minor_t)
        .saturating_add(at_reference(bale.fittings_kg, references.fittings_minor_t));
    let alloys = shop
        .alloys_kg
        .saturating_add(yield_kg)
        .saturating_add(bale.alloys_kg);
    let fittings = shop.fittings_kg.saturating_add(bale.fittings_kg);
    Terms {
        structural_minor,
        parts_minor,
        bale_minor,
        alloys_t: alloys / KG_PER_T,
        fittings_t: fittings / KG_PER_T,
        shop_after: Bale {
            alloys_kg: alloys % KG_PER_T,
            fittings_kg: fittings % KG_PER_T,
        },
    }
}

/// `kg` at `minor_t` a tonne, rounded down.
fn at_reference(kg: u64, minor_t: u64) -> u64 {
    u64::try_from(u128::from(kg) * u128::from(minor_t) / u128::from(KG_PER_T)).unwrap_or(u64::MAX)
}

/// **One row of a scrap's sale** (doc 48 cut EX.3; Q-48-9): one good the holder sells the
/// operator, its units and its price a unit, minor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaleRow {
    /// The good.
    pub good: CanonicalId,
    /// How many: whole tonnes of a bulk good, one part lot.
    pub units: u64,
    /// The price a unit, minor units (at least 1).
    pub price_minor: u64,
}

impl SaleRow {
    /// The row's whole price.
    #[must_use]
    pub const fn total_minor(&self) -> u64 {
        self.units.saturating_mul(self.price_minor)
    }
}

/// The rows' whole price.
#[must_use]
pub fn rows_minor(rows: &[SaleRow]) -> u64 {
    rows.iter()
        .fold(0u64, |sum, row| sum.saturating_add(row.total_minor()))
}

/// **What a scrap or a return sells the operator** (doc 48 cut EX.3; Q-48-9, the lead's
/// rulings): scrap is paid by the whole tonne.
///
/// - **Each part** that comes off is a row of one lot at its used value less its removal. A part
///   worth nothing after its removal is no part sale: the scrap does not mint it as a part, and
///   its mass joins the bulk (`strip::scrap` adds it to the yield before [`terms`]).
/// - **Each bulk good** with whole tonnes is a row at its value over its tonnes, rounded up, at
///   least 1 minor a tonne: `structural_alloys` carries the structure's credit and the bale's
///   alloys, `industrial_parts` the bale's fittings. The shop bale's earlier remainder rides in
///   the whole tonnes at their share.
/// - **A good with no whole tonne pays nothing**: what is under a tonne stays in the operator's
///   shop bale, as before.
#[must_use]
pub fn sale_rows(
    terms: &Terms,
    references: References,
    bale: Option<Bale>,
    parts: &[(CanonicalId, u64)],
) -> Vec<SaleRow> {
    let bale = bale.unwrap_or_default();
    let alloys_minor = terms
        .structural_minor
        .saturating_add(at_reference(bale.alloys_kg, references.alloys_minor_t));
    let fittings_minor = at_reference(bale.fittings_kg, references.fittings_minor_t);
    let mut rows = Vec::new();
    for (good, units, value) in [
        (super::ALLOYS_GOOD, terms.alloys_t, alloys_minor),
        (super::PARTS_GOOD, terms.fittings_t, fittings_minor),
    ] {
        let Ok(good) = CanonicalId::new(good) else {
            continue;
        };
        if units > 0 {
            rows.push(SaleRow {
                good,
                units,
                price_minor: value.div_ceil(units).max(1),
            });
        }
    }
    rows.extend(
        parts
            .iter()
            .filter(|(_, value)| *value > 0)
            .map(|(good, value)| SaleRow {
                good: good.clone(),
                units: 1,
                price_minor: *value,
            }),
    );
    rows
}

/// **A part's sale value** (§4.6): its used value less its removal, 0 when the removal takes it
/// all.
#[must_use]
pub fn part_value(part: PartIn, removal_minor: u64) -> u64 {
    let used = if part.intact {
        USED_INTACT_PERMILLE
    } else {
        USED_WORN_PERMILLE
    };
    (part.reference_minor.saturating_mul(used) / 1_000).saturating_sub(removal_minor)
}

/// **The settlement an owed scrap or return carries** (doc 39 cut IN.8; doc 48 cut EX.3): what
/// the holder sells the operator, where its matter lies, the shop bale it leaves, and whether it
/// empties the hull's bale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settlement {
    /// What the sale comes to, minor units ([`rows_minor`] of `rows`).
    pub credit_minor: u64,
    /// The operator org, `OrgId.0`.
    pub operator: u64,
    /// The operator's scoped credit account, `AccountId.0`: it funds the desk's bids.
    pub operator_account: u64,
    /// The holder's scrap vault at the workshop, where the matter is minted.
    pub vault: u64,
    /// The operator's storefront, where the bids receive what they buy.
    pub stores: u64,
    /// The workshop's venue word.
    pub venue: SmolStr,
    /// What the holder sells, in order.
    pub rows: Vec<SaleRow>,
    /// The shop bale once the mint lands (its `pending` cleared).
    pub shop: ShopBale,
    /// Whether the hull's own bale is emptied (bought back).
    pub clear_bale: bool,
    /// **The kilograms the scrap mints from the hull** (doc 41's RS.9a follow-up): her
    /// structure's yield and her stripped parts' lots, which the census counts as a source only
    /// once they land (`land`). 0 for a bale return, and omitted.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub minted_kg: u64,
}

/// Serde's skip for a zero count.
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// **What a landed scrap or return still sells** (doc 48 cut EX.3), on the hull's record until
/// every row is sold or given up: one row a step, as the holder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sale {
    /// The act whose matter this is.
    pub act: u32,
    /// The holder, who sells.
    pub holder: ObjId,
    /// The holder's scrap vault, the sale's source.
    pub vault: u64,
    /// The operator's storefront, the bids' destination.
    pub stores: u64,
    /// The operator's scoped credit account.
    pub operator_account: u64,
    /// The workshop's venue word.
    pub venue: SmolStr,
    /// The rows still to sell, in order.
    pub rows: Vec<SaleRow>,
    /// Refusals of the first row so far.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub tries: u32,
}

/// Serde's skip for a zero count.
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

/// A row refused this many times is given up: its matter stays in the holder's scrap vault.
pub const MAX_SALE_TRIES: u32 = 8;

/// The holder hears that nothing whole came of a scrap or a return, so nothing is paid.
pub const NOTHING_WHOLE: &str =
    "Nothing whole came of it, so the yard pays nothing: scrap is paid by the whole tonne.";
/// The holder hears a row is given up: `{good}`, `{venue}`.
pub const SALE_GIVEN_UP: &str =
    "The yard could not buy your {good}; it stays in your scrap at {venue}.";

/// **Whether the operator can pay `credit` now**: its account's available balance covers it.
#[must_use]
pub fn operator_covers(w: &World, account: AccountId, credit: u64) -> bool {
    credit == 0
        || w.module_state::<EconomyStore>()
            .and_then(|store| store.snapshot().account(account)?.available())
            .is_some_and(|available| {
                u64::try_from(available.minor()).is_ok_and(|have| have >= credit)
            })
}

/// **Land a settlement** for act `act` on `hull`, by `by` (the holder): the shop bale is
/// written, the hull's bale emptied when it was bought back, and the sale put on the record, its
/// first row sold in this same step. With nothing whole to sell, the holder hears that the yard
/// pays nothing. No money moves off the board.
pub(crate) fn land(ctx: &mut Ctx<'_>, hull: ObjId, act: u32, by: ObjId, settle: &Settlement) {
    crate::store::put_shop_bale(ctx, &settle.shop);
    let Ok(mut record) = super::record_of(ctx.w, hull) else {
        return;
    };
    if settle.clear_bale {
        record.bale = None;
    }
    // Doc 41's RS.9a follow-up: the scrap's mint is a census source now that its lots and
    // its shop bale stand where the census reads them, never before.
    record.scrapped_kg = record.scrapped_kg.saturating_add(settle.minted_kg);
    if settle.rows.is_empty() {
        put_bay_hull(ctx, &record);
        ctx.out.sys_to_actor(by, "tacsim", NOTHING_WHOLE.to_owned());
        return;
    }
    record.sale = Some(Box::new(Sale {
        act,
        holder: by,
        vault: settle.vault,
        stores: settle.stores,
        operator_account: settle.operator_account,
        venue: settle.venue.clone(),
        rows: settle.rows.clone(),
        tries: 0,
    }));
    put_bay_hull(ctx, &record);
    sell_step(ctx, hull);
}

/// **One row of a hull's sale** (doc 48 cut EX.3; Q-48-9, doc 48a A-3): the port's buy-back
/// desk posts a private bid to the holder for the row, funded from its purse (the operator tops it up by
/// what it lacks), and the holder sells the row's matter from their scrap vault into it, as
/// themself. The market escrows the bid, holds the sale until the matter is delivered, and
/// settles; a leg that cannot be staged refunds the escrow to the purse and the matter stays in
/// the holder's vault. The bid is drawn to nothing by the sale and goes; a refused sale withdraws
/// it at once. Waits while either party has a submission queued or the engine's queue is full.
pub(crate) fn sell_step(ctx: &mut Ctx<'_>, hull: ObjId) {
    let Ok(mut record) = super::record_of(ctx.w, hull) else {
        return;
    };
    let Some(mut sale) = record.sale.clone() else {
        return;
    };
    let Some(row) = sale.rows.first().cloned() else {
        record.sale = None;
        put_bay_hull(ctx, &record);
        return;
    };
    let Some(market) = mush_market::MarketId::new(sale.venue.as_str()) else {
        return;
    };
    // The port's buy-back desk bids, laid up on the first sale at the port.
    let desk = match desk(ctx, &market) {
        Ok(desk) => desk,
        Err(error) => {
            tracing::warn!(hull = hull.0, %error, "tacsim: the port's buy-back desk is not laid up");
            return;
        }
    };
    if ctx.authoritative_queue_full()
        || ctx.authoritative_pending(sale.holder)
        || ctx.authoritative_pending(desk)
        || standing(ctx.w, sale.holder, desk) >= mush_market::MAX_OPEN_SELLS_PER_VENDOR
    {
        return;
    }
    // A bulk row's tonnes mint as a lot; the market sells bulk from the pool, so the holder
    // dissolves the lot into the scrap vault's pool first (no try spent), and sells it next step.
    if pour(ctx, &sale, &row) {
        return;
    }
    let sold = sell_row(ctx, hull, &sale, desk, &row);
    match sold {
        Ok(()) => {
            sale.rows.remove(0);
            sale.tries = 0;
        }
        Err(error) => {
            sale.tries = sale.tries.saturating_add(1);
            tracing::info!(hull = hull.0, good = %row.good, %error, tries = sale.tries,
                "tacsim: a scrap's row did not sell");
            if sale.tries >= MAX_SALE_TRIES {
                sale.rows.remove(0);
                sale.tries = 0;
                ctx.out.sys_to_actor(
                    sale.holder,
                    "tacsim",
                    SALE_GIVEN_UP
                        .replace("{good}", row.good.as_str())
                        .replace("{venue}", sale.venue.as_str()),
                );
            }
        }
    }
    record.sale = (!sale.rows.is_empty()).then_some(sale);
    put_bay_hull(ctx, &record);
}

/// **Pour a bulk row's lot into the pool** of the holder's scrap vault, as the holder, when the
/// pool holds less than the row sells and a lot of the good stands there. Whether it submitted
/// (the row waits a step).
fn pour(ctx: &mut Ctx<'_>, sale: &Sale, row: &SaleRow) -> bool {
    if crate::port_tick::is_part(&row.good) {
        return false;
    }
    let endpoint = mush_inventory::EndpointRef(mush_economy::VaultId(sale.vault));
    let pooled = crate::port_tick::read_pool(ctx, endpoint, &row.good).unwrap_or(0);
    if pooled >= row.units {
        return false;
    }
    let Some((lot, ..)) = crate::port::works::lots_in(ctx, endpoint)
        .into_iter()
        .find(|(_, good, _)| *good == row.good)
    else {
        return false;
    };
    let Some(dissolve) =
        mush_inventory::action_ref(ctx.reg, mush_inventory::InventoryVerb::Dissolve)
    else {
        return false;
    };
    let Ok(key) =
        mush_core::IdempotencyKeyText::parse_canonical(format!("{DISSOLVE_KEY_PREFIX}{}", lot.0))
    else {
        return false;
    };
    let saved = ctx.actor;
    ctx.actor = sale.holder;
    ctx.submit_authoritative(
        dissolve,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({
            "endpoint": sale.vault.to_string(),
            "lot": lot.to_string(),
        }),
        key,
    );
    ctx.actor = saved;
    true
}

/// A scrap's dissolve's idempotency-key prefix: `tacsim.bay_scrap.dissolve/<lot>`.
pub const DISSOLVE_KEY_PREFIX: &str = "tacsim.bay_scrap.dissolve/";

/// The holder's sales to the desk not yet settled: the market takes at most
/// [`mush_market::MAX_OPEN_SELLS_PER_VENDOR`] at once, so the next row waits for one to settle.
fn standing(w: &World, holder: ObjId, desk: ObjId) -> usize {
    w.module_state::<mush_market::MarketStore>()
        .map_or(0, |market| {
            market
                .quotes_of_seller(&PrincipalId::Object(holder))
                .filter(|quote| quote.buyer == PrincipalId::Object(desk) && quote.settled.is_none())
                .count()
        })
}

/// The desk's name at a port: `{port} buy-back desk`.
pub const DESK_NAME: &str = "{port} buy-back desk";

/// **The port's buy-back desk** (doc 48 cut EX.3, the lead's ruling): a `DARK` Thing of the
/// port's clerk, standing in the port's presentation room, that bids for the holders' scrap from
/// its own purse, with the clerk's `vault.withdraw` grant (lead ruling L25) on the workshop
/// operator's root scope so it may buy into the storefront. The bidder is the clerk's own, not
/// GOD's, because any two GOD-owned NPCs act for each other (`mush_core::perm::controls`: one
/// owner), and the market refuses a holder selling into a listing they act for as their own;
/// an NPC holder (Tycho's dispatcher) could not sell into the clerk's bid. Laid up once, on the
/// first sale at the port, and recorded on the port map ([`crate::port::map::DeskRow`]): a
/// restart finds it and never makes a second, nor a second purse ([`crate::port::pools::purse_of`]
/// finds the desk's own account).
///
/// # Errors
///
/// The port, its clerk, its workshop or its operator's row is missing, or the world refuses.
pub(crate) fn desk(ctx: &mut Ctx<'_>, market: &mush_market::MarketId) -> Result<ObjId, String> {
    let (entry, def, map) = crate::port::market::ports(ctx.w)
        .into_iter()
        .find(|(_, def, _)| def.venue.as_str() == market.as_str())
        .ok_or("no port trades at that venue")?;
    let mut map = map.cloned().ok_or("the port is not laid up")?;
    if let Some(desk) = &map.desk {
        return Ok(ObjId(desk.thing));
    }
    let operator = def
        .workshop
        .as_ref()
        .map(|workshop| workshop.operator.clone())
        .ok_or("the port has no workshop")?;
    let clerk = map
        .clerk
        .as_ref()
        .map(|clerk| ObjId(clerk.thing))
        .ok_or("the port has no clerk")?;
    let room = map
        .presentation
        .map(ObjId)
        .ok_or("the port has no presentation room")?;
    let row = map
        .orgs
        .get(&operator)
        .ok_or("the workshop's operator has no row")?;
    let (org, root_scope) = (
        mush_org::OrgId(row.org),
        mush_org::OrgScopeId(row.root_scope),
    );
    let name = DESK_NAME.replace("{port}", &crate::port::coop::port_name(def));
    let thing = ctx
        .w
        .create(mush_core::ObjKind::Thing, &name, clerk, Some(room))
        .map_err(|e| format!("{entry}: the buy-back desk: {e}"))?;
    ctx.w
        .set_flag(thing, mush_core::Flags::DARK, true)
        .map_err(|e| format!("{entry}: the buy-back desk: {e}"))?;
    let grant = crate::port::apply::alloc_grant_id(ctx)?;
    let capability = mush_org::CapabilityId::new(crate::port::vendor::CLERK_CAPABILITY)
        .ok_or("vault.withdraw is not a capability id")?;
    mush_org::submit(
        ctx,
        &mush_org::OrgMutationPlan::of(vec![mush_org::OrgMutationOp::PutGrant {
            grant: mush_org::CapabilityGrant {
                id: grant,
                org,
                grantee: thing,
                capability,
                scope: mush_org::ScopeSelector::Exact(root_scope),
                limit: None,
                delegable: false,
                valid_from: None,
                valid_until: None,
                revision: 1,
            },
        }]),
        mush_org::OrgAction {
            action: SmolStr::new_static("grant.create"),
            org: Some(org),
            detail: serde_json::json!({
                "port": entry.to_string(),
                "desk": thing.0,
            }),
        },
    )
    .map_err(|e| format!("{entry}: the buy-back desk's grant: {e}"))?;
    map.desk = Some(crate::port::map::DeskRow {
        thing: thing.0,
        grant: grant.0,
    });
    crate::store::put_port_map(ctx, &map);
    Ok(thing)
}

/// The row's bid and its sale; a refused sale withdraws the bid it posted.
fn sell_row(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    sale: &Sale,
    desk: ObjId,
    row: &SaleRow,
) -> Result<(), String> {
    let purse = crate::port::pools::purse_of(ctx, desk, "buy-back desk purse")?;
    let total = row.total_minor();
    let held = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(purse)
                .and_then(mush_economy::Account::available)
        })
        .map_or(0, |amount| u64::try_from(amount.minor()).unwrap_or(0));
    let lacks = total.saturating_sub(held);
    if lacks > 0 {
        fund(
            ctx,
            hull,
            sale,
            (AccountId(sale.operator_account), purse),
            (row, lacks),
        )?;
    }
    let instrument = mush_economy::InstrumentId::new(crate::economy::CREDIT)
        .ok_or("the credit instrument is unknown")?;
    let market = mush_market::MarketId::new(sale.venue.as_str()).ok_or("no such venue")?;
    let price = i64::try_from(row.price_minor).map_err(|_| "a price overflows")?;
    let saved = ctx.actor;
    ctx.actor = desk;
    let bid = mush_market::post_bid(
        ctx,
        mush_market::PersonalBid {
            bidder: PrincipalId::Object(desk),
            market,
            instrument,
            catalog: mush_economy::CatalogRef {
                ruleset: SmolStr::new_static(crate::NS),
                code: SmolStr::new(row.good.as_str()),
            },
            into: mush_inventory::EndpointRef(mush_economy::VaultId(sale.stores)),
            price: Amount::from_minor(price),
            quantity: row.units,
            private_to: Some(PrincipalId::Object(sale.holder)),
            ttl_secs: BID_TTL_SECS,
        },
    );
    ctx.actor = saved;
    let listing = bid.map_err(|error| format!("the bid: {error:?}"))?;
    ctx.actor = sale.holder;
    let sold = mush_market::sell_into_bid(
        ctx,
        &mush_market::BidSaleRequest {
            listing,
            quantity: row.units,
            source: Some(mush_inventory::EndpointRef(mush_economy::VaultId(
                sale.vault,
            ))),
            presence: mush_market::Presence::Here,
        },
    );
    ctx.actor = saved;
    if let Err(error) = sold {
        ctx.actor = desk;
        let withdrawn = mush_market::withdraw_own(ctx, listing);
        ctx.actor = saved;
        if let Err(again) = withdrawn {
            tracing::warn!(
                listing = listing.0,
                ?again,
                "tacsim: a scrap's bid was not withdrawn"
            );
        }
        return Err(format!("the sale: {error:?}"));
    }
    Ok(())
}

/// How long a scrap's bid stands, real seconds: the one step it is sold in, with room.
pub const BID_TTL_SECS: i64 = 3_600;

/// The operator's top-up of its clerk's purse for one row, under a key of the hull, the act and
/// the row's good.
fn fund(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    sale: &Sale,
    (payer, purse): (AccountId, AccountId),
    (row, minor): (&SaleRow, u64),
) -> Result<(), String> {
    let minor = i64::try_from(minor).map_err(|_| "an amount overflows")?;
    let debit = Amount::from_minor(minor)
        .checked_neg()
        .ok_or("an amount overflows")?;
    let plan = EconomyPlan {
        description: format!("The workshop funds its bid for #{}'s {}", hull.0, row.good),
        ..EconomyPlan::new(
            IdempotencyKey::new(
                CREDIT_NAMESPACE,
                format!("{}/{}/{}/{}", hull.0, sale.act, row.good, sale.rows.len()),
            ),
            PrincipalId::Organization(OrgId(
                ctx.w
                    .module_state::<EconomyStore>()
                    .and_then(|e| e.snapshot().account(payer).map(|a| a.owner.clone()))
                    .and_then(|owner| match owner {
                        PrincipalId::Organization(org) => Some(org.0),
                        _ => None,
                    })
                    .ok_or("the operator's account has no organization")?,
            )),
            EconomicCause::Internal {
                kind: SmolStr::new_static(CREDIT_NAMESPACE),
                id: u64::from(hull.0),
            },
            vec![
                RequestedLine {
                    account: payer,
                    delta: debit,
                    memo: SmolStr::new_static(CREDIT_NAMESPACE),
                },
                RequestedLine {
                    account: purse,
                    delta: Amount::from_minor(minor),
                    memo: SmolStr::new_static(CREDIT_NAMESPACE),
                },
            ],
        )
    };
    match mush_economy::post(ctx, &plan) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        other => Err(format!("the operator could not fund the bid: {other:?}")),
    }
}

/// **The holder's scrap vault at a workshop** (doc 48 cut EX.3, the lead's ruling): one a holder
/// a venue, reused by every scrap and return there, the holder's own and uncapped (no capacity
/// plan reads it), backed by a container in LIMBO the holder owns. Found by its owner and name.
#[must_use]
pub fn scrap_vault(w: &World, holder: ObjId, venue: &str) -> Option<mush_economy::VaultId> {
    let name = scrap_vault_name(venue);
    w.module_state::<EconomyStore>()?
        .snapshot()
        .vaults()
        .find(|v| {
            v.owner == PrincipalId::Object(holder)
                && v.name.as_str() == name
                && v.status == mush_economy::VaultStatus::Active
        })
        .map(|v| v.id)
}

/// Whether `vault` is a scrap vault at `venue`, and whose.
#[must_use]
pub fn scrap_vault_owner(w: &World, vault: mush_economy::VaultId, venue: &str) -> Option<ObjId> {
    let v = w.module_state::<EconomyStore>()?.snapshot().vault(vault)?;
    match &v.owner {
        PrincipalId::Object(owner) if v.name.as_str() == scrap_vault_name(venue) => Some(*owner),
        _ => None,
    }
}

/// **Whether `vault` is a scrap vault at any venue** (doc 48e, exchange G15): the yard alone
/// fills one, so no purchase, bid or move names it as a destination.
#[must_use]
pub fn is_scrap_vault(w: &World, vault: mush_economy::VaultId) -> bool {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().vault(vault).cloned())
        .is_some_and(|v| {
            matches!(v.owner, PrincipalId::Object(_))
                && v.name
                    .as_str()
                    .strip_prefix("scrap at ")
                    .is_some_and(|venue| v.name.as_str() == scrap_vault_name(venue))
        })
}

/// A scrap vault's name at `venue`.
fn scrap_vault_name(venue: &str) -> String {
    format!("scrap at {venue}")
}

/// **Open the holder's scrap vault at `venue`**, or find the one that stands.
///
/// # Errors
///
/// The economy cannot open one, as a sentence.
pub(crate) fn open_scrap_vault(
    ctx: &mut Ctx<'_>,
    holder: ObjId,
    venue: &str,
) -> Result<mush_economy::VaultId, String> {
    if let Some(vault) = scrap_vault(ctx.w, holder, venue) {
        return Ok(vault);
    }
    let vault = ctx
        .w
        .module_state_mut::<EconomyStore>()
        .ok_or("The economy is not running.")?
        .alloc_vault()
        .ok_or("No vault can be opened for her matter right now.")?;
    let name = scrap_vault_name(venue);
    let container = ctx
        .w
        .create(
            mush_core::ObjKind::Thing,
            &name,
            holder,
            Some(mush_core::LIMBO),
        )
        .map_err(|_| "No vault can be opened for her matter right now.".to_owned())?;
    mush_economy::create_vault(
        ctx,
        mush_economy::Vault {
            id: vault,
            owner: PrincipalId::Object(holder),
            scope: None,
            name: SmolStr::new(name),
            backing: mush_economy::CustodyRef::Container(container),
            status: mush_economy::VaultStatus::Active,
            revision: 1,
        },
    );
    Ok(vault)
}

/// **A hull's surviving structural points** (§4.6): every facet's armour and every section cell
/// above its minimum, never the heat pair.
#[must_use]
pub fn surviving_points(unit: &mush_tactics::TacticalUnit) -> u64 {
    use mush_tactics::unit::CellAddr;
    unit.cells()
        .filter(|(addr, _)| match addr {
            CellAddr::Section { cell, .. } => {
                cell.as_str() != crate::definitions::unit::HEAT_CELL
                    && cell.as_str() != crate::definitions::unit::HEAT_SINK_CELL
            }
            CellAddr::Facet { .. } => true,
            _ => false,
        })
        .fold(0u64, |sum, (_, cell)| {
            sum.saturating_add(u64::try_from(cell.current.saturating_sub(cell.min)).unwrap_or(0))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT: References = References {
        hull_minor: 7_549_500,
        alloys_minor_t: 45_000,
        fittings_minor_t: 52_000,
    };

    fn escort_parts(intact: bool) -> Vec<PartIn> {
        [2_800_000, 250_000, 900_000, 2_200_000]
            .into_iter()
            .map(|reference_minor| PartIn {
                reference_minor,
                intact,
            })
            .collect()
    }

    /// **Doc 39 §4.6's escort** (cut IN.8): a whole intact escort at Stickney — 692 cr for the
    /// structure (under the 2,799 fifth), 44,525 for the four parts, about 45,200 in all; 1 t
    /// into the pool and 539 kg to the shop bale.
    #[test]
    fn in8_a_whole_escort_scraps_for_its_parts_and_a_tonne() {
        let t = terms(
            AT,
            1_539,
            &escort_parts(true),
            40_000,
            None,
            Bale::default(),
        );
        assert_eq!(t.structural_minor, 69_255);
        assert_eq!(t.parts_minor, 4_452_500);
        assert_eq!(t.credit_minor(), 4_521_755);
        assert_eq!((t.alloys_t, t.fittings_t), (1, 0));
        assert_eq!(t.shop_after.alloys_kg, 539);
        // Worn parts come back at 0.40.
        let worn = terms(
            AT,
            1_539,
            &escort_parts(false),
            40_000,
            None,
            Bale::default(),
        );
        assert_eq!(worn.parts_minor, 2_460_000 - 160_000);
    }

    /// **The Dray** (§4.6): 90,000 with a 30,000 plant; the fifth (12,000) caps the 94 t;
    /// the plant used is 22,100 after its removal; 34,100 in all and 94 t of alloys. With the
    /// plant destroyed the fifth of 90,000 is 18,000.
    #[test]
    fn in8_a_dray_is_capped_by_the_fifth() {
        let at = References {
            hull_minor: 9_000_000,
            ..AT
        };
        let plant = [PartIn {
            reference_minor: 3_000_000,
            intact: true,
        }];
        let t = terms(at, 94_500, &plant, 40_000, None, Bale::default());
        assert_eq!(t.structural_minor, 1_200_000);
        assert_eq!(t.credit_minor(), 3_410_000);
        assert_eq!(t.alloys_t, 94);
        let wrecked = terms(at, 94_500, &[], 40_000, None, Bale::default());
        assert_eq!(wrecked.credit_minor(), 1_800_000);
    }

    /// **Doc 48 cut EX.3: scrap is paid by the whole tonne.** The escort's sale is its tonne at the
    /// structure's credit and its parts at their values; a part worth nothing is no row; a bulk
    /// good's price is its value over its tonnes rounded up, at least 1; and with nothing whole,
    /// there is no row at all.
    #[test]
    fn ex3_the_sale_rows_are_the_whole_tonnes_and_the_parts() {
        let t = terms(
            AT,
            1_539,
            &escort_parts(true),
            40_000,
            None,
            Bale::default(),
        );
        let gun = CanonicalId::new("tacsim.part.weapon.mg_coax").unwrap();
        let rows = sale_rows(&t, AT, None, &[(gun.clone(), 147_500), (gun, 0)]);
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert_eq!(
            (rows[0].good.as_str(), rows[0].units, rows[0].price_minor),
            ("structural_alloys", 1, 69_255)
        );
        assert_eq!(rows[1].total_minor(), 147_500);
        assert_eq!(rows_minor(&rows), 69_255 + 147_500);
        // 94 t of 1,200,000: 12,766 a tonne, rounded up (the holder is paid 4 minor more).
        let dray = Terms {
            structural_minor: 1_200_000,
            alloys_t: 94,
            ..Terms::default()
        };
        let rows = sale_rows(&dray, AT, None, &[]);
        assert_eq!((rows[0].units, rows[0].price_minor), (94, 12_766));
        // A tonne of the shop's own remainder with nothing of the holder's in it: 1 minor.
        let shop_only = Terms {
            fittings_t: 1,
            ..Terms::default()
        };
        assert_eq!(sale_rows(&shop_only, AT, None, &[])[0].price_minor, 1);
        // Under a tonne: nothing whole, no row, nothing paid.
        let under = terms(
            AT,
            0,
            &[],
            0,
            Some(Bale {
                alloys_kg: 400,
                fittings_kg: 0,
            }),
            Bale::default(),
        );
        assert!(
            sale_rows(
                &under,
                AT,
                Some(Bale {
                    alloys_kg: 400,
                    fittings_kg: 0
                }),
                &[]
            )
            .is_empty()
        );
        // A part's value is its used value less its removal, 0 when the removal takes it all.
        let worn = PartIn {
            reference_minor: 50_000,
            intact: false,
        };
        assert_eq!(part_value(worn, 40_000), 0);
        assert_eq!(part_value(worn, 10_000), 10_000);
    }

    /// **A return** (§4.10): 914 kg of alloys back at 411 cr into the shop bale; a second
    /// return takes it past a tonne, which mints.
    #[test]
    fn in8_a_returned_bale_is_bought_at_the_reference_and_mints_at_a_tonne() {
        let none = References {
            hull_minor: 0,
            ..AT
        };
        let bale = Bale {
            alloys_kg: 914,
            fittings_kg: 0,
        };
        let first = terms(none, 0, &[], 0, Some(bale), Bale::default());
        assert_eq!(first.credit_minor(), 41_130);
        assert_eq!((first.alloys_t, first.shop_after.alloys_kg), (0, 914));
        let second = terms(none, 0, &[], 0, Some(bale), first.shop_after);
        assert_eq!((second.alloys_t, second.shop_after.alloys_kg), (1, 828));
    }
}
