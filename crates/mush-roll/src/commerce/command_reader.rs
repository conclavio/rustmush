//! 17g C2a (C2 plan §2.4): the bounded Roll read kinds behind the line-mode
//! mind commands. Every kind answers only for the calling actor, whom core
//! supplies from dispatch; no request names another character. It is
//! source-free and carries no digest, account id, rating or other holder: a
//! command builds its wrapped action's parameters from these facts, and the
//! action's own provider then rechecks every authority at preparation.
//!
//! **Capacity (C2 plan §2.4, fallback 1).** The format-2 Active launch already
//! held 31 owner-read services and all 32 grants before C2, so these kinds
//! are not a service of their own: they are folded into
//! `holding_reader::OriginalOwnerReader` (`original_owner_holding` v3), which
//! is already own-only over `RollStore` and already granted to TacSim. A
//! command wraps its request in `OriginalReadRequest::command` and unwraps
//! `OriginalReadResult::into_command`. The kinds answer only when Active
//! commerce composes the player market (`CommerceConfig::with_mind_entitlements`
//! sets `with_mind_commands`); everywhere else they read
//! `OwnerReadError::Unavailable`.
//!
//! - `OpenShell`: the request id of the actor's own open filing reservation,
//!   through `ShellReservations::open_request` (the core accessor C2a adds).
//!   `@mind/file` reserves first when there is none, and files with it after.
//! - `OwnFiled { filed }`: for a filed revision whose charter's title the
//!   actor holds now, the charter, line and owner licence the commands derive
//!   from it (17g C2 plan §2.2: the position names the line and charter, the
//!   charter's initial filing operation names the owner licence), whether the
//!   line is dependent, the charter's public name and the owner licence's
//!   current phase. Anything else, a foreign filed number, a missing one or a
//!   row that cannot be read, is `OwnerReadError::Refused`, identically, so
//!   the read is no oracle for another holder's filings.
//!
//! - `OwnHoldings { section: Owner, after }` (17g C2c): the filed minds the
//!   actor owns now, through the actor's own E4 charter assets (the
//!   original-holdings page's `assets_of_after`, so no other holder's index
//!   is walked); each row is exactly what `OwnFiled` answers for the line's
//!   newest filed revision (`LineRecord::head`), plus `selling` when the
//!   charter has a funded pending transfer. A charter `OwnFiled` would refuse
//!   (a title in transit) is left out. At most `OWNER_ROWS_PER_READ` rows
//!   and `OWN_ASSETS_PER_READ` assets per read, inside the declared
//!   `MAXIMUM_ROWS`; the command pages with the asset cursor.
//! - `OwnHoldings { section: Paid, after }` (17g C2g, the C2e plan §4.1
//!   item 1): the paid licences the actor holds now, through 17g C2f's
//!   per-holder index (`roll_mind_paid_holdings`, `paid_holdings::page`):
//!   one bounded scan of the actor's own key prefix, so a neighbour holder's
//!   rows are never read, and each row built exactly as `OwnPaid` builds
//!   one (`own_paid`). At most `paid_holdings::PAID_ROWS_PER_READ` rows a
//!   read; the cursor is the last row's licence number.
//! - `CharterNames { charters }` (17g C2c): the public Roll name of up to
//!   `MAX_CHARTER_NAMES` charters, in one batched read; an unknown charter
//!   is left out. Names only.
//!
//! - `OwnPaid { licence }` (17g C2d): a paid licence the actor holds now
//!   (the personal paid-licence pointer names the actor), as `+minds/renew`,
//!   `+minds/resell` and the holdings' paid rows show it: the charter, its
//!   public name, whether it is a House Pin, its form, transferability and
//!   phase, whether its current term is revoked, whether a funded resale is
//!   selling it, and whether a renewal is offered or paid. A licence another
//!   character holds, a missing one and a row that cannot be read are
//!   `Refused`, identically. Point reads of the licence's own rows only.
//!
//! - `CharterOffers { filed }` (17g C2g, the C2e plan §4.1 item 4): the live
//!   pending-charter offers naming the actor on the filed mind's charter
//!   (the filed revision's charter is public), through 17g C2f's per-(buyer,
//!   charter) index (`roll_mind_charter_pending_buyer_offers`) keyed by the
//!   actor, then each offer row the actor's row names, whose named buyer and
//!   charter are checked: `{ offer, price_minor, expires_at_utc }` each, at
//!   most `MAX_BUYER_OFFERS`, an offer at or past its expiry left out. A
//!   filed number that does not exist is `Refused` and a charter nobody
//!   offered the actor the empty list, which the command prints alike, so
//!   the read is no oracle.
//!
//! - `ProcurementTally` and `ProcurementHoldings` (doc 17h §3 M8f): the
//!   one exception to "own only": GOD only, the procurement tick's reads,
//!   refused to every other caller (`procurement_read`); and, the same way,
//!   `HouseAttestationDue` (doc 17h §3 M8h), the House attestation tick's
//!   walk (`house_attestation_read`).
//!
//! The named buyer's resale step (immediate Accept or escrowed Fund) is not
//! a kind of this read: the C2d fix round removed a `ResaleStep` kind that
//! ran the paid-resale reader's `observe` for any licence (reviewers C2D-R2
//! and C2D-R3: an existence and open-claim oracle, not own-only as this
//! read's descriptor promises). 17g C2g rebuilt it on the offer's committed
//! evidence: the market's own-only `ResaleOffers` read asks Roll's
//! `paid_resale::reader::resale_step` only for a licence on which it found a
//! live offer naming the actor.

use super::Holder;
use super::families::CommerceFamilies;
use super::paid::PaidForm;
use super::read::CommerceRead;
use super::records::Record;
use super::{CharterId, FiledRevisionId, LicenceId, LineId, TermPhase, WorldIcSeconds};
use crate::RollStore;
use mush_core::read_service::{OwnerReadContext, OwnerReadError};
use mush_core::reserved_shell::ShellReservations;
use mush_core::{RequestUid, StateFamilyHandle};
use serde::Serialize;

/// The command kinds' declared row ceiling, inside `MAX_OWNER_READ_ROWS`
/// (declared in `original_owner_holding` v3's configuration).
pub(super) const MAXIMUM_ROWS: u16 = 64;
const DAY: i64 = 86_400;

/// One bounded request. There is deliberately no Deserialize: only the
/// registered TacSim command adapter builds one, from the player's selection,
/// and sends it inside `OriginalReadRequest::command`.
#[derive(Serialize)]
pub struct RollCommandReadRequest {
    kind: ReadKind,
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ReadKind {
    OpenShell,
    OwnFiled {
        filed: FiledRevisionId,
        /// M8c B3: the organisation the actor reads for (`org=<org>`): the
        /// filed mind is the organisation's House filing and the actor acts
        /// for it on the House's scope. Absent, the actor's own filed mind.
        #[serde(skip_serializing_if = "Option::is_none")]
        org: Option<mush_org::OrgId>,
    },
    OwnHoldings {
        section: HoldingSection,
        after: Option<u64>,
        /// M8c B3: the organisation whose holdings the actor reads
        /// (`+minds/holdings org=<org>`), under a live `roll.holder.org`
        /// grant; absent, the actor's own.
        #[serde(skip_serializing_if = "Option::is_none")]
        org: Option<mush_org::OrgId>,
    },
    CharterNames {
        charters: Vec<CharterId>,
    },
    OwnPaid {
        licence: LicenceId,
        /// M8c B3: the organisation holding the licence, read by a character
        /// acting for it; absent, a licence the actor holds itself.
        #[serde(skip_serializing_if = "Option::is_none")]
        org: Option<mush_org::OrgId>,
    },
    CharterOffers {
        filed: FiledRevisionId,
    },
    /// Doc 17h §3 M8f: GOD only (`procurement_read`): the score count of
    /// one rating's result pages, from page `from`.
    ProcurementTally {
        rating: super::rating::RatingRef,
        from: u16,
    },
    /// Doc 17h §3 M8f: GOD only (`procurement_read`): an organisation's
    /// current paid licences by charter, after `after`.
    ProcurementHoldings {
        org: mush_org::OrgId,
        after: Option<LicenceId>,
    },
    /// Doc 17h §3 M8h: GOD only (`house_attestation_read`): the House owner
    /// licences due for attestation, walking the owner licences after
    /// `after`.
    HouseAttestationDue {
        after: Option<LicenceId>,
    },
}
/// 17g C2c: the two halves of `+minds/holdings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldingSection {
    /// The filed minds the actor owns (their owner licences).
    Owner,
    /// The paid licences the actor holds (17g C2g: through the per-holder
    /// paid index, `roll_mind_paid_holdings`).
    Paid,
}
/// 17g C2c: owner rows one `OwnHoldings` read answers at most.
pub const OWNER_ROWS_PER_READ: usize = 3;
/// 17g C2c: own E4 assets one `OwnHoldings` read examines at most.
pub const OWN_ASSETS_PER_READ: usize = 16;
/// 17g C2c: charters one `CharterNames` read names at most.
pub const MAX_CHARTER_NAMES: usize = 20;
impl RollCommandReadRequest {
    /// The actor's own open filing reservation, if any.
    #[must_use]
    pub const fn open_shell() -> Self {
        Self {
            kind: ReadKind::OpenShell,
        }
    }
    /// The actor's own filed mind selected by one of its filed revisions.
    #[must_use]
    pub const fn own_filed(filed: FiledRevisionId) -> Self {
        Self::own_filed_for(filed, None)
    }
    /// M8c B3: `own_filed` for the organisation `org` (`ReadKind::OwnFiled`).
    #[must_use]
    pub const fn own_filed_for(filed: FiledRevisionId, org: Option<mush_org::OrgId>) -> Self {
        Self {
            kind: ReadKind::OwnFiled { filed, org },
        }
    }
    /// 17g C2c: one page of the actor's own holdings in `section`, after the
    /// cursor a previous page returned (an E4 asset id for `Owner`; 17g C2g:
    /// a licence number for `Paid`).
    #[must_use]
    pub const fn own_holdings(section: HoldingSection, after: Option<u64>) -> Self {
        Self::own_holdings_for(section, after, None)
    }
    /// M8c B3: `own_holdings` for the organisation `org`.
    #[must_use]
    pub const fn own_holdings_for(
        section: HoldingSection,
        after: Option<u64>,
        org: Option<mush_org::OrgId>,
    ) -> Self {
        Self {
            kind: ReadKind::OwnHoldings {
                section,
                after,
                org,
            },
        }
    }
    /// 17g C2c: the public names of up to `MAX_CHARTER_NAMES` charters.
    #[must_use]
    pub const fn charter_names(charters: Vec<CharterId>) -> Self {
        Self {
            kind: ReadKind::CharterNames { charters },
        }
    }
    /// 17g C2d: a paid licence the actor holds now.
    #[must_use]
    pub const fn own_paid(licence: LicenceId) -> Self {
        Self::own_paid_for(licence, None)
    }
    /// M8c B3: `own_paid` for a licence the organisation `org` holds.
    #[must_use]
    pub const fn own_paid_for(licence: LicenceId, org: Option<mush_org::OrgId>) -> Self {
        Self {
            kind: ReadKind::OwnPaid { licence, org },
        }
    }
    /// Doc 17h §3 M8f: GOD only; the procurement tick's rating count
    /// (`procurement_read::tally`).
    #[must_use]
    pub const fn procurement_tally(rating: super::rating::RatingRef, from: u16) -> Self {
        Self {
            kind: ReadKind::ProcurementTally { rating, from },
        }
    }
    /// Doc 17h §3 M8f: GOD only; the organisation's current licences
    /// (`procurement_read::holdings`).
    #[must_use]
    pub const fn procurement_holdings(org: mush_org::OrgId, after: Option<LicenceId>) -> Self {
        Self {
            kind: ReadKind::ProcurementHoldings { org, after },
        }
    }
    /// Doc 17h §3 M8h: GOD only; the House attestation tick's walk
    /// (`house_attestation_read::due`).
    #[must_use]
    pub const fn house_attestation_due(after: Option<LicenceId>) -> Self {
        Self {
            kind: ReadKind::HouseAttestationDue { after },
        }
    }
    /// 17g C2g: the live pending-charter offers naming the actor on the
    /// charter of filed revision `filed`.
    #[must_use]
    pub const fn charter_offers(filed: FiledRevisionId) -> Self {
        Self {
            kind: ReadKind::CharterOffers { filed },
        }
    }
}

/// 17g C2g: one live pending-charter offer naming the actor
/// (`CharterOffers`): its token (the Fund's `offer`), the frozen price in
/// minor units and its expiry (world UTC seconds; the provider refuses a
/// Fund at or after it). No seller, account, digest or other buyer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CharterOfferView {
    offer: mush_core::CommitToken,
    price_minor: i64,
    expires_at_utc: i64,
}
impl CharterOfferView {
    #[must_use]
    pub const fn offer(&self) -> mush_core::CommitToken {
        self.offer
    }
    #[must_use]
    pub const fn price_minor(&self) -> i64 {
        self.price_minor
    }
    #[must_use]
    pub const fn expires_at_utc(&self) -> i64 {
        self.expires_at_utc
    }
}

/// 17g C2d: a paid licence's renewal, from its personal or House renewal
/// head: a drafted offer awaits the holder (`Offered`), or the holder has
/// paid it into escrow and the service will complete it (`Paid`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RenewalMark {
    Offered,
    Paid,
}

/// 17g C2d: one paid licence the actor holds (`OwnPaid`), as the holdings'
/// paid rows and the trade commands show it. No source, digest, account,
/// rating, seller or other holder; `name` is the charter's public Roll name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PaidHoldingView {
    licence: LicenceId,
    charter: CharterId,
    house: bool,
    name: String,
    form: PaidForm,
    transferable: bool,
    phase: OwnerPhase,
    revoked: bool,
    selling: bool,
    renewal: Option<RenewalMark>,
    /// 17g C2h: never encoded (the view's serialized form carries no
    /// operation, `command_reader_trade_kinds_answer_only_their_own_party`);
    /// the owner read hands the view over as a value.
    #[serde(skip)]
    renewal_operation: Option<mush_core::CommitToken>,
}
impl PaidHoldingView {
    #[must_use]
    pub const fn licence(&self) -> LicenceId {
        self.licence
    }
    #[must_use]
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    /// A House Pin (bought from a House catalogue), renewed by the House
    /// renewal service; otherwise a personal paid licence.
    #[must_use]
    pub const fn house(&self) -> bool {
        self.house
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn form(&self) -> PaidForm {
        self.form
    }
    /// Whether the current term may be resold (the D-UM-40 mark and the
    /// deployment's policy, frozen in the term).
    #[must_use]
    pub const fn transferable(&self) -> bool {
        self.transferable
    }
    /// The current term's phase, in whole IC days rounded up.
    #[must_use]
    pub const fn phase(&self) -> OwnerPhase {
        self.phase
    }
    /// The current term was revoked by the Registry.
    #[must_use]
    pub const fn revoked(&self) -> bool {
        self.revoked
    }
    /// A funded resale holds the buyer's payment in escrow.
    #[must_use]
    pub const fn selling(&self) -> bool {
        self.selling
    }
    #[must_use]
    pub const fn renewal(&self) -> Option<RenewalMark> {
        self.renewal
    }
    /// 17g C2h (the C2e plan §5.1): the renewal head's pending operation
    /// (the drafted or funded renewal `renewal` marks), the token
    /// `+minds/renew/pay` and `/decline` bind the holder's step to, so a
    /// payment answers the offer the holder saw. Own-only, as the whole view
    /// is: a licence another character holds reads like a missing one.
    #[must_use]
    pub const fn renewal_operation(&self) -> Option<mush_core::CommitToken> {
        self.renewal_operation
    }
}

/// 17g C2c: one filed mind the actor owns, as `+minds/holdings` lists it:
/// `OwnFiled`'s answer for the line's newest filed revision, and whether the
/// charter is being sold (a funded pending transfer). No source, digest,
/// account, rating or other holder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnerHoldingView {
    filed: FiledRevisionId,
    charter: CharterId,
    owner_licence: LicenceId,
    dependent: bool,
    name: String,
    owner_phase: OwnerPhase,
    selling: bool,
}
impl OwnerHoldingView {
    /// The line's newest filed revision (`LineRecord::head`).
    #[must_use]
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    #[must_use]
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    #[must_use]
    pub const fn owner_licence(&self) -> LicenceId {
        self.owner_licence
    }
    #[must_use]
    pub const fn dependent(&self) -> bool {
        self.dependent
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn owner_phase(&self) -> OwnerPhase {
        self.owner_phase
    }
    /// A funded pending charter transfer holds the title's sale in escrow.
    /// An open, unfunded pending offer is not shown: the offers are keyed by
    /// their operation, so no bounded per-charter read finds one.
    #[must_use]
    pub const fn selling(&self) -> bool {
        self.selling
    }
}
/// 17g C2c: one `OwnHoldings { section: Owner }` page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnerHoldingsPage {
    rows: Vec<OwnerHoldingView>,
    next: Option<u64>,
}
impl OwnerHoldingsPage {
    #[must_use]
    pub fn rows(&self) -> &[OwnerHoldingView] {
        &self.rows
    }
    /// The asset cursor of the next page, `None` at the end.
    #[must_use]
    pub const fn next(&self) -> Option<u64> {
        self.next
    }
}
/// 17g C2c: one charter's public Roll name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CharterName {
    charter: CharterId,
    name: String,
}
impl CharterName {
    #[must_use]
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// The owner licence's current term, in whole IC days rounded up.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "phase", rename_all = "snake_case")]
pub enum OwnerPhase {
    Active { days_left: u64 },
    Grace { days_left: u64 },
    Lapsed,
}

/// What a command may know about a filed mind the actor owns now. No source,
/// digest, account, rating or other holder; `name` is the charter's public
/// Roll entry name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OwnFiledView {
    filed: FiledRevisionId,
    charter: CharterId,
    line: LineId,
    owner_licence: LicenceId,
    dependent: bool,
    name: String,
    owner_phase: OwnerPhase,
}
impl OwnFiledView {
    #[must_use]
    pub const fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    #[must_use]
    pub const fn charter(&self) -> CharterId {
        self.charter
    }
    #[must_use]
    pub const fn line(&self) -> LineId {
        self.line
    }
    #[must_use]
    pub const fn owner_licence(&self) -> LicenceId {
        self.owner_licence
    }
    /// Whether the line was born a dependent (derived) filing, so its update
    /// is `roll-update-derived-mind` rather than `roll-update-original-mind`.
    #[must_use]
    pub const fn dependent(&self) -> bool {
        self.dependent
    }
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
    #[must_use]
    pub const fn owner_phase(&self) -> OwnerPhase {
        self.owner_phase
    }
}

/// One result per request kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RollCommandReadResult {
    /// `request` is the reservation's request id, or `None` with no open shell.
    OpenShell {
        request: Option<RequestUid>,
    },
    OwnFiled(OwnFiledView),
    /// 17g C2c: `OwnHoldings { section: Owner }`.
    OwnerHoldings(OwnerHoldingsPage),
    /// 17g C2c: `CharterNames`, in the requested order, unknown ones left out.
    CharterNames {
        names: Vec<CharterName>,
    },
    /// 17g C2d: `OwnPaid`.
    OwnPaid(PaidHoldingView),
    /// 17g C2g: `OwnHoldings { section: Paid }`.
    PaidHoldings(super::paid_holdings::PaidHoldingsPage),
    /// 17g C2g: `CharterOffers`, in offer token order.
    CharterOffers {
        offers: Vec<CharterOfferView>,
    },
    /// Doc 17h §3 M8f: `ProcurementTally`.
    ProcurementTally(super::procurement_read::ProcurementTallyPage),
    /// Doc 17h §3 M8f: `ProcurementHoldings`.
    ProcurementHoldings(super::procurement_read::ProcurementHoldingsPage),
    /// Doc 17h §3 M8h: `HouseAttestationDue`.
    HouseAttestationDue(super::house_attestation_read::HouseAttestationDuePage),
}
impl RollCommandReadResult {
    /// Doc 17h §3 M8f: the `ProcurementTally` answer.
    #[must_use]
    pub fn into_procurement_tally(self) -> Option<super::procurement_read::ProcurementTallyPage> {
        match self {
            Self::ProcurementTally(page) => Some(page),
            _ => None,
        }
    }
    /// Doc 17h §3 M8f: the `ProcurementHoldings` answer.
    #[must_use]
    pub fn into_procurement_holdings(
        self,
    ) -> Option<super::procurement_read::ProcurementHoldingsPage> {
        match self {
            Self::ProcurementHoldings(page) => Some(page),
            _ => None,
        }
    }
    /// Doc 17h §3 M8h: the `HouseAttestationDue` answer.
    #[must_use]
    pub fn into_house_attestation_due(
        self,
    ) -> Option<super::house_attestation_read::HouseAttestationDuePage> {
        match self {
            Self::HouseAttestationDue(page) => Some(page),
            _ => None,
        }
    }
    /// The open-shell answer, or `None` if this answered another kind.
    #[must_use]
    pub const fn open_shell(&self) -> Option<Option<RequestUid>> {
        match self {
            Self::OpenShell { request } => Some(*request),
            _ => None,
        }
    }
    /// The filed-mind answer, or `None` if this answered another kind.
    #[must_use]
    pub fn into_own_filed(self) -> Option<OwnFiledView> {
        match self {
            Self::OwnFiled(view) => Some(view),
            _ => None,
        }
    }
    /// 17g C2c: the owner-holdings page, or `None` for another kind.
    #[must_use]
    pub fn into_owner_holdings(self) -> Option<OwnerHoldingsPage> {
        match self {
            Self::OwnerHoldings(page) => Some(page),
            _ => None,
        }
    }
    /// 17g C2c: the charter names, or `None` for another kind.
    #[must_use]
    pub fn into_charter_names(self) -> Option<Vec<CharterName>> {
        match self {
            Self::CharterNames { names } => Some(names),
            _ => None,
        }
    }
    /// 17g C2d: the paid licence, or `None` for another kind.
    #[must_use]
    pub fn into_own_paid(self) -> Option<PaidHoldingView> {
        match self {
            Self::OwnPaid(view) => Some(view),
            _ => None,
        }
    }
    /// 17g C2g: the paid-holdings page, or `None` for another kind.
    #[must_use]
    pub fn into_paid_holdings(self) -> Option<super::paid_holdings::PaidHoldingsPage> {
        match self {
            Self::PaidHoldings(page) => Some(page),
            _ => None,
        }
    }
    /// 17g C2g: the live charter offers naming the actor, or `None` for
    /// another kind.
    #[must_use]
    pub fn into_charter_offers(self) -> Option<Vec<CharterOfferView>> {
        match self {
            Self::CharterOffers { offers } => Some(offers),
            _ => None,
        }
    }
}

/// Answer one command kind for the calling actor. `shells` is the Roll
/// module's own reservation handle, present only when the player market is
/// composed (`OriginalOwnerReader`'s `commands`); without it every kind is
/// `Unavailable`. The caller has already checked `commerce_ready`.
pub(super) fn read(
    ctx: &OwnerReadContext<'_>,
    roll: &RollStore,
    (families, ratings): (&CommerceFamilies, &super::rating::RatingFamilies),
    shells: Option<&ShellReservations>,
    request: RollCommandReadRequest,
) -> Result<RollCommandReadResult, OwnerReadError> {
    // Doc 17h §3 M8f: the two procurement kinds read no filing shell, so they
    // answer wherever the original-holdings read is composed (GOD only).
    match request.kind {
        ReadKind::ProcurementTally { rating, from } => {
            return super::procurement_read::tally(ctx, ratings, &rating, from)
                .map(RollCommandReadResult::ProcurementTally);
        }
        ReadKind::ProcurementHoldings { org, after } => {
            return super::procurement_read::holdings(ctx, families, org, after)
                .map(RollCommandReadResult::ProcurementHoldings);
        }
        // Doc 17h §3 M8h: the House attestation tick's walk, as GOD's.
        ReadKind::HouseAttestationDue { after } => {
            return super::house_attestation_read::due(ctx, roll, families, after)
                .map(RollCommandReadResult::HouseAttestationDue);
        }
        _ => {}
    }
    let shells = shells.ok_or(OwnerReadError::Unavailable)?;
    // M8c B3: the holder a read is for: the actor itself, or the organisation
    // named by `org=` acting through the actor. Each own read re-asks the
    // grant for the organisation arm; nothing about the organisation is
    // read before it passes, so a foreign organisation reads as a missing
    // number.
    let holder_for = |org: Option<mush_org::OrgId>| match org {
        None => Holder::Character(ctx.actor()),
        Some(org) => Holder::Organization {
            org,
            acting: ctx.actor(),
        },
    };
    match request.kind {
        ReadKind::OpenShell => Ok(RollCommandReadResult::OpenShell {
            request: shells.open_request(ctx)?,
        }),
        ReadKind::OwnFiled { filed, org } => own_filed(ctx, roll, families, filed, holder_for(org))
            .map(RollCommandReadResult::OwnFiled)
            .map_err(|_| OwnerReadError::Refused),
        ReadKind::OwnHoldings {
            section: HoldingSection::Owner,
            after,
            org,
        } => owner_holdings(ctx, roll, families, after, holder_for(org))
            .map(RollCommandReadResult::OwnerHoldings),
        ReadKind::OwnHoldings {
            section: HoldingSection::Paid,
            after,
            org,
        } => {
            let after = after
                .map(LicenceId::new)
                .transpose()
                .map_err(|_| OwnerReadError::Refused)?;
            super::paid_holdings::page(ctx, families, holder_for(org), after)
                .map(RollCommandReadResult::PaidHoldings)
        }
        ReadKind::CharterNames { charters } => charter_names(ctx, families, &charters)
            .map(|names| RollCommandReadResult::CharterNames { names }),
        ReadKind::OwnPaid { licence, org } => own_paid(ctx, families, licence, holder_for(org))
            .map(RollCommandReadResult::OwnPaid)
            .map_err(|_| OwnerReadError::Refused),
        ReadKind::CharterOffers { filed } => charter_offers(ctx, families, filed)
            .map(|offers| RollCommandReadResult::CharterOffers { offers }),
        // Answered above.
        ReadKind::ProcurementTally { .. }
        | ReadKind::ProcurementHoldings { .. }
        | ReadKind::HouseAttestationDue { .. } => Err(OwnerReadError::Unavailable),
    }
}

/// 17g C2g: the live pending-charter offers naming the actor on the charter
/// of `filed` (the module doc). A missing filed number is `Refused`; the
/// actor's own `(buyer, charter)` index row is one point read and the offer
/// rows it names one batched read (`named_offers`).
fn charter_offers(
    ctx: &OwnerReadContext<'_>,
    families: &CommerceFamilies,
    filed: FiledRevisionId,
) -> Result<Vec<CharterOfferView>, OwnerReadError> {
    let actor = ctx.actor();
    let charter = row(ctx, &families.filed, filed.key())?
        .content()
        .position()
        .charter;
    let key = super::charter_pending::records::PendingBuyerOffers::key_for(actor, charter);
    let index = ctx.action_state(&families.charter_pending_buyer_offers, &key)?;
    named_offers(actor, charter, index, ctx.world().now(), |keys| {
        ctx.action_states(&families.charter_pending_offers, keys)
    })
}

/// 17g C2g: `CharterOffers`' answer from the actor's own index row (`index`,
/// `None` for no row) and the offer rows it names (`offers`, one batched
/// read by token key): each row must be that token's and name this buyer
/// and charter (else `Integrity`); an offer at or past its expiry at `now`
/// (world UTC seconds) is left out. Kept apart from the owner lens so the
/// tests prove it over real offer rows (a pending offer row cannot be
/// written outside the composed protocol).
pub(super) fn named_offers<'a>(
    actor: mush_core::ObjId,
    charter: CharterId,
    index: Option<&super::charter_pending::records::PendingBuyerOffers>,
    now: i64,
    offers: impl FnOnce(
        &[String],
    ) -> Result<
        Vec<Option<&'a super::charter_pending::records::PendingOffer>>,
        OwnerReadError,
    >,
) -> Result<Vec<CharterOfferView>, OwnerReadError> {
    use super::charter_pending::records::MAX_BUYER_OFFERS;
    let Some(index) = index else {
        return Ok(Vec::new());
    };
    index.validate().map_err(|_| OwnerReadError::Integrity)?;
    if index.buyer != actor || index.charter != charter || index.offers.len() > MAX_BUYER_OFFERS {
        return Err(OwnerReadError::Integrity);
    }
    let tokens: Vec<mush_core::CommitToken> = index.offers.iter().copied().collect();
    let keys: Vec<String> = tokens.iter().map(|token| token.to_hex()).collect();
    let mut live = Vec::new();
    for (token, offer) in tokens.iter().zip(offers(&keys)?) {
        let offer = offer.ok_or(OwnerReadError::Integrity)?;
        offer.validate().map_err(|_| OwnerReadError::Integrity)?;
        if offer.operation != *token || offer.buyer != actor || offer.charter != charter {
            return Err(OwnerReadError::Integrity);
        }
        if now < offer.expires_at_utc {
            live.push(CharterOfferView {
                offer: offer.operation,
                price_minor: offer.price.minor(),
                expires_at_utc: offer.expires_at_utc,
            });
        }
    }
    Ok(live)
}

/// 17g C2d: the paid licence `licence` when the actor holds it now (`read`
/// maps every failure to `Refused`, so another holder's licence reads like a
/// missing one). Each fact is one point read of the licence's own rows: the
/// pointer and its current term, the charter's name, the House allocation of
/// its birth operation, the term's revocation status, the resale head and
/// the renewal head with its pending operation.
pub(super) fn own_paid(
    ctx: &OwnerReadContext<'_>,
    f: &CommerceFamilies,
    licence: LicenceId,
    holder: Holder,
) -> Result<PaidHoldingView, OwnerReadError> {
    let pointer = row(ctx, &f.paid_licences, licence.key())?;
    if pointer.id() != licence {
        return Err(OwnerReadError::Integrity);
    }
    // M8c B3: a character reads a licence it holds; a character acting for
    // the holding organisation (`org_holder::acts_as`, re-asked here) reads
    // the organisation's, whoever acted when it was bought. The bare read
    // never answers an organisation's licence, nor the reverse.
    if !pointer.holder().same_principal(holder)
        // Doc 17h §3 M8f (P2a): a read, so a limited grant reads.
        || super::org_holder::reads_as(ctx.world(), pointer.holder(), ctx.actor()).is_err()
    {
        return Err(OwnerReadError::Refused);
    }
    let term = row(
        ctx,
        &f.paid_terms,
        format!("{}/{}", licence.key(), pointer.term().key()),
    )?;
    term.matches_current(pointer)
        .map_err(|_| OwnerReadError::Integrity)?;
    let charter = row(ctx, &f.charters, pointer.charter().key())?;
    if charter.id != pointer.charter() {
        return Err(OwnerReadError::Integrity);
    }
    let house = ctx
        .get(&f.house_allocations, &pointer.operation().to_hex())
        .map_err(|_| OwnerReadError::Unavailable)?
        .is_some();
    let revoked = ctx
        .get(
            &f.revoked_terms,
            &super::registry_revocation::records::RevokedTerm::key_for(licence, term.generation()),
        )
        .map_err(|_| OwnerReadError::Unavailable)?
        .is_some();
    let selling = ctx
        .get(&f.paid_resale_heads, &licence.key())
        .map_err(|_| OwnerReadError::Unavailable)?
        .is_some_and(|head| matches!(head.state(), super::paid_resale::PaidResaleState::Funded {}));
    let (renewal, renewal_operation) = if house {
        let pending = ctx
            .get(&f.house_renewal_heads, &licence.key())
            .map_err(|_| OwnerReadError::Unavailable)?
            .and_then(super::house_renewal::RenewalHeadV1::pending);
        match pending {
            None => (None, None),
            Some(operation) => {
                use super::house_renewal::RenewalStateV1 as State;
                let row = row(ctx, &f.house_renewal_operations, operation.to_hex())?;
                let mark = renewal_mark(
                    matches!(row.state(), State::Drafted {}),
                    matches!(row.state(), State::Funded { .. }),
                );
                (mark, Some(operation))
            }
        }
    } else {
        let pending = ctx
            .get(&f.renewal_heads, &licence.key())
            .map_err(|_| OwnerReadError::Unavailable)?
            .and_then(super::renewal::RenewalHeadV1::pending);
        match pending {
            None => (None, None),
            Some(operation) => {
                use super::renewal::RenewalStateV1 as State;
                let row = row(ctx, &f.renewal_operations, operation.to_hex())?;
                let mark = renewal_mark(
                    matches!(row.state(), State::Drafted {}),
                    matches!(row.state(), State::Funded { .. }),
                );
                (mark, Some(operation))
            }
        }
    };
    Ok(PaidHoldingView {
        licence,
        charter: charter.id,
        house,
        name: charter.name.clone(),
        form: term.terms().form(),
        transferable: term.terms().transferable(),
        phase: phase(term.window(), WorldIcSeconds(ctx.world().game_now_unix())),
        revoked,
        selling,
        renewal,
        renewal_operation,
    })
}

/// 17g C2d: a pending renewal operation's mark: drafted is `Offered`, funded
/// is `Paid`; any other phase is no mark.
pub(super) const fn renewal_mark(drafted: bool, funded: bool) -> Option<RenewalMark> {
    if drafted {
        Some(RenewalMark::Offered)
    } else if funded {
        Some(RenewalMark::Paid)
    } else {
        None
    }
}

/// 17g C2c: one page of the actor's own filed minds. The assets are the
/// actor's own current E4 titles after `after` (`assets_of_after`, the
/// original-holdings page's seek); a title that is not a Roll charter is
/// skipped, and so is a charter `own_filed` refuses for this actor.
fn owner_holdings(
    ctx: &OwnerReadContext<'_>,
    roll: &RollStore,
    families: &CommerceFamilies,
    after: Option<u64>,
    holder: Holder,
) -> Result<OwnerHoldingsPage, OwnerReadError> {
    // M8c B3/F5: an organisation's titles are read by a character granted on
    // any of its scopes (a House's curator on the House's scope), asked
    // before any title is looked at; each row is then projected on its own
    // title's scope (`project_owner_for`, `check_title`).
    if match holder {
        super::Holder::Character(c) => c != ctx.actor(),
        super::Holder::Organization { org, .. } => {
            super::org_holder::acts_anywhere(ctx.world(), org, ctx.actor()).is_err()
        }
    } {
        return Err(OwnerReadError::Refused);
    }
    let after = after
        .map(|n| u32::try_from(n).map(mush_core::ObjId))
        .transpose()
        .map_err(|_| OwnerReadError::Refused)?;
    let economy = ctx
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .ok_or(OwnerReadError::Unavailable)?;
    if economy.is_frozen() {
        return Err(OwnerReadError::Unavailable);
    }
    let owned: Vec<mush_core::ObjId> = economy
        .snapshot()
        .assets_of_after(&holder.principal(), after, OWN_ASSETS_PER_READ + 1)
        .map(|asset| asset.object)
        .collect();
    let (rows, next) = page(&owned, |object| {
        let charter_id = CharterId::from_object(object).map_err(|_| OwnerReadError::Integrity)?;
        let Some(charter) = ctx.action_state(&families.charters, &charter_id.key())? else {
            return Ok(None);
        };
        charter.validate().map_err(|_| OwnerReadError::Integrity)?;
        if charter.id != charter_id {
            return Err(OwnerReadError::Integrity);
        }
        let line = row(ctx, &families.lines, charter.line.key())?;
        if line.id() != charter.line || line.charter() != charter.id {
            return Err(OwnerReadError::Integrity);
        }
        let view = match own_filed(ctx, roll, families, line.head(), holder) {
            Ok(view) => view,
            Err(OwnerReadError::Refused) => return Ok(None),
            Err(error) => return Err(error),
        };
        if view.charter != charter.id {
            return Err(OwnerReadError::Integrity);
        }
        Ok(Some(OwnerHoldingView {
            selling: pending_sale(ctx, families, charter.id)?,
            filed: view.filed,
            charter: view.charter,
            owner_licence: view.owner_licence,
            dependent: view.dependent,
            name: view.name,
            owner_phase: view.owner_phase,
        }))
    })?;
    // M8c F14: an organisation's titles may be scoped where the reader is
    // not granted (`check_title` skips them), so the cursor names the last
    // VISIBLE row's title when one was shown. With none shown, the last
    // examined asset stays the cursor: it is the organisation's own title,
    // and without it a window of unseen titles could never be paged past.
    let next = match (holder, rows.last(), next) {
        (super::Holder::Organization { .. }, Some(last), Some(_)) => {
            Some(u64::from(last.charter.object().0))
        }
        (_, _, next) => next,
    };
    Ok(OwnerHoldingsPage { rows, next })
}

/// One `OwnHoldings` read's paging over `owned` (the actor's own assets in
/// id order, `OWN_ASSETS_PER_READ + 1` of them at most, the extra one only
/// telling whether more follow): `row_of` each of the first
/// `OWN_ASSETS_PER_READ` until `OWNER_ROWS_PER_READ` rows are found, and the
/// cursor to continue after, `None` at the end of the actor's assets.
/// - Cut at `OWNER_ROWS_PER_READ`: the cursor is the last row's asset, when
///   any asset follows it.
/// - The examined assets ran out first: the cursor is the last examined
///   asset when one more exists, so a read whose assets held no filed mind
///   still goes on (its rows are then empty).
fn page<T>(
    owned: &[mush_core::ObjId],
    mut row_of: impl FnMut(mush_core::ObjId) -> Result<Option<T>, OwnerReadError>,
) -> Result<(Vec<T>, Option<u64>), OwnerReadError> {
    let mut rows = Vec::new();
    for (offset, object) in owned.iter().take(OWN_ASSETS_PER_READ).enumerate() {
        let Some(found) = row_of(*object)? else {
            continue;
        };
        rows.push(found);
        if rows.len() == OWNER_ROWS_PER_READ {
            let more = offset + 1 < owned.len();
            return Ok((rows, more.then(|| u64::from(object.0))));
        }
    }
    let next = if owned.len() > OWN_ASSETS_PER_READ {
        owned
            .get(OWN_ASSETS_PER_READ - 1)
            .map(|object| u64::from(object.0))
    } else {
        None
    };
    Ok((rows, next))
}

/// Whether the charter's pending-transfer head is `Funded`: the buyer has
/// paid into escrow and the transfer has not settled or refunded.
fn pending_sale(
    ctx: &OwnerReadContext<'_>,
    families: &CommerceFamilies,
    charter: CharterId,
) -> Result<bool, OwnerReadError> {
    funded(
        ctx.action_state(&families.charter_pending_heads, &charter.key())?,
        charter,
    )
}

/// `pending_sale`'s answer from the charter's pending-transfer head: none,
/// or a settled or refunded one, is no sale; a head of another charter, or
/// one that does not validate, is `Integrity`.
fn funded(
    head: Option<&super::charter_pending::records::PendingHead>,
    charter: CharterId,
) -> Result<bool, OwnerReadError> {
    let Some(head) = head else {
        return Ok(false);
    };
    head.validate().map_err(|_| OwnerReadError::Integrity)?;
    if head.charter != charter {
        return Err(OwnerReadError::Integrity);
    }
    Ok(matches!(
        head.state,
        super::charter_pending::records::PendingState::Funded {}
    ))
}

/// The public names of `charters` (at most `MAX_CHARTER_NAMES`), one batched
/// read; a charter Roll does not hold is left out.
fn charter_names(
    ctx: &OwnerReadContext<'_>,
    families: &CommerceFamilies,
    charters: &[CharterId],
) -> Result<Vec<CharterName>, OwnerReadError> {
    if charters.is_empty() {
        return Ok(Vec::new());
    }
    if charters.len() > MAX_CHARTER_NAMES {
        return Err(OwnerReadError::Limit);
    }
    let keys: Vec<String> = charters.iter().map(|c| c.key()).collect();
    let found = ctx.action_states(&families.charters, &keys)?;
    let mut names = Vec::new();
    for (id, record) in charters.iter().zip(found) {
        let Some(record) = record else {
            continue;
        };
        record.validate().map_err(|_| OwnerReadError::Integrity)?;
        if record.id != *id {
            return Err(OwnerReadError::Integrity);
        }
        names.push(CharterName {
            charter: *id,
            name: record.name.clone(),
        });
    }
    Ok(names)
}

/// Any failure here is a refusal to the caller (`read` maps every error to
/// `Refused`), so "not yours" and "does not exist" cannot be told apart.
fn own_filed(
    ctx: &OwnerReadContext<'_>,
    roll: &RollStore,
    families: &CommerceFamilies,
    filed: FiledRevisionId,
    holder: Holder,
) -> Result<OwnFiledView, OwnerReadError> {
    let record = row(ctx, &families.filed, filed.key())?;
    let position = record.content().position();
    let charter = row(ctx, &families.charters, position.charter.key())?;
    if charter.id != position.charter || charter.line != position.line {
        return Err(OwnerReadError::Integrity);
    }
    let birth = row(ctx, &families.operations, charter.operation.to_string())?;
    if birth.charter != charter.id
        || birth.line != charter.line
        || birth.commit != charter.operation
    {
        return Err(OwnerReadError::Integrity);
    }
    let licence = row(ctx, &families.licences, birth.licence.key())?;
    if licence.charter != charter.id || licence.line != charter.line {
        return Err(OwnerReadError::Integrity);
    }
    // The title check the own-holding read uses: the charter's current
    // ownership history names this holder and E4 title is theirs now
    // (`charter_transfer::history::current_for`, `current_title_for`). After
    // a charter transfer the new holder passes and the old one does not; the
    // owner licence is the charter's, so it follows the title. M8c B3: for
    // an organisation holder the birth must be the organisation's House
    // filing and the actor must act for it on the House's scope.
    super::charter_transfer::history::current_for(ctx, families, birth, holder)
        .map_err(|_| OwnerReadError::Refused)?;
    let term = super::owner_attestation::history::current(ctx, roll, families, licence)
        .map_err(|_| OwnerReadError::Integrity)?;
    if term.licence != licence.id {
        return Err(OwnerReadError::Integrity);
    }
    Ok(OwnFiledView {
        filed,
        charter: charter.id,
        line: charter.line,
        owner_licence: licence.id,
        dependent: birth.parent.is_some(),
        name: charter.name.clone(),
        owner_phase: phase(term.window, WorldIcSeconds(ctx.world().game_now_unix())),
    })
}

/// The owner term's phase now, with whole IC days left rounded up. A term
/// not yet effective reads as Active with its days to the term's end.
fn phase(window: super::TermWindow, now: WorldIcSeconds) -> OwnerPhase {
    let days = |end: WorldIcSeconds| -> u64 {
        let left = end.get().saturating_sub(now.get()).max(0);
        u64::try_from(left.saturating_add(DAY - 1) / DAY).unwrap_or(0)
    };
    match window.phase_at(now) {
        TermPhase::NotEffective | TermPhase::Active => OwnerPhase::Active {
            days_left: days(window.term_end()),
        },
        TermPhase::Grace => OwnerPhase::Grace {
            days_left: days(window.grace_end()),
        },
        TermPhase::Lapsed => OwnerPhase::Lapsed,
    }
}

fn row<'a, R: Record>(
    ctx: &'a OwnerReadContext<'_>,
    handle: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, OwnerReadError> {
    let row = ctx
        .get(handle, &key)
        .map_err(|_| OwnerReadError::Unavailable)?
        .ok_or(OwnerReadError::Refused)?;
    row.validate().map_err(|_| OwnerReadError::Integrity)?;
    if row.key() != key {
        return Err(OwnerReadError::Integrity);
    }
    Ok(row)
}

#[cfg(test)]
mod tests;
