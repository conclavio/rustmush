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
//! - **Holdings**: every pool and lot of a chain good outside the deposits (`pools`, `lots`), the
//!   hulls' opened bales and the workshops' shop bales (`bales`).
//! - **Sinks and declared losses**: every completed work order's inputs less its outputs
//!   (`worked`: the recipes' losses, and at a yard the hull's bulk, a kit having no mass), the
//!   mills' ore less their product (`milled`), and the parts a commission consumed into a hull
//!   (`built`), the parts the bay fitted into hulls (`fitted`: a fit consumes the lot and
//!   keeps its provenance), and the bale kilograms repairs consumed into hulls (`repaired`: a
//!   workshop's or carrier bay's on the hull's bay record, a tender's on its site's record).
//!
//! `balance` = `stock` + `opened` + `scrapped` − `in_ground` − `pools` − `lots` − `bales` −
//! `worked` − `milled` − `built` − `fitted` − `repaired` (the owner's IN.11 rulings of
//! 2026-11-05 added the last and `scrapped`; the durable matter ledger stays parked). Each commission also prints one `tacsim.matter_build` line: the inputs
//! (the bill's bulk, its parts and its core) against the hull's dry mass and the bill's declared
//! waste, which holds `inputs = hull + waste` at every build. The chain's goods are the ores,
//! the refined goods and the parts; volatiles, propellant, provisions and munitions are not
//! counted (they burn and fire, outside the chain).

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
    /// The mills' ore less their product.
    pub milled: u64,
    /// The parts commissions consumed into hulls.
    pub built: u64,
    /// The parts the bay fitted into hulls.
    pub fitted: u64,
    /// The bale kilograms repairs consumed into hulls.
    pub repaired: u64,
    /// The alloys remains scrap minted on sites.
    pub scrapped: u64,
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
        i128::from(self.stock) + i128::from(self.opened) + i128::from(self.scrapped)
            - i128::from(self.in_ground)
            - i128::from(self.pools)
            - i128::from(self.lots)
            - i128::from(self.bales)
            - i128::from(self.worked)
            - i128::from(self.milled)
            - i128::from(self.built)
            - i128::from(self.fitted)
            - i128::from(self.repaired)
    }

    /// **The census line**: `tacsim.matter_census mined= opened= in_ground= stock= pools= lots=
    /// bales= worked= milled= built= fitted= repaired= scrapped= balance=`, kilograms.
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "{CENSUS_LINE} mined={} opened={} in_ground={} stock={} pools={} lots={} bales={} \
             worked={} milled={} built={} fitted={} repaired={} scrapped={} balance={}",
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
    let counted =
        |good: &CanonicalId| CHAIN_BULK.contains(&good.as_str()) || crate::port_tick::is_part(good);
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
    scan(ctx, &industry.deposits, |deposit| {
        if counted(deposit.commodity()) {
            ground.insert(deposit.endpoint());
            deposits.push((deposit.endpoint(), deposit.commodity().clone()));
            out.opened = out
                .opened
                .saturating_add(kg(deposit.commodity(), deposit.initial_reserve()));
        }
    });
    for (endpoint, good) in &deposits {
        let units = crate::port_tick::read_pool(ctx, *endpoint, good).unwrap_or(0);
        out.in_ground = out.in_ground.saturating_add(kg(good, units));
    }
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
        // A mill of the chain's goods only (Occator's ice mill is outside it).
        if !counted(&ledger.ore) {
            return;
        }
        let ore = kg(&ledger.ore, ledger.ore_t);
        let product = kg(&ledger.into, ledger.product_t);
        out.milled = out.milled.saturating_add(ore.saturating_sub(product));
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
        let inputs = bulk
            .saturating_add(parts)
            .saturating_add(bill.budget.core_kg);
        out.builds.push((
            record.order,
            inputs,
            bill.budget.dry_mass_kg,
            bill.waste_kg(),
        ));
    }
    Ok(out)
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
