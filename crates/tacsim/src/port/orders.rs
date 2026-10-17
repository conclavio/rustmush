//! **Captain's orders: the sailing plan the port runs while the pilot is off
//! watch** (doc 18 cut IC11; doc 18 §4.4 "The queued-orders model", "Verbs",
//! "The fiction of the queue", "Failure handling"; D-18-31, D-18-32, D-18-41;
//! Q-18-22, Q-18-24, Q-18-28; lead rulings L1–L29 of the IC11 brief).
//!
//! - **The document** ([`Orders`], one `tacsim:orders:{hull}` per hull, D-18-31):
//!   a closed vocabulary (buy | sell | depart | stop, [`What`]), at most
//!   [`MAX_ORDERS`] orders, a FIFO log of at most [`MAX_ORDER_LOG_LINES`] lines of
//!   at most [`MAX_ORDER_LOG_BYTES`] bytes, the one step in flight ([`Inflight`])
//!   and the hold that stands, if any ([`HoldNote`]). It is written whole by
//!   `crate::store::put_orders`, which runs [`Orders::check`] first, so no writer —
//!   a verb or the order arm — can store a shape the loader would refuse.
//! - **The verbs** (`+ship/orders`, `+ship/order …`, Q-18-22/28): file and amend the
//!   plan aboard the Dray or in the port where she is docked; they write the one
//!   document and submit nothing. Edits write no log line (L18): the log carries
//!   execution only, and every rendered line names the document's author as its
//!   signatory (Q-18-24, L27).
//! - **The executor** is `crate::port::executor` on the `tacsim.port` tick (D-18-32):
//!   it acts only as the document's `author`, and only while that author's live
//!   charter names the hull (L14, D-18-23).
//!
//! **The four hold shapes** (reviewer F2; the IC11 brief's item 2 table) — every hold
//! the arm or a verb writes is one of them, and [`Orders::check`] admits exactly
//! these: *stopped* (plan `Held`, a note with `retry: false` naming the cursor order,
//! which is `Held`); *stopped over inflight* (plan `Held`, the note naming the
//! `Inflight` order, whose record stays); *waiting* (plan `Running`, a note with
//! `retry: true` naming the cursor order, `Pending`); *waiting over inflight* (plan
//! `Running`, the note naming the `Inflight` order).

use std::collections::BTreeSet;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::definitions::GoodsTable;
use crate::port::apply::{credit_scale, minor_of};
use crate::port::board::{label_of, money, unit_of};
use crate::port::coop::{live_charter, name_of, port_name};
use crate::port::def::PortDef;
use crate::port::executor::MAX_ORDER_REFUSALS;
use crate::port::families::{TicketId, strict_decimal_u32};
use crate::port::ship::{Berth, berth_of, hull_view};
use crate::port::voyage::{
    Passage, VIA_NOT_YET, VoyageState, aboard, lane_ports, passage_of, souls_aboard, ticket_at,
    utc_day_word, utc_hhmm, venue_name,
};
use crate::port::when::{
    MAX_WINDOW_SECS, NotBefore, hhmm_minutes, parse_at, parse_until, render_due, resolve_due,
};
use crate::store::{TacsimStore, put_orders};

// ---- the document ------------------------------------------------------------------

/// The sailing plan's document prefix (`tacsim:orders:{hull.0}`).
pub const ORDERS_PREFIX: &str = "tacsim:orders:";
/// The sailing plan's schema.
pub const ORDERS_SCHEMA: u16 = 1;
/// Most orders one plan holds (doc 18 §4.4).
pub const MAX_ORDERS: usize = 16;
/// Most log lines one plan keeps (oldest dropped; doc 18 §4.4).
pub const MAX_ORDER_LOG_LINES: usize = 32;
/// Most bytes one log line keeps (cut on a char boundary).
pub const MAX_ORDER_LOG_BYTES: usize = 240;
/// How many log lines `+ship/orders` shows.
pub const ORDERS_SHOWN_LOG: usize = 8;
/// Most listed passengers one depart carries (owner item O6).
pub const MAX_DEPART_CREW: usize = 15;
/// The highest `min=`/`max=` price, major units (owner item O6).
pub const MAX_PRICE_MAJOR: u64 = 1_000_000;

/// `tacsim:orders:{hull.0}` — the numeric id, never `ObjId`'s `#`-prefixed Display.
#[must_use]
pub fn orders_key(hull: ObjId) -> SmolStr {
    SmolStr::new(format!("{ORDERS_PREFIX}{}", hull.0))
}

/// The strict inverse of [`orders_key`]: the tail as a strict decimal `u32`.
#[must_use]
pub fn orders_key_owner(key: &str) -> Option<ObjId> {
    key.strip_prefix(ORDERS_PREFIX)
        .and_then(strict_decimal_u32)
        .map(ObjId)
}

/// **One hull's sailing plan** (doc 18 §4.4, D-18-31): the whole
/// `tacsim:orders:{hull}` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Orders {
    /// [`ORDERS_SCHEMA`].
    pub schema: u16,
    /// The hull (the identity rule: its key's).
    pub hull: ObjId,
    /// The pilot who filed it: the executor acts only as them (L14).
    pub author: ObjId,
    /// Bumped by every edit verb, never by the executor.
    pub rev: u64,
    /// Running, paused or held.
    pub state: PlanState,
    /// The hold that stands, if any (one of the four shapes).
    pub hold: Option<HoldNote>,
    /// The id of the first order whose state is pending, inflight or held (L16).
    pub cursor: Option<u32>,
    /// The next order id; ids are never reused within a document (reviewer F11).
    pub next_id: u32,
    /// The one step in flight.
    pub inflight: Option<Inflight>,
    /// Queue order, at most [`MAX_ORDERS`].
    pub orders: Vec<Order>,
    /// Oldest first, at most [`MAX_ORDER_LOG_LINES`].
    pub log: Vec<LogLine>,
}

/// The plan's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanState {
    /// The executor takes the next step when it is due.
    Running,
    /// No new step (an inflight one is still verified) until `+ship/order resume`.
    Paused,
    /// A stopped hold stands until `+ship/order resume` (owner item O4).
    Held,
}

/// **A hold** (doc 18 §4.4 "Failure handling"; lead ruling L10): a *stopped* one
/// (`retry: false`) needs `+ship/order resume`; a *waiting* one (`retry: true`) is
/// re-checked every fire and clears by itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HoldNote {
    /// The order it names: always the cursor.
    pub order: u32,
    /// The reason, a sentence.
    pub reason: String,
    /// The real second it was raised.
    pub since: i64,
    /// `true` for a waiting note, `false` for a stopped hold.
    pub retry: bool,
}

/// **One order** (doc 18 §4.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Order {
    /// Stable within the document; what `del`, `move` and `skip` name.
    pub id: u32,
    /// Pending, inflight, done, skipped or held.
    pub state: OrderState,
    /// `>= 1`; bumped ONLY when a staged act is proven dropped, so a depart's burn key
    /// `…/<order>/<attempt>` changes only on a proven drop (L7, L26).
    pub attempt: u32,
    /// Closed calls refused in a row, `0..=MAX_ORDER_REFUSALS`; reset on done, skipped
    /// and resume (reviewer F6).
    pub refusals: u8,
    /// The real second before which the executor leaves it (a dropped or refused act).
    pub retry_at: Option<i64>,
    /// A depart's "set for" line has been logged.
    pub announced: bool,
    /// A depart's reach (lead ruling L9; reviewer F8); `None` on every other kind.
    pub reached: Option<Reached>,
    /// What it does.
    pub what: What,
}

/// **Where and when a depart was first found runnable** (L9): valid only while the
/// hull is docked at `venue`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reached {
    /// The real second of the reach.
    pub utc: i64,
    /// The venue the hull was docked at.
    pub venue: CanonicalId,
}

/// An order's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderState {
    /// Not yet run.
    Pending,
    /// Its act is staged and being verified.
    Inflight,
    /// Carried out.
    Done,
    /// Passed over.
    Skipped,
    /// Stopped under a hold.
    Held,
}

/// **The closed vocabulary** (doc 18 §4.4; nested wire, lead ruling L3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum What {
    /// Buy a good at the venue's lowest ask.
    Buy {
        /// The good.
        good: CanonicalId,
        /// Units or `fill` (consumables only).
        qty: Qty,
        /// The highest price per unit, major units.
        max_major: Option<u64>,
        /// What a short clamp does.
        on_short: OnShort,
    },
    /// Sell a good into the venue's best bid.
    Sell {
        /// The good.
        good: CanonicalId,
        /// Units or `all`.
        qty: Qty,
        /// The lowest price per unit, major units.
        min_major: Option<u64>,
        /// What a short clamp does.
        on_short: OnShort,
    },
    /// Leave for a lane port.
    Depart {
        /// The destination's venue word.
        to: CanonicalId,
        /// When it may leave.
        not_before: NotBefore,
        /// How long after its hour it may still leave, real seconds.
        not_after_s: Option<u32>,
        /// What a failure does (`skip` or `hold`).
        on_fail: OnShort,
        /// The listed passengers: aboard at filing, other than the pilot (L11).
        crew: Vec<ObjId>,
    },
    /// A barrier: when reached it marks itself done and pauses the plan (owner item O5).
    Stop,
}

/// **An order's quantity**: `all` (sell), `fill` (a consumable buy) or a count.
/// The wire is `"all"`, `"fill"` or an integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Qty {
    /// Everything aboard (sell only).
    All,
    /// What the next departure needs (a consumable buy only; D-18-41).
    Fill,
    /// A count of ledger units.
    Units(u64),
}

/// The wire of [`Qty`].
#[derive(Deserialize)]
#[serde(untagged)]
enum QtyWire {
    Word(String),
    Units(u64),
}

impl Serialize for Qty {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Qty::All => serializer.serialize_str("all"),
            Qty::Fill => serializer.serialize_str("fill"),
            Qty::Units(n) => serializer.serialize_u64(*n),
        }
    }
}

impl<'de> Deserialize<'de> for Qty {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match QtyWire::deserialize(deserializer)? {
            QtyWire::Units(n) => Ok(Qty::Units(n)),
            QtyWire::Word(word) => match word.as_str() {
                "all" => Ok(Qty::All),
                "fill" => Ok(Qty::Fill),
                _ => Err(serde::de::Error::custom(
                    "a quantity is \"all\", \"fill\" or a count",
                )),
            },
        }
    }
}

/// What a short clamp (or a failed depart) does (doc 18 §4.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnShort {
    /// Take what the clamp allows.
    Partial,
    /// Pass the order over.
    Skip,
    /// Stop the plan at it.
    Hold,
}

/// **The one step in flight** (doc 18 §4.4 "Offline execution"): written in the same
/// outbox as the act it records, verified by pull on later fires.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inflight {
    /// The order.
    pub order: u32,
    /// The order's attempt at staging.
    pub attempt: u32,
    /// The burn key, or the market leg's key `market.lot.<op>.<contract:x>` (audit).
    pub key: String,
    /// What the act made.
    pub refs: InflightRefs,
    /// The real second it was staged, or last re-armed by resume or by a late first
    /// re-stage (the re-stage clock: the second deadline is always a full
    /// `ORDER_SETTLE_GRACE_SECS` past the first re-stage).
    pub staged_utc: i64,
    /// Informational: a bare IC second on the continuous axis.
    pub issued_ic: i64,
    /// 0 as staged; 1 re-staged once; 2 the second deadline evaluated. At most 2.
    pub restaged: u8,
    /// What the order asked.
    pub want: u64,
    /// What the clamp staged.
    pub qty: u64,
    /// The bound words that bit, in log order.
    pub bounds: Vec<SmolStr>,
    /// A sale's line (`BidSale.line`), clamped to [`MAX_ORDER_LOG_BYTES`].
    pub line: Option<String>,
}

/// What an inflight act made — filled after the call, so [`Orders::check`] does not
/// constrain it (the pre-check runs with every field `None`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InflightRefs {
    /// The market quote, `QuoteId.0`.
    pub quote: Option<u64>,
    /// The economy contract, `ContractId.0`.
    pub contract: Option<u64>,
    /// The departure ticket, `TicketId::key_text`.
    pub ticket: Option<String>,
}

/// One log line.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogLine {
    /// The real second it was written.
    pub utc: i64,
    /// Informational: the IC second it was written (a bare second, never relabelled).
    pub ic: i64,
    /// The order it is about.
    pub order: u32,
    /// The bare line (the signature is rendered, not stored; L27).
    pub text: String,
}

/// **One hull's sailing plan, as the store holds it.**
#[derive(Clone, Copy, Debug)]
pub enum OrdersState<'a> {
    /// No document.
    Absent,
    /// The readable document.
    Present(&'a Orders),
    /// A claimed key whose body did not read: fail closed.
    Unreadable,
}

/// Whether an order is unfinished (the cursor's states, L16).
fn unfinished(state: OrderState) -> bool {
    matches!(
        state,
        OrderState::Pending | OrderState::Inflight | OrderState::Held
    )
}

impl Orders {
    /// **A fresh plan** for `hull` filed by `author`: revision 0, running, ids from
    /// `next_id` (reviewer F11's seed).
    #[must_use]
    pub(crate) fn fresh(hull: ObjId, author: ObjId, next_id: u32) -> Orders {
        Orders {
            schema: ORDERS_SCHEMA,
            hull,
            author,
            rev: 0,
            state: PlanState::Running,
            hold: None,
            cursor: None,
            next_id: next_id.max(1),
            inflight: None,
            orders: Vec::new(),
            log: Vec::new(),
        }
    }

    /// The first unfinished order's id (L16).
    #[must_use]
    pub fn first_unfinished(&self) -> Option<u32> {
        self.orders
            .iter()
            .find(|order| unfinished(order.state))
            .map(|order| order.id)
    }

    /// Recompute [`Orders::cursor`]; when it changes, clear `reached` on every order
    /// but the new cursor (a reach is the cursor's alone, L9).
    pub(crate) fn derive_cursor(&mut self) {
        let cursor = self.first_unfinished();
        if cursor != self.cursor {
            for order in &mut self.orders {
                if Some(order.id) != cursor {
                    order.reached = None;
                }
            }
        }
        self.cursor = cursor;
    }

    /// Append one log line: newlines flattened, clamped on a char boundary to
    /// [`MAX_ORDER_LOG_BYTES`], the oldest dropped past [`MAX_ORDER_LOG_LINES`] (the
    /// reconnect queue's rule).
    pub(crate) fn push_log(&mut self, utc: i64, ic: i64, order: u32, text: &str) {
        let text = clamp_line(text);
        while self.log.len() >= MAX_ORDER_LOG_LINES {
            self.log.remove(0);
        }
        self.log.push(LogLine {
            utc,
            ic,
            order,
            text,
        });
    }

    /// The order with id `id`.
    #[must_use]
    pub fn order(&self, id: u32) -> Option<&Order> {
        self.orders.iter().find(|order| order.id == id)
    }

    /// The order with id `id`, mutably.
    pub(crate) fn order_mut(&mut self, id: u32) -> Option<&mut Order> {
        self.orders.iter_mut().find(|order| order.id == id)
    }

    /// The queue place (0-based) of order `id`.
    #[must_use]
    pub fn position(&self, id: u32) -> Option<usize> {
        self.orders.iter().position(|order| order.id == id)
    }

    /// Drop a hold note that names `id`; a stopped plan returns to `Running`
    /// (`del`, `skip`, and the self-heal of a landed step).
    pub(crate) fn clear_hold_naming(&mut self, id: u32) {
        if self.hold.as_ref().is_some_and(|note| note.order == id) {
            self.hold = None;
            if self.state == PlanState::Held {
                self.state = PlanState::Running;
            }
        }
    }

    /// **A stopped hold at order `id`** (the stopped shapes): plan `Held`, a
    /// `retry: false` note; the order `Held` unless it is `Inflight` (a hold *over*
    /// the inflight step keeps it and its record, L24).
    pub(crate) fn stop_at(&mut self, id: u32, reason: &str, since: i64) {
        if let Some(order) = self.order_mut(id)
            && order.state != OrderState::Inflight
        {
            order.state = OrderState::Held;
        }
        self.state = PlanState::Held;
        self.hold = Some(HoldNote {
            order: id,
            reason: reason.to_owned(),
            since,
            retry: false,
        });
    }

    /// **A waiting note at order `id`** (the waiting shapes): plan `Running`, a
    /// `retry: true` note; the order keeps its state (`Pending`, or `Inflight`).
    /// `true` when the note is new or its reason changed (it is then logged and
    /// notified once, L10).
    pub(crate) fn wait_at(&mut self, id: u32, reason: &str, since: i64) -> bool {
        let same = self
            .hold
            .as_ref()
            .is_some_and(|note| note.retry && note.order == id && note.reason == reason);
        if same {
            return false;
        }
        self.state = PlanState::Running;
        self.hold = Some(HoldNote {
            order: id,
            reason: reason.to_owned(),
            since,
            retry: true,
        });
        true
    }

    /// **The document's invariants** (the IC11 brief's item 2), the first broken one
    /// named. `Inflight.refs` is not constrained: the executor pre-checks with every
    /// ref `None` before its closed call and fills them after (reviewer F2).
    ///
    /// # Errors
    ///
    /// A message naming the first rule the document breaks.
    #[allow(clippy::too_many_lines)] // one ordered rule list
    pub fn check(&self) -> Result<(), String> {
        if self.schema != ORDERS_SCHEMA {
            return Err(format!("a sailing plan is schema {ORDERS_SCHEMA}"));
        }
        if self.orders.len() > MAX_ORDERS {
            return Err(format!("a sailing plan holds at most {MAX_ORDERS} orders"));
        }
        let mut ids: BTreeSet<u32> = BTreeSet::new();
        for order in &self.orders {
            if !ids.insert(order.id) {
                return Err(format!("order {} appears twice", order.id));
            }
            if order.id >= self.next_id {
                return Err(format!("order {} is not below the next id", order.id));
            }
            if order.attempt == 0 {
                return Err(format!("order {}'s attempt counts from 1", order.id));
            }
            if order.refusals > MAX_ORDER_REFUSALS {
                return Err(format!(
                    "order {} counts at most {MAX_ORDER_REFUSALS} refusals",
                    order.id
                ));
            }
            check_what(order.id, &order.what)?;
            let depart_open = matches!(order.what, What::Depart { .. })
                && !matches!(order.state, OrderState::Done | OrderState::Skipped);
            if order.reached.is_some() && !depart_open {
                return Err(format!(
                    "order {} carries a reach, which only an open depart does",
                    order.id
                ));
            }
        }
        // The one step in flight.
        let inflight_orders: Vec<&Order> = self
            .orders
            .iter()
            .filter(|order| order.state == OrderState::Inflight)
            .collect();
        if inflight_orders.len() > 1 {
            return Err("at most one order is in flight".to_owned());
        }
        match (inflight_orders.first(), &self.inflight) {
            (None, None) => {}
            (Some(order), Some(inflight)) => {
                if inflight.order != order.id || inflight.attempt != order.attempt {
                    return Err("the step in flight names its order and attempt".to_owned());
                }
                if inflight.restaged > 2 {
                    return Err("a step in flight is re-staged at most twice".to_owned());
                }
                if self.cursor != Some(inflight.order) {
                    return Err("the step in flight is the cursor".to_owned());
                }
            }
            (Some(_), None) | (None, Some(_)) => {
                return Err(
                    "an order is in flight exactly when the plan records its step".to_owned(),
                );
            }
        }
        if self.cursor != self.first_unfinished() {
            return Err("the cursor is the first unfinished order".to_owned());
        }
        // The hold shapes (reviewer F2).
        match (&self.hold, self.state) {
            (None, PlanState::Held) => {
                return Err("a held plan carries its hold".to_owned());
            }
            (Some(_), PlanState::Paused) => {
                return Err("a paused plan carries no hold".to_owned());
            }
            (Some(note), state) => {
                if Some(note.order) != self.cursor {
                    return Err("a hold names the cursor order".to_owned());
                }
                let named = self
                    .order(note.order)
                    .map(|order| order.state)
                    .ok_or_else(|| "a hold names an order of the plan".to_owned())?;
                if note.retry {
                    if state != PlanState::Running {
                        return Err("a waiting note stands on a running plan".to_owned());
                    }
                    if !matches!(named, OrderState::Pending | OrderState::Inflight) {
                        return Err("a waiting note names a pending or inflight order".to_owned());
                    }
                } else {
                    if state != PlanState::Held {
                        return Err("a stopped hold stands on a held plan".to_owned());
                    }
                    if !matches!(named, OrderState::Held | OrderState::Inflight) {
                        return Err("a stopped hold names a held or inflight order".to_owned());
                    }
                }
            }
            (None, PlanState::Running | PlanState::Paused) => {}
        }
        // An order is `Held` only under the stopped hold that names it.
        for order in &self.orders {
            if order.state == OrderState::Held
                && !self
                    .hold
                    .as_ref()
                    .is_some_and(|note| !note.retry && note.order == order.id)
            {
                return Err(format!(
                    "order {} is held with no stopped hold naming it",
                    order.id
                ));
            }
        }
        if self.log.len() > MAX_ORDER_LOG_LINES {
            return Err(format!(
                "a sailing plan keeps at most {MAX_ORDER_LOG_LINES} log lines"
            ));
        }
        for line in &self.log {
            if line.text.len() > MAX_ORDER_LOG_BYTES {
                return Err(format!("a log line is at most {MAX_ORDER_LOG_BYTES} bytes"));
            }
            if line.text.contains(['\n', '\r']) {
                return Err("a log line is one row".to_owned());
            }
        }
        Ok(())
    }
}

/// One order's vocabulary rules.
fn check_what(id: u32, what: &What) -> Result<(), String> {
    let units_ok = |qty: &Qty| match qty {
        Qty::Units(n) => (1..=mush_market::MAX_QUANTITY).contains(n),
        Qty::All | Qty::Fill => true,
    };
    let price_ok = |price: &Option<u64>| price.is_none_or(|p| (1..=MAX_PRICE_MAJOR).contains(&p));
    match what {
        What::Buy { qty, max_major, .. } => {
            if !units_ok(qty) || matches!(qty, Qty::All) {
                return Err(format!("order {id} buys a count or fill"));
            }
            if !price_ok(max_major) {
                return Err(format!("order {id}'s price is 1 to {MAX_PRICE_MAJOR}"));
            }
        }
        What::Sell { qty, min_major, .. } => {
            if !units_ok(qty) || matches!(qty, Qty::Fill) {
                return Err(format!("order {id} sells a count or all"));
            }
            if !price_ok(min_major) {
                return Err(format!("order {id}'s price is 1 to {MAX_PRICE_MAJOR}"));
            }
        }
        What::Depart {
            not_before,
            not_after_s,
            on_fail,
            crew,
            ..
        } => {
            if *on_fail == OnShort::Partial {
                return Err(format!("order {id}, a depart, skips or holds"));
            }
            if not_after_s.is_some_and(|window| window > MAX_WINDOW_SECS) {
                return Err(format!("order {id}'s window is at most a day"));
            }
            if let NotBefore::UtcDaily(hhmm) = not_before
                && hhmm_minutes(hhmm).is_none()
            {
                return Err(format!("order {id}'s daily time reads HH:MM"));
            }
            if crew.len() > MAX_DEPART_CREW {
                return Err(format!(
                    "order {id} lists at most {MAX_DEPART_CREW} passengers"
                ));
            }
        }
        What::Stop => {}
    }
    Ok(())
}

/// Flatten newlines and cut to [`MAX_ORDER_LOG_BYTES`] on a char boundary.
pub(crate) fn clamp_line(text: &str) -> String {
    let mut flat: String = text
        .chars()
        .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
        .collect();
    if flat.len() > MAX_ORDER_LOG_BYTES {
        let mut cut = MAX_ORDER_LOG_BYTES;
        while cut > 0 && !flat.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        flat.truncate(cut);
    }
    flat
}

/// **The hull's sailing plan, as the store holds it.**
#[must_use]
pub fn orders_state(w: &World, hull: ObjId) -> OrdersState<'_> {
    w.module_state::<TacsimStore>()
        .map_or(OrdersState::Absent, |store| store.orders_state(hull))
}

// ---- the texts (the planner's wording, owner item O1) ------------------------------

/// `+ship/order`'s grammar.
pub const ORDER_USAGE: &str = "Usage: +ship/order sell <good> <qty|all> [min=<cr>] [short=partial|skip|hold] | buy <good> <qty|fill> [max=<cr>] [short=partial|skip|hold] | depart <port> [at=now|HH:MM|daily HH:MM|+Nh] [until=+Nh] | stop | del <n> | move <n>=<pos> | clear | pause | resume | skip <n>";
/// Anything the vocabulary does not take (lead note 2g).
pub const NOT_ORDERABLE: &str = "The sailing plan takes sell, buy, depart and stop. Boarding, leaving, enrolling and buying out are done in person.";
/// Away from the hull and its port (Q-18-28): `{name}`.
pub const ORDERS_WHERE: &str =
    "You file the {name}'s sailing plan aboard her, or in the port where she is docked.";
/// An unreadable plan: `{name}`.
pub const ORDERS_DARK: &str =
    "The {name}'s sailing plan cannot be read right now. Ask staff to look at it.";
/// No plan (or another tenant's): `{name}`.
pub const NO_PLAN: &str = "No sailing plan is filed for the {name}.";
/// Sixteen orders, none finished.
pub const ORDERS_FULL: &str = "The sailing plan holds 16 orders; strike one first.";
/// The step in flight: `{n}`.
pub const ORDER_INFLIGHT: &str =
    "Order {n} is being carried out at the counter; it cannot be changed now.";
/// A move ahead of the step in flight: `{n}`, `{m}`.
pub const AHEAD_OF_INFLIGHT: &str =
    "Order {n} cannot go ahead of order {m}, which is being carried out.";
/// No such order: `{n}`.
pub const NO_SUCH_ORDER: &str = "The sailing plan has no order {n}.";
/// An unknown good: `{list}`.
pub const UNKNOWN_GOOD: &str = "No good goes by that name; the goods are {list}.";
/// `fill` on a good that is not a consumable.
pub const FILL_ONLY_CONSUMABLE: &str =
    "fill buys reaction mass or voyage provisions, sized for the next departure.";
/// An unknown port: `{list}`.
pub const UNKNOWN_PORT: &str = "No port by that name is on the lane; it serves {list}.";
/// A filed order: `{n}`, `{summary}`, `{rev}`.
pub const ORDER_FILED: &str = "Order {n} filed: {summary}. The sailing plan is at revision {rev}.";
/// The new order is the cursor and due now (doc 18 §2 step 10): `{n}`.
pub const ORDER_FIRES_NEXT: &str = "Order {n} fires on the next tick.";
/// A depart precedes the new order.
pub const ORDER_AFTER_DEPART: &str = "Orders after a departure run at the next port.";
/// `del`: `{n}`.
pub const DELETED: &str = "Order {n} is struck from the sailing plan.";
/// `move`: `{n}`, `{pos}`.
pub const MOVED: &str = "Order {n} moves to place {pos}.";
/// `skip`: `{n}`.
pub const SKIPPED: &str = "Order {n} will be passed over.";
/// `skip` of an order already done or passed over (nothing written): `{n}`.
pub const ORDER_FINISHED: &str =
    "Order {n} has already run or been passed over; there is nothing to skip.";
/// `clear`.
pub const CLEARED: &str = "The sailing plan is cleared.";
/// `clear` with an inflight order left: `{n}`.
pub const CLEARED_INFLIGHT: &str = " Order {n}, being carried out, stays.";
/// `pause`.
pub const PAUSED: &str = "The sailing plan is paused; nothing more runs until +ship/order resume.";
/// `resume`: `{n}`.
pub const RESUMED: &str = "The sailing plan runs again from order {n}.";
/// `resume` with nothing left.
pub const RESUMED_EMPTY: &str = "The sailing plan runs again; it has nothing left to do.";
/// `resume` on a running plan.
pub const NOT_HELD: &str = "The sailing plan is running; there is nothing to resume.";

// ---- reads the verbs and the executor share ----------------------------------------

/// **The seed of a fresh document's order ids** (reviewer F11): the replaced
/// document's `next_id` when one reads, else one past the highest order id on the
/// hull's last 64 tickets, else 1 — so a fresh plan never reuses an id whose burn key
/// may sit inside the idempotency window.
#[must_use]
pub(crate) fn seed_next_id(ctx: &Ctx<'_>, hull: ObjId) -> u32 {
    if let OrdersState::Present(old) = orders_state(ctx.w, hull) {
        return old.next_id.max(1);
    }
    let last_seq = match ctx
        .w
        .module_state::<TacsimStore>()
        .map_or(VoyageState::Absent, |store| store.voyage_state(hull))
    {
        VoyageState::Present(voyage) => voyage.last_seq,
        VoyageState::Absent | VoyageState::Unreadable => 0,
    };
    let mut highest: Option<u32> = None;
    for seq in last_seq.saturating_sub(63)..=last_seq {
        if seq == 0 {
            continue;
        }
        if let Ok(Some(ticket)) = ticket_at(ctx, TicketId { hull, seq })
            && let Some(order) = ticket.order
        {
            highest = Some(highest.map_or(order, |h| h.max(order)));
        }
    }
    highest.and_then(|h| h.checked_add(1)).unwrap_or(1)
}

/// `{qty} {unit}` for a count of `good`.
fn units(goods: &GoodsTable, good: &CanonicalId, n: u64) -> String {
    format!("{n} {}", unit_of(goods, good))
}

/// A price in major units as the economy renders it (`250.00cr`).
fn major_money(w: &World, major: u64) -> String {
    match minor_of(major, credit_scale(w)) {
        Some(minor) => money(w, mush_economy::Amount::from_minor(minor)),
        None => format!("{major}cr"),
    }
}

/// `, if short skip` / `, if short hold`; the default `partial` is not printed.
fn short_clause(on_short: OnShort) -> &'static str {
    match on_short {
        OnShort::Partial => "",
        OnShort::Skip => ", if short skip",
        OnShort::Hold => ", if short hold",
    }
}

/// **An order's one-line summary** (`+ship/orders` and the edit replies).
#[must_use]
pub(crate) fn order_summary(w: &World, goods: &GoodsTable, what: &What) -> String {
    match what {
        What::Sell {
            good,
            qty,
            min_major,
            on_short,
        } => {
            let amount = match qty {
                Qty::All => "all".to_owned(),
                Qty::Fill => "fill".to_owned(),
                Qty::Units(n) => units(goods, good, *n),
            };
            let price = min_major.map_or_else(String::new, |p| {
                format!(", not under {}/{}", major_money(w, p), unit_of(goods, good))
            });
            format!(
                "sell {}, {amount}{price}{}",
                label_of(goods, good),
                short_clause(*on_short)
            )
        }
        What::Buy {
            good,
            qty,
            max_major,
            on_short,
        } => {
            let amount = match qty {
                Qty::All => "all".to_owned(),
                Qty::Fill => "fill".to_owned(),
                Qty::Units(n) => units(goods, good, *n),
            };
            let price = max_major.map_or_else(String::new, |p| {
                format!(", not over {}/{}", major_money(w, p), unit_of(goods, good))
            });
            format!(
                "buy {}, {amount}{price}{}",
                label_of(goods, good),
                short_clause(*on_short)
            )
        }
        What::Depart {
            to,
            not_before,
            not_after_s,
            ..
        } => {
            let when = match not_before {
                NotBefore::Now => " now".to_owned(),
                NotBefore::Utc(at) => {
                    format!(" at {} UTC {}", utc_hhmm(*at), utc_day_word(w.now(), *at))
                }
                NotBefore::UtcDaily(hhmm) => format!(" daily at {hhmm} UTC"),
            };
            let until = not_after_s.map_or_else(String::new, |window| {
                format!(
                    ", until {} after",
                    crate::port::voyage::real_span(i64::from(window))
                )
            });
            format!("depart for {}{when}{until}", venue_name(w, to))
        }
        What::Stop => "stop".to_owned(),
    }
}

/// The layer's goods table.
fn goods_of(w: &World) -> Option<&GoodsTable> {
    w.module_state::<TacsimStore>().and_then(TacsimStore::goods)
}

/// Say one line to the actor under the `tacsim` tag.
fn say(ctx: &mut Ctx<'_>, line: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "tacsim", line.into());
}

/// The port (by name) each order of `orders` will run at, from the cursor on —
/// `None` for finished orders and where it is not known (lead note 2j(i)): the walk
/// starts at the hull's berth (or, under way, the passage's destination) and switches
/// to each open depart's destination after it.
fn ports_of_orders(w: &World, orders: &Orders) -> Vec<Option<String>> {
    let hull = orders.hull;
    let mut here: Option<String> = match (passage_of(w, hull), berth_of(w, hull)) {
        (Some(Passage::UnderWay { port, .. }), _) => Some(port),
        (_, Berth::Docked { def, .. }) => Some(port_name(def)),
        (_, Berth::UnderWay | Berth::Elsewhere) => None,
    };
    let start = orders
        .cursor
        .and_then(|id| orders.position(id))
        .unwrap_or(orders.orders.len());
    orders
        .orders
        .iter()
        .enumerate()
        .map(|(index, order)| {
            if index < start || !unfinished(order.state) {
                return None;
            }
            let at = here.clone();
            if let What::Depart { to, .. } = &order.what {
                here = Some(venue_name(w, to));
            }
            at
        })
        .collect()
}

/// The venue the hull is docked at, if it is.
fn docked_venue(w: &World, hull: ObjId) -> Option<CanonicalId> {
    match berth_of(w, hull) {
        Berth::Docked { def, .. } => Some(def.venue.clone()),
        Berth::UnderWay | Berth::Elsewhere => None,
    }
}

/// **When a depart at the cursor is due, as the renders read it**: its stored reach
/// when it names the venue the hull is docked at, else what it would resolve to if
/// reached now (the `+ship/orders` rule).
#[must_use]
pub(crate) fn render_reach(w: &World, hull: ObjId, order: &Order) -> i64 {
    let venue = docked_venue(w, hull);
    order
        .reached
        .as_ref()
        .filter(|reached| Some(&reached.venue) == venue.as_ref())
        .map_or(w.now(), |reached| reached.utc)
}

/// **The due instant of a depart at the cursor** (`None` for any other kind).
#[must_use]
pub(crate) fn depart_due(w: &World, hull: ObjId, order: &Order) -> Option<i64> {
    match &order.what {
        What::Depart {
            not_before,
            not_after_s,
            ..
        } => Some(resolve_due(
            not_before,
            *not_after_s,
            render_reach(w, hull, order),
        )),
        What::Buy { .. } | What::Sell { .. } | What::Stop => None,
    }
}

/// **The departure hour the status renders name** (doc 18 §2 step 15, §4.4: "the
/// real-time … departure clock (UTC) appear only in these renders, in `+ship` and in
/// the `look` status line"): `Some(due)` when the hull is docked with no passage, its
/// plan reads and runs under its author's live charter of this hull (L14), and its
/// cursor order is a depart due in the future.
#[must_use]
pub(crate) fn departs_at(w: &World, hull: ObjId) -> Option<i64> {
    if passage_of(w, hull).is_some() || !matches!(berth_of(w, hull), Berth::Docked { .. }) {
        return None;
    }
    let OrdersState::Present(orders) = orders_state(w, hull) else {
        return None;
    };
    if orders.state != PlanState::Running {
        return None;
    }
    // Only a plan its author still runs: a lapsed tenant's plan (left for IC13 to
    // prune) is never offered by the arm, so its hour is no departure hour.
    if !live_charter(w, orders.author).is_some_and(|charter| charter.hull == hull) {
        return None;
    }
    let order = orders.order(orders.cursor?)?;
    if order.state != OrderState::Pending {
        return None;
    }
    depart_due(w, hull, order).filter(|due| *due > w.now())
}

// ---- +ship/orders --------------------------------------------------------------------

/// **`+ship/orders`** (doc 18 §4.4 "Verbs"): the plan, each order with its state and
/// where it will run, the next one due (UTC, IC and real time), any hold, and the last
/// [`ORDERS_SHOWN_LOG`] log lines signed by the document's author (Q-18-24, L27).
/// Readable anywhere with a live charter; writes nothing.
///
/// # Errors
///
/// [`NO_PLAN`], [`ORDERS_DARK`], or the no-charter refusal.
pub(crate) fn orders_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Some(charter) = live_charter(ctx.w, actor) else {
        return Err(crate::port::ship::no_charter(ctx.w, actor));
    };
    let hull = charter.hull;
    let lines = {
        let w: &World = ctx.w;
        let name = name_of(w, hull);
        let orders = match orders_state(w, hull) {
            OrdersState::Unreadable => {
                return Err(CmdError::msg(ORDERS_DARK.replace("{name}", &name)));
            }
            OrdersState::Present(orders) if orders.author == actor => orders,
            OrdersState::Present(_) | OrdersState::Absent => {
                return Err(CmdError::msg(NO_PLAN.replace("{name}", &name)));
            }
        };
        let Some(goods) = goods_of(w) else {
            return Err(CmdError::msg(ORDERS_DARK.replace("{name}", &name)));
        };
        render_orders(w, goods, orders)
    };
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// The `+ship/orders` lines.
fn render_orders(w: &World, goods: &GoodsTable, orders: &Orders) -> Vec<String> {
    let hull = orders.hull;
    let name = name_of(w, hull);
    let mark = hull_view(w, hull).map_or_else(String::new, |view| view.row.mark.clone());
    let state = match orders.state {
        PlanState::Running => "running",
        PlanState::Paused => "paused",
        PlanState::Held => "held",
    };
    let mut lines = vec![format!(
        "The {name}'s sailing plan ({mark}), revision {}: {state}.",
        orders.rev
    )];
    let present = docked_venue(w, hull).map(|venue| venue_name(w, &venue));
    let under_way = matches!(berth_of(w, hull), Berth::UnderWay);
    let ports = ports_of_orders(w, orders);
    for (index, order) in orders.orders.iter().enumerate() {
        let word = match order.state {
            OrderState::Pending if orders.cursor == Some(order.id) => "next",
            OrderState::Pending => "pending",
            OrderState::Inflight => "at the counter",
            OrderState::Done => "done",
            OrderState::Skipped => "skipped",
            OrderState::Held => "held",
        };
        let at = ports
            .get(index)
            .cloned()
            .flatten()
            .filter(|port| under_way || present.as_ref() != Some(port))
            .map_or_else(String::new, |port| format!(", at {port}"));
        lines.push(format!(
            "  {}. {} — {word}{at}",
            order.id,
            order_summary(w, goods, &order.what)
        ));
    }
    lines.push(format!("Next due: {}", next_due(w, orders, &ports)));
    if let Some(note) = &orders.hold {
        let head = if note.retry { "Waiting" } else { "Held" };
        lines.push(format!("{head}: {}", note.reason));
    }
    let pilot = name_of(w, orders.author);
    if orders.log.is_empty() {
        lines.push("Log: nothing yet.".to_owned());
    } else {
        lines.push(format!("Log, each line signed {pilot}:"));
        let skip = orders.log.len().saturating_sub(ORDERS_SHOWN_LOG);
        for line in orders.log.iter().skip(skip) {
            lines.push(format!(
                "  {} UTC, order {}, signed {pilot}: {}",
                utc_hhmm(line.utc),
                line.order,
                line.text
            ));
        }
    }
    lines
}

/// The `Next due:` clause.
fn next_due(w: &World, orders: &Orders, ports: &[Option<String>]) -> String {
    let Some(id) = orders.cursor else {
        return "nothing; the plan is done.".to_owned();
    };
    match orders.state {
        PlanState::Paused => {
            return format!("nothing while the plan is paused (order {id} is next).");
        }
        PlanState::Held => return format!("nothing while order {id} is held."),
        PlanState::Running => {}
    }
    let Some(order) = orders.order(id) else {
        return "nothing; the plan is done.".to_owned();
    };
    if order.state == OrderState::Inflight {
        return format!("order {id} is at the counter.");
    }
    let hull = orders.hull;
    match (&order.what, berth_of(w, hull)) {
        (What::Depart { to, .. }, _) => {
            let due = depart_due(w, hull, order).unwrap_or_else(|| w.now());
            format!(
                "order {id}, departure for {}, {}",
                venue_name(w, to),
                render_due(w, due)
            )
        }
        (What::Stop, _) | (What::Buy { .. } | What::Sell { .. }, Berth::Docked { .. }) => {
            format!("order {id} on the next tick.")
        }
        (What::Buy { .. } | What::Sell { .. }, Berth::UnderWay | Berth::Elsewhere) => {
            let port = orders
                .position(id)
                .and_then(|index| ports.get(index).cloned().flatten());
            let name = name_of(w, hull);
            match port {
                Some(port) => format!("order {id} when the {name} docks at {port}."),
                None => format!("order {id} when the {name} docks."),
            }
        }
    }
}

// ---- +ship/order -------------------------------------------------------------------

/// What one `+ship/order` line asks.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Edit {
    File(What),
    Del(u32),
    Move(u32, usize),
    Clear,
    Pause,
    Resume,
    Skip(u32),
}

/// A good by id (case-insensitive) or by a case-insensitive prefix of exactly one
/// label.
fn match_good(goods: &GoodsTable, word: &str) -> Option<CanonicalId> {
    let lower = word.to_lowercase();
    if let Some(good) = goods
        .goods
        .iter()
        .find(|good| good.id.as_str().eq_ignore_ascii_case(&lower))
    {
        return Some(good.id.clone());
    }
    let mut prefixed = goods
        .goods
        .iter()
        .filter(|good| good.label.to_lowercase().starts_with(&lower));
    let only = prefixed.next()?;
    prefixed.next().is_none().then(|| only.id.clone())
}

/// The lane port `word` names: its venue word, or a case-insensitive prefix of its
/// name; `Err` with every lane port's name.
fn match_port(w: &World, word: &str) -> Result<CanonicalId, String> {
    let ports: Vec<&PortDef> = lane_ports(w).into_iter().map(|(def, _)| def).collect();
    let lower = word.trim().to_lowercase();
    let found = (!lower.is_empty())
        .then(|| {
            ports
                .iter()
                .find(|def| def.venue.as_str().eq_ignore_ascii_case(&lower))
                .or_else(|| {
                    ports
                        .iter()
                        .find(|def| port_name(def).to_lowercase().starts_with(&lower))
                })
        })
        .flatten();
    match found {
        Some(def) => Ok(def.venue.clone()),
        None => {
            let names: Vec<String> = ports.iter().map(|def| port_name(def)).collect();
            Err(UNKNOWN_PORT.replace("{list}", &names.join(", ")))
        }
    }
}

/// A strict decimal order id.
fn order_id(word: Option<&str>) -> Result<u32, CmdError> {
    word.and_then(strict_decimal_u32)
        .ok_or_else(|| CmdError::msg(ORDER_USAGE))
}

/// `short=`'s value.
fn on_short_of(value: &str) -> Option<OnShort> {
    match value.to_ascii_lowercase().as_str() {
        "partial" => Some(OnShort::Partial),
        "skip" => Some(OnShort::Skip),
        "hold" => Some(OnShort::Hold),
        _ => None,
    }
}

/// `sell …` and `buy …`.
fn parse_trade(goods: &GoodsTable, sell: bool, args: &[&str]) -> Result<What, CmdError> {
    let usage = || CmdError::msg(ORDER_USAGE);
    let (Some(good_word), Some(qty_word)) = (args.first(), args.get(1)) else {
        return Err(usage());
    };
    let Some(good) = match_good(goods, good_word) else {
        let ids: Vec<&str> = goods.goods.iter().map(|good| good.id.as_str()).collect();
        return Err(CmdError::msg(
            UNKNOWN_GOOD.replace("{list}", &ids.join(", ")),
        ));
    };
    let qty = match qty_word.to_ascii_lowercase().as_str() {
        "all" if sell => Qty::All,
        "fill" if !sell => {
            if goods.good(&good).is_none_or(|row| row.consumable.is_none()) {
                return Err(CmdError::msg(FILL_ONLY_CONSUMABLE));
            }
            Qty::Fill
        }
        word => {
            let strict = word.bytes().all(|b| b.is_ascii_digit()) && !word.starts_with('0');
            let n = word
                .parse::<u64>()
                .ok()
                .filter(|n| strict && (1..=mush_market::MAX_QUANTITY).contains(n))
                .ok_or_else(usage)?;
            Qty::Units(n)
        }
    };
    let mut price: Option<u64> = None;
    let mut on_short: Option<OnShort> = None;
    let price_key = if sell { "min" } else { "max" };
    for option in args.iter().skip(2) {
        let (key, value) = option.split_once('=').ok_or_else(usage)?;
        let key = key.to_ascii_lowercase();
        if key == price_key && price.is_none() {
            let p = strict_decimal_u32(value)
                .map(u64::from)
                .filter(|p| (1..=MAX_PRICE_MAJOR).contains(p))
                .ok_or_else(usage)?;
            price = Some(p);
        } else if key == "short" && on_short.is_none() {
            on_short = Some(on_short_of(value).ok_or_else(usage)?);
        } else {
            return Err(usage());
        }
    }
    let on_short = on_short.unwrap_or(OnShort::Partial);
    Ok(if sell {
        What::Sell {
            good,
            qty,
            min_major: price,
            on_short,
        }
    } else {
        What::Buy {
            good,
            qty,
            max_major: price,
            on_short,
        }
    })
}

/// `depart <port> [at=…] [until=…] [via=…]`; `crew` is filled by the caller.
fn parse_depart(w: &World, args: &[&str]) -> Result<What, CmdError> {
    let mut words: Vec<&str> = Vec::new();
    let mut at: Option<NotBefore> = None;
    let mut until: Option<u32> = None;
    let mut tokens = args.iter().copied().peekable();
    while let Some(token) = tokens.next() {
        let lower = token.to_ascii_lowercase();
        if lower.starts_with("via=") {
            return Err(CmdError::msg(VIA_NOT_YET));
        }
        if let Some(value) = lower.strip_prefix("at=") {
            if at.is_some() {
                return Err(CmdError::msg(ORDER_USAGE));
            }
            let (parsed, consumed) =
                parse_at(value, tokens.peek().copied(), w.now()).map_err(CmdError::msg)?;
            if consumed {
                let _ = tokens.next();
            }
            at = Some(parsed);
            continue;
        }
        if let Some(value) = lower.strip_prefix("until=") {
            if until.is_some() {
                return Err(CmdError::msg(ORDER_USAGE));
            }
            until = Some(parse_until(value).map_err(CmdError::msg)?);
            continue;
        }
        if token.contains('=') {
            return Err(CmdError::msg(ORDER_USAGE));
        }
        words.push(token);
    }
    if words.is_empty() {
        return Err(CmdError::msg(ORDER_USAGE));
    }
    let to = match_port(w, &words.join(" ")).map_err(CmdError::msg)?;
    Ok(What::Depart {
        to,
        not_before: at.unwrap_or(NotBefore::Now),
        not_after_s: until,
        on_fail: OnShort::Hold,
        crew: Vec::new(),
    })
}

/// Parse one `+ship/order` line (grammar only; the document's rules come after).
fn parse_edit(w: &World, goods: &GoodsTable, raw: &str) -> Result<Edit, CmdError> {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let Some(first) = words.first() else {
        return Err(CmdError::msg(ORDER_USAGE));
    };
    let rest: Vec<&str> = words.iter().skip(1).copied().collect();
    let none_more = |edit: Edit| {
        if rest.is_empty() {
            Ok(edit)
        } else {
            Err(CmdError::msg(ORDER_USAGE))
        }
    };
    match first.to_ascii_lowercase().as_str() {
        "sell" => parse_trade(goods, true, &rest).map(Edit::File),
        "buy" => parse_trade(goods, false, &rest).map(Edit::File),
        "depart" => parse_depart(w, &rest).map(Edit::File),
        "stop" => none_more(Edit::File(What::Stop)),
        "clear" => none_more(Edit::Clear),
        "pause" => none_more(Edit::Pause),
        "resume" => none_more(Edit::Resume),
        "del" | "skip" => {
            if rest.len() != 1 {
                return Err(CmdError::msg(ORDER_USAGE));
            }
            let n = order_id(rest.first().copied())?;
            Ok(if first.eq_ignore_ascii_case("del") {
                Edit::Del(n)
            } else {
                Edit::Skip(n)
            })
        }
        "move" => {
            if rest.len() != 1 {
                return Err(CmdError::msg(ORDER_USAGE));
            }
            let (n, pos) = rest
                .first()
                .and_then(|arg| arg.split_once('='))
                .ok_or_else(|| CmdError::msg(ORDER_USAGE))?;
            let n = order_id(Some(n))?;
            let pos = strict_decimal_u32(pos)
                .and_then(|pos| usize::try_from(pos).ok())
                .filter(|pos| *pos >= 1)
                .ok_or_else(|| CmdError::msg(ORDER_USAGE))?;
            Ok(Edit::Move(n, pos))
        }
        _ => Err(CmdError::msg(NOT_ORDERABLE)),
    }
}

/// `{n}`-filled [`ORDER_INFLIGHT`].
fn inflight_refusal(n: u32) -> CmdError {
    CmdError::msg(ORDER_INFLIGHT.replace("{n}", &n.to_string()))
}

/// **`+ship/order <sub-verb> …`** (doc 18 §4.4 "Verbs"; the IC11 brief's item 3):
/// every gate in the brief's order — the charter, the document, where, the grammar and
/// the document's rules — then ONE `put_orders` with `rev` bumped and the reply. Every
/// refusal writes nothing; the verb submits nothing.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line.
#[allow(clippy::too_many_lines)] // one ordered gate list, then one write
pub(crate) fn order_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let raw = ctx.args.raw().to_owned();
    // 2. The charter.
    let Some(charter) = live_charter(ctx.w, actor) else {
        return Err(crate::port::ship::no_charter(ctx.w, actor));
    };
    let hull = charter.hull;
    let name = name_of(ctx.w, hull);
    let dark = || CmdError::msg(ORDERS_DARK.replace("{name}", &name));
    // 3. The document: an unreadable one fails closed; another author's reads as
    // absent (a new tenant files a fresh plan, L14) unless its step is in flight.
    let existing: Option<Orders> = match orders_state(ctx.w, hull) {
        OrdersState::Unreadable => return Err(dark()),
        OrdersState::Present(orders) if orders.author == actor => Some(orders.clone()),
        OrdersState::Present(orders) => {
            if let Some(inflight) = &orders.inflight {
                return Err(inflight_refusal(inflight.order));
            }
            None
        }
        OrdersState::Absent => None,
    };
    // 4. Where (Q-18-28): aboard, or in a room of the port where she is docked.
    let here = ctx.w.get(actor).and_then(|o| o.location);
    let in_port = match berth_of(ctx.w, hull) {
        Berth::Docked { def, .. } => {
            here.is_some_and(|room| crate::port::market::port_rooms(ctx.w, def).contains(&room))
        }
        Berth::UnderWay | Berth::Elsewhere => false,
    };
    if !aboard(ctx.w, hull, actor) && !in_port {
        return Err(CmdError::msg(ORDERS_WHERE.replace("{name}", &name)));
    }
    // 5. The grammar.
    let Some(goods) = goods_of(ctx.w).cloned() else {
        return Err(dark());
    };
    let edit = parse_edit(ctx.w, &goods, &raw)?;
    let now = ctx.w.now();
    let mut orders = match (existing, &edit) {
        (Some(orders), _) => orders,
        (None, Edit::File(_)) => Orders::fresh(hull, actor, seed_next_id(ctx, hull)),
        (None, _) => return Err(CmdError::msg(NO_PLAN.replace("{name}", &name))),
    };
    let no_such = |n: u32| CmdError::msg(NO_SUCH_ORDER.replace("{n}", &n.to_string()));
    let inflight_id = orders.inflight.as_ref().map(|inflight| inflight.order);
    let mut replies: Vec<String> = Vec::new();
    match edit {
        Edit::File(what) => {
            let what = match what {
                What::Depart {
                    to,
                    not_before,
                    not_after_s,
                    on_fail,
                    ..
                } => {
                    // L11: the listed passengers are whoever is aboard at filing, the
                    // pilot aside.
                    let crew: Vec<ObjId> = hull_view(ctx.w, hull)
                        .and_then(|view| view.airlock())
                        .map(|airlock| souls_aboard(ctx.w, airlock))
                        .unwrap_or_default()
                        .into_iter()
                        .filter(|person| *person != actor)
                        .take(MAX_DEPART_CREW)
                        .collect();
                    What::Depart {
                        to,
                        not_before,
                        not_after_s,
                        on_fail,
                        crew,
                    }
                }
                other => other,
            };
            if orders.orders.len() >= MAX_ORDERS {
                // Make room: the oldest finished order goes first.
                let Some(index) = orders.orders.iter().position(|order| {
                    matches!(order.state, OrderState::Done | OrderState::Skipped)
                }) else {
                    return Err(CmdError::msg(ORDERS_FULL));
                };
                orders.orders.remove(index);
            }
            let id = orders.next_id;
            orders.next_id = id.checked_add(1).ok_or_else(dark)?;
            let depart_before = orders
                .orders
                .iter()
                .any(|order| unfinished(order.state) && matches!(order.what, What::Depart { .. }));
            let summary = order_summary(ctx.w, &goods, &what);
            let due_now = match &what {
                What::Buy { .. } | What::Sell { .. } => true,
                What::Depart { not_before, .. } => *not_before == NotBefore::Now,
                What::Stop => false,
            };
            orders.orders.push(Order {
                id,
                state: OrderState::Pending,
                attempt: 1,
                refusals: 0,
                retry_at: None,
                announced: false,
                reached: None,
                what,
            });
            orders.derive_cursor();
            let rev = orders.rev.checked_add(1).ok_or_else(dark)?;
            replies.push(
                ORDER_FILED
                    .replace("{n}", &id.to_string())
                    .replace("{summary}", &summary)
                    .replace("{rev}", &rev.to_string()),
            );
            let docked = matches!(berth_of(ctx.w, hull), Berth::Docked { .. });
            if orders.cursor == Some(id) && orders.state == PlanState::Running && docked && due_now
            {
                replies.push(ORDER_FIRES_NEXT.replace("{n}", &id.to_string()));
            }
            if depart_before {
                replies.push(ORDER_AFTER_DEPART.to_owned());
            }
        }
        Edit::Del(n) => {
            let index = orders.position(n).ok_or_else(|| no_such(n))?;
            if inflight_id == Some(n) {
                return Err(inflight_refusal(n));
            }
            orders.clear_hold_naming(n);
            orders.orders.remove(index);
            replies.push(DELETED.replace("{n}", &n.to_string()));
        }
        Edit::Skip(n) => {
            let order = orders.order(n).ok_or_else(|| no_such(n))?;
            if inflight_id == Some(n) {
                return Err(inflight_refusal(n));
            }
            if matches!(order.state, OrderState::Done | OrderState::Skipped) {
                // Already run or passed over: a true refusal, nothing written (the
                // brief scopes `skip` to a pending or held order).
                return Err(CmdError::msg(ORDER_FINISHED.replace("{n}", &n.to_string())));
            }
            orders.clear_hold_naming(n);
            if let Some(order) = orders.order_mut(n) {
                order.state = OrderState::Skipped;
                order.refusals = 0;
                order.reached = None;
            }
            replies.push(SKIPPED.replace("{n}", &n.to_string()));
        }
        Edit::Move(n, pos) => {
            let from = orders.position(n).ok_or_else(|| no_such(n))?;
            if inflight_id == Some(n) {
                return Err(inflight_refusal(n));
            }
            if pos > orders.orders.len() {
                return Err(CmdError::msg(ORDER_USAGE));
            }
            if let Some(m) = inflight_id
                && let Some(place) = orders.position(m)
                && pos <= place.saturating_add(1)
            {
                return Err(CmdError::msg(
                    AHEAD_OF_INFLIGHT
                        .replace("{n}", &n.to_string())
                        .replace("{m}", &m.to_string()),
                ));
            }
            let order = orders.orders.remove(from);
            let to = pos.saturating_sub(1).min(orders.orders.len());
            orders.orders.insert(to, order);
            // A move that changes the cursor under a hold re-queues the order the hold
            // named: the note is dropped and a held order is pending again.
            let cursor = orders.first_unfinished();
            if let Some(named) = orders.hold.as_ref().map(|note| note.order)
                && cursor != Some(named)
            {
                orders.clear_hold_naming(named);
                if let Some(order) = orders.order_mut(named)
                    && order.state == OrderState::Held
                {
                    order.state = OrderState::Pending;
                    order.refusals = 0;
                    order.retry_at = None;
                }
            }
            replies.push(
                MOVED
                    .replace("{n}", &n.to_string())
                    .replace("{pos}", &pos.to_string()),
            );
        }
        Edit::Clear => {
            orders.orders.retain(|order| Some(order.id) == inflight_id);
            orders.state = PlanState::Running;
            orders.hold = None;
            let mut line = CLEARED.to_owned();
            if let Some(n) = inflight_id {
                line.push_str(&CLEARED_INFLIGHT.replace("{n}", &n.to_string()));
            }
            replies.push(line);
        }
        Edit::Pause => {
            // A pause supersedes any hold: a held order is pending again under it
            // (resume re-runs it, as resume from a hold would), and a waiting note
            // re-arises by itself if its cause still stands.
            if let Some(named) = orders.hold.take().map(|note| note.order)
                && let Some(order) = orders.order_mut(named)
                && order.state == OrderState::Held
            {
                order.state = OrderState::Pending;
                order.refusals = 0;
                order.retry_at = None;
            }
            orders.state = PlanState::Paused;
            replies.push(PAUSED.to_owned());
        }
        Edit::Resume => {
            match orders.state {
                PlanState::Running => return Err(CmdError::msg(NOT_HELD)),
                PlanState::Paused => orders.state = PlanState::Running,
                PlanState::Held => {
                    let named = orders.hold.take().map(|note| note.order);
                    orders.state = PlanState::Running;
                    if let Some(named) = named {
                        if inflight_id == Some(named) {
                            // A stopped hold over the step in flight: the step stays and
                            // its re-stage clock is re-armed; nothing new is staged.
                            if let Some(inflight) = orders.inflight.as_mut() {
                                inflight.restaged = 0;
                                inflight.staged_utc = now;
                            }
                        } else if let Some(order) = orders.order_mut(named) {
                            order.state = OrderState::Pending;
                            order.retry_at = None;
                            order.refusals = 0;
                            // "resume sails now": a depart whose window passed.
                            if let What::Depart {
                                not_before,
                                not_after_s,
                                ..
                            } = &mut order.what
                                && not_after_s.is_some()
                            {
                                let due = resolve_due(not_before, *not_after_s, now);
                                let window = i64::from(not_after_s.unwrap_or(0));
                                let daily = matches!(not_before, NotBefore::UtcDaily(_));
                                if !daily && now > due.saturating_add(window) {
                                    *not_before = NotBefore::Now;
                                    *not_after_s = None;
                                }
                            }
                        }
                    }
                }
            }
            orders.derive_cursor();
            replies.push(orders.cursor.map_or_else(
                || RESUMED_EMPTY.to_owned(),
                |n| RESUMED.replace("{n}", &n.to_string()),
            ));
        }
    }
    // 6. The cursor, ONE checked write with `rev` bumped, and the reply.
    orders.derive_cursor();
    orders.rev = orders.rev.checked_add(1).ok_or_else(dark)?;
    if let Err(error) = put_orders(ctx, &orders) {
        tracing::error!(hull = hull.0, %error, "tacsim: +ship/order: the edited sailing plan does not check; nothing was written");
        return Err(dark());
    }
    for line in replies {
        say(ctx, line);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cid(text: &str) -> CanonicalId {
        CanonicalId::new(text).expect("id")
    }

    fn pending(id: u32, what: What) -> Order {
        Order {
            id,
            state: OrderState::Pending,
            attempt: 1,
            refusals: 0,
            retry_at: None,
            announced: false,
            reached: None,
            what,
        }
    }

    fn sell_metals() -> What {
        What::Sell {
            good: cid("metals"),
            qty: Qty::All,
            min_major: Some(250),
            on_short: OnShort::Partial,
        }
    }

    fn depart_ceres() -> What {
        What::Depart {
            to: cid("ceres"),
            not_before: NotBefore::UtcDaily("19:30".to_owned()),
            not_after_s: Some(7_200),
            on_fail: OnShort::Hold,
            crew: Vec::new(),
        }
    }

    /// A three-order plan: sell, buy fill, depart.
    fn plan() -> Orders {
        let mut orders = Orders::fresh(ObjId(5011), ObjId(42), 1);
        orders.orders = vec![
            pending(1, sell_metals()),
            pending(
                2,
                What::Buy {
                    good: cid("propellant"),
                    qty: Qty::Fill,
                    max_major: Some(60),
                    on_short: OnShort::Partial,
                },
            ),
            pending(3, depart_ceres()),
        ];
        orders.next_id = 4;
        orders.derive_cursor();
        orders
    }

    fn inflight(order: u32) -> Inflight {
        Inflight {
            order,
            attempt: 1,
            key: "market.lot.deliver.1f".to_owned(),
            refs: InflightRefs::default(),
            staged_utc: 1_000,
            issued_ic: 2_000,
            restaged: 0,
            want: 20,
            qty: 20,
            bounds: Vec::new(),
            line: None,
        }
    }

    #[test]
    fn ic11_orders_keys_are_strict() {
        assert_eq!(orders_key(ObjId(5011)), "tacsim:orders:5011");
        assert_eq!(orders_key_owner("tacsim:orders:5011"), Some(ObjId(5011)));
        assert_eq!(orders_key_owner("tacsim:orders:0"), Some(ObjId(0)));
        for bad in [
            "tacsim:orders:05011",
            "tacsim:orders:#5011",
            "tacsim:orders:",
            "tacsim:orders:-1",
            "tacsim:voyage:5011",
        ] {
            assert_eq!(orders_key_owner(bad), None, "{bad}");
        }
    }

    #[test]
    #[allow(clippy::too_many_lines)]
    fn ic11_the_orders_document_checks_its_bounds() {
        let good = plan();
        good.check().unwrap();
        let with = |edit: &dyn Fn(&mut Orders)| {
            let mut orders = good.clone();
            edit(&mut orders);
            orders
        };
        let cases: Vec<(&str, Orders)> = vec![
            ("schema 2", with(&|o| o.schema = 2)),
            (
                "a 17th order",
                with(&|o| {
                    for id in 4..=17 {
                        o.orders.push(pending(id, What::Stop));
                    }
                    o.next_id = 18;
                }),
            ),
            (
                "a duplicate id",
                with(&|o| o.orders.push(pending(3, What::Stop))),
            ),
            ("an id at next_id", with(&|o| o.next_id = 3)),
            (
                "attempt 0",
                with(&|o| {
                    if let Some(order) = o.order_mut(1) {
                        order.attempt = 0;
                    }
                }),
            ),
            (
                "refusals 4",
                with(&|o| {
                    if let Some(order) = o.order_mut(1) {
                        order.refusals = 4;
                    }
                }),
            ),
            (
                "a count of 0",
                with(&|o| {
                    o.orders.push(pending(
                        4,
                        What::Buy {
                            good: cid("metals"),
                            qty: Qty::Units(0),
                            max_major: None,
                            on_short: OnShort::Partial,
                        },
                    ));
                    o.next_id = 5;
                }),
            ),
            (
                "a price over the cap",
                with(&|o| {
                    if let Some(order) = o.order_mut(1) {
                        order.what = What::Sell {
                            good: cid("metals"),
                            qty: Qty::All,
                            min_major: Some(MAX_PRICE_MAJOR + 1),
                            on_short: OnShort::Partial,
                        };
                    }
                }),
            ),
            (
                "fill on a sell",
                with(&|o| {
                    if let Some(order) = o.order_mut(1) {
                        order.what = What::Sell {
                            good: cid("metals"),
                            qty: Qty::Fill,
                            min_major: None,
                            on_short: OnShort::Partial,
                        };
                    }
                }),
            ),
            (
                "all on a buy",
                with(&|o| {
                    if let Some(order) = o.order_mut(2) {
                        order.what = What::Buy {
                            good: cid("metals"),
                            qty: Qty::All,
                            max_major: None,
                            on_short: OnShort::Partial,
                        };
                    }
                }),
            ),
            (
                "a partial depart",
                with(&|o| {
                    if let Some(order) = o.order_mut(3)
                        && let What::Depart { on_fail, .. } = &mut order.what
                    {
                        *on_fail = OnShort::Partial;
                    }
                }),
            ),
            (
                "a window past a day",
                with(&|o| {
                    if let Some(order) = o.order_mut(3)
                        && let What::Depart { not_after_s, .. } = &mut order.what
                    {
                        *not_after_s = Some(MAX_WINDOW_SECS + 1);
                    }
                }),
            ),
            (
                "a bad daily time",
                with(&|o| {
                    if let Some(order) = o.order_mut(3)
                        && let What::Depart { not_before, .. } = &mut order.what
                    {
                        *not_before = NotBefore::UtcDaily("7:30".to_owned());
                    }
                }),
            ),
            (
                "sixteen passengers",
                with(&|o| {
                    if let Some(order) = o.order_mut(3)
                        && let What::Depart { crew, .. } = &mut order.what
                    {
                        *crew = (1..=16).map(ObjId).collect();
                    }
                }),
            ),
            (
                "a reach on a buy",
                with(&|o| {
                    if let Some(order) = o.order_mut(2) {
                        order.reached = Some(Reached {
                            utc: 1,
                            venue: cid("ceres"),
                        });
                    }
                }),
            ),
            (
                "two inflight orders",
                with(&|o| {
                    for id in [1, 2] {
                        if let Some(order) = o.order_mut(id) {
                            order.state = OrderState::Inflight;
                        }
                    }
                    o.inflight = Some(inflight(1));
                }),
            ),
            (
                "an inflight order without its record",
                with(&|o| {
                    if let Some(order) = o.order_mut(1) {
                        order.state = OrderState::Inflight;
                    }
                }),
            ),
            (
                "a record re-staged three times",
                with(&|o| {
                    if let Some(order) = o.order_mut(1) {
                        order.state = OrderState::Inflight;
                    }
                    let mut record = inflight(1);
                    record.restaged = 3;
                    o.inflight = Some(record);
                }),
            ),
            (
                "a cursor that is not the first unfinished",
                with(&|o| o.cursor = Some(2)),
            ),
            (
                "a held state without a note",
                with(&|o| o.state = PlanState::Held),
            ),
            (
                "a note naming a non-cursor order",
                with(&|o| {
                    o.stop_at(2, "x", 0);
                }),
            ),
            (
                "a paused plan with a note",
                with(&|o| {
                    o.stop_at(1, "x", 0);
                    o.state = PlanState::Paused;
                }),
            ),
            (
                "a held order with no note",
                with(&|o| {
                    if let Some(order) = o.order_mut(1) {
                        order.state = OrderState::Held;
                    }
                }),
            ),
            (
                "a 241-byte log line",
                with(&|o| {
                    o.log.push(LogLine {
                        utc: 0,
                        ic: 0,
                        order: 1,
                        text: "x".repeat(241),
                    });
                }),
            ),
            (
                "a two-row log line",
                with(&|o| {
                    o.log.push(LogLine {
                        utc: 0,
                        ic: 0,
                        order: 1,
                        text: "a\nb".to_owned(),
                    });
                }),
            ),
        ];
        for (why, bad) in cases {
            assert!(bad.check().is_err(), "{why} is refused");
        }
        // push_log keeps the newest 32, clamps on a char boundary and flattens rows.
        let mut long = good.clone();
        for n in 1..=40 {
            long.push_log(n, n, 1, &format!("line {n}"));
        }
        assert_eq!(long.log.len(), MAX_ORDER_LOG_LINES);
        assert_eq!(long.log.first().map(|l| l.text.as_str()), Some("line 9"));
        let mut text = "a".repeat(239);
        text.push('€');
        long.push_log(0, 0, 1, &text);
        assert_eq!(long.log.last().map(|l| l.text.len()), Some(239));
        long.push_log(0, 0, 1, "a\nb");
        assert_eq!(long.log.last().map(|l| l.text.as_str()), Some("a b"));
        long.check().unwrap();
    }

    /// Reviewer F2: the four hold shapes, each built through the helper the executor
    /// and the verbs use, serialized, decoded and checked.
    #[test]
    fn ic11_every_hold_shape_the_executor_writes_checks() {
        let round_trip = |orders: &Orders| {
            orders.check().unwrap();
            let wire = serde_json::to_value(orders).unwrap();
            let back: Orders = serde_json::from_value(wire).unwrap();
            assert_eq!(&back, orders);
            back.check().unwrap();
        };
        // Stopped.
        let mut stopped = plan();
        stopped.stop_at(1, "no house here buys metal concentrate.", 10);
        assert_eq!(stopped.state, PlanState::Held);
        assert_eq!(stopped.order(1).map(|o| o.state), Some(OrderState::Held));
        round_trip(&stopped);
        // Stopped over inflight.
        let mut over = plan();
        if let Some(order) = over.order_mut(1) {
            order.state = OrderState::Inflight;
        }
        let mut record = inflight(1);
        record.restaged = 2;
        over.inflight = Some(record);
        over.stop_at(
            1,
            "the counter has not confirmed it; staff have been told.",
            10,
        );
        assert_eq!(over.order(1).map(|o| o.state), Some(OrderState::Inflight));
        round_trip(&over);
        // Waiting.
        let mut waiting = plan();
        assert!(waiting.wait_at(
            1,
            "your charter of the Salt Moth is no longer in force.",
            10
        ));
        assert!(!waiting.wait_at(
            1,
            "your charter of the Salt Moth is no longer in force.",
            20
        ));
        assert_eq!(waiting.state, PlanState::Running);
        round_trip(&waiting);
        // Waiting over inflight.
        let mut waiting_over = plan();
        if let Some(order) = waiting_over.order_mut(1) {
            order.state = OrderState::Inflight;
        }
        waiting_over.inflight = Some(inflight(1));
        assert!(waiting_over.wait_at(
            1,
            "your charter of the Salt Moth is no longer in force.",
            10
        ));
        round_trip(&waiting_over);
        // The self-heal and del/skip clear a note naming the order.
        let mut healed = stopped.clone();
        healed.clear_hold_naming(1);
        assert_eq!(healed.state, PlanState::Running);
        assert!(healed.hold.is_none());

        // `put_orders` refuses a broken document with nothing written (reviewer F2):
        // no RAM install, no claimed key, no queued op — then a good one writes once.
        // The `Ctx` is built by hand as `Engine::modules_loaded` builds one (the
        // `seed.rs` precedent).
        let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(
            crate::TacsimRuleset::with_config(crate::fixture::config()),
        )]);
        let engine = tw.engine_mut();
        let mut out = mush_core::Outbox::new();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: mush_core::GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(crate::store::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let mut broken = plan();
        broken.hold = Some(HoldNote {
            order: 2,
            reason: "a note on an order the cursor has not reached.".to_owned(),
            since: 10,
            retry: true,
        });
        assert!(broken.check().is_err(), "the note names a non-cursor order");
        let hull = broken.hull;
        assert!(crate::store::put_orders(&mut ctx, &broken).is_err());
        assert!(matches!(orders_state(ctx.w, hull), OrdersState::Absent));
        assert!(
            !ctx.w
                .module_state::<TacsimStore>()
                .expect("the tacsim slot")
                .orders_keys
                .contains(&orders_key(hull)),
            "no key claimed"
        );
        assert!(ctx.out.deferred().is_empty(), "no op queued");
        let good = plan();
        crate::store::put_orders(&mut ctx, &good).expect("a good plan writes");
        assert!(matches!(orders_state(ctx.w, hull), OrdersState::Present(doc) if *doc == good));
        assert_eq!(ctx.out.deferred().len(), 1, "one whole-document write");
    }

    #[test]
    fn ic11_the_wire_round_trips_and_refuses_unknown_fields() {
        let wire = serde_json::json!({
            "schema": 1,
            "hull": 5011,
            "author": 42,
            "rev": 3,
            "state": "running",
            "hold": null,
            "cursor": 2,
            "next_id": 4,
            "inflight": null,
            "orders": [
                {"id":1,"state":"done","attempt":1,"refusals":0,"retry_at":null,"announced":false,"reached":null,
                 "what":{"sell":{"good":"metals","qty":"all","min_major":250,"on_short":"partial"}}},
                {"id":2,"state":"pending","attempt":1,"refusals":0,"retry_at":null,"announced":false,"reached":null,
                 "what":{"buy":{"good":"propellant","qty":"fill","max_major":60,"on_short":"partial"}}},
                {"id":3,"state":"pending","attempt":1,"refusals":0,"retry_at":null,"announced":false,"reached":null,
                 "what":{"depart":{"to":"ceres","not_before":{"utc_daily":"19:30"},"not_after_s":7200,"on_fail":"hold","crew":[]}}}
            ],
            "log": [{"utc": 1, "ic": 2, "order": 1, "text": "Sold."}]
        });
        let orders: Orders = serde_json::from_value(wire.clone()).unwrap();
        orders.check().unwrap();
        assert_eq!(serde_json::to_value(&orders).unwrap(), wire);
        let stop: Order = serde_json::from_value(serde_json::json!(
            {"id":5,"state":"pending","attempt":1,"refusals":0,"retry_at":null,"announced":false,"reached":null,"what":"stop"}
        ))
        .unwrap();
        assert_eq!(stop.what, What::Stop);
        for (qty, want) in [
            (serde_json::json!("fill"), Qty::Fill),
            (serde_json::json!("all"), Qty::All),
            (serde_json::json!(20), Qty::Units(20)),
        ] {
            assert_eq!(serde_json::from_value::<Qty>(qty).unwrap(), want);
        }
        assert!(serde_json::from_value::<Qty>(serde_json::json!("some")).is_err());
        // An extra field at each level is refused.
        let mut top = wire.clone();
        top["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Orders>(top).is_err());
        let mut order = wire.clone();
        order["orders"][0]["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Orders>(order).is_err());
        let mut what = wire.clone();
        what["orders"][0]["what"]["sell"]["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Orders>(what).is_err());
        let mut line = wire.clone();
        line["log"][0]["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Orders>(line).is_err());
        let mut depart = wire;
        depart["orders"][2]["what"]["depart"]["extra"] = serde_json::json!(1);
        assert!(serde_json::from_value::<Orders>(depart).is_err());
    }
}
