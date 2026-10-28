//! **The port's typed families** (doc 18 cut IC5; §4.2 "the ticket", §4.3
//! "Finite seeded stock", §4.8 "the seams", D-18-11, D-18-33, D-18-46).
//!
//! Nine production families, all registered unconditionally (a family is a fact
//! about the composition, and hydrating a later cut's records must never depend
//! on a flag):
//!
//! - **`tacsim.port_stock`** — one [`StockReceipt`] per stock mint, keyed by its
//!   [`StockIssueId`] (§4.3: *the durable replay guard of one mint; idempotency
//!   keys do not survive a reboot*). Written in the mint's own bundle by IC8's
//!   `tacsim_port_stock`.
//! - **`tacsim.ticket`** — one [`Ticket`] per burn, keyed by its [`TicketId`]
//!   (D-18-11 as extended by D-18-46). Written in the burn's own bundle by IC9's
//!   `tacsim_port_burn` (`crate::port::burn`) and read by the launch arm
//!   (`crate::port::voyage`).
//! - **`tacsim.excursion`** (doc 20 cut SM.3b; §8, D-20-11) — one
//!   [`crate::surface::excursion::RigRecord`] per rig, keyed by its terrain vehicle
//!   ([`crate::surface::excursion::RigKey`]): the act in flight with the charge it
//!   reserved, the open excursion, the lazy charge and the last finished act. Written
//!   in the start's and the finish's own bundles (`tacsim_surface_act`,
//!   `tacsim_surface_done`, `crate::surface::act`) and read by the `tacsim.surface`
//!   tick, `+tv` and the departure's bay gate.
//! - **`tacsim.chart`** (doc 20 cut SM.4a; §5, D-20-8) — one
//!   [`crate::surface::chart::ChartRecord`] per (character, site), keyed by
//!   [`crate::surface::chart::ChartKey`]: the operator's coverage and provenance, never a
//!   reading. Written in the finish's bundle of a scan or a sounding
//!   (`tacsim_surface_done`) and by `+tv/forget`'s own action (`tacsim_chart_forget`), its
//!   record cap tightened to 64 KiB.
//! - **`tacsim.pockets`** (doc 20 cut SM.6; §3.4, D-20-5) — one
//!   [`crate::surface::cursor::PocketCursor`] per surface deposit, keyed by the deposit's
//!   id: what the surface provider drew, and the lock's one home.
//! - **`tacsim.claims`** (doc 20 cut SM.10; §9) — one
//!   [`crate::surface::claims::BlockTenure`] per (site, block), keyed by
//!   [`crate::surface::claims::BlockKey`], staged inside industry's own tenure bundle.
//! - **`tacsim.relay_fabrications`** (relay cut R9a; doc 15a) — one bounded book of
//!   fabrication receipts per relay foundry, keyed by the foundry's object.
//! - **`tacsim.port_mill`** (doc 20 cut SM.12a; §7.3 "The mill") — one
//!   [`crate::port::mill::MillLedger`] per mill, keyed by its
//!   [`crate::port::mill::MillId`]: the mill's totals, its IC day's tally and its last
//!   sixteen batches. Written in the batch's own bundle by `tacsim_port_mill`
//!   (`crate::port::mill`), and read by the restock, the stock arm's list step and the
//!   readiness report.
//! - **`tacsim.dispatch`** (doc 20 cut SM.12b; §12.3 "Occator's drones") — one
//!   [`crate::port::yard::DispatchLedger`] per yard, keyed by its site
//!   ([`crate::port::yard::DispatchKey`]): the yard's delivery totals, its IC day's tally and its
//!   last sixteen deliveries. Written in the delivery's own bundle by `tacsim_dispatch`
//!   (`crate::port::yard`), and read by the dispatcher's step and the readiness report.
//!
//! The ticket carries the whole [`PassagePlan`] of §4.8, **declared here as
//! data** (lead ruling L6): IC9's `port/passage.rs` writes it, but the ticket's
//! wire must exist first (D-18-46). Every record is strict
//! (`deny_unknown_fields`) and carries a `check` its family's validator calls,
//! so a hand-edited **value** fails at both gates (the
//! `mush-inventory/src/module.rs` rule). The value validator cannot see the
//! key, so a value stored under another record's key passes it (doc 18 cut
//! IC8, lead 2c-iii): **writers derive the key from the value** (the stock
//! provider re-derives the [`StockIssueId`] it inserts under), and **readers
//! re-check it** (`receipt.issue_id()? == key`; a mismatch reads as
//! unreadable, fail closed, `crate::port::stock::receipt_at`).
//!
//! These are tacsim's first typed families. IC5 landed the declarations, the
//! handles ([`PortHandles`], published by `crate::port_tick::register`) and the
//! probe that proves the lane; IC8's `tacsim_port_stock`
//! (`crate::port::stock`) is the first production writer.

use std::collections::BTreeMap;

use mush_core::{
    CanonicalHash, CanonicalId, DefinitionDigest, ModError, ObjId, Registrar,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The stock-issue family (§4.3; lead ruling L5: doc 18's name verbatim).
pub const PORT_STOCK_FAMILY: &str = "tacsim.port_stock";
/// The departure-ticket family (D-18-11, D-18-46; lead ruling L5).
pub const TICKET_FAMILY: &str = "tacsim.ticket";
/// Most distinct goods one manifest records (the slice carries six).
pub const MAX_MANIFEST_GOODS: usize = 16;
/// Most occupants one ticket records. The burn refuses with no pilot aboard
/// (§4.4 "Failure handling"), so the floor is one.
pub const MAX_TICKET_OCCUPANTS: u32 = 16;
/// The stock-issue identity's hash domain (§4.3).
const STOCK_ISSUE_DOMAIN: &str = "tacsim.stock_issue_id.v1";
/// The canonical-bytes ceiling of one stock-issue identity's input.
const STOCK_ISSUE_MAX_BYTES: usize = 1024;

// ---- §4.8: the passage plan ------------------------------------------------

/// §4.8, D-18-46: one leg of a planned passage. `to` is the place the leg ends at, `body` the
/// body that place is on; `secs` the leg's laden seconds; `t0`/`t1` its IC instants.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegPlan {
    /// The place the leg ends at.
    pub to: CanonicalId,
    /// The body that place is on.
    pub body: CanonicalId,
    /// The leg's distance at departure, km (a same-body leg may be 0).
    pub dist_km: u64,
    /// The leg's laden seconds, `> 0`.
    pub secs: u32,
    /// The IC second the leg begins.
    pub t0: i64,
    /// The IC second the leg ends: `t0 + secs`.
    pub t1: i64,
    /// **Doc 20 cut SM.2 (§3.3, D-20-3)** — the ascent Δv this leg pays, m/s:
    /// √(g·R) of the site's body at each end of the leg that is a site (a
    /// [`crate::pack::SITE_ROLE`] location), summed; 0 for a leg with no site end.
    /// A port end (Ceres Freeport, every station) pays none. Frozen with the plan
    /// (T1), so the burn's prepare recomputes the same tonnes from the ticket.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub ascent_mps: u32,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

/// §4.8, D-18-44/46: what `plan_passage` (cut IC9, `port/passage.rs`) returns and what the
/// burn ticket freezes. Declared here as data so the ticket's wire is fixed before its writer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PassagePlan {
    /// The route the plan follows.
    pub route: CanonicalId,
    /// The drive law the plan was computed under (§4.8's tagged law).
    pub law: CanonicalId,
    /// The IC second of departure; the first leg's `t0`.
    pub depart_ic: i64,
    /// The sky revision the plan was computed against (D-18-43).
    pub sky_rev: DefinitionDigest,
    /// The legs, contiguous, `1..=mush_sim::MAX_LEGS`.
    pub legs: Vec<LegPlan>,
    /// The whole passage's seconds: the last leg's `t1 - depart_ic`.
    pub total_s: i64,
    /// The propellant the burn debits, t.
    pub burn_t: u64,
    /// The provisions the burn debits, crew-days.
    pub crew_days: u32,
    /// **Doc 19 cut A.2 (D-19-9)** — the throttle the plan was flown at, permille
    /// of the hull's thrust; a plan from before the cut reads full thrust.
    #[serde(default = "full_throttle")]
    pub throttle_permille: u32,
    /// **Doc 20 cut SM.2 (§3.3 as amended, Q-20-7; the SM.2 brief's lead ruling
    /// 5)** — the crew-days of the stay on a site charged at this lift, included in
    /// `crew_days`; 0 for every other passage, and then absent on the wire.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub landed_crew_days: u32,
}

/// The throttle a plan without one was flown at: full thrust.
fn full_throttle() -> u32 {
    1_000
}

impl PassagePlan {
    /// The plan's own invariants (§4.8): `1..=MAX_LEGS` legs, each with
    /// `secs > 0` and `t1 == t0 + secs`; the first leg starts at `depart_ic`,
    /// each later one where the previous ended; `total_s` is the last leg's end
    /// less `depart_ic`, and positive. `dist_km`, `burn_t` and `crew_days` are
    /// unconstrained.
    ///
    /// # Errors
    ///
    /// A message naming the first invariant the plan breaks.
    pub fn check(&self) -> Result<(), String> {
        if self.legs.is_empty() || self.legs.len() > mush_sim::MAX_LEGS {
            return Err(format!(
                "a passage plan has 1 to {} legs, not {}",
                mush_sim::MAX_LEGS,
                self.legs.len()
            ));
        }
        let (Some(first), Some(last)) = (self.legs.first(), self.legs.last()) else {
            return Err("a passage plan has at least one leg".to_owned());
        };
        if first.t0 != self.depart_ic {
            return Err("a passage plan's first leg begins at its departure".to_owned());
        }
        for leg in &self.legs {
            if leg.secs == 0 {
                return Err("every leg of a passage plan takes some time".to_owned());
            }
            if leg.t0.checked_add(i64::from(leg.secs)) != Some(leg.t1) {
                return Err("a leg of a passage plan ends secs after it begins".to_owned());
            }
        }
        for pair in self.legs.windows(2) {
            if let [before, after] = pair
                && after.t0 != before.t1
            {
                return Err("each leg of a passage plan begins where the last one ended".to_owned());
            }
        }
        // Doc 20 cut SM.2: the stay's charge is part of the crew-days, never more.
        if self.landed_crew_days > self.crew_days {
            return Err("a passage plan's landed crew-days are part of its crew-days".to_owned());
        }
        match last.t1.checked_sub(self.depart_ic) {
            Some(total) if total == self.total_s && total > 0 => Ok(()),
            _ => Err("a passage plan's total is its last leg's end less its departure".to_owned()),
        }
    }
}

// ---- D-18-11: the manifest and the ticket ------------------------------------

/// D-18-11: what the hull carried when it burned, BEFORE the burn's own debit (the plan's burn_t
/// and crew_days are that debit, so the after-image is derivable and nothing is stored twice).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// Good → quantity in the good's unit (tonnes); zero goods omitted.
    pub cargo: BTreeMap<CanonicalId, u64>,
    /// Propellant, t.
    pub tank: u64,
    /// Provisions, crew-days.
    pub locker: u64,
    /// Souls aboard, `1..=MAX_TICKET_OCCUPANTS`.
    pub occupants: u32,
}

impl Manifest {
    /// At most [`MAX_MANIFEST_GOODS`] goods, none of them zero, and
    /// `1..=`[`MAX_TICKET_OCCUPANTS`] occupants.
    ///
    /// # Errors
    ///
    /// A message naming the first bound the manifest breaks.
    pub fn check(&self) -> Result<(), String> {
        if self.cargo.len() > MAX_MANIFEST_GOODS {
            return Err(format!(
                "a manifest records at most {MAX_MANIFEST_GOODS} goods, not {}",
                self.cargo.len()
            ));
        }
        if self.cargo.values().any(|quantity| *quantity == 0) {
            return Err("a manifest omits a good it carries none of".to_owned());
        }
        if self.occupants == 0 || self.occupants > MAX_TICKET_OCCUPANTS {
            return Err(format!(
                "a manifest records 1 to {MAX_TICKET_OCCUPANTS} occupants, not {}",
                self.occupants
            ));
        }
        Ok(())
    }
}

/// D-18-11 as extended by D-18-46 (§4.2's ticket): the departure ticket, written by
/// `tacsim_port_burn` (IC9, `crate::port::burn`) in the burn's own bundle and read by the
/// launch arm.
///
/// **Doc 18 cut IC9 amended the wire** (lead rulings L5/L36; no world holds a ticket yet, so
/// nothing migrates): `seq` was added so the key is derived from the value
/// ([`Ticket::ticket_id`], the IC8 key rule), and `launched` was removed — a typed family is
/// written only in a prepare and the launch is a mush-sim placement write, so no writer could
/// ever set it; launched-ness is the hull's journey present at `plan.depart_ic`, pulled.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ticket {
    /// The hull that burned.
    pub hull: ObjId,
    /// The hull's burn sequence (minted by the voyage document, `crate::port::voyage`); with
    /// `hull` it is the ticket's key.
    pub seq: u32,
    /// The route it burned for; the plan's own `route`.
    pub route: CanonicalId,
    /// The route pack, pinned with `route_version` (published versions are immutable; the
    /// lead note's "route pack and version").
    pub route_pack: CanonicalId,
    /// The route pack's version.
    pub route_version: u32,
    /// §4.2's "definition digest", read as the hull's fielded UNIT definition whose drive law
    /// the burn used (doc 18 cut IC5 lead ruling L7). The route is pinned by
    /// `route_pack` + `route_version`, not by this digest.
    pub unit_definition: DefinitionDigest,
    /// What the hull carried before the burn's debit.
    pub manifest: Manifest,
    /// The hull's payload mass at the burn, kg.
    pub payload_kg: u64,
    /// The frozen passage (§4.8, D-18-46).
    pub plan: PassagePlan,
    /// The queue's order id; `None` for a live `+ship/undock`.
    pub order: Option<u32>,
    /// The attempt this burn is, `>= 1`.
    pub attempt: u32,
    /// The IC second of the burn.
    pub burned_at: i64,
}

impl Ticket {
    /// The ticket's key, derived from the value (doc 18 cut IC9; the IC8 rule: writers derive
    /// the key from the value, readers re-check it).
    #[must_use]
    pub fn ticket_id(&self) -> TicketId {
        TicketId {
            hull: self.hull,
            seq: self.seq,
        }
    }

    /// The plan's and the manifest's own checks; the plan follows the ticket's
    /// route; `attempt >= 1`; and the burn debits no more than the hull held
    /// (`burn_t <= tank`, `crew_days <= locker`), so a plan that could not have
    /// come from the burn never hydrates.
    ///
    /// # Errors
    ///
    /// A message naming the first invariant the ticket breaks.
    pub fn check(&self) -> Result<(), String> {
        self.plan.check()?;
        self.manifest.check()?;
        if self.plan.route != self.route {
            return Err("a ticket's plan follows the ticket's own route".to_owned());
        }
        if self.attempt == 0 {
            return Err("a ticket's attempt counts from 1".to_owned());
        }
        if self.plan.burn_t > self.manifest.tank {
            return Err("a ticket's burn debits no more propellant than the tank held".to_owned());
        }
        if u64::from(self.plan.crew_days) > self.manifest.locker {
            return Err(
                "a ticket's burn debits no more provisions than the locker held".to_owned(),
            );
        }
        Ok(())
    }
}

/// The ticket's key: `{hull}:{seq}` in decimal (`hull.0`, never `ObjId`'s `#`-prefixed
/// Display), the spelling §4.4's orders example uses (`"ticket": "5011:9"`); `seq` is minted
/// by the hull's sailing plan (`crate::port::voyage::Voyage::last_seq`, doc 18 cut IC9).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TicketId {
    /// The hull that burned.
    pub hull: ObjId,
    /// The hull's burn sequence (minted by IC9).
    pub seq: u32,
}

impl TicketId {
    /// `{hull.0}:{seq}`.
    #[must_use]
    pub fn key_text(&self) -> String {
        format!("{}:{}", self.hull.0, self.seq)
    }

    /// The strict inverse of [`TicketId::key_text`]: exactly one `:`, both sides
    /// unsigned decimal with no leading zero but `0`, each fitting `u32`.
    #[must_use]
    pub fn parse(text: &str) -> Option<TicketId> {
        let (hull, seq) = text.split_once(':')?;
        Some(TicketId {
            hull: ObjId(strict_decimal_u32(hull)?),
            seq: strict_decimal_u32(seq)?,
        })
    }
}

/// A strict decimal `u32`: ASCII digits only (no sign, no `#`, no space), no
/// leading zero unless the whole number is `0`. The one parser behind
/// [`TicketId::parse`] and `crate::port_tick::notice_key_owner`.
pub(crate) fn strict_decimal_u32(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None;
    }
    text.parse::<u32>().ok()
}

// ---- §4.3: the stock issue ------------------------------------------------

/// §4.3: the stock-issue identity, `hash(pack, entry, org, good, issue)`; the durable replay guard
/// of one mint (idempotency keys do not survive a reboot).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StockIssueId(CanonicalHash);

impl StockIssueId {
    /// `CanonicalHash::for_canonical_value("tacsim.stock_issue_id.v1", 1, {pack, entry, org,
    /// good, issue}, 1024)`.
    ///
    /// # Errors
    ///
    /// The hash's own refusal, as text (it cannot refuse canonical ids and a `u32`).
    pub fn derive(
        pack: &CanonicalId,
        entry: &CanonicalId,
        org: &CanonicalId,
        good: &CanonicalId,
        issue: u32,
    ) -> Result<StockIssueId, String> {
        CanonicalHash::for_canonical_value(
            STOCK_ISSUE_DOMAIN,
            1,
            &json!({
                "pack": pack.as_str(),
                "entry": entry.as_str(),
                "org": org.as_str(),
                "good": good.as_str(),
                "issue": issue,
            }),
            STOCK_ISSUE_MAX_BYTES,
        )
        .map(StockIssueId)
        .map_err(|error| error.to_string())
    }

    /// The key text: 64 lowercase hexadecimal digits.
    #[must_use]
    pub fn key_text(&self) -> String {
        self.0.to_hex()
    }

    /// The strict inverse of [`StockIssueId::key_text`].
    #[must_use]
    pub fn parse(text: &str) -> Option<StockIssueId> {
        CanonicalHash::from_hex(text).ok().map(StockIssueId)
    }

    /// §4.3 "LotId derived from the issue id": the first 8 bytes, big-endian.
    #[must_use]
    pub fn lot_id(&self) -> mush_inventory::record::LotId {
        let bytes = self.0.as_bytes();
        let mut head = [0_u8; 8];
        for (slot, byte) in head.iter_mut().zip(bytes.iter()) {
            *slot = *byte;
        }
        mush_inventory::record::LotId(u64::from_be_bytes(head))
    }
}

/// §4.3: one mint's receipt, inserted in the mint's own bundle by `tacsim_port_stock` (IC8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StockReceipt {
    /// The pack the stock row was authored in.
    pub pack: CanonicalId,
    /// The port entry.
    pub entry: CanonicalId,
    /// The org whose storefront was stocked.
    pub org: CanonicalId,
    /// The good minted.
    pub good: CanonicalId,
    /// The issue number of this mint.
    pub issue: u32,
    /// The quantity minted, `1..=mush_inventory::record::MAX_LOT_QUANTITY`.
    pub qty: u64,
    /// A valid `mush_inventory` grade (§4.3 mints at 50).
    pub grade: u8,
    /// The storefront vault the lot was minted into.
    pub vault: mush_economy::VaultId,
    /// `StockIssueId::lot_id().0`.
    pub lot: u64,
    /// The stock clerk the tick acted as.
    pub clerk: ObjId,
}

impl StockReceipt {
    /// This receipt's issue id.
    ///
    /// # Errors
    ///
    /// As [`StockIssueId::derive`].
    pub fn issue_id(&self) -> Result<StockIssueId, String> {
        StockIssueId::derive(&self.pack, &self.entry, &self.org, &self.good, self.issue)
    }

    /// `qty` in range, a valid grade, and the receipt names its own issue's lot.
    ///
    /// # Errors
    ///
    /// A message naming the first invariant the receipt breaks.
    pub fn check(&self) -> Result<(), String> {
        if self.qty == 0 || self.qty > mush_inventory::record::MAX_LOT_QUANTITY {
            return Err(format!(
                "a stock receipt mints 1 to {} units, not {}",
                mush_inventory::record::MAX_LOT_QUANTITY,
                self.qty
            ));
        }
        mush_inventory::record::Grade::new(self.grade).map_err(|error| error.to_string())?;
        if self.issue_id()?.lot_id().0 != self.lot {
            return Err("a stock receipt names its own issue's lot".to_owned());
        }
        Ok(())
    }
}

// ---- the families and the handles ------------------------------------------

/// The port's nine production family handles.
#[derive(Clone, Debug)]
pub struct PortFamilies {
    /// Conserved private relay production receipts, one bounded book per foundry.
    pub relay_fabrications: StateFamilyHandle<ObjId, crate::relay_foundry::Fabrications>,
    /// `tacsim.port_stock`.
    pub port_stock: StateFamilyHandle<StockIssueId, StockReceipt>,
    /// `tacsim.ticket`.
    pub tickets: StateFamilyHandle<TicketId, Ticket>,
    /// **`tacsim.excursion`** (doc 20 cut SM.3b; §8): one rig record per terrain vehicle.
    pub excursions:
        StateFamilyHandle<crate::surface::excursion::RigKey, crate::surface::excursion::RigRecord>,
    /// **`tacsim.chart`** (doc 20 cut SM.4a; §5, D-20-8): one chart row per (character, site).
    pub charts:
        StateFamilyHandle<crate::surface::chart::ChartKey, crate::surface::chart::ChartRecord>,
    /// Current attempts on the finite survey commission board.
    pub surveys:
        StateFamilyHandle<crate::surface::survey::SurveyKey, crate::surface::survey::SurveyRecord>,
    /// Permanent passenger receipts and the bounded current-booking index.
    pub passengers: StateFamilyHandle<
        crate::port::passenger::store::BookingKey,
        crate::port::passenger::store::Booking,
    >,
    pub passenger_index: StateFamilyHandle<u8, crate::port::passenger::store::BookingIndex>,
    /// Permanent, private quest progress, keyed by source and paid passage.
    pub quests: StateFamilyHandle<crate::quest::model::QuestKey, crate::quest::model::QuestRecord>,
    pub quest_index:
        StateFamilyHandle<crate::quest::model::QuestIndexKey, crate::quest::model::QuestIndex>,
    pub quest_survey_evidence: StateFamilyHandle<ObjId, crate::quest::evidence::SurveyEvidenceBook>,
    /// **`tacsim.pockets`** (doc 20 cut SM.6; §3.4, D-20-5): one pocket cursor per surface
    /// deposit, keyed by the deposit's id — what the surface provider drew, and the lock's
    /// one home. Derived truth, never projected.
    pub pockets:
        StateFamilyHandle<mush_industry::OreDepositId, crate::surface::cursor::PocketCursor>,
    /// **`tacsim.claims`** (doc 20 cut SM.10; §9, D17): one block tenure per (site, block) —
    /// the one public source of a block's claim and paper, staged inside industry's own tenure
    /// bundle by the composed `SurfaceTenure`.
    pub claims:
        StateFamilyHandle<crate::surface::claims::BlockKey, crate::surface::claims::BlockTenure>,
    /// **`tacsim.port_mill`** (doc 20 cut SM.12a; §7.3 "The mill"): one ledger per mill,
    /// keyed by its [`crate::port::mill::MillId`] — the batch's receipt and replay guard.
    pub mills: StateFamilyHandle<crate::port::mill::MillId, crate::port::mill::MillLedger>,
    /// **`tacsim.dispatch`** (doc 20 cut SM.12b; §12.3): one delivery ledger per yard, keyed by
    /// its site — the delivery's receipt and replay guard.
    pub dispatch:
        StateFamilyHandle<crate::port::yard::DispatchKey, crate::port::yard::DispatchLedger>,
}

/// Published in module state by `crate::port_tick::register` — read-only capabilities, the
/// `InventoryHandles` shape (`mush-inventory/src/module.rs:370`).
#[derive(Clone, Debug)]
pub struct PortHandles {
    /// Real foundry production conversion action.
    pub relay_fabricate: Option<mush_core::DefinitionRef>,
    /// The port's own families.
    pub families: PortFamilies,
    /// `mush-inventory`'s families, from the cell the composition handed the layer
    /// (`server_modules_with_tacsim`); `None` on a roster that did not (every read then refuses).
    pub inventory: Option<mush_inventory::InventoryFamilies>,
    /// The test world's probe family (`crate::port_tick`); `None` in production.
    pub probe: Option<StateFamilyHandle<crate::port_tick::ProbeKey, crate::port_tick::ProbeMark>>,
    /// Doc 18 cut IC9: the registered `tacsim_port_burn` action (`crate::port::burn`); `Some`
    /// exactly when its provider registered (the inventory cell filled and the burn table
    /// built), so `+ship/undock` refuses `UNDOCK_UNWIRED` on a roster without it.
    pub burn: Option<mush_core::DefinitionRef>,
    /// **Doc 20 cut SM.3b**: the registered `tacsim_surface_act` action (every act's start,
    /// `crate::surface::act`); `Some` exactly when its provider registered (the inventory
    /// cell filled and the tools table built — the burn rule), so every `+tv` act answers
    /// `TV_UNWIRED` on a roster without it.
    pub surface_act: Option<mush_core::DefinitionRef>,
    /// **Doc 20 cut SM.3b**: the registered `tacsim_surface_done` action (every act's
    /// finish, submitted by the `tacsim.surface` tick as the operator); `Some` exactly
    /// when its provider registered.
    pub surface_done: Option<mush_core::DefinitionRef>,
    /// Staff recovery of a rig's work, placement and hopper, through the authoritative lane.
    pub surface_recovery: Option<mush_core::DefinitionRef>,
    /// **Doc 20 cut SM.3b** (finding F5): the one tools table the surface providers, the
    /// command's preview, `+tv` status and the departure's bay gate read — the battery
    /// has one source. `None` when the table did not build (logged; the providers are
    /// then unregistered and the verb answers `TV_UNWIRED`).
    pub surface_tools:
        Option<std::sync::Arc<BTreeMap<CanonicalId, crate::definitions::tools::ToolDef>>>,
    /// **Doc 20 cut SM.4a** (D9): the registered `tacsim_chart_forget` action (`+tv/forget`'s
    /// own, `crate::surface::chart::ChartForgetProvider`); registered on every roster, since it
    /// needs only the chart family.
    pub chart_forget: Option<mush_core::DefinitionRef>,
    /// Reserve one exact survey offer before Occator funds its contract.
    pub survey_accept: Option<mush_core::DefinitionRef>,
    /// Cancel an unfinished funded survey through its retained cancellation fact.
    pub survey_cancel: Option<mush_core::DefinitionRef>,
    /// Authenticated passenger booking and journey receipts.
    pub passenger: Option<mush_core::DefinitionRef>,
    pub quest: Option<mush_core::DefinitionRef>,
    /// **Doc 20 cut SM.6** (§3.4): the registered `tacsim_site_work` action (the site clerk's
    /// worked transfer, `crate::port::site::SiteWorkProvider`); `Some` exactly when its
    /// provider registered (the inventory cell filled).
    pub site_work: Option<mush_core::DefinitionRef>,
    /// **Doc 20 cut SM.12b** (§12.3): the registered `tacsim_dispatch` action (a site clerk's
    /// house survey and a yard dispatcher's delivery, `crate::port::yard::DispatchProvider`);
    /// `Some` exactly when its provider registered (the inventory cell filled).
    pub dispatch: Option<mush_core::DefinitionRef>,
}

pub(crate) fn canonical(text: &str) -> Result<CanonicalId, ModError> {
    CanonicalId::new(text).map_err(|error| ModError::msg(error.to_string()))
}

/// One family's descriptor on the suffix pattern every tacsim family follows:
/// `{value}.decode_json.v1`, `{value}.checks.v1`, `{value}.json.v1` beside the
/// key schema `key` and the value schema `{value}.v1`, version 1.
pub(crate) fn family_descriptor(
    family: &str,
    key: &str,
    value: &str,
) -> Result<StateFamilyDescriptor, ModError> {
    StateFamilyDescriptor::new(
        StateFamilyId::new(family).map_err(|error| ModError::msg(error.to_string()))?,
        canonical(key)?,
        canonical(&format!("{value}.v1"))?,
        1,
        canonical(&format!("{value}.decode_json.v1"))?,
        canonical(&format!("{value}.checks.v1"))?,
        canonical(&format!("{value}.json.v1"))?,
    )
    .map_err(|error| ModError::msg(error.to_string()))
}

/// Register the nine production families (doc 18 cut IC5; doc 20 cut SM.3b's
/// `tacsim.excursion`; doc 20 cut SM.4a's `tacsim.chart`; doc 20 cut SM.6's
/// `tacsim.pockets`; doc 20 cut SM.10's `tacsim.claims`; relay cut R9a's
/// `tacsim.relay_fabrications`; doc 20 cut SM.12a's `tacsim.port_mill`; doc 20 cut SM.12b's
/// `tacsim.dispatch`), unconditionally.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<PortFamilies, ModError> {
    let port_stock = reg.state_family::<StockIssueId, StockReceipt, _, _, _, _>(
        family_descriptor(
            PORT_STOCK_FAMILY,
            "tacsim.stock_issue_id.v1",
            "tacsim.port_stock",
        )?,
        |key: &StockIssueId| Ok(key.key_text()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &StockReceipt| value.check(),
        |value: &StockReceipt| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let tickets = reg.state_family::<TicketId, Ticket, _, _, _, _>(
        family_descriptor(TICKET_FAMILY, "tacsim.ticket_id.v1", "tacsim.ticket")?,
        |key: &TicketId| Ok(key.key_text()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &Ticket| value.check(),
        |value: &Ticket| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    // Doc 20 cut SM.3b (§8, D-20-11): the rigs' excursion records.
    let excursions = reg.state_family::<
        crate::surface::excursion::RigKey,
        crate::surface::excursion::RigRecord,
        _,
        _,
        _,
        _,
    >(
        family_descriptor(
            crate::surface::excursion::EXCURSION_FAMILY,
            "tacsim.excursion_key.v1",
            "tacsim.excursion",
        )?,
        |key: &crate::surface::excursion::RigKey| Ok(key.key_text()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::surface::excursion::RigRecord| value.check(),
        |value: &crate::surface::excursion::RigRecord| {
            serde_json::to_value(value).map_err(|e| e.to_string())
        },
    )?;
    // Doc 20 cut SM.4a (§5, D-20-8; the brief's lead ruling 3): the operators' charts, their
    // record cap tightened to `MAX_CHART_RECORD_BYTES` (the key limit unchanged).
    let chart_descriptor = family_descriptor(
        crate::surface::chart::CHART_FAMILY,
        "tacsim.chart_key.v1",
        "tacsim.chart",
    )?;
    let key_limit = chart_descriptor.max_key_bytes();
    let chart_descriptor = chart_descriptor
        .with_limits(key_limit, crate::surface::chart::MAX_CHART_RECORD_BYTES)
        .map_err(|error| ModError::msg(error.to_string()))?;
    let charts = reg.state_family::<
        crate::surface::chart::ChartKey,
        crate::surface::chart::ChartRecord,
        _,
        _,
        _,
        _,
    >(
        chart_descriptor,
        |key: &crate::surface::chart::ChartKey| Ok(key.key_text()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::surface::chart::ChartRecord| value.check(),
        |value: &crate::surface::chart::ChartRecord| {
            serde_json::to_value(value).map_err(|e| e.to_string())
        },
    )?;
    let survey_descriptor = family_descriptor(
        crate::surface::survey::SURVEY_FAMILY,
        "tacsim.survey_key.v1",
        "tacsim.survey",
    )?;
    let key_limit = survey_descriptor.max_key_bytes();
    let survey_descriptor = survey_descriptor
        .with_limits(key_limit, crate::surface::survey::MAX_SURVEY_RECORD_BYTES)
        .map_err(|error| ModError::msg(error.to_string()))?;
    let surveys = reg.state_family::<
        crate::surface::survey::SurveyKey,
        crate::surface::survey::SurveyRecord,
        _, _, _, _,
    >(
        survey_descriptor,
        |key: &crate::surface::survey::SurveyKey| {
            crate::surface::survey::SurveyKey::parse(key.as_str())
                .map(|key| key.key_text())
                .ok_or_else(|| "the survey key is not on the board".to_owned())
        },
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::surface::survey::SurveyRecord| value.check(),
        |value: &crate::surface::survey::SurveyRecord| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let passenger_descriptor = family_descriptor(
        crate::port::passenger::store::FAMILY,
        "tacsim.passenger_key.v1",
        "tacsim.passenger",
    )?;
    let passenger_key_limit = passenger_descriptor.max_key_bytes();
    let passenger_descriptor = passenger_descriptor
        .with_limits(
            passenger_key_limit,
            crate::port::passenger::store::MAX_RECORD_BYTES,
        )
        .map_err(|e| ModError::msg(e.to_string()))?;
    let passengers = reg.state_family::<crate::port::passenger::store::BookingKey, crate::port::passenger::store::Booking, _, _, _, _>(
        passenger_descriptor,
        |key: &crate::port::passenger::store::BookingKey| Ok(key.0.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::port::passenger::store::Booking| value.check(),
        |value: &crate::port::passenger::store::Booking| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let passenger_index = reg
        .state_family::<u8, crate::port::passenger::store::BookingIndex, _, _, _, _>(
            family_descriptor(
                crate::port::passenger::store::INDEX_FAMILY,
                "tacsim.passenger_index_key.v1",
                "tacsim.passenger_index",
            )?,
            |key: &u8| {
                if *key == 0 {
                    Ok("0".into())
                } else {
                    Err("passenger index key must be zero".into())
                }
            },
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &crate::port::passenger::store::BookingIndex| value.check(),
            |value: &crate::port::passenger::store::BookingIndex| {
                serde_json::to_value(value).map_err(|e| e.to_string())
            },
        )?;
    // Doc 20 cut SM.6 (§3.4, D-20-5): the pocket cursor sidecar, keyed by the deposit id in
    // industry's own `{:016x}` text, strict on parse.
    let pockets = reg.state_family::<
        mush_industry::OreDepositId,
        crate::surface::cursor::PocketCursor,
        _,
        _,
        _,
        _,
    >(
        family_descriptor(
            crate::surface::cursor::POCKETS_FAMILY,
            "tacsim.pockets_key.v1",
            "tacsim.pockets",
        )?,
        |key: &mush_industry::OreDepositId| Ok(crate::surface::cursor::cursor_key_text(key)),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::surface::cursor::PocketCursor| value.check(),
        |value: &crate::surface::cursor::PocketCursor| {
            serde_json::to_value(value).map_err(|e| e.to_string())
        },
    )?;
    // Doc 20 cut SM.10 (§9, D17): the block tenure records, keyed `{site}/{L}`, strict on parse.
    let claims = reg.state_family::<
        crate::surface::claims::BlockKey,
        crate::surface::claims::BlockTenure,
        _,
        _,
        _,
        _,
    >(
        family_descriptor(
            crate::surface::claims::CLAIMS_FAMILY,
            "tacsim.claims_key.v1",
            "tacsim.claims",
        )?,
        |key: &crate::surface::claims::BlockKey| Ok(key.key_text()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::surface::claims::BlockTenure| value.check(),
        |value: &crate::surface::claims::BlockTenure| {
            serde_json::to_value(value).map_err(|e| e.to_string())
        },
    )?;
    let relay_fabrications = reg
        .state_family::<ObjId, crate::relay_foundry::Fabrications, _, _, _, _>(
            family_descriptor(
                crate::relay_foundry::FAMILY,
                "tacsim.relay_foundry_id.v1",
                "tacsim.relay_fabrications",
            )?,
            |key: &ObjId| Ok(key.0.to_string()),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &crate::relay_foundry::Fabrications| value.check(),
            |value: &crate::relay_foundry::Fabrications| {
                serde_json::to_value(value).map_err(|e| e.to_string())
            },
        )?;
    // Doc 20 cut SM.12a (§7.3 "The mill"): one ledger per mill, keyed by its `MillId`.
    let mills = reg
        .state_family::<crate::port::mill::MillId, crate::port::mill::MillLedger, _, _, _, _>(
            family_descriptor(
                crate::port::mill::MILL_FAMILY,
                "tacsim.mill_id.v1",
                "tacsim.port_mill",
            )?,
            |key: &crate::port::mill::MillId| Ok(key.key_text()),
            |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
            |value: &crate::port::mill::MillLedger| value.check(),
            |value: &crate::port::mill::MillLedger| {
                serde_json::to_value(value).map_err(|e| e.to_string())
            },
        )?;
    // Doc 20 cut SM.12b (§12.3): one delivery ledger per yard, keyed by its site.
    let dispatch = reg.state_family::<
        crate::port::yard::DispatchKey,
        crate::port::yard::DispatchLedger,
        _,
        _,
        _,
        _,
    >(
        family_descriptor(
            crate::port::yard::DISPATCH_FAMILY,
            "tacsim.dispatch_key.v1",
            "tacsim.dispatch",
        )?,
        |key: &crate::port::yard::DispatchKey| Ok(key.key_text()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &crate::port::yard::DispatchLedger| value.check(),
        |value: &crate::port::yard::DispatchLedger| {
            serde_json::to_value(value).map_err(|e| e.to_string())
        },
    )?;
    let (quests, quest_index) = crate::quest::registration::families(reg)?;
    let quest_survey_evidence = crate::quest::evidence::register_survey_family(reg)?;
    Ok(PortFamilies {
        relay_fabrications,
        port_stock,
        tickets,
        excursions,
        charts,
        surveys,
        passengers,
        passenger_index,
        quests,
        quest_index,
        quest_survey_evidence,
        pockets,
        claims,
        mills,
        dispatch,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(text: &str) -> CanonicalId {
        CanonicalId::new(text).unwrap()
    }

    /// The golden stock-issue id later cuts must not move (doc 18 cut IC5):
    /// `("tacsim.lane.ceres_phobos", "ceres_port", "kasei", "metals", 1)`.
    const GOLDEN_ISSUE: &str = "1c6b17275ecaeac34f09a75be41768970921acb2b29b65c0b86dc5acb9c8ff4d";

    #[test]
    fn ic5_stock_issue_id_is_stable_and_names_its_parts() {
        let parts = (
            id("tacsim.lane.ceres_phobos"),
            id("ceres_port"),
            id("kasei"),
            id("metals"),
        );
        let base = StockIssueId::derive(&parts.0, &parts.1, &parts.2, &parts.3, 1).unwrap();
        assert_eq!(
            base,
            StockIssueId::derive(&parts.0, &parts.1, &parts.2, &parts.3, 1).unwrap(),
            "the same parts give the same id"
        );
        let other = id("other");
        for changed in [
            StockIssueId::derive(&other, &parts.1, &parts.2, &parts.3, 1),
            StockIssueId::derive(&parts.0, &other, &parts.2, &parts.3, 1),
            StockIssueId::derive(&parts.0, &parts.1, &other, &parts.3, 1),
            StockIssueId::derive(&parts.0, &parts.1, &parts.2, &other, 1),
            StockIssueId::derive(&parts.0, &parts.1, &parts.2, &parts.3, 2),
        ] {
            assert_ne!(changed.unwrap(), base, "each part names the id");
        }
        let text = base.key_text();
        assert_eq!(text.len(), 64);
        assert_eq!(StockIssueId::parse(&text), Some(base));
        assert_eq!(StockIssueId::parse(&text.to_uppercase()), None);
        assert_eq!(base.lot_id(), base.lot_id());
        assert_eq!(text, GOLDEN_ISSUE, "the pinned stock-issue id");
        let head = u64::from_str_radix(&text[..16], 16).unwrap();
        assert_eq!(base.lot_id().0, head, "the lot is the id's first 8 bytes");
    }

    fn receipt() -> StockReceipt {
        let (pack, entry, org, good) = (
            id("tacsim.lane.ceres_phobos"),
            id("ceres_port"),
            id("kasei"),
            id("metals"),
        );
        let lot = StockIssueId::derive(&pack, &entry, &org, &good, 3)
            .unwrap()
            .lot_id()
            .0;
        StockReceipt {
            pack,
            entry,
            org,
            good,
            issue: 3,
            qty: 40,
            grade: 50,
            vault: mush_economy::VaultId(7),
            lot,
            clerk: ObjId(90),
        }
    }

    #[test]
    fn ic5_stock_receipt_wire_is_strict() {
        let good = receipt();
        good.check().unwrap();
        let wire = serde_json::to_value(&good).unwrap();
        let back: StockReceipt = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(back, good);
        let mut extra = wire;
        extra
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), json!(1));
        assert!(serde_json::from_value::<StockReceipt>(extra).is_err());
        for qty in [0, mush_inventory::record::MAX_LOT_QUANTITY + 1] {
            let bad = StockReceipt { qty, ..receipt() };
            assert!(bad.check().is_err(), "qty {qty} is refused");
        }
        let bad = StockReceipt {
            grade: mush_inventory::record::MAX_GRADE + 1,
            ..receipt()
        };
        assert!(bad.check().is_err(), "an invalid grade is refused");
        let mut bad = receipt();
        bad.lot = bad.lot.wrapping_add(1);
        assert!(bad.check().is_err(), "a lot that is not its issue's");
    }

    fn leg(to: &str, t0: i64, secs: u32) -> LegPlan {
        LegPlan {
            to: id(to),
            body: id(to),
            dist_km: 1_000,
            secs,
            t0,
            t1: t0 + i64::from(secs),
            ascent_mps: 0,
        }
    }

    fn ticket() -> Ticket {
        let depart = 1_000_000;
        let a = leg("phobos", depart, 965_623);
        let b = leg("ceres", a.t1, 730_645);
        let total = b.t1 - depart;
        Ticket {
            hull: ObjId(5011),
            seq: 9,
            route: id("ceres_phobos"),
            route_pack: id("tacsim.lane.ceres_phobos"),
            route_version: 2,
            unit_definition: DefinitionDigest::from_bytes([7; 32]),
            manifest: Manifest {
                cargo: [(id("ice"), 80), (id("concentrate"), 20)]
                    .into_iter()
                    .collect(),
                tank: 41,
                locker: 21,
                occupants: 1,
            },
            payload_kg: 100_000,
            plan: PassagePlan {
                route: id("ceres_phobos"),
                law: id("coast"),
                depart_ic: depart,
                sky_rev: DefinitionDigest::from_bytes([9; 32]),
                legs: vec![a, b],
                total_s: total,
                burn_t: 41,
                crew_days: 21,
                throttle_permille: 1_000,
                landed_crew_days: 0,
            },
            order: Some(9),
            attempt: 1,
            burned_at: depart,
        }
    }

    fn refused(ticket: &Ticket, why: &str) {
        assert!(ticket.check().is_err(), "{why} is refused");
    }

    #[test]
    fn ic5_ticket_wire_is_strict_and_the_plan_checks() {
        let good = ticket();
        good.check().unwrap();
        // Doc 18 cut IC9: the key is derived from the value.
        assert_eq!(
            good.ticket_id(),
            TicketId {
                hull: ObjId(5011),
                seq: 9
            }
        );
        let wire = serde_json::to_value(&good).unwrap();
        assert_eq!(
            serde_json::from_value::<Ticket>(wire.clone()).unwrap(),
            good
        );
        // Doc 18 cut IC9: `launched` is no longer a field of the wire.
        let mut stale = wire.clone();
        stale
            .as_object_mut()
            .unwrap()
            .insert("launched".into(), json!(false));
        assert!(serde_json::from_value::<Ticket>(stale).is_err());
        // An unknown field at each level.
        for path in [
            vec![],
            vec!["manifest"],
            vec!["plan"],
            vec!["plan", "legs", "0"],
        ] {
            let mut bad = wire.clone();
            let mut at = &mut bad;
            for step in &path {
                at = match step.parse::<usize>() {
                    Ok(index) => at.get_mut(index).unwrap(),
                    Err(_) => at.get_mut(*step).unwrap(),
                };
            }
            at.as_object_mut()
                .unwrap()
                .insert("extra".into(), json!(true));
            assert!(
                serde_json::from_value::<Ticket>(bad).is_err(),
                "an unknown field at {path:?} is refused"
            );
        }
        let mut t = ticket();
        t.plan.legs.clear();
        refused(&t, "zero legs");
        let mut t = ticket();
        let mut legs = Vec::new();
        let mut at = t.plan.depart_ic;
        for _ in 0..33 {
            let l = leg("ceres", at, 10);
            at = l.t1;
            legs.push(l);
        }
        t.plan.total_s = at - t.plan.depart_ic;
        t.plan.legs = legs;
        refused(&t, "33 legs");
        let mut t = ticket();
        t.plan.legs[1].t1 += 1;
        t.plan.total_s += 1;
        refused(&t, "a leg whose end is not t0 + secs");
        let mut t = ticket();
        t.plan.legs[1].t0 += 1;
        t.plan.legs[1].t1 += 1;
        t.plan.total_s += 1;
        refused(&t, "a gap between legs");
        let mut t = ticket();
        t.plan.depart_ic -= 1;
        t.plan.total_s += 1;
        refused(&t, "a first leg that does not begin at departure");
        let mut t = ticket();
        t.plan.total_s += 1;
        refused(&t, "a total mismatch");
        let mut t = ticket();
        t.plan.route = id("elsewhere");
        refused(&t, "a plan on another route");
        let mut t = ticket();
        t.attempt = 0;
        refused(&t, "attempt 0");
        let mut t = ticket();
        t.manifest.occupants = 0;
        refused(&t, "no occupants");
        let mut t = ticket();
        t.manifest.cargo.insert(id("water"), 0);
        refused(&t, "a zero cargo quantity");
        let mut t = ticket();
        t.manifest.cargo = (0..17).map(|n| (id(&format!("good_{n}")), 1)).collect();
        refused(&t, "17 goods");
        let mut t = ticket();
        t.manifest.tank = 40;
        refused(&t, "a burn above the tank");
        let mut t = ticket();
        t.manifest.locker = 20;
        refused(&t, "provisions above the locker");
    }

    #[test]
    fn ic5_ticket_id_key_text_is_strict() {
        let id = TicketId {
            hull: ObjId(5011),
            seq: 9,
        };
        assert_eq!(id.key_text(), "5011:9");
        assert_eq!(TicketId::parse("5011:9"), Some(id));
        assert_eq!(
            TicketId::parse("0:0"),
            Some(TicketId {
                hull: ObjId(0),
                seq: 0
            })
        );
        for bad in [
            "05011:9",
            "5011",
            "5011:9:1",
            "a:1",
            "+1:2",
            "#5011:9",
            "4294967296:0",
        ] {
            assert_eq!(TicketId::parse(bad), None, "{bad:?} is refused");
        }
    }
}
