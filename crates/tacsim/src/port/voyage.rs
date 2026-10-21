//! **Undock and fly: the sailing plan, the launch and the passage renders** (doc 18
//! cut IC9; doc 18 §4.4 "Passage: the planner over the sky at departure", "Failure
//! handling", §4.2 "The helm", §2 steps 9, 11, 12; D-18-11, D-18-18, D-18-23,
//! D-18-30, D-18-44 … D-18-47, D-18-50; lead rulings L3, L4, L7, L8, L13, L14, L21,
//! L24, L26, L28, L29, L31 of the IC9 brief).
//!
//! - **The filed sailing plan** ([`Voyage`], one `tacsim:voyage:{hull}` document
//!   per programme hull, lead ruling L4): the passage `+ship/undock` planned and
//!   filed — the *departing* phase of §4.4's lifecycle — and the per-hull burn
//!   sequence (`last_seq`; IC11 reuses it as the order queue's minter). The burn's
//!   offer reads the pending passage from here, so the plan reaches the prepare
//!   only from the server's own document ([`crate::port::burn`]).
//! - **`+ship/plan [<port>]`** (`plan_cmd`): a write-nothing dry run — both hold
//!   slacks with the binding measure, the room per good on this port's board, the
//!   payload, the burn, the stores and the passage in IC and real time.
//! - **`+ship/undock <port>`** (`undock_cmd`): every refusal before any write
//!   (the launch included, dry-run through `mush_sim::depart_ineligible`, lead
//!   ruling L28), then the voyage document and ONE authoritative
//!   `tacsim_port_burn` as the pilot.
//! - **The launch arm** (`launch_step`, `launch_verify`) on the `tacsim.port`
//!   tick: it finds the burned ticket and calls `mush_sim::depart` with the plan's
//!   frozen leg times from the plan's own `depart_ic` (lead ruling L3) — a direct
//!   placement write, no submission and no swap (L7) — atomically with clearing
//!   the pending; a launch held by a later change pages staff once and retries
//!   after its backoff (`launch_hold`, L29); a burn that never lands lapses
//!   after [`LAUNCH_GRACE_FIRES`] (L26).
//! - **The goods-not-yet-aboard rule** (`goods_pending_aboard`, lead ruling L13):
//!   promised matter holds the hull, read over mush-market's public reads only —
//!   and, since doc 18 cut IC10, a sale's goods still leaving the hold
//!   ([`GOODS_LEAVING`]).
//! - **The passage renders** (`passage_of`): `+ship`'s first line and the hull
//!   rooms' status line, departing and under way (the leg, the ETA in IC and real
//!   time), and the carrier hooks' narration texts.
//!
//! Real time appears only on §4.4's OOC surfaces (lead ruling L25): `+ship`,
//! `+ship/plan`, [`CLEARANCE_FILED`], the status line and the notices' UTC prefix.

use std::collections::BTreeMap;

use mush_core::{
    CanonicalId, CmdError, Ctx, DefinitionDigest, IdempotencyKeyText, ObjId, ObjKind, PerceivedRef,
    World,
};
use mush_economy::{EconomyStore, VaultId};
use mush_inventory::record::EndpointRef;
use mush_market::{ListingId, LotMatter, MarketStore, PaidKind, QuoteId};
use mush_sim::data::{SpatialParent, SpatialState};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::definitions::content::LANE_PACK;
use crate::definitions::goods::Consumable;
use crate::definitions::logistics::{MASS_KG, VOLUME_L};
use crate::port::burn::{BURN_KEY_PREFIX, HullSpec, consumable_good, hull_spec_of};
use crate::port::capacity::{self, Compartment, format_m3, format_tonnes};
use crate::port::chart::PLAN_WINDOW_DAYS;
use crate::port::coop::{helm_of, name_of, port_name};
use crate::port::def::PortDef;
use crate::port::families::{MAX_TICKET_OCCUPANTS, PassagePlan, PortHandles, Ticket, TicketId};
use crate::port::gauges;
use crate::port::map::PortMap;
use crate::port::passage::{
    DEFAULT_THROTTLE_PERMILLE, MAX_THROTTLE_PERMILLE, MIN_THROTTLE_PERMILLE, PlanError,
    RoutePolicy, choose_route, flip_profile, payload_kg, route_landing_floor,
};
use crate::port::readiness::report_ports;
use crate::port::ship::{
    self, Berth, HullView, SHIP_CLEARED, SHIP_UNDER_WAY, STATUS_CLEARED, STATUS_UNDER_WAY,
    berth_of, hold_vault, hull_view,
};
use crate::port_tick::{PortTickState, Step, gone, notify, queue_notice, read_pool};
use crate::sky::Sky;
use crate::store::{TacsimStore, put_voyage};

// ---- the document --------------------------------------------------------------

/// The sailing-plan document prefix (`tacsim:voyage:{hull.0}`).
pub const VOYAGE_PREFIX: &str = "tacsim:voyage:";
/// The sailing plan's schema.
pub const VOYAGE_SCHEMA: u16 = 1;

/// Fires a pending passage with no ticket waits before it lapses (about two
/// minutes at the 5 s cadence; lead ruling L26), and the backoff a held launch
/// waits before it retries (L29).
pub const LAUNCH_GRACE_FIRES: u64 = 24;
/// Fires a staged launch is watched for its pull verification before it is
/// dropped (about two minutes).
pub const LAUNCH_WATCH_FIRES: u64 = 24;

/// `tacsim:voyage:{hull.0}` — the numeric id, never `ObjId`'s `#`-prefixed Display.
#[must_use]
pub fn voyage_key(hull: ObjId) -> SmolStr {
    SmolStr::new(format!("{VOYAGE_PREFIX}{}", hull.0))
}

/// The strict inverse of [`voyage_key`]: the tail as a strict decimal `u32`.
#[must_use]
pub fn voyage_key_owner(key: &str) -> Option<ObjId> {
    key.strip_prefix(VOYAGE_PREFIX)
        .and_then(crate::port::families::strict_decimal_u32)
        .map(ObjId)
}

/// **One hull's filed sailing plan** (doc 18 cut IC9; lead ruling L4): the whole
/// `tacsim:voyage:{hull}` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Voyage {
    /// [`VOYAGE_SCHEMA`].
    pub schema: u16,
    /// The hull (the identity rule: its key's).
    pub hull: ObjId,
    /// The last burn sequence this hull minted (0 before the first); never reused.
    pub last_seq: u32,
    /// Doc 18 cut IC14 row 21 (lead rulings L8b, L20): the highest `seq` whose launch
    /// the pilot was told of (`CLEARANCE_GRANTED`), so the grant is told once across
    /// any number of reboots. Required on the wire (no default: voyage documents are
    /// slice-only and pre-launch; a defaulted 0 would re-tell every old grant once);
    /// never above `last_seq`.
    pub told: u32,
    /// The passage filed and not yet launched, lapsed or stood down.
    pub pending: Option<PendingVoyage>,
    /// **Doc 20 cut SM.2 (Q-20-7)** — the launched passage's site arrival, carried from
    /// the pending passage when the launch lands (`launch_step`) and read world-only by
    /// the arrival lines (the carrier hook cannot read the pools); `None` for a passage
    /// to a station, and cleared by the next filing. Skipped when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival: Option<SiteArrival>,
}

/// **What a hull bound for a site will arrive with** (doc 20 cut SM.2; Q-20-7): the
/// locker after the passage's own crew-days, the crew-days the filing held back for the
/// leg home, and the souls aboard — what the arrival's stores line is computed from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteArrival {
    /// The locker at arrival, crew-days.
    pub locker: u64,
    /// The leg home's reserve, crew-days.
    pub home_crew_days: u32,
    /// The souls aboard.
    pub souls: u32,
}

/// **A filed passage** (doc 18 §4.4 "departing"): everything the burn's prepare
/// re-checks and the ticket freezes, written by `+ship/undock` from the server's
/// own planner.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingVoyage {
    /// The burn sequence this passage burns under: `last_seq + 1`.
    pub seq: u32,
    /// The pilot who filed it (the burn's only submitter).
    pub pilot: ObjId,
    /// The destination port's venue word.
    pub to_venue: CanonicalId,
    /// The hold vault, `VaultId.0`.
    pub vault: u64,
    /// Souls aboard at filing, `1..=MAX_TICKET_OCCUPANTS`.
    pub occupants: u32,
    /// The payload at filing, kilograms.
    pub payload_kg: u64,
    /// The route pack the plan was made from (the lane).
    pub route_pack: CanonicalId,
    /// That pack's version (the latched one; lead ruling L23).
    pub route_version: u32,
    /// The hull's unit definition digest (lead ruling L34).
    pub unit_definition: DefinitionDigest,
    /// The plan (§4.8).
    pub plan: PassagePlan,
    /// The queue's order id; `None` for a live `+ship/undock` (IC11).
    pub order: Option<u32>,
    /// The attempt, `>= 1`.
    pub attempt: u32,
    /// The IC second it was filed.
    pub filed_ic: i64,
    /// The real second it was filed (`World::now`).
    pub filed_at: i64,
    /// **Doc 20 cut SM.2 (Q-20-7)** — for a passage to a site, what the hull will arrive
    /// with ([`SiteArrival`]); `None` otherwise. Skipped when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arrival: Option<SiteArrival>,
    /// **Doc 20 cut SM.3a (§4.3, D-20-7)** — the rigs in the hull's bay at filing,
    /// each its dry mass, hopper vault and hopper goods: `payload_kg` counts them,
    /// and the burn's prepare re-reads their hoppers through its own transaction
    /// (a placement is a legacy slot it cannot read, so the list is frozen here).
    /// It cannot go stale: `+tv/lease` refuses while a clearance is filed. Skipped
    /// when empty, so no pending filed before the cut re-encodes differently.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carried: Vec<crate::port::passage::CarriedRig>,
}

impl Voyage {
    /// Schema 1; a pending passage's `seq` is `last_seq + 1`, its plan checks,
    /// names a route, carries `1..=MAX_TICKET_OCCUPANTS` occupants and an
    /// attempt of at least 1.
    ///
    /// # Errors
    ///
    /// A message naming the first invariant the document breaks.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != VOYAGE_SCHEMA {
            return Err(format!("a sailing plan is schema {VOYAGE_SCHEMA}"));
        }
        if self.told > self.last_seq {
            return Err("a sailing plan tells no launch it has not burned".to_owned());
        }
        let Some(pending) = &self.pending else {
            return Ok(());
        };
        if self.last_seq.checked_add(1) != Some(pending.seq) {
            return Err("a filed passage burns under the next sequence".to_owned());
        }
        pending.plan.check()?;
        if pending.plan.route.as_str().is_empty() {
            return Err("a filed passage names its route".to_owned());
        }
        if pending.occupants == 0 || pending.occupants > MAX_TICKET_OCCUPANTS {
            return Err(format!(
                "a filed passage carries 1 to {MAX_TICKET_OCCUPANTS} occupants"
            ));
        }
        if pending.attempt == 0 {
            return Err("a filed passage's attempt counts from 1".to_owned());
        }
        Ok(())
    }
}

/// **One hull's sailing plan, as the store holds it.**
#[derive(Clone, Copy, Debug)]
pub enum VoyageState<'a> {
    /// No document: the hull has never filed.
    Absent,
    /// The readable document.
    Present(&'a Voyage),
    /// A claimed key whose body did not read: fail closed.
    Unreadable,
}

/// **Whether a hull's sailing plan blocks a hold-, crew- or pool-changing act** (doc
/// 18 cut IC14 row 11; lead ruling L17): a pending passage is filed, or the document
/// does not read — an unreadable plan is treated as filed at every gate (fail closed),
/// in each gate's own words.
#[must_use]
pub fn voyage_blocks(w: &World, hull: ObjId) -> bool {
    voyage_dark(w, hull) || pending_of(w, hull).is_some()
}

/// Whether the hull's sailing plan is a claimed key whose body did not read (doc 18
/// cut IC14 row 11).
#[must_use]
pub fn voyage_dark(w: &World, hull: ObjId) -> bool {
    w.module_state::<TacsimStore>()
        .is_some_and(|store| matches!(store.voyage_state(hull), VoyageState::Unreadable))
}

/// The hull's pending passage, when its plan reads and one is filed.
#[must_use]
pub fn pending_of(w: &World, hull: ObjId) -> Option<&PendingVoyage> {
    match w.module_state::<TacsimStore>()?.voyage_state(hull) {
        VoyageState::Present(voyage) => voyage.pending.as_ref(),
        VoyageState::Absent | VoyageState::Unreadable => None,
    }
}

// ---- the texts ---------------------------------------------------------------------

/// The crew hear the launch (doc 18 §2 step 11): `{name}`.
pub const LOCK_SEALS: &str = "The {name}'s lock seals. Berth clamps release; the drive lights.";
/// An intermediate boundary (§2 "Day 1/2"): `{waypoint}`, `{detail}`, `{n}`, `{m}`.
pub const BOUNDARY: &str = "{waypoint} slides past{detail}. Leg {n} of {m}.";
/// The arrival, aboard (§6): `{name}`, `{port}`.
pub const DOCKS: &str = "{name} docks at {port}.";
/// The arrival, for an absent tenant (§2 step 12): `{hhmm}`, `{name}`, `{port}`, `{gallery}`.
pub const ARRIVAL_NOTICE: &str = "{hhmm} UTC {name} docked at {port}, {gallery}.";
/// The launch verified (§6's line completed): `{burn}`, `{days}`, `{name}`, `{port}`, `{icdays}`.
pub const CLEARANCE_GRANTED: &str = "Clearance granted. {burn} t reaction mass and {days} crew-days committed; the {name} is under way for {port}, arrival in {icdays} IC days.";
/// A burn that never landed: `{port}`.
pub const CLEARANCE_LAPSED: &str =
    "The clearance for {port} lapsed before the burn; nothing was burned.";
/// The manual arm refusing a programme hull: `{name}`.
pub const MANUAL_DEPARTURE: &str =
    "The {name} flies only a passage its pilot files with +ship/undock.";
/// The manual arm refusing a lane route: `{route}`.
pub const LANE_UNPLANNED: &str =
    "Route {route} is a lane passage; it is flown only from a plan filed with +ship/undock.";
/// A held launch, told to the pilot once: `{name}`, `{port}`.
pub const LAUNCH_HELD: &str = "The {name}'s clearance for {port} is held: the passage cannot begin yet. Staff have been told.";
/// A clearance staff stood down: `{name}`, `{port}`.
pub const CLEARANCE_STOOD_DOWN: &str =
    "The {name}'s clearance for {port} was stood down by the port. Nothing burned is returned.";
/// `+ship/undock`'s reply: `{port}`, `{burn}`, `{days}`, `{icdays}`, `{hhmm}`, `{day}`.
pub const CLEARANCE_FILED: &str = "Clearance filed for {port}: the plan burns {burn} t of reaction mass and {days} crew-days; arrival in {icdays} IC days, about {hhmm} UTC {day}.";
/// Under way already: `{name}`.
pub const UNDOCK_UNDER_WAY: &str = "The {name} is already under way.";
/// Departing already: `{name}`, `{port}`.
pub const UNDOCK_CLEARED: &str =
    "The {name} already has clearance for {port}; the lock is sealing.";
/// Not aboard: `{name}`.
pub const NOT_ABOARD: &str = "You undock from aboard the {name}.";
/// Not at a berth: `{name}`.
pub const NOT_DOCKED: &str = "The {name} is not at a berth.";
/// No such destination: `{here}`, `{list}`.
pub const NO_ROUTE: &str = "No passage leaves {here} for that port; {here} clears for {list}.";
/// `via=` named no lane place (doc 19 cut A.2): `{via}`, `{list}`.
pub const VIA_UNKNOWN: &str = "No lane place is called {via}; the waypoints are {list}.";
/// `throttle=` outside its band (doc 19 cut A.2): `{min}`, `{max}`.
pub const THROTTLE_USAGE: &str =
    "throttle= is {min} to {max} permille of thrust (1000 is full thrust).";
/// `policy=` other than the two words (doc 19 cut A.2).
pub const POLICY_USAGE: &str = "policy= is fastest or cheapest.";
/// `at=` other than `now` (doc 18 cut IC11, lead ruling L8: a later hour is a queued
/// depart).
pub const AT_NOT_YET: &str =
    "+ship/undock leaves now; to leave at a later hour, queue +ship/order depart <port> at=<time>.";
/// The launch dry run refused: `{name}`, `{port}`.
pub const PORT_CLOSED: &str =
    "The {name} cannot be cleared for {port} right now. Nothing was burned.";
/// Promised goods not yet aboard: `{quote}`, `{name}`.
pub const GOODS_COMING: &str =
    "Quote #{quote}'s goods have not come aboard yet; the {name} waits for them.";
/// A sale's goods not yet out of the hold (doc 18 cut IC10, decision 14): `{quote}`,
/// `{name}`. The remedy it names is `restage_deliver`, behind `+market/accept`.
pub const GOODS_LEAVING: &str = "Quote #{quote}'s goods have not left the hold yet; the {name} waits for the hand-over (+market/accept {quote} asks for it again).";
/// A listing holds goods in the hold: `{listing}`, `{name}`.
pub const LISTING_HOLDS: &str = "Your listing #{listing} holds goods in the {name}'s hold; withdraw it, or let it lapse and be released, before you undock.";
/// Over the rail: `{name}`, `{payload}`, `{ceiling}`.
pub const OVERLOADED: &str = "The {name} carries {payload} t against a rated {ceiling} t; lighten the hold before you undock. Nothing was burned.";
/// The tank below the burn: `{port}`, `{burn}`, `{tank}`.
pub const TANK_SHORT: &str = "The plan for {port} burns {burn} t of reaction mass; the tank holds {tank} t. Nothing was burned.";
/// The locker below the crew-days: `{port}`, `{days}`, `{have}`.
pub const STORES_SHORT: &str = "The passage to {port} needs {days} crew-days of stores; the locker holds {have}. Nothing was burned.";
/// **`+ship/plan`'s `Throttle:` line** (doc 19 cut A.8, D-19-22): the highest
/// throttle that fits the tank and the locker with both filled to capacity, printed
/// only when the plan at the asked throttle does not — the line never starts with
/// `Burn ` or `Stores `, so every reader of those lines stands.
pub const THROTTLE_FITS: &str = "Throttle: with the tank and locker full, throttle={throttle} fits: burn {burn} t, {days} crew-days, {passage} IC days.";
/// The same line when no throttle in the band fits.
pub const THROTTLE_NONE: &str =
    "Throttle: no throttle from {min} permille fits this load even with the tank and locker full.";
/// **The `Throttle:` line on a site's pad** (doc 20 cut SM.2, decision 7): nothing is
/// sold on a pad, so the line reads the tank and the locker aboard, never filled — the
/// throttle the filing held the leg home back at (`SITE_HOME`) is one that fits.
pub const THROTTLE_FITS_PAD: &str = "Throttle: with the tank and locker as they stand, throttle={throttle} fits: burn {burn} t, {days} crew-days, {passage} IC days.";
/// The same line on a pad when no throttle from the landing floor fits.
pub const THROTTLE_NONE_PAD: &str = "Throttle: no throttle from {min} permille fits this load with the tank and locker as they stand.";
/// **The `Throttle:` line for a site destination** (doc 20 cut SM.2, the lead's
/// pre-commit item 2): the departure holds back the leg home (`SITE_HOME`), so the line
/// names the highest throttle from the destination's landing floor at which the passage
/// out and the leg home together fit: `{pools}` (`full`, or `as they stand` on a pad),
/// `{throttle}`, `{burn}` and `{home}` (tonnes out and held back), `{days}` and
/// `{home_days}` (crew-days out and held back), `{passage}` (IC days).
pub const THROTTLE_FITS_SITE: &str = "Throttle: with the tank and locker {pools}, throttle={throttle} fits the passage and the leg home: burn {burn} t and {home} t held back, {days} and {home_days} crew-days, {passage} IC days.";
/// The same when no throttle fits today but the landing floor does on a later day of
/// `+chart`'s window (the IC year `+chart <port>` scans, `chart::WINDOW_DAYS`): `{min}`
/// (the floor), `{pools}`, `{date}`, `{offset}` (IC days from now).
pub const THROTTLE_LATER_SITE: &str = "Throttle: no throttle from {min} permille fits the passage and the leg home today with the tank and locker {pools}; throttle={min} does on {date} (in {offset} IC days).";
/// The same when no day of that window fits: `{min}`, `{pools}`, `{window}` (365).
pub const THROTTLE_NONE_SITE: &str = "Throttle: no throttle from {min} permille fits the passage and the leg home with the tank and locker {pools}, today or in the next {window} IC days.";
/// The planner could not answer: `{name}` (`+ship/plan` without the last sentence).
pub const PLAN_DARK: &str = "The {name}'s passage cannot be planned right now. Nothing was burned.";
/// `+ship/plan` under way: `{name}`.
pub const PLAN_UNDER_WAY: &str = "The {name} is under way; a passage is planned at a berth.";
/// An unreadable sailing plan: `{name}`.
pub const VOYAGE_DARK: &str =
    "The {name}'s sailing papers cannot be read right now. Ask staff to look at them.";
/// The burn action is not registered on this roster.
pub const UNDOCK_UNWIRED: &str = "Clearance cannot be filed on this server.";
/// `+ship/leave` and `+ship/board` while a clearance is filed: `{name}`.
pub const LEAVE_CLEARED: &str = "The {name} has clearance; the lock is sealed.";
/// `+ship/undock` with no destination.
pub const UNDOCK_USAGE: &str = "Usage: +ship/undock <port>";
/// The last sentence `+ship/plan` drops from [`PLAN_DARK`] and ends with.
pub const NOTHING_BURNED: &str = " Nothing was burned.";
/// `+ship/plan`'s closing line.
pub const NOTHING_WRITTEN: &str = "Nothing was written.";
/// **Doc 20 cut SM.2**: the arrival at a site, aboard (§1 beat 3): `{name}`, `{port}`.
pub const SETTLES: &str = "{name} settles onto the pad at {port}.";
/// The arrival at a site for an absent tenant: `{hhmm}`, `{name}`, `{port}`. With the
/// stores line it is the once-per-stay short-rations notice (decision 8).
pub const SETTLED_NOTICE: &str = "{hhmm} UTC {name} settled on the pad at {port}.";
/// **The landing law refused** (D-20-3): `{port}` the site, `{body}` its body's display
/// name, `{name}` the hull, `{twr}` the thrust-to-weight in hundredths, floored.
pub const LANDING_REFUSED: &str = "{port} is on {body}: at this load and throttle the {name} lifts {twr} times her weight there, under the 1.20 a landing needs. Nothing was burned.";
/// A departure for a site whose tank cannot hold the leg home back too: `{port}`,
/// `{burn}`, `{tank}`.
pub const SITE_TANK_SHORT: &str = "The passage to {port} and the leg home burn {burn} t of reaction mass; the tank holds {tank} t. Nothing was burned.";
/// A departure for a site whose locker cannot hold the leg home back too: `{port}`,
/// `{days}`, `{have}`.
pub const SITE_STORES_SHORT: &str = "The passage to {port} and the leg home need {days} crew-days of stores; the locker holds {have}. Nothing was burned.";
/// `+ship/plan`'s leg-home line for a site destination: `{port}`, `{burn}`, `{days}`,
/// `{throttle}`.
pub const SITE_HOME: &str = "Leg home from {port}: burn {burn} t, {days} crew-days at throttle={throttle}, the most in the next 30 IC days; the filing holds both back.";
/// The crew hear a lift from a pad: `{name}`.
pub const LIFT_SEALS: &str = "The {name}'s lock seals. The pad drops away under the drive.";
/// `+ship/lift` with no destination.
pub const LIFT_USAGE: &str = "Usage: +ship/lift <port>";
/// **Doc 20 cut SM.2 (lead override 3)**: a departure to a site whose readiness lines are
/// not all ready: `{site}`.
pub const SITE_CLOSED: &str = "{site} is closed: its pad is not ready.";
/// **Doc 20 cut SM.3b (§4.3, D-20-7)**: a rig out on a site: `{n}`, `{site}`, `{name}`.
pub const TV_OUT: &str = "Rig {n} is out on {site}; the {name} lifts only with every rig in her bay: +tv/{n} recover brings it in.";
/// **Doc 20 cut SM.3b (D-20-7; decision 21: the refusal names a remedy that works)**: a rig
/// logged out while its hull is **not** on a site's pad — at a port, or anywhere a recover
/// cannot reach (only a staff move gets it there): `{n}`, `{where}` (`on {site}`, or `away
/// from her bay` when no site can be named), `{name}`. Under the same `DepartRefusalKind::TvOut`.
pub const TV_AWAY: &str = "Rig {n} is logged out {where}; the {name} lifts only with every rig in her bay: staff must bring it in.";
/// **Doc 20 cut SM.3b**: a rig mid-act: `{n}`, `{doing}`, `{name}`.
pub const TV_BUSY: &str = "Rig {n} is {doing}; the {name} lifts once it is done.";
/// **Doc 20 cut SM.3b (§4.3)**: a hopper with ore in it: `{n}`, `{t}` whole tonnes, `{name}`.
/// (SM.8 adds the unload to the remedy.)
pub const HOPPER_LOADED: &str = "Rig {n}'s hopper holds {t} t of ore; the {name} lifts with every hopper empty: +tv/{n} dump empties it onto the spoil.";

/// **The waypoint details** (lead ruling L9: code data keyed by the lane place id,
/// so no pack version moves): the clause [`BOUNDARY`]'s `{detail}` takes.
pub const WAYPOINT_DETAILS: [(&str, &str); 1] = [(
    "kirkwood_depot",
    ": the Co-op's tank farm, a Republic picket's transponder asking for your manifest",
)];
/// **The waypoints' short words** (lead ruling L35; 18a §3's "Kirkwood astern"),
/// keyed like [`WAYPOINT_DETAILS`]; any other waypoint reads as its display name.
pub const WAYPOINT_SHORT: [(&str, &str); 1] = [("kirkwood_depot", "Kirkwood")];

// ---- the time renders (pure) ------------------------------------------------------

/// IC seconds as IC days to one decimal, truncated (`1_694_961` → `19.6`).
#[must_use]
pub(crate) fn ic_days(secs: i64) -> String {
    let tenths = secs.max(0).saturating_mul(10) / 86_400;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Real seconds as `{h} h {m} min`, `{m} min` or `under a minute` — **the one spelling of a
/// real wall-clock span** (doc 20 cut SM.3b, the lead's ruling in its fixer round: minutes are
/// `min`, so a real span never reads as the metres doc 20's texts print as `m`). The arena's
/// match durations (`match_::record::duration_words`) and the chart's light lag
/// (`chart::lag_text`) are other quantities with their own renders.
#[must_use]
pub(crate) fn real_span(secs: i64) -> String {
    let secs = secs.max(0);
    let (hours, minutes) = (secs / 3_600, (secs % 3_600) / 60);
    if hours > 0 {
        format!("{hours} h {minutes} min")
    } else if minutes > 0 {
        format!("{minutes} min")
    } else {
        "under a minute".to_owned()
    }
}

/// The civil date of a day count since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`, integer arithmetic throughout).
#[must_use]
pub(crate) fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days.saturating_add(719_468);
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe
        .saturating_add(era.saturating_mul(400))
        .saturating_add(i64::from(month <= 2));
    (year, month, day)
}

/// A unix second's UTC wall time, `HH:MM`.
#[must_use]
pub(crate) fn utc_hhmm(unix: i64) -> String {
    let of_day = unix.rem_euclid(86_400);
    format!("{:02}:{:02}", of_day / 3_600, (of_day % 3_600) / 60)
}

/// The day word for `at` seen from `now` (both unix seconds, UTC): `today`,
/// `tomorrow`, or `on {YYYY-MM-DD}`.
#[must_use]
pub(crate) fn utc_day_word(now: i64, at: i64) -> String {
    let (today, then) = (now.div_euclid(86_400), at.div_euclid(86_400));
    match then.saturating_sub(today) {
        0 => "today".to_owned(),
        1 => "tomorrow".to_owned(),
        _ => {
            let (year, month, day) = civil_from_days(then);
            format!("on {year:04}-{month:02}-{day:02}")
        }
    }
}

/// The real seconds until IC instant `target`, at the world's live rate
/// (`ceil((target − now) / rate)`); `None` while the clock is stopped.
#[must_use]
pub(crate) fn real_until(w: &World, target_ic: i64) -> Option<i64> {
    let rate = i64::from(w.world_timeline().current().rate);
    if rate == 0 {
        return None;
    }
    let ic = target_ic.saturating_sub(w.world_ic_now().at).max(0);
    Some(ic.saturating_add(rate - 1) / rate)
}

/// `(hh:mm, day word)` of the real instant IC instant `target` falls at; `None`
/// while the clock is stopped.
fn eta(w: &World, target_ic: i64) -> Option<(String, String)> {
    let real = real_until(w, target_ic)?;
    let at = w.now().saturating_add(real);
    Some((utc_hhmm(at), utc_day_word(w.now(), at)))
}

/// A mass in kilograms as tonnes to one decimal, half up (`142_184` → `142.2`).
#[must_use]
pub(crate) fn tonnes_one_decimal(kg: u64) -> String {
    let tenths = kg.saturating_add(50) / 100;
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// **A payload over its rail, in tonnes to one decimal** (doc 18 cut IC14 row 22; lead
/// ruling L18): when `kg` exceeds `ceiling_kg` the tenths round **up**, so a payload
/// over the rating never reads equal to it (181,001 kg against 181,000 reads `181.1`,
/// not `181.0`); otherwise [`tonnes_one_decimal`].
#[must_use]
pub(crate) fn tonnes_over(kg: u64, ceiling_kg: u64) -> String {
    if kg > ceiling_kg {
        let tenths = kg.div_ceil(100);
        format!("{}.{}", tenths / 10, tenths % 10)
    } else {
        tonnes_one_decimal(kg)
    }
}

// ---- lane geography ----------------------------------------------------------------

/// The lane pack's place id of world object `obj` (a reverse lookup over the
/// latched lane's placed map).
fn place_id_of(w: &World, obj: ObjId) -> Option<CanonicalId> {
    let store = w.module_state::<TacsimStore>()?;
    let lane = CanonicalId::new(LANE_PACK).ok()?;
    let version = store.applied_in(w, &lane)?;
    store
        .placed_in(w, &lane, version)?
        .into_iter()
        .find(|(_, placed)| *placed == obj)
        .map(|(id, _)| id)
}

/// A waypoint's [`WAYPOINT_DETAILS`] clause, or nothing.
fn waypoint_detail(w: &World, obj: ObjId) -> &'static str {
    let Some(id) = place_id_of(w, obj) else {
        return "";
    };
    WAYPOINT_DETAILS
        .iter()
        .find(|(place, _)| *place == id.as_str())
        .map_or("", |(_, detail)| detail)
}

/// A waypoint's [`WAYPOINT_SHORT`] word, else its display name.
fn waypoint_short(w: &World, obj: ObjId) -> String {
    place_id_of(w, obj)
        .and_then(|id| {
            WAYPOINT_SHORT
                .iter()
                .find(|(place, _)| *place == id.as_str())
                .map(|(_, short)| (*short).to_owned())
        })
        .unwrap_or_else(|| name_of(w, obj))
}

/// The lane's port entries with their maps.
pub(crate) fn lane_ports(w: &World) -> Vec<(&PortDef, Option<&PortMap>)> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    let Ok(lane) = CanonicalId::new(LANE_PACK) else {
        return Vec::new();
    };
    let (_, ports) = report_ports(store, w);
    ports
        .into_iter()
        .map(|(entry, def)| {
            let map = match store.port_map(&lane, &entry) {
                crate::port::PortMapState::Present(map) => Some(map),
                crate::port::PortMapState::Absent | crate::port::PortMapState::Unreadable => None,
            };
            (def, map)
        })
        .collect()
}

/// The port whose venue is `venue`: its name and its map.
fn port_of_venue<'a>(w: &'a World, venue: &CanonicalId) -> Option<(String, Option<&'a PortMap>)> {
    lane_ports(w)
        .into_iter()
        .find(|(def, _)| def.venue == *venue)
        .map(|(def, map)| (port_name(def), map))
}

/// The port whose map's domain host is `host`: its name and map.
fn port_of_host(w: &World, host: ObjId) -> Option<(String, &PortMap)> {
    lane_ports(w).into_iter().find_map(|(def, map)| {
        let map = map?;
        (map.host == Some(host.0)).then(|| (port_name(def), map))
    })
}

/// A venue's player-facing port name, else the venue word.
pub(crate) fn venue_name(w: &World, venue: &CanonicalId) -> String {
    port_of_venue(w, venue).map_or_else(|| venue.to_string(), |(name, _)| name)
}

// ---- the passage renders -----------------------------------------------------------

/// **Where a hull's passage stands** — the renders' one read (doc 18 §4.4's
/// lifecycle: departing, then under way).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Passage {
    /// A clearance is filed and the lock is sealing.
    Departing {
        /// The destination port's name.
        port: String,
    },
    /// On a journey.
    UnderWay {
        /// The destination port's name.
        port: String,
        /// The leg being flown, `min(cursor + 1, m)`.
        n: usize,
        /// The route's legs.
        m: usize,
        /// The journey's arrival instant (IC).
        arrival_at: Option<i64>,
        /// The last passed waypoint's short word, once a boundary is passed.
        astern: Option<String>,
    },
}

/// **The hull's passage**: under way when its journey is, else departing when a
/// clearance is filed, else `None`.
#[must_use]
pub(crate) fn passage_of(w: &World, hull: ObjId) -> Option<Passage> {
    if let Some(journey) = w
        .module_state::<mush_sim::JourneyBook>()
        .and_then(|book| book.get(hull))
        .filter(|journey| journey.state().is_under_way())
    {
        let legs = journey.route().legs();
        let m = legs.len();
        let cursor = usize::try_from(journey.cursor()).unwrap_or(usize::MAX);
        let n = cursor.saturating_add(1).min(m);
        let destination = journey.route().endpoints().1;
        let port =
            port_of_host(w, destination).map_or_else(|| name_of(w, destination), |(name, _)| name);
        let astern = cursor
            .checked_sub(1)
            .and_then(|last| legs.get(last))
            .map(|leg| waypoint_short(w, leg.to()));
        return Some(Passage::UnderWay {
            port,
            n,
            m,
            arrival_at: journey.arrival_at(),
            astern,
        });
    }
    pending_of(w, hull).map(|pending| Passage::Departing {
        port: venue_name(w, &pending.to_venue),
    })
}

/// `+ship`'s first line for a hull on passage (`{name}`, `{class}` left for the caller).
#[must_use]
pub(crate) fn ship_passage_line(w: &World, passage: &Passage) -> String {
    match passage {
        Passage::Departing { port } => SHIP_CLEARED.replace("{port}", port),
        Passage::UnderWay {
            port,
            n,
            m,
            arrival_at,
            ..
        } => {
            let remaining = arrival_at.map_or(0, |at| at.saturating_sub(w.world_ic_now().at));
            let base = SHIP_UNDER_WAY
                .replace("{port}", port)
                .replace("{n}", &n.to_string())
                .replace("{m}", &m.to_string())
                .replace("{icdays}", &ic_days(remaining));
            match arrival_at.and_then(|at| eta(w, at)) {
                Some((hhmm, day)) => base.replace("{hhmm}", &hhmm).replace("{day}", &day),
                None => base.replace(", about {hhmm} UTC {day}", ""),
            }
        }
    }
}

/// The hull room's status line for a hull on passage (`{name}`, `{mark}` left
/// for the caller).
#[must_use]
pub(crate) fn status_passage_line(w: &World, passage: &Passage) -> String {
    match passage {
        Passage::Departing { port } => STATUS_CLEARED.replace("{port}", port),
        Passage::UnderWay {
            n,
            m,
            arrival_at,
            astern,
            ..
        } => {
            let astern = astern
                .as_ref()
                .map_or_else(String::new, |short| format!(", {short} astern"));
            let base = STATUS_UNDER_WAY
                .replace("{n}", &n.to_string())
                .replace("{m}", &m.to_string())
                .replace("{astern}", &astern);
            match arrival_at.and_then(|at| eta(w, at)) {
                Some((hhmm, _)) => base.replace("{hhmm}", &hhmm),
                None => base.replace("{hhmm} UTC", "unknown"),
            }
        }
    }
}

// ---- the narration (the carrier hooks and the launch) ------------------------------

/// Every walkable room of a programme hull: its template rooms but the store,
/// which no exit reaches.
pub(crate) fn walkable_rooms(view: &HullView<'_>) -> Vec<ObjId> {
    let store = view.stowage();
    view.row
        .rooms
        .values()
        .map(|obj| ObjId(*obj))
        .filter(|room| Some(*room) != store)
        .collect()
}

/// Send one unattributed line to every walkable room of `hull` (a programme
/// hull; nothing otherwise).
pub(crate) fn narrate_hull(ctx: &mut Ctx<'_>, hull: ObjId, text: &str) {
    let rooms = match hull_view(ctx.w, hull) {
        Some(view) => walkable_rooms(&view),
        None => return,
    };
    for room in rooms {
        ctx.out.to_room_from(
            room,
            Vec::new(),
            hull,
            mush_proto::OutputEvent::Narrative {
                kind: mush_proto::NarrativeKind::Emit,
                actor: None,
                text: mush_proto::RichText::new().text(text.to_owned()),
            },
        );
    }
}

/// [`BOUNDARY`] for a live boundary: the place just reached (`legs[reached − 1]`),
/// its detail, and the leg now flown (`reached + 1` of `m`). `None` past the route.
#[must_use]
pub(crate) fn boundary_line(
    w: &World,
    journey: &mush_sim::CarrierJourney,
    reached: u32,
) -> Option<String> {
    let legs = journey.route().legs();
    let index = usize::try_from(reached).ok()?.checked_sub(1)?;
    let place = legs.get(index)?.to();
    Some(
        BOUNDARY
            .replace("{waypoint}", &name_of(w, place))
            .replace("{detail}", waypoint_detail(w, place))
            .replace("{n}", &reached.saturating_add(1).to_string())
            .replace("{m}", &legs.len().to_string()),
    )
}

/// [`DOCKS`] and [`ARRIVAL_NOTICE`] (without its time) for a hull's arrival: the
/// destination port's name and its gallery.
#[must_use]
pub(crate) fn arrival_lines(w: &World, journey: &mush_sim::CarrierJourney) -> (String, String) {
    let hull = journey.root();
    let name = name_of(w, hull);
    let destination = journey.route().endpoints().1;
    // Doc 20 cut SM.2 (§1 beat 3): a site's pad — the hull settles, and the stores line
    // (computed world-only from the filing's arrival row; at arrival the effective
    // stores equal the row's locker) follows. The notice is the once-per-stay
    // short-rations notice (decision 8).
    let site = lane_ports(w).into_iter().find_map(|(def, map)| {
        let map = map?;
        (def.site.is_some() && map.host == Some(destination.0)).then(|| port_name(def))
    });
    if let Some(port) = site {
        let stores = match w
            .module_state::<TacsimStore>()
            .map_or(VoyageState::Absent, |store| store.voyage_state(hull))
        {
            // Read at the landing instant itself: the hook may run a tick past it, and
            // nothing of the stay is owed yet (⌈0⌉ days), so the figure is the row's locker.
            VoyageState::Present(voyage) => voyage.arrival.as_ref().and_then(|row| {
                let since = journey.arrival_at()?;
                crate::port::site::stores_line(
                    w,
                    row.locker,
                    row.souls,
                    row.home_crew_days,
                    since,
                    since,
                )
            }),
            VoyageState::Absent | VoyageState::Unreadable => None,
        };
        let tail = stores.map_or_else(String::new, |line| format!(" {line}"));
        let settles = format!(
            "{}{tail}",
            SETTLES.replace("{name}", &name).replace("{port}", &port)
        );
        let notice = format!(
            "{}{tail}",
            SETTLED_NOTICE
                .replace("{name}", &name)
                .replace("{port}", &port)
        );
        return (settles, notice);
    }
    let (port, gallery) = match port_of_host(w, destination) {
        Some((port, map)) => (
            port,
            map.presentation
                .map_or_else(String::new, |g| name_of(w, ObjId(g))),
        ),
        None => (name_of(w, destination), String::new()),
    };
    let docks = DOCKS.replace("{name}", &name).replace("{port}", &port);
    let notice = ARRIVAL_NOTICE
        .replace("{name}", &name)
        .replace("{port}", &port)
        .replace("{gallery}", &gallery);
    (docks, notice)
}

/// `{HH:MM} UTC ` for a notice queued now.
#[must_use]
pub(crate) fn utc_prefix(w: &World) -> String {
    format!("{} UTC ", utc_hhmm(w.now()))
}

/// Queue `line` for the hull's tenant when they are not connected (lead ruling
/// L24: a connected pilot is aboard and hears the narration). Cut IC13: the tenant
/// is whoever commands her, a graduate title holder included (`helm_holder`).
pub(crate) fn tell_absent_tenant(ctx: &mut Ctx<'_>, hull: ObjId, line: &str) {
    let Some(tenant) = crate::port::coop::helm_holder(ctx.w, hull) else {
        return;
    };
    if !ctx.w.is_connected(tenant) {
        queue_notice(ctx, tenant, line);
    }
}

// ---- the goods-not-yet-aboard rule (lead ruling L13) ------------------------------

/// What promised matter holds a hull at its berth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Pending {
    /// An accepted quote whose goods are owed into the hold.
    Quote(QuoteId),
    /// A hold-backed listing whose stock sits in the hold, live or lapsed.
    Listing(ListingId),
    /// Doc 18 cut IC10: a sale into a bid whose goods have not yet left the hold.
    Sale(QuoteId),
}

/// **Goods not yet aboard** (lead ruling L13; mush-market unchanged, reviewer F9),
/// the lowest id first: (1) a quote whose paid lot is bound for `hold`, not
/// settled, whose contract stands non-terminal in the economy's book and whose
/// matter has not landed (a Draw not yet drawn, or a hold-backed consume, which
/// lands with its settlement); then (2) a hold-backed listing whose matter sits in
/// `hold`, live or lapsed. Both `+ship/undock` and IC11's depart ask this one
/// function.
///
/// **Doc 18 cut IC10 (decision 14)** adds the mirror of the first rule: (0) a sale
/// into a bid out of `hold` — activated, unsettled, its contract non-terminal, inside
/// its cutoff `draw_by` (past it the leg is never offered, so nothing can leave and
/// the hull is not held), and whose `inventory.transferred` marker inventory's mirror
/// has not seen — holds the hull too ([`Pending::Sale`]): the burn's payload and the
/// hold must not change under a filed passage. After a landing the hull waits at most
/// one inventory sweep for the mirror.
#[must_use]
pub(crate) fn goods_pending_aboard(w: &World, hold: EndpointRef) -> Option<Pending> {
    let market = w.module_state::<MarketStore>()?;
    let economy = w.module_state::<EconomyStore>();
    let mirror = w.module_state::<mush_inventory::InventoryStore>();
    let marker_seen = |marker: Option<&smol_str::SmolStr>| {
        marker.is_some_and(|m| mirror.is_some_and(|s| s.marker(m).is_some()))
    };
    for quote in market.quotes() {
        let Some(lot) = &quote.lot else {
            continue;
        };
        // ---- (0) doc 18 cut IC10: a sale's goods still leaving this hold ----
        if lot.kind == PaidKind::Deliver
            && lot.source.vault() == hold.vault()
            && quote.activated
            && quote.settled.is_none()
            && lot.draw_by.is_some_and(|by| w.now() <= by)
            && !marker_seen(lot.marker.as_ref())
            && quote.contract.is_some_and(|contract| {
                economy
                    .and_then(|e| {
                        e.snapshot()
                            .contract(contract)
                            .map(|c| !c.state.is_terminal())
                    })
                    .unwrap_or(false)
            })
        {
            return Some(Pending::Sale(quote.id));
        }
        if lot.destination.vault() != hold.vault() || quote.settled.is_some() {
            continue;
        }
        let Some(contract) = quote.contract else {
            continue;
        };
        let standing = economy
            .and_then(|e| {
                e.snapshot()
                    .contract(contract)
                    .map(|c| !c.state.is_terminal())
            })
            .unwrap_or(false);
        if !standing {
            continue;
        }
        let landed = match lot.kind {
            PaidKind::Draw => quote.drawn_at.is_some(),
            PaidKind::Hold => false,
            // A sale's destination is the house's ungoverned storefront, so it never
            // matches a hold in practice; read the same mirror all the same.
            PaidKind::Deliver => marker_seen(lot.marker.as_ref()),
        };
        if !landed {
            return Some(Pending::Quote(quote.id));
        }
    }
    market
        .lot_listings()
        .find_map(|listing| match market.lot_matter(listing) {
            Some(LotMatter::Hold { endpoint, .. }) if endpoint.vault() == hold.vault() => {
                Some(Pending::Listing(listing.id))
            }
            Some(LotMatter::Hold { .. } | LotMatter::Draw { .. }) | None => None,
        })
}

// ---- planning --------------------------------------------------------------------

/// **How a passage is to be flown** (doc 19 cut A.2, D-19-9, D-19-13): the
/// throttle, the route policy and a waypoint the route must call at. The
/// defaults are full thrust, the fastest listed route and no waypoint — the
/// slice's passage exactly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassageOptions {
    /// Permille of the hull's thrust (`MIN_THROTTLE_PERMILLE..=MAX_THROTTLE_PERMILLE`).
    pub throttle_permille: u32,
    /// Which listed route to keep.
    pub policy: RoutePolicy,
    /// The lane place the route must call at before its destination.
    pub via: Option<CanonicalId>,
}

impl Default for PassageOptions {
    fn default() -> PassageOptions {
        PassageOptions {
            throttle_permille: DEFAULT_THROTTLE_PERMILLE,
            policy: RoutePolicy::default(),
            via: None,
        }
    }
}

impl PassageOptions {
    /// A queued depart's options: its throttle when it names one, its waypoint,
    /// the fastest route (the queue's policy).
    #[must_use]
    pub fn for_order(throttle_permille: Option<u32>, via: Option<CanonicalId>) -> PassageOptions {
        PassageOptions {
            throttle_permille: throttle_permille.unwrap_or(DEFAULT_THROTTLE_PERMILLE),
            policy: RoutePolicy::default(),
            via,
        }
    }
}

/// [`THROTTLE_USAGE`] with the band filled in.
pub(crate) fn throttle_usage() -> String {
    THROTTLE_USAGE
        .replace("{min}", &MIN_THROTTLE_PERMILLE.to_string())
        .replace("{max}", &MAX_THROTTLE_PERMILLE.to_string())
}

/// `throttle=`'s value: a whole permille in the band.
///
/// # Errors
///
/// [`throttle_usage`].
pub(crate) fn parse_throttle(text: &str) -> Result<u32, String> {
    let permille: u32 = text.trim().parse().map_err(|_| throttle_usage())?;
    if !(MIN_THROTTLE_PERMILLE..=MAX_THROTTLE_PERMILLE).contains(&permille) {
        return Err(throttle_usage());
    }
    Ok(permille)
}

/// **A lane place by its id or a prefix of its label** (doc 19 cut A.2's `via=`),
/// over the latched lane's `location` entries.
///
/// # Errors
///
/// [`VIA_UNKNOWN`] naming the places, or the layer's own absence.
pub(crate) fn match_waypoint(w: &World, text: &str) -> Result<CanonicalId, String> {
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| "the layer's state is absent".to_owned())?;
    let (pack, _) = report_ports(store, w);
    let pack = pack.ok_or_else(|| "no lane pack is loaded".to_owned())?;
    let places: Vec<(&CanonicalId, &crate::pack::LocationDef)> = pack
        .entries()
        .iter()
        .filter_map(|(id, entry)| match entry {
            crate::pack::PackEntry::Location(def) => Some((id, def)),
            _ => None,
        })
        .collect();
    let lower = text.trim().to_lowercase();
    let found = (!lower.is_empty())
        .then(|| {
            places
                .iter()
                .find(|(id, _)| id.as_str().eq_ignore_ascii_case(&lower))
                .or_else(|| {
                    places
                        .iter()
                        .find(|(_, def)| def.label.to_lowercase().starts_with(&lower))
                })
        })
        .flatten();
    match found {
        Some((id, _)) => Ok((*id).clone()),
        None => {
            let list: Vec<String> = places.iter().map(|(_, def)| def.label.clone()).collect();
            Err(VIA_UNKNOWN
                .replace("{via}", text.trim())
                .replace("{list}", &list.join(", ")))
        }
    }
}

/// A lane place's label, for a line that names a waypoint; the id when the
/// lane does not know it.
pub(crate) fn waypoint_label(w: &World, place: &CanonicalId) -> String {
    let label = w
        .module_state::<TacsimStore>()
        .and_then(|store| report_ports(store, w).0)
        .and_then(|pack| match pack.entry(place) {
            Some(crate::pack::PackEntry::Location(def)) => Some(def.label.clone()),
            _ => None,
        });
    label.unwrap_or_else(|| place.to_string())
}

/// The parsed arguments of `+ship/plan` and `+ship/undock`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct PlanArgs {
    /// The destination word (a routes key, or a prefix of the port's name).
    word: Option<String>,
    /// `throttle=<permille>`.
    throttle: Option<u32>,
    /// `policy=fastest|cheapest`.
    policy: Option<RoutePolicy>,
    /// `via=<place>`, unresolved.
    via: Option<String>,
}

/// The arguments `+ship/plan` and `+ship/undock` take: a destination word (a
/// routes key, or a prefix of the destination port's name), `at=now`, and since
/// doc 19 cut A.2 `throttle=<permille>`, `policy=fastest|cheapest` and
/// `via=<place>`.
fn parse_args(raw: &str) -> Result<PlanArgs, CmdError> {
    let mut args = PlanArgs::default();
    let mut words: Vec<&str> = Vec::new();
    for token in raw.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if let Some(via) = token.get(4..).filter(|_| lower.starts_with("via=")) {
            if args.via.is_some() || via.is_empty() {
                return Err(CmdError::msg(
                    VIA_UNKNOWN
                        .replace("{via}", via)
                        .replace("{list}", "the lane's places"),
                ));
            }
            args.via = Some(via.to_owned());
            continue;
        }
        if let Some(value) = lower.strip_prefix("throttle=") {
            if args.throttle.is_some() {
                return Err(CmdError::msg(throttle_usage()));
            }
            args.throttle = Some(parse_throttle(value).map_err(CmdError::msg)?);
            continue;
        }
        if let Some(value) = lower.strip_prefix("policy=") {
            let policy = RoutePolicy::parse(value).ok_or_else(|| CmdError::msg(POLICY_USAGE))?;
            if args.policy.is_some() {
                return Err(CmdError::msg(POLICY_USAGE));
            }
            args.policy = Some(policy);
            continue;
        }
        if let Some(at) = lower.strip_prefix("at=") {
            if at != "now" {
                return Err(CmdError::msg(AT_NOT_YET));
            }
            continue;
        }
        words.push(token);
    }
    args.word = (!words.is_empty()).then(|| words.join(" "));
    Ok(args)
}

/// The options the parsed arguments ask for, the waypoint resolved against the
/// lane.
fn options_of(w: &World, args: &PlanArgs) -> Result<PassageOptions, CmdError> {
    let via = match &args.via {
        Some(text) => Some(match_waypoint(w, text).map_err(CmdError::msg)?),
        None => None,
    };
    Ok(PassageOptions {
        throttle_permille: args.throttle.unwrap_or(DEFAULT_THROTTLE_PERMILLE),
        policy: args.policy.unwrap_or_default(),
        via,
    })
}

/// The basis flown under `options`.
pub(crate) fn apply_options(mut basis: Basis, options: &PassageOptions) -> Basis {
    basis.spec.drive.throttle_permille = options.throttle_permille;
    basis.policy = options.policy;
    basis.via = options.via.clone();
    basis
}

/// One destination a berth clears for: its venue word, its port name and its
/// route list (D-18-47).
#[derive(Clone, Debug)]
pub(crate) struct Destination {
    pub(crate) venue: CanonicalId,
    pub(crate) port: String,
    pub(crate) routes: Vec<CanonicalId>,
}

/// The destinations `def` clears for, in its `routes` map order.
pub(crate) fn destinations(w: &World, def: &PortDef) -> Vec<Destination> {
    def.routes
        .iter()
        .map(|(venue, routes)| Destination {
            venue: venue.clone(),
            port: venue_name(w, venue),
            routes: routes.clone(),
        })
        .collect()
}

/// The destination `word` names: a routes key, case-insensitively, or a
/// case-insensitive prefix of its port's name.
pub(crate) fn match_destination<'d>(
    dests: &'d [Destination],
    word: &str,
) -> Option<&'d Destination> {
    let word = word.trim().to_lowercase();
    if word.is_empty() {
        return None;
    }
    dests
        .iter()
        .find(|d| d.venue.as_str().eq_ignore_ascii_case(&word))
        .or_else(|| {
            dests
                .iter()
                .find(|d| d.port.to_lowercase().starts_with(&word))
        })
}

/// **The destination a queued depart names, exactly** (doc 18 cut IC14 row 25): the
/// routes key equal to the stored canonical venue word, case-sensitively — never the
/// live verb's prefix fallback, so a stored word that is only a prefix of a port's name
/// (a plan written before a port was renamed, or a hand-edited one) resolves to
/// nothing and holds `NO_ROUTE` rather than flying somewhere else.
fn match_destination_exact<'d>(dests: &'d [Destination], word: &str) -> Option<&'d Destination> {
    dests.iter().find(|d| d.venue.as_str() == word)
}

/// [`NO_ROUTE`] for a berth.
fn no_route(here: &str, dests: &[Destination]) -> String {
    let list: Vec<&str> = dests.iter().map(|d| d.port.as_str()).collect();
    NO_ROUTE
        .replace("{here}", here)
        .replace("{list}", &list.join(", "))
}

/// The persons `Interior`-placed in `airlock` (lead ruling L12).
pub(crate) fn occupants(w: &World, airlock: ObjId) -> u32 {
    u32::try_from(souls_aboard(w, airlock).len()).unwrap_or(u32::MAX)
}

/// **Who is aboard**: the persons (players and NPCs) `Interior`-placed in `airlock`,
/// in the placement index's order — the souls [`occupants`] counts, and (doc 18 cut
/// IC11, lead ruling L11) a queued depart's listed passengers at filing.
pub(crate) fn souls_aboard(w: &World, airlock: ObjId) -> Vec<ObjId> {
    let Some(st) = w.module_state::<SpatialState>() else {
        return Vec::new();
    };
    st.derived.children.get(&airlock).map_or_else(Vec::new, |children| {
        children
            .iter()
            .copied()
            .filter(|obj| {
                w.get(*obj)
                    .is_some_and(|o| matches!(o.kind, ObjKind::Player | ObjKind::Npc))
                    && matches!(st.parent_of(*obj), SpatialParent::Interior { space, .. } if space == airlock)
            })
            .collect()
    })
}

/// **`person` is aboard `hull`**: `Interior`-placed in the hull's airlock space
/// (any member room). The one predicate `+ship/undock`, the burn's offer and the
/// launch share (lead rulings L12, L14: a burned hull never launches with its
/// pilot ashore, whatever moved them).
pub(crate) fn aboard(w: &World, hull: ObjId, person: ObjId) -> bool {
    let Some(airlock) = hull_view(w, hull).and_then(|view| view.airlock()) else {
        return false;
    };
    w.module_state::<SpatialState>().is_some_and(|st| {
        matches!(st.parent_of(person), SpatialParent::Interior { space, .. } if space == airlock)
    })
}

/// What a passage is planned on: everything read from the world at the berth.
#[derive(Clone, Debug)]
pub(crate) struct Basis {
    /// The latched lane version.
    pub(crate) version: u32,
    /// The hull's unit definition digest (L34).
    pub(crate) digest: DefinitionDigest,
    /// The hull spec (drive, dry mass, allowance, accepted goods).
    pub(crate) spec: HullSpec,
    /// The hold vault.
    pub(crate) vault: VaultId,
    /// Souls aboard.
    pub(crate) occupants: u32,
    /// The payload, kilograms: the hold's and, since doc 20 cut SM.3a, the bay's
    /// rigs ([`Basis::carried`]).
    pub(crate) payload: u64,
    /// **Doc 20 cut SM.3a**: the rigs in the hull's bay, as the payload counts them.
    pub(crate) carried: Vec<crate::port::passage::CarriedRig>,
    /// The propellant pool, tonnes.
    pub(crate) tank: u64,
    /// The provisions pool, crew-days.
    pub(crate) locker: u64,
    /// The IC second the plan departs at.
    pub(crate) depart_ic: i64,
    /// Which listed route to keep (doc 19 cut A.2, D-19-13).
    pub(crate) policy: RoutePolicy,
    /// A waypoint the route must call at, when the captain named one.
    pub(crate) via: Option<CanonicalId>,
}

/// **What the rigs in `view`'s bay weigh** (doc 20 cut SM.3a; §4.3, D-20-7): the
/// rows [`crate::port::vehicle::carried_rigs`] lists and their weight through
/// [`crate::port::passage::carried_kg`], each hopper read through
/// [`read_pool`]. A hull with no bay carries nothing.
///
/// # Errors
///
/// A rig that cannot be described, or a hopper that will not read.
pub(crate) fn carried_of(
    ctx: &Ctx<'_>,
    view: &HullView<'_>,
) -> Result<(Vec<crate::port::passage::CarriedRig>, u64), String> {
    let w: &World = ctx.w;
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(|| "the goods table is not loaded".to_owned())?;
    let rigs = crate::port::vehicle::carried_rigs(w, view, goods)?;
    let kg = crate::port::passage::carried_kg(goods, &rigs, |hopper, good| {
        read_pool(ctx, EndpointRef(VaultId(hopper)), good).map_err(|error| PlanError::Unreadable {
            what: format!("a hopper's {good} ({error:?})"),
        })
    })
    .map_err(|e| e.to_string())?;
    Ok((rigs, kg))
}

/// Read the basis of a passage for `view`'s hull, `occupants` souls aboard.
///
/// Doc 20 cut SM.3a: the hull's **own** unit (`HullView::unit`, the Dray-P on a
/// refitted hull) is planned, and the payload adds the rigs in its bay
/// ([`carried_of`]). A hull whose unit record boot held plans nothing
/// (`PlanError::Unreadable`; reviewer F6).
pub(crate) fn basis(ctx: &Ctx<'_>, view: &HullView<'_>, occupants: u32) -> Result<Basis, String> {
    if view.unit_held {
        return Err(PlanError::Unreadable {
            what: format!("hull #{}'s unit record (held at boot)", view.hull().0),
        }
        .to_string());
    }
    let w: &World = ctx.w;
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| "the layer's state is absent".to_owned())?;
    let goods = store
        .goods()
        .ok_or_else(|| "the goods table is not loaded".to_owned())?;
    let lane = CanonicalId::new(LANE_PACK).map_err(|e| e.to_string())?;
    let version = store
        .applied_in(w, &lane)
        .ok_or_else(|| "the lane is not latched".to_owned())?;
    let (digest, spec) = hull_spec_of(store.definitions(), goods, &view.unit)?;
    let vault = hold_vault(w, view).ok_or_else(|| "the hull has no hold vault".to_owned())?;
    let read = |good: &CanonicalId| {
        read_pool(ctx, EndpointRef(vault), good).map_err(|error| PlanError::Unreadable {
            what: format!("the hold's {good} ({error:?})"),
        })
    };
    let (carried, carried_weight) = carried_of(ctx, view)?;
    let payload = payload_kg(goods, &spec.accepted, read, occupants, spec.allowance_kg)
        .map_err(|e| e.to_string())?
        .checked_add(carried_weight)
        .ok_or_else(|| PlanError::Overflow.to_string())?;
    let pool_of = |kind: Consumable| -> Result<u64, String> {
        match consumable_good(goods, kind) {
            Some(good) => read(&good).map_err(|e| e.to_string()),
            None => Err(format!("the goods table names no {kind:?}")),
        }
    };
    Ok(Basis {
        version,
        digest,
        spec,
        vault,
        occupants,
        payload,
        carried,
        tank: pool_of(Consumable::Propellant)?,
        locker: pool_of(Consumable::Provisions)?,
        depart_ic: w.world_ic_now().at,
        policy: RoutePolicy::default(),
        via: None,
    })
}

/// Plan the fastest listed route on `basis` over the latched lane's sky (§4.4:
/// one planner over the sky at departure; lead ruling L23: the lane addressed by
/// its latched `(pack, version)`).
pub(crate) fn plan_on(
    w: &World,
    basis: &Basis,
    routes: &[CanonicalId],
) -> Result<PassagePlan, PlanError> {
    let pack = lane_pack(w, basis.version)?;
    let sky = lane_sky(w, pack)?;
    plan_on_sky((&sky, pack), basis, routes)
}

/// [`plan_on`] over a sky and lane the caller built once (doc 20 cut SM.2's fixer, the
/// reviewer's SM2-FX-2): a caller that plans many readings — the throttle hint's bisection
/// and its window scan, `+ship/plan`'s Window line — builds the lane's [`Sky`] once
/// ([`lane_sky_of`]) instead of once a reading. `lane` is `(sky, pack)` for the basis's
/// latched version; the answer is [`plan_on`]'s.
///
/// # Errors
///
/// Any planner refusal.
pub(crate) fn plan_on_sky(
    lane: (&Sky, &crate::pack::RulesPack),
    basis: &Basis,
    routes: &[CanonicalId],
) -> Result<PassagePlan, PlanError> {
    let (sky, pack) = lane;
    choose_route(
        sky,
        pack,
        routes,
        &basis.spec.drive,
        basis.spec.dry_kg,
        basis.payload,
        basis.occupants,
        basis.depart_ic,
        basis.policy,
        basis.via.as_ref(),
    )
}

/// **The lane pack at `version`** (factored out of [`plan_on`] by doc 20 cut SM.2, for the
/// throttle hint's landing floor and the site reserve): the store's exact `(id, version)`.
///
/// # Errors
///
/// [`PlanError::Unreadable`] when the layer or that version is not loaded.
pub(crate) fn lane_pack(w: &World, version: u32) -> Result<&crate::pack::RulesPack, PlanError> {
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| PlanError::Unreadable {
            what: "the layer's state".to_owned(),
        })?;
    let lane = CanonicalId::new(LANE_PACK).map_err(|_| PlanError::Overflow)?;
    store
        .pack(&lane, version)
        .map(|loaded| &loaded.pack)
        .ok_or_else(|| PlanError::Unreadable {
            what: format!("the lane at version {version}"),
        })
}

/// **The latched lane at `version` and its sky** (the SM.2 fixer, SM2-FX-2): built once for
/// a caller that plans many readings ([`plan_on_sky`], [`site_reserve_on`]).
///
/// # Errors
///
/// [`PlanError::Unreadable`] when the layer or that version is not loaded.
pub(crate) fn lane_sky_of(
    w: &World,
    version: u32,
) -> Result<(Sky, &crate::pack::RulesPack), PlanError> {
    let pack = lane_pack(w, version)?;
    Ok((lane_sky(w, pack)?, pack))
}

/// The sky over `pack` under this world's ephemeris.
fn lane_sky(w: &World, pack: &crate::pack::RulesPack) -> Result<Sky, PlanError> {
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| PlanError::Unreadable {
            what: "the layer's state".to_owned(),
        })?;
    Ok(Sky::from_pack(pack, &store.config().time.ephemeris))
}

// ---- doc 20 cut SM.2: sites --------------------------------------------------------

/// The lane's port entry whose venue is `venue`, with its entry id, when it is a site.
pub(crate) fn site_port<'a>(
    w: &'a World,
    venue: &CanonicalId,
) -> Option<(CanonicalId, &'a PortDef)> {
    let store = w.module_state::<TacsimStore>()?;
    let (_, ports) = report_ports(store, w);
    ports
        .into_iter()
        .find(|(_, def)| def.venue == *venue && def.site.is_some())
}

/// The kilograms one unit of a consumable weighs (the goods table's mass measure).
fn consumable_kg(goods: &crate::definitions::GoodsTable, kind: Consumable) -> Option<u64> {
    let mass = CanonicalId::new(MASS_KG).ok()?;
    let row = goods.good(&consumable_good(goods, kind)?)?;
    row.measures.get(&mass).copied()
}

/// **The leg home a departure for a site holds back** (doc 20 cut SM.2, decision 7): the
/// pure [`HomeReserve`](crate::port::passage::HomeReserve), named for this module's readers.
pub(crate) type Reserve = crate::port::passage::HomeReserve;

/// **The site reserve** (decision 7; §3.3's "the hop home is always plannable"): the
/// site's routes for the departure berth's venue (`from_venue`) if it lists it, else its
/// first listed routes key; the home payload is the basis's with the tank and the locker
/// at what the outbound burn and crew-days leave of them (none when short); the rest is
/// [`home_reserve`](crate::port::passage::home_reserve) over the latched lane's sky from
/// the passage's arrival, over `window_days` (every departure's is [`PLAN_WINDOW_DAYS`];
/// the throttle hint's `0` reads the arrival day alone, a lower bound of the reserve that
/// spares the window's plans for a throttle or a day that overruns on it already).
///
/// # Errors
///
/// [`PlanError::Landing`] when no home route lifts at the home payload, and any other
/// planner refusal.
pub(crate) fn site_reserve(
    w: &World,
    basis: &Basis,
    out: &PassagePlan,
    site: &PortDef,
    from_venue: &CanonicalId,
    window_days: i64,
) -> Result<Reserve, PlanError> {
    let (sky, pack) = lane_sky_of(w, basis.version)?;
    site_reserve_on(w, (&sky, pack), basis, out, site, from_venue, window_days)
}

/// [`site_reserve`] over a sky and lane the caller built once ([`lane_sky_of`]; the SM.2
/// fixer, SM2-FX-2): `lane` is `(sky, pack)` for the basis's latched version.
///
/// # Errors
///
/// [`site_reserve`]'s.
pub(crate) fn site_reserve_on(
    w: &World,
    lane: (&Sky, &crate::pack::RulesPack),
    basis: &Basis,
    out: &PassagePlan,
    site: &PortDef,
    from_venue: &CanonicalId,
    window_days: i64,
) -> Result<Reserve, PlanError> {
    let (sky, pack) = lane;
    let routes = site
        .routes
        .get(from_venue)
        .or_else(|| site.routes.values().next())
        .ok_or_else(|| PlanError::UnknownRoute {
            route: "(none listed)".to_owned(),
        })?;
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(|| PlanError::Unreadable {
            what: "the goods table".to_owned(),
        })?;
    let unreadable = |what: &str| PlanError::Unreadable {
        what: what.to_owned(),
    };
    let tank_kg =
        consumable_kg(goods, Consumable::Propellant).ok_or_else(|| unreadable("propellant"))?;
    let locker_kg =
        consumable_kg(goods, Consumable::Provisions).ok_or_else(|| unreadable("provisions"))?;
    // The home payload: the basis with the tank and the locker swapped for what is left of
    // each after the passage out — none when the passage out is short of either, since
    // the departure refuses then anyway (`SITE_TANK_SHORT`, `SITE_STORES_SHORT`) and the
    // reserve is still read for its line. Never "out × mass" off the payload: the
    // passage out can burn more than the tank aboard holds, and that subtraction would
    // underflow into a dark plan.
    let tank_left = basis.tank.saturating_sub(out.burn_t);
    let locker_left = basis.locker.saturating_sub(u64::from(out.crew_days));
    let home_payload = basis
        .tank
        .checked_mul(tank_kg)
        .and_then(|tank| basis.payload.checked_sub(tank))
        .and_then(|p| p.checked_sub(basis.locker.checked_mul(locker_kg)?))
        .and_then(|p| p.checked_add(tank_left.checked_mul(tank_kg)?))
        .and_then(|p| p.checked_add(locker_left.checked_mul(locker_kg)?))
        .ok_or(PlanError::Overflow)?;
    let arrive = basis
        .depart_ic
        .checked_add(out.total_s)
        .ok_or(PlanError::Overflow)?;
    crate::port::passage::home_reserve(
        sky,
        pack,
        routes,
        &basis.spec.drive,
        basis.spec.dry_kg,
        home_payload,
        basis.occupants,
        arrive,
        window_days,
    )
}

/// **The stay's charge on a plan from a site berth** (Q-20-7; lead ruling 5): at a site
/// whose landing instant reads, `landed_charge(souls, since, depart, locker, the plan's
/// own crew-days)` is added to the plan's crew-days and frozen as its
/// `landed_crew_days`. The one helper `+ship/plan` and the filing share, so the plan
/// read is the plan filed.
///
/// # Errors
///
/// [`PlanError::Overflow`] when the crew-days leave `u32`.
pub(crate) fn charge_stay(
    w: &World,
    hull: ObjId,
    berth: (&PortDef, &PortMap),
    basis: &Basis,
    plan: &mut PassagePlan,
) -> Result<(), PlanError> {
    let (def, map) = berth;
    if def.site.is_none() {
        return Ok(());
    }
    let Some(since) = crate::port::site::landed_since(w, hull, map) else {
        return Ok(());
    };
    let landed = crate::port::site::landed_charge(
        basis.occupants,
        since,
        basis.depart_ic,
        basis.locker,
        plan.crew_days,
    );
    plan.crew_days = plan
        .crew_days
        .checked_add(landed)
        .ok_or(PlanError::Overflow)?;
    plan.landed_crew_days = landed;
    Ok(())
}

/// [`LANDING_REFUSED`] for a refused landing: the site's label, its body's display
/// name, the hull and the TWR in hundredths, floored.
fn landing_line(w: &World, name: &str, place: &str, body: &str, twr_permille: u64) -> String {
    let port = CanonicalId::new(place)
        .map_or_else(|_| place.to_owned(), |place| waypoint_label(w, &place));
    let body = CanonicalId::new(body).map_or_else(
        |_| body.to_owned(),
        |body| crate::port::site::body_name(&body),
    );
    let hundredths = twr_permille / 10;
    LANDING_REFUSED
        .replace("{port}", &port)
        .replace("{body}", &body)
        .replace("{name}", name)
        .replace(
            "{twr}",
            &format!("{}.{:02}", hundredths / 100, hundredths % 100),
        )
}

/// **`+ship`'s stores line on a pad** (§1 beat 3; the SM.2 brief's item 11): the
/// effective stores from the live pools, the leg home being the plan `+ship/plan <home>`
/// would make now with no throttle word (the site's first listed routes key), the
/// landing instant from the hull's durable journey. `None` off a pad or when any read
/// fails.
pub(crate) fn settled_stores_line(ctx: &Ctx<'_>, view: &HullView<'_>) -> Option<String> {
    let w: &World = ctx.w;
    let Berth::Docked { def, map } = berth_of(w, view.hull()) else {
        return None;
    };
    // Only on a site's pad.
    def.site.as_ref()?;
    let since = crate::port::site::landed_since(w, view.hull(), map)?;
    let souls = view
        .airlock()
        .map_or(0, |airlock| occupants(w, airlock))
        .max(1);
    let basis = basis(ctx, view, souls).ok()?;
    let routes = def.routes.values().next()?;
    let home = plan_on(w, &basis, routes).ok()?;
    crate::port::site::stores_line(
        w,
        basis.locker,
        basis.occupants,
        home.crew_days,
        since,
        basis.depart_ic,
    )
}

/// The display names of a plan's intermediate places (the lane's location
/// labels), joined `, `.
fn waypoints(w: &World, plan: &PassagePlan) -> Option<String> {
    let store = w.module_state::<TacsimStore>()?;
    let lane = CanonicalId::new(LANE_PACK).ok()?;
    let version = store.applied_in(w, &lane)?;
    let pack = &store.pack(&lane, version)?.pack;
    let count = plan.legs.len().checked_sub(1)?;
    let names: Vec<String> = plan
        .legs
        .iter()
        .take(count)
        .map(|leg| match pack.entry(&leg.to) {
            Some(crate::pack::PackEntry::Location(location)) => location.label.clone(),
            _ => leg.to.to_string(),
        })
        .collect();
    (!names.is_empty()).then(|| names.join(", "))
}

/// `{n} t`, `{x} m³`, else `{n} {unit}` for a free amount in `measure`.
fn free_text(goods: &crate::definitions::GoodsTable, measure: &CanonicalId, value: u64) -> String {
    match measure.as_str() {
        MASS_KG => format!("{} t free", format_tonnes(value)),
        VOLUME_L => format!("{} m³ free", format_m3(value)),
        _ => format!("{value} {} free", gauges::measure_unit(goods, measure)),
    }
}

/// `+ship/plan`'s line 2 for one compartment: [`gauges::measures_line`] and, when a
/// limited measure is not full, the free amounts from [`capacity::load`].
fn slack_line(
    goods: &crate::definitions::GoodsTable,
    comp: &Compartment,
    contents: &BTreeMap<CanonicalId, u64>,
) -> Result<String, capacity::CapacityError> {
    let measures = gauges::measures_line(goods, comp, contents, true)?;
    let loaded = capacity::load(goods, comp, contents)?;
    let free: Vec<String> = comp
        .limits
        .iter()
        .filter_map(|(measure, limit)| {
            let used = loaded.get(measure).copied().unwrap_or(0);
            (used < *limit).then(|| free_text(goods, measure, limit.saturating_sub(used)))
        })
        .collect();
    let label = gauges::compartment_words(&comp.id).0;
    let mut chars = label.chars();
    let label = chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    });
    let tail = if free.is_empty() {
        String::new()
    } else {
        format!(" — {}", free.join(" and "))
    };
    Ok(format!("{label}: {measures}{tail}."))
}

/// `+ship/plan`'s line 3: every good this port's vendors ask, authored order,
/// that the hull accepts, with the units that still fit and the binding measures
/// (`capacity::compartment_for` + `max_more`, the market gate's own functions).
/// `None` when the board asks nothing the hull carries.
fn room_line(
    goods: &crate::definitions::GoodsTable,
    def: &PortDef,
    readings: &[gauges::Reading],
) -> Result<Option<String>, capacity::CapacityError> {
    let comps: Vec<Compartment> = readings.iter().map(|(_, comp, _)| comp.clone()).collect();
    let mut asked: Vec<&CanonicalId> = Vec::new();
    for ask in def.vendors.iter().flat_map(|vendor| vendor.asks.iter()) {
        if !asked.contains(&&ask.good) {
            asked.push(&ask.good);
        }
    }
    let mut parts: Vec<String> = Vec::new();
    for good in asked {
        let Some(comp) = capacity::compartment_for(goods, &comps, good) else {
            continue;
        };
        let Some(def) = goods.good(good) else {
            continue;
        };
        let contents = readings
            .iter()
            .find(|(id, _, _)| *id == comp.id)
            .map(|(_, _, contents)| contents.clone())
            .unwrap_or_default();
        let (more, bound) = capacity::max_more(goods, &comp, &contents, good)?;
        let words: Vec<String> = bound
            .iter()
            .map(|measure| gauges::measure_word(goods, measure))
            .collect();
        parts.push(format!(
            "{} {more} {} ({})",
            def.label,
            gauges::unit_word(def),
            words.join(" and ")
        ));
    }
    Ok((!parts.is_empty()).then(|| format!("Room on this board: {}.", parts.join(", "))))
}

/// The refusal for an actor with no live charter (`+ship`'s).
fn no_charter(w: &World, actor: ObjId) -> CmdError {
    ship::no_charter(w, actor)
}

// ---- +ship/plan ------------------------------------------------------------------

/// **`+ship/plan [<port>]`** (doc 18 §4.4; the IC9 brief's item 10): a dry run
/// that writes nothing — readable anywhere the pilot holds a live charter, the
/// hull docked.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
pub(crate) fn plan_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let args = parse_args(ctx.args.raw())?;
    let word = args.word.clone();
    let options = options_of(ctx.w, &args)?;
    let Some(charter) = helm_of(ctx.w, actor) else {
        return Err(no_charter(ctx.w, actor));
    };
    let lines = {
        let w: &World = ctx.w;
        let Some(view) = hull_view(w, charter.hull) else {
            return Err(no_charter(w, actor));
        };
        let name = name_of(w, charter.hull);
        let def = match berth_of(w, charter.hull) {
            Berth::UnderWay => {
                return Err(CmdError::msg(PLAN_UNDER_WAY.replace("{name}", &name)));
            }
            Berth::Elsewhere => return Err(CmdError::msg(NOT_DOCKED.replace("{name}", &name))),
            Berth::Docked { def, .. } => def,
        };
        let here = port_name(def);
        let dests = destinations(w, def);
        let chosen: Vec<&Destination> = match &word {
            Some(word) => {
                vec![
                    match_destination(&dests, word)
                        .ok_or_else(|| CmdError::msg(no_route(&here, &dests)))?,
                ]
            }
            None => dests.iter().collect(),
        };
        // The pilot flies with the plan: a plan read ashore counts them aboard.
        let souls = view
            .airlock()
            .map_or(0, |airlock| occupants(w, airlock))
            .max(1);
        let basis = match basis(ctx, &view, souls) {
            Ok(basis) => apply_options(basis, &options),
            Err(error) => {
                tracing::warn!(hull = charter.hull.0, %error, "tacsim: +ship/plan could not read the passage's basis");
                return Err(CmdError::msg(plan_dark(&name)));
            }
        };
        let mut lines: Vec<String> = Vec::new();
        for dest in chosen {
            match plan_block(ctx, &view, def, &basis, dest) {
                Ok(block) => lines.extend(block),
                Err(error) => {
                    tracing::warn!(hull = charter.hull.0, %error, "tacsim: +ship/plan could not plan a passage");
                    lines.push(format!("Plan for {}:", dest.port));
                    lines.push(plan_dark(&name));
                }
            }
        }
        // Doc 18 cut IC11 (§4.4 "+ship/plan … a dry run of the queue"): the sailing
        // plan from its cursor, through the executor's own planning core.
        if let Berth::Docked { map, .. } = berth_of(w, charter.hull) {
            match crate::port::orders::orders_state(w, charter.hull) {
                crate::port::orders::OrdersState::Present(orders)
                    if orders.author == actor && orders.cursor.is_some() =>
                {
                    lines.extend(crate::port::executor::plan_queue(
                        ctx, &view, def, map, orders,
                    ));
                }
                crate::port::orders::OrdersState::Unreadable => {
                    lines.push(crate::port::orders::ORDERS_DARK.replace("{name}", &name));
                }
                crate::port::orders::OrdersState::Present(_)
                | crate::port::orders::OrdersState::Absent => {}
            }
        }
        lines.push(NOTHING_WRITTEN.to_owned());
        lines
    };
    for line in lines {
        let actor = ctx.actor;
        ctx.out.sys_to_actor(actor, "tacsim", line);
    }
    Ok(())
}

/// [`PLAN_DARK`] without its last sentence (`+ship/plan`'s form).
fn plan_dark(name: &str) -> String {
    PLAN_DARK
        .replace("{name}", name)
        .replace(NOTHING_BURNED, "")
}

/// One destination's block of `+ship/plan` (lines 1–7).
fn plan_block(
    ctx: &Ctx<'_>,
    view: &HullView<'_>,
    def: &PortDef,
    basis: &Basis,
    dest: &Destination,
) -> Result<Vec<String>, String> {
    let w: &World = ctx.w;
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(|| "the goods table is not loaded".to_owned())?;
    // Doc 20 cut SM.2 (lead override 3): a site whose pad is not ready plans nothing.
    let site_dest = match site_gate(w, dest) {
        Ok(site_dest) => site_dest,
        Err(closed) => return Ok(vec![format!("Plan for {}:", dest.port), closed.line]),
    };
    // Doc 20 cut SM.3b (lead ruling 7): a rig out or a loaded hopper plans nothing either.
    if let Err(refused) = bay_gate(ctx, view, &name_of(w, view.hull())) {
        return Ok(vec![format!("Plan for {}:", dest.port), refused.line]);
    }
    let mut plan = plan_on(w, basis, &dest.routes);
    // Doc 20 cut SM.2 (Q-20-7): at a site berth the plan carries the stay's charge the
    // filing would make.
    if let (Ok(plan), Berth::Docked { def, map }) = (plan.as_mut(), berth_of(w, view.hull())) {
        charge_stay(w, view.hull(), (def, map), basis, plan).map_err(|e| e.to_string())?;
    }
    let mut lines: Vec<String> = Vec::new();
    let heading = match plan.as_ref().ok().and_then(|plan| waypoints(w, plan)) {
        Some(via) => format!("Plan for {}, by way of {via}:", dest.port),
        None => format!("Plan for {}:", dest.port),
    };
    lines.push(heading);
    // 2–3: the slacks and the room, from the same reading the gauges use.
    let readings = ship::readings_of(ctx, view)?;
    for (_, comp, contents) in &readings {
        let carries_trade = goods
            .goods
            .iter()
            .any(|good| good.consumable.is_none() && comp.accepts.contains(&good.stowage));
        if carries_trade {
            lines.push(slack_line(goods, comp, contents).map_err(|e| e.to_string())?);
        }
    }
    if let Some(room) = room_line(goods, def, &readings).map_err(|e| e.to_string())? {
        lines.push(room);
    }
    let ceiling = basis.spec.drive.payload_ceiling_kg;
    let plan = match plan {
        Ok(plan) => plan,
        Err(PlanError::Overloaded { payload_kg, .. }) => {
            lines.push(format!(
                "Payload {} t is over the rated {} t; no passage can be planned until the hold is \
                 lightened.",
                tonnes_over(payload_kg, ceiling),
                ceiling / 1_000
            ));
            return Ok(lines);
        }
        // Doc 20 cut SM.2 (D-20-3): the landing law, without the last sentence.
        Err(PlanError::Landing {
            place,
            body,
            twr_permille,
        }) => {
            let name = name_of(w, view.hull());
            lines.push(
                landing_line(w, &name, &place, &body, twr_permille).replace(NOTHING_BURNED, ""),
            );
            return Ok(lines);
        }
        Err(error) => return Err(error.to_string()),
    };
    // 4–7.
    lines.push(format!(
        "Payload {} t of {} t.",
        tonnes_one_decimal(basis.payload),
        ceiling / 1_000
    ));
    // Doc 19 cut A.2 (D-19-8): the law's own sentence — the coast law's doctrine
    // fraction, or the flip law's acceleration, Δv and peak at this load.
    let law_line = match basis.spec.drive.law {
        crate::definitions::logistics::DriveLaw::Coast { burn_permille, .. } => {
            format!("Every tonne aboard, reserve included, costs {burn_permille} kg of burn.")
        }
        crate::definitions::logistics::DriveLaw::Flip {
            accel_mg,
            exhaust_kms,
        } => match flip_profile(
            &basis.spec.drive,
            basis.spec.dry_kg,
            basis.payload,
            &plan.legs,
        ) {
            Some(profile) => format!(
                "Flip and burn at {:.2} m/s² ({accel_mg} milli-g rated, throttle {}‰): Δv {} km/s, \
                 peak {} km/s, exhaust {exhaust_kms} km/s; a farther port costs more.",
                profile.accel_mps2,
                basis.spec.drive.throttle_permille,
                (profile.dv_mps / 1_000.0).round(),
                (profile.peak_mps / 1_000.0).round()
            ),
            None => "Flip and burn.".to_owned(),
        },
    };
    let tank = if basis.tank >= plan.burn_t {
        format!("{} t left", basis.tank - plan.burn_t)
    } else {
        format!("{} t short", plan.burn_t - basis.tank)
    };
    lines.push(format!(
        "Burn {} t (tank {} t; {tank}). {law_line}",
        plan.burn_t, basis.tank
    ));
    let crew = u64::from(plan.crew_days);
    let locker = if basis.locker >= crew {
        format!("{} left", basis.locker - crew)
    } else {
        format!("{} short", crew - basis.locker)
    };
    lines.push(format!(
        "Stores {} crew-days (locker {}; {locker}).",
        plan.crew_days, basis.locker
    ));
    // Doc 19 cut A.8 (D-19-22): the throttle that would fit, when the asked one
    // does not even with the tank and the locker full.
    // Doc 20 cut SM.2 (decision 7): on a site's pad the hint reads the pools aboard.
    let on_pad =
        matches!(berth_of(w, view.hull()), Berth::Docked { def, .. } if def.site.is_some());
    // The SM.2 fixer (the lead's item 2): for a site destination the hint counts the leg
    // home the filing holds back.
    let site_hint = site_dest.as_ref().map(|(_, site)| (*site, &def.venue));
    if let Some(hint) = throttle_hint(w, goods, basis, &readings, &dest.routes, on_pad, site_hint) {
        lines.push(hint);
    }
    // Doc 20 cut SM.2 (§1 beat 3): on a pad, the effective stores — the leg's own
    // crew-days kept back.
    if let Berth::Docked { def, map } = berth_of(w, view.hull())
        && def.site.is_some()
        && let Some(since) = crate::port::site::landed_since(w, view.hull(), map)
        && let Some(line) = crate::port::site::stores_line(
            w,
            basis.locker,
            basis.occupants,
            plan.crew_days.saturating_sub(plan.landed_crew_days),
            since,
            basis.depart_ic,
        )
    {
        lines.push(line);
    }
    // Doc 20 cut SM.2 (decision 7): for a site destination, the leg home the filing
    // holds back. A reserve that cannot be read costs this block its one line, never the
    // block: the Burn and Stores lines above stand (a refused lift reads as the filing
    // would refuse it).
    if let Some((_, site)) = &site_dest
        && let Berth::Docked { def: here, .. } = berth_of(w, view.hull())
    {
        match site_reserve(w, basis, &plan, site, &here.venue, PLAN_WINDOW_DAYS) {
            Ok(reserve) => lines.push(
                SITE_HOME
                    .replace("{port}", &dest.port)
                    .replace("{burn}", &reserve.burn_t.to_string())
                    .replace("{days}", &reserve.crew_days.to_string())
                    .replace("{throttle}", &reserve.throttle.to_string()),
            ),
            Err(PlanError::Landing {
                place,
                body,
                twr_permille,
            }) => {
                let name = name_of(w, view.hull());
                lines.push(
                    landing_line(w, &name, &place, &body, twr_permille).replace(NOTHING_BURNED, ""),
                );
            }
            Err(error) => {
                tracing::warn!(hull = view.hull().0, %error, "tacsim: +ship/plan could not plan a site's leg home");
            }
        }
    }
    let icdays = ic_days(plan.total_s);
    let arrival = plan.depart_ic.saturating_add(plan.total_s);
    lines.push(match (real_until(w, arrival), eta(w, arrival)) {
        (Some(real), Some((hhmm, day))) => format!(
            "Passage {icdays} IC days ≈ {} real; arrives about {hhmm} UTC {day}.",
            real_span(real)
        ),
        _ => format!("Passage {icdays} IC days; the clock is stopped."),
    });
    // Doc 19 cut A.3 (§5): the window — the best departure in the next thirty IC
    // days against today's, when the sky moves.
    if let Some(window) = crate::port::chart::plan_window_line(w, basis, &dest.routes, &plan) {
        lines.push(window);
    }
    Ok(lines)
}

/// **The `Throttle:` line** ([`THROTTLE_FITS`], [`THROTTLE_NONE`]; doc 19 cut A.8,
/// D-19-22): the plan is re-based with the tank and the locker filled to their
/// capacities (today's pools swapped for the compartments' limits); when the plan
/// at the asked throttle still burns more than the tank holds or needs more
/// crew-days than the locker holds, the highest throttle in the band whose burn
/// fits is found by bisection ([`highest_fitting`]; the flip law's burn grows with
/// the throttle) and its burn, crew-days and passage are printed — or the line
/// says no throttle fits (the band's floor burns over the tank, or the fitting
/// throttle's passage outlasts the locker). `None` when the plan fits as asked,
/// or when nothing can be read.
///
/// **Doc 20 cut SM.2** (decision 7): `on_pad` — the berth is a site's pad, where
/// nothing is sold — reads the tank and the locker aboard instead of their capacities
/// ([`THROTTLE_FITS_PAD`], [`THROTTLE_NONE_PAD`]), so a lift whose leg home overruns
/// the tank at the asked throttle names the one that fits what she carries (the
/// filing held that leg home back at its landing floor).
///
/// **The SM.2 fixer (the lead's pre-commit item 2)**: `site` — the destination is a site,
/// with the departure berth's venue — makes the hint reserve-aware, since the filing holds
/// the leg home back ([`site_reserve`], [`SITE_TANK_SHORT`]): a throttle fits when the
/// passage out and the leg home together fit the tank (and, at the throttle found, the
/// locker), and the line is [`THROTTLE_FITS_SITE`]; when no throttle from the landing floor
/// fits today, the floor is tried on each later IC day of `+chart`'s window
/// (`chart::first_day` over [`WINDOW_DAYS`](crate::port::chart::WINDOW_DAYS), the IC year
/// `+chart <port>` scans; skipped under a constant sky, where every day reads as today) and
/// the first that fits is named ([`THROTTLE_LATER_SITE`]), else none is
/// ([`THROTTLE_NONE_SITE`]). The scan is bounded: at most 365 days, each costing the
/// passage out and the arrival day's leg home (two plans) unless both fit, when the
/// reserve's 30-day window is read too; every reading shares one [`Sky`] (SM2-FX-2). A
/// station destination reads exactly as A.8's did.
pub(crate) fn throttle_hint(
    w: &World,
    goods: &crate::definitions::GoodsTable,
    basis: &Basis,
    readings: &[gauges::Reading],
    routes: &[CanonicalId],
    on_pad: bool,
    site: Option<(&PortDef, &CanonicalId)>,
) -> Option<String> {
    use crate::port::chart::{WINDOW_DAYS, first_day};
    use crate::port::passage::{DAY_SECS, MAX_THROTTLE_PERMILLE};
    let mass = CanonicalId::new(MASS_KG).ok()?;
    let tank_row = goods.good(&consumable_good(goods, Consumable::Propellant)?)?;
    let locker_row = goods.good(&consumable_good(goods, Consumable::Provisions)?)?;
    let compartment = |stowage: &CanonicalId| -> Option<&Compartment> {
        readings
            .iter()
            .find(|(_, comp, _)| comp.accepts.contains(stowage))
            .map(|(_, comp, _)| comp)
    };
    // A compartment's capacity in units of its good: the tightest limit.
    let capacity = |comp: &Compartment, row: &crate::definitions::GoodDef| -> Option<u64> {
        comp.limits
            .iter()
            .filter_map(|(measure, limit)| {
                let per_unit = *row.measures.get(measure)?;
                (per_unit > 0).then(|| limit / per_unit)
            })
            .min()
    };
    let (tank_cap, locker_cap, payload) = if on_pad {
        // On a pad: the pools as they stand, the payload as it is.
        (basis.tank, basis.locker, basis.payload)
    } else {
        let tank_cap = capacity(compartment(&tank_row.stowage)?, tank_row)?;
        let locker_cap = capacity(compartment(&locker_row.stowage)?, locker_row)?;
        let tank_kg = *tank_row.measures.get(&mass)?;
        let locker_kg = *locker_row.measures.get(&mass)?;
        // The payload with both full: today's pools swapped for the capacities.
        let payload = basis
            .payload
            .checked_sub(basis.tank.checked_mul(tank_kg)?)?
            .checked_sub(basis.locker.checked_mul(locker_kg)?)?
            .checked_add(tank_cap.checked_mul(tank_kg)?)?
            .checked_add(locker_cap.checked_mul(locker_kg)?)?;
        (tank_cap, locker_cap, payload)
    };
    let pools = if on_pad { "as they stand" } else { "full" };
    // The basis the hint plans on: the pools at what the line assumes (the site reserve
    // reads them for the home payload).
    let mut full = basis.clone();
    full.payload = payload;
    full.tank = tank_cap;
    full.locker = locker_cap;
    // The latched lane and its sky, built once for every reading below (SM2-FX-2: a
    // reading used to rebuild the sky twice, once for the passage and once for the reserve).
    let (sky, pack) = lane_sky_of(w, basis.version).ok()?;
    let lane = (&sky, pack);
    // One reading at `throttle`, `offset` IC days on: the passage out and, for a site, the
    // leg home it holds back — none when the passage out alone overruns the tank (the sum
    // does then too, and the reserve's plans are spared).
    let at = |throttle: u32, offset: i64| -> Option<HintReading> {
        let mut b = full.clone();
        b.spec.drive.throttle_permille = throttle;
        b.depart_ic = offset
            .checked_mul(i64::from(DAY_SECS))
            .and_then(|secs| basis.depart_ic.checked_add(secs))?;
        let plan = plan_on_sky(lane, &b, routes).ok()?;
        let reserve = match site {
            Some((def, from)) if plan.burn_t <= tank_cap => {
                // The arrival day's leg home first: the window's worst is at least it, so
                // a reading that overruns on it is refused without the window's plans.
                let first = site_reserve_on(w, lane, &b, &plan, def, from, 0).ok()?;
                if plan.burn_t.saturating_add(first.burn_t) > tank_cap {
                    Some(first)
                } else {
                    Some(site_reserve_on(w, lane, &b, &plan, def, from, PLAN_WINDOW_DAYS).ok()?)
                }
            }
            Some(_) | None => None,
        };
        Some((plan, reserve))
    };
    let burn_fits = |reading: &HintReading| reading_needs(reading).0 <= tank_cap;
    let fits = |reading: &HintReading| {
        let (burn, days) = reading_needs(reading);
        burn <= tank_cap && days <= locker_cap
    };
    let asked = at(basis.spec.drive.throttle_permille, 0)?;
    if fits(&asked) {
        return None;
    }
    // Doc 20 cut SM.2 (decision 21): the bisection starts at the landing floor — the least
    // throttle that lands at this payload over the listed routes that can land at all —
    // since the landing law refuses below it (A.8's bound assumed every throttle from the
    // band's floor plans). A non-site route's floor is the band's own, so A.8's figures
    // stand; no listed route lands at all: no hint.
    let lo = hint_floor(pack, routes, &basis.spec.drive, basis.spec.dry_kg, payload)?;
    let found = highest_fitting(lo, MAX_THROTTLE_PERMILLE, |t| {
        at(t, 0).is_some_and(|reading| burn_fits(&reading))
    })
    .and_then(|throttle| Some((throttle, at(throttle, 0)?)))
    .filter(|(_, reading)| fits(reading));
    let line = match (found, site) {
        (Some((throttle, (plan, _))), None) => if on_pad {
            THROTTLE_FITS_PAD
        } else {
            THROTTLE_FITS
        }
        .replace("{throttle}", &throttle.to_string())
        .replace("{burn}", &plan.burn_t.to_string())
        .replace("{days}", &plan.crew_days.to_string())
        .replace("{passage}", &ic_days(plan.total_s)),
        (None, None) => if on_pad {
            THROTTLE_NONE_PAD
        } else {
            THROTTLE_NONE
        }
        .replace("{min}", &lo.to_string()),
        (Some((throttle, (plan, reserve))), Some(_)) => {
            let reserve = reserve?;
            THROTTLE_FITS_SITE
                .replace("{pools}", pools)
                .replace("{throttle}", &throttle.to_string())
                .replace("{burn}", &plan.burn_t.to_string())
                .replace("{home}", &reserve.burn_t.to_string())
                .replace("{days}", &plan.crew_days.to_string())
                .replace("{home_days}", &reserve.crew_days.to_string())
                .replace("{passage}", &ic_days(plan.total_s))
        }
        (None, Some(_)) => {
            // Nothing fits today: the floor on each later day of `+chart`'s window (the IC
            // year), when the sky moves (under a constant sky every day reads as today).
            let moving = w
                .module_state::<TacsimStore>()
                .is_some_and(|store| store.config().time.ephemeris.rate != 0);
            let later = if moving {
                first_day(WINDOW_DAYS, |offset| {
                    at(lo, offset).is_some_and(|reading| fits(&reading))
                })
            } else {
                None
            };
            match later {
                Some(offset) => {
                    let day = offset
                        .checked_mul(i64::from(DAY_SECS))
                        .and_then(|secs| basis.depart_ic.checked_add(secs))?;
                    THROTTLE_LATER_SITE
                        .replace("{min}", &lo.to_string())
                        .replace("{pools}", pools)
                        .replace("{date}", &crate::sky::epoch_date(day))
                        .replace("{offset}", &offset.to_string())
                }
                None => THROTTLE_NONE_SITE
                    .replace("{min}", &lo.to_string())
                    .replace("{pools}", pools)
                    .replace("{window}", &WINDOW_DAYS.to_string()),
            }
        }
    };
    Some(line)
}

/// One reading of [`throttle_hint`]: the passage out and, for a site destination, the leg
/// home it holds back.
type HintReading = (PassagePlan, Option<Reserve>);

/// The tonnes and crew-days a [`HintReading`] needs: the passage's, plus the leg home's (a site
/// reading with no reserve overran the tank on the passage out alone, so its sum overruns
/// too).
fn reading_needs((plan, reserve): &HintReading) -> (u64, u64) {
    let (home_t, home_days) = reserve.map_or((0, 0), |r| (r.burn_t, r.crew_days));
    (
        plan.burn_t.saturating_add(home_t),
        u64::from(plan.crew_days).saturating_add(u64::from(home_days)),
    )
}

/// **The throttle hint's lower bound** (doc 20 cut SM.2, decision 21): the least
/// [`route_landing_floor`] over the listed `routes` that answer at all, floored at the
/// band's [`MIN_THROTTLE_PERMILLE`] — the
/// least throttle any of them lands at, at `payload_kg`. A route with no site end
/// answers the band's floor, so a station's hint starts where A.8's did. `None` when no
/// listed route lands at all (then no hint is given).
pub(crate) fn hint_floor(
    pack: &crate::pack::RulesPack,
    routes: &[CanonicalId],
    drive: &crate::port::passage::HullDrive,
    dry_kg: u64,
    payload_kg: u64,
) -> Option<u32> {
    routes
        .iter()
        .filter_map(|route| route_landing_floor(pack, route, drive, dry_kg, payload_kg).ok())
        .min()
        .map(|lo| lo.max(crate::port::passage::MIN_THROTTLE_PERMILLE))
}

/// The highest value in `lo..=hi` that `fits`, by bisection over a predicate
/// that is true up to some value and false past it; `None` when `lo` itself
/// does not fit.
fn highest_fitting(lo: u32, hi: u32, mut fits: impl FnMut(u32) -> bool) -> Option<u32> {
    if !fits(lo) {
        return None;
    }
    if fits(hi) {
        return Some(hi);
    }
    let (mut lo, mut hi) = (lo, hi);
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if fits(mid) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(lo)
}

// ---- +ship/undock ------------------------------------------------------------------

/// **A departure ready to file** (doc 18 cut IC11, lead note 2j(i)): the sailing plan
/// with its pending passage, the destination's name, the burn's key and sequence and
/// the registered burn action — what `+ship/undock` and the order arm's depart step
/// both write and submit.
#[derive(Clone, Debug)]
pub(crate) struct DepartFiling {
    /// The sailing plan, its pending passage filed.
    pub voyage: Voyage,
    /// The destination port's name.
    pub port: String,
    /// The burn's idempotency key.
    pub key: IdempotencyKeyText,
    /// The burn sequence (`pending.seq`).
    pub seq: u32,
    /// The registered `tacsim_port_burn` action.
    pub burn: mush_core::DefinitionRef,
}

/// **Who files the departure** (doc 18 cut IC11): a live `+ship/undock`, or a queued
/// depart order at its attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Queued {
    /// `+ship/undock`: `order: None, attempt: 1`, keyed `…/{hull}/undock/{seq}`.
    Live,
    /// A queued depart: keyed `…/{hull}/{order}/{attempt}` (doc 18 §4.4).
    Order {
        /// The order id.
        order: u32,
        /// The order's attempt.
        attempt: u32,
    },
}

/// **Why a departure cannot be filed, typed** (reviewer F7): the order arm matches on
/// the kind, never on the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DepartRefusalKind {
    /// The burn action is not registered.
    Unwired,
    /// The sailing plan does not read.
    VoyageDark,
    /// A clearance is filed already.
    Cleared,
    /// The hull is under way.
    UnderWay,
    /// The pilot is not aboard.
    NotAboard,
    /// The hull is at no berth.
    NotDocked,
    /// No route of this berth serves the destination.
    NoRoute,
    /// Bought goods are not aboard yet.
    GoodsComing,
    /// A listing holds goods in the hold.
    ListingHolds,
    /// A sale's goods have not left the hold yet.
    GoodsLeaving,
    /// The planner, the lane or the filing could not answer.
    PlanDark,
    /// Over the payload rail.
    Overloaded,
    /// The tank below the burn.
    TankShort,
    /// The locker below the crew-days.
    StoresShort,
    /// The launch dry run refused.
    PortClosed,
    /// **Doc 20 cut SM.2 (D-20-3)**: the landing law refused the passage.
    Landing,
    /// **Doc 20 cut SM.2 (lead override 3)**: the destination site's pad is not ready.
    SiteClosed,
    /// **Doc 20 cut SM.3b (§4.3, D-20-7)**: a rig is out of the bay or mid-act
    /// ([`TV_OUT`], [`TV_BUSY`]).
    TvOut,
    /// **Doc 20 cut SM.3b (§4.3)**: a rig's hopper holds ore ([`HOPPER_LOADED`]).
    HopperLoaded,
}

/// A typed refusal: its kind and the line `+ship/undock` answers with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DepartRefusal {
    /// The kind.
    pub kind: DepartRefusalKind,
    /// The line.
    pub line: String,
}

impl From<DepartRefusal> for CmdError {
    fn from(refusal: DepartRefusal) -> Self {
        CmdError::msg(refusal.line)
    }
}

/// A refusal of `kind` with `line`.
fn refusal(kind: DepartRefusalKind, line: String) -> DepartRefusal {
    DepartRefusal { kind, line }
}

/// **The site gate** (doc 20 cut SM.2, lead override 3): the destination's site port, when
/// it is one — refused with [`SITE_CLOSED`] when its readiness lines are not all ready.
/// The one check `+ship/plan`, the filing and the queue's dry run (`executor::plan_queue`)
/// share. Since doc 20 cut SM.6 (D2) that includes a keyed site still opening its deposits
/// (warming, "opening, N of M", or failed): its pad admits no hull until its clerk is done.
///
/// # Errors
///
/// [`DepartRefusalKind::SiteClosed`].
pub(crate) fn site_gate<'w>(
    w: &'w World,
    dest: &Destination,
) -> Result<Option<(CanonicalId, &'w PortDef)>, DepartRefusal> {
    let site_dest = site_port(w, &dest.venue);
    if let Some((entry, _)) = &site_dest
        && !crate::port::readiness::port_ready(w, entry)
    {
        return Err(refusal(
            DepartRefusalKind::SiteClosed,
            SITE_CLOSED.replace("{site}", &dest.port),
        ));
    }
    Ok(site_dest)
}

/// **The bay gate** (doc 20 cut SM.3b; §4.3's two typed departure refusals, D-20-7; lead
/// ruling 7, decision 7): beside [`site_gate`], called right after it by
/// [`prepare_departure`] (so `+ship/undock`, `+ship/lift` and the order fire refuse),
/// `+ship/plan`'s block and the queue's dry run (`executor::plan_queue`). The hull's rigs are
/// the placements in its bay and the live lease records naming it, by slot. Each rig not
/// `Interior` in this hull's bay, or whose `tacsim.excursion` record holds an act or an open
/// excursion, refuses — [`TV_BUSY`] with an act in flight; else, with the hull on a site's
/// pad, [`TV_OUT`] (`{site}` where it stands, or the open excursion's site: a rig a staff move
/// left in the bay with its excursion open reads `TV_OUT`, and its remedy, `+tv/{n} recover`,
/// closes it — decision 21); else (the hull at a port or anywhere a recover cannot reach, which
/// only a staff move makes) [`TV_AWAY`], whose remedy is staff's (the fixer round's item 2).
/// The record is read through `crate::port::tv::rig_flight`, the reader the lease return and
/// the repossession share. Then each rig in the bay whose hopper holds anything refuses [`HOPPER_LOADED`]. A
/// record or a pool that will not read is `PlanDark`, logged. A standard Dray (no bay, no
/// records) answers `Ok(())` at once.
///
/// # Errors
///
/// [`DepartRefusalKind::TvOut`], [`DepartRefusalKind::HopperLoaded`] or
/// [`DepartRefusalKind::PlanDark`].
pub(crate) fn bay_gate(
    ctx: &Ctx<'_>,
    view: &HullView<'_>,
    name: &str,
) -> Result<(), DepartRefusal> {
    use crate::port::tv::RigFlight;
    use crate::surface::act::{
        Where, doing_words, ramp_label, site_name, site_of_domain, where_is,
    };
    use DepartRefusalKind as K;
    let w: &World = ctx.w;
    let hull = view.hull();
    let dark = || refusal(K::PlanDark, PLAN_DARK.replace("{name}", name));
    // The rigs: the bay's placements and the live records naming the hull, by slot.
    let mut rigs: Vec<(u8, ObjId)> = crate::port::vehicle::rigs_in_bay(w, view)
        .iter()
        .map(|rig| (rig.slot, rig.tv))
        .collect();
    for record in crate::port::tv::live_on_hull(w, hull) {
        if !rigs.iter().any(|(_, tv)| *tv == record.tv) {
            rigs.push((record.slot, record.tv));
        }
    }
    if rigs.is_empty() {
        return Ok(());
    }
    rigs.sort_by_key(|(slot, tv)| (*slot == 0, *slot, *tv));
    if w.module_state::<PortHandles>().is_none() {
        return Err(dark());
    }
    let n_of = |slot: u8| {
        if slot == 0 {
            "?".to_owned()
        } else {
            slot.to_string()
        }
    };
    let ramp = ramp_label(w, hull);
    // The pad the hull is settled on, when it is a site's: there `+tv/{n} recover` works, so
    // `TV_OUT` names it; anywhere else only staff can bring a rig in (`TV_AWAY`; decision 21,
    // the fixer round's item 2).
    let pad = match berth_of(w, hull) {
        Berth::Docked { def, .. } if def.site.is_some() => Some(port_name(def)),
        Berth::Docked { .. } | Berth::UnderWay | Berth::Elsewhere => None,
    };
    for (slot, tv) in &rigs {
        // The one reader the lease return and the repossession share (the fixer round's
        // item 1), so the three cannot drift.
        let flight = match crate::port::tv::rig_flight(ctx, *tv) {
            Ok(flight) => flight,
            Err(error) => {
                tracing::warn!(tv = tv.0, %error, "tacsim: a departure could not read a rig's excursion record");
                return Err(dark());
            }
        };
        let place = where_is(w, *tv);
        let in_bay = place == (Where::Bay { hull });
        if let RigFlight::Busy { kind, .. } = flight {
            return Err(refusal(
                K::TvOut,
                TV_BUSY
                    .replace("{n}", &n_of(*slot))
                    .replace("{doing}", &doing_words(kind, &ramp))
                    .replace("{name}", name),
            ));
        }
        let open = match &flight {
            RigFlight::Out { site } => Some(site),
            RigFlight::Idle | RigFlight::Busy { .. } => None,
        };
        if !in_bay || open.is_some() {
            let named = match (open, place) {
                (Some(site), _) => Some(site_name(w, site)),
                (None, Where::Ground { domain, .. }) => {
                    site_of_domain(w, domain).map(|entry| site_name(w, &entry))
                }
                (None, Where::Bay { .. } | Where::Elsewhere) => None,
            };
            let line = match &pad {
                Some(pad) => {
                    let site = named.unwrap_or_else(|| match place {
                        Where::Ground { .. } => "the ground".to_owned(),
                        Where::Bay { .. } | Where::Elsewhere => pad.clone(),
                    });
                    TV_OUT
                        .replace("{n}", &n_of(*slot))
                        .replace("{site}", &site)
                        .replace("{name}", name)
                }
                None => TV_AWAY
                    .replace("{n}", &n_of(*slot))
                    .replace(
                        "{where}",
                        &named.map_or_else(
                            || "away from her bay".to_owned(),
                            |site| format!("on {site}"),
                        ),
                    )
                    .replace("{name}", name),
            };
            return Err(refusal(K::TvOut, line));
        }
    }
    for (slot, tv) in &rigs {
        let (kg, _) = crate::port::tv::hopper_load_of(ctx, *tv).map_err(|error| {
            tracing::warn!(tv = tv.0, %error, "tacsim: a departure could not read a rig's hopper");
            dark()
        })?;
        if kg > 0 {
            return Err(refusal(
                K::HopperLoaded,
                HOPPER_LOADED
                    .replace("{n}", &n_of(*slot))
                    .replace("{t}", &(kg / 1_000).to_string())
                    .replace("{name}", name),
            ));
        }
    }
    Ok(())
}

/// **A planner refusal the departure names** (doc 20 cut SM.2, D-20-3; doc 18 cut IC9):
/// the landing law ([`LANDING_REFUSED`]) and the payload rail ([`OVERLOADED`]) for the hull
/// `name`; any other refusal is handed back for the caller to log as a dark plan.
///
/// # Errors
///
/// The planner's own refusal when the departure has no sentence for it.
pub(crate) fn plan_refusal(
    w: &World,
    name: &str,
    error: PlanError,
) -> Result<DepartRefusal, PlanError> {
    use DepartRefusalKind as K;
    match error {
        PlanError::Landing {
            place,
            body,
            twr_permille,
        } => Ok(refusal(
            K::Landing,
            landing_line(w, name, &place, &body, twr_permille),
        )),
        PlanError::Overloaded {
            payload_kg,
            ceiling_kg,
        } => Ok(refusal(
            K::Overloaded,
            OVERLOADED
                .replace("{name}", name)
                .replace("{payload}", &tonnes_over(payload_kg, ceiling_kg))
                .replace("{ceiling}", &(ceiling_kg / 1_000).to_string()),
        )),
        other => Err(other),
    }
}

/// **What a planned departure holds back, checked** (doc 20 cut SM.2, Q-20-7, decision 7;
/// doc 18 cut IC9's tank and locker; factored by the SM.2 fixer so the queue's dry run
/// reads the filing's own checks): at a site berth the stay's charge is added to `plan`
/// ([`charge_stay`], so the crew-days printed are the ones filed); for a `site`
/// destination the leg home ([`site_reserve`]) must fit beside the passage
/// ([`SITE_TANK_SHORT`], [`SITE_STORES_SHORT`]); then the passage alone ([`TANK_SHORT`],
/// [`STORES_SHORT`]). `here` is the departure berth, `port` the destination's name, `name`
/// the hull's. Answers the arrival row a site destination files.
///
/// # Errors
///
/// [`DepartRefusal`], typed: `Landing` when the leg home cannot lift, `TankShort` /
/// `StoresShort`, and `PlanDark` (logged) when the charge or the reserve cannot be read.
#[allow(clippy::too_many_arguments)] // the filing's whole context
pub(crate) fn hold_back(
    w: &World,
    hull: ObjId,
    name: &str,
    basis: &Basis,
    port: &str,
    here: &PortDef,
    site: Option<&PortDef>,
    plan: &mut PassagePlan,
) -> Result<Option<SiteArrival>, DepartRefusal> {
    use DepartRefusalKind as K;
    let dark = || refusal(K::PlanDark, PLAN_DARK.replace("{name}", name));
    // Doc 20 cut SM.2 (Q-20-7; lead ruling 5): lifting from a site charges the stay, inside
    // this burn — capped so the locker still covers this leg.
    if let Berth::Docked { def, map } = berth_of(w, hull) {
        charge_stay(w, hull, (def, map), basis, plan).map_err(|error| {
            tracing::warn!(hull = hull.0, %error, "tacsim: a departure could not charge the stay");
            dark()
        })?;
    }
    // Doc 20 cut SM.2 (decision 7): a departure for a site holds back the leg home.
    let mut arrival: Option<SiteArrival> = None;
    if let Some(site) = site {
        let reserve = match site_reserve(w, basis, plan, site, &here.venue, PLAN_WINDOW_DAYS) {
            Ok(reserve) => reserve,
            Err(PlanError::Landing {
                place,
                body,
                twr_permille,
            }) => {
                return Err(refusal(
                    K::Landing,
                    landing_line(w, name, &place, &body, twr_permille),
                ));
            }
            Err(error) => {
                tracing::warn!(hull = hull.0, %error, "tacsim: a departure could not plan the leg home");
                return Err(dark());
            }
        };
        let burn = plan.burn_t.saturating_add(reserve.burn_t);
        if basis.tank < burn {
            return Err(refusal(
                K::TankShort,
                SITE_TANK_SHORT
                    .replace("{port}", port)
                    .replace("{burn}", &burn.to_string())
                    .replace("{tank}", &basis.tank.to_string()),
            ));
        }
        let days = u64::from(plan.crew_days).saturating_add(u64::from(reserve.crew_days));
        if basis.locker < days {
            return Err(refusal(
                K::StoresShort,
                SITE_STORES_SHORT
                    .replace("{port}", port)
                    .replace("{days}", &days.to_string())
                    .replace("{have}", &basis.locker.to_string()),
            ));
        }
        arrival = Some(SiteArrival {
            locker: basis.locker.saturating_sub(u64::from(plan.crew_days)),
            home_crew_days: reserve.crew_days,
            souls: basis.occupants,
        });
    }
    if basis.tank < plan.burn_t {
        return Err(refusal(
            K::TankShort,
            TANK_SHORT
                .replace("{port}", port)
                .replace("{burn}", &plan.burn_t.to_string())
                .replace("{tank}", &basis.tank.to_string()),
        ));
    }
    if basis.locker < u64::from(plan.crew_days) {
        return Err(refusal(
            K::StoresShort,
            STORES_SHORT
                .replace("{port}", port)
                .replace("{days}", &plan.crew_days.to_string())
                .replace("{have}", &basis.locker.to_string()),
        ));
    }
    Ok(arrival)
}

/// **Every check before a departure is filed, then the filing** (doc 18 cut IC9's
/// `+ship/undock` body, factored for IC11's depart step; lead rulings L4, L13, L14,
/// L21, L28 of IC9, L8 of IC11): exactly the undock's refusals in its order — the
/// burn wired, the sailing papers, the berth, the pilot aboard, the route, the goods
/// not yet aboard, the plan, the tank and the locker, the launch dry run, the
/// sequence, the check and the key — each typed. The plan is made **now**: the burn's
/// prepare refuses a future `depart_ic`, so a queued `at=` is the order arm's due gate,
/// never a future-dated plan (L8). Writes nothing.
///
/// # Errors
///
/// [`DepartRefusal`], typed.
#[allow(clippy::too_many_lines)] // one ordered refusal list
pub(crate) fn prepare_departure(
    ctx: &Ctx<'_>,
    pilot: ObjId,
    hull: ObjId,
    dest_word: &str,
    options: &PassageOptions,
    queued: Queued,
) -> Result<DepartFiling, DepartRefusal> {
    use DepartRefusalKind as K;
    let w: &World = ctx.w;
    let Some(burn) = w
        .module_state::<PortHandles>()
        .and_then(|handles| handles.burn.clone())
    else {
        return Err(refusal(K::Unwired, UNDOCK_UNWIRED.to_owned()));
    };
    let name = name_of(w, hull);
    let dark = || refusal(K::PlanDark, PLAN_DARK.replace("{name}", &name));
    let (last_seq, told) = match w
        .module_state::<TacsimStore>()
        .map_or(VoyageState::Absent, |store| store.voyage_state(hull))
    {
        VoyageState::Unreadable => {
            return Err(refusal(K::VoyageDark, VOYAGE_DARK.replace("{name}", &name)));
        }
        VoyageState::Present(voyage) => {
            if let Some(pending) = &voyage.pending {
                return Err(refusal(
                    K::Cleared,
                    UNDOCK_CLEARED
                        .replace("{name}", &name)
                        .replace("{port}", &venue_name(w, &pending.to_venue)),
                ));
            }
            (voyage.last_seq, voyage.told)
        }
        VoyageState::Absent => (0, 0),
    };
    let Some(view) = hull_view(w, hull) else {
        let line = no_charter(w, pilot).to_string();
        return Err(refusal(K::PlanDark, line));
    };
    if matches!(berth_of(w, hull), Berth::UnderWay) {
        return Err(refusal(
            K::UnderWay,
            UNDOCK_UNDER_WAY.replace("{name}", &name),
        ));
    }
    let Some(airlock) = view.airlock() else {
        return Err(refusal(K::NotAboard, NOT_ABOARD.replace("{name}", &name)));
    };
    if !aboard(w, hull, pilot) {
        return Err(refusal(K::NotAboard, NOT_ABOARD.replace("{name}", &name)));
    }
    let Berth::Docked { def, .. } = berth_of(w, hull) else {
        return Err(refusal(K::NotDocked, NOT_DOCKED.replace("{name}", &name)));
    };
    let here = port_name(def);
    let dests = destinations(w, def);
    // Doc 18 cut IC14 row 25: a queued depart resolves its stored venue word exactly;
    // the live `+ship/undock` keeps the prefix matcher.
    let found = match queued {
        Queued::Live => match_destination(&dests, dest_word),
        Queued::Order { .. } => match_destination_exact(&dests, dest_word),
    };
    let dest = found
        .cloned()
        .ok_or_else(|| refusal(K::NoRoute, no_route(&here, &dests)))?;
    let port = dest.port.clone();
    // Doc 20 cut SM.2 (lead override 3): a site whose pad is not ready takes no
    // departure (its lines never gate the desk, so they gate the passage here).
    let site_dest = site_gate(w, &dest)?;
    // Doc 20 cut SM.3b (lead ruling 7): every rig in the bay, every hopper empty.
    bay_gate(ctx, &view, &name)?;
    // Lead ruling L13: promised goods hold the hull.
    if let Some(vault) = hold_vault(w, &view) {
        match goods_pending_aboard(w, EndpointRef(vault)) {
            Some(Pending::Quote(quote)) => {
                return Err(refusal(
                    K::GoodsComing,
                    GOODS_COMING
                        .replace("{quote}", &quote.to_string())
                        .replace("{name}", &name),
                ));
            }
            Some(Pending::Listing(listing)) => {
                return Err(refusal(
                    K::ListingHolds,
                    LISTING_HOLDS
                        .replace("{listing}", &listing.to_string())
                        .replace("{name}", &name),
                ));
            }
            Some(Pending::Sale(quote)) => {
                return Err(refusal(
                    K::GoodsLeaving,
                    GOODS_LEAVING
                        .replace("{quote}", &quote.to_string())
                        .replace("{name}", &name),
                ));
            }
            None => {}
        }
    }
    let souls = occupants(w, airlock);
    let basis = basis(ctx, &view, souls).map_err(|error| {
        tracing::warn!(hull = hull.0, %error, "tacsim: a departure could not read the passage's basis");
        dark()
    })?;
    let basis = apply_options(basis, options);
    let mut plan = plan_on(w, &basis, &dest.routes).map_err(|error| {
        plan_refusal(w, &name, error).unwrap_or_else(|error| {
            tracing::warn!(hull = hull.0, %error, "tacsim: a departure could not plan the passage");
            dark()
        })
    })?;
    // Doc 20 cut SM.2: the stay's charge, the leg home a site holds back, the tank and the
    // locker — the checks the queue's dry run reads through the same function.
    let site = site_dest.as_ref().map(|(_, site)| *site);
    let arrival = match hold_back(w, hull, &name, &basis, &port, def, site, &mut plan) {
        Ok(arrival) => arrival,
        Err(mut refused) => {
            // The SM.2 fixer (the lead's item 2): a live departure for a site whose tank or
            // locker cannot hold the leg home back names the throttle (or the day) that
            // would. A queued depart's waiting note keeps the bare sentence: the hint moves
            // with the sky, and a note is re-told whenever its text changes.
            if queued == Queued::Live
                && matches!(refused.kind, K::TankShort | K::StoresShort)
                && let Some(site) = site
                && let Some(goods) = w.module_state::<TacsimStore>().and_then(TacsimStore::goods)
                && let Ok(readings) = ship::readings_of(ctx, &view)
                && let Some(hint) = throttle_hint(
                    w,
                    goods,
                    &basis,
                    &readings,
                    &dest.routes,
                    def.site.is_some(),
                    Some((site, &def.venue)),
                )
            {
                refused.line = format!("{}\n{hint}", refused.line);
            }
            return Err(refused);
        }
    };
    // Lead ruling L28 (reviewer F2): the launch, dry-run before anything is
    // burned — the route materializes at the version the plan was made at, and
    // mush-sim would depart it now.
    let store = w.module_state::<TacsimStore>().ok_or_else(dark)?;
    let Some(def_route) = store.route_in_world(w, &plan.route) else {
        tracing::warn!(hull = hull.0, route = %plan.route, "tacsim: a departure: the planned route does not materialize");
        return Err(dark());
    };
    let same_version = store.route_source(w, &def_route).is_some_and(|(pack, _)| {
        pack.manifest().id().as_str() == LANE_PACK && pack.manifest().version() == basis.version
    });
    if !same_version {
        tracing::warn!(hull = hull.0, route = %plan.route, version = basis.version, "tacsim: a departure: the route does not materialize at the plan's lane version");
        return Err(dark());
    }
    if let Some(why) = mush_sim::depart_ineligible(ctx, hull, &def_route) {
        tracing::info!(hull = hull.0, %why, "tacsim: a departure: the launch dry run refused");
        return Err(refusal(
            K::PortClosed,
            PORT_CLOSED
                .replace("{name}", &name)
                .replace("{port}", &port),
        ));
    }
    let Some(seq) = last_seq.checked_add(1) else {
        tracing::error!(
            hull = hull.0,
            "tacsim: a departure: the burn sequence is exhausted"
        );
        return Err(dark());
    };
    let lane = CanonicalId::new(LANE_PACK).map_err(|_| dark())?;
    let (order, attempt) = match queued {
        Queued::Live => (None, 1),
        Queued::Order { order, attempt } => (Some(order), attempt),
    };
    let pending = PendingVoyage {
        seq,
        pilot,
        to_venue: dest.venue.clone(),
        vault: basis.vault.0,
        occupants: basis.occupants,
        payload_kg: basis.payload,
        route_pack: lane,
        route_version: basis.version,
        unit_definition: basis.digest,
        plan,
        order,
        attempt,
        filed_ic: basis.depart_ic,
        filed_at: w.now(),
        arrival,
        carried: basis.carried.clone(),
    };
    let voyage = Voyage {
        schema: VOYAGE_SCHEMA,
        hull,
        last_seq,
        told,
        pending: Some(pending),
        arrival: None,
    };
    if voyage.check().is_err() {
        tracing::error!(
            hull = hull.0,
            "tacsim: a departure: the filed passage does not check"
        );
        return Err(dark());
    }
    let key_text = match queued {
        Queued::Live => format!("{BURN_KEY_PREFIX}{}/undock/{seq}", hull.0),
        Queued::Order { order, attempt } => {
            format!("{BURN_KEY_PREFIX}{}/{order}/{attempt}", hull.0)
        }
    };
    let Ok(key) = IdempotencyKeyText::parse_canonical(key_text) else {
        tracing::error!(
            hull = hull.0,
            "tacsim: a departure: the burn's key did not form"
        );
        return Err(dark());
    };
    Ok(DepartFiling {
        voyage,
        port,
        key,
        seq,
        burn,
    })
}

/// **`+ship/undock <port>`** (doc 18 §4.4 "`+ship/undock <port>` (live, at=now
/// only)"; lead rulings L4, L13, L14, L21, L28): the usage, the charter, then every
/// check of [`prepare_departure`] before any write, then the voyage document and ONE
/// `tacsim_port_burn` submission as the actor.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line, and on every refusal nothing is written.
pub(crate) fn undock_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let raw = ctx.args.raw().to_owned();
    // Doc 20 cut SM.2: `+ship/lift` is this verb, worded for a pad.
    let usage = if ctx.switches.has("lift") {
        LIFT_USAGE
    } else {
        UNDOCK_USAGE
    };
    if raw.split_whitespace().all(|token| token.contains('=')) {
        return Err(CmdError::msg(usage));
    }
    let args = parse_args(&raw)?;
    let Some(word) = args.word.clone() else {
        return Err(CmdError::msg(usage));
    };
    let options = options_of(ctx.w, &args)?;
    let Some(charter) = helm_of(ctx.w, actor) else {
        return Err(no_charter(ctx.w, actor));
    };
    let filing = prepare_departure(ctx, actor, charter.hull, &word, &options, Queued::Live)?;
    let Some(pending) = filing.voyage.pending.clone() else {
        return Err(CmdError::msg(
            PLAN_DARK.replace("{name}", &name_of(ctx.w, charter.hull)),
        ));
    };
    put_voyage(ctx, &filing.voyage);
    ctx.submit_authoritative(
        filing.burn,
        PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({ "seq": filing.seq.to_string() }),
        filing.key,
    );
    let plan = &pending.plan;
    let arrival = plan.depart_ic.saturating_add(plan.total_s);
    let base = CLEARANCE_FILED
        .replace("{port}", &filing.port)
        .replace("{burn}", &plan.burn_t.to_string())
        .replace("{days}", &plan.crew_days.to_string())
        .replace("{icdays}", &ic_days(plan.total_s));
    let reply = match eta(ctx.w, arrival) {
        Some((hhmm, day)) => base.replace("{hhmm}", &hhmm).replace("{day}", &day),
        None => base.replace(", about {hhmm} UTC {day}", ""),
    };
    ctx.out.sys_to_actor(actor, "tacsim", reply);
    Ok(())
}

// ---- the launch arm (the `tacsim.port` tick) --------------------------------------

/// One launch awaiting its pull verification (RAM only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Launching {
    /// The pilot, told `CLEARANCE_GRANTED` once the journey reads.
    pub pilot: ObjId,
    /// The burn sequence.
    pub seq: u32,
    /// `PortTickState.fires` when it launched.
    pub staged_fire: u64,
    /// The plan's departure instant: the journey's `departed.at`.
    pub depart_ic: i64,
    /// The destination port's venue.
    pub to_venue: CanonicalId,
    /// The burn, tonnes.
    pub burn_t: u64,
    /// The crew-days.
    pub crew_days: u32,
    /// The passage's seconds.
    pub total_s: i64,
}

/// The hull's journey departed on `route` at `depart_ic` — the launched-ness the
/// ticket no longer carries (lead ruling L5), pulled.
pub(crate) fn launched(w: &World, hull: ObjId, route: &CanonicalId, depart_ic: i64) -> bool {
    w.module_state::<mush_sim::JourneyBook>()
        .and_then(|book| book.get(hull))
        .is_some_and(|journey| journey.route().id() == route && journey.departed().at == depart_ic)
}

/// Clear a hull's pending passage, keeping its sequence (never reused) and what the
/// pilot was told (doc 18 cut IC14 row 21).
fn clear_pending(ctx: &mut Ctx<'_>, hull: ObjId, seq: u32) {
    clear_pending_with(ctx, hull, seq, None);
}

/// [`clear_pending`] keeping a launched passage's site arrival (doc 20 cut SM.2): the
/// launch writes the pending's `arrival` into the voyage it leaves, for the arrival lines.
fn clear_pending_with(ctx: &mut Ctx<'_>, hull: ObjId, seq: u32, arrival: Option<SiteArrival>) {
    let told = told_of(ctx.w, hull).min(seq);
    put_voyage(
        ctx,
        &Voyage {
            schema: VOYAGE_SCHEMA,
            hull,
            last_seq: seq,
            told,
            pending: None,
            arrival,
        },
    );
}

/// The hull's `Voyage.told` (0 with no readable document).
fn told_of(w: &World, hull: ObjId) -> u32 {
    match w
        .module_state::<TacsimStore>()
        .map_or(VoyageState::Absent, |store| store.voyage_state(hull))
    {
        VoyageState::Present(voyage) => voyage.told,
        VoyageState::Absent | VoyageState::Unreadable => 0,
    }
}

/// Record that the pilot was told of `seq`'s launch (doc 18 cut IC14 row 21): the
/// document re-written with `told = seq`, nothing else changed. A document that does
/// not read is left alone.
fn mark_told(ctx: &mut Ctx<'_>, hull: ObjId, seq: u32) {
    let voyage = match ctx
        .w
        .module_state::<TacsimStore>()
        .map_or(VoyageState::Absent, |store| store.voyage_state(hull))
    {
        VoyageState::Present(voyage) if voyage.told < seq && seq <= voyage.last_seq => Voyage {
            told: seq,
            ..voyage.clone()
        },
        VoyageState::Present(_) | VoyageState::Absent | VoyageState::Unreadable => return,
    };
    put_voyage(ctx, &voyage);
}

/// **The launch agents** this fire: one per hull whose sailing plan reads with a
/// pending passage (agent = the hull, principal = the filing pilot), skipping
/// hulls whose held launch is in its backoff.
pub(crate) fn launch_agents(w: &World, state: &PortTickState) -> Vec<(ObjId, ObjId)> {
    let Some(store) = w.module_state::<TacsimStore>() else {
        return Vec::new();
    };
    store
        .voyages()
        .filter_map(|voyage| {
            let pending = voyage.pending.as_ref()?;
            let hull = voyage.hull;
            let backing_off = state
                .launch_backoff
                .get(&hull.0)
                .is_some_and(|at| state.fires.saturating_sub(*at) < LAUNCH_GRACE_FIRES);
            (!backing_off).then_some((hull, pending.pilot))
        })
        .collect()
}

/// Drop the launch arm's RAM for every hull whose sailing plan has no pending
/// passage (a launch, a lapse or a staff clear ended it) — one pass over three
/// small maps, at the top of every fire.
pub(crate) fn launch_prune(w: &World, state: &mut PortTickState) {
    let has_pending = |hull: u32| pending_of(w, ObjId(hull)).is_some();
    state.launch_backoff.retain(|hull, _| has_pending(*hull));
    state.launch_warned.retain(|hull| has_pending(*hull));
    state.filed_seen.retain(|hull, _| has_pending(*hull));
}

/// Read one ticket through this module's own typed-state lens, the key
/// re-checked (`ticket.ticket_id() == key`; the IC8 rule). `Err` when the lens
/// refuses or the ticket names another key — never "absent".
pub(crate) fn ticket_at(ctx: &Ctx<'_>, id: TicketId) -> Result<Option<Ticket>, ()> {
    let handles = ctx.w.module_state::<PortHandles>().ok_or(())?;
    let found = ctx
        .action_state(&handles.families.tickets, &id)
        .map_err(|_| ())?;
    match found {
        None => Ok(None),
        Some(ticket) if ticket.ticket_id() == id => Ok(Some(ticket.clone())),
        Some(_) => Err(()),
    }
}

/// **The launch step for one hull** (lead rulings L7, L14, L26, L29, L31): no
/// submission and no swap — the launch is a direct `mush_sim::depart` write.
pub(crate) fn launch_step(ctx: &mut Ctx<'_>, state: &mut PortTickState, hull: u32) -> Step {
    let hull = ObjId(hull);
    if gone(ctx.w, hull) {
        return Step::Refused;
    }
    let Some(pending) = pending_of(ctx.w, hull).cloned() else {
        return Step::Idle;
    };
    let id = TicketId {
        hull,
        seq: pending.seq,
    };
    let ticket = match ticket_at(ctx, id) {
        Ok(ticket) => ticket,
        Err(()) => {
            // Fail closed: an unreadable ticket (or one stored under another key)
            // holds the launch — paged once, the pilot told once, backed off.
            return launch_hold(
                ctx,
                state,
                hull,
                &pending,
                "the departure ticket does not read (or names another key)",
            );
        }
    };
    let Some(ticket) = ticket else {
        // No ticket: the burn may still be in flight (IC8's seam), else it lapses
        // after its grace, and the sequence is burned so a late landing is
        // impossible (the offer needs the pending).
        // Doc 18 cut IC14 row 10 (lead ruling L15): a burn passes queue → synchronous
        // prepare and stage → the writer's custody → install, and the ticket appears
        // only at the install, so `ticket_at` alone cannot see the middle. The lapse
        // waits while the pilot's burn is queued (`authoritative_pending`) **and**
        // while any prepared action owns the typed store
        // (`authoritative_commit_pending`: one at a time, so a burn in custody is
        // seen) — delayed a fire by another actor's commit at worst, never clearing a
        // pending a landing burn still needs.
        if ctx.authoritative_pending(pending.pilot) || ctx.authoritative_commit_pending() {
            return Step::Deferred;
        }
        let first_seen = *state.filed_seen.entry(hull.0).or_insert(state.fires);
        if state.fires.saturating_sub(first_seen) < LAUNCH_GRACE_FIRES {
            return Step::Idle;
        }
        clear_pending(ctx, hull, pending.seq);
        state.filed_seen.remove(&hull.0);
        if pending.order.is_none() {
            let port = venue_name(ctx.w, &pending.to_venue);
            notify(
                ctx,
                pending.pilot,
                &CLEARANCE_LAPSED.replace("{port}", &port),
            );
        } else {
            // Doc 18 cut IC11 (reviewer F1, lead rulings L7, L12): a queued depart's
            // lapse IS its "not admitted" — the order arm reads it by pull (Dropped)
            // and files again under the next attempt; retries are never notified.
            tracing::info!(
                hull = hull.0,
                seq = pending.seq,
                order = pending.order,
                attempt = pending.attempt,
                "tacsim: a queued departure's burn was not taken up; the order arm files it again"
            );
        }
        return Step::Worked;
    };
    // Doc 18 cut IC11 (IC9 residual F2, lead ruling L19): the ticket must be the
    // filing's — the plan and every filing field — or the launch holds, flying nothing.
    if ticket.plan != pending.plan
        || ticket.order != pending.order
        || ticket.attempt != pending.attempt
        || ticket.route_pack != pending.route_pack
        || ticket.route_version != pending.route_version
        || ticket.unit_definition != pending.unit_definition
    {
        return launch_hold(
            ctx,
            state,
            hull,
            &pending,
            "the departure ticket does not match the filed passage",
        );
    }
    let plan = &ticket.plan;
    if launched(ctx.w, hull, &ticket.route, plan.depart_ic) {
        clear_pending_with(ctx, hull, pending.seq, pending.arrival.clone());
        return Step::Worked;
    }
    // D-18-23 "at every executed step" (L31): a lease ended since the burn holds
    // the launch; it never flies. Cut IC13: the title counts as the charter does.
    if helm_of(ctx.w, pending.pilot).is_none_or(|helm| helm.hull != hull) {
        return launch_hold(
            ctx,
            state,
            hull,
            &pending,
            "the pilot neither holds this hull's title nor a charter of it",
        );
    }
    // L14 at the executed step: `+ship/leave`/`+ship/board` refuse while filed, but
    // a staff placement can still move the pilot off a docked hull; a burned
    // launch never flies with its pilot ashore — it holds until staff put them
    // back aboard or stand the clearance down.
    if !aboard(ctx.w, hull, pending.pilot) {
        return launch_hold(ctx, state, hull, &pending, "the pilot is no longer aboard");
    }
    let route = ctx.w.module_state::<TacsimStore>().and_then(|store| {
        let def = store.route_in_world(ctx.w, &ticket.route)?;
        let (pack, _) = store.route_source(ctx.w, &def)?;
        (pack.manifest().id() == &ticket.route_pack
            && pack.manifest().version() == ticket.route_version)
            .then_some(def)
    });
    let Some(route) = route else {
        return launch_hold(
            ctx,
            state,
            hull,
            &pending,
            "the route no longer materializes at the ticket's lane version",
        );
    };
    let leg_s: Vec<u32> = plan.legs.iter().map(|leg| leg.secs).collect();
    // Doc 20 cut SM.2: a hull lifting from a pad hears the pad drop away.
    let from_pad = matches!(berth_of(ctx.w, hull), Berth::Docked { def, .. } if def.site.is_some());
    match mush_sim::depart(ctx, hull, route, Some(leg_s), plan.depart_ic) {
        Ok(_) => {}
        Err(error) => {
            let why = error.to_string();
            return launch_hold(ctx, state, hull, &pending, &why);
        }
    }
    // In the same outbox as the journey: a restart sees both or neither.
    clear_pending_with(ctx, hull, pending.seq, pending.arrival.clone());
    state.filed_seen.remove(&hull.0);
    let name = name_of(ctx.w, hull);
    let seals = if from_pad { LIFT_SEALS } else { LOCK_SEALS };
    narrate_hull(ctx, hull, &seals.replace("{name}", &name));
    state.launching.insert(
        hull.0,
        Launching {
            pilot: pending.pilot,
            seq: pending.seq,
            staged_fire: state.fires,
            depart_ic: plan.depart_ic,
            to_venue: pending.to_venue.clone(),
            burn_t: plan.burn_t,
            crew_days: plan.crew_days,
            total_s: plan.total_s,
        },
    );
    Step::Worked
}

/// **A held launch** (lead ruling L29): page staff ONCE per hull per boot, tell
/// the pilot once, back off [`LAUNCH_GRACE_FIRES`]. The pending stays (the fuel is
/// spent and the ticket is the record); every backoff expiry retries, so a cause
/// staff fix launches the frozen plan at its own `depart_ic`.
pub(crate) fn launch_hold(
    ctx: &mut Ctx<'_>,
    state: &mut PortTickState,
    hull: ObjId,
    pending: &PendingVoyage,
    why: &str,
) -> Step {
    if state.launch_warned.insert(hull.0) {
        tracing::error!(
            hull = hull.0,
            seq = pending.seq,
            why,
            runbook = "fix the cause and the arm retries after its backoff (@domain/resume, \
                       @journey/show #<hull>); or @tacsim/port clear #<hull> to stand the \
                       clearance down",
            "tacsim: a burned launch is held"
        );
        let line = LAUNCH_HELD
            .replace("{name}", &name_of(ctx.w, hull))
            .replace("{port}", &venue_name(ctx.w, &pending.to_venue));
        notify(ctx, pending.pilot, &line);
    }
    state.launch_backoff.insert(hull.0, state.fires);
    // Doc 18 cut IC14 row 20 (lead ruling L16): already paged, so a held step — never
    // the walk's generic refusal warning, which re-fired every backoff cycle.
    Step::Held
}

/// **The launch's pull verification** (top of every fire; no swap): a launch
/// whose journey reads at its `depart_ic` tells the pilot [`CLEARANCE_GRANTED`]
/// and is forgotten; one not seen within [`LAUNCH_WATCH_FIRES`] is logged and
/// forgotten.
pub(crate) fn launch_verify(ctx: &mut Ctx<'_>, state: &mut PortTickState) {
    let staged: Vec<(u32, Launching)> = state
        .launching
        .iter()
        .map(|(hull, launching)| (*hull, launching.clone()))
        .collect();
    for (hull_id, launching) in staged {
        let hull = ObjId(hull_id);
        let journey = ctx
            .w
            .module_state::<mush_sim::JourneyBook>()
            .and_then(|book| book.get(hull))
            .filter(|journey| journey.departed().at == launching.depart_ic)
            .cloned();
        if let Some(journey) = journey {
            let remaining = journey.arrival_at().map_or(launching.total_s, |at| {
                at.saturating_sub(ctx.w.world_ic_now().at)
            });
            let line = CLEARANCE_GRANTED
                .replace("{burn}", &launching.burn_t.to_string())
                .replace("{days}", &launching.crew_days.to_string())
                .replace("{name}", &name_of(ctx.w, hull))
                .replace("{port}", &venue_name(ctx.w, &launching.to_venue))
                .replace("{icdays}", &ic_days(remaining));
            notify(ctx, launching.pilot, &line);
            // Doc 18 cut IC14 row 21: durable, so a reboot never tells it again.
            mark_told(ctx, hull, launching.seq);
            state.launching.remove(&hull_id);
        } else if state.fires.saturating_sub(launching.staged_fire) >= LAUNCH_WATCH_FIRES {
            tracing::warn!(
                hull = hull_id,
                seq = launching.seq,
                "tacsim: a launch's journey was not seen; its verification is dropped"
            );
            state.launching.remove(&hull_id);
        }
    }
}

/// **The grants a reboot lost** (doc 18 cut IC14 row 21; lead ruling L20): on the
/// first fire after a boot, every readable sailing plan with no pending passage and a
/// burned sequence the pilot was not told of (`last_seq > told`) is verified by pull —
/// its ticket `(hull, last_seq)` read, the hull's journey departed at the ticket's
/// `depart_ic` — and its `CLEARANCE_GRANTED` told once (the render and the notice rule
/// [`launch_verify`] applies), `told` written. A launch the RAM map is still verifying
/// is left to it, so nothing is told twice.
pub(crate) fn launch_retell(ctx: &mut Ctx<'_>, state: &PortTickState) {
    let untold: Vec<(ObjId, u32)> = ctx
        .w
        .module_state::<TacsimStore>()
        .map(|store| {
            store
                .voyages()
                .filter(|v| v.pending.is_none() && v.last_seq > v.told)
                .filter(|v| !state.launching.contains_key(&v.hull.0))
                .map(|v| (v.hull, v.last_seq))
                .collect()
        })
        .unwrap_or_default();
    for (hull, seq) in untold {
        let Ok(Some(ticket)) = ticket_at(ctx, TicketId { hull, seq }) else {
            continue;
        };
        let journey = ctx
            .w
            .module_state::<mush_sim::JourneyBook>()
            .and_then(|book| book.get(hull))
            .filter(|journey| journey.departed().at == ticket.plan.depart_ic)
            .cloned();
        let Some(journey) = journey else {
            continue;
        };
        // The ticket names neither the filer nor the destination's venue: the helm
        // (the charter's tenant or the title holder) is told, and the destination is
        // the journey's own far end, as `+ship` names it.
        let Some(pilot) = crate::port::coop::helm_holder(ctx.w, hull) else {
            continue;
        };
        let destination = journey.route().endpoints().1;
        let port = port_of_host(ctx.w, destination)
            .map_or_else(|| name_of(ctx.w, destination), |(name, _)| name);
        let remaining = journey.arrival_at().map_or(ticket.plan.total_s, |at| {
            at.saturating_sub(ctx.w.world_ic_now().at)
        });
        let line = CLEARANCE_GRANTED
            .replace("{burn}", &ticket.plan.burn_t.to_string())
            .replace("{days}", &ticket.plan.crew_days.to_string())
            .replace("{name}", &name_of(ctx.w, hull))
            .replace("{port}", &port)
            .replace("{icdays}", &ic_days(remaining));
        notify(ctx, pilot, &line);
        mark_told(ctx, hull, seq);
    }
}

// ---- @tacsim/port clear (the staff terminal, lead ruling L29) ----------------------

/// **Stand a hull's clearance down** (staff): the pending passage cleared with its
/// sequence kept, the pilot told [`CLEARANCE_STOOD_DOWN`]; a burned ticket is
/// kept as the record and nothing is refunded (lead ruling L30). Touches no tick
/// RAM: the next fire's [`launch_prune`] drops it.
///
/// # Errors
///
/// The staff-facing refusals.
pub(crate) fn clear_cmd(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<String, CmdError> {
    let no_clearance = || CmdError::msg(format!("#{} has no clearance on file.", hull.0));
    let Some(pending) = pending_of(ctx.w, hull).cloned() else {
        return Err(no_clearance());
    };
    if ctx
        .w
        .module_state::<mush_sim::JourneyBook>()
        .is_some_and(|book| book.under_way(hull))
    {
        return Err(CmdError::msg(format!(
            "#{} is under way; its clearance has flown.",
            hull.0
        )));
    }
    clear_pending(ctx, hull, pending.seq);
    let port = venue_name(ctx.w, &pending.to_venue);
    let line = CLEARANCE_STOOD_DOWN
        .replace("{name}", &name_of(ctx.w, hull))
        .replace("{port}", &port);
    notify(ctx, pending.pilot, &line);
    // Doc 18 cut IC11 (reviewer F4, lead ruling L28): a queued depart stood down is
    // recorded on its sailing plan in the same command — a stopped hold, the attempt
    // consumed — so the order arm never mistakes it for a drop and re-files it.
    let held = pending.order.is_some_and(|order| {
        crate::port::executor::stand_down(ctx, hull, order, pending.attempt, &port)
    });
    let tail = match (pending.order, held) {
        (Some(order), true) => format!(" The sailing plan holds at order {order}."),
        _ => String::new(),
    };
    Ok(format!(
        "Clearance {} for #{} stood down; the ticket, if burned, is kept as the record and \
         nothing is refunded.{tail}",
        pending.seq, hull.0
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ic9_time_renders() {
        assert_eq!(ic_days(1_694_961), "19.6");
        assert_eq!(ic_days(86_399), "0.9");
        assert_eq!(ic_days(-5), "0.0");
        assert_eq!(real_span(0), "under a minute");
        assert_eq!(real_span(59), "under a minute");
        assert_eq!(real_span(267), "4 min");
        assert_eq!(real_span(3_600 + 120), "1 h 2 min");
        assert_eq!(utc_hhmm(0), "00:00");
        assert_eq!(utc_hhmm(54_360), "15:06");
        assert_eq!(utc_hhmm(-60), "23:59");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_743), (2026, 10, 17));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        let now = 20_743 * 86_400 + 60;
        assert_eq!(utc_day_word(now, now + 100), "today");
        assert_eq!(utc_day_word(now, now + 86_400), "tomorrow");
        assert_eq!(utc_day_word(now, now + 2 * 86_400), "on 2026-10-19");
        assert_eq!(tonnes_one_decimal(142_184), "142.2");
        assert_eq!(tonnes_one_decimal(142_149), "142.1");
        assert_eq!(tonnes_one_decimal(142_150), "142.2");
    }

    #[test]
    fn ic9_voyage_keys_are_strict_and_the_document_checks() {
        assert_eq!(voyage_key(ObjId(5011)), "tacsim:voyage:5011");
        assert_eq!(voyage_key_owner("tacsim:voyage:5011"), Some(ObjId(5011)));
        for bad in [
            "tacsim:voyage:05011",
            "tacsim:voyage:#5011",
            "tacsim:voyage:",
            "x:5011",
        ] {
            assert_eq!(voyage_key_owner(bad), None, "{bad}");
        }
        let empty = Voyage {
            schema: VOYAGE_SCHEMA,
            hull: ObjId(5011),
            last_seq: 3,
            told: 3,
            pending: None,
            arrival: None,
        };
        assert!(empty.check().is_ok());
        assert!(
            Voyage {
                schema: 2,
                ..empty.clone()
            }
            .check()
            .is_err()
        );
        let wire = serde_json::to_value(&empty).unwrap();
        assert_eq!(
            serde_json::from_value::<Voyage>(wire.clone()).unwrap(),
            empty
        );
        let mut extra = wire;
        extra
            .as_object_mut()
            .unwrap()
            .insert("extra".into(), serde_json::json!(1));
        assert!(serde_json::from_value::<Voyage>(extra).is_err());
    }

    /// A filed passage that checks (`families.rs`'s ticket fixture's plan).
    fn filed(seq: u32) -> PendingVoyage {
        let id = |word: &str| CanonicalId::new(word).unwrap();
        let depart = 1_000_000;
        let a = crate::port::families::LegPlan {
            to: id("kirkwood_depot"),
            body: id("kirkwood"),
            dist_km: 1_000,
            secs: 110_305,
            t0: depart,
            t1: depart + 110_305,
            ascent_mps: 0,
        };
        let b = crate::port::families::LegPlan {
            to: id("stickney_yards"),
            body: id("mars"),
            dist_km: 1_000,
            secs: 1_683_834,
            t0: a.t1,
            t1: a.t1 + 1_683_834,
            ascent_mps: 0,
        };
        let total = b.t1 - depart;
        PendingVoyage {
            seq,
            pilot: ObjId(42),
            to_venue: id("stickney"),
            vault: 7,
            occupants: 1,
            payload_kg: 142_184,
            route_pack: id(LANE_PACK),
            route_version: 3,
            unit_definition: DefinitionDigest::from_bytes([7; 32]),
            plan: PassagePlan {
                route: id("ceres_stickney"),
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
            order: None,
            attempt: 1,
            filed_ic: depart,
            filed_at: 1_700_000_000,
            arrival: None,
            carried: Vec::new(),
        }
    }

    /// Reviewer T4: the pending rules `Voyage::check` holds (the load arm skips a
    /// document that breaks one, keeping its key claimed).
    #[test]
    fn ic9_a_pending_passage_checks_its_sequence_occupants_and_attempt() {
        let good = Voyage {
            schema: VOYAGE_SCHEMA,
            hull: ObjId(5011),
            last_seq: 3,
            told: 3,
            pending: Some(filed(4)),
            arrival: None,
        };
        good.check().unwrap();
        let wire = serde_json::to_value(&good).unwrap();
        assert_eq!(serde_json::from_value::<Voyage>(wire).unwrap(), good);
        let with = |edit: &dyn Fn(&mut PendingVoyage)| {
            let mut voyage = good.clone();
            if let Some(pending) = voyage.pending.as_mut() {
                edit(pending);
            }
            voyage
        };
        for (why, bad) in [
            ("a reused sequence", with(&|p| p.seq = 3)),
            ("a skipped sequence", with(&|p| p.seq = 5)),
            ("no occupants", with(&|p| p.occupants = 0)),
            (
                "too many occupants",
                with(&|p| p.occupants = MAX_TICKET_OCCUPANTS + 1),
            ),
            ("attempt 0", with(&|p| p.attempt = 0)),
            (
                "a plan that does not add up",
                with(&|p| p.plan.total_s += 1),
            ),
        ] {
            assert!(bad.check().is_err(), "{why} is refused");
        }
        assert_eq!(MAX_TICKET_OCCUPANTS, 16, "occupants 1 to 16");
        assert!(with(&|p| p.occupants = 16).check().is_ok());
        // `last_seq` at the top of its range cannot file another passage.
        assert!(
            Voyage {
                last_seq: u32::MAX,
                pending: Some(filed(0)),
                ..good.clone()
            }
            .check()
            .is_err()
        );
    }

    #[test]
    fn ic9_args_take_a_word_at_now_and_refuse_later() {
        assert_eq!(
            parse_args("stickney").unwrap().word,
            Some("stickney".to_owned())
        );
        assert_eq!(
            parse_args("Stickney Yards").unwrap().word,
            Some("Stickney Yards".to_owned())
        );
        assert_eq!(
            parse_args("stickney at=now").unwrap().word,
            Some("stickney".to_owned())
        );
        assert_eq!(parse_args("").unwrap().word, None);
        assert!(parse_args("stickney at=+2h").is_err());
    }

    /// Doc 19 cut A.2: `via=`, `throttle=` and `policy=` are read, bounded and
    /// taken once each.
    #[test]
    fn a2_args_take_a_waypoint_a_throttle_and_a_policy() {
        let args = parse_args("stickney via=kirkwood_depot throttle=500 policy=cheapest").unwrap();
        assert_eq!(args.word, Some("stickney".to_owned()));
        assert_eq!(args.via, Some("kirkwood_depot".to_owned()));
        assert_eq!(args.throttle, Some(500));
        assert_eq!(args.policy, Some(RoutePolicy::Cheapest));
        assert!(
            parse_args("stickney throttle=100").is_err(),
            "below the band"
        );
        assert!(
            parse_args("stickney throttle=1001").is_err(),
            "above the band"
        );
        assert!(
            parse_args("stickney throttle=full").is_err(),
            "not a number"
        );
        assert!(parse_args("stickney policy=slow").is_err());
        assert!(parse_args("stickney via=").is_err());
        assert_eq!(parse_throttle("250"), Ok(250));
        assert_eq!(parse_throttle("1000"), Ok(1_000));
        assert!(parse_throttle("249").is_err());
    }

    /// Doc 18 cut IC14 row 21 (lead ruling L8b): `told` is required on the wire and
    /// never above `last_seq`.
    #[test]
    fn ic14_told_is_required_and_bounded() {
        let voyage = Voyage {
            schema: VOYAGE_SCHEMA,
            hull: ObjId(5011),
            last_seq: 3,
            told: 2,
            pending: None,
            arrival: None,
        };
        assert!(voyage.check().is_ok());
        assert!(
            Voyage {
                told: 4,
                ..voyage.clone()
            }
            .check()
            .is_err(),
            "told past last_seq"
        );
        let mut wire = serde_json::to_value(&voyage).expect("serializes");
        wire.as_object_mut().map(|m| m.remove("told"));
        assert!(
            serde_json::from_value::<Voyage>(wire).is_err(),
            "no default: a document without `told` does not read"
        );
    }

    /// Doc 18 cut IC14 row 22 (lead ruling L18): over the rail the tenths round up, so
    /// 181,001–181,049 kg never read "181.0" against a rated 181 t.
    #[test]
    fn ic14_an_over_rail_payload_never_reads_equal_to_its_rating() {
        for kg in [181_001_u64, 181_049, 181_050] {
            assert_eq!(tonnes_over(kg, 181_000), "181.1", "{kg}");
        }
        assert_eq!(tonnes_over(181_000, 181_000), "181.0");
        assert_eq!(tonnes_over(142_184, 181_000), tonnes_one_decimal(142_184));
        assert_eq!(tonnes_over(181_101, 181_000), "181.2");
    }

    /// Doc 19 cut A.8 (D-19-22): the bisection finds the last throttle that fits.
    #[test]
    fn a8_the_highest_fitting_throttle_is_found_by_bisection() {
        assert_eq!(highest_fitting(250, 1_000, |t| t <= 450), Some(450));
        assert_eq!(highest_fitting(250, 1_000, |t| t <= 999), Some(999));
        assert_eq!(highest_fitting(250, 1_000, |t| t <= 251), Some(251));
        assert_eq!(highest_fitting(250, 1_000, |t| t <= 250), Some(250));
        assert_eq!(highest_fitting(250, 1_000, |_| true), Some(1_000));
        assert_eq!(highest_fitting(250, 1_000, |_| false), None);
        let mut calls = 0;
        assert_eq!(
            highest_fitting(250, 1_000, |t| {
                calls += 1;
                t <= 466
            }),
            Some(466)
        );
        assert!(calls <= 12, "{calls} plans");
    }

    /// **Doc 20 cut SM.2 (decision 21)**: A.8's bisection assumed every throttle from the
    /// band's floor plans; the landing law refuses below a body's floor (Vesta at the rail
    /// under 600), so with a predicate false under 600 (the landing floor) and over 800
    /// (the tank) the band's floor finds nothing — the defect — and the landing floor
    /// finds 800.
    #[test]
    fn sm2_the_hint_bisects_from_the_landing_floor() {
        let fits = |t: u32| (600..=800).contains(&t);
        assert_eq!(highest_fitting(250, 1_000, fits), None, "A.8's bound");
        assert_eq!(highest_fitting(600, 1_000, fits), Some(800));
    }

    /// **The hint's lower bound is the landing floor** (decision 21; reviewer T3):
    /// [`hint_floor`] over the shipped lane — a site route at the rail starts at its
    /// landing floor (Ceres 672, Vesta 600), a station route at the band's 250, a listing
    /// of both at the lesser, and a drive that lands nowhere gives no bound.
    #[test]
    fn sm2_the_hint_starts_at_the_least_landing_floor() {
        use crate::definitions::logistics::DriveLaw;
        let Ok(pack) = crate::pack::load_pack(crate::definitions::content::LANE_JSON) else {
            panic!("the shipped lane loads");
        };
        let id = |s: &str| CanonicalId::new(s).expect("a canonical id");
        let flip = crate::port::passage::HullDrive {
            class: id("coast_hauler"),
            law: DriveLaw::Flip {
                accel_mg: 50,
                exhaust_kms: 6_000,
            },
            payload_ceiling_kg: 181_000,
            throttle_permille: DEFAULT_THROTTLE_PERMILLE,
        };
        let floor = |routes: &[&str]| {
            hint_floor(
                &pack,
                &routes.iter().map(|r| id(r)).collect::<Vec<_>>(),
                &flip,
                240_000,
                181_000,
            )
        };
        assert_eq!(floor(&["ceres_ahuna"]), Some(672));
        assert_eq!(floor(&["ceres_rheasilvia"]), Some(600));
        assert_eq!(floor(&["ceres_stickney"]), Some(MIN_THROTTLE_PERMILLE));
        assert_eq!(
            floor(&["ceres_ahuna", "ceres_rheasilvia"]),
            Some(600),
            "the lesser"
        );
        assert_eq!(
            floor(&["nowhere", "ceres_ahuna"]),
            Some(672),
            "a route that answers"
        );
        assert_eq!(floor(&["nowhere"]), None);
        let coast = crate::port::passage::HullDrive {
            law: DriveLaw::Coast {
                kms: 226,
                burn_permille: 105,
            },
            ..flip.clone()
        };
        assert_eq!(
            hint_floor(&pack, &[id("ceres_ahuna")], &coast, 240_000, 181_000),
            None,
            "a coast drive lands nowhere: no hint"
        );
    }
}
