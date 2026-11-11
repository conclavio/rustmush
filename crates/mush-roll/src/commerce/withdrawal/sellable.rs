//! Doc 44 MW.2 (Q-44-15; §4c row 8): whether a listing the market froze can
//! still be sold, answered from the Roll's own rows.
//!
//! The market's counter (`+minds`) and the House procurement list used to show
//! every listing whose price and cuts were still the deployment's, and only the
//! quote asked the Roll anything (`entitlement::reader`, `Listed`). A withdrawn
//! or recalled revision's listing therefore stayed on the counter for ever and
//! a House agent quoted it at every tick. This read answers, for a page of
//! listings at once, the four ways a listing dies that the Roll alone knows:
//!
//! - **withdrawn** or **recalled**: the listed revision's status row;
//! - **lapsed with its seller's licence**: the seller's owner licence is past
//!   Grace, has moved to another term (the listing names the term it was made
//!   under, and a quote refuses any other), is revoked by the Registry, or its
//!   holder is barred by an inspection for now;
//! - **lapsed with its parent's licence** (a derived listing): the seller's
//!   qualifying parent licence is not theirs any more, is past Grace, is not
//!   an open-resale term, is revoked, or the parent source is recalled;
//! - **moved with its charter**: the charter's title is no longer the seller's
//!   at the title revision the listing was made under, or the seller is gone.
//!
//! It is deliberately slimmer than the quote's read: it proves no ancestry,
//! root, rating or royalty, so a `Yes` here is not a promise that a quote
//! lands (a quote still runs the whole of the sale reader), and a row it
//! cannot read is `Unproved`, which the market treats as unsellable. What it
//! costs is fixed and small (`SELLABLE_ROWS_ORIGINAL`, `SELLABLE_ROWS_DERIVED`
//! rows a listing, in at most `SELLABLE_READS` batched reads a call), because
//! it runs inside the market's own bounded read, on that read's budget.
//!
//! **Whose read.** The Roll owns it: the rows are the Roll's, read through the
//! market's lens under the read-only grants `EntitlementPurchaseWriter::install`
//! declares, and the market holds no handle to them. The market reaches it only
//! as `EntitlementPurchaseWriter::sellable`.
use super::super::dependent_owner::OwnerParentBinding;
use super::super::families::CommerceFamilies;
use super::super::filed::ParentContent;
use super::super::owner_qualification::records::QualificationHead;
use super::super::paid::PaidForm;
use super::super::read::CommerceRead;
use super::super::registry_inspection::principal::principal_key;
use super::super::registry_revocation::records::RevokedTerm;
use super::super::{
    CharterId, FiledRevisionId, Holder, LicenceId, TermGeneration, TermPhase, TermWindow,
    WorldIcSeconds,
};
use crate::RollStore;
use mush_core::read_service::OwnerReadError;
use mush_core::{Flags, ObjId, ObjKind, StateFamilyHandle, World};
use mush_economy::{AssetLifecycle, EconomicAsset, EconomyStore};
use serde::Serialize;
use std::any::TypeId;

/// Rows one listing of an original mind costs at most: the revision's status,
/// the owner licence, its revoked-term mark and the seller's bar.
pub const SELLABLE_ROWS_ORIGINAL: usize = 4;
/// Rows one derived listing costs at most: the original's four, then the
/// birth operation, the parent binding, the qualification head and its
/// operation, the parent licence, its term, its revoked-term mark and the
/// parent source's status.
pub const SELLABLE_ROWS_DERIVED: usize = 12;
/// Listings one call answers at most (each batched read stays inside core's
/// page of 64 keys).
pub const SELLABLE_SUBJECTS: usize = 32;
/// Batched reads one call makes at most, whatever the number of listings.
pub const SELLABLE_READS: usize = 12;

/// What the market froze in a listing row when it was listed, as the Roll's
/// sale facts gave it: the listing's own evidence, never a caller's claim
/// about anyone's rights (every field is checked against the Roll's rows).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListedSubject {
    pub seller: ObjId,
    pub charter: CharterId,
    pub title_revision: u64,
    pub filed: FiledRevisionId,
    pub owner_licence: LicenceId,
    pub owner_term: TermGeneration,
    pub owner_window: TermWindow,
    pub parent: Option<ParentContent>,
}
impl ListedSubject {
    /// The most rows this listing's answer reads.
    #[must_use]
    pub const fn rows(&self) -> usize {
        if self.parent.is_some() {
            SELLABLE_ROWS_DERIVED
        } else {
            SELLABLE_ROWS_ORIGINAL
        }
    }
}

/// Why a listing cannot be sold now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Unsellable {
    /// The author withdrew the listed revision.
    Withdrawn,
    /// The Registry recalled the listed revision.
    Recalled,
    /// The seller's owner licence lapsed, moved to another term, was revoked,
    /// or its holder is barred for now.
    SellerLicence,
    /// A derived listing's qualifying parent licence lapsed, left the seller,
    /// was revoked or is no open-resale term, or the parent source is recalled.
    ParentLicence,
    /// The charter's title is not the seller's at the listed title revision,
    /// or the seller is no live character.
    CharterMoved,
    /// A row the answer needs is missing or does not match the listing.
    Unproved,
}

/// One listing's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sellable {
    Yes,
    No(Unsellable),
}
impl Sellable {
    #[must_use]
    pub const fn is_sellable(self) -> bool {
        matches!(self, Self::Yes)
    }
}

/// The title half, pure: the charter's current title is the seller's own, at
/// the revision the listing froze (the same test `EntitlementSaleFacts::
/// check_fresh` makes at a listing's commit).
pub(crate) fn title_verdict(
    s: &ListedSubject,
    title: Option<&EconomicAsset>,
) -> Option<Unsellable> {
    let moved = title.is_none_or(|t| {
        t.object != s.charter.object()
            || t.revision != s.title_revision
            || t.title_owner != mush_org::PrincipalId::Object(s.seller)
            || t.scope.is_some()
            || t.frozen
            || t.stolen.is_some()
            || t.lifecycle != AssetLifecycle::Active
    });
    moved.then_some(Unsellable::CharterMoved)
}

/// The seller's own rows as read: the listed revision's status (`filed`,
/// withdrawn, recalled), the owner licence (`charter`, current term), whether
/// the listed term is revoked and whether the seller is barred now.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct OwnerRows {
    pub(crate) status: Option<(FiledRevisionId, bool, bool)>,
    pub(crate) licence: Option<(CharterId, TermGeneration)>,
    pub(crate) revoked: bool,
    pub(crate) barred: bool,
}

/// The seller's half, pure. Recall is told before withdrawal (a recalled
/// revision is the Registry's act, whatever its author did first).
pub(crate) fn owner_verdict(
    s: &ListedSubject,
    now: WorldIcSeconds,
    rows: OwnerRows,
) -> Option<Unsellable> {
    let Some((filed, withdrawn, recalled)) = rows.status else {
        return Some(Unsellable::Unproved);
    };
    if filed != s.filed {
        return Some(Unsellable::Unproved);
    }
    if recalled {
        return Some(Unsellable::Recalled);
    }
    if withdrawn {
        return Some(Unsellable::Withdrawn);
    }
    let Some((charter, term)) = rows.licence else {
        return Some(Unsellable::Unproved);
    };
    if charter != s.charter {
        return Some(Unsellable::Unproved);
    }
    if term != s.owner_term
        || rows.revoked
        || rows.barred
        || !matches!(
            s.owner_window.phase_at(now),
            TermPhase::Active | TermPhase::Grace
        )
    {
        return Some(Unsellable::SellerLicence);
    }
    None
}

/// Which parent licence qualifies a derived listing's seller now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ParentChoice {
    /// The licence the child was filed under (the original owner, who never
    /// selected another).
    Bound(LicenceId),
    /// A committed replacement, named by its qualification operation.
    Replacement(mush_core::CommitToken),
}

/// The choice, pure: `requester` is who filed the child (its birth
/// operation), `binding` the parent source and licence the child was filed
/// under, `head` the seller's replacement, when one is committed. A seller
/// who is not the original owner qualifies only through a replacement
/// (`owner_qualification::current::resolve`).
pub(crate) fn parent_choice(
    s: &ListedSubject,
    requester: Option<ObjId>,
    binding: Option<(ParentContent, LicenceId)>,
    head: Option<mush_core::CommitToken>,
) -> Result<ParentChoice, Unsellable> {
    let (Some(requester), Some((source, licence))) = (requester, binding) else {
        return Err(Unsellable::Unproved);
    };
    if s.parent != Some(source) {
        return Err(Unsellable::Unproved);
    }
    match head {
        Some(operation) => Ok(ParentChoice::Replacement(operation)),
        None if requester == s.seller => Ok(ParentChoice::Bound(licence)),
        None => Err(Unsellable::ParentLicence),
    }
}

/// The qualifying parent's rows as read: who holds the parent licence, its
/// current term's window and form, whether that term is revoked, and the
/// parent source's status (`filed`, recalled).
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ParentRows {
    pub(crate) holder: Option<Holder>,
    pub(crate) term: Option<(TermWindow, PaidForm)>,
    pub(crate) revoked: bool,
    pub(crate) source: Option<(FiledRevisionId, bool)>,
}

/// The parent's half, pure. A withdrawn parent source does not stop a sale
/// (an acquired pinned source survives withdrawal); a recalled one does.
pub(crate) fn parent_verdict(
    s: &ListedSubject,
    now: WorldIcSeconds,
    rows: ParentRows,
) -> Option<Unsellable> {
    let (Some(parent), Some((filed, recalled))) = (s.parent, rows.source) else {
        return Some(Unsellable::Unproved);
    };
    if filed != parent.filed {
        return Some(Unsellable::Unproved);
    }
    let Some(holder) = rows.holder else {
        return Some(Unsellable::Unproved);
    };
    let Some((window, form)) = rows.term else {
        return Some(Unsellable::Unproved);
    };
    if recalled
        || holder != Holder::Character(s.seller)
        || rows.revoked
        || form != (PaidForm::OpenResale {})
        || !matches!(window.phase_at(now), TermPhase::Active | TermPhase::Grace)
    {
        return Some(Unsellable::ParentLicence);
    }
    None
}

/// One batched read of `handle` for the keys that are `Some`, answered in the
/// keys' own positions; a `None` key reads nothing. No key at all makes no
/// read (core refuses an empty batch).
fn fetch<'c, V>(
    ctx: &'c impl CommerceRead,
    handle: &StateFamilyHandle<String, V>,
    keys: &[Option<String>],
) -> Result<Vec<Option<&'c V>>, OwnerReadError>
where
    V: Clone + Send + Sync + 'static,
{
    let wanted: Vec<String> = keys.iter().flatten().cloned().collect();
    if wanted.is_empty() {
        return Ok(keys.iter().map(|_| None).collect());
    }
    let mut found = ctx
        .get_many(handle, &wanted)
        .map_err(|_| OwnerReadError::Unavailable)?
        .into_iter();
    Ok(keys
        .iter()
        .map(|key| key.as_ref().and_then(|_| found.next().flatten()))
        .collect())
}

/// The world half: the seller is a live character and the charter's title is
/// still theirs. No row is read.
fn world_verdict(world: &World, economy: &EconomyStore, s: &ListedSubject) -> Option<Unsellable> {
    let gone = world
        .get(s.seller)
        .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        || world
            .get(s.charter.object())
            .is_none_or(|o| o.flags.contains(Flags::GOING));
    if gone {
        return Some(Unsellable::CharterMoved);
    }
    title_verdict(s, economy.snapshot().asset(s.charter.object()))
}

/// The answer for each of `subjects`, in order (the module note).
///
/// # Errors
/// `Limit` for more than [`SELLABLE_SUBJECTS`] listings; `Unavailable` when
/// the Roll's commerce or the economy is not ready, or a family cannot be
/// read at all (a missing row is `Unproved` for its listing, never an error).
pub(crate) fn check(
    ctx: &impl CommerceRead,
    f: &CommerceFamilies,
    subjects: &[ListedSubject],
) -> Result<Vec<Sellable>, OwnerReadError> {
    if subjects.len() > SELLABLE_SUBJECTS {
        return Err(OwnerReadError::Limit);
    }
    let world = ctx.world();
    let roll = world
        .module_state::<RollStore>()
        .ok_or(OwnerReadError::Unavailable)?;
    roll.commerce_ready()
        .map_err(|_| OwnerReadError::Unavailable)?;
    let economy = world
        .module_state::<EconomyStore>()
        .ok_or(OwnerReadError::Unavailable)?;
    if economy.is_frozen()
        || world
            .module_state_owner(TypeId::of::<EconomyStore>())
            .is_none_or(|owner| owner.as_str() != "economy")
    {
        return Err(OwnerReadError::Unavailable);
    }
    let now = WorldIcSeconds(world.game_now_unix());
    let bar_now = world.world_ic_now().at;
    let mut verdicts: Vec<Option<Unsellable>> = subjects
        .iter()
        .map(|s| world_verdict(world, economy, s))
        .collect();
    // A key for every listing still undecided (and, with `derived`, derived).
    let keys = |verdicts: &[Option<Unsellable>],
                derived: bool,
                key: &dyn Fn(usize, &ListedSubject) -> Option<String>| {
        subjects
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (verdicts.get(i).is_some_and(Option::is_none) && (!derived || s.parent.is_some()))
                    .then(|| key(i, s))
                    .flatten()
            })
            .collect::<Vec<_>>()
    };

    // The seller's own rows.
    let status = fetch(
        ctx,
        &f.status,
        &keys(&verdicts, false, &|_, s| Some(s.filed.key())),
    )?;
    let licences = fetch(
        ctx,
        &f.licences,
        &keys(&verdicts, false, &|_, s| Some(s.owner_licence.key())),
    )?;
    let revoked = fetch(
        ctx,
        &f.revoked_terms,
        &keys(&verdicts, false, &|_, s| {
            Some(RevokedTerm::key_for(s.owner_licence, s.owner_term))
        }),
    )?;
    let bars = fetch(
        ctx,
        &f.revoked_principals,
        &keys(&verdicts, false, &|_, s| {
            principal_key(&Holder::Character(s.seller).principal())
        }),
    )?;
    for (i, s) in subjects.iter().enumerate() {
        let Some(verdict) = verdicts.get_mut(i) else {
            continue;
        };
        if verdict.is_some() {
            continue;
        }
        *verdict = owner_verdict(
            s,
            now,
            OwnerRows {
                status: status
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|r| (r.filed, r.withdrawn_by.is_some(), r.recalled_by.is_some())),
                licence: licences
                    .get(i)
                    .copied()
                    .flatten()
                    .filter(|l| l.id == s.owner_licence)
                    .map(|l| (l.charter, l.term)),
                revoked: revoked.get(i).copied().flatten().is_some(),
                barred: bars.get(i).copied().flatten().is_some_and(|r| {
                    r.principal == Holder::Character(s.seller).principal() && r.bars(bar_now)
                }),
            },
        );
    }

    // A derived listing's qualifying parent: who filed the child, the parent
    // it was filed under, and the seller's replacement when there is one.
    let births = fetch(
        ctx,
        &f.operations,
        &keys(&verdicts, true, &|i, _| {
            licences
                .get(i)
                .copied()
                .flatten()
                .map(|l| l.operation.to_hex())
        }),
    )?;
    let requester = |i: usize, s: &ListedSubject| {
        births
            .get(i)
            .copied()
            .flatten()
            .filter(|op| op.licence == s.owner_licence)
            .map(|op| op.requester)
    };
    let bindings = fetch(
        ctx,
        &f.owner_parents,
        &keys(&verdicts, true, &|i, s| {
            requester(i, s).map(|who| OwnerParentBinding::key_for(s.owner_licence, who))
        }),
    )?;
    let heads = fetch(
        ctx,
        &f.owner_qualification_heads,
        &keys(&verdicts, true, &|_, s| {
            Some(QualificationHead::key_for(s.owner_licence, s.seller))
        }),
    )?;
    let mut choices: Vec<Option<ParentChoice>> = subjects.iter().map(|_| None).collect();
    for (i, s) in subjects.iter().enumerate() {
        if s.parent.is_none() || verdicts.get(i).is_none_or(Option::is_some) {
            continue;
        }
        let choice = parent_choice(
            s,
            requester(i, s),
            bindings
                .get(i)
                .copied()
                .flatten()
                .filter(|b| b.licence == s.owner_licence)
                .map(|b| (b.parent.parent, b.parent.licence)),
            heads
                .get(i)
                .copied()
                .flatten()
                .filter(|h| h.owner == s.seller && h.owner_licence == s.owner_licence)
                .map(|h| h.operation),
        );
        match choice {
            Ok(choice) => {
                if let Some(slot) = choices.get_mut(i) {
                    *slot = Some(choice);
                }
            }
            Err(why) => {
                if let Some(verdict) = verdicts.get_mut(i) {
                    *verdict = Some(why);
                }
            }
        }
    }
    let replacements = fetch(
        ctx,
        &f.owner_qualification_operations,
        &keys(
            &verdicts,
            true,
            &|i, _| match choices.get(i).copied().flatten() {
                Some(ParentChoice::Replacement(operation)) => Some(operation.to_hex()),
                _ => None,
            },
        ),
    )?;
    let selected: Vec<Option<LicenceId>> = subjects
        .iter()
        .enumerate()
        .map(|(i, s)| match choices.get(i).copied().flatten()? {
            ParentChoice::Bound(licence) => Some(licence),
            ParentChoice::Replacement(operation) => replacements
                .get(i)
                .copied()
                .flatten()
                .filter(|op| {
                    op.operation == operation
                        && op.owner == s.seller
                        && op.owner_licence == s.owner_licence
                })
                .map(|op| op.selected_parent.licence),
        })
        .collect();
    let select = |i: usize| selected.get(i).copied().flatten();
    let pointers = fetch(
        ctx,
        &f.paid_licences,
        &keys(&verdicts, true, &|i, _| select(i).map(LicenceId::key)),
    )?;
    let sources = fetch(
        ctx,
        &f.status,
        &keys(&verdicts, true, &|_, s| s.parent.map(|p| p.filed.key())),
    )?;
    let pointer = |i: usize| {
        pointers
            .get(i)
            .copied()
            .flatten()
            .filter(|p| Some(p.id()) == select(i))
    };
    let terms = fetch(
        ctx,
        &f.paid_terms,
        &keys(&verdicts, true, &|i, _| {
            pointer(i).map(|p| format!("{}/{}", p.id().key(), p.term().key()))
        }),
    )?;
    let parent_revoked = fetch(
        ctx,
        &f.revoked_terms,
        &keys(&verdicts, true, &|i, _| {
            pointer(i).map(|p| RevokedTerm::key_for(p.id(), p.term()))
        }),
    )?;
    for (i, s) in subjects.iter().enumerate() {
        if s.parent.is_none() || verdicts.get(i).is_none_or(Option::is_some) {
            continue;
        }
        let why = parent_verdict(
            s,
            now,
            ParentRows {
                holder: pointer(i).map(|p| p.holder()),
                term: pointer(i).and_then(|p| {
                    terms
                        .get(i)
                        .copied()
                        .flatten()
                        .filter(|t| t.licence() == p.id() && t.generation() == p.term())
                        .map(|t| (t.window(), t.terms().form()))
                }),
                revoked: parent_revoked.get(i).copied().flatten().is_some(),
                source: sources
                    .get(i)
                    .copied()
                    .flatten()
                    .map(|r| (r.filed, r.recalled_by.is_some())),
            },
        );
        if let Some(verdict) = verdicts.get_mut(i) {
            *verdict = why;
        }
    }
    Ok(verdicts
        .into_iter()
        .map(|v| v.map_or(Sellable::Yes, Sellable::No))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::CommitToken;

    type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

    const SELLER: ObjId = ObjId(7);

    fn window() -> TestResult<TermWindow> {
        Ok(TermWindow::new(
            WorldIcSeconds(0),
            WorldIcSeconds(0),
            WorldIcSeconds(100),
            WorldIcSeconds(150),
        )?)
    }
    fn subject(derived: bool) -> TestResult<ListedSubject> {
        let digest = |byte: u8| mush_core::DefinitionDigest::from_bytes([byte; 32]);
        let revision = serde_json::from_value(serde_json::json!({"format":1,
            "revision":{"key":{"character":1,"mind":3,"number":1},"digest":digest(1)},
            "source":digest(2)}))?;
        Ok(ListedSubject {
            seller: SELLER,
            charter: CharterId::new(21)?,
            title_revision: 4,
            filed: FiledRevisionId::new(5)?,
            owner_licence: LicenceId::new(6)?,
            owner_term: TermGeneration::new(1)?,
            owner_window: window()?,
            parent: derived.then_some(ParentContent {
                filed: FiledRevisionId::new(2)?,
                revision,
            }),
        })
    }
    fn title(owner: ObjId, revision: u64) -> EconomicAsset {
        EconomicAsset {
            object: ObjId(21),
            title_owner: mush_org::PrincipalId::Object(owner),
            scope: None,
            assignment: None,
            maintenance_account: None,
            catalog: None,
            encumbrances: vec![],
            acquisition: None,
            lifecycle: AssetLifecycle::Active,
            last_journal_seq: 1,
            revision,
            stolen: None,
            frozen: false,
            frozen_reason: None,
        }
    }
    fn owner_rows(s: &ListedSubject) -> OwnerRows {
        OwnerRows {
            status: Some((s.filed, false, false)),
            licence: Some((s.charter, s.owner_term)),
            revoked: false,
            barred: false,
        }
    }
    fn parent_rows(s: &ListedSubject) -> TestResult<ParentRows> {
        Ok(ParentRows {
            holder: Some(Holder::Character(s.seller)),
            term: Some((window()?, PaidForm::OpenResale {})),
            revoked: false,
            source: Some((s.parent.ok_or("derived")?.filed, false)),
        })
    }

    #[test]
    fn a_listing_moved_with_its_charter_is_unsellable() -> TestResult {
        let s = subject(false)?;
        check_eq!(title_verdict(&s, Some(&title(SELLER, 4))), None);
        // Another owner, a later title revision, a frozen or missing title.
        for moved in [
            Some(title(ObjId(8), 4)),
            Some(title(SELLER, 5)),
            Some(EconomicAsset {
                frozen: true,
                ..title(SELLER, 4)
            }),
            Some(EconomicAsset {
                scope: Some(mush_org::OrgScopeId(1)),
                ..title(SELLER, 4)
            }),
            None,
        ] {
            check_eq!(
                title_verdict(&s, moved.as_ref()),
                Some(Unsellable::CharterMoved)
            );
        }
        Ok(())
    }

    #[test]
    fn a_withdrawn_or_recalled_revision_is_unsellable_and_recall_is_told_first() -> TestResult {
        let s = subject(false)?;
        let now = WorldIcSeconds(10);
        check_eq!(owner_verdict(&s, now, owner_rows(&s)), None);
        let with = |withdrawn, recalled| OwnerRows {
            status: Some((s.filed, withdrawn, recalled)),
            ..owner_rows(&s)
        };
        check_eq!(
            owner_verdict(&s, now, with(true, false)),
            Some(Unsellable::Withdrawn)
        );
        check_eq!(
            owner_verdict(&s, now, with(false, true)),
            Some(Unsellable::Recalled)
        );
        check_eq!(
            owner_verdict(&s, now, with(true, true)),
            Some(Unsellable::Recalled)
        );
        // A missing status row, or another revision's, proves nothing.
        check_eq!(
            owner_verdict(
                &s,
                now,
                OwnerRows {
                    status: None,
                    ..owner_rows(&s)
                }
            ),
            Some(Unsellable::Unproved)
        );
        check_eq!(
            owner_verdict(
                &s,
                now,
                OwnerRows {
                    status: Some((FiledRevisionId::new(9)?, false, false)),
                    ..owner_rows(&s)
                }
            ),
            Some(Unsellable::Unproved)
        );
        Ok(())
    }

    #[test]
    fn a_listing_lapses_with_its_sellers_licence() -> TestResult {
        let s = subject(false)?;
        // Active and Grace sell; the instant Grace ends does not.
        for now in [0, 99, 100, 149] {
            check_eq!(owner_verdict(&s, WorldIcSeconds(now), owner_rows(&s)), None);
        }
        for now in [-1, 150, 151, i64::MAX] {
            check_eq!(
                owner_verdict(&s, WorldIcSeconds(now), owner_rows(&s)),
                Some(Unsellable::SellerLicence)
            );
        }
        let now = WorldIcSeconds(10);
        // The licence moved to another term, was revoked, or its holder is
        // barred: each is the seller's licence.
        for rows in [
            OwnerRows {
                licence: Some((s.charter, TermGeneration::new(2)?)),
                ..owner_rows(&s)
            },
            OwnerRows {
                revoked: true,
                ..owner_rows(&s)
            },
            OwnerRows {
                barred: true,
                ..owner_rows(&s)
            },
        ] {
            check_eq!(
                owner_verdict(&s, now, rows),
                Some(Unsellable::SellerLicence)
            );
        }
        // No licence row, or another charter's, proves nothing.
        for rows in [
            OwnerRows {
                licence: None,
                ..owner_rows(&s)
            },
            OwnerRows {
                licence: Some((CharterId::new(22)?, s.owner_term)),
                ..owner_rows(&s)
            },
        ] {
            check_eq!(owner_verdict(&s, now, rows), Some(Unsellable::Unproved));
        }
        Ok(())
    }

    #[test]
    fn the_qualifying_parent_is_the_bound_licence_or_a_committed_replacement() -> TestResult {
        let s = subject(true)?;
        let source = s.parent.ok_or("derived")?;
        let bound = LicenceId::new(30)?;
        let replacement = CommitToken::from_bytes([9; 16]);
        // The original owner with no replacement keeps the licence it filed under.
        check_eq!(
            parent_choice(&s, Some(SELLER), Some((source, bound)), None),
            Ok(ParentChoice::Bound(bound))
        );
        // A replacement wins for anyone.
        for requester in [SELLER, ObjId(8)] {
            check_eq!(
                parent_choice(
                    &s,
                    Some(requester),
                    Some((source, bound)),
                    Some(replacement)
                ),
                Ok(ParentChoice::Replacement(replacement))
            );
        }
        // A later owner who selected no parent of their own does not qualify.
        check_eq!(
            parent_choice(&s, Some(ObjId(8)), Some((source, bound)), None),
            Err(Unsellable::ParentLicence)
        );
        // No birth, no binding, or a binding to another source proves nothing.
        let other = ParentContent {
            filed: FiledRevisionId::new(3)?,
            ..source
        };
        for (requester, binding) in [
            (None, Some((source, bound))),
            (Some(SELLER), None),
            (Some(SELLER), Some((other, bound))),
        ] {
            check_eq!(
                parent_choice(&s, requester, binding, None),
                Err(Unsellable::Unproved)
            );
        }
        Ok(())
    }

    #[test]
    fn a_derived_listing_lapses_with_its_parents_licence() -> TestResult {
        let s = subject(true)?;
        let now = WorldIcSeconds(10);
        check_eq!(parent_verdict(&s, now, parent_rows(&s)?), None);
        // Grace still sells; past it does not.
        check_eq!(
            parent_verdict(&s, WorldIcSeconds(149), parent_rows(&s)?),
            None
        );
        check_eq!(
            parent_verdict(&s, WorldIcSeconds(150), parent_rows(&s)?),
            Some(Unsellable::ParentLicence)
        );
        let source = s.parent.ok_or("derived")?.filed;
        for rows in [
            // Resold to someone else, or held by an organisation.
            ParentRows {
                holder: Some(Holder::Character(ObjId(8))),
                ..parent_rows(&s)?
            },
            ParentRows {
                holder: Some(Holder::Organization {
                    org: mush_org::OrgId(1),
                    acting: SELLER,
                }),
                ..parent_rows(&s)?
            },
            // Revoked, not an open-resale term, or the parent source recalled.
            ParentRows {
                revoked: true,
                ..parent_rows(&s)?
            },
            ParentRows {
                term: Some((window()?, PaidForm::Open {})),
                ..parent_rows(&s)?
            },
            ParentRows {
                source: Some((source, true)),
                ..parent_rows(&s)?
            },
        ] {
            check_eq!(
                parent_verdict(&s, now, rows),
                Some(Unsellable::ParentLicence)
            );
        }
        for rows in [
            ParentRows {
                holder: None,
                ..parent_rows(&s)?
            },
            ParentRows {
                term: None,
                ..parent_rows(&s)?
            },
            ParentRows {
                source: None,
                ..parent_rows(&s)?
            },
            ParentRows {
                source: Some((FiledRevisionId::new(9)?, false)),
                ..parent_rows(&s)?
            },
        ] {
            check_eq!(parent_verdict(&s, now, rows), Some(Unsellable::Unproved));
        }
        Ok(())
    }

    #[test]
    fn the_row_budget_a_listing_declares_covers_every_read_the_answer_makes() -> TestResult {
        // `check` reads, for one listing: status, licence, revoked term and
        // bar; and for a derived one also the birth, the binding, the head,
        // the replacement, the parent licence, the parent source's status,
        // the parent term and its revoked mark. One batched read each.
        check_eq!(subject(false)?.rows(), 4);
        check_eq!(subject(true)?.rows(), 12);
        check_eq!(SELLABLE_READS, SELLABLE_ROWS_DERIVED);
        // Two status batches (the listed revision's and the parent source's)
        // and two revoked-term batches each stay inside core's page of keys.
        check!(SELLABLE_SUBJECTS <= mush_core::read_service::MAX_OWNER_READ_PAGE);
        Ok(())
    }
}
