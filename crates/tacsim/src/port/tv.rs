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
//! number.
//!
//! **Doc 20 cut SM.3b** gives the verb its acts (§1 beats 4 and 9, §4.3, §8, §10): `+tv/<n>
//! launch|recover|dump` previews and commits nothing; `go` commits exactly the preview, as
//! ONE `tacsim_surface_act` submission by the operator (`crate::surface::act`), so the verb
//! moves to the authoritative class. A leading slot switch `/1`…`/4` names the rig, else the
//! rig the actor named last (`RigPointer`, RAM, a convenience), else the only rig. `+tv`
//! status prints per rig where it is and its charge now, and an act in flight on a second
//! line. A charter's end holds the hull (never strips a rig) while a rig is out on a site or
//! mid-act (`repossess`, decision 13).

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
+tv/bay in|out  from the cargo hold, enter the carrier's vehicle bay; from the bay, return to the hold.
+tv/<n> board  in the carrier's bay, take the rig's physical helm billet with current OperateCrew permission; then use +focus and @station/handoff yourself.
+tv/<n> leave  step from a rig in the bay into the carrier and give back your billet. Field units must return first.
+tv/lease [go]  at Occator's counter in Ceres Freeport, with your Dray-P docked: lease a terrain vehicle into a free bay slot, 1400.00cr a term of 168 IC days and a 5000.00cr deposit held on your account. A lease needs a prospecting licence in force or in its grace (+licence). Without go it shows the terms and signs nothing.
+tv/<n>/permissions [go]  at Occator's counter: inspect or request an explicit operational loan for the actual rental. Operate/crew and command may be delegated within this lease; deploy and recover/load may not. No title or refit right, and no additional charge.
+tv/<n> lease return [go]  at the same counter: hand rig n back, its hopper empty, and the deposit is released.
A lease past its term is re-debited at your next +tv/lease, or the deposit is forfeit.
+tv/<n> launch [mind=<mind>] [org=<org>] [go]  with your Dray-P settled on a site's pad and you aboard: send rig n down the stern ramp onto the site (300 IC s). A rig with a core flies the mind you name sealed aboard, filed:<licence> for a licence you hold (org=<org> for one your organisation holds) or, on an ungoverned core, draft:<name>; without one it flies Standard Doctrine.
+tv/<n> recover [go]  bring rig n up the ramp into the bay (300 IC s); a rig further off drives to the ramp first, even while the instruments are dark. With ore in its hopper it unloads into the hold next, on its own.
+tv/<n> drive <target> [go]  drive rig n over the ground to a pocket (C117), a field (C-NW), a block (C), a site cell (140,96) or a bearing (ne 300); the preview names the charge left after the trip home.
+tv[/<n>]/map [site] [family]  the site map around rig n, or the whole site: your own readings, the rigs and the hull.
+tv/<n> dump [go]  empty rig n's hopper onto the site's spoil, for good.
+tv/<n> unload [<t>] [go]  with rig n in the bay on a site's pad: move its hopper's ore into the hold, as much as the hold has room for (600 IC s), or <t> tonnes of the first ore.
+tv/<n> scan [go]  (alias survey) read the nine fields around rig n by richness, sharper from rock than from regolith; nothing from ice (600 IC s, 1,000 cu).
+tv/<n> sound [quiet] [go]  read every pocket of the field rig n stands in, heard across the site; quiet reads a quarter of it and nobody hears (1,200 IC s, 2,000 cu).
+tv/<n> mine [family] [cycles] [go]  (alias drill) mine the pocket rig n stands on, from your own reading of it: the family you name, else the one with most tonnes left; no number runs until the pocket, the hopper or the hold's room gives out (at most eight cycles). An unread pocket is cored one cycle (1,800 IC s, 3,600 on rock; 2,000 cu a cycle).
+tv/<n> mine stop  the cycle in flight finishes and yields; the run ends there.
+tv/<n> plan <pockets and blocks> [family=<good>] [go]  leave rig n working: mine the pockets you name and every pocket of a block you read fair or better, sounding a field first where you have only scanned it; it comes home and unloads at every full hopper, and parks when the hold has no room for another cycle or the list is worked out, within 288 IC h. It is never re-armed. While it works, the rig takes no other order.
+tv/<n> plan show  read rig n's plan back.  +tv/<n> plan clear  end it after the step in flight, or strike a finished one.
+tv/quiet [on|off]  stop (or start) hearing other rigs' soundings.
+tv/forget <site>|<body> [go]  strike your readings of a site, or of every site on a body, from your chart.
From the bay, a scan, a sounding or a drive goes down the ramp first. Mining is help mining.
Each act shows what it would do and commits nothing; go commits it. Without a number it addresses the rig you named last, or your only rig.";

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
/// **No licence in force** (doc 20 cut SM.5; §4.3, §9.1): the lease is the one
/// non-mining act that reads the prospecting licence.
pub const LEASE_LICENCE: &str = "Occator leases rigs only to a licensed prospector: +licence/buy takes the Freeport Registry's licence, from anywhere.";
/// A step that should not fail did.
pub const LEASE_FAILED: &str =
    "Occator could not complete it; nothing of the lease or the return was signed.";
/// **The grammar** (doc 20 cut SM.3b; §10): the status, the three acts and the lease.
/// Replaces SM.3a's `LEASE_USAGE`; no second constant is kept.
pub const TV_USAGE: &str = "Usage: +tv[/<n>]  or  +tv[/<n>]/<launch|recover|dump|scan> [go]  or  +tv[/<n>]/launch mind=<mind> [org=<org>] [go]  or  +tv[/<n>]/sound [quiet] [go]  or  +tv/<n> drive <target> [go]  or  +tv[/<n>]/mine [family] [cycles] [go] (alias drill)  or  +tv[/<n>]/mine stop  or  +tv[/<n>]/unload [<t>] [go]  or  +tv[/<n>]/plan <pockets|blocks> [family=<good>] [go]  or  +tv[/<n>]/plan show|clear  or  +tv[/<n>]/map [site] [family]  or  +tv[/<n>]/lease [return] [go]  or  +tv/forget <site>|<body> [go]  or  +tv/quiet [on|off]";
/// A rig within its term (doc 20 cut SM.3b: where it is and its charge): `{n}`, `{mark}`,
/// `{where}`, `{charge}`, `{span}`, `{hopper}`.
pub const RIG_LINE: &str = "Rig {n}: {mark}, {where}; charge {charge} %; leased from Occator for {span} more; hopper {hopper}.";
/// A rig past its term: `{n}`, `{mark}`, `{where}`, `{charge}`, `{fee}`, `{hopper}`.
pub const RIG_PAST_TERM: &str = "Rig {n}: {mark}, {where}; charge {charge} %; past its term: {fee} is owed at your next +tv/lease, or the deposit is forfeit; hopper {hopper}.";
/// A forfeit rig: `{n}`, `{mark}`, `{where}`, `{charge}`, `{hopper}`.
pub const RIG_FORFEIT: &str = "Rig {n}: {mark}, {where}; charge {charge} %; its deposit is forfeit: return it to Occator; hopper {hopper}.";
/// A rig in the bay with no live record: `{mark}`.
pub const RIG_UNLISTED: &str = "Rig ?: {mark}, in the bay; Occator's books for it cannot be read.";
/// An empty slot: `{n}`, `{counter}`, `{port}`.
pub const RIG_EMPTY: &str = "Rig {n}: empty. Occator leases rigs at the {counter} in {port}.";
/// A live record whose economy lease staff ended (reviewer F15): `{n}`, `{mark}`,
/// `{where}`, `{charge}`, `{counter}`, `{port}`, `{hopper}`.
pub const RIG_LEASE_ENDED: &str = "Rig {n}: {mark}, {where}; charge {charge} %; its lease from Occator has ended: hand it back at the {counter} in {port}; hopper {hopper}.";
/// The rigs cannot be read: `{name}`.
pub const TV_DARK: &str = "The {name}'s rigs cannot be read right now; nothing changes.";
/// A repossession within the term: `{mark}`, `{name}`, `{deposit}`.
pub const REPOSSESSED: &str = "Occator took {mark} back out of the {name}'s bay when her charter ended; your {deposit} deposit is released.";
/// A repossession past the term or of a forfeit rig: `{mark}`, `{name}`.
pub const REPOSSESSED_FORFEIT: &str = "Occator took {mark} back out of the {name}'s bay when her charter ended; its deposit was forfeit.";

// ---- doc 20 cut SM.3b: the acts (§1 beats 4 and 9, §4.3, §8, §10) ----------------------
//
// Placeholders: `{n}` the slot, `{mark}`, `{name}` the hull, `{site}` the site's name,
// `{ramp}` the egress label, `{port}`, `{counter}`, `{secs}` IC seconds, `{real}` ", about
// {span}" (empty at world rate 0), `{when}` (`due_words`), `{now}`/`{after}`/`{floor}`/
// `{charge}` whole percent, `{load}` "{q} t of {good}" joined ", ", `{m}` metres, `{cap}`,
// `{act}` the act word, `{slots}` "1 or 2", `{doing}` (`act::doing_words`), `{hopper}`
// (`hopper_text`).

/// Two rigs and none named: `{act}`, `{slots}`.
pub const WHICH_RIG_ACT: &str = "Which rig? +tv/<n> {act}, with n {slots}.";
/// The surface providers are not registered on this roster.
pub const TV_UNWIRED: &str = "Rigs cannot be worked on this server right now; nothing changes.";
/// Not aboard: `{name}`.
pub const TV_NOT_ABOARD: &str = "You work the rigs from aboard the {name}.";
/// Not on a site's pad: `{name}`.
pub const LAUNCH_NOT_SITE: &str =
    "The {name} is not settled on a site's pad; rigs go down the ramp only onto a site.";
/// Under way: `{name}`.
pub const LAUNCH_UNDER_WAY: &str = "The {name} is under way; nothing leaves the bay.";
/// A clearance filed: `{name}`, `{port}`.
pub const LAUNCH_CLEARED: &str = "The {name} is cleared for {port}; nothing leaves the bay.";
/// The rig is not in the bay: `{n}`.
pub const LAUNCH_OUT: &str = "Rig {n} is already out of the bay.";
/// An excursion still open: `{n}`.
pub const LAUNCH_OPEN: &str = "Rig {n} still has an excursion open; +tv/{n} recover closes it.";
/// A forfeit deposit: `{n}`, `{counter}`, `{port}`.
pub const LAUNCH_FORFEIT: &str = "Rig {n}'s deposit is forfeit; it stays in the bay until it goes back to the {counter} in {port}.";
/// An ended lease: `{n}`, `{counter}`, `{port}`.
pub const LAUNCH_ENDED: &str = "Rig {n}'s lease from Occator has ended; it stays in the bay until it goes back to the {counter} in {port}.";
/// The launch's preview: `{n}`, `{mark}`, `{ramp}`, `{site}`, `{secs}`, `{real}`, `{now}`,
/// `{after}`.
pub const LAUNCH_PREVIEW: &str = "Rig {n} ({mark}) would go down the {ramp} onto {site}: {secs} IC s{real}; charge {now} → {after} %. +tv/{n} launch go sends it.";
/// The launch, filed: `{n}`, `{ramp}`, `{when}`.
pub const LAUNCH_FILED: &str = "Rig {n} starts down the {ramp}: on the ground {when}.";
/// The launch, done: `{n}`, `{ramp}`, `{site}`.
pub const LAUNCHED: &str = "Rig {n} is down the {ramp} on {site}.";
/// **Doc 17h cut M8e E4**: the launch's end, told to the operator alone, when the licence named
/// had lapsed (or been revoked) at the dispatch and the rig flies Standard Doctrine: `{n}`,
/// `{licence}`, `{state}`.
pub const LAUNCHED_DOCTRINE: &str =
    "Licence {licence} is {state}: rig {n} flies Standard Doctrine sealed aboard.";
/// A recover with the rig in the bay already: `{n}`.
pub const RECOVER_IN_BAY: &str = "Rig {n} is in the bay already.";
/// A recover with the rig not on this hull's ground: `{n}`, `{name}`.
pub const RECOVER_ELSEWHERE: &str = "Rig {n} is not on the ground beside the {name}.";
/// A recover too far from the ramp: `{n}`, `{m}`, `{ramp}`. The rule is the ramp's cell or one
/// of the eight beside it (Chebyshev ≤ 1 cell) and `{m}` is that cell count × 10, so the words
/// promise no radius (the lead's ruling in the SM.3b fixer round, item 7).
pub const RECOVER_FAR: &str =
    "Rig {n} is about {m} m from the {ramp}; it climbs aboard from beside it.";
/// The recover's preview: `{n}`, `{mark}`, `{ramp}`, `{name}`, `{secs}`, `{real}`, `{now}`,
/// `{after}`, `{hopper}`, `{then}` (doc 20 cut SM.8: the unload that follows, [`RECOVER_THEN`],
/// [`RECOVER_THEN_PART`] or [`RECOVER_THEN_NONE`], empty for an empty hopper).
pub const RECOVER_PREVIEW: &str = "Rig {n} ({mark}) would climb the {ramp} into the {name}'s bay: {secs} IC s{real}; charge {now} → {after} %; hopper {hopper}{then}. +tv/{n} recover go brings it in.";
/// The recover's preview when the rig's battery row cannot be read (§4.1 "a TV cannot be
/// stranded by its battery": a recover spends nothing, so it never needs the battery — the
/// fixer round's item 3): `{n}`, `{mark}`, `{ramp}`, `{name}`, `{secs}`, `{real}`, `{hopper}`,
/// `{then}` (doc 20 cut SM.8: the unload that follows, [`RECOVER_THEN`], or empty).
pub const RECOVER_PREVIEW_UNREAD: &str = "Rig {n} ({mark}) would climb the {ramp} into the {name}'s bay: {secs} IC s{real}; its charge cannot be read; hopper {hopper}{then}. +tv/{n} recover go brings it in.";
/// The recover, filed: `{n}`, `{ramp}`, `{when}`.
pub const RECOVER_FILED: &str = "Rig {n} starts up the {ramp}: in the bay {when}.";
/// The recover, done: `{n}`, `{name}`.
pub const RECOVERED: &str = "Rig {n} is back in the {name}'s bay.";
/// A dump away from a site: `{n}`, `{name}`.
pub const DUMP_NOT_SITE: &str =
    "Rig {n} dumps only onto a site's spoil, with the {name} settled on its pad.";
/// A site with no spoil laid: `{site}`.
pub const DUMP_NO_SPOIL: &str = "{site} has no spoil heap laid out; nothing is dumped.";
/// An empty hopper: `{n}`.
pub const DUMP_EMPTY: &str = "Rig {n}'s hopper is empty.";
/// The dump's preview: `{n}`, `{mark}`, `{load}`, `{site}`.
pub const DUMP_PREVIEW: &str = "Rig {n} ({mark}) would dump {load} onto the spoil at {site}; it cannot be taken back. +tv/{n} dump go dumps it.";
/// The dump, filed: `{n}`.
pub const DUMP_FILED: &str = "Rig {n} opens its hopper over the spoil.";
/// The dump, done: `{n}`, `{load}`, `{site}`.
pub const DUMPED: &str = "Rig {n} dumps {load} onto the spoil at {site}.";
/// A refused finish: `{n}`, `{act}`, `{why}` (one of [`WHY_NEVER_LEFT`],
/// [`WHY_NOT_BESIDE`], [`WHY_HOPPER_CHANGED`], [`WHY_SPOIL_GONE`]).
pub const ACT_FAILED: &str = "Rig {n}'s {act} did not happen: {why}.";
/// `ACT_FAILED`'s `{why}` for a launch whose rig never left the bay.
pub const WHY_NEVER_LEFT: &str = "it never left the bay";
/// `ACT_FAILED`'s `{why}` for a recover whose rig was not beside the ramp: `{ramp}`.
pub const WHY_NOT_BESIDE: &str = "it was not beside the {ramp}";
/// `ACT_FAILED`'s `{why}` for a dump whose hopper changed.
pub const WHY_HOPPER_CHANGED: &str = "its hopper no longer held what was to be dumped";
/// `ACT_FAILED`'s `{why}` for a dump the site's spoil would not take (the fixer round's item
/// 4: the spoil gone, or its pools refusing the transfer).
pub const WHY_SPOIL_GONE: &str =
    "the spoil would not take the load; it stays in the hopper and nothing moved";
/// An act in flight: `{n}`, `{doing}`, `{when}`. A run in flight (doc 20 cut SM.7) uses it
/// too, `{doing}` from `crate::surface::act::drilling_words` and [`RUN_STOP_REMEDY`] appended;
/// the lease return of a rig working a plan (doc 20 cut SM.11) with `{doing}` [`PLAN_DOING`],
/// no `done {when}` clause, and [`PLAN_REMEDY`] appended.
pub const RIG_BUSY: &str = "Rig {n} is {doing}: done {when}.";
/// **The lease return with a rig still logged out** (the fixer round's item 1): its
/// `tacsim.excursion` record holds an open excursion but no act. A rig can only be in
/// Occator's reach that way after a staff move (every act needs a site's pad), so the remedy
/// is staff's: `{n}`, `{site}`.
pub const RIG_LOGGED_OUT: &str =
    "Rig {n} is still logged out on {site} and staff must bring it in; nothing changes.";
/// The reserve: `{n}`, `{now}`, `{after}`, `{floor}`.
pub const RESERVE_LOW: &str =
    "Rig {n} has {now} % charge; that would leave {after} %, under the {floor} % reserve.";
/// The operator's cap: `{cap}`.
pub const CAP_OPERATOR: &str =
    "You have {cap} rigs out already; that is as many as one pilot may run.";
/// The site's cap: `{site}`, `{cap}`.
pub const CAP_SITE: &str = "{site} has {cap} rigs out already; no more go down until one comes in.";
/// The world's cap: `{cap}`.
pub const CAP_WORLD: &str =
    "The Registry has {cap} rigs out across the sites; no more go down until one comes in.";
/// `{where}`: in this hull's bay.
pub const WHERE_BAY: &str = "in the bay";
/// `{where}`: on this hull's site within the ramp's cell: `{ramp}`, `{site}`.
pub const WHERE_RAMP: &str = "on the ground at the {ramp} on {site}";
/// `{where}`: on this hull's site further off: `{site}`, `{m}` (cells × 10, as
/// [`RECOVER_FAR`]), `{ramp}`.
pub const WHERE_GROUND: &str = "on the ground on {site}, about {m} m from the {ramp}";
/// `{where}`: anywhere else (a staff move): `{name}`.
pub const WHERE_AWAY: &str = "not in the {name}'s bay";
/// The status's line under a rig with an act in flight: `{doing}`, `{when}`.
pub const ACT_LINE: &str = "  {doing}: done {when}.";

// ---- doc 20 cut SM.4a: knowing (§1 beats 4–6, §5, §10; the brief's 3.8) ------------------
//
// Placeholders beyond SM.3b's: `{field}` a field's name (`C-NW`), `{addr}` a pocket's address
// (`C117`), `{count}` the units read, `{footing}` a footing's words, `{hidden}` the hidden
// count's clause, `{launch}` the implied launch's clause, `{time}` the act's IC seconds (two
// legs when it launches), `{heard}` who hears it, `{quiet}` " quiet" or "", `{quietly}`
// " quietly" or "", `{m}` metres (cells × 10), `{dir}` an octant's short form, `{r}` the scan's
// reach in metres, `{better}` a sharper footing's name, `{down}` the launch's IC seconds,
// `{bearing}` an octant's word, `{band}` a range band, `{block}` a block letter, `{tenure}` its
// tenure word, `{reading}` a reading or the hint, `{terrain}` a terrain word, `{body}` a body's
// name, `{what}` a site's or a body's name, `{arg}` the forget's name as typed, `{fields}` and
// `{pockets}` counts. Richness is barren, lean, fair, rich: never P0–P3 or A–D.

/// The scan's preview: `{n}`, `{mark}`, `{launch}`, `{count}`, `{field}`, `{footing}`,
/// `{hidden}`, `{time}`, `{real}`, `{now}`, `{after}`.
pub const SCAN_PREVIEW: &str = "Rig {n} ({mark}) would {launch}scan the {count} fields around {field} {footing}{hidden}: {time}{real}; charge {now} → {after} %. +tv/{n} scan go starts it.";
/// The sounding's preview: `{n}`, `{mark}`, `{launch}`, `{count}`, `{addr}`, `{heard}`,
/// `{time}`, `{real}`, `{now}`, `{after}`, `{quiet}`.
pub const SOUND_PREVIEW: &str = "Rig {n} ({mark}) would {launch}sound the {count} pockets around {addr}{heard}: {time}{real}; charge {now} → {after} %. +tv/{n} sound{quiet} go starts it.";
/// `{launch}` when the act implies the launch: `{ramp}`, `{site}`.
pub const IMPLIED_LAUNCH: &str = "go down the {ramp} onto {site}, then ";
/// `{time}` when the act implies the launch: `{down}`, `{secs}`.
pub const TWO_LEGS: &str = "{down} IC s down, then {secs} IC s";
/// `{footing}` from rock.
pub const FOOTING_ROCK: &str = "from rock: fields read exactly";
/// `{footing}` from regolith with rock in reach: `{m}`, `{dir}` (doc 20 §5, verbatim).
pub const FOOTING_REGOLITH: &str = "from regolith: fields read as a band; rock {m} m {dir}";
/// `{footing}` from regolith with no rock in reach: `{r}`.
pub const FOOTING_REGOLITH_NONE: &str =
    "from regolith: fields read as a band; no rock within {r} m";
/// `{footing}` from ice with a sharper footing in reach: `{better}`, `{m}`, `{dir}`.
pub const FOOTING_ICE: &str = "from ice: no return; {better} {m} m {dir}";
/// `{footing}` from ice with nothing sharper in reach: `{r}`.
pub const FOOTING_ICE_NONE: &str = "from ice: no return; no regolith or rock within {r} m";
/// `{hidden}` when the mast does not see some fields: `{k}`.
pub const HIDDEN_FIELDS: &str = "; {k} hidden from the mast";
/// `{heard}` for a loud sounding: `{site}`.
pub const HEARD_ACROSS: &str = ", heard across {site}";
/// `{heard}` for a quiet sounding.
pub const HEARD_BY_NOBODY: &str = " quietly, heard by nobody";
/// The scan, filed: `{n}`, `{when}`.
pub const SCAN_FILED: &str = "Rig {n} starts its scan: done {when}.";
/// The scan, filed from the bay: `{n}`, `{ramp}`, `{when}`.
pub const SCAN_FILED_LAUNCH: &str = "Rig {n} starts down the {ramp} to scan: done {when}.";
/// The sounding, filed: `{n}`, `{when}`.
pub const SOUND_FILED: &str = "Rig {n} starts its sounding: done {when}.";
/// The sounding, filed from the bay: `{n}`, `{ramp}`, `{when}`.
pub const SOUND_FILED_LAUNCH: &str = "Rig {n} starts down the {ramp} to sound: done {when}.";
/// The scan, done (then one `"  {row}"` line per field, north-west first): `{n}`, `{count}`,
/// `{footing}`, `{hidden}`.
pub const SCANNED: &str = "Rig {n} scanned {count} fields from {footing}{hidden}:";
/// The scan from ice, done: `{n}`, `{count}`.
pub const SCANNED_ICE: &str = "Rig {n} scanned from ice: no return from {count} fields.";
/// The sounding, done (then `"  {family}: {list}"` per family with ore): `{n}`, `{count}`,
/// `{addr}`, `{quietly}`.
pub const SOUNDED: &str =
    "Rig {n} sounded {count} pockets around {addr}{quietly}; every pocket not named reads barren:";
/// The sounding, done, every pocket barren: `{n}`, `{count}`, `{addr}`, `{quietly}`.
pub const SOUNDED_BARREN: &str =
    "Rig {n} sounded {count} pockets around {addr}{quietly}: every one reads barren.";
/// **The heard sounding** (§5, Q-20-9): `{bearing}`, `{band}` and nothing more.
pub const HEARD: &str = "A sounding thumps through the ground from the {bearing}, {band}.";
/// The heard sounding on the listener's own cell.
pub const HEARD_HERE: &str = "A sounding thumps through the ground right under you.";
/// `{band}` under 50 cells.
pub const BAND_NEAR: &str = "under 500 m off";
/// `{band}` 50 to 100 cells.
pub const BAND_MID: &str = "500 m to 1 km off";
/// `{band}` over 100 cells.
pub const BAND_FAR: &str = "over 1 km off";
/// **A repeat from the same cell adding nothing** (doc 20 §5, verbatim).
pub const LEARN_NOTHING: &str = "You learn nothing new here.";
/// **No reading** — byte-identical for an unknown site, an uncovered unit and a barren pocket
/// never sounded (doc 20 §5, verbatim).
pub const NO_READING: &str = "You have no reading there.";
/// The status's hint where the rig stands on nothing read yet (doc 20 §1 beat 4).
pub const NO_READINGS_HERE: &str =
    "no readings here yet: +tv/scan reads the nine fields around you";
/// The status's line under a rig on the ground: `{block}`, `{tenure}`, `{field}`,
/// `{footing}`, `{reading}`.
pub const GROUND_LINE: &str =
    "  Block {block} ({tenure}), field {field}, footing {footing} — {reading}.";
/// The status's line while the site's ground is still being mapped: `{block}`, `{tenure}`,
/// `{field}`.
pub const GROUND_COLD_LINE: &str =
    "  Block {block} ({tenure}), field {field}; the ground here is still being mapped.";
/// The status's line where the instruments cannot read the ground: `{block}`, `{tenure}`,
/// `{field}`.
pub const GROUND_DARK_LINE: &str =
    "  Block {block} ({tenure}), field {field}; the instruments cannot read the ground here.";
/// A finished reading whose chart row is gone: `{n}`, `{act}`.
pub const READINGS_GONE: &str =
    "Rig {n} finished its {act}; its readings are no longer on your chart.";
/// A finished reading whose chart row was written but whose site's ground cannot be read at the
/// tell (the geology layer dark, the body's table missing, the site's spec unreadable): `{n}`,
/// `{act}`. The row stands; `+tv` reads it once the ground reads again (the SM.4a fixer round's
/// finding F2: never `READINGS_GONE`, which would say paid-for coverage was lost).
pub const READINGS_DARK: &str = "Rig {n} finished its {act}; its readings are on your chart, but the instruments cannot read the ground right now.";
/// A cold site: `{site}`.
pub const GROUND_COLD: &str = "{site}'s ground is still being mapped; try again in a moment.";
/// The geology layer dark for this site.
pub const INSTRUMENTS_DARK: &str =
    "The rigs' instruments cannot read the ground here right now; nothing changes.";
/// No footing under the rig: `{n}`, `{terrain}`.
pub const NO_FOOTING: &str =
    "Rig {n} stands on {terrain}; the mast reads from rock, regolith or ice.";
/// The body's chart full: `{body}`.
pub const CHART_FULL: &str = "Your chart of {body} is full; +tv/forget <site> frees room.";
/// `ACT_FAILED`'s `{why}` for a scan or a sounding whose rig was moved.
pub const WHY_MOVED: &str = "it was moved before it finished";
/// The forget's preview: `{what}`, `{fields}`, `{pockets}`, `{arg}`.
pub const FORGET_PREVIEW: &str = "You would strike {what} from your chart: {fields} fields scanned and {pockets} pockets sounded. It cannot be undone. +tv/forget {arg} go strikes it.";
/// The forget, done: `{what}`.
pub const FORGOTTEN: &str = "You strike {what} from your chart.";
/// Nothing to forget: `{what}`.
pub const FORGET_NOTHING: &str = "Your chart holds no readings of {what}.";
/// A chart row of the forget's target that will not read (the pre-commit round's item 8: never
/// `INSTRUMENTS_DARK`, which speaks of the ground under a rig): `{what}`.
pub const CHART_UNREAD: &str = "Your chart of {what} cannot be read right now; nothing changes.";
/// An unknown name: `{arg}`.
pub const FORGET_UNKNOWN: &str = "There is no mining site or body called {arg}.";
/// No name.
pub const FORGET_WHICH: &str = "Forget which? +tv/forget <site> or +tv/forget <body>.";
/// Quiet on.
pub const QUIET_ON: &str = "You will not hear other rigs' soundings.";
/// Quiet off.
pub const QUIET_OFF: &str = "You will hear other rigs' soundings again.";

// ---- doc 20 cut SM.4b: driving and the map (§1 beats 4, 7 and 9, §8, §10; the brief's 3.5,
// 3.6 and 3.10) ----------------------------------------------------------------------------------
//
// Placeholders beyond SM.4a's: `{target}` a drive's target in words (`C117`, `field C-NW`,
// `block C`, `cell 140,96`, `300 m NE`), `{arg}` the target as typed, `{home}` the charge after
// the trip home (whole percent), `{drive}`/`{climb}` a recover's two legs in IC seconds, `{need}`
// a leg's charge (whole percent, rounded up), `{family}` an ore family's label, `{word}` a word as
// typed.

/// The drive's preview: `{n}`, `{mark}`, `{launch}`, `{m}`, `{target}`, `{time}`, `{real}`,
/// `{now}`, `{after}` (after the drive), `{home}` (after the trip home), `{arg}`.
pub const DRIVE_PREVIEW: &str = "Rig {n} ({mark}) would {launch}drive {m} m to {target}: {time}{real}; charge {now} → {after} %, {home} % after the trip home. +tv/{n} drive {arg} go sends it.";
/// The drive, filed: `{n}`, `{target}`, `{when}`.
pub const DRIVE_FILED: &str = "Rig {n} drives for {target}: there {when}.";
/// The drive, filed from the bay: `{n}`, `{ramp}`, `{target}`, `{when}`.
pub const DRIVE_FILED_LAUNCH: &str =
    "Rig {n} starts down the {ramp} and drives for {target}: there {when}.";
/// **The arrival** (the completion line of a drive): `{n}`, `{addr}` the end cell's pocket
/// address, `{reading}` ` ({words})` where the operator's OWN chart has sounded that pocket, else
/// empty — never from the ground.
pub const ARRIVED: &str = "Rig {n} arrives at {addr}{reading}.";
/// A target that does not parse: `{n}`.
pub const DRIVE_USAGE: &str = "Drive where? +tv/{n} drive <pocket>|<field>|<block>|<x>,<y>|<bearing> <metres>, as C117, C-NW, C, 140,96 or ne 300.";
/// The rig stands on the target's cell already: `{n}`.
pub const DRIVE_HERE: &str = "Rig {n} is there already.";
/// The target is off the site: `{site}`.
pub const DRIVE_OFF_SITE: &str = "That point is off {site}.";
/// No way over the ground: `{n}`, `{target}`.
pub const NO_PATH: &str = "Rig {n} has no way over the ground to {target}.";
/// More than 32 waypoints even at 80 m: `{target}`, `{cap}`.
pub const TOO_WINDING: &str = "The way to {target} winds too much to drive in one leg ({cap} waypoints at most). Drive to a nearer point first.";
/// The end stand has no way back to the ramp (R2; the drive queues nothing): `{n}`, `{ramp}`,
/// `{target}`.
pub const NO_WAY_HOME: &str =
    "Rig {n} could not get back to the {ramp} from {target}, so it stays where it is.";
/// A drive's start with no order behind it (a forged start, or one a restart lost).
pub const DRIVE_UNPLANNED: &str = "That drive was not planned; preview it again.";
/// A drive's start whose rig no longer stands where its leg begins.
pub const DRIVE_MOVED: &str = "That rig has moved since the drive was planned; preview it again.";
/// The recover's preview from farther than beside the ramp: `{n}`, `{mark}`, `{m}`, `{ramp}`,
/// `{name}`, `{drive}`, `{climb}`, `{real}`, `{now}`, `{after}`, `{hopper}`, `{then}` (doc 20 cut
/// SM.8).
pub const RECOVER_DRIVE_PREVIEW: &str = "Rig {n} ({mark}) would drive {m} m to the {ramp} and climb into the {name}'s bay: {drive} IC s, then {climb} IC s{real}; charge {now} → {after} %; hopper {hopper}{then}. +tv/{n} recover go brings it in.";
/// The same with the rig's battery row unreadable: `{n}`, `{mark}`, `{m}`, `{ramp}`, `{name}`,
/// `{drive}`, `{climb}`, `{real}`, `{hopper}`, `{then}`.
pub const RECOVER_DRIVE_PREVIEW_UNREAD: &str = "Rig {n} ({mark}) would drive {m} m to the {ramp} and climb into the {name}'s bay: {drive} IC s, then {climb} IC s{real}; its charge cannot be read; hopper {hopper}{then}. +tv/{n} recover go brings it in.";
/// The recover with a leg, filed: `{n}`, `{ramp}`, `{when}`.
pub const RECOVER_DRIVE_FILED: &str = "Rig {n} drives to the {ramp}: in the bay {when}.";
/// The rig cannot afford its own leg home (lead ruling 5): `{n}`, `{now}`, `{need}`, `{ramp}`.
pub const RECOVER_SHORT: &str = "Rig {n} has {now} % charge and the drive to the {ramp} needs {need} %; it cannot come in on its own.";
/// The way home's terrain cannot be mapped this boot (R3): `{n}`, `{ramp}`.
pub const RECOVER_DARK: &str =
    "Rig {n}'s way back to the {ramp} cannot be mapped just now; staff can bring it in.";
/// `+tv/map` with the hull not settled on a site.
pub const MAP_NOT_SITE: &str =
    "Your Dray-P is not settled on a site; +tv/map shows a site's ground.";
/// `+tv/map` naming a rig neither on the site's ground nor in the bay: `{n}`, `{site}`.
pub const MAP_NO_RIG: &str = "Rig {n} is not on {site}'s ground or in the bay.";
/// `+tv/map` naming no rig, with none of the actor's rigs on the site's ground or in the bay
/// (the review's conformance/F2: never a slot the actor may not hold): `{site}`.
pub const MAP_NO_RIGS: &str = "None of your rigs is on {site}'s ground or in the bay.";
/// `+tv/map` naming no ore family: `{word}`.
pub const MAP_FAMILY_UNKNOWN: &str =
    "There is no ore family called {word}: metals, volatiles, carbon or strategic.";

// ---- doc 20 cut SM.7: mining (§1 beat 8, §6, §8, §10; the brief's 3.6) --------------------------
//
// Placeholders beyond SM.4b's: `{addr}` the pocket's address (`C117`), `{class}` lean, fair or
// rich (never P0–P3 or A–D), `{ore}` the family's ore good's id with `_` read as a space (`metal
// ore`, `ice ore`, `carbon ore`, `strategic ore`), `{cycles}` and `{k}` counts, `{yields}` the
// cycles' tonnes joined `/` (`2/2/2/1`), `{total}` their sum, `{secs}` the run's IC seconds,
// `{hop0}`/`{hop1}`/`{cap}` the hopper before and after the run and its limit in whole tonnes,
// `{room}` whole tonnes (the hold's unreserved mass after the run, or a blind core's room),
// `{home}` the charge after the trip home, `{also}` the other families' clause, `{arg}` the
// family and count as typed with a leading space (or empty), `{fam}` the family word as typed with
// a leading space (or empty), `{word}` a family's word, `{m}` a count named, `{fit}` the count
// that fits, `{why}` the bound's clause, `{ord}` an ordinal word, `{t}` tonnes, `{lift}` what a
// core lifted, `{reading}` the operator's own reading of the pocket, `{line}` a rights line,
// `{stop}` the stop clause, `{pit}` the public pit clause. No width bound on prose (the 120-column
// floor is for grids and tables; reviewer F1).

/// **The mining preview from the operator's own reading** (§6; §1 beat 8): `{n}`, `{mark}`,
/// `{addr}`, `{class}`, `{ore}`, `{cycles}`, `{yields}`, `{total}`, `{secs}`, `{real}`, `{hop0}`,
/// `{hop1}`, `{cap}`, `{room}`, `{now}`, `{after}`, `{home}`, `{also}`, `{arg}`.
pub const MINE_PREVIEW: &str = "Rig {n} ({mark}) would mine {addr} {class} {ore}: {cycles}, {yields} = {total} t, {secs} IC s{real}; hopper {hop0} → {hop1} of {cap} t; hold room after: {room} t; charge {now} → {after} % ({home} % after the trip home).{also} +tv/{n} mine{arg} go starts it.";
/// `{cycles}` for one cycle.
pub const ONE_CYCLE: &str = "1 cycle";
/// `{cycles}` for more: `{c}`.
pub const SOME_CYCLES: &str = "{c} cycles";
/// `{also}` per other family the operator's reading shows: `{class}`, `{ore}`, `{n}`, `{word}`.
pub const MINE_ALSO: &str = " Also {class} {ore} here: +tv/{n} mine {word}.";
/// **The blind preview** (§6, verbatim: "No reading here: the first cycle will tell." and "3 t of
/// room needed to core an unread pocket"): `{n}`, `{mark}`, `{addr}`, `{ore}`, `{secs}`, `{real}`,
/// `{room}`, `{now}`, `{after}`, `{home}`, `{pit}`, `{arg}`.
pub const MINE_PREVIEW_BLIND: &str = "No reading here: the first cycle will tell. Rig {n} ({mark}) would core {addr} for {ore}: 1 cycle, {secs} IC s{real}; {room} t of room needed to core an unread pocket; charge {now} → {after} % ({home} % after the trip home).{pit} +tv/{n} mine{arg} go starts it.";
/// `{pit}` over a worked pocket (a public fact, the same for every asker).
pub const PIT_WORKED: &str = " Every map shows a pit here: someone has mined this pocket.";
/// `{pit}` over a pocket every map shows worked out.
pub const PIT_WORKED_OUT: &str = " Every map shows this pit worked out.";
/// The run, filed: `{n}`, `{addr}`, `{when}` (the first cycle's due).
pub const MINE_FILED: &str = "Rig {n} starts drilling {addr}: the first cycle {when}.";
/// The blind core, filed: `{n}`, `{addr}`, `{when}`.
pub const CORE_FILED: &str = "Rig {n} starts coring {addr}: done {when}.";
/// **A cycle, done** (§1's line, verbatim): `{n}`, `{k}`, `{cycles}`, `{t}`, `{ore}`, `{hopper}`.
pub const CYCLE_DONE: &str = "Rig {n}, cycle {k} of {cycles}: {t} t {ore}; hopper {hopper}.";
/// The last cycle's clause when the run's cycles are done: `{addr}`.
pub const RUN_DONE: &str = " The run at {addr} is done.";
/// The last cycle's clause when the pocket is worked out: `{addr}`, `{ore}`.
pub const RUN_WORKED_OUT: &str = " {addr} is worked out for {ore}.";
/// The last cycle's clause after `+tv/mine stop`: `{n}`.
pub const RUN_STOPPED: &str = " Rig {n} stops drilling, as you asked.";
/// The last cycle's clause when a right stops the next: `{n}`, `{addr}`, `{line}`.
pub const RUN_RIGHTS: &str = " Rig {n} stops drilling {addr}: {line}";
/// **The last cycle's clause when the registry cannot be read** (the SM.7 lead's pre-commit round,
/// item 3; the lead's wording, for the owner to review): `RunEnd::Dark` — the tenure read at
/// landing failed, so the run ends (fail closed) without telling a claim that may not exist.
/// `{addr}`.
pub const RUN_DARK: &str =
    " The registry cannot be read here; the run at {addr} ends with this cycle.";
/// **A blind core, done**: `{n}`, `{reading}` (the operator's own new reading), `{lift}`,
/// `{hopper}`.
pub const CORED: &str = "Rig {n} cored {reading}: {lift}; hopper {hopper}.";
/// `{lift}` when the core lifted nothing.
pub const NOTHING_LIFTED: &str = "nothing to lift";
/// A cycle that did not happen: `{n}`, `{k}`, `{cycles}`, `{addr}`, `{why}`.
pub const CYCLE_FAILED: &str =
    "Rig {n}'s cycle {k} of {cycles} at {addr} did not happen: {why}; the run ends.";
/// `CYCLE_FAILED`'s `{why}` when the block would not give up its ore.
pub const WHY_UNWORKED: &str = "the ground would not give up its ore; nothing moved";
/// A rig in the bay: `{n}`.
pub const MINE_IN_BAY: &str =
    "Rig {n} is in the bay; it drills only standing on a pocket: +tv/{n} drive <pocket> first.";
/// A rig already drilling: `{n}`, `{addr}`, `{k}`, `{cycles}`, `{when}`.
pub const MINE_RUNNING: &str = "Rig {n} is drilling {addr}, cycle {k} of {cycles}: done {when}. +tv/{n} mine stop ends the run after this cycle.";
/// No footing for the drill: `{n}`, `{terrain}`.
pub const MINE_NO_FOOTING: &str =
    "Rig {n} stands on {terrain}; the drill bites rock, regolith or ice.";
/// No licence at all (every other standing prints `+licence`'s own line).
pub const MINE_NO_LICENCE: &str = "Mining needs a prospecting licence: +licence/buy takes the Freeport Registry's, from anywhere.";
/// A forfeit deposit: `{n}`, `{counter}`, `{port}`.
pub const MINE_FORFEIT: &str = "Rig {n}'s deposit is forfeit; it mines nothing until it goes back to the {counter} in {port}. +tv/{n} recover brings it home.";
/// An ended lease: `{n}`, `{counter}`, `{port}`.
pub const MINE_ENDED: &str = "Rig {n}'s lease from Occator has ended; it mines nothing until it goes back to the {counter} in {port}. +tv/{n} recover brings it home.";
/// **A block under claim** (doc 20 §5, verbatim).
pub const MINE_CLAIMED: &str = "That block is under claim.";
/// **Another rig on the pocket** (doc 20 §6, verbatim).
pub const MINE_LOCKED: &str = "Another rig is working that pocket.";
/// The hopper full: `{n}`, `{hopper}`.
pub const MINE_HOPPER_FULL: &str = "Rig {n}'s hopper has no room for another cycle: {hopper}.";
/// The hold's room used up: `{name}`.
pub const MINE_HOLD_FULL: &str = "The {name}'s hold has no unreserved room for another cycle.";
/// The pocket worked out for the family: `{addr}`, `{ore}`.
pub const MINE_WORKED_OUT: &str = "{addr} is worked out for {ore}.";
/// **A named count that does not fit** (doc 20 §6's line, with the slot): `{m}`, `{why}`, `{n}`,
/// `{fam}`, `{fit}`.
pub const MINE_NO_FIT: &str =
    "{m} cycles do not fit: {why} — +tv/{n} mine{fam} {fit}, or +tv/{n} mine{fam} with no number.";
/// `{why}` bound by the hopper: `{ord}`.
pub const FIT_HOPPER: &str = "the hopper is full after the {ord}";
/// `{why}` bound by the hold: `{ord}`.
pub const FIT_HOLD: &str = "the hold's room is used after the {ord}";
/// `{why}` bound by the pocket: `{ord}`.
pub const FIT_POCKET: &str = "the pocket is worked out after the {ord}";
/// `{why}` bound by the cap (unreachable under §11's ranges; a text, never a panic).
pub const FIT_CAP: &str = "a run is at most eight cycles";
/// **The only room line for an unread pocket** (lead ruling 2): `{room}`.
pub const MINE_BLIND_ROOM: &str =
    "{room} t of room needed to core an unread pocket; the hopper or the hold has less.";
/// A count named on an unread pocket: `{n}`, `{fam}`.
pub const MINE_BLIND_COUNT: &str =
    "No reading here, so the run is one cycle that reads the pocket: +tv/{n} mine{fam}.";
/// The family named reads barren: `{addr}`, `{ore}`.
pub const MINE_NO_ORE: &str = "Your reading of {addr} shows no {ore}.";
/// Every family reads barren: `{addr}`.
pub const MINE_ALL_BARREN: &str = "Your reading of {addr} is barren: nothing to mine here.";
/// The run changed between the preview and its start.
pub const MINE_CHANGED: &str = "That run changed since its preview; preview it again.";
/// The grammar: `{n}`.
pub const MINE_USAGE: &str = "Mine how? +tv/{n} mine [family] [cycles] [go], or +tv/{n} mine stop.";
/// `stop` with no run: `{n}`.
pub const MINE_NOT_RUNNING: &str = "Rig {n} is not drilling.";
/// `stop`, filed: `{n}`, `{addr}`, `{when}`.
pub const MINE_STOPPING: &str = "Rig {n} stops drilling {addr} after this cycle: done {when}.";
/// `stop` already asked: `{n}`.
pub const MINE_STOPPING_ALREADY: &str = "Rig {n} already stops after this cycle.";
/// **The status's line under a drilling rig**: `{addr}`, `{ore}`, `{k}`, `{cycles}`, `{when}`,
/// `{stop}`.
pub const RUN_LINE: &str =
    "  Drilling {addr} for {ore}, cycle {k} of {cycles}: this cycle done {when}{stop}.";
/// The status's line under a coring rig: `{addr}`, `{ore}`, `{when}`.
pub const CORE_LINE: &str = "  Coring {addr} for {ore}: done {when}.";
/// `{stop}` once `+tv/mine stop` was asked.
pub const RUN_STOP_CLAUSE: &str = "; it stops after this cycle";
/// **The remedy a drilling rig's busy line carries** (the SM.7 lead's pre-commit round, item 2):
/// the departure's `TV_BUSY` and the lease return's [`RIG_BUSY`] for a run in flight, in
/// [`MINE_RUNNING`]'s words: `{n}`.
pub const RUN_STOP_REMEDY: &str = " +tv/{n} mine stop ends the run after this cycle.";

// ---- doc 20 cut SM.8: unloading (§1 beat 9, §7.1, §8, §10; the brief's 3.5 and 3.6) ------------
//
// Placeholders beyond SM.7's: `{load}` and `{stays}` "{q} t of {label}" per good, joined as a
// list — `a`, `a and b`, `a, b and c` (`crate::surface::act::load_words`; goods v3's labels carry
// commas, so a plain ", " join was ambiguous — the lead's pre-commit round, item 5), `{room}` the hold's unreserved mass after the unload in
// whole tonnes (floored), `{hop0}`/`{hop1}`/`{cap}` the hopper before and after and its limit in
// whole tonnes (floored), `{q}` the tonnes named, `{fit}` the tonnes that fit (always ≥ 1),
// `{ore}` SM.7's ore words (`ore_words`: `metal ore`), `{arg}` the count as typed with a leading
// space (or empty), `{then}` a recover preview's unload clause (or empty).

/// **The recover's unload clause** (doc 20 §1 beat 9; lead ruling 1): `{then}` in every recover
/// preview when the hopper holds ore and the hold's room takes all of it: `{load}`, `{room}`.
pub const RECOVER_THEN: &str = "; then it unloads {load} into the hold, {room} t of room after";
/// `{then}` when the hold's room takes part of the hopper (lead ruling 3: "unloads what fits and
/// says what stays"): `{load}`, `{stays}`.
pub const RECOVER_THEN_PART: &str = "; then it unloads {load} into the hold and {stays} stays in the hopper: the hold has no room for it";
/// `{then}` when the hold's room takes none of it: `{stays}`, `{n}`.
pub const RECOVER_THEN_NONE: &str =
    "; the hold has no room for its {stays}: +tv/{n} dump empties it onto the spoil";
/// **The unload's preview** (§7.1 "previewed per good"): `{n}`, `{mark}`, `{load}`, `{name}`,
/// `{secs}`, `{real}`, `{hop0}`, `{hop1}`, `{cap}`, `{room}`, `{stays}` ([`UNLOAD_STAYS`] for a
/// whole unload that leaves some, else empty), `{arg}`.
pub const UNLOAD_PREVIEW: &str = "Rig {n} ({mark}) would unload {load} into the {name}'s hold: {secs} IC s{real}; hopper {hop0} → {hop1} of {cap} t; hold room after: {room} t.{stays} +tv/{n} unload{arg} go moves it.";
/// What an unload leaves in the hopper: `{stays}`.
pub const UNLOAD_STAYS: &str = " {stays} stays in the hopper: the hold has no room for it.";
/// The unload, filed: `{n}`, `{when}`.
pub const UNLOAD_FILED: &str = "Rig {n} starts unloading into the hold: done {when}.";
/// **The unload, done**: `{n}`, `{load}`, `{name}`, `{hopper}`; [`UNLOAD_STAYS`] follows when
/// something stayed.
pub const UNLOADED: &str = "Rig {n} unloads {load} into the {name}'s hold; hopper {hopper}.";
/// A queued unload that moved nothing (the hold had no room): `{n}`, `{stays}`, `{name}`.
pub const UNLOADED_NONE: &str = "Rig {n}'s hopper still holds {stays}: the {name}'s hold has no room for it. +tv/{n} dump empties it onto the spoil.";
/// **A queued unload that moved nothing because nothing was there** (the lead's pre-commit round,
/// item 3): the hopper read empty at the start's offer — staff emptied it after the recover — so
/// nothing stays either and [`UNLOADED_NONE`] would name nothing: `{n}`, `{name}`.
pub const UNLOADED_NONE_EMPTY: &str =
    "Rig {n} unloads nothing into the {name}'s hold: its hopper was already empty.";
/// **A recover that queued its unload, done** (lead ruling 1): `{n}`, `{name}`.
pub const RECOVERED_UNLOADING: &str =
    "Rig {n} is back in the {name}'s bay; it unloads its hopper into the hold next.";
/// Under way: `{name}`.
pub const UNLOAD_UNDER_WAY: &str = "The {name} is under way; her rigs unload only on a site's pad.";
/// Not on a site's pad (the brief's decision R5): `{name}`, `{n}`.
pub const UNLOAD_NOT_SITE: &str =
    "The {name} is not on a site's pad; rig {n} unloads into the hold only there.";
/// The rig is out on the site's ground (decision R9): `{n}`, `{site}`.
pub const UNLOAD_OUT: &str = "Rig {n} is out on {site}; it unloads from the bay: +tv/{n} recover brings it in and unloads it.";
/// The hold takes nothing of it (decision R18): `{name}`, `{n}`, `{load}`.
pub const UNLOAD_NO_ROOM: &str =
    "The {name}'s hold has no room for rig {n}'s {load}: +tv/{n} dump empties it onto the spoil.";
/// A named tonnage that does not fit, `{fit}` ≥ 1 (decisions R10, R18): `{q}`, `{n}`, `{fit}`,
/// `{ore}`.
pub const UNLOAD_NO_FIT: &str = "{q} t do not fit: rig {n} can unload {fit} t of {ore} now — +tv/{n} unload {fit}, or +tv/{n} unload with no number.";
/// **A named tonnage past the hopper's own stock**, `{fit}` ≥ 1 (the lead's pre-commit round,
/// item 4: the stock, not the hold's room, bound the fit, so [`UNLOAD_NO_FIT`]'s "do not fit"
/// would be wrong): `{n}`, `{fit}`, `{ore}`.
pub const UNLOAD_NO_STOCK: &str = "Rig {n}'s hopper holds only {fit} t of {ore}: +tv/{n} unload {fit}, or +tv/{n} unload with no number.";
/// The hopper or the hold's room changed between the preview and its start: `{n}`.
pub const UNLOAD_CHANGED: &str =
    "Rig {n}'s hopper or the hold's room changed since the preview; preview it again.";
/// **The queued unload's start refused while the hopper's unload log is full** (doc 20 cut SM.11,
/// the lead's pre-commit round, item 1): the surface tick's own start, never a typed one, so the
/// line is the start's wire refusal's only: `{n}`.
pub const UNLOAD_MARKERS: &str =
    "Rig {n}'s unload log is full; its hopper unloads by itself once the log frees.";
/// The grammar: `{n}`.
pub const UNLOAD_USAGE: &str = "Unload how? +tv/{n} unload [tonnes] [go].";
/// `ACT_FAILED`'s `{why}` for an unload whose rig left the bay.
pub const WHY_LEFT_BAY: &str = "it left the bay before it finished";
/// `ACT_FAILED`'s `{why}` for an unload the hold would not take (decision R8).
pub const WHY_HOLD_REFUSED: &str =
    "the hold would not take the load; it stays in the hopper and nothing moved";
/// `ACT_FAILED`'s `{why}` for an unload whose hopper's marker ring is full (the cut SM.8 fix
/// round's correctness/F2): the ring frees itself as its markers age, so the remedy is time.
pub const WHY_MARKERS_FULL: &str =
    "the rig's unload log is full; it stays in the hopper and nothing moved: try again later";
/// `ACT_FAILED`'s `{why}` for a **queued** unload whose hopper's marker ring is full (doc 20 cut
/// SM.11, the lead's pre-commit round, item 1): the finish keeps the queue, and the surface tick
/// starts it again once the ring frees — the remedy is time, and nobody has to type it.
pub const WHY_MARKERS_QUEUED: &str = "the rig's unload log is full; the load stays in the hopper and unloads by itself once the log frees";
/// The status's line under a rig with a queued unload.
pub const QUEUED_LINE: &str = "  Unloads its hopper into the hold next.";
/// The most tonnes `+tv/unload` takes as typed (`1..=99`).
pub const MAX_TYPED_TONNES: u64 = 99;

// ---- doc 20 cut SM.11: the fill plan's texts (the planner's wording for the owner; the two §5
// lines byte for byte) ----

/// `+tv/plan`'s grammar.
pub const PLAN_USAGE: &str = "Usage: +tv[/<n>]/plan <pocket|block>[,<pocket|block>…] [family=<good>] [go]  or  +tv[/<n>]/plan show  or  +tv[/<n>]/plan clear";
/// The preview's head: `{n}`, `{mark}`, `{site}`, `{ore}`, `{real}` (", about {span}" of 288 IC h).
pub const PLAN_PREVIEW_HEAD: &str =
    "Rig {n} ({mark}) would work this plan on {site}, mining {ore}, for at most 288 IC h{real}:";
/// One step of the preview: `{k}`, `{words}`, `{hours}` ([`PLAN_STEP_HOURS`] or empty), `{far}`
/// ([`PLAN_PREVIEW_FAR`] or empty).
pub const PLAN_PREVIEW_STEP: &str = "  {k}. {words}{hours}{far}";
/// A step's estimate: `{h}` whole IC hours.
pub const PLAN_STEP_HOURS: &str = ", about {h} IC h";
/// A block step's far pockets: `{k}`.
pub const PLAN_PREVIEW_FAR: &str = " ({k} of its pockets you read lie beyond what a full charge can work and bring home; the plan passes over them)";
/// The preview's tail: `{name}`, `{room}`, `{shared}` ([`PLAN_SHARED`] per other rig), `{hours}`,
/// `{n}`, `{list}`.
pub const PLAN_PREVIEW_TAIL: &str = "Between them it drives, recovers and unloads at every full hopper, and parks when no cycle fits the {name}'s hold ({room} t unreserved{shared}) or the list is worked out: at least {hours} IC h of work. +tv/{n} plan {list} go files it.";
/// Another rig's live plan draws on the hold: `{m}`.
pub const PLAN_SHARED: &str = "; rig {m}'s plan draws on it too";
/// Filed: `{n}`, `{no}`, `{when}`.
pub const PLAN_FILED: &str = "Rig {n}'s plan {no} is filed; it works until {when} at the latest. +tv/{n} plan show reads it; +tv/{n} plan clear ends it.";
/// A refusal at one address: `{addr}`, `{line}` ([`NO_READING`], [`MINE_CLAIMED`],
/// [`PLAN_TOO_FAR_LINE`] or [`PLAN_DARK_LINE`]).
pub const PLAN_REFUSED_AT: &str = "{addr}: {line}";
/// Too far: `{n}`.
pub const PLAN_TOO_FAR_LINE: &str = "a full charge cannot work it and bring rig {n} home.";
/// The registry dark.
pub const PLAN_DARK_LINE: &str = "the registry cannot be read here.";
/// A sounded-barren address: `{addr}`, `{family}`.
pub const PLAN_BARREN: &str = "{addr} reads barren in {family}; strike it from the list.";
/// Too many addresses: `{max}`.
pub const PLAN_TOO_MANY: &str = "A plan names at most {max} pockets and blocks.";
/// Named twice: `{addr}`.
pub const PLAN_TWICE: &str = "{addr} is named twice.";
/// Too many steps: `{max}`.
pub const PLAN_TOO_MANY_STEPS: &str = "That list makes more than {max} steps; name fewer blocks.";
/// A full hold: `{name}`.
pub const PLAN_HOLD_FULL: &str =
    "The {name}'s hold has no unreserved room for another cycle; a plan would only park.";
/// Nothing left to mine (doc 20 cut SM.12c-lane follow-up): `{family}`. Every pocket the walk could
/// take is worked out and no sounding is left to make, so a plan would end at once.
pub const PLAN_NOTHING_LEFT: &str = "That list has nothing left to mine in {family}: no pocket you read there is fair or better and unworked, and nothing is left to sound; a plan would end at once.";
/// Too long: `{hours}`.
pub const PLAN_TOO_LONG: &str = "That list needs at least {hours} IC h; a plan works at most 288.";
/// A plan is live already: `{n}`, `{no}`.
pub const PLAN_ALREADY: &str =
    "Rig {n} is already working plan {no}: +tv/{n} plan show reads it; +tv/{n} plan clear ends it.";
/// No plan: `{n}`.
pub const PLAN_NONE: &str = "Rig {n} has no plan.";
/// The plan changed since the preview: `{n}`.
pub const PLAN_CHANGED: &str = "Rig {n}'s plan changed since the preview; preview it again.";
/// A live plan holds its rig: `{n}`, `{no}`.
pub const PLAN_LIVE: &str =
    "Rig {n} is working plan {no}: +tv/{n} plan clear ends it after the step in flight.";
/// A clear landing on a plan that ended by itself: `{n}`, `{no}`.
pub const PLAN_ALREADY_ENDED: &str =
    "Rig {n}'s plan {no} has already ended: +tv/{n} plan clear strikes it.";
/// Cleared: `{n}`, `{no}`.
pub const PLAN_CLEARED: &str =
    "Rig {n}'s plan {no} is cleared: the step in flight finishes, and the rig is yours again.";
/// Struck: `{n}`, `{no}`.
pub const PLAN_DROPPED: &str = "Rig {n}'s finished plan {no} is struck from its record.";
/// `+tv/plan show`'s head: `{n}`, `{no}`, `{site}`, `{ore}`, `{state}`, `{filed}`, `{deadline}`.
pub const PLAN_SHOW_HEAD: &str = "Rig {n}'s plan {no} on {site}, mining {ore}: {state}; filed {filed}, works until {deadline} at the latest.";
/// One step: `{k}`, `{words}`, `{mark}`.
pub const PLAN_SHOW_STEP: &str = "  {k}. {words}: {mark}";
/// The tally: `{t}`, `{ore}`, `{c}`, `{trips}`.
pub const PLAN_SHOW_TALLY: &str = "So far: {t} t of {ore} in {c} over {trips}.";
/// What it does now: `{doing}`.
pub const PLAN_SHOW_NOW: &str = "Now: {doing}.";
/// When it ended: `{at}`, `{why}`.
pub const PLAN_SHOW_ENDED: &str = "It ended {at}: {why}.";
/// A live plan's state word.
pub const STATE_LIVE: &str = "working";
/// A parking plan's state word.
pub const STATE_PARKING: &str = "coming home to park";
/// An ended plan's state word.
pub const STATE_ENDED: &str = "ended";
/// A step done.
pub const MARK_DONE: &str = "done";
/// The step it works.
pub const MARK_NOW: &str = "now";
/// The step after.
pub const MARK_NEXT: &str = "next";
/// A later step.
pub const MARK_PENDING: &str = "pending";
/// A step passed over.
pub const MARK_PASSED: &str = "passed over";
/// The status's plan line: `{no}`, `{k}`, `{steps}`, `{words}`, `{when}`.
pub const PLAN_LINE: &str =
    "  Plan {no}: step {k} of {steps}, {words}; until {when} at the latest.";
/// The status's ended plan: `{no}`, `{why}`.
pub const PLAN_LINE_ENDED: &str = "  Plan {no} ended: {why}.";
/// A pass, told: `{n}`, `{addr}`, `{why}`.
pub const PLAN_PASSED: &str = "Rig {n}'s plan passes over {addr}: {why}.";
/// A parking for a right, told: `{n}`, `{why}` (a sentence).
pub const PLAN_PARKING: &str =
    "Rig {n}'s plan stops mining: {why} It comes home, unloads and parks.";
/// The end's summary: `{n}`, `{site}`, `{why}` (a sentence), `{t}`, `{ore}`, `{c}`, `{trips}`,
/// `{name}`, `{held}`, `{room}`.
pub const PLAN_ENDED: &str = "Rig {n}'s plan at {site} ended: {why} It mined {t} t of {ore} in {c} over {trips}; the {name}'s hold holds {held} t, {room} t of it unreserved.";
/// A wait, told once a boot to a connected operator: `{n}`, `{why}`.
pub const PLAN_WAITS: &str = "Rig {n}'s plan waits: {why}.";
/// The idle return: `{n}`, `{site}`, `{name}`, `{then}` ([`IDLE_THEN`] or empty).
pub const IDLE_HOME: &str =
    "Rig {n} stood a day idle on {site} and came home into the {name}'s bay{then}.";
/// The idle return's unload clause.
pub const IDLE_THEN: &str = "; it unloads its hopper next";
/// Parked where it stands: `{n}`, `{ramp}`, `{site}`.
pub const STRANDED: &str = "Rig {n} cannot reach the {ramp} on its charge; it is parked where it stands on {site}. Staff can bring it in.";
/// The remedy after `TV_BUSY` or `RIG_BUSY` for a rig working a plan: `{n}`.
pub const PLAN_REMEDY: &str = " +tv/{n} plan clear ends it after the step in flight.";
/// `{doing}` for a rig working a plan.
pub const PLAN_DOING: &str = "working its plan";

/// **`{c}` and `{trips}` in words** (doc 20 cut SM.11): `1 cycle`, `13 cycles`.
#[must_use]
pub fn count_words(n: u64, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}
/// An end: the list worked out.
pub const END_WORKED_OUT: &str = "its list is worked out.";
/// An end: the hold full — `{name}`.
pub const END_HOLD_FULL: &str = "the {name}'s hold has no room for another cycle.";
/// An end: out of time.
pub const END_BOUND: &str = "its 288 IC hours ran out.";
/// An end: the lease ended.
pub const END_LEASE_ENDED: &str = "the rig's lease has ended.";
/// An end: the deposit forfeit.
pub const END_LEASE_FORFEIT: &str = "the rig's deposit is forfeit.";
/// An end: the registry dark.
pub const END_DARK: &str = "the registry cannot be read here.";
/// An end: the chart full — `{body}`.
pub const END_CHART_FULL: &str = "your chart of {body} is full; +tv/forget makes room.";
/// An end: cleared.
pub const END_CLEARED: &str = "you cleared it.";
/// An end: stranded — `{ramp}`.
pub const END_STRANDED: &str = "it cannot reach the {ramp} on its charge.";
/// A wait: the helm — `{name}`.
pub const WAIT_HELM: &str = "you no longer hold the {name}'s helm";
/// A wait: ashore — `{name}`.
pub const WAIT_ASHORE: &str = "you are not aboard the {name}";
/// A wait: the cap.
pub const WAIT_CAP: &str = "you have as many rigs out as one pilot may run";
/// A wait: charging — `{when}`.
pub const WAIT_CHARGING: &str = "it charges in the bay until {when}";
/// A wait: the ground cold.
pub const WAIT_COLD: &str = "the ground is still being mapped";
/// A wait: dark.
pub const WAIT_DARK: &str = "the rig or the ground cannot be read right now";
/// A wait: the hopper's queued unload waits for its full marker ring to free (the SM.11 fixer
/// round's correctness/F1; the lead's pre-commit round, item 1: the queue stands and the tick
/// starts it again, so the plan resumes by itself).
pub const WAIT_MARKERS: &str =
    "the rig's unload log is full; its hopper unloads once the log frees, then the plan goes on";
/// `+tv/plan show`'s `Now:` for an act in flight.
pub const WAIT_BUSY: &str = "the step in flight finishes first";
/// A start refused because the plan's next step cannot start.
pub const WAIT_NEXT: &str = "its next step cannot start yet";
/// A pass: worked out.
pub const PASS_WORKED_OUT: &str = "it is worked out";
/// A pass: barren — `{family}`.
pub const PASS_BARREN: &str = "it reads barren in {family}";
/// A pass: locked.
pub const PASS_LOCKED: &str = "another rig is working it";
/// A pass: no way there.
pub const PASS_NO_WAY: &str = "there is no way there";
/// A pass: too far.
pub const PASS_TOO_FAR: &str = "a full charge cannot work it and come home";
/// A pass: nothing new.
pub const PASS_NOTHING_NEW: &str = "you learn nothing new there";

/// **`{ore}`**: the family's ore good's id with `_` read as a space (`metal ore`).
#[must_use]
pub fn ore_words(good: &str) -> String {
    good.replace('_', " ")
}

/// **The quiet preference's document prefix** (doc 20 cut SM.4a; D10): `tacsim:tvpref:{char}`.
/// Never a prefix of `tacsim:tv:`'s or `tacsim:tvlease:`'s keys, nor they of it: they differ at
/// the first character after `tacsim:tv` (`p` against `:` and `l`).
pub const TV_PREF_PREFIX: &str = "tacsim:tvpref:";
/// [`TvPrefs::schema`].
pub const TV_PREF_SCHEMA: u16 = 1;

/// **A character's `+tv` preferences** (doc 20 cut SM.4a; D10): whether they hear other rigs'
/// soundings. A small whole document on the normal lane, written by `+tv/quiet`; absent reads
/// as not quiet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TvPrefs {
    /// [`TV_PREF_SCHEMA`].
    pub schema: u16,
    /// The character: the key.
    pub character: ObjId,
    /// Not hearing other rigs' soundings.
    pub quiet: bool,
}

impl TvPrefs {
    /// **`tacsim:tvpref:{character.0}`**.
    #[must_use]
    pub fn key(&self) -> SmolStr {
        tv_pref_key(self.character)
    }
}

/// **`tacsim:tvpref:{character.0}`** — the numeric id, never `ObjId`'s `#`-prefixed Display.
#[must_use]
pub fn tv_pref_key(character: ObjId) -> SmolStr {
    SmolStr::new(format!("{TV_PREF_PREFIX}{}", character.0))
}

/// The inverse: the tail after [`TV_PREF_PREFIX`] as a strict decimal `u32`.
#[must_use]
pub fn tv_pref_key_owner(key: &str) -> Option<ObjId> {
    key.strip_prefix(TV_PREF_PREFIX)
        .and_then(crate::port::families::strict_decimal_u32)
        .map(ObjId)
}

/// **The slot switches** (doc 20 cut SM.3b; SM.3a's residual F10): `/1`…`/4`, one per
/// `MAX_BAY_ITEMS`.
pub const SLOT_SWITCHES: [&str; crate::definitions::logistics::MAX_BAY_ITEMS as usize] =
    ["1", "2", "3", "4"];

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
        // Doc 20 cut SM.12b: a house view ([`TvLease::house_view`]) is never stored.
        if self.lease == Self::HOUSE_LEASE {
            return Err("lease must be at least 1".to_owned());
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

    /// The lease word a house view carries. [`TvLease::check`] refuses it, so a house view can
    /// never be stored or mistaken for one. mush-economy numbers contracts from 0, so a stored
    /// rig lease on contract 0 would read unreadable; none can exist, because a rig is leased
    /// into a chartered hull's bay and that hull's charter contract is always opened first.
    pub const HOUSE_LEASE: u64 = 0;

    /// **The house's standing over its own rig, in the lease's shape** (doc 20 cut SM.12b; the
    /// lead's ruling 2: "the lease machinery sees Occator as the rigs' own lessor-operator — no
    /// lease"), so every operator gate SM.3b–SM.11 built (`RigAt.record`, `lease_gate`,
    /// `FillPlan::live_for`) reads it unchanged: the dispatcher its pilot, the yard hull its
    /// hull, its bay slot, `Active` for ever. No contract, fee, deposit or term; **never
    /// stored** — `check` refuses `lease == HOUSE_LEASE`.
    #[must_use]
    pub fn house_view(tv: ObjId, mark: &str, dispatcher: ObjId, hull: ObjId, slot: u8) -> TvLease {
        TvLease {
            schema: TV_LEASE_SCHEMA,
            lease: Self::HOUSE_LEASE,
            tv,
            mark: mark.to_owned(),
            pilot: dispatcher,
            hull,
            slot,
            hold: 0,
            fee_minor: 0,
            deposit_minor: 0,
            term_secs: 0,
            leased_at_ic: 0,
            term_ends_ic: i64::MAX,
            terms_paid: 0,
            status: TvLeaseStatus::Active,
            ended_at_ic: None,
            repossessed: false,
        }
    }

    /// **Whether this is a house view** ([`TvLease::house_view`]), never a stored lease.
    #[must_use]
    pub fn is_house(&self) -> bool {
        self.lease == Self::HOUSE_LEASE
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

/// **The licence check** (doc 20 §4.3, §9.1; SM.3a's seam, filled by cut SM.5, lead
/// ruling 6): a lease is refused unless [`crate::port::licence::licence_gate`] answers
/// `Ok` at the world's IC now — in force, or in its grace. Doc 20 §8's rule stands:
/// among the rig's acts only mining, staking and renewal read the licence; **the
/// lease is the one non-mining act that does**, and launch, recover and dump never
/// call this. It sits at check 6 of `plan_take`, before the slot, so a preview or a
/// `go` without a licence takes no rig; since doc 20b RK-1 the refusal still runs (a
/// `go`) or shows (a preview) the term settle first, so a lapsed pilot's past-term rig
/// is re-debited or forfeited at their next lease act like anyone's.
///
/// # Errors
///
/// [`LICENCE_BOOKS_DARK`](crate::port::licence::LICENCE_BOOKS_DARK) for an unreadable
/// record; [`LEASE_LICENCE`] for every other standing that does not authorize.
pub(crate) fn licence_check(w: &World, pilot: ObjId) -> Result<(), String> {
    match crate::port::licence::licence_gate(w, pilot, w.world_ic_now().at) {
        Ok(()) => Ok(()),
        Err(crate::port::licence::LicenceStanding::Unreadable) => {
            Err(crate::port::licence::LICENCE_BOOKS_DARK.to_owned())
        }
        Err(_) => Err(LEASE_LICENCE.to_owned()),
    }
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
    hopper_load_with(ctx.w, tv, pool_unit, &|endpoint, good| {
        crate::port_tick::read_pool(ctx, endpoint, good)
    })
}

/// **What a rig's hopper holds, through any pool reader** (doc 20 cut SM.4b: the map's feed reads
/// through its query's typed snapshot, the hold's unreserved room through either; one rule,
/// never a copy): `(kilograms, capacity kilograms)` of ore. `Err` when the rig has no hopper or a
/// pool will not read.
pub(crate) fn hopper_load_with(
    w: &World,
    tv: ObjId,
    pool_unit: Option<&CanonicalId>,
    read: &dyn Fn(EndpointRef, &CanonicalId) -> Result<u64, crate::port_tick::PoolReadError>,
) -> Result<(u64, u64), String> {
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
        let qty = read(EndpointRef(vault), good)
            .map_err(|error| format!("rig #{}'s hopper {good}: {error:?}", tv.0))?;
        kg = qty
            .checked_mul(per_unit)
            .and_then(|add| kg.checked_add(add))
            .ok_or_else(|| "a hopper's mass overflows".to_owned())?;
    }
    Ok((kg, capacity))
}

/// **What a rig's hopper holds, good by good, through any pool reader** (doc 20 cut SM.7: the
/// hold's unreserved room weighs each hopper per measure, mass and packed volume; the mining plan
/// reads the hopper's load per measure): each accepted good's quantity, zero quantities left out.
/// `Err` when the rig has no hopper or a pool will not read.
pub(crate) fn hopper_contents_with(
    w: &World,
    tv: ObjId,
    pool_unit: Option<&CanonicalId>,
    read: &dyn Fn(EndpointRef, &CanonicalId) -> Result<u64, crate::port_tick::PoolReadError>,
) -> Result<std::collections::BTreeMap<CanonicalId, u64>, String> {
    let vault = hopper_of(w, tv).ok_or_else(|| format!("rig #{} has no hopper vault", tv.0))?;
    let (accepted, _) = hopper_goods(w, tv, pool_unit)
        .ok_or_else(|| format!("rig #{}'s unit cannot be read", tv.0))?;
    let mut out = std::collections::BTreeMap::new();
    for good in accepted {
        let qty = read(EndpointRef(vault), &good)
            .map_err(|error| format!("rig #{}'s hopper {good}: {error:?}", tv.0))?;
        if qty > 0 {
            out.insert(good, qty);
        }
    }
    Ok(out)
}

/// **A rig's hopper compartments** (doc 20 cut SM.7): its unit's stowage (the live unit's, else
/// the pool's), as the capacity function reads it — the Mk1's one `hopper` of 12,000 kg and
/// 13,200 L.
pub(crate) fn hopper_compartments(
    w: &World,
    tv: ObjId,
    pool_unit: Option<&CanonicalId>,
) -> Option<Vec<crate::port::capacity::Compartment>> {
    let store = w.module_state::<TacsimStore>()?;
    let unit = crate::port::hull::live_unit_id(w, tv).or_else(|| pool_unit.cloned())?;
    let logistics = store.definitions().specs.get(&unit)?.logistics.as_ref()?;
    Some(
        logistics
            .stowage
            .iter()
            .map(|(id, def)| crate::port::capacity::Compartment::from_stowage(id, def))
            .collect(),
    )
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
pub(crate) fn hopper_text(kg: u64, capacity_kg: u64) -> String {
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

/// The live records on `hull` (any lessee), lease order — **the hull's one rig census**. **Doc 20
/// cut SM.12b**: for a yard hull (`crate::port::yard::yard_of_hull`), the stored records followed
/// by the yard's rigs as house views in slot order (`crate::port::yard::house_rigs`), so the
/// hold's room, a plan's compile and its `shared` slots, the mine act's bound, the unload's
/// siblings, the end summary, the site map's hold room, `+tv`'s slots and `bay_slot_word` see
/// both yard rigs at once.
pub(crate) fn live_on_hull(w: &World, hull: ObjId) -> Vec<TvLease> {
    let mut records = w
        .module_state::<TacsimStore>()
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
                .collect::<Vec<_>>()
        });
    if let Some(dispatcher) = crate::port::yard::yard_of_hull(w, hull)
        .and_then(|yard| yard.row.dispatcher)
        .map(ObjId)
    {
        records.extend(crate::port::yard::house_rigs(w, dispatcher));
    }
    records
}

/// Whether `record`'s economy lease is live: Occator's live lease on its TV is
/// the record's own contract (reviewer F15).
fn economy_lease_live(w: &World, lessor: &PrincipalId, record: &TvLease) -> bool {
    occator_lease_on(w, lessor, record.tv).is_some_and(|c| c.id.0 == record.lease)
}

/// The programme port's rig pool's unit (the unit a rig without a live unit record reads
/// as). Doc 20 cut SM.4b: the drive's rates read it too (`crate::surface::act::rig_rates`).
pub(crate) fn pool_unit(w: &World) -> Option<CanonicalId> {
    programme_port(w).and_then(|port| port.def.vehicles.as_deref().map(|pool| pool.unit.clone()))
}

/// **A rig's hopper's accepted goods and mass limit** (doc 20 cut SM.3b): `hopper_goods`
/// with the programme port's pool unit — what the dump freezes over and the bay gate reads.
#[must_use]
pub(crate) fn hopper_goods_of(w: &World, tv: ObjId) -> Option<(Vec<CanonicalId>, u64)> {
    hopper_goods(w, tv, pool_unit(w).as_ref())
}

/// **What a rig's hopper holds**, `(kilograms, capacity kilograms)` (doc 20 cut SM.3b):
/// `hopper_load` with the programme port's pool unit.
///
/// # Errors
///
/// The rig has no hopper, or a pool will not read.
pub(crate) fn hopper_load_of(ctx: &Ctx<'_>, tv: ObjId) -> Result<(u64, u64), String> {
    let unit = pool_unit(ctx.w);
    hopper_load(ctx, tv, unit.as_ref())
}

/// **Occator's counter and its port, by name** (doc 20 cut SM.3b's `LAUNCH_FORFEIT` and
/// `LAUNCH_ENDED`): the lessor's counter room's name and the port's, else the authored
/// words.
#[must_use]
pub(crate) fn counter_words(w: &World) -> (String, String) {
    programme_port(w)
        .and_then(|port| lessor_of(w, &port))
        .map_or_else(
            || ("counter".to_owned(), "Ceres Freeport".to_owned()),
            |lessor| (lessor.counter_name, lessor.port),
        )
}

/// **Whether a live record's economy lease is live** (doc 20 cut SM.3b; SM.3a's operate
/// gate R7): Occator's live lease on its TV is the record's own contract. **Doc 20 cut SM.12b**:
/// a house view ([`TvLease::is_house`]) is the house's own rig — always live.
#[must_use]
pub(crate) fn rig_lease_live(w: &World, record: &TvLease) -> bool {
    if record.is_house() {
        return true;
    }
    programme_port(w)
        .and_then(|port| lessor_of(w, &port))
        .is_some_and(|lessor| economy_lease_live(w, &lessor.principal, record))
}

/// **Real seconds as a short span** (doc 20 cut SM.3b's `{span}`): `{s} s` under a minute,
/// else [`crate::port::voyage::real_span`].
#[must_use]
pub(crate) fn span_words(real_secs: i64) -> String {
    if real_secs < 60 {
        format!("{} s", real_secs.max(0))
    } else {
        crate::port::voyage::real_span(real_secs)
    }
}

/// **An act's due instant in words** (doc 20 cut SM.3b's `{when}`): `"{HH:MM} UTC (IC
/// {civil}; in {span})"`, or `"IC {civil}; the IC clock is stopped"` at world rate 0.
#[must_use]
pub(crate) fn due_words(w: &World, due_ic: i64) -> String {
    let civil = mush_core::CivilDate::from_ic_seconds(due_ic);
    match crate::port::voyage::real_until(w, due_ic) {
        Some(real) => format!(
            "{} UTC (IC {civil}; in {})",
            crate::port::voyage::utc_hhmm(w.now().saturating_add(real)),
            span_words(real)
        ),
        None => format!("IC {civil}; the IC clock is stopped"),
    }
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
#[derive(Clone, Debug, PartialEq, Eq)]
enum Act {
    /// The status.
    Status,
    /// A take, committed or previewed.
    Take { go: bool },
    /// A return, committed or previewed.
    Return { go: bool },
    /// **Doc 20 cut SM.3b**: a launch, a recover or a dump — and since SM.4a a scan or a
    /// sounding (`quiet` the sounding's word after `sound`) — committed or previewed.
    Work {
        /// Which act.
        kind: crate::surface::excursion::ActKind,
        /// A quiet sounding.
        quiet: bool,
        /// Committed.
        go: bool,
        /// **Doc 17h cut M8e E4**: a launch's `mind=<word>` and `org=<org>`, as typed (case
        /// kept): the mind sealed aboard a cored rig.
        mind: Option<(String, Option<String>)>,
    },
    /// **Doc 20 cut SM.4a**: `+tv/forget <site>|<body> [go]` — the name as typed (case kept,
    /// whitespace collapsed; the brief's reviewer finding F10).
    Forget {
        /// The name.
        name: String,
        /// Committed.
        go: bool,
    },
    /// **Doc 20 cut SM.4a**: `+tv/quiet [on|off]`; `None` toggles.
    Quiet {
        /// On, off, or toggle.
        set: Option<bool>,
    },
    /// **Doc 20 cut SM.4b**: `+tv[/<n>]/drive <target…> [go]` — the words after the act as typed
    /// (whitespace collapsed, case kept, `go` removed).
    Drive {
        /// The target's words.
        words: String,
        /// Committed.
        go: bool,
    },
    /// **Doc 20 cut SM.4b**: `+tv[/<n>]/map [site] [family]` — a pure read.
    Map {
        /// The whole site rather than the drive view.
        site: bool,
        /// The family word, as typed (lowercased).
        family: Option<String>,
    },
    /// **Doc 20 cut SM.7**: `+tv[/<n>]/mine [family] [cycles] [go]` (alias `drill`).
    Mine {
        /// The family word, as typed (lowercased).
        family: Option<String>,
        /// The count named, `1..=99`.
        cycles: Option<u8>,
        /// Committed.
        go: bool,
    },
    /// **Doc 20 cut SM.7**: `+tv[/<n>]/mine stop` — committed at once (the brief's decision R7).
    MineStop,
    /// **Doc 20 cut SM.8**: `+tv[/<n>]/unload [<t>] [go]` — the whole hopper, or `<t>` tonnes of
    /// the first good (`1..=`[`MAX_TYPED_TONNES`]).
    Unload {
        /// The tonnes named.
        qty: Option<u64>,
        /// Committed.
        go: bool,
    },
    /// **Doc 20 cut SM.11**: `+tv[/<n>]/plan <list> [family=<good>] [go]` — the list as typed
    /// (case kept, `go` and `family=` removed).
    Plan {
        /// The pockets and blocks.
        list: String,
        /// `family=`'s word, as typed.
        family: Option<String>,
        /// Committed.
        go: bool,
    },
    /// **Doc 20 cut SM.11**: `+tv[/<n>]/plan show` or `+tv[/<n>]/plan/show` — a pure read.
    PlanShow,
    /// **Doc 20 cut SM.11**: `+tv[/<n>]/plan clear` — committed with no `go` (D6).
    PlanClear,
}

/// The act words a `+tv` switch or first word may name (the lease's, the acts', and since doc
/// 20 cut SM.4a `scan` with its alias `survey`, `sound`, `forget` and `quiet`; since SM.4b
/// `drive` and `map`; since SM.7 `mine` with its alias `drill`; since SM.8 `unload`; since SM.11
/// `plan`).
const ACT_WORDS: [&str; 15] = [
    "lease", "launch", "recover", "dump", "scan", "survey", "sound", "forget", "quiet", "drive",
    "map", "mine", "drill", "unload", "plan",
];

/// **`/show`** (doc 20 cut SM.11; the brief's D11): a switch accepted only beside `plan`, so the
/// TUI's run allowlist can name `+tv/plan/show` — a first word that admits nothing but the read.
pub const SHOW_SWITCH: &str = "show";

/// **The most cycles `+tv/mine` takes as typed** (the brief's 3.6: `1..=99`; the plan refuses a
/// count that does not fit and names the count that does).
const MAX_TYPED_CYCLES: u8 = 99;

/// **The grammar** (doc 20 §10, cut SM.3b, replacing SM.3a's): `+tv[/<n>]` (status),
/// `+tv[/<n>][/<act>] [<act>] [go]` with `<act>` one of `launch`, `recover`, `dump` either
/// as a switch or as the first word (SM.3a's `lease` rule), and `+tv[/<n>]/lease
/// [take|return] [go]` unchanged. Two slot switches, two act words, an unknown word, or a
/// word after `go` is [`TV_USAGE`]. The slot switches are [`SLOT_SWITCHES`]; `/5` and above
/// never reach here (the dispatcher refuses the undeclared switch).
///
/// **Doc 20 cut SM.4a** adds `scan` (alias `survey`) and `sound [quiet]` as acts, `forget
/// <site>|<body> [go]` (the name read from the raw text; empty → [`FORGET_WHICH`]) and `quiet
/// [on|off]`; a slot switch with `forget` or `quiet` is [`TV_USAGE`].
fn parse(ctx: &Ctx<'_>) -> Result<(Option<u8>, Act), CmdError> {
    parse_parts(&|word| ctx.switches.has(word), ctx.args.raw())
}

/// [`parse`] over its two inputs: whether a switch was given, and the raw argument text. Pure.
fn parse_parts(has: &dyn Fn(&str) -> bool, raw: &str) -> Result<(Option<u8>, Act), CmdError> {
    let usage = || CmdError::msg(TV_USAGE);
    let mut slot: Option<u8> = None;
    for (index, word) in SLOT_SWITCHES.iter().enumerate() {
        if has(word) {
            if slot.is_some() {
                return Err(usage());
            }
            slot = u8::try_from(index.saturating_add(1)).ok();
        }
    }
    let switched: Vec<&str> = ACT_WORDS.iter().copied().filter(|word| has(word)).collect();
    let mut act: Option<String> = match switched.as_slice() {
        [] => None,
        [one] => Some((*one).to_owned()),
        _ => return Err(usage()),
    };
    let mut words = raw.split_whitespace().map(str::to_ascii_lowercase);
    let mut next = words.next();
    let mut first_word = false;
    if act.is_none()
        && let Some(word) = next.as_deref()
        && ACT_WORDS.contains(&word)
    {
        act = Some(word.to_owned());
        first_word = true;
        next = words.next();
    }
    let Some(act) = act else {
        return if next.is_none() && !has(SHOW_SWITCH) {
            Ok((slot, Act::Status))
        } else {
            Err(usage())
        };
    };
    // Doc 20 cut SM.11 (D11): `/show` only beside `plan`.
    if has(SHOW_SWITCH) && act != "plan" {
        return Err(usage());
    }
    match act.as_str() {
        // Doc 20 cut SM.11 (the brief's 3.5): `plan <list> [family=<good>] [go]`, `plan show`
        // (or the `/show` switch), `plan clear`.
        "plan" => {
            let plan_usage = || CmdError::msg(PLAN_USAGE);
            if has(SHOW_SWITCH) {
                return if next.is_none() {
                    Ok((slot, Act::PlanShow))
                } else {
                    Err(plan_usage())
                };
            }
            match next.as_deref() {
                None => return Err(plan_usage()),
                Some("show") => {
                    return if words.next().is_none() {
                        Ok((slot, Act::PlanShow))
                    } else {
                        Err(plan_usage())
                    };
                }
                Some("clear") => {
                    return if words.next().is_none() {
                        Ok((slot, Act::PlanClear))
                    } else {
                        Err(plan_usage())
                    };
                }
                Some(_) => {}
            }
            let (typed, go) = act_words(raw, "plan", first_word);
            let mut family: Option<String> = None;
            let mut list: Vec<&str> = Vec::new();
            for token in typed.split_whitespace() {
                let lower = token.to_ascii_lowercase();
                if let Some(value) = lower.strip_prefix("family=") {
                    if family.is_some() || value.is_empty() {
                        return Err(plan_usage());
                    }
                    family = token.get("family=".len()..).map(str::to_owned);
                } else if lower == "go" {
                    return Err(plan_usage());
                } else {
                    list.push(token);
                }
            }
            if list.is_empty() {
                return Err(plan_usage());
            }
            return Ok((
                slot,
                Act::Plan {
                    list: list.join(" "),
                    family,
                    go,
                },
            ));
        }
        // Doc 20 cut SM.4a: the forget names no rig; its name is the raw text (F10).
        "forget" => {
            if slot.is_some() {
                return Err(usage());
            }
            let (name, go) = forget_name(raw, first_word);
            if name.is_empty() {
                return Err(CmdError::msg(FORGET_WHICH));
            }
            return Ok((None, Act::Forget { name, go }));
        }
        "quiet" => {
            if slot.is_some() {
                return Err(usage());
            }
            let set = match next.as_deref() {
                None => None,
                Some("on") => Some(true),
                Some("off") => Some(false),
                Some(_) => return Err(usage()),
            };
            if next.is_some() && words.next().is_some() {
                return Err(usage());
            }
            return Ok((None, Act::Quiet { set }));
        }
        // Doc 20 cut SM.4b: the drive's target is the raw text (case kept for the preview's
        // `{arg}`); an empty target reaches the command, which answers `DRIVE_USAGE`.
        "drive" => {
            let (words, go) = act_words(raw, "drive", first_word);
            return Ok((slot, Act::Drive { words, go }));
        }
        "map" => {
            let mut site = false;
            if next.as_deref() == Some("site") {
                site = true;
                next = words.next();
            }
            let family = next.take();
            if family.is_some() && words.next().is_some() {
                return Err(usage());
            }
            if family.as_deref() == Some("go") {
                return Err(usage());
            }
            return Ok((slot, Act::Map { site, family }));
        }
        // Doc 20 cut SM.7 (the brief's 3.6): `mine [family] [cycles] [go]` — at most one family
        // word and one count (`1..=99`), either order, then `go` — or `mine stop`.
        "mine" | "drill" => {
            let mine_usage = || {
                CmdError::msg(MINE_USAGE.replace(
                    "{n}",
                    &slot.map_or_else(|| "<n>".to_owned(), |n| n.to_string()),
                ))
            };
            if next.as_deref() == Some("stop") {
                if words.next().is_some() {
                    return Err(mine_usage());
                }
                return Ok((slot, Act::MineStop));
            }
            let (mut family, mut cycles, mut go) = (None, None, false);
            while let Some(word) = next.take() {
                if go {
                    return Err(mine_usage());
                }
                if word == "go" {
                    go = true;
                } else if !word.is_empty() && word.bytes().all(|b| b.is_ascii_digit()) {
                    let count = word
                        .parse::<u8>()
                        .ok()
                        .filter(|count| (1..=MAX_TYPED_CYCLES).contains(count));
                    match (count, cycles) {
                        (Some(count), None) => cycles = Some(count),
                        _ => return Err(mine_usage()),
                    }
                } else if family.is_none() {
                    family = Some(word);
                } else {
                    return Err(mine_usage());
                }
                next = words.next();
            }
            return Ok((slot, Act::Mine { family, cycles, go }));
        }
        // Doc 20 cut SM.8 (the brief's 3.5): `unload [<t>] [go]` — at most one whole tonnage
        // (`1..=99`, decimal digits only), then `go`; anything else is `UNLOAD_USAGE`.
        "unload" => {
            let unload_usage = || {
                CmdError::msg(UNLOAD_USAGE.replace(
                    "{n}",
                    &slot.map_or_else(|| "<n>".to_owned(), |n| n.to_string()),
                ))
            };
            let (mut qty, mut go) = (None, false);
            while let Some(word) = next.take() {
                if go {
                    return Err(unload_usage());
                }
                if word == "go" {
                    go = true;
                } else if qty.is_none()
                    && !word.is_empty()
                    && word.bytes().all(|b| b.is_ascii_digit())
                {
                    qty = Some(
                        word.parse::<u64>()
                            .ok()
                            .filter(|t| (1..=MAX_TYPED_TONNES).contains(t))
                            .ok_or_else(unload_usage)?,
                    );
                } else {
                    return Err(unload_usage());
                }
                next = words.next();
            }
            return Ok((slot, Act::Unload { qty, go }));
        }
        _ => {}
    }
    let mut back = false;
    let mut quiet = false;
    if act == "lease" {
        match next.as_deref() {
            Some("take") => next = words.next(),
            Some("return") => {
                back = true;
                next = words.next();
            }
            _ => {}
        }
    }
    if act == "sound" && next.as_deref() == Some("quiet") {
        quiet = true;
        next = words.next();
    }
    // Doc 17h cut M8e E4: `launch [mind=<word>] [org=<org>] [go]`, each once, the values as
    // typed (a draft's name keeps its case).
    let mut mind: Option<String> = None;
    let mut org: Option<String> = None;
    if act == "launch" {
        let typed: Vec<&str> = raw.split_whitespace().collect();
        while let Some(word) = next.as_deref() {
            let slot = if word.starts_with("mind=") {
                &mut mind
            } else if word.starts_with("org=") {
                &mut org
            } else {
                break;
            };
            let value = typed
                .iter()
                .find(|token| token.to_ascii_lowercase() == word)
                .and_then(|token| token.split_once('='))
                .map(|(_, value)| value.to_owned())
                .filter(|value| !value.is_empty());
            if slot.is_some() || value.is_none() {
                return Err(usage());
            }
            *slot = value;
            next = words.next();
        }
        if org.is_some() && mind.is_none() {
            return Err(usage());
        }
    }
    let go = match next.as_deref() {
        None => false,
        Some("go") => true,
        Some(_) => return Err(usage()),
    };
    if words.next().is_some() {
        return Err(usage());
    }
    let word = if act == "survey" {
        "scan"
    } else {
        act.as_str()
    };
    let parsed = match word {
        "lease" if back => Act::Return { go },
        "lease" => Act::Take { go },
        word => match crate::surface::excursion::ActKind::from_word(word) {
            Some(kind) => Act::Work {
                kind,
                quiet,
                go,
                mind: mind.map(|mind| (mind, org)),
            },
            None => return Err(usage()),
        },
    };
    Ok((slot, parsed))
}

/// **The forget's name, as typed** (doc 20 cut SM.4a; the brief's reviewer finding F10): the
/// raw text trimmed, a leading `forget` word stripped when the act was typed as a first word,
/// a final `go` (any case) stripped, internal whitespace collapsed to single spaces — case
/// kept. Returns the name and whether `go` was given.
fn forget_name(raw: &str, first_word: bool) -> (String, bool) {
    act_words(raw, "forget", first_word)
}

/// **An act's words, as typed** (doc 20 cut SM.4a's forget rule, shared since SM.4b with the
/// drive's target): the raw text, a leading `act` word stripped when the act was typed as a
/// first word, a final `go` (any case) stripped, whitespace collapsed — case kept.
fn act_words(raw: &str, act: &str, first_word: bool) -> (String, bool) {
    let mut words: Vec<&str> = raw.split_whitespace().collect();
    if first_word && words.first().is_some_and(|w| w.eq_ignore_ascii_case(act)) {
        words.remove(0);
    }
    let go = words.last().is_some_and(|w| w.eq_ignore_ascii_case("go"));
    if go {
        words.pop();
    }
    (words.join(" "), go)
}

/// **The last rig each operator named** (doc 20 cut SM.3b; lead ruling 6): a RAM slot,
/// **a convenience, never a fact** — bounded at `MAX_POINTERS` operators (the smallest key
/// dropped past it). A restart forgets it, so a two-rig act without a number then refuses
/// `WHICH_RIG_ACT` rather than guessing (decision 12).
#[derive(Clone, Debug, Default)]
pub(crate) struct RigPointer(std::collections::BTreeMap<ObjId, u8>);

/// The most operators `RigPointer` remembers.
pub const MAX_POINTERS: usize = 4_096;

/// Remember that `actor` named slot `n`.
fn point(ctx: &mut Ctx<'_>, actor: ObjId, n: u8) {
    if let Some(pointer) = ctx.w.module_state_mut::<RigPointer>() {
        pointer.0.insert(actor, n);
        while pointer.0.len() > MAX_POINTERS {
            if pointer.0.pop_first().is_none() {
                break;
            }
        }
    }
}

/// The slot `actor` named last, if any.
fn pointed(w: &World, actor: ObjId) -> Option<u8> {
    w.module_state::<RigPointer>()
        .and_then(|pointer| pointer.0.get(&actor).copied())
}

/// `{slots}`: `1 or 2`, `1, 2 or 3`, from the slots the actor's rigs hold.
fn slots_words(slots: &[u8]) -> String {
    let words: Vec<String> = slots.iter().map(u8::to_string).collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
        Some((last, _)) => last.clone(),
        None => String::new(),
    }
}

/// **`+tv`, `+tv/<n>`, `+tv/lease`, `+tv/<n> launch|recover|dump|scan|sound [go]`,
/// `+tv/forget`, `+tv/quiet`** (doc 20 §4.3, §5, §10; Q-20-6, Q-20-8; cuts SM.3b and SM.4a).
/// Any command that names a slot records it for the actor (`RigPointer`, the status included).
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn tv_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    if ctx.switches.has("bay") {
        return super::boarding::bay_access(ctx).map_err(CmdError::msg);
    }
    if ctx.switches.has("board")
        || ctx.switches.has("leave")
        || matches!(ctx.args.raw().trim(), "board" | "leave")
    {
        return super::boarding::player(ctx).map_err(CmdError::msg);
    }
    if ctx.switches.has("permissions") {
        return super::equipment::command(ctx).map_err(CmdError::msg);
    }
    if programme_port(ctx.w).is_none() {
        return Err(CmdError::msg(NOT_OPEN_HERE));
    }
    let (slot, act) = parse(ctx)?;
    if let Some(n) = slot {
        let actor = ctx.actor;
        point(ctx, actor, n);
    }
    match act {
        Act::Status => status(ctx, slot),
        Act::Take { go } => take(ctx, slot, go),
        Act::Return { go } => give_back(ctx, slot, go),
        Act::Work {
            kind,
            quiet,
            go,
            mind,
        } => {
            let rig = select(ctx, slot, kind).map_err(CmdError::msg)?;
            crate::surface::act::act_cmd(ctx, &rig, kind, quiet, go, mind.as_ref())
        }
        Act::Forget { name, go } => forget(ctx, &name, go),
        Act::Quiet { set } => quiet(ctx, set),
        // Doc 20 cut SM.4b (§3.5, §3.10 of the brief).
        Act::Drive { words, go } => {
            let rig = select(ctx, slot, crate::surface::excursion::ActKind::Drive)
                .map_err(CmdError::msg)?;
            crate::surface::act::drive_cmd(ctx, &rig, &words, go)
        }
        Act::Map { site, family } => {
            crate::port::site_map::map_cmd(ctx, slot, site, family.as_deref())
        }
        // Doc 20 cut SM.7 (§6, §10; the brief's 3.4 and 3.6).
        Act::Mine { family, cycles, go } => {
            let rig = select(ctx, slot, crate::surface::excursion::ActKind::Mine)
                .map_err(CmdError::msg)?;
            let named = match family.as_deref() {
                None => None,
                Some(word) => Some(
                    crate::surface::geology::family_of_word(word)
                        .ok_or_else(|| CmdError::msg(MAP_FAMILY_UNKNOWN.replace("{word}", word)))?,
                ),
            };
            let words: Vec<String> = family
                .iter()
                .cloned()
                .chain(cycles.map(|count| count.to_string()))
                .collect();
            let choice = crate::surface::mine::MineChoice {
                family: named,
                word: family,
                cycles,
            };
            crate::surface::act::mine_cmd(ctx, &rig, &choice, &words.join(" "), go)
        }
        Act::MineStop => {
            let rig = select(ctx, slot, crate::surface::excursion::ActKind::Mine)
                .map_err(CmdError::msg)?;
            crate::surface::act::mine_stop_cmd(ctx, &rig)
        }
        // Doc 20 cut SM.8 (§7.1, §10; the brief's 3.5).
        Act::Unload { qty, go } => {
            let rig = select(ctx, slot, crate::surface::excursion::ActKind::Unload)
                .map_err(CmdError::msg)?;
            crate::surface::act::unload_cmd(ctx, &rig, qty, go)
        }
        // Doc 20 cut SM.11 (§8 "Leaving it running", §10; the brief's 3.5).
        Act::Plan { list, family, go } => {
            let rig = select_word(ctx, slot, "plan").map_err(CmdError::msg)?;
            let named = match family.as_deref() {
                None => None,
                Some(word) => Some(
                    crate::surface::geology::family_of_word(word)
                        .ok_or_else(|| CmdError::msg(MAP_FAMILY_UNKNOWN.replace("{word}", word)))?,
                ),
            };
            crate::surface::act::plan_cmd(ctx, &rig, &list, named, family.as_deref(), go)
        }
        Act::PlanShow => {
            let rig = select_plan_word(ctx, slot, "plan show").map_err(CmdError::msg)?;
            crate::surface::act::plan_show_cmd(ctx, &rig)
        }
        Act::PlanClear => {
            let rig = select_plan_word(ctx, slot, "plan clear").map_err(CmdError::msg)?;
            crate::surface::act::plan_clear_cmd(ctx, &rig)
        }
    }
}

// ---- doc 20 cut SM.4a: +tv/forget and +tv/quiet ----------------------------------------

/// What a forget names: the target the action takes, its words and the site entries it covers.
struct ForgetTarget {
    /// The site entry or the body id.
    target: CanonicalId,
    /// The site's name or the body's display name.
    what: String,
    /// The site entries it covers.
    sites: Vec<CanonicalId>,
}

/// **Resolve a forget's name** (doc 20 cut SM.4a; F10), case-insensitively: each lane site's
/// name, location id and entry id, then each body's id and display name.
fn forget_target(w: &World, name: &str) -> Option<ForgetTarget> {
    // Doc 20 cut SM.4b: the site half is `chart::site_named` (`@geology`'s site form reads it
    // too); one rule, never a copy.
    if let Some(site) = crate::surface::chart::site_named(w, name) {
        return Some(ForgetTarget {
            target: site.entry.clone(),
            what: site.name.clone(),
            sites: vec![site.entry],
        });
    }
    let sites = crate::surface::chart::lane_sites(w);
    let wanted = name.to_lowercase();
    let body = sites.iter().map(|site| &site.body).find(|body| {
        body.as_str() == wanted || crate::port::site::body_name(body).to_lowercase() == wanted
    })?;
    Some(ForgetTarget {
        target: body.clone(),
        what: crate::port::site::body_name(body),
        sites: sites
            .iter()
            .filter(|site| &site.body == body)
            .map(|site| site.entry.clone())
            .collect(),
    })
}

/// **`+tv/forget <site>|<body> [go]`** (doc 20 §5, §10; cut SM.4a, D9, D27): the name
/// resolved (`FORGET_UNKNOWN`); the actor's rows it covers counted — fields scanned, pockets
/// sounded, and budget-keepers whose IC hour is over (the pre-commit round's item 6: the forget
/// prunes them, their only path to deletion). A keeper holds no reading the player can see, so
/// with no fields and no pockets the answer is `FORGET_NOTHING` whatever the keeper's hour (the
/// lead-round review's S1): bare, nothing is written; with `go` and an expired keeper, the same
/// `tacsim_chart_forget` is submitted to prune it before `FORGET_NOTHING`. An unreadable row
/// answers `CHART_UNREAD`; otherwise bare, the preview;
/// with `go`, ONE `tacsim_chart_forget` submission keyed `tacsim.chart/{actor}/forget/{target}/
/// {now_ic}` and `FORGOTTEN`. No rig, hull or place is needed; no licence or tenure is read.
fn forget(ctx: &mut Ctx<'_>, name: &str, go: bool) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let handles = ctx
        .w
        .module_state::<crate::port::families::PortHandles>()
        .cloned()
        .ok_or_else(|| CmdError::msg(TV_UNWIRED))?;
    let Some(action) = handles.chart_forget.clone() else {
        return Err(CmdError::msg(TV_UNWIRED));
    };
    let Some(target) = forget_target(ctx.w, name) else {
        return Err(CmdError::msg(FORGET_UNKNOWN.replace("{arg}", name)));
    };
    let now_ic = ctx.w.world_ic_now().at;
    let mut fields = 0_usize;
    let mut pockets = 0_usize;
    let mut stale = 0_usize;
    for site in &target.sites {
        let key = crate::surface::chart::ChartKey {
            principal: actor,
            site: site.clone(),
        };
        let unread = || CmdError::msg(CHART_UNREAD.replace("{what}", &target.what));
        let row = match ctx.action_state(&handles.families.charts, &key) {
            Ok(row) => row,
            Err(error) => {
                tracing::warn!(pilot = actor.0, %error, "tacsim: a chart row does not read");
                return Err(unread());
            }
        };
        match row {
            // The families rule: a reader re-checks the key (the prepare refuses it too).
            Some(row) if row.key() != key => {
                tracing::warn!(
                    pilot = actor.0,
                    site = %site,
                    "tacsim: a chart row disagrees with its own key"
                );
                return Err(unread());
            }
            Some(row) if row.has_coverage() => {
                fields = fields.saturating_add(row.fields.len());
                pockets = pockets.saturating_add(row.sounded_pockets());
            }
            // A budget-keeper whose IC hour is over: the forget prunes it.
            Some(row) if crate::surface::chart::heard_line_due(row.heard_ic, now_ic) => {
                stale = stale.saturating_add(1);
            }
            Some(_) | None => {}
        }
    }
    // A budget-keeper holds no readings the player can see, so it never surfaces as a forget
    // of "0 fields and 0 pockets" (the lead-round review's S1): with nothing visible the answer
    // is `FORGET_NOTHING` either side of the keeper's hour. Bare, nothing is written; with `go`
    // and an expired keeper, the same forget is submitted silently to prune it — and once it
    // lands `FORGET_NOTHING` is simply true.
    let nothing = FORGET_NOTHING.replace("{what}", &target.what);
    if fields == 0 && pockets == 0 {
        if !go || stale == 0 {
            return Err(CmdError::msg(nothing));
        }
        let Some(key) = crate::surface::chart::forget_key(actor, &target.target, now_ic) else {
            tracing::error!(pilot = actor.0, "tacsim: a forget's key did not form");
            return Err(CmdError::msg(TV_UNWIRED));
        };
        ctx.submit_authoritative(
            action,
            mush_core::PerceivedRef::SelfActor,
            Vec::new(),
            serde_json::json!({ "target": target.target.as_str() }),
            key,
        );
        say(ctx, nothing);
        return Ok(());
    }
    if !go {
        say(
            ctx,
            FORGET_PREVIEW
                .replace("{what}", &target.what)
                .replace("{fields}", &fields.to_string())
                .replace("{pockets}", &pockets.to_string())
                .replace("{arg}", name),
        );
        return Ok(());
    }
    let Some(key) = crate::surface::chart::forget_key(actor, &target.target, now_ic) else {
        tracing::error!(pilot = actor.0, "tacsim: a forget's key did not form");
        return Err(CmdError::msg(TV_UNWIRED));
    };
    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({ "target": target.target.as_str() }),
        key,
    );
    say(ctx, FORGOTTEN.replace("{what}", &target.what));
    Ok(())
}

/// **`+tv/quiet [on|off]`** (doc 20 §5, §10; cut SM.4a, D10): set, or toggle, the actor's
/// quiet preference (`tacsim:tvpref:<character>`) and say which. Not an act: no `go`.
fn quiet(ctx: &mut Ctx<'_>, set: Option<bool>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let now = ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(|store| store.tv_quiet(actor));
    let quiet = set.unwrap_or(!now);
    crate::store::put_tv_prefs(
        ctx,
        &TvPrefs {
            schema: TV_PREF_SCHEMA,
            character: actor,
            quiet,
        },
    );
    say(ctx, if quiet { QUIET_ON } else { QUIET_OFF });
    Ok(())
}
/// **The slot selector** (doc 20 cut SM.3b; lead ruling 6; SM.3a's finding F10): a named
/// slot must hold one of the actor's rigs (`NO_RIG`, also past the bay's slots); with none
/// named, the slot the actor named last if a rig is still there, else the only rig, else
/// [`WHICH_RIG_ACT`] with two or more; with no rig at all, the line the status answers in
/// its own order (no charter, dark, `NO_BAY`), else `NO_RIG` for slot 1.
fn select(
    ctx: &Ctx<'_>,
    named: Option<u8>,
    kind: crate::surface::excursion::ActKind,
) -> Result<crate::surface::act::RigAt, String> {
    select_word(ctx, named, kind.word())
}

/// [`select`] for an act named by its words (doc 20 cut SM.11: `plan`, `plan show`, `plan
/// clear`).
fn select_word(
    ctx: &Ctx<'_>,
    named: Option<u8>,
    word: &str,
) -> Result<crate::surface::act::RigAt, String> {
    if let Some(physical) = crate::detachment::sortie::surface::rig(ctx) {
        return select_among(
            ctx.w,
            ctx.actor,
            vec![physical],
            named,
            &WHICH_RIG_ACT.replace("{act}", word),
        );
    }
    let w: &World = ctx.w;
    if crate::surface::act::operator_rigs(w, ctx.actor).is_empty() {
        status_lines(ctx, named, &mut Vec::new())?;
    }
    select_in(w, ctx.actor, named, &WHICH_RIG_ACT.replace("{act}", word))
}

/// [`select_word`] for the plan's own read and clear (doc 20 cut SM.11; the lead's pre-commit
/// round, item 3, the judge's correctness/F5: D22's "a plan stays clearable"): at the helm,
/// exactly [`select_word`]; off it, the actor's own leased rigs on a hull they are aboard, by the
/// same rule ([`select_among`]) — `plan_show_cmd` and `plan_clear_cmd` then read only the actor's
/// own plan. With no such rig, [`select_word`]'s refusal.
fn select_plan_word(
    ctx: &Ctx<'_>,
    named: Option<u8>,
    word: &str,
) -> Result<crate::surface::act::RigAt, String> {
    if crate::detachment::sortie::surface::rig(ctx).is_some() {
        return select_word(ctx, named, word);
    }
    let w: &World = ctx.w;
    if !crate::surface::act::operator_rigs(w, ctx.actor).is_empty() {
        return select_word(ctx, named, word);
    }
    let aboard: Vec<crate::surface::act::RigAt> = crate::surface::act::leased_rigs(w, ctx.actor)
        .into_iter()
        .filter(|rig| crate::port::voyage::aboard(w, rig.hull, ctx.actor))
        .collect();
    if aboard.is_empty() {
        return select_word(ctx, named, word);
    }
    select_among(
        w,
        ctx.actor,
        aboard,
        named,
        &WHICH_RIG_ACT.replace("{act}", word),
    )
}

/// **Which rig an act names** (doc 20 cut SM.3b's rule; factored out by cut SM.10 so the
/// stake's plan, which holds no `Ctx` at the offer, reads the same one): the slot named, else the
/// slot the actor named last (`RigPointer`), else the only rig; with none, `NO_RIG` (or
/// `NO_CHARTER` with no hull); with two and none named, `which` with its `{slots}` filled.
pub(crate) fn select_in(
    w: &World,
    actor: ObjId,
    named: Option<u8>,
    which: &str,
) -> Result<crate::surface::act::RigAt, String> {
    select_among(
        w,
        actor,
        crate::surface::act::operator_rigs(w, actor),
        named,
        which,
    )
}

/// [`select_in`]'s rule over `rigs` (the lead's pre-commit round, item 3: the helm's rigs, or the
/// plan author's own off it).
fn select_among(
    w: &World,
    actor: ObjId,
    rigs: Vec<crate::surface::act::RigAt>,
    named: Option<u8>,
    which: &str,
) -> Result<crate::surface::act::RigAt, String> {
    let no_rig = |n: u8, hull: ObjId| {
        NO_RIG
            .replace("{n}", &n.to_string())
            .replace("{name}", &hull_name(w, hull))
    };
    if rigs.is_empty() {
        // Doc 20 cut SM.12b: the helm's hull, else a dispatcher's yard hull.
        let hull = crate::port::yard::operating_hull(w, actor)
            .or_else(|| live_records(w, actor).first().map(|record| record.hull));
        return Err(match hull {
            Some(hull) => no_rig(named.unwrap_or(1), hull),
            None => NO_CHARTER.to_owned(),
        });
    }
    if let Some(n) = named {
        return rigs
            .iter()
            .find(|rig| rig.slot == n)
            .cloned()
            .ok_or_else(|| no_rig(n, rigs.first().map_or(ObjId(0), |rig| rig.hull)));
    }
    if let Some(n) = pointed(w, actor)
        && let Some(rig) = rigs.iter().find(|rig| rig.slot == n)
    {
        return Ok(rig.clone());
    }
    match rigs.as_slice() {
        [only] => Ok(only.clone()),
        _ => {
            let slots: Vec<u8> = rigs.iter().map(|rig| rig.slot).collect();
            Err(which.replace("{slots}", &slots_words(&slots)))
        }
    }
}

// ---- the status --------------------------------------------------------------------

/// **`+tv`** — a pure read (doc 20 §4.3): the hull (the live charter's, else the
/// one the actor's live records name), then one line per bay slot, the unlisted
/// rigs, and on a site's pad SM.2's stores line.
fn status(ctx: &mut Ctx<'_>, only: Option<u8>) -> Result<(), CmdError> {
    let mut wants = Vec::new();
    let lines = status_lines(ctx, only, &mut wants).map_err(CmdError::msg)?;
    // Doc 20 cut SM.4a (lead ruling 6): a cold site the status peeked is asked for (RAM only).
    for entry in wants {
        crate::surface::want_site_ground(ctx.w, &entry);
    }
    for line in lines {
        say(ctx, line);
    }
    Ok(())
}

/// The status, read-only; `Err` is the refusal. `wants` gathers the cold sites the ground
/// lines peeked (doc 20 cut SM.4a), for the caller to ask for.
#[allow(clippy::too_many_lines)] // one slot walk, one line per state
fn status_lines(
    ctx: &Ctx<'_>,
    only: Option<u8>,
    wants: &mut Vec<CanonicalId>,
) -> Result<Vec<String>, String> {
    let w: &World = ctx.w;
    let actor = ctx.actor;
    let port = programme_port(w).ok_or_else(|| NOT_OPEN_HERE.to_owned())?;
    // 1. The hull: (doc 20 cut SM.12b) a dispatcher's yard hull first; else the current helm's,
    // else the one the actor's live records name; with neither, an unreadable record of the
    // actor's still darkens the whole reply (reviewer finding correctness/F3) before the
    // charter's own refusal is said.
    let yard_hull = crate::port::yard::yard_of_dispatcher(w, actor)
        .and_then(|yard| yard.row.hulls.first().map(|row| ObjId(row.hull)));
    let hull = match yard_hull.map_or_else(|| crate::port::coop::mining_hull(ctx), Ok) {
        Ok(hull) => hull,
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
    // Doc 20 cut SM.3b: a slot past the bay's names no rig (the switches run to
    // `MAX_BAY_ITEMS`, the bay may hold fewer).
    if let Some(n) = only.filter(|n| *n > max) {
        return Err(NO_RIG
            .replace("{n}", &n.to_string())
            .replace("{name}", &name));
    }
    // Doc 20 cut SM.3b (lead ruling 9): where each rig is and its charge now, read from
    // the one tools table and the rig's typed record; the hull's site berth, once.
    let berth = crate::surface::act::site_berth(w, &view).ok();
    let handles = w.module_state::<crate::port::families::PortHandles>();
    // Doc 20 cut SM.4a (the brief's reviewer finding F5): each site's composite is peeked at
    // most once for the whole status.
    let mut peeks = crate::surface::Peeks::default();
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
                        lines.push(if crate::relay_foundry::known_cargo(w,rig.tv){
                format!("Relay cargo #{}: 5 tonnes, one bay slot; unload with +relaypair/unload {} at the dock.",rig.tv.0,rig.tv)
            }else{RIG_UNLISTED.replace("{mark}", &rig.mark)});
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
        // Doc 20 cut SM.12b: a house rig reads its own slot line and the house's line, read
        // before the economy lease (it has none).
        let house = record.is_house();
        let line = if house {
            crate::port::yard::HOUSE_RIG_SLOT.to_owned()
        } else if !economy_lease_live(w, &lessor.principal, record) {
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
        let (place, charge, acts) = rig_state(ctx, handles, &view, berth.as_ref(), record.tv, now);
        lines.push(
            line.replace("{n}", &n.to_string())
                .replace("{mark}", &record.mark)
                .replace("{where}", &place)
                .replace("{charge}", &charge)
                .replace("{hopper}", &hopper),
        );
        if house {
            lines.push(crate::port::yard::house_rig_line(w, record.pilot));
        }
        lines.extend(acts);
        // Doc 20 cut SM.4a (§5, §10; lead ruling 8): where a rig on the ground stands.
        if let (Some(berth), Some(handles)) = (berth.as_ref(), handles)
            && let Some(ground) = ground_line(ctx, handles, berth, record.tv, &mut peeks, wants)
        {
            lines.push(ground);
        }
    }
    if only.is_none() {
        for rig in unlisted {
            lines.push(if crate::relay_foundry::known_cargo(w,rig.tv){
                format!("Relay cargo #{}: 5 tonnes, one bay slot; unload with +relaypair/unload {} at the dock.",rig.tv.0,rig.tv)
            }else{RIG_UNLISTED.replace("{mark}", &rig.mark)});
        }
    }
    // 4. On a site's pad, SM.2's stores line (the SM.2 note deferred it here); never for a yard
    // hull (doc 20 cut SM.12b: a yard is no landed passage).
    if !view.yard
        && let Some(stores) = crate::port::voyage::settled_stores_line(ctx, &view)
    {
        lines.push(stores);
    }
    Ok(lines)
}

/// **One rig's `{where}`, `{charge}` and act line** (doc 20 cut SM.3b; lead ruling 9,
/// finding F9): `WHERE_BAY` in this hull's bay; on this hull's site domain `WHERE_RAMP`
/// within the ramp's cell, else `WHERE_GROUND` (`{m}` = cells × 10); anything else
/// `WHERE_AWAY` (a staff move). `{charge}` is `⌊charge now × 100 / capacity⌋`, or `?` when
/// the rig's battery or its record cannot be read. The act line (`ACT_LINE`) when an act
/// is in flight; **doc 20 cut SM.7**: first the run's line (`RUN_LINE`, or `CORE_LINE` for a blind
/// core) when a run drills — an act beside it prints its own line after.
fn rig_state(
    ctx: &Ctx<'_>,
    handles: Option<&crate::port::families::PortHandles>,
    view: &HullView<'_>,
    berth: Option<&crate::surface::act::SiteBerth<'_>>,
    tv: ObjId,
    now: i64,
) -> (String, String, Vec<String>) {
    use crate::surface::act::{Where, where_is};
    use crate::surface::excursion::{RigKey, RigRecord, cells_apart, charge_shown, site_cell};
    let w: &World = ctx.w;
    let name = hull_name(w, view.hull());
    let ramp = crate::surface::act::ramp_label(w, view.hull());
    let place = match (where_is(w, tv), berth) {
        (Where::Bay { hull }, _) if hull == view.hull() => WHERE_BAY.to_owned(),
        (
            Where::Ground {
                domain, x_cm, y_cm, ..
            },
            Some(berth),
        ) if domain == berth.domain => {
            let cells = cells_apart(
                site_cell(x_cm, y_cm),
                site_cell(berth.ramp.x_cm, berth.ramp.y_cm),
            );
            if cells == 0 {
                WHERE_RAMP
                    .replace("{ramp}", &ramp)
                    .replace("{site}", &berth.name)
            } else {
                WHERE_GROUND
                    .replace("{site}", &berth.name)
                    .replace("{m}", &cells.saturating_mul(10).to_string())
                    .replace("{ramp}", &ramp)
            }
        }
        _ => WHERE_AWAY.replace("{name}", &name),
    };
    let battery = handles
        .and_then(|handles| handles.surface_tools.clone())
        .and_then(|tools| {
            crate::surface::act::rig_battery_id(w, tv)
                .and_then(|id| crate::surface::act::battery_of(&tools, &id).cloned())
        });
    let record = handles.and_then(|handles| {
        match ctx.action_state(&handles.families.excursions, &RigKey(tv)) {
            Ok(found) => Some(found.cloned()),
            Err(error) => {
                tracing::warn!(tv = tv.0, %error, "tacsim: +tv could not read a rig's record");
                None
            }
        }
    });
    // The act line reads the record alone: a rig whose battery row stops resolving still shows
    // what it is doing (§4.1 "never stranded by its battery"; the fixer round's item 3).
    let mut acts: Vec<String> = Vec::new();
    if let Some(run) = record
        .as_ref()
        .and_then(|found| found.as_ref())
        .and_then(|record| record.run.as_ref())
    {
        let addr = crate::surface::address::pocket_address(run.pocket[0], run.pocket[1])
            .unwrap_or_default();
        let ore = ore_words(run.good.as_str());
        let when = due_words(w, run.due_ic);
        acts.push(if run.blind {
            CORE_LINE
                .replace("{addr}", &addr)
                .replace("{ore}", &ore)
                .replace("{when}", &when)
        } else {
            RUN_LINE
                .replace("{addr}", &addr)
                .replace("{ore}", &ore)
                .replace("{k}", &run.k.to_string())
                .replace("{cycles}", &run.cycles.to_string())
                .replace("{when}", &when)
                .replace("{stop}", if run.stop { RUN_STOP_CLAUSE } else { "" })
        });
    }
    if let Some(act) = record
        .as_ref()
        .and_then(|found| found.as_ref())
        .and_then(|record| record.act.as_ref())
    {
        acts.push(
            ACT_LINE
                .replace(
                    "{doing}",
                    &crate::surface::act::doing_words(act.kind, &act.params, &ramp),
                )
                .replace("{when}", &due_words(w, act.due_ic)),
        );
    }
    // Doc 20 cut SM.8 (lead ruling 1): a recover's queued unload, waiting on the surface tick.
    if record
        .as_ref()
        .and_then(|found| found.as_ref())
        .is_some_and(|record| record.queued.is_some())
    {
        acts.push(QUEUED_LINE.to_owned());
    }
    // Doc 20 cut SM.11 (the brief's 3.5): the rig's plan, live or ended, under its lines — only
    // the asker's own (the fixer round's conformance/F1: a previous lessee's plan is never shown).
    if let Some(plan) = record
        .as_ref()
        .and_then(|found| found.as_ref())
        .and_then(|record| record.plan.as_ref())
        .filter(|plan| plan.author == ctx.actor)
    {
        use crate::surface::plan::FillState;
        // The step clamped to the count, and a parking plan named as coming home, never by the
        // step it skips (the fixer round's correctness/F4).
        let view = crate::surface::plan::plan_view(plan);
        let line = match plan.state {
            FillState::Live | FillState::Parking { .. } => PLAN_LINE
                .replace("{no}", &plan.no.to_string())
                .replace("{k}", &view.step.to_string())
                .replace("{steps}", &view.steps.to_string())
                .replace(
                    "{words}",
                    view.next.first().map_or(STATE_PARKING, String::as_str),
                )
                .replace("{when}", &due_words(w, plan.deadline_ic)),
            FillState::Ended { why, .. } => {
                let words = crate::surface::act::PlanWords::of(w, plan);
                PLAN_LINE_ENDED
                    .replace("{no}", &plan.no.to_string())
                    .replace(
                        "{why}",
                        crate::surface::act::end_why(w, plan.author, why, &words)
                            .trim_end_matches('.'),
                    )
            }
        };
        acts.push(line);
    }
    let charge = match (battery, record) {
        (Some(battery), Some(record)) => {
            let record = record.unwrap_or_else(|| RigRecord::fresh(tv, &battery));
            // Doc 20 cut SM.4a (D22): an implied launch's bay credit is read no later than its
            // launch half.
            let now_cu = charge_shown(&record, now, &battery);
            crate::surface::act::percent(now_cu, &battery).to_string()
        }
        _ => "?".to_owned(),
    };
    (place, charge, acts)
}

/// **The status's ground line** (doc 20 cut SM.4a; §5 "the `+tv` status line always says where
/// each rig stands", §10; lead ruling 8, D18, D25): for a rig `Direct` on the hull's site
/// domain, its block letter and the block's tenure word (`commons`, `open`, `held by {org}`),
/// its field's name, and — through the non-building peek (`peeks`, one per site per call) — its
/// footing and the reading there: the pocket's text when the actor's chart has sounded the
/// rig's pocket, else the field's row when it has scanned the field, else
/// [`NO_READINGS_HERE`]. The block, the tenure and the field come from the address arithmetic
/// and the lane's site block alone, so they always print; a dark site prints
/// [`GROUND_DARK_LINE`], a cold one [`GROUND_COLD_LINE`] and is added to `wants`. `None` for a
/// rig anywhere else.
fn ground_line(
    ctx: &Ctx<'_>,
    handles: &crate::port::families::PortHandles,
    berth: &crate::surface::act::SiteBerth<'_>,
    tv: ObjId,
    peeks: &mut crate::surface::Peeks,
    wants: &mut Vec<CanonicalId>,
) -> Option<String> {
    use crate::surface::act::{Where, where_is};
    use crate::surface::address;
    use crate::surface::chart::{self, SitePeek};
    let w: &World = ctx.w;
    let Where::Ground {
        domain, x_cm, y_cm, ..
    } = where_is(w, tv)
    else {
        return None;
    };
    if domain != berth.domain {
        return None;
    }
    let cell = crate::surface::excursion::site_cell(x_cm, y_cm);
    let pocket = address::pocket_of_cell(cell)?;
    let field = address::field_of_cell(cell)?;
    let letter = address::block_letter(pocket.0, pocket.1)?;
    let site_def = berth.def.site.as_ref()?;
    let pack = w
        .module_state::<TacsimStore>()
        .and_then(|store| crate::port::readiness::report_ports(store, w).0);
    // Doc 20 cut SM.10 (the brief's 3.8 item 20): the block's claim record, else the
    // authored word.
    let tenure = crate::port::claims::block_tenure_now(
        w,
        ctx,
        ctx.actor,
        &berth.entry,
        site_def,
        letter,
        pack,
        w.world_ic_now().at,
    );
    let field_word = address::field_name(field.0, field.1)?;
    let fill = |template: &str| {
        template
            .replace("{block}", &letter.to_string())
            .replace("{tenure}", &tenure)
            .replace("{field}", &field_word)
    };
    let ground = match chart::peek_site(w, &berth.entry, peeks) {
        SitePeek::Dark => return Some(fill(GROUND_DARK_LINE)),
        SitePeek::Cold => {
            if !wants.contains(&berth.entry) {
                wants.push(berth.entry.clone());
            }
            return Some(fill(GROUND_COLD_LINE));
        }
        SitePeek::Warm(ground) => ground,
    };
    let (Some(surface), Ok(site)) = (
        chart::ground_surface(&ground),
        chart::site_context(w, &berth.entry),
    ) else {
        return Some(fill(GROUND_DARK_LINE));
    };
    let footing = match chart::footing_at(surface, cell) {
        Ok(footing) => footing.name().to_owned(),
        Err(terrain) => terrain.label().replace('_', " "),
    };
    let key = chart::ChartKey {
        principal: ctx.actor,
        site: berth.entry.clone(),
    };
    let row = match ctx.action_state(&handles.families.charts, &key) {
        Ok(found) => found.cloned(),
        Err(error) => {
            tracing::warn!(pilot = ctx.actor.0, %error, "tacsim: +tv could not read a chart row");
            None
        }
    };
    let row = row.as_ref();
    let reading = if row.is_some_and(|row| {
        row.rule == crate::surface::READING_RULE_VERSION && row.pocket_sounded(pocket)
    }) {
        chart::pocket_text(&site, row, pocket)
    } else if row.is_some_and(|row| {
        row.rule == crate::surface::READING_RULE_VERSION
            && address::field_index(field.0, field.1).is_some_and(|i| row.fields.contains_key(&i))
    }) {
        chart::field_text(&site, row, field)
    } else {
        NO_READINGS_HERE.to_owned()
    };
    Some(
        fill(GROUND_LINE)
            .replace("{footing}", &footing)
            .replace("{reading}", &reading),
    )
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

/// **Why the take refused** (doc 20b RK-1): a line, or the licence's refusal with the
/// term settle it must not block — the steps and the pilot's account, `None` when the
/// account does not read — and the lessor that settle posts to.
enum TakeRefusal {
    /// Every refusal but the licence's.
    Line(String),
    /// Check 6: the licence does not authorize.
    Licence(Box<LicenceRefusal>),
}

/// [`TakeRefusal::Licence`]'s fields.
struct LicenceRefusal {
    line: String,
    settle: Option<(Vec<Settle>, AccountId)>,
    lessor: Lessor,
}

impl From<String> for TakeRefusal {
    fn from(line: String) -> Self {
        TakeRefusal::Line(line)
    }
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
fn plan_take(ctx: &Ctx<'_>, named: Option<u8>) -> Result<TakePlan, TakeRefusal> {
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
    // 1. The current charter or title, never graduation alone.
    let hull = crate::port::coop::mining_hull(ctx)?;
    let name = hull_name(w, hull);
    // 2. A prospector hull.
    let view = hull_view(w, hull).ok_or_else(|| failed("the hull's view"))?;
    if view.unit_held {
        tracing::warn!(
            hull = hull.0,
            "tacsim: +tv/lease: the hull's unit record is held"
        );
        return Err(TakeRefusal::from(TV_DARK.replace("{name}", &name)));
    }
    if !view.is_prospector() {
        let (_, office, port_label) = office_of(w, &port);
        return Err(TakeRefusal::from(
            NO_BAY
                .replace("{name}", &name)
                .replace("{office}", &office)
                .replace("{port}", &port_label),
        ));
    }
    let lessor = lessor_of(w, &port).ok_or_else(|| failed("the rig lessor"))?;
    // 3. The berth.
    match berth_of(w, hull) {
        Berth::UnderWay => {
            return Err(TakeRefusal::from(
                LEASE_UNDER_WAY
                    .replace("{name}", &name)
                    .replace("{port}", &lessor.port),
            ));
        }
        Berth::Docked { map, .. } if map.entry == port.entry => {}
        Berth::Docked { .. } | Berth::Elsewhere => {
            return Err(TakeRefusal::from(
                LEASE_NOT_HERE.replace("{port}", &lessor.port),
            ));
        }
    }
    let hold = crate::port::coop::mining_hold(w, actor, &view)
        .ok_or_else(|| failed("the hull's active, pilot-owned hold"))?;
    // 4. Nothing in flight, using the current hull and actual hold.
    if let Some((_, line)) = crate::port::buyout::in_flight_on(w, hull, actor, hold, &name) {
        return Err(TakeRefusal::from(line));
    }
    // 5. At the counter.
    let here = w.get(actor).and_then(|o| o.location);
    if lessor.counter.is_none() || here != lessor.counter {
        return Err(TakeRefusal::from(where_line(LEASE_WHERE, &lessor)));
    }
    // 6. The licence (doc 20 cut SM.5): in force or in grace, read at the act. Doc 20b
    // RK-1: the refusal carries the term settle, which a lapsed licence never blocks.
    if let Err(line) = licence_check(w, actor) {
        let settle = funds(w, actor).map(|(pilot_account, available)| {
            let now = w.world_ic_now().at;
            (
                plan_settle(w, actor, &lessor.principal, now, available).0,
                pilot_account,
            )
        });
        return Err(TakeRefusal::Licence(Box::new(LicenceRefusal {
            line,
            settle,
            lessor,
        })));
    }
    // 7. The slot.
    let (bay, bay_room) = view.bay().ok_or_else(|| failed("the hull's bay"))?;
    if !crate::port::hull::bay_laid_up(w, view.row, bay) {
        return Err(TakeRefusal::from(failed("the bay is not laid up")));
    }
    let bay_slot =
        mush_sim::SlotId::new(bay.slot.id.as_str()).ok_or_else(|| failed("the bay slot"))?;
    let max = u8::try_from(bay.slot.maximum_items).unwrap_or(u8::MAX);
    let taken: Vec<u8> = live_on_hull(w, hull)
        .iter()
        .map(|record| record.slot)
        .collect();
    let slot = match named {
        Some(n) if n > max => {
            return Err(TakeRefusal::from(
                NO_RIG
                    .replace("{n}", &n.to_string())
                    .replace("{name}", &name),
            ));
        }
        Some(n) if taken.contains(&n) => {
            return Err(TakeRefusal::from(
                SLOT_TAKEN
                    .replace("{n}", &n.to_string())
                    .replace("{name}", &name),
            ));
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
        return Err(TakeRefusal::from(BAY_FULL.replace("{name}", &name)));
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
        return Err(TakeRefusal::from(
            LEASE_FUNDS
                .replace("{total}", &money(ctx, total))
                .replace("{fee}", &money(ctx, fee_minor))
                .replace("{deposit}", &money(ctx, deposit_minor))
                .replace("{available}", &money(ctx, left)),
        ));
    }
    // 10. The economy, Occator's account and scope, the hopper vault.
    let economy = w
        .module_state::<EconomyStore>()
        .ok_or_else(|| failed("the economy"))?;
    if economy.is_frozen() {
        return Err(TakeRefusal::from(failed("the economy is frozen")));
    }
    let lessor_account = lessor.account.ok_or_else(|| failed("Occator's account"))?;
    if economy.snapshot().account(lessor_account).is_none() {
        return Err(TakeRefusal::from(failed("Occator's account")));
    }
    let tv = ObjId(rig.tv);
    if rig
        .vault
        .is_none_or(|vault| economy.snapshot().vault(VaultId(vault)).is_none())
    {
        return Err(TakeRefusal::from(failed("the rig's hopper vault")));
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
    let now = ctx.w.world_ic_now().at;
    let plan = match plan_take(ctx, named) {
        Ok(plan) => plan,
        Err(TakeRefusal::Line(line)) => return Err(CmdError::msg(line)),
        // Doc 20b RK-1: a lapsed licence refuses the lease, never the term settle (D8's
        // "at the pilot's next `+tv/lease` act"), so a past-term rig is re-debited or
        // forfeited like anyone's. With a settle due, a preview says it and a `go` runs
        // it, and the licence's line closes the reply instead of refusing it: a refusal
        // would withdraw the settle's own lines (dispatch keeps no success line of a
        // handler that errs). With none due, it is the plain refusal it always was.
        Err(TakeRefusal::Licence(refusal)) => {
            let LicenceRefusal {
                line,
                settle,
                lessor,
            } = *refusal;
            let Some((steps, pilot_account)) = settle.filter(|(steps, _)| !steps.is_empty()) else {
                return Err(CmdError::msg(line));
            };
            if go {
                if !run_settle(ctx, &steps, pilot_account, &lessor) {
                    return Err(CmdError::msg(line));
                }
            } else {
                for step in &steps {
                    let said = settle_line(ctx, step, now);
                    say(ctx, said);
                }
            }
            say(ctx, line);
            return Ok(());
        }
    };
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
    let charter = crate::port::coop::mining_hull(ctx);
    let charter_name = charter
        .as_ref()
        .map_or_else(|_| String::new(), |hull| hull_name(w, *hull));
    // 1. The actor's live records.
    if store.tv_lease_unreadable_for(actor) {
        tracing::error!(
            pilot = actor.0,
            "tacsim: +tv/lease return found an unreadable tacsim:tvlease: record"
        );
        return Err(dark_line(w, store, actor, charter.as_ref().ok().copied()));
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
    // 3. The record's current hull view and actual hold. The lessee may return a
    // rig after losing the helm, without a surviving charter document.
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
    let hold = crate::port::ship::hold_vault(w, &view).ok_or_else(|| failed("the hull's hold"))?;
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
    if super::boarding::occupied(w, record.tv) {
        return Err(
            "Crew are still inside this rig; have them step into the vehicle bay first.".into(),
        );
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
    if let Some((_, line)) =
        crate::port::buyout::in_flight_on(w, record.hull, record.pilot, hold, &name)
    {
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
    // 11. Doc 20 cut SM.3b: no act in flight and no excursion open (a backstop against a
    // staff move: every act needs the hull on a site's pad, the return Occator's counter at
    // Ceres). Read through `rig_flight`, the reader `repossess` and the bay gate share, and
    // fail closed (the fixer round's item 1): a record that will not read refuses and pages,
    // so a rig never goes back to the shed with an excursion still counted against the caps.
    match rig_flight(ctx, record.tv) {
        Ok(RigFlight::Idle) => {}
        Ok(RigFlight::Busy {
            kind,
            due_ic,
            params,
        }) => {
            let ramp = crate::surface::act::ramp_label(w, record.hull);
            return Err(RIG_BUSY
                .replace("{n}", &slot)
                .replace(
                    "{doing}",
                    &crate::surface::act::doing_words(kind, &params, &ramp),
                )
                .replace("{when}", &due_words(w, due_ic)));
        }
        Ok(RigFlight::Drilling { pocket, due_ic }) => {
            return Err(format!(
                "{}{}",
                RIG_BUSY
                    .replace("{n}", &slot)
                    .replace("{doing}", &crate::surface::act::drilling_words(&pocket))
                    .replace("{when}", &due_words(w, due_ic)),
                RUN_STOP_REMEDY.replace("{n}", &slot)
            ));
        }
        // Doc 20 cut SM.11 (the brief's 3.9): a rig working a plan, and the clear that ends it.
        Ok(RigFlight::Planning { .. }) => {
            return Err(format!(
                "{}{}",
                RIG_BUSY
                    .replace("{n}", &slot)
                    .replace("{doing}", PLAN_DOING)
                    .replace(": done {when}", ""),
                PLAN_REMEDY.replace("{n}", &slot)
            ));
        }
        Ok(RigFlight::Out { site }) => {
            return Err(RIG_LOGGED_OUT
                .replace("{n}", &slot)
                .replace("{site}", &crate::surface::act::site_name(w, &site)));
        }
        Err(error) => {
            tracing::error!(
                tv = record.tv.0,
                lease = record.lease,
                %error,
                "tacsim: +tv/lease return: the rig's tacsim.excursion record does not read; \
                 the return is refused until staff repair the record"
            );
            return Err(LEASE_FAILED.to_owned());
        }
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
    // Doc 20 cut SM.12b (D22): a yard hull is never chartered, so never repossessed; the gather
    // below stays stored-only by design (a repossession ends stored leases).
    if crate::port::yard::yard_of_hull(w, hull).is_some() {
        return Err("the yard hull is never repossessed".to_owned());
    }
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
    // Deduped by rig (doc 20 cut SM.12b: two house views would both carry `HOUSE_LEASE`).
    for record in live_on_hull(w, hull) {
        if !records.iter().any(|seen| seen.tv == record.tv) {
            records.push(record);
        }
    }
    for record in &records {
        if super::boarding::occupied(w, record.tv) {
            return Err(format!("rig {} still contains crew", record.mark));
        }
        // Doc 20 cut SM.3b (decision 13): a rig out on a site or mid-act holds the hull
        // (`BAY_PAGE`) rather than being pulled off the ground under a live record.
        if rig_out_or_busy(ctx, record.tv) {
            return Err(format!("rig {} is out on a site or busy", record.mark));
        }
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

/// **What a rig's `tacsim.excursion` record says it is doing** (doc 20 cut SM.3b; the fixer
/// round's item 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RigFlight {
    /// No record, or one with neither an act, a run nor an open excursion.
    Idle,
    /// An act in flight — it outranks an open excursion: its kind and due instant.
    Busy {
        /// What act.
        kind: crate::surface::excursion::ActKind,
        /// Its due instant, IC seconds.
        due_ic: i64,
        /// Its frozen parameters (doc 20 cut SM.4b: a drive's target names what it is doing).
        params: crate::surface::excursion::ActParams,
    },
    /// **A mining run in flight** (doc 20 cut SM.7; the SM.7 lead's pre-commit round, item 2): no
    /// act, but the record's cycle slot holds a run — read before an open excursion, so the
    /// departure names `+tv/{n} mine stop` rather than a recover the run would refuse, and a run
    /// with no excursion (a staff placement) is busy, never idle.
    Drilling {
        /// The pocket's address (`C117`).
        pocket: String,
        /// The running cycle's due instant, IC seconds.
        due_ic: i64,
    },
    /// **Working a plan** (doc 20 cut SM.11; the brief's 3.9): no act and no run, and a plan live
    /// for the rig's lessee — read after a run and before an open excursion, so the departure and
    /// the lease return name `+tv/{n} plan clear`.
    Planning {
        /// The plan's number.
        no: u32,
        /// The rig stands in a bay (a parked plan holds nothing physical).
        in_bay: bool,
    },
    /// No act and no run, but an excursion open on `site` (the site port entry).
    Out {
        /// The excursion's site.
        site: CanonicalId,
    },
}

/// **One reader of a rig's `tacsim.excursion` record** (doc 20 cut SM.3b; the fixer round's
/// item 1): the lease return's step 11, `repossess` and the departure's bay gate
/// (`crate::port::voyage::bay_gate`) all read through it, so the three cannot drift. A roster
/// without the port tick's handles has no family and reads `Idle`.
///
/// # Errors
///
/// The record will not read, or disagrees with its own key (the families rule: readers
/// re-check the key). Every caller fails closed on it.
pub(crate) fn rig_flight(ctx: &Ctx<'_>, tv: ObjId) -> Result<RigFlight, String> {
    let Some(handles) = ctx.w.module_state::<crate::port::families::PortHandles>() else {
        return Ok(RigFlight::Idle);
    };
    let found = ctx
        .action_state(
            &handles.families.excursions,
            &crate::surface::excursion::RigKey(tv),
        )
        .map_err(|error| error.to_string())?;
    let Some(record) = found else {
        return Ok(RigFlight::Idle);
    };
    if record.tv != tv {
        return Err(format!(
            "the record under rig #{}'s key names rig #{}",
            tv.0, record.tv.0
        ));
    }
    // A staff home operation holds the rig until its durable finish, including the
    // interval after the normal-lane placement has already put it in the bay.
    if let Some(recovery) = record.recovery.as_ref().filter(|recovery| recovery.pending) {
        return Ok(RigFlight::Busy {
            kind: crate::surface::excursion::ActKind::Recover,
            due_ic: recovery.at_ic,
            params: crate::surface::excursion::ActParams::None {},
        });
    }
    // An act in flight outranks a run (a scan from a drilling rig names the scan); a run
    // outranks the open excursion it drills from (the SM.7 lead's pre-commit round, item 2); doc
    // 20 cut SM.11: a live plan of the rig's lessee is read between the two.
    // Doc 20 cut SM.12b: the one lessee reader (a yard rig's is its dispatcher).
    let lessee = crate::port::yard::rig_lease(ctx.w, tv).map(|lease| lease.pilot);
    let planning = lessee
        .and_then(|lessee| record.live_plan_for(lessee))
        .map(|plan| plan.no);
    Ok(
        match (&record.act, &record.run, planning, &record.excursion) {
            (Some(act), _, _, _) => RigFlight::Busy {
                kind: act.kind,
                due_ic: act.due_ic,
                params: act.params.clone(),
            },
            (None, Some(run), _, _) => RigFlight::Drilling {
                pocket: crate::surface::address::pocket_address(run.pocket[0], run.pocket[1])
                    .unwrap_or_default(),
                due_ic: run.due_ic,
            },
            (None, None, Some(no), _) => RigFlight::Planning {
                no,
                in_bay: matches!(
                    crate::surface::act::where_is(ctx.w, tv),
                    crate::surface::act::Where::Bay { .. }
                ),
            },
            (None, None, None, Some(excursion)) => RigFlight::Out {
                site: excursion.site.clone(),
            },
            (None, None, None, None) => RigFlight::Idle,
        },
    )
}

/// **Whether a rig is parked, for `await tv`** (doc 20 cut SM.11; the brief's 3.10, lead ruling 5,
/// the reviewer's F3), pure: true when either (a) it stands in this hull's bay (`in_this_bay`) and
/// its record (none is idle) holds no act, no run, no queued unload and no plan live for `lessee`;
/// or (b) it is on the ground with no act, no run and no queued unload, and either its plan ended
/// with no act since (`ended.at_ic ≥ last.at_ic`; no outcome counts as since) — a plan that ended
/// stranded where it stands — or its projection reads [`crate::surface::TvState::Parked`] (an idle
/// rig stranded with no plan). A rig out by hand with yesterday's ended plan is not parked once it
/// has acted since; an unreadable record never counts (the caller passes it as not parked).
#[must_use]
pub(crate) fn rig_parked(
    record: Option<&crate::surface::RigRecord>,
    lessee: Option<ObjId>,
    in_this_bay: bool,
    projected: Option<crate::surface::TvState>,
) -> bool {
    use crate::surface::plan::FillState;
    let Some(record) = record else {
        return in_this_bay;
    };
    let quiet = record.act.is_none()
        && record.run.is_none()
        && record.queued.is_none()
        && !record
            .recovery
            .as_ref()
            .is_some_and(|recovery| recovery.pending);
    if !quiet {
        return false;
    }
    if in_this_bay {
        return lessee.is_none_or(|lessee| record.live_plan_for(lessee).is_none());
    }
    let ended_since = record.plan.as_ref().is_some_and(|plan| match plan.state {
        FillState::Ended { at_ic, .. } => {
            record.last.as_ref().is_none_or(|last| at_ic >= last.at_ic)
        }
        FillState::Live | FillState::Parking { .. } => false,
    });
    ended_since || projected == Some(crate::surface::TvState::Parked)
}

/// **Whether a rig is out on a site or mid-act** (doc 20 cut SM.3b, decision 13):
/// [`rig_flight`] reads an act, a run (doc 20 cut SM.7) or an open excursion. A record that
/// will not read counts as out (fail closed: the hull is held and paged, never stripped).
fn rig_out_or_busy(ctx: &Ctx<'_>, tv: ObjId) -> bool {
    match rig_flight(ctx, tv) {
        // Doc 20 cut SM.11 (the brief's 3.9, D12): a plan parked in the bay holds nothing
        // physical — once the lease ends it is orphaned and inert; a plan's rig out is out.
        Ok(RigFlight::Planning { in_bay: true, .. }) => false,
        Ok(flight) => flight != RigFlight::Idle,
        Err(error) => {
            tracing::warn!(tv = tv.0, %error, "tacsim: a rig's excursion record does not read");
            true
        }
    }
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

    /// Doc 20 cut SM.3b: the slot switches are `1..=MAX_BAY_ITEMS` spelled, and `{slots}`
    /// reads as a list.
    #[test]
    fn sm3b_the_slot_switches_are_the_bay_items() {
        let spelled: Vec<String> = (1..=crate::definitions::logistics::MAX_BAY_ITEMS)
            .map(|n| n.to_string())
            .collect();
        assert_eq!(
            SLOT_SWITCHES
                .iter()
                .map(|s| (*s).to_owned())
                .collect::<Vec<_>>(),
            spelled
        );
        assert_eq!(slots_words(&[1, 2]), "1 or 2");
        assert_eq!(slots_words(&[1, 2, 4]), "1, 2 or 4");
        assert_eq!(slots_words(&[3]), "3");
    }

    /// **The grammar** (doc 20 cut SM.4a; the brief's 3.7): `scan`, its alias `survey`, `sound
    /// quiet go` as switches or first words, with a slot; `forget Ahuna Flank go` keeps the
    /// name as typed; `quiet on|off` and the bare toggle; a slot with `forget` or `quiet`, two
    /// act words, a word after `go`, or `quiet` after a scan is the usage line; an empty forget
    /// asks which.
    #[test]
    fn sm4a_the_grammar() {
        use crate::surface::excursion::ActKind;
        let parse = |switches: &[&str], raw: &str| -> Result<(Option<u8>, Act), String> {
            parse_parts(&|word| switches.contains(&word), raw).map_err(|e| e.to_string())
        };
        let work = |kind, quiet, go| Act::Work {
            kind,
            quiet,
            go,
            mind: None,
        };
        assert_eq!(
            parse(&["1", "scan"], ""),
            Ok((Some(1), work(ActKind::Scan, false, false)))
        );
        assert_eq!(
            parse(&["2"], "survey go"),
            Ok((Some(2), work(ActKind::Scan, false, true)))
        );
        assert_eq!(
            parse(&["survey"], "GO"),
            Ok((None, work(ActKind::Scan, false, true)))
        );
        assert_eq!(
            parse(&["1"], "sound quiet go"),
            Ok((Some(1), work(ActKind::Sound, true, true)))
        );
        assert_eq!(
            parse(&["sound"], "quiet"),
            Ok((None, work(ActKind::Sound, true, false)))
        );
        assert_eq!(
            parse(&[], "forget Ahuna  Flank go"),
            Ok((
                None,
                Act::Forget {
                    name: "Ahuna Flank".to_owned(),
                    go: true
                }
            ))
        );
        assert_eq!(
            parse(&["forget"], "ahuna FLANK"),
            Ok((
                None,
                Act::Forget {
                    name: "ahuna FLANK".to_owned(),
                    go: false
                }
            ))
        );
        assert_eq!(
            parse(&[], "quiet on"),
            Ok((None, Act::Quiet { set: Some(true) }))
        );
        assert_eq!(
            parse(&["quiet"], "off"),
            Ok((None, Act::Quiet { set: Some(false) }))
        );
        assert_eq!(parse(&["quiet"], ""), Ok((None, Act::Quiet { set: None })));
        for (switches, raw) in [
            (&["1", "forget"][..], "ahuna"),
            (&["1"][..], "forget ahuna"),
            (&["2", "quiet"][..], ""),
            (&["scan"][..], "sound"),
            (&["scan", "sound"][..], ""),
            (&[][..], "scan go now"),
            (&[][..], "scan quiet"),
            (&[][..], "quiet loud"),
            (&[][..], "quiet on go"),
            (&[][..], "probe"),
        ] {
            assert_eq!(
                parse(switches, raw),
                Err(TV_USAGE.to_owned()),
                "{switches:?} {raw:?}"
            );
        }
        assert_eq!(parse(&["forget"], "go"), Err(FORGET_WHICH.to_owned()));
        assert_eq!(parse(&[], "forget"), Err(FORGET_WHICH.to_owned()));
        assert_eq!(
            forget_name("  forget   Ahuna   Flank  GO ", true),
            ("Ahuna Flank".to_owned(), true)
        );
        assert_eq!(
            forget_name("forget", false),
            ("forget".to_owned(), false),
            "a switch's raw text is all name"
        );
        assert_eq!(tv_pref_key(ObjId(42)).as_str(), "tacsim:tvpref:42");
        assert_eq!(tv_pref_key_owner("tacsim:tvpref:42"), Some(ObjId(42)));
        assert_eq!(tv_pref_key_owner("tacsim:tv:42"), None);
        assert!(!"tacsim:tvpref:42".starts_with(crate::surface::excursion::TV_REC_PREFIX));
        assert!(!"tacsim:tvpref:42".starts_with(TV_LEASE_PREFIX));
    }

    /// **The drive's and the map's grammar** (doc 20 cut SM.4b; §3.5 and §3.10 of the brief):
    /// `+tv[/<n>]/drive <target…> [go]` as a switch or a first word, the target as typed (case
    /// kept, whitespace collapsed, `go` stripped), an empty target left for the command's
    /// `DRIVE_USAGE`; `+tv[/<n>]/map [site] [family]`; a word past the family, or `go` after
    /// `map`, is the usage line.
    #[test]
    fn sm4b_the_drive_and_map_grammar() {
        let parse = |switches: &[&str], raw: &str| -> Result<(Option<u8>, Act), String> {
            parse_parts(&|word| switches.contains(&word), raw).map_err(|e| e.to_string())
        };
        let drive = |words: &str, go| Act::Drive {
            words: words.to_owned(),
            go,
        };
        assert_eq!(
            parse(&["1", "drive"], "C117"),
            Ok((Some(1), drive("C117", false)))
        );
        assert_eq!(
            parse(&["2"], "drive c-nw GO"),
            Ok((Some(2), drive("c-nw", true)))
        );
        assert_eq!(
            parse(&["drive"], "ne  300 m go"),
            Ok((None, drive("ne 300 m", true)))
        );
        assert_eq!(
            parse(&[], "Drive 140,96"),
            Ok((None, drive("140,96", false)))
        );
        assert_eq!(parse(&["drive"], ""), Ok((None, drive("", false))));
        assert_eq!(
            parse(&["map"], ""),
            Ok((
                None,
                Act::Map {
                    site: false,
                    family: None
                }
            ))
        );
        assert_eq!(
            parse(&["1", "map"], "volatiles"),
            Ok((
                Some(1),
                Act::Map {
                    site: false,
                    family: Some("volatiles".to_owned())
                }
            ))
        );
        assert_eq!(
            parse(&[], "map Site Metals"),
            Ok((
                None,
                Act::Map {
                    site: true,
                    family: Some("metals".to_owned())
                }
            ))
        );
        for (switches, raw) in [
            (&["map"][..], "site metals extra"),
            (&["map"][..], "go"),
            (&["map", "drive"][..], ""),
        ] {
            assert_eq!(
                parse(switches, raw),
                Err(TV_USAGE.to_owned()),
                "{switches:?} {raw:?}"
            );
        }
    }

    /// **The mining grammar** (doc 20 cut SM.7; the brief's 3.6): `+tv[/<n>]/mine [family]
    /// [cycles] [go]` and the alias `drill`, as a switch or a first word, at most one family word
    /// and one count `1..=99` in either order, then `go`; `+tv[/<n>]/mine stop` with nothing after;
    /// two counts, two families, a count of 0 or 100, a word after `go` and `stop go` are the
    /// mining usage line (naming the slot, or `<n>`).
    #[test]
    fn sm7_the_mining_grammar() {
        let parse = |switches: &[&str], raw: &str| -> Result<(Option<u8>, Act), String> {
            parse_parts(&|word| switches.contains(&word), raw).map_err(|e| e.to_string())
        };
        let mine = |family: Option<&str>, cycles: Option<u8>, go| Act::Mine {
            family: family.map(str::to_owned),
            cycles,
            go,
        };
        assert_eq!(
            parse(&["1", "mine"], ""),
            Ok((Some(1), mine(None, None, false)))
        );
        assert_eq!(
            parse(&["1"], "mine go"),
            Ok((Some(1), mine(None, None, true)))
        );
        assert_eq!(
            parse(&["2"], "drill ice 3 go"),
            Ok((Some(2), mine(Some("ice"), Some(3), true)))
        );
        assert_eq!(
            parse(&["drill"], "4 Metals"),
            Ok((None, mine(Some("metals"), Some(4), false)))
        );
        assert_eq!(parse(&[], "mine 3"), Ok((None, mine(None, Some(3), false))));
        assert_eq!(parse(&["1", "mine"], "stop"), Ok((Some(1), Act::MineStop)));
        assert_eq!(parse(&[], "drill stop"), Ok((None, Act::MineStop)));
        let usage1 = MINE_USAGE.replace("{n}", "1");
        let usage = MINE_USAGE.replace("{n}", "<n>");
        for (switches, raw, want) in [
            (&["1", "mine"][..], "3 4", &usage1),
            (&["1", "mine"][..], "ice metal", &usage1),
            (&["1", "mine"][..], "0", &usage1),
            (&["1", "mine"][..], "100", &usage1),
            (&["1", "mine"][..], "go 3", &usage1),
            (&["1", "mine"][..], "stop go", &usage1),
            (&[][..], "mine stop now", &usage),
        ] {
            assert_eq!(
                parse(switches, raw),
                Err(want.clone()),
                "{switches:?} {raw:?}"
            );
        }
        assert_eq!(
            parse(&["mine", "drive"], ""),
            Err(TV_USAGE.to_owned()),
            "two act words"
        );
    }

    /// **The mining texts fill** (doc 20 cut SM.7; the brief's 3.6, reviewer F1): every template
    /// filled with the widest figures holds no newline and no unfilled `{`; and the lines doc
    /// 20 gives verbatim — §1's cycle line, "That block is under claim.", "Another rig is
    /// working that pocket.", §6's no-fit line for A4 (with the slot), and the blind sentence
    /// and its room clause filled with the shipped drill row's blind room — equal doc 20's
    /// words byte for byte.
    #[test]
    fn sm7_the_mine_texts_fill() {
        let fill = |template: &str| {
            [
                ("{n}", "4"),
                ("{mark}", "OWP-TV-04"),
                ("{addr}", "P256"),
                ("{class}", "rich"),
                ("{ore}", "strategic ore"),
                ("{cycles}", "8 cycles"),
                ("{yields}", "3/3/3/3/3/3/3/3"),
                ("{total}", "24"),
                ("{secs}", "28800"),
                ("{real}", ", about 1 h 20 min"),
                ("{hop0}", "12"),
                ("{hop1}", "12"),
                ("{cap}", "12"),
                ("{room}", "86"),
                ("{now}", "100"),
                ("{after}", "84"),
                ("{home}", "83"),
                ("{also}", " Also rich ice ore here: +tv/4 mine ice."),
                ("{arg}", " strategic 8"),
                ("{pit}", PIT_WORKED_OUT),
                ("{when}", "23:59 UTC (IC 2350-12-31 23:59; in 59 min)"),
                ("{k}", "8"),
                ("{t}", "24"),
                ("{hopper}", "12 of 12 t"),
                ("{reading}", "P256 rich metals, rich volatiles"),
                ("{lift}", "24 t strategic ore"),
                ("{why}", WHY_UNWORKED),
                ("{terrain}", "deep water"),
                ("{counter}", "Exchange Hall counter"),
                ("{port}", "Ceres Freeport"),
                ("{name}", "Salt Moth"),
                ("{m}", "99"),
                ("{fam}", " strategic"),
                ("{fit}", "7"),
                ("{ord}", "seventh"),
                ("{line}", LICENCE_LAPSED_FILLED),
                ("{stop}", RUN_STOP_CLAUSE),
                ("{c}", "8"),
                ("{word}", "strategic"),
            ]
            .iter()
            .fold(template.to_owned(), |text, (key, value)| {
                text.replace(key, value)
            })
        };
        for template in [
            MINE_PREVIEW,
            MINE_ALSO,
            MINE_PREVIEW_BLIND,
            MINE_FILED,
            CORE_FILED,
            CYCLE_DONE,
            RUN_DONE,
            RUN_WORKED_OUT,
            RUN_STOPPED,
            RUN_RIGHTS,
            CORED,
            CYCLE_FAILED,
            MINE_IN_BAY,
            MINE_RUNNING,
            MINE_NO_FOOTING,
            MINE_NO_LICENCE,
            MINE_FORFEIT,
            MINE_ENDED,
            MINE_CLAIMED,
            MINE_LOCKED,
            MINE_HOPPER_FULL,
            MINE_HOLD_FULL,
            MINE_WORKED_OUT,
            MINE_NO_FIT,
            FIT_HOPPER,
            FIT_HOLD,
            FIT_POCKET,
            FIT_CAP,
            MINE_BLIND_ROOM,
            MINE_BLIND_COUNT,
            MINE_NO_ORE,
            MINE_ALL_BARREN,
            MINE_CHANGED,
            MINE_USAGE,
            MINE_NOT_RUNNING,
            MINE_STOPPING,
            MINE_STOPPING_ALREADY,
            RUN_LINE,
            CORE_LINE,
            ONE_CYCLE,
            SOME_CYCLES,
            PIT_WORKED,
            PIT_WORKED_OUT,
            NOTHING_LIFTED,
            WHY_UNWORKED,
        ] {
            let text = fill(template);
            assert!(!text.contains('\n'), "{template}");
            assert!(!text.contains('{'), "{template}: {text}");
        }
        // Doc 20 §1's cycle line, verbatim.
        assert_eq!(
            CYCLE_DONE
                .replace("{n}", "1")
                .replace("{k}", "2")
                .replace("{cycles}", "4")
                .replace("{t}", "2")
                .replace("{ore}", &ore_words("metal_ore"))
                .replace("{hopper}", &hopper_text(4_000, 12_000)),
            "Rig 1, cycle 2 of 4: 2 t metal ore; hopper 4 of 12 t."
        );
        assert_eq!(MINE_CLAIMED, "That block is under claim.");
        assert_eq!(MINE_LOCKED, "Another rig is working that pocket.");
        // §6's no-fit line for A4, the slot inserted after `+tv` and the code spans plain.
        assert_eq!(
            MINE_NO_FIT
                .replace("{m}", "4")
                .replace("{why}", &FIT_HOPPER.replace("{ord}", "third"))
                .replace("{n}", "1")
                .replace("{fam}", "")
                .replace("{fit}", "3"),
            "4 cycles do not fit: the hopper is full after the third — +tv/1 mine 3, or +tv/1 \
             mine with no number."
        );
        // §6's blind sentence and its room clause, with the shipped drill row's blind room.
        let room = crate::definitions::tools::tools_table()
            .ok()
            .and_then(|tools| {
                tools
                    .values()
                    .find_map(crate::definitions::tools::ToolDef::drill)
                    .map(crate::surface::mine::blind_room_t)
            })
            .expect("the drill row");
        let blind = MINE_PREVIEW_BLIND.replace("{room}", &room.to_string());
        assert!(blind.starts_with("No reading here: the first cycle will tell. "));
        assert!(blind.contains("; 3 t of room needed to core an unread pocket; "));
        assert_eq!(
            MINE_BLIND_ROOM.replace("{room}", &room.to_string()),
            "3 t of room needed to core an unread pocket; the hopper or the hold has less."
        );
        assert_eq!(ore_words("carbon_ore"), "carbon ore");
    }

    /// A filled `LICENCE_LAPSED`, for the rights clause.
    const LICENCE_LAPSED_FILLED: &str =
        "Your licence lapsed at 05-21 14:00. +licence/buy takes a new one from today.";

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

    /// **The unloading grammar** (doc 20 cut SM.8; the brief's 3.5): `+tv[/<n>]/unload [<t>] [go]`
    /// as a switch or a first word, `<t>` decimal whole tonnes `1..=99`; `0`, `100`, a word, two
    /// counts, a signed or fractional count and a word after `go` are the unload's usage line
    /// (naming the slot, or `<n>`).
    #[test]
    fn sm8_the_unloading_grammar() {
        let parse = |switches: &[&str], raw: &str| -> Result<(Option<u8>, Act), String> {
            parse_parts(&|word| switches.contains(&word), raw).map_err(|e| e.to_string())
        };
        let unload = |qty: Option<u64>, go| Act::Unload { qty, go };
        assert_eq!(
            parse(&["1", "unload"], ""),
            Ok((Some(1), unload(None, false)))
        );
        assert_eq!(
            parse(&["1", "unload"], "go"),
            Ok((Some(1), unload(None, true)))
        );
        assert_eq!(
            parse(&["1"], "unload 3 go"),
            Ok((Some(1), unload(Some(3), true)))
        );
        assert_eq!(parse(&[], "unload 99"), Ok((None, unload(Some(99), false))));
        assert_eq!(
            parse(&["2", "unload"], "1"),
            Ok((Some(2), unload(Some(1), false)))
        );
        let usage1 = UNLOAD_USAGE.replace("{n}", "1");
        let usage = UNLOAD_USAGE.replace("{n}", "<n>");
        for (switches, raw, want) in [
            (&["1", "unload"][..], "0", &usage1),
            (&["1", "unload"][..], "100", &usage1),
            (&["1", "unload"][..], "x", &usage1),
            (&["1", "unload"][..], "3 4", &usage1),
            (&["1", "unload"][..], "-3", &usage1),
            (&["1", "unload"][..], "2.5", &usage1),
            (&["1", "unload"][..], "go 3", &usage1),
            (&["1", "unload"][..], "3 go now", &usage1),
            (&[][..], "unload all", &usage),
        ] {
            assert_eq!(
                parse(switches, raw),
                Err(want.clone()),
                "{switches:?} {raw:?}"
            );
        }
        assert_eq!(
            parse(&["unload", "dump"], ""),
            Err(TV_USAGE.to_owned()),
            "two act words"
        );
        assert!(TV_USAGE.contains("+tv[/<n>]/unload [<t>] [go]"));
        assert!(TV_HELP.contains("+tv/<n> unload [<t>] [go]"));
    }

    /// **The unloading texts fill** (doc 20 cut SM.8; the brief's 3.6): every template filled with
    /// the widest figures holds no newline and no unfilled `{`; the recover's unload clause reads in
    /// doc 20 §1 beat 9's shape ("then … into the hold, … of room"), and an empty hopper's recover
    /// preview is byte-for-byte SM.4b's (the clause empty); the departure's two hopper lines name a
    /// remedy that works where each is said.
    #[test]
    fn sm8_the_unload_texts_fill() {
        let fill = |template: &str| {
            [
                ("{n}", "4"),
                ("{mark}", "OWP-TV-04"),
                ("{load}", "12 t of Strategic ore and 12 t of Water ice ore"),
                ("{stays}", "12 t of Carbon ore and 12 t of Metal ore"),
                ("{name}", "Salt Moth"),
                ("{secs}", "600"),
                ("{real}", ", about 25 s"),
                ("{hop0}", "12"),
                ("{hop1}", "0"),
                ("{cap}", "12"),
                ("{room}", "86"),
                ("{arg}", " 99"),
                ("{when}", "23:59 UTC (IC 2350-12-31 23:59; in 25 s)"),
                ("{hopper}", "12 of 12 t"),
                ("{site}", "Ahuna Flank"),
                ("{q}", "99"),
                ("{fit}", "12"),
                ("{ore}", "Strategic ore"),
                ("{then}", ""),
                ("{t}", "12"),
            ]
            .iter()
            .fold(template.to_owned(), |text, (key, value)| {
                text.replace(key, value)
            })
        };
        for template in [
            RECOVER_THEN,
            RECOVER_THEN_PART,
            RECOVER_THEN_NONE,
            UNLOAD_PREVIEW,
            UNLOAD_STAYS,
            UNLOAD_FILED,
            UNLOADED,
            UNLOADED_NONE,
            UNLOADED_NONE_EMPTY,
            RECOVERED_UNLOADING,
            UNLOAD_UNDER_WAY,
            UNLOAD_NOT_SITE,
            UNLOAD_OUT,
            UNLOAD_NO_ROOM,
            UNLOAD_NO_FIT,
            UNLOAD_NO_STOCK,
            UNLOAD_CHANGED,
            UNLOAD_USAGE,
            WHY_LEFT_BAY,
            WHY_HOLD_REFUSED,
            WHY_MARKERS_FULL,
            WHY_MARKERS_QUEUED,
            UNLOAD_MARKERS,
            QUEUED_LINE,
            crate::port::voyage::HOPPER_LOADED,
            crate::port::voyage::HOPPER_LOADED_AWAY,
        ] {
            let text = fill(template);
            assert!(!text.contains('\n'), "{text}");
            assert!(!text.contains('{'), "{text}");
        }
        // §1 beat 9's shape: "…; hopper 12 of 12 t; then it unloads 12 t of Metal ore into the
        // hold, 62 t of room after. +tv/1 recover go brings it in."
        let then = RECOVER_THEN
            .replace("{load}", "12 t of Metal ore")
            .replace("{room}", "62");
        let preview = RECOVER_PREVIEW
            .replace("{n}", "1")
            .replace("{mark}", "OWP-TV-01")
            .replace("{ramp}", "stern ramp")
            .replace("{name}", "Salt Moth")
            .replace("{secs}", "300")
            .replace("{real}", "")
            .replace("{now}", "60")
            .replace("{after}", "60")
            .replace("{hopper}", "12 of 12 t")
            .replace("{then}", &then);
        assert!(
            preview.ends_with(
                "; hopper 12 of 12 t; then it unloads 12 t of Metal ore into the hold, 62 t of \
                 room after. +tv/1 recover go brings it in."
            ),
            "{preview}"
        );
        let empty = RECOVER_PREVIEW.replace("{then}", "");
        assert!(
            empty.ends_with("; hopper {hopper}. +tv/{n} recover go brings it in."),
            "an empty hopper's preview is SM.4b's"
        );
        assert!(crate::port::voyage::HOPPER_LOADED.contains("+tv/{n} unload moves it"));
        assert!(crate::port::voyage::HOPPER_LOADED.contains("+tv/{n} dump empties it"));
        assert!(crate::port::voyage::HOPPER_LOADED_AWAY.ends_with("staff must empty it."));
        assert_eq!(
            UNLOAD_NO_FIT
                .replace("{q}", "5")
                .replace("{n}", "1")
                .replace("{fit}", "3")
                .replace("{ore}", "Metal ore"),
            "5 t do not fit: rig 1 can unload 3 t of Metal ore now — +tv/1 unload 3, or +tv/1 \
             unload with no number."
        );
    }
    /// **The plan grammar** (doc 20 cut SM.11; the brief's 3.5 and D11): `+tv[/<n>]/plan <list>
    /// [family=<good>] [go]` as a switch or a first word, the list whole (commas and spaces kept
    /// for the compiler); `plan show`, `plan/show` and `plan clear`; `/show` only beside `plan`;
    /// every malformed form is the plan's usage line.
    #[test]
    fn sm11_the_plan_grammar() {
        let parse = |switches: &[&str], raw: &str| -> Result<(Option<u8>, Act), String> {
            parse_parts(&|word| switches.contains(&word), raw).map_err(|e| e.to_string())
        };
        let plan = |list: &str, family: Option<&str>, go| Act::Plan {
            list: list.to_owned(),
            family: family.map(str::to_owned),
            go,
        };
        assert_eq!(
            parse(&["1", "plan"], "C117,C118,D family=metal_ore"),
            Ok((Some(1), plan("C117,C118,D", Some("metal_ore"), false)))
        );
        assert_eq!(
            parse(&["1"], "plan C117,C118,D family=metal_ore go"),
            Ok((Some(1), plan("C117,C118,D", Some("metal_ore"), true)))
        );
        assert_eq!(
            parse(&[], "plan C117 D go"),
            Ok((None, plan("C117 D", None, true)))
        );
        assert_eq!(parse(&["1", "plan"], "show"), Ok((Some(1), Act::PlanShow)));
        assert_eq!(parse(&["1"], "plan show"), Ok((Some(1), Act::PlanShow)));
        assert_eq!(
            parse(&["1", "plan", "show"], ""),
            Ok((Some(1), Act::PlanShow)),
            "the switch form the allowlist names"
        );
        assert_eq!(parse(&["plan", "show"], ""), Ok((None, Act::PlanShow)));
        assert_eq!(
            parse(&["1", "plan"], "clear"),
            Ok((Some(1), Act::PlanClear))
        );
        assert_eq!(parse(&["1"], "plan clear"), Ok((Some(1), Act::PlanClear)));
        for (switches, raw) in [
            (&["1", "plan"][..], ""),
            (&["1", "plan"][..], "go"),
            (&["1", "plan"][..], "family=metal_ore"),
            (&["1", "plan"][..], "family= C117"),
            (&["1", "plan"][..], "C117 family=a family=b"),
            (&["1", "plan"][..], "C117 go D"),
            (&["1", "plan"][..], "show now"),
            (&["1", "plan"][..], "clear go"),
            (&["1", "plan", "show"][..], "C117 go"),
            (&["1", "plan", "show"][..], "show"),
        ] {
            assert_eq!(
                parse(switches, raw),
                Err(PLAN_USAGE.to_owned()),
                "{switches:?} {raw:?}"
            );
        }
        assert_eq!(
            parse(&["1", "show"], ""),
            Err(TV_USAGE.to_owned()),
            "/show alone"
        );
        assert_eq!(
            parse(&["1", "scan", "show"], ""),
            Err(TV_USAGE.to_owned()),
            "/show beside another act"
        );
        assert!(TV_USAGE.contains("+tv[/<n>]/plan "), "{TV_USAGE}");
        assert!(TV_HELP.contains("+tv/<n> plan "), "{TV_HELP}");
        assert!(ACT_WORDS.contains(&"plan"));
    }

    /// **The plan's texts fill** (doc 20 cut SM.11; the brief's 3.5): every template filled with
    /// the widest figures holds no newline and no unfilled `{`, and the §5 fog line stands byte
    /// for byte behind its address.
    #[test]
    fn sm11_the_plan_texts_fill() {
        let fill = |template: &str| {
            [
                ("{n}", "4"),
                ("{m}", "3"),
                ("{no}", "4294967295"),
                ("{mark}", "OWP-TV-04"),
                ("{site}", "Ahuna Flank"),
                ("{ore}", "Water ice ore"),
                ("{real}", ", about 3 real h"),
                ("{k}", "64"),
                (
                    "{words}",
                    "mine every pocket of block P that reads fair or better",
                ),
                ("{hours}", ", about 288 IC h"),
                ("{far}", ""),
                ("{h}", "288"),
                ("{name}", "Salt Moth"),
                ("{room}", "86"),
                ("{shared}", "; rig 3's plan draws on it too"),
                ("{list}", "C117,C118,D"),
                ("{when}", "23:59 UTC (IC 2350-12-31 23:59; in 25 s)"),
                ("{addr}", "C117"),
                ("{line}", "You have no reading there."),
                ("{family}", "Water ice ore"),
                ("{max}", "64"),
                ("{state}", "coming home to park"),
                ("{filed}", "IC 2350-12-31 23:59"),
                ("{deadline}", "IC 2351-01-12 23:59"),
                ("{t}", "4294967295"),
                ("{c}", "65535 cycles"),
                ("{trips}", "65535 trips"),
                ("{doing}", "its next step cannot start yet"),
                ("{at}", "IC 2351-01-12 23:59"),
                ("{why}", "its 288 IC hours ran out."),
                ("{steps}", "64"),
                ("{held}", "120"),
                ("{then}", IDLE_THEN),
                ("{ramp}", "Salt Moth's ramp"),
                ("{body}", "Ceres"),
            ]
            .iter()
            .fold(template.to_owned(), |text, (key, value)| {
                text.replace(key, value)
            })
        };
        for template in [
            PLAN_USAGE,
            PLAN_PREVIEW_HEAD,
            PLAN_PREVIEW_STEP,
            PLAN_STEP_HOURS,
            PLAN_PREVIEW_FAR,
            PLAN_PREVIEW_TAIL,
            PLAN_SHARED,
            PLAN_FILED,
            PLAN_REFUSED_AT,
            PLAN_TOO_FAR_LINE,
            PLAN_DARK_LINE,
            PLAN_BARREN,
            PLAN_TOO_MANY,
            PLAN_TWICE,
            PLAN_TOO_MANY_STEPS,
            PLAN_HOLD_FULL,
            PLAN_NOTHING_LEFT,
            PLAN_TOO_LONG,
            PLAN_ALREADY,
            PLAN_NONE,
            PLAN_CHANGED,
            PLAN_LIVE,
            PLAN_ALREADY_ENDED,
            PLAN_CLEARED,
            PLAN_DROPPED,
            PLAN_SHOW_HEAD,
            PLAN_SHOW_STEP,
            PLAN_SHOW_TALLY,
            PLAN_SHOW_NOW,
            PLAN_SHOW_ENDED,
            PLAN_LINE,
            PLAN_LINE_ENDED,
            PLAN_PASSED,
            PLAN_PARKING,
            PLAN_ENDED,
            PLAN_WAITS,
            IDLE_HOME,
            STRANDED,
            PLAN_REMEDY,
            END_HOLD_FULL,
            END_CHART_FULL,
            END_STRANDED,
            WAIT_HELM,
            WAIT_ASHORE,
            WAIT_CHARGING,
            PASS_BARREN,
        ] {
            let text = fill(template);
            assert!(
                !text.contains('{') && !text.contains('\n'),
                "{template}: {text}"
            );
        }
        for word in [
            STATE_LIVE,
            STATE_PARKING,
            STATE_ENDED,
            MARK_DONE,
            MARK_NOW,
            MARK_NEXT,
            MARK_PENDING,
            MARK_PASSED,
            END_WORKED_OUT,
            END_BOUND,
            END_LEASE_ENDED,
            END_LEASE_FORFEIT,
            END_DARK,
            END_CLEARED,
            WAIT_CAP,
            WAIT_COLD,
            WAIT_DARK,
            WAIT_BUSY,
            WAIT_NEXT,
            PASS_WORKED_OUT,
            PASS_LOCKED,
            PASS_NO_WAY,
            PASS_TOO_FAR,
            PASS_NOTHING_NEW,
            PLAN_DOING,
        ] {
            assert!(!word.is_empty() && !word.contains(['{', '\n']), "{word}");
        }
        assert_eq!(
            fill(PLAN_REFUSED_AT),
            "C117: You have no reading there.",
            "the §5 fog line, behind the address"
        );
        assert_eq!(
            fill(PLAN_ENDED),
            "Rig 4's plan at Ahuna Flank ended: its 288 IC hours ran out. It mined 4294967295 t of \
             Water ice ore in 65535 cycles over 65535 trips; the Salt Moth's hold holds 120 t, 86 t \
             of it unreserved."
        );
    }

    /// A record with plan 1 by the pilot 42 in `state`, whose last outcome landed at `last_ic`.
    fn parked_record(
        state: crate::surface::plan::FillState,
        last_ic: Option<i64>,
    ) -> crate::surface::RigRecord {
        use crate::surface::plan::{FillPlan, PLAN_BOUND_IC_SECS, PlanStep, PlanTally};
        let mut record = crate::surface::RigRecord::fresh_unread(ObjId(900));
        record.ordinal = 5;
        record.plans = 1;
        record.plan = Some(FillPlan {
            no: 1,
            author: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            site: CanonicalId::new("ahuna_port").expect("an id"),
            family: crate::surface::geology::OreFamily::Metals,
            filed_ic: 1_000,
            deadline_ic: 1_000 + PLAN_BOUND_IC_SECS,
            list: vec!["D".to_owned()],
            steps: vec![PlanStep::Block { letter: 'D' }],
            cursor: 0,
            passed: Vec::new(),
            state,
            seq: 1,
            tally: PlanTally::default(),
            notes: Vec::new(),
        });
        record.last = last_ic.map(|at_ic| crate::surface::excursion::Outcome {
            ordinal: 5,
            kind: crate::surface::excursion::ActKind::Launch,
            operator: ObjId(42),
            slot: 1,
            site: CanonicalId::new("ahuna_port").expect("an id"),
            at_ic,
            result: crate::surface::excursion::ActResult::Done,
            moved: std::collections::BTreeMap::new(),
            span: None,
            arrived: None,
            mined: None,
            stays: std::collections::BTreeMap::new(),
            by: None,
        });
        record
    }

    /// **`rig_parked` reads both cases** (doc 20 cut SM.11; the brief's 3.10, the reviewer's F3):
    /// in the bay idle → parked; in the bay with a live plan or a queued unload → not; out with
    /// nothing in flight and a plan ended after the last outcome → parked; out with an ended plan
    /// older than the last outcome → not; out, no plan, projection `Parked` → parked, `Out` or
    /// unread → not; no record and in the bay → parked, out → not.
    #[test]
    fn sm11_rig_parked_reads_both_cases() {
        use crate::surface::TvState;
        use crate::surface::plan::{FillState, PlanEnd};
        let pilot = Some(ObjId(42));
        let live = parked_record(FillState::Live, Some(2_000));
        let ended = |at_ic| {
            parked_record(
                FillState::Ended {
                    why: PlanEnd::HoldFull,
                    at_ic,
                },
                Some(2_000),
            )
        };
        assert!(rig_parked(None, pilot, true, None), "no record, in the bay");
        assert!(!rig_parked(None, pilot, false, None), "no record, out");
        let idle = crate::surface::RigRecord::fresh_unread(ObjId(900));
        assert!(
            rig_parked(Some(&idle), pilot, true, None),
            "idle in the bay"
        );
        assert!(
            !rig_parked(Some(&live), pilot, true, None),
            "a live plan in the bay"
        );
        assert!(
            rig_parked(Some(&live), Some(ObjId(43)), true, None),
            "an orphaned plan in the bay holds nothing"
        );
        let mut unloading = ended(3_000);
        unloading.queued = Some(crate::surface::excursion::Queued {
            kind: crate::surface::excursion::ActKind::Unload,
            operator: ObjId(42),
            hull: ObjId(500),
            slot: 1,
            site: CanonicalId::new("ahuna_port").expect("an id"),
            after: 5,
            at_ic: 2_000,
            excursion: None,
        });
        assert!(
            !rig_parked(Some(&unloading), pilot, true, None),
            "a queued unload"
        );
        assert!(
            rig_parked(Some(&ended(3_000)), pilot, true, None),
            "an ended plan in the bay"
        );
        assert!(
            rig_parked(Some(&ended(3_000)), pilot, false, None),
            "ended after the last outcome"
        );
        assert!(
            rig_parked(Some(&ended(2_000)), pilot, false, None),
            "ended at the last outcome"
        );
        assert!(
            !rig_parked(Some(&ended(1_500)), pilot, false, None),
            "acted since it ended"
        );
        let mut planless = live.clone();
        planless.plan = None;
        assert!(
            rig_parked(Some(&planless), pilot, false, Some(TvState::Parked)),
            "stranded with no plan"
        );
        assert!(
            !rig_parked(Some(&planless), pilot, false, Some(TvState::Out)),
            "out"
        );
        assert!(
            !rig_parked(Some(&planless), pilot, false, None),
            "an unread projection"
        );
        assert!(
            !rig_parked(Some(&live), pilot, false, Some(TvState::Out)),
            "a live plan out"
        );
    }

    /// **Doc 20 cut SM.12b** (the lead's ruling 2, D4): a house view is the lease's shape and never
    /// a stored lease — `check` refuses it and any record whose lease is 0; it is live, and its
    /// economy lease reads live with no economy at all.
    #[test]
    fn sm12b_a_house_view_is_never_a_stored_lease() {
        let view = TvLease::house_view(ObjId(900), "OWP-YT-01", ObjId(77), ObjId(500), 1);
        assert!(view.is_house());
        assert!(view.live());
        assert_eq!(
            (view.lease, view.pilot, view.hull, view.slot, view.status),
            (
                TvLease::HOUSE_LEASE,
                ObjId(77),
                ObjId(500),
                1,
                TvLeaseStatus::Active
            )
        );
        assert_eq!(view.check(), Err("lease must be at least 1".to_owned()));
        assert_eq!(
            TvLease {
                lease: 0,
                ..sample()
            }
            .check(),
            Err("lease must be at least 1".to_owned())
        );
        assert!(!sample().is_house());
        assert_eq!(sample().check(), Ok(()));
        let w = World::new_minimal();
        assert!(rig_lease_live(&w, &view));
        assert!(!rig_lease_live(&w, &sample()));
    }
}
