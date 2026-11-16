//! **`@industry/census`: the matter census** (doc 39 cut IN.11, §4.10; Q-39-8): the chain's
//! conservation in kilograms, read off durable state, one line a world.
//!
//! The rule §4.10 states — Σ matter = pools + lots + hull bales + shop bales + declared waste —
//! is checked **across a window** (the owner's ruling for IN.11): a world's seeded stock carries
//! no lineage once its lots are pooled, so the census prints absolute terms and a story reads
//! two censuses and holds their difference. Over any window in which no other source or sink
//! acts, `balance` does not move.
//!
//! - **Sources**: the opened deposits' reserves (`opened`; mining is a transfer out of a deposit's
//!   vault, so `mined` = `opened` − `in_ground`), the stock clerk's mints (`stock`, the
//!   `tacsim.port_stock` receipts), and what a scrap mints (`scrapped`: the hull's mass was never
//!   in the census) — the alloys remains scrap mints on a site (the sub-tonne under its whole
//!   tonnes is §4.6's declared loss), and a workshop's scrap's yield and stripped parts (doc 41
//!   cut RS.9a; its sub-tonne goes to the shop bale, a holding).
//! - **Re-rolls** (doc 48 unit PL.10, added by EX.0): `rerolled`, the tonnes a worked-out
//!   deposit's re-roll credits back into its pool (`tacsim.pockets`' re-roll ledgers), a source
//!   the ground never held.
//! - **Holdings**: every pool and lot of a chain good outside the deposits (`pools`, `lots`), the
//!   hulls' opened bales and the workshops' shop bales (`bales`).
//! - **Sinks and declared losses**: every completed work order's inputs less its outputs
//!   (`worked`: the recipes' losses, and at a yard the hull's bulk, a kit having no mass), the
//!   mills' counted inputs (the ore, and a process's further inputs, doc 48 units PL.2 and
//!   PL.3) less their counted product (`milled`),
//!   and the parts a commission consumed into a hull
//!   (`built`), the parts the bay fitted into hulls (`fitted`: a fit consumes the lot and
//!   keeps its provenance), and the bale kilograms repairs consumed into hulls (`repaired`: a
//!   workshop's or carrier bay's on the hull's bay record, a tender's on its site's record).
//! - **The consumables' sinks** (doc 48 cut EX.0; Q-48-31):
//!   - `burned`: the propellant and provisions the burns spent. A departure burn's
//!     `tacsim.ticket` is its debit, written in the burn's own bundle and never deleted; an
//!     in-flight burn adds its debit to its hull's `tacsim.space_burn_receipt` totals, in its
//!     own bundle too.
//!   - `loaded`: the munitions the rearms drew into magazines: the bay's, on the hull's bay
//!     record as each draw lands, and a tender's, on its site's record. A round in a magazine
//!     is no longer a good, and what it does there is the combat code's.
//!   - `seized`: the unlicensed strategic ore an in-flight burn's customs picket took, on the
//!     same receipt.
//!   - `consumed`: what a consumer used up (Q-48-19's consumption arm, EX.9's). No act
//!     consumes yet, so it reads 0.
//!
//! `balance` = `stock` + `opened` + `scrapped` + `rerolled` − `in_ground` − `pools` − `lots` −
//! `bales` − `worked` − `milled` − `built` − `fitted` − `repaired` − `burned` − `loaded` −
//! `seized` − `consumed`. The owner's IN.11 rulings of 2026-11-05 added `repaired` and
//! `scrapped`; EX.0 added `rerolled` and the last four; the durable matter ledger stays parked.
//! A new term is one field, one line in [`balance`](crate::port::census::MatterCensus::balance)
//! and one in [`line`](crate::port::census::MatterCensus::line). Each commission also prints one
//! `tacsim.matter_build` line: the inputs (the bill's bulk and its parts, a stock core's module
//! among them, doc 48 unit PL.7) against the hull's dry mass and the bill's declared waste, which
//! holds `inputs = hull + waste` at every build.
//!
//! **The goods counted**: the chain's (the ores, the refined goods and the parts) and, since
//! EX.0, the consumables ([`COUNTED_BULK`](crate::port::census::COUNTED_BULK): volatiles and
//! the ice they are milled from; propellant and provisions by their `consumable` mark;
//! munitions by their `munitions` row).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, Ctx, World};
use mush_inventory::record::EndpointRef;

use crate::store::TacsimStore;

/// The census line's lead word (what RS.9 and IN.11's stories read).
pub const CENSUS_LINE: &str = "tacsim.matter_census";
/// A build's line's lead word.
pub const BUILD_LINE: &str = "tacsim.matter_build";
/// The bulk goods of the chain (every part good counts too).
pub const CHAIN_BULK: &[&str] = &[
    "metal_ore",
    "carbon_ore",
    "strategic_ore",
    "metals",
    "structural_alloys",
    "industrial_parts",
    "instrument_boards",
    "carbon_feedstock",
];
/// **The goods outside the chain the census counts by id** (doc 48 cut EX.0; Q-48-31): the
/// volatiles, and the ice ore Occator's mill makes them from, so the mill balances. Propellant
/// and provisions are counted by their `consumable` mark and munitions by their `munitions`
/// row, never by id.
pub const COUNTED_BULK: &[&str] = &["ice_ore", "volatiles"];

/// **Whether the census counts `good`**: a chain good, a part, a [`COUNTED_BULK`] good, a
/// consumable or a munitions good.
#[must_use]
pub fn counts(goods: &crate::definitions::GoodsTable, good: &CanonicalId) -> bool {
    CHAIN_BULK.contains(&good.as_str())
        || COUNTED_BULK.contains(&good.as_str())
        || crate::port_tick::is_part(good)
        || goods
            .good(good)
            .is_some_and(|def| def.consumable.is_some() || def.munitions.is_some())
}

/// The page a family scan reads at a time.
const PAGE: usize = 256;
/// Pages a family scan reads at most.
const MAX_PAGES: usize = 4_096;

/// **One census**, kilograms by term.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatterCensus {
    /// The opened deposits' reserves.
    pub opened: u64,
    /// What the deposits still hold.
    pub in_ground: u64,
    /// The stock clerk's mints.
    pub stock: u64,
    /// Pools of chain goods outside the deposits.
    pub pools: u64,
    /// Lots of chain goods.
    pub lots: u64,
    /// The hulls' and the workshops' bales.
    pub bales: u64,
    /// The completed work orders' inputs less their outputs.
    pub worked: u64,
    /// The mills' counted inputs (the ore, and a process's further inputs, doc 48 units PL.2 and
    /// PL.3) less their counted product.
    pub milled: u64,
    /// The parts commissions consumed into hulls.
    pub built: u64,
    /// The parts the bay fitted into hulls.
    pub fitted: u64,
    /// The bale kilograms repairs consumed into hulls.
    pub repaired: u64,
    /// The alloys remains scrap minted on sites.
    pub scrapped: u64,
    /// The tonnes the deposits' re-rolls credited back, weighed (doc 48 unit PL.10).
    pub rerolled: u64,
    /// The propellant and provisions the burns spent (doc 48 cut EX.0).
    pub burned: u64,
    /// The munitions the rearms drew into magazines (doc 48 cut EX.0).
    pub loaded: u64,
    /// The strategic ore in-flight customs pickets seized (doc 48 cut EX.0).
    pub seized: u64,
    /// What a consumer used up (Q-48-19; EX.9 writes it, 0 until then).
    pub consumed: u64,
    /// One line a commission: `(order, inputs, hull, waste)`.
    pub builds: Vec<(u64, u64, u64, u64)>,
}

impl MatterCensus {
    /// Mined: the opened reserves less what the ground still holds.
    #[must_use]
    pub const fn mined(&self) -> u64 {
        self.opened.saturating_sub(self.in_ground)
    }

    /// Sources less holdings and sinks; constant over a window with no other source or sink.
    #[must_use]
    pub fn balance(&self) -> i128 {
        i128::from(self.stock)
            + i128::from(self.opened)
            + i128::from(self.scrapped)
            + i128::from(self.rerolled)
            - i128::from(self.in_ground)
            - i128::from(self.pools)
            - i128::from(self.lots)
            - i128::from(self.bales)
            - i128::from(self.worked)
            - i128::from(self.milled)
            - i128::from(self.built)
            - i128::from(self.fitted)
            - i128::from(self.repaired)
            - i128::from(self.burned)
            - i128::from(self.loaded)
            - i128::from(self.seized)
            - i128::from(self.consumed)
    }

    /// **The census line**: `tacsim.matter_census mined= opened= in_ground= stock= pools= lots=
    /// bales= worked= milled= built= fitted= repaired= scrapped= rerolled= burned= loaded=
    /// seized= consumed= balance=`, kilograms.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{CENSUS_LINE} mined={} opened={} in_ground={} stock={} pools={} lots={} bales={} \
             worked={} milled={} built={} fitted={} repaired={} scrapped={} rerolled={} \
             burned={} loaded={} seized={} consumed={} balance={}",
            self.mined(),
            self.opened,
            self.in_ground,
            self.stock,
            self.pools,
            self.lots,
            self.bales,
            self.worked,
            self.milled,
            self.built,
            self.fitted,
            self.repaired,
            self.scrapped,
            self.rerolled,
            self.burned,
            self.loaded,
            self.seized,
            self.consumed,
            self.balance()
        )
    }

    /// The build lines, one a commission.
    #[must_use]
    pub fn build_lines(&self) -> Vec<String> {
        self.builds
            .iter()
            .map(|(order, inputs, hull, waste)| {
                format!("{BUILD_LINE} order={order} inputs={inputs} hull={hull} waste={waste}")
            })
            .collect()
    }
}

/// Every value of one family, page by page.
fn scan<K, V>(ctx: &Ctx<'_>, handle: &mush_core::StateFamilyHandle<K, V>, mut each: impl FnMut(&V))
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let mut cursor: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let Ok(page) = ctx.scan_action_state(handle, cursor.as_deref(), PAGE) else {
            return;
        };
        for entry in page.entries() {
            each(entry.value());
        }
        match page.next_after() {
            Some(next) => cursor = Some(next.to_string()),
            None => return,
        }
    }
}

/// **Take the census.**
///
/// # Errors
///
/// The tacsim layer, its goods table or its families are not composed.
#[allow(clippy::too_many_lines)] // one term after another
pub fn take(ctx: &Ctx<'_>) -> Result<MatterCensus, String> {
    let w: &World = ctx.w;
    let store = w
        .module_state::<TacsimStore>()
        .ok_or("the tacsim layer is not composed")?;
    let goods = store.goods().ok_or("the goods table is not loaded")?;
    let counted = |good: &CanonicalId| counts(goods, good);
    let kg = |good: &CanonicalId, units: u64| -> u64 {
        goods
            .good(good)
            .and_then(|def| def.mass_kg_per_unit())
            .map_or(0, |per| per.saturating_mul(units))
    };
    let handles = w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
        .ok_or("the port families are not composed")?;
    let inventory = handles
        .inventory
        .clone()
        .ok_or("the inventory families are not composed")?;
    let industry = w
        .module_state::<mush_industry::IndustryHandles>()
        .map(|h| h.0.clone())
        .ok_or("industry is not composed")?;
    let mut out = MatterCensus::default();

    // Sources: the deposits (their endpoints are the ground), the stock clerk's mints.
    let mut ground: BTreeSet<EndpointRef> = BTreeSet::new();
    let mut deposits = Vec::new();
    let mut deposit_goods: BTreeMap<mush_industry::OreDepositId, CanonicalId> = BTreeMap::new();
    scan(ctx, &industry.deposits, |deposit| {
        if counted(deposit.commodity()) {
            ground.insert(deposit.endpoint());
            deposits.push((deposit.endpoint(), deposit.commodity().clone()));
            deposit_goods.insert(deposit.id(), deposit.commodity().clone());
            out.opened = out
                .opened
                .saturating_add(kg(deposit.commodity(), deposit.initial_reserve()));
        }
    });
    for (endpoint, good) in &deposits {
        let units = crate::port_tick::read_pool(ctx, *endpoint, good).unwrap_or(0);
        out.in_ground = out.in_ground.saturating_add(kg(good, units));
    }
    // Doc 48 unit PL.10: a re-roll credits a worked-out deposit's pool again, from no source.
    scan(ctx, &handles.families.pockets, |cursor| {
        if let (Some(ledger), Some(good)) = (&cursor.reroll, deposit_goods.get(&cursor.deposit)) {
            out.rerolled = out.rerolled.saturating_add(kg(good, ledger.rerolled_t));
        }
    });
    scan(ctx, &handles.families.port_stock, |receipt| {
        if counted(&receipt.good) {
            out.stock = out.stock.saturating_add(kg(&receipt.good, receipt.qty));
        }
    });

    // Holdings: the pools by their books' commodity, the lots, the bales.
    let mut books: BTreeMap<mush_core::ReservationBookId, CanonicalId> = BTreeMap::new();
    scan(ctx, &inventory.books, |book| {
        let good = book.resource().id.clone();
        if counted(&good) {
            books.insert(book.id(), good);
        }
    });
    let mut ground_books = BTreeSet::new();
    for (endpoint, good) in &deposits {
        if let Ok(key) = mush_inventory::pool_key(*endpoint, good) {
            ground_books.insert(key);
        }
    }
    for (id, good) in &books {
        if ground_books.contains(id) {
            continue;
        }
        let units = ctx
            .action_state(&inventory.pools, id)
            .ok()
            .flatten()
            .map_or(0, |scalar| u64::try_from(scalar.current()).unwrap_or(0));
        out.pools = out.pools.saturating_add(kg(good, units));
    }
    scan(ctx, &inventory.lots, |lot| {
        if counted(lot.commodity()) && !ground.contains(&lot.endpoint()) {
            out.lots = out.lots.saturating_add(kg(lot.commodity(), lot.quantity()));
        }
    });
    for state in store.bay_hulls.values() {
        if let crate::port::bay_work::BayHullState::Present(row) = state {
            out.repaired = out.repaired.saturating_add(row.repaired_kg);
            // Doc 41 cut RS.9a: a workshop's scrap mints from a hull the census never held.
            out.scrapped = out.scrapped.saturating_add(row.scrapped_kg);
            // Doc 48 cut EX.0: the munitions the bay's draws took for her magazines.
            out.loaded = out.loaded.saturating_add(row.loaded_kg);
            if let Some(bale) = row.bale.as_ref() {
                out.bales = out
                    .bales
                    .saturating_add(bale.alloys_kg)
                    .saturating_add(bale.fittings_kg);
            }
        }
    }
    // The sites' matter: the tenders' repairs (a sink) and remains scrap's mints (a source).
    scan(ctx, &handles.families.sites, |site| {
        out.repaired = out.repaired.saturating_add(site.matter.repaired_kg);
        out.scrapped = out.scrapped.saturating_add(site.matter.scrapped_kg);
        // Doc 48 cut EX.0: the munitions a tender's rearms drew, weighed here.
        for (good, units) in &site.matter.loaded {
            if counted(good) {
                out.loaded = out.loaded.saturating_add(kg(good, *units));
            }
        }
    });
    for bale in store.shop_bales.values().flatten() {
        out.bales = out
            .bales
            .saturating_add(bale.kg.alloys_kg)
            .saturating_add(bale.kg.fittings_kg);
    }

    // Sinks: the completed work orders, the mills, the commissions.
    scan(ctx, &industry.work_orders, |order| {
        if !matches!(
            order.state(),
            mush_industry::WorkOrderState::Completed { .. }
        ) {
            return;
        }
        let quantity = u64::from(order.quantity());
        let mass = |lines: &[mush_industry::RecipeLine]| -> u64 {
            lines
                .iter()
                .filter(|line| counted(line.commodity()))
                .map(|line| kg(line.commodity(), line.units().saturating_mul(quantity)))
                .sum()
        };
        let pinned = order.pinned();
        out.worked = out
            .worked
            .saturating_add(mass(pinned.inputs()).saturating_sub(mass(pinned.outputs())));
    });
    scan(ctx, &handles.families.mills, |ledger| {
        // Every counted input in, the counted product out (EX.0 counts Occator's ice mill too):
        // the ore and, for a process, its further inputs (doc 48 units PL.2 and PL.3).
        let weigh = |good: &CanonicalId, units: u64| {
            if counted(good) { kg(good, units) } else { 0 }
        };
        let inputs = ledger
            .with
            .iter()
            .zip(&ledger.with_t)
            .fold(weigh(&ledger.ore, ledger.ore_t), |sum, (with, units)| {
                sum.saturating_add(weigh(&with.good, *units))
            });
        out.milled = out
            .milled
            .saturating_add(inputs.saturating_sub(weigh(&ledger.into, ledger.product_t)));
    });
    // The parts the bay fitted into hulls: each lot consumed, its provenance kept.
    if let Some(bay) = w.module_state::<crate::port::bay_work::act::BayActHandles>() {
        scan(ctx, &bay.parts, |part| {
            if part.fitted_into.is_none() {
                return;
            }
            // The part's good: its row's, else Q-39-35's spelling, `tacsim.part.<kind>.<row>` for
            // a hardware row `tacsim.<kind>.<row>` (a plant row is not in the slot index).
            let good = crate::hardware::bay::part_of(w, &part.definition)
                .map(|row| row.commodity)
                .or_else(|| {
                    part.definition
                        .key()
                        .id
                        .as_str()
                        .strip_prefix("tacsim.")
                        .and_then(|rest| CanonicalId::new(format!("tacsim.part.{rest}")).ok())
                });
            if let Some(good) = good {
                out.fitted = out.fitted.saturating_add(kg(&good, 1));
            }
        });
    }
    // Doc 48 cut EX.0: the departure burns, each ticket one burn's propellant and crew-days.
    let consumable = |kind| crate::port::burn::consumable_good(goods, kind);
    let propellant = consumable(crate::definitions::goods::Consumable::Propellant);
    let provisions = consumable(crate::definitions::goods::Consumable::Provisions);
    let spent = |good: &Option<CanonicalId>, units: u64| {
        good.as_ref()
            .filter(|good| counted(good))
            .map_or(0, |good| kg(good, units))
    };
    scan(ctx, &handles.families.tickets, |ticket| {
        out.burned = out
            .burned
            .saturating_add(spent(&propellant, ticket.plan.burn_t))
            .saturating_add(spent(&provisions, u64::from(ticket.plan.crew_days)));
    });
    // The in-flight burns: each hull's receipt totals, and the ore its pickets seized.
    let strategic_ore = CanonicalId::new("strategic_ore").ok();
    scan(ctx, &handles.families.space_burn_receipts, |receipts| {
        out.burned = out
            .burned
            .saturating_add(spent(&propellant, receipts.burned_t))
            .saturating_add(spent(&provisions, receipts.crew_days));
        out.seized = out
            .seized
            .saturating_add(spent(&strategic_ore, receipts.seized_t));
    });
    for record in crate::port::commission::commissions(w) {
        if record.step == crate::port::commission::Step::Consuming {
            continue;
        }
        let Ok(bill) = crate::port::bill::design_bill(store.definitions(), goods, &record.design)
        else {
            continue;
        };
        let parts: u64 = bill.part_lines().iter().map(|(good, n)| kg(good, *n)).sum();
        out.built = out.built.saturating_add(parts);
        let bulk = bill
            .alloys_t
            .saturating_add(bill.fittings_t)
            .saturating_mul(1_000);
        // Doc 48 unit PL.7: a stock core is billed as its core module, one of the part lines, so
        // the core's kilograms are in `parts` already (a coreless design has none).
        let inputs = bulk.saturating_add(parts);
        out.builds.push((
            record.order,
            inputs,
            bill.budget.dry_mass_kg,
            bill.waste_kg(),
        ));
    }
    Ok(out)
}

/// **The munitions kilograms in `goods`** (doc 48 cut EX.0): the rows of goods with a
/// `munitions` row, weighed; the bay's draw counts them into a hull's `loaded_kg` as it lands.
#[must_use]
pub fn munitions_kg(w: &World, goods: &[crate::port::bay_work::act::GoodUnits]) -> u64 {
    let Some(table) = w.module_state::<TacsimStore>().and_then(TacsimStore::goods) else {
        return 0;
    };
    goods
        .iter()
        .filter_map(|row| {
            let def = table.good(&row.good)?;
            def.munitions.as_ref()?;
            Some(def.mass_kg_per_unit()?.saturating_mul(row.units))
        })
        .fold(0u64, u64::saturating_add)
}

/// **The money census's hook** (doc 48 cut EX.0): [`money_classes`], as `mush_economy`'s
/// `MoneyCensusHook`, which tacsim's registration installs.
#[must_use]
pub fn money_census_hook() -> mush_economy::census::MoneyCensusHook {
    mush_economy::census::MoneyCensusHook {
        org_classes: money_classes,
    }
}

/// **The money census's word on the game's organisations** (doc 48 cut EX.0; Q-48-17): every
/// institution a port made is a lane institution, and the launch's three named Houses are
/// Houses. Installed as `mush_economy::census::MoneyCensusHook`; bounded by the lane's own
/// content (its ports' institutions) and three Houses.
#[must_use]
pub fn money_classes(w: &World) -> BTreeMap<mush_org::OrgId, mush_economy::census::HoldingClass> {
    use mush_economy::census::HoldingClass;
    let mut out = BTreeMap::new();
    if let Some(store) = w.module_state::<TacsimStore>() {
        for map in store.port_maps() {
            for row in map.orgs.values() {
                out.insert(mush_org::OrgId(row.org), HoldingClass::Lane);
            }
        }
    }
    if let Some(launch) = w.module_state::<crate::launch::registration::LaunchService>() {
        let houses = &launch.0.deployment().houses;
        for house in [&houses.meridiani, &houses.coprates, &houses.tycho] {
            out.insert(house.organization, HoldingClass::House);
        }
    }
    out
}

/// One `tacsim.matter_lot` line a lot of `good`: the lot, its quantity, its vault, the vault's
/// name and owner.
fn lot_lines(ctx: &Ctx<'_>, good: &CanonicalId) -> Vec<String> {
    let Some(inventory) = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .and_then(|h| h.inventory.clone())
    else {
        return Vec::new();
    };
    let economy = ctx.w.module_state::<mush_economy::EconomyStore>();
    let mut out = Vec::new();
    scan(ctx, &inventory.lots, |lot| {
        if lot.commodity() != good || out.len() >= 64 {
            return;
        }
        let vault = lot.endpoint().vault();
        let (name, owner) = economy
            .and_then(|e| e.snapshot().vault(vault).cloned())
            .map_or_else(
                || ("?".to_owned(), "?".to_owned()),
                |v| (v.name.to_string(), format!("{:?}", v.owner)),
            );
        out.push(format!(
            "tacsim.matter_lot {} x{} vault#{} \"{name}\" owner={owner}",
            lot.id(),
            lot.quantity(),
            vault.0
        ));
    });
    out
}

/// **`@industry/census`** (staff): the census line, then one build line a commission.
///
/// # Errors
///
/// Not staff, the layer off, or the census unreadable.
pub(crate) fn census_cmd(ctx: &mut Ctx<'_>) -> Result<(), mush_core::CmdError> {
    crate::commands::require_staff(ctx)?;
    crate::commands::require_enabled(ctx)?;
    let census = take(ctx).map_err(mush_core::CmdError::msg)?;
    let actor = ctx.actor;
    // `@industry/census <good>`: where every lot of that good stands (a staff diagnostic).
    let raw = ctx.args.raw().trim().to_owned();
    if !raw.is_empty() {
        let good = CanonicalId::new(&raw).map_err(|_| mush_core::CmdError::msg("No such good."))?;
        for line in lot_lines(ctx, &good) {
            ctx.out.sys_to_actor(actor, "tacsim", line);
        }
    }
    ctx.out.sys_to_actor(actor, "tacsim", census.line());
    for line in census.build_lines() {
        ctx.out.sys_to_actor(actor, "tacsim", line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::definitions::GoodsTable;
    use crate::definitions::goods::GOODS_JSON;

    type R = Result<(), Box<dyn std::error::Error>>;

    fn id(text: &str) -> Result<CanonicalId, Box<dyn std::error::Error>> {
        CanonicalId::new(text).map_err(|e| format!("{e:?}").into())
    }

    /// Doc 48 cut EX.0 (Q-48-31): the consumables count beside the chain; a kit never does.
    #[test]
    fn ex0_the_census_counts_the_consumables_and_the_chain() -> R {
        let goods = GoodsTable::load("goods.json", GOODS_JSON).map_err(|e| format!("{e:?}"))?;
        for good in [
            "volatiles",
            "ice_ore",
            "propellant",
            "provisions",
            "ammo_ap_30",
            "ammo_ball_762",
            "ammo_he_105",
            "ammo_missile",
            "metal_ore",
            "structural_alloys",
            "tacsim.part.weapon.cannon_105",
        ] {
            check!(counts(&goods, &id(good)?), "{good} is counted");
        }
        check!(
            !counts(&goods, &id("tacsim.hull_kit.dray")?),
            "a kit has no mass"
        );
        check!(!counts(&goods, &id("no_such_good")?), "an unknown good");
        // Every munitions good of the table counts, by its row, never by its id.
        for good in goods.goods.iter().filter(|g| g.munitions.is_some()) {
            check!(counts(&goods, &good.id), "{} is counted", good.id);
        }
        Ok(())
    }

    /// The approved writers' fields (doc 48 cut EX.0) encode as before while they are 0 or
    /// empty, so no existing record moves, and they round-trip once written.
    #[test]
    fn ex0_the_new_counters_encode_as_before_until_they_count() -> R {
        use crate::port::bay_work::BayHull;
        use crate::port::burn::SpaceBurnReceipts;
        use crate::surface::site_record::SiteMatter;
        let hull = BayHull::new(mush_core::ObjId(5));
        let wire = serde_json::to_value(&hull)?;
        check!(wire.get("loaded_kg").is_none(), "{wire}");
        let mut loaded = hull.clone();
        loaded.loaded_kg = 63;
        check_eq!(
            serde_json::from_value::<BayHull>(serde_json::to_value(&loaded)?)?,
            loaded
        );
        // A bay record written before EX.0 reads with nothing loaded.
        check_eq!(serde_json::from_value::<BayHull>(wire)?.loaded_kg, 0);

        let mut receipts = SpaceBurnReceipts {
            hull: mush_core::ObjId(7),
            landed: Vec::new(),
            burned_t: 0,
            crew_days: 0,
            seized_t: 0,
        };
        let fresh = serde_json::to_value(&receipts)?;
        for key in ["burned_t", "crew_days", "seized_t"] {
            check!(fresh.get(key).is_none(), "{fresh}");
        }
        receipts.record(1);
        receipts.spend(4, 2, 0);
        receipts.record(2);
        receipts.spend(3, 0, 5);
        check_eq!(
            (receipts.burned_t, receipts.crew_days, receipts.seized_t),
            (7, 2, 5)
        );
        check_eq!(
            serde_json::from_value::<SpaceBurnReceipts>(serde_json::to_value(&receipts)?)?,
            receipts
        );

        let mut matter = SiteMatter::default();
        check!(matter.is_fresh());
        check_eq!(
            serde_json::to_value(&matter)?,
            serde_json::json!({
                "repaired_kg": 0,
                "scrapped_kg": 0
            })
        );
        matter.loaded.insert(id("ammo_he_105")?, 10);
        check!(!matter.is_fresh(), "a tender's rearm is matter moved");
        check_eq!(
            serde_json::from_value::<SiteMatter>(serde_json::to_value(&matter)?)?,
            matter
        );
        Ok(())
    }

    /// The three sinks are subtracted, and the line names each before the balance.
    #[test]
    fn ex0_the_sink_terms_balance_and_print() {
        let census = MatterCensus {
            stock: 9_000,
            rerolled: 1_000,
            pools: 4_000,
            burned: 3_000,
            loaded: 1_500,
            seized: 500,
            consumed: 1_000,
            ..MatterCensus::default()
        };
        assert_eq!(census.balance(), 0);
        let line = census.line();
        assert!(
            line.ends_with("burned=3000 loaded=1500 seized=500 consumed=1000 balance=0"),
            "{line}"
        );
        assert!(line.contains(" scrapped=0 rerolled=1000 burned="), "{line}");
        let mut more = census.clone();
        more.rerolled += 1;
        assert_eq!(more.balance(), 1, "a re-roll is a source");
        for term in ["burned", "loaded", "seized", "consumed"] {
            let mut more = census.clone();
            match term {
                "burned" => more.burned += 1,
                "loaded" => more.loaded += 1,
                "seized" => more.seized += 1,
                _ => more.consumed += 1,
            }
            assert_eq!(more.balance(), -1, "{term} is a sink");
        }
    }
}
