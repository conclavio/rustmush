//! **The boards** (doc 18 cut IC8; doc 18 §4.3 "Stock and capital"; 18a C07, C08,
//! P07, P08, P09 and P11 Items; lead ruling L15).
//!
//! The rooms 18a puts a board in show, on `look`, what is left: each ask's remaining
//! stock and price, `SOLD OUT` once the last tonne is drawn, and each standing bid's
//! price and intake still wanted, or "not buying" when the bidder cannot fund a
//! unit. Which room shows which line is content: each ask and bid in lane v3 names
//! the room roles whose board carries it (`boards`), and the applier records each
//! role's room in the port map, so this hook finds its rooms in the map and never
//! scans the atlas at look time. `SOLD OUT` reads the port map and the stock
//! receipts, which the market cannot see — which is why the board is the game's and
//! `+market/list` stays the market's own render.
//!
//! The board prints no recommended mix. P07's "funded balance" figure (18a P07 Items;
//! IC8 L15 deferred it to cut IC10) is deferred again, to IC12's readiness/board pass
//! (doc 18 cut IC10, lead 2h-v). Cut IC10 adds only the exhausted bid's "not buying".
//! Nothing renders for any other room, nor while the store is absent or the layer is
//! disabled; nothing is written.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{CanonicalId, EventHook, HookPoint, HookReadContext, ObjId, World};
use mush_economy::{Amount, EconomyStore, InstrumentId, VaultId};
use mush_inventory::record::{EndpointRef, LotId};
use mush_market::{ListingId, MarketStore};
use mush_proto::RichText;

use crate::definitions::GoodsTable;
use crate::definitions::content::LANE_PACK;
use crate::economy::CREDIT;
use crate::port::def::{BidDef, PortDef};
use crate::port::families::{PortHandles, StockIssueId};
use crate::port::map::{PortMap, VendorRow};
use crate::port::readiness::report_ports;
use crate::port::stock::{ask_reading, receipt_in};
use crate::port_tick::read_pool_in;
use crate::store::TacsimStore;

/// The line the board block opens with.
pub const BOARD_HEADING: &str = "The board:";

/// **Thousands grouped with commas** (`9,600`; doc 18 §4.3 writes "seeded 9,600 t").
#[must_use]
pub fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len().saturating_add(digits.len() / 3));
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && digits.len().saturating_sub(i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The word a quantity of `good` is counted in: `t`, `crew-days`, else the ledger
/// unit.
pub(crate) fn unit_of(goods: &GoodsTable, good: &CanonicalId) -> String {
    match goods.good(good).map(|row| row.ledger_unit.as_str()) {
        Some("t") => "t".to_owned(),
        Some("crew_day") => "crew-days".to_owned(),
        Some(other) => other.to_owned(),
        None => String::new(),
    }
}

/// A good's label, else its id.
pub(crate) fn label_of(goods: &GoodsTable, good: &CanonicalId) -> String {
    goods
        .good(good)
        .map_or_else(|| good.to_string(), |row| row.label.clone())
}

/// A price in the credit's render (`200.00cr`).
pub(crate) fn money(w: &World, amount: Amount) -> String {
    match (w.module_state::<EconomyStore>(), InstrumentId::new(CREDIT)) {
        (Some(economy), Some(instrument)) => {
            mush_economy::render_amount(economy.snapshot(), &instrument, amount)
        }
        _ => format!("{}", amount.minor()),
    }
}

/// An object's name, or nothing.
fn name_of(w: &World, obj: u32) -> String {
    w.get(ObjId(obj))
        .map_or_else(String::new, |o| o.name.clone())
}

/// **The name a house trades under** (doc 18 cut IC8's board, factored for cut IC10's
/// sale line): the bid's `as` name when it has one, else the org's authored name, else
/// the org id.
pub(crate) fn house_name(def: &PortDef, org: &CanonicalId, bid: Option<&BidDef>) -> String {
    bid.and_then(|b| b.as_name.clone()).unwrap_or_else(|| {
        def.orgs
            .iter()
            .find(|o| &o.id == org)
            .map_or_else(|| org.to_string(), |o| o.name.clone())
    })
}

/// Whether the vendor behind `listing` can fund one unit at `price` from its own
/// account. Unreadable reads as not.
pub(crate) fn funds_one(w: &World, market: &MarketStore, listing: ListingId) -> bool {
    let Some(listing) = market.listing(listing) else {
        return false;
    };
    let Some(vendor) = market.vendor(listing.vendor) else {
        return false;
    };
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(vendor.account).cloned())
        .is_some_and(|a| {
            a.posted_balance
                .minor()
                .saturating_sub(a.held_amount.minor())
                >= listing.price.minor()
        })
}

/// What a vendor's stock rows for one good read as: `Some((seeded, receipted qty))`
/// when every row's receipt reads (seeded = every receipt present and its lot gone),
/// `None` when anything is unreadable.
fn rows_seeded(
    hook: &HookReadContext<'_>,
    handles: &PortHandles,
    row: &VendorRow,
    good: &CanonicalId,
) -> Option<(bool, u64)> {
    let inventory = handles.inventory.as_ref()?;
    let mut seeded = true;
    let mut total: u64 = 0;
    for stock in row.stock.iter().filter(|s| &s.good == good) {
        let id = StockIssueId::parse(&stock.id)?;
        match receipt_in(hook, &handles.families.port_stock, &id).ok()? {
            None => seeded = false,
            Some(receipt) => {
                total = total.saturating_add(receipt.qty);
                if hook
                    .action_state(&inventory.lots, &LotId(receipt.lot))
                    .ok()?
                    .is_some()
                {
                    seeded = false;
                }
            }
        }
    }
    Some((seeded, total))
}

/// **The board lines one room shows** (lead ruling L15): every ask, then every bid, of
/// every vendor in authored order whose `boards` names one of `roles`.
#[allow(clippy::too_many_arguments)]
fn port_lines(
    hook: &HookReadContext<'_>,
    goods: &GoodsTable,
    handles: &PortHandles,
    market: &MarketStore,
    def: &PortDef,
    map: &PortMap,
    roles: &[CanonicalId],
    out: &mut Vec<String>,
) {
    let w = hook.world();
    let house_of = |org: &CanonicalId| house_name(def, org, None);
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        let house = house_of(&vendor.org);
        let signatory = name_of(w, row.signatory);
        for ask in vendor
            .asks
            .iter()
            .filter(|a| a.boards.iter().any(|b| roles.contains(b)))
        {
            let label = label_of(goods, &ask.good);
            let unit = unit_of(goods, &ask.good);
            let reading = ask_reading(market, row.vault, &ask.good);
            let line = match reading.listing {
                Some((id, quantity)) => {
                    let price = market
                        .listing(id)
                        .map_or_else(String::new, |l| money(w, l.price));
                    format!(
                        "  {label}: {price}/{unit}, {} {unit} left ({house}; signed {signatory}).",
                        group_thousands(quantity)
                    )
                }
                None => {
                    let pool = read_pool_in(hook, EndpointRef(VaultId(row.vault)), &ask.good).ok();
                    match (rows_seeded(hook, handles, row, &ask.good), pool) {
                        (Some((true, seeded)), Some(pool)) if pool <= reading.undrawn => format!(
                            "  {label}: SOLD OUT: seeded {} {unit}, 0 left ({house}).",
                            group_thousands(seeded)
                        ),
                        _ => format!("  {label}: not yet on the board ({house})."),
                    }
                }
            };
            out.push(line);
        }
    }
    for vendor in &def.vendors {
        let Some(row) = map.vendors.get(&vendor.org) else {
            continue;
        };
        let signatory = name_of(w, row.signatory);
        for bid in vendor
            .bids
            .iter()
            .filter(|b| b.boards.iter().any(|r| roles.contains(r)))
        {
            let label = label_of(goods, &bid.good);
            let unit = unit_of(goods, &bid.good);
            let house = house_name(def, &vendor.org, Some(bid));
            let live = row
                .bids
                .get(&bid.good)
                .map(|b| ListingId(b.listing))
                .and_then(|id| market.listing(id).filter(|l| l.source.is_wanted()));
            // Doc 18 cut IC10: an exhausted bid stands at quantity 0 and is not buying.
            let line = match live {
                Some(listing) if listing.quantity > 0 && funds_one(w, market, listing.id) => {
                    format!(
                        "  wanted, {label}: {}/{unit}, {} {unit} more ({house}; signed {signatory}).",
                        money(w, listing.price),
                        group_thousands(listing.quantity)
                    )
                }
                _ => format!("  wanted, {label}: not buying ({house})."),
            };
            out.push(line);
        }
    }
}

/// **The board hook** (doc 18 cut IC8; lead ruling L15).
#[derive(Debug)]
pub struct BoardLook;

impl EventHook for BoardLook {
    fn look_rendered(
        &self,
        ctx: &HookReadContext<'_>,
        _looker: ObjId,
        room: ObjId,
    ) -> Vec<RichText> {
        let w = ctx.world();
        let Some(store) = w.module_state::<TacsimStore>() else {
            return Vec::new();
        };
        if !store.enabled() {
            return Vec::new();
        }
        let (Some(market), Some(handles), Some(goods)) = (
            w.module_state::<MarketStore>(),
            w.module_state::<PortHandles>(),
            store.goods(),
        ) else {
            return Vec::new();
        };
        let (_, ports) = report_ports(store, w);
        let mut lines: Vec<String> = Vec::new();
        for map in store
            .port_maps()
            .filter(|map| map.pack.as_str() == LANE_PACK)
        {
            let roles: Vec<CanonicalId> = map
                .boards
                .iter()
                .filter(|(_, obj)| **obj == room.0)
                .map(|(role, _)| role.clone())
                .collect();
            if roles.is_empty() {
                continue;
            }
            let Some((_, def)) = ports.iter().find(|(entry, _)| *entry == map.entry) else {
                continue;
            };
            port_lines(ctx, goods, handles, market, def, map, &roles, &mut lines);
        }
        if lines.is_empty() {
            return Vec::new();
        }
        std::iter::once(BOARD_HEADING.to_owned())
            .chain(lines)
            .map(|line| RichText::new().text(line))
            .collect()
    }
}

/// Register [`BoardLook`] on the `LookRendered` point.
///
/// # Errors
///
/// Never today; the `Result` is the registrar's shape.
pub fn register_hooks(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.hook(HookPoint::LookRendered, Arc::new(BoardLook));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ic8_thousands_are_grouped() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(30), "30");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(9_600), "9,600");
        assert_eq!(group_thousands(4_000_000), "4,000,000");
    }
}
