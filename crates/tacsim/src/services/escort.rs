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
//! **For a haul, and for an organization** (doc 50 cut OP.3; Q-50-23, Q-50-63):
//! - `+escort/post haul=<job> fee=<cr>` posts an escort for a haul job, by its shipper (or, with
//!   `as=<org>`, for the organization that posted the haul). It binds to the hauler's hull when
//!   the haul is taken ([`UNBOUND`] until then), and covers the haul's passage from its origin to
//!   its destination across every hop. A haul that closes untaken calls the escort off.
//! - `as=<org>` posts for an organization whose root compartment grants the poster
//!   [`super::JOBS_POST_CAPABILITY`], the fee from its account; a hull escort `as=<org>` is of a
//!   hull the organization holds the title to.
//! - **A passage ends at its destination** (doc 50a A-8b): docking at a port on the way is a hop,
//!   not the end, and the time docked between hops counts to neither total. The escort may still
//!   be docked for [`TOGETHER_IC`] IC seconds or [`LAUNCH_GRACE_WALL_S`] real seconds after the
//!   protected hull launches from a port, whichever is longer, before it has left the journey.
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
/// **How far the contract's term runs past the watch's** (48e services:G2), IC seconds: one IC
/// day. The host decides a taken escort at [`ESCORT_TERM_IC_DAYS`] (calls off one that never
/// started, fails one that did) before the economy's expiry can forfeit the bond on its own.
pub const DECIDE_LEAD_IC: i64 = DAY;
/// **The real seconds an escort may still lie docked after the protected hull launches**
/// (doc 50 cut OP.3, Q-50-63's provisional grace): a dozen port fires, so a high clock rate
/// cannot turn the one fire between two undocks into a departure.
pub const LAUNCH_GRACE_WALL_S: i64 = 60;
/// **Maximum cumulative linkdead grace IC seconds credited over one escort contract**: two IC
/// hours ([`crate::space::tick::SPACE_LINKDEAD_GRACE_WALL_S`] at 24:1), bounding cumulative
/// grace even under accelerated clock rates or disconnect/reconnect cycling.
pub const LINKDEAD_GRACE_IC_S: i64 = 7_200;

/// Whether `n` is zero (for serde `skip_serializing_if`).
#[allow(clippy::trivially_copy_pass_by_ref)]
const fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

/// **The departure gap that still counts as flying together** (doc 50b, P3): [`TOGETHER_IC`], or the
/// IC seconds [`LAUNCH_GRACE_WALL_S`] real seconds make at the world's clock rate, whichever is
/// longer. At 24:1 it is [`TOGETHER_IC`]; at a test rate the launch grace carries through to the
/// proximity test, so an escort that set out inside the grace is near on the passage too.
#[must_use]
pub fn together_ic(w: &World) -> i64 {
    together_ic_at(w.world_timeline().current().rate)
}

/// [`together_ic`] at clock rate `rate`.
#[must_use]
pub fn together_ic_at(rate: u32) -> i64 {
    TOGETHER_IC.max(i64::from(rate.max(1)).saturating_mul(LAUNCH_GRACE_WALL_S))
}
/// **The protected hull of an escort posted for a haul not yet taken** (doc 50 cut OP.3): no
/// hull is Limbo.
pub const UNBOUND: ObjId = ObjId(0);
/// Most open (non-closed) escorts one actor may post concurrently.
pub const MAX_OPEN_ESCORTS_PER_ACTOR: usize = 16;
/// Most open (non-closed) escorts across the world concurrently.
pub const MAX_OPEN_ESCORTS_GLOBAL: usize = 512;
/// Most recently-closed escorts retained before oldest closed records are deleted.
pub const MAX_CLOSED_ESCORTS: usize = 64;
/// IC seconds in an IC day.
const DAY: i64 = 86_400;
/// The channel lines go out on.
const CHANNEL: &str = "tacsim.escort";

// ---- the player's sentences -----------------------------------------------------------------

/// The usage line.
pub const ESCORT_USAGE: &str = "Usage: +escort  |  +escort/post <hull> to=<port>|field=<site> fee=<cr> [as=<org>] [go]  |  +escort/post haul=<job> fee=<cr> [as=<org>] [go]\n       \
+escort/accept <job> with=<hull> [as=<org>] [go]  |  +escort/withdraw <job> [go]";
/// The help.
pub const ESCORT_HELP: &str = "+escort                                your escorts, posted and taken\n\
+escort/post <hull> to=<port> fee=<cr> [go]\n\
                                       post a job to escort your hull on her next passage from\n\
                                       here to that port. The fee is escrowed now.\n\
+escort/post <hull> field=<site> fee=<cr> [go]\n\
                                       the same for her next sortie in that field.\n\
+escort/post haul=<job> fee=<cr> [go] an escort for a haul you posted: it binds to the\n\
                                       hauler's hull when the haul is taken, and runs to the\n\
                                       haul's destination across every stop.\n\
+escort/post ... as=<org> [go]        post for an organization you may post jobs for, the\n\
                                       fee from its account.\n\
+escort/accept <job> with=<hull> [as=<org>] [go]\n\
                                       take an escort with a hull or rig of yours. Stay near\n\
                                       her for nine tenths of the journey: the same passage or\n\
                                       bubble in space, the same field in a sortie. A link drop\n\
                                       inside the grace does not count against you. The job\n\
                                       fails if she is wrecked or captured, or you leave.\n\
                                       If her journey has not begun in 28 IC days, the job is\n\
                                       called off and the fee and any bond come back.\n\
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
/// The refusal when the poster already has the maximum number of open escorts (`{cap}`).
pub const TOO_MANY_OPEN: &str =
    "You already have {cap} open escorts; finish or withdraw one first.";
/// The refusal when the world's open escort board is full.
pub const BOARD_FULL: &str = "The escort board is full right now; try again later.";
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
/// The notice of an escort called off because she never set out within the term (`{job}`,
/// `{days}`): the fee and the escort's bond go back (48e services:G2).
pub const TOLD_UNSTARTED: &str = "Escort {job} is called off: her journey never began within {days} IC      days. The fee and any bond are back in the accounts they came from.";
/// The refusal for `field=` at a port with no field (48e services:G2; `{port}`).
pub const NO_FIELD: &str = "There is no field at {port}; a sortie escort names a field site.";
/// No haul to escort (doc 50 cut OP.3).
pub const NO_HAUL: &str = "There is no haul {job} of yours still on its way to escort.";
/// The line both read when the haul an escort was posted for closes untaken (doc 50 cut OP.3).
pub const TOLD_HAUL_GONE: &str = "Escort {job} is called off: the haul it was posted for closed before a hull took it. The fee and any bond are back in the accounts they came from.";
/// Told both when the haul is taken in the escort's own hull (doc 50b): `{job}`.
pub const TOLD_SAME_HULL: &str = "Escort {job} is called off: the haul it was posted for was taken in the escort's own hull, and a hull does not escort herself. The fee and any bond are back in the accounts they came from.";
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
    /// **When the protected hull last launched** (doc 50 cut OP.3), IC seconds: the watch's
    /// start, then each departure from a port on the way.
    #[serde(default)]
    pub launch_ic: i64,
    /// The same instant, real seconds.
    #[serde(default)]
    pub launch_wall: i64,
    /// Whether the protected hull lay docked at a port on the way at the last fire.
    #[serde(default)]
    pub between: bool,
    /// Whether the escort had verified same-mode proximity (`near == true`) on the current leg or
    /// sortie before entering linkdead grace.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub was_near: bool,
    /// The last real wall second (`World::now`) this watch stepped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_wall: Option<i64>,
    /// Cumulative real wall seconds of linkdead grace consumed over this escort contract
    /// (persisted on the escort record and never reset when a player reconnects).
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub grace_wall_s: i64,
    /// Cumulative IC seconds of linkdead grace consumed over this escort contract
    /// (persisted on the escort record and never reset when a player reconnects).
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub grace_ic: i64,
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
    /// **The organization that posted it** (doc 50 cut OP.3), `OrgId.0`, when it was posted
    /// `as=<org>`; `poster` is then the member who posted it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poster_org: Option<u64>,
    /// **The haul it was posted for** (doc 50 cut OP.3, Q-50-63), `ContractId.0`: `protected`
    /// is [`UNBOUND`] until the haul is taken, then the hauler's hull.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub haul: Option<u64>,
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
        if let Some(watch) = &self.watch
            && (watch.grace_wall_s < 0 || watch.grace_ic < 0)
        {
            return Err("cumulative grace cannot be negative".to_owned());
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

/// Every readable non-closed escort, by reference (O(live), no cloning).
pub(crate) fn live_escorts(w: &World) -> impl Iterator<Item = &EscortJob> {
    super::book(w).into_iter().flat_map(|book| {
        book.live_escorts.iter().filter_map(|id| {
            book.escorts
                .get(id)
                .and_then(Held::present)
                .filter(|job| job.stage != EscortStage::Closed)
        })
    })
}

/// The escort of `contract`, when it reads.
#[must_use]
pub fn escort_of(w: &World, contract: u64) -> Option<EscortJob> {
    super::book(w)?.escorts.get(&contract)?.present().cloned()
}

/// The record key of escort `contract`.
#[must_use]
pub fn escort_key(contract: u64) -> String {
    format!("{ESCORT_PREFIX}{contract}")
}

/// Delete an escort record from RAM and persistent storage.
pub(crate) fn delete(ctx: &mut Ctx<'_>, contract: u64) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.services.escorts.remove(&contract);
        store.services.live_escorts.remove(&contract);
        store.services.closed_escorts.remove(&contract);
    }
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::Del {
            key: escort_key(contract).into(),
        },
    );
}

/// Delete oldest retained `Closed` escorts once closed count exceeds [`MAX_CLOSED_ESCORTS`].
pub(crate) fn prune_closed(ctx: &mut Ctx<'_>) {
    let excess: Vec<u64> = ctx
        .w
        .module_state::<TacsimStore>()
        .map(|store| {
            let over = store
                .services
                .closed_escorts
                .len()
                .saturating_sub(MAX_CLOSED_ESCORTS);
            store
                .services
                .closed_escorts
                .iter()
                .take(over)
                .copied()
                .collect()
        })
        .unwrap_or_default();
    for contract in excess {
        delete(ctx, contract);
    }
}

/// Install a record and queue its document.
pub(crate) fn put(ctx: &mut Ctx<'_>, job: &EscortJob) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        if job.stage == EscortStage::Closed {
            store.services.live_escorts.remove(&job.contract);
            store.services.closed_escorts.insert(job.contract);
        } else {
            store.services.closed_escorts.remove(&job.contract);
            store.services.live_escorts.insert(job.contract);
        }
        store
            .services
            .escorts
            .insert(job.contract, Held::Present(Box::new(job.clone())));
    }
    crate::port::owned::write(ctx, job.key(), job);
    if job.stage == EscortStage::Closed {
        prune_closed(ctx);
    }
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

/// **Whether `escort` is near `protected` in space** (Q-48-46, provisional): `escort` is a
/// space-capable ship hull, and both are in one bubble, or both on one passage (the same route,
/// departed within [`together_ic`], and neither held in a bubble the other is not in).
#[must_use]
pub fn near_in_space(w: &World, protected: ObjId, escort: ObjId) -> bool {
    if crate::port::ship::hull_view(w, escort).is_none() {
        return false;
    }
    if let Some(book) = w.module_state::<crate::space::tick::SpaceBubbleBook>() {
        if let Some(bubble) = book.active_for_hull(protected) {
            return bubble.contains_carrier(escort)
                || book
                    .active_for_hull(escort)
                    .is_some_and(|other| other.domain == bubble.domain);
        }
        if book.active_for_hull(escort).is_some()
            || book.free_flights.contains_key(&escort)
            || book.cluster_sessions.contains_key(&escort)
        {
            return false;
        }
    }
    let Some(journeys) = w.module_state::<mush_sim::JourneyBook>() else {
        return false;
    };
    let (Some(a), Some(b)) = (journeys.get(protected), journeys.get(escort)) else {
        return false;
    };
    matches!(
        b.state(),
        mush_sim::JourneyState::Cruising | mush_sim::JourneyState::Arrived
    ) && a.route() == b.route()
        && (a.departed().at - b.departed().at).abs() <= together_ic(w)
}

/// **Whether `escort` is near `protected` in a field**: both deployed in the one site.
#[must_use]
pub fn near_in_field(w: &World, site: &str, escort: ObjId) -> bool {
    crate::field::field_of_unit(w, escort).is_some_and(|s| s.as_str() == site)
}

/// Cumulative real wall seconds of linkdead grace permitted for `journey` over the whole contract.
const fn max_grace_wall_s(journey: &Journey) -> i64 {
    match journey {
        Journey::Passage { .. } => crate::space::tick::SPACE_LINKDEAD_GRACE_WALL_S,
        Journey::Field { .. } => crate::field::absent::FIELD_LINKDEAD_GRACE_WALL_S,
    }
}

/// The wall second when `escort`'s current same-mode linkdead window began, if `escort` matches
/// `journey`'s mode and has an active, unexpired linkdead grace state for that mode.
fn raw_grace_started_wall(w: &World, journey: &Journey, escort: ObjId) -> Option<i64> {
    match journey {
        Journey::Passage { .. } => {
            // A ground rig or non-ship vehicle can never satisfy space passage escort grace.
            crate::port::ship::hull_view(w, escort)?;
            let book = w.module_state::<crate::space::tick::SpaceBubbleBook>()?;
            // Only hulls actually cruising on a passage or inside an active space bubble qualify
            // (never hulls parked in decoupled free-flight or asteroid cluster sessions, whose
            // paused `JourneyState::Promoted` record still sits in `JourneyBook`).
            if book.free_flights.contains_key(&escort)
                || book.cluster_sessions.contains_key(&escort)
            {
                return None;
            }
            let on_passage = w
                .module_state::<mush_sim::JourneyBook>()
                .and_then(|jb| jb.get(escort))
                .is_some_and(|j| matches!(j.state(), mush_sim::JourneyState::Cruising));
            let in_bubble = book.active_for_hull(escort).is_some();
            if !on_passage && !in_bubble {
                return None;
            }
            let state = book.linkdead_state(escort)?;
            (!state.grace_expired
                && w.now().saturating_sub(state.grace_started_wall_s)
                    < crate::space::tick::SPACE_LINKDEAD_GRACE_WALL_S)
                .then_some(state.grace_started_wall_s)
        }
        Journey::Field { site } => {
            if !near_in_field(w, site, escort) {
                return None;
            }
            let away = crate::field::absent::away_of(w, escort)?;
            (!away.handed
                && w.now().saturating_sub(away.since_wall_s)
                    < crate::field::absent::FIELD_LINKDEAD_GRACE_WALL_S)
                .then_some(away.since_wall_s)
        }
    }
}

/// **Whether `escort`'s player has dropped and the contract's mode-scoped linkdead grace still
/// runs**: requires matching journey mode, prior verified proximity (`Watch::was_near`), active
/// unexpired linkdead state, and remaining cumulative contract grace budget.
#[must_use]
pub fn within_grace(w: &World, job: &EscortJob, escort: ObjId) -> bool {
    let Some(watch) = job.watch.as_ref() else {
        return false;
    };
    watch.was_near
        && watch.grace_wall_s < max_grace_wall_s(&job.journey)
        && watch.grace_ic < LINKDEAD_GRACE_IC_S
        && raw_grace_started_wall(w, &job.journey, escort).is_some()
}

/// Whether `escort` matches the asset class required for `journey`.
fn escort_matches_journey(w: &World, journey: &Journey, escort: ObjId) -> Result<(), &'static str> {
    match journey {
        Journey::Passage { .. } => crate::port::ship::hull_view(w, escort)
            .map(|_| ())
            .ok_or("a passage escort takes a ship hull"),
        Journey::Field { .. } => mush_tactics::module::unit_of(w, escort)
            .map(|_| ())
            .ok_or("a field escort takes a combat unit or rig"),
    }
}

/// Whether the haul an escort was posted for has closed or entered pre-delivery return before the
/// escort's journey began.
fn haul_cancelled_before_journey(w: &World, contract: u64) -> bool {
    super::haul::haul_of(w, contract).is_none_or(|h| {
        matches!(
            h.stage,
            super::haul::HaulStage::Closed | super::haul::HaulStage::Returning
        )
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
        Journey::Passage { to, .. } => {
            // Doc 50 cut OP.3 (doc 50a A-8b): under way is off any berth on a journey begun
            // since the escort was taken; the end is the destination, not the first port.
            let docked = super::docked_venue(w, job.protected);
            let under_way = docked.is_none()
                && w.module_state::<mush_sim::JourneyBook>()
                    .is_some_and(|book| {
                        book.get(job.protected).is_some_and(|j| {
                            job.accepted_ic.is_some_and(|at| j.departed().at >= at)
                        })
                    });
            let arrived = docked.is_some_and(|v| v.as_str() == to.as_str());
            (under_way, arrived)
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

/// Whether the escort has left the journey, disregarding linkdead grace.
fn left_without_grace(w: &World, job: &EscortJob, escort: ObjId) -> bool {
    match &job.journey {
        Journey::Passage { to, .. } => {
            // Still docked inside the departure gap is not leaving (48e services:G3; doc 50 cut
            // OP.3): `near_in_space` counts a passage flown together within [`together_ic`] of
            // hers, and a high clock rate is allowed [`LAUNCH_GRACE_WALL_S`] real seconds.
            let now = w.world_ic_now().at;
            let wall = w.now();
            let setting_out = job.watch.as_ref().is_some_and(|watch| {
                now.saturating_sub(watch.launch_ic) <= together_ic(w)
                    || wall.saturating_sub(watch.launch_wall) <= LAUNCH_GRACE_WALL_S
            });
            !setting_out
                && super::docked_venue(w, escort).is_some_and(|v| v.as_str() != to.as_str())
        }
        Journey::Field { site } => !near_in_field(w, site, escort),
    }
}

// ---- the arm --------------------------------------------------------------------------------

/// Whether `job` needs a mutable step on this fire (read-only check by ref, zero cloning).
fn needs_step(ctx: &Ctx<'_>, job: &EscortJob) -> bool {
    let now = ctx.w.world_ic_now().at;
    let live = super::contract_state(ctx.w, job.contract).is_some_and(|s| s.is_live());
    match job.stage {
        EscortStage::Posted => {
            now > job.accept_by_ic
                || !live
                || job
                    .haul
                    .is_some_and(|contract| haul_cancelled_before_journey(ctx.w, contract))
        }
        EscortStage::Taken => {
            job.with.is_none()
                || job
                    .haul
                    .is_some_and(|contract| haul_cancelled_before_journey(ctx.w, contract))
                || (job.protected == UNBOUND
                    && job.haul.is_some_and(|contract| {
                        super::haul::haul_of(ctx.w, contract).is_some_and(|h| {
                            h.stage == super::haul::HaulStage::Aboard && h.hull.is_some()
                        })
                    }))
                || !live
                || job.due_ic.is_some_and(|due| now >= due)
                || (job.protected != UNBOUND
                    && (wrecked(ctx.w, job.protected)
                        || captured(ctx.w, job.protected)
                        || journey_now(ctx.w, job).0))
        }
        EscortStage::Watching => true,
        EscortStage::Kept | EscortStage::Failed => !live,
        EscortStage::Closed => false,
    }
}

/// **The host's watch** (doc 48 cut EX.6): each live escort's next step, at the port arm's fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    prune_closed(ctx);
    let due: Vec<u64> = live_escorts(ctx.w)
        .filter(|job| needs_step(ctx, job))
        .map(|job| job.contract)
        .collect();
    for contract in due {
        let Some(mut job) = escort_of(ctx.w, contract) else {
            continue;
        };
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
            if job
                .haul
                .is_some_and(|contract| haul_cancelled_before_journey(ctx.w, contract))
            {
                if live
                    && !matches!(
                        mush_economy::cancel_job(ctx, ContractId(job.contract)),
                        Ok(true)
                    )
                {
                    return false;
                }
                job.stage = EscortStage::Closed;
                let line = TOLD_HAUL_GONE.replace("{job}", &job.job());
                tell_both(ctx, job, &line);
                return true;
            }
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

/// **One fire's interval into the watch**; the interval, IC seconds.
///
/// The interval counts to the journey's time, and to the near time when `is_near`. A field
/// journey's **closing interval** (`field_closed`: the fire that finds the field journey ended)
/// counts to neither (owner's ruling, 2026-11-19, on doc 43 TS.10a's question): once the field is
/// gone the host cannot see where the escort was, so the keep or fail is decided on what was
/// counted before it. A field journey with nothing counted before its close (watched on one fire,
/// ended on the next) counts that interval as it always has, so its outcome is today's
/// (provisional, flagged for the owner in doc 48's EX.6 cut note). A passage's closing interval
/// counts as before: the passage's record outlives the arrival, so [`near_in_space`] still reads
/// it.
fn count(watch: &mut Watch, now: i64, is_near: bool, field_closed: bool) -> i64 {
    let dt = now.saturating_sub(watch.last_ic).max(0);
    watch.last_ic = now;
    if field_closed && watch.journey_ic > 0 {
        return dt;
    }
    watch.journey_ic = watch.journey_ic.saturating_add(dt);
    if is_near {
        watch.near_ic = watch.near_ic.saturating_add(dt);
    }
    dt
}

/// Handle haul binding and pre-journey haul cancellation for a `Taken` haul escort (`P2-TSW-22`).
/// Returns `Some(changed)` when `watch` should return early, or `None` to continue normal checks.
fn step_haul_binding(
    ctx: &mut Ctx<'_>,
    job: &mut EscortJob,
    escort: ObjId,
    now: i64,
    live: bool,
) -> Option<bool> {
    if job.stage != EscortStage::Taken {
        return None;
    }
    let contract = job.haul?;
    if haul_cancelled_before_journey(ctx.w, contract) {
        if live
            && !matches!(
                mush_economy::cancel_job(ctx, ContractId(job.contract)),
                Ok(true)
            )
        {
            return Some(false);
        }
        job.stage = EscortStage::Closed;
        let line = TOLD_HAUL_GONE.replace("{job}", &job.job());
        tell_both(ctx, job, &line);
        return Some(true);
    }
    if job.protected != UNBOUND {
        return None;
    }
    let haul = super::haul::haul_of(ctx.w, contract);
    // Only bind once the haul's `Load` leg has actually landed (`HaulStage::Aboard`), so a
    // pre-load cancellation (`no_room` / `MAX_LEG_TRIES`) never leaves an escort bound to an
    // empty hull.
    if let Some(hull) = haul
        .as_ref()
        .filter(|h| h.stage == super::haul::HaulStage::Aboard)
        .and_then(|h| h.hull)
    {
        // Doc 50b (OP.3): the hauler took the haul in the escort's own hull. A hull does not
        // escort herself, whichever contract was taken first, so the escort is called off.
        if hull == escort {
            if !matches!(
                mush_economy::cancel_job(ctx, ContractId(job.contract)),
                Ok(true)
            ) {
                return Some(false);
            }
            job.stage = EscortStage::Closed;
            let line = TOLD_SAME_HULL.replace("{job}", &job.job());
            tell_both(ctx, job, &line);
            return Some(true);
        }
        job.protected = hull;
        return Some(true);
    }
    if !live || job.due_ic.is_some_and(|due| now >= due) {
        return None;
    }
    Some(false)
}

/// Update `watch`'s prior-proximity and cumulative grace counters for one fire (`P2-TSW-10`).
/// Returns `true` when this fire is covered by mode-scoped, bounded linkdead grace.
fn advance_grace(
    journey: &Journey,
    watch: &mut Watch,
    actually_near: bool,
    grace_started: Option<i64>,
    now: i64,
    wall: i64,
) -> bool {
    let dt_ic = now.saturating_sub(watch.last_ic).max(0);
    let wall_base = watch
        .last_wall
        .unwrap_or_else(|| grace_started.unwrap_or(wall));
    let dt_wall = wall.saturating_sub(wall_base).max(0);
    watch.last_wall = Some(wall);
    if actually_near {
        watch.was_near = true;
        return false;
    }
    let wall_cap = max_grace_wall_s(journey);
    if watch.was_near
        && grace_started.is_some()
        && watch.grace_wall_s < wall_cap
        && watch.grace_ic < LINKDEAD_GRACE_IC_S
    {
        let next_wall = watch.grace_wall_s.saturating_add(dt_wall);
        let next_ic = watch.grace_ic.saturating_add(dt_ic);
        if next_wall <= wall_cap && next_ic <= LINKDEAD_GRACE_IC_S {
            watch.grace_wall_s = next_wall;
            watch.grace_ic = next_ic;
            return true;
        }
        // Cumulative grace ceiling crossed: exhaust grace permanently for this contract.
        watch.grace_wall_s = wall_cap;
        watch.grace_ic = LINKDEAD_GRACE_IC_S;
    }
    watch.was_near = false;
    false
}

/// **The watch over a taken job.** Each fire's interval is counted by [`count`], which leaves a
/// field journey's closing interval out; the outcome is decided when the journey ends.
fn watch(ctx: &mut Ctx<'_>, job: &mut EscortJob, now: i64, live: bool) -> bool {
    let Some(escort) = job.with else {
        job.stage = EscortStage::Closed;
        return true;
    };
    if let Some(changed) = step_haul_binding(ctx, job, escort, now, live) {
        return changed;
    }
    if live && job.stage == EscortStage::Taken && job.due_ic.is_some_and(|due| now >= due) {
        // The term ran out and her journey never began (48e services:G2): no escort was owed,
        // so the job is called off and the unwind returns the fee and the escort's bond. The
        // contract's term runs [`DECIDE_LEAD_IC`] past this, so the host always gets here first.
        if !matches!(
            mush_economy::cancel_job(ctx, ContractId(job.contract)),
            Ok(true)
        ) {
            // The economy refused (frozen, or busy): try again at the next fire.
            return false;
        }
        job.stage = EscortStage::Closed;
        let line = TOLD_UNSTARTED
            .replace("{job}", &job.job())
            .replace("{days}", &ESCORT_TERM_IC_DAYS.to_string());
        tell_both(ctx, job, &line);
        return true;
    }
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
    let wall = ctx.w.now();
    if job.stage == EscortStage::Taken {
        if !running {
            return false;
        }
        let initially_near = near(ctx.w, job, escort);
        job.stage = EscortStage::Watching;
        job.watch = Some(Watch {
            started_ic: now,
            last_ic: now,
            journey_ic: 0,
            near_ic: 0,
            launch_ic: now,
            launch_wall: wall,
            between: false,
            was_near: initially_near,
            last_wall: Some(wall),
            grace_wall_s: 0,
            grace_ic: 0,
        });
        let line = TOLD_STARTED.replace("{job}", &job.job());
        tell_both(ctx, job, &line);
        return true;
    }
    // Doc 50 cut OP.3 (doc 50a A-8b): docked at a port on the way is a hop. Nothing counts while
    // she lies there, and her next departure restarts the grace to set out.
    if matches!(job.journey, Journey::Passage { .. }) && !running && !ended {
        let Some(watch) = job.watch.as_mut() else {
            return false;
        };
        watch.last_ic = now;
        watch.last_wall = Some(wall);
        watch.between = true;
        return true;
    }
    let actually_near = near(ctx.w, job, escort);
    if let Some(watch) = job.watch.as_mut()
        && watch.between
    {
        watch.between = false;
        watch.launch_ic = now;
        watch.launch_wall = wall;
        watch.last_wall = Some(wall);
        // Require verified proximity on the new leg before linkdead grace applies to it.
        watch.was_near = actually_near;
    }
    let grace_started = raw_grace_started_wall(ctx.w, &job.journey, escort);
    let in_field = matches!(job.journey, Journey::Field { .. });
    let journey = job.journey.clone();
    let Some(watch) = job.watch.as_mut() else {
        return false;
    };
    let prev_grace = (watch.was_near, watch.grace_wall_s, watch.grace_ic);
    let in_grace = advance_grace(&journey, watch, actually_near, grace_started, now, wall);
    let is_near = actually_near || in_grace;
    let dt = count(watch, now, is_near, in_field && ended);
    let permille = watch.permille();
    let grace_changed = prev_grace != (watch.was_near, watch.grace_wall_s, watch.grace_ic);
    let left_it = !ended && !in_grace && left_without_grace(ctx.w, job, escort);
    if left_it {
        fail(ctx, job, WHY_LEFT);
        return true;
    }
    if !ended {
        return dt > 0 || grace_changed;
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
    let rows: Vec<String> = live_escorts(ctx.w)
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

/// Whether the poster may post for `principal`'s haul or hull: themself, or the organization.
fn posts_for(poster: &PrincipalId, org: Option<u64>, shipper: ObjId) -> bool {
    match poster {
        PrincipalId::Object(who) => org.is_none() && *who == shipper,
        PrincipalId::Organization(o) => org == Some(o.0),
        PrincipalId::System(_) => false,
    }
}

#[allow(clippy::too_many_lines)] // one ordered refusal list, then the post
/// **Post an escort as `ctx.actor`** (doc 50 cut OP.4: the Freeport arm's agent), `rest` as
/// `+escort/post` reads it: the new escort's contract.
///
/// # Errors
///
/// The line the verb would answer.
pub(crate) fn post_as(ctx: &mut Ctx<'_>, rest: &str) -> Result<u64, String> {
    post_cmd(ctx, rest, true)?;
    escorts(ctx.w)
        .into_iter()
        .filter(|j| j.poster == ctx.actor)
        .map(|j| j.contract)
        .max()
        .ok_or_else(|| "the escort was not recorded".to_owned())
}

fn post_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let (words, opts) = super::options(rest);
    // Doc 50 cut OP.3 (Q-50-63): an escort for a haul.
    let haul = match opts.get("haul") {
        Some(word) => Some(
            word.trim_start_matches('#')
                .parse::<u64>()
                .ok()
                .and_then(|c| super::haul::haul_of(ctx.w, c))
                .filter(|h| {
                    matches!(
                        h.stage,
                        super::haul::HaulStage::Posted
                            | super::haul::HaulStage::Taken
                            | super::haul::HaulStage::Aboard
                    )
                })
                .ok_or_else(|| NO_HAUL.replace("{job}", word))?,
        ),
        None => None,
    };
    let hull = match (&haul, words.as_slice()) {
        (Some(h), []) if h.stage == super::haul::HaulStage::Aboard => h.hull.unwrap_or(UNBOUND),
        (Some(_), []) => UNBOUND,
        (None, [word]) => hull_word(word).ok_or_else(|| ESCORT_USAGE.to_owned())?,
        _ => return Err(ESCORT_USAGE.to_owned()),
    };
    let as_org = opts.get("as").map(String::as_str);
    match &haul {
        Some(h) => {
            let (who, _) = super::poster(ctx, as_org)?;
            if !posts_for(&who, h.shipper_org, h.shipper) {
                return Err(NO_HAUL.replace("{job}", &h.job()));
            }
        }
        None => match as_org {
            Some(_) => {
                let (who, _) = super::poster(ctx, as_org)?;
                let titled = ctx
                    .w
                    .module_state::<mush_economy::EconomyStore>()
                    .and_then(|store| store.snapshot().asset(hull))
                    .is_some_and(|asset| asset.title_owner == who);
                if !titled {
                    return Err(NOT_YOURS.to_owned());
                }
            }
            None => {
                if !holds(ctx.w, ctx.actor, hull) {
                    return Err(NOT_YOURS.to_owned());
                }
            }
        },
    }
    let fee = opts
        .get("fee")
        .and_then(|f| super::credits(f))
        .ok_or_else(|| ESCORT_USAGE.to_owned())?;
    // The board: where the poster stands, else the port their own hull lies at (a field's
    // pad is a port entry with a venue of its own).
    let venue = match &haul {
        Some(h) => {
            mush_market::MarketId::new(h.origin.as_str()).ok_or_else(|| NO_VENUE.to_owned())?
        }
        None => super::standing_venue(ctx.w, ctx.actor)
            .or_else(|| {
                crate::port::coop::helm_of(ctx.w, ctx.actor)
                    .and_then(|h| super::docked_venue(ctx.w, h.hull))
            })
            .ok_or_else(|| NO_VENUE.to_owned())?,
    };
    let journey = match (opts.get("to"), opts.get("field")) {
        _ if haul.is_some() => {
            let h = haul.as_ref().ok_or_else(|| ESCORT_USAGE.to_owned())?;
            Journey::Passage {
                from: h.origin.clone(),
                to: h.destination.clone(),
            }
        }
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
        (None, Some(site)) => {
            let entry =
                super::entry_word(ctx.w, site).ok_or_else(|| NO_PORT.replace("{port}", site))?;
            // A sortie runs only where the port has a field (48e services:G2).
            let has_field = crate::port::market::ports(ctx.w)
                .into_iter()
                .any(|(e, def, _)| e.as_str() == entry && def.site.is_some());
            if !has_field {
                return Err(NO_FIELD.replace("{port}", site));
            }
            Journey::Field {
                site: SmolStr::new(entry),
            }
        }
        _ => return Err(ESCORT_USAGE.to_owned()),
    };
    if let Some(open) = live_escorts(ctx.w).find(|j| {
        (if let Some(h) = &haul {
            j.haul == Some(h.contract)
        } else {
            j.protected == hull
        }) && matches!(
            j.stage,
            EscortStage::Posted | EscortStage::Taken | EscortStage::Watching
        )
    }) {
        return Err(ALREADY.replace("{job}", &open.job()));
    }
    let open_total = live_escorts(ctx.w).count();
    let open_actor = live_escorts(ctx.w)
        .filter(|j| j.poster == ctx.actor)
        .count();
    if open_actor >= MAX_OPEN_ESCORTS_PER_ACTOR {
        return Err(TOO_MANY_OPEN.replace("{cap}", &MAX_OPEN_ESCORTS_PER_ACTOR.to_string()));
    }
    if open_total >= MAX_OPEN_ESCORTS_GLOBAL {
        return Err(BOARD_FULL.to_owned());
    }
    let journey_line = journey_words(ctx.w, &journey);
    if !go {
        return Ok(POST_PREVIEW
            .replace("{name}", &name(ctx.w, hull))
            .replace("{journey}", &journey_line)
            .replace("{fee}", &money_minor(ctx, fee))
            .replace("{days}", &ESCORT_TERM_IC_DAYS.to_string()));
    }
    let (poster, account) = super::poster(ctx, as_org)?;
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
            poster: poster.clone(),
            poster_account: account,
            fee: Amount::from_minor(fee),
            fact: fact_of(peeked),
            deadline_ic: now
                .saturating_add(window)
                .saturating_add(term)
                .saturating_add(DECIDE_LEAD_IC),
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
        poster_org: match &poster {
            PrincipalId::Organization(org) => Some(org.0),
            _ => None,
        },
        haul: haul.as_ref().map(|h| h.contract),
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
    if job
        .haul
        .is_some_and(|contract| haul_cancelled_before_journey(ctx.w, contract))
    {
        return Err(NO_SUCH_JOB.replace("{job}", word));
    }
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
    if let Err(why) = escort_matches_journey(ctx.w, &job.journey, with) {
        return Err(ACCEPT_FAILED.replace("{why}", why));
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
    if taker == PrincipalId::Object(job.poster)
        || job
            .poster_org
            .is_some_and(|org| taker == PrincipalId::Organization(mush_org::OrgId(org)))
    {
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
            term_ic: Some(term.saturating_add(DECIDE_LEAD_IC)),
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

/// Whether `actor` may withdraw `job` (`P2-TSW-48`): for an organization-owned job, live
/// [`super::JOBS_POST_CAPABILITY`] on the posting organization; for a personal job, the poster.
fn may_withdraw(w: &World, actor: ObjId, job: &EscortJob) -> bool {
    match job.poster_org {
        Some(org) => super::may_post_for_org(w, actor, mush_org::OrgId(org)),
        None => job.poster == actor,
    }
}

fn withdraw_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let word = rest.trim();
    let job = word
        .trim_start_matches('#')
        .parse::<u64>()
        .ok()
        .and_then(|c| escort_of(ctx.w, c))
        .filter(|j| j.stage == EscortStage::Posted && may_withdraw(ctx.w, ctx.actor, j))
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
    let job_word = job.job();
    delete(ctx, job.contract);
    Ok(WITHDRAWN.replace("{job}", &job_word))
}

// ---- the board ------------------------------------------------------------------------------

/// **The escorts on `venue`'s board** (Q-48-12): each posted job, where it was posted.
#[must_use]
pub fn services(w: &World, venue: &MarketId) -> Vec<mush_market::Service> {
    let instrument = mush_economy::InstrumentId::new(crate::economy::CREDIT);
    live_escorts(w)
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
                "escort {} on {}{}",
                name(w, j.protected),
                journey_words(w, &j.journey),
                // Doc 50 cut OP.7 (Q-50-30): the operation's work says so on the board.
                crate::op_page::job_tag(w, j.contract)
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

    /// **Doc 50b (P3): the launch grace carries into the proximity test.** At 24:1 the gap is
    /// [`TOGETHER_IC`]; at 3,600:1 it is the IC seconds [`LAUNCH_GRACE_WALL_S`] make.
    #[test]
    fn op50b_the_together_gap_follows_the_clock_rate() {
        assert_eq!(together_ic_at(24), TOGETHER_IC);
        assert_eq!(together_ic_at(3_600), 3_600 * LAUNCH_GRACE_WALL_S);
        assert_eq!(together_ic_at(0), TOGETHER_IC);
    }

    /// **Doc 48 cut EX.6**: the near share is per-mille of the journey, and 90% keeps it.
    #[test]
    fn ex6_the_near_share_is_of_the_journey() {
        let watch = Watch {
            started_ic: 0,
            last_ic: 100,
            journey_ic: 1_000,
            near_ic: 900,
            ..Watch::default()
        };
        assert_eq!(watch.permille(), 900);
        assert!(watch.permille() >= NEAR_PERMILLE);
        assert_eq!(Watch::default().permille(), 0, "no journey, no share");
        assert_eq!(percent(899), "89%");
    }

    /// **Doc 48 cut EX.6, the owner's ruling (2026-11-19)**: a field journey's closing interval
    /// counts to neither the journey nor the near time; the outcome is decided on what came
    /// before it. A passage's closing interval, and a field journey with nothing counted before
    /// its close, count as they always have.
    #[test]
    fn ex6_a_field_journeys_closing_interval_is_not_counted() {
        let running = Watch {
            started_ic: 0,
            last_ic: 140,
            journey_ic: 140,
            near_ic: 140,
            ..Watch::default()
        };
        // The field closed: the escort reads far, and that interval is not counted.
        let mut field = running.clone();
        assert_eq!(count(&mut field, 175, false, true), 35);
        assert_eq!(
            (field.last_ic, field.journey_ic, field.near_ic),
            (175, 140, 140)
        );
        assert!(
            field.permille() >= NEAR_PERMILLE,
            "kept on what came before"
        );
        // A passage's arrival (or any interval while the journey runs) counts.
        let mut passage = running;
        assert_eq!(count(&mut passage, 175, false, false), 35);
        assert_eq!((passage.journey_ic, passage.near_ic), (175, 140));
        assert!(passage.permille() < NEAR_PERMILLE);
        // Nothing counted before the close: that interval counts, as it always has.
        let mut first = Watch {
            started_ic: 100,
            last_ic: 100,
            journey_ic: 0,
            near_ic: 0,
            ..Watch::default()
        };
        assert_eq!(count(&mut first, 130, false, true), 30);
        assert_eq!((first.journey_ic, first.near_ic), (30, 0));
    }

    #[test]
    fn high41_closed_escorts_are_pruned_and_live_iter_skips_closed() {
        struct StoreMod;
        impl mush_core::module::GameModule for StoreMod {
            fn name(&self) -> &'static str {
                "tacsim"
            }
            fn register(
                &mut self,
                reg: &mut mush_core::Registrar<'_>,
            ) -> Result<(), mush_core::ModError> {
                reg.state(TacsimStore::default());
                Ok(())
            }
        }
        let mut tw = mush_testkit::TestWorld::with_modules(vec![Box::new(StoreMod)]);
        let engine = tw.engine_mut();
        let mut out = mush_core::Outbox::default();
        let mut ctx = Ctx {
            w: &mut engine.world,
            reg: &engine.registry,
            out: &mut out,
            actor: ObjId(5),
            args: Default::default(),
            switches: Default::default(),
            module: Some(crate::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };

        let sample = |contract: u64, stage: EscortStage| EscortJob {
            schema: ESCORT_SCHEMA,
            contract,
            poster: ObjId(5),
            poster_org: None,
            haul: None,
            protected: ObjId(10),
            journey: Journey::Passage {
                from: "ceres".into(),
                to: "vesta".into(),
            },
            venue: "ceres".into(),
            fee_minor: 5_000,
            bond_minor: 0,
            posted_ic: 1,
            accept_by_ic: 100,
            escort: None,
            with: None,
            accepted_ic: None,
            due_ic: None,
            watch: None,
            why: None,
            stage,
        };

        for id in 1..=(MAX_CLOSED_ESCORTS as u64 + 4) {
            put(&mut ctx, &sample(id, EscortStage::Closed));
        }
        put(&mut ctx, &sample(2_000, EscortStage::Posted));
        put(&mut ctx, &sample(2_001, EscortStage::Taken));

        let store = ctx.w.module_state::<TacsimStore>().expect("store");
        assert_eq!(store.services.closed_escorts.len(), MAX_CLOSED_ESCORTS);
        assert_eq!(store.services.live_escorts.len(), 2);
        assert!(escort_of(ctx.w, 1).is_none());
        assert!(escort_of(ctx.w, 4).is_none());
        assert!(escort_of(ctx.w, 5).is_some());

        let live_ids: Vec<u64> = live_escorts(ctx.w).map(|j| j.contract).collect();
        assert_eq!(live_ids, vec![2_000, 2_001]);

        delete(&mut ctx, 2_000);
        assert!(escort_of(ctx.w, 2_000).is_none());
        let dels: Vec<String> = ctx
            .out
            .deferred()
            .iter()
            .filter_map(|d| match d {
                mush_core::Deferred::Persist(op) => match &op.op {
                    mush_core::ModuleWrite::Del { key } => Some(key.to_string()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        assert!(dels.contains(&escort_key(1)));
        assert!(dels.contains(&escort_key(4)));
        assert!(dels.contains(&escort_key(2_000)));
    }

    /// **P2-TSW-10**: `advance_grace` requires prior verified proximity (`was_near`), clears
    /// `was_near` if the escort leaves while connected, and enforces non-renewable cumulative
    /// wall-second and IC-second ceilings across disconnect/reconnect cycles.
    #[test]
    fn p2_tsw_10_grace_requires_prior_proximity_and_caps_reconnect_cycling() {
        let passage = Journey::Passage {
            from: "ceres".into(),
            to: "stickney".into(),
        };
        // Never near -> linkdead grants zero grace.
        let mut never_near = Watch {
            started_ic: 0,
            last_ic: 0,
            launch_ic: 0,
            launch_wall: 100,
            last_wall: Some(100),
            was_near: false,
            ..Watch::default()
        };
        assert!(
            !advance_grace(&passage, &mut never_near, false, Some(105), 720, 130),
            "no prior proximity -> zero grace"
        );
        assert_eq!(never_near.grace_wall_s, 0);

        // Verified proximity, then disconnect -> grace covers up to 300 wall s / 7,200 IC s total,
        // even when the escort reconnects and drops again to reset `grace_started_wall_s`.
        let mut watch = Watch {
            started_ic: 0,
            last_ic: 0,
            launch_ic: 0,
            launch_wall: 100,
            last_wall: Some(100),
            was_near: false,
            ..Watch::default()
        };
        assert!(!advance_grace(&passage, &mut watch, true, None, 720, 130));
        count(&mut watch, 720, true, false);
        assert!(watch.was_near);

        // Cycle 1: linkdead for 240 wall s (8 fires of 30s at 24:1 = 5,760 IC s).
        let mut wall = 130;
        let mut ic = 720;
        for _ in 0..8 {
            wall += 30;
            ic += 720;
            assert!(advance_grace(
                &passage,
                &mut watch,
                false,
                Some(131),
                ic,
                wall
            ));
            count(&mut watch, ic, true, false);
        }
        assert_eq!(watch.grace_wall_s, 240);
        assert_eq!(watch.grace_ic, 5_760);

        // Reconnect briefly and re-drop right before the next tick (fresh `grace_started_wall_s`
        // = wall - 1). Two more 30s fires exhaust the remaining 60s budget cleanly.
        for _ in 0..2 {
            wall += 30;
            ic += 720;
            assert!(advance_grace(
                &passage,
                &mut watch,
                false,
                Some(wall - 1),
                ic,
                wall
            ));
            count(&mut watch, ic, true, false);
        }
        assert_eq!(watch.grace_wall_s, 300);
        assert_eq!(watch.grace_ic, LINKDEAD_GRACE_IC_S);

        // Even after reconnecting with a brand-new linkdead window, cumulative grace is spent.
        wall += 30;
        ic += 720;
        assert!(
            !advance_grace(&passage, &mut watch, false, Some(wall - 1), ic, wall),
            "reconnect cycling cannot extend grace past the contract ceiling"
        );
        assert!(!watch.was_near);

        // Even if the escort flies near again later, cumulative grace stays exhausted.
        wall += 30;
        ic += 720;
        assert!(!advance_grace(&passage, &mut watch, true, None, ic, wall));
        count(&mut watch, ic, true, false);
        assert!(watch.was_near);
        wall += 30;
        ic += 720;
        assert!(
            !advance_grace(&passage, &mut watch, false, Some(wall - 1), ic, wall),
            "cumulative grace never resets for the remainder of the contract"
        );
    }

    /// **P2-TSW-10**: leaving proximity while connected clears `was_near`, and an accelerated IC
    /// jump cannot credit more than [`LINKDEAD_GRACE_IC_S`].
    #[test]
    fn p2_tsw_10_leaving_clears_was_near_and_ic_cap_bounds_fast_clocks() {
        let passage = Journey::Passage {
            from: "ceres".into(),
            to: "stickney".into(),
        };
        let mut watch = Watch {
            started_ic: 0,
            last_ic: 0,
            launch_ic: 0,
            launch_wall: 100,
            last_wall: Some(100),
            was_near: true,
            ..Watch::default()
        };
        // Not near while connected -> clears `was_near`.
        assert!(!advance_grace(&passage, &mut watch, false, None, 720, 130));
        count(&mut watch, 720, false, false);
        assert!(!watch.was_near);
        // Going linkdead on the next tick is denied because `was_near` was cleared.
        assert!(!advance_grace(
            &passage,
            &mut watch,
            false,
            Some(131),
            1_440,
            160
        ));

        // Accelerated clock: 10 wall seconds = 10,000 IC seconds > LINKDEAD_GRACE_IC_S -> denied.
        let mut fast = Watch {
            started_ic: 0,
            last_ic: 0,
            launch_ic: 0,
            launch_wall: 100,
            last_wall: Some(100),
            was_near: true,
            ..Watch::default()
        };
        assert!(
            !advance_grace(
                &passage,
                &mut fast,
                false,
                Some(101),
                LINKDEAD_GRACE_IC_S + 1,
                110
            ),
            "IC grace ceiling blocks fast-clock grace abuse"
        );
    }

    /// **P2-TSW-48**: `+escort/withdraw` enforces live `JOBS_POST_CAPABILITY` for org-owned
    /// escorts (blocking a revoked original poster and permitting an active officer), while
    /// personal escorts continue checking `poster == actor`.
    #[test]
    fn p2_tsw_48_org_escort_withdraw_requires_live_capability() {
        let mut tw =
            mush_testkit::TestWorld::with_modules(vec![Box::new(mush_org::OrgModule::new())]);
        let ex_poster = tw.spawn_player("ExEscortPoster");
        let officer = tw.spawn_player("EscortOfficer");
        let org_id = mush_org::OrgId(88);
        let root_scope = mush_org::OrgScopeId(880);
        {
            let engine = tw.engine_mut();
            let mut out = mush_core::Outbox::default();
            let mut ctx = Ctx {
                w: &mut engine.world,
                reg: &engine.registry,
                out: &mut out,
                actor: mush_core::GOD,
                args: Default::default(),
                switches: Default::default(),
                module: Some(crate::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            let plan = mush_org::OrgMutationPlan::of(vec![
                mush_org::OrgMutationOp::CreateOrg {
                    org: mush_org::Organization {
                        id: org_id,
                        name: "Occator".into(),
                        aliases: Vec::new(),
                        status: mush_org::OrgStatus::Active,
                        hidden: false,
                        charter: String::new(),
                        root_scope,
                        revision: 1,
                        ruleset_ext: mush_org::RulesValue::default(),
                    },
                    root: mush_org::OrgScope {
                        id: root_scope,
                        org: org_id,
                        parent: None,
                        name: "root".into(),
                        status: mush_org::ScopeStatus::Active,
                        depth: 0,
                        created_rev: 1,
                        revision: 1,
                    },
                },
                mush_org::OrgMutationOp::PutGrant {
                    grant: mush_org::CapabilityGrant {
                        id: mush_org::GrantId(2),
                        org: org_id,
                        grantee: officer,
                        capability: mush_org::CapabilityId::new(super::super::JOBS_POST_CAPABILITY)
                            .expect("cap"),
                        scope: mush_org::ScopeSelector::Exact(root_scope),
                        limit: None,
                        delegable: false,
                        valid_from: None,
                        valid_until: None,
                        revision: 1,
                    },
                },
            ]);
            mush_org::submit(
                &mut ctx,
                &plan,
                mush_org::OrgAction {
                    action: "test.org".into(),
                    org: Some(org_id),
                    detail: serde_json::Value::Null,
                },
            )
            .expect("org setup");
        }

        let mut org_job = EscortJob {
            schema: ESCORT_SCHEMA,
            contract: 90,
            poster: ex_poster,
            poster_org: Some(org_id.0),
            haul: None,
            protected: ObjId(500),
            journey: Journey::Passage {
                from: "ceres".into(),
                to: "stickney".into(),
            },
            venue: "ceres".into(),
            fee_minor: 50_000,
            bond_minor: 0,
            posted_ic: 100,
            accept_by_ic: 100 + DAY,
            escort: None,
            with: None,
            accepted_ic: None,
            due_ic: None,
            watch: None,
            why: None,
            stage: EscortStage::Posted,
        };
        assert!(!may_withdraw(tw.world(), ex_poster, &org_job));
        assert!(may_withdraw(tw.world(), officer, &org_job));

        org_job.poster_org = None;
        assert!(may_withdraw(tw.world(), ex_poster, &org_job));
        assert!(!may_withdraw(tw.world(), officer, &org_job));
    }
}
