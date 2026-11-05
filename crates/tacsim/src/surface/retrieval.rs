//! **The retrieval contract** (doc 41 cut RS.5; §4.3, Q-41-8..11, Q-38-6).
//!
//! A hull's title holder posts a public job to bring her to a port; whoever accepts it tows her,
//! loads her into their own bay as **retrieval custody** (no salvage row, no claim), and hands her
//! over at the destination, where one of the four handover acts posts the delivery fact
//! ([`FACT_DELIVERED`]) the contract's condition reads, and the escrowed fee pays the acceptor.
//!
//! - **The contract** is `mush_economy::post_retrieval`'s shape, issued by the poster, its fee
//!   escrowed at posting, its one deadline twice [`RETRIEVAL_WINDOW_IC_DAYS`];
//!   `mush_economy::accept_retrieval` adds the acceptor in one write.
//! - **The windows** live here, on [`RetrievalOrder`] (`tacsim:retrieval:{hull}`), because a
//!   contract's deadline is hashed into its consent: 28 IC days to accept from posting, 28 to
//!   perform from a PC's acceptance (Q-41-11). The clerk's arm (`tick`, on the port tick)
//!   hands an unaccepted job to Tycho Mutual at the acceptance window's end, on the same
//!   contract; at a PC's lapse it cancels the contract (the escrow back to the poster) and posts
//!   a fresh one for Tycho with a window's deadline, which Tycho accepts on the same tick.
//! - **The fee** ([`floor_minor`]): Tycho's quote is the floor and nobody posts under it —
//!   the recovery row's slot-day at its margin over the voyage's days plus one ground day.
//! - **Who posts**: `repossess` and the write-off, for a lease rig left on a site (from
//!   Occator's account, to Ceres), and any title holder by hand (`+salvage/retrieve`).
//! - **The handovers** ([`deliver`]): a berth into the destination's workshop, a stow into the
//!   poster's own carrier there, the shed's hand-back of a lease rig at the destination when
//!   the acceptor's carrier docks ([`on_docked`]), and a hull's own docking there (the space
//!   seam, RS.8's to call).
//!
//! `+salvage/retrieve`, `+salvage/jobs` and `+salvage/accept` are this module's verbs; `+debts`
//! is `debts_cmd`.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{
    AccountId, Amount, ContractId, ContractState, EconomyStore, ExternalRef, FactProvider,
    FactState,
};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::store::TacsimStore;

/// **`RETRIEVAL_WINDOW_IC_DAYS`** (Q-41-11): the acceptance window and a PC acceptor's
/// performance window, IC days (provisional; `[tacsim.recovery].retrieval_window_ic_days`).
pub const RETRIEVAL_WINDOW_IC_DAYS: i64 = 28;
/// The largest window the config admits, IC days.
pub const MAX_RETRIEVAL_WINDOW_IC_DAYS: i64 = 365;
/// The record's key prefix.
pub const RETRIEVAL_PREFIX: &str = "tacsim:retrieval:";
/// The fact namespace this module answers (the composition registers [`RetrievalFactProvider`]).
pub const PROVIDER: &str = "tacsim.retrieval";
/// The delivery fact: `{ contract, port }` — the hull handed over at its destination.
pub const FACT_DELIVERED: &str = "tacsim.retrieval.delivered";
/// The namespace of every one-term debt this layer opens (Q-41-13), which `+debts` reads.
pub const DEBT_NAMESPACE: &str = "tacsim.debt";
/// **A one-term debt's grace, IC days** (the owner's 2026-11-04 rule: any wait on a player's
/// response is at least 28 IC days): a debt this layer opens falls due this long after it is
/// opened, payable at any time before (`+debts/pay`); only then can it go delinquent, so the
/// desk's refusal and port security's quarantine wait for it.
pub const DEBT_GRACE_IC_DAYS: i64 = 28;

/// **When a debt opened now falls due**: [`DEBT_GRACE_IC_DAYS`] from World IC now.
#[must_use]
pub fn debt_due(w: &World) -> i64 {
    w.world_ic_now()
        .at
        .saturating_add(DEBT_GRACE_IC_DAYS.saturating_mul(86_400))
}
/// The record's schema.
pub const RETRIEVAL_SCHEMA: u32 = 1;

/// IC seconds in an IC day.
const DAY: i64 = 86_400;

/// `+salvage/retrieve`'s usage.
pub const RETRIEVE_USAGE: &str = "Usage: +salvage/retrieve <hull>[=<port>] [fee=<cr>] [go] — post a job to bring a hull of \
     yours to a port.";
/// `+salvage/accept`'s usage.
pub const ACCEPT_USAGE: &str = "Usage: +salvage/accept <job> [go] — take a retrieval job.";
/// No such open job.
pub const NO_SUCH_JOB: &str = "There is no open retrieval job {job}; +salvage/jobs lists them.";
/// The board, empty.
pub const NO_JOBS: &str = "No retrieval jobs are open.";
/// The board's header.
pub const JOBS_HEAD: &str = "Open retrieval jobs:";
/// One job: `{job}`, `{name}`, `{lies}`, `{port}`, `{fee}`, `{days}`, `{who}`.
pub const JOB_LINE: &str =
    "  {job}  {name} — lies {lies}; to {port}; fee {fee}; {days} IC days left{who}";
/// The post's preview: `{name}`, `{port}`, `{fee}`, `{floor}`, `{days}`.
pub const POST_PREVIEW: &str = "Post a retrieval of {name} to {port} for {fee} (Tycho Mutual's \
     quote, the floor, is {floor}). The fee is escrowed now and paid on delivery; anyone may \
     accept within {days} IC days, then Tycho Mutual takes it. Add go to post.";
/// The post, done: `{name}`, `{job}`, `{fee}`.
pub const POSTED: &str = "Retrieval job {job} posted for {name}: {fee} escrowed, paid on \
     delivery. +salvage/jobs lists it.";
/// Under the floor: `{floor}`.
pub const UNDER_FLOOR: &str =
    "A retrieval is never posted under Tycho Mutual's quote, the floor: {floor}.";
/// Not the title holder's.
pub const NOT_YOUR_HULL: &str = "You hold no title to that hull.";
/// Already posted.
pub const ALREADY_POSTED: &str = "A retrieval of that hull is open already ({job}).";
/// Aboard a carrier: nothing to retrieve.
pub const ABOARD: &str =
    "That hull is aboard a carrier; a retrieval is for a hull that lies on a site.";
/// No such port.
pub const NO_SUCH_PORT: &str = "No port answers to {port}.";
/// The economy refused.
pub const POST_FAILED: &str = "The retrieval could not be posted: {why}.";
/// The accept's preview: `{name}`, `{lies}`, `{port}`, `{fee}`, `{days}`.
pub const ACCEPT_PREVIEW: &str = "Accept the job: bring {name} (lying {lies}) to {port} within \
     {days} IC days of accepting, for {fee}. Tow her, load her into your own bay, and hand her \
     over there: a berth in its workshop, or the shed's hand-back for a lease rig. Add go to \
     accept.";
/// The accept, done: `{job}`, `{name}`, `{days}`.
pub const ACCEPTED: &str = "You accept job {job}: {name} is yours to bring in within {days} IC days. Loaded into your \
     own bay she is held for the job, not as salvage.";
/// Your own job.
pub const OWN_JOB: &str = "That job is your own.";
/// The economy refused the acceptance.
pub const ACCEPT_FAILED: &str = "The job could not be accepted: {why}.";
/// To the poster, on an acceptance: `{name}`, `{who}`.
pub const TOLD_ACCEPTED: &str = "{who} has accepted the retrieval of {name}.";
/// To the poster, when Tycho takes it: `{name}`.
pub const TOLD_HOUSE: &str = "Nobody took the retrieval of {name}; Tycho Mutual has it now.";
/// To a lapsed acceptor: `{name}`.
pub const TOLD_LAPSED: &str =
    "Your retrieval of {name} has lapsed; the job is Tycho Mutual's now. Nothing is charged.";
/// To both, on the delivery: `{name}`, `{port}`.
pub const TOLD_DELIVERED: &str = "{name} has been handed over at {port}; the fee follows.";
/// To the acceptor, paid: `{name}`, `{fee}`.
pub const TOLD_PAID: &str = "You are paid {fee} for the retrieval of {name}.";

// ---- the record ----------------------------------------------------------------------------

/// **`tacsim:retrieval:{hull.0}`**.
#[must_use]
pub fn retrieval_key(hull: ObjId) -> SmolStr {
    SmolStr::new(format!("{RETRIEVAL_PREFIX}{}", hull.0))
}

/// The hull a retrieval record's key names.
#[must_use]
pub fn retrieval_key_owner(key: &str) -> Option<ObjId> {
    key.strip_prefix(RETRIEVAL_PREFIX)?
        .parse::<u32>()
        .ok()
        .map(ObjId)
}

/// **Where a retrieval stands.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStage {
    /// Posted, nobody has accepted: on the board.
    Open,
    /// A player (or their organization) accepted; the performance window runs.
    Accepted,
    /// Tycho Mutual's: unaccepted at the window's end, or a player's lapse (Q-41-11).
    House,
    /// Handed over at the destination; the contract settles on the sweep.
    Delivered,
    /// Paid, cancelled or expired: nothing more happens.
    Closed,
}

/// **A handover** (the delivery fact's content): where, when, and by whose act.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    /// The port entry it happened at.
    pub port: CanonicalId,
    /// IC seconds.
    pub at_ic: i64,
    /// Who handed her over.
    pub by: ObjId,
}

/// **One retrieval** (doc 41 cut RS.5, Q-41-11): the job's two windows under its one contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetrievalOrder {
    /// [`RETRIEVAL_SCHEMA`].
    pub schema: u32,
    /// The hull.
    pub hull: ObjId,
    /// The contract, `ContractId.0`: the current one (a lapse replaces it).
    pub contract: u64,
    /// The delivery fact's revision: bumped by every fresh contract, so a fact never answers a
    /// contract it was not written for.
    pub serial: u64,
    /// The poster (the title holder).
    pub poster: PrincipalId,
    /// The poster's account, `AccountId.0`.
    pub poster_account: u64,
    /// The destination port entry.
    pub destination: CanonicalId,
    /// The fee, minor units.
    pub fee_minor: i64,
    /// Tycho's quote at posting, minor units: the floor, and what a repossession charges.
    pub floor_minor: i64,
    /// Posted, IC seconds.
    pub posted_ic: i64,
    /// The acceptance window's end, IC seconds.
    pub accept_by_ic: i64,
    /// Who accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptor: Option<PrincipalId>,
    /// A PC acceptor's performance window's end, IC seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub perform_by_ic: Option<i64>,
    /// The handover, once made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivered: Option<Delivery>,
    /// Where it stands.
    pub stage: RetrievalStage,
}

impl RetrievalOrder {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// The first bound that fails.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != RETRIEVAL_SCHEMA {
            return Err(format!("schema {} is not {RETRIEVAL_SCHEMA}", self.schema));
        }
        if self.fee_minor <= 0 || self.floor_minor < 0 || self.fee_minor < self.floor_minor {
            return Err("the fee is under the floor or not positive".to_owned());
        }
        if self.accept_by_ic < self.posted_ic {
            return Err("the acceptance window ends before the posting".to_owned());
        }
        Ok(())
    }

    /// Whether the job is still to do (on the board, accepted, or Tycho's).
    #[must_use]
    pub fn active(&self) -> bool {
        matches!(
            self.stage,
            RetrievalStage::Open | RetrievalStage::Accepted | RetrievalStage::House
        )
    }

    /// **Whether it still stands in the way of a new post** (review 38a #2): any stage but
    /// `Closed` — a delivered job is still unpaid until the sweep settles it, and its delivery fact
    /// is the one its contract settles on, so a repost before then would overwrite it.
    #[must_use]
    pub fn open(&self) -> bool {
        self.stage != RetrievalStage::Closed
    }

    /// The job's board word: the hull's dbref.
    #[must_use]
    pub fn job(&self) -> String {
        format!("#{}", self.hull.0)
    }
}

/// A record as it loaded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetrievalSlot {
    /// Read and checked.
    Present(Box<RetrievalOrder>),
    /// The key is claimed but the document will not read: the hull's job is held.
    Unreadable,
}

/// The retrieval record on `hull`, when one reads.
#[must_use]
pub fn order_of(w: &World, hull: ObjId) -> Option<RetrievalOrder> {
    match w.module_state::<TacsimStore>()?.retrievals.get(&hull)? {
        RetrievalSlot::Present(order) => Some((**order).clone()),
        RetrievalSlot::Unreadable => None,
    }
}

/// Every readable retrieval record, by hull.
pub(crate) fn orders(w: &World) -> Vec<RetrievalOrder> {
    w.module_state::<TacsimStore>()
        .map(|store| {
            store
                .retrievals
                .values()
                .filter_map(|slot| match slot {
                    RetrievalSlot::Present(order) => Some((**order).clone()),
                    RetrievalSlot::Unreadable => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// **Whether `who` acts for `principal`**: the character themself, or one the economy names
/// as acting for the organization (its members and house agents).
#[must_use]
pub fn acts_for(w: &World, who: ObjId, principal: &PrincipalId) -> bool {
    match principal {
        PrincipalId::Object(object) => *object == who,
        _ => mush_economy::principal_holders(w, principal).contains(&who),
    }
}

/// **The active retrieval `who` holds on `hull`** (Q-41-8's retrieval custody): an order
/// accepted by `who` or an organization `who` acts for.
#[must_use]
pub fn held_by(w: &World, hull: ObjId, who: ObjId) -> Option<RetrievalOrder> {
    order_of(w, hull).filter(|order| {
        matches!(
            order.stage,
            RetrievalStage::Accepted | RetrievalStage::House
        ) && order
            .acceptor
            .as_ref()
            .is_some_and(|acceptor| acts_for(w, who, acceptor))
    })
}

/// The window, IC seconds, from the config.
fn window_secs(w: &World) -> i64 {
    w.module_state::<TacsimStore>()
        .map_or(RETRIEVAL_WINDOW_IC_DAYS, |store| {
            store.config().recovery.retrieval_window_ic_days
        })
        .clamp(1, MAX_RETRIEVAL_WINDOW_IC_DAYS)
        .saturating_mul(DAY)
}

/// The delivery fact a record's contract reads.
fn fact_of(hull: ObjId, serial: u64) -> ExternalRef {
    ExternalRef {
        provider: PROVIDER.into(),
        kind: FACT_DELIVERED.into(),
        id: hull.0.to_string().into(),
        revision: Some(serial),
    }
}

// ---- the fact ------------------------------------------------------------------------------

/// **The delivery fact's provider** (`tacsim.retrieval`): [`FACT_DELIVERED`] holds for a
/// record whose serial the reference names, once a handover at its destination is recorded.
/// Public: a retrieval is the poster's own notice of their own property, on a public board.
#[derive(Debug, Default)]
pub struct RetrievalFactProvider;

impl FactProvider for RetrievalFactProvider {
    fn provider(&self) -> &str {
        PROVIDER
    }

    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        if kind != FACT_DELIVERED {
            return None;
        }
        let hull = ObjId(id.parse::<u32>().ok()?);
        let order = order_of(w, hull)?;
        if Some(order.serial) != revision {
            return None;
        }
        let delivery = order
            .delivered
            .as_ref()
            .filter(|d| d.port == order.destination)?;
        Some(FactState {
            satisfied: true,
            actor: Some(delivery.by),
            at: 0,
        })
    }
}

// ---- the fee -------------------------------------------------------------------------------

/// The lane's ports by venue, and each venue's neighbours (both directions of every route).
fn venue_graph(w: &World) -> BTreeMap<CanonicalId, Vec<CanonicalId>> {
    let mut graph: BTreeMap<CanonicalId, Vec<CanonicalId>> = BTreeMap::new();
    for (_, def, _) in crate::port::market::ports(w) {
        for to in def.routes.keys() {
            graph.entry(def.venue.clone()).or_default().push(to.clone());
            graph.entry(to.clone()).or_default().push(def.venue.clone());
        }
    }
    graph
}

/// Route hops between two venues, breadth first (`None` when unconnected).
fn hops(
    graph: &BTreeMap<CanonicalId, Vec<CanonicalId>>,
    from: &CanonicalId,
    to: &CanonicalId,
) -> Option<u64> {
    if from == to {
        return Some(0);
    }
    let mut seen: BTreeMap<&CanonicalId, u64> = BTreeMap::new();
    seen.insert(from, 0);
    let mut frontier = vec![from];
    let mut depth = 0_u64;
    while !frontier.is_empty() && depth < 32 {
        depth = depth.saturating_add(1);
        let mut next = Vec::new();
        for venue in frontier {
            for neighbour in graph.get(venue).into_iter().flatten() {
                if neighbour == to {
                    return Some(depth);
                }
                if !seen.contains_key(neighbour) {
                    seen.insert(neighbour, depth);
                    next.push(neighbour);
                }
            }
        }
        frontier = next;
    }
    None
}

/// A port entry's venue.
fn venue_of(w: &World, entry: &CanonicalId) -> Option<CanonicalId> {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(id, _, _)| id == entry)
        .map(|(_, def, _)| def.venue.clone())
}

/// **Tycho Mutual**, as the clerk reaches it: its port entry, its organization and its account.
#[derive(Clone, Debug)]
pub struct House {
    /// The port entry its recovery row is authored on (Ceres Freeport).
    pub port: CanonicalId,
    /// The organization.
    pub org: PrincipalId,
    /// Its account, once opened.
    pub account: Option<AccountId>,
    /// The recovery row.
    pub recovery: crate::port::def::RecoveryDef,
}

/// The port that authors a recovery service, and its operator.
#[must_use]
pub fn house(w: &World) -> Option<House> {
    crate::port::market::ports(w)
        .into_iter()
        .find_map(|(entry, def, map)| {
            let recovery = def.recovery.as_deref()?.clone();
            let row = map?.orgs.get(&recovery.operator)?;
            Some(House {
                port: entry,
                org: PrincipalId::Organization(OrgId(row.org)),
                account: row.account.map(AccountId),
                recovery,
            })
        })
}

/// **The voyage's days** (Q-41-10, provisional): Tycho's port to where the hull lies and on to
/// the destination, by route hops, one day a hop less the one the round trip shares — a site
/// of Tycho's own port (Ahuna Flank, Urvara Reach: a landing from Ceres orbit) is one day —
/// plus one ground day. The planner's legs replace the hop count when RS.6's lifter has a
/// basis to plan on.
fn days(w: &World, lies: Option<&CanonicalId>, destination: &CanonicalId) -> u64 {
    let graph = venue_graph(w);
    let house = house(w).and_then(|h| venue_of(w, &h.port));
    let to = venue_of(w, destination);
    let at = lies.and_then(|site| venue_of(w, site));
    let legs = match (house.as_ref(), at.as_ref(), to.as_ref()) {
        (Some(house), Some(at), Some(to)) => hops(&graph, house, at)
            .unwrap_or(1)
            .saturating_add(hops(&graph, at, to).unwrap_or(1)),
        (Some(house), None, Some(to)) => hops(&graph, house, to).unwrap_or(1).max(1),
        _ => 2,
    };
    legs.saturating_sub(1).max(1).saturating_add(1)
}

/// **Tycho Mutual's quote for retrieving `hull` to `destination`** (Q-41-10): the floor every
/// posting is held to and what a repossession charges — the recovery row's fee for
/// `days`. The lifter's landing propellant is RS.6's to add with the lifter. `None` with no
/// recovery row on the lane.
#[must_use]
pub fn floor_minor(w: &World, hull: ObjId, destination: &CanonicalId) -> Option<i64> {
    let house = house(w)?;
    let lies = crate::surface::tow::ground_pose(w, hull).map(|(site, _)| site);
    let fee = house
        .recovery
        .retrieval_fee_minor(days(w, lies.as_ref(), destination));
    i64::try_from(fee).ok()
}

// ---- post, accept, deliver -------------------------------------------------------------

/// The hull's name as the board prints it.
fn name(w: &World, hull: ObjId) -> String {
    w.get(hull)
        .map_or_else(|| format!("#{}", hull.0), |o| o.name.clone())
}

/// A port entry's name.
pub(crate) fn port_name(w: &World, entry: &CanonicalId) -> String {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(id, _, _)| id == entry)
        .map_or_else(
            || entry.to_string(),
            |(_, def, _)| crate::port::coop::port_name(def),
        )
}

/// Where a hull lies, for the board.
fn lies_line(w: &World, hull: ObjId) -> String {
    if let Some((site, _)) = crate::surface::tow::ground_pose(w, hull) {
        return format!("on {}", port_name(w, &site));
    }
    match crate::surface::act::where_is(w, hull) {
        crate::surface::act::Where::Bay { hull: carrier } => format!("aboard {}", name(w, carrier)),
        _ => "elsewhere".to_owned(),
    }
}

/// **Post a retrieval of `hull`** (Q-41-8, Q-41-9): the contract and the record, the fee
/// escrowed from `poster_account`. `fee_minor` is never under [`floor_minor`].
///
/// # Errors
///
/// The refusal as a sentence: a job open already, no recovery row, a fee under the floor, or
/// the economy's refusal. Nothing is written on any.
pub fn post(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    (poster, poster_account): (PrincipalId, AccountId),
    destination: CanonicalId,
    fee_minor: Option<i64>,
) -> Result<RetrievalOrder, String> {
    let prior = order_of(ctx.w, hull);
    if let Some(order) = prior.as_ref().filter(|order| order.open()) {
        return Err(ALREADY_POSTED.replace("{job}", &order.job()));
    }
    let floor = floor_minor(ctx.w, hull, &destination).ok_or("no port runs a recovery service")?;
    let fee = fee_minor.unwrap_or(floor);
    if fee < floor {
        return Err(UNDER_FLOOR.replace("{floor}", &crate::port::coop::money_minor(ctx, floor)));
    }
    let now = ctx.w.world_ic_now().at;
    let window = window_secs(ctx.w);
    let serial = prior.map_or(1, |order| order.serial.saturating_add(1));
    let contract = mush_economy::post_retrieval(
        ctx,
        mush_economy::RetrievalSpec {
            name: SmolStr::new(format!("Retrieval: {}", name(ctx.w, hull))),
            poster: poster.clone(),
            poster_account,
            hull,
            fee: Amount::from_minor(fee),
            delivered: fact_of(hull, serial),
            deadline_ic: now.saturating_add(window.saturating_mul(2)),
        },
    )
    .map_err(|error| POST_FAILED.replace("{why}", &error.to_string()))?;
    let order = RetrievalOrder {
        schema: RETRIEVAL_SCHEMA,
        hull,
        contract: contract.0,
        serial,
        poster,
        poster_account: poster_account.0,
        destination,
        fee_minor: fee,
        floor_minor: floor,
        posted_ic: now,
        accept_by_ic: now.saturating_add(window),
        acceptor: None,
        perform_by_ic: None,
        delivered: None,
        stage: RetrievalStage::Open,
    };
    crate::store::put_retrieval(ctx, &order);
    Ok(order)
}

/// **Accept `hull`'s open job** for `acceptor` (Q-41-8): the contract's second party, and the
/// record's acceptor — with a performance window for a player, none for the house (Tycho's
/// window is what remains of the contract).
///
/// # Errors
///
/// No open job, the poster's own, or the economy's refusal. Nothing written on any.
pub fn accept(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    (acceptor, account): (PrincipalId, AccountId),
    house_acceptance: bool,
) -> Result<RetrievalOrder, String> {
    let mut order = order_of(ctx.w, hull)
        .filter(|order| order.stage == RetrievalStage::Open)
        .ok_or_else(|| NO_SUCH_JOB.replace("{job}", &format!("#{}", hull.0)))?;
    if order.poster == acceptor {
        return Err(OWN_JOB.to_owned());
    }
    mush_economy::accept_retrieval(ctx, ContractId(order.contract), &acceptor, account)
        .map_err(|error| ACCEPT_FAILED.replace("{why}", &error.to_string()))?;
    let now = ctx.w.world_ic_now().at;
    order.acceptor = Some(acceptor);
    if house_acceptance {
        order.stage = RetrievalStage::House;
        order.perform_by_ic = None;
    } else {
        order.stage = RetrievalStage::Accepted;
        order.perform_by_ic = Some(now.saturating_add(window_secs(ctx.w)));
    }
    crate::store::put_retrieval(ctx, &order);
    Ok(order)
}

/// **A handover act landed `hull` at `port`** by `by`'s hand (the four handovers, Q-41-8):
/// when an active retrieval held by `by` names `port` its destination, the delivery is
/// recorded — the fact its contract reads — and the poster and the acceptor are told; the
/// sweep settles the contract and pays the fee. Answers whether a delivery was recorded.
pub fn deliver(ctx: &mut Ctx<'_>, hull: ObjId, port: &CanonicalId, by: ObjId) -> bool {
    let Some(mut order) = held_by(ctx.w, hull, by) else {
        return false;
    };
    if &order.destination != port || order.delivered.is_some() {
        return false;
    }
    order.delivered = Some(Delivery {
        port: port.clone(),
        at_ic: ctx.w.world_ic_now().at,
        by,
    });
    order.stage = RetrievalStage::Delivered;
    crate::store::put_retrieval(ctx, &order);
    let line = TOLD_DELIVERED
        .replace("{name}", &name(ctx.w, hull))
        .replace("{port}", &port_name(ctx.w, port));
    tell(ctx, &order.poster, &line);
    if let Some(acceptor) = order.acceptor.clone() {
        tell(ctx, &acceptor, &line);
    }
    true
}

/// **A carrier docked at `port`** (the third handover, and Q-38-7's hand-back; the docking
/// hook): every hull in her bay under a retrieval her helm holds whose destination is `port`
/// is handed over — a lease rig (Occator's title) by the shed's hand-back, run here at the
/// destination rather than at the load on the site; and a written-off rig aboard (the second
/// write-off clock, Q-41-14) goes to the shed when `port` is the lessor's.
pub fn on_docked(ctx: &mut Ctx<'_>, carrier: ObjId, port: &CanonicalId) {
    let Some(helm) = crate::port::coop::helm_holder(ctx.w, carrier) else {
        return;
    };
    let lessor_port = crate::port::coop::programme_port(ctx.w).map(|p| p.entry.clone());
    for order in orders(ctx.w) {
        if !order.active()
            || &order.destination != port
            || crate::surface::act::where_is(ctx.w, order.hull)
                != (crate::surface::act::Where::Bay { hull: carrier })
        {
            continue;
        }
        let lease_rig = crate::port::tv::is_lease_rig(ctx.w, order.hull);
        if lease_rig {
            if lessor_port.as_ref() != Some(port) || held_by(ctx.w, order.hull, helm).is_none() {
                continue;
            }
            if crate::port::tv::hand_back(ctx, order.hull) {
                let _ = deliver(ctx, order.hull, port, helm);
            }
        }
    }
    if lessor_port.as_ref() == Some(port) {
        crate::port::tv::hand_back_aboard(ctx, carrier);
    }
}

/// **Withdraw `hull`'s open job** (nothing left to retrieve: the hull is home by another
/// way): its contract cancelled, the escrow back to the poster, the record closed. A job
/// already delivered or closed is left alone.
pub fn withdraw(ctx: &mut Ctx<'_>, hull: ObjId) {
    let Some(mut order) = order_of(ctx.w, hull).filter(RetrievalOrder::active) else {
        return;
    };
    match mush_economy::cancel_retrieval(ctx, ContractId(order.contract)) {
        Ok(_) => {
            order.stage = RetrievalStage::Closed;
            crate::store::put_retrieval(ctx, &order);
        }
        Err(error) => {
            tracing::warn!(hull = hull.0, %error, "tacsim: a retrieval could not be withdrawn");
        }
    }
}

/// Tell `principal` a line: the character, or everyone acting for the organization.
fn tell(ctx: &mut Ctx<'_>, principal: &PrincipalId, line: &str) {
    let who = match principal {
        PrincipalId::Object(object) => vec![*object],
        _ => mush_economy::principal_holders(ctx.w, principal),
    };
    for object in who {
        ctx.out
            .sys_to_actor(object, "tacsim.retrieval", line.to_owned());
    }
}

// ---- the clerk's arm -------------------------------------------------------------------------

/// The contract's state, when it reads.
fn contract_state(w: &World, contract: u64) -> Option<ContractState> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .contract(ContractId(contract))
        .map(|c| c.state)
}

/// **The clerk's arm** (Q-41-11), on the port tick: an unaccepted job past its acceptance
/// window goes to Tycho on the same contract; a player's lapse is cancelled and posted afresh
/// for Tycho, who accepts it on the same tick; a delivered job closes when its contract has
/// settled; a contract that ended any other way closes its record.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let now = ctx.w.world_ic_now().at;
    for mut order in orders(ctx.w) {
        if order.stage == RetrievalStage::Closed {
            continue;
        }
        let state = contract_state(ctx.w, order.contract);
        match (order.stage, state) {
            (RetrievalStage::Delivered, Some(ContractState::Settled { .. })) => {
                order.stage = RetrievalStage::Closed;
                crate::store::put_retrieval(ctx, &order);
                if let Some(acceptor) = order.acceptor.clone() {
                    let line = TOLD_PAID
                        .replace("{name}", &name(ctx.w, order.hull))
                        .replace(
                            "{fee}",
                            &crate::port::coop::money_minor(ctx, order.fee_minor),
                        );
                    tell(ctx, &acceptor, &line);
                }
            }
            (_, Some(state)) if state.is_terminal() => {
                order.stage = RetrievalStage::Closed;
                crate::store::put_retrieval(ctx, &order);
            }
            (RetrievalStage::Open, Some(_)) if now >= order.accept_by_ic => {
                to_house(ctx, &order);
            }
            (RetrievalStage::Accepted, Some(_))
                if order.perform_by_ic.is_some_and(|by| now >= by) =>
            {
                lapse(ctx, order);
            }
            _ => {}
        }
    }
}

/// Tycho accepts `order`'s contract (the acceptance window ran out).
fn to_house(ctx: &mut Ctx<'_>, order: &RetrievalOrder) {
    let Some(house) = house(ctx.w) else {
        return;
    };
    // Review 38b F-10: the house's arm delivers to its own port only; a job bound elsewhere stays
    // Open on the board for a player.
    if order.destination != house.port {
        return;
    }
    let Some(account) = house.account else {
        tracing::error!(
            hull = order.hull.0,
            "tacsim: Tycho Mutual has no account; the retrieval waits"
        );
        return;
    };
    match accept(ctx, order.hull, (house.org, account), true) {
        Ok(_) => {
            let line = TOLD_HOUSE.replace("{name}", &name(ctx.w, order.hull));
            tell(ctx, &order.poster, &line);
        }
        Err(why) => {
            tracing::warn!(hull = order.hull.0, %why, "tacsim: Tycho Mutual could not take a retrieval; the next tick tries again");
        }
    }
}

/// A player's acceptance lapsed: the contract cancelled, a fresh one posted for Tycho with one
/// window's deadline, and Tycho's acceptance on the same tick.
fn lapse(ctx: &mut Ctx<'_>, order: RetrievalOrder) {
    let Some(house) = house(ctx.w) else {
        return;
    };
    let Some(account) = house.account else {
        return;
    };
    match mush_economy::cancel_retrieval(ctx, ContractId(order.contract)) {
        Ok(true) => {}
        Ok(false) => return,
        Err(error) => {
            tracing::warn!(hull = order.hull.0, %error, "tacsim: a lapsed retrieval could not be cancelled; the next tick tries again");
            return;
        }
    }
    let now = ctx.w.world_ic_now().at;
    let serial = order.serial.saturating_add(1);
    let fresh = mush_economy::post_retrieval(
        ctx,
        mush_economy::RetrievalSpec {
            name: SmolStr::new(format!("Retrieval: {}", name(ctx.w, order.hull))),
            poster: order.poster.clone(),
            poster_account: AccountId(order.poster_account),
            hull: order.hull,
            fee: Amount::from_minor(order.fee_minor),
            delivered: fact_of(order.hull, serial),
            deadline_ic: now.saturating_add(window_secs(ctx.w)),
        },
    );
    let lapsed = order.acceptor.clone();
    let mut next = RetrievalOrder {
        serial,
        acceptor: None,
        perform_by_ic: None,
        posted_ic: now,
        accept_by_ic: now,
        stage: RetrievalStage::Open,
        ..order
    };
    match fresh {
        Ok(contract) => {
            next.contract = contract.0;
            // Review 38b F-10: bound elsewhere than the house's port, the fresh job stays Open on
            // the board for a player, for its whole acceptance window.
            let ours = next.destination == house.port;
            if !ours {
                next.accept_by_ic = now.saturating_add(window_secs(ctx.w));
            }
            crate::store::put_retrieval(ctx, &next);
            if ours && let Err(why) = accept(ctx, next.hull, (house.org, account), true) {
                tracing::warn!(hull = next.hull.0, %why, "tacsim: Tycho Mutual could not take a lapsed retrieval; the next tick tries again");
            }
        }
        Err(error) => {
            tracing::error!(hull = next.hull.0, %error, "tacsim: a lapsed retrieval could not be posted afresh; the job is closed");
            next.stage = RetrievalStage::Closed;
            crate::store::put_retrieval(ctx, &next);
        }
    }
    if let Some(acceptor) = lapsed {
        let line = TOLD_LAPSED.replace("{name}", &name(ctx.w, next.hull));
        tell(ctx, &acceptor, &line);
    }
}

// ---- the verbs -------------------------------------------------------------------------------

/// `[go]` off the tail.
pub(crate) fn split_go(raw: &str) -> (String, bool) {
    let raw = raw.trim();
    match raw.rsplit_once(char::is_whitespace) {
        Some((head, "go")) => (head.trim().to_owned(), true),
        _ if raw == "go" => (String::new(), true),
        _ => (raw.to_owned(), false),
    }
}

/// A hull word: `#<dbref>`, or a name matched against the hulls the actor holds titles to.
fn hull_word(ctx: &Ctx<'_>, word: &str) -> Option<ObjId> {
    let word = word.trim();
    if let Some(n) = word.strip_prefix('#') {
        return n.parse::<u32>().ok().map(ObjId);
    }
    let store = ctx.w.module_state::<EconomyStore>()?;
    store
        .snapshot()
        .assets()
        .filter(|asset| asset.title_owner == PrincipalId::Object(ctx.actor))
        .map(|asset| asset.object)
        .find(|object| {
            ctx.w
                .get(*object)
                .is_some_and(|o| o.name.eq_ignore_ascii_case(word))
        })
}

/// A port word (its venue, `ceres`) to its entry.
fn port_word(w: &World, word: &str) -> Option<CanonicalId> {
    let word = word.trim().to_ascii_lowercase();
    crate::port::market::ports(w)
        .into_iter()
        .find(|(entry, def, _)| {
            def.venue.as_str() == word
                || entry.as_str() == word
                || crate::port::coop::port_name(def).eq_ignore_ascii_case(&word)
        })
        .map(|(entry, _, _)| entry)
}

/// **`+salvage/retrieve <hull>[=<port>] [fee=<cr>] [go]`** (Q-41-9): a title holder posts a job
/// for a hull of theirs that lies on a site (or in someone else's custody with no claim, or a
/// wreck anywhere off a carrier); the destination defaults to Tycho's port.
///
/// # Errors
///
/// The refusal.
pub(crate) fn retrieve_cmd(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let (rest, go) = split_go(raw);
    let mut words = rest.split_whitespace();
    let head = words.next().ok_or_else(|| CmdError::msg(RETRIEVE_USAGE))?;
    let mut fee: Option<i64> = None;
    for word in words {
        let credits = word
            .strip_prefix("fee=")
            .and_then(|n| n.parse::<u64>().ok())
            .ok_or_else(|| CmdError::msg(RETRIEVE_USAGE))?;
        fee = Some(
            crate::port::apply::minor_of(credits, crate::port::apply::credit_scale(ctx.w))
                .ok_or_else(|| CmdError::msg(RETRIEVE_USAGE))?,
        );
    }
    let (hull_text, port_text) = match head.split_once('=') {
        Some((hull, port)) => (hull, Some(port)),
        None => (head, None),
    };
    let hull = hull_word(ctx, hull_text).ok_or_else(|| CmdError::msg(NOT_YOUR_HULL))?;
    // Review 38b F-21b: the title holder, or one who acts for its organization, which then posts
    // the job in its own name and from its own account.
    let owner = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|store| {
            store
                .snapshot()
                .asset(hull)
                .map(|asset| asset.title_owner.clone())
        })
        .filter(|owner| acts_for(ctx.w, ctx.actor, owner))
        .ok_or_else(|| CmdError::msg(NOT_YOUR_HULL))?;
    if matches!(
        crate::surface::act::where_is(ctx.w, hull),
        crate::surface::act::Where::Bay { .. }
    ) {
        return Err(CmdError::msg(ABOARD));
    }
    let destination = match port_text {
        Some(word) => port_word(ctx.w, word)
            .ok_or_else(|| CmdError::msg(NO_SUCH_PORT.replace("{port}", word)))?,
        None => house(ctx.w)
            .map(|h| h.port)
            .ok_or_else(|| CmdError::msg(NO_SUCH_PORT.replace("{port}", "home")))?,
    };
    let floor = floor_minor(ctx.w, hull, &destination)
        .ok_or_else(|| CmdError::msg("No port runs a recovery service."))?;
    let fee_minor = fee.unwrap_or(floor);
    if fee_minor < floor {
        return Err(CmdError::msg(
            UNDER_FLOOR.replace("{floor}", &crate::port::coop::money_minor(ctx, floor)),
        ));
    }
    if let Some(order) = order_of(ctx.w, hull).filter(RetrievalOrder::open) {
        return Err(CmdError::msg(ALREADY_POSTED.replace("{job}", &order.job())));
    }
    if !go {
        let line = POST_PREVIEW
            .replace("{name}", &name(ctx.w, hull))
            .replace("{port}", &port_name(ctx.w, &destination))
            .replace("{fee}", &crate::port::coop::money_minor(ctx, fee_minor))
            .replace("{floor}", &crate::port::coop::money_minor(ctx, floor))
            .replace("{days}", &(window_secs(ctx.w) / DAY).to_string());
        ctx.out.sys_to_actor(ctx.actor, "tacsim.retrieval", line);
        return Ok(());
    }
    let account = match &owner {
        PrincipalId::Object(_) => crate::port::credit::own_account(ctx.w, ctx.actor).map(|a| a.id),
        org => ctx.w.module_state::<EconomyStore>().and_then(|store| {
            store
                .snapshot()
                .accounts_of(org)
                .find(|a| {
                    a.instrument.as_str() == crate::economy::CREDIT
                        && a.status == mush_economy::AccountStatus::Active
                })
                .map(|a| a.id)
        }),
    }
    .ok_or_else(|| CmdError::msg("There is no account to escrow the fee from."))?;
    let order =
        post(ctx, hull, (owner, account), destination, Some(fee_minor)).map_err(CmdError::msg)?;
    let line = POSTED
        .replace("{name}", &name(ctx.w, hull))
        .replace("{job}", &order.job())
        .replace(
            "{fee}",
            &crate::port::coop::money_minor(ctx, order.fee_minor),
        );
    ctx.out.sys_to_actor(ctx.actor, "tacsim.retrieval", line);
    Ok(())
}

/// **`+salvage/jobs`** (Q-41-9): the open retrievals — hull, where it lies, destination, fee,
/// days left; a job accepted names its acceptor.
///
/// # Errors
///
/// None today; the signature is the switch table's.
pub(crate) fn jobs_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let now = ctx.w.world_ic_now().at;
    let mut lines = Vec::new();
    for order in orders(ctx.w) {
        let until = match order.stage {
            RetrievalStage::Open => order.accept_by_ic,
            RetrievalStage::Accepted => order.perform_by_ic.unwrap_or(order.accept_by_ic),
            _ => continue,
        };
        let who = match (&order.stage, &order.acceptor) {
            (RetrievalStage::Accepted, Some(PrincipalId::Object(o))) => {
                format!(" (taken by {})", name(ctx.w, *o))
            }
            (RetrievalStage::Accepted, Some(_)) => " (taken)".to_owned(),
            _ => String::new(),
        };
        lines.push(
            JOB_LINE
                .replace("{job}", &order.job())
                .replace("{name}", &name(ctx.w, order.hull))
                .replace("{lies}", &lies_line(ctx.w, order.hull))
                .replace("{port}", &port_name(ctx.w, &order.destination))
                .replace(
                    "{fee}",
                    &crate::port::coop::money_minor(ctx, order.fee_minor),
                )
                .replace(
                    "{days}",
                    &(until.saturating_sub(now).max(0).saturating_add(DAY - 1) / DAY).to_string(),
                )
                .replace("{who}", &who),
        );
    }
    let text = if lines.is_empty() {
        NO_JOBS.to_owned()
    } else {
        format!("{JOBS_HEAD}\n{}", lines.join("\n"))
    };
    ctx.out.sys_to_actor(ctx.actor, "tacsim.retrieval", text);
    Ok(())
}

/// **`+salvage/accept <job> [go]`** (Q-41-9): take an open job for yourself.
///
/// # Errors
///
/// The refusal.
pub(crate) fn accept_cmd(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let (word, go) = split_go(raw);
    if word.is_empty() {
        return Err(CmdError::msg(ACCEPT_USAGE));
    }
    let hull = word
        .trim()
        .trim_start_matches('#')
        .parse::<u32>()
        .ok()
        .map(ObjId)
        .ok_or_else(|| CmdError::msg(NO_SUCH_JOB.replace("{job}", word.trim())))?;
    let order = order_of(ctx.w, hull)
        .filter(|order| order.stage == RetrievalStage::Open)
        .ok_or_else(|| CmdError::msg(NO_SUCH_JOB.replace("{job}", word.trim())))?;
    if acts_for(ctx.w, ctx.actor, &order.poster) {
        return Err(CmdError::msg(OWN_JOB));
    }
    if !go {
        let line = ACCEPT_PREVIEW
            .replace("{name}", &name(ctx.w, hull))
            .replace("{lies}", &lies_line(ctx.w, hull))
            .replace("{port}", &port_name(ctx.w, &order.destination))
            .replace(
                "{fee}",
                &crate::port::coop::money_minor(ctx, order.fee_minor),
            )
            .replace("{days}", &(window_secs(ctx.w) / DAY).to_string());
        ctx.out.sys_to_actor(ctx.actor, "tacsim.retrieval", line);
        return Ok(());
    }
    let account = crate::port::credit::own_account(ctx.w, ctx.actor)
        .map(|a| a.id)
        .ok_or_else(|| CmdError::msg("You have no account to be paid into."))?;
    let order = accept(ctx, hull, (PrincipalId::Object(ctx.actor), account), false)
        .map_err(CmdError::msg)?;
    let line = ACCEPTED
        .replace("{job}", &order.job())
        .replace("{name}", &name(ctx.w, hull))
        .replace("{days}", &(window_secs(ctx.w) / DAY).to_string());
    ctx.out.sys_to_actor(ctx.actor, "tacsim.retrieval", line);
    let told = TOLD_ACCEPTED
        .replace("{name}", &name(ctx.w, hull))
        .replace("{who}", &name(ctx.w, ctx.actor));
    tell(ctx, &order.poster, &told);
    Ok(())
}

// ---- debts -----------------------------------------------------------------------------------

/// `+debts`: none owed.
pub const NO_DEBTS: &str = "You owe nothing.";
/// `+debts`: the header.
pub const DEBTS_HEAD: &str = "Your debts:";
/// One debt: `{id}`, `{amount}`, `{creditor}`, `{state}`.
pub const DEBT_LINE: &str = "  #{id}  {amount} to {creditor} ({state}) — +debts/pay {id}";
/// Paid: `{id}`, `{amount}`, `{creditor}`.
pub const DEBT_PAID: &str = "Debt #{id} paid: {amount} to {creditor}.";
/// Not one of yours.
pub const NO_SUCH_DEBT: &str = "You owe no debt #{id}; +debts lists yours.";
/// The account cannot cover it: `{amount}`.
pub const DEBT_SHORT: &str = "Your account cannot cover {amount}; nothing was paid.";
/// `+debts`' usage.
pub const DEBTS_USAGE: &str = "Usage: +debts  |  +debts/pay <id>";

/// The ruleset namespaces `+debts` reads: this layer's one.
pub const DEBT_NAMESPACES: &[&str] = &[DEBT_NAMESPACE];

/// **The sanction keys `+debts` reads beside them** (doc 42 cut CR.2, Q-42-7): a captive's
/// release fee, a one-shot `Sanction` keyed `release:<captive>:<due>`
/// (`crate::field::crew::release_key`), levied by a port authority's clerk.
pub const DEBT_SANCTION_PREFIXES: &[&str] = &["release:"];

/// The live one-term debts `who` owes that `+debts` reads: this layer's, and the release fees.
fn owed(w: &World, who: &PrincipalId) -> Vec<mush_economy::Obligation> {
    mush_economy::debts_of_with(w, who, DEBT_NAMESPACES, DEBT_SANCTION_PREFIXES)
}

/// **The cause `o` is paid or reinstated under**: its ruleset namespace, or the sanction prefix
/// its key starts with.
fn debt_cause(o: &mush_economy::Obligation) -> Option<mush_economy::DebtCause<'_>> {
    match &o.cause {
        mush_economy::ObligationCause::Ruleset { namespace } => {
            Some(mush_economy::DebtCause::Ruleset(namespace.as_str()))
        }
        mush_economy::ObligationCause::Sanction { inspection } => DEBT_SANCTION_PREFIXES
            .iter()
            .find(|prefix| inspection.starts_with(*prefix))
            .map(|prefix| mush_economy::DebtCause::Sanction(prefix)),
        _ => None,
    }
}

/// A principal's name for the debt lines.
pub(crate) fn principal_name(w: &World, who: &PrincipalId) -> String {
    match who {
        PrincipalId::Object(o) => name(w, *o),
        PrincipalId::Organization(org) => crate::port::market::ports(w)
            .into_iter()
            .filter_map(|(_, def, map)| map.map(|m| (def, m)))
            .find_map(|(def, map)| {
                map.orgs.iter().find_map(|(id, row)| {
                    (OrgId(row.org) == *org).then(|| {
                        def.orgs
                            .iter()
                            .find(|o| &o.id == id)
                            .map_or_else(|| id.to_string(), |o| o.name.clone())
                    })
                })
            })
            .unwrap_or_else(|| format!("organization {}", org.0)),
        other => format!("{other:?}"),
    }
}

/// **Whether a one-term debt is past its grace unpaid** (Q-41-13, Q-41-25; review 38a #1 / 38b
/// F-13): `Delinquent`, or reinstated to `Active` by the desk or the docking (`missed_total` keeps
/// the missed period) and not yet collected. A reinstatement only queues the next sweep's retry, so
/// until it pays, the debt still refuses the desk and holds the carrier.
#[must_use]
pub fn past_due(o: &mush_economy::Obligation) -> bool {
    match o.status {
        mush_economy::ObligationStatus::Delinquent => true,
        mush_economy::ObligationStatus::Active => o.missed_total > 0,
        _ => false,
    }
}

/// **The debts `actor` owes this layer past their grace** ([`past_due`]), with their amounts
/// (the desk's read).
#[must_use]
pub fn delinquent_debts(w: &World, actor: ObjId) -> Vec<mush_economy::Obligation> {
    owed(w, &PrincipalId::Object(actor))
        .into_iter()
        .filter(past_due)
        .collect()
}

/// **The desk's refusal of a delinquent debtor** (Q-41-13): `None` when `actor` owes nothing
/// delinquent; else the sentence naming the amount — and every such debt is reinstated, so the
/// next sweep tries it once more.
pub fn desk_refusal(ctx: &mut Ctx<'_>) -> Option<String> {
    let owed = delinquent_debts(ctx.w, ctx.actor);
    if owed.is_empty() {
        return None;
    }
    let total = owed
        .iter()
        .fold(0_i64, |sum, o| sum.saturating_add(o.amount.minor()));
    for o in owed
        .iter()
        .filter(|o| matches!(o.status, mush_economy::ObligationStatus::Delinquent))
    {
        let reinstated = match debt_cause(o) {
            Some(mush_economy::DebtCause::Ruleset(namespace)) => {
                mush_economy::reinstate_debt(ctx, o.id, namespace)
            }
            Some(mush_economy::DebtCause::Sanction(prefix)) => {
                mush_economy::reinstate_sanction_debt(ctx, o.id, prefix)
            }
            None => continue,
        };
        if let Err(error) = reinstated {
            tracing::warn!(obligation = o.id.0, %error, "tacsim: a delinquent debt could not be reinstated at the desk");
        }
    }
    Some(format!(
        "You owe {} in delinquent debts; the desk will not deal with you until they are paid \
         (+debts lists them; +debts/pay <id> pays one).",
        crate::port::coop::money_minor(ctx, total)
    ))
}

/// **`+debts`** and **`+debts/pay <id>`** (Q-41-13): the actor's one-term debts, and paying one
/// now — reinstated and settled in a single post when the account covers it.
///
/// # Errors
///
/// The refusal.
pub(crate) fn debts_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_owned();
    let me = PrincipalId::Object(ctx.actor);
    if ctx.switches.has("pay") {
        let id = raw
            .trim_start_matches('#')
            .parse::<u64>()
            .map_err(|_| CmdError::msg(DEBTS_USAGE))?;
        let debt = owed(ctx.w, &me)
            .into_iter()
            .find(|o| o.id.0 == id)
            .ok_or_else(|| CmdError::msg(NO_SUCH_DEBT.replace("{id}", &id.to_string())))?;
        let paid = match debt_cause(&debt) {
            Some(mush_economy::DebtCause::Ruleset(namespace)) => {
                mush_economy::pay_debt(ctx, debt.id, namespace)
            }
            Some(mush_economy::DebtCause::Sanction(prefix)) => {
                mush_economy::pay_sanction_debt(ctx, debt.id, prefix)
            }
            None => return Err(CmdError::msg(NO_SUCH_DEBT.replace("{id}", &id.to_string()))),
        };
        let amount = crate::port::coop::money_minor(ctx, debt.amount.minor());
        return match paid {
            Ok(mush_economy::PostOutcome::Posted(_)) => {
                let line = DEBT_PAID
                    .replace("{id}", &id.to_string())
                    .replace("{amount}", &amount)
                    .replace("{creditor}", &principal_name(ctx.w, &debt.creditor));
                ctx.out.sys_to_actor(ctx.actor, "tacsim.retrieval", line);
                Ok(())
            }
            Ok(_)
            | Err(
                mush_economy::EconomyError::InsufficientAvailable { .. }
                | mush_economy::EconomyError::CreditViolation(_),
            ) => Err(CmdError::msg(DEBT_SHORT.replace("{amount}", &amount))),
            Err(error) => Err(CmdError::msg(format!(
                "The debt could not be paid: {error}."
            ))),
        };
    }
    if !raw.is_empty() {
        return Err(CmdError::msg(DEBTS_USAGE));
    }
    let debts = owed(ctx.w, &me);
    let text = if debts.is_empty() {
        NO_DEBTS.to_owned()
    } else {
        let lines: Vec<String> = debts
            .iter()
            .map(|o| {
                let state = match o.status {
                    mush_economy::ObligationStatus::Delinquent => "delinquent",
                    _ => "due",
                };
                DEBT_LINE
                    .replace("{id}", &o.id.0.to_string())
                    .replace(
                        "{amount}",
                        &crate::port::coop::money_minor(ctx, o.amount.minor()),
                    )
                    .replace("{creditor}", &principal_name(ctx.w, &o.creditor))
                    .replace("{state}", state)
            })
            .collect();
        format!("{DEBTS_HEAD}\n{}", lines.join("\n"))
    };
    ctx.out.sys_to_actor(ctx.actor, "tacsim.retrieval", text);
    Ok(())
}
