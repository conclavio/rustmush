//! **The Freeport Registry's mining claims** (doc 20 cut SM.10; §1 beat 10, §9.2–§9.6, §10,
//! §11's Claims row; D-20-12; the SM.10 brief's lead rulings 1–14 and decisions D1–D20).
//!
//! A licensed pilot whose rig stands on an open block they have read **by hand** stakes it:
//! `+claim/stake` previews (their own readings of the block, the soundings the site heard on it
//! since it last opened before theirs, any cooling term that binds them, the fee) and commits
//! nothing; `+claim/stake go` checks their funds, offers and escrows a **Public fee contract**
//! (the licence's shape, into the Registry's account, left `Offered`), writes one bounded RAM
//! `ClaimOrders` entry and submits **ONE** `industry_claim_tenure` — industry's own verb,
//! admitted only for the ONE server-authored `mush_industry::TenureAct` the composed authority
//! (`crate::composition::industry_seams::SurfaceTenure`) vouches for from that order. The bundle
//! files a claim over every opened deposit of the block (possibly none) under the site's
//! issuer, and stages tacsim's own block record ([`crate::surface::claims::BlockTenure`]) in
//! the same bundle (`stage`). The site clerk accepts the fee for the Registry once the act has
//! landed (`crate::port::site::claims_step`), the economy settles it and the chronicle records
//! it (`CanonEvidenceBridge`).
//!
//! `+claim/renew` extends a live claim in place once the term's worked floor is met (on a
//! papered block's first term the Registrar's 100 t, and the renewal suspends the paper);
//! `+claim/release` lapses it by choice, with the cooling term; `+claim/contest` takes a
//! claim on the priority of a loud sounding the site heard after the block last opened and
//! before the holder's reading; `+claim/list` and `+registry <site>` are pure reads of the
//! block records. **Nothing printed or charged reads how many deposits a block holds** (lead
//! ruling 1; SM.6's D14): every public line reads the block record, and the fee is per block.
//!
//! `plan_claim` is the one rule the command and the authority share: the command plans in
//! `Phase::Preview` / `Phase::Go`, the authority re-plans the order in `Phase::Offer` at the
//! offer and offers nothing unless the plan's
//! [`PlanCore`](crate::port::claims::PlanCore) is unchanged (reviewer F8). Funds are read only
//! by the command (reviewer F1); the offer verifies the order's own fee contract.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ClockStamp, CmdError, Ctx, Engine, ObjId, World};
use mush_economy::{
    AcceptOutcome, Account, AccountId, Amount, ConsiderationSpec, ContractError, ContractId,
    ContractRoles, ContractSpec, ContractState, EconomicClock, EconomicDeadline,
    EconomicVisibility, EconomyError, EconomyStore, EvidenceKind, Expiry, OutcomeTerms,
    PayoutShare, Share, TypedCondition,
};
use mush_industry::{ClaimRecord, ClaimRev, OreDepositId, TenureAct, TenureKind, TenureOutcome};
use mush_org::{OrgId, PrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::port::apply::{credit_scale, minor_of};
use crate::port::coop::{money_minor, name_of, say};
use crate::port::licence::ic_stamp;
use crate::port::site_map::{EngineReads, SurfaceReads};
use crate::surface::claims::{
    BlockClaim, BlockKey, BlockPaper, BlockTenure, PaperState, RefundOwed,
};
use crate::surface::geology::OreFamily;
use crate::surface::tenure::{
    CLAIM_FEE_WINDOW_SECS, CLAIM_ORDER_TTL_IC_SECS, CLAIM_TERM_IC_SECS, CONTEST_WINDOW_IC_SECS,
    COOLING_TERM_IC_SECS, MAX_HEARD_SCAN_ROWS, MAX_LIVE_CLAIMS_PER_ACCOUNT, REGISTRAR_WORKED_T,
    contest_open, evidence_at, heard_before, heard_count, renewal_floor, worked_this_term,
};

// ---- the texts (the planner's wording for the owner, OC10) --------------------------------

/// The verb.
pub const CLAIM_VERB: &str = "+claim";
/// The registry's verb.
pub const REGISTRY_VERB: &str = "+registry";
/// `+claim`'s help.
pub const CLAIM_HELP: &str = "+claim  --  The Freeport Registry's mining claims, a block at a time. +claim/stake [<rig>] [<family>] [go] from a rig standing in an open block you have read yourself; +claim/renew [<site>] <block> [go] once the term's worked floor is met; +claim/release [<site>] <block> [go]; +claim/contest <block> [go] with a sounding the site heard first; +claim/list. Bare, each previews and commits nothing; go commits exactly the preview.";
/// A bad shape.
pub const CLAIM_USAGE: &str = "Usage: +claim/stake [<rig>] [<family>] [go], +claim/renew [<site>] <block> [go], +claim/release [<site>] <block> [go], +claim/contest <block> [go], +claim/list";
/// The stake's preview: `{block}`, `{readings}`, `{heard}`, `{stake}`, `{term}`, `{total}`.
pub const STAKE_PREVIEW: &str = "Block {block}, your readings: {readings}; soundings heard on this block since it last opened, before yours: {heard}; {stake} now and {term} a term ({total} today).";
/// The declared family's clause: `{family}`.
pub const DECLARED_CLAUSE: &str = " You declare {family}.";
/// The paper's clause on a stake: `{paper}`, `{registrar}`.
pub const PAPER_CLAUSE: &str = " The {paper}'s paper stands over this block: work {registrar} t in your first term and the Registrar suspends it when you renew.";
/// The stake filed: `{block}`, `{site}`, `{total}`.
pub const STAKE_FILED: &str = "You stake block {block} at {site}; {total} is held until the Registry records it, and comes back within the hour if it does not.";
/// The stake recorded (the clerk's tell): `{issuer}`, `{block}`, `{site}`, `{until}`.
pub const STAKE_RECORDED: &str =
    "The {issuer} records your claim on block {block} at {site} until {until}.";
/// Commons ground.
pub const STAKE_COMMONS: &str = "Commons ground is never staked.";
/// The actor's own block: `{block}`.
pub const STAKE_YOURS: &str = "Block {block} is already yours.";
/// A past-term claim the clerk has not lapsed yet: `{block}`.
pub const STAKE_LAPSING: &str =
    "Block {block}'s claim is lapsing; it reopens when the Registry records the lapse.";
/// The cooling term: `{block}`, `{until}`.
pub const STAKE_COOLING: &str = "Block {block}'s last claim lapsed from you or your account; the Registry takes no stake on it from you before {until}.";
/// No hand-made reading: `{block}`.
pub const STAKE_NO_EVIDENCE: &str = "You have no reading of your own of block {block} since it last opened: scan, sound or core it first.";
/// The rig in the bay: `{n}`.
pub const STAKE_IN_BAY: &str = "Rig {n} is in the bay; drive it onto the block you would stake.";
/// Two rigs and none named: `{slots}`.
pub const WHICH_RIG_STAKE: &str = "Which rig? +claim/stake <n>, with n {slots}.";
/// The two-claim cap: `{max}`.
pub const CLAIM_CAP: &str =
    "Your account holds or is filing {max} claims, the most one account may; release one first.";
/// Short of funds: `{total}`, `{available}`.
pub const CLAIM_FUNDS: &str = "That costs {total}; you have {available}.";
/// The holder's fee not yet recorded: `{block}`, `{site}`.
pub const CLAIM_PENDING: &str =
    "The Registry has not recorded block {block} at {site} yet; try again shortly.";
/// Not the actor's: `{block}`, `{site}`.
pub const CLAIM_NOT_YOURS: &str = "You hold no claim on block {block} at {site}.";
/// More than one claim and none named.
pub const CLAIM_WHICH: &str =
    "You hold more than one claim; name the block: +claim/<renew|release> [<site>] <block>.";
/// The fee or the act could not be taken.
pub const CLAIMS_FAILED: &str = "The Registry could not take that; nothing was paid.";
/// The books cannot be read: `{site}`.
pub const CLAIMS_DARK: &str =
    "The Registry's books for {site} cannot be read right now; nothing changes.";
/// The renewal's preview: `{block}`, `{site}`, `{worked}`, `{floor}`, `{fee}`, `{until}`.
pub const RENEW_PREVIEW: &str =
    "Block {block} at {site}: worked {worked} of {floor} t this term; {fee} renews it to {until}.";
/// The Registrar's clause on a renewal: `{paper}`.
pub const REGISTRAR_CLAUSE: &str = " The Registrar suspends the {paper}'s paper with this renewal.";
/// Short of the floor: `{block}`, `{site}`, `{worked}`, `{floor}`, `{until}`.
pub const RENEW_SHORT: &str = "Block {block} at {site}: worked {worked} of {floor} t this term, so it cannot be renewed; it lapses at {until}.";
/// Short of the Registrar's floor: `{block}`, `{site}`, `{worked}`, `{floor}`, `{paper}`,
/// `{until}`.
pub const RENEW_PAPER_SHORT: &str = "Block {block} at {site}: worked {worked} of {floor} t this term; the Registrar honours only worked ground, so the {paper}'s paper stands and your claim lapses at {until}.";
/// The renewal filed: `{block}`, `{site}`, `{fee}`.
pub const RENEW_FILED: &str =
    "You renew block {block} at {site}; {fee} is held until the Registry records it.";
/// The renewal recorded: `{issuer}`, `{block}`, `{site}`, `{until}`.
pub const RENEW_RECORDED: &str =
    "The {issuer} renews your claim on block {block} at {site} until {until}.";
/// The Registrar's tell after a renewal: `{paper}`.
pub const REGISTRAR_TOLD: &str = " It suspends the {paper}'s paper there: the ground is worked.";
/// The release's preview: `{block}`, `{site}`, `{until}`.
pub const RELEASE_PREVIEW: &str = "Releasing block {block} at {site} ends your claim now; the Registry takes no stake on it from you or your account before {until}.";
/// The release filed: `{block}`, `{site}`.
pub const RELEASE_FILED: &str = "You release block {block} at {site}.";
/// The lapse told: `{block}`, `{site}`, `{until}`.
pub const LAPSED_TOLD: &str = "Your claim on block {block} at {site} has lapsed; the Registry takes no stake on it from you or your account before {until}.";
/// A fee cancelled after landing: `{block}`, `{site}`.
pub const UNPAID_LAPSED: &str =
    "Your fee for block {block} at {site} was withdrawn, so the Registry lapses the claim.";
/// A fee expired unaccepted after landing (an outage; no cooling term, OC16): `{block}`,
/// `{site}`.
pub const EXPIRED_LAPSED: &str = "Your fee for block {block} at {site} lapsed unpaid, so the Registry lapses the claim; you may stake it again.";
/// The contest's preview: `{block}`, `{site}`, `{at}`, `{holder}`, `{discovered}`, `{total}`,
/// `{until}`.
pub const CONTEST_PREVIEW: &str = "Block {block} at {site}: the site heard your sounding at {at}, before {holder}'s claim rests ({discovered}); {total} takes the claim to {until}, and {holder}'s last term fee goes back to them.";
/// No heard sounding: `{block}`.
pub const CONTEST_NO_EVIDENCE: &str = "You have no sounding of block {block} the site heard since it last opened and before its holder's reading.";
/// The window shut: `{block}`, `{days}`.
pub const CONTEST_SHUT: &str = "Block {block}'s claim was registered more than {days} IC days ago; it can no longer be contested.";
/// No claim to contest: `{block}`.
pub const CONTEST_OPEN: &str = "Block {block} is not under claim; stake it instead.";
/// An org's: `{block}`, `{org}`.
pub const CONTEST_HELD: &str = "Block {block} is held by {org}; it cannot be contested.";
/// The contest filed: `{block}`, `{site}`, `{total}`.
pub const CONTEST_FILED: &str =
    "You contest block {block} at {site}; {total} is held until the Registry records it.";
/// The contest recorded: `{issuer}`, `{block}`, `{site}`, `{until}`.
pub const CONTEST_RECORDED: &str = "The {issuer} passes block {block} at {site} to you on your sounding's priority, until {until}.";
/// The loser told: `{contester}`, `{block}`, `{site}`.
pub const CONTEST_LOST: &str = "{contester}'s heard sounding predates your claim on block {block} at {site}; the Registry passes the claim to them and returns your last term fee.";
/// The loser told when their fee was never paid: `{contester}`, `{block}`, `{site}`.
pub const CONTEST_LOST_UNPAID: &str = "{contester}'s heard sounding predates your claim on block {block} at {site}; the Registry passes the claim to them.";
/// A block of the viewer's own.
pub const TENURE_YOURS: &str = "your claim";
/// A block of another character's: `{holder}`.
pub const TENURE_CLAIMED: &str = "claimed by {holder}";
/// `+registry`'s help.
pub const REGISTRY_HELP: &str = "+registry <site>  --  The Freeport Registry's claims at a site: each claimed block's holder, term, epoch, declared family, first reading, tonnes worked against the term's floor and contest state, and the paper standing over it.";
/// A bad shape.
pub const REGISTRY_USAGE: &str = "Usage: +registry <site>";
/// No such site: `{arg}`.
pub const REGISTRY_UNKNOWN: &str = "There is no Registry site called {arg}.";
/// The books cannot be read: `{site}`.
pub const REGISTRY_DARK: &str = "The Registry's books for {site} cannot be read right now.";
/// The head: `{issuer}`, `{site}`.
pub const REGISTRY_HEAD: &str = "The {issuer}'s claims at {site}:";
/// A player's claim: `{block}`, `{holder}`, `{until}`, `{epoch}`, `{declared}`,
/// `{discovered}`, `{worked}`, `{floor}`, `{contest}`.
pub const REGISTRY_CLAIM: &str = "  Block {block}: {holder}; term to {until}; epoch {epoch}; {declared}; first reading {discovered}; worked {worked} of {floor} t; {contest}.";
/// A past-term claim: `{block}`, `{holder}`, `{until}`.
pub const REGISTRY_LAPSING: &str = "  Block {block}: {holder}; term ended {until}, lapsing.";
/// An org's: `{block}`, `{org}`.
pub const REGISTRY_HELD: &str = "  Block {block}: held by {org}.";
/// A paper: `{block}`, `{paper}`, `{state}`.
pub const REGISTRY_PAPER: &str = "  Block {block}: the {paper}'s paper, {state}.";
/// The tail: `{open}`, `{commons}`.
pub const REGISTRY_TAIL: &str = "  {open} open, {commons} commons, unclaimed.";
/// No block under claim.
pub const REGISTRY_NONE: &str = "  No block is under claim.";
/// `+claim/list`'s head: `{n}`, `{max}`.
pub const CLAIM_LIST_HEAD: &str = "Your claims ({n} of {max}):";
/// One row: `{site}`, `{block}`, `{until}`, `{worked}`, `{floor}`.
pub const CLAIM_LIST_ROW: &str =
    "  {site} block {block}: term to {until}; worked {worked} of {floor} t.";
/// None held.
pub const CLAIM_LIST_NONE: &str = "You hold no claims.";
/// The canon line of a lapse (public, in the chronicle): `{holder}`, `{block}`, `{site}`.
pub const LAPSE_CANON_LINE: &str = "{holder}'s claim on block {block} at {site} lapsed.";
/// The canon kind of a lapse.
pub const KIND_CLAIM_LAPSED: &str = "tacsim.claim.lapsed";
/// The fee contract's name.
pub const CLAIM_FEE_NAME: &str = "Freeport Registry claim fee";
/// The receipt of the act's outcome (never a count, a stamp or a reserve).
pub const TENURE_SUMMARY: &str = "Tenure recorded.";
/// An IC day, seconds.
const IC_DAY: i64 = 86_400;

// ---- the Registry, the sites and the blocks -------------------------------------------------

/// **The Registry as the claim acts read it** (lead ruling 3): the licence port's issuer org, its
/// principal and account, its name, and the two claim fees in minor units.
#[derive(Clone, Debug)]
pub(crate) struct RegistryInfo {
    /// The issuer as a principal.
    pub principal: PrincipalId,
    /// Its scoped `tacsim.credit` account.
    pub account: AccountId,
    /// Its name (`Freeport Registry`).
    pub name: String,
    /// The stake fee, minor units.
    pub stake_minor: i64,
    /// The term fee, minor units.
    pub term_minor: i64,
}

/// The lane's Registry, when its licence port, org row and account are present.
#[must_use]
pub(crate) fn registry_info(w: &World) -> Option<RegistryInfo> {
    let (def, map, licence) = crate::port::licence::licence_port(w)?;
    let row = map.orgs.get(&licence.org)?;
    let scale = credit_scale(w);
    Some(RegistryInfo {
        principal: PrincipalId::Organization(OrgId(row.org)),
        account: AccountId(row.account?),
        name: def
            .orgs
            .iter()
            .find(|org| org.id == licence.org)
            .map_or_else(|| licence.org.to_string(), |org| org.name.clone()),
        stake_minor: minor_of(licence.claim_stake_major, scale)?,
        term_minor: minor_of(licence.claim_term_major, scale)?,
    })
}

/// A lane site's entry and name, by entry.
fn site_name_of(w: &World, entry: &CanonicalId) -> String {
    crate::surface::chart::lane_sites(w)
        .into_iter()
        .find(|site| &site.entry == entry)
        .map_or_else(|| entry.to_string(), |site| site.name)
}

/// **A block's place Thing** (the site's port map row).
#[must_use]
pub(crate) fn block_place(w: &World, entry: &CanonicalId, letter: char) -> Option<ObjId> {
    w.module_state::<crate::store::TacsimStore>()?
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .find(|map| &map.entry == entry)?
        .site
        .as_ref()?
        .blocks
        .get(letter.to_string().as_str())
        .map(|id| ObjId(*id))
}

/// **The block's four receipt-derived deposit ids** (SM.6's `receipt`), whether opened or not.
#[must_use]
pub(crate) fn block_deposit_ids(entry: &CanonicalId, letter: char) -> Vec<OreDepositId> {
    OreFamily::ALL
        .into_iter()
        .filter_map(|family| {
            mush_industry::derive_system_deposit_id(&crate::surface::opening::receipt(
                entry, letter, family,
            ))
            .ok()
        })
        .collect()
}

/// **The block's opened deposits** (lead ruling 1): its receipt-derived ids that `reads.deposit`
/// finds — possibly none. Never printed, priced or counted aloud.
///
/// # Errors
///
/// A deposit that cannot be read.
pub(crate) fn opened_deposits(
    reads: &dyn SurfaceReads,
    entry: &CanonicalId,
    letter: char,
) -> Result<Vec<OreDepositId>, String> {
    let mut out = Vec::new();
    for id in block_deposit_ids(entry, letter) {
        if reads.deposit(id)?.is_some() {
            out.push(id);
        }
    }
    Ok(out)
}

/// **Σ `drawn_t` over the block's cursors now** (lead ruling 4): an absent cursor is 0.
///
/// # Errors
///
/// A cursor that cannot be read or does not add.
pub(crate) fn drawn_now(
    reads: &dyn SurfaceReads,
    entry: &CanonicalId,
    letter: char,
) -> Result<u64, String> {
    let mut sum = 0_u64;
    for id in block_deposit_ids(entry, letter) {
        if let Some(cursor) = reads.cursor(id)? {
            sum = cursor
                .drawn_total()
                .and_then(|drawn| sum.checked_add(drawn))
                .ok_or_else(|| "a block's worked tonnes overflow".to_owned())?;
        }
    }
    Ok(sum)
}

/// The claims of `issuer` over `deposits` (each with its deposit).
fn issuer_claims(
    reads: &dyn SurfaceReads,
    deposits: &[OreDepositId],
    issuer: &PrincipalId,
) -> Result<Vec<(OreDepositId, ClaimRecord)>, String> {
    let mut out = Vec::new();
    for id in deposits {
        for claim in reads.claims(*id)? {
            if claim.issuer() == issuer {
                out.push((*id, claim));
            }
        }
    }
    Ok(out)
}

/// A claim at its revision.
fn rev(claim: &ClaimRecord) -> ClaimRev {
    ClaimRev {
        claim: claim.id(),
        revision: claim.revision(),
    }
}

/// **A holder's name** as a player reads it: a character's name; an org's authored lane name
/// (its id without one); "an organization" when it names no lane org.
#[must_use]
pub(crate) fn holder_name(w: &World, holder: &PrincipalId) -> String {
    match holder {
        PrincipalId::Object(obj) => name_of(w, *obj),
        PrincipalId::Organization(org) => {
            let Some(store) = w.module_state::<crate::store::TacsimStore>() else {
                return "an organization".to_owned();
            };
            match (
                lane_org_id(w, org),
                crate::port::readiness::report_ports(store, w).0,
            ) {
                (Some(id), Some(pack)) => crate::port::site::org_name(pack, &id),
                (Some(id), None) => id.to_string(),
                (None, _) => "an organization".to_owned(),
            }
        }
        PrincipalId::System(_) => "the Registry".to_owned(),
    }
}

/// The lane org id an org principal is (its port map row), if the lane authors it.
fn lane_org_id(w: &World, org: &OrgId) -> Option<CanonicalId> {
    w.module_state::<crate::store::TacsimStore>()?
        .port_maps()
        .filter(|map| map.pack.as_str() == crate::definitions::content::LANE_PACK)
        .find_map(|map| {
            map.orgs
                .iter()
                .find(|(_, row)| row.org == org.0)
                .map(|(id, _)| id.clone())
        })
}

/// **A site's tenure in words, from its block records** (lead note 0a: `+chart`'s
/// `TENURE_LINE` and the ground chart's `ground.sites[].tenure` read [`BlockTenure`]):
/// `crate::port::site::tenure_parts`'s groups in its order — commons, open, then `{n} claimed`
/// (live player claims), then each org's `{n} held by {org}` — each block read by
/// [`block_tenure_now`]'s rule: a **live** claim speaks (a player's is "claimed", an org's is
/// held by that org); anything else — no record, no claim, a lapsed or past-term one (SM.7's
/// R20), an unreadable record, an org no lane row names — is the authored word. A seeded
/// holding names the org the lane authored for it, so Occator Rim reads exactly as authored
/// until a player stakes there. Never a deposit count: a block is one word whatever it holds.
#[must_use]
pub(crate) fn tenure_parts_now(
    w: &World,
    reads: &dyn SurfaceReads,
    pack: &crate::pack::RulesPack,
    entry: &CanonicalId,
    site: &crate::port::def::SiteDef,
    now: i64,
) -> String {
    use crate::port::def::Tenure;
    let mut commons = 0_usize;
    let mut open = 0_usize;
    let mut claimed = 0_usize;
    let mut held_ids: Vec<CanonicalId> = Vec::new();
    for letter in crate::port::def::BLOCK_LETTERS {
        let live = reads
            .block_tenure(&BlockKey {
                site: entry.clone(),
                letter,
            })
            .ok()
            .flatten()
            .and_then(|record| record.claim)
            .filter(|claim| claim.live(now));
        match live.as_ref().map(|claim| &claim.holder) {
            Some(PrincipalId::Object(_)) => {
                claimed = claimed.saturating_add(1);
                continue;
            }
            Some(PrincipalId::Organization(org)) => {
                if let Some(id) = lane_org_id(w, org) {
                    held_ids.push(id);
                    continue;
                }
            }
            Some(PrincipalId::System(_)) | None => {}
        }
        let block = site.block(letter);
        match (block.tenure, block.holder) {
            (Tenure::Commons, _) => commons = commons.saturating_add(1),
            (Tenure::Open, _) | (Tenure::Held, None) => open = open.saturating_add(1),
            (Tenure::Held, Some(holder)) => held_ids.push(holder.clone()),
        }
    }
    let mut held: BTreeMap<&CanonicalId, usize> = BTreeMap::new();
    for id in &held_ids {
        let n = held.entry(id).or_insert(0);
        *n = n.saturating_add(1);
    }
    crate::port::site::tenure_words(pack, commons, open, claimed, &held)
}

/// An org's name by its lane id.
fn org_word(w: &World, id: &CanonicalId) -> String {
    let pack = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| crate::port::readiness::report_ports(store, w).0);
    pack.map_or_else(
        || id.to_string(),
        |pack| crate::port::site::org_name(pack, id),
    )
}

/// **A fee contract's state** in the economy's snapshot, if it exists.
fn contract_state(w: &World, id: u64) -> Option<ContractState> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .contract(ContractId(id))
        .map(|contract| contract.state)
}

/// **The fee a landed claim's contract owes** (D4, OC1): a renewal's (its term began after its
/// registration) is the term fee; a stake's or a contest's is the stake and the term.
#[must_use]
pub(crate) fn fee_due(claim: &BlockClaim, registry: &RegistryInfo) -> Option<i64> {
    if claim.term_start_ic > claim.registered_ic {
        Some(registry.term_minor)
    } else {
        registry.stake_minor.checked_add(registry.term_minor)
    }
}

/// **Whether a fee contract still carries the claim fee's own terms** ([`claim_fee_spec`]'s
/// shape; the fixer's round on SM.10's review, correctness F1/F2): `Offered`, issued by `payer`
/// and accepted by it on the surface as it stands (its consent intact); **Public** (the
/// chronicle's line, lead ruling 3); the payer and the Registry its only parties; no scope, no
/// adjudicator, no deliverable, `Before` conditions only;
/// one consideration of exactly `fee_minor` from the payer; one outcome paying exactly
/// `fee_minor` to the Registry's account; `Forfeit` to it on expiry. **An issuer may amend an
/// `Offered` contract back to `Draft`, keeping its own escrow, and re-offer other terms**
/// (`@contract/amend`, mush-economy's consent by authorship), so the offer and the clerk's
/// acceptance read the terms, never only the state and the sum.
#[must_use]
pub(crate) fn fee_terms_hold(
    contract: &mush_economy::EconomicContract,
    payer: &PrincipalId,
    registry: &RegistryInfo,
    fee_minor: i64,
) -> bool {
    let consent = &contract.consent;
    let terms = &consent.terms;
    let fee = Amount::from_minor(fee_minor);
    let one_payout = |outcome: &OutcomeTerms| {
        outcome.asset_consequences.is_empty()
            && matches!(
                outcome.payouts.as_slice(),
                [payout] if payout.to == registry.account && payout.share == Share::Exact(fee)
            )
    };
    contract.state == ContractState::Offered
        && contract.issuer == *payer
        && contract.accepted_by(payer)
        && consent.visibility == EconomicVisibility::Public
        && consent.scope.is_none()
        && consent.adjudicator.is_none()
        && consent.parties.len() == 2
        && consent.parties.contains(payer)
        && consent.parties.contains(&registry.principal)
        && consent
            .conditions
            .iter()
            .all(|condition| matches!(condition, TypedCondition::Before { .. }))
        && matches!(
            terms.consideration.as_slice(),
            [row] if row.from == *payer && row.amount == fee
        )
        && terms.deliverables.is_empty()
        && terms.split.is_empty()
        && terms.on_expiry
            == (Expiry::Forfeit {
                to: registry.account,
            })
        && matches!(terms.outcomes.as_slice(), [outcome] if one_payout(outcome))
}

/// **Why a landed claim's fee will never be paid** (D19, OC16): the cause the site clerk's
/// unpaid lapse carries, which decides its cooling term and its told line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FeeUnpaid {
    /// Withdrawn by its holder (reviewer F2a; the fixer's round, correctness F1/F2): the lapse
    /// cools the holder for a term (D19) and tells [`UNPAID_LAPSED`].
    Withdrawn,
    /// Expired before the Registry accepted it — an outage past the fee's real hour, the
    /// escrow refunded by the economy's own expiry (C17's path). No fault of the holder's (the
    /// lead's ruling on R1, OC16): the lapse cools nobody, the block reopens at once, and it
    /// tells [`EXPIRED_LAPSED`]. A fee the Registry **accepted** before it expired is not this:
    /// it was forfeit to the Registry ([`fee_forfeit`]), so the claim is paid.
    Expired,
}

/// **A fee forfeit to the Registry** (the lead's ruling on R1, OC16): `Expired` after the
/// Registry accepted it — an outage between the clerk's acceptance and the settlement, so the
/// deadline passed on a bound contract and the economy paid its escrow to the Registry's
/// account (`Expiry::Forfeit`, which forfeits only a fully accepted contract and refunds any
/// other). The Registry holds the fee as surely as a settlement would have paid it: the claim
/// stands paid, its acts proceed, and nothing lapses it unpaid.
#[must_use]
pub(crate) fn fee_forfeit(contract: &mush_economy::EconomicContract) -> bool {
    contract.state == ContractState::Expired && contract.fully_accepted()
}

/// **A landed claim's fee that will never be paid** (reviewer F2a; the fixer's round,
/// correctness F1/F2; the lead's ruling on R1, OC16), or `None` while it is paid (forfeit to
/// the Registry included, [`fee_forfeit`]), being paid or still live: [`FeeUnpaid::Withdrawn`]
/// for one `Cancelled`, amended back to `Draft` (the author's escrow kept, the Registry's
/// acceptance impossible) or `Offered` again on terms that are no longer the claim fee's
/// ([`fee_terms_hold`]); [`FeeUnpaid::Expired`] for one that expired unaccepted. The clerk
/// lapses either unpaid at its next visit (D19); until then the claim's acts answer
/// [`CLAIM_PENDING`]. A withdrawn fee is caught at the clerk's next fire, long before its hour
/// runs out, so only an outage leaves an `Expired` one.
#[must_use]
pub(crate) fn fee_unpaid(
    w: &World,
    claim: &BlockClaim,
    registry: &RegistryInfo,
) -> Option<FeeUnpaid> {
    let economy = w.module_state::<EconomyStore>()?;
    let contract = claim
        .fee
        .and_then(|id| economy.snapshot().contract(ContractId(id)))?;
    match contract.state {
        ContractState::Cancelled | ContractState::Draft => Some(FeeUnpaid::Withdrawn),
        ContractState::Offered => (!fee_due(claim, registry)
            .is_some_and(|due| fee_terms_hold(contract, &claim.holder, registry, due)))
        .then_some(FeeUnpaid::Withdrawn),
        ContractState::Expired => (!fee_forfeit(contract)).then_some(FeeUnpaid::Expired),
        ContractState::Active
        | ContractState::ReadyToSettle
        | ContractState::Settled { .. }
        | ContractState::Disputed => None,
    }
}

/// Whether a fee is paid or being paid: `Active`, `ReadyToSettle` or `Settled` (reviewer F2),
/// or forfeit to the Registry ([`fee_forfeit`]; the lead's ruling on R1, OC16 — a fee the
/// Registry accepted and an outage then expired is paid, so its claim's acts never wait on it).
fn fee_recorded(w: &World, fee: Option<u64>) -> bool {
    let Some(economy) = w.module_state::<EconomyStore>() else {
        return false;
    };
    fee.and_then(|id| economy.snapshot().contract(ContractId(id)))
        .is_some_and(|contract| {
            matches!(
                contract.state,
                ContractState::Active
                    | ContractState::ReadyToSettle
                    | ContractState::Settled { .. }
            ) || fee_forfeit(contract)
        })
}

/// **The Registry's books read dark** at `site_name`: the cause logged (staff repair what it
/// names), [`CLAIMS_DARK`] the line.
fn dark_because(site_name: &str, why: &dyn std::fmt::Display) -> String {
    tracing::error!(site = %site_name, %why, "tacsim: a claim act's read failed; the Registry's books read dark");
    CLAIMS_DARK.replace("{site}", site_name)
}

/// One block record, or the line a dark read prints.
fn block_record(
    reads: &dyn SurfaceReads,
    entry: &CanonicalId,
    letter: char,
    site_name: &str,
) -> Result<Option<BlockTenure>, String> {
    reads
        .block_tenure(&BlockKey {
            site: entry.clone(),
            letter,
        })
        .map_err(|error| {
            tracing::error!(site = %entry, %letter, %error, "tacsim: a block's claim record cannot be read");
            CLAIMS_DARK.replace("{site}", site_name)
        })
}

// ---- the plan --------------------------------------------------------------------------------

/// **What a `+claim` act asks** (the brief's 3.6 item 17).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ClaimAsk {
    /// `+claim/stake [<rig>] [<family>]`.
    Stake {
        /// The rig's slot, when named (the order fixes the one planned).
        slot: Option<u8>,
        /// The declared family.
        declared: Option<OreFamily>,
    },
    /// `+claim/renew [<site>] <block>`.
    Renew {
        /// The site, when named.
        site: Option<CanonicalId>,
        /// The block, when named.
        letter: Option<char>,
    },
    /// `+claim/release [<site>] <block>`.
    Release {
        /// The site, when named.
        site: Option<CanonicalId>,
        /// The block, when named.
        letter: Option<char>,
    },
    /// `+claim/contest <block>` (the site is the hull's).
    Contest {
        /// The block.
        letter: char,
    },
}

/// **Who plans** (reviewer F1): the command's preview and `go`, or the authority's re-plan at
/// the offer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    /// The bare command: nothing is written.
    Preview,
    /// `go`.
    Go,
    /// The authority's re-plan of an order at the offer.
    Offer,
}

/// The act a plan makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimKind {
    /// A stake (a filing, a take-over, or both).
    Stake,
    /// A renewal.
    Renew,
    /// A release (a lapse by choice, OC6).
    Release,
    /// A contest transfer.
    Contest,
}

impl ClaimKind {
    fn word(self) -> &'static str {
        match self {
            ClaimKind::Stake => "stake",
            ClaimKind::Renew => "renew",
            ClaimKind::Release => "release",
            ClaimKind::Contest => "contest",
        }
    }

    /// Whether a pending order of this kind adds a claim to its account (the cap).
    fn adds_a_claim(self) -> bool {
        matches!(self, ClaimKind::Stake | ClaimKind::Contest)
    }
}

/// **The compared half of a plan** (reviewer F8): exactly what the offer re-plans and requires
/// unchanged — never a tonnage a running mine moves, never a now-derived stamp. It names the
/// block's opened deposits, so it is derived truth and never serialises
/// (`truth_serialization_policy.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanCore {
    /// The act.
    pub kind: ClaimKind,
    /// The site's port entry.
    pub site: CanonicalId,
    /// The block letter.
    pub letter: char,
    /// The block place.
    pub place: ObjId,
    /// The block's opened deposits.
    pub deposits: Vec<OreDepositId>,
    /// A stake's deposits to file a new claim over.
    pub file: Vec<OreDepositId>,
    /// A stake's lapsed claims to take over.
    pub take_over: Vec<ClaimRev>,
    /// A renewal's, a release's or a contest's site-issuer claims over the block.
    pub claims: Vec<ClaimRev>,
    /// The stake re-opens a lapsed block.
    pub reopen: bool,
    /// The holder the act names (the actor; a contest's loser is read at the stage).
    pub holder: PrincipalId,
    /// The site's issuer.
    pub issuer: PrincipalId,
    /// The block's tenure epoch before the act.
    pub epoch: u32,
    /// When that epoch began (`None` for epoch 0).
    pub since: Option<i64>,
    /// A contest's window opening (the holder's `opened_ic`).
    pub opened_ic: Option<i64>,
    /// The discovery stamp: the stake's evidence or the contest's heard sounding.
    pub discovered: Option<i64>,
    /// A renewal's paper claims to suspend.
    pub suspend: Vec<ClaimRev>,
    /// A renewal suspends the block's paper.
    pub suspend_paper: bool,
    /// A renewal's worked floor.
    pub floor: u64,
    /// Whether it is met (a bool, never the tonnes).
    pub floor_met: bool,
    /// The fee, minor units (0 for a release).
    pub fee_minor: i64,
    /// The declared family.
    pub declared: Option<OreFamily>,
    /// The actor's gateway account.
    pub account: Option<mush_core::AccountId>,
}

/// **The uncompared half of a plan** (reviewer F8): the preview's words and every now-derived
/// figure.
#[derive(Clone, Debug, Default)]
pub struct PlanView {
    /// The site's name.
    pub site_name: String,
    /// The rig's slot (a stake).
    pub slot: u8,
    /// The readings clause (a stake).
    pub readings: String,
    /// The heard count and whether the scan stopped short (a stake).
    pub heard: usize,
    /// The heard scan stopped at its bound.
    pub heard_truncated: bool,
    /// Worked this term, tonnes (a renewal).
    pub worked: u64,
    /// The claim's current term end (a renewal).
    pub term_end: i64,
    /// The new term end (stake, renewal, contest) or the cooling end (release).
    pub until: i64,
    /// The standing paper's issuer name, when a paper stands `Contested`.
    pub paper: Option<String>,
    /// A contest's holder.
    pub holder_name: String,
    /// A contest's heard stamp.
    pub heard_at: i64,
    /// The stake fee, minor units.
    pub stake_minor: i64,
    /// The term fee, minor units.
    pub term_minor: i64,
}

/// **A plan** (reviewer F8): the compared core and the view.
#[derive(Clone, Debug)]
pub struct ClaimPlan {
    /// Compared at the offer.
    pub core: PlanCore,
    /// Never compared.
    pub view: PlanView,
}

/// The actor's gateway account, logged when absent.
fn account_or_cap(w: &World, actor: ObjId) -> Result<mush_core::AccountId, String> {
    crate::port::licence::gateway_account(w, actor).ok_or_else(|| {
        tracing::warn!(
            actor = actor.0,
            "tacsim: a claim act has no gateway account; refused as the cap (fail closed)"
        );
        CLAIM_CAP.replace("{max}", &MAX_LIVE_CLAIMS_PER_ACCOUNT.to_string())
    })
}

/// **The claims an account holds or is filing** (reviewer F7): the live block claims over the
/// lane's sites × A–P whose `account` is `account`, plus the claim orders of the same account
/// still `Offered` and not yet landed — a stake's or a contest's — except `excluded`'s own. An
/// order older than [`CLAIM_ORDER_TTL_IC_SECS`] is not counted: the offer no longer vouches for
/// it ([`order_tenure`]), so it can never land, and its contract only waits out its hour to
/// refund (the fixer's round, correctness F6 — a lost race or a dropped submission never locks
/// the account out for the fee's whole window).
fn account_claims(
    w: &World,
    reads: &dyn SurfaceReads,
    account: &mush_core::AccountId,
    now: i64,
    excluded: Option<ObjId>,
) -> Result<usize, String> {
    let mut n = 0_usize;
    let mut landed: Vec<(CanonicalId, char, u64)> = Vec::new();
    for site in crate::surface::chart::lane_sites(w) {
        for letter in crate::port::def::BLOCK_LETTERS {
            let record = reads.block_tenure(&BlockKey {
                site: site.entry.clone(),
                letter,
            })?;
            if let Some(claim) = record.and_then(|record| record.claim) {
                if claim.account.as_ref() == Some(account) && claim.live(now) {
                    n = n.saturating_add(1);
                }
                if let Some(fee) = claim.fee {
                    landed.push((site.entry.clone(), letter, fee));
                }
            }
        }
    }
    if let Some(orders) = w.module_state::<ClaimOrders>() {
        for order in orders.0.values() {
            if Some(order.actor) == excluded
                || order.account.as_ref() != Some(account)
                || !order.core.kind.adds_a_claim()
                || order.made_ic.saturating_add(CLAIM_ORDER_TTL_IC_SECS) < now
            {
                continue;
            }
            let Some(contract) = order.contract else {
                continue;
            };
            if contract_state(w, contract) != Some(ContractState::Offered) {
                continue;
            }
            if landed.iter().any(|(site, letter, fee)| {
                *site == order.core.site && *letter == order.core.letter && *fee == contract
            }) {
                continue;
            }
            n = n.saturating_add(1);
        }
    }
    Ok(n)
}

/// **The cooling term, three ways** (lead ruling 5, OC15, reviewer F9): a live cooling row of
/// the block's tenure binds `actor` when its holder is the actor, someone the actor acts for
/// (`mush_economy::acts_for`), or a character on the actor's gateway account (the licence and
/// the cap count per account). A stake and a contest both read it (the fixer's round,
/// correctness F4: a cooling challenger is refused before anything is escrowed, and an alt or a
/// controlled character never retakes the block inside the cooling term). `Err` is
/// [`STAKE_COOLING`] with the row's end.
fn cooling_bar(
    w: &World,
    actor: ObjId,
    account: &mush_core::AccountId,
    tenure: Option<&mush_industry::TenureRecord>,
    now: &ClockStamp,
    block: &str,
) -> Result<(), String> {
    let Some(tenure) = tenure else {
        return Ok(());
    };
    for row in tenure.cooling() {
        let Some(until) = tenure.cooling_for(&row.holder, now) else {
            continue;
        };
        let same_account = match &row.holder {
            PrincipalId::Object(holder) => {
                crate::port::licence::gateway_account(w, *holder).as_ref() == Some(account)
            }
            _ => false,
        };
        if row.holder == PrincipalId::Object(actor)
            || mush_economy::acts_for(w, actor, &row.holder)
            || same_account
        {
            return Err(STAKE_COOLING
                .replace("{block}", block)
                .replace("{until}", &ic_stamp(until.at)));
        }
    }
    Ok(())
}

/// The licence gate, as a claim act reads it.
fn licence(w: &World, actor: ObjId, now: i64) -> Result<(), String> {
    crate::port::licence::licence_gate(w, actor, now).map_err(|standing| {
        crate::port::licence::standing_line(w, actor, &standing)
            .unwrap_or_else(|| crate::port::tv::MINE_NO_LICENCE.to_owned())
    })
}

/// **The stake preview's readings clause**: the block's fields the actor sounded, else scanned
/// (`C-NW sounded`), then the pockets they cored (`C117 cored`), at most four, then
/// `, and {n} more`. The actor's own row only.
fn readings_clause(row: &crate::surface::chart::ChartRecord, letter: char) -> String {
    // Doc 32 cut S.1: the row's own site's scheme (four fields a block at 256, sixteen at 512).
    let scheme = row.scheme();
    let pockets = usize::try_from(scheme.pockets_in_block()).unwrap_or(0);
    let mut parts: Vec<String> = Vec::new();
    let cells = row.blocks.get(letter.to_string().as_str());
    if let Some(mut fields) = scheme.fields_of_block(letter) {
        // The block's fields, north-west first (the address's own reading order).
        fields.sort_by_key(|(fx, fy)| (std::cmp::Reverse(*fy), *fx));
        for (fx, fy) in fields {
            let Some(name) = scheme.field_name(fx, fy) else {
                continue;
            };
            let sounded = cells.is_some_and(|block| {
                (0..pockets).any(|index| {
                    block.cell(index).is_some_and(|byte| {
                        crate::surface::chart::cell_tier(byte) == crate::surface::chart::TIER_R2
                    }) && scheme
                        .pocket_of_block_index(letter, index)
                        .and_then(|pocket| scheme.field_of_pocket(pocket))
                        == Some((fx, fy))
                })
            });
            let scanned = scheme
                .field_index(fx, fy)
                .is_some_and(|index| row.fields.contains_key(&index));
            if sounded {
                parts.push(format!("{name} sounded"));
            } else if scanned {
                parts.push(format!("{name} scanned"));
            }
        }
    }
    if let Some(block) = cells {
        for index in 0..pockets {
            if block
                .cell(index)
                .is_some_and(|byte| byte & crate::surface::chart::PIT_SEEN != 0)
            {
                parts.push(format!("{letter}{} cored", index.saturating_add(1)));
            }
        }
    }
    if parts.len() > 4 {
        let more = parts.len().saturating_sub(4);
        parts.truncate(4);
        format!("{}, and {more} more", parts.join(", "))
    } else {
        parts.join(", ")
    }
}

/// **The one rule the command and the authority share** (the brief's 3.6 item 17): the block's
/// public tenure is read from its [`BlockTenure`] (never from the industry claims); the industry
/// claims supply only the lists the act names. **Funds are never read here** (reviewer F1).
/// `Err` carries the line.
///
/// # Errors
///
/// The refusal, in the brief's order for each act.
pub(crate) fn plan_claim(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    ask: &ClaimAsk,
    phase: Phase,
    now: i64,
) -> Result<ClaimPlan, String> {
    let registry = registry_info(w).ok_or_else(|| {
        tracing::error!("tacsim: the Registry's licence port, org or account is not present");
        CLAIMS_FAILED.to_owned()
    })?;
    match ask {
        ClaimAsk::Stake { slot, declared } => {
            plan_stake(w, reads, actor, *slot, *declared, phase, now, &registry)
        }
        ClaimAsk::Renew { site, letter } => {
            plan_renew(w, reads, actor, site.as_ref(), *letter, now, &registry)
        }
        ClaimAsk::Release { site, letter } => {
            plan_release(w, reads, actor, site.as_ref(), *letter, now, &registry)
        }
        ClaimAsk::Contest { letter } => {
            plan_contest(w, reads, actor, *letter, phase, now, &registry)
        }
    }
}

/// The stake, in the brief's order.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn plan_stake(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    slot: Option<u8>,
    declared: Option<OreFamily>,
    phase: Phase,
    now: i64,
    registry: &RegistryInfo,
) -> Result<ClaimPlan, String> {
    // 1. The rig.
    let rig = crate::port::tv::select_in(w, actor, slot, WHICH_RIG_STAKE)?;
    // 2. Its stand.
    let stand = crate::surface::act::stand_of(w, actor, &rig).map_err(|refusal| refusal.line())?;
    if stand.from_bay {
        return Err(STAKE_IN_BAY.replace("{n}", &rig.slot.to_string()));
    }
    let site_name = stand.site_name.clone();
    // 3. The block and the site.
    let letter = stand
        .scheme
        .pocket_of_cell(stand.cell)
        .and_then(|(px, py)| stand.scheme.block_letter(px, py))
        .ok_or_else(|| {
            dark_because(
                &site_name,
                &"a site, block place or issuer does not resolve",
            )
        })?;
    let site = crate::surface::act::site_def_of(w, &stand.site).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    let place = block_place(w, &stand.site, letter).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    let block = letter.to_string();
    // 4. The licence.
    licence(w, actor, now)?;
    // 5. Commons.
    let authored = crate::surface::act::authored_of(&site, letter);
    if authored == crate::surface::act::Authored::Commons {
        return Err(STAKE_COMMONS.to_owned());
    }
    // 6. The site's issuer.
    let issuer = crate::port::site::org_principal(w, &site.issuer).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    // 7. The block's public tenure.
    let record = block_record(reads, &stand.site, letter, &site_name)?;
    let claimed = || crate::port::tv::MINE_CLAIMED.to_owned();
    let mut reopen = false;
    match record.as_ref().and_then(|record| record.claim.as_ref()) {
        Some(claim) if claim.live(now) => {
            return Err(if claim.holder == PrincipalId::Object(actor) {
                STAKE_YOURS.replace("{block}", &block)
            } else {
                claimed()
            });
        }
        Some(claim) if claim.lapsing(now) => {
            return Err(STAKE_LAPSING.replace("{block}", &block));
        }
        Some(_) => reopen = true,
        None if authored == crate::surface::act::Authored::Held => return Err(claimed()),
        None => {}
    }
    let deposits = opened_deposits(reads, &stand.site, letter)
        .map_err(|error| dark_because(&site_name, &error))?;
    let claims = issuer_claims(reads, &deposits, &issuer)
        .map_err(|error| dark_because(&site_name, &error))?;
    let now_stamp = ClockStamp {
        clock: w.world_ic_now().clock,
        at: now,
    };
    if claims
        .iter()
        .any(|(_, claim)| crate::surface::act::claim_live(claim, &now_stamp))
    {
        return Err(claimed());
    }
    // 8. The cooling term, three ways (reviewer F9).
    let tenure = reads
        .tenure(place)
        .map_err(|error| dark_because(&site_name, &error))?;
    let since = tenure
        .as_ref()
        .and_then(|tenure| tenure.since().map(|since| since.at));
    let epoch = tenure
        .as_ref()
        .map_or(0, mush_industry::TenureRecord::epoch);
    let account = account_or_cap(w, actor)?;
    cooling_bar(w, actor, &account, tenure.as_ref(), &now_stamp, &block)?;
    // 9. The cap.
    let excluded = (phase == Phase::Offer).then_some(actor);
    if account_claims(w, reads, &account, now, excluded)
        .map_err(|error| dark_because(&site_name, &error))?
        >= MAX_LIVE_CLAIMS_PER_ACCOUNT
    {
        return Err(CLAIM_CAP.replace("{max}", &MAX_LIVE_CLAIMS_PER_ACCOUNT.to_string()));
    }
    // 10. The hand-made evidence.
    let row = reads
        .chart(actor, &stand.site)
        .map_err(|error| dark_because(&site_name, &error))?;
    let evidence = row
        .as_ref()
        .and_then(|row| evidence_at(row, letter, since))
        .ok_or_else(|| STAKE_NO_EVIDENCE.replace("{block}", &block))?;
    // 11. The heard count, in the command's phases only (D12).
    let (heard, heard_truncated) = if phase == Phase::Offer {
        (0, false)
    } else {
        let (rows, truncated) = reads
            .site_charts(&stand.site, MAX_HEARD_SCAN_ROWS)
            .map_err(|error| dark_because(&site_name, &error))?;
        if truncated {
            warn_heard_bound_once(w);
        }
        (
            heard_count(&rows, actor, letter, since, evidence),
            truncated,
        )
    };
    // 12. The lists: lapsed site-issuer claims are taken over, the rest filed.
    let mut file = Vec::new();
    let mut take_over = Vec::new();
    for id in &deposits {
        match claims
            .iter()
            .find(|(deposit, claim)| deposit == id && claim.is_lapsed())
        {
            Some((_, claim)) => take_over.push(rev(claim)),
            None => file.push(*id),
        }
    }
    if !take_over.is_empty() {
        reopen = true;
    }
    // 13. The fee.
    let fee_minor = registry
        .stake_minor
        .checked_add(registry.term_minor)
        .ok_or_else(|| CLAIMS_FAILED.to_owned())?;
    let paper = record
        .as_ref()
        .and_then(|record| record.paper.as_ref())
        .filter(|paper| paper.state == PaperState::Contested)
        .map(|paper| org_word(w, &paper.issuer));
    Ok(ClaimPlan {
        core: PlanCore {
            kind: ClaimKind::Stake,
            site: stand.site.clone(),
            letter,
            place,
            deposits,
            file,
            take_over,
            claims: Vec::new(),
            reopen,
            holder: PrincipalId::Object(actor),
            issuer,
            epoch,
            since,
            opened_ic: None,
            discovered: Some(evidence),
            suspend: Vec::new(),
            suspend_paper: false,
            floor: 0,
            floor_met: true,
            fee_minor,
            declared,
            account: Some(account),
        },
        view: PlanView {
            site_name,
            slot: rig.slot,
            readings: row
                .as_ref()
                .map(|row| readings_clause(row, letter))
                .unwrap_or_default(),
            heard,
            heard_truncated,
            until: now.saturating_add(CLAIM_TERM_IC_SECS),
            paper,
            stake_minor: registry.stake_minor,
            term_minor: registry.term_minor,
            ..PlanView::default()
        },
    })
}

/// Say once a boot that the heard count's scan stopped at its bound (D12).
fn warn_heard_bound_once(w: &World) {
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    let _ = w;
    if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!(
            bound = MAX_HEARD_SCAN_ROWS,
            "tacsim: a stake's heard count stopped at its scan bound; the preview says \"at least\" \
             (doc 20 cut SM.10, D12)"
        );
    }
}

/// **The actor's own claim a renewal or a release names** (D11: from anywhere): the actor's
/// unlapsed player claims over the lane's sites, narrowed by the site and the block named; one
/// left names it.
fn own_claim(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    site: Option<&CanonicalId>,
    letter: Option<char>,
) -> Result<(CanonicalId, char, BlockTenure, BlockClaim), String> {
    let me = PrincipalId::Object(actor);
    let mut found: Vec<(CanonicalId, char, BlockTenure, BlockClaim)> = Vec::new();
    for lane in crate::surface::chart::lane_sites(w) {
        if site.is_some_and(|site| *site != lane.entry) {
            continue;
        }
        for each in crate::port::def::BLOCK_LETTERS {
            if letter.is_some_and(|letter| letter != each) {
                continue;
            }
            let record = block_record(reads, &lane.entry, each, &lane.name)?;
            if let Some(record) = record
                && let Some(claim) = record.claim.clone()
                && claim.holder == me
                && claim.lapsed_ic.is_none()
            {
                found.push((lane.entry.clone(), each, record, claim));
            }
        }
    }
    match found.len() {
        1 => found.pop().ok_or_else(|| CLAIMS_FAILED.to_owned()),
        0 => Err(match letter {
            None => CLAIM_LIST_NONE.to_owned(),
            Some(letter) => {
                let name = site
                    .map(|site| site_name_of(w, site))
                    .or_else(|| hull_site(w, actor).map(|(_, name)| name))
                    .unwrap_or_else(|| "any site".to_owned());
                CLAIM_NOT_YOURS
                    .replace("{block}", &letter.to_string())
                    .replace("{site}", &name)
            }
        }),
        _ => Err(CLAIM_WHICH.to_owned()),
    }
}

/// The site the actor's helm hull stands on, if it is settled on one.
fn hull_site(w: &World, actor: ObjId) -> Option<(CanonicalId, String)> {
    let hull = crate::port::coop::helm_of(w, actor)?.hull;
    match crate::port::ship::berth_of(w, hull) {
        crate::port::ship::Berth::Docked { map, def } if def.site.is_some() => {
            Some((map.entry.clone(), crate::port::coop::port_name(def)))
        }
        _ => None,
    }
}

/// The paper standing `Contested` over a block, and its issuer's principal.
fn standing_paper(w: &World, record: &BlockTenure) -> Option<(BlockPaper, PrincipalId)> {
    let paper = record.paper.clone()?;
    if paper.state != PaperState::Contested {
        return None;
    }
    let principal = crate::port::site::org_principal(w, &paper.issuer)?;
    Some((paper, principal))
}

/// The renewal.
fn plan_renew(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    site: Option<&CanonicalId>,
    letter: Option<char>,
    now: i64,
    registry: &RegistryInfo,
) -> Result<ClaimPlan, String> {
    let (entry, letter, record, claim) = own_claim(w, reads, actor, site, letter)?;
    let site_name = site_name_of(w, &entry);
    let block = letter.to_string();
    if !fee_recorded(w, claim.fee) {
        return Err(CLAIM_PENDING
            .replace("{block}", &block)
            .replace("{site}", &site_name));
    }
    if claim.lapsing(now) {
        return Err(STAKE_LAPSING.replace("{block}", &block));
    }
    licence(w, actor, now)?;
    let site_def = crate::surface::act::site_def_of(w, &entry).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    let issuer = crate::port::site::org_principal(w, &site_def.issuer).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    let drawn =
        drawn_now(reads, &entry, letter).map_err(|error| dark_because(&site_name, &error))?;
    let worked = worked_this_term(drawn, claim.worked_at_term_start);
    let paper = standing_paper(w, &record);
    let floor = renewal_floor(paper.is_some());
    let paper_name = paper.as_ref().map(|(paper, _)| org_word(w, &paper.issuer));
    if worked < floor {
        let line = match &paper_name {
            Some(paper) => RENEW_PAPER_SHORT.replace("{paper}", paper),
            None => RENEW_SHORT.to_owned(),
        };
        return Err(line
            .replace("{block}", &block)
            .replace("{site}", &site_name)
            .replace("{worked}", &worked.to_string())
            .replace("{floor}", &floor.to_string())
            .replace("{until}", &ic_stamp(claim.term_end_ic)));
    }
    let deposits =
        opened_deposits(reads, &entry, letter).map_err(|error| dark_because(&site_name, &error))?;
    let me = PrincipalId::Object(actor);
    let claims: Vec<ClaimRev> = issuer_claims(reads, &deposits, &issuer)
        .map_err(|error| dark_because(&site_name, &error))?
        .iter()
        .filter(|(_, c)| !c.is_lapsed() && c.holder() == &me)
        .map(|(_, c)| rev(c))
        .collect();
    let suspend: Vec<ClaimRev> = match &paper {
        Some((_, principal)) => issuer_claims(reads, &deposits, principal)
            .map_err(|error| dark_because(&site_name, &error))?
            .iter()
            .filter(|(_, c)| {
                matches!(
                    c.contest_state(),
                    mush_industry::ContestState::Contested { .. }
                )
            })
            .map(|(_, c)| rev(c))
            .collect(),
        None => Vec::new(),
    };
    let tenure = reads
        .tenure(record.place)
        .map_err(|error| dark_because(&site_name, &error))?;
    Ok(ClaimPlan {
        core: PlanCore {
            kind: ClaimKind::Renew,
            site: entry,
            letter,
            place: record.place,
            deposits,
            file: Vec::new(),
            take_over: Vec::new(),
            claims,
            reopen: false,
            holder: me,
            issuer,
            epoch: tenure
                .as_ref()
                .map_or(0, mush_industry::TenureRecord::epoch),
            since: tenure
                .as_ref()
                .and_then(|tenure| tenure.since().map(|since| since.at)),
            opened_ic: claim.opened_ic,
            discovered: claim.discovered_ic,
            suspend,
            suspend_paper: paper.is_some(),
            floor,
            floor_met: true,
            fee_minor: registry.term_minor,
            declared: claim.declared,
            account: claim.account,
        },
        view: PlanView {
            site_name,
            worked,
            term_end: claim.term_end_ic,
            until: claim.term_end_ic.saturating_add(CLAIM_TERM_IC_SECS),
            paper: paper_name,
            stake_minor: registry.stake_minor,
            term_minor: registry.term_minor,
            ..PlanView::default()
        },
    })
}

/// The release (no licence needed). The actor's **live** claim (brief 3.6): a past-term claim
/// is the clerk's to lapse at its term's end (lead ruling 5), so a release of one refuses with
/// [`STAKE_LAPSING`]'s words as a renewal does — never a lapse stamped now with the cooling
/// counted from now (the fixer's round, conformance F4).
fn plan_release(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    site: Option<&CanonicalId>,
    letter: Option<char>,
    now: i64,
    registry: &RegistryInfo,
) -> Result<ClaimPlan, String> {
    let (entry, letter, record, claim) = own_claim(w, reads, actor, site, letter)?;
    let site_name = site_name_of(w, &entry);
    let block = letter.to_string();
    if !fee_recorded(w, claim.fee) {
        return Err(CLAIM_PENDING
            .replace("{block}", &block)
            .replace("{site}", &site_name));
    }
    if claim.lapsing(now) {
        return Err(STAKE_LAPSING.replace("{block}", &block));
    }
    let site_def = crate::surface::act::site_def_of(w, &entry).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    let issuer = crate::port::site::org_principal(w, &site_def.issuer).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    let deposits =
        opened_deposits(reads, &entry, letter).map_err(|error| dark_because(&site_name, &error))?;
    let me = PrincipalId::Object(actor);
    let claims: Vec<ClaimRev> = issuer_claims(reads, &deposits, &issuer)
        .map_err(|error| dark_because(&site_name, &error))?
        .iter()
        .filter(|(_, c)| !c.is_lapsed() && c.holder() == &me)
        .map(|(_, c)| rev(c))
        .collect();
    let tenure = reads
        .tenure(record.place)
        .map_err(|error| dark_because(&site_name, &error))?;
    Ok(ClaimPlan {
        core: PlanCore {
            kind: ClaimKind::Release,
            site: entry,
            letter,
            place: record.place,
            deposits,
            file: Vec::new(),
            take_over: Vec::new(),
            claims,
            reopen: false,
            holder: me,
            issuer,
            epoch: tenure
                .as_ref()
                .map_or(0, mush_industry::TenureRecord::epoch),
            since: tenure
                .as_ref()
                .and_then(|tenure| tenure.since().map(|since| since.at)),
            opened_ic: claim.opened_ic,
            discovered: claim.discovered_ic,
            suspend: Vec::new(),
            suspend_paper: false,
            floor: 0,
            floor_met: true,
            fee_minor: 0,
            declared: claim.declared,
            account: claim.account,
        },
        view: PlanView {
            site_name,
            until: now.saturating_add(COOLING_TERM_IC_SECS),
            stake_minor: registry.stake_minor,
            term_minor: registry.term_minor,
            ..PlanView::default()
        },
    })
}

/// **Whether the refund a claim owes its predecessor is still unpaid** (the fixer's round,
/// correctness F3): the claim carries a [`RefundOwed`] and the site's claim log has not recorded
/// it for the claim's epoch (paid, or found owed nothing). An absent or unreadable log reads
/// unpaid — the clerk rewrites the log the next time it settles the refund (its idempotency key
/// answers `AlreadyPosted` for one already paid), so the wait is short and nothing is lost.
fn refund_outstanding(w: &World, entry: &CanonicalId, letter: char, claim: &BlockClaim) -> bool {
    if claim.refund.is_none() {
        return false;
    }
    let recorded = w
        .module_state::<crate::store::TacsimStore>()
        .and_then(|store| store.claim_log(entry))
        .is_some_and(|state| match state {
            crate::surface::claims::ClaimLogState::Present(log) => {
                log.refunded.get(letter.to_string().as_str()) == Some(&claim.epoch)
            }
            crate::surface::claims::ClaimLogState::Unreadable => false,
        });
    !recorded
}

/// The contest, in the brief's order.
#[allow(clippy::too_many_lines)]
fn plan_contest(
    w: &World,
    reads: &dyn SurfaceReads,
    actor: ObjId,
    letter: char,
    phase: Phase,
    now: i64,
    registry: &RegistryInfo,
) -> Result<ClaimPlan, String> {
    let block = letter.to_string();
    // From aboard the hull settled on the site, with a live rig lease (D11).
    let rig = crate::surface::act::operator_rigs(w, actor)
        .into_iter()
        .next()
        .ok_or_else(|| crate::port::coop::NO_CHARTER.to_owned())?;
    let (entry, site_name) =
        crate::surface::act::aboard_site(w, actor, &rig).map_err(|refusal| refusal.line())?;
    licence(w, actor, now)?;
    let site = crate::surface::act::site_def_of(w, &entry).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    let issuer = crate::port::site::org_principal(w, &site.issuer).ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    let record = block_record(reads, &entry, letter, &site_name)?;
    let claim = match record.as_ref().and_then(|record| record.claim.clone()) {
        Some(claim) if claim.live(now) => claim,
        // A past-term claim the clerk has not lapsed: the stake's own line, never "stake it
        // instead" for a block the stake refuses too (the fixer's round, conformance F4).
        Some(claim) if claim.lapsing(now) => {
            return Err(STAKE_LAPSING.replace("{block}", &block));
        }
        _ => {
            if let (crate::surface::act::Authored::Held, Some(holder)) = (
                crate::surface::act::authored_of(&site, letter),
                site.block(letter).holder,
            ) {
                return Err(CONTEST_HELD
                    .replace("{block}", &block)
                    .replace("{org}", &org_word(w, holder)));
            }
            return Err(CONTEST_OPEN.replace("{block}", &block));
        }
    };
    let record = record.ok_or_else(|| {
        dark_because(
            &site_name,
            &"a site, block place or issuer does not resolve",
        )
    })?;
    match &claim.holder {
        PrincipalId::Object(holder) if *holder == actor => {
            return Err(STAKE_YOURS.replace("{block}", &block));
        }
        PrincipalId::Object(_) => {}
        other => {
            return Err(CONTEST_HELD
                .replace("{block}", &block)
                .replace("{org}", &holder_name(w, other)));
        }
    }
    // The holder's fee recorded (reviewer F2), and the refund an earlier contest owes the
    // holder's predecessor paid or found owed nothing (the fixer's round, correctness F3): the
    // record holds one refund slot, and a second transfer before the clerk pays the first would
    // overwrite it.
    if !fee_recorded(w, claim.fee) || refund_outstanding(w, &entry, letter, &claim) {
        return Err(CLAIM_PENDING
            .replace("{block}", &block)
            .replace("{site}", &site_name));
    }
    let tenure = reads
        .tenure(record.place)
        .map_err(|error| dark_because(&site_name, &error))?;
    let epoch = tenure
        .as_ref()
        .map_or(0, mush_industry::TenureRecord::epoch);
    if claim.epoch != epoch {
        tracing::error!(
            site = %entry,
            %letter,
            claim_epoch = claim.epoch,
            tenure_epoch = epoch,
            "tacsim: a block's claim record speaks for another tenure epoch than industry's; the \
             Registry's books read dark there"
        );
        return Err(CLAIMS_DARK.replace("{site}", &site_name));
    }
    if !contest_open(claim.registered_ic, now) {
        return Err(CONTEST_SHUT
            .replace("{block}", &block)
            .replace("{days}", &(CONTEST_WINDOW_IC_SECS / IC_DAY).to_string()));
    }
    let account = account_or_cap(w, actor)?;
    // The cooling term, as the stake reads it (the fixer's round, correctness F4): checked
    // before anything is escrowed, and binding the actor's controller's characters and alts.
    let now_stamp = ClockStamp {
        clock: w.world_ic_now().clock,
        at: now,
    };
    cooling_bar(w, actor, &account, tenure.as_ref(), &now_stamp, &block)?;
    let excluded = (phase == Phase::Offer).then_some(actor);
    if account_claims(w, reads, &account, now, excluded)
        .map_err(|error| dark_because(&site_name, &error))?
        >= MAX_LIVE_CLAIMS_PER_ACCOUNT
    {
        return Err(CLAIM_CAP.replace("{max}", &MAX_LIVE_CLAIMS_PER_ACCOUNT.to_string()));
    }
    // The window opens where the block last opened (D18, reviewer F3).
    let before = claim.discovered_ic.unwrap_or(i64::MIN);
    let heard_at = reads
        .chart(actor, &entry)
        .map_err(|error| dark_because(&site_name, &error))?
        .and_then(|row| {
            heard_before(&row, letter, claim.opened_ic, before)
                .into_iter()
                .min()
        })
        .ok_or_else(|| CONTEST_NO_EVIDENCE.replace("{block}", &block))?;
    let deposits =
        opened_deposits(reads, &entry, letter).map_err(|error| dark_because(&site_name, &error))?;
    let claims: Vec<ClaimRev> = issuer_claims(reads, &deposits, &issuer)
        .map_err(|error| dark_because(&site_name, &error))?
        .iter()
        .filter(|(_, c)| !c.is_lapsed() && c.holder() == &claim.holder)
        .map(|(_, c)| rev(c))
        .collect();
    let fee_minor = registry
        .stake_minor
        .checked_add(registry.term_minor)
        .ok_or_else(|| CLAIMS_FAILED.to_owned())?;
    Ok(ClaimPlan {
        core: PlanCore {
            kind: ClaimKind::Contest,
            site: entry,
            letter,
            place: record.place,
            deposits,
            file: Vec::new(),
            take_over: Vec::new(),
            claims,
            reopen: false,
            holder: PrincipalId::Object(actor),
            issuer,
            epoch,
            since: tenure
                .as_ref()
                .and_then(|tenure| tenure.since().map(|since| since.at)),
            opened_ic: claim.opened_ic,
            discovered: Some(heard_at),
            suspend: Vec::new(),
            suspend_paper: false,
            floor: 0,
            floor_met: true,
            fee_minor,
            declared: None,
            account: Some(account),
        },
        view: PlanView {
            site_name,
            until: now.saturating_add(CLAIM_TERM_IC_SECS),
            holder_name: holder_name(w, &claim.holder),
            heard_at,
            term_end: claim.discovered_ic.unwrap_or(0),
            stake_minor: registry.stake_minor,
            term_minor: registry.term_minor,
            ..PlanView::default()
        },
    })
}

// ---- the order slot -------------------------------------------------------------------------

/// **One claim order** (D5): what the command planned, for the authority's re-plan at the offer.
#[derive(Clone, Debug)]
pub(crate) struct ClaimOrder {
    /// The actor.
    pub actor: ObjId,
    /// Its gateway account (the cap's key).
    pub account: Option<mush_core::AccountId>,
    /// What it asked (a stake's slot fixed to the rig planned).
    pub ask: ClaimAsk,
    /// The compared plan.
    pub core: PlanCore,
    /// Its fee contract (`None` for a release).
    pub contract: Option<u64>,
    /// When it was made, IC seconds.
    pub made_ic: i64,
    /// The act's receipt.
    pub receipt: String,
}

/// **The claim orders** (D5; `ActOrders`' shape): tacsim's own RAM slot, keyed by the actor,
/// bounded at `crate::port::tv::MAX_POINTERS` (the smallest key dropped past it). A convenience
/// for the offer, never a fact: the prepare re-checks everything, and an order goes stale after
/// [`CLAIM_ORDER_TTL_IC_SECS`] or once its contract is not `Offered`.
#[derive(Clone, Debug, Default)]
pub(crate) struct ClaimOrders(BTreeMap<ObjId, ClaimOrder>);

/// Write `order` for its actor.
fn put_order(ctx: &mut Ctx<'_>, order: ClaimOrder) {
    if let Some(orders) = ctx.w.module_state_mut::<ClaimOrders>() {
        orders.0.insert(order.actor, order);
        while orders.0.len() > crate::port::tv::MAX_POINTERS {
            if orders.0.pop_first().is_none() {
                break;
            }
        }
    }
}

/// Drop `actor`'s order.
fn drop_order(ctx: &mut Ctx<'_>, actor: ObjId) {
    if let Some(orders) = ctx.w.module_state_mut::<ClaimOrders>() {
        orders.0.remove(&actor);
    }
}

/// **Forget an actor's claim order** — a test seam for the order slot's eviction (the slot is
/// bounded and RAM: `crate::port::tv::MAX_POINTERS`), so a test can reach the stage's own
/// in-bundle cap recount (reviewer F7). Never called by the game.
#[doc(hidden)]
pub fn forget_claim_order_for_tests(w: &mut World, actor: ObjId) {
    if let Some(orders) = w.module_state_mut::<ClaimOrders>() {
        orders.0.remove(&actor);
    }
}

/// Whether `actor` has a claim order in the slot (a test read).
#[doc(hidden)]
#[must_use]
pub fn claim_order_for_tests(w: &World, actor: ObjId) -> Option<u64> {
    w.module_state::<ClaimOrders>()
        .and_then(|orders| orders.0.get(&actor))
        .map(|order| order.contract.unwrap_or(0))
}

// ---- the fee contract ------------------------------------------------------------------------

/// **The claim fee contract** (lead ruling 3, D3): `licence_spec`'s shape exactly — the pilot is
/// issuer, author and payer; the Registry the one party; one consideration (the fee from the
/// pilot's account) and one outcome paying exactly the fee to the Registry's account; a bare
/// `Before` row on a real deadline [`CLAIM_FEE_WINDOW_SECS`] out; `Forfeit` to the Registry on
/// expiry of a bound contract; **Public**. Left `Offered` after the pilot's escrow, and an
/// `Offered` contract always refunds at its deadline, so a refused act costs nothing (OC14).
#[must_use]
pub(crate) fn claim_fee_spec(
    pilot_account: AccountId,
    registry: &PrincipalId,
    registry_account: AccountId,
    pilot: ObjId,
    fee: Amount,
    now: i64,
) -> ContractSpec {
    let deadline = EconomicDeadline {
        clock: EconomicClock::RealUtc,
        at: now.saturating_add(CLAIM_FEE_WINDOW_SECS),
    };
    let me = PrincipalId::Object(pilot);
    ContractSpec {
        name: SmolStr::new_static(CLAIM_FEE_NAME),
        scope: None,
        parties: vec![registry.clone()],
        considerations: vec![ConsiderationSpec {
            account: pilot_account,
            amount: fee,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("recorded"),
            when: vec![0],
            payouts: vec![PayoutShare {
                to: registry_account,
                share: Share::Exact(fee),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: Vec::new(),
        }],
        remainder_to: pilot_account,
        conditions: vec![TypedCondition::Before { deadline }],
        adjudicator: None,
        evidence: vec![EvidenceKind::Before],
        deadline: Some(deadline),
        on_expiry: Expiry::Forfeit {
            to: registry_account,
        },
        visibility: EconomicVisibility::Public,
        roles: ContractRoles {
            author: Some(me.clone()),
            payer: Some(me),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: Vec::new(),
    }
}

/// Cancel the verb's own fee contract (`licence::cancel_own`'s rule): `true` when it is
/// terminal now.
fn cancel_own(ctx: &mut Ctx<'_>, id: ContractId) -> bool {
    let outcome = mush_economy::cancel_contract(ctx, id);
    if matches!(outcome, Ok(true)) {
        return true;
    }
    tracing::error!(
        contract = id.0,
        ?outcome,
        "tacsim: +claim could not cancel its own fee contract; it refunds at its deadline; \
         `@contract/show #<id>`"
    );
    false
}

// ---- the commands ----------------------------------------------------------------------------

/// The parsed `+claim` line.
enum Parsed {
    /// `/list` or bare.
    List,
    /// A fee or release act, and whether `go` was typed.
    Act(ClaimAsk, bool),
}

/// A word that is one block letter.
fn letter_word(word: &str) -> Option<char> {
    let mut chars = word.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => {
            let upper = c.to_ascii_uppercase();
            crate::port::def::BLOCK_LETTERS
                .contains(&upper)
                .then_some(upper)
        }
        _ => None,
    }
}

/// Parse the switches and the words.
fn parse(ctx: &Ctx<'_>) -> Result<Parsed, String> {
    let raw = ctx.args.raw().trim().to_owned();
    let mut words: Vec<&str> = raw.split_whitespace().collect();
    let go = words
        .last()
        .is_some_and(|word| word.eq_ignore_ascii_case("go"));
    if go {
        words.pop();
    }
    let usage = || CLAIM_USAGE.to_owned();
    if ctx.switches.has("stake") {
        let mut slot = None;
        let mut declared = None;
        let mut rest = words.as_slice();
        if let Some((first, tail)) = rest.split_first()
            && let Ok(n) = first.parse::<u8>()
            && (1..=4).contains(&n)
            && first.len() == 1
        {
            slot = Some(n);
            rest = tail;
        }
        match rest {
            [] => {}
            [word] => {
                declared = Some(
                    crate::surface::geology::family_of_word(word).ok_or_else(|| {
                        crate::port::tv::MAP_FAMILY_UNKNOWN.replace("{word}", word)
                    })?,
                );
            }
            _ => return Err(usage()),
        }
        return Ok(Parsed::Act(ClaimAsk::Stake { slot, declared }, go));
    }
    let named =
        |ctx: &Ctx<'_>, words: &[&str]| -> Result<(Option<CanonicalId>, Option<char>), String> {
            match words.split_last() {
                None => Ok((None, None)),
                Some((last, rest)) => {
                    let letter = letter_word(last).ok_or_else(usage)?;
                    if rest.is_empty() {
                        return Ok((None, Some(letter)));
                    }
                    let name = rest.join(" ");
                    let site = crate::surface::chart::site_named(ctx.w, &name)
                        .ok_or_else(|| REGISTRY_UNKNOWN.replace("{arg}", &name))?;
                    Ok((Some(site.entry), Some(letter)))
                }
            }
        };
    if ctx.switches.has("renew") {
        let (site, letter) = named(ctx, &words)?;
        return Ok(Parsed::Act(ClaimAsk::Renew { site, letter }, go));
    }
    if ctx.switches.has("release") {
        let (site, letter) = named(ctx, &words)?;
        return Ok(Parsed::Act(ClaimAsk::Release { site, letter }, go));
    }
    if ctx.switches.has("contest") {
        return match words.as_slice() {
            [word] => Ok(Parsed::Act(
                ClaimAsk::Contest {
                    letter: letter_word(word).ok_or_else(usage)?,
                },
                go,
            )),
            _ => Err(usage()),
        };
    }
    if go || !words.is_empty() {
        return Err(usage());
    }
    Ok(Parsed::List)
}

/// **`+claim/stake|renew|release|contest|list`** (doc 20 §9; cut SM.10). Bare, each previews and
/// commits nothing; `go` commits exactly the preview. A bare `+claim` is `/list`.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn claim_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    if registry_info(ctx.w).is_none() {
        return Err(CmdError::msg(crate::port::licence::LICENCE_NOT_HERE));
    }
    match parse(ctx).map_err(CmdError::msg)? {
        Parsed::List => list(ctx),
        Parsed::Act(ask, false) => preview(ctx, &ask),
        Parsed::Act(ask, true) => go(ctx, ask),
    }
}

/// The bare act: plan and print. Writes nothing.
fn preview(ctx: &mut Ctx<'_>, ask: &ClaimAsk) -> Result<(), CmdError> {
    let now = ctx.w.world_ic_now().at;
    let plan =
        plan_claim(ctx.w, &*ctx, ctx.actor, ask, Phase::Preview, now).map_err(CmdError::msg)?;
    let line = preview_line(ctx, &plan);
    say(ctx, line);
    Ok(())
}

/// The preview's words.
fn preview_line(ctx: &Ctx<'_>, plan: &ClaimPlan) -> String {
    let (core, view) = (&plan.core, &plan.view);
    let block = core.letter.to_string();
    let money = |minor: i64| money_minor(ctx, minor);
    match core.kind {
        ClaimKind::Stake => {
            let heard = if view.heard_truncated {
                format!("at least {}", view.heard)
            } else {
                view.heard.to_string()
            };
            let mut line = STAKE_PREVIEW
                .replace("{block}", &block)
                .replace("{readings}", &view.readings)
                .replace("{heard}", &heard)
                .replace("{stake}", &money(view.stake_minor))
                .replace("{term}", &money(view.term_minor))
                .replace("{total}", &money(core.fee_minor));
            if let Some(family) = core.declared {
                line.push_str(&DECLARED_CLAUSE.replace("{family}", family.label()));
            }
            if let Some(paper) = &view.paper {
                line.push_str(
                    &PAPER_CLAUSE
                        .replace("{paper}", paper)
                        .replace("{registrar}", &REGISTRAR_WORKED_T.to_string()),
                );
            }
            line
        }
        ClaimKind::Renew => {
            let mut line = RENEW_PREVIEW
                .replace("{block}", &block)
                .replace("{site}", &view.site_name)
                .replace("{worked}", &view.worked.to_string())
                .replace("{floor}", &core.floor.to_string())
                .replace("{fee}", &money(core.fee_minor))
                .replace("{until}", &ic_stamp(view.until));
            if core.suspend_paper
                && let Some(paper) = &view.paper
            {
                line.push_str(&REGISTRAR_CLAUSE.replace("{paper}", paper));
            }
            line
        }
        ClaimKind::Release => RELEASE_PREVIEW
            .replace("{block}", &block)
            .replace("{site}", &view.site_name)
            .replace("{until}", &ic_stamp(view.until)),
        ClaimKind::Contest => CONTEST_PREVIEW
            .replace("{block}", &block)
            .replace("{site}", &view.site_name)
            .replace("{at}", &ic_stamp(view.heard_at))
            .replace("{holder}", &view.holder_name)
            .replace("{discovered}", &ic_stamp(view.term_end))
            .replace("{total}", &money(core.fee_minor))
            .replace("{until}", &ic_stamp(view.until)),
    }
}

/// The filed line.
fn filed_line(ctx: &Ctx<'_>, plan: &ClaimPlan) -> String {
    let (core, view) = (&plan.core, &plan.view);
    let money = money_minor(ctx, core.fee_minor);
    let template = match core.kind {
        ClaimKind::Stake => STAKE_FILED,
        ClaimKind::Renew => RENEW_FILED,
        ClaimKind::Release => RELEASE_FILED,
        ClaimKind::Contest => CONTEST_FILED,
    };
    template
        .replace("{block}", &core.letter.to_string())
        .replace("{site}", &view.site_name)
        .replace("{total}", &money)
        .replace("{fee}", &money)
}

/// A refusal after the first write: the deferred queue is pinned (`licence::refuse_after_writes`).
fn refuse_after_writes(ctx: &mut Ctx<'_>, line: String) -> CmdError {
    ctx.out.pin_deferred();
    CmdError::msg(line)
}

/// **`go`** — the plan, the funds, the offer, the escrow, the order and ONE submission (the
/// brief's 3.6 item 17).
#[allow(clippy::too_many_lines)]
fn go(ctx: &mut Ctx<'_>, ask: ClaimAsk) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let now = ctx.w.world_ic_now().at;
    let plan = plan_claim(ctx.w, &*ctx, actor, &ask, Phase::Go, now).map_err(CmdError::msg)?;
    let Some(registry) = registry_info(ctx.w) else {
        return Err(CmdError::msg(CLAIMS_FAILED));
    };
    let ask = match ask {
        ClaimAsk::Stake { declared, .. } => ClaimAsk::Stake {
            slot: Some(plan.view.slot),
            declared,
        },
        other => other,
    };
    let Some(action) = mush_industry::action_ref(ctx.reg, mush_industry::IndustryVerb::ClaimTenure)
    else {
        tracing::error!("tacsim: +claim: industry_claim_tenure is not registered");
        return Err(CmdError::msg(CLAIMS_FAILED));
    };
    let block = plan.core.letter.to_ascii_lowercase();
    let mut contract: Option<ContractId> = None;
    if plan.core.kind != ClaimKind::Release {
        // The funds (reviewer F1).
        let Some(credit) = crate::port::credit::own_account(ctx.w, actor) else {
            return Err(CmdError::msg(
                CLAIM_FUNDS
                    .replace("{total}", &money_minor(ctx, plan.core.fee_minor))
                    .replace("{available}", &money_minor(ctx, 0)),
            ));
        };
        let pilot_account = credit.id;
        let available = credit.available().map_or(0, Amount::minor);
        if available < plan.core.fee_minor {
            return Err(CmdError::msg(
                CLAIM_FUNDS
                    .replace("{total}", &money_minor(ctx, plan.core.fee_minor))
                    .replace("{available}", &money_minor(ctx, available)),
            ));
        }
        // The offer, as the pilot.
        let spec = claim_fee_spec(
            pilot_account,
            &registry.principal,
            registry.account,
            actor,
            Amount::from_minor(plan.core.fee_minor),
            ctx.w.now(),
        );
        let id = match mush_economy::offer_contract(ctx, spec) {
            Ok(id) => id,
            Err(error) => {
                tracing::error!(%error, pilot = actor.0, "tacsim: +claim: the fee contract was not offered");
                return Err(CmdError::msg(CLAIMS_FAILED));
            }
        };
        // The pilot's escrow; the contract stays Offered (the Registry has not accepted).
        match mush_economy::accept_contract(ctx, id) {
            Ok(AcceptOutcome::Accepted { .. }) => {}
            other => {
                let short = matches!(
                    other,
                    Err(ContractError::Refused(EconomyError::InsufficientAvailable(
                        _
                    )))
                );
                tracing::error!(contract = id.0, outcome = ?other, "tacsim: +claim: the pilot's escrow was not placed");
                cancel_own(ctx, id);
                let line = if short {
                    let available = crate::port::credit::own_account(ctx.w, actor)
                        .and_then(Account::available)
                        .map_or(0, Amount::minor);
                    CLAIM_FUNDS
                        .replace("{total}", &money_minor(ctx, plan.core.fee_minor))
                        .replace("{available}", &money_minor(ctx, available))
                } else {
                    CLAIMS_FAILED.to_owned()
                };
                return Err(refuse_after_writes(ctx, line));
            }
        }
        contract = Some(id);
    }
    let receipt = match contract {
        Some(id) => format!("tacsim.claim/{}/{}", actor.0, id.0),
        None => format!(
            "tacsim.claim/{}/release/{}/{block}/{}",
            actor.0, plan.core.site, plan.core.epoch
        ),
    };
    let key = mush_core::IdempotencyKeyText::parse_canonical(receipt.clone())
        .ok()
        .filter(|_| mush_industry::is_receipt(&receipt));
    let Some(key) = key else {
        tracing::error!(%receipt, "tacsim: +claim: the act's key did not form");
        if let Some(id) = contract {
            cancel_own(ctx, id);
        }
        return Err(refuse_after_writes(ctx, CLAIMS_FAILED.to_owned()));
    };
    put_order(
        ctx,
        ClaimOrder {
            actor,
            account: plan.core.account,
            ask,
            core: plan.core.clone(),
            contract: contract.map(|id| id.0),
            made_ic: now,
            receipt: receipt.clone(),
        },
    );
    ctx.submit_authoritative(
        action,
        mush_core::PerceivedRef::SelfActor,
        Vec::new(),
        serde_json::json!({ "receipt": receipt }),
        key,
    );
    let line = filed_line(ctx, &plan);
    say(ctx, line);
    // An order whose contract is no longer pending leaves the slot (a convenience prune).
    let stale: Vec<ObjId> = ctx
        .w
        .module_state::<ClaimOrders>()
        .map(|orders| {
            orders
                .0
                .values()
                .filter(|order| {
                    order.made_ic.saturating_add(CLAIM_ORDER_TTL_IC_SECS) < now
                        || order.contract.is_some_and(|id| {
                            contract_state(ctx.w, id) != Some(ContractState::Offered)
                        })
                })
                .map(|order| order.actor)
                .collect()
        })
        .unwrap_or_default();
    for who in stale {
        if who != actor {
            drop_order(ctx, who);
        }
    }
    Ok(())
}

/// The worked tonnes and the floor of one block claim.
fn worked_and_floor(
    w: &World,
    reads: &dyn SurfaceReads,
    entry: &CanonicalId,
    letter: char,
    record: &BlockTenure,
    claim: &BlockClaim,
) -> Option<(u64, u64)> {
    let drawn = drawn_now(reads, entry, letter).ok()?;
    let paper = standing_paper(w, record);
    Some((
        worked_this_term(drawn, claim.worked_at_term_start),
        renewal_floor(paper.is_some()),
    ))
}

/// **`+claim/list`** — a pure read of the actor's own unlapsed claims.
fn list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = PrincipalId::Object(ctx.actor);
    let mut rows: Vec<String> = Vec::new();
    for site in crate::surface::chart::lane_sites(ctx.w) {
        for letter in crate::port::def::BLOCK_LETTERS {
            let record =
                block_record(&*ctx, &site.entry, letter, &site.name).map_err(CmdError::msg)?;
            let Some(record) = record else {
                continue;
            };
            let Some(claim) = record.claim.as_ref() else {
                continue;
            };
            if claim.holder != me || claim.lapsed_ic.is_some() {
                continue;
            }
            let (worked, floor) =
                worked_and_floor(ctx.w, &*ctx, &site.entry, letter, &record, claim)
                    .ok_or_else(|| CmdError::msg(CLAIMS_DARK.replace("{site}", &site.name)))?;
            rows.push(
                CLAIM_LIST_ROW
                    .replace("{site}", &site.name)
                    .replace("{block}", &letter.to_string())
                    .replace("{until}", &ic_stamp(claim.term_end_ic))
                    .replace("{worked}", &worked.to_string())
                    .replace("{floor}", &floor.to_string()),
            );
        }
    }
    let line = if rows.is_empty() {
        CLAIM_LIST_NONE.to_owned()
    } else {
        let mut lines = vec![
            CLAIM_LIST_HEAD
                .replace("{n}", &rows.len().to_string())
                .replace("{max}", &MAX_LIVE_CLAIMS_PER_ACCOUNT.to_string()),
        ];
        lines.extend(rows);
        lines.join("\n")
    };
    say(ctx, line);
    Ok(())
}

/// **`+registry [<site>]`** (doc 20 §9; cut SM.10, reviewer F5) — a pure read of the site's
/// sixteen block records: one row per block with a claim or a paper, A–P, then the tail. Nothing
/// it prints reads an industry claim or a deposit.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn registry_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    crate::commands::require_enabled(ctx)?;
    let Some(registry) = registry_info(ctx.w) else {
        return Err(CmdError::msg(crate::port::licence::LICENCE_NOT_HERE));
    };
    let raw = ctx.args.raw().trim().to_owned();
    let (entry, name) = if raw.is_empty() {
        hull_site(ctx.w, ctx.actor).ok_or_else(|| CmdError::msg(REGISTRY_USAGE))?
    } else {
        let site = crate::surface::chart::site_named(ctx.w, &raw)
            .ok_or_else(|| CmdError::msg(REGISTRY_UNKNOWN.replace("{arg}", &raw)))?;
        (site.entry, site.name)
    };
    let lines = registry_lines(ctx.w, &*ctx, &registry, &entry, &name)
        .map_err(|_| CmdError::msg(REGISTRY_DARK.replace("{site}", &name)))?;
    say(ctx, lines.join("\n"));
    Ok(())
}

/// The registry's lines for one site (pure over the reads).
fn registry_lines(
    w: &World,
    reads: &dyn SurfaceReads,
    registry: &RegistryInfo,
    entry: &CanonicalId,
    name: &str,
) -> Result<Vec<String>, String> {
    let now = w.world_ic_now().at;
    let site = crate::surface::act::site_def_of(w, entry).ok_or_else(|| "no site".to_owned())?;
    let mut lines = vec![
        REGISTRY_HEAD
            .replace("{issuer}", &registry.name)
            .replace("{site}", name),
    ];
    let mut rows = 0_usize;
    let (mut open, mut commons) = (0_usize, 0_usize);
    for letter in crate::port::def::BLOCK_LETTERS {
        let block = letter.to_string();
        let record = reads.block_tenure(&BlockKey {
            site: entry.clone(),
            letter,
        })?;
        let claim = record
            .as_ref()
            .and_then(|record| record.claim.as_ref())
            .filter(|claim| claim.lapsed_ic.is_none());
        let authored = crate::surface::act::authored_of(&site, letter);
        match claim {
            Some(claim) if !claim.player_held() => {
                lines.push(
                    REGISTRY_HELD
                        .replace("{block}", &block)
                        .replace("{org}", &holder_name(w, &claim.holder)),
                );
                rows = rows.saturating_add(1);
            }
            Some(claim) if claim.lapsing(now) => {
                lines.push(
                    REGISTRY_LAPSING
                        .replace("{block}", &block)
                        .replace("{holder}", &holder_name(w, &claim.holder))
                        .replace("{until}", &ic_stamp(claim.term_end_ic)),
                );
                rows = rows.saturating_add(1);
            }
            Some(claim) => {
                let record = record.as_ref().ok_or_else(|| "no record".to_owned())?;
                let (worked, floor) = worked_and_floor(w, reads, entry, letter, record, claim)
                    .ok_or_else(|| "the cursors do not read".to_owned())?;
                let declared = claim.declared.map_or_else(
                    || "no family declared".to_owned(),
                    |family| format!("declared {}", family.label()),
                );
                let contest = if contest_open(claim.registered_ic, now) {
                    format!(
                        "contestable to {}",
                        ic_stamp(claim.registered_ic.saturating_add(CONTEST_WINDOW_IC_SECS))
                    )
                } else {
                    "uncontested".to_owned()
                };
                lines.push(
                    REGISTRY_CLAIM
                        .replace("{block}", &block)
                        .replace("{holder}", &holder_name(w, &claim.holder))
                        .replace("{until}", &ic_stamp(claim.term_end_ic))
                        .replace("{epoch}", &claim.epoch.to_string())
                        .replace("{declared}", &declared)
                        .replace(
                            "{discovered}",
                            &claim.discovered_ic.map_or_else(String::new, ic_stamp),
                        )
                        .replace("{worked}", &worked.to_string())
                        .replace("{floor}", &floor.to_string())
                        .replace("{contest}", &contest),
                );
                rows = rows.saturating_add(1);
            }
            None => match authored {
                crate::surface::act::Authored::Commons => commons = commons.saturating_add(1),
                crate::surface::act::Authored::Held => {
                    // R5's fallback: a held block the clerk has not seeded yet.
                    if let Some(holder) = site.block(letter).holder {
                        lines.push(
                            REGISTRY_HELD
                                .replace("{block}", &block)
                                .replace("{org}", &org_word(w, holder)),
                        );
                        rows = rows.saturating_add(1);
                    }
                }
                crate::surface::act::Authored::Open => open = open.saturating_add(1),
            },
        }
        if let Some(paper) = record.as_ref().and_then(|record| record.paper.as_ref()) {
            let state = match paper.state {
                PaperState::Contested => "contested".to_owned(),
                PaperState::Suspended => format!("suspended by the {}", registry.name),
            };
            lines.push(
                REGISTRY_PAPER
                    .replace("{block}", &block)
                    .replace("{paper}", &org_word(w, &paper.issuer))
                    .replace("{state}", &state),
            );
            rows = rows.saturating_add(1);
        }
    }
    if rows == 0 {
        lines.push(REGISTRY_NONE.to_owned());
    } else {
        lines.push(
            REGISTRY_TAIL
                .replace("{open}", &open.to_string())
                .replace("{commons}", &commons.to_string()),
        );
    }
    Ok(lines)
}

// ---- the authority's two halves --------------------------------------------------------------

/// **The game's payload on a tenure act** (`TenureAct::game`): what [`stage`] needs that
/// industry's row does not carry. Never a count, a stamp of a reading or a reserve.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct TenureGame {
    /// The site's port entry.
    pub site: CanonicalId,
    /// The block letter.
    pub letter: String,
    /// The act (`stake`, `renew`, `release`, `contest`, `lapse`, `unpaid`, `expired`,
    /// `seed_held`, `seed_paper`).
    pub kind: String,
    /// The fee contract (a fee act) or the paper's adjudication contract (a paper seed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contract: Option<u64>,
    /// The declared family.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub declared: Option<OreFamily>,
    /// The actor's gateway account (a stake or a contest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<mush_core::AccountId>,
    /// Every lane site entry (the stage's in-bundle cap recount, reviewer F7).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sites: Vec<CanonicalId>,
    /// The loser's term fee, minor units (a contest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refund_minor: Option<i64>,
    /// The renewal suspends the block's paper.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub suspend_paper: bool,
    /// A paper seed's issuer (lane org id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_issuer: Option<CanonicalId>,
    /// A paper seed's papers row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper_row: Option<CanonicalId>,
}

impl TenureGame {
    /// A payload for `kind` on `(site, letter)` with every option empty.
    pub(crate) fn new(site: &CanonicalId, letter: char, kind: &str) -> TenureGame {
        TenureGame {
            site: site.clone(),
            letter: letter.to_string(),
            kind: kind.to_owned(),
            contract: None,
            declared: None,
            account: None,
            sites: Vec::new(),
            refund_minor: None,
            suspend_paper: false,
            paper_issuer: None,
            paper_row: None,
        }
    }
}

/// **The act an order vouches for at the offer** (the brief's 3.6 item 18): the order present
/// and young; its fee contract (a fee act's) still `Offered`, issued and paid by the actor, the
/// actor's acceptance present, and carrying the claim fee's whole shape at the core's fee
/// ([`fee_terms_hold`]; reviewer F1, the fixer's round's correctness F2); the
/// re-plan in `Phase::Offer` unchanged in its core (reviewer F8). `None` otherwise: no offer.
#[must_use]
pub(crate) fn order_tenure(engine: &Engine, actor: ObjId) -> Option<TenureAct> {
    let w = &engine.world;
    let reads = EngineReads(engine);
    let now = w.world_ic_now().at;
    let order = w.module_state::<ClaimOrders>()?.0.get(&actor)?.clone();
    if order.made_ic.saturating_add(CLAIM_ORDER_TTL_IC_SECS) < now {
        return None;
    }
    let me = PrincipalId::Object(actor);
    match order.contract {
        Some(id) => {
            // The whole fee shape, not only its state and its sum (the fixer's round,
            // correctness F2): Offered, the actor's own and accepted, paying exactly the core's
            // fee to the Registry and nothing else.
            let registry = registry_info(w)?;
            let economy = w.module_state::<EconomyStore>()?;
            let contract = economy.snapshot().contract(ContractId(id))?;
            if !fee_terms_hold(contract, &me, &registry, order.core.fee_minor) {
                return None;
            }
        }
        None if order.core.kind == ClaimKind::Release => {}
        None => return None,
    }
    let plan = plan_claim(w, &reads, actor, &order.ask, Phase::Offer, now).ok()?;
    if plan.core != order.core {
        return None;
    }
    let core = &plan.core;
    let kind = match core.kind {
        ClaimKind::Stake => TenureKind::Stake {
            issuer: core.issuer.clone(),
            holder: core.holder.clone(),
            term_at: now.checked_add(CLAIM_TERM_IC_SECS)?,
            discovered_at: core.discovered?,
            reopen: core.reopen,
            file: core.file.clone(),
            take_over: core.take_over.clone(),
        },
        ClaimKind::Renew => TenureKind::Renew {
            claims: core.claims.clone(),
            term_at: plan.view.until,
            suspend: core.suspend.clone(),
            by: (!core.suspend.is_empty()).then(|| core.issuer.clone()),
        },
        ClaimKind::Release => TenureKind::Lapse {
            holder: core.holder.clone(),
            claims: core.claims.clone(),
            at: now,
            cooling_until: now.checked_add(COOLING_TERM_IC_SECS)?,
        },
        ClaimKind::Contest => TenureKind::Contest {
            claims: core.claims.clone(),
            to: core.holder.clone(),
            discovered_at: core.discovered?,
            term_at: now.checked_add(CLAIM_TERM_IC_SECS)?,
        },
    };
    let mut game = TenureGame::new(&core.site, core.letter, core.kind.word());
    game.contract = order.contract;
    game.declared = core.declared;
    game.account = core.account;
    game.suspend_paper = core.suspend_paper;
    if core.kind.adds_a_claim() {
        game.sites = crate::surface::chart::lane_sites(w)
            .into_iter()
            .map(|site| site.entry)
            .collect();
    }
    if core.kind == ClaimKind::Contest {
        game.refund_minor = registry_info(w).map(|registry| registry.term_minor);
    }
    Some(TenureAct {
        receipt: SmolStr::new(&order.receipt),
        actor,
        place: core.place,
        kind,
        summary: TENURE_SUMMARY.to_owned(),
        game: serde_json::to_value(&game).ok()?,
    })
}

/// The handles [`stage`] writes through.
#[derive(Clone, Debug)]
pub(crate) struct ClaimHandles {
    /// `tacsim.claims`.
    pub claims: mush_core::StateFamilyHandle<BlockKey, BlockTenure>,
    /// `tacsim.pockets`.
    pub pockets: mush_core::StateFamilyHandle<OreDepositId, crate::surface::cursor::PocketCursor>,
}

/// The books cannot be read (the stage's fail-closed line).
const STAGE_DARK: &str = "The Registry's books cannot be read right now; nothing changes.";

/// **The game's own writes in industry's tenure bundle** (the brief's 3.6 item 18; D2, OC11):
/// `tacsim.claims` only — never an industry family (the D10 tripwire). The block's worked basis
/// from its cursors (an absent cursor is 0); per act: a stake writes a fresh [`BlockClaim`]
/// (`opened_ic` the tenure's `since` before the act, reviewer F3) after re-counting the account's
/// live claims inside the bundle (reviewer F7); a renewal moves the term on from its end and
/// resets the basis (and suspends the paper when it does); a lapse stamps `lapsed_ic`; a contest
/// passes the claim to the challenger, keeps `opened_ic` and owes the loser their paid term fee;
/// a seed writes the org's claim or the paper. An `Err` refuses the whole act.
///
/// # Errors
///
/// The refusal line.
#[allow(clippy::too_many_lines)]
pub(crate) fn stage(
    txn: &mut mush_core::ActionTxn<'_>,
    handles: &ClaimHandles,
    act: &TenureAct,
    outcome: &TenureOutcome,
    now: &ClockStamp,
) -> Result<(), String> {
    let game: TenureGame =
        serde_json::from_value(act.game.clone()).map_err(|_| STAGE_DARK.to_owned())?;
    let letter = {
        let mut chars = game.letter.chars();
        match (chars.next(), chars.next()) {
            (Some(letter), None) => letter,
            _ => return Err(STAGE_DARK.to_owned()),
        }
    };
    let key = BlockKey {
        site: game.site.clone(),
        letter,
    };
    let existing = txn
        .get(&handles.claims, &key)
        .map_err(|_| STAGE_DARK.to_owned())?
        .cloned();
    let mut record = existing
        .clone()
        .unwrap_or_else(|| BlockTenure::new(game.site.clone(), letter, act.place));
    if record.place != act.place || record.key().as_ref() != Some(&key) {
        return Err(STAGE_DARK.to_owned());
    }
    let mut drawn = 0_u64;
    for id in block_deposit_ids(&game.site, letter) {
        if let Some(cursor) = txn
            .get(&handles.pockets, &id)
            .map_err(|_| STAGE_DARK.to_owned())?
        {
            if cursor.deposit != id {
                return Err(STAGE_DARK.to_owned());
            }
            drawn = cursor
                .drawn_total()
                .and_then(|total| drawn.checked_add(total))
                .ok_or_else(|| STAGE_DARK.to_owned())?;
        }
    }
    let claimed = || crate::port::tv::MINE_CLAIMED.to_owned();
    let at = now.at;
    match (&act.kind, game.kind.as_str()) {
        (
            TenureKind::Stake {
                holder,
                term_at,
                discovered_at,
                reopen,
                ..
            },
            "stake",
        ) => {
            match (record.claim.as_ref(), reopen) {
                (None, false) => {}
                (Some(old), true) if old.lapsed_ic.is_some() => {}
                _ => return Err(claimed()),
            }
            let account = game.account.ok_or_else(|| STAGE_DARK.to_owned())?;
            let mut n = 0_usize;
            for site in &game.sites {
                for each in crate::port::def::BLOCK_LETTERS {
                    let other = BlockKey {
                        site: site.clone(),
                        letter: each,
                    };
                    if other == key {
                        continue;
                    }
                    let held = txn
                        .get(&handles.claims, &other)
                        .map_err(|_| STAGE_DARK.to_owned())?
                        .and_then(|row| row.claim.as_ref())
                        .is_some_and(|claim| claim.account == Some(account) && claim.live(at));
                    if held {
                        n = n.saturating_add(1);
                    }
                }
            }
            if n >= MAX_LIVE_CLAIMS_PER_ACCOUNT {
                return Err(CLAIM_CAP.replace("{max}", &MAX_LIVE_CLAIMS_PER_ACCOUNT.to_string()));
            }
            record.claim = Some(BlockClaim {
                holder: holder.clone(),
                account: Some(account),
                epoch: outcome.epoch_after,
                opened_ic: outcome.since_before,
                registered_ic: at,
                term_start_ic: at,
                term_end_ic: *term_at,
                worked_at_term_start: drawn,
                discovered_ic: Some(*discovered_at),
                declared: game.declared,
                fee: game.contract,
                refund: None,
                lapsed_ic: None,
            });
        }
        (TenureKind::Renew { term_at, .. }, "renew") => {
            let Some(claim) = record.claim.as_mut() else {
                return Err(STAGE_DARK.to_owned());
            };
            if claim.lapsed_ic.is_some() || claim.holder != PrincipalId::Object(act.actor) {
                return Err(STAGE_DARK.to_owned());
            }
            let start = claim.term_end_ic;
            claim.term_start_ic = start;
            claim.term_end_ic = *term_at;
            claim.worked_at_term_start = drawn;
            claim.fee = game.contract;
            if game.suspend_paper
                && let Some(paper) = record.paper.as_mut()
            {
                paper.state = PaperState::Suspended;
                paper.suspended_ic = Some(start);
            }
        }
        (
            TenureKind::Lapse {
                holder,
                at: lapse_at,
                ..
            },
            "release" | "lapse" | "unpaid" | "expired",
        ) => {
            let Some(claim) = record.claim.as_mut() else {
                return Err(STAGE_DARK.to_owned());
            };
            if claim.lapsed_ic.is_some() || &claim.holder != holder {
                return Err(STAGE_DARK.to_owned());
            }
            claim.lapsed_ic = Some(*lapse_at);
        }
        (
            TenureKind::Contest {
                to,
                discovered_at,
                term_at,
                ..
            },
            "contest",
        ) => {
            let Some(old) = record.claim.clone() else {
                return Err(STAGE_DARK.to_owned());
            };
            if !old.live(at) || &old.holder == to || !old.player_held() {
                return Err(STAGE_DARK.to_owned());
            }
            let refund = match (&old.holder, old.fee, game.refund_minor) {
                (PrincipalId::Object(loser), Some(paid_by), Some(minor)) if minor > 0 => {
                    Some(RefundOwed {
                        to: *loser,
                        minor,
                        paid_by,
                    })
                }
                _ => None,
            };
            record.claim = Some(BlockClaim {
                holder: to.clone(),
                account: game.account,
                epoch: outcome.epoch_after,
                opened_ic: old.opened_ic,
                registered_ic: at,
                term_start_ic: at,
                term_end_ic: *term_at,
                worked_at_term_start: drawn,
                discovered_ic: Some(*discovered_at),
                declared: None,
                fee: game.contract,
                refund,
                lapsed_ic: None,
            });
        }
        (
            TenureKind::Seed {
                holder, term_at, ..
            },
            "seed_held",
        ) => {
            if record.claim.is_some() {
                return Err("that block is already seeded".to_owned());
            }
            record.claim = Some(BlockClaim {
                holder: holder.clone(),
                account: None,
                epoch: outcome.epoch_after,
                opened_ic: None,
                registered_ic: at,
                term_start_ic: at,
                term_end_ic: *term_at,
                worked_at_term_start: 0,
                discovered_ic: None,
                declared: None,
                fee: None,
                refund: None,
                lapsed_ic: None,
            });
        }
        (TenureKind::Seed { contest, .. }, "seed_paper") => {
            if record.paper.is_some() {
                return Err("that block is already seeded".to_owned());
            }
            let (Some(issuer), Some(row), Some(contract)) = (
                game.paper_issuer.clone(),
                game.paper_row.clone(),
                contest.map(|id| id.0),
            ) else {
                return Err(STAGE_DARK.to_owned());
            };
            record.paper = Some(BlockPaper {
                issuer,
                row,
                contract,
                state: PaperState::Contested,
                suspended_ic: None,
            });
        }
        _ => return Err(STAGE_DARK.to_owned()),
    }
    record.check().map_err(|error| {
        tracing::error!(site = %game.site, %letter, %error, "tacsim: a staged block record failed its check");
        STAGE_DARK.to_owned()
    })?;
    let written = if existing.is_some() {
        txn.replace(&handles.claims, &key, record)
    } else {
        txn.insert(&handles.claims, &key, record)
    };
    written.map_err(|_| STAGE_DARK.to_owned())
}

// ---- the public word ----------------------------------------------------------------------------

/// **A block's tenure word for `viewer`, from its record** (the brief's 3.8 item 20): a live
/// org's claim "held by {org}", the viewer's own [`TENURE_YOURS`], another character's
/// [`TENURE_CLAIMED`]; otherwise — no record, no claim, a lapsed one or one past its term
/// (SM.7's R20) — the authored word (`crate::port::site::block_tenure`, unchanged).
#[must_use]
#[allow(clippy::too_many_arguments)] // The ground line's own inputs, each a borrow it already holds.
pub(crate) fn block_tenure_now(
    w: &World,
    reads: &dyn SurfaceReads,
    viewer: ObjId,
    entry: &CanonicalId,
    site: &crate::port::def::SiteDef,
    letter: char,
    pack: Option<&crate::pack::RulesPack>,
    now: i64,
) -> String {
    let claim = reads
        .block_tenure(&BlockKey {
            site: entry.clone(),
            letter,
        })
        .ok()
        .flatten()
        .and_then(|record| record.claim)
        .filter(|claim| claim.live(now));
    match claim {
        Some(claim) if claim.holder == PrincipalId::Object(viewer) => TENURE_YOURS.to_owned(),
        Some(claim) if claim.player_held() => {
            TENURE_CLAIMED.replace("{holder}", &holder_name(w, &claim.holder))
        }
        Some(claim) => format!("held by {}", holder_name(w, &claim.holder)),
        None => crate::port::site::block_tenure(pack, site, letter),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fee contract is `licence_spec`'s shape exactly but for its name and outcome word.
    #[test]
    fn sm10_the_claim_fee_spec_is_the_licences_shape() {
        let registry = PrincipalId::Organization(OrgId(7));
        let fee = Amount::from_minor(400_000);
        let claim = claim_fee_spec(AccountId(1), &registry, AccountId(2), ObjId(9), fee, 1_000);
        let licence = crate::port::licence::licence_spec(
            AccountId(1),
            &registry,
            AccountId(2),
            ObjId(9),
            fee,
            1_000,
        );
        assert_eq!(claim.name.as_str(), CLAIM_FEE_NAME);
        assert_eq!(claim.scope, licence.scope);
        assert_eq!(claim.parties, licence.parties);
        assert_eq!(claim.considerations.len(), 1);
        assert_eq!(claim.considerations[0].account, AccountId(1));
        assert_eq!(claim.considerations[0].amount, fee);
        assert_eq!(claim.outcomes.len(), 1);
        assert_eq!(claim.outcomes[0].payouts.len(), 1);
        assert_eq!(claim.outcomes[0].payouts[0].to, AccountId(2));
        assert_eq!(claim.outcomes[0].payouts[0].share, Share::Exact(fee));
        assert_eq!(claim.conditions, licence.conditions);
        assert_eq!(claim.deadline, licence.deadline);
        assert_eq!(claim.on_expiry, licence.on_expiry);
        assert_eq!(claim.visibility, EconomicVisibility::Public);
        assert_eq!(claim.roles, licence.roles);
        assert!(claim.deterministic && claim.adjudicator.is_none());
        assert_eq!(claim.evidence, licence.evidence);
    }

    /// The block-letter word.
    #[test]
    fn sm10_a_block_word_is_one_letter() {
        assert_eq!(letter_word("c"), Some('C'));
        assert_eq!(letter_word("P"), Some('P'));
        assert_eq!(letter_word("Q"), None);
        assert_eq!(letter_word("CD"), None);
    }
}
