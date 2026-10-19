//! Private root spelling and exact obligation-derived requests.
//!
//! A root retains content; it never grants use/source/settlement authority.
//! External owner-string APIs cannot construct or remove this root kind.

use super::rating::RatingId;
use super::records::{
    FiledRecord, InitialFilingOperation, OwnerTerm, PublicationError, RevisionPolicy,
};
use super::{FiledRevisionId, LicenceId, RevisionIdentity, TermGeneration};
use crate::{NS, ReferenceKey, ReferenceKind, RevisionRef};
use mush_core::CommitToken;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CommerceReadiness {
    #[default]
    Disabled,
    Closed,
    Ready {
        source_epoch: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum RootAnchor {
    Filed(FiledRevisionId),
    Term(LicenceId, TermGeneration),
    Operation(CommitToken),
    UpdateOperation(CommitToken),
    OwnerParent(LicenceId, mush_core::ObjId),
    PaidOperation(CommitToken),
    RenewalOperation(CommitToken),
    DraftRetirement(CommitToken, mush_core::DefinitionDigest),
    AuthorWithdrawal(CommitToken, mush_core::DefinitionDigest),
    OwnerQualification(CommitToken, mush_core::DefinitionDigest),
    Rating {
        job: CommitToken,
        id: RatingId,
    },
    RatingJob(CommitToken),
    /// Actual queued/running output obligation; never a certified receipt.
    RatingPending(CommitToken),
}

impl RootAnchor {
    pub(crate) fn record(&self) -> String {
        match self {
            Self::Filed(id) => format!("v1/{}", id.key()),
            Self::Term(id, term) => format!("v1/{}/{}", id.key(), term.key()),
            Self::Operation(token) => format!("v1/operation/{token}"),
            Self::UpdateOperation(token) => format!("v1/line_update/{token}"),
            Self::OwnerParent(licence, owner) => {
                format!("v1/owner_parent/{:016x}/{:08x}", licence.get(), owner.0)
            }
            Self::PaidOperation(token) => format!("v1/paid_operation/{token}"),
            Self::RenewalOperation(token) => format!("v1/renewal_operation/{token}"),
            Self::DraftRetirement(token, digest) => format!("v1/draft_retirement/{token}/{digest}"),
            Self::OwnerQualification(token, digest) => {
                format!("v1/owner_qualification/{token}/{digest}")
            }
            Self::AuthorWithdrawal(token, digest) => {
                format!("v1/author_withdrawal/{token}/{digest}")
            }
            Self::Rating { job, id } => format!("v1/rating/{job}/{id}"),
            Self::RatingJob(token) => format!("v1/rating_job/{token}"),
            Self::RatingPending(token) => format!("v1/rating_pending/{token}"),
        }
    }
    pub(crate) fn key(&self) -> ReferenceKey {
        ReferenceKey {
            owner: NS.into(),
            kind: ReferenceKind::Commercial,
            record: self.record(),
        }
    }
    pub(crate) fn parse(text: &str) -> Result<Self, PublicationError> {
        let mut parts = text.split('/');
        if parts.next() != Some("v1") {
            return Err(PublicationError::Invalid);
        }
        let kind = parts.next().ok_or(PublicationError::Invalid)?;
        let id = parts.next().ok_or(PublicationError::Invalid)?;
        let anchor = match kind {
            "filed" => Self::Filed(
                FiledRevisionId::new(number(id)?).map_err(|_| PublicationError::Invalid)?,
            ),
            "licence" => {
                let licence = LicenceId::new(number(id)?).map_err(|_| PublicationError::Invalid)?;
                if parts.next() != Some("term") {
                    return Err(PublicationError::Invalid);
                }
                let generation =
                    TermGeneration::new(number(parts.next().ok_or(PublicationError::Invalid)?)?)
                        .map_err(|_| PublicationError::Invalid)?;
                Self::Term(licence, generation)
            }
            "owner_parent" => {
                let licence = LicenceId::new(number(id)?).map_err(|_| PublicationError::Invalid)?;
                let owner = parts.next().ok_or(PublicationError::Invalid)?;
                if owner.len() != 8
                    || !owner
                        .bytes()
                        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
                {
                    return Err(PublicationError::Invalid);
                }
                let owner =
                    u32::from_str_radix(owner, 16).map_err(|_| PublicationError::Invalid)?;
                if owner == 0 {
                    return Err(PublicationError::Invalid);
                }
                Self::OwnerParent(licence, mush_core::ObjId(owner))
            }
            "operation" => {
                Self::Operation(CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?)
            }
            "line_update" => Self::UpdateOperation(
                CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?,
            ),
            "paid_operation" => Self::PaidOperation(
                CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?,
            ),
            "renewal_operation" => Self::RenewalOperation(
                CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?,
            ),
            "owner_qualification" => Self::OwnerQualification(
                CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?,
                mush_core::DefinitionDigest::from_hex(
                    parts.next().ok_or(PublicationError::Invalid)?,
                )
                .map_err(|_| PublicationError::Invalid)?,
            ),
            "author_withdrawal" => Self::AuthorWithdrawal(
                CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?,
                mush_core::DefinitionDigest::from_hex(
                    parts.next().ok_or(PublicationError::Invalid)?,
                )
                .map_err(|_| PublicationError::Invalid)?,
            ),
            "draft_retirement" => Self::DraftRetirement(
                CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?,
                mush_core::DefinitionDigest::from_hex(
                    parts.next().ok_or(PublicationError::Invalid)?,
                )
                .map_err(|_| PublicationError::Invalid)?,
            ),
            "rating" => Self::Rating {
                job: CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?,
                id: RatingId::from_hex(parts.next().ok_or(PublicationError::Invalid)?)
                    .map_err(|_| PublicationError::Invalid)?,
            },
            "rating_pending" => Self::RatingPending(
                CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?,
            ),
            "rating_job" => {
                Self::RatingJob(CommitToken::from_hex(id).map_err(|_| PublicationError::Invalid)?)
            }
            _ => return Err(PublicationError::Invalid),
        };
        if parts.next().is_some() || anchor.record() != text {
            return Err(PublicationError::Invalid);
        }
        Ok(anchor)
    }
}

fn number(text: &str) -> Result<u64, PublicationError> {
    if text.len() != 16
        || !text
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(PublicationError::Invalid);
    }
    let value = u64::from_str_radix(text, 16).map_err(|_| PublicationError::Invalid)?;
    if value == 0 {
        return Err(PublicationError::Invalid);
    }
    Ok(value)
}

#[derive(Clone, Debug)]
pub(crate) struct RequiredRoot {
    pub(crate) anchor: RootAnchor,
    pub(crate) identity: RevisionIdentity,
}
impl RequiredRoot {
    pub(crate) fn target(&self) -> RevisionRef {
        self.identity.revision()
    }
}

/// Nonserializable request minted only from the proposed full first-filing
/// records. The filing builder additionally checks the actual rating/E4 result
/// and reads back those exact ActionTxn post-images before requesting roots.
pub(crate) struct InitialRoots {
    commit: CommitToken,
    roots: Vec<RequiredRoot>,
}
impl InitialRoots {
    pub(crate) fn from_records(
        filed: &FiledRecord,
        term: &OwnerTerm,
        operation: &InitialFilingOperation,
    ) -> Result<Self, PublicationError> {
        use super::records::Record;
        filed.validate()?;
        term.validate()?;
        operation.validate()?;
        if filed.operation != operation.commit
            || filed.content.id() != operation.filed
            || filed.content.revision() != operation.revision
            || filed.rating != operation.rating
            || term.operation != operation.commit
            || term.licence != operation.licence
            || term.generation != operation.term
        {
            return Err(PublicationError::Mismatch);
        }
        let mut roots = vec![
            RequiredRoot {
                anchor: RootAnchor::Filed(filed.content.id()),
                identity: operation.revision,
            },
            RequiredRoot {
                anchor: RootAnchor::Operation(operation.commit),
                identity: operation.revision,
            },
        ];
        if term.parent != operation.parent
            || filed.content.parent() != operation.parent.as_ref().map(|p| p.parent)
        {
            return Err(PublicationError::Mismatch);
        }
        if let Some(parent) = &operation.parent {
            roots.push(RequiredRoot {
                anchor: RootAnchor::OwnerParent(operation.licence, parent.owner),
                identity: parent.parent.revision,
            });
        }
        match term.revision {
            RevisionPolicy::Pin {
                filed: id,
                revision,
            } => {
                if id != operation.filed || revision != operation.revision {
                    return Err(PublicationError::Mismatch);
                }
                roots.push(RequiredRoot {
                    anchor: RootAnchor::Term(term.licence, term.generation),
                    identity: revision,
                });
            }
            RevisionPolicy::Follow { line } => {
                if line != operation.line {
                    return Err(PublicationError::Mismatch);
                }
            }
        }
        Ok(Self {
            commit: operation.commit,
            roots,
        })
    }
    pub(crate) fn roots(&self) -> &[RequiredRoot] {
        &self.roots
    }
    pub(crate) const fn commit(&self) -> CommitToken {
        self.commit
    }
}
