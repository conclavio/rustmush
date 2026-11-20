//! **The Isidis stockpile and the Freeport arm** (doc 50 cut OP.4; Q-50-21, Q-50-22, Q-50-24,
//! Q-50-60).
//!
//! - **The stockpile** (Q-50-21, Q-50-58): what Occator's tank farm at Isidis holds of
//!   [`STOCKPILE_GOOD`] in its storefront vault, against [`STOCKPILE_THRESHOLD_T`]. The port tick
//!   reads the level every fire and keeps it on the arm's record ([`FreeportArm::level`]); the
//!   operation's close probe ([`TANK_FARM_PROBE`], [`close_probe`]) reads that record at the
//!   window's close, so a level is safe across a restart and nothing reads canon.
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
use mush_org::OrgId;
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
    /// **Laid up by staff** (`@tacsim/port arm off`): it reads nothing and posts nothing; what it
    /// posted runs on. A rig whose stories predate OP.4 lays it up before its first fire, so it
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
    /// What the arm committed in IC month `month`, minor units.
    #[must_use]
    pub fn spent_in(&self, month: i64) -> i64 {
        self.posts
            .iter()
            .filter(|p| p.month == month)
            .map(|p| p.spent_minor)
            .sum()
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

/// **The close probe** (`CLOSE_PROBES`): whether the tank farm's last reading stands at its
/// threshold. `None` before any reading.
#[must_use]
pub fn close_probe(w: &World, _operation: u64) -> Option<bool> {
    arm(w)?.level.map(|level| level >= STOCKPILE_THRESHOLD_T)
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
    if doc.laid_up {
        return;
    }
    step(ctx, occator, &mut doc);
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
    // 3. Every fire: the reading (kept when it changes) and the news.
    let now = ctx.w.world_ic_now().at;
    let today = now.div_euclid(IC_DAY);
    if let Some(level) = read_level(ctx) {
        if doc.level != Some(level) {
            doc.level = Some(level);
            doc.read_day = Some(today);
        }
        let _ = threshold_news(ctx, level);
    }
    // 4. Once an IC day: a post, while the farm is short. A refusal is looked at again on the
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
    use super::{ARM_MONTHLY_CAP_CR, ArmPost, FreeportArm};

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
}
