//! **`tacsim:baywork:{hull}`** — one hull's bay record (doc 36 cut GC.8): where it is berthed
//! from, and its one open or last repair order. A bounded whole document on the normal lane,
//! written through `crate::store::put_bay_hull` (the `TvLease` pattern: RAM, then one
//! `JsonSet`); an unreadable one holds its hull (no berth, stow or order) until staff repair it.

use mush_core::ObjId;
use mush_sim::SlotId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use super::jobs::{FacilityClass, QuotedJob};

/// The prefix of every bay record.
pub const BAY_PREFIX: &str = "tacsim:baywork:";
/// [`BayHull::schema`].
pub const BAY_SCHEMA: u16 = 1;
/// The most jobs one order carries.
pub const MAX_ORDER_JOBS: usize = 64;
/// The most lots one hull's record lists under pending salvage claims (doc 41 §4.6).
pub const MAX_CLAIM_LOTS: usize = 16;

/// **`tacsim:baywork:{hull.0}`**.
#[must_use]
pub fn bay_key(hull: ObjId) -> SmolStr {
    SmolStr::new(format!("{BAY_PREFIX}{}", hull.0))
}

/// The inverse: the hull a key names.
#[must_use]
pub fn bay_key_owner(key: &str) -> Option<ObjId> {
    key.strip_prefix(BAY_PREFIX)
        .and_then(crate::port::families::strict_decimal_u32)
        .map(ObjId)
}

/// **Where a berthed hull came from** (Q-36-53): the carrier and its bay slot, so `stow`
/// puts it back where it was.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BerthRow {
    /// The workshop room it stands in: the facility.
    pub workshop: ObjId,
    /// The carrier it came off.
    pub carrier: ObjId,
    /// The carrier's bay room it left.
    pub bay_room: ObjId,
    /// The bay slot it left, when it had one.
    pub slot: Option<SlotId>,
    /// Who berthed it.
    pub by: ObjId,
    /// The IC second it was berthed.
    pub at_ic: i64,
}

/// **Where an order stands.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
    /// Accepted, the fee escrowed, the work running on the IC clock; the hull held.
    Open,
    /// The work was written to the hull and the fee paid to the facility.
    Done,
    /// Cancelled, or refused at completion (the hull moved or changed); the fee released.
    Cancelled,
}

/// **Who ordered the work, and on what right** (Q-36-33).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Right {
    /// The title holder.
    Title,
    /// A holder of a current `unit.maintain_refit` grant on the hull.
    Grant,
    /// The lessee of a leased rig: repair and rearm, never refit, strip or scrap.
    Lessee,
    /// A salvor holding custody under a pending claim: repair at their own risk.
    Salvor,
}

/// **One repair order** (Q-36-32, Q-36-40).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayOrder {
    /// Monotonic per hull: the record's `orders` after this one was taken.
    pub seq: u32,
    /// Who ordered and pays.
    pub requester: ObjId,
    /// On what right.
    pub right: Right,
    /// Where the work is done: the workshop room, or the carrier's bay room.
    pub facility: ObjId,
    /// The facility's class.
    pub class: FacilityClass,
    /// The jobs, as quoted.
    pub jobs: Vec<QuotedJob>,
    /// The fee, minor units: the labour (the same in either class) and the price of a part from
    /// the workshop's stores (doc 39 cut IN.6).
    pub fee_minor: i64,
    /// The fee contract, `ContractId.0`: the requester's escrow, captured by the operator at
    /// completion or cancelled. `None` for an order in a carrier's bay away from port, which
    /// files no fee (doc 41 cut RS.2, Q-41-7).
    pub contract: Option<u64>,
    /// The hold the order's goods come from (`VaultId.0`), when it draws any (doc 36 cut
    /// GC.8b): munitions for a rearm, `industrial_parts` for its points, a fitted part's lot.
    pub munitions_vault: Option<u64>,
    /// **Where the order's part lies** (doc 39 cut IN.6): the operator's storefront vault
    /// (`VaultId.0`) for a part from the workshop's stores, which the order holds from its take
    /// to its draw; `None` for a part fitted from the hold, which lies in
    /// [`BayOrder::munitions_vault`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_vault: Option<u64>,
    /// The goods the order draws from that hold at completion (GC.8b).
    #[serde(default)]
    pub goods: Vec<super::act::GoodUnits>,
    /// The ordinal of the draw this order owes at completion, once owed (GC.8b).
    #[serde(default)]
    pub draw_act: Option<u32>,
    /// **The hull's bale once this order's work is done** (doc 39 cut IN.1, §4.10): the tonnes
    /// [`BayOrder::goods`] opens or tops it up by, less the kilograms the work consumes; `None`
    /// for an order that touches no bale.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bale_after: Option<Bale>,
    /// The hull's unit revision when the order was taken: the work is refused if it moved.
    pub unit_revision: u64,
    /// The IC second the order was taken.
    pub started_ic: i64,
    /// The IC second the work is done (the IC clock pauses under a W-clock freeze).
    pub due_ic: i64,
    /// Where it stands.
    pub status: OrderStatus,
    /// When it closed, IC.
    pub closed_ic: Option<i64>,
    /// Why it was cancelled, when it was.
    pub note: Option<String>,
    /// **IC seconds left when a field on her carrier's site paused the work** (doc 41 cut RS.2,
    /// Q-41-5); `None` while it runs. Resumed, the order is due that long after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused: Option<i64>,
}

/// **One hull's bay record.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayHull {
    /// [`BAY_SCHEMA`].
    pub schema: u16,
    /// The hull: the key.
    pub hull: ObjId,
    /// Where it is berthed from, while it stands in a workshop.
    pub berth: Option<BerthRow>,
    /// The open or last order.
    pub order: Option<BayOrder>,
    /// Orders taken on this hull, ever.
    pub orders: u32,
    /// Inventory acts owed on this hull, ever: the next owed act's ordinal is one more
    /// (doc 36 cut GC.8b; the hull's `tacsim.bay_work` ledger lands them in order).
    #[serde(default)]
    pub acts: u32,
    /// The inventory act this record owes, until the hull's ledger shows it landed.
    #[serde(default)]
    pub owed: Option<Box<super::act::OwedAct>>,
    /// When the hull was scrapped, IC: it is an irrecoverable shell after it.
    #[serde(default)]
    pub scrapped_ic: Option<i64>,
    /// **The lots stripped off this hull under a pending salvage claim** (doc 41 cut RS.3,
    /// §4.6, round-2 item 41-12a), at most [`MAX_CLAIM_LOTS`]: the claims' tick marks each
    /// vacated when its claim is, and flags it hot through
    /// [`crate::port::bay_work::strip::vacate_claim`] one act at a time. Omitted while empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub claim_lots: Vec<ClaimLot>,
    /// **The hull's opened bale** (doc 39 cut IN.1, §4.10, Q-39-26), once a repair has opened
    /// one: the kilograms of each bulk good debited to it and not yet consumed. The title
    /// holder's; it rides the hull from workshop to carrier bay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bale: Option<Bale>,
}

/// **A hull's bale** (doc 39 §4.10, Q-39-26): the sub-tonne remainder of the whole tonnes a
/// repair debited from the hold, kilograms of each good, so no repair rounds matter into the
/// hull or out of it. The first repair opens it with a tonne of each good; a line that runs
/// short takes the next whole tonne.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bale {
    /// Kilograms of `structural_alloys`.
    pub alloys_kg: u64,
    /// Kilograms of `industrial_parts` (yard fittings).
    pub fittings_kg: u64,
}

/// The kilograms in one ledger tonne.
pub const KG_PER_T: u64 = 1_000;

/// **What a repair draws through the bale** (doc 39 §4.10): the whole tonnes of each good
/// debited from the hold, and the bale after the work's kilograms come out of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BaleDraw {
    /// Tonnes of `structural_alloys` debited into the bale.
    pub alloys_t: u64,
    /// Tonnes of `industrial_parts` debited into the bale.
    pub fittings_t: u64,
    /// The bale once the work has consumed `need`.
    pub after: Bale,
}

impl Bale {
    /// **The draw `need` makes on `bale`** (doc 39 §4.10): `None` when the work needs no
    /// kilograms. An unopened bale opens with one tonne of each good; then each line takes
    /// whole tonnes until it covers its kilograms, which the work consumes. Up for the hold,
    /// exact for the hull: nothing rounds matter in or out.
    #[must_use]
    pub fn draw(bale: Option<Bale>, need: Bale) -> Option<BaleDraw> {
        if need == Bale::default() {
            return None;
        }
        let (mut have, open) = match bale {
            Some(bale) => (bale, 0),
            None => (
                Bale {
                    alloys_kg: KG_PER_T,
                    fittings_kg: KG_PER_T,
                },
                1,
            ),
        };
        let top_up = |have: u64, need: u64| need.saturating_sub(have).div_ceil(KG_PER_T);
        let alloys_t = top_up(have.alloys_kg, need.alloys_kg);
        let fittings_t = top_up(have.fittings_kg, need.fittings_kg);
        have.alloys_kg = have
            .alloys_kg
            .saturating_add(alloys_t.saturating_mul(KG_PER_T))
            .saturating_sub(need.alloys_kg);
        have.fittings_kg = have
            .fittings_kg
            .saturating_add(fittings_t.saturating_mul(KG_PER_T))
            .saturating_sub(need.fittings_kg);
        Some(BaleDraw {
            alloys_t: alloys_t.saturating_add(open),
            fittings_t: fittings_t.saturating_add(open),
            after: have,
        })
    }
}

/// **A lot stripped under a pending salvage claim** (doc 41 §4.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClaimLot {
    /// The lot, `LotId.0`.
    pub lot: u64,
    /// The claim it came off under: the filed contract, `ContractId.0`.
    pub claim: u64,
    /// Once the claim is vacated: who may clear the flag, and whose hand submits it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vacated: Option<VacatedLot>,
}

/// **A listed lot's claim was vacated** (doc 41 §4.6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VacatedLot {
    /// The title holder the claim never moved the title from.
    pub rightful_owner: ObjId,
    /// The hand that submits the flag (a character, never GOD).
    pub by: ObjId,
}

impl BayHull {
    /// A hull with no record yet.
    #[must_use]
    pub const fn new(hull: ObjId) -> Self {
        Self {
            schema: BAY_SCHEMA,
            hull,
            berth: None,
            order: None,
            orders: 0,
            acts: 0,
            owed: None,
            scrapped_ic: None,
            claim_lots: Vec::new(),
            bale: None,
        }
    }

    /// The open order, if any: the hull is held while it stands.
    #[must_use]
    pub fn open(&self) -> Option<&BayOrder> {
        self.order
            .as_ref()
            .filter(|order| order.status == OrderStatus::Open)
    }

    /// Bounds and shape.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != BAY_SCHEMA {
            return Err(format!("schema {} is not {BAY_SCHEMA}", self.schema));
        }
        if let Some(order) = &self.order {
            if order.jobs.is_empty() || order.jobs.len() > MAX_ORDER_JOBS {
                return Err("an order holds 1 to 64 jobs".to_owned());
            }
            if order.fee_minor < 0 || order.due_ic < order.started_ic || order.seq == 0 {
                return Err("an order's fee, times or sequence are out of bounds".to_owned());
            }
            if order.seq > self.orders {
                return Err("an order's sequence runs ahead of the record's".to_owned());
            }
            if order.draw_act.is_some_and(|act| act > self.acts) {
                return Err("an order's draw runs ahead of the record's acts".to_owned());
            }
        }
        if self.claim_lots.len() > MAX_CLAIM_LOTS {
            return Err(format!(
                "a hull lists at most {MAX_CLAIM_LOTS} lots under claims"
            ));
        }
        if let Some(owed) = &self.owed {
            owed.check()?;
            if owed.act > self.acts {
                return Err("an owed act runs ahead of the record's acts".to_owned());
            }
        }
        Ok(())
    }
}

/// A bay record as the boot read it.
// One per hull that has seen the bay, in RAM; the rare `Unreadable` costs a record's size.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BayHullState {
    /// It read.
    Present(BayHull),
    /// Its key claims the hull but its body did not read: the hull is held.
    Unreadable,
}
