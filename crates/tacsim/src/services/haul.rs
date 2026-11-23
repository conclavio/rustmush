//! **The haul** (doc 48 cut EX.6; Q-48-11, -42, -57, -58; doc 48b W-48-3b, W-48-3c).
//!
//! A shipper posts a job to carry goods from their locker at one port to their locker at
//! another. A hauler whose hull is docked at the origin takes it. The goods ride in her hold as
//! a held lot reserved to the job, and the job pays on the delivery's inventory marker.
//!
//! - **The post** (`+haul/post`): one standing job ([`mush_economy::post_job`]), the fee escrowed
//!   from the shipper's account. Its one condition is an `inventory.transferred` marker whose id
//!   is derived now ([`mush_inventory::derived_marker_id`]), so only the delivery leg can satisfy
//!   it. The declared value is capped at the reference price × quantity (Q-48-57). The shipper's
//!   locker at the destination opens with the post, so the delivery has somewhere to land.
//! - **The acceptance** (`+haul/accept`): [`mush_economy::join_job`] with a bond of 25% of the
//!   declared value (Q-48-42) and a term of [`HAUL_TERM_IC_DAYS`] from acceptance (Q-48-58).
//!   The same command submits the load leg: the goods leave the shipper's locker and land in the
//!   hold, reserved to the job ([`mush_inventory::Carriage`], `reserve_for`'s deadline) for the
//!   term and [`RETURN_GRACE_IC_DAYS`] beyond it, so the hold outlives the job. Held goods are
//!   not the hauler's to sell, burn or move: only a leg of this job moves them.
//! - **The delivery** (`+haul/deliver`, the delivery verb): at the destination, before the
//!   term ends, the leg consumes the hold and lands the goods free in the shipper's locker,
//!   writing the marker the contract names. The economy's sweep then pays the fee and returns
//!   the bond.
//! - **The deadline**: an unperformed job expires. The economy refunds the fee and forfeits the
//!   bond to the shipper (Q-48-42). Cargo still aboard stays held, and at the hull's next port
//!   the port arm submits the return leg as the hauler: the goods go back free to the shipper's
//!   locker there (doc 48b W-48-3c).
//!
//! - **For an organization** (doc 50 cut OP.3, Q-50-23): `+haul/post ... as=<org>` posts for an
//!   organization whose root compartment grants the poster
//!   [`super::JOBS_POST_CAPABILITY`]: the fee from its account, the goods from its vault at the
//!   origin to a vault at the destination: its own, or, with `into=<org>`, another
//!   organization's (vault to vault; [`super::org_vault`]). A late return goes to the
//!   organization's vault where the hull lies, and waits for a port that has one.
//! - **A loss** (doc 50 cut OP.3, Q-50-25): a hull wrecked or captured with the goods aboard
//!   owes the [`LegKind::Lost`] leg, which releases the job's hold so the goods are part of the
//!   wreck, and the arm lapses the job: the fee back to the shipper and the bond to them, as a
//!   seizure does.
//!
//! The legs are works' shape (doc 39 cut IN.3): the record names the leg owed, the command or the
//! port arm submits it as its actor, the offer projects it from the record, and the prepare moves
//! the goods once under the job's ledger ordinal (`tacsim.haul_ledger`).

use std::collections::BTreeMap;

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalHash, CanonicalId, ClockStamp, CmdError, CommitToken, Ctx, DefinitionDescriptor,
    DefinitionId, DefinitionKind, DefinitionRef, IdempotencyKey, IdempotencyKeyText,
    IdempotencyScope, ModError, ObjId, OwnerId, PerceivedRef, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    Registrar, ReservationId, RevisionSet, RulesValue, ScopedIdempotencyKey, StateFamilyHandle,
    SubjectKey, World,
};
use mush_economy::{Amount, ContractId, VaultId};
use mush_inventory::InventoryFamilies;
use mush_inventory::record::EndpointRef;
use mush_market::{CapacityProvider, MarketId};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use smol_str::SmolStr;

use crate::port::coop::money_minor;
use crate::port::market::PortCapacity;
use crate::port::owned::Held;
use crate::port_tick::port_error;
use crate::store::TacsimStore;

/// The verb.
pub const HAUL_VERB: &str = "+haul";
/// The record's key prefix: `tacsim:haul:{contract}`.
pub const HAUL_PREFIX: &str = "tacsim:haul:";
/// The record's schema.
pub const HAUL_SCHEMA: u16 = 1;
/// The legs' action.
pub const HAUL_ACTION: &str = "tacsim_haul";
/// Its outcome.
pub const HAUL_OUTCOME: &str = "tacsim_haul_result";
/// The ledger family: the last landed leg a job.
pub const HAUL_FAMILY: &str = "tacsim.haul_ledger";
/// How long a posted haul waits for a taker, IC days (the owner's rule: a wait on a player is
/// at least 28 IC days, 28 real hours at 24:1).
pub const ACCEPT_WINDOW_IC_DAYS: i64 = 28;
/// **The haul's term from acceptance**, IC days (Q-48-58, provisional): 28 real hours at 24:1.
pub const HAUL_TERM_IC_DAYS: i64 = 28;
/// **How long the hold outlives the job**, IC days: the cargo stays held this long past the
/// term, on top of the inventory TTL, so it is still the job's when the hull next docks.
pub const RETURN_GRACE_IC_DAYS: i64 = 28;
/// **The hauler's bond** (Q-48-42, provisional): 25% of the declared value, in per-mille.
pub const HAUL_BOND_PERMILLE: i64 = 250;
/// How many times the arm submits one leg before it gives up on it.
pub const MAX_LEG_TRIES: u32 = 8;
/// Most open (non-closed) hauls one actor may post concurrently.
pub const MAX_OPEN_HAULS_PER_ACTOR: usize = 16;
/// Most open (non-closed) hauls across the world concurrently.
pub const MAX_OPEN_HAULS_GLOBAL: usize = 512;
/// Most recently-closed hauls retained before oldest closed records are deleted.
pub const MAX_CLOSED_HAULS: usize = 64;
/// The most legs one offer carries.
const MAX_OFFER_ROWS: usize = 16;
/// IC seconds in an IC day.
const DAY: i64 = 86_400;
/// The channel lines go out on.
const CHANNEL: &str = "tacsim.haul";

// ---- the player's sentences -----------------------------------------------------------------

/// The usage line.
pub const HAUL_USAGE: &str = "Usage: +haul  |  +haul/post <qty> <good> to=<port> fee=<cr> [value=<cr>] [as=<org> [into=<org>]] [go]\n       \
+haul/accept <job> [as=<org>] [go]  |  +haul/deliver <job> [go]  |  +haul/withdraw <job> [go]";
/// The help.
pub const HAUL_HELP: &str = "+haul                                  your hauls, posted and taken\n\
+haul/post <qty> <good> to=<port> fee=<cr> [value=<cr>] [go]\n\
                                       post a job to carry goods from your locker here to your\n\
                                       locker there. The fee is escrowed now and paid on\n\
                                       delivery. The value you declare is capped at the\n\
                                       reference price; the hauler bonds a quarter of it.\n\
+haul/post ... as=<org> [into=<org>] [go]\n\
                                       post for an organization you may post jobs for: its\n\
                                       goods from its vault here to its vault there (or\n\
                                       into=<org>'s), the fee from its account.\n\
+haul/accept <job> [as=<org>] [go]     take a haul with your hull docked here: you escrow the\n\
                                       bond, the goods load into your hold held for the job,\n\
                                       and you have 28 IC days to deliver them.\n\
+haul/deliver <job> [go]               hand the goods over at the destination; the fee and\n\
                                       your bond follow.\n\
+haul/withdraw <job> [go]              withdraw a haul nobody has taken; the fee comes back.\n\
Goods not delivered in time go back to the shipper at your next port, and the bond is theirs.";
/// The post's preview (`{qty}`, `{good}`, `{origin}`, `{port}`, `{fee}`, `{value}`, `{bond}`).
pub const POST_PREVIEW: &str = "Post a haul of {qty} {good} from {origin} to {port} for {fee}, \
declared at {value} (a hauler bonds {bond}). +haul/post ... go posts it.";
/// The post's line (`{job}`, `{qty}`, `{good}`, `{port}`, `{fee}`).
pub const POSTED: &str =
    "Haul {job} posted: {qty} {good} to {port}, {fee} escrowed and paid on delivery.";
/// The refusal for no venue here.
pub const NO_VENUE: &str = "There is no market here to ship from.";
/// The refusal for no locker of the shipper's here.
pub const NO_LOCKER: &str =
    "A haul ships from your locker, and you have none here yet (+locker/open).";
/// The refusal for too little of the good in the locker (`{good}`).
pub const SHORT: &str = "Your locker here holds too little {good} for that haul.";
/// The refusal for an unknown good (`{good}`).
pub const NO_GOOD: &str = "No good answers to {good}.";
/// The refusal for an unknown port (`{port}`).
pub const NO_PORT: &str = "No port answers to {port}; name a port of the lane.";
/// The refusal for a destination that is here.
pub const SAME_PORT: &str = "A haul goes to another port.";
/// The refusal for a declared value over the cap (`{cap}`).
pub const OVER_CAP: &str = "A declared value is at most the reference price of the goods, {cap}.";
/// The refusal for a good with no reference price.
pub const NO_REFERENCE: &str = "That good has no reference price, so its value cannot be declared.";
/// The economy's refusal (`{why}`).
pub const POST_FAILED: &str = "The haul could not be posted: {why}.";
/// The refusal when the shipper already has the maximum number of open hauls (`{cap}`).
pub const TOO_MANY_OPEN: &str = "You already have {cap} open hauls; finish or withdraw one first.";
/// The refusal when the world's open haul board is full.
pub const BOARD_FULL: &str = "The haul board is full right now; try again later.";
/// The refusal for no such open job (`{job}`).
pub const NO_SUCH_JOB: &str = "There is no open haul {job}; +market/jobs haul lists them.";
/// The refusal for the shipper's own job.
pub const OWN_JOB: &str = "That haul is your own.";
/// The refusal for no hull of the taker's docked at the origin (`{origin}`).
pub const NOT_DOCKED: &str = "The goods wait at {origin}, and your hull is not docked there.";
/// The refusal for a hold with no room.
pub const NO_ROOM: &str = "Your hold has no room for those goods.";
/// Told the hauler when her hold has no room for a taken haul's goods at the load (doc 50b): `{job}`.
pub const TOLD_NO_ROOM: &str = "Haul {job} is called off: your hold has no room for its goods, so they never loaded. The fee goes back to the shipper and your bond back to you.";
/// The refusal for goods the shipper no longer has.
pub const GOODS_GONE: &str = "The shipper's goods are no longer there to load.";
/// The acceptance's preview (`{qty}`, `{good}`, `{port}`, `{fee}`, `{value}`, `{bond}`, `{days}`).
pub const ACCEPT_PREVIEW: &str = "Take the haul: carry {qty} {good} to {port} within {days} IC days \
for {fee}. The goods are declared at {value}; you escrow a bond of {bond}, yours again on delivery.";
/// The acceptance's line (`{job}`, `{days}`).
pub const ACCEPTED: &str = "You take haul {job}. The goods load into your hold, held for the job; \
deliver them within {days} IC days.";
/// The economy's refusal at acceptance (`{why}`).
pub const ACCEPT_FAILED: &str = "The haul could not be taken: {why}.";
/// The shipper's notice (`{who}`, `{job}`).
pub const TOLD_TAKEN: &str = "{who} has taken haul {job}.";
/// The refusal for a delivery away from the destination (`{port}`).
pub const NOT_THERE: &str = "Your hull is not docked at {port}, the haul's destination.";
/// The refusal for a delivery past the term.
pub const TOO_LATE: &str = "The haul's term has ended; the goods go back to the shipper.";
/// The refusal for a job not loaded yet, or not the taker's.
pub const NOT_ABOARD: &str = "You carry no goods for haul {job}.";
/// The delivery's preview (`{qty}`, `{good}`, `{port}`).
pub const DELIVER_PREVIEW: &str =
    "Hand over {qty} {good} at {port}. +haul/deliver <job> go hands them over.";
/// The delivery's line (`{job}`).
pub const DELIVERING: &str =
    "The goods of haul {job} are handed over; the fee and your bond follow.";
/// The shipper's notice of the delivery (`{job}`, `{port}`).
pub const TOLD_DELIVERED: &str = "Haul {job} is delivered to your locker at {port}.";
/// The shipper's notice of the return (`{job}`, `{port}`).
pub const TOLD_RETURNED: &str =
    "Haul {job} was not delivered in time; its goods are back in your locker at {port}.";
/// The shipper's notice of a seizure (`{job}`): doc 50 unit CR.
pub const TOLD_SEIZED: &str = "Haul {job} was seized by customs on the way; its goods are lost. The fee comes back to you, and the hauler's bond is yours.";
/// The hauler's notice of a seizure (`{job}`).
pub const TOLD_HAULER_SEIZED: &str = "Customs took the goods of haul {job} from your hold; the job has failed and your bond is lost.";
/// The shipper's line when a haul's hull is wrecked or captured with its goods (doc 50 cut OP.3).
pub const TOLD_LOST: &str = "Haul {job} was lost on the way with its hull; its goods are part of the wreck. The fee comes back to you, and the hauler's bond is yours.";
/// The hauler's line when her hull is lost with a haul's goods (doc 50 cut OP.3).
pub const TOLD_HAULER_LOST: &str = "Haul {job} is lost with your hull; its goods are part of the wreck, the job has failed and your bond is lost.";
/// An organization keeps no vault at a port (doc 50 cut OP.3).
pub const NO_ORG_VAULT: &str = "That organization keeps no vault at {port}.";
/// The refusal for withdrawing a taken or another's job.
pub const NOT_WITHDRAWABLE: &str =
    "Only the shipper withdraws a haul, and only before it is taken.";
/// The withdrawal's line (`{job}`).
pub const WITHDRAWN: &str = "Haul {job} is withdrawn; the fee comes back.";
/// No hauls.
pub const NONE: &str = "You have no hauls.";
/// The list head.
pub const HEAD: &str = "Your hauls, posted and taken:";
/// One list row (`{job}`, `{qty}`, `{good}`, `{origin}`, `{port}`, `{stage}`).
pub const ROW: &str = "  {job}  {qty} {good} on hire from {origin} to {port}: {stage}";

// ---- the record -----------------------------------------------------------------------------

/// **Where a haul stands.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HaulStage {
    /// Posted, on the board, nobody has taken it.
    Posted,
    /// Taken; the load leg is owed.
    Taken,
    /// The goods are aboard, held for the job.
    Aboard,
    /// Handed over; the sweep pays the fee.
    Delivered,
    /// The term ended with the goods aboard; the return leg is owed at the hull's next port.
    Returning,
    /// Done: paid, returned, withdrawn or lapsed.
    Closed,
}

impl HaulStage {
    /// The list's word.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Posted => "posted, waiting for a hauler",
            Self::Taken => "taken, loading",
            Self::Aboard => "aboard, held for the job",
            Self::Delivered => "delivered",
            Self::Returning => "late, going back to the shipper",
            Self::Closed => "closed",
        }
    }
}

/// **Which leg.**
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegKind {
    /// The shipper's locker to the hold, held for the job.
    Load,
    /// The hold to the shipper's locker at the destination, the contract's marker.
    Deliver,
    /// The hold to the shipper's locker at the hull's port, after the term.
    Return,
    /// **Not a leg: the cargo was seized** (doc 50 unit CR; Q-50-25 as corrected). The customs
    /// ruling that took it released the job's hold and wrote this on the job's ledger in its
    /// own transaction, so no leg of the job lands after it; the arm then fails the job.
    Seized,
    /// **The hull was lost** (doc 50 cut OP.3; Q-50-25): wrecked or captured with the goods
    /// aboard. The leg releases the job's hold, so the goods are part of the wreck, and moves
    /// nothing; the arm then fails the job.
    Lost,
}

/// **The leg a haul owes**: what the arm submits and the offer projects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HaulLeg {
    /// Its ordinal on the job's ledger: 1 the load, 2 the delivery or the return.
    pub act: u32,
    /// Which leg.
    pub kind: LegKind,
    /// Who submits it: the hauler's pilot.
    pub by: ObjId,
    /// The vault the goods leave, `VaultId.0`.
    pub from: u64,
    /// The vault they land in.
    pub to: u64,
    /// The good.
    pub good: CanonicalId,
    /// How much.
    pub quantity: u64,
    /// The marker's id.
    pub marker: SmolStr,
    /// The IC second the leg must land by (the delivery's term); `None` for none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by_ic: Option<i64>,
    /// The IC seconds the load's hold must outlive (the term and the grace).
    #[serde(default)]
    pub lead_ic: i64,
}

/// **One haul** (doc 48 cut EX.6): the job's terms, its taker and the leg it owes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HaulJob {
    /// [`HAUL_SCHEMA`].
    pub schema: u16,
    /// The contract, `ContractId.0`: the job's number.
    pub contract: u64,
    /// The shipper.
    pub shipper: ObjId,
    /// The shipper's account the fee came from.
    pub shipper_account: u64,
    /// **The organization that posted it** (doc 50 cut OP.3), `OrgId.0`, when it was posted
    /// `as=<org>`: `shipper` is then the member who posted it, and the goods ship vault to vault.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shipper_org: Option<u64>,
    /// The origin's venue.
    pub origin: SmolStr,
    /// The destination's venue.
    pub destination: SmolStr,
    /// The shipper's locker at the origin, `VaultId.0`.
    pub source: u64,
    /// The shipper's locker at the destination.
    pub drop: u64,
    /// The good.
    pub good: CanonicalId,
    /// How much.
    pub quantity: u64,
    /// The declared value, minor units (at most the reference × quantity, Q-48-57).
    pub declared_minor: i64,
    /// The fee, minor units.
    pub fee_minor: i64,
    /// The hauler's bond, minor units (25% of the declared value).
    pub bond_minor: i64,
    /// Posted, IC seconds.
    pub posted_ic: i64,
    /// The end of the wait for a taker.
    pub accept_by_ic: i64,
    /// The delivery marker the contract names.
    pub marker: SmolStr,
    /// Who took it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hauler: Option<PrincipalId>,
    /// The pilot whose hull carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pilot: Option<ObjId>,
    /// The hull.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hull: Option<ObjId>,
    /// The hull's hold, `VaultId.0`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<u64>,
    /// The end of the term, IC seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_ic: Option<i64>,
    /// The leg owed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owed: Option<HaulLeg>,
    /// How many times the owed leg was submitted.
    #[serde(default)]
    pub tries: u32,
    /// Where it stands.
    pub stage: HaulStage,
}

impl HaulJob {
    /// The record's own rules.
    ///
    /// # Errors
    ///
    /// The rule broken.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != HAUL_SCHEMA {
            return Err(format!("schema {} is not {HAUL_SCHEMA}", self.schema));
        }
        if self.quantity == 0 || self.fee_minor <= 0 || self.declared_minor < 0 {
            return Err("a haul carries goods for a positive fee".to_owned());
        }
        if self.bond_minor != bond_of(self.declared_minor) {
            return Err("the bond is not a quarter of the declared value".to_owned());
        }
        if self.origin == self.destination || self.source == self.drop {
            return Err("a haul goes from one port to another".to_owned());
        }
        if !mush_inventory::is_marker_id(&self.marker) {
            return Err("the delivery marker is not a marker id".to_owned());
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
        haul_key(self.contract)
    }
}

/// The record key of job `contract`.
#[must_use]
pub fn haul_key(contract: u64) -> String {
    format!("{HAUL_PREFIX}{contract}")
}

/// **The hauler's bond on a declared value** (Q-48-42): 25%, rounded down.
#[must_use]
pub const fn bond_of(declared_minor: i64) -> i64 {
    declared_minor.saturating_mul(HAUL_BOND_PERMILLE) / 1_000
}

/// **The ledger row**: the last leg a job landed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HaulLedger {
    /// The job.
    pub contract: u64,
    /// The last landed leg's ordinal.
    pub acts: u32,
    /// Which leg it was.
    pub last: LegKind,
}

/// Every readable haul.
#[must_use]
pub fn hauls(w: &World) -> Vec<HaulJob> {
    super::book(w)
        .map(|book| {
            book.hauls
                .values()
                .filter_map(Held::present)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// Every readable non-closed haul, by reference (O(live), no cloning).
pub(crate) fn live_hauls(w: &World) -> impl Iterator<Item = &HaulJob> {
    super::book(w).into_iter().flat_map(|book| {
        book.live_hauls.iter().filter_map(|id| {
            book.hauls
                .get(id)
                .and_then(Held::present)
                .filter(|job| job.stage != HaulStage::Closed)
        })
    })
}

/// The haul of `contract`, when it reads.
#[must_use]
pub fn haul_of(w: &World, contract: u64) -> Option<HaulJob> {
    super::book(w)?.hauls.get(&contract)?.present().cloned()
}

/// Delete a haul record from RAM and persistent storage.
pub(crate) fn delete(ctx: &mut Ctx<'_>, contract: u64) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store.services.hauls.remove(&contract);
        store.services.live_hauls.remove(&contract);
        store.services.closed_hauls.remove(&contract);
    }
    crate::store::push_op(
        ctx.out,
        mush_core::ModuleWrite::Del {
            key: haul_key(contract).into(),
        },
    );
}

/// Delete oldest retained `Closed` hauls once closed count exceeds [`MAX_CLOSED_HAULS`].
pub(crate) fn prune_closed(ctx: &mut Ctx<'_>) {
    let excess: Vec<u64> = ctx
        .w
        .module_state::<TacsimStore>()
        .map(|store| {
            let over = store
                .services
                .closed_hauls
                .len()
                .saturating_sub(MAX_CLOSED_HAULS);
            store
                .services
                .closed_hauls
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
pub(crate) fn put(ctx: &mut Ctx<'_>, job: &HaulJob) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        if job.stage == HaulStage::Closed {
            store.services.live_hauls.remove(&job.contract);
            store.services.closed_hauls.insert(job.contract);
        } else {
            store.services.closed_hauls.remove(&job.contract);
            store.services.live_hauls.insert(job.contract);
        }
        store
            .services
            .hauls
            .insert(job.contract, Held::Present(Box::new(job.clone())));
    }
    crate::port::owned::write(ctx, job.key(), job);
    if job.stage == HaulStage::Closed {
        prune_closed(ctx);
    }
}

/// **The holder a job's cargo is reserved to**: a `Scoped` principal of this crate, class
/// `haul`, one per job, with a deterministic key, so every leg re-derives the hold the load
/// placed.
///
/// # Errors
///
/// A key that does not build.
pub fn haul_holder(contract: u64) -> Result<ScopedIdempotencyKey, String> {
    let owner = OwnerId::new(crate::NS).map_err(|e| e.to_string())?;
    let id = IdempotencyKeyText::parse_canonical(format!("haul{contract:016x}"))
        .map_err(|e| e.to_string())?;
    let principal = mush_core::PrincipalId::Scoped {
        owner: owner.clone(),
        class: CanonicalId::new("haul").map_err(|e| e.to_string())?,
        id,
    };
    let scope = IdempotencyScope {
        principal,
        owner,
        namespace: CanonicalId::new(crate::NS).map_err(|e| e.to_string())?,
    };
    let key = CanonicalHash::for_canonical_value(
        "tacsim.haul_holder.v1",
        1,
        &json!({ "haul": contract }),
        1024,
    )
    .map_err(|e| e.to_string())?;
    Ok(ScopedIdempotencyKey::new(
        scope,
        0,
        IdempotencyKey::Deterministic(key),
    ))
}

/// The hold the load placed in `hold` for job `contract`.
///
/// # Errors
///
/// A holder or a resource that does not derive.
pub fn haul_reservation(
    contract: u64,
    hold: u64,
    good: &CanonicalId,
) -> Result<ReservationId, String> {
    let holder = haul_holder(contract)?;
    let resource = mush_inventory::pool_resource(EndpointRef(VaultId(hold)), good)
        .map_err(|e| e.to_string())?;
    ReservationId::derive(&holder, &resource).map_err(|e| e.to_string())
}

/// A leg's marker id.
fn marker(at_ic: i64, causal: &Value) -> Option<SmolStr> {
    mush_inventory::derived_marker_id(at_ic, causal).ok()
}

/// The fact the contract names: the delivery's `inventory.transferred` marker.
fn fact(marker: &str) -> mush_economy::ExternalRef {
    mush_economy::ExternalRef {
        provider: mush_inventory::record::NS.into(),
        kind: mush_inventory::FACT_TRANSFERRED.into(),
        id: marker.into(),
        revision: None,
    }
}

fn goods_def<'a>(w: &'a World, word: &str) -> Option<&'a crate::definitions::goods::GoodDef> {
    let word = word.trim().to_ascii_lowercase();
    w.module_state::<TacsimStore>()?
        .goods()?
        .goods
        .iter()
        .find(|g| g.id.as_str() == word || g.label.eq_ignore_ascii_case(&word))
}

pub(crate) fn label(w: &World, good: &CanonicalId) -> String {
    goods_def(w, good.as_str()).map_or_else(|| good.to_string(), |g| g.label.clone())
}

// ---- the commands ---------------------------------------------------------------------------

/// **`+haul`'s spec**.
#[must_use]
pub(crate) fn haul_spec() -> mush_core::CommandSpec {
    mush_core::CommandSpec {
        name: HAUL_VERB.into(),
        shape: mush_core::ArgShape::Raw,
        switches: ["post", "accept", "deliver", "withdraw"]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
        help: HAUL_HELP.into(),
        ..mush_core::CommandSpec::default()
    }
}

/// **`+haul`** (doc 48 cut EX.6).
///
/// # Errors
///
/// The refusal.
pub(crate) fn haul_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    let raw = ctx.args.raw().to_owned();
    let (rest, go) = crate::surface::retrieval::split_go(&raw);
    let line = if ctx.switches.has("post") {
        post_cmd(ctx, &rest, go)
    } else if ctx.switches.has("accept") {
        accept_cmd(ctx, &rest, go)
    } else if ctx.switches.has("deliver") {
        deliver_cmd(ctx, &rest, go)
    } else if ctx.switches.has("withdraw") {
        withdraw_cmd(ctx, &rest, go)
    } else if rest.is_empty() {
        Ok(list(ctx))
    } else {
        Err(HAUL_USAGE.to_owned())
    }
    .map_err(CmdError::msg)?;
    ctx.out.sys_to_actor(ctx.actor, CHANNEL, line);
    Ok(())
}

fn list(ctx: &Ctx<'_>) -> String {
    let rows: Vec<String> = live_hauls(ctx.w)
        .filter(|j| {
            j.shipper == ctx.actor
                || j.pilot == Some(ctx.actor)
                || j.hauler
                    .as_ref()
                    .is_some_and(|h| crate::surface::retrieval::acts_for(ctx.w, ctx.actor, h))
        })
        .map(|j| {
            ROW.replace("{job}", &j.job())
                .replace("{qty}", &j.quantity.to_string())
                .replace("{good}", &label(ctx.w, &j.good))
                .replace("{origin}", &super::venue_name(ctx.w, &j.origin))
                .replace("{port}", &super::venue_name(ctx.w, &j.destination))
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

/// The reference cap on `quantity` of `good`, minor units (Q-48-57).
fn cap_of(w: &World, good: &CanonicalId, quantity: u64) -> Option<i64> {
    let unit = crate::definitions::mass::good_reference_minor(w, good.as_str());
    (unit > 0).then(|| i64::try_from(unit.saturating_mul(quantity)).unwrap_or(i64::MAX))
}

#[allow(clippy::too_many_lines)] // one ordered refusal list, then the post
/// **Post a haul as `ctx.actor`** (doc 50 cut OP.4: the Freeport arm's agent), `rest` as
/// `+haul/post` reads it: the new haul's contract.
///
/// # Errors
///
/// The line the verb would answer.
pub(crate) fn post_as(ctx: &mut Ctx<'_>, rest: &str) -> Result<u64, String> {
    post_cmd(ctx, rest, true)?;
    hauls(ctx.w)
        .into_iter()
        .filter(|j| j.shipper == ctx.actor)
        .map(|j| j.contract)
        .max()
        .ok_or_else(|| "the haul was not recorded".to_owned())
}

/// **Take a haul as `ctx.actor`** (doc 50 cut OP.4: a hauling house's helm, Q-50-24), `rest` as
/// `+haul/accept` reads it.
///
/// # Errors
///
/// The line the verb would answer.
pub(crate) fn accept_as(ctx: &mut Ctx<'_>, rest: &str) -> Result<String, String> {
    accept_cmd(ctx, rest, true)
}

/// **Deliver a haul as `ctx.actor`** (doc 50 cut OP.4: a hauling house's helm, Q-50-24).
///
/// # Errors
///
/// The line the verb would answer.
pub(crate) fn deliver_as(ctx: &mut Ctx<'_>, job: u64) -> Result<String, String> {
    deliver_cmd(ctx, &format!("#{job}"), true)
}

fn post_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let (words, opts) = super::options(rest);
    let [qty, good] = words.as_slice() else {
        return Err(HAUL_USAGE.to_owned());
    };
    let quantity = qty
        .parse::<u64>()
        .ok()
        .filter(|q| *q > 0)
        .ok_or_else(|| HAUL_USAGE.to_owned())?;
    let (good, good_label) = goods_def(ctx.w, good)
        .map(|def| (def.id.clone(), def.label.clone()))
        .ok_or_else(|| NO_GOOD.replace("{good}", good))?;
    let to = opts.get("to").ok_or_else(|| HAUL_USAGE.to_owned())?;
    let fee = opts
        .get("fee")
        .and_then(|f| super::credits(f))
        .ok_or_else(|| HAUL_USAGE.to_owned())?;
    let origin = super::standing_venue(ctx.w, ctx.actor).ok_or_else(|| NO_VENUE.to_owned())?;
    let destination = super::venue_word(ctx.w, to).ok_or_else(|| NO_PORT.replace("{port}", to))?;
    if origin == destination {
        return Err(SAME_PORT.to_owned());
    }
    // Doc 50 cut OP.3 (Q-50-23) & P2-TSW-23: authorize `as=<org>` before inspecting any
    // organization vault so `+haul/post` preview cannot act as an unauthenticated exact-inventory
    // oracle on another organization's storefront vault.
    let (poster, account) = super::poster(ctx, opts.get("as").map(String::as_str))?;
    let org = match &poster {
        PrincipalId::Organization(org) => Some(*org),
        _ => None,
    };
    let source = match org {
        Some(org) => super::org_vault(ctx.w, org, origin.as_str()).ok_or_else(|| {
            NO_ORG_VAULT.replace("{port}", &super::venue_name(ctx.w, origin.as_str()))
        })?,
        None => crate::port::locker::locker_of(ctx.w, ctx.actor, &origin)
            .map(|l| l.vault)
            .ok_or_else(|| NO_LOCKER.to_owned())?,
    };
    let org_drop = match org {
        Some(org) => {
            let into = match opts.get("into") {
                Some(name) => org_word(ctx, name)?,
                None => org,
            };
            Some(
                super::org_vault(ctx.w, into, destination.as_str()).ok_or_else(|| {
                    NO_ORG_VAULT.replace("{port}", &super::venue_name(ctx.w, destination.as_str()))
                })?,
            )
        }
        None => None,
    };
    let held = crate::port_tick::read_pool(ctx, EndpointRef(VaultId(source)), &good).unwrap_or(0);
    if held < quantity {
        return Err(SHORT.replace("{good}", &good_label));
    }
    let cap = cap_of(ctx.w, &good, quantity).ok_or_else(|| NO_REFERENCE.to_owned())?;
    let declared = match opts.get("value") {
        Some(v) => super::credits(v).ok_or_else(|| HAUL_USAGE.to_owned())?,
        None => cap,
    };
    if declared > cap {
        return Err(OVER_CAP.replace("{cap}", &money_minor(ctx, cap)));
    }
    let bond = bond_of(declared);
    let open_total = live_hauls(ctx.w).count();
    let open_actor = live_hauls(ctx.w).filter(|j| j.shipper == ctx.actor).count();
    if open_actor >= MAX_OPEN_HAULS_PER_ACTOR {
        return Err(TOO_MANY_OPEN.replace("{cap}", &MAX_OPEN_HAULS_PER_ACTOR.to_string()));
    }
    if open_total >= MAX_OPEN_HAULS_GLOBAL {
        return Err(BOARD_FULL.to_owned());
    }
    if !go {
        return Ok(POST_PREVIEW
            .replace("{qty}", &quantity.to_string())
            .replace("{good}", &good_label)
            .replace("{origin}", &super::venue_name(ctx.w, origin.as_str()))
            .replace("{port}", &super::venue_name(ctx.w, destination.as_str()))
            .replace("{fee}", &money_minor(ctx, fee))
            .replace("{value}", &money_minor(ctx, declared))
            .replace("{bond}", &money_minor(ctx, bond)));
    }
    let drop = match org_drop {
        Some(drop) => drop,
        None => {
            crate::port::locker::open(ctx, ctx.actor, &destination)?;
            crate::port::locker::locker_of(ctx.w, ctx.actor, &destination)
                .map(|l| l.vault)
                .ok_or_else(|| "Your locker at the destination could not be opened.".to_owned())?
        }
    };
    let peeked = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .map(mush_economy::EconomyStore::peek_next_contract)
        .ok_or_else(|| "The economy is not running.".to_owned())?;
    let now = ctx.w.world_ic_now().at;
    let delivery = marker(
        now,
        &json!({"kind": "tacsim.haul.deliver", "contract": peeked, "to": drop,
            "good": good.as_str(), "quantity": quantity}),
    )
    .ok_or_else(|| "The haul's marker could not be named.".to_owned())?;
    let window = ACCEPT_WINDOW_IC_DAYS.saturating_mul(DAY);
    let term = HAUL_TERM_IC_DAYS.saturating_mul(DAY);
    let contract = mush_economy::post_job(
        ctx,
        mush_economy::JobSpec {
            name: SmolStr::new(format!("Haul: {quantity} {good_label}")),
            poster,
            poster_account: account,
            fee: Amount::from_minor(fee),
            fact: fact(&delivery),
            deadline_ic: now.saturating_add(window).saturating_add(term),
        },
    )
    .map_err(|error| POST_FAILED.replace("{why}", &error.to_string()))?;
    if contract.0 != peeked {
        let _ = mush_economy::cancel_job(ctx, contract);
        return Err(POST_FAILED.replace("{why}", "the economy was busy; try again"));
    }
    let job = HaulJob {
        schema: HAUL_SCHEMA,
        contract: contract.0,
        shipper: ctx.actor,
        shipper_account: account.0,
        shipper_org: org.map(|o| o.0),
        origin: SmolStr::new(origin.as_str()),
        destination: SmolStr::new(destination.as_str()),
        source,
        drop,
        good,
        quantity,
        declared_minor: declared,
        fee_minor: fee,
        bond_minor: bond,
        posted_ic: now,
        accept_by_ic: now.saturating_add(window),
        marker: delivery,
        hauler: None,
        pilot: None,
        hull: None,
        hold: None,
        due_ic: None,
        owed: None,
        tries: 0,
        stage: HaulStage::Posted,
    };
    put(ctx, &job);
    Ok(POSTED
        .replace("{job}", &job.job())
        .replace("{qty}", &quantity.to_string())
        .replace("{good}", &good_label)
        .replace("{port}", &super::venue_name(ctx.w, destination.as_str()))
        .replace("{fee}", &money_minor(ctx, fee)))
}

/// The organization `name` names (`org:` optional), as the poster may see it.
fn org_word(ctx: &mut Ctx<'_>, name: &str) -> Result<mush_org::OrgId, String> {
    let name = name.strip_prefix("org:").unwrap_or(name);
    mush_economy::resolve_org_visible(ctx, name).ok_or_else(|| super::NOT_POSTER.to_owned())
}

fn job_word(word: &str) -> Option<u64> {
    word.trim().trim_start_matches('#').parse::<u64>().ok()
}

/// **Whether `hold` has room for `quantity` of `good`** (doc 50b; doc 50 §7's cargo reading):
/// the capacity plan over everything the hold holds, pooled or in lots
/// (`crate::port_tick::read_held`), so goods bought as an exact lot take room too. A hold with no
/// plan has room; one whose contents will not read has none.
fn hold_has_room(ctx: &Ctx<'_>, hold: VaultId, good: &CanonicalId, quantity: u64) -> bool {
    let hold_ep = EndpointRef(hold);
    let Some(plan) = PortCapacity.plan(ctx.w, hold_ep) else {
        return true;
    };
    let mut read = |g: &CanonicalId| {
        crate::port_tick::read_held(ctx, hold_ep, g).map_err(|_| mush_market::CapacityReadError)
    };
    PortCapacity.check(&plan, good, quantity, &mut read).is_ok()
}

#[allow(clippy::too_many_lines)] // one ordered refusal list, the join, then the load
fn accept_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let (words, opts) = super::options(rest);
    let [word] = words.as_slice() else {
        return Err(HAUL_USAGE.to_owned());
    };
    let mut job = job_word(word)
        .and_then(|c| haul_of(ctx.w, c))
        .filter(|j| j.stage == HaulStage::Posted)
        .ok_or_else(|| NO_SUCH_JOB.replace("{job}", word))?;
    if job.shipper == ctx.actor {
        return Err(OWN_JOB.to_owned());
    }
    let helm = crate::port::coop::helm_of(ctx.w, ctx.actor);
    let origin_name = super::venue_name(ctx.w, &job.origin);
    let hull = helm
        .map(|h| h.hull)
        .filter(|hull| {
            super::docked_venue(ctx.w, *hull).is_some_and(|v| v.as_str() == job.origin.as_str())
        })
        .ok_or_else(|| NOT_DOCKED.replace("{origin}", &origin_name))?;
    let hold = crate::port::ship::hull_view(ctx.w, hull)
        .and_then(|view| crate::port::ship::hold_vault(ctx.w, &view))
        .ok_or_else(|| NO_ROOM.to_owned())?;
    if !hold_has_room(ctx, hold, &job.good, job.quantity) {
        return Err(NO_ROOM.to_owned());
    }
    let held =
        crate::port_tick::read_pool(ctx, EndpointRef(VaultId(job.source)), &job.good).unwrap_or(0);
    if held < job.quantity {
        return Err(GOODS_GONE.to_owned());
    }
    let days = HAUL_TERM_IC_DAYS;
    if !go {
        return Ok(ACCEPT_PREVIEW
            .replace("{qty}", &job.quantity.to_string())
            .replace("{good}", &label(ctx.w, &job.good))
            .replace("{port}", &super::venue_name(ctx.w, &job.destination))
            .replace("{fee}", &money_minor(ctx, job.fee_minor))
            .replace("{value}", &money_minor(ctx, job.declared_minor))
            .replace("{bond}", &money_minor(ctx, job.bond_minor))
            .replace("{days}", &days.to_string()));
    }
    let (taker, account) = super::taker(ctx, opts.get("as").map(String::as_str))?;
    if taker == PrincipalId::Object(job.shipper)
        || job
            .shipper_org
            .is_some_and(|org| taker == PrincipalId::Organization(mush_org::OrgId(org)))
    {
        return Err(OWN_JOB.to_owned());
    }
    let term = HAUL_TERM_IC_DAYS.saturating_mul(DAY);
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
    let load = marker(
        now,
        &json!({"kind": "tacsim.haul.load", "contract": job.contract, "hold": hold.0}),
    )
    .ok_or_else(|| "The haul's marker could not be named.".to_owned())?;
    job.hauler = Some(taker.clone());
    job.pilot = Some(ctx.actor);
    job.hull = Some(hull);
    job.hold = Some(hold.0);
    job.due_ic = Some(now.saturating_add(term));
    job.owed = Some(HaulLeg {
        act: 1,
        kind: LegKind::Load,
        by: ctx.actor,
        from: job.source,
        to: hold.0,
        good: job.good.clone(),
        quantity: job.quantity,
        marker: load,
        by_ic: None,
        lead_ic: term.saturating_add(RETURN_GRACE_IC_DAYS.saturating_mul(DAY)),
    });
    job.tries = 0;
    job.stage = HaulStage::Taken;
    submit(ctx, &mut job);
    put(ctx, &job);
    let who = crate::surface::retrieval::principal_name(ctx.w, &taker);
    let told = TOLD_TAKEN
        .replace("{who}", &who)
        .replace("{job}", &job.job());
    super::tell(ctx, &PrincipalId::Object(job.shipper), CHANNEL, &told);
    Ok(ACCEPTED
        .replace("{job}", &job.job())
        .replace("{days}", &days.to_string()))
}

/// **`+haul/deliver <job> [go]`**: the delivery verb.
fn deliver_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let word = rest.trim();
    let mut job = job_word(word)
        .and_then(|c| haul_of(ctx.w, c))
        .filter(|j| j.pilot == Some(ctx.actor) && j.stage == HaulStage::Aboard)
        .ok_or_else(|| NOT_ABOARD.replace("{job}", word))?;
    let (Some(hull), Some(hold), Some(due)) = (job.hull, job.hold, job.due_ic) else {
        return Err(NOT_ABOARD.replace("{job}", word));
    };
    let port = super::venue_name(ctx.w, &job.destination);
    if !super::docked_venue(ctx.w, hull).is_some_and(|v| v.as_str() == job.destination.as_str()) {
        return Err(NOT_THERE.replace("{port}", &port));
    }
    if ctx.w.world_ic_now().at >= due {
        return Err(TOO_LATE.to_owned());
    }
    if !go {
        return Ok(DELIVER_PREVIEW
            .replace("{qty}", &job.quantity.to_string())
            .replace("{good}", &label(ctx.w, &job.good))
            .replace("{port}", &port));
    }
    job.owed = Some(HaulLeg {
        act: 2,
        kind: LegKind::Deliver,
        by: ctx.actor,
        from: hold,
        to: job.drop,
        good: job.good.clone(),
        quantity: job.quantity,
        marker: job.marker.clone(),
        by_ic: Some(due),
        lead_ic: 0,
    });
    job.tries = 0;
    submit(ctx, &mut job);
    put(ctx, &job);
    Ok(DELIVERING.replace("{job}", &job.job()))
}

/// Whether `actor` may withdraw `job` (`P2-TSW-48`): for an organization-owned job, live
/// [`super::JOBS_POST_CAPABILITY`] on the posting organization; for a personal job, the shipper.
fn may_withdraw(w: &World, actor: ObjId, job: &HaulJob) -> bool {
    match job.shipper_org {
        Some(org) => super::may_post_for_org(w, actor, mush_org::OrgId(org)),
        None => job.shipper == actor,
    }
}

fn withdraw_cmd(ctx: &mut Ctx<'_>, rest: &str, go: bool) -> Result<String, String> {
    let word = rest.trim();
    let job = job_word(word)
        .and_then(|c| haul_of(ctx.w, c))
        .filter(|j| j.stage == HaulStage::Posted && may_withdraw(ctx.w, ctx.actor, j))
        .ok_or_else(|| NOT_WITHDRAWABLE.to_owned())?;
    if !go {
        return Ok(format!(
            "{} +haul/withdraw {} go withdraws it.",
            HaulStage::Posted.word(),
            job.job()
        ));
    }
    mush_economy::cancel_job(ctx, ContractId(job.contract))
        .map_err(|error| POST_FAILED.replace("{why}", &error.to_string()))?;
    let job_word = job.job();
    delete(ctx, job.contract);
    Ok(WITHDRAWN.replace("{job}", &job_word))
}

// ---- the arm --------------------------------------------------------------------------------

/// Submit `job`'s owed leg as its `by`, counting the try. `false` when nothing was submitted.
fn submit(ctx: &mut Ctx<'_>, job: &mut HaulJob) -> bool {
    let Some(owed) = job.owed.clone() else {
        return false;
    };
    let Some(action) = ctx
        .w
        .module_state::<HaulHandles>()
        .and_then(|h| h.action.clone())
    else {
        return false;
    };
    if ctx.out.authoritative_pending(owed.by) {
        return false;
    }
    let Ok(key) = IdempotencyKeyText::parse_canonical(format!(
        "tacsim.haul/{}/{}/{}",
        job.contract, owed.act, job.tries
    )) else {
        return false;
    };
    job.tries = job.tries.saturating_add(1);
    let saved = ctx.actor;
    ctx.actor = owed.by;
    ctx.submit_authoritative(
        action,
        PerceivedRef::SelfActor,
        vec![],
        json!({"contract": job.contract, "act": owed.act}),
        key,
    );
    ctx.actor = saved;
    true
}

/// The last leg job `contract` landed.
fn landed(ctx: &Ctx<'_>, contract: u64) -> Option<HaulLedger> {
    let handles = ctx.w.module_state::<HaulHandles>()?;
    match ctx.action_state(&handles.ledgers, &contract) {
        Ok(Some(ledger)) if ledger.contract == contract => Some(ledger.clone()),
        _ => None,
    }
}

/// Whether `job` needs a mutable step on this fire (read-only check by ref, zero cloning).
fn needs_step(ctx: &Ctx<'_>, job: &HaulJob) -> bool {
    if job.stage == HaulStage::Closed {
        return false;
    }
    let ledger = landed(ctx, job.contract);
    if ledger
        .as_ref()
        .is_some_and(|l| matches!(l.last, LegKind::Seized | LegKind::Lost))
    {
        return true;
    }
    let now = ctx.w.world_ic_now().at;
    let state = super::contract_state(ctx.w, job.contract);
    match job.stage {
        HaulStage::Posted => now > job.accept_by_ic || !state.as_ref().is_some_and(|s| s.is_live()),
        HaulStage::Taken | HaulStage::Returning => true,
        HaulStage::Aboard => {
            job.owed.is_some()
                || job.hull.is_some_and(|hull| {
                    super::escort::wrecked(ctx.w, hull) || super::escort::captured(ctx.w, hull)
                })
                || job.due_ic.is_none_or(|due| now >= due)
                || state.as_ref().is_some_and(|s| s.is_terminal())
        }
        HaulStage::Delivered => state.as_ref().is_none_or(|s| s.is_terminal()),
        HaulStage::Closed => false,
    }
}

/// **The port arm** (doc 48 cut EX.6): each live haul's next step.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    prune_closed(ctx);
    let due: Vec<u64> = live_hauls(ctx.w)
        .filter(|job| needs_step(ctx, job))
        .map(|job| job.contract)
        .collect();
    for contract in due {
        let Some(mut job) = haul_of(ctx.w, contract) else {
            continue;
        };
        if step(ctx, &mut job) {
            put(ctx, &job);
        }
    }
}

/// One job's step; `true` when the record changed.
#[allow(clippy::too_many_lines)] // one job through its stages, in order
fn step(ctx: &mut Ctx<'_>, job: &mut HaulJob) -> bool {
    let now = ctx.w.world_ic_now().at;
    let state = super::contract_state(ctx.w, job.contract);
    let ledger = landed(ctx, job.contract);
    let acts = ledger.as_ref().map_or(0, |l| l.acts);
    if job.stage != HaulStage::Closed && ledger.as_ref().is_some_and(|l| l.last == LegKind::Seized)
    {
        return seized(ctx, job);
    }
    // Doc 50 cut OP.3 (Q-50-25): the hull went down with the goods; the leg released the hold.
    if job.stage != HaulStage::Closed && ledger.as_ref().is_some_and(|l| l.last == LegKind::Lost) {
        return lost(ctx, job);
    }
    match job.stage {
        HaulStage::Posted => {
            if now <= job.accept_by_ic && state.as_ref().is_some_and(|s| s.is_live()) {
                return false;
            }
            let _ = mush_economy::cancel_job(ctx, ContractId(job.contract));
            job.stage = HaulStage::Closed;
            true
        }
        HaulStage::Taken => {
            if acts >= 1 {
                job.owed = None;
                job.tries = 0;
                job.stage = HaulStage::Aboard;
                return true;
            }
            // Doc 50b (doc 50 §7's cargo reading): the load re-reads the hold's room before each
            // try, so a hold filled since the haul was taken never takes the goods.
            let no_room = job.owed.as_ref().is_some_and(|o| {
                o.kind == LegKind::Load && !hold_has_room(ctx, VaultId(o.to), &o.good, o.quantity)
            });
            if no_room
                || job.tries >= MAX_LEG_TRIES
                || state.as_ref().is_none_or(|s| s.is_terminal())
            {
                if no_room && let Some(pilot) = job.pilot {
                    let line = TOLD_NO_ROOM.replace("{job}", &job.job());
                    super::tell(ctx, &PrincipalId::Object(pilot), CHANNEL, &line);
                }
                // The goods never loaded: nothing is aboard, so nothing returns. A job still live
                // is cancelled, the fee back to the shipper and the bond to the hauler.
                let _ = mush_economy::cancel_job(ctx, ContractId(job.contract));
                job.owed = None;
                job.stage = HaulStage::Closed;
                return true;
            }
            submit(ctx, job)
        }
        HaulStage::Aboard => {
            // Doc 50b (OP.3): a delivery the ledger already committed is acknowledged first, so a
            // hull lost after it docked owes no lost leg the ledger would refuse at that ordinal.
            if job.owed.as_ref().is_some_and(|o| {
                o.kind == LegKind::Deliver
                    && acts >= o.act
                    && ledger.as_ref().is_some_and(|l| l.last == LegKind::Deliver)
            }) {
                return delivered(ctx, job);
            }
            // Doc 50 cut OP.3 (Q-50-25): her hull wrecked or captured with the goods aboard owes
            // the lost leg, which releases the hold.
            if job.owed.as_ref().is_none_or(|o| o.kind != LegKind::Lost)
                && job.hull.is_some_and(|hull| {
                    super::escort::wrecked(ctx.w, hull) || super::escort::captured(ctx.w, hull)
                })
            {
                return owe_lost(ctx, job, now);
            }
            if job.owed.as_ref().is_some_and(|o| o.kind == LegKind::Lost) {
                return job.tries < MAX_LEG_TRIES && submit(ctx, job);
            }
            if let Some(owed) = job.owed.clone() {
                if acts >= owed.act && ledger.as_ref().is_some_and(|l| l.last == LegKind::Deliver) {
                    return delivered(ctx, job);
                }
                if job.tries < MAX_LEG_TRIES && owed.by_ic.is_none_or(|by| now < by) {
                    return submit(ctx, job);
                }
                job.owed = None;
                job.tries = 0;
                return true;
            }
            let lapsed = job.due_ic.is_none_or(|due| now >= due)
                || state.as_ref().is_some_and(|s| s.is_terminal());
            if !lapsed {
                return false;
            }
            job.stage = HaulStage::Returning;
            true
        }
        HaulStage::Returning => returning(ctx, job, acts, ledger.as_ref()),
        HaulStage::Delivered => {
            if state.as_ref().is_none_or(|s| s.is_terminal()) {
                job.stage = HaulStage::Closed;
                return true;
            }
            false
        }
        HaulStage::Closed => false,
    }
}

/// **A seizure fails the job** (doc 50 unit CR; Q-50-25 as corrected): the customs ruling took
/// the cargo and wrote [`LegKind::Seized`] on the ledger; the arm lapses the contract now, as its
/// deadline would (the fee back to the shipper and the bond to them, Q-48-42), and closes the
/// record. A contract that will not lapse here (a frozen economy) still lapses at its deadline.
fn seized(ctx: &mut Ctx<'_>, job: &mut HaulJob) -> bool {
    if let Some(hull) = job.hull {
        loss_moves_the_other_side(ctx, hull, job.contract, &job.destination, "seized");
    }
    let _ = mush_economy::lapse_job(ctx, ContractId(job.contract));
    job.owed = None;
    job.tries = 0;
    job.stage = HaulStage::Closed;
    let line = TOLD_SEIZED.replace("{job}", &job.job());
    super::tell(ctx, &PrincipalId::Object(job.shipper), CHANNEL, &line);
    if let Some(pilot) = job.pilot {
        let line = TOLD_HAULER_SEIZED.replace("{job}", &job.job());
        super::tell(ctx, &PrincipalId::Object(pilot), CHANNEL, &line);
    }
    true
}

/// The cause-id prefix of a side's loss (Q-50-25, Q-50-51); the contract follows it.
pub const LOSS_CAUSE: &str = "haul.loss/";

/// **A loss moves the other side's clock** (doc 50b; Q-50-25 as corrected, Q-50-51): when the
/// hull carrying a haul flies for a side of the operation in its window
/// (`crate::space::op_sides::side_of`) and its cargo is seized or lost, the other side's front
/// clock advances one segment by a typed cause keyed by the contract, so a retry counts once. The
/// Republic's clock is the Concession Review and the Freeport's is Hold the Concession; a clock
/// already full, or one not seeded, moves no further.
pub fn loss_moves_the_other_side(
    ctx: &mut Ctx<'_>,
    hull: ObjId,
    contract: u64,
    destination: &str,
    what: &str,
) {
    let Some(status) = crate::space::op_sides::carriers_contested(ctx.w) else {
        return;
    };
    let Some(side) = crate::space::op_sides::side_of(ctx.w, &status, hull) else {
        return;
    };
    let Some((other, _)) = status
        .sides
        .iter()
        .enumerate()
        .find(|(i, _)| *i != side)
        .map(|(_, s)| s.clone())
    else {
        return;
    };
    let clock = match other.as_str() {
        "republic" => crate::concession::review_clock(ctx.w),
        "freeport" => crate::concession::hold_clock(ctx.w),
        _ => None,
    };
    let Some(clock) = clock.filter(|c| !c.completed) else {
        return;
    };
    let mine = status.sides.get(side).map_or("", |(tag, _)| tag.as_str());
    let line = crate::concession::draft(
        ctx,
        "op.haul_loss",
        format!(
            "A haul for {} was {what} on the way to {}; {} advances.",
            crate::polity::polity_name(mine),
            super::venue_name(ctx.w, destination),
            clock.title
        ),
        format!("{LOSS_CAUSE}{contract}"),
    );
    if matches!(
        mush_faction::advance_clock_by_cause(ctx, clock.id, 1, line),
        Ok(mush_faction::ClockAdvance::Advanced { .. })
    ) {
        ctx.out.pin_deferred();
    }
}

/// **The delivery is acknowledged**: the leg committed, so the job is delivered and its shipper
/// told.
fn delivered(ctx: &mut Ctx<'_>, job: &mut HaulJob) -> bool {
    job.owed = None;
    job.tries = 0;
    job.stage = HaulStage::Delivered;
    let line = TOLD_DELIVERED
        .replace("{job}", &job.job())
        .replace("{port}", &super::venue_name(ctx.w, &job.destination));
    super::tell(ctx, &PrincipalId::Object(job.shipper), CHANNEL, &line);
    true
}

/// **Owe the lost leg** (doc 50 cut OP.3): the hold released, nothing moved.
fn owe_lost(ctx: &mut Ctx<'_>, job: &mut HaulJob, now: i64) -> bool {
    let (Some(hold), Some(pilot)) = (job.hold, job.pilot) else {
        return false;
    };
    let Some(marker) = marker(
        now,
        &json!({"kind": "tacsim.haul.lost", "contract": job.contract, "hold": hold}),
    ) else {
        return false;
    };
    job.owed = Some(HaulLeg {
        act: 2,
        kind: LegKind::Lost,
        by: pilot,
        from: hold,
        to: hold,
        good: job.good.clone(),
        quantity: job.quantity,
        marker,
        by_ic: None,
        lead_ic: 0,
    });
    job.tries = 0;
    submit(ctx, job);
    true
}

/// **A lost hull fails the job** (doc 50 cut OP.3; Q-50-25): the lost leg released the hold, so
/// the goods are part of the wreck; the arm lapses the contract as a seizure does (the fee back to
/// the shipper and the bond to them) and closes the record.
fn lost(ctx: &mut Ctx<'_>, job: &mut HaulJob) -> bool {
    if let Some(hull) = job.hull {
        loss_moves_the_other_side(ctx, hull, job.contract, &job.destination, "lost");
    }
    let _ = mush_economy::lapse_job(ctx, ContractId(job.contract));
    job.owed = None;
    job.tries = 0;
    job.stage = HaulStage::Closed;
    let line = TOLD_LOST.replace("{job}", &job.job());
    super::tell(ctx, &PrincipalId::Object(job.shipper), CHANNEL, &line);
    if let Some(pilot) = job.pilot {
        let line = TOLD_HAULER_LOST.replace("{job}", &job.job());
        super::tell(ctx, &PrincipalId::Object(pilot), CHANNEL, &line);
    }
    true
}

/// The return: at the hull's next port, the goods go back to the shipper's locker there.
fn returning(ctx: &mut Ctx<'_>, job: &mut HaulJob, acts: u32, ledger: Option<&HaulLedger>) -> bool {
    if acts >= 2 && ledger.is_some_and(|l| l.last == LegKind::Return) {
        let port = job
            .owed
            .as_ref()
            .and_then(|o| locker_venue(ctx.w, job.shipper, o.to))
            .or_else(|| {
                job.hull
                    .and_then(|h| super::docked_venue(ctx.w, h))
                    .map(|v| SmolStr::new(v.as_str()))
            })
            .map_or_else(String::new, |v| super::venue_name(ctx.w, &v));
        job.owed = None;
        job.stage = HaulStage::Closed;
        let line = TOLD_RETURNED
            .replace("{job}", &job.job())
            .replace("{port}", &port);
        super::tell(ctx, &PrincipalId::Object(job.shipper), CHANNEL, &line);
        return true;
    }
    let (Some(hull), Some(hold), Some(pilot)) = (job.hull, job.hold, job.pilot) else {
        job.stage = HaulStage::Closed;
        return true;
    };
    if job.owed.is_some() && job.tries < MAX_LEG_TRIES {
        return submit(ctx, job);
    }
    let Some(venue) = super::docked_venue(ctx.w, hull) else {
        return false;
    };
    // Doc 50 cut OP.3: an organization's goods go back to its vault there, when it keeps one.
    let to = if let Some(org) = job.shipper_org {
        let Some(to) = super::org_vault(ctx.w, mush_org::OrgId(org), venue.as_str()) else {
            return false;
        };
        to
    } else {
        if crate::port::locker::open(ctx, job.shipper, &venue).is_err() {
            return false;
        }
        let Some(to) = crate::port::locker::locker_of(ctx.w, job.shipper, &venue).map(|l| l.vault)
        else {
            return false;
        };
        to
    };
    let now = ctx.w.world_ic_now().at;
    let Some(back) = marker(
        now,
        &json!({"kind": "tacsim.haul.return", "contract": job.contract, "to": to}),
    ) else {
        return false;
    };
    job.owed = Some(HaulLeg {
        act: 2,
        kind: LegKind::Return,
        by: pilot,
        from: hold,
        to,
        good: job.good.clone(),
        quantity: job.quantity,
        marker: back,
        by_ic: None,
        lead_ic: 0,
    });
    job.tries = 0;
    submit(ctx, job);
    true
}

/// The venue of `player`'s locker whose vault is `vault`.
fn locker_venue(w: &World, player: ObjId, vault: u64) -> Option<SmolStr> {
    w.module_state::<TacsimStore>()?
        .lockers
        .values()
        .find(|l| l.player == player && l.vault == vault)
        .map(|l| l.venue.clone())
}

// ---- the board ------------------------------------------------------------------------------

/// **The hauls on `venue`'s board** (Q-48-12): each posted job, on its origin's board.
#[must_use]
pub fn services(w: &World, venue: &MarketId) -> Vec<mush_market::Service> {
    let instrument = mush_economy::InstrumentId::new(crate::economy::CREDIT);
    live_hauls(w)
        .filter(|j| j.stage == HaulStage::Posted && j.origin.as_str() == venue.as_str())
        .map(|j| mush_market::Service {
            kind: mush_market::ServiceKind::Haul,
            side: mush_market::ServiceSide::Wanted,
            venue: venue.clone(),
            poster: PrincipalId::Object(j.shipper),
            poster_name: w
                .get(j.shipper)
                .map_or_else(|| SmolStr::new("someone"), |o| o.name.clone().into()),
            subject: format!(
                "carry {} {} to {}{}{}",
                j.quantity,
                label(w, &j.good),
                super::venue_name(w, &j.destination),
                // Doc 50 cut KC.3 (Q-50-8): the duty its hauler will meet at Kirkwood.
                crate::space::duty::haul_clause(
                    w,
                    j.origin.as_str(),
                    j.destination.as_str(),
                    j.quantity.saturating_mul(
                        w.module_state::<crate::store::TacsimStore>()
                            .and_then(crate::store::TacsimStore::goods)
                            .and_then(|g| g.good(&j.good))
                            .map_or(0, |g| g.reference_major)
                    )
                ),
                // Doc 50 cut OP.7 (Q-50-30): the operation's work says so on the board.
                crate::op_page::job_tag(w, j.contract)
            )
            .into(),
            pay: Amount::from_minor(j.fee_minor),
            instrument: instrument.clone(),
            negotiable: false,
            bond: (j.bond_minor > 0).then(|| Amount::from_minor(j.bond_minor)),
            term: mush_market::ServiceTerm::FromAcceptance {
                ic_secs: HAUL_TERM_IC_DAYS.saturating_mul(DAY),
            },
            contract: Some(ContractId(j.contract)),
            take: format!("+haul/accept {}", j.job()).into(),
        })
        .collect()
}

// ---- the provider ---------------------------------------------------------------------------

/// The provider's handles, for the commands and the arm.
#[derive(Clone, Debug)]
pub(crate) struct HaulHandles {
    pub action: Option<DefinitionRef>,
    pub ledgers: StateFamilyHandle<u64, HaulLedger>,
}

/// **`tacsim_haul`**: one leg of one job, once.
struct HaulProvider {
    action: DefinitionRef,
    outcome: DefinitionRef,
    ledgers: StateFamilyHandle<u64, HaulLedger>,
    inventory: InventoryFamilies,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "the haul leg is self-scoped, takes no targets and names one job and its leg",
    )
}

fn parameters(
    invocation: &ActionInvocation,
) -> Result<(&Map<String, Value>, u64, u32), mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let parameters = invocation.parameters().as_object().ok_or_else(invalid)?;
    let contract = parameters
        .get("contract")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    let act = parameters
        .get("act")
        .and_then(Value::as_u64)
        .and_then(|a| u32::try_from(a).ok())
        .filter(|a| *a > 0)
        .ok_or_else(invalid)?;
    Ok((parameters, contract, act))
}

/// The legs `actor` owes, keyed by job: derived from the store at projection, never from the
/// submitter.
fn owed_rows(w: &World, actor: ObjId) -> Map<String, Value> {
    let mut rows = Map::new();
    for job in live_hauls(w) {
        if rows.len() >= MAX_OFFER_ROWS {
            break;
        }
        let Some(owed) = job.owed.as_ref().filter(|o| o.by == actor) else {
            continue;
        };
        if let Ok(value) = serde_json::to_value(owed) {
            rows.insert(job.contract.to_string(), value);
        }
    }
    rows
}

impl ActionProvider for HaulProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let rows = owed_rows(&engine.world, query.actor());
        if rows.is_empty() {
            return Ok(None);
        }
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the haul's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the haul's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the haul's definition guard is unavailable"))?;
        let integer = |id: &str,
                       minimum: i64,
                       maximum: i64|
         -> Result<ProjectedParameter, mush_core::ActionError> {
            Ok(ProjectedParameter {
                id: CanonicalId::new(id)
                    .map_err(|_| integrity("the haul's parameter id is invalid"))?,
                required: true,
                kind: ProjectedParameterKind::Integer { minimum, maximum },
            })
        };
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({
                "rows": Value::Object(rows),
                "now": engine.world.world_ic_now(),
                "wall": engine.world.now(),
            }),
            parameters: ProjectedParameterSchema::new(vec![
                // Doc 50 cut OP.4: a world's first contract is `ContractId(0)`, so a haul's
                // number may be 0 (the Freeport arm's first post on a fresh world is one).
                integer("contract", 0, i64::MAX)?,
                integer("act", 1, 2)?,
            ])
            .map_err(|_| integrity("the haul's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-haul",
                "Haul leg",
                "Load, deliver or return a haul's goods, held for the job.",
            )
            .map_err(|_| integrity("the haul's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the haul's offer guards are invalid"))?,
        }))
    }

    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        // 1. The leg the offer carried, the actor's own.
        let (parameters, contract, act) = parameters(invocation)?;
        let owed: HaulLeg = parameters
            .get("rows")
            .and_then(Value::as_object)
            .and_then(|rows| rows.get(&contract.to_string()))
            .cloned()
            .and_then(|row| serde_json::from_value(row).ok())
            .ok_or_else(invalid)?;
        let actor = invocation.actor();
        if owed.by != actor || owed.act != act || owed.quantity == 0 {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that haul leg is not yours to submit",
            ));
        }
        let now: ClockStamp = parameters
            .get("now")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .ok_or_else(invalid)?;
        let wall = parameters
            .get("wall")
            .and_then(Value::as_i64)
            .ok_or_else(invalid)?;
        // 2. The replay guard: only the ledger's next ordinal, and a delivery or a return only
        // after the load.
        let prior = txn
            .get(&self.ledgers, &contract)
            .map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the haul ledgers are unavailable",
                )
            })?
            .cloned();
        if prior.as_ref().is_some_and(|l| l.contract != contract) {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "a haul ledger is stored under another job's key",
            ));
        }
        let next = prior.as_ref().map_or(1, |l| l.acts.saturating_add(1));
        let in_order = match owed.kind {
            LegKind::Load => act == 1,
            LegKind::Deliver | LegKind::Return | LegKind::Lost => act == 2,
            // Doc 50 unit CR: a seizure is written by the customs ruling, never owed as a leg.
            LegKind::Seized => false,
        };
        if next != act || !in_order {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that haul leg has already landed",
            ));
        }
        // 3. A delivery lands within its term.
        if owed.by_ic.is_some_and(|by| now.at >= by) {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the haul's term has ended",
            ));
        }
        // 4. The move, under the job's hold.
        let refused =
            |message: &'static str| port_error(ActionErrorCode::ProviderRejected, message);
        let holder =
            haul_holder(contract).map_err(|_| refused("the haul's holder will not form"))?;
        // Doc 50 cut OP.3 (Q-50-25): the lost leg releases the job's hold and moves nothing; a
        // hold already gone (lapsed) leaves nothing to release.
        if owed.kind == LegKind::Lost {
            let hold = EndpointRef(VaultId(owed.from));
            let reservation = haul_reservation(contract, owed.from, &owed.good)
                .map_err(|_| refused("the haul's hold will not form"))?;
            let record = self
                .inventory
                .reservation(txn, hold, &owed.good, reservation)
                .map_err(|_| refused("the haul's hold could not be read"))?;
            if let Some(record) = record.filter(|r| r.held_amount().is_some()) {
                self.inventory
                    .release(txn, hold, &owed.good, reservation, record.revision())
                    .map_err(|_| refused("the haul's hold could not be released"))?;
            }
        }
        let held = match owed.kind {
            LegKind::Load | LegKind::Seized | LegKind::Lost => None,
            LegKind::Deliver | LegKind::Return => Some(
                haul_reservation(contract, owed.from, &owed.good)
                    .map_err(|_| refused("the haul's hold will not form"))?,
            ),
        };
        let keep = match owed.kind {
            LegKind::Load => Some((&holder, owed.lead_ic.max(0))),
            LegKind::Deliver | LegKind::Return | LegKind::Seized | LegKind::Lost => None,
        };
        let commit = CommitToken::from_bytes(*invocation.request().as_bytes());
        if owed.kind != LegKind::Lost {
            self.inventory
                .carry(
                    txn,
                    &mush_inventory::Carriage {
                        from: EndpointRef(VaultId(owed.from)),
                        to: EndpointRef(VaultId(owed.to)),
                        commodity: &owed.good,
                        amount: owed.quantity,
                        held,
                        keep,
                        marker: &owed.marker,
                        actor,
                        now: &now,
                        wall,
                        commit,
                    },
                )
                .map_err(|_| refused("the haul's goods could not be moved"))?;
        }
        // 5. The ledger, and the outcome.
        let ledger = HaulLedger {
            contract,
            acts: act,
            last: owed.kind,
        };
        let staged = if prior.is_some() {
            txn.replace(&self.ledgers, &contract, ledger)
        } else {
            txn.insert(&self.ledgers, &contract, ledger)
        };
        staged
            .map_err(|_| port_error(ActionErrorCode::Integrity, "the haul ledger did not stage"))?;
        let outcome = RulesValue::new(
            self.outcome.clone(),
            json!({"actor": actor.0, "contract": contract, "act": act}),
        )
        .map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the haul leg's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

fn descriptor(kind: &str, id: &str, payload: Value) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new(id).map_err(|e| ModError::msg(e.to_string()))?,
        1,
        payload,
    )
    .map_err(|e| ModError::msg(e.to_string()))
}

/// The legs' action descriptor.
///
/// # Errors
///
/// A descriptor that does not build.
pub(crate) fn action_descriptor(
    outcomes: &BTreeMap<String, DefinitionRef>,
) -> Result<DefinitionDescriptor, ModError> {
    outcomes
        .get(HAUL_OUTCOME)
        .ok_or_else(|| ModError::msg("the haul's outcome did not register"))?;
    descriptor(
        "action",
        HAUL_ACTION,
        json!({"format": 1, "protocol": "haul_leg_v1", "subject": "self_actor", "targets": 0,
            "legs": ["load_held", "deliver_marked", "return_marked"]}),
    )
}

/// The legs' outcome descriptor.
///
/// # Errors
///
/// A descriptor that does not build.
pub(crate) fn outcome_descriptor() -> Result<DefinitionDescriptor, ModError> {
    descriptor("outcome", HAUL_OUTCOME, json!({"format": 1}))
}

/// **Register the haul** (doc 48 cut EX.6): its ledger family always, and its provider when
/// the inventory cell is filled (the works rule). Answers the ledger family's handle: a customs
/// seizure that takes a job's cargo fails the job on its ledger in the seizure's own transaction
/// (doc 50 unit CR).
///
/// # Errors
///
/// A family or a provider that does not register.
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    actions: &BTreeMap<String, DefinitionRef>,
    outcomes: &BTreeMap<String, DefinitionRef>,
    inventory: Option<&InventoryFamilies>,
) -> Result<StateFamilyHandle<u64, HaulLedger>, ModError> {
    use crate::port::families::family_descriptor;
    let ledgers = reg.state_family::<u64, HaulLedger, _, _, _, _>(
        family_descriptor(
            HAUL_FAMILY,
            "tacsim.haul_ledger_key.v1",
            "tacsim.haul_ledger",
        )?,
        |key: &u64| Ok(key.to_string()),
        |value: &Value| serde_json::from_value(value.clone()).map_err(|e| e.to_string()),
        |value: &HaulLedger| {
            if (1..=2).contains(&value.acts) {
                Ok(())
            } else {
                Err("a haul ledger records one or two landed legs".to_owned())
            }
        },
        |value: &HaulLedger| serde_json::to_value(value).map_err(|e| e.to_string()),
    )?;
    let action = match (actions.get(HAUL_ACTION), inventory) {
        (Some(action), Some(inventory)) => {
            let outcome = outcomes
                .get(HAUL_OUTCOME)
                .cloned()
                .ok_or_else(|| ModError::msg("the haul's outcome did not register"))?;
            reg.action_provider(
                action.clone(),
                HaulProvider {
                    action: action.clone(),
                    outcome,
                    ledgers: ledgers.clone(),
                    inventory: inventory.clone(),
                },
            )?;
            Some(action.clone())
        }
        _ => None,
    };
    reg.state(HaulHandles {
        action,
        ledgers: ledgers.clone(),
    });
    Ok(ledgers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> HaulJob {
        HaulJob {
            schema: HAUL_SCHEMA,
            contract: 7,
            shipper: ObjId(5),
            shipper_account: 1,
            shipper_org: None,
            origin: "ceres".into(),
            destination: "vesta".into(),
            source: 10,
            drop: 11,
            good: CanonicalId::new("metals").expect("good"),
            quantity: 4,
            declared_minor: 40_000,
            fee_minor: 5_000,
            bond_minor: 10_000,
            posted_ic: 1,
            accept_by_ic: 2,
            marker: marker(1, &json!({"t": 1})).expect("marker"),
            hauler: None,
            pilot: None,
            hull: None,
            hold: None,
            due_ic: None,
            owed: None,
            tries: 0,
            stage: HaulStage::Posted,
        }
    }

    /// **Doc 48 cut EX.6**: the bond is a quarter of the declared value, and the record refuses
    /// one that is not, a haul to its own port, and a malformed marker.
    #[test]
    fn ex6_a_haul_record_keeps_its_own_rules() {
        assert_eq!(bond_of(40_000), 10_000);
        assert_eq!(bond_of(3), 0);
        assert!(job().check().is_ok());
        let mut bad = job();
        bad.bond_minor = 8_000;
        assert!(bad.check().is_err(), "the bond is a quarter");
        let mut bad = job();
        bad.destination = "ceres".into();
        assert!(bad.check().is_err(), "a haul goes elsewhere");
        let mut bad = job();
        bad.marker = "not hex".into();
        assert!(bad.check().is_err(), "the marker is a marker id");
        let holder = haul_holder(7).expect("holder");
        assert_eq!(holder, haul_holder(7).expect("holder"), "one holder a job");
        assert_ne!(holder, haul_holder(8).expect("holder"));
    }

    #[test]
    fn high41_closed_hauls_are_pruned_and_live_iter_skips_closed() {
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

        // Insert MAX_CLOSED_HAULS + 5 closed rows plus 2 live rows.
        for id in 1..=(MAX_CLOSED_HAULS as u64 + 5) {
            let mut row = job();
            row.contract = id;
            row.stage = HaulStage::Closed;
            put(&mut ctx, &row);
        }
        for id in 1_000..=1_001 {
            let mut row = job();
            row.contract = id;
            row.stage = HaulStage::Posted;
            put(&mut ctx, &row);
        }

        // Oldest 5 closed rows were deleted from RAM and emitted ModuleWrite::Del.
        let store = ctx.w.module_state::<TacsimStore>().expect("store");
        assert_eq!(store.services.closed_hauls.len(), MAX_CLOSED_HAULS);
        assert_eq!(store.services.live_hauls.len(), 2);
        assert!(haul_of(ctx.w, 1).is_none());
        assert!(haul_of(ctx.w, 5).is_none());
        assert!(haul_of(ctx.w, 6).is_some());

        // Live iterator sees only the 2 live rows by ref.
        let live_ids: Vec<u64> = live_hauls(ctx.w).map(|j| j.contract).collect();
        assert_eq!(live_ids, vec![1_000, 1_001]);

        // Direct deletion (used on withdraw) removes the live row immediately and emits Del.
        delete(&mut ctx, 1_000);
        assert!(haul_of(ctx.w, 1_000).is_none());
        let live_after: Vec<u64> = live_hauls(ctx.w).map(|j| j.contract).collect();
        assert_eq!(live_after, vec![1_001]);
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
        assert!(dels.contains(&haul_key(1)));
        assert!(dels.contains(&haul_key(5)));
        assert!(dels.contains(&haul_key(1_000)));
    }

    /// **P2-TSW-23 & P2-TSW-48**:
    /// 1. `+haul/post ... as=<org>` preview runs `super::poster` before any `org_vault` /
    ///    `read_pool` check, so an unprivileged player cannot binary-search an org's stock.
    /// 2. `+haul/withdraw` on an org-owned job checks live `JOBS_POST_CAPABILITY`: a revoked
    ///    original poster is blocked, while another active officer holding the capability succeeds.
    #[test]
    fn p2_tsw_23_48_org_haul_preview_auth_and_live_withdraw_capability() {
        let mut tw = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_org::OrgModule::new()),
            Box::new(mush_economy::EconomyModule::new()),
            Box::new(crate::TacsimRuleset::default()),
        ]);
        let ex_poster = tw.spawn_player("ExPoster");
        let officer = tw.spawn_player("ActiveOfficer");
        let outsider = tw.spawn_player("Outsider");
        let org_id = mush_org::OrgId(77);
        let root_scope = mush_org::OrgScopeId(770);
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
                        name: "Isidis Tank Farm".into(),
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
                        id: mush_org::GrantId(1),
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

        // P2-TSW-48: org-owned haul checks live JOBS_POST_CAPABILITY, not original `shipper`.
        let mut org_job = job();
        org_job.shipper = ex_poster;
        org_job.shipper_org = Some(org_id.0);
        assert!(
            !may_withdraw(tw.world(), ex_poster, &org_job),
            "revoked original poster must not withdraw org haul"
        );
        assert!(
            may_withdraw(tw.world(), officer, &org_job),
            "active officer with JOBS_POST_CAPABILITY may withdraw org haul"
        );
        assert!(
            !may_withdraw(tw.world(), outsider, &org_job),
            "outsider must not withdraw org haul"
        );

        // Personal haul still checks `shipper == actor`.
        let mut personal_job = job();
        personal_job.shipper = ex_poster;
        personal_job.shipper_org = None;
        assert!(may_withdraw(tw.world(), ex_poster, &personal_job));
        assert!(!may_withdraw(tw.world(), officer, &personal_job));

        // P2-TSW-23: `super::poster` refuses an unprivileged caller for a visible org before any
        // vault read happens.
        let engine = tw.engine_mut();
        let mut out = mush_core::Outbox::default();
        let mut ctx = Ctx {
            w: &mut engine.world,
            reg: &engine.registry,
            out: &mut out,
            actor: outsider,
            args: Default::default(),
            switches: Default::default(),
            module: Some(crate::NS.into()),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let err = super::super::poster(&mut ctx, Some("Isidis Tank Farm"))
            .expect_err("outsider must be refused by poster()");
        assert_eq!(err, super::super::NOT_POSTER);
    }
}
