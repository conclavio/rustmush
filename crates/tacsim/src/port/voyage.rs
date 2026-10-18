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
use crate::port::coop::{helm_of, name_of, port_name};
use crate::port::def::PortDef;
use crate::port::families::{MAX_TICKET_OCCUPANTS, PassagePlan, PortHandles, Ticket, TicketId};
use crate::port::gauges;
use crate::port::map::PortMap;
use crate::port::passage::{PlanError, choose_route, payload_kg};
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
/// `via=` (reserved for the astronomy engine).
pub const VIA_NOT_YET: &str =
    "A passage by a chosen waypoint is not yet offered; the planner takes the fastest route.";
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

/// Real seconds as `{h} h {m} m`, `{m} m` or `under a minute`.
#[must_use]
pub(crate) fn real_span(secs: i64) -> String {
    let secs = secs.max(0);
    let (hours, minutes) = (secs / 3_600, (secs % 3_600) / 60);
    if hours > 0 {
        format!("{hours} h {minutes} m")
    } else if minutes > 0 {
        format!("{minutes} m")
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

/// The arguments `+ship/plan` and `+ship/undock` take: a destination word (a
/// routes key, or a prefix of the destination port's name), `at=now`, and the
/// reserved `via=`.
fn parse_args(raw: &str) -> Result<Option<String>, CmdError> {
    let mut words: Vec<&str> = Vec::new();
    for token in raw.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("via=") {
            return Err(CmdError::msg(VIA_NOT_YET));
        }
        if let Some(at) = lower.strip_prefix("at=") {
            if at != "now" {
                return Err(CmdError::msg(AT_NOT_YET));
            }
            continue;
        }
        words.push(token);
    }
    Ok((!words.is_empty()).then(|| words.join(" ")))
}

/// One destination a berth clears for: its venue word, its port name and its
/// route list (D-18-47).
#[derive(Clone, Debug)]
struct Destination {
    venue: CanonicalId,
    port: String,
    routes: Vec<CanonicalId>,
}

/// The destinations `def` clears for, in its `routes` map order.
fn destinations(w: &World, def: &PortDef) -> Vec<Destination> {
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
fn match_destination<'d>(dests: &'d [Destination], word: &str) -> Option<&'d Destination> {
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
fn occupants(w: &World, airlock: ObjId) -> u32 {
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
struct Basis {
    /// The latched lane version.
    version: u32,
    /// The hull's unit definition digest (L34).
    digest: DefinitionDigest,
    /// The hull spec (drive, dry mass, allowance, accepted goods).
    spec: HullSpec,
    /// The hold vault.
    vault: VaultId,
    /// Souls aboard.
    occupants: u32,
    /// The payload, kilograms.
    payload: u64,
    /// The propellant pool, tonnes.
    tank: u64,
    /// The provisions pool, crew-days.
    locker: u64,
    /// The IC second the plan departs at.
    depart_ic: i64,
}

/// Read the basis of a passage for `view`'s hull, `occupants` souls aboard.
fn basis(ctx: &Ctx<'_>, view: &HullView<'_>, occupants: u32) -> Result<Basis, String> {
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
    let (digest, spec) = hull_spec_of(store.definitions(), goods, &view.port.programme.unit)?;
    let vault = hold_vault(w, view).ok_or_else(|| "the hull has no hold vault".to_owned())?;
    let read = |good: &CanonicalId| {
        read_pool(ctx, EndpointRef(vault), good).map_err(|error| PlanError::Unreadable {
            what: format!("the hold's {good} ({error:?})"),
        })
    };
    let payload = payload_kg(goods, &spec.accepted, read, occupants, spec.allowance_kg)
        .map_err(|e| e.to_string())?;
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
        tank: pool_of(Consumable::Propellant)?,
        locker: pool_of(Consumable::Provisions)?,
        depart_ic: w.world_ic_now().at,
    })
}

/// Plan the fastest listed route on `basis` over the latched lane's sky (§4.4:
/// one planner over the sky at departure; lead ruling L23: the lane addressed by
/// its latched `(pack, version)`).
fn plan_on(w: &World, basis: &Basis, routes: &[CanonicalId]) -> Result<PassagePlan, PlanError> {
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| PlanError::Unreadable {
            what: "the layer's state".to_owned(),
        })?;
    let lane = CanonicalId::new(LANE_PACK).map_err(|_| PlanError::Overflow)?;
    let loaded = store
        .pack(&lane, basis.version)
        .ok_or_else(|| PlanError::Unreadable {
            what: format!("the lane at version {}", basis.version),
        })?;
    let sky = Sky::from_pack(&loaded.pack, &store.config().time.ephemeris);
    choose_route(
        &sky,
        &loaded.pack,
        routes,
        &basis.spec.drive,
        basis.spec.dry_kg,
        basis.payload,
        basis.occupants,
        basis.depart_ic,
        None,
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
    let word = parse_args(ctx.args.raw())?;
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
            Ok(basis) => basis,
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
    let plan = plan_on(w, basis, &dest.routes);
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
        Err(error) => return Err(error.to_string()),
    };
    // 4–7.
    lines.push(format!(
        "Payload {} t of {} t.",
        tonnes_one_decimal(basis.payload),
        ceiling / 1_000
    ));
    let permille = match basis.spec.drive.law {
        crate::definitions::logistics::DriveLaw::Coast { burn_permille, .. } => burn_permille,
    };
    let tank = if basis.tank >= plan.burn_t {
        format!("{} t left", basis.tank - plan.burn_t)
    } else {
        format!("{} t short", plan.burn_t - basis.tank)
    };
    lines.push(format!(
        "Burn {} t (tank {} t; {tank}). Every tonne aboard, reserve included, costs {permille} kg \
         of burn.",
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
    let icdays = ic_days(plan.total_s);
    let arrival = plan.depart_ic.saturating_add(plan.total_s);
    lines.push(match (real_until(w, arrival), eta(w, arrival)) {
        (Some(real), Some((hhmm, day))) => format!(
            "Passage {icdays} IC days ≈ {} real; arrives about {hhmm} UTC {day}.",
            real_span(real)
        ),
        _ => format!("Passage {icdays} IC days; the clock is stopped."),
    });
    Ok(lines)
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
    let plan = match plan_on(w, &basis, &dest.routes) {
        Ok(plan) => plan,
        Err(PlanError::Overloaded {
            payload_kg,
            ceiling_kg,
        }) => {
            return Err(refusal(
                K::Overloaded,
                OVERLOADED
                    .replace("{name}", &name)
                    .replace("{payload}", &tonnes_over(payload_kg, ceiling_kg))
                    .replace("{ceiling}", &(ceiling_kg / 1_000).to_string()),
            ));
        }
        Err(error) => {
            tracing::warn!(hull = hull.0, %error, "tacsim: a departure could not plan the passage");
            return Err(dark());
        }
    };
    if basis.tank < plan.burn_t {
        return Err(refusal(
            K::TankShort,
            TANK_SHORT
                .replace("{port}", &port)
                .replace("{burn}", &plan.burn_t.to_string())
                .replace("{tank}", &basis.tank.to_string()),
        ));
    }
    if basis.locker < u64::from(plan.crew_days) {
        return Err(refusal(
            K::StoresShort,
            STORES_SHORT
                .replace("{port}", &port)
                .replace("{days}", &plan.crew_days.to_string())
                .replace("{have}", &basis.locker.to_string()),
        ));
    }
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
    };
    let voyage = Voyage {
        schema: VOYAGE_SCHEMA,
        hull,
        last_seq,
        told,
        pending: Some(pending),
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
    if raw.split_whitespace().all(|token| token.contains('=')) {
        return Err(CmdError::msg(UNDOCK_USAGE));
    }
    let Some(word) = parse_args(&raw)? else {
        return Err(CmdError::msg(UNDOCK_USAGE));
    };
    let Some(charter) = helm_of(ctx.w, actor) else {
        return Err(no_charter(ctx.w, actor));
    };
    let filing = prepare_departure(ctx, actor, charter.hull, &word, Queued::Live)?;
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
    let told = told_of(ctx.w, hull).min(seq);
    put_voyage(
        ctx,
        &Voyage {
            schema: VOYAGE_SCHEMA,
            hull,
            last_seq: seq,
            told,
            pending: None,
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
        clear_pending(ctx, hull, pending.seq);
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
    match mush_sim::depart(ctx, hull, route, Some(leg_s), plan.depart_ic) {
        Ok(_) => {}
        Err(error) => {
            let why = error.to_string();
            return launch_hold(ctx, state, hull, &pending, &why);
        }
    }
    // In the same outbox as the journey: a restart sees both or neither.
    clear_pending(ctx, hull, pending.seq);
    state.filed_seen.remove(&hull.0);
    let name = name_of(ctx.w, hull);
    narrate_hull(ctx, hull, &LOCK_SEALS.replace("{name}", &name));
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
        assert_eq!(real_span(267), "4 m");
        assert_eq!(real_span(3_600 + 120), "1 h 2 m");
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
            secs: 959_361,
            t0: depart,
            t1: depart + 959_361,
        };
        let b = crate::port::families::LegPlan {
            to: id("stickney_yards"),
            body: id("mars"),
            dist_km: 1_000,
            secs: 735_600,
            t0: a.t1,
            t1: a.t1 + 735_600,
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
            },
            order: None,
            attempt: 1,
            filed_ic: depart,
            filed_at: 1_700_000_000,
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
    fn ic9_args_take_a_word_at_now_and_refuse_via_and_later() {
        assert_eq!(parse_args("stickney").unwrap(), Some("stickney".to_owned()));
        assert_eq!(
            parse_args("Stickney Yards").unwrap(),
            Some("Stickney Yards".to_owned())
        );
        assert_eq!(
            parse_args("stickney at=now").unwrap(),
            Some("stickney".to_owned())
        );
        assert_eq!(parse_args("").unwrap(), None);
        assert!(parse_args("stickney via=kirkwood_depot").is_err());
        assert!(parse_args("stickney at=+2h").is_err());
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
}
