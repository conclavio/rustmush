//! **The Isidis stockpile and the Freeport arm** (doc 50 cut OP.4; Q-50-21, Q-50-22, Q-50-24,
//! Q-50-60).
//!
//! - **The stockpile** (Q-50-21, Q-50-58): what Occator's tank farm at Isidis holds of
//!   [`STOCKPILE_GOOD`] in its storefront vault, against [`STOCKPILE_THRESHOLD_T`]. The port tick
//!   reads the level every fire and keeps it on the arm's record ([`FreeportArm::level`]). Doc
//!   50b item 3: on the first fire at or after the operation's close it reads the vault once more
//!   for that operation ([`FreeportArm::close_read`]), and the close probe ([`TANK_FARM_PROBE`],
//!   [`close_probe`]) answers from that reading alone, waiting until it is taken. A level is safe
//!   across a restart and nothing reads canon.
//! - **The threshold is news** (Q-50-21 as corrected): an IC month in which a reading stands at the
//!   threshold moves Hold the Concession one segment (doc 11 §6: "Occator stockpiles reach
//!   threshold"), its cause id `stockpile.threshold/<month>`, and the clock's line is the
//!   chronicle's record. It decides no operation.
//! - **The drain** (Q-50-60) is the lane's own: a `consume` row on the tank farm (EX.9's arm, lane
//!   v49), so each window's close reflects what was landed during that campaign.
//! - **The Freeport arm** (Q-50-22, procurement's pattern): an agent object of Occator's, a `DARK`
//!   Thing standing at Ceres' exchange with [`crate::services::JOBS_POST_CAPABILITY`] on Occator's
//!   root compartment, posts through the players' own verbs, once an IC day while the farm is
//!   under its threshold: a haul of [`ARM_HAUL_T`] t from Occator's vault at Ceres into the tank
//!   farm's at Isidis (`+haul/post ... as=occator into=isidis_tank_farm`), and an escort for it
//!   (`+escort/post haul=<job> ... as=occator`). Its money is Occator's treasury: a month's posts
//!   are capped at [`ARM_MONTHLY_CAP_CR`], counted from its own rows ([`ArmPost`]), and posting
//!   stops when the cap or the treasury runs out. No money is minted. The farm's standing bid,
//!   refreshed by the lane (v49), is the third arm Q-50-22 names.
//! - **If no player takes a haul** (Q-50-24): after [`FALLBACK_IC_DAYS`] an arm's haul still
//!   untaken at Ceres is carried by the port's hauling house (EX.10), in a real hull that crosses
//!   the picket and can be intercepted (`crate::port::hauler`, [`untaken`]).
//!
//! **The restart argument.** The record (`tacsim:freeport_arm`) holds the agent, its grant, the
//! last reading and every post; a post and its row are written in one fire's batch, and every
//! decision is re-read from the record, the hauls and the economy, so a restart posts nothing
//! twice. A post is at most once an IC day ([`FreeportArm::posted_day`]).

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::AccountId;
use mush_faction::ClockAdvance;
use mush_inventory::record::EndpointRef;
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};

use crate::port::stock::IC_DAY;
use crate::services::haul::{self, HaulStage};

/// The record's key.
pub const ARM_KEY: &str = "tacsim:freeport_arm";
/// The tank farm's lane organization.
pub const TANK_FARM_ORG: &str = "isidis_tank_farm";
/// The tank farm's port entry.
pub const TANK_FARM_ENTRY: &str = "isidis_port";
/// The stockpile's one good (Q-50-58: propellant only in the first operation).
pub const STOCKPILE_GOOD: &str = "propellant";
/// The level the farm must stand at, whole tonnes (provisional): a thousand tonnes over the 3,000
/// it is stocked with, so a fresh world starts short.
pub const STOCKPILE_THRESHOLD_T: u64 = 4_000;
/// The close probe's name (doc 50 cut OP.0's `CLOSE_PROBES`): met when the farm is at its
/// threshold, the defenders' success.
pub const TANK_FARM_PROBE: &str = "isidis.tank_farm";
/// Hold the Concession's cause-id prefix for the threshold; the IC month index follows it.
pub const THRESHOLD_CAUSE: &str = "stockpile.threshold/";
/// The arm's organization: Occator, the tank farm's backer (Q-50-22).
pub const ARM_ORG: &str = "occator";
/// Where the arm posts from: Ceres, Occator's port.
pub const ARM_ENTRY: &str = "ceres_port";
/// The room its agent stands in.
pub const ARM_ROOM: &str = "exchange";
/// One haul's size, whole tonnes (provisional).
pub const ARM_HAUL_T: u64 = 20;
/// One haul's fee, whole credits (provisional).
pub const ARM_HAUL_FEE_CR: i64 = 600;
/// One escort's fee, whole credits (provisional).
pub const ARM_ESCORT_FEE_CR: i64 = 400;
/// A month's cap on what the arm posts, whole credits (provisional; Q-50-22).
pub const ARM_MONTHLY_CAP_CR: i64 = 20_000;
/// At most this many of the arm's hauls on the board or under way at once.
pub const ARM_MAX_OPEN: usize = 2;
/// The IC days an arm's haul waits for a player before the hauling house takes it (Q-50-24,
/// provisional).
pub const FALLBACK_IC_DAYS: i64 = 7;
/// Posts the record keeps (the oldest drop off; a month's cap needs a month's rows).
pub const MAX_POSTS: usize = 128;

/// **The arm's record**, written whole when it changes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreeportArm {
    /// **Laid up by staff** (`@tacsim/port arm off`): it posts nothing, and what it posted runs
    /// on; the farm is still read (doc 50 cut OP.5: the close probe answers in any world). A rig whose stories predate OP.4 lays it up before its first fire, so it
    /// makes no agent.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub laid_up: bool,
    /// Its agent, a `DARK` Thing of Occator's at Ceres' exchange.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<u32>,
    /// The agent's grant of [`crate::services::JOBS_POST_CAPABILITY`], `GrantId.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<u64>,
    /// The tank farm's level at the last reading, whole tonnes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u64>,
    /// The IC day that reading was first taken.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_day: Option<i64>,
    /// The IC day the arm last posted, so it posts at most once a day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub posted_day: Option<i64>,
    /// Every post, oldest first (at most [`MAX_POSTS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub posts: Vec<ArmPost>,
    /// **The reading taken at the operation's close** (doc 50b item 3; Q-50-21: "read once, at
    /// the window's close"): the first good reading of the vault at or after the close, kept
    /// for that operation. The close probe answers from it alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close_read: Option<CloseRead>,
    /// **The tank farm's standing bid's fills** (doc 50b item 8; Q-50-22): each sale into it,
    /// counted in its IC month's spend beside the posts, and paid back to the farm from Occator
    /// (at most [`MAX_POSTS`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fills: Vec<ArmFill>,
}

/// **One sale into the tank farm's bid** (doc 50b item 8): its ledger row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmFill {
    /// The sale's quote, `QuoteId.0`: the fill's key, so a settlement told twice counts once.
    pub quote: u64,
    /// The IC month it settled in.
    pub month: i64,
    /// What the farm paid for it, minor units.
    pub spent_minor: i64,
    /// Whether Occator has paid it back to the farm.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paid: bool,
}

/// **The tank farm read at an operation's close** (doc 50b item 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseRead {
    /// The operation whose window closed.
    pub operation: u64,
    /// The vault's level, whole tonnes.
    pub level: u64,
    /// The IC instant it was read, at or after the close.
    pub read_ic: i64,
}

/// **One of the arm's posts**: its ledger row (Q-50-22: the cap is counted from these).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArmPost {
    /// The haul's contract.
    pub haul: u64,
    /// Its escort's contract, when that posted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escort: Option<u64>,
    /// The IC month it was posted in.
    pub month: i64,
    /// What it committed, minor units: the haul's fee and the escort's.
    pub spent_minor: i64,
}

impl FreeportArm {
    /// What the arm committed in IC month `month`, minor units: its posts, and (doc 50b item 8)
    /// the tank farm's bid's fills.
    #[must_use]
    pub fn spent_in(&self, month: i64) -> i64 {
        let posts: i64 = self
            .posts
            .iter()
            .filter(|p| p.month == month)
            .map(|p| p.spent_minor)
            .sum();
        let fills: i64 = self
            .fills
            .iter()
            .filter(|f| f.month == month)
            .map(|f| f.spent_minor)
            .sum();
        posts.saturating_add(fills)
    }

    /// **Whether a post costing `cost_minor` fits month `month`'s cap** (Q-50-22): what the
    /// month's rows committed, and it, no more than [`ARM_MONTHLY_CAP_CR`].
    #[must_use]
    pub fn within_cap(&self, month: i64, cost_minor: i64) -> bool {
        self.spent_in(month).saturating_add(cost_minor) <= ARM_MONTHLY_CAP_CR.saturating_mul(100)
    }
}

/// The arm's record, as the store holds it.
#[must_use]
pub fn arm(w: &World) -> Option<&FreeportArm> {
    w.module_state::<crate::TacsimStore>()?.freeport_arm()
}

/// **The close probe** (`CLOSE_PROBES`): whether the tank farm stood at its threshold when it
/// was read at `operation`'s close ([`FreeportArm::close_read`]). `None`, so the verdict waits,
/// until that reading is taken: never the last cached reading (doc 50b item 3), which a delivery
/// or a drain since may have changed, and never one older than the close.
#[must_use]
pub fn close_probe(w: &World, operation: u64) -> Option<bool> {
    arm(w)?
        .close_read
        .filter(|read| read.operation == operation)
        .map(|read| read.level >= STOCKPILE_THRESHOLD_T)
}

/// The line a sale into the tank farm's bid reads once the month's budget is spent (doc 50b
/// item 8; Q-50-22: "when the cap or the treasury runs out, posting stops").
pub const FARM_BUDGET_SPENT: &str = "Occator's Isidis budget for this IC month is spent; the tank \
     farm takes no more until the month turns.";

/// Whether `house` is the tank farm and `commodity` its stockpile's good: a fill of its bid.
fn is_farm_fill(w: &World, house: &PrincipalId, commodity: &CanonicalId) -> bool {
    commodity.as_str() == STOCKPILE_GOOD
        && org_of(w, TANK_FARM_ENTRY, TANK_FARM_ORG)
            .is_some_and(|farm| house == &PrincipalId::Organization(farm))
}

/// **Whether a sale of `gross` into the tank farm's bid is refused** (doc 50b item 8; Q-50-22):
/// Occator funds the bid, so a sale past its month's cap, or past what its account holds, is
/// refused with [`FARM_BUDGET_SPENT`]. Any other bid, or a world with no arm, is not.
#[must_use]
pub fn refuse_fill(
    w: &World,
    house: &PrincipalId,
    commodity: &CanonicalId,
    gross: i64,
) -> Option<&'static str> {
    if !is_farm_fill(w, house, commodity) {
        return None;
    }
    let month = crate::concession::ic_month(w.world_ic_now().at);
    let occator = org_of(w, ARM_ENTRY, ARM_ORG)
        .and_then(|occator| org_account(w, occator))
        .map(|account| available(w, account));
    fill_refused(arm(w), month, gross, occator).then_some(FARM_BUDGET_SPENT)
}

/// **Whether Occator cannot fund a fill of `gross` in IC month `month`** (doc 50b item 8): past
/// the arm's month's cap, or past what Occator's account holds (`None`: no account).
fn fill_refused(doc: Option<&FreeportArm>, month: i64, gross: i64, occator: Option<i64>) -> bool {
    let within = doc.is_none_or(|doc| doc.within_cap(month, gross));
    let funded = occator.is_some_and(|held| held >= gross);
    !within || !funded
}

/// **A sale into the tank farm's bid settled** (doc 50b item 8): its fill, kept once under its
/// quote and counted in this month's spend; the port tick pays it back to the farm from Occator.
pub(crate) fn on_fill(
    ctx: &mut Ctx<'_>,
    quote: u64,
    house: &PrincipalId,
    commodity: &CanonicalId,
    gross: i64,
) {
    if gross <= 0 || !is_farm_fill(ctx.w, house, commodity) {
        return;
    }
    let mut doc = arm(ctx.w).cloned().unwrap_or_default();
    if doc.fills.iter().any(|f| f.quote == quote) {
        return;
    }
    doc.fills.push(ArmFill {
        quote,
        month: crate::concession::ic_month(ctx.w.world_ic_now().at),
        spent_minor: gross,
        paid: false,
    });
    if doc.fills.len() > MAX_POSTS {
        // The oldest paid fills go first; an unpaid one is kept until it is paid.
        if let Some(at) = doc.fills.iter().position(|f| f.paid) {
            doc.fills.remove(at);
        }
    }
    put(ctx, &doc);
}

/// **Occator pays the tank farm back for its bid's fills** (doc 50b item 8; Q-50-22): each unpaid
/// fill, from Occator's account to the farm's, keyed by its quote so a retry pays once. One the
/// account cannot cover waits for the next fire.
fn fund_fills(ctx: &mut Ctx<'_>, occator: OrgId, doc: &mut FreeportArm) {
    if doc.fills.iter().all(|f| f.paid) {
        return;
    }
    let (Some(from), Some(to)) = (
        org_account(ctx.w, occator),
        org_of(ctx.w, TANK_FARM_ENTRY, TANK_FARM_ORG).and_then(|farm| org_account(ctx.w, farm)),
    ) else {
        return;
    };
    for fill in doc.fills.iter_mut().filter(|f| !f.paid) {
        if available(ctx.w, from) < fill.spent_minor {
            return;
        }
        fill.paid = crate::port::hauler::pay_in(
            ctx,
            ARM_KEY,
            format!("fill/{}", fill.quote),
            (from, to),
            fill.spent_minor,
            "Occator funds the Isidis tank farm's bid".to_owned(),
        );
    }
}

/// **The vault read at the operation's close** (doc 50b item 3): on the first fire at or after
/// the window's close of the operation the declaring arm holds, the vault is read once and kept
/// for it. A failed read keeps nothing, so the next fire tries again and the probe waits.
fn read_at_close(ctx: &Ctx<'_>, doc: &mut FreeportArm) {
    let Some(operation) = crate::isidis_op::record(ctx.w).and_then(|r| r.operation) else {
        return;
    };
    if doc
        .close_read
        .is_some_and(|read| read.operation == operation)
    {
        return;
    }
    let Some(status) = crate::isidis_op::status(ctx.w) else {
        return;
    };
    let now = ctx.w.world_ic_now().at;
    if now < status.window.1 {
        return;
    }
    if let Some(level) = read_level(ctx) {
        doc.close_read = Some(CloseRead {
            operation,
            level,
            read_ic: now,
        });
    }
}

/// The tank farm's storefront vault, `VaultId.0`.
#[must_use]
pub fn tank_farm_vault(w: &World) -> Option<u64> {
    let org = org_of(w, TANK_FARM_ENTRY, TANK_FARM_ORG)?;
    crate::services::org_vault(w, org, "isidis")
}

/// A lane organization's id, read off its port's map.
fn org_of(w: &World, entry: &str, id: &str) -> Option<OrgId> {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(e, _, _)| e.as_str() == entry)
        .and_then(|(_, _, map)| {
            map?.orgs
                .get(&CanonicalId::new(id).ok()?)
                .map(|row| OrgId(row.org))
        })
}

/// **The tank farm's level now**, whole tonnes (what the storefront pools of the good).
#[must_use]
pub fn read_level(ctx: &Ctx<'_>) -> Option<u64> {
    let vault = tank_farm_vault(ctx.w)?;
    let good = CanonicalId::new(STOCKPILE_GOOD).ok()?;
    crate::port_tick::read_pool(ctx, EndpointRef(mush_economy::VaultId(vault)), &good).ok()
}

/// **An arm's haul a hauling house may carry from `venue`** (Q-50-24): posted there at least
/// [`FALLBACK_IC_DAYS`] ago, still untaken, and not in `claimed`. The oldest first.
#[must_use]
pub fn untaken(w: &World, venue: &str, now_ic: i64, claimed: &[u64]) -> Option<u64> {
    let arm = arm(w)?;
    let wait = FALLBACK_IC_DAYS.saturating_mul(IC_DAY);
    arm.posts
        .iter()
        .filter(|p| !claimed.contains(&p.haul))
        .filter_map(|p| haul::haul_of(w, p.haul))
        .filter(|job| {
            job.stage == HaulStage::Posted
                && job.origin.as_str() == venue
                && now_ic >= job.posted_ic.saturating_add(wait)
                && now_ic < job.accept_by_ic
        })
        .map(|job| job.contract)
        .min()
}

/// **An arm's haul on the board at `venue`** (doc 50 cut OP.8; Q-50-24): posted there, untaken
/// and inside its acceptance window, however young. A hauling house's hull lying idle away from
/// that port is called home for it (`crate::port::hauler`), so she is there to carry it when its
/// [`FALLBACK_IC_DAYS`] are up. The oldest first.
#[must_use]
pub fn awaiting(w: &World, venue: &str, now_ic: i64) -> Option<u64> {
    arm(w)?
        .posts
        .iter()
        .filter_map(|p| haul::haul_of(w, p.haul))
        .filter(|job| {
            job.stage == HaulStage::Posted
                && job.origin.as_str() == venue
                && now_ic < job.accept_by_ic
        })
        .map(|job| job.contract)
        .min()
}

/// Whether contract `haul` is one of the arm's.
#[must_use]
pub fn is_arm_haul(w: &World, haul: u64) -> bool {
    arm(w).is_some_and(|a| a.posts.iter().any(|p| p.haul == haul))
}

/// **Write the arm's record**: RAM, then one `JsonSet`.
fn put(ctx: &mut Ctx<'_>, doc: &FreeportArm) {
    crate::store::put_freeport_arm(ctx, doc);
}

/// A credit account's available balance, minor units.
fn available(w: &World, account: AccountId) -> i64 {
    w.module_state::<mush_economy::EconomyStore>()
        .and_then(|e| e.snapshot().account(account).and_then(|a| a.available()))
        .map_or(0, |a| a.minor())
}

/// **Hold the Concession at the threshold** (this module's head): one segment an IC month the
/// reading stands at it. Answers whether the clock moved.
fn threshold_news(ctx: &mut Ctx<'_>, level: u64) -> bool {
    if level < STOCKPILE_THRESHOLD_T {
        return false;
    }
    let Some(clock) = crate::concession::hold_clock(ctx.w) else {
        return false;
    };
    if clock.completed {
        return false;
    }
    let month = crate::concession::ic_month(ctx.w.world_ic_now().at);
    let cause = format!("{THRESHOLD_CAUSE}{month}");
    if clock.causes.iter().any(|c| c == &cause) {
        return false;
    }
    let line = crate::concession::draft(
        ctx,
        "op.hold_concession",
        format!(
            "Occator's tank farm at Isidis stands at its stockpile threshold, {level} t of reaction mass; {} advances.",
            crate::concession::HOLD_THE_CONCESSION_TITLE
        ),
        cause,
    );
    let moved = matches!(
        mush_faction::advance_clock_by_cause(ctx, clock.id, 1, line),
        Ok(ClockAdvance::Advanced { .. })
    );
    if moved {
        ctx.out.pin_deferred();
    }
    moved
}

/// **The arm's fire** (on the port tick, after the hauling houses): the agent and its grant, one
/// step a fire; then every fire the reading and the news, and at most once an IC day a post.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let Some(occator) = org_of(ctx.w, ARM_ENTRY, ARM_ORG) else {
        return;
    };
    // The lane without the Concession (a stripped rig) has no tank farm, and no arm.
    if tank_farm_vault(ctx.w).is_none() {
        return;
    }
    let kept = arm(ctx.w).cloned();
    let mut doc = kept.clone().unwrap_or_default();
    // Every fire, laid up or not: the reading (kept when it changes) and the news. The close
    // probe reads it, so an operation's objective is answered in any world (doc 50 cut OP.5).
    if let Some(level) = read_level(ctx) {
        if doc.level != Some(level) {
            doc.level = Some(level);
            doc.read_day = Some(ctx.w.world_ic_now().at.div_euclid(IC_DAY));
        }
        let _ = threshold_news(ctx, level);
    }
    read_at_close(ctx, &mut doc);
    fund_fills(ctx, occator, &mut doc);
    if !doc.laid_up {
        step(ctx, occator, &mut doc);
    }
    if kept.as_ref() != Some(&doc) {
        put(ctx, &doc);
    }
}

fn step(ctx: &mut Ctx<'_>, occator: OrgId, doc: &mut FreeportArm) {
    // 1. The agent, standing at Ceres' exchange.
    let Some(agent) = doc.agent.map(ObjId) else {
        let Some(room) = arm_room(ctx.w) else {
            return;
        };
        let Ok(id) = CanonicalId::new(ARM_ORG) else {
            return;
        };
        match crate::port::site::dark_thing(ctx, &id, "Occator Isidis supply agent", room) {
            Ok(thing) => doc.agent = Some(thing),
            Err(error) => {
                tracing::warn!(%error, "tacsim: the Freeport arm's agent could not be made")
            }
        }
        return;
    };
    // 2. Its grant.
    if doc.grant.is_none() {
        match grant(ctx, occator, agent) {
            Ok(id) => doc.grant = Some(id),
            Err(error) => {
                tracing::warn!(%error, "tacsim: the Freeport arm's agent could not be granted")
            }
        }
        return;
    }
    let now = ctx.w.world_ic_now().at;
    let today = now.div_euclid(IC_DAY);
    // 3. Once an IC day: a post, while the farm is short. A refusal is looked at again on the
    //    next fire; only a post marks the day.
    if doc.posted_day == Some(today) {
        return;
    }
    if doc
        .level
        .is_some_and(|level| level >= STOCKPILE_THRESHOLD_T)
    {
        return;
    }
    let open = doc
        .posts
        .iter()
        .filter_map(|p| haul::haul_of(ctx.w, p.haul))
        .filter(|job| {
            matches!(
                job.stage,
                HaulStage::Posted | HaulStage::Taken | HaulStage::Aboard
            )
        })
        .count();
    if open >= ARM_MAX_OPEN {
        return;
    }
    let month = crate::concession::ic_month(now);
    let cost = (ARM_HAUL_FEE_CR + ARM_ESCORT_FEE_CR).saturating_mul(100);
    if !doc.within_cap(month, cost) {
        return;
    }
    let Some(account) = org_account(ctx.w, occator) else {
        return;
    };
    if available(ctx.w, account) < cost {
        return;
    }
    if let Some(post) = post(ctx, agent, occator, month) {
        doc.posted_day = Some(today);
        doc.posts.push(post);
        if doc.posts.len() > MAX_POSTS {
            let drop = doc.posts.len() - MAX_POSTS;
            doc.posts.drain(..drop);
        }
    }
}

/// **`@tacsim/port arm [on|off]`** (staff): the arm's state, or lay it up and open it again.
pub(crate) fn arm_cmd(ctx: &mut Ctx<'_>, set: Option<bool>) -> Vec<String> {
    let mut doc = arm(ctx.w).cloned().unwrap_or_default();
    if let Some(open) = set {
        doc.laid_up = !open;
        put(ctx, &doc);
    }
    let month = crate::concession::ic_month(ctx.w.world_ic_now().at);
    vec![format!(
        "Freeport arm: {}; the tank farm at {}, threshold {STOCKPILE_THRESHOLD_T} t; {} posts, \
         {} cr committed this IC month of {ARM_MONTHLY_CAP_CR} cr.",
        if doc.laid_up { "laid up" } else { "open" },
        doc.level
            .map_or_else(|| "no reading".to_owned(), |level| format!("{level} t")),
        doc.posts.len(),
        doc.spent_in(month) / 100,
    )]
}

/// The haul and its escort, posted as the agent. `None` when the haul did not post.
fn post(ctx: &mut Ctx<'_>, agent: ObjId, occator: OrgId, month: i64) -> Option<ArmPost> {
    let farm = org_of(ctx.w, TANK_FARM_ENTRY, TANK_FARM_ORG)?;
    let saved = ctx.actor;
    ctx.actor = agent;
    let line = format!(
        "{ARM_HAUL_T} {STOCKPILE_GOOD} to=isidis fee={ARM_HAUL_FEE_CR} as=#{} into=#{}",
        occator.0, farm.0
    );
    let posted = haul::post_as(ctx, &line);
    let result = match posted {
        Ok(job) => {
            let escort = crate::services::escort::post_as(
                ctx,
                &format!("haul={} fee={ARM_ESCORT_FEE_CR} as=#{}", job, occator.0),
            );
            if let Err(why) = &escort {
                tracing::info!(%why, "tacsim: the Freeport arm's escort did not post");
            }
            let escort = escort.ok();
            Some(ArmPost {
                haul: job,
                escort,
                month,
                spent_minor: ARM_HAUL_FEE_CR
                    .saturating_mul(100)
                    .saturating_add(escort.map_or(0, |_| ARM_ESCORT_FEE_CR.saturating_mul(100))),
            })
        }
        Err(why) => {
            tracing::debug!(%why, "tacsim: the Freeport arm's haul did not post");
            None
        }
    };
    ctx.actor = saved;
    result
}

/// The agent's room: Ceres' exchange.
fn arm_room(w: &World) -> Option<ObjId> {
    let (_, def, _) = crate::port::market::ports(w)
        .into_iter()
        .find(|(e, _, _)| e.as_str() == ARM_ENTRY)?;
    let reference = def.rooms.get(&CanonicalId::new(ARM_ROOM).ok()?)?;
    let key = crate::port::def::room_key(&def.area, reference).ok()?;
    crate::port::apply::room_by_key(w, &key)
}

/// An organization's credit account.
fn org_account(w: &World, org: OrgId) -> Option<AccountId> {
    let principal = mush_org::PrincipalId::Organization(org);
    w.module_state::<mush_economy::EconomyStore>()
        .and_then(|store| {
            store
                .snapshot()
                .accounts_of(&principal)
                .find(|a| {
                    a.instrument.as_str() == crate::economy::CREDIT
                        && a.status == mush_economy::AccountStatus::Active
                })
                .map(|a| a.id)
        })
}

/// The agent's grant of [`crate::services::JOBS_POST_CAPABILITY`] on Occator's root compartment.
fn grant(ctx: &mut Ctx<'_>, org: OrgId, agent: ObjId) -> Result<u64, String> {
    let root = ctx
        .w
        .module_state::<mush_org::OrgStore>()
        .and_then(|store| store.state().org(org).map(|o| o.root_scope))
        .ok_or_else(|| "Occator has no root compartment".to_owned())?;
    let id = crate::port::apply::alloc_grant_id(ctx)?;
    let plan = mush_org::OrgMutationPlan::of(vec![mush_org::OrgMutationOp::PutGrant {
        grant: mush_org::CapabilityGrant {
            id,
            org,
            grantee: agent,
            capability: mush_org::CapabilityId::new(crate::services::JOBS_POST_CAPABILITY)
                .ok_or_else(|| "not a capability id".to_owned())?,
            scope: mush_org::ScopeSelector::Exact(root),
            limit: None,
            delegable: false,
            valid_from: None,
            valid_until: None,
            revision: 1,
        },
    }]);
    mush_org::submit(
        ctx,
        &plan,
        mush_org::OrgAction {
            action: "tacsim.freeport_arm.agent".into(),
            org: Some(org),
            detail: serde_json::json!({ "agent": agent.0 }),
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(id.0)
}

#[cfg(test)]
mod tests {
    use super::{ARM_MONTHLY_CAP_CR, ArmFill, ArmPost, FreeportArm, fill_refused};

    fn post(month: i64, spent_minor: i64) -> ArmPost {
        ArmPost {
            haul: 1,
            escort: None,
            month,
            spent_minor,
        }
    }

    /// **The cap is a month's rows** (Q-50-22): posts of other months do not count, and a post
    /// that would take the month past the cap does not fit.
    #[test]
    fn op4_the_arms_cap_counts_its_own_months_rows() {
        let cap = ARM_MONTHLY_CAP_CR * 100;
        let mut arm = FreeportArm::default();
        arm.posts.push(post(3, cap - 100_000));
        arm.posts.push(post(2, cap));
        assert_eq!(arm.spent_in(3), cap - 100_000);
        assert!(arm.within_cap(3, 100_000), "exactly at the cap fits");
        assert!(!arm.within_cap(3, 100_001), "past it does not");
        assert!(arm.within_cap(4, cap), "a new month starts afresh");
    }

    /// **Doc 50b item 8: the tank farm's fills share the cap with the posts** (Q-50-22): a fill
    /// counts in its month, and a fill past the cap, or past what Occator holds, is refused.
    #[test]
    fn op4_the_farms_fills_count_in_the_arms_cap() {
        let cap = ARM_MONTHLY_CAP_CR * 100;
        let mut arm = FreeportArm::default();
        arm.posts.push(post(3, cap - 300_000));
        arm.fills.push(ArmFill {
            quote: 1,
            month: 3,
            spent_minor: 200_000,
            paid: true,
        });
        assert_eq!(arm.spent_in(3), cap - 100_000, "the fill is in the month");
        assert!(
            !fill_refused(Some(&arm), 3, 100_000, Some(cap)),
            "at the cap fits"
        );
        assert!(
            fill_refused(Some(&arm), 3, 100_001, Some(cap)),
            "past the cap is refused"
        );
        assert!(
            fill_refused(Some(&arm), 4, 100_000, Some(99_999)),
            "past Occator's money"
        );
        assert!(fill_refused(Some(&arm), 4, 1, None), "no account, no fill");
        assert!(
            !fill_refused(None, 4, 100_000, Some(100_000)),
            "no arm yet: the money decides"
        );
    }
}
