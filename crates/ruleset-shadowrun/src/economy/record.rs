//! The E6a records (08f §3.1) and the wires they deserialize through.
//!
//! Two container documents live here — `sr:profile:{obj}` and `sr:run:{id}` — and
//! both follow the **E5a fold rule**: the persisted shape is a separate `…Wire`
//! struct, and the live record is reached only through a `TryFrom` that re-runs the
//! constructor invariants. A document that fails it is dropped loudly at boot; the
//! run board is a projection of contracts that still exist, so a dropped listing
//! costs a board entry and never a contract, an account or a hold.
//!
//! Neither record duplicates money, authority or consent: those are the contract's
//! (D-E6-5). [`RunTemplate`] carries the contract's **id** and a copied notice; the
//! contract never points back at the party (`RunTemplate.party` is the board's own
//! back-reference, and `mush_party` holds the other half).

use mush_core::ObjId;
use mush_economy::{AccountId, Amount, ContractId, EconomicDeadline, ObligationId, TransactionId};
use mush_faction::FactionId;
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::economy::config::{
    LifestyleTier, MAX_MINT_KEY_BYTES, MAX_NOTICE_KIND_BYTES, MAX_NOTICE_KINDS, MAX_RUN_APPLICANTS,
    MAX_RUN_NAME_BYTES, MAX_RUN_OUTCOMES, MAX_RUN_SLOTS, MAX_RUN_WINDOW_SECS,
};

/// A board listing's id.
#[derive(
    Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash,
)]
pub struct RunId(pub u64);

impl std::fmt::Display for RunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A one-shot attributed post this profile has already made (M12, M13).
///
/// The idempotency key is the authority — a re-run resolves to `AlreadyPosted` — and
/// this record is what `@sr/migrate` reads to tell "already minted" from "never
/// minted", and to report a HIT with the original result.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mint {
    /// The deterministic idempotency key (`sr.chargen/{obj}`).
    pub key: SmolStr,
    /// The transaction it landed on, when the post reported one.
    #[serde(default)]
    pub transaction: Option<TransactionId>,
    /// How much was minted, in minor units.
    pub amount: Amount,
    /// When (wall clock).
    pub at: i64,
}

/// What `@sr/migrate commit` bound when it adopted a sheet's `nuyen` (answer 18).
///
/// `SheetDoc` carries no revision (`mush-ruleset/src/doc.rs`), so the binding is the
/// **sheet value at the commit** plus the approval stamp. A sheet edited afterwards
/// is reported as a mismatch and never re-minted.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Migration {
    /// The sheet `nuyen` this commit bound.
    pub from_sheet_value: i64,
    /// The approval stamp's instant (`ApprovalState::Approved { at }`).
    pub approved_at: i64,
    /// Who approved (`ApprovalState::Approved { by }`).
    pub approved_by: ObjId,
    /// The transaction it landed on, when the post reported one.
    #[serde(default)]
    pub transaction: Option<TransactionId>,
    /// When the commit ran (wall clock).
    pub at: i64,
}

/// A runner's economic profile — `sr:profile:{obj}` (§3.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "SrProfileWire")]
pub struct SrProfile {
    /// Whose it is. The identity rule: `obj` must equal the document key (DL-07).
    pub obj: ObjId,
    /// Opened at chargen approval or migration.
    pub account: Option<AccountId>,
    /// The tier the character pays for.
    pub lifestyle: LifestyleTier,
    /// The `sr.month` obligation — **a cache**. The record self-identifies by
    /// `cause: Ruleset { "sr.lifestyle" }` plus a credit to `system:sr.lifestyle`
    /// (DL-07), and boot re-derives this field from that.
    pub upkeep: Option<ObligationId>,
    /// Consecutive unpaid periods, keyed on the `sr.month` ordinal of the last miss
    /// (never wall time), carried across a downgrade replacement (M11).
    ///
    /// `u32` since the E6a verify fold (M3): a `u8` saturated at 255 and then stopped
    /// counting, and it was also being read as the miss high-water mark — see
    /// [`SrProfile::missed_seen`], which is now that mark and nothing else.
    pub unpaid_periods: u32,
    /// The highest `Obligation.missed_total` this profile has already accounted for
    /// (E6a verify fold, M3).
    ///
    /// **Per record, not per character.** A downgrade *replaces* the obligation, and
    /// the replacement's `missed_total` restarts at 0 — so comparing it against the
    /// carried `unpaid_periods` swallowed every later miss and parked the character at
    /// `Delinquent`, out of the due pool, with the ladder and the rent both stopped.
    /// Reset to 0 whenever a record is created or replaced (`open_upkeep`), and
    /// compared against `missed_total` alone.
    #[serde(default)]
    pub missed_seen: u32,
    /// The `sr.month` ordinal the counters are keyed on (E5c T3).
    pub unpaid_since: i64,
    /// What `+lifestyle/pay` posts, in minor units.
    pub arrears: Amount,
    /// The chargen allotment (M12).
    pub minted: Option<Mint>,
    /// The migration mint (M13, answer 18).
    pub migrated: Option<Migration>,
    /// Bumped by every write.
    pub revision: u64,
}

/// The persisted shape of [`SrProfile`].
#[derive(Clone, Debug, Deserialize)]
struct SrProfileWire {
    obj: u32,
    #[serde(default)]
    account: Option<AccountId>,
    #[serde(default)]
    lifestyle: LifestyleTier,
    #[serde(default)]
    upkeep: Option<ObligationId>,
    #[serde(default)]
    unpaid_periods: u32,
    #[serde(default)]
    missed_seen: u32,
    #[serde(default)]
    unpaid_since: i64,
    #[serde(default = "amount_zero")]
    arrears: Amount,
    #[serde(default)]
    minted: Option<Mint>,
    #[serde(default)]
    migrated: Option<Migration>,
    #[serde(default)]
    revision: u64,
}

/// `Amount` has no `Default` (checked-only arithmetic, E2a §6.1), so the wires name
/// their zero explicitly.
fn amount_zero() -> Amount {
    Amount::ZERO
}

/// Why a persisted record could not become a live one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordError(pub &'static str);

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for RecordError {}

impl TryFrom<SrProfileWire> for SrProfile {
    type Error = RecordError;

    fn try_from(w: SrProfileWire) -> Result<SrProfile, RecordError> {
        if w.obj == 0 {
            return Err(RecordError("a profile has no owner"));
        }
        if w.arrears.is_negative() {
            return Err(RecordError("a profile's arrears are negative"));
        }
        // D8: `Mint.key` is a persisted, rendered string, so it is capped on the way
        // in exactly like every other stored string in this crate.
        if let Some(m) = &w.minted
            && (m.amount.is_negative() || m.key.is_empty() || m.key.len() > MAX_MINT_KEY_BYTES)
        {
            return Err(RecordError("a profile's mint record is malformed"));
        }
        Ok(SrProfile {
            obj: ObjId(w.obj),
            account: w.account,
            lifestyle: w.lifestyle,
            upkeep: w.upkeep,
            unpaid_periods: w.unpaid_periods,
            missed_seen: w.missed_seen,
            unpaid_since: w.unpaid_since,
            arrears: w.arrears,
            minted: w.minted,
            migrated: w.migrated,
            revision: w.revision.max(1),
        })
    }
}

impl SrProfile {
    /// A fresh profile for `obj` at the default tier.
    #[must_use]
    pub fn new(obj: ObjId) -> SrProfile {
        SrProfile {
            obj,
            account: None,
            lifestyle: LifestyleTier::default(),
            upkeep: None,
            unpaid_periods: 0,
            missed_seen: 0,
            unpaid_since: 0,
            arrears: Amount::ZERO,
            minted: None,
            migrated: None,
            revision: 1,
        }
    }

    /// Whether this character owes anything (the `+lifestyle/set` gate — D-E6-23).
    #[must_use]
    pub fn in_arrears(&self) -> bool {
        self.unpaid_periods > 0 || self.arrears.minor() > 0
    }
}

/// Who may see a board entry (§3.1, M5). **Enforced ruleset-side**, never through
/// the contract's `Audience`: a faction is not a principal, and `may_view_contract`'s
/// audience clause is `acts_for`, which for an organization means spenders and not
/// members.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardVisibility {
    /// Any profiled character.
    Public,
    /// Members of one faction, through `mush_faction`'s own predicates.
    Faction(FactionId),
    /// One party's members and invitees.
    Invitation {
        /// The invited party.
        party: u64,
    },
}

impl BoardVisibility {
    /// The word `+run/list` filters on.
    #[must_use]
    pub fn word(&self) -> &'static str {
        match self {
            BoardVisibility::Public => "public",
            BoardVisibility::Faction(_) => "faction",
            BoardVisibility::Invitation { .. } => "invite",
        }
    }
}

/// Where a listing is in its life (§3.2 "Board"), mirrored from the contract's own
/// `ContractChanged` by the board observer — never decided independently.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Offered with escrow placed, nobody crewed.
    #[default]
    Listed,
    /// The crew was added and re-offered; the runners have not all accepted.
    Crewing,
    /// Every runner accepted; the contract is `Active`.
    Crewed,
    /// The contract reached a terminal state.
    Closed,
}

impl RunState {
    /// The rendered word.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            RunState::Listed => "listed",
            RunState::Crewing => "crewing",
            RunState::Crewed => "crewed",
            RunState::Closed => "closed",
        }
    }

    /// Whether the listing is still open for applications.
    #[must_use]
    pub fn is_open(self) -> bool {
        matches!(self, RunState::Listed)
    }

    /// Whether the listing has finished.
    #[must_use]
    pub fn is_closed(self) -> bool {
        matches!(self, RunState::Closed)
    }
}

/// The board's **own** notice — what `+run/show` renders, and never the terms (M5,
/// G7). Copied at post; the contract's own notice is the economy's to render.
///
/// Every field is bounded on the way in by [`RunTemplate`]'s own `TryFrom`
/// ([`check_notice`]): the name by [`MAX_RUN_NAME_BYTES`], the kinds by
/// [`MAX_NOTICE_KINDS`] × [`MAX_NOTICE_KIND_BYTES`], and the window by
/// [`MAX_RUN_WINDOW_SECS`] (E6a verify fold, D8/R6). Everything here is rendered to
/// whoever can see the listing, so nothing here is an uncapped stored string.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunNotice {
    /// The listing's name.
    pub name: SmolStr,
    /// When it lapses, in seconds on `RealUtc`.
    pub window: i64,
    /// The *kinds* of condition it settles on (E6a: always `attest`).
    pub kinds: Vec<SmolStr>,
    /// The visible representative, when the poster announced one (answer 4, G7).
    #[serde(default)]
    pub johnson: Option<PrincipalId>,
}

/// One board listing — `sr:run:{id}` (§3.1). Money, authority and consent are E4b2's
/// and are never duplicated here.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RunTemplateWire")]
pub struct RunTemplate {
    /// The listing id.
    pub id: RunId,
    /// The contract that holds the money, the consent and the authority.
    pub contract: ContractId,
    /// The accountable payer: `Object` or `Organization` — never `System` (D-E6-4).
    pub principal: PrincipalId,
    /// The account the escrow came from (rendered only to viewers of that account).
    pub account: AccountId,
    /// The board's own notice.
    pub notice: RunNotice,
    /// Who may see it.
    pub board: BoardVisibility,
    /// The `Faction(_)` payload, denormalized for `+run/list faction`.
    #[serde(default)]
    pub faction: Option<FactionId>,
    /// How many runners it seats.
    pub slots: u8,
    /// The escrowed total, in minor units (rendered only to viewers of `account`).
    pub pay: Amount,
    /// The fixer's fee, in minor units (0 when there is no fixer).
    pub fee: Amount,
    /// The expense advance, in minor units.
    pub advance: Amount,
    /// The fixer's account, when a fee row exists.
    #[serde(default)]
    pub fixer_account: Option<AccountId>,
    /// The outcome row names, in table order.
    pub outcomes: Vec<SmolStr>,
    /// When it was posted (wall clock).
    pub posted_at: i64,
    /// The contract's own deadline (`RealUtc`) — nothing else expires an `Offered`
    /// contract holding escrow (M8).
    pub deadline: EconomicDeadline,
    /// That deadline's instant.
    pub expires_at: i64,
    /// Parties that applied and await the payer's answer (≤ [`MAX_RUN_APPLICANTS`]),
    /// in application order. `+run/crew accept` answers the first unless told
    /// otherwise; `decline` clears them.
    #[serde(default)]
    pub applied: Vec<u64>,
    /// The crewed party, once one was accepted.
    #[serde(default)]
    pub party: Option<u64>,
    /// The board's mirror of the contract's state.
    #[serde(default)]
    pub state: RunState,
    /// Bumped by every write.
    pub revision: u64,
}

/// The persisted shape of [`RunTemplate`].
#[derive(Clone, Debug, Deserialize)]
struct RunTemplateWire {
    id: u64,
    contract: u64,
    principal: PrincipalId,
    account: AccountId,
    notice: RunNotice,
    board: BoardVisibility,
    #[serde(default)]
    faction: Option<FactionId>,
    #[serde(default)]
    slots: u8,
    #[serde(default = "amount_zero")]
    pay: Amount,
    #[serde(default = "amount_zero")]
    fee: Amount,
    #[serde(default = "amount_zero")]
    advance: Amount,
    #[serde(default)]
    fixer_account: Option<AccountId>,
    #[serde(default)]
    outcomes: Vec<SmolStr>,
    #[serde(default)]
    posted_at: i64,
    deadline: EconomicDeadline,
    #[serde(default)]
    expires_at: i64,
    #[serde(default)]
    applied: Vec<u64>,
    #[serde(default)]
    party: Option<u64>,
    #[serde(default)]
    state: RunState,
    #[serde(default)]
    revision: u64,
}

impl TryFrom<RunTemplateWire> for RunTemplate {
    type Error = RecordError;

    fn try_from(w: RunTemplateWire) -> Result<RunTemplate, RecordError> {
        if matches!(w.principal, PrincipalId::System(_)) {
            return Err(RecordError("a listing names a System payer (D-E6-4)"));
        }
        check_notice(&w.notice)?;
        if w.outcomes.is_empty() || w.outcomes.len() > MAX_RUN_OUTCOMES {
            return Err(RecordError("a listing has no usable outcome table"));
        }
        // D2: an outcome row name is frozen into the consent surface, copied here,
        // and broadcast by `+run/show`. `commands::outcome_names` capped the count and
        // the duplicates and not the bytes, so a 4 kB row name reached the board and
        // the contract alike; the economy refuses one at `check_structure` now, and
        // this is the load-path half of the same rule.
        if w.outcomes
            .iter()
            .any(|n| n.trim().is_empty() || n.len() > mush_economy::MAX_OUTCOME_NAME_BYTES)
        {
            return Err(RecordError(
                "a listing's outcome row name is empty or too long",
            ));
        }
        if w.slots == 0 || w.slots > MAX_RUN_SLOTS {
            return Err(RecordError("a listing seats an impossible crew"));
        }
        if w.pay.is_negative() || w.fee.is_negative() || w.advance.is_negative() {
            return Err(RecordError("a listing carries a negative amount"));
        }
        if w.applied.len() > MAX_RUN_APPLICANTS {
            return Err(RecordError("a listing carries too many applications"));
        }
        let matched = match (&w.board, w.faction) {
            (BoardVisibility::Faction(f), Some(g)) => *f == g,
            (BoardVisibility::Faction(_), None) => false,
            (_, None) => true,
            (_, Some(_)) => false,
        };
        if !matched {
            return Err(RecordError(
                "a listing's denormalized faction disagrees with its board",
            ));
        }
        Ok(RunTemplate {
            id: RunId(w.id),
            contract: ContractId(w.contract),
            principal: w.principal,
            account: w.account,
            notice: w.notice,
            board: w.board,
            faction: w.faction,
            slots: w.slots,
            pay: w.pay,
            fee: w.fee,
            advance: w.advance,
            fixer_account: w.fixer_account,
            outcomes: w.outcomes,
            posted_at: w.posted_at,
            deadline: w.deadline,
            expires_at: w.expires_at,
            applied: w.applied,
            party: w.party,
            state: w.state,
            revision: w.revision.max(1),
        })
    }
}

/// Every bound a persisted [`RunNotice`] must satisfy (E6a verify fold, D8/R6).
///
/// # Errors
///
/// The first bound it breaks, in words.
fn check_notice(n: &RunNotice) -> Result<(), RecordError> {
    if n.name.is_empty() || n.name.len() > MAX_RUN_NAME_BYTES {
        return Err(RecordError("a listing's name is empty or too long"));
    }
    if n.kinds.len() > MAX_NOTICE_KINDS {
        return Err(RecordError("a listing's notice names too many kinds"));
    }
    if n.kinds
        .iter()
        .any(|k| k.trim().is_empty() || k.len() > MAX_NOTICE_KIND_BYTES)
    {
        return Err(RecordError("a listing's notice kind is empty or too long"));
    }
    if n.window < 0 || n.window > MAX_RUN_WINDOW_SECS {
        return Err(RecordError("a listing's window is out of range"));
    }
    Ok(())
}

/// Clip a display string to a bounded length on a char boundary.
#[must_use]
pub fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile_json(obj: u32) -> serde_json::Value {
        serde_json::json!({ "obj": obj, "lifestyle": "middle", "revision": 3 })
    }

    #[test]
    fn a_profile_round_trips_through_its_wire() {
        let p: SrProfile =
            serde_json::from_value(profile_json(7)).expect("a well-formed profile parses");
        assert_eq!(p.obj, ObjId(7));
        assert_eq!(p.lifestyle, LifestyleTier::Middle);
        assert_eq!(p.revision, 3);
        let back = serde_json::to_value(&p).expect("serializes");
        let again: SrProfile = serde_json::from_value(back).expect("round trip");
        assert_eq!(again, p);
    }

    #[test]
    fn a_profile_with_no_owner_or_negative_arrears_is_refused() {
        assert!(serde_json::from_value::<SrProfile>(profile_json(0)).is_err());
        let bad = serde_json::json!({ "obj": 4, "arrears": -5 });
        assert!(serde_json::from_value::<SrProfile>(bad).is_err());
    }

    #[test]
    fn clip_never_splits_a_char() {
        assert_eq!(clip("abcdef", 3), "abc");
        assert_eq!(clip("\u{a5}\u{a5}\u{a5}", 3), "\u{a5}");
    }
}
