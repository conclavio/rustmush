//! **`+tv` and `+tv/lease`** (doc 20 cut SM.3a; doc 20 §1 beat 1, §4.3, Q-20-6,
//! Q-20-8; lead rulings 7 and 9 of the SM.3a brief, decisions D6–D10, D14, D16).
//!
//! A First Captain with a Dray-P docked at Ceres stands at Occator's counter (the
//! Exchange Hall, decision D11) and types `+tv/lease`: without `go` it prints the
//! terms and signs nothing (decision D9); with `go` it is the `+coop/enrol` shape,
//! **every check first, then the writes, the post last of the fallible ones**:
//!
//! 0. **the term settle** (lead ruling 9; decision D8): each of the pilot's Active
//!    leases past its term is re-debited a whole number of terms, or — the account
//!    short — its whole deposit is captured to Occator and the record goes Forfeit.
//!    The settle is **its own committed unit** (reviewer F5): its lines are said and
//!    its writes stand whatever follows, and every refusal after it wrote anything
//!    goes through `refuse_after_writes`;
//! 1. **the lease** — `mush_economy::offer_lease` with Occator as the
//!    server-authored lessor and the Trust's 1-minor Manual rent (decision D7), then
//!    `accept_contract` as the pilot;
//! 2. **one post as the pilot** — the term fee to Occator's account, **public**, and
//!    the deposit as an engine-owned `tacsim.tv.deposit` reservation on the pilot's
//!    own account (decision D6: only `settle_engine_reservation` moves it, so the
//!    pilot cannot `@hold/release` it with the rig aboard);
//! 3. **the placement** — the rig from Occator's shed into the bay's `rigs`
//!    compartment;
//! 4. **the record** — one bounded whole-document `tacsim:tvlease:<lease>`
//!    ([`TvLease`]).
//!
//! `+tv/<n> lease return [go]` reverses it: the lease ended, the deposit released
//! (nothing when Forfeit), the rig back in the shed, the record `Returned`. **The
//! return and the status are keyed on the lessee's own live record, not on a live
//! charter** (reviewer F1(b)), so a pilot whose charter has ended can still hand a
//! rig back. A charter that ends without the pilot (`@coop/revoke`, dormancy, the
//! return arm) **repossesses** every rig in the bay first (`repossess`, decision
//! D16's money rule: released within the paid term, captured past it, the engine
//! never debits the pilot).
//!
//! **A live record** is one whose status is not `Returned` (Active or Forfeit;
//! reviewer F2): it holds its slot, so a Forfeit rig still in the bay keeps its
//! number. Nothing here submits anything authoritative; cut SM.3b moves `+tv` to the
//! authoritative class with its act switches.

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{
    AccountId, Amount, ContractId, ContractState, EconomicCause, EconomicClock, EconomicSchedule,
    EconomicVisibility, EconomyPlan, EconomyStore, GracePolicy, HoldId, HoldOp, HoldPurpose,
    HoldStatus, IdempotencyKey, LeaseEnd, LeaseKey, LeaseSpec, ObligationTemplate, PostOutcome,
    RequestedLine, VaultId,
};
use mush_inventory::record::EndpointRef;
use mush_org::{OrgId, OrgScopeId, PrincipalId};
use mush_sim::data::{SpatialParent, SpatialState};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::port::coop::{
    NO_CHARTER, NOT_OPEN_HERE, ProgrammePort, money_minor, name_of, office_of, port_name,
    programme_port, say,
};
use crate::port::def::VehiclesDef;
use crate::port::ship::{Berth, HullView, berth_of, hull_view};
use crate::port::vehicle::{RigInBay, hopper_of, occator_lease_on, rigs_in_bay};
use crate::store::{TacsimStore, put_tv_lease};

/// The verb.
pub const TV_VERB: &str = "+tv";

/// `+tv`'s help (doc 20 §4.3).
pub const TV_HELP: &str = "+tv  --  Your Dray-P's rigs: each bay slot's terrain vehicle, its lease from Occator and its hopper, anywhere.
+tv/<n>  the same for rig n.
+tv/lease [go]  at Occator's counter in Ceres Freeport, with your Dray-P docked: lease a terrain vehicle into a free bay slot, 1400.00cr a term of 168 IC days and a 5000.00cr deposit held on your account. Without go it shows the terms and signs nothing.
+tv/<n> lease return [go]  at the same counter: hand rig n back, its hopper empty, and the deposit is released.
A lease past its term is re-debited at your next +tv/lease, or the deposit is forfeit.";

/// A hull with no bay: `{name}`, `{office}`, `{port}`.
pub const NO_BAY: &str =
    "The {name} has no vehicle bay; +coop/refit prospector at the {office} in {port} fits one.";
/// Under way: `{name}`, `{port}`.
pub const LEASE_UNDER_WAY: &str =
    "The {name} is under way; Occator leases its rigs into a bay docked at {port}.";
/// Docked elsewhere, or anywhere else: `{port}`.
pub const LEASE_NOT_HERE: &str = "Occator leases its rigs into a bay docked at {port}.";
/// Away from the counter: `{counter}`, `{port}`.
pub const LEASE_WHERE: &str = "Occator's rigs are leased at the {counter} in {port}.";
/// Every slot taken: `{name}`.
pub const BAY_FULL: &str = "The {name}'s bay is full.";
/// The named slot taken: `{n}`, `{name}`.
pub const SLOT_TAKEN: &str = "Rig {n} is already in the {name}'s bay.";
/// No rig in the shed: the pool is out.
pub const NO_RIG_FREE: &str = "Occator has no rig free to lease.";
/// Short of funds: `{total}`, `{fee}`, `{deposit}`, `{available}`.
pub const LEASE_FUNDS: &str = "The lease needs {total} available: {fee} for the term and {deposit} held as a deposit; you have {available}.";
/// The take's preview: `{mark}`, `{name}`, `{n}`, `{fee}`, `{days}`, `{deposit}`.
pub const LEASE_PREVIEW: &str = "Occator leases you {mark} into the {name}'s bay as rig {n}: {fee} now for {days} IC days, and {deposit} held on your account as a deposit. +tv/lease go signs it.";
/// The take: `{mark}`, `{name}`, `{n}`, `{fee}`, `{days}`, `{deposit}`.
pub const LEASED: &str = "Occator Water & Propellant leases you {mark} into the {name}'s bay as rig {n}: {fee} paid for {days} IC days, {deposit} held as a deposit.";
/// A return with two rigs leased and no slot named.
pub const WHICH_RIG: &str = "Which rig? +tv/1 lease return or +tv/2 lease return.";
/// No such rig: `{n}`, `{name}`.
pub const NO_RIG: &str = "There is no rig {n} in the {name}'s bay.";
/// The rig is not in the bay: `{n}`, `{name}`.
pub const RIG_OUT: &str =
    "Rig {n} is not in the {name}'s bay; it goes back to Occator from the bay.";
/// The hopper is not empty: `{n}`.
pub const HOPPER_FULL: &str = "Rig {n}'s hopper is not empty; it goes back to Occator empty.";
/// The return's preview: `{mark}`, `{deposit}`, `{n}`.
pub const RETURN_PREVIEW: &str = "You hand {mark} back to Occator; your {deposit} deposit is released. +tv/{n} lease return go signs it.";
/// The return's preview, the deposit forfeit: `{mark}`, `{n}`.
pub const RETURN_PREVIEW_FORFEIT: &str = "You hand {mark} back to Occator; its deposit was forfeit, so nothing is released. +tv/{n} lease return go signs it.";
/// The return: `{mark}`, `{deposit}`.
pub const RETURNED: &str =
    "You hand {mark} back across Occator's counter; your {deposit} deposit is released.";
/// The return, the deposit forfeit: `{mark}`.
pub const RETURNED_FORFEIT: &str =
    "You hand {mark} back across Occator's counter; its deposit was forfeit.";
/// The settle's re-debit: `{fee}`, `{mark}`, `{terms}`, `{days}`.
pub const TERM_RENEWED: &str =
    "Occator re-debits {fee} for {mark}: {terms} more term(s), to {days} IC days on.";
/// The settle's forfeit: `{mark}`, `{owed}`, `{deposit}`.
pub const TERM_FORFEIT: &str = "{mark}'s lease ran past its term and your account cannot pay {owed}: Occator keeps the {deposit} deposit.";
/// A step that should not fail did.
pub const LEASE_FAILED: &str =
    "Occator could not complete it; nothing of the lease or the return was signed.";
/// The grammar.
pub const LEASE_USAGE: &str = "Usage: +tv[/<n>]  or  +tv[/<n>]/lease [return] [go]";
/// A rig within its term: `{n}`, `{mark}`, `{span}`, `{hopper}`.
pub const RIG_LINE: &str =
    "Rig {n}: {mark}, in the bay; leased from Occator for {span} more; hopper {hopper}.";
/// A rig past its term: `{n}`, `{mark}`, `{fee}`, `{hopper}`.
pub const RIG_PAST_TERM: &str = "Rig {n}: {mark}, in the bay; past its term: {fee} is owed at your next +tv/lease, or the deposit is forfeit; hopper {hopper}.";
/// A forfeit rig: `{n}`, `{mark}`, `{hopper}`.
pub const RIG_FORFEIT: &str =
    "Rig {n}: {mark}, in the bay; its deposit is forfeit: return it to Occator; hopper {hopper}.";
/// A rig in the bay with no live record: `{mark}`.
pub const RIG_UNLISTED: &str = "Rig ?: {mark}, in the bay; Occator's books for it cannot be read.";
/// An empty slot: `{n}`, `{counter}`, `{port}`.
pub const RIG_EMPTY: &str = "Rig {n}: empty. Occator leases rigs at the {counter} in {port}.";
/// A live record whose economy lease staff ended (reviewer F15): `{n}`, `{mark}`,
/// `{counter}`, `{port}`, `{hopper}`.
pub const RIG_LEASE_ENDED: &str = "Rig {n}: {mark}, in the bay; its lease from Occator has ended: hand it back at the {counter} in {port}; hopper {hopper}.";
/// The rigs cannot be read: `{name}`.
pub const TV_DARK: &str = "The {name}'s rigs cannot be read right now; nothing changes.";
/// A repossession within the term: `{mark}`, `{name}`, `{deposit}`.
pub const REPOSSESSED: &str = "Occator took {mark} back out of the {name}'s bay when her charter ended; your {deposit} deposit is released.";
/// A repossession past the term or of a forfeit rig: `{mark}`, `{name}`.
pub const REPOSSESSED_FORFEIT: &str = "Occator took {mark} back out of the {name}'s bay when her charter ended; its deposit was forfeit.";

/// The prefix of every rig lease record (`tacsim:tvlease:{lease}`).
pub const TV_LEASE_PREFIX: &str = "tacsim:tvlease:";
/// The record's schema version.
pub const TV_LEASE_SCHEMA: u16 = 1;

/// **The deposit's reservation namespace** (decision D6): an
/// [`mush_economy::ENGINE_OWNED_RESERVATIONS`] entry, so only
/// `settle_engine_reservation` releases or captures it. Also the release's
/// idempotency namespace (`tacsim.tv.deposit/{lease}`).
pub const DEPOSIT_KIND: &str = "tacsim.tv.deposit";
/// The lease post's idempotency namespace and cause kind (`tacsim.tv.lease/{lease}`).
pub const LEASE_KEY_NAMESPACE: &str = "tacsim.tv.lease";
/// A term re-debit's idempotency namespace (`tacsim.tv.fee/{lease}/{term}`).
pub const FEE_KEY_NAMESPACE: &str = "tacsim.tv.fee";
/// A deposit capture's idempotency namespace (`tacsim.tv.forfeit/{lease}`).
pub const FORFEIT_KEY_NAMESPACE: &str = "tacsim.tv.forfeit";

/// IC seconds in an IC day.
const DAY: i64 = 86_400;

/// **`tacsim:tvlease:{lease}`** — the one key of one rig lease's record.
#[must_use]
pub fn tv_lease_key(lease: u64) -> SmolStr {
    SmolStr::new(format!("{TV_LEASE_PREFIX}{lease}"))
}

/// **Where a rig lease stands** (doc 20 cut SM.3a).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TvLeaseStatus {
    /// Leased and paid to `term_ends_ic`.
    Active,
    /// The term ran out on an empty account: the deposit was captured, nothing
    /// more accrues, and the rig waits to be returned (decision D8).
    Forfeit,
    /// Back in Occator's shed.
    Returned,
}

/// **One rig lease's record** (doc 20 cut SM.3a; §4.3, Q-20-6): bounded,
/// whole-document, strict on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvLease {
    /// [`TV_LEASE_SCHEMA`].
    pub schema: u16,
    /// The lease contract, `ContractId.0`: the key.
    pub lease: u64,
    /// The terrain vehicle.
    pub tv: ObjId,
    /// Its registry mark (`OWP-TV-01`).
    pub mark: String,
    /// The lessee.
    pub pilot: ObjId,
    /// The hull whose bay it was leased into.
    pub hull: ObjId,
    /// Its bay slot, `1..`.
    pub slot: u8,
    /// The deposit hold, `HoldId.0`.
    pub hold: u64,
    /// One term's fee, minor units.
    pub fee_minor: i64,
    /// The deposit, minor units.
    pub deposit_minor: i64,
    /// One term, IC seconds.
    pub term_secs: i64,
    /// The IC second it was leased.
    pub leased_at_ic: i64,
    /// The IC second its paid term ends.
    pub term_ends_ic: i64,
    /// Terms paid so far (the first at signing).
    pub terms_paid: u32,
    /// Where it stands.
    pub status: TvLeaseStatus,
    /// The IC second it was returned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at_ic: Option<i64>,
    /// The rig went back by a repossession (a charter's end), not the lessee's own return.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repossessed: bool,
}

/// The longest mark a record may carry, bytes.
const MAX_MARK_BYTES: usize = 32;

impl TvLease {
    /// The record's own invariants (the loader runs it; a failure is an
    /// unreadable record): the schema, a mark, `slot ≥ 1`, positive amounts and
    /// term, and `term_ends_ic > leased_at_ic`.
    ///
    /// # Errors
    ///
    /// The first invariant broken, as a sentence.
    pub fn check(&self) -> Result<(), String> {
        if self.schema != TV_LEASE_SCHEMA {
            return Err(format!("schema {} is not {TV_LEASE_SCHEMA}", self.schema));
        }
        if self.mark.is_empty() || self.mark.len() > MAX_MARK_BYTES {
            return Err(format!("mark must be 1..={MAX_MARK_BYTES} bytes"));
        }
        if self.slot == 0 {
            return Err("slot must be at least 1".to_owned());
        }
        if self.fee_minor <= 0 || self.deposit_minor <= 0 || self.term_secs <= 0 {
            return Err("fee_minor, deposit_minor and term_secs must be positive".to_owned());
        }
        if self.term_ends_ic <= self.leased_at_ic {
            return Err("term_ends_ic must be after leased_at_ic".to_owned());
        }
        if self.terms_paid == 0 {
            return Err("terms_paid must be at least 1".to_owned());
        }
        if (self.status == TvLeaseStatus::Returned) != self.ended_at_ic.is_some() {
            return Err("ended_at_ic is set iff the rig is returned".to_owned());
        }
        Ok(())
    }

    /// **Whether the record is live** (reviewer F2): not `Returned`.
    #[must_use]
    pub fn live(&self) -> bool {
        self.status != TvLeaseStatus::Returned
    }
}

/// **One rig lease, as the store holds it.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TvLeaseState {
    /// The record read.
    Present(TvLease),
    /// A key claims the lease but its body did not read: it bars only its own
    /// lease. What the raw body still names, so the right pilot reads dark and
    /// the right hull is held.
    Unreadable {
        /// The `pilot` field, when it read.
        pilot: Option<ObjId>,
        /// The `hull` field, when it read.
        hull: Option<ObjId>,
        /// The `tv` field, when it read.
        tv: Option<ObjId>,
    },
}

/// **The licence seam** (doc 20 §4.3; cut SM.3a): a lease is refused without an
/// Active licence; the licence instrument is SM.5's, which fills this seam. SM.3a
/// refuses nothing here.
///
/// # Errors
///
/// None in cut SM.3a.
pub(crate) fn licence_seam(w: &World, pilot: ObjId) -> Result<(), String> {
    let _ = (w, pilot);
    Ok(())
}

/// **A refusal after the first write** (reviewer F5; `buyout.rs`'s shape): the
/// deferred queue is pinned so RAM and the store agree on what was written.
fn refuse_after_writes(ctx: &mut Ctx<'_>, line: impl Into<String>) -> CmdError {
    ctx.out.pin_deferred();
    CmdError::msg(line.into())
}

// ---- resolution ----------------------------------------------------------------

/// Occator, as the lease, the posts and the texts address it.
#[derive(Clone, Debug)]
struct Lessor {
    /// The pool.
    pool: VehiclesDef,
    /// The org principal.
    principal: PrincipalId,
    /// Its root scope.
    scope: OrgScopeId,
    /// Its account, once opened.
    account: Option<AccountId>,
    /// The counter room.
    counter: Option<ObjId>,
    /// The counter's name.
    counter_name: String,
    /// The port's name.
    port: String,
    /// The shed.
    shed: Option<ObjId>,
}

/// The programme port's rig lessor, when the latched lane authors one.
fn lessor_of(w: &World, port: &ProgrammePort<'_>) -> Option<Lessor> {
    let pool = port.def.vehicles.as_deref().cloned()?;
    let row = port.map.orgs.get(&pool.org)?;
    let counter = crate::port::vendor::role_room(w, port.def, &pool.counter);
    let counter_name = counter.map_or_else(|| pool.counter.to_string(), |room| name_of(w, room));
    Some(Lessor {
        principal: PrincipalId::Organization(OrgId(row.org)),
        scope: OrgScopeId(row.root_scope),
        account: row.account.map(AccountId),
        counter,
        counter_name,
        port: port_name(port.def),
        shed: port
            .map
            .vehicles
            .as_ref()
            .and_then(|row| row.shed)
            .map(ObjId),
        pool,
    })
}

/// The counter refusal and the empty-slot line read the counter and the port.
fn where_line(template: &str, lessor: &Lessor) -> String {
    template
        .replace("{counter}", &lessor.counter_name)
        .replace("{port}", &lessor.port)
}

/// A money amount in minor units through the economy's render.
fn money(ctx: &Ctx<'_>, minor: i64) -> String {
    money_minor(ctx, minor)
}

/// Whole IC days, rounded up, from `now` to `until` (never below 0).
fn ic_days_until(now: i64, until: i64) -> i64 {
    until.saturating_sub(now).max(0).saturating_add(DAY - 1) / DAY
}

/// **The real time to `term_ends_ic`** (reviewer F12), exactly as
/// `site::stores_line` reads a span: [`crate::port::voyage::real_until`] through
/// [`crate::port::site::pad_span`], and `{n} IC days` while the clock is stopped.
fn span_to(w: &World, term_ends_ic: i64) -> String {
    match crate::port::voyage::real_until(w, term_ends_ic) {
        Some(real) => crate::port::site::pad_span(real),
        None => format!(
            "{} IC days",
            ic_days_until(w.world_ic_now().at, term_ends_ic)
        ),
    }
}

/// The goods a rig's hopper accepts and its mass limit, kilograms, from its unit
/// (its live record's, else the pool's).
fn hopper_goods(
    w: &World,
    tv: ObjId,
    pool_unit: Option<&CanonicalId>,
) -> Option<(Vec<CanonicalId>, u64)> {
    let store = w.module_state::<TacsimStore>()?;
    let goods = store.goods()?;
    let unit = crate::port::hull::live_unit_id(w, tv).or_else(|| pool_unit.cloned())?;
    let logistics = store.definitions().specs.get(&unit)?.logistics.as_ref()?;
    let accepted = goods
        .goods
        .iter()
        .filter(|good| {
            logistics
                .stowage
                .values()
                .any(|compartment| compartment.accepts.contains(&good.stowage))
        })
        .map(|good| good.id.clone())
        .collect();
    Some((accepted, logistics.payload_floor_kg()?))
}

/// **What a rig's hopper holds**: `(kilograms, capacity kilograms)` of ore, each
/// pool read through [`crate::port_tick::read_pool`]. `Err` when the rig has no
/// hopper or a pool will not read.
fn hopper_load(
    ctx: &Ctx<'_>,
    tv: ObjId,
    pool_unit: Option<&CanonicalId>,
) -> Result<(u64, u64), String> {
    let w: &World = ctx.w;
    let goods = w
        .module_state::<TacsimStore>()
        .and_then(TacsimStore::goods)
        .ok_or_else(|| "the goods table is not loaded".to_owned())?;
    let vault = hopper_of(w, tv).ok_or_else(|| format!("rig #{} has no hopper vault", tv.0))?;
    let (accepted, _) = hopper_goods(w, tv, pool_unit)
        .ok_or_else(|| format!("rig #{}'s unit cannot be read", tv.0))?;
    let capacity = hopper_capacity(w, tv, pool_unit).unwrap_or(0);
    let mut kg: u64 = 0;
    for good in &accepted {
        let per_unit = goods
            .good(good)
            .and_then(crate::definitions::GoodDef::mass_kg_per_unit)
            .ok_or_else(|| format!("the goods table does not weigh {good}"))?;
        let qty = crate::port_tick::read_pool(ctx, EndpointRef(vault), good)
            .map_err(|error| format!("rig #{}'s hopper {good}: {error:?}", tv.0))?;
        kg = qty
            .checked_mul(per_unit)
            .and_then(|add| kg.checked_add(add))
            .ok_or_else(|| "a hopper's mass overflows".to_owned())?;
    }
    Ok((kg, capacity))
}

/// A rig's hopper mass limit, kilograms: the mass limits of its stowage.
fn hopper_capacity(w: &World, tv: ObjId, pool_unit: Option<&CanonicalId>) -> Option<u64> {
    let store = w.module_state::<TacsimStore>()?;
    let unit = crate::port::hull::live_unit_id(w, tv).or_else(|| pool_unit.cloned())?;
    let logistics = store.definitions().specs.get(&unit)?.logistics.as_ref()?;
    let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).ok()?;
    let mut sum: u64 = 0;
    for compartment in logistics.stowage.values() {
        sum = sum.checked_add(compartment.limits.get(&mass).copied().unwrap_or(0))?;
    }
    Some(sum)
}

/// `{hopper}`: `empty`, or `{t} of {cap} t` in whole tonnes, floored.
fn hopper_text(kg: u64, capacity_kg: u64) -> String {
    if kg == 0 {
        "empty".to_owned()
    } else {
        format!("{} of {} t", kg / 1_000, capacity_kg / 1_000)
    }
}

/// The actor's live records (any hull), lease order.
fn live_records(w: &World, actor: ObjId) -> Vec<TvLease> {
    w.module_state::<TacsimStore>()
        .map_or_else(Vec::new, |store| {
            store
                .tv_leases_of(actor)
                .filter(|record| record.live())
                .cloned()
                .collect()
        })
}

/// The live records on `hull` (any lessee), lease order.
fn live_on_hull(w: &World, hull: ObjId) -> Vec<TvLease> {
    w.module_state::<TacsimStore>()
        .map_or_else(Vec::new, |store| {
            store
                .tv_leases
                .values()
                .filter_map(|state| match state {
                    TvLeaseState::Present(record) if record.hull == hull && record.live() => {
                        Some(record.clone())
                    }
                    TvLeaseState::Present(_) | TvLeaseState::Unreadable { .. } => None,
                })
                .collect()
        })
}

/// Whether `record`'s economy lease is live: Occator's live lease on its TV is
/// the record's own contract (reviewer F15).
fn economy_lease_live(w: &World, lessor: &PrincipalId, record: &TvLease) -> bool {
    occator_lease_on(w, lessor, record.tv).is_some_and(|c| c.id.0 == record.lease)
}

/// `{name}` and the other hull words.
fn hull_name(w: &World, hull: ObjId) -> String {
    name_of(w, hull)
}

/// **`TV_DARK` for an actor whose own record will not read** (reviewer findings
/// correctness/F3, conformance/F4): `{name}` is the live charter's hull, else the
/// hull the unreadable record still names (a Revoked pilot has no charter), else
/// the class word — never an empty name.
fn dark_line(w: &World, store: &TacsimStore, actor: ObjId, charter_hull: Option<ObjId>) -> String {
    let hull = charter_hull.or_else(|| {
        store.tv_leases.values().find_map(|state| match state {
            TvLeaseState::Unreadable {
                pilot: Some(pilot),
                hull: Some(hull),
                ..
            } if *pilot == actor => Some(*hull),
            TvLeaseState::Unreadable { .. } | TvLeaseState::Present(_) => None,
        })
    });
    let name = hull.map_or_else(
        || crate::port::ship::HULL_CLASS_PROSPECTOR.to_owned(),
        |hull| hull_name(w, hull),
    );
    TV_DARK.replace("{name}", &name)
}

// ---- the verb --------------------------------------------------------------------

/// What `+tv` was asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    /// The status.
    Status,
    /// A take, committed or previewed.
    Take { go: bool },
    /// A return, committed or previewed.
    Return { go: bool },
}

/// **The grammar** (doc 20 §1's slot rule): `+tv[/<n>]`, `+tv[/<n>]/lease [take]
/// [go]`, `+tv[/<n>]/lease return [go]`, and the same with `lease` as the first
/// word instead of the switch. Two slot switches, or an unknown word, is
/// [`LEASE_USAGE`]; `/3` and above never reach here (the dispatcher refuses the
/// undeclared switch).
fn parse(ctx: &Ctx<'_>) -> Result<(Option<u8>, Act), CmdError> {
    let usage = || CmdError::msg(LEASE_USAGE);
    let slot = match (ctx.switches.has("1"), ctx.switches.has("2")) {
        (true, true) => return Err(usage()),
        (true, false) => Some(1),
        (false, true) => Some(2),
        (false, false) => None,
    };
    let mut words = ctx
        .args
        .raw()
        .split_whitespace()
        .map(str::to_ascii_lowercase);
    let mut lease = ctx.switches.has("lease");
    let mut next = words.next();
    if !lease && next.as_deref() == Some("lease") {
        lease = true;
        next = words.next();
    }
    if !lease {
        return if next.is_none() {
            Ok((slot, Act::Status))
        } else {
            Err(usage())
        };
    }
    let mut back = false;
    match next.as_deref() {
        Some("take") => next = words.next(),
        Some("return") => {
            back = true;
            next = words.next();
        }
        _ => {}
    }
    let go = match next.as_deref() {
        None => false,
        Some("go") => true,
        Some(_) => return Err(usage()),
    };
    if words.next().is_some() {
        return Err(usage());
    }
    Ok((
        slot,
        if back {
            Act::Return { go }
        } else {
            Act::Take { go }
        },
    ))
}

/// **`+tv`, `+tv/<n>`, `+tv/lease`** (doc 20 §4.3; Q-20-6, Q-20-8).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn tv_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    if programme_port(ctx.w).is_none() {
        return Err(CmdError::msg(NOT_OPEN_HERE));
    }
    let (slot, act) = parse(ctx)?;
    match act {
        Act::Status => status(ctx, slot),
        Act::Take { go } => take(ctx, slot, go),
        Act::Return { go } => give_back(ctx, slot, go),
    }
}

// ---- the status --------------------------------------------------------------------

/// **`+tv`** — a pure read (doc 20 §4.3): the hull (the live charter's, else the
/// one the actor's live records name), then one line per bay slot, the unlisted
/// rigs, and on a site's pad SM.2's stores line.
fn status(ctx: &mut Ctx<'_>, only: Option<u8>) -> Result<(), CmdError> {
    let lines = status_lines(ctx, only).map_err(CmdError::msg)?;
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// The status, read-only; `Err` is the refusal.
#[allow(clippy::too_many_lines)] // one slot walk, one line per state
fn status_lines(ctx: &Ctx<'_>, only: Option<u8>) -> Result<Vec<String>, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    let port = programme_port(w).ok_or_else(|| NOT_OPEN_HERE.to_owned())?;
    // 1. The hull: the live charter's, else the one the actor's live records name;
    // with neither, an unreadable record of the actor's still darkens the whole reply
    // (reviewer finding correctness/F3) before the charter's own refusal is said.
    let hull = match crate::port::recall::chartered(ctx) {
        Ok(e) => e.hull,
        Err(line) => match live_records(w, actor).first() {
            Some(record) => record.hull,
            None => {
                if let Some(store) = w.module_state::<TacsimStore>()
                    && store.tv_lease_unreadable_for(actor)
                {
                    tracing::error!(
                        pilot = actor.0,
                        "tacsim: +tv found an unreadable tacsim:tvlease: record; staff repair it"
                    );
                    return Err(dark_line(w, store, actor, None));
                }
                return Err(line);
            }
        },
    };
    let name = hull_name(w, hull);
    let dark = || TV_DARK.replace("{name}", &name);
    let Some(view) = hull_view(w, hull) else {
        tracing::error!(
            hull = hull.0,
            "tacsim: +tv found no programme view of the hull"
        );
        return Err(dark());
    };
    // 2. The view.
    if view.unit_held {
        tracing::warn!(hull = hull.0, "tacsim: +tv: the hull's unit record is held");
        return Err(dark());
    }
    if !view.is_prospector() {
        let (_, office, port_label) = office_of(w, &port);
        return Err(NO_BAY
            .replace("{name}", &name)
            .replace("{office}", &office)
            .replace("{port}", &port_label));
    }
    let Some((bay, _)) = view.bay() else {
        tracing::error!(
            hull = hull.0,
            "tacsim: +tv: the Dray-P's bay is not laid up"
        );
        return Err(dark());
    };
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| NOT_OPEN_HERE.to_owned())?;
    if store.tv_lease_unreadable_for(actor) || store.tv_lease_unreadable_for(hull) {
        tracing::error!(
            pilot = actor.0,
            hull = hull.0,
            "tacsim: +tv found an unreadable tacsim:tvlease: record; staff repair it"
        );
        return Err(dark());
    }
    let lessor = lessor_of(w, &port).ok_or_else(|| {
        tracing::error!("tacsim: +tv: the programme port authors no rig lessor");
        dark()
    })?;
    let pool_unit = Some(&lessor.pool.unit);
    let now = w.world_ic_now().at;
    let records = live_on_hull(w, hull);
    let rigs = rigs_in_bay(w, &view);
    let unlisted: Vec<&RigInBay> = rigs.iter().filter(|rig| rig.slot == 0).collect();
    let max = u8::try_from(bay.slot.maximum_items).unwrap_or(u8::MAX);
    // Slots no live record names, in order: the first `unlisted.len()` are filled by
    // the unlisted rigs, the rest are empty.
    let free: Vec<u8> = (1..=max)
        .filter(|n| !records.iter().any(|record| record.slot == *n))
        .collect();
    let filled_unlisted: Vec<u8> = free.iter().copied().take(unlisted.len()).collect();
    let mut lines = Vec::new();
    for n in 1..=max {
        if only.is_some_and(|want| want != n) {
            continue;
        }
        let Some(record) = records.iter().find(|record| record.slot == n) else {
            match filled_unlisted.iter().position(|slot| *slot == n) {
                None => {
                    lines.push(where_line(RIG_EMPTY, &lessor).replace("{n}", &n.to_string()));
                }
                // `+tv/<n>` on a slot an unlisted rig fills answers that rig (reviewer
                // finding conformance/F2), never nothing; the bare `+tv` lists every
                // unlisted rig after the numbered lines.
                Some(i) if only.is_some() => {
                    if let Some(rig) = unlisted.get(i) {
                        lines.push(RIG_UNLISTED.replace("{mark}", &rig.mark));
                    }
                }
                Some(_) => {}
            }
            continue;
        };
        let hopper = match hopper_load(ctx, record.tv, pool_unit) {
            Ok((kg, cap)) => hopper_text(kg, cap),
            Err(error) => {
                tracing::warn!(tv = record.tv.0, %error, "tacsim: +tv could not read a hopper");
                return Err(dark());
            }
        };
        let line = if !economy_lease_live(w, &lessor.principal, record) {
            where_line(RIG_LEASE_ENDED, &lessor)
        } else {
            match record.status {
                TvLeaseStatus::Forfeit => RIG_FORFEIT.to_owned(),
                TvLeaseStatus::Active if now >= record.term_ends_ic => {
                    let owed = owed_of(record, now).map_or(record.fee_minor, |(_, owed)| owed);
                    RIG_PAST_TERM.replace("{fee}", &money(ctx, owed))
                }
                TvLeaseStatus::Active | TvLeaseStatus::Returned => {
                    RIG_LINE.replace("{span}", &span_to(w, record.term_ends_ic))
                }
            }
        };
        lines.push(
            line.replace("{n}", &n.to_string())
                .replace("{mark}", &record.mark)
                .replace("{hopper}", &hopper),
        );
    }
    if only.is_none() {
        for rig in unlisted {
            lines.push(RIG_UNLISTED.replace("{mark}", &rig.mark));
        }
    }
    // 4. On a site's pad, SM.2's stores line (the SM.2 note deferred it here).
    if let Some(stores) = crate::port::voyage::settled_stores_line(ctx, &view) {
        lines.push(stores);
    }
    Ok(lines)
}

// ---- the term settle --------------------------------------------------------------------

/// One term settle step (lead ruling 9; decision D8).
#[derive(Clone, Debug)]
enum Settle {
    /// Re-debit `due` terms, `owed` minor units.
    Renew {
        record: TvLease,
        due: u32,
        owed: i64,
    },
    /// Capture the whole deposit: the account cannot pay `owed`.
    Forfeit { record: TvLease, owed: i64 },
}

/// The terms due on `record` at `now` and what they cost: `due = (now −
/// term_ends) / term_secs + 1`, `owed = due × fee`. `None` before the term ends or
/// on overflow.
fn owed_of(record: &TvLease, now: i64) -> Option<(u32, i64)> {
    if now < record.term_ends_ic {
        return None;
    }
    let elapsed = now.checked_sub(record.term_ends_ic)?;
    let due = elapsed.checked_div(record.term_secs)?.checked_add(1)?;
    let due = u32::try_from(due).ok()?;
    let owed = record.fee_minor.checked_mul(i64::from(due))?;
    Some((due, owed))
}

/// **Plan the settle** for `actor`'s Active records past their term whose economy
/// lease is still live (reviewer F15: a lease staff ended accrues nothing), in
/// lease order, against `available`: a renewal while the account can pay, else a
/// forfeit. Answers the steps and the available left after the renewals.
fn plan_settle(
    w: &World,
    actor: ObjId,
    lessor: &PrincipalId,
    now: i64,
    mut available: i64,
) -> (Vec<Settle>, i64) {
    let mut steps = Vec::new();
    for record in live_records(w, actor) {
        if record.status != TvLeaseStatus::Active || !economy_lease_live(w, lessor, &record) {
            continue;
        }
        let Some((due, owed)) = owed_of(&record, now) else {
            continue;
        };
        if available >= owed {
            available = available.saturating_sub(owed);
            steps.push(Settle::Renew { record, due, owed });
        } else {
            steps.push(Settle::Forfeit { record, owed });
        }
    }
    (steps, available)
}

/// The line a settle step says.
fn settle_line(ctx: &Ctx<'_>, step: &Settle, now: i64) -> String {
    match step {
        Settle::Renew { record, due, owed } => {
            let ends = record
                .term_secs
                .checked_mul(i64::from(*due))
                .and_then(|add| record.term_ends_ic.checked_add(add))
                .unwrap_or(i64::MAX);
            TERM_RENEWED
                .replace("{fee}", &money(ctx, *owed))
                .replace("{mark}", &record.mark)
                .replace("{terms}", &due.to_string())
                .replace("{days}", &ic_days_until(now, ends).to_string())
        }
        Settle::Forfeit { record, owed } => TERM_FORFEIT
            .replace("{mark}", &record.mark)
            .replace("{owed}", &money(ctx, *owed))
            .replace("{deposit}", &money(ctx, record.deposit_minor)),
    }
}

/// **Run the settle** (reviewer F5: its own committed unit): each step's money,
/// then its record, then its line; a step the economy refuses is logged and
/// skipped. Answers whether anything was written.
fn run_settle(
    ctx: &mut Ctx<'_>,
    steps: &[Settle],
    pilot_account: AccountId,
    lessor: &Lessor,
) -> bool {
    let now = ctx.w.world_ic_now().at;
    let mut wrote = false;
    for step in steps {
        let line = settle_line(ctx, step, now);
        match step {
            Settle::Renew { record, due, owed } => {
                let Some(lessor_account) = lessor.account else {
                    tracing::error!(
                        lease = record.lease,
                        "tacsim: a term re-debit found no Occator account"
                    );
                    continue;
                };
                let (Some(debit), credit) = (
                    Amount::from_minor(*owed).checked_neg(),
                    Amount::from_minor(*owed),
                ) else {
                    continue;
                };
                let term = record.terms_paid.saturating_add(1);
                let plan = EconomyPlan {
                    description: format!("rig lease {} term {term}", record.lease),
                    visibility: EconomicVisibility::Public,
                    ..EconomyPlan::new(
                        IdempotencyKey::new(FEE_KEY_NAMESPACE, format!("{}/{term}", record.lease)),
                        PrincipalId::Object(record.pilot),
                        EconomicCause::Internal {
                            kind: SmolStr::new_static(FEE_KEY_NAMESPACE),
                            id: record.lease,
                        },
                        vec![
                            RequestedLine {
                                account: pilot_account,
                                delta: debit,
                                memo: SmolStr::new_static(FEE_KEY_NAMESPACE),
                            },
                            RequestedLine {
                                account: lessor_account,
                                delta: credit,
                                memo: SmolStr::new_static(FEE_KEY_NAMESPACE),
                            },
                        ],
                    )
                };
                match mush_economy::post(ctx, &plan) {
                    Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {
                        wrote = true;
                        let ends = record
                            .term_secs
                            .checked_mul(i64::from(*due))
                            .and_then(|add| record.term_ends_ic.checked_add(add));
                        let Some(ends) = ends else {
                            tracing::error!(
                                lease = record.lease,
                                "tacsim: a re-debited term overflows; the record is left as it was"
                            );
                            continue;
                        };
                        let renewed = TvLease {
                            terms_paid: record.terms_paid.saturating_add(*due),
                            term_ends_ic: ends,
                            ..record.clone()
                        };
                        put_tv_lease(ctx, &renewed);
                        say(ctx, line);
                    }
                    other => {
                        tracing::error!(lease = record.lease, outcome = ?other, "tacsim: a term re-debit did not post; the record is left as it was");
                    }
                }
            }
            Settle::Forfeit { record, .. } => {
                let Some(lessor_account) = lessor.account else {
                    tracing::error!(
                        lease = record.lease,
                        "tacsim: a deposit capture found no Occator account"
                    );
                    continue;
                };
                match settle_deposit(ctx, record, Some((&lessor.principal, lessor_account))) {
                    Ok(Deposit::Captured) => {
                        wrote = true;
                        let forfeit = TvLease {
                            status: TvLeaseStatus::Forfeit,
                            ..record.clone()
                        };
                        put_tv_lease(ctx, &forfeit);
                        say(ctx, line);
                    }
                    Ok(Deposit::Released) => {
                        tracing::error!(
                            lease = record.lease,
                            "tacsim: a forfeit found its deposit already released; the record is \
                             left as it was"
                        );
                    }
                    Err(error) => {
                        tracing::error!(lease = record.lease, %error, "tacsim: a forfeit deposit was not captured; the record is left as it was");
                    }
                }
            }
        }
    }
    wrote
}

/// **How a rig's deposit stands once settled** (decision D6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Deposit {
    /// Released back to the pilot's available funds.
    Released,
    /// Captured whole to Occator's account.
    Captured,
}

/// **`record`'s deposit hold**, any status: the hold the record names, and only when
/// it is this lease's `tacsim.tv.deposit` reservation (a record naming a hold that
/// is not its own deposit — the take's hold-`0` breach — reads `Err`, so it is never
/// settled as someone else's).
fn deposit_hold(w: &World, record: &TvLease) -> Result<mush_economy::Hold, String> {
    let hold = w
        .module_state::<EconomyStore>()
        .and_then(|economy| economy.snapshot().hold(HoldId(record.hold)).cloned())
        .ok_or_else(|| format!("hold #{} does not exist", record.hold))?;
    let own = HoldPurpose::Reservation {
        kind: SmolStr::new_static(DEPOSIT_KIND),
        id: record.lease,
    };
    if hold.purpose != own {
        return Err(format!(
            "hold #{} is not lease {}'s deposit",
            record.hold, record.lease
        ));
    }
    Ok(hold)
}

/// **Settle a deposit, idempotently** (decision D6; reviewer finding correctness/F1):
/// a `Held` deposit is released (`capture` `None`) or captured whole to the lessor's
/// account through `settle_engine_reservation`; a deposit an earlier run already
/// settled answers how it stands (`Released`/`Expired` → released, `Consumed` →
/// captured) without a second post. So a return or a repossession that stopped
/// after the deposit runs again cleanly, and **no caller writes a record `Returned`
/// while its deposit is still `Held`**: only `settle_engine_reservation` moves an
/// engine-owned hold (staff included), so a `Held` deposit under a `Returned`
/// record would be stranded for good.
///
/// # Errors
///
/// The hold is missing or not this lease's deposit, or the economy refused.
fn settle_deposit(
    ctx: &mut Ctx<'_>,
    record: &TvLease,
    capture: Option<(&PrincipalId, AccountId)>,
) -> Result<Deposit, String> {
    let hold = deposit_hold(ctx.w, record)?;
    match hold.status {
        HoldStatus::Held => {}
        HoldStatus::Released | HoldStatus::Expired => return Ok(Deposit::Released),
        HoldStatus::Consumed { .. } => return Ok(Deposit::Captured),
    }
    match capture {
        None => release_deposit(ctx, record, &hold).map(|()| Deposit::Released),
        Some((lessor, account)) => {
            capture_deposit(ctx, record, &hold, lessor, account).map(|()| Deposit::Captured)
        }
    }
}

/// **Release a deposit** through the engine's own seam (decision D6), key
/// `tacsim.tv.deposit/{lease}`.
fn release_deposit(
    ctx: &mut Ctx<'_>,
    record: &TvLease,
    hold: &mush_economy::Hold,
) -> Result<(), String> {
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Release { hold: hold.id }],
        description: format!("rig lease {} deposit released", record.lease),
        ..EconomyPlan::new(
            IdempotencyKey::new(DEPOSIT_KIND, record.lease.to_string()),
            hold.placed_by.clone(),
            EconomicCause::Internal {
                kind: SmolStr::new_static(DEPOSIT_KIND),
                id: record.lease,
            },
            Vec::new(),
        )
    };
    match mush_economy::settle_engine_reservation(ctx, &plan, hold.id, None) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        other => Err(format!("{other:?}")),
    }
}

/// **Capture a whole deposit to Occator** through the engine's own seam (decision
/// D6: `settle_engine_reservation` captures the hold's whole amount), key
/// `tacsim.tv.forfeit/{lease}`.
fn capture_deposit(
    ctx: &mut Ctx<'_>,
    record: &TvLease,
    hold: &mush_economy::Hold,
    lessor: &PrincipalId,
    lessor_account: AccountId,
) -> Result<(), String> {
    let amount = hold.amount;
    let debit = amount
        .checked_neg()
        .ok_or_else(|| "the deposit does not negate".to_owned())?;
    let plan = EconomyPlan {
        hold_ops: vec![HoldOp::Capture {
            hold: hold.id,
            amount,
        }],
        description: format!("rig lease {} deposit forfeit", record.lease),
        visibility: EconomicVisibility::Public,
        ..EconomyPlan::new(
            IdempotencyKey::new(FORFEIT_KEY_NAMESPACE, record.lease.to_string()),
            lessor.clone(),
            EconomicCause::Internal {
                kind: SmolStr::new_static(FORFEIT_KEY_NAMESPACE),
                id: record.lease,
            },
            vec![
                RequestedLine {
                    account: hold.account,
                    delta: debit,
                    memo: SmolStr::new_static(FORFEIT_KEY_NAMESPACE),
                },
                RequestedLine {
                    account: lessor_account,
                    delta: amount,
                    memo: SmolStr::new_static(FORFEIT_KEY_NAMESPACE),
                },
            ],
        )
    };
    match mush_economy::settle_engine_reservation(ctx, &plan, hold.id, Some(lessor_account)) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => Ok(()),
        other => Err(format!("{other:?}")),
    }
}

// ---- the take --------------------------------------------------------------------

/// Everything the take resolved before its first write.
struct TakePlan {
    lessor: Lessor,
    hull: ObjId,
    name: String,
    slot: u8,
    bay_room: ObjId,
    bay_slot: mush_sim::SlotId,
    tv: ObjId,
    mark: String,
    pilot_account: AccountId,
    lessor_account: AccountId,
    fee_minor: i64,
    deposit_minor: i64,
    term_secs: i64,
    settle: Vec<Settle>,
}

/// The pilot's credit account and its available funds.
fn funds(w: &World, actor: ObjId) -> Option<(AccountId, i64)> {
    let account = crate::port::credit::own_account(w, actor)?;
    Some((account.id, account.available()?.minor()))
}

/// Major units at the credit scale, as minor.
fn minor(w: &World, major: u64) -> Option<i64> {
    crate::port::apply::minor_of(major, crate::port::apply::credit_scale(w))
}

/// **The take's checks, in order** (the `plan_buyout` order), read-only. `Err`
/// carries the refusal.
#[allow(clippy::too_many_lines)] // one ordered check list
fn plan_take(ctx: &Ctx<'_>, named: Option<u8>) -> Result<TakePlan, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    let port = programme_port(w).ok_or_else(|| NOT_OPEN_HERE.to_owned())?;
    let failed = |what: &str| {
        tracing::error!(
            what,
            pilot = actor.0,
            "tacsim: +tv/lease could not resolve a step"
        );
        LEASE_FAILED.to_owned()
    };
    // 1. The charter (graduates are refused with the charter's own line: D10).
    let e = crate::port::recall::chartered(ctx)?;
    let hull = e.hull;
    let name = hull_name(w, hull);
    // 2. A prospector hull.
    let view = hull_view(w, hull).ok_or_else(|| failed("the hull's view"))?;
    if view.unit_held {
        tracing::warn!(
            hull = hull.0,
            "tacsim: +tv/lease: the hull's unit record is held"
        );
        return Err(TV_DARK.replace("{name}", &name));
    }
    if !view.is_prospector() {
        let (_, office, port_label) = office_of(w, &port);
        return Err(NO_BAY
            .replace("{name}", &name)
            .replace("{office}", &office)
            .replace("{port}", &port_label));
    }
    let lessor = lessor_of(w, &port).ok_or_else(|| failed("the rig lessor"))?;
    // 3. The berth.
    match berth_of(w, hull) {
        Berth::UnderWay => {
            return Err(LEASE_UNDER_WAY
                .replace("{name}", &name)
                .replace("{port}", &lessor.port));
        }
        Berth::Docked { map, .. } if map.entry == port.entry => {}
        Berth::Docked { .. } | Berth::Elsewhere => {
            return Err(LEASE_NOT_HERE.replace("{port}", &lessor.port));
        }
    }
    // 4. Nothing in flight.
    if let Some((_, line)) = crate::port::buyout::in_flight(w, &e) {
        return Err(line);
    }
    // 5. At the counter.
    let here = w.get(actor).and_then(|o| o.location);
    if lessor.counter.is_none() || here != lessor.counter {
        return Err(where_line(LEASE_WHERE, &lessor));
    }
    // 6. The licence seam (SM.5 fills it).
    licence_seam(w, actor)?;
    // 7. The slot.
    let (bay, bay_room) = view.bay().ok_or_else(|| failed("the hull's bay"))?;
    if !crate::port::hull::bay_laid_up(w, view.row, bay) {
        return Err(failed("the bay is not laid up"));
    }
    let bay_slot =
        mush_sim::SlotId::new(bay.slot.id.as_str()).ok_or_else(|| failed("the bay slot"))?;
    let max = u8::try_from(bay.slot.maximum_items).unwrap_or(u8::MAX);
    let taken: Vec<u8> = live_on_hull(w, hull)
        .iter()
        .map(|record| record.slot)
        .collect();
    let slot = match named {
        Some(n) if n > max => return Err(LEASE_USAGE.to_owned()),
        Some(n) if taken.contains(&n) => {
            return Err(SLOT_TAKEN
                .replace("{n}", &n.to_string())
                .replace("{name}", &name));
        }
        Some(n) => n,
        None => (1..=max)
            .find(|n| !taken.contains(n))
            .ok_or_else(|| BAY_FULL.replace("{name}", &name))?,
    };
    let occupied = w
        .module_state::<SpatialState>()
        .map_or(0, |st| st.occupants(bay_room, Some(&bay_slot)));
    if occupied >= usize::try_from(bay.slot.maximum_items).unwrap_or(usize::MAX) {
        return Err(BAY_FULL.replace("{name}", &name));
    }
    // 8. A free rig: the lowest-numbered rig in the shed, live, titled to Occator,
    // with no live lease from it and no live record.
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| failed("the layer"))?;
    let shed = lessor.shed;
    let rig = port
        .map
        .vehicles
        .as_ref()
        .into_iter()
        .flat_map(|row| row.rigs.iter())
        .filter(|rig| {
            let tv = ObjId(rig.tv);
            crate::port::hull::live(w, rig.tv)
                && shed.is_some_and(|shed| {
                    w.module_state::<SpatialState>().is_some_and(|st| {
                        matches!(st.parent_of(tv), SpatialParent::Interior { space, .. } if space == shed)
                    })
                })
                && crate::port::coop::titled_to(w, tv, &lessor.principal)
                && occator_lease_on(w, &lessor.principal, tv).is_none()
                && store.tv_lease_on(tv).is_none()
        })
        .min_by_key(|rig| rig.n)
        .ok_or_else(|| NO_RIG_FREE.to_owned())?;
    // 9. The funds, after the settle a `go` would run first.
    let (pilot_account, available) =
        funds(w, actor).ok_or_else(|| failed("the pilot's account"))?;
    let fee_minor = minor(w, lessor.pool.fee_major).ok_or_else(|| failed("the fee"))?;
    let deposit_minor = minor(w, lessor.pool.deposit_major).ok_or_else(|| failed("the deposit"))?;
    let now = w.world_ic_now().at;
    let (settle, left) = plan_settle(w, actor, &lessor.principal, now, available);
    let total = fee_minor
        .checked_add(deposit_minor)
        .ok_or_else(|| failed("the total"))?;
    if left < total {
        return Err(LEASE_FUNDS
            .replace("{total}", &money(ctx, total))
            .replace("{fee}", &money(ctx, fee_minor))
            .replace("{deposit}", &money(ctx, deposit_minor))
            .replace("{available}", &money(ctx, left)));
    }
    // 10. The economy, Occator's account and scope, the hopper vault.
    let economy = w
        .module_state::<EconomyStore>()
        .ok_or_else(|| failed("the economy"))?;
    if economy.is_frozen() {
        return Err(failed("the economy is frozen"));
    }
    let lessor_account = lessor.account.ok_or_else(|| failed("Occator's account"))?;
    if economy.snapshot().account(lessor_account).is_none() {
        return Err(failed("Occator's account"));
    }
    let tv = ObjId(rig.tv);
    if rig
        .vault
        .is_none_or(|vault| economy.snapshot().vault(VaultId(vault)).is_none())
    {
        return Err(failed("the rig's hopper vault"));
    }
    let term_secs = i64::from(lessor.pool.term_ic_days)
        .checked_mul(DAY)
        .ok_or_else(|| failed("the term"))?;
    Ok(TakePlan {
        hull,
        name,
        slot,
        bay_room,
        bay_slot,
        tv,
        mark: rig.mark.clone(),
        pilot_account,
        lessor_account,
        fee_minor,
        deposit_minor,
        term_secs,
        settle,
        lessor,
    })
}

/// The reply line of a take, preview or signed.
fn take_line(ctx: &Ctx<'_>, template: &str, plan: &TakePlan) -> String {
    template
        .replace("{mark}", &plan.mark)
        .replace("{name}", &plan.name)
        .replace("{n}", &plan.slot.to_string())
        .replace("{fee}", &money(ctx, plan.fee_minor))
        .replace("{days}", &plan.lessor.pool.term_ic_days.to_string())
        .replace("{deposit}", &money(ctx, plan.deposit_minor))
}

/// End a lease this take offered (a refusal after the offer: `coop::unwind_lease`'s
/// shape).
fn unwind_lease(ctx: &mut Ctx<'_>, lease: ContractId, lessor: &PrincipalId, tv: ObjId) {
    let key = LeaseKey {
        lessor: lessor.clone(),
        premises: tv,
    };
    match mush_economy::end_lease(ctx, lease, &key) {
        Ok(outcome) => {
            tracing::error!(
                lease = lease.0,
                ?outcome,
                "tacsim: +tv/lease ended its own lease"
            );
        }
        Err(error) => {
            tracing::error!(lease = lease.0, %error, "tacsim: +tv/lease could not end its own lease");
        }
    }
}

/// **`+tv[/n]/lease [go]`** — the take (doc 20 §4.3).
#[allow(clippy::too_many_lines)] // one ordered write list with each step's residue
fn take(ctx: &mut Ctx<'_>, named: Option<u8>, go: bool) -> Result<(), CmdError> {
    let plan = plan_take(ctx, named).map_err(CmdError::msg)?;
    let now = ctx.w.world_ic_now().at;
    if !go {
        for step in &plan.settle {
            let line = settle_line(ctx, step, now);
            say(ctx, line);
        }
        let line = take_line(ctx, LEASE_PREVIEW, &plan);
        say(ctx, line);
        return Ok(());
    }
    let actor = ctx.actor;
    // 0. The term settle: its own committed unit (reviewer F5).
    let wrote = run_settle(ctx, &plan.settle, plan.pilot_account, &plan.lessor);
    let refuse = |ctx: &mut Ctx<'_>, wrote: bool| {
        if wrote {
            refuse_after_writes(ctx, LEASE_FAILED)
        } else {
            CmdError::msg(LEASE_FAILED)
        }
    };
    // a. The lease (decision D7: the Trust's 1-minor Manual rent).
    let lease = match mush_economy::offer_lease(
        ctx,
        LeaseSpec {
            name: SmolStr::new(format!("Occator rig lease, {}", plan.mark)),
            lessor: plan.lessor.principal.clone(),
            scope: Some(plan.lessor.scope),
            tenants: vec![PrincipalId::Object(actor)],
            premises: plan.tv,
            rent: ObligationTemplate {
                debtor: PrincipalId::Object(actor),
                creditor: plan.lessor.principal.clone(),
                debit_account: plan.pilot_account,
                credit_account: plan.lessor_account,
                amount: Amount::from_minor(1),
                schedule: EconomicSchedule::new(EconomicClock::Manual, i64::MAX, None, 1),
                grace: GracePolicy::default(),
                notify_on_success: false,
            },
        },
    ) {
        Ok(lease) => lease,
        Err(error) => {
            tracing::error!(%error, "tacsim: +tv/lease: the lease was not offered");
            return Err(refuse(ctx, wrote));
        }
    };
    // ---- the offer wrote: every refusal from here pins the queue ----
    // b. The pilot accepts, as themselves; the lease must be Active.
    let accepted = matches!(
        mush_economy::accept_contract(ctx, lease),
        Ok(mush_economy::AcceptOutcome::Accepted { .. })
    );
    let active = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().contract(lease).map(|c| c.state))
        == Some(ContractState::Active);
    if !(accepted && active) {
        tracing::error!(
            lease = lease.0,
            "tacsim: +tv/lease: the lease did not come into force"
        );
        unwind_lease(ctx, lease, &plan.lessor.principal, plan.tv);
        return Err(refuse_after_writes(ctx, LEASE_FAILED));
    }
    // c. ONE post as the pilot: the term fee to Occator, public, and the deposit held
    // as an engine-owned reservation.
    let (Some(debit), credit) = (
        Amount::from_minor(plan.fee_minor).checked_neg(),
        Amount::from_minor(plan.fee_minor),
    ) else {
        unwind_lease(ctx, lease, &plan.lessor.principal, plan.tv);
        return Err(refuse_after_writes(ctx, LEASE_FAILED));
    };
    let post = EconomyPlan {
        description: format!("Occator rig lease {} for #{}", plan.mark, actor.0),
        visibility: EconomicVisibility::Public,
        hold_ops: vec![HoldOp::Place {
            account: plan.pilot_account,
            amount: Amount::from_minor(plan.deposit_minor),
            beneficiary: Some(plan.lessor.principal.clone()),
            purpose: HoldPurpose::Reservation {
                kind: SmolStr::new_static(DEPOSIT_KIND),
                id: lease.0,
            },
            expires: None,
        }],
        ..EconomyPlan::new(
            IdempotencyKey::new(LEASE_KEY_NAMESPACE, lease.0.to_string()),
            PrincipalId::Object(actor),
            EconomicCause::Internal {
                kind: SmolStr::new_static(LEASE_KEY_NAMESPACE),
                id: lease.0,
            },
            vec![
                RequestedLine {
                    account: plan.pilot_account,
                    delta: debit,
                    memo: SmolStr::new_static(LEASE_KEY_NAMESPACE),
                },
                RequestedLine {
                    account: plan.lessor_account,
                    delta: credit,
                    memo: SmolStr::new_static(LEASE_KEY_NAMESPACE),
                },
            ],
        )
    };
    match mush_economy::post(ctx, &post) {
        Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {}
        other => {
            tracing::error!(lease = lease.0, outcome = ?other, "tacsim: +tv/lease: the fee and deposit did not post");
            unwind_lease(ctx, lease, &plan.lessor.principal, plan.tv);
            return Err(refuse_after_writes(ctx, LEASE_FAILED));
        }
    }
    // ---- past the post nothing is fallible (the enrolment's L6 rule) ----
    // d. The deposit's hold id.
    let hold = ctx.w.module_state::<EconomyStore>().and_then(|store| {
        store
            .snapshot()
            .holds_of(plan.pilot_account)
            .find(|hold| {
                hold.status == HoldStatus::Held
                    && hold.purpose
                        == (HoldPurpose::Reservation {
                            kind: SmolStr::new_static(DEPOSIT_KIND),
                            id: lease.0,
                        })
            })
            .map(|hold| hold.id.0)
    });
    let hold = hold.unwrap_or_else(|| {
        tracing::error!(
            lease = lease.0,
            "tacsim: +tv/lease: invariant breach: the posted deposit hold cannot be found; the \
             lease stands; repair the tacsim:tvlease: record's hold by hand"
        );
        0
    });
    // e. The placement: into the bay's compartment (step 7 proved room).
    if let Err(error) =
        mush_sim::place_interior(ctx, plan.tv, plan.bay_room, Some(plan.bay_slot.clone()))
    {
        tracing::error!(
            lease = lease.0,
            tv = plan.tv.0,
            %error,
            "tacsim: +tv/lease: invariant breach: the rig could not be placed in the bay; the \
             lease stands; staff place it by hand"
        );
    }
    // f. The record.
    let record = TvLease {
        schema: TV_LEASE_SCHEMA,
        lease: lease.0,
        tv: plan.tv,
        mark: plan.mark.clone(),
        pilot: actor,
        hull: plan.hull,
        slot: plan.slot,
        hold,
        fee_minor: plan.fee_minor,
        deposit_minor: plan.deposit_minor,
        term_secs: plan.term_secs,
        leased_at_ic: now,
        term_ends_ic: now.saturating_add(plan.term_secs),
        terms_paid: 1,
        status: TvLeaseStatus::Active,
        ended_at_ic: None,
        repossessed: false,
    };
    if let Err(error) = record.check() {
        tracing::error!(lease = lease.0, %error, "tacsim: +tv/lease: the record does not check; it is written anyway so the lease is not lost");
    }
    put_tv_lease(ctx, &record);
    // g. The reply.
    let line = take_line(ctx, LEASED, &plan);
    say(ctx, line);
    Ok(())
}

// ---- the return --------------------------------------------------------------------

/// Everything the return resolved before its first write.
struct ReturnPlan {
    lessor: Lessor,
    record: TvLease,
    settle: Vec<Settle>,
    pilot_account: Option<AccountId>,
}

/// **The return's checks, in order** (reviewer F1(b): keyed on the lessee's own
/// live record, not on a live charter). Read-only; `Err` carries the refusal.
#[allow(clippy::too_many_lines)] // one ordered check list
fn plan_return(ctx: &Ctx<'_>, named: Option<u8>) -> Result<ReturnPlan, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    let port = programme_port(w).ok_or_else(|| NOT_OPEN_HERE.to_owned())?;
    let failed = |what: &str| {
        tracing::error!(
            what,
            pilot = actor.0,
            "tacsim: +tv/lease return could not resolve a step"
        );
        LEASE_FAILED.to_owned()
    };
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| NOT_OPEN_HERE.to_owned())?;
    let charter = crate::port::recall::chartered(ctx);
    let charter_name = charter
        .as_ref()
        .map_or_else(|_| String::new(), |e| hull_name(w, e.hull));
    // 1. The actor's live records.
    if store.tv_lease_unreadable_for(actor) {
        tracing::error!(
            pilot = actor.0,
            "tacsim: +tv/lease return found an unreadable tacsim:tvlease: record"
        );
        return Err(dark_line(
            w,
            store,
            actor,
            charter.as_ref().ok().map(|e| e.hull),
        ));
    }
    let records = live_records(w, actor);
    let n = named.unwrap_or(1);
    if records.is_empty() {
        return match charter {
            Err(_) => Err(NO_CHARTER.to_owned()),
            Ok(_) => Err(NO_RIG
                .replace("{n}", &n.to_string())
                .replace("{name}", &charter_name)),
        };
    }
    // 2. The slot.
    let record = match named {
        Some(n) => match records.iter().find(|record| record.slot == n) {
            Some(record) => record.clone(),
            None => {
                let name = records
                    .first()
                    .map_or_else(String::new, |record| hull_name(w, record.hull));
                return Err(NO_RIG
                    .replace("{n}", &n.to_string())
                    .replace("{name}", &name));
            }
        },
        None => match records.as_slice() {
            [only] => only.clone(),
            _ => return Err(WHICH_RIG.to_owned()),
        },
    };
    let name = hull_name(w, record.hull);
    // 3. The record's hull view and its governing enrolment.
    let view = hull_view(w, record.hull).ok_or_else(|| failed("the hull's view"))?;
    if view.unit_held {
        return Err(TV_DARK.replace("{name}", &name));
    }
    if !view.is_prospector() {
        let (_, office, port_label) = office_of(w, &port);
        return Err(NO_BAY
            .replace("{name}", &name)
            .replace("{office}", &office)
            .replace("{port}", &port_label));
    }
    let enrolment = store
        .enrolment_for_hull(record.hull)
        .cloned()
        .ok_or_else(|| failed("the hull's enrolment"))?;
    let lessor = lessor_of(w, &port).ok_or_else(|| failed("the rig lessor"))?;
    // 4. The economy lease is read, not checked (reviewer F15): live, or ended by
    // staff; either is returnable.
    // 5. The rig is in this hull's bay.
    let in_bay = crate::port::vehicle::bay_rooms(w, &view).iter().any(|room| {
        w.module_state::<SpatialState>().is_some_and(|st| {
            matches!(st.parent_of(record.tv), SpatialParent::Interior { space, .. } if space == *room)
        })
    });
    let slot = record.slot.to_string();
    if !in_bay {
        return Err(RIG_OUT.replace("{n}", &slot).replace("{name}", &name));
    }
    // 6. The hopper is empty.
    match hopper_load(ctx, record.tv, Some(&lessor.pool.unit)) {
        Ok((0, _)) => {}
        Ok(_) => return Err(HOPPER_FULL.replace("{n}", &slot)),
        Err(error) => {
            tracing::error!(tv = record.tv.0, %error, "tacsim: +tv/lease return could not read the hopper");
            return Err(LEASE_FAILED.to_owned());
        }
    }
    // 7. The berth.
    match berth_of(w, record.hull) {
        Berth::UnderWay => {
            return Err(LEASE_UNDER_WAY
                .replace("{name}", &name)
                .replace("{port}", &lessor.port));
        }
        Berth::Docked { map, .. } if map.entry == port.entry => {}
        Berth::Docked { .. } | Berth::Elsewhere => {
            return Err(LEASE_NOT_HERE.replace("{port}", &lessor.port));
        }
    }
    // 8. Nothing in flight.
    if let Some((_, line)) = crate::port::buyout::in_flight(w, &enrolment) {
        return Err(line);
    }
    // 9. At the counter.
    let here = w.get(actor).and_then(|o| o.location);
    if lessor.counter.is_none() || here != lessor.counter {
        return Err(where_line(LEASE_WHERE, &lessor));
    }
    // 10. An Active record's deposit resolves (reviewer finding correctness/F1): its
    // hold is this lease's own reservation, so the release at step b cannot meet a
    // record that names no deposit after the lease has ended.
    if record.status == TvLeaseStatus::Active
        && let Err(error) = deposit_hold(w, &record)
    {
        tracing::error!(
            lease = record.lease,
            %error,
            "tacsim: +tv/lease return: the record's deposit hold does not resolve; staff \
             repair the tacsim:tvlease: record's hold"
        );
        return Err(LEASE_FAILED.to_owned());
    }
    let funds = funds(w, actor);
    let now = w.world_ic_now().at;
    let (settle, _) = plan_settle(
        w,
        actor,
        &lessor.principal,
        now,
        funds.map_or(0, |(_, available)| available),
    );
    Ok(ReturnPlan {
        lessor,
        record,
        settle,
        pilot_account: funds.map(|(account, _)| account),
    })
}

/// Whether the settle forfeits `record`.
fn settle_forfeits(steps: &[Settle], record: &TvLease) -> bool {
    steps
        .iter()
        .any(|step| matches!(step, Settle::Forfeit { record: r, .. } if r.lease == record.lease))
}

/// **`+tv[/n]/lease return [go]`** — the return (doc 20 §4.3).
fn give_back(ctx: &mut Ctx<'_>, named: Option<u8>, go: bool) -> Result<(), CmdError> {
    let plan = plan_return(ctx, named).map_err(CmdError::msg)?;
    let now = ctx.w.world_ic_now().at;
    let forfeit_now =
        plan.record.status == TvLeaseStatus::Forfeit || settle_forfeits(&plan.settle, &plan.record);
    let slot = plan.record.slot.to_string();
    if !go {
        for step in &plan.settle {
            let line = settle_line(ctx, step, now);
            say(ctx, line);
        }
        let template = if forfeit_now {
            RETURN_PREVIEW_FORFEIT
        } else {
            RETURN_PREVIEW
        };
        let line = template
            .replace("{mark}", &plan.record.mark)
            .replace("{deposit}", &money(ctx, plan.record.deposit_minor))
            .replace("{n}", &slot);
        say(ctx, line);
        return Ok(());
    }
    // 0. The term settle: its own committed unit (reviewer F5).
    let wrote = match plan.pilot_account {
        Some(account) => run_settle(ctx, &plan.settle, account, &plan.lessor),
        None => false,
    };
    // a. The lease ends (`AlreadyEnded` is success: a lease staff ended).
    let key = LeaseKey {
        lessor: plan.lessor.principal.clone(),
        premises: plan.record.tv,
    };
    match mush_economy::end_lease(ctx, ContractId(plan.record.lease), &key) {
        Ok(LeaseEnd::Ended | LeaseEnd::AlreadyEnded) => {}
        Ok(LeaseEnd::Retrying) => {
            tracing::error!(
                lease = plan.record.lease,
                "tacsim: +tv/lease return: the lease end is retrying in the economy sweep"
            );
            return Err(refuse_after_writes(ctx, LEASE_FAILED));
        }
        Err(error) => {
            tracing::error!(lease = plan.record.lease, %error, "tacsim: +tv/lease return: the lease did not end");
            return Err(if wrote {
                refuse_after_writes(ctx, LEASE_FAILED)
            } else {
                CmdError::msg(LEASE_FAILED)
            });
        }
    }
    // ---- past the lease end nothing refuses ----
    // The record as the settle left it.
    let record = ctx
        .w
        .module_state::<TacsimStore>()
        .and_then(|store| match store.tv_lease(plan.record.lease) {
            Some(TvLeaseState::Present(record)) => Some(record.clone()),
            Some(TvLeaseState::Unreadable { .. }) | None => None,
        })
        .unwrap_or_else(|| plan.record.clone());
    // b. The deposit, when Active (reviewer finding correctness/F1): a deposit that will
    // not settle refuses the return **before** the record goes `Returned`, so the record
    // stays live and the next `+tv/lease return go` runs again — its lease answers
    // `AlreadyEnded` and the settle is idempotent. A `Returned` record over a `Held`
    // engine-owned deposit would strand it: nobody but this seam can move it.
    let forfeit = match record.status {
        TvLeaseStatus::Active => match settle_deposit(ctx, &record, None) {
            Ok(deposit) => deposit == Deposit::Captured,
            Err(error) => {
                tracing::error!(
                    lease = record.lease,
                    %error,
                    "tacsim: +tv/lease return: the deposit did not settle; the lease record \
                     stays live and the return runs again"
                );
                return Err(refuse_after_writes(ctx, LEASE_FAILED));
            }
        },
        TvLeaseStatus::Forfeit | TvLeaseStatus::Returned => true,
    };
    // c. The placement: back in the shed.
    match plan.lessor.shed {
        Some(shed) => {
            if let Err(error) = mush_sim::place_interior(ctx, record.tv, shed, None) {
                tracing::error!(tv = record.tv.0, %error, "tacsim: +tv/lease return: invariant breach: the rig could not go back to the shed; staff place it by hand");
            }
        }
        None => tracing::error!(
            tv = record.tv.0,
            "tacsim: +tv/lease return: Occator has no shed recorded"
        ),
    }
    // d. The record.
    let returned = TvLease {
        status: TvLeaseStatus::Returned,
        ended_at_ic: Some(now),
        ..record.clone()
    };
    put_tv_lease(ctx, &returned);
    // e. The reply.
    let line = if forfeit {
        RETURNED_FORFEIT.replace("{mark}", &record.mark)
    } else {
        RETURNED
            .replace("{mark}", &record.mark)
            .replace("{deposit}", &money(ctx, record.deposit_minor))
    };
    say(ctx, line);
    Ok(())
}

// ---- a charter's end with rigs aboard --------------------------------------------------

/// **Doc 20 cut SM.3a (D16)**: every rig `hull` carries goes back to Occator
/// because the hull's charter is ending (a revoke, the dormancy recall, the return
/// arm). The rigs are every placement in the bay **and every live record naming the
/// hull** (reviewer finding correctness/F2: a live record whose rig is out of the
/// bay — the take's placement breach, a staff move — is repossessed from wherever
/// it stands, so no record or deposit outlives the charter). All checks first;
/// nothing written on `Err`. Per rig, in slot order, its own committed unit:
///
/// 1. **the deposit, by the term** (D16; the engine never debits the pilot) —
///    released within the paid term or when staff had already ended the economy
///    lease, captured whole past it, nothing for a Forfeit record. It runs **before
///    the lease ends** (reviewer finding correctness/F1), and through the idempotent
///    `settle_deposit`: a deposit that will not settle stops the repossession with
///    nothing of that rig written, so the record stays live, the hull stays held
///    and the recall arm's next fire retries — never a `Returned` record over a
///    `Held` engine-owned deposit that nobody, staff included, can move again. The
///    choice "staff ended it" is read from the economy lease before this run ends
///    it, and a retry reads how the deposit was settled from the hold itself;
/// 2. the lease ended (`AlreadyEnded` is success);
/// 3. the rig back in the shed;
/// 4. the record `Returned` and `repossessed`;
/// 5. the pilot told.
///
/// # Errors
///
/// Why the bay cannot be cleared, for the log: an unlisted rig, an unreadable
/// record, a hopper not empty or unreadable, a deposit hold that does not resolve
/// or will not settle, a frozen economy, Occator unresolved, or a lease that will
/// not end (the rigs before it stand returned).
#[allow(clippy::too_many_lines)] // one check list, then one ordered write list per rig
pub(crate) fn repossess(ctx: &mut Ctx<'_>, hull: ObjId) -> Result<usize, String> {
    let w: &World = ctx.w;
    let view =
        hull_view(w, hull).ok_or_else(|| format!("hull #{} has no programme view", hull.0))?;
    let port = programme_port(w).ok_or_else(|| "no programme port".to_owned())?;
    let lessor =
        lessor_of(w, &port).ok_or_else(|| "the programme port authors no rig lessor".to_owned())?;
    let store = w
        .module_state::<TacsimStore>()
        .ok_or_else(|| "the layer's state is absent".to_owned())?;
    // Every check first.
    let mut records: Vec<TvLease> = Vec::new();
    for rig in rigs_in_bay(w, &view) {
        let record = store.tv_lease_on(rig.tv).cloned().ok_or_else(|| {
            format!(
                "rig #{} in the bay has no readable live lease record",
                rig.tv.0
            )
        })?;
        records.push(record);
    }
    for record in live_on_hull(w, hull) {
        if !records.iter().any(|seen| seen.lease == record.lease) {
            records.push(record);
        }
    }
    for record in &records {
        match hopper_load(ctx, record.tv, Some(&lessor.pool.unit)) {
            Ok((0, _)) => {}
            Ok(_) => {
                return Err(format!(
                    "rig {} (#{})'s hopper is not empty",
                    record.mark, record.tv.0
                ));
            }
            Err(error) => {
                return Err(format!(
                    "rig #{}'s hopper cannot be read: {error}",
                    record.tv.0
                ));
            }
        }
        if record.status == TvLeaseStatus::Active
            && let Err(error) = deposit_hold(w, record)
        {
            return Err(format!(
                "rig {}'s deposit does not resolve ({error}); staff repair the \
                 tacsim:tvlease:{} record's hold",
                record.mark, record.lease
            ));
        }
    }
    if records.is_empty() {
        return Ok(0);
    }
    let economy_ok = w
        .module_state::<EconomyStore>()
        .is_some_and(|economy| !economy.is_frozen());
    if !economy_ok {
        return Err("the economy is frozen".to_owned());
    }
    let lessor_account = lessor
        .account
        .ok_or_else(|| "Occator's account does not resolve".to_owned())?;
    let shed = lessor
        .shed
        .ok_or_else(|| "Occator has no shed recorded".to_owned())?;
    let name = hull_name(w, hull);
    records.sort_by_key(|record| (record.slot, record.lease));
    let mut done = 0_usize;
    for record in records {
        let now = ctx.w.world_ic_now().at;
        // 1. The deposit, by the term (D16), before the lease ends.
        let forfeit = match record.status {
            TvLeaseStatus::Forfeit | TvLeaseStatus::Returned => true,
            TvLeaseStatus::Active => {
                let ended_by_staff = !economy_lease_live(ctx.w, &lessor.principal, &record);
                let capture = (now >= record.term_ends_ic && !ended_by_staff)
                    .then_some((&lessor.principal, lessor_account));
                match settle_deposit(ctx, &record, capture) {
                    Ok(deposit) => deposit == Deposit::Captured,
                    Err(error) => {
                        return Err(format!(
                            "rig {}'s deposit (hold #{}) will not settle: {error}; the lease \
                             record stays live and the recall retries",
                            record.mark, record.hold
                        ));
                    }
                }
            }
        };
        // 2. The lease.
        let key = LeaseKey {
            lessor: lessor.principal.clone(),
            premises: record.tv,
        };
        match mush_economy::end_lease(ctx, ContractId(record.lease), &key) {
            Ok(LeaseEnd::Ended | LeaseEnd::AlreadyEnded) => {}
            Ok(LeaseEnd::Retrying) => {
                return Err(format!("rig {}'s lease end is retrying", record.mark));
            }
            Err(error) => return Err(format!("rig {}'s lease will not end: {error}", record.mark)),
        }
        // 3. The rig, back in the shed (from the bay, or wherever a live record's rig
        // stands).
        let in_shed = ctx.w.module_state::<SpatialState>().is_some_and(|st| {
            matches!(st.parent_of(record.tv), SpatialParent::Interior { space, .. } if space == shed)
        });
        if !in_shed && let Err(error) = mush_sim::place_interior(ctx, record.tv, shed, None) {
            tracing::error!(tv = record.tv.0, %error, "tacsim: a repossession: invariant breach: the rig could not go back to the shed; staff place it by hand");
        }
        // 4. The record.
        let returned = TvLease {
            status: TvLeaseStatus::Returned,
            ended_at_ic: Some(now),
            repossessed: true,
            ..record.clone()
        };
        put_tv_lease(ctx, &returned);
        // 5. The pilot, told.
        let line = if forfeit {
            REPOSSESSED_FORFEIT
                .replace("{mark}", &record.mark)
                .replace("{name}", &name)
        } else {
            REPOSSESSED
                .replace("{mark}", &record.mark)
                .replace("{name}", &name)
                .replace("{deposit}", &money(ctx, record.deposit_minor))
        };
        crate::port_tick::notify(ctx, record.pilot, &line);
        tracing::info!(
            lease = record.lease,
            tv = record.tv.0,
            hull = hull.0,
            "tacsim: a rig was repossessed at its charter's end"
        );
        done = done.saturating_add(1);
    }
    Ok(done)
}

/// **Whether `view`'s hull still carries a rig, and its `{n}`** (doc 20 cut SM.3a;
/// reviewer finding correctness/F2): a placement in the bay **or a live lease
/// record naming the hull**, so a rig whose record outlives its placement in the
/// bay (the take's placement breach, a staff move) still holds the refit, the
/// buyout and the recall until it is returned or repossessed. `{n}` is the lowest
/// live slot among them, else `?`; `None` when the hull carries none.
pub(crate) fn bay_slot_word(w: &World, view: &HullView<'_>) -> Option<String> {
    let rigs = rigs_in_bay(w, view);
    let records = live_on_hull(w, view.hull());
    if rigs.is_empty() && records.is_empty() {
        return None;
    }
    Some(
        rigs.iter()
            .map(|rig| rig.slot)
            .chain(records.iter().map(|record| record.slot))
            .filter(|slot| *slot > 0)
            .min()
            .map_or_else(|| "?".to_owned(), |slot| slot.to_string()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TvLease {
        TvLease {
            schema: TV_LEASE_SCHEMA,
            lease: 41,
            tv: ObjId(900),
            mark: "OWP-TV-01".to_owned(),
            pilot: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            hold: 7,
            fee_minor: 140_000,
            deposit_minor: 500_000,
            term_secs: 168 * DAY,
            leased_at_ic: 1_000,
            term_ends_ic: 1_000 + 168 * DAY,
            terms_paid: 1,
            status: TvLeaseStatus::Active,
            ended_at_ic: None,
            repossessed: false,
        }
    }

    /// Doc 20 cut SM.3a: the record's wire is strict, `repossessed` and
    /// `ended_at_ic` are skipped when unset, and `check` holds its invariants.
    #[test]
    fn sm3a_the_lease_record_is_strict_and_checks() {
        let record = sample();
        assert_eq!(record.check(), Ok(()));
        let value = serde_json::to_value(&record).expect("serializes");
        assert!(value.get("repossessed").is_none() && value.get("ended_at_ic").is_none());
        assert_eq!(value["status"], "active");
        let back: TvLease = serde_json::from_value(value.clone()).expect("round-trips");
        assert_eq!(back, record);
        let mut extra = value;
        extra["surprise"] = serde_json::json!(1);
        assert!(serde_json::from_value::<TvLease>(extra).is_err());
        for broken in [
            TvLease {
                schema: 2,
                ..sample()
            },
            TvLease {
                slot: 0,
                ..sample()
            },
            TvLease {
                fee_minor: 0,
                ..sample()
            },
            TvLease {
                deposit_minor: -1,
                ..sample()
            },
            TvLease {
                term_secs: 0,
                ..sample()
            },
            TvLease {
                term_ends_ic: 1_000,
                ..sample()
            },
            TvLease {
                mark: String::new(),
                ..sample()
            },
            TvLease {
                status: TvLeaseStatus::Returned,
                ..sample()
            },
        ] {
            assert!(broken.check().is_err(), "{broken:?} passed its check");
        }
        assert_eq!(tv_lease_key(41).as_str(), "tacsim:tvlease:41");
    }

    /// Doc 20 cut SM.3a (lead ruling 9): the terms due past the end are whole
    /// terms, the first counted at the boundary.
    #[test]
    fn sm3a_terms_due_are_whole_terms_from_the_end() {
        let record = sample();
        let end = record.term_ends_ic;
        assert_eq!(owed_of(&record, end - 1), None);
        assert_eq!(owed_of(&record, end), Some((1, 140_000)));
        assert_eq!(owed_of(&record, end + 2 * DAY), Some((1, 140_000)));
        assert_eq!(owed_of(&record, end + record.term_secs), Some((2, 280_000)));
    }
}
