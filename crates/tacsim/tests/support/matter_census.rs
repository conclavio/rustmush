//! **`tacsim.matter_census`** (doc 39 cut IN.11, §4.10; Q-39-8): the chain's conservation, to the
//! kilogram, read off durable state.
//!
//! **The rule**: Σ mined = pools + lots + hull bales + shop bales + declared waste. Mining is a
//! transfer out of a deposit's vault, never a mint, so *mined* is the pocket cursors' drawn
//! tonnes (`tacsim.pockets`, `PocketCursor::drawn_total`). The pools and the lots are the
//! inventory families' (a pool's commodity from its reservation book, a lot's its own), each in
//! kilograms by the goods table. No loss is stored: the story declares each one from its ledger
//! or its bill (the mill's `ore_t − product_t`, a recipe's inputs less its outputs, a bill's
//! assembly loss and rounding) and passes them in.
//!
//! [`MatterCensus::line`] is the census line other stories read (doc 41's RS.9 among them):
//! `tacsim.matter_census mined={kg} pools={kg} lots={kg} hull_bales={kg} shop_bales={kg}
//! waste={kg} balance={kg}`, `balance` the mined less everything else (zero when it holds).

use std::collections::BTreeMap;

use mush_core::CanonicalId;
use mush_testkit::TestWorld;

/// **One census**: kilograms by term, and the goods it counted, by commodity.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MatterCensus {
    /// The pocket cursors' drawn tonnes, kilograms.
    pub mined_kg: u64,
    /// Every pool of a counted good, kilograms.
    pub pools_kg: u64,
    /// Every lot of a counted good, kilograms.
    pub lots_kg: u64,
    /// The hulls' opened bales, kilograms.
    pub hull_bales_kg: u64,
    /// The workshops' shop bales, kilograms.
    pub shop_bales_kg: u64,
    /// The declared losses, kilograms.
    pub waste_kg: u64,
    /// The pools and lots by commodity, kilograms (the story's diagnostics).
    pub by_good: BTreeMap<CanonicalId, u64>,
}

impl MatterCensus {
    /// Mined less everything else, kilograms (negative when matter appeared).
    #[must_use]
    pub fn balance_kg(&self) -> i128 {
        i128::from(self.mined_kg)
            - i128::from(self.pools_kg)
            - i128::from(self.lots_kg)
            - i128::from(self.hull_bales_kg)
            - i128::from(self.shop_bales_kg)
            - i128::from(self.waste_kg)
    }

    /// **The census line.**
    #[must_use]
    pub fn line(&self) -> String {
        format!(
            "tacsim.matter_census mined={} pools={} lots={} hull_bales={} shop_bales={} waste={} \
             balance={}",
            self.mined_kg,
            self.pools_kg,
            self.lots_kg,
            self.hull_bales_kg,
            self.shop_bales_kg,
            self.waste_kg,
            self.balance_kg()
        )
    }
}

/// Every page of `handle`, each value handed to `each`.
fn scan<K, V>(tw: &TestWorld, handle: &mush_core::StateFamilyHandle<K, V>, mut each: impl FnMut(&V))
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    let state = tw.engine().action_state();
    let mut cursor: Option<String> = None;
    loop {
        let page = state
            .scan_family(handle, cursor.as_deref(), 256)
            .expect("the family reads");
        for entry in page.entries() {
            each(entry.value());
        }
        match page.next_after().map(ToString::to_string) {
            Some(next) => cursor = Some(next),
            None => break,
        }
    }
}

/// **Take the census** over the goods `counted` (the chain's: its ores, concentrates, alloys,
/// fittings, parts and kits), with the story's declared `waste_kg`.
///
/// # Panics
///
/// When the world's families or goods table cannot be read.
#[must_use]
pub fn census(tw: &TestWorld, counted: &[CanonicalId], waste_kg: u64) -> MatterCensus {
    let store = tw
        .world()
        .module_state::<tacsim::store::TacsimStore>()
        .expect("the tacsim store");
    let goods = store.goods().expect("the goods table");
    let kg = |good: &CanonicalId, units: u64| -> u64 {
        goods
            .good(good)
            .and_then(|def| def.mass_kg_per_unit())
            .map_or(0, |per| per.saturating_mul(units))
    };
    let handles = tw
        .world()
        .module_state::<tacsim::port::PortHandles>()
        .expect("the port handles")
        .clone();
    let inventory = handles.inventory.clone().expect("the inventory families");
    let mut out = MatterCensus {
        waste_kg,
        ..MatterCensus::default()
    };
    // Mined: the pocket cursors' drawn tonnes.
    scan(tw, &handles.families.pockets, |cursor| {
        out.mined_kg = out
            .mined_kg
            .saturating_add(u64::from(cursor.drawn_total().unwrap_or(0)).saturating_mul(1_000));
    });
    // Pools: each reservation book's commodity, then its pool's units.
    let mut books: BTreeMap<mush_core::ReservationBookId, CanonicalId> = BTreeMap::new();
    scan(tw, &inventory.books, |book| {
        let good = book.resource().id.clone();
        if counted.contains(&good) {
            books.insert(book.id(), good);
        }
    });
    let state = tw.engine().action_state();
    for (id, good) in &books {
        let units = state
            .get(&inventory.pools, id)
            .ok()
            .flatten()
            .map_or(0, |scalar| u64::try_from(scalar.current()).unwrap_or(0));
        let mass = kg(good, units);
        out.pools_kg = out.pools_kg.saturating_add(mass);
        *out.by_good.entry(good.clone()).or_insert(0) += mass;
    }
    // Lots.
    scan(tw, &inventory.lots, |lot| {
        if counted.contains(lot.commodity()) {
            let mass = kg(lot.commodity(), lot.quantity());
            out.lots_kg = out.lots_kg.saturating_add(mass);
            *out.by_good.entry(lot.commodity().clone()).or_insert(0) += mass;
        }
    });
    // The hulls' bales.
    for hull in store.bay_hull_ids() {
        if let Some(tacsim::port::bay_work::BayHullState::Present(row)) = store.bay_hull(hull)
            && let Some(bale) = row.bale.as_ref()
        {
            out.hull_bales_kg = out
                .hull_bales_kg
                .saturating_add(bale.alloys_kg)
                .saturating_add(bale.fittings_kg);
        }
    }
    out
}
