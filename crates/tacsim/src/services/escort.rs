//! **The escort** (doc 48 cut EX.6; Q-48-11, -46, -58; doc 48a A-4, doc 48b W-48-3a).
//!
//! A hull's title holder (or her helm) posts a job to be escorted through one journey: her next
//! passage from the port she lies at to a named port, or her next sortie in a named field. Another
//! player takes it with a hull or unit of theirs. The host that runs the journey writes the fact:
//!
//! - **Presence.** Each port-arm fire while the journey runs adds its IC seconds to the journey's
//!   time, and to the escort's near time when the escort is near the protected hull
//!   ([`near_in_space`], [`near_in_field`]). Near is, provisionally, the same passage (the same
//!   route, departed together) or the same bubble in space, and the same field in a sortie.
//! - **The linkdead grace.** While the escort's player has dropped and the grace has not run out
//!   ([`crate::space::tick::SPACE_LINKDEAD_GRACE_WALL_S`],
//!   [`crate::field::absent::FIELD_LINKDEAD_GRACE_WALL_S`]), the time counts as near.
//! - **The outcome.** When the protected hull arrives or withdraws, the escort is kept if the
//!   near time is at least [`NEAR_PERMILLE`] of the journey's time. It fails at once if the
//!   protected hull is wrecked or captured, or the escort leaves the journey.
//!
//! The fact (`tacsim.escort.kept`) is keyed by the job's contract, so it is named when the job
//! is posted (doc 48b W-48-3a). A kept escort's contract settles on the sweep: the fee and any
//! bond to the escort. A failed one never sees its fact; at the term's end the contract expires,
//! the fee back to the poster and the bond forfeit to them (Q-48-13).

use mush_core::{CmdError, Ctx, ObjId, World};
use mush_economy::{Amount, ContractId, ExternalRef, FactProvider, FactState};
use mush_market::MarketId;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::port::coop::money_minor;
use crate::port::owned::Held;
use crate::store::TacsimStore;

/// The verb.
pub const ESCORT_VERB: &str = "+escort";
/// The record's key prefix: `tacsim:escort:{contract}`.
pub const ESCORT_PREFIX: &str = "tacsim:escort:";
/// The record's schema.
pub const ESCORT_SCHEMA: u16 = 1;
/// The fact namespace (the composition registers [`EscortFactProvider`]).
pub const PROVIDER: &str = "tacsim.escort";
/// The fact: the escort was kept.
pub const FACT_KEPT: &str = "tacsim.escort.kept";
/// How long a posted escort waits for a taker, IC days (at least 28, the owner's rule).
pub const ACCEPT_WINDOW_IC_DAYS: i64 = 28;
/// **The escort's term from acceptance**, IC days (Q-48-58, provisional): the journey must
/// start and end within it.
pub const ESCORT_TERM_IC_DAYS: i64 = 28;
/// **How much of the journey the escort must be near** (Q-48-46, provisional): 90%.
pub const NEAR_PERMILLE: i64 = 900;
/// **Departed together** (provisional): two hulls on one route that left within this many IC
/// seconds of each other fly one passage. One IC hour, 150 real seconds at 24:1.
pub const TOGETHER_IC: i64 = 3_600;
/// IC seconds in an IC day.
const DAY: i64 = 86_400;
/// The channel lines go out on.
const CHANNEL: &str = "tacsim.escort";

// ---- the player's sentences -----------------------------------------------------------------

/// The usage line.
pub const ESCORT_USAGE: &str = "Usage: +escort  |  +escort/post <hull> to=<port>|field=<site> fee=<cr> [go]\n       \
+escort/accept <job> with=<hull> [as=<org>] [go]  |  +escort/withdraw <job> [go]";
/// The help.
pub const ESCORT_HELP: &str = "+escort                                your escorts, posted and taken\n\
+escort/post <hull> to=<port> fee=<cr> [go]\n\
                                       post a job to escort your hull on her next passage from\n\
                                       here to that port. The fee is escrowed now.\n\
+escort/post <hull> field=<site> fee=<cr> [go]\n\
                                       the same for her next sortie in that field.\n\
+escort/accept <job> with=<hull> [as=<org>] [go]\n\
                                       take an escort with a hull or rig of yours. Stay near\n\
                                       her for nine tenths of the journey: the same passage or\n\
                                       bubble in space, the same field in a sortie. A link drop\n\
                                       inside the grace does not count against you. The job\n\
                                       fails if she is wrecked or captured, or you leave.\n\
+escort/withdraw <job> [go]            withdraw an escort nobody has taken; the fee comes back.\n\
A fee over 10,000 cr asks the escort for a bond of a fifth of it.";
/// The post's preview (`{name}`, `{journey}`, `{fee}`, `{days}`).
pub const POST_PREVIEW: &str = "Post an escort of {name} on {journey} for {fee}; whoever takes it has \
{days} IC days. +escort/post ... go posts it.";
/// The post's line (`{job}`, `{name}`, `{journey}`, `{fee}`).
pub const POSTED: &str = "Escort {job} posted: {name} on {journey}, {fee} escrowed and paid when she \
is seen through.";
/// The refusal for a hull the poster does not hold.
pub const NOT_YOURS: &str = "You hold no title to that hull and do not fly her.";
/// The refusal for a passage from a hull not docked.
pub const NOT_DOCKED: &str = "She is not docked at a port, so there is no passage to escort yet.";
/// The refusal for an unknown port (`{port}`).
pub const NO_PORT: &str = "No port answers to {port}; she has no passage there.";
/// The refusal for a passage to the port she lies at.
pub const SAME_PORT: &str = "Her passage goes to another port.";
/// The refusal for no venue where the poster stands.
pub const NO_VENUE: &str = "There is no market here to post at.";
/// The refusal for a job already posted on her (`{job}`).
pub const ALREADY: &str = "An escort of her is open already ({job}).";
/// The economy's refusal (`{why}`).
pub const POST_FAILED: &str = "The escort could not be posted: {why}.";
/// The refusal for no such open job (`{job}`).
pub const NO_SUCH_JOB: &str = "There is no open escort {job}; +market/jobs escort lists them.";
/// The refusal for the poster's own job.
pub const OWN_JOB: &str = "That escort is your own.";
/// The refusal for the protected hull as her own escort.
pub const SAME_HULL: &str = "She cannot escort herself.";
/// The acceptance's preview (`{name}`, `{journey}`, `{fee}`, `{days}`).
pub const ACCEPT_PREVIEW: &str = "Take the escort: stay near {name} on {journey} within {days} IC days, \
for {fee}.";
/// The bond clause (`{bond}`).
pub const BOND_CLAUSE: &str = " Your bond of {bond} is escrowed: it comes back with the fee if she is seen \
through, and goes to the poster if not.";
/// The acceptance's line (`{job}`, `{name}`).
pub const ACCEPTED: &str = "You take escort {job}. Stay near {name} for the whole journey.";
/// The economy's refusal at acceptance (`{why}`).
pub const ACCEPT_FAILED: &str = "The escort could not be taken: {why}.";
/// The poster's notice (`{who}`, `{job}`).
pub const TOLD_TAKEN: &str = "{who} has taken escort {job}.";
/// The notice when the journey starts (`{job}`).
pub const TOLD_STARTED: &str = "Escort {job}: the journey has begun.";
/// The notice of a kept escort (`{job}`, `{near}`).
pub const TOLD_KEPT: &str =
    "Escort {job} is kept: near her for {near} of the journey. The fee follows.";
/// The notice of a failed escort (`{job}`, `{why}`).
pub const TOLD_FAILED: &str = "Escort {job} has failed, and its fee is not paid: {why}.";
/// Why: wrecked.
pub const WHY_WRECKED: &str = "she was wrecked";
/// Why: captured.
pub const WHY_CAPTURED: &str = "she was captured";
/// Why: the escort left.
pub const WHY_LEFT: &str = "the escort left the journey";
/// Why: too little of the journey near her (`{near}`).
pub const WHY_FAR: &str = "the escort was near her for only {near} of the journey";
/// The refusal for withdrawing a taken or another's job.
pub const NOT_WITHDRAWABLE: &str =
    "Only the poster withdraws an escort, and only before it is taken.";
/// The withdrawal's line (`{job}`).
pub const WITHDRAWN: &str = "Escort {job} is withdrawn; your fee is back in your account.";
/// No escorts.
pub const NONE: &str = "You have no escorts.";
/// The list head.
pub const HEAD: &str = "Your escorts:";
/// One list row (`{job}`, `{name}`, `{journey}`, `{stage}`).
pub const ROW: &str = "  {job}  {name}, escorted on {journey}: {stage}";

// ---- the record -----------------------------------------------------------------------------

/// **The journey an escort covers.**
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Journey {
    /// Her next passage from one port to another (venues).
    Passage {
        /// Where she lies.
        from: SmolStr,
        /// Where she is bound.
        to: SmolStr,
    },
    /// Her next sortie in one field (the site's id).
    Field {
        /// The site.
        site: SmolStr,
    },
}

/// **Where an escort stands.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EscortStage {
    /// Posted, on the board.
    Posted,
    /// Taken; waiting for her journey to begin.
    Taken,
    /// Her journey is running; the host watches.
    Watching,
    /// Seen through: the fact holds and the sweep pays.
    Kept,
    /// Failed: the fact never holds, and the term's end forfeits the bond.
    Failed,
    /// Done: paid, expired or withdrawn.
    Closed,
}

impl EscortStage {
    /// The list's word.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Posted => "posted, waiting for an escort",
            Self::Taken => "taken, waiting for her journey",
            Self::Watching => "under way, watched",
            Self::Kept => "kept",
            Self::Failed => "failed",
            Self::Closed => "closed",
        }
    }
}

/// **The host's watch** over the journey: its IC seconds, and the escort's near seconds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Watch {
    /// When the journey began, IC seconds.
    pub started_ic: i64,
    /// The last fire that counted, IC seconds.
    pub last_ic: i64,
    /// The journey's IC seconds so far.
    pub journey_ic: i64,
    /// The escort's near IC seconds so far (the linkdead grace included).
    pub near_ic: i64,
}

impl Watch {
    /// The near share, per-mille (0 for no journey).
    #[must_use]
    pub fn permille(&self) -> i64 {
        if self.journey_ic <= 0 {
            return 0;
        }
        self.near_ic.saturating_mul(1_000) / self.journey_ic
    }
}

/// **One escort** (doc 48 cut EX.6).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscortJob {
    /// [`ESCORT_SCHEMA`].
    pub schema: u16,
    /// The contract, `ContractId.0`: the job's number and the fact's key.
    pub contract: u64,
    /// The poster.
    pub poster: ObjId,
    /// The hull to be escorted.
    pub protected: ObjId,
    /// The journey.
    pub journey: Journey,
    /// The board it is posted on (a venue).
    pub venue: SmolStr,
    /// The fee, minor units.
    pub fee_minor: i64,
    /// The escort's bond, minor units (Q-48-13: 20% of a fee over 10,000 cr; else 0).
    pub bond_minor: i64,
    /// Posted, IC seconds.
    pub posted_ic: i64,
    /// The end of the wait for a taker.
    pub accept_by_ic: i64,
    /// Who took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escort: Option<PrincipalId>,
    /// The escort's hull or rig.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub with: Option<ObjId>,
    /// Taken, IC seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepted_ic: Option<i64>,
    /// The end of the term.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_ic: Option<i64>,
    /// The host's watch, once the journey began.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub watch: Option<Watch>,
    /// Why it failed, once it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<SmolStr>,
    /// Where it stands.
    pub stage: EscortStage,
}

impl EscortJob {
    /// The record's own rules.
    ///
    /// # Errors
    ///
    /// The rule broken.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != ESCORT_SCHEMA {
            return Err(format!("schema {} is not {ESCORT_SCHEMA}", self.schema));
        }
        if self.fee_minor <= 0 || self.bond_minor < 0 {
            return Err("an escort pays a positive fee".to_owned());
        }
        if let Journey::Passage { from, to } = &self.journey
            && from == to
        {
            return Err("a passage goes from one port to another".to_owned());
        }
        if self.with == Some(self.protected) {
            return Err("a hull does not escort herself".to_owned());
        }
        if self.accept_by_ic < self.posted_ic {
            return Err("the wait ends before the posting".to_owned());
        }
        Ok(())
    }

    /// The job's board word.
    #[must_use]
    pub fn job(&self) -> String {
        self.contract.to_string()
    }

    /// The record's key.
    #[must_use]
    pub fn key(&self) -> String {
        format!("{ESCORT_PREFIX}{}", self.contract)
    }
}

/// Every readable escort.
#[must_use]
pub fn escorts(w: &World) -> Vec<EscortJob> {
    super::book(w)
        .map(|book| {
            book.escorts
                .values()
                .filter_map(Held::present)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// The escort of `contract`, when it reads.
#[must_use]
pub fn escort_of(w: &World, contract: u64) -> Option<EscortJob> {
    super::book(w)?.escorts.get(&contract)?.present().cloned()
}

/// Install a record and queue its document.
pub(crate) fn put(ctx: &mut Ctx<'_>, job: &EscortJob) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store
            .services
            .escorts
            .insert(job.contract, Held::Present(Box::new(job.clone())));
    }
    crate::port::owned::write(ctx, job.key(), job);
}

/// The fact a job's contract reads.
fn fact_of(contract: u64) -> ExternalRef {
    ExternalRef {
        provider: PROVIDER.into(),
        kind: FACT_KEPT.into(),
        id: contract.to_string().into(),
        revision: Some(1),
    }
}

// ---- the fact -------------------------------------------------------------------------------

/// **The escort's fact provider** (`tacsim.escort`): [`FACT_KEPT`] holds once the host has
/// found the escort kept. Public: an escort is posted on a public board.
#[derive(Debug, Default)]
pub struct EscortFactProvider;

impl FactProvider for EscortFactProvider {
    fn provider(&self) -> &str {
        PROVIDER
    }

    fn resolve(&self, w: &World, kind: &str, id: &str, revision: Option<u64>) -> Option<FactState> {
        if kind != FACT_KEPT || revision != Some(1) {
            return None;
        }
        let job = escort_of(w, id.parse::<u64>().ok()?)?;
        (job.stage == EscortStage::Kept
            || (job.stage == EscortStage::Closed && job.why.is_none() && job.watch.is_some()))
        .then_some(FactState {
            satisfied: true,
            actor: match &job.escort {
                Some(PrincipalId::Object(who)) => Some(*who),
                _ => job.with,
            },
            at: 0,
        })
    }
}

// ---- the host's reading ---------------------------------------------------------------------

/// **Whether `escort` is near `protected` in space** (Q-48-46, provisional): both in one bubble,
/// or both on one passage: the same route, departed within [`TOGETHER_IC`], and neither held in
/// a bubble the other is not in.
#[must_use]
pub fn near_in_space(w: &World, protected: ObjId, escort: ObjId) -> bool {
    if let Some(book) = w.module_state::<crate::space::tick::SpaceBubbleBook>() {
        if let Some(bubble) = book.active_for_hull(protected) {
            return bubble.contains_carrier(escort)
                || book
                    .active_for_hull(escort)
                    .is_some_and(|other| other.domain == bubble.domain);
        }
        if book.active_for_hull(escort).is_some() {
            return false;
        }
    }
    let Some(journeys) = w.module_state::<mush_sim::JourneyBook>() else {
        return false;
    };
    let (Some(a), Some(b)) = (journeys.get(protected), journeys.get(escort)) else {
        return false;
    };
    a.route() == b.route() && (a.departed().at - b.departed().at).abs() <= TOGETHER_IC
}

/// **Whether `escort` is near `protected` in a field**: both deployed in the one site.
#[must_use]
pub fn near_in_field(w: &World, site: &str, escort: ObjId) -> bool {
    crate::field::field_of_unit(w, escort).is_some_and(|s| s.as_str() == site)
}

/// **Whether `escort`'s player has dropped and the grace still runs**: in space the linkdead
/// window not expired; in a field the unit away with its player's link lost under the grace.
#[must_use]
pub fn within_grace(w: &World, escort: ObjId) -> bool {
    if let Some(state) = w
        .module_state::<crate::space::tick::SpaceBubbleBook>()
        .and_then(|book| book.linkdead_state(escort))
    {
        return !state.grace_expired
            && w.now() - state.grace_started_wall_s
                < crate::space::tick::SPACE_LINKDEAD_GRACE_WALL_S;
    }
    crate::field::absent::away_of(w, escort).is_some_and(|away| {
        !away.handed
            && w.now() - away.since_wall_s < crate::field::absent::FIELD_LINKDEAD_GRACE_WALL_S
    })
}

/// Whether `hull` is wrecked: a derelict in space, or a wreck in a field.
#[must_use]
pub fn wrecked(w: &World, hull: ObjId) -> bool {
    w.module_state::<crate::space::tick::SpaceBubbleBook>()
        .is_some_and(|book| book.derelict_for_hull(hull).is_some())
        || crate::port::tv::wrecked(w, hull)
}

/// Whether `hull` is captured: yielded in a field (doc 42b's surrender).
#[must_use]
pub fn captured(w: &World, hull: ObjId) -> bool {
    crate::field::raiders::yielded(w, hull)
}

/// Whether the journey is running for `job`, and whether it has ended (arrived or withdrew).
fn journey_now(w: &World, job: &EscortJob) -> (bool, bool) {
    match &job.journey {
        Journey::Passage { from, .. } => {
            let under_way = w
                .module_state::<mush_sim::JourneyBook>()
                .is_some_and(|book| {
                    book.get(job.protected)
                        .is_some_and(|j| job.accepted_ic.is_some_and(|at| j.departed().at >= at))
                });
            let docked_away =
                super::docked_venue(w, job.protected).is_some_and(|v| v.as_str() != from.as_str());
            (under_way, docked_away)
        }
        Journey::Field { site } => {
            let inside = crate::field::field_of_unit(w, job.protected)
                .is_some_and(|s| s.as_str() == site.as_str());
            (inside, !inside)
        }
    }
}

/// Whether the escort is near, by the journey's rule.
fn near(w: &World, job: &EscortJob, escort: ObjId) -> bool {
    match &job.journey {
        Journey::Passage { .. } => near_in_space(w, job.protected, escort),
        Journey::Field { site } => near_in_field(w, site, escort),
    }
}

/// Whether the escort has left the journey: docked elsewhere while she is under way, or out of
/// the field while she is in it, and not inside the linkdead grace.
fn left(w: &World, job: &EscortJob, escort: ObjId) -> bool {
    if within_grace(w, escort) {
        return false;
    }
    match &job.journey {
        Journey::Passage { to, .. } => {
            super::docked_venue(w, escort).is_some_and(|v| v.as_str() != to.as_str())
        }
        Journey::Field { site } => !near_in_field(w, site, escort),
    }
}

// ---- the arm --------------------------------------------------------------------------------

/// **The host's watch** (doc 48 cut EX.6): each live escort's next step, at the port arm's fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let live: Vec<EscortJob> = escorts(ctx.w)
        .into_iter()
        .filter(|j| j.stage != EscortStage::Closed)
        .collect();
    for mut job in live {
        if step(ctx, &mut job) {
            put(ctx, &job);
        }
    }
}

fn tell_both(ctx: &mut Ctx<'_>, job: &EscortJob, line: &str) {
    super::tell(ctx, &PrincipalId::Object(job.poster), CHANNEL, line);
    if let Some(escort) = job.escort.clone() {
        super::tell(ctx, &escort, CHANNEL, line);
    }
}

fn fail(ctx: &mut Ctx<'_>, job: &mut EscortJob, why: &str) {
    job.stage = EscortStage::Failed;
    job.why = Some(SmolStr::new(why));
    let line = TOLD_FAILED
        .replace("{job}", &job.job())
        .replace("{why}", why);
    tell_both(ctx, job, &line);
}

fn percent(permille: i64) -> String {
    format!("{}%", permille / 10)
}

/// One job's step; `true` when the record changed.
fn step(ctx: &mut Ctx<'_>, job: &mut EscortJob) -> bool {
    let now = ctx.w.world_ic_now().at;
    let state = super::contract_state(ctx.w, job.contract);
    let live = state.as_ref().is_some_and(|s| s.is_live());
    match job.stage {
        EscortStage::Posted => {
            if now <= job.accept_by_ic && live {
                return false;
            }
            let _ = mush_economy::cancel_job(ctx, ContractId(job.contract));
            job.stage = EscortStage::Closed;
            true
        }
        EscortStage::Taken | EscortStage::Watching => watch(ctx, job, now, live),
        EscortStage::Kept | EscortStage::Failed => {
            if live {
                return false;
            }
            job.stage = EscortStage::Closed;
            true
        }
        EscortStage::Closed => false,
    }
}

/// The watch over a taken job.
fn watch(ctx: &mut Ctx<'_>, job: &mut EscortJob, now: i64, live: bool) -> bool {
    let Some(escort) = job.with else {
        job.stage = EscortStage::Closed;
        return true;
    };
    if !live || job.due_ic.is_some_and(|due| now >= due) {
        // The term ran out before the journey ended: the fact never holds.
        fail(
            ctx,
            job,
            WHY_FAR
                .replace(
                    "{near}",
                    &percent(job.watch.as_ref().map_or(0, Watch::permille)),
                )
                .as_str(),
        );
        return true;
    }
    if wrecked(ctx.w, job.protected) {
        fail(ctx, job, WHY_WRECKED);
        return true;
    }
    if captured(ctx.w, job.protected) {
        fail(ctx, job, WHY_CAPTURED);
        return true;
    }
    let (running, ended) = journey_now(ctx.w, job);
    if job.stage == EscortStage::Taken {
        if !running {
            return false;
        }
        job.stage = EscortStage::Watching;
        job.watch = Some(Watch {
            started_ic: now,
            last_ic: now,
            journey_ic: 0,
            near_ic: 0,
        });
        let line = TOLD_STARTED.replace("{job}", &job.job());
        tell_both(ctx, job, &line);
        return true;
    }
    let is_near = near(ctx.w, job, escort) || within_grace(ctx.w, escort);
    let left_it = !ended && left(ctx.w, job, escort);
    let Some(watch) = job.watch.as_mut() else {
        return false;
    };
    let dt = now.saturating_sub(watch.last_ic).max(0);
    watch.last_ic = now;
    watch.journey_ic = watch.journey_ic.saturating_add(dt);
    if is_near {
        watch.near_ic = watch.near_ic.saturating_add(dt);
    }
    let permille = watch.permille();
    if left_it {
        fail(ctx, job, WHY_LEFT);
        return true;
    }
    if !ended {
        return dt > 0;
    }
    if permille >= NEAR_PERMILLE {
        job.stage = EscortStage::Kept;
        let line = TOLD_KEPT
            .replace("{job}", &job.job())
            .replace("{near}", &percent(permille));
        tell_both(ctx, job, &line);
    } else {
        fail(
            ctx,
            job,
            WHY_FAR.replace("{near}", &percent(permille)).as_str(),
        );
    }
    true
}

// ---- the commands ---------------------------------------------------------------------------

/// **`+escort`'s spec**.
#[must_use]
pub(crate) fn escort_spec() -> mush_core::CommandSpec {
    mush_core::CommandSpec {
        name: ESCORT_VERB.into(),
        shape: mush_core::ArgShape::Raw,
        switches: ["post", "accept", "withdraw"]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
        help: ESCORT_HELP.into(),
        ..mush_core::CommandSpec::default()
    }
}

/// **`+escort`** (doc 48 cut EX.6).
///
/// # Errors
///
/// The refusal.
pub(crate) fn escort_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    let raw = ctx.args.raw().to_owned();
    let (rest, go) = crate::surface::retrieval::split_go(&raw);
    let line = if ctx.switches.has("post") {
        post_cmd(ctx, &rest, go)
    } else if ctx.switches.has("accept") {
        accept_cmd(ctx, &rest, go)
    } else if ctx.switches.has("withdraw") {
        withdraw_cmd(ctx, &rest, go)
    } else if rest.is_empty() {
        Ok(list(ctx))
    } else {
        Err(ESCORT_USAGE.to_owned())
    }
    .map_err(CmdError::msg)?;
    ctx.out.sys_to_actor(ctx.actor, CHANNEL, line);
    Ok(())
}

fn journey_words(w: &World, journey: &Journey) -> String {
    match journey {
        Journey::Passage { from, to } => format!(
            "the passage from {} to {}",
            super::venue_name(w, from),
            super::venue_name(w, to)
        ),
        Journey::Field { site } => format!("a sortie in {site}"),
    }
}

fn name(w: &World, hull: ObjId) -> String {
    w.get(hull)
        .map_or_else(|| format!("#{}", hull.0), |o| o.name.to_string())
}

fn list(ctx: &Ctx<'_>) -> String {
    let rows: Vec<String> = escorts(ctx.w)
        .into_iter()
        .filter(|j| j.stage != EscortStage::Closed)
        .filter(|j| {
            j.poster == ctx.actor
                || j.escort
                    .as_ref()
                    .is_some_and(|e| crate::surface::retrieval::acts_for(ctx.w, ctx.actor, e))
        })
        .map(|j| {
            ROW.replace("{job}", &j.job())
                .replace("{name}", &name(ctx.w, j.protected))
                .replace("{journey}", &journey_words(ctx.w, &j.journey))
                .replace("{stage}", j.stage.word())
        })
        .collect();
    if rows.is_empty() {
        return NONE.to_owned();
    }
    let mut out = vec![HEAD.to_owned()];
    out.extend(rows);
    out.join("\n")
}

/// A hull word: `#<dbref>`.
fn hull_word(word: &str) -> Option<ObjId> {
    word.trim()
        .strip_prefix('#')
        .and_then(|n| n.parse::<u32>().ok())
        .map(ObjId)
}

/// Whether `who` answers for `hull`: her title, her helm, a live lease of the rig, or the seat.
fn holds(w: &World, who: ObjId, hull: ObjId) -> bool {
    crate::port::coop::helm_of(w, who).is_some_and(|h| h.hull == hull)
        || w.module_state::<mush_economy::EconomyStore>()
            .and_then(|store| store.snapshot().asset(hull))
            .is_some_and(|asset| asset.title_owner == PrincipalId::Object(who))
        || w.module_state::<TacsimStore>().is_some_and(|store| {
            store
                .tv_leases_of(who)
                .any(|lease| lease.live() && lease.tv == hull)
        })
        || crate::field::crew::seated_in(w, hull) == Some(who)
}

fn post_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let (words, opts) = super::options(rest);
    let [word] = words.as_slice() else {
        return Err(ESCORT_USAGE.to_owned());
    };
    let hull = hull_word(word).ok_or_else(|| ESCORT_USAGE.to_owned())?;
    if !holds(ctx.w, ctx.actor, hull) {
        return Err(NOT_YOURS.to_owned());
    }
    let fee = opts
        .get("fee")
        .and_then(|f| super::credits(f))
        .ok_or_else(|| ESCORT_USAGE.to_owned())?;
    // The board: where the poster stands, else the port their own hull lies at (a field's
    // pad is a port entry with a venue of its own).
    let venue = super::standing_venue(ctx.w, ctx.actor)
        .or_else(|| {
            crate::port::coop::helm_of(ctx.w, ctx.actor)
                .and_then(|h| super::docked_venue(ctx.w, h.hull))
        })
        .ok_or_else(|| NO_VENUE.to_owned())?;
    let journey = match (opts.get("to"), opts.get("field")) {
        (Some(to), None) => {
            let from = super::docked_venue(ctx.w, hull).ok_or_else(|| NOT_DOCKED.to_owned())?;
            let to = super::venue_word(ctx.w, to).ok_or_else(|| NO_PORT.replace("{port}", to))?;
            if from == to {
                return Err(SAME_PORT.to_owned());
            }
            Journey::Passage {
                from: SmolStr::new(from.as_str()),
                to: SmolStr::new(to.as_str()),
            }
        }
        (None, Some(site)) => Journey::Field {
            site: SmolStr::new(
                super::entry_word(ctx.w, site).ok_or_else(|| NO_PORT.replace("{port}", site))?,
            ),
        },
        _ => return Err(ESCORT_USAGE.to_owned()),
    };
    if let Some(open) = escorts(ctx.w).into_iter().find(|j| {
        j.protected == hull
            && matches!(
                j.stage,
                EscortStage::Posted | EscortStage::Taken | EscortStage::Watching
            )
    }) {
        return Err(ALREADY.replace("{job}", &open.job()));
    }
    let journey_line = journey_words(ctx.w, &journey);
    if !go {
        return Ok(POST_PREVIEW
            .replace("{name}", &name(ctx.w, hull))
            .replace("{journey}", &journey_line)
            .replace("{fee}", &money_minor(ctx, fee))
            .replace("{days}", &ESCORT_TERM_IC_DAYS.to_string()));
    }
    let account = crate::port::credit::own_account(ctx.w, ctx.actor)
        .map(|a| a.id)
        .ok_or_else(|| "You have no account to pay the fee from.".to_owned())?;
    let peeked = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .map(mush_economy::EconomyStore::peek_next_contract)
        .ok_or_else(|| "The economy is not running.".to_owned())?;
    let now = ctx.w.world_ic_now().at;
    let window = ACCEPT_WINDOW_IC_DAYS.saturating_mul(DAY);
    let term = ESCORT_TERM_IC_DAYS.saturating_mul(DAY);
    let contract = mush_economy::post_job(
        ctx,
        mush_economy::JobSpec {
            name: SmolStr::new(format!("Escort: {}", name(ctx.w, hull))),
            poster: PrincipalId::Object(ctx.actor),
            poster_account: account,
            fee: Amount::from_minor(fee),
            fact: fact_of(peeked),
            deadline_ic: now.saturating_add(window).saturating_add(term),
        },
    )
    .map_err(|error| POST_FAILED.replace("{why}", &error.to_string()))?;
    if contract.0 != peeked {
        let _ = mush_economy::cancel_job(ctx, contract);
        return Err(POST_FAILED.replace("{why}", "the economy was busy; try again"));
    }
    let job = EscortJob {
        schema: ESCORT_SCHEMA,
        contract: contract.0,
        poster: ctx.actor,
        protected: hull,
        journey,
        venue: SmolStr::new(venue.as_str()),
        fee_minor: fee,
        bond_minor: mush_economy::job_bond(Amount::from_minor(fee)).map_or(0, |b| b.minor()),
        posted_ic: now,
        accept_by_ic: now.saturating_add(window),
        escort: None,
        with: None,
        accepted_ic: None,
        due_ic: None,
        watch: None,
        why: None,
        stage: EscortStage::Posted,
    };
    put(ctx, &job);
    Ok(POSTED
        .replace("{job}", &job.job())
        .replace("{name}", &name(ctx.w, hull))
        .replace("{journey}", &journey_line)
        .replace("{fee}", &money_minor(ctx, fee)))
}

fn accept_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let (words, opts) = super::options(rest);
    let [word] = words.as_slice() else {
        return Err(ESCORT_USAGE.to_owned());
    };
    let mut job = word
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .and_then(|c| escort_of(ctx.w, c))
        .filter(|j| j.stage == EscortStage::Posted)
        .ok_or_else(|| NO_SUCH_JOB.replace("{job}", word))?;
    if job.poster == ctx.actor {
        return Err(OWN_JOB.to_owned());
    }
    let with = opts
        .get("with")
        .and_then(|w| hull_word(w))
        .ok_or_else(|| ESCORT_USAGE.to_owned())?;
    if with == job.protected {
        return Err(SAME_HULL.to_owned());
    }
    if !holds(ctx.w, ctx.actor, with) {
        return Err(NOT_YOURS.to_owned());
    }
    let bond = if job.bond_minor > 0 {
        BOND_CLAUSE.replace("{bond}", &money_minor(ctx, job.bond_minor))
    } else {
        String::new()
    };
    if !go {
        let mut line = ACCEPT_PREVIEW
            .replace("{name}", &name(ctx.w, job.protected))
            .replace("{journey}", &journey_words(ctx.w, &job.journey))
            .replace("{fee}", &money_minor(ctx, job.fee_minor))
            .replace("{days}", &ESCORT_TERM_IC_DAYS.to_string());
        line.push_str(&bond);
        return Ok(line);
    }
    let (taker, account) = super::taker(ctx, opts.get("as").map(String::as_str))?;
    if taker == PrincipalId::Object(job.poster) {
        return Err(OWN_JOB.to_owned());
    }
    let term = ESCORT_TERM_IC_DAYS.saturating_mul(DAY);
    mush_economy::join_job(
        ctx,
        ContractId(job.contract),
        mush_economy::JoinTerms {
            acceptor: taker.clone(),
            account,
            bond: (job.bond_minor > 0).then(|| Amount::from_minor(job.bond_minor)),
            term_ic: Some(term),
        },
    )
    .map_err(|error| ACCEPT_FAILED.replace("{why}", &error.to_string()))?;
    let now = ctx.w.world_ic_now().at;
    job.escort = Some(taker.clone());
    job.with = Some(with);
    job.accepted_ic = Some(now);
    job.due_ic = Some(now.saturating_add(term));
    job.stage = EscortStage::Taken;
    put(ctx, &job);
    let told = TOLD_TAKEN
        .replace(
            "{who}",
            &crate::surface::retrieval::principal_name(ctx.w, &taker),
        )
        .replace("{job}", &job.job());
    super::tell(ctx, &PrincipalId::Object(job.poster), CHANNEL, &told);
    let mut line = ACCEPTED
        .replace("{job}", &job.job())
        .replace("{name}", &name(ctx.w, job.protected));
    line.push_str(&bond);
    Ok(line)
}

fn withdraw_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let word = rest.trim();
    let mut job = word
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .and_then(|c| escort_of(ctx.w, c))
        .filter(|j| j.poster == ctx.actor && j.stage == EscortStage::Posted)
        .ok_or_else(|| NOT_WITHDRAWABLE.to_owned())?;
    if !go {
        return Ok(format!(
            "{} +escort/withdraw {} go withdraws it.",
            EscortStage::Posted.word(),
            job.job()
        ));
    }
    mush_economy::cancel_job(ctx, ContractId(job.contract))
        .map_err(|error| POST_FAILED.replace("{why}", &error.to_string()))?;
    job.stage = EscortStage::Closed;
    put(ctx, &job);
    Ok(WITHDRAWN.replace("{job}", &job.job()))
}

// ---- the board ------------------------------------------------------------------------------

/// **The escorts on `venue`'s board** (Q-48-12): each posted job, where it was posted.
#[must_use]
pub fn services(w: &World, venue: &MarketId) -> Vec<mush_market::Service> {
    let instrument = mush_economy::InstrumentId::new(crate::economy::CREDIT);
    escorts(w)
        .into_iter()
        .filter(|j| j.stage == EscortStage::Posted && j.venue.as_str() == venue.as_str())
        .map(|j| mush_market::Service {
            kind: mush_market::ServiceKind::Escort,
            side: mush_market::ServiceSide::Wanted,
            venue: venue.clone(),
            poster: PrincipalId::Object(j.poster),
            poster_name: w
                .get(j.poster)
                .map_or_else(|| SmolStr::new("someone"), |o| o.name.clone().into()),
            subject: format!(
                "escort {} on {}",
                name(w, j.protected),
                journey_words(w, &j.journey)
            )
            .into(),
            pay: Amount::from_minor(j.fee_minor),
            instrument: instrument.clone(),
            negotiable: false,
            bond: (j.bond_minor > 0).then(|| Amount::from_minor(j.bond_minor)),
            term: mush_market::ServiceTerm::FromAcceptance {
                ic_secs: ESCORT_TERM_IC_DAYS.saturating_mul(DAY),
            },
            contract: Some(ContractId(j.contract)),
            take: format!("+escort/accept {} with=<hull>", j.job()).into(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 48 cut EX.6**: the near share is per-mille of the journey, and 90% keeps it.
    #[test]
    fn ex6_the_near_share_is_of_the_journey() {
        let watch = Watch {
            started_ic: 0,
            last_ic: 100,
            journey_ic: 1_000,
            near_ic: 900,
        };
        assert_eq!(watch.permille(), 900);
        assert!(watch.permille() >= NEAR_PERMILLE);
        assert_eq!(Watch::default().permille(), 0, "no journey, no share");
        assert_eq!(percent(899), "89%");
    }
}
