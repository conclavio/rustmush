//! **The salvage claim** (doc 36 cut GC.6; Q-36-24 to Q-36-28, Q-36-30; doc 27 FO-1, FO-4,
//! Q-S1–Q-S4, Q-S13–Q-S15, Q-F3, Q-R1, P-3).
//!
//! A hull loaded into a carrier that is not its own party's or a Friend's sits in the carrier
//! operator's custody with a claim owed (GC.5, [`crate::surface::salvage`]). This module is
//! that claim's life, the host of `mush_tactics`' salvage library made live and durable:
//!
//! - **The tenure pin** (Q-S3, Q-36-26). When a tug starts an attach, the ground's tenure at
//!   the casualty's cell, the casualty's title and who saw it are pinned on the site's record
//!   ([`TenurePin`], in [`crate::surface::tow::Tows::pins`]). The pin is kept across the same
//!   tug's detach and reattach, a tow a field interrupted, and a restart with the attachment
//!   intact; a `release`, or another tug's attach, starts it again. The load carries it into
//!   the custody record.
//! - **Filing** (Q-36-24, Q-36-25). `+salvage/claim <hull> go` by the custodian files an
//!   economy contract at the site's port registry (the Freeport Registry for every frontier
//!   site): `mush_economy::file_salvage_claim`, a `RealUtc` contest window of
//!   `[tacsim.recovery].claim_window_ic_days` (48 IC days, filed in real seconds at the world's rate), which the world clock's outages pause
//!   through the economy's own `contract_live`. A written-off lease rig is Occator's alone and is
//!   refused at filing (Q-36-29). The custody record's seam goes from `Owed` to
//!   `Filed { contract }`, and the library's [`SalvageClaim`] is kept beside it. **Notice** goes
//!   by IC mail to the title holder and, on owned ground, to the landholder ([`ClaimMail`]); it
//!   is never a broadcast (doc 27 Q-S4).
//! - **Evidence** (Q-36-28). A party's own sensor and `known` records (the pin's witnesses: a
//!   unit of theirs in sight of the casualty, or their own `known` row placing it there, when
//!   the attach began), the durable surface black box read with bay access
//!   (`+salvage/inspect`, P-3: aboard the carrier that holds the hull), and reports delivered by
//!   another holder (`+salvage/report <hull>=<who>`). The RAM tape in `SalvageClaimBook` is
//!   retired for the ground. GC.5's `picked:unlawful` outcomes, an unlawful recovery and an
//!   unlawful scrapping are written to the site's evidence log
//!   ([`crate::surface::site_record::SiteRecord::unlawful`]) for a later crime system: evidence
//!   only, no enforcement verb (Q-36-30).
//! - **Contest and adjudication** (Q-36-27). `+salvage/contest <hull> go` files the
//!   challenger's best ground through the library's rules ([`SalvageClaim::contest`]): proven
//!   unlawful recovery with evidence they hold, else bare prior title, which never defeats a
//!   lawful claim (Q-R1). A contest that would vacate settles the claim at once; otherwise the
//!   window runs out and the claim settles by [`SalvageClaim::adjudicate`]. An uncontested claim
//!   becomes title: the registry moves it through the economy's journal
//!   (`mush_economy::award_salvage_title`), so GC.8b's rights table then reads the claimant as
//!   the title holder. A vacated claim's contract is closed and the custody record goes: the
//!   title never moved.
//! - **Remains scrapped where they lie** (GC.3's follow-up, Q-36-39). `+tv/<n> scrap [n] [go]`
//!   takes remains lying within a cell of the rig off the site's record and mints their
//!   structural mass as one `structural_alloys` lot at `SCRAP_KG_PER_T` in the same bundle, into
//!   the hold of the rig's own carrier (provisional). It is open to the hull's title holder,
//!   which a winning claimant is once the award lands; untitled remains are open to anyone, and
//!   the evidence log records it on owned ground for anyone but the landholder.
//!
//! **The claim's close** (doc 41 cut RS.3; Q-41-20 to Q-41-23, Q-41-28, Q-38-19, Q-S13, Q-S14):
//!
//! - **Who answers for a title** ([`answerers_of`]): a character's own title is theirs; an
//!   organization's is answered by everyone it names — its live rig lessee first, then its
//!   active members and its grantees, unfiltered by capability (Q-41-28: an NPC organization's
//!   only grantee is its house agent, the lessor agent or a clerk). Each is told at filing and
//!   any may contest; the pin's `answering` is the first. An organization's hull with no live
//!   lease or charter is claimable and awardable (Q-41-22).
//! - **`+salvage/authorize`** (Q-41-21): the title holder's authorization is the existing
//!   `unit.recover_load` grant, issued per hull through Org's unit authority; the landholder's
//!   is a recovery delegation row on the site's record ([`LandGrant`]), bound to the block's
//!   live claim. Both are read **at the attach** into the pin, never later.
//! - **The contest kinds** (Q-41-23): `+salvage/contest <hull>=recovery|procedure|right`, the
//!   bare form keeping the automatic choice. `recovery` rests on the challenger's own sensor or
//!   witness record; `procedure` on the registry's own records — the audience against who
//!   answered at the attach, the pin, and the hull's black box read against the custody row's
//!   tug and carrier, never against the claimant; `right` on an authorization that bound the
//!   claimant at the attach, filed by its grantor (the library vacates only when the obligor is
//!   the claimant; a third party's authorization is no ground).
//! - **Repairs at risk** (Q-41-20, Q-S14): a bay order a salvor completed is written onto the
//!   custody row's claim by the claims' tick and listed in the award and vacate notices; nothing
//!   is repaid.
//! - **Hot parts** (§4.6): a lot stripped under a pending claim is listed on the hull's bay
//!   record; the tick flags each through `strip::vacate_claim` when the claim is vacated.
//!
//! **Where it is written.** One action writes the site's record: GC.4's `tacsim_surface_tow`,
//! with two more ops, `claim` (file, contest, report, inspect and settle) and `scrap`. The
//! commands submit as the actor; the surface tick's claim tick submits a settlement as the
//! custodian once the economy says the window has run (or a contest vacates it), after the
//! title award or the contract's close has posted.

use std::sync::Arc;

use mush_core::{CanonicalId, CmdError, Ctx, ObjId, World};
use mush_economy::{ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use mush_tactics::field_ops::{
    AtRiskRepairEntry, ContestGround, CustodyTitleStatus, GroundTenure, RecoveryAuthorityContext,
    RecoveryOpId, SalvageClaim, SalvageClaimId, SalvageClaimState, SalvageEvidenceKind,
    SalvageEvidenceRecord, SalvageSubject, evaluate_recovery_entitlement,
};
use serde::{Deserialize, Serialize};

use crate::surface::excursion::{cells_apart, site_cell};
use crate::surface::salvage::{ClaimSeam, SalvageCustody};
use crate::surface::site_record::{GroundKind, KnownWhat, SiteKey, SiteRecord};

/// The verb.
pub const SALVAGE_VERB: &str = "+salvage";
/// Its help.
pub const SALVAGE_HELP: &str = "+salvage  --  Salvage claims on hulls recovered into a carrier's custody. +salvage lists the claims you stand in. +salvage/claim <hull> [go] files the custodian's claim at the site's port registry; everyone who answers for the title, and the landholder on owned ground, are told by IC mail. +salvage/contest <hull>[=recovery|procedure|right] [go] contests it: recovery on your own record of an unlawful recovery, procedure on the registry's own records (for whoever answers for the title), right on an authorization of yours that bound the claimant; bare, on the best ground you hold, where bare title never defeats a lawful claim. +salvage/report <hull>=<who> [go] hands your evidence to another. +salvage/inspect <hull> reads the hull's black box from aboard the carrier that holds it. +salvage/authorize <hull>=<who> [until=<UTC seconds>] [go] authorizes a recovery of your hull; +salvage/authorize <site>/<block>=<who> [until=<IC seconds>] [go] a recovery off your block. +salvage/ask <site>/<block> [go] asks a block's landholder for consent to recover off it, by the registry's notice; 28 IC days of silence is consent. +salvage/refuse <site>/<block>=<who> [go] refuses one for 28 IC days. +salvage/forsale lists the hulls the recovery houses have restored for sale, at their reference price; +salvage/buy <hull> [go] buys one, her title passing at the next settlement. Bare, each previews and commits nothing; go commits exactly the preview.";
/// The most witnesses a pin keeps.
pub const MAX_WITNESSES: usize = 16;
/// The most delivered or read evidence records one custody keeps.
pub const MAX_EVIDENCE: usize = 16;
/// The most entries the site's evidence log keeps; past it the oldest goes.
pub const MAX_UNLAWFUL: usize = 64;
/// The salvage award's registered title issuer: the lane's registry
/// (`crate::port::hull::REGISTRY_ISSUER`, already a title issuer on every world).
pub const AWARD_ISSUER: &str = crate::port::hull::REGISTRY_ISSUER;

/// `+salvage` usage.
pub const SALVAGE_USAGE: &str = "Usage: +salvage, +salvage/claim <hull> [go], +salvage/contest <hull>[=recovery|procedure|right] [go], +salvage/report <hull>=<who> [go], +salvage/report <site>[=<who>|#<hull>] registry, +salvage/inspect <hull>, +salvage/forsale, +salvage/buy <hull> [go], +salvage/authorize <hull>|<site>/<block>=<who> [until=<when>] [go], +salvage/ask <site>/<block> [go] or +salvage/refuse <site>/<block>=<who> [go].";
/// Nothing to list.
pub const NO_CLAIMS: &str = "You stand in no salvage claim.";
/// No custody record names the hull: `{hull}`.
pub const NOT_IN_CUSTODY: &str = "No salvage custody names {hull}.";
/// Only the custodian files: `{name}`.
pub const NOT_CUSTODIAN: &str = "Only the {name}'s custodian may file a claim on it.";
/// Already filed: `{name}`, `{contract}`.
pub const ALREADY_FILED: &str = "The claim on the {name} is filed already (contract #{contract}).";
/// Not filed yet: `{name}`.
pub const NOT_FILED: &str = "No claim on the {name} has been filed yet.";
/// The Occator write-off refusal: `{name}`.
pub const WRITTEN_OFF: &str =
    "The {name} is an Occator write-off: it is Occator's alone, and no claim on it is filed.";
/// No title to move: `{name}`.
pub const NO_TITLE: &str =
    "The {name} carries no title the registry can name, so no claim on it can be filed.";
/// The claim preview: `{name}`, `{registry}`, `{window}`, `{told}`.
pub const CLAIM_PREVIEW: &str = "You would file a salvage claim on the {name} at the {registry}: a contest window of {window}, the world clock's outages excluded. {told} +salvage/claim {hull} go";
/// The claim filed: `{name}`, `{registry}`, `{contract}`, `{window}`.
pub const CLAIM_FILED: &str = "You file a salvage claim on the {name} at the {registry} (contract #{contract}). It becomes your title if nobody defeats it within {window}.";
/// The notice: `{name}`, `{claimant}`, `{registry}`, `{site}`, `{window}`, `{contract}`.
pub const NOTICE_BODY: &str = "{claimant} has filed a salvage claim on the {name}, recovered at {site}, at the {registry} (contract #{contract}). It becomes {claimant}'s title unless it is defeated within {window}. +salvage/contest {hull} go contests it.";
/// The notice's subject: `{name}`.
pub const NOTICE_SUBJECT: &str = "Salvage claim: the {name}";
/// The contest preview on evidence: `{name}`, `{kind}`.
pub const CONTEST_PREVIEW_EVIDENCE: &str = "You would contest the claim on the {name} on your evidence of an unlawful recovery ({kind}). +salvage/contest {hull} go";
/// The contest preview on bare title: `{name}`.
pub const CONTEST_PREVIEW_TITLE: &str = "You would contest the claim on the {name} on your bare title. Bare title never defeats a lawful claim; only evidence of an unlawful recovery does. +salvage/contest {hull} go";
/// Nothing to contest on: `{name}`.
pub const NO_GROUND: &str =
    "You hold neither evidence of how the {name} was recovered nor its title.";
/// The contest filed: `{name}`.
pub const CONTEST_FILED: &str = "You file your contest of the claim on the {name}.";
/// The window is closed: `{name}`.
pub const WINDOW_CLOSED: &str = "The contest window on the {name} has closed.";
/// The window is paused: `{name}`.
pub const WINDOW_PAUSED: &str = "The registry's books are closed while the world clock is stopped; the {name}'s contest window is paused.";
/// No evidence to hand on: `{name}`.
pub const NO_EVIDENCE: &str = "You hold no evidence of how the {name} was recovered.";
/// The report preview: `{name}`, `{who}`.
pub const REPORT_PREVIEW: &str = "You would hand your evidence of the {name}'s recovery to {who}. +salvage/report {hull}={who} go";
/// The report delivered: `{name}`, `{who}`.
pub const REPORT_SENT: &str = "You hand your evidence of the {name}'s recovery to {who}.";
/// No such character: `{who}`.
pub const NO_SUCH_WHO: &str = "There is no one called {who}.";
/// Inspection needs bay access: `{name}`, `{carrier}`.
pub const NO_BAY_ACCESS: &str =
    "The {name}'s black box is read aboard the {carrier}, which holds it.";
/// The inspection: `{name}`, `{lines}`.
pub const INSPECTED: &str =
    "You read the {name}'s black box: {lines}. What you read is your evidence of its recovery.";
/// A field is open where the custody is kept: `{site}`.
pub const FIELD_OPEN: &str = "The registry's books for {site} wait while a field is open there.";
/// The award told: `{name}`, `{claimant}`, `{registry}`.
pub const AWARDED: &str = "The {registry} awards the {name} to {claimant}: the salvage claim stood, and the title is theirs.";
/// The vacate told: `{name}`, `{registry}`, `{why}`.
pub const VACATED: &str = "The {registry} vacates the salvage claim on the {name}: {why}, and the title stays where it was.";
/// Why a vacate: the recovery.
pub const WHY_RECOVERY: &str = "the recovery is proven unlawful";
/// Why a vacate: the procedure.
pub const WHY_PROCEDURE: &str = "the claim's procedure was defective";
/// Why a vacate: a right.
pub const WHY_RIGHT: &str = "the claimant was bound by a prior right to deliver her";
/// The repairs at risk in a notice: `{list}`.
pub const AT_RISK_NOTICE: &str =
    " Repairs the salvor made at their own risk, repaid by nobody: {list}.";
/// The Q-S14 line a salvor's quote prints first.
pub const AT_RISK_QUOTE: &str =
    "At your risk: the claim on her is open, and a failed claim repays nothing.";
/// The contest preview on procedure: `{name}`, `{defect}`.
pub const CONTEST_PREVIEW_PROCEDURE: &str = "You would contest the claim on the {name} on its procedure: the registry's own records show {defect}. +salvage/contest {hull}=procedure go";
/// No defect the registry's records show: `{name}`.
pub const NO_DEFECT: &str = "The registry's own records show no defect in the claim on the {name}: everyone who answered for her title was told, the pin stands, and her black box agrees with the custody.";
/// Only whoever answers for the title contests the procedure: `{name}`.
pub const NOT_ANSWERING: &str =
    "Only someone who answers for the {name}'s title may contest the claim's procedure.";
/// The contest preview on a right: `{name}`, `{claimant}`, `{right}`.
pub const CONTEST_PREVIEW_RIGHT: &str = "You would contest the claim on the {name} on your prior right: {right}, which bound {claimant} to deliver her. +salvage/contest {hull}=right go";
/// No right binding the claimant: `{name}`, `{claimant}`.
pub const NO_RIGHT: &str = "You hold no right that bound {claimant} over the {name} when the recovery began: an authorization of your own, or one {claimant} never held, is no ground against their claim.";
/// Nothing to contest the recovery on: `{name}`.
pub const NO_RECOVERY_GROUND: &str = "You hold no record of how the {name} was recovered.";
/// A contest kind not known: `{kind}`.
pub const NO_SUCH_KIND: &str =
    "A contest is on recovery, procedure or right; {kind} is none of them.";
/// The authorize preview for a hull: `{name}`, `{who}`, `{until}`, `{hull}`, `{until_arg}`.
pub const AUTHORIZE_HULL_PREVIEW: &str = "You would authorize {who} to recover the {name} and load her{until}; a claim they file on her afterwards is defeated by your right. +salvage/authorize {hull}={who}{until_arg} go";
/// The authorize sent for a hull: `{name}`, `{who}`.
pub const AUTHORIZE_HULL_SENT: &str =
    "You authorize {who} to recover the {name}; Org's unit authority records the grant.";
/// Not the hull's title holder: `{name}`.
pub const NOT_TITLED: &str =
    "You do not answer for the {name}'s title, so its recovery is not yours to authorize.";
/// The authorize preview for a block: `{block}`, `{who}`, `{until}`, `{until_arg}`.
pub const AUTHORIZE_BLOCK_PREVIEW: &str = "You would authorize {who} to recover hulls off your block {block}{until}, while your claim on it stands. +salvage/authorize {block}={who}{until_arg} go";
/// The authorize written for a block: `{block}`, `{who}`.
pub const AUTHORIZE_BLOCK_SENT: &str =
    "You authorize {who} to recover hulls off your block {block}.";
/// Not the block's holder: `{block}`.
pub const NOT_LANDHOLDER: &str = "You hold no live claim on block {block}.";
/// Doc 41 cut RS.10, the ask's preview: `{block}`, `{who}`, `{days}`.
pub const ASK_PREVIEW: &str = "You would ask {who}, who holds block {block}, for consent to recover hulls off it. The registry's notice reaches them now or when they next connect; their silence for {days} IC days is consent. Add go to ask.";
/// The ask sent: `{block}`, `{who}`, `{days}`.
pub const ASK_SENT: &str = "The registry has sent {who} your ask to recover off block {block}. Unanswered in {days} IC days, it is consent.";
/// Consent already stands: `{block}`.
pub const ASK_GIVEN: &str = "You may already recover off block {block}.";
/// An ask already waits: `{block}`.
pub const ASK_PENDING: &str =
    "Your ask to recover off block {block} is still waiting on its landholder.";
/// A refusal stands: `{block}`.
pub const ASK_REFUSED: &str =
    "Block {block}'s landholder has refused you; you may ask again when the refusal lapses.";
/// The landholder's notice: `{who}`, `{block}`, `{days}` (within the economy's 240-byte notice).
pub const CONSENT_NOTICE: &str = "Registry: {who} asks to recover hulls off your block {block}. +salvage/authorize or +salvage/refuse {block}={who}; {days} IC days of silence is consent.";
/// The refusal's preview: `{block}`, `{who}`, `{days}`.
pub const REFUSE_PREVIEW: &str = "You would refuse {who} recovery off block {block} for {days} IC days, and void any authorization you gave them there. Add go to refuse.";
/// The refusal sent: `{block}`, `{who}`, `{days}`.
pub const REFUSE_SENT: &str = "The registry records your refusal: {who} may not recover off block {block} for {days} IC days.";
/// The asker's notice of a refusal: `{who}`, `{block}`, `{days}`.
pub const REFUSED_NOTICE: &str =
    "Registry: {who} refuses you recovery off block {block} for {days} IC days.";
/// A claim filed off a block whose landholder refused the claimant: `{block}`.
pub const CLAIM_REFUSED: &str = "Block {block}'s landholder has refused you recovery there; the registry will not take your claim while the refusal stands.";
/// No such block: `{block}`.
pub const NO_SUCH_BLOCK: &str = "There is no block {block} on the lane's sites.";
/// The most recovery delegations one site's record keeps.
pub const MAX_LAND_GRANTS: usize = 32;
/// The most repairs at risk one claim keeps.
pub const MAX_AT_RISK: usize = 16;

// ---- the pin ------------------------------------------------------------------------------

/// **How a witness saw the casualty** (Q-36-28).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WitnessKind {
    /// A unit of theirs stood in sight of the casualty when the attach began.
    Sensor,
    /// Their own `known` row placed the casualty at its cell when the attach began.
    Known,
}

/// **One witness of an attach** (Q-36-28): who holds the record, and through which unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Witness {
    /// Who holds the record.
    pub holder: ObjId,
    /// The unit that saw it (the casualty itself for a `known` record).
    pub unit: ObjId,
    /// How.
    pub kind: WitnessKind,
}

/// **The tenure pinned when an attach began** (doc 27 Q-S3, doc 36 Q-36-26): the ground's
/// tenure at the casualty's cell, its title then, and who saw it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TenurePin {
    /// The casualty.
    pub casualty: ObjId,
    /// The tug whose attach pinned it.
    pub tug: ObjId,
    /// The tug's operator.
    pub recoverer: ObjId,
    /// The casualty's title holder then, when it had a title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<PrincipalId>,
    /// Who answers for that title first: the character who holds it, or, for an
    /// organization's title, the first of [`TenurePin::answerers`] (doc 41 Q-41-28).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answering: Option<ObjId>,
    /// **Everyone who answered for that title then** (doc 41 Q-41-22, Q-41-28;
    /// [`answerers_of`]), at most [`MAX_WITNESSES`]: empty for a character's title, whose one
    /// answerer is `answering`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answerers: Vec<ObjId>,
    /// **The title holder's authorization the recoverer held then** (doc 41 Q-41-21): their
    /// live `unit.recover_load` grant on the casualty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_grant: Option<PinnedGrant>,
    /// **Whether the landholder's recovery delegation covered the recoverer then** (doc 41
    /// Q-41-21; [`LandGrant`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub landholder_authorized: bool,
    /// The ground's tenure at the casualty's cell.
    pub tenure: GroundTenure,
    /// Who saw it, at most [`MAX_WITNESSES`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub witnesses: Vec<Witness>,
    /// When it was pinned, IC.
    pub pinned_ic: i64,
}

/// **A recovery authorization a pin read** (doc 41 Q-41-21): the `unit.recover_load` grant on
/// the casualty, and who issued it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinnedGrant {
    /// The grant's id in Org's unit authority.
    pub grant: u64,
    /// Who issued it: the one whose right it is.
    pub grantor: ObjId,
}

/// **A landholder's recovery delegation** (doc 41 Q-41-21): `to` may recover hulls off the
/// block `letter` while the claim it rides — `landholder`'s, registered at `registered_ic` —
/// stands, until `until_ic` when set. Kept on the site's record
/// ([`crate::surface::site_record::SiteRecord::land_grants`]), which the claim's own action
/// writes; a new holding of the block voids it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandGrant {
    /// The block letter.
    pub letter: char,
    /// The claim's holder who issued it.
    pub landholder: ObjId,
    /// The claim's registration instant: the holding it rides.
    pub registered_ic: i64,
    /// Who may recover.
    pub to: ObjId,
    /// When it lapses, IC; `None` while the claim stands.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until_ic: Option<i64>,
    /// When it was issued, IC.
    pub issued_ic: i64,
}

impl LandGrant {
    /// Whether it covers `to` under `claim` at `now_ic`.
    #[must_use]
    pub fn covers(&self, to: ObjId, claim: (ObjId, i64), now_ic: i64) -> bool {
        self.to == to
            && (self.landholder, self.registered_ic) == claim
            && self.until_ic.is_none_or(|until| now_ic < until)
    }
}

/// **How long a landholder's silence or refusal stands**, IC days (doc 41 cut RS.10; the owner's
/// standing rule of 2026-11-04: any wait on a player's response is at least 28 IC days). An ask
/// unanswered this long is consent; a refusal blocks the asker on the block this long.
pub const CONSENT_IC_DAYS: i64 = 28;

/// The same, IC seconds.
pub const CONSENT_IC_SECS: i64 = CONSENT_IC_DAYS * 86_400;

/// **A salvor's ask for a landholder's consent** (doc 41 cut RS.10, the owner's rulings of
/// 2026-11-04): `asker` asked `landholder` — holding block `letter` under the claim registered at
/// `registered_ic` — by the registry's notice at `asked_ic`, to recover hulls off the block. The
/// landholder answers with `+salvage/authorize` (a [`LandGrant`]) or `+salvage/refuse`
/// (`refused_ic`); silence for [`CONSENT_IC_DAYS`] is consent. Kept on the site's record
/// ([`crate::surface::site_record::SiteRecord::consent_asks`]) beside the delegations, and read
/// at the attach as they are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConsentAsk {
    /// The block letter.
    pub letter: char,
    /// The claim's holder asked.
    pub landholder: ObjId,
    /// The claim's registration instant: the holding it rides.
    pub registered_ic: i64,
    /// Who asked.
    pub asker: ObjId,
    /// When the notice went, IC.
    pub asked_ic: i64,
    /// When the landholder refused, IC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refused_ic: Option<i64>,
}

impl ConsentAsk {
    /// Whether it rides `claim` for `asker`.
    #[must_use]
    pub fn of(&self, asker: ObjId, claim: (ObjId, i64)) -> bool {
        self.asker == asker && (self.landholder, self.registered_ic) == claim
    }

    /// Whether the landholder's silence is consent at `now_ic`.
    #[must_use]
    pub fn consented(&self, now_ic: i64) -> bool {
        self.refused_ic.is_none() && now_ic >= self.asked_ic.saturating_add(CONSENT_IC_SECS)
    }

    /// Whether a refusal blocks the asker at `now_ic`.
    #[must_use]
    pub fn refuses(&self, now_ic: i64) -> bool {
        self.refused_ic
            .is_some_and(|at| now_ic < at.saturating_add(CONSENT_IC_SECS))
    }
}

/// **Where a salvor stands with a block's landholder** (doc 41 cut RS.10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Consent {
    /// Open ground, the salvor's own block, a live delegation, or consent by silence.
    Given,
    /// Asked; the landholder has neither answered nor been silent long enough.
    Asked,
    /// Refused, and the refusal stands.
    Refused,
    /// Owned ground and nothing asked (or a refusal lapsed).
    NotAsked,
}

impl TenurePin {
    /// The landholder, on owned ground.
    #[must_use]
    pub fn landholder(&self) -> Option<ObjId> {
        match &self.tenure {
            GroundTenure::Owned { landholder, .. } => Some(*landholder),
            GroundTenure::Unclaimed => None,
        }
    }
}

/// **Pin `pin` on a site's tows** at an attach's start (Q-36-26): an earlier pin by the same
/// tug on the same casualty is kept (a detach and reattach, an interrupted tow); any other is
/// replaced.
pub(crate) fn pin(pins: &mut Vec<TenurePin>, pin: TenurePin) {
    if pins
        .iter()
        .any(|kept| kept.casualty == pin.casualty && kept.tug == pin.tug)
    {
        return;
    }
    pins.retain(|kept| kept.casualty != pin.casualty);
    let at = pins.partition_point(|kept| kept.casualty < pin.casualty);
    pins.insert(at, pin);
    while pins.len() > crate::surface::tow::MAX_TOW_ACTS {
        let oldest = pins
            .iter()
            .enumerate()
            .min_by_key(|(_, kept)| (kept.pinned_ic, kept.casualty))
            .map(|(at, _)| at);
        match oldest {
            Some(at) => {
                pins.remove(at);
            }
            None => break,
        }
    }
}

/// **Who answers for `title`** (doc 41 Q-41-22, Q-41-28): a character's own title is theirs;
/// an organization's is answered by its live rig lessee on `hull` first, then by its active
/// members and its live grantees, unfiltered by capability — an NPC organization's grantee is
/// its house agent (the lessor agent, a clerk), a player organization's its officers — in
/// object order, at most [`MAX_WITNESSES`]. A system principal answers by nobody.
#[must_use]
pub fn answerers_of(w: &World, hull: ObjId, title: &PrincipalId) -> Vec<ObjId> {
    match title {
        PrincipalId::Object(holder) => vec![*holder],
        PrincipalId::System(_) => Vec::new(),
        PrincipalId::Organization(org) => {
            let mut found: Vec<ObjId> = w
                .module_state::<crate::store::TacsimStore>()
                .and_then(|store| store.tv_lease_on(hull))
                .map(|lease| lease.pilot)
                .into_iter()
                .collect();
            let now = w.now();
            let mut named: std::collections::BTreeSet<ObjId> = std::collections::BTreeSet::new();
            if let Some(store) = w.module_state::<mush_org::OrgStore>() {
                let state = store.state();
                named.extend(
                    state
                        .members_in(*org)
                        .filter(|m| m.status == mush_org::MembershipStatus::Active)
                        .map(|m| m.member),
                );
                named.extend(
                    state
                        .grants_in(*org)
                        .filter(|g| {
                            g.valid_from.is_none_or(|from| from <= now)
                                && g.valid_until.is_none_or(|until| now < until)
                        })
                        .map(|g| g.grantee),
                );
            }
            for who in named {
                if who.0 != 0 && !found.contains(&who) {
                    found.push(who);
                }
            }
            found.truncate(MAX_WITNESSES);
            found
        }
    }
}

/// **The title of `hull` now**, and everyone who answers for it ([`answerers_of`]).
pub(crate) fn title_of(w: &World, hull: ObjId) -> (Option<PrincipalId>, Vec<ObjId>) {
    let title = w
        .module_state::<EconomyStore>()
        .and_then(|store| store.snapshot().asset(hull).map(|a| a.title_owner.clone()));
    let answerers = title
        .as_ref()
        .map(|title| answerers_of(w, hull, title))
        .unwrap_or_default();
    (title, answerers)
}

/// **The live claim on the block at `cell` on `site`**: its letter and its record.
fn block_claim_at(
    ctx: &Ctx<'_>,
    site: &CanonicalId,
    cell: (i64, i64),
) -> Option<(char, crate::surface::claims::BlockClaim)> {
    use crate::port::site_map::SurfaceReads;
    let scheme = crate::surface::chart::site_scheme(ctx.w, site);
    let pose = mush_tactics::field_ops::FieldPose {
        site: ObjId(1),
        x_cm: cell.0.saturating_mul(1_000).saturating_add(500),
        y_cm: cell.1.saturating_mul(1_000).saturating_add(500),
    };
    let letter = crate::field_ops::block_letter_at_site_pose(pose, scheme)?;
    let now = ctx.w.world_ic_now().at;
    ctx.block_tenure(&crate::surface::claims::BlockKey {
        site: site.clone(),
        letter,
    })
    .ok()
    .flatten()
    .and_then(|record| record.claim)
    .filter(|claim| claim.live(now))
    .map(|claim| (letter, claim))
}

/// **The ground's tenure at `cell` on `site`** (FO-4, Q-S1): the live claim on its block, when a
/// character holds one. An organization's seed reads as open ground (provisional: a tenure names
/// a character).
pub(crate) fn tenure_at(ctx: &Ctx<'_>, site: &CanonicalId, cell: (i64, i64)) -> GroundTenure {
    match block_claim_at(ctx, site, cell) {
        Some((letter, claim)) => match claim.holder {
            PrincipalId::Object(landholder) if landholder.0 != 0 => GroundTenure::Owned {
                landholder,
                claim_id: Some(smol_str::SmolStr::new(format!(
                    "{site}/{}",
                    letter.to_ascii_lowercase()
                ))),
            },
            _ => GroundTenure::Unclaimed,
        },
        None => GroundTenure::Unclaimed,
    }
}

/// **The `unit.recover_load` grant `ctx.actor` holds on `casualty` now** (doc 41 Q-41-21):
/// Org's unit authority allows it and one of the actor's own grants of the capability covers
/// the casualty. `None` without the authority's reader.
fn recover_grant(ctx: &Ctx<'_>, casualty: ObjId) -> Option<PinnedGrant> {
    use mush_org::unit_authority::reader::{UnitAuthorityRequest, UnitAuthorityResponse};
    use mush_org::unit_authority::{UnitCapability, UnitGrantStatus, UnitGrantee, UnitScope};
    let reader = ctx
        .w
        .module_state::<crate::detachment::planning::PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.clone())?;
    let allowed = matches!(
        ctx.read_owner(
            &reader,
            UnitAuthorityRequest::Authorize {
                unit: casualty,
                capability: UnitCapability::RecoverLoad,
                delegate: false,
                mission: None,
            }
        ),
        Ok(UnitAuthorityResponse::Authority { allowed: true })
    );
    if !allowed {
        return None;
    }
    let now = ctx.w.now();
    let mut after = 0_u64;
    // The registry answers 32 rows a page; a character's own grants are few.
    for _ in 0..8 {
        let Ok(UnitAuthorityResponse::Grants { grants, more }) =
            ctx.read_owner(&reader, UnitAuthorityRequest::List { after })
        else {
            return None;
        };
        if let Some(grant) = grants.iter().find(|g| {
            g.capability == UnitCapability::RecoverLoad
                && g.status == UnitGrantStatus::Active
                && g.grantee == UnitGrantee::Character(ctx.actor)
                && g.grantor != ctx.actor
                && matches!(&g.scope, UnitScope::Units { units } if units.contains(&casualty))
                && g.valid_from.is_none_or(|from| from <= now)
                && g.valid_until.is_none_or(|until| now < until)
        }) {
            return Some(PinnedGrant {
                grant: grant.id.0,
                grantor: grant.grantor,
            });
        }
        match grants.last() {
            Some(last) if more => after = last.id.0,
            _ => return None,
        }
    }
    None
}

/// **Who saw `casualty` at `cell`** (Q-36-28): the principal behind every other unit on the
/// site's ground within sight of it (its party's owner, else its leader, else its pilot), and
/// every principal whose own `known` row places a casualty or a wreck at that cell. The tug,
/// the casualty and the recoverer's own units are never witnesses.
fn witnesses(
    ctx: &Ctx<'_>,
    record: &SiteRecord,
    site: &CanonicalId,
    (casualty, tug): (ObjId, ObjId),
    cell: (i64, i64),
) -> Vec<Witness> {
    let mut near: Vec<ObjId> = Vec::new();
    if let Some(units) = ctx.w.module_state::<mush_tactics::UnitStore>() {
        for unit in units.units.keys().copied() {
            if unit == casualty || unit == tug {
                continue;
            }
            let Some((there, pose)) = crate::surface::tow::ground_pose(ctx.w, unit) else {
                continue;
            };
            if &there == site
                && cells_apart(cell, site_cell(pose.x_cm, pose.y_cm))
                    <= crate::surface::presence::SIGHT_CELLS
            {
                near.push(unit);
            }
        }
    }
    // A carrier on the pad is berthed, not on the ground: it sees from its ramp.
    for hull in crate::surface::recovery::world_hulls(ctx.w) {
        if hull == casualty || hull == tug || near.contains(&hull) {
            continue;
        }
        if let Some((entry, _, ramp, _, _)) = crate::surface::tick::hull_view_berth(ctx.w, hull)
            && &entry == site
            && cells_apart(cell, site_cell(ramp.x_cm, ramp.y_cm))
                <= crate::surface::presence::SIGHT_CELLS
        {
            near.push(hull);
        }
    }
    near.sort_unstable();
    let mut found: Vec<Witness> = Vec::new();
    for entrant in crate::field::parties::entrants(ctx, site, &near) {
        let holder = entrant
            .owner
            .or(entrant.leader)
            .or(entrant.pilot.as_ref().map(|(pilot, _)| *pilot));
        if let Some(holder) = holder
            && holder != ctx.actor
            && !found.iter().any(|w| w.holder == holder)
        {
            found.push(Witness {
                holder,
                unit: entrant.unit,
                kind: WitnessKind::Sensor,
            });
        }
    }
    for row in &record.known {
        let placed = row.items.iter().any(|item| {
            matches!(item.0, KnownWhat::Casualty | KnownWhat::Wreck) && item.cell() == cell
        });
        if placed && row.principal != ctx.actor && !found.iter().any(|w| w.holder == row.principal)
        {
            found.push(Witness {
                holder: row.principal,
                unit: casualty,
                kind: WitnessKind::Known,
            });
        }
    }
    found.truncate(MAX_WITNESSES);
    found
}

/// **Where `asker` stands with the landholder of `cell` on `site`** (doc 41 cut RS.10): open
/// ground (or a holder that is not a character) and the asker's own block are [`Consent::Given`];
/// on another's block, a live delegation covering the asker or an ask unanswered past
/// [`CONSENT_IC_DAYS`] is given, a standing refusal refuses, an ask inside its wait is asked.
#[must_use]
pub fn consent_at(
    ctx: &Ctx<'_>,
    record: &SiteRecord,
    site: &CanonicalId,
    cell: (i64, i64),
    asker: ObjId,
) -> Consent {
    let Some((letter, claim)) = block_claim_at(ctx, site, cell) else {
        return Consent::Given;
    };
    let PrincipalId::Object(landholder) = claim.holder else {
        return Consent::Given;
    };
    consent_on(
        record,
        letter,
        (landholder, claim.registered_ic),
        asker,
        ctx.w.world_ic_now().at,
    )
}

/// [`consent_at`] on block `letter` held under `claim` `(landholder, registered_ic)`.
#[must_use]
pub fn consent_on(
    record: &SiteRecord,
    letter: char,
    claim: (ObjId, i64),
    asker: ObjId,
    now_ic: i64,
) -> Consent {
    if asker == claim.0 {
        return Consent::Given;
    }
    if record
        .land_grants
        .iter()
        .any(|grant| grant.letter == letter && grant.covers(asker, claim, now_ic))
    {
        return Consent::Given;
    }
    match record
        .consent_asks
        .iter()
        .find(|ask| ask.letter == letter && ask.of(asker, claim))
    {
        Some(ask) if ask.refuses(now_ic) => Consent::Refused,
        Some(ask) if ask.consented(now_ic) => Consent::Given,
        Some(ask) if ask.refused_ic.is_none() => Consent::Asked,
        _ => Consent::NotAsked,
    }
}

/// **The pin an attach of `casualty` by `tug` starts now** (the command's reading; the
/// start's prepare writes it).
pub(crate) fn pin_for(
    ctx: &Ctx<'_>,
    record: &SiteRecord,
    site: &CanonicalId,
    (tug, casualty): (ObjId, ObjId),
    cell: (i64, i64),
) -> TenurePin {
    let (title, answerers) = title_of(ctx.w, casualty);
    // The landholder's delegation, or (doc 41 cut RS.10) its silence on the recoverer's ask.
    let landholder_authorized = block_claim_at(ctx, site, cell).is_some()
        && consent_at(ctx, record, site, cell, ctx.actor) == Consent::Given;
    TenurePin {
        casualty,
        tug,
        recoverer: ctx.actor,
        answering: answerers.first().copied(),
        // A character's title answers by its holder alone (`answering`); the list is kept for
        // an organization's, so a character's pin prints as it did.
        answerers: if matches!(title, Some(PrincipalId::Organization(_))) {
            answerers
        } else {
            Vec::new()
        },
        title,
        owner_grant: recover_grant(ctx, casualty),
        landholder_authorized,
        tenure: tenure_at(ctx, site, cell),
        witnesses: witnesses(ctx, record, site, (casualty, tug), cell),
        pinned_ic: ctx.w.world_ic_now().at,
    }
}

// ---- the evidence log (Q-36-30) -----------------------------------------------------------

/// **What an unlawful act was** (Q-36-30).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnlawfulKind {
    /// GC.5's `picked:unlawful`: a crate taken off owned ground.
    Pickup,
    /// A hull recovered off owned ground with neither its title holder's nor the landholder's
    /// authority, loaded into a non-friend's bay.
    Recovery,
    /// Untitled remains scrapped off owned ground by anyone but the landholder.
    Scrap,
    /// A salvage claim vacated on proof of an unlawful recovery.
    Vacated,
    /// **Doc 42 cut CR.1** (Q-42-22): a person carried off owned ground by a load with neither the
    /// title holder's nor the landholder's authority; the row's `subject` is the captive, its
    /// `actor` the captor.
    Capture,
}

/// **One unlawful act on a site's ground** (Q-36-30): evidence for a later crime system, never
/// an enforcement.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnlawfulAct {
    /// What it was.
    pub kind: UnlawfulKind,
    /// Who did it.
    pub actor: ObjId,
    /// The hull, or the object number on the site, it was done to.
    pub subject: u32,
    /// Whose ground: the landholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub landholder: Option<ObjId>,
    /// When, IC.
    pub at_ic: i64,
}

/// **Write `act` to the site's evidence log**, past [`MAX_UNLAWFUL`] dropping the oldest.
pub(crate) fn log_unlawful(record: &mut SiteRecord, act: UnlawfulAct) {
    record.unlawful.push(act);
    while record.unlawful.len() > MAX_UNLAWFUL {
        record.unlawful.remove(0);
    }
}

/// **The captives an unlawful recovery carried off** (doc 42 cut CR.1, Q-42-22): one
/// [`UnlawfulKind::Capture`] row each, beside the load's `Recovery` row and under its condition,
/// naming the captor and the captive. Evidence only; doc 41's consumer reads it.
pub(crate) fn log_captures(
    record: &mut SiteRecord,
    captor: ObjId,
    landholder: ObjId,
    captives: &[ObjId],
    at_ic: i64,
) {
    for captive in captives {
        log_unlawful(
            record,
            UnlawfulAct {
                kind: UnlawfulKind::Capture,
                actor: captor,
                subject: captive.0,
                landholder: Some(landholder),
                at_ic,
            },
        );
    }
}

// ---- evidence -----------------------------------------------------------------------------

/// The evidence kind's words.
fn evidence_words(kind: &SalvageEvidenceKind) -> &'static str {
    match kind {
        SalvageEvidenceKind::DirectWitness { .. } => "your own record of the ground",
        SalvageEvidenceKind::SensorLog { .. } => "your own sensor record",
        SalvageEvidenceKind::CustodyInspection { .. } => "an inspection in the bay",
        SalvageEvidenceKind::RecoveredBlackBox { .. } => "the hull's black box",
    }
}

/// **The evidence `who` holds of `custody`'s recovery** (Q-36-28): their own witness record
/// from the pin, then any delivered report or black-box reading of theirs. `None` without a
/// recovery operation to bind it to.
#[must_use]
pub fn evidence_of(custody: &SalvageCustody, who: ObjId) -> Option<SalvageEvidenceRecord> {
    let operation = custody.operation?;
    let pin = custody.pin.as_ref();
    if let Some(witness) = pin.and_then(|pin| pin.witnesses.iter().find(|w| w.holder == who)) {
        let at = pin.map_or(custody.recovered_ic, |pin| pin.pinned_ic);
        return Some(SalvageEvidenceRecord {
            operation_id: operation,
            casualty: custody.hull,
            holder: who,
            kind: match witness.kind {
                WitnessKind::Sensor => SalvageEvidenceKind::SensorLog {
                    sensor_unit: witness.unit,
                    recorded_at_ic: at,
                },
                WitnessKind::Known => SalvageEvidenceKind::DirectWitness {
                    witness: who,
                    observed_at_ic: at,
                },
            },
        });
    }
    custody
        .evidence
        .iter()
        .find(|record| record.holder == who && record.operation_id == operation)
        .cloned()
}

// ---- the claim's orders -------------------------------------------------------------------

/// **How a settled claim ends** (Q-36-27).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Settlement {
    /// The window ran out with no contest that vacates it: the title is the claimant's.
    Award,
    /// A contest proved the recovery unlawful (or the procedure defective): the claim is void.
    Vacate,
    /// The contract was cancelled outside the claim (staff): the claim goes back to owed.
    Withdrawn,
    /// **Doc 36a P0-3** (Q-36-29): the hull was written off while in custody — it is Occator's
    /// alone, handed back to the shed; the contract is closed and the custody row goes, with
    /// no title moved and nothing logged.
    HandedBack,
}

/// **One server-written claim order** (this module's head), carried by GC.4's tow action under
/// its `claim` op, keyed by the hull.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "claim", rename_all = "snake_case")]
pub enum ClaimOrder {
    /// `+salvage/claim <hull> go`.
    File {
        /// The site whose record holds the custody.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// The economy contract filed, `ContractId.0`.
        contract: u64,
        /// The window's effective end, real seconds.
        window_end: i64,
        /// Now, real seconds.
        now_real: i64,
        /// Who was told.
        told: Vec<ObjId>,
    },
    /// `+salvage/contest <hull>[=<kind>] go`.
    Contest {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// The ground, as the command read it.
        ground: ContestGround,
        /// The window's effective end now, real seconds.
        window_end: i64,
        /// Now, real seconds.
        now_real: i64,
    },
    /// `+salvage/report <hull>=<who> go`.
    Report {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// Who receives it.
        to: ObjId,
    },
    /// `+salvage/inspect <hull>`.
    Inspect {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
    },
    /// The tick's settlement.
    Settle {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// How it ends.
        settlement: Settlement,
        /// The window's effective end, real seconds.
        window_end: i64,
        /// Now, real seconds.
        now_real: i64,
    },
    /// **Doc 41 cut RS.3** (Q-41-20): the tick writes a salvor's completed bay order onto the
    /// claim, as the custodian.
    AtRisk {
        /// The site.
        site: CanonicalId,
        /// The hull.
        hull: ObjId,
        /// The repair.
        entry: AtRiskRepairEntry,
    },
    /// **Doc 41 cut RS.3** (Q-41-21): `+salvage/authorize <site>/<block>=<who> go`, keyed by
    /// the block's place.
    AuthorizeBlock {
        /// The site.
        site: CanonicalId,
        /// The block's place object.
        place: ObjId,
        /// The delegation.
        grant: LandGrant,
    },
    /// **Doc 41 cut RS.10**: `+salvage/ask <site>/<block> go` (and Tycho's sweep), keyed by the
    /// block's place: the salvor's ask for the landholder's consent.
    Ask {
        /// The site.
        site: CanonicalId,
        /// The block's place object.
        place: ObjId,
        /// The ask.
        ask: ConsentAsk,
    },
    /// **Doc 41 cut RS.10**: `+salvage/refuse <site>/<block>=<who> go`, the landholder's refusal
    /// (`ask.refused_ic` set), keyed by the block's place.
    Refuse {
        /// The site.
        site: CanonicalId,
        /// The block's place object.
        place: ObjId,
        /// The refused ask.
        ask: ConsentAsk,
    },
}

impl ClaimOrder {
    /// The site whose record it writes.
    #[must_use]
    pub fn site(&self) -> &CanonicalId {
        match self {
            Self::File { site, .. }
            | Self::Contest { site, .. }
            | Self::Report { site, .. }
            | Self::Inspect { site, .. }
            | Self::Settle { site, .. }
            | Self::AtRisk { site, .. }
            | Self::AuthorizeBlock { site, .. }
            | Self::Ask { site, .. }
            | Self::Refuse { site, .. } => site,
        }
    }

    /// The hull it is about (a block delegation's block place).
    #[must_use]
    pub fn hull(&self) -> ObjId {
        match self {
            Self::File { hull, .. }
            | Self::Contest { hull, .. }
            | Self::Report { hull, .. }
            | Self::Inspect { hull, .. }
            | Self::Settle { hull, .. }
            | Self::AtRisk { hull, .. } => *hull,
            Self::AuthorizeBlock { place, .. }
            | Self::Ask { place, .. }
            | Self::Refuse { place, .. } => *place,
        }
    }
}

/// The custody row over `hull` on `record`, mutably.
fn custody_mut(record: &mut SiteRecord, hull: ObjId) -> Option<&mut SalvageCustody> {
    record.custody.iter_mut().find(|row| row.hull == hull)
}

impl TenurePin {
    /// Whether `who` answered for the casualty's title when the attach began (doc 41 Q-41-22).
    #[must_use]
    pub fn answers_for(&self, who: ObjId) -> bool {
        who.0 != 0 && (self.answering == Some(who) || self.answerers.contains(&who))
    }
}

/// **The contest kinds** (doc 41 Q-41-23), as `+salvage/contest <hull>=<kind>` names them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContestKind {
    /// An unlawful recovery, on the challenger's own record.
    Recovery,
    /// A defective procedure, on the registry's own records.
    Procedure,
    /// A superior specific right that bound the claimant.
    Right,
}

impl ContestKind {
    /// The kind a word names.
    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        match word.trim().to_ascii_lowercase().as_str() {
            "recovery" => Some(Self::Recovery),
            "procedure" => Some(Self::Procedure),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// **What the hull's black box says of her recovery** (doc 41 Q-41-23): the last
/// `recovered_to_base` close line's base and recoverer. The registry reads it, never a player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxRecovery {
    /// The carrier the line says she was recovered into.
    pub base: ObjId,
    /// The tug the line names.
    pub recovered_by: Option<ObjId>,
}

/// **The registry's own reading of a claim's procedure** (doc 41 Q-41-23, round-2 item 41-1):
/// a defect code when its records show one — someone who answered for the title at the attach
/// (or the landholder) was not told; the pin is missing or another tug's; the hull's black box
/// names another tug or another carrier than the **custody row's** (`recovered_by`,
/// `carrier`), never compared with the claimant, who is a character. A hull whose box holds no
/// recovery line (a casualty no sortie carried) shows no defect by that absence.
#[must_use]
pub fn procedure_defect(
    custody: &SalvageCustody,
    black_box: Option<BoxRecovery>,
) -> Option<String> {
    let Some(pin) = custody.pin.as_ref() else {
        return Some("pin_missing".to_owned());
    };
    if pin.casualty != custody.hull || pin.tug != custody.recovered_by {
        return Some("pin_replaced".to_owned());
    }
    let owed = pin
        .answering
        .into_iter()
        .chain(pin.answerers.iter().copied())
        .chain(pin.landholder());
    for who in owed {
        if who != custody.holder && !custody.told.contains(&who) {
            return Some(format!("unnotified:{}", who.0));
        }
    }
    match black_box {
        Some(line)
            if line
                .recovered_by
                .is_some_and(|tug| tug != custody.recovered_by) =>
        {
            Some("box_recoverer".to_owned())
        }
        Some(line) if line.base != custody.carrier => Some("box_base".to_owned()),
        _ => None,
    }
}

/// A defect code in words, as the preview prints it.
fn defect_words(w: &World, code: &str) -> String {
    match code.split_once(':') {
        Some(("unnotified", id)) => id.parse::<u32>().map_or_else(
            |_| "someone who answered for her title was never told".to_owned(),
            |id| {
                format!(
                    "{} answered for her title and was never told",
                    name(w, ObjId(id))
                )
            },
        ),
        _ => match code {
            "pin_missing" => "no tenure was pinned when her recovery began".to_owned(),
            "pin_replaced" => "the pin is another tug's than the one that recovered her".to_owned(),
            "box_recoverer" => "her black box names another tug than the custody does".to_owned(),
            "box_base" => "her black box names another carrier than the custody does".to_owned(),
            other => other.to_owned(),
        },
    }
}

/// **The right a pin records that bound the claimant** (doc 41 Q-41-23, round-2 item 41-10):
/// the title holder's `unit.recover_load` grant the recoverer held at the attach, when the
/// recoverer is the claimant and `who` issued it — the contract id `grant/<id>` and the obligor.
/// A grant the claimant never held, or one `who` did not issue, is no ground.
#[must_use]
pub fn right_of_grantor(custody: &SalvageCustody, who: ObjId) -> Option<(String, ObjId)> {
    let pin = custody.pin.as_ref()?;
    let grant = pin.owner_grant?;
    (grant.grantor == who && pin.recoverer == custody.holder)
        .then(|| (format!("grant/{}", grant.grant), custody.holder))
}

/// **The library's claim** a filing opens over `custody` (Q-36-24): the claimant the custodian,
/// the prior owner whoever answers for the title first, the entitlement evaluated from the pinned
/// tenure and the authorizations read at the attach (Q-S1, Q-S3, doc 41 Q-41-21), and the
/// repairs already made at the custodian's risk.
///
/// # Errors
///
/// The refusal code: no title to name, or the library refuses the ids.
fn open_claim(
    custody: &SalvageCustody,
    contract: u64,
    (now_real, window_end): (i64, i64),
) -> Result<SalvageClaim, &'static str> {
    let pin = custody.pin.as_ref();
    let prior = pin.and_then(|pin| pin.answering).ok_or("claim_no_title")?;
    let tenure = pin.map_or(GroundTenure::Unclaimed, |pin| pin.tenure.clone());
    let entitlement = evaluate_recovery_entitlement(&RecoveryAuthorityContext {
        recoverer: custody.holder,
        unit_title_holder: prior,
        has_owner_authorization: pin.is_some_and(|pin| pin.owner_grant.is_some()),
        has_landholder_authorization: pin.is_some_and(|pin| pin.landholder_authorized),
        ground_tenure_at_attach: tenure.clone(),
    })
    .map_err(|_| "claim_no_title")?;
    let operation = custody.operation.ok_or("claim_no_operation")?;
    Ok(SalvageClaim {
        id: SalvageClaimId::new(contract).map_err(|_| "claim_no_contract")?,
        subject: SalvageSubject::Hull { unit: custody.hull },
        excluded_components: std::collections::BTreeSet::new(),
        claimant: custody.holder,
        prior_owner: prior,
        operation_id: operation,
        entitlement_at_attach: entitlement,
        ground_tenure_at_attach: tenure,
        filed_at_real_s: now_real,
        deadline_real_s: window_end,
        paused_at_real_s: None,
        total_paused_real_s: 0,
        contests: Vec::new(),
        documented_repairs_at_risk: custody.at_risk.clone(),
        state: SalvageClaimState::Open,
    })
}

/// **A block's recovery delegation written** (doc 41 Q-41-21): one row a block, landholder and
/// grantee, replaced; past [`MAX_LAND_GRANTS`] the oldest goes.
fn put_land_grant(
    record: &mut SiteRecord,
    actor: ObjId,
    grant: &LandGrant,
) -> Result<(), &'static str> {
    if grant.landholder != actor || grant.to.0 == 0 || grant.to == actor {
        return Err("claim_not_landholder");
    }
    record.land_grants.retain(|kept| {
        !(kept.letter == grant.letter && kept.landholder == grant.landholder && kept.to == grant.to)
    });
    record.land_grants.push(grant.clone());
    while record.land_grants.len() > MAX_LAND_GRANTS {
        record.land_grants.remove(0);
    }
    Ok(())
}

/// **A consent ask or refusal written** (doc 41 cut RS.10): one row a block, landholder and
/// asker, replaced. An ask is the asker's own, refused while a refusal stands; a refusal is the
/// landholder's, and voids any delegation it gave the asker on the block. Past
/// [`MAX_LAND_GRANTS`] the oldest goes.
fn put_ask(
    record: &mut SiteRecord,
    actor: ObjId,
    ask: &ConsentAsk,
    now_ic: i64,
) -> Result<String, &'static str> {
    let same = |kept: &ConsentAsk| {
        kept.letter == ask.letter && kept.landholder == ask.landholder && kept.asker == ask.asker
    };
    if ask.asker.0 == 0 || ask.asker == ask.landholder {
        return Err("claim_not_landholder");
    }
    let word = if ask.refused_ic.is_some() {
        if ask.landholder != actor {
            return Err("claim_not_landholder");
        }
        record
            .land_grants
            .retain(|g| !(g.letter == ask.letter && g.landholder == actor && g.to == ask.asker));
        "refused"
    } else {
        if ask.asker != actor {
            return Err("claim_not_asker");
        }
        if record.consent_asks.iter().any(|kept| {
            same(kept) && kept.registered_ic == ask.registered_ic && kept.refuses(now_ic)
        }) {
            return Err("claim_refused");
        }
        "asked"
    };
    record.consent_asks.retain(|kept| !same(kept));
    record.consent_asks.push(ask.clone());
    while record.consent_asks.len() > MAX_LAND_GRANTS {
        record.consent_asks.remove(0);
    }
    Ok(word.to_owned())
}

/// **A claim order's prepare** (the tow provider's `claim` op): every fact re-checked against
/// the record in the act's own transaction. Answers the outcome's result word.
///
/// # Errors
///
/// The refusal code.
#[allow(clippy::too_many_lines)]
pub(crate) fn prepare(
    record: &mut SiteRecord,
    actor: ObjId,
    order: &ClaimOrder,
    now_ic: i64,
) -> Result<String, &'static str> {
    if let ClaimOrder::AuthorizeBlock { grant, .. } = order {
        put_land_grant(record, actor, grant)?;
        return Ok("authorized".to_owned());
    }
    if let ClaimOrder::Ask { ask, .. } | ClaimOrder::Refuse { ask, .. } = order {
        return put_ask(record, actor, ask, now_ic);
    }
    let hull = order.hull();
    let custody = custody_mut(record, hull).ok_or("claim_no_custody")?;
    match order {
        ClaimOrder::File {
            contract,
            window_end,
            now_real,
            told,
            ..
        } => {
            if custody.holder != actor {
                return Err("claim_not_custodian");
            }
            if custody.claim != ClaimSeam::Owed {
                return Err("claim_filed");
            }
            let claim = open_claim(custody, *contract, (*now_real, *window_end))?;
            custody.claim = ClaimSeam::Filed {
                contract: *contract,
            };
            custody.file = Some(Box::new(claim));
            custody.at_risk.clear();
            custody.told.clone_from(told);
            custody.told.truncate(MAX_WITNESSES);
            Ok("filed".to_owned())
        }
        ClaimOrder::Contest {
            ground,
            window_end,
            now_real,
            ..
        } => {
            match ground {
                ContestGround::UnlawfulRecovery { evidence } => {
                    if evidence_of(custody, actor).as_ref() != Some(evidence) {
                        return Err("claim_no_evidence");
                    }
                }
                ContestGround::BarePriorTitle => {
                    let answers = custody
                        .file
                        .as_ref()
                        .is_some_and(|file| file.prior_owner == actor)
                        || custody
                            .pin
                            .as_ref()
                            .is_some_and(|pin| pin.answers_for(actor));
                    if !answers {
                        return Err("claim_no_ground");
                    }
                }
                // Doc 41 Q-41-23: whoever answers for the title, on a defect the record shows; a
                // black-box defect is the command's own reading of the box (the registry's).
                ContestGround::DefectiveProcedure { defect_code } => {
                    if !custody
                        .pin
                        .as_ref()
                        .is_some_and(|pin| pin.answers_for(actor))
                    {
                        return Err("claim_not_answering");
                    }
                    let shown = procedure_defect(custody, None);
                    let boxed = defect_code.starts_with("box_");
                    if !boxed && shown.as_deref() != Some(defect_code.as_str()) {
                        return Err("claim_no_defect");
                    }
                }
                // Doc 41 Q-41-23 (round-2 item 41-10): only the right's grantor, against the
                // claimant it bound.
                ContestGround::SuperiorSpecificRight {
                    contract_id,
                    obligor,
                } => {
                    let held = right_of_grantor(custody, actor);
                    if held.as_ref().map(|(id, by)| (id.as_str(), *by))
                        != Some((contract_id.as_str(), *obligor))
                    {
                        return Err("claim_no_right");
                    }
                }
            }
            // Doc 36a P0-7: the window's last second is the award's, never a contest's.
            if now_real >= window_end {
                return Err("claim_window_closed");
            }
            let mut file = custody.file.as_deref().cloned().ok_or("claim_not_filed")?;
            file.deadline_real_s = *window_end;
            file.contest(actor, ground.clone(), *now_real)
                .map_err(|_| "claim_contest_refused")?;
            custody.file = Some(Box::new(file));
            Ok("contested".to_owned())
        }
        ClaimOrder::Report { to, .. } => {
            let mut evidence = evidence_of(custody, actor).ok_or("claim_no_evidence")?;
            if *to == actor || to.0 == 0 {
                return Err("claim_no_recipient");
            }
            evidence.holder = *to;
            if !custody.evidence.contains(&evidence) {
                if custody.evidence.len() >= MAX_EVIDENCE {
                    return Err("claim_evidence_full");
                }
                custody.evidence.push(evidence);
            }
            Ok("reported".to_owned())
        }
        ClaimOrder::Inspect { .. } => {
            let operation = custody.operation.ok_or("claim_no_operation")?;
            let read = SalvageEvidenceRecord {
                operation_id: operation,
                casualty: hull,
                holder: actor,
                kind: SalvageEvidenceKind::RecoveredBlackBox {
                    hull,
                    read_at_ic: now_ic,
                },
            };
            if !custody
                .evidence
                .iter()
                .any(|e| e.holder == actor && e.operation_id == operation)
            {
                if custody.evidence.len() >= MAX_EVIDENCE {
                    return Err("claim_evidence_full");
                }
                custody.evidence.push(read);
            }
            Ok("inspected".to_owned())
        }
        // Doc 41 Q-41-20 (Q-S14): onto the filed claim, else kept for its filing; once.
        ClaimOrder::AtRisk { entry, .. } => {
            if custody.holder != actor {
                return Err("claim_not_custodian");
            }
            let list = match custody.file.as_mut() {
                Some(file) => &mut file.documented_repairs_at_risk,
                None => &mut custody.at_risk,
            };
            if list.contains(entry) {
                return Ok("recorded".to_owned());
            }
            if list.len() >= MAX_AT_RISK {
                return Err("claim_at_risk_full");
            }
            list.push(entry.clone());
            Ok("recorded".to_owned())
        }
        ClaimOrder::Settle {
            settlement,
            window_end,
            now_real,
            ..
        } => settle(record, hull, *settlement, (*window_end, *now_real), now_ic),
        ClaimOrder::AuthorizeBlock { .. } | ClaimOrder::Ask { .. } | ClaimOrder::Refuse { .. } => {
            Err("claim_no_custody")
        }
    }
}

/// **A claim settled** (Q-36-27): adjudicated by the library's rules, which must agree with the
/// settlement the tick read; the custody record goes (an award's hull is the claimant's own, a
/// vacated claim's custody is void), and a vacated claim is written to the evidence log.
fn settle(
    record: &mut SiteRecord,
    hull: ObjId,
    settlement: Settlement,
    (window_end, now_real): (i64, i64),
    now_ic: i64,
) -> Result<String, &'static str> {
    let custody = custody_mut(record, hull).ok_or("claim_no_custody")?;
    if settlement == Settlement::Withdrawn {
        custody.claim = ClaimSeam::Owed;
        custody.file = None;
        return Ok("withdrawn".to_owned());
    }
    if settlement == Settlement::HandedBack {
        record.custody.retain(|row| row.hull != hull);
        return Ok("handed_back".to_owned());
    }
    let mut file = custody.file.as_deref().cloned().ok_or("claim_not_filed")?;
    file.deadline_real_s = window_end;
    let status = file.adjudicate(now_real).map_err(|_| "claim_not_due")?;
    let awarded = matches!(file.state, SalvageClaimState::AwardedToClaimant { .. });
    if awarded != (settlement == Settlement::Award) {
        return Err("claim_settled_otherwise");
    }
    let CustodyTitleStatus::ResolvedTitle { .. } = status else {
        return Err("claim_not_due");
    };
    custody.file = Some(Box::new(file));
    let (holder, landholder) = (
        custody.holder,
        custody.pin.as_ref().and_then(TenurePin::landholder),
    );
    // The settlement's bundle drops the custody row either way (doc 36a P0-7): once the award
    // has posted there is no custody left for a contest to vacate; a vacated claim's custody
    // is void.
    record.custody.retain(|row| row.hull != hull);
    if awarded {
        Ok("awarded".to_owned())
    } else {
        log_unlawful(
            record,
            UnlawfulAct {
                kind: UnlawfulKind::Vacated,
                actor: holder,
                subject: hull.0,
                landholder,
                at_ic: now_ic,
            },
        );
        Ok("vacated".to_owned())
    }
}

// ---- notice (Q-36-25) ---------------------------------------------------------------------

/// The IC mail seam a claim's notice goes through: `(ctx, from, to, subject, body)`.
pub type ClaimMailFn =
    dyn Fn(&mut Ctx<'_>, ObjId, &[ObjId], &str, &str) -> Result<(), String> + Send + Sync;

/// **The IC mail hook** (Q-36-25): installed by the server after composition
/// ([`install_claim_mail`], `mush-mail`'s notice); empty on a world without mail, where the
/// notice is the online line alone.
#[derive(Clone, Default)]
pub struct ClaimMail(Option<Arc<ClaimMailFn>>);

impl std::fmt::Debug for ClaimMail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("ClaimMail").field(&self.0.is_some()).finish()
    }
}

/// **Install the claim notice's IC mail** on `w`: `false` when tacsim's slot is not composed.
pub fn install_claim_mail(w: &mut World, mail: Arc<ClaimMailFn>) -> bool {
    match w.module_state_mut::<ClaimMail>() {
        Some(slot) => {
            slot.0 = Some(mail);
            true
        }
        None => false,
    }
}

/// The site's clerk, whose name a registry's letters go under; else the actor.
fn clerk_of(w: &World, site: &CanonicalId) -> Option<ObjId> {
    w.module_state::<crate::store::TacsimStore>()?
        .port_maps()
        .find(|map| &map.entry == site)?
        .site
        .as_ref()
        .and_then(|row| row.clerk)
        .map(ObjId)
}

/// **Tell `to` by IC mail**, and online: the mail when the hook is installed (a refused letter
/// is logged), the line always.
fn tell(ctx: &mut Ctx<'_>, site: &CanonicalId, to: &[ObjId], subject: &str, body: &str) {
    if to.is_empty() {
        return;
    }
    let mail = ctx
        .w
        .module_state::<ClaimMail>()
        .and_then(|slot| slot.0.clone());
    if let Some(mail) = mail {
        let from = clerk_of(ctx.w, site).unwrap_or(ctx.actor);
        // One letter each (doc 41 Q-41-28): a house agent the post refuses never costs the
        // others theirs.
        for who in to {
            if let Err(error) = mail(ctx, from, std::slice::from_ref(who), subject, body) {
                tracing::warn!(%error, to = who.0, "tacsim: a salvage claim's notice was not mailed");
            }
        }
    }
    for who in to {
        ctx.out
            .sys_to_actor(*who, "tacsim.salvage", body.to_owned());
    }
}

// ---- reads --------------------------------------------------------------------------------

/// The registry a site's claims are filed at: the lane's Registry (the Freeport Registry for
/// every frontier site).
fn registry_name(w: &World) -> String {
    crate::port::claims::registry_info(w).map_or_else(|| "Registry".to_owned(), |r| r.name)
}

/// A unit's name as the lines print it.
fn name(w: &World, unit: ObjId) -> String {
    crate::port::coop::name_of(w, unit)
}

/// A site's name.
fn site_name(w: &World, site: &CanonicalId) -> String {
    crate::surface::chart::lane_sites(w)
        .into_iter()
        .find(|s| &s.entry == site)
        .map_or_else(|| site.to_string(), |s| s.name)
}

/// A window's length in words.
fn window_words(secs: i64) -> String {
    let secs = secs.max(0);
    if secs >= 3_600 {
        format!("{} real hours", (secs + 3_599) / 3_600)
    } else {
        format!("{} real minutes", ((secs + 59) / 60).max(1))
    }
}

/// **Every custody record on every lane site's record** (plus any space derelict custody in
/// [`crate::space::tick::SpaceBubbleBook::space_custody`], `SF.R19`), with its site.
fn custody_rows(ctx: &Ctx<'_>) -> Vec<(CanonicalId, SalvageCustody)> {
    let mut rows = Vec::new();
    if let Some(handles) = ctx.w.module_state::<crate::port::families::PortHandles>() {
        for site in crate::surface::chart::lane_sites(ctx.w) {
            let key = SiteKey(site.entry.clone());
            if let Ok(Some(record)) = ctx.action_state(&handles.families.sites, &key) {
                for row in &record.custody {
                    rows.push((site.entry.clone(), row.clone()));
                }
            }
        }
    }
    if let Some(book) = ctx.w.module_state::<crate::space::tick::SpaceBubbleBook>() {
        let fallback_site = CanonicalId::new("freeport").ok().or_else(|| {
            crate::port::voyage::lane_ports(ctx.w)
                .into_iter()
                .next()
                .and_then(|(_, map)| map.map(|m| m.entry.clone()))
        });
        for row in book.space_custody.rows() {
            if rows.iter().any(|(_, r)| r.hull == row.hull) {
                continue;
            }
            let site = match crate::port::ship::berth_of(ctx.w, row.carrier) {
                crate::port::ship::Berth::Docked { map, .. } => Some(map.entry.clone()),
                _ => match crate::port::ship::berth_of(ctx.w, row.hull) {
                    crate::port::ship::Berth::Docked { map, .. } => Some(map.entry.clone()),
                    _ => fallback_site.clone(),
                },
            };
            if let Some(site_id) = site {
                rows.push((site_id, row.clone()));
            }
        }
    }
    rows
}

/// The hull a `+salvage` argument names among the custody rows: `#n`, or a hull's name.
fn named(ctx: &Ctx<'_>, word: &str) -> Result<(CanonicalId, SalvageCustody), CmdError> {
    let wanted = word.trim().trim_start_matches('#');
    let (site, row) = custody_rows(ctx)
        .into_iter()
        .find(|(_, row)| {
            row.hull.0.to_string() == wanted
                || name(ctx.w, row.hull).eq_ignore_ascii_case(word.trim())
        })
        .ok_or_else(|| CmdError::msg(NOT_IN_CUSTODY.replace("{hull}", word.trim())))?;
    if promoted(ctx, &site) {
        return Err(CmdError::msg(
            FIELD_OPEN.replace("{site}", &site_name(ctx.w, &site)),
        ));
    }
    Ok((site, row))
}

/// Whether a field is open on `site`: every claim act waits (Q-CF-30).
fn promoted(ctx: &Ctx<'_>, site: &CanonicalId) -> bool {
    crate::surface::tow::record_or_new(ctx, site)
        .open_field()
        .is_some()
}

/// The economy's window over contract `id`: its effective end and whether it is paused.
fn window(w: &World, id: u64) -> Option<(i64, bool)> {
    mush_economy::salvage_claim_window(w, ContractId(id))
}

/// **The window a filed claim actually runs** (doc 36a P0-2): the economy's effective end, never
/// earlier than the deadline the claim was filed with — an outage extends the end; nothing
/// shortens it below the filing's — and whether it is paused. `None` for an unfiled row, or a
/// contract the economy no longer knows.
fn effective_window(w: &World, row: &SalvageCustody) -> Option<(i64, bool)> {
    let ClaimSeam::Filed { contract } = row.claim else {
        return None;
    };
    let (end, paused) = window(w, contract)?;
    let floor = row.file.as_deref().map_or(end, |file| file.deadline_real_s);
    Some((end.max(floor), paused))
}

/// The economy contract's state.
fn contract_state(w: &World, id: u64) -> Option<ContractState> {
    w.module_state::<EconomyStore>()?
        .snapshot()
        .contract(ContractId(id))
        .map(|c| c.state)
}

/// The claim window configured, real seconds ([`claim_window_real_secs`]).
fn window_secs(w: &World) -> i64 {
    claim_window_real_secs(w)
}

/// **A salvage claim's contest window, real seconds** (the owner's 2026-11-04 rule: any wait on a
/// player's response is at least 28 IC days): the configured window, IC seconds
/// (`[tacsim.recovery].claim_window_ic_days`, 48), divided by the world's rate now, rounded up —
/// 48 real hours at rate 24, 48 real days at rate 1. The claim contract's deadline is real time
/// (its shape), so the window is converted once, at filing. Clamped to
/// `1..=mush_economy::MAX_SALVAGE_CLAIM_WINDOW_SECS`.
#[must_use]
pub fn claim_window_real_secs(w: &World) -> i64 {
    let ic = w
        .module_state::<crate::store::TacsimStore>()
        .map_or(crate::config::CLAIM_WINDOW_IC_DAYS * 86_400, |store| {
            store.config().recovery.claim_window_ic_s
        });
    let rate = i64::from(w.world_timeline().current().rate.max(1));
    (ic.saturating_add(rate - 1) / rate).clamp(1, mush_economy::MAX_SALVAGE_CLAIM_WINDOW_SECS)
}

// ---- the command --------------------------------------------------------------------------

/// Split `rest` into its words and a trailing `go`.
fn split_go(rest: &str) -> (String, bool) {
    let trimmed = rest.trim();
    match trimmed.rsplit_once(char::is_whitespace) {
        Some((head, "go")) => (head.trim().to_owned(), true),
        _ if trimmed == "go" => (String::new(), true),
        _ => (trimmed.to_owned(), false),
    }
}

/// Apply `order` when `order.hull()` is a space derelict held in
/// [`crate::space::tick::SpaceBubbleBook::space_custody`] (`SF.R19`; doc 41 cut RS.8,
/// Q-36-46), then write the domain's `tacsim:space:custody` document.
fn submit_space_claim_order(ctx: &mut Ctx<'_>, actor: ObjId, order: &ClaimOrder) -> Option<bool> {
    let applied = apply_space_claim_order(ctx, actor, order)?;
    if applied {
        crate::space::tick::persist_space_state(ctx);
    }
    Some(applied)
}

/// The space custody's half of [`submit_space_claim_order`]: the row's change, as the tow
/// action's prepare makes it on a site's record.
fn apply_space_claim_order(ctx: &mut Ctx<'_>, actor: ObjId, order: &ClaimOrder) -> Option<bool> {
    let hull = order.hull();
    let is_space = ctx
        .w
        .module_state::<crate::space::tick::SpaceBubbleBook>()
        .is_some_and(|b| b.space_custody.contains_key(&hull));
    if !is_space {
        return None;
    }
    let now_ic = ctx.w.world_ic_now().at;
    match order {
        ClaimOrder::File {
            contract,
            window_end,
            now_real,
            told,
            ..
        } => {
            let book = ctx
                .w
                .module_state_mut::<crate::space::tick::SpaceBubbleBook>()?;
            let custody = book.space_custody.get_mut(&hull)?;
            if custody.holder != actor || custody.claim != ClaimSeam::Owed {
                return Some(false);
            }
            let Ok(claim) = open_claim(custody, *contract, (*now_real, *window_end)) else {
                return Some(false);
            };
            custody.claim = ClaimSeam::Filed {
                contract: *contract,
            };
            custody.file = Some(Box::new(claim));
            custody.told.clone_from(told);
            custody.told.truncate(MAX_WITNESSES);
            Some(true)
        }
        ClaimOrder::Contest {
            ground,
            window_end,
            now_real,
            ..
        } => {
            if now_real >= window_end {
                return Some(false);
            }
            let book = ctx
                .w
                .module_state_mut::<crate::space::tick::SpaceBubbleBook>()?;
            let custody = book.space_custody.get_mut(&hull)?;
            if let ContestGround::UnlawfulRecovery { evidence } = ground
                && evidence_of(custody, actor).as_ref() != Some(evidence)
            {
                return Some(false);
            }
            if matches!(ground, ContestGround::BarePriorTitle)
                && custody
                    .file
                    .as_ref()
                    .is_none_or(|file| file.prior_owner != actor)
            {
                return Some(false);
            }
            let mut file = custody.file.as_deref().cloned()?;
            file.deadline_real_s = *window_end;
            if file.contest(actor, ground.clone(), *now_real).is_err() {
                return Some(false);
            }
            custody.file = Some(Box::new(file));
            Some(true)
        }
        ClaimOrder::Report { to, .. } => {
            if *to == actor || to.0 == 0 {
                return Some(false);
            }
            let book = ctx
                .w
                .module_state_mut::<crate::space::tick::SpaceBubbleBook>()?;
            let custody = book.space_custody.get_mut(&hull)?;
            let mut evidence = evidence_of(custody, actor)?;
            evidence.holder = *to;
            if !custody.evidence.contains(&evidence) {
                if custody.evidence.len() >= MAX_EVIDENCE {
                    return Some(false);
                }
                custody.evidence.push(evidence);
            }
            Some(true)
        }
        ClaimOrder::Inspect { .. } => {
            let book = ctx
                .w
                .module_state_mut::<crate::space::tick::SpaceBubbleBook>()?;
            let custody = book.space_custody.get_mut(&hull)?;
            let operation = custody.operation?;
            let read = SalvageEvidenceRecord {
                operation_id: operation,
                casualty: hull,
                holder: actor,
                kind: SalvageEvidenceKind::RecoveredBlackBox {
                    hull,
                    read_at_ic: now_ic,
                },
            };
            if !custody
                .evidence
                .iter()
                .any(|e| e.holder == actor && e.operation_id == operation)
            {
                if custody.evidence.len() >= MAX_EVIDENCE {
                    return Some(false);
                }
                custody.evidence.push(read);
            }
            Some(true)
        }
        ClaimOrder::Settle {
            settlement,
            window_end,
            now_real,
            ..
        } => {
            let book = ctx
                .w
                .module_state_mut::<crate::space::tick::SpaceBubbleBook>()?;
            let custody = book.space_custody.get_mut(&hull)?;
            if *settlement == Settlement::Withdrawn {
                custody.claim = ClaimSeam::Owed;
                custody.file = None;
                return Some(true);
            }
            if *settlement == Settlement::HandedBack {
                book.space_custody.remove(&hull);
                return Some(true);
            }
            let mut file = custody.file.as_deref().cloned()?;
            file.deadline_real_s = *window_end;
            let Ok(status) = file.adjudicate(*now_real) else {
                return Some(false);
            };
            let awarded = matches!(file.state, SalvageClaimState::AwardedToClaimant { .. });
            if awarded != (*settlement == Settlement::Award) {
                return Some(false);
            }
            if !matches!(status, CustodyTitleStatus::ResolvedTitle { .. }) {
                return Some(false);
            }
            book.space_custody.remove(&hull);
            if let Some(derelict) = book.derelicts.get_mut(&hull) {
                derelict.salvage_title_status = Some(status);
            }
            Some(true)
        }
        ClaimOrder::AtRisk { entry, .. } => {
            let book = ctx
                .w
                .module_state_mut::<crate::space::tick::SpaceBubbleBook>()?;
            let custody = book.space_custody.get_mut(&hull)?;
            if custody.holder != actor {
                return Some(false);
            }
            let list = match custody.file.as_mut() {
                Some(file) => &mut file.documented_repairs_at_risk,
                None => &mut custody.at_risk,
            };
            if !list.contains(entry) {
                if list.len() >= MAX_AT_RISK {
                    return Some(false);
                }
                list.push(entry.clone());
            }
            Some(true)
        }
        ClaimOrder::AuthorizeBlock { .. } | ClaimOrder::Ask { .. } | ClaimOrder::Refuse { .. } => {
            None
        }
    }
}

/// Submit `order` as `actor` through the tow action (or directly on `SpaceBubbleBook::space_custody` for space hulls).
fn submit(ctx: &mut Ctx<'_>, actor: ObjId, order: ClaimOrder, tag: u32) -> bool {
    if let Some(ok) = submit_space_claim_order(ctx, actor, &order) {
        return ok;
    }
    let hull = order.hull();
    crate::surface::tow::submit(
        ctx,
        actor,
        hull,
        crate::surface::tow::TowOrder::Claim {
            order: Box::new(order),
        },
        tag,
    )
}

/// **`+salvage`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn salvage_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // Doc 42 cut CR.0 (Q-42-2): every `+salvage` switch is the held gate's.
    if ctx.switches.iter().next().is_some() {
        crate::field::crew::gate(ctx, crate::field::crew::Gate::Operate)?;
    }
    let raw = ctx.args.raw().to_owned();
    if ctx.switches.has("claim") {
        let (word, go) = split_go(&raw);
        return claim_cmd(ctx, &word, go);
    }
    if ctx.switches.has("contest") {
        let (rest, go) = split_go(&raw);
        let (word, kind) = match rest.split_once('=') {
            Some((word, kind)) => {
                let kind = ContestKind::parse(kind)
                    .ok_or_else(|| CmdError::msg(NO_SUCH_KIND.replace("{kind}", kind.trim())))?;
                (word.trim().to_owned(), Some(kind))
            }
            None => (rest, None),
        };
        return contest_cmd(ctx, &word, kind, go);
    }
    // Doc 41 cut RS.10: the landholder consent step.
    if ctx.switches.has("ask") {
        let (rest, go) = split_go(&raw);
        return ask_cmd(ctx, rest.trim(), go);
    }
    if ctx.switches.has("refuse") {
        let (rest, go) = split_go(&raw);
        let (word, who) = rest
            .split_once('=')
            .ok_or_else(|| CmdError::msg(SALVAGE_USAGE))?;
        return refuse_cmd(ctx, word.trim(), who.trim(), go);
    }
    if ctx.switches.has("authorize") {
        let (rest, go) = split_go(&raw);
        let (word, who) = rest
            .split_once('=')
            .ok_or_else(|| CmdError::msg(SALVAGE_USAGE))?;
        let (who, until) = match who.trim().split_once(char::is_whitespace) {
            Some((who, tail)) => {
                let until = tail
                    .trim()
                    .strip_prefix("until=")
                    .and_then(|n| n.trim().parse::<i64>().ok())
                    .ok_or_else(|| CmdError::msg(SALVAGE_USAGE))?;
                (who.trim(), Some(until))
            }
            None => (who.trim(), None),
        };
        return authorize_cmd(ctx, word.trim(), who, until, go);
    }
    if ctx.switches.has("report") {
        // Doc 41 cut RS.7 (Q-41-24): the registry's form reads the site's log, not a custody row.
        if let Some(head) = raw.trim().strip_suffix(" registry") {
            return crate::surface::security::report_cmd(ctx, head);
        }
        let (rest, go) = split_go(&raw);
        let (word, who) = rest
            .split_once('=')
            .ok_or_else(|| CmdError::msg(SALVAGE_USAGE))?;
        return report_cmd(ctx, word.trim(), who.trim(), go);
    }
    if ctx.switches.has("inspect") {
        return inspect_cmd(ctx, raw.trim());
    }
    // Doc 41 cut RS.5 (Q-41-9): the retrieval's three verbs.
    if ctx.switches.has("retrieve") {
        return crate::surface::retrieval::retrieve_cmd(ctx, &raw);
    }
    if ctx.switches.has("jobs") {
        return crate::surface::retrieval::jobs_cmd(ctx);
    }
    if ctx.switches.has("accept") {
        return crate::surface::retrieval::accept_cmd(ctx, &raw);
    }
    // Doc 41 cut RS.6c-2: the recovery houses' resale.
    if ctx.switches.has("forsale") {
        return crate::port::house_sale::forsale_cmd(ctx);
    }
    if ctx.switches.has("buy") {
        return crate::port::house_sale::buy_cmd(ctx, &raw);
    }
    if !raw.trim().is_empty() {
        return Err(CmdError::msg(SALVAGE_USAGE));
    }
    list_cmd(ctx)
}

/// `+salvage`: the claims the actor stands in.
fn list_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = ctx.actor;
    let mut lines: Vec<String> = Vec::new();
    for (site, row) in custody_rows(ctx) {
        let pin = row.pin.as_ref();
        let mut roles: Vec<&str> = Vec::new();
        if row.holder == me {
            roles.push("custodian");
        }
        if pin.is_some_and(|p| p.answers_for(me)) {
            roles.push("answers for the title");
        }
        if pin.and_then(TenurePin::landholder) == Some(me) {
            roles.push("landholder");
        }
        if evidence_of(&row, me).is_some() {
            roles.push("you hold evidence");
        }
        if roles.is_empty() && !row.told.contains(&me) {
            continue;
        }
        let claim = match (&row.claim, row.file.as_deref()) {
            (ClaimSeam::Owed, _) => "a claim owed".to_owned(),
            (ClaimSeam::Filed { contract }, file) => {
                let contests = file.map_or(0, |f| f.contests.len());
                let when = match effective_window(ctx.w, &row) {
                    Some((_, true)) => "its window paused".to_owned(),
                    Some((end, false)) => format!(
                        "its window closing in {}",
                        window_words(end.saturating_sub(ctx.w.now()))
                    ),
                    None => "its window closed".to_owned(),
                };
                format!("filed as contract #{contract}, {when}, {contests} contest(s)")
            }
        };
        lines.push(format!(
            "#{} the {} — recovered at {}, in the {}'s bay; {claim}; {}.",
            row.hull.0,
            name(ctx.w, row.hull),
            site_name(ctx.w, &site),
            name(ctx.w, row.carrier),
            roles.join(", ")
        ));
    }
    let text = if lines.is_empty() {
        NO_CLAIMS.to_owned()
    } else {
        lines.join("\n")
    };
    ctx.out.sys_to_actor(me, "tacsim.salvage", text);
    Ok(())
}

/// `+salvage/claim <hull> [go]`.
/// **File the custodian's claim on the hull `word` names** (doc 41 cut RS.8, Q-36-46):
/// `+salvage/claim <hull> go`, for `+helm/tow`, so a space tow files on the ground's one path.
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn tow_files_salvage(ctx: &mut Ctx<'_>, word: &str) -> Result<(), CmdError> {
    claim_cmd(ctx, word, true)
}

fn claim_cmd(ctx: &mut Ctx<'_>, word: &str, go: bool) -> Result<(), CmdError> {
    let (site, row) = named(ctx, word)?;
    let label = name(ctx.w, row.hull);
    let hull_arg = format!("#{}", row.hull.0);
    if crate::port::tv::written_off(ctx.w, row.hull) {
        return Err(CmdError::msg(WRITTEN_OFF.replace("{name}", &label)));
    }
    if row.holder != ctx.actor {
        return Err(CmdError::msg(NOT_CUSTODIAN.replace("{name}", &label)));
    }
    if let ClaimSeam::Filed { contract } = row.claim {
        return Err(CmdError::msg(
            ALREADY_FILED
                .replace("{name}", &label)
                .replace("{contract}", &contract.to_string()),
        ));
    }
    let pin = row.pin.clone();
    let answering = pin.as_ref().and_then(|p| p.answering);
    if answering.is_none() || row.operation.is_none() {
        return Err(CmdError::msg(NO_TITLE.replace("{name}", &label)));
    }
    // Doc 41 cut RS.10 (ruling 3): a landholder's standing refusal of the claimant on the block
    // she was pinned on blocks the claim.
    if let Some(GroundTenure::Owned {
        landholder,
        claim_id: Some(block),
    }) = pin.as_ref().map(|p| &p.tenure)
        && let Some(letter) = block
            .rsplit('/')
            .next()
            .and_then(|l| l.chars().next())
            .map(|l| l.to_ascii_uppercase())
    {
        let now_ic = ctx.w.world_ic_now().at;
        let record = crate::surface::tow::record_or_new(ctx, &site);
        if record.consent_asks.iter().any(|ask| {
            ask.letter == letter
                && ask.landholder == *landholder
                && ask.asker == ctx.actor
                && ask.refuses(now_ic)
        }) {
            return Err(CmdError::msg(CLAIM_REFUSED.replace(
                "{block}",
                &format!("{}/{letter}", site_name(ctx.w, &site)),
            )));
        }
    }
    // Doc 41 Q-41-22, Q-41-28: everyone who answered for the title at the attach and everyone
    // who answers for it now, then the landholder.
    let now_answering = title_of(ctx.w, row.hull).1;
    let mut told: Vec<ObjId> = Vec::new();
    for who in answering
        .into_iter()
        .chain(pin.iter().flat_map(|p| p.answerers.iter().copied()))
        .chain(now_answering)
        .chain(pin.as_ref().and_then(TenurePin::landholder))
    {
        if who != ctx.actor && !told.contains(&who) {
            told.push(who);
        }
    }
    told.truncate(MAX_WITNESSES);
    let registry = registry_name(ctx.w);
    let secs = window_secs(ctx.w);
    let window_text = window_words(secs);
    if !go {
        let told_words = if told.is_empty() {
            "Nobody else is told.".to_owned()
        } else {
            format!(
                "{} told by IC mail.",
                told.iter()
                    .map(|who| name(ctx.w, *who))
                    .collect::<Vec<_>>()
                    .join(" and ")
                    + if told.len() == 1 { " is" } else { " are" }
            )
        };
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.salvage",
            CLAIM_PREVIEW
                .replace("{name}", &label)
                .replace("{registry}", &registry)
                .replace("{window}", &window_text)
                .replace("{told}", &told_words)
                .replace("{hull}", &hull_arg),
        );
        return Ok(());
    }
    let remainder = crate::port::credit::own_account(ctx.w, ctx.actor)
        .map(|account| account.id)
        .ok_or_else(|| CmdError::msg(crate::port::claims::CLAIMS_FAILED))?;
    let audience: Vec<PrincipalId> = told.iter().map(|who| PrincipalId::Object(*who)).collect();
    let contract = mush_economy::file_salvage_claim(
        ctx,
        mush_economy::SalvageClaimSpec {
            name: smol_str::SmolStr::new(format!("Salvage claim: the {label}")),
            claimant: ctx.actor,
            hull: row.hull,
            audience,
            window_secs: secs,
            remainder_to: remainder,
        },
    )
    .map_err(|error| {
        tracing::warn!(hull = row.hull.0, %error, "tacsim: a salvage claim was not filed");
        match error {
            mush_economy::ContractError::NoSuchAsset(_) => {
                CmdError::msg(NO_TITLE.replace("{name}", &label))
            }
            _ => CmdError::msg(crate::port::claims::CLAIMS_FAILED),
        }
    })?;
    let window_end =
        window(ctx.w, contract.0).map_or(ctx.w.now().saturating_add(secs), |(end, _)| end);
    let actor = ctx.actor;
    let order = ClaimOrder::File {
        site: site.clone(),
        hull: row.hull,
        contract: contract.0,
        window_end,
        now_real: ctx.w.now(),
        told: told.clone(),
    };
    if !submit(ctx, actor, order, 1) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        actor,
        "tacsim.salvage",
        CLAIM_FILED
            .replace("{name}", &label)
            .replace("{registry}", &registry)
            .replace("{contract}", &contract.0.to_string())
            .replace("{window}", &window_text),
    );
    let body = NOTICE_BODY
        .replace("{claimant}", &name(ctx.w, actor))
        .replace("{name}", &label)
        .replace("{site}", &site_name(ctx.w, &site))
        .replace("{registry}", &registry)
        .replace("{contract}", &contract.0.to_string())
        .replace("{window}", &window_text)
        .replace("{hull}", &hull_arg);
    tell(
        ctx,
        &site,
        &told,
        &NOTICE_SUBJECT.replace("{name}", &label),
        &body,
    );
    Ok(())
}

/// The filed claim's contract and its open window, or the line that refuses.
fn open_window(ctx: &Ctx<'_>, row: &SalvageCustody, label: &str) -> Result<(u64, i64), CmdError> {
    let ClaimSeam::Filed { contract } = row.claim else {
        return Err(CmdError::msg(NOT_FILED.replace("{name}", label)));
    };
    match effective_window(ctx.w, row) {
        Some((_, true)) => Err(CmdError::msg(WINDOW_PAUSED.replace("{name}", label))),
        // Doc 36a P0-7: a contest is admitted only while `now < end`. The tick awards at
        // `now >= end`, so the two never share a second: no contest lands on a claim whose title
        // has posted.
        Some((end, false))
            if ctx.w.now() < end
                && contract_state(ctx.w, contract).is_some_and(|s| !s.is_terminal()) =>
        {
            Ok((contract, end))
        }
        _ => Err(CmdError::msg(WINDOW_CLOSED.replace("{name}", label))),
    }
}

/// **What the hull's black box says of her recovery** (doc 41 Q-41-23): its last
/// `recovered_to_base` close line, read by the registry.
fn box_recovery(w: &World, hull: ObjId) -> Option<BoxRecovery> {
    let (lines, _) = w
        .module_state::<crate::store::TacsimStore>()?
        .black_box_lines(hull);
    lines.into_iter().rev().find_map(|(_, line)| match line {
        crate::surface::black_box::BlackBoxLine::SortieClosed {
            close: mush_roll::commerce::ic_claims::IcCloseKind::RecoveredToBase { base },
            recovered_by,
            ..
        } => Some(BoxRecovery { base, recovered_by }),
        _ => None,
    })
}

/// `+salvage/contest <hull>[=<kind>] [go]`.
fn contest_cmd(
    ctx: &mut Ctx<'_>,
    word: &str,
    kind: Option<ContestKind>,
    go: bool,
) -> Result<(), CmdError> {
    let (site, row) = named(ctx, word)?;
    let label = name(ctx.w, row.hull);
    let hull_arg = format!("#{}", row.hull.0);
    let (_, end) = open_window(ctx, &row, &label)?;
    let me = ctx.actor;
    let answers = row.pin.as_ref().is_some_and(|pin| pin.answers_for(me))
        || row.file.as_ref().is_some_and(|f| f.prior_owner == me);
    let recovery = |evidence: SalvageEvidenceRecord| {
        let preview = CONTEST_PREVIEW_EVIDENCE
            .replace("{name}", &label)
            .replace("{kind}", evidence_words(&evidence.kind))
            .replace("{hull}", &hull_arg);
        (ContestGround::UnlawfulRecovery { evidence }, preview)
    };
    let (ground, preview) = match kind {
        None => match evidence_of(&row, me) {
            Some(evidence) => recovery(evidence),
            None if answers => (
                ContestGround::BarePriorTitle,
                CONTEST_PREVIEW_TITLE
                    .replace("{name}", &label)
                    .replace("{hull}", &hull_arg),
            ),
            None => return Err(CmdError::msg(NO_GROUND.replace("{name}", &label))),
        },
        Some(ContestKind::Recovery) => recovery(
            evidence_of(&row, me)
                .ok_or_else(|| CmdError::msg(NO_RECOVERY_GROUND.replace("{name}", &label)))?,
        ),
        Some(ContestKind::Procedure) => {
            if !answers {
                return Err(CmdError::msg(NOT_ANSWERING.replace("{name}", &label)));
            }
            let defect = procedure_defect(&row, box_recovery(ctx.w, row.hull))
                .ok_or_else(|| CmdError::msg(NO_DEFECT.replace("{name}", &label)))?;
            let preview = CONTEST_PREVIEW_PROCEDURE
                .replace("{name}", &label)
                .replace("{defect}", &defect_words(ctx.w, &defect))
                .replace("{hull}", &hull_arg);
            (
                ContestGround::DefectiveProcedure {
                    defect_code: smol_str::SmolStr::new(defect),
                },
                preview,
            )
        }
        Some(ContestKind::Right) => {
            let claimant = name(ctx.w, row.holder);
            let (contract_id, obligor) = right_of_grantor(&row, me).ok_or_else(|| {
                CmdError::msg(
                    NO_RIGHT
                        .replace("{name}", &label)
                        .replace("{claimant}", &claimant),
                )
            })?;
            let right = format!(
                "the recovery authorization you issued them ({contract_id}), held when the \
                 recovery began"
            );
            let preview = CONTEST_PREVIEW_RIGHT
                .replace("{name}", &label)
                .replace("{right}", &right)
                .replace("{claimant}", &claimant)
                .replace("{hull}", &hull_arg);
            (
                ContestGround::SuperiorSpecificRight {
                    contract_id: smol_str::SmolStr::new(contract_id),
                    obligor,
                },
                preview,
            )
        }
    };
    if !go {
        ctx.out.sys_to_actor(me, "tacsim.salvage", preview);
        return Ok(());
    }
    let order = ClaimOrder::Contest {
        site,
        hull: row.hull,
        ground,
        window_end: end,
        now_real: ctx.w.now(),
    };
    if !submit(ctx, me, order, 2) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        me,
        "tacsim.salvage",
        CONTEST_FILED.replace("{name}", &label),
    );
    Ok(())
}

/// The character a `<who>` names: `#n`, or a player's name.
fn who_named(ctx: &Ctx<'_>, who: &str) -> Result<ObjId, CmdError> {
    let wanted = who.trim();
    // Doc 41 cut RS.10: a recovery house's dispatcher asks landholders too, and is answered by name.
    let houses = crate::port::house::dispatchers(ctx.w);
    ctx.w
        .iter()
        .find(|o| {
            (o.kind == mush_core::ObjKind::Player || houses.contains(&o.id))
                && (o.name.eq_ignore_ascii_case(wanted)
                    || wanted.strip_prefix('#') == Some(o.id.0.to_string().as_str()))
        })
        .map(|o| o.id)
        .ok_or_else(|| CmdError::msg(NO_SUCH_WHO.replace("{who}", wanted)))
}

/// `+salvage/authorize <hull>|<site>/<block>=<who> [until=<when>] [go]` (doc 41 Q-41-21).
fn authorize_cmd(
    ctx: &mut Ctx<'_>,
    word: &str,
    who: &str,
    until: Option<i64>,
    go: bool,
) -> Result<(), CmdError> {
    let to = who_named(ctx, who)?;
    let who_name = name(ctx.w, to);
    let until_arg = until.map_or_else(String::new, |at| format!(" until={at}"));
    if let Some((site_word, letter_word)) = word.split_once('/') {
        return authorize_block(ctx, (site_word, letter_word), (to, &who_name), until, go);
    }
    // The title holder's: `unit.recover_load` on the hull, through Org's unit authority.
    let wanted = word.trim().trim_start_matches('#');
    let hull = crate::surface::recovery::world_hulls(ctx.w)
        .into_iter()
        .chain(
            ctx.w
                .module_state::<mush_tactics::UnitStore>()
                .map(|units| units.units.keys().copied().collect::<Vec<_>>())
                .unwrap_or_default(),
        )
        .find(|hull| {
            hull.0.to_string() == wanted || name(ctx.w, *hull).eq_ignore_ascii_case(word.trim())
        })
        .ok_or_else(|| CmdError::msg(NOT_IN_CUSTODY.replace("{hull}", word.trim())))?;
    let label = name(ctx.w, hull);
    let (_, answering) = title_of(ctx.w, hull);
    if !answering.contains(&ctx.actor) {
        return Err(CmdError::msg(NOT_TITLED.replace("{name}", &label)));
    }
    if !go {
        let until_words = until.map_or_else(String::new, |at| format!(" until {at} (UTC)"));
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.salvage",
            AUTHORIZE_HULL_PREVIEW
                .replace("{name}", &label)
                .replace("{who}", &who_name)
                .replace("{until}", &until_words)
                .replace("{until_arg}", &until_arg)
                .replace("{hull}", &format!("#{}", hull.0)),
        );
        return Ok(());
    }
    let reader = ctx
        .w
        .module_state::<crate::detachment::planning::PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.clone())
        .ok_or_else(|| CmdError::msg(crate::port::tv::TV_UNWIRED))?;
    let draft = mush_org::unit_authority::reader::UnitGrantDraft {
        grantee: mush_org::unit_authority::UnitGrantee::Character(to),
        capability: mush_org::unit_authority::UnitCapability::RecoverLoad,
        scope: mush_org::unit_authority::UnitScope::Units {
            units: std::collections::BTreeSet::from([hull]),
        },
        valid_until: until,
        delegable: false,
        recovery: mush_org::unit_authority::RecoveryPolicy::None,
        parent: None,
    };
    mush_org::unit_authority::runtime::submit_from(
        ctx,
        &reader,
        mush_org::unit_authority::reader::UnitGrantMutation::Issue(draft),
    )
    .map_err(CmdError::msg)?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.salvage",
        AUTHORIZE_HULL_SENT
            .replace("{name}", &label)
            .replace("{who}", &who_name),
    );
    Ok(())
}

/// **A block named `<site>/<letter>`** with a live claim on it: `(the words, site, letter, the
/// block's place, the claim)`.
///
/// # Errors
///
/// [`NO_SUCH_BLOCK`] (no such site, letter or live claim) or [`FIELD_OPEN`].
fn block_named(
    ctx: &Ctx<'_>,
    (site_word, letter_word): (&str, &str),
) -> Result<
    (
        String,
        CanonicalId,
        char,
        ObjId,
        crate::surface::claims::BlockClaim,
    ),
    CmdError,
> {
    use crate::port::site_map::SurfaceReads;
    let block = format!("{}/{}", site_word.trim(), letter_word.trim());
    let no_block = || CmdError::msg(NO_SUCH_BLOCK.replace("{block}", &block));
    let site = crate::surface::chart::lane_sites(ctx.w)
        .into_iter()
        .find(|s| {
            s.entry.as_str().eq_ignore_ascii_case(site_word.trim())
                || s.name.eq_ignore_ascii_case(site_word.trim())
        })
        .map(|s| s.entry)
        .ok_or_else(no_block)?;
    let letter = {
        let mut chars = letter_word.trim().chars();
        match (chars.next(), chars.next()) {
            (Some(letter), None) if letter.is_ascii_alphabetic() => letter.to_ascii_uppercase(),
            _ => return Err(no_block()),
        }
    };
    if promoted(ctx, &site) {
        return Err(CmdError::msg(
            FIELD_OPEN.replace("{site}", &site_name(ctx.w, &site)),
        ));
    }
    let now_ic = ctx.w.world_ic_now().at;
    let tenure = ctx
        .block_tenure(&crate::surface::claims::BlockKey {
            site: site.clone(),
            letter,
        })
        .ok()
        .flatten()
        .ok_or_else(no_block)?;
    let claim = tenure
        .claim
        .clone()
        .filter(|claim| claim.live(now_ic))
        .ok_or_else(no_block)?;
    Ok((block, site, letter, tenure.place, claim))
}

/// **Ask for a landholder's consent** (doc 41 cut RS.10) as `ctx.actor`, on block `letter` of
/// `site` under `claim`: one [`ClaimOrder::Ask`] submitted, and the registry's notice to the
/// landholder, which reaches them now or at their next connect. Shared by `+salvage/ask` and a
/// recovery house's sweep.
///
/// # Errors
///
/// The sentence: a standing refusal, or the tow action unwired.
pub(crate) fn ask_consent(
    ctx: &mut Ctx<'_>,
    (site, letter, place): (&CanonicalId, char, ObjId),
    claim: &crate::surface::claims::BlockClaim,
) -> Result<ObjId, String> {
    let PrincipalId::Object(landholder) = claim.holder else {
        return Err("Nobody answers for that block's claim.".to_owned());
    };
    let actor = ctx.actor;
    let now_ic = ctx.w.world_ic_now().at;
    let block = format!("{}/{letter}", site_name(ctx.w, site));
    let record = crate::surface::tow::record_or_new(ctx, site);
    if consent_on(
        &record,
        letter,
        (landholder, claim.registered_ic),
        actor,
        now_ic,
    ) == Consent::Refused
    {
        return Err(ASK_REFUSED.replace("{block}", &block));
    }
    let order = ClaimOrder::Ask {
        site: site.clone(),
        place,
        ask: ConsentAsk {
            letter,
            landholder,
            registered_ic: claim.registered_ic,
            asker: actor,
            asked_ic: now_ic,
            refused_ic: None,
        },
    };
    if !submit(ctx, actor, order, 7) {
        return Err(crate::port::tv::TV_UNWIRED.to_owned());
    }
    mush_economy::deliver_notice(
        ctx,
        landholder,
        CONSENT_NOTICE
            .replace("{who}", &name(ctx.w, actor))
            .replace("{block}", &block)
            .replace("{days}", &CONSENT_IC_DAYS.to_string()),
    );
    Ok(landholder)
}

/// **Ask the landholder of `cell` on `site`** ([`ask_consent`]), for a recovery house's sweep.
///
/// # Errors
///
/// The sentence: open ground, or [`ask_consent`]'s.
pub(crate) fn ask_consent_at(
    ctx: &mut Ctx<'_>,
    site: &CanonicalId,
    cell: (i64, i64),
) -> Result<(), String> {
    use crate::port::site_map::SurfaceReads;
    let (letter, claim) = block_claim_at(ctx, site, cell).ok_or("that ground is open")?;
    let place = ctx
        .block_tenure(&crate::surface::claims::BlockKey {
            site: site.clone(),
            letter,
        })
        .ok()
        .flatten()
        .ok_or("that block has no place")?
        .place;
    ask_consent(ctx, (site, letter, place), &claim).map(|_| ())
}

/// `+salvage/ask <site>/<block> [go]` (doc 41 cut RS.10): ask the block's landholder for consent
/// to recover hulls off it.
fn ask_cmd(ctx: &mut Ctx<'_>, word: &str, go: bool) -> Result<(), CmdError> {
    let (site_word, letter_word) = word
        .split_once('/')
        .ok_or_else(|| CmdError::msg(SALVAGE_USAGE))?;
    let (block, site, letter, place, claim) = block_named(ctx, (site_word, letter_word))?;
    let PrincipalId::Object(landholder) = claim.holder else {
        return Err(CmdError::msg(NO_SUCH_BLOCK.replace("{block}", &block)));
    };
    let now_ic = ctx.w.world_ic_now().at;
    let record = crate::surface::tow::record_or_new(ctx, &site);
    let held = (landholder, claim.registered_ic);
    let line = match consent_on(&record, letter, held, ctx.actor, now_ic) {
        Consent::Given => Some(ASK_GIVEN.replace("{block}", &block)),
        Consent::Refused => Some(ASK_REFUSED.replace("{block}", &block)),
        Consent::Asked => Some(ASK_PENDING.replace("{block}", &block)),
        Consent::NotAsked => None,
    };
    if let Some(line) = line {
        return Err(CmdError::msg(line));
    }
    if !go {
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.salvage",
            ASK_PREVIEW
                .replace("{block}", &block)
                .replace("{who}", &name(ctx.w, landholder))
                .replace("{days}", &CONSENT_IC_DAYS.to_string()),
        );
        return Ok(());
    }
    ask_consent(ctx, (&site, letter, place), &claim).map_err(CmdError::msg)?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.salvage",
        ASK_SENT
            .replace("{block}", &block)
            .replace("{who}", &name(ctx.w, landholder))
            .replace("{days}", &CONSENT_IC_DAYS.to_string()),
    );
    Ok(())
}

/// `+salvage/refuse <site>/<block>=<who> [go]` (doc 41 cut RS.10): the landholder refuses `who`
/// recovery off the block for [`CONSENT_IC_DAYS`], asked or not; any delegation it gave them there
/// is void.
fn refuse_cmd(ctx: &mut Ctx<'_>, word: &str, who: &str, go: bool) -> Result<(), CmdError> {
    let (site_word, letter_word) = word
        .split_once('/')
        .ok_or_else(|| CmdError::msg(SALVAGE_USAGE))?;
    let asker = who_named(ctx, who)?;
    let (block, site, letter, place, claim) = block_named(ctx, (site_word, letter_word))?;
    if claim.holder != PrincipalId::Object(ctx.actor) {
        return Err(CmdError::msg(NOT_LANDHOLDER.replace("{block}", &block)));
    }
    if asker == ctx.actor {
        return Err(CmdError::msg(SALVAGE_USAGE));
    }
    let who_name = name(ctx.w, asker);
    if !go {
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.salvage",
            REFUSE_PREVIEW
                .replace("{block}", &block)
                .replace("{who}", &who_name)
                .replace("{days}", &CONSENT_IC_DAYS.to_string()),
        );
        return Ok(());
    }
    let actor = ctx.actor;
    let now_ic = ctx.w.world_ic_now().at;
    let record = crate::surface::tow::record_or_new(ctx, &site);
    let asked_ic = record
        .consent_asks
        .iter()
        .find(|ask| ask.letter == letter && ask.of(asker, (actor, claim.registered_ic)))
        .map_or(now_ic, |ask| ask.asked_ic);
    let order = ClaimOrder::Refuse {
        site,
        place,
        ask: ConsentAsk {
            letter,
            landholder: actor,
            registered_ic: claim.registered_ic,
            asker,
            asked_ic,
            refused_ic: Some(now_ic),
        },
    };
    if !submit(ctx, actor, order, 8) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    mush_economy::deliver_notice(
        ctx,
        asker,
        REFUSED_NOTICE
            .replace("{who}", &name(ctx.w, actor))
            .replace("{block}", &block)
            .replace("{days}", &CONSENT_IC_DAYS.to_string()),
    );
    ctx.out.sys_to_actor(
        actor,
        "tacsim.salvage",
        REFUSE_SENT
            .replace("{block}", &block)
            .replace("{who}", &who_name)
            .replace("{days}", &CONSENT_IC_DAYS.to_string()),
    );
    Ok(())
}

/// `+salvage/authorize <site>/<block>=<who> [until=<IC seconds>] [go]`: the landholder's
/// recovery delegation on the block's live claim (doc 41 Q-41-21).
fn authorize_block(
    ctx: &mut Ctx<'_>,
    (site_word, letter_word): (&str, &str),
    (to, who_name): (ObjId, &str),
    until: Option<i64>,
    go: bool,
) -> Result<(), CmdError> {
    let (block, site, letter, place, claim) = block_named(ctx, (site_word, letter_word))?;
    let now_ic = ctx.w.world_ic_now().at;
    if claim.holder != PrincipalId::Object(ctx.actor) {
        return Err(CmdError::msg(NOT_LANDHOLDER.replace("{block}", &block)));
    }
    if to == ctx.actor {
        return Err(CmdError::msg(SALVAGE_USAGE));
    }
    let until_arg = until.map_or_else(String::new, |at| format!(" until={at}"));
    if !go {
        let until_words = until.map_or_else(String::new, |at| format!(" until IC {at}"));
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.salvage",
            AUTHORIZE_BLOCK_PREVIEW
                .replace("{block}", &block)
                .replace("{who}", who_name)
                .replace("{until}", &until_words)
                .replace("{until_arg}", &until_arg),
        );
        return Ok(());
    }
    let actor = ctx.actor;
    let order = ClaimOrder::AuthorizeBlock {
        site,
        place,
        grant: LandGrant {
            letter,
            landholder: actor,
            registered_ic: claim.registered_ic,
            to,
            until_ic: until,
            issued_ic: now_ic,
        },
    };
    if !submit(ctx, actor, order, 6) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        actor,
        "tacsim.salvage",
        AUTHORIZE_BLOCK_SENT
            .replace("{block}", &block)
            .replace("{who}", who_name),
    );
    Ok(())
}

/// `+salvage/report <hull>=<who> [go]`.
fn report_cmd(ctx: &mut Ctx<'_>, word: &str, who: &str, go: bool) -> Result<(), CmdError> {
    let (site, row) = named(ctx, word)?;
    let label = name(ctx.w, row.hull);
    let me = ctx.actor;
    if evidence_of(&row, me).is_none() {
        return Err(CmdError::msg(NO_EVIDENCE.replace("{name}", &label)));
    }
    let to = ctx
        .w
        .iter()
        .find(|o| o.kind == mush_core::ObjKind::Player && o.name.eq_ignore_ascii_case(who))
        .map(|o| o.id)
        .ok_or_else(|| CmdError::msg(NO_SUCH_WHO.replace("{who}", who)))?;
    let fill = |line: &str| {
        line.replace("{name}", &label)
            .replace("{who}", who)
            .replace("{hull}", &format!("#{}", row.hull.0))
    };
    if !go {
        ctx.out
            .sys_to_actor(me, "tacsim.salvage", fill(REPORT_PREVIEW));
        return Ok(());
    }
    let order = ClaimOrder::Report {
        site: site.clone(),
        hull: row.hull,
        to,
    };
    if !submit(ctx, me, order, 3) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out
        .sys_to_actor(me, "tacsim.salvage", fill(REPORT_SENT));
    let body = format!(
        "{} hands you their evidence of the {label}'s recovery: +salvage lists it.",
        name(ctx.w, me)
    );
    tell(
        ctx,
        &site,
        &[to],
        &NOTICE_SUBJECT.replace("{name}", &label),
        &body,
    );
    Ok(())
}

/// **Whether `who` stands aboard `carrier`**: physical bay access (P-3).
fn aboard(w: &World, who: ObjId, carrier: ObjId) -> bool {
    let Some(at) = w.get(who).and_then(|o| o.location) else {
        return false;
    };
    crate::port::ship::hull_view(w, carrier)
        .is_some_and(|view| at == carrier || view.row.rooms.values().any(|room| ObjId(*room) == at))
}

/// `+salvage/inspect <hull>`.
fn inspect_cmd(ctx: &mut Ctx<'_>, word: &str) -> Result<(), CmdError> {
    let (site, row) = named(ctx, word)?;
    let label = name(ctx.w, row.hull);
    let me = ctx.actor;
    if !aboard(ctx.w, me, row.carrier) && !aboard(ctx.w, me, row.hull) {
        return Err(CmdError::msg(
            NO_BAY_ACCESS
                .replace("{name}", &label)
                .replace("{carrier}", &name(ctx.w, row.carrier)),
        ));
    }
    let lines = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .map_or(0, |store| store.black_box(row.hull).len())
        .max(
            ctx.w
                .module_state::<mush_tactics::field_ops::SalvageClaimBook>()
                .map_or(0, |sb| sb.black_box_tape_for_hull(row.hull).len()),
        );
    let lines_text = match lines {
        0 => "it holds no lines".to_owned(),
        1 => "one line".to_owned(),
        n => format!("{n} lines"),
    };
    let order = ClaimOrder::Inspect {
        site,
        hull: row.hull,
    };
    if !submit(ctx, me, order, 4) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(
        me,
        "tacsim.salvage",
        INSPECTED
            .replace("{name}", &label)
            .replace("{lines}", &lines_text),
    );
    Ok(())
}

// ---- the tick -----------------------------------------------------------------------------

/// A good's reference price, minor units, from the goods table (0 when it does not read).
fn reference_minor(w: &World, good: &CanonicalId) -> u64 {
    w.module_state::<crate::store::TacsimStore>()
        .and_then(crate::store::TacsimStore::goods)
        .and_then(|goods| goods.good(good).map(|row| row.reference_major))
        .and_then(|major| crate::port::apply::minor_of(major, crate::port::apply::credit_scale(w)))
        .and_then(|minor| u64::try_from(minor).ok())
        .unwrap_or(0)
}

/// **The repair a salvor's last completed bay order made at their own risk** (doc 41 Q-41-20,
/// Q-S14): an order the custodian took on the salvor's right, done since the recovery — its job
/// lines, its fee plus its goods at their reference, and when it closed.
fn at_risk_of(w: &World, row: &SalvageCustody) -> Option<AtRiskRepairEntry> {
    use crate::port::bay_work::{BayHullState, OrderStatus, Right};
    let Some(BayHullState::Present(bay)) = w
        .module_state::<crate::store::TacsimStore>()?
        .bay_hull(row.hull)
    else {
        return None;
    };
    let order = bay.order.as_ref()?;
    let closed = order.closed_ic?;
    if order.status != OrderStatus::Done
        || order.right != Right::Salvor
        || order.requester != row.holder
        || closed < row.recovered_ic
    {
        return None;
    }
    let goods: u64 = order
        .goods
        .iter()
        .map(|g| reference_minor(w, &g.good).saturating_mul(g.units))
        .fold(0, u64::saturating_add);
    let mut description = order
        .jobs
        .iter()
        .map(|job| job.job.describe())
        .collect::<Vec<_>>()
        .join(", ");
    if description.len() > 160 {
        let mut cut = 157;
        while !description.is_char_boundary(cut) {
            cut -= 1;
        }
        description.truncate(cut);
        description.push_str("...");
    }
    Some(AtRiskRepairEntry {
        description: smol_str::SmolStr::new(description),
        cost_incurred: u64::try_from(order.fee_minor)
            .unwrap_or(0)
            .saturating_add(goods),
        performed_at_ic: closed,
    })
}

/// The repairs at risk a notice lists (doc 41 Q-41-20), or nothing.
fn at_risk_words(ctx: &Ctx<'_>, entries: &[AtRiskRepairEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let list = entries
        .iter()
        .map(|entry| {
            format!(
                "{} ({})",
                entry.description,
                crate::port::coop::money_minor(
                    ctx,
                    i64::try_from(entry.cost_incurred).unwrap_or(i64::MAX)
                )
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    AT_RISK_NOTICE.replace("{list}", &list)
}

/// Why the vacating contest won, in words.
fn why_vacated(file: &SalvageClaim) -> &'static str {
    use mush_tactics::field_ops::ContestSuccessReason;
    match file
        .contests
        .iter()
        .find_map(|contest| contest.valid_to_vacate.clone())
    {
        Some(ContestSuccessReason::DefectiveProcedure) => WHY_PROCEDURE,
        Some(ContestSuccessReason::SuperiorSpecificRight) => WHY_RIGHT,
        Some(ContestSuccessReason::ProvenUnlawfulRecovery) | None => WHY_RECOVERY,
    }
}

/// Whether a claim was vacated on a proven unlawful recovery (a `recovery` contest).
fn vacated_on_recovery(file: &SalvageClaim) -> bool {
    use mush_tactics::field_ops::ContestSuccessReason;
    matches!(
        file.contests
            .iter()
            .find_map(|contest| contest.valid_to_vacate.clone()),
        Some(ContestSuccessReason::ProvenUnlawfulRecovery)
    )
}

/// **The recovery a `recovery` vacate proves** (doc 41 cut RS.7): the site's `Recovery` row on
/// the hull by the custodian, the latest; when the ring has dropped it, the row as the custody
/// knew it.
fn proven_recovery(ctx: &Ctx<'_>, site: &CanonicalId, row: &SalvageCustody) -> UnlawfulAct {
    crate::surface::tow::record_or_new(ctx, site)
        .unlawful
        .iter()
        .rev()
        .find(|act| {
            act.kind == UnlawfulKind::Recovery
                && act.actor == row.holder
                && act.subject == row.hull.0
        })
        .cloned()
        .unwrap_or_else(|| UnlawfulAct {
            kind: UnlawfulKind::Recovery,
            actor: row.holder,
            subject: row.hull.0,
            landholder: row.pin.as_ref().and_then(TenurePin::landholder),
            at_ic: row.recovered_ic,
        })
}

/// **Whether an organization's hull is under a live lease or charter** (doc 41 Q-41-22, doc 36
/// Q-36-29): a rig lease, or a programme charter; such a hull is never awarded by salvage.
fn leased_or_chartered(w: &World, hull: ObjId) -> bool {
    w.module_state::<crate::store::TacsimStore>()
        .and_then(|store| store.tv_lease_on(hull))
        .is_some()
        || crate::port::coop::charter_tenant(w, hull).is_some()
}

/// **The claims' tick** (on the surface tick, beside the tow's): every filed claim whose window
/// has run or which a contest vacates is settled — the economy first (the title award, or the
/// contract's close), then the site's record — and the parties are told. A salvor's completed
/// bay order is written onto the claim first (doc 41 Q-41-20), and a vacated claim's listed lots
/// are flagged hot one act at a time (§4.6).
#[allow(clippy::too_many_lines)]
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let now_real = ctx.w.now();
    let registry = registry_name(ctx.w);
    let Some(issuer) = mush_org::SystemPrincipalId::new(AWARD_ISSUER) else {
        return;
    };
    let waiting: Vec<ObjId> = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .map(crate::store::TacsimStore::bay_vacated_lot_hulls)
        .unwrap_or_default();
    for hull in waiting {
        let _ = crate::port::bay_work::strip::flag_next_vacated(ctx, hull);
    }
    for (site, row) in custody_rows(ctx) {
        if ctx.authoritative_pending(row.holder) || promoted(ctx, &site) {
            continue;
        }
        // Doc 41 Q-41-20: a repair the salvor finished is written before anything settles.
        if !crate::port::tv::written_off(ctx.w, row.hull)
            && let Some(entry) = at_risk_of(ctx.w, &row)
        {
            let kept = row
                .file
                .as_ref()
                .map_or(&row.at_risk, |file| &file.documented_repairs_at_risk);
            if !kept.contains(&entry) {
                let order = ClaimOrder::AtRisk {
                    site: site.clone(),
                    hull: row.hull,
                    entry,
                };
                let _ = submit(ctx, row.holder, order, 7);
                continue;
            }
        }
        // Doc 36a P0-3 (Q-36-29, Q-36-22): a hull written off while in custody is Occator's
        // alone — the write-off already handed it back to the shed. Its claim's contract is
        // closed and the custody row goes; the title never moves.
        if crate::port::tv::written_off(ctx.w, row.hull) {
            let closed = match row.claim {
                ClaimSeam::Owed => true,
                ClaimSeam::Filed { contract } => match mush_economy::close_salvage_claim(
                    ctx,
                    ContractId(contract),
                    row.holder,
                    row.hull,
                ) {
                    Ok(done) => done,
                    Err(error) => {
                        tracing::warn!(hull = row.hull.0, contract, %error, "tacsim: a written-off hull's salvage claim did not close; retried next tick");
                        false
                    }
                },
            };
            if closed {
                let order = ClaimOrder::Settle {
                    site: site.clone(),
                    hull: row.hull,
                    settlement: Settlement::HandedBack,
                    window_end: now_real,
                    now_real,
                };
                let _ = submit(ctx, row.holder, order, 5);
            }
            continue;
        }
        let ClaimSeam::Filed { contract } = row.claim else {
            continue;
        };
        let Some(file) = row.file.as_deref() else {
            continue;
        };
        if !file.state.is_pending() {
            continue;
        }
        let state = contract_state(ctx.w, contract);
        let (end, paused) = effective_window(ctx.w, &row).unwrap_or((file.deadline_real_s, false));
        let settlement = if file.has_vacating_contest() {
            Settlement::Vacate
        } else if matches!(state, Some(ContractState::Cancelled) | None) {
            Settlement::Withdrawn
        } else if matches!(state, Some(ContractState::Expired)) || (!paused && now_real >= end) {
            Settlement::Award
        } else {
            continue;
        };
        // Doc 36a P0-3 (Q-36-29), narrowed by doc 41 Q-41-22: an organization's hull under a
        // live lease or charter is never awarded by salvage — a leased rig is Occator's alone,
        // and the 28-day write-off takes it home — so the claim stands pending until then, or
        // until a contest vacates it. An organization's hull with neither is awarded.
        if settlement == Settlement::Award
            && matches!(
                title_of(ctx.w, row.hull).0,
                Some(PrincipalId::Organization(_))
            )
            && leased_or_chartered(ctx.w, row.hull)
        {
            continue;
        }
        let label = name(ctx.w, row.hull);
        let ready = match settlement {
            Settlement::Award => match mush_economy::award_salvage_title(
                ctx,
                ContractId(contract),
                row.holder,
                row.hull,
                issuer.clone(),
            ) {
                Ok(_) => true,
                Err(mush_economy::EconomyError::RetryShortly) => false,
                Err(error) => {
                    tracing::warn!(hull = row.hull.0, contract, %error, "tacsim: a salvage award did not post; retried next tick");
                    false
                }
            },
            Settlement::Vacate => {
                match mush_economy::close_salvage_claim(
                    ctx,
                    ContractId(contract),
                    row.holder,
                    row.hull,
                ) {
                    Ok(done) => done,
                    Err(error) => {
                        tracing::warn!(hull = row.hull.0, contract, %error, "tacsim: a vacated salvage claim's contract did not close");
                        false
                    }
                }
            }
            Settlement::Withdrawn | Settlement::HandedBack => true,
        };
        if !ready {
            continue;
        }
        let order = ClaimOrder::Settle {
            site: site.clone(),
            hull: row.hull,
            settlement,
            window_end: end,
            now_real,
        };
        if !submit(ctx, row.holder, order, 5) {
            continue;
        }
        // Doc 41 cut RS.4 (Q-41-18): a vacated hull is held for her title holder, who takes her
        // out of the custodian's bay; nothing else can move her.
        if settlement == Settlement::Vacate
            && let Some(owner) = title_of(ctx.w, row.hull).0
        {
            crate::port::bay_work::hold_for(ctx, row.hull, owner);
        }
        // Doc 41 cut RS.7 (Q-41-24): a `recovery` contest's vacate is the registry's finding, so
        // the recovery it vacated is proven and goes port-side.
        if settlement == Settlement::Vacate && vacated_on_recovery(file) {
            let act = proven_recovery(ctx, &site, &row);
            let _ = crate::surface::security::prove(
                ctx,
                &site,
                &act,
                crate::surface::security::Proof::Vacated,
            );
        }
        // Doc 41 §4.6: every lot the hull's record lists under this claim is flagged hot.
        if settlement == Settlement::Vacate
            && crate::port::bay_work::strip::claim_vacated(
                ctx,
                row.hull,
                contract,
                (file.prior_owner, row.holder),
            )
        {
            let _ = crate::port::bay_work::strip::flag_next_vacated(ctx, row.hull);
        }
        let mut told: Vec<ObjId> = vec![row.holder];
        for who in row
            .told
            .iter()
            .copied()
            .chain(file.contests.iter().map(|contest| contest.challenger))
        {
            if !told.contains(&who) {
                told.push(who);
            }
        }
        let mut line = match settlement {
            Settlement::Award => AWARDED
                .replace("{registry}", &registry)
                .replace("{name}", &label)
                .replace("{claimant}", &name(ctx.w, row.holder)),
            Settlement::Vacate => VACATED
                .replace("{registry}", &registry)
                .replace("{name}", &label)
                .replace("{why}", why_vacated(file)),
            Settlement::Withdrawn | Settlement::HandedBack => continue,
        };
        line.push_str(&at_risk_words(ctx, &file.documented_repairs_at_risk));
        tell(
            ctx,
            &site,
            &told,
            &NOTICE_SUBJECT.replace("{name}", &label),
            &line,
        );
    }
}

// ---- remains scrapped where they lie (GC.3's follow-up) -----------------------------------

/// The scrap's preview: `{n}`, `{what}`, `{t}`, `{carrier}`.
pub const SCRAP_PREVIEW: &str = "Rig {n} can break up {what} where it lies: {t} t of structural_alloys into the {carrier}'s hold. +tv/{n} scrap go";
/// The scrap done: `{n}`, `{what}`, `{t}`, `{carrier}`.
pub const SCRAPPED: &str = "Rig {n} breaks up {what} where it lies: {t} t of structural_alloys go into the {carrier}'s hold.";
/// Nothing to scrap: `{n}`.
pub const NO_REMAINS: &str = "No remains lie within a cell of rig {n}.";
/// The named object is not remains in reach: `{n}`, `{id}`.
pub const REMAINS_NOT_IN_REACH: &str = "Object {id} is not remains within a cell of rig {n}.";
/// Not the title holder: `{what}`.
pub const SCRAP_NOT_TITLED: &str = "Only the title holder of {what} may break them up; a winning claimant holds that title once the registry awards it.";
/// **The preview of remains under a tonne** (doc 39 cut IN.1, §4.10): scrap mints down to whole
/// tonnes, and on a site, where no facility keeps the fraction, it is a declared loss. `{n}`,
/// `{what}`, `{carrier}`.
/// The hold has no room for the remains' alloys (doc 39 cut IN.8).
pub const SCRAP_NO_ROOM: &str =
    "Rig {n}'s carrier's hold will not take {t} t of structural_alloys: {line}";
pub const SCRAP_LOSS_PREVIEW: &str = "Rig {n} can break up {what} where it lies: under a tonne of structural mass, which is lost; nothing goes into the {carrier}'s hold. +tv/{n} scrap go";
/// The scrap of remains under a tonne done: `{n}`, `{what}`, `{carrier}`.
pub const SCRAPPED_LOSS: &str = "Rig {n} breaks up {what} where it lies: under a tonne of structural mass, lost; nothing goes into the {carrier}'s hold.";
/// No hold to take the alloys: `{n}`.
pub const SCRAP_NO_HOLD: &str =
    "Rig {n}'s carrier has no hold on this ground to take the alloys into.";
/// `+tv/<n> scrap` usage.
pub const SCRAP_USAGE: &str = "Usage: +tv/<n> scrap [object number] [go].";

/// **A scrap's facts**, fixed by the server for the tow provider's prepare.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScrapFacts {
    /// The site.
    pub site: CanonicalId,
    /// The remains' number on the site.
    pub object: u32,
    /// Their hull, as the command read it.
    pub of: Option<ObjId>,
    /// Their mass, kg, as the command read it.
    pub mass_kg: u64,
    /// The tonnes of `structural_alloys` they yield (doc 39 cut IN.8: the design's structure at
    /// the recovery share, capped by the fifth of its reference — §4.6 on a site — and minted
    /// down to whole tonnes, the rest lost to the ground). The server fixes it; the prepare mints
    /// it as fixed.
    pub alloys_t: u64,
    /// The hold vault the lot is laid into, `VaultId.0`.
    pub hold: u64,
    /// The lot's id, derived from the site and the object.
    pub lot: u64,
    /// Whether the scrap is lawful (FO-4).
    pub lawful: bool,
    /// The landholder, on owned ground.
    pub landholder: Option<ObjId>,
    /// **The hold's capacity plan** (doc 39 cut IN.8; doc 36a P2-4), when it is governed: the
    /// prepare checks the alloys fit it by the pools it reads in its own bundle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<serde_json::Value>,
}

/// **The tonnes remains of `of` yield on a site** (doc 39 cut IN.8, §4.6): the design's whole
/// structure at the recovery share, capped at the fifth of its reference price at the alloys'
/// reference — nothing can be stripped on a site (Q-36-39), so the whole reference is
/// scrappable — minted down to whole tonnes. Remains of no known design yield nothing.
#[must_use]
pub fn remains_alloys_t(w: &World, of: Option<ObjId>) -> u64 {
    let Some((design, budget)) = of.and_then(|hull| crate::definitions::mass::hull_budget(w, hull))
    else {
        return 0;
    };
    let reference = crate::definitions::mass::reference_price_minor(w, &design, &budget);
    let alloy =
        crate::definitions::mass::good_reference_minor(w, crate::port::bay_work::strip::SCRAP_GOOD);
    let cap_kg = if alloy == 0 {
        0
    } else {
        u64::try_from(
            u128::from(reference / crate::port::bay_work::scrap::CEILING_DIVISOR)
                * u128::from(crate::port::bay_work::KG_PER_T)
                / u128::from(alloy),
        )
        .unwrap_or(u64::MAX)
    };
    budget.remains_yield_kg().min(cap_kg) / crate::port::bay_work::KG_PER_T
}

/// **The lot a site's remains scrap into**: a hash of the site and the object number.
///
/// # Errors
///
/// A hash that does not build.
pub fn remains_lot(site: &CanonicalId, object: u32) -> Result<mush_inventory::LotId, String> {
    let hash = mush_core::CanonicalHash::for_canonical_value(
        "tacsim.remains_lot.v1",
        1,
        &serde_json::json!({"site": site.as_str(), "object": object}),
        256,
    )
    .map_err(|e| e.to_string())?;
    let head: [u8; 8] = hash
        .as_bytes()
        .get(..8)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| "a canonical hash is shorter than a lot id".to_owned())?;
    Ok(mush_inventory::LotId(u64::from_be_bytes(head)))
}

/// **`+tv/<n> scrap [n] [go]`** (this module's head).
///
/// # Errors
///
/// The line that refuses.
pub(crate) fn scrap_cmd(
    ctx: &mut Ctx<'_>,
    rig: &crate::surface::act::RigAt,
    named_object: Option<u32>,
    go: bool,
) -> Result<(), CmdError> {
    let n = rig.slot.to_string();
    let (site, pose) = crate::surface::tow::ground_pose(ctx.w, rig.tv)
        .ok_or_else(|| CmdError::msg(crate::surface::tow::NOT_ON_GROUND.replace("{n}", &n)))?;
    let here = site_cell(pose.x_cm, pose.y_cm);
    let record = crate::surface::tow::record_or_new(ctx, &site);
    let remains: Vec<&crate::surface::site_record::GroundObject> = record
        .objects
        .iter()
        .filter(|object| matches!(object.kind, GroundKind::Remains { .. }))
        .filter(|object| {
            cells_apart(here, object.pose.cell())
                <= crate::surface::salvage::PICKUP_REACH_CELLS
                    .saturating_add(u64::from(object.footprint / 2))
        })
        .collect();
    let found = match named_object {
        Some(id) => remains
            .iter()
            .find(|object| object.id == id)
            .copied()
            .ok_or_else(|| {
                CmdError::msg(
                    REMAINS_NOT_IN_REACH
                        .replace("{n}", &n)
                        .replace("{id}", &id.to_string()),
                )
            })?,
        None => remains
            .first()
            .copied()
            .ok_or_else(|| CmdError::msg(NO_REMAINS.replace("{n}", &n)))?,
    };
    let GroundKind::Remains { of, mass_kg, .. } = &found.kind else {
        return Err(CmdError::msg(NO_REMAINS.replace("{n}", &n)));
    };
    let what = match of {
        Some(hull) => format!("the remains of the {}", name(ctx.w, *hull)),
        None => "the remains".to_owned(),
    };
    let tenure = tenure_at(ctx, &site, found.pose.cell());
    let landholder = match &tenure {
        GroundTenure::Owned { landholder, .. } => Some(*landholder),
        GroundTenure::Unclaimed => None,
    };
    let title = of.map(|hull| title_of(ctx.w, hull).0).unwrap_or_default();
    let lawful = match &title {
        Some(PrincipalId::Object(holder)) if *holder == ctx.actor => true,
        Some(_) => return Err(CmdError::msg(SCRAP_NOT_TITLED.replace("{what}", &what))),
        None => landholder.is_none_or(|holder| holder == ctx.actor),
    };
    // Doc 39 cut IN.8 (§4.6): the capped yield, down to whole tonnes; under one, a declared loss.
    let alloys_t = remains_alloys_t(ctx.w, *of);
    let (preview, done) = if alloys_t == 0 {
        (SCRAP_LOSS_PREVIEW, SCRAPPED_LOSS)
    } else {
        (SCRAP_PREVIEW, SCRAPPED)
    };
    let hold = crate::port::ship::hull_view(ctx.w, rig.hull)
        .and_then(|view| crate::port::ship::hold_vault(ctx.w, &view))
        .filter(|_| {
            crate::surface::tick::hull_view_berth(ctx.w, rig.hull)
                .is_some_and(|(there, ..)| there == site)
        })
        .ok_or_else(|| CmdError::msg(SCRAP_NO_HOLD.replace("{n}", &n)))?;
    // Doc 39 cut IN.8 (the room check): the alloys must fit the hold as it stands.
    let plan = remains_room(ctx, hold, alloys_t).map_err(|line| {
        CmdError::msg(
            SCRAP_NO_ROOM
                .replace("{n}", &n)
                .replace("{t}", &alloys_t.to_string())
                .replace("{line}", &line),
        )
    })?;
    let carrier = name(ctx.w, rig.hull);
    let fill = |line: &str| {
        line.replace("{n}", &n)
            .replace("{what}", &what)
            .replace("{t}", &alloys_t.to_string())
            .replace("{carrier}", &carrier)
    };
    if !go {
        ctx.out.sys_to_actor(ctx.actor, "tacsim.tow", fill(preview));
        return Ok(());
    }
    let lot = remains_lot(&site, found.id).map_err(CmdError::msg)?;
    let facts = ScrapFacts {
        site,
        object: found.id,
        of: *of,
        mass_kg: *mass_kg,
        alloys_t,
        hold: hold.0,
        lot: lot.0,
        lawful,
        landholder,
        plan,
    };
    let actor = ctx.actor;
    if !crate::surface::tow::submit(
        ctx,
        actor,
        rig.tv,
        crate::surface::tow::TowOrder::Scrap {
            facts: Box::new(facts),
        },
        0,
    ) {
        return Err(CmdError::msg(crate::port::tv::TV_UNWIRED));
    }
    ctx.out.sys_to_actor(actor, "tacsim.tow", fill(done));
    Ok(())
}

/// **Whether `hold` takes `alloys_t` of the remains' alloys** (doc 39 cut IN.8; doc 36a P2-4):
/// the capacity plan of a governed hold, to carry to the prepare, once the pools as they stand
/// fit; `Ok(None)` for an ungoverned hold or nothing to lay.
///
/// # Errors
///
/// The refusal's line: the hold's own, or that it cannot be read.
fn remains_room(
    ctx: &Ctx<'_>,
    hold: mush_economy::VaultId,
    alloys_t: u64,
) -> Result<Option<serde_json::Value>, String> {
    use mush_market::{CapacityProvider, CapacityReadError, CapacityRefusal};
    let endpoint = mush_inventory::EndpointRef(hold);
    let Some(plan) = crate::port::market::PortCapacity.plan(ctx.w, endpoint) else {
        return Ok(None);
    };
    if alloys_t == 0 {
        return Ok(None);
    }
    let good =
        CanonicalId::new(crate::port::bay_work::strip::SCRAP_GOOD).map_err(|e| e.to_string())?;
    let mut read = |good: &CanonicalId| {
        crate::port_tick::read_pool(ctx, endpoint, good).map_err(|_| CapacityReadError)
    };
    match crate::port::market::PortCapacity.check(&plan, &good, alloys_t, &mut read) {
        Ok(()) => Ok(Some(plan.0)),
        Err(CapacityRefusal::Refused { line, .. }) => Err(line),
        Err(CapacityRefusal::Unreadable) => Err("its room cannot be read".to_owned()),
    }
}

/// The recovery operation a load closed, as the custody record keeps it.
#[must_use]
pub fn operation_of(custody: &SalvageCustody) -> Option<RecoveryOpId> {
    custody.operation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::salvage::{ClaimSeam, SalvageCustody};

    fn custody(pin: TenurePin) -> SalvageCustody {
        SalvageCustody {
            hull: ObjId(7),
            holder: ObjId(20),
            carrier: ObjId(21),
            recovered_by: ObjId(22),
            recovered_ic: 100,
            left_ic: Some(10),
            claim: ClaimSeam::Owed,
            operation: Some(RecoveryOpId::new(3).expect("an op")),
            pin: Some(pin),
            file: None,
            told: Vec::new(),
            evidence: Vec::new(),
            at_risk: Vec::new(),
        }
    }

    fn pin_on(tenure: GroundTenure) -> TenurePin {
        TenurePin {
            casualty: ObjId(7),
            tug: ObjId(22),
            recoverer: ObjId(20),
            title: Some(PrincipalId::Object(ObjId(30))),
            answering: Some(ObjId(30)),
            answerers: Vec::new(),
            owner_grant: None,
            landholder_authorized: false,
            tenure,
            witnesses: vec![Witness {
                holder: ObjId(30),
                unit: ObjId(31),
                kind: WitnessKind::Sensor,
            }],
            pinned_ic: 50,
        }
    }

    /// **Doc 42 cut CR.1** (Q-42-22): an unlawful recovery that carried crew writes one `Capture`
    /// row a captive, naming the captor and the captive; the row round-trips, and a load that
    /// carried nobody writes none.
    #[test]
    fn cr1_a_capture_off_owned_ground_is_evidence_naming_captor_and_captive() {
        let mut record = SiteRecord::new(CanonicalId::new("ahuna_port").expect("a site"));
        log_captures(&mut record, ObjId(20), ObjId(40), &[], 500);
        assert!(record.unlawful.is_empty());
        log_captures(
            &mut record,
            ObjId(20),
            ObjId(40),
            &[ObjId(7), ObjId(8)],
            500,
        );
        assert_eq!(record.unlawful.len(), 2);
        let row = &record.unlawful[0];
        assert_eq!(
            (row.kind, row.actor, row.subject, row.landholder, row.at_ic),
            (UnlawfulKind::Capture, ObjId(20), 7, Some(ObjId(40)), 500)
        );
        let wire = serde_json::to_value(row).expect("encodes");
        assert_eq!(wire["kind"], "capture");
        let back: UnlawfulAct = serde_json::from_value(wire).expect("decodes");
        assert_eq!(&back, row);
    }

    /// **Q-36-26**: the same tug's reattach keeps the pin; another tug's attach replaces it.
    #[test]
    fn gc6_a_pin_survives_the_same_tugs_reattach_and_not_another_tugs() {
        let mut pins: Vec<TenurePin> = Vec::new();
        pin(&mut pins, pin_on(GroundTenure::Unclaimed));
        let mut later = pin_on(GroundTenure::Owned {
            landholder: ObjId(40),
            claim_id: None,
        });
        later.pinned_ic = 90;
        pin(&mut pins, later.clone());
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].pinned_ic, 50, "the same tug keeps its first pin");
        later.tug = ObjId(23);
        pin(&mut pins, later);
        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].tug, ObjId(23), "another tug starts it again");
        assert_eq!(pins[0].landholder(), Some(ObjId(40)));
    }

    /// **Q-36-27, Q-36-28, Q-R1**: on owned ground a witness's own record vacates the claim; bare
    /// title alone never does, so an uncontested (or barely contested) claim is awarded at the
    /// deadline; and a report hands evidence on.
    #[test]
    fn gc6_evidence_vacates_an_unlawful_claim_and_bare_title_never_does() {
        let owned = GroundTenure::Owned {
            landholder: ObjId(40),
            claim_id: None,
        };
        let mut record = SiteRecord::new(CanonicalId::new("tacsim.port.urvara").expect("an id"));
        record.custody.push(custody(pin_on(owned)));
        let file = ClaimOrder::File {
            site: record.site.clone(),
            hull: ObjId(7),
            contract: 9,
            window_end: 1_000,
            now_real: 100,
            told: vec![ObjId(30), ObjId(40)],
        };
        assert_eq!(
            prepare(&mut record, ObjId(21), &file, 1),
            Err("claim_not_custodian")
        );
        assert_eq!(
            prepare(&mut record, ObjId(20), &file, 1).as_deref(),
            Ok("filed")
        );
        assert_eq!(
            prepare(&mut record, ObjId(20), &file, 1),
            Err("claim_filed")
        );
        let filed = record.custody[0].file.clone().expect("the claim");
        assert!(!filed.entitlement_at_attach.is_lawful(), "{filed:?}");
        // The landholder holds no evidence until the title holder reports to them.
        assert!(evidence_of(&record.custody[0], ObjId(40)).is_none());
        let report = ClaimOrder::Report {
            site: record.site.clone(),
            hull: ObjId(7),
            to: ObjId(40),
        };
        assert_eq!(
            prepare(&mut record, ObjId(40), &report, 2),
            Err("claim_no_evidence")
        );
        assert_eq!(
            prepare(&mut record, ObjId(30), &report, 2).as_deref(),
            Ok("reported")
        );
        let evidence = evidence_of(&record.custody[0], ObjId(40)).expect("delivered");
        let contest = ClaimOrder::Contest {
            site: record.site.clone(),
            hull: ObjId(7),
            ground: ContestGround::UnlawfulRecovery { evidence },
            window_end: 1_000,
            now_real: 200,
        };
        assert_eq!(
            prepare(&mut record, ObjId(40), &contest, 3).as_deref(),
            Ok("contested")
        );
        assert!(
            record.custody[0]
                .file
                .as_ref()
                .expect("filed")
                .has_vacating_contest()
        );
        let award = settle_on(&record, Settlement::Award, 1_200);
        assert_eq!(
            prepare(&mut record, ObjId(20), &award, 4),
            Err("claim_settled_otherwise")
        );
        let vacate = settle_on(&record, Settlement::Vacate, 1_200);
        assert_eq!(
            prepare(&mut record, ObjId(20), &vacate, 4).as_deref(),
            Ok("vacated")
        );
        assert!(record.custody.is_empty());
        assert_eq!(record.unlawful.len(), 1);
        assert_eq!(record.unlawful[0].kind, UnlawfulKind::Vacated);

        // Open ground: lawful; bare title is recorded and does not vacate; awarded at the end.
        record
            .custody
            .push(custody(pin_on(GroundTenure::Unclaimed)));
        let refile = file_on(&record, 10);
        assert_eq!(
            prepare(&mut record, ObjId(20), &refile, 5).as_deref(),
            Ok("filed")
        );
        let bare = ClaimOrder::Contest {
            site: record.site.clone(),
            hull: ObjId(7),
            ground: ContestGround::BarePriorTitle,
            window_end: 1_000,
            now_real: 300,
        };
        assert_eq!(
            prepare(&mut record, ObjId(40), &bare, 6),
            Err("claim_no_ground")
        );
        assert_eq!(
            prepare(&mut record, ObjId(30), &bare, 6).as_deref(),
            Ok("contested")
        );
        let early = settle_on(&record, Settlement::Award, 900);
        assert_eq!(
            prepare(&mut record, ObjId(20), &early, 7),
            Err("claim_not_due")
        );
        let due = settle_on(&record, Settlement::Award, 1_000);
        assert_eq!(
            prepare(&mut record, ObjId(20), &due, 7).as_deref(),
            Ok("awarded")
        );
        assert!(record.custody.is_empty());
    }

    fn file_on(record: &SiteRecord, contract: u64) -> ClaimOrder {
        ClaimOrder::File {
            site: record.site.clone(),
            hull: ObjId(7),
            contract,
            window_end: 1_000,
            now_real: 100,
            told: vec![ObjId(30)],
        }
    }

    fn settle_on(record: &SiteRecord, settlement: Settlement, now_real: i64) -> ClaimOrder {
        ClaimOrder::Settle {
            site: record.site.clone(),
            hull: ObjId(7),
            settlement,
            window_end: 1_000,
            now_real,
        }
    }

    fn contest_on(ground: ContestGround) -> ClaimOrder {
        ClaimOrder::Contest {
            site: CanonicalId::new("tacsim.port.urvara").expect("an id"),
            hull: ObjId(7),
            ground,
            window_end: 1_000,
            now_real: 200,
        }
    }

    fn filed_record(pin: TenurePin, told: Vec<ObjId>) -> SiteRecord {
        let mut record = SiteRecord::new(CanonicalId::new("tacsim.port.urvara").expect("an id"));
        record.custody.push(custody(pin));
        let mut file = file_on(&record, 11);
        if let ClaimOrder::File { told: kept, .. } = &mut file {
            *kept = told;
        }
        assert_eq!(
            prepare(&mut record, ObjId(20), &file, 1).as_deref(),
            Ok("filed")
        );
        record
    }

    /// **Doc 41 Q-41-23 (round-2 item 41-1)**: the registry's procedure reading — an answerer
    /// not told is a defect, and only an answerer contests on it; the black box is compared with
    /// the custody row's tug and carrier, never with the claimant; a clean claim shows nothing.
    #[test]
    fn rs3_procedure_reads_the_audience_the_pin_and_the_box_against_the_custody() {
        let mut org_pin = pin_on(GroundTenure::Unclaimed);
        org_pin.title = Some(PrincipalId::Organization(mush_org::OrgId(5)));
        org_pin.answerers = vec![ObjId(30), ObjId(33)];
        // Filed telling 30 alone: 33 answered for the title and was never told.
        let mut record = filed_record(org_pin.clone(), vec![ObjId(30)]);
        let row = record.custody[0].clone();
        assert_eq!(
            procedure_defect(&row, None).as_deref(),
            Some("unnotified:33")
        );
        let ground = ContestGround::DefectiveProcedure {
            defect_code: smol_str::SmolStr::new("unnotified:33"),
        };
        assert_eq!(
            prepare(&mut record, ObjId(40), &contest_on(ground.clone()), 2),
            Err("claim_not_answering"),
            "the landholder of nothing, a stranger, answers for no title"
        );
        let wrong = ContestGround::DefectiveProcedure {
            defect_code: smol_str::SmolStr::new("pin_missing"),
        };
        assert_eq!(
            prepare(&mut record, ObjId(33), &contest_on(wrong), 2),
            Err("claim_no_defect")
        );
        assert_eq!(
            prepare(&mut record, ObjId(33), &contest_on(ground), 2).as_deref(),
            Ok("contested")
        );
        assert!(
            record.custody[0]
                .file
                .as_ref()
                .expect("filed")
                .has_vacating_contest()
        );
        let vacate = settle_on(&record, Settlement::Vacate, 300);
        assert_eq!(
            prepare(&mut record, ObjId(20), &vacate, 3).as_deref(),
            Ok("vacated")
        );

        // Everyone told: the box agreeing with the custody (tug 22 into carrier 21) is no
        // defect, though the claimant (20) is neither; a box naming another tug or carrier is.
        let record = filed_record(org_pin, vec![ObjId(30), ObjId(33)]);
        let row = &record.custody[0];
        let agrees = BoxRecovery {
            base: ObjId(21),
            recovered_by: Some(ObjId(22)),
        };
        assert_eq!(procedure_defect(row, None), None);
        assert_eq!(procedure_defect(row, Some(agrees)), None);
        assert_eq!(
            procedure_defect(
                row,
                Some(BoxRecovery {
                    recovered_by: Some(ObjId(23)),
                    ..agrees
                })
            )
            .as_deref(),
            Some("box_recoverer")
        );
        assert_eq!(
            procedure_defect(
                row,
                Some(BoxRecovery {
                    base: ObjId(24),
                    ..agrees
                })
            )
            .as_deref(),
            Some("box_base")
        );
        let mut replaced = row.clone();
        if let Some(pin) = replaced.pin.as_mut() {
            pin.tug = ObjId(23);
        }
        assert_eq!(
            procedure_defect(&replaced, None).as_deref(),
            Some("pin_replaced")
        );
    }

    /// **Doc 41 Q-41-23 (round-2 item 41-10)**: a right vacates only when it bound the claimant
    /// and its grantor files it; a third party's authorization is no ground. The authorization
    /// read at the attach makes the recovery lawful, so the same claimant's recovery stands
    /// against a recovery contest.
    #[test]
    fn rs3_a_right_binds_the_claimant_and_only_its_grantor_files_it() {
        let mut pin = pin_on(GroundTenure::Owned {
            landholder: ObjId(40),
            claim_id: None,
        });
        pin.owner_grant = Some(PinnedGrant {
            grant: 9,
            grantor: ObjId(30),
        });
        let mut record = filed_record(pin, vec![ObjId(30), ObjId(40)]);
        let row = record.custody[0].clone();
        assert!(
            row.file
                .as_ref()
                .expect("filed")
                .entitlement_at_attach
                .is_lawful(),
            "the title holder's authorization, read at the attach"
        );
        assert_eq!(
            right_of_grantor(&row, ObjId(40)),
            None,
            "a third party holds none"
        );
        let (contract_id, obligor) = right_of_grantor(&row, ObjId(30)).expect("T's right");
        assert_eq!((contract_id.as_str(), obligor), ("grant/9", ObjId(20)));
        let ground = ContestGround::SuperiorSpecificRight {
            contract_id: smol_str::SmolStr::new(contract_id),
            obligor,
        };
        assert_eq!(
            prepare(&mut record, ObjId(40), &contest_on(ground.clone()), 2),
            Err("claim_no_right")
        );
        // The witness's own record is evidence, but of a lawful recovery: no vacate.
        let evidence = evidence_of(&record.custody[0], ObjId(30)).expect("T saw it");
        let recovery = contest_on(ContestGround::UnlawfulRecovery { evidence });
        assert_eq!(
            prepare(&mut record, ObjId(30), &recovery, 2).as_deref(),
            Ok("contested")
        );
        assert!(
            !record.custody[0]
                .file
                .as_ref()
                .expect("filed")
                .has_vacating_contest()
        );
        assert_eq!(
            prepare(&mut record, ObjId(30), &contest_on(ground), 3).as_deref(),
            Ok("contested")
        );
        assert!(
            record.custody[0]
                .file
                .as_ref()
                .expect("filed")
                .has_vacating_contest()
        );
        // A grant the claimant never held (another recoverer took it in tow) binds nobody.
        let mut other = record.custody[0].clone();
        if let Some(pin) = other.pin.as_mut() {
            pin.recoverer = ObjId(25);
        }
        assert_eq!(right_of_grantor(&other, ObjId(30)), None);
    }

    /// **Doc 41 Q-41-20 (Q-S14)**: a repair at the salvor's risk is kept on the custody before
    /// filing and carried into the claim at filing; after, it goes onto the claim; once each.
    #[test]
    fn rs3_repairs_at_risk_ride_the_claim() {
        let mut record = SiteRecord::new(CanonicalId::new("tacsim.port.urvara").expect("an id"));
        record
            .custody
            .push(custody(pin_on(GroundTenure::Unclaimed)));
        let entry = |at: i64| AtRiskRepairEntry {
            description: smol_str::SmolStr::new("mend the chassis"),
            cost_incurred: 125_000,
            performed_at_ic: at,
        };
        let at_risk = |at: i64| ClaimOrder::AtRisk {
            site: CanonicalId::new("tacsim.port.urvara").expect("an id"),
            hull: ObjId(7),
            entry: entry(at),
        };
        assert_eq!(
            prepare(&mut record, ObjId(30), &at_risk(60), 1),
            Err("claim_not_custodian")
        );
        for _ in 0..2 {
            assert_eq!(
                prepare(&mut record, ObjId(20), &at_risk(60), 1).as_deref(),
                Ok("recorded")
            );
        }
        assert_eq!(record.custody[0].at_risk, vec![entry(60)]);
        let file = file_on(&record, 12);
        assert_eq!(
            prepare(&mut record, ObjId(20), &file, 2).as_deref(),
            Ok("filed")
        );
        assert!(record.custody[0].at_risk.is_empty());
        assert_eq!(
            prepare(&mut record, ObjId(20), &at_risk(90), 3).as_deref(),
            Ok("recorded")
        );
        assert_eq!(
            record.custody[0]
                .file
                .as_ref()
                .expect("filed")
                .documented_repairs_at_risk,
            vec![entry(60), entry(90)]
        );
    }

    /// **Doc 41 Q-41-21**: a landholder's delegation is the landholder's alone to write, one row
    /// a grantee, and covers only under the holding it was issued on and before it lapses.
    #[test]
    fn rs3_a_land_grant_rides_its_holding() {
        let mut record = SiteRecord::new(CanonicalId::new("tacsim.port.urvara").expect("an id"));
        let grant = LandGrant {
            letter: 'F',
            landholder: ObjId(40),
            registered_ic: 10,
            to: ObjId(20),
            until_ic: Some(500),
            issued_ic: 12,
        };
        let order = ClaimOrder::AuthorizeBlock {
            site: record.site.clone(),
            place: ObjId(99),
            grant: grant.clone(),
        };
        assert_eq!(order.hull(), ObjId(99));
        assert_eq!(
            prepare(&mut record, ObjId(20), &order, 12),
            Err("claim_not_landholder")
        );
        for _ in 0..2 {
            assert_eq!(
                prepare(&mut record, ObjId(40), &order, 12).as_deref(),
                Ok("authorized")
            );
        }
        assert_eq!(record.land_grants, vec![grant.clone()]);
        assert!(grant.covers(ObjId(20), (ObjId(40), 10), 100));
        assert!(
            !grant.covers(ObjId(20), (ObjId(40), 11), 100),
            "a new holding"
        );
        assert!(!grant.covers(ObjId(20), (ObjId(40), 10), 500), "lapsed");
        assert!(
            !grant.covers(ObjId(21), (ObjId(40), 10), 100),
            "another recoverer"
        );
        assert!(record.check().is_ok());
    }

    /// **Doc 41 cut RS.10**: an ask is the asker's own; silence for 28 IC days is consent; a
    /// refusal is the landholder's, voids its delegation, blocks a new ask for 28 IC days and
    /// then lapses.
    #[test]
    fn rs10_an_ask_is_consent_by_silence_and_a_refusal_blocks() {
        let mut record = SiteRecord::new(CanonicalId::new("tacsim.port.urvara").expect("an id"));
        let held = (ObjId(40), 10);
        let ask = ConsentAsk {
            letter: 'F',
            landholder: ObjId(40),
            registered_ic: 10,
            asker: ObjId(20),
            asked_ic: 100,
            refused_ic: None,
        };
        let asked = ClaimOrder::Ask {
            site: record.site.clone(),
            place: ObjId(99),
            ask: ask.clone(),
        };
        assert_eq!(asked.hull(), ObjId(99));
        assert_eq!(
            consent_on(&record, 'F', held, ObjId(20), 100),
            Consent::NotAsked
        );
        assert_eq!(
            consent_on(&record, 'F', held, ObjId(40), 100),
            Consent::Given
        );
        assert_eq!(
            prepare(&mut record, ObjId(21), &asked, 100),
            Err("claim_not_asker")
        );
        assert_eq!(
            prepare(&mut record, ObjId(20), &asked, 100).as_deref(),
            Ok("asked")
        );
        assert_eq!(
            consent_on(&record, 'F', held, ObjId(20), 101),
            Consent::Asked
        );
        assert_eq!(
            consent_on(&record, 'F', held, ObjId(20), 100 + CONSENT_IC_SECS),
            Consent::Given,
            "silence for 28 IC days"
        );
        assert_eq!(
            consent_on(
                &record,
                'F',
                (ObjId(40), 11),
                ObjId(20),
                100 + CONSENT_IC_SECS
            ),
            Consent::NotAsked,
            "a new holding"
        );
        // A delegation, then the refusal that voids it.
        record.land_grants.push(LandGrant {
            letter: 'F',
            landholder: ObjId(40),
            registered_ic: 10,
            to: ObjId(20),
            until_ic: None,
            issued_ic: 120,
        });
        let refused = ClaimOrder::Refuse {
            site: record.site.clone(),
            place: ObjId(99),
            ask: ConsentAsk {
                refused_ic: Some(200),
                ..ask.clone()
            },
        };
        assert_eq!(
            prepare(&mut record, ObjId(20), &refused, 200),
            Err("claim_not_landholder")
        );
        assert_eq!(
            prepare(&mut record, ObjId(40), &refused, 200).as_deref(),
            Ok("refused")
        );
        assert!(
            record.land_grants.is_empty(),
            "the refusal voids the delegation"
        );
        assert_eq!(record.consent_asks.len(), 1);
        assert_eq!(
            consent_on(&record, 'F', held, ObjId(20), 201),
            Consent::Refused
        );
        assert_eq!(
            prepare(&mut record, ObjId(20), &asked, 300),
            Err("claim_refused"),
            "no new ask while the refusal stands"
        );
        let lapsed = 200 + CONSENT_IC_SECS;
        assert_eq!(
            consent_on(&record, 'F', held, ObjId(20), lapsed),
            Consent::NotAsked
        );
        let again = ClaimOrder::Ask {
            site: record.site.clone(),
            place: ObjId(99),
            ask: ConsentAsk {
                asked_ic: lapsed,
                ..ask
            },
        };
        assert_eq!(
            prepare(&mut record, ObjId(20), &again, lapsed).as_deref(),
            Ok("asked")
        );
        assert_eq!(
            consent_on(&record, 'F', held, ObjId(20), lapsed),
            Consent::Asked
        );
        assert!(record.check().is_ok());
    }
}
