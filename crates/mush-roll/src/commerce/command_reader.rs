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
//! `holding_reader::OriginalOwnerReader` (`original_owner_holding` v2), which
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
//!   `section: Paid` is `Unavailable` until the per-holder paid index lands
//!   (`roll_mind_paid_holdings`, C2 plan §2.5 item 2, held at C2b).
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
//! **Not built (the C2d builder's report):** `CharterOffers { filed }`, the
//! live pending-charter offers naming the actor, reads the per-(buyer,
//! charter) offer index (`roll_mind_charter_pending_buyer_offers`, C2 plan
//! §2.5 item 4), which C2b held for the lead's mush-core ruling; the offers
//! are otherwise keyed only by their own operation, so there is no bounded
//! lookup. It waits for the index sub-cut.
//!
//! **Held (the C2d fix round, reviewers C2D-R2, C2D-R3, F2 and T3):**
//! `ResaleStep { licence }`, the named buyer's choice between the resale's
//! immediate Accept and its escrowed Fund (the paid-resale reader's own
//! `observe` with the actor as the buyer and the pointer's holder as the
//! seller, Accept exactly when `claims().open_paid() == 0`). `observe` checks
//! no offer naming the buyer, so for any third party it would answer
//! whether a licence exists, is resellable and has a sortie out under it,
//! where a missing licence is `Refused`: it is not own-only, as this read's
//! descriptor promises (`commands.own_only`). Its only caller, the live
//! `+minds/resell/accept`, waits for the per-(buyer, licence) offer index,
//! and no fixture here can reach a granted, transferable licence for a
//! positive test, so it waits with them: the index sub-cut builds it with
//! the offer's committed (buyer, licence) binding as its evidence.

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
/// (declared in `original_owner_holding` v2's configuration).
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
    },
    OwnHoldings {
        section: HoldingSection,
        after: Option<u64>,
    },
    CharterNames {
        charters: Vec<CharterId>,
    },
    OwnPaid {
        licence: LicenceId,
    },
}
/// 17g C2c: the two halves of `+minds/holdings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldingSection {
    /// The filed minds the actor owns (their owner licences).
    Owner,
    /// The paid licences the actor holds: `Unavailable` until the
    /// per-holder paid index (`roll_mind_paid_holdings`) lands.
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
        Self {
            kind: ReadKind::OwnFiled { filed },
        }
    }
    /// 17g C2c: one page of the actor's own holdings in `section`, after the
    /// cursor a previous page returned (an E4 asset id for `Owner`).
    #[must_use]
    pub const fn own_holdings(section: HoldingSection, after: Option<u64>) -> Self {
        Self {
            kind: ReadKind::OwnHoldings { section, after },
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
        Self {
            kind: ReadKind::OwnPaid { licence },
        }
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
}
impl RollCommandReadResult {
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
}

/// Answer one command kind for the calling actor. `shells` is the Roll
/// module's own reservation handle, present only when the player market is
/// composed (`OriginalOwnerReader`'s `commands`); without it every kind is
/// `Unavailable`. The caller has already checked `commerce_ready`.
pub(super) fn read(
    ctx: &OwnerReadContext<'_>,
    roll: &RollStore,
    families: &CommerceFamilies,
    shells: Option<&ShellReservations>,
    request: RollCommandReadRequest,
) -> Result<RollCommandReadResult, OwnerReadError> {
    let shells = shells.ok_or(OwnerReadError::Unavailable)?;
    match request.kind {
        ReadKind::OpenShell => Ok(RollCommandReadResult::OpenShell {
            request: shells.open_request(ctx)?,
        }),
        ReadKind::OwnFiled { filed } => own_filed(ctx, roll, families, filed)
            .map(RollCommandReadResult::OwnFiled)
            .map_err(|_| OwnerReadError::Refused),
        ReadKind::OwnHoldings {
            section: HoldingSection::Owner,
            after,
        } => owner_holdings(ctx, roll, families, after).map(RollCommandReadResult::OwnerHoldings),
        ReadKind::OwnHoldings {
            section: HoldingSection::Paid,
            ..
        } => Err(OwnerReadError::Unavailable),
        ReadKind::CharterNames { charters } => charter_names(ctx, families, &charters)
            .map(|names| RollCommandReadResult::CharterNames { names }),
        ReadKind::OwnPaid { licence } => own_paid(ctx, families, licence)
            .map(RollCommandReadResult::OwnPaid)
            .map_err(|_| OwnerReadError::Refused),
    }
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
) -> Result<PaidHoldingView, OwnerReadError> {
    let pointer = row(ctx, &f.paid_licences, licence.key())?;
    if pointer.id() != licence {
        return Err(OwnerReadError::Integrity);
    }
    if pointer.holder() != ctx.actor() {
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
    let renewal = if house {
        let pending = ctx
            .get(&f.house_renewal_heads, &licence.key())
            .map_err(|_| OwnerReadError::Unavailable)?
            .and_then(super::house_renewal::RenewalHeadV1::pending);
        match pending {
            None => None,
            Some(operation) => {
                use super::house_renewal::RenewalStateV1 as State;
                let row = row(ctx, &f.house_renewal_operations, operation.to_hex())?;
                renewal_mark(
                    matches!(row.state(), State::Drafted {}),
                    matches!(row.state(), State::Funded { .. }),
                )
            }
        }
    } else {
        let pending = ctx
            .get(&f.renewal_heads, &licence.key())
            .map_err(|_| OwnerReadError::Unavailable)?
            .and_then(super::renewal::RenewalHeadV1::pending);
        match pending {
            None => None,
            Some(operation) => {
                use super::renewal::RenewalStateV1 as State;
                let row = row(ctx, &f.renewal_operations, operation.to_hex())?;
                renewal_mark(
                    matches!(row.state(), State::Drafted {}),
                    matches!(row.state(), State::Funded { .. }),
                )
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
) -> Result<OwnerHoldingsPage, OwnerReadError> {
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
        .assets_of_after(
            &mush_org::PrincipalId::Object(ctx.actor()),
            after,
            OWN_ASSETS_PER_READ + 1,
        )
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
        let view = match own_filed(ctx, roll, families, line.head()) {
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
) -> Result<OwnFiledView, OwnerReadError> {
    let actor = ctx.actor();
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
    // ownership history names this actor and E4 title is theirs now
    // (`charter_transfer::history::current`, `current_title`). After a
    // charter transfer the new holder passes and the old one does not; the
    // owner licence is the charter's, so it follows the title.
    super::charter_transfer::history::current(ctx, families, birth, actor)
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
