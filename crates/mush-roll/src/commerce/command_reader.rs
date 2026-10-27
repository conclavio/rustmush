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
//! C2c and C2d add kinds here (`OwnHoldings`, `CharterNames`, `OwnPaid`,
//! `ResaleStep`, `CharterOffers`), on the same folded read.

use super::families::CommerceFamilies;
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
    OwnFiled { filed: FiledRevisionId },
}
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
}
impl RollCommandReadResult {
    /// The open-shell answer, or `None` if this answered another kind.
    #[must_use]
    pub const fn open_shell(&self) -> Option<Option<RequestUid>> {
        match self {
            Self::OpenShell { request } => Some(*request),
            Self::OwnFiled(_) => None,
        }
    }
    /// The filed-mind answer, or `None` if this answered another kind.
    #[must_use]
    pub fn into_own_filed(self) -> Option<OwnFiledView> {
        match self {
            Self::OwnFiled(view) => Some(view),
            Self::OpenShell { .. } => None,
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
    }
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
