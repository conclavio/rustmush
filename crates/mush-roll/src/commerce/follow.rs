//! Nonserialized Follow locators. A candidate is never an execution warrant.
//!
//! Full cold audits establish the inventory; closed Root images update it only
//! after acknowledgement. The selected actual row, status, complete current
//! certificate, source and all holder predicates are still read by consumers.

mod audit;
#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use mush_core::DefinitionDigest;

use super::rating::{RatingCompatibility, RatingReceipt, RatingRef};
use super::records::{FiledRecord, LineRecord, PublicationError, Record};
use super::{CharterId, FiledRevisionId, LineId, RevisionIdentity, WorldIcSeconds};

pub(crate) use audit::rebuild;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    filed: FiledRevisionId,
    charter: CharterId,
    line: LineId,
    sequence: u64,
    at: i64,
    identity: RevisionIdentity,
    birth_rating: RatingRef,
    blocked: bool,
    // None records an actual unsuccessful/pending current-plan head. It must
    // override a historical birth certificate of the same compatibility.
    certificates: BTreeMap<DefinitionDigest, Option<RatingRef>>,
}

impl Entry {
    fn filing(filed: &FiledRecord, rating: &RatingReceipt) -> Result<Self, PublicationError> {
        filed.validate()?;
        rating.validate().map_err(|_| PublicationError::Mismatch)?;
        if rating.reference().map_err(|_| PublicationError::Mismatch)? != filed.rating
            || rating.subject().revision() != filed.content.revision()
            || rating.subject().definition() != filed.definition
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(Self {
            filed: filed.content.id(),
            charter: filed.content.position().charter,
            line: filed.content.position().line,
            sequence: filed.content.position().sequence.get(),
            at: filed.content.filed_at().get(),
            identity: filed.content.revision(),
            birth_rating: filed.rating,
            blocked: false,
            certificates: BTreeMap::from([(rating.compatibility().digest(), Some(filed.rating))]),
        })
    }

    fn matches(&self, filed: &FiledRecord) -> Result<(), PublicationError> {
        filed.validate()?;
        if self.filed != filed.content.id()
            || self.charter != filed.content.position().charter
            || self.line != filed.content.position().line
            || self.sequence != filed.content.position().sequence.get()
            || self.at != filed.content.filed_at().get()
            || self.identity != filed.content.revision()
            || self.birth_rating != filed.rating
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

/// Constructed only inside actual filing/Registry producers after their exact
/// typed post-image and opaque owner-output checks. No wire constructor exists.
#[derive(Clone)]
pub(crate) struct Change(Kind);

#[derive(Clone)]
enum Kind {
    Filing {
        entry: Entry,
        previous: Option<FiledRevisionId>,
    },
    Block(FiledRevisionId),
    Certificate {
        filed: FiledRevisionId,
        identity: RevisionIdentity,
        birth_rating: RatingRef,
        compatibility: DefinitionDigest,
        rating: Option<RatingRef>,
    },
}

impl Change {
    pub(crate) fn filing(
        filed: &FiledRecord,
        rating: &RatingReceipt,
        previous: Option<FiledRevisionId>,
    ) -> Result<Self, PublicationError> {
        Ok(Self(Kind::Filing {
            entry: Entry::filing(filed, rating)?,
            previous,
        }))
    }

    pub(crate) fn blocked(root: &super::roots::RequiredRoot) -> Result<Self, PublicationError> {
        match &root.anchor {
            super::roots::RootAnchor::Filed(filed) => Ok(Self(Kind::Block(*filed))),
            _ => Err(PublicationError::Mismatch),
        }
    }

    pub(crate) fn certificate(
        filed: &FiledRecord,
        job: &super::rating::RatingJob,
        execution: &super::rating::execution::RatingExecution,
    ) -> Result<Self, PublicationError> {
        filed.validate()?;
        execution
            .matches_job(job)
            .map_err(|_| PublicationError::Mismatch)?;
        let admission = execution
            .admission()
            .reconformance
            .as_ref()
            .ok_or(PublicationError::Mismatch)?;
        if admission.filed() != filed.content.id()
            || admission.birth_rating() != filed.rating
            || job.subject().revision() != filed.content.revision()
            || job.subject().definition() != filed.definition
        {
            return Err(PublicationError::Mismatch);
        }
        let rating = match job.state() {
            super::rating::RatingJobState::Completed { receipt } => Some(*receipt),
            _ => None,
        };
        Ok(Self(Kind::Certificate {
            filed: filed.content.id(),
            identity: filed.content.revision(),
            birth_rating: filed.rating,
            compatibility: job.compatibility().digest(),
            rating,
        }))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Head {
    filed: FiledRevisionId,
    sequence: u64,
    at: i64,
}

/// At most MAX_RECORDS filed rows and MAX_RECORDS additional current-plan
/// heads, matching the existing independently bounded durable families.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Index {
    entries: BTreeMap<FiledRevisionId, Entry>,
    heads: BTreeMap<LineId, Head>,
    eligible: BTreeMap<(LineId, DefinitionDigest), BTreeMap<(i64, u64), FiledRevisionId>>,
    certificate_count: usize,
}

/// The replacement entry is built and validated before ACK. Apply has no lookup
/// of external state, decoding, callbacks or fallible authority decisions.
pub(crate) struct Prepared {
    before: Option<Entry>,
    after: Entry,
    head: Option<Head>,
    certificate_count: usize,
}

impl Index {
    pub(crate) fn prepare(&self, change: &Change) -> Result<Prepared, PublicationError> {
        let (before, after, head) = match &change.0 {
            Kind::Filing { entry, previous } => {
                if self.entries.contains_key(&entry.filed)
                    || self.entries.len() >= crate::MAX_RECORDS
                {
                    return Err(PublicationError::Capacity);
                }
                match (self.heads.get(&entry.line), previous) {
                    (None, None) if entry.sequence == 1 => {}
                    (Some(old), Some(previous))
                        if old.filed == *previous
                            && old.sequence.checked_add(1) == Some(entry.sequence)
                            && old.at <= entry.at
                            && self.entries.get(&old.filed).is_some_and(|before| {
                                before.charter == entry.charter && before.line == entry.line
                            }) => {}
                    _ => return Err(PublicationError::Mismatch),
                }
                let head = Head {
                    filed: entry.filed,
                    sequence: entry.sequence,
                    at: entry.at,
                };
                (None, entry.clone(), Some(head))
            }
            Kind::Block(filed) => {
                let old = self.entries.get(filed).ok_or(PublicationError::Missing)?;
                let mut next = old.clone();
                next.blocked = true;
                (Some(old.clone()), next, None)
            }
            Kind::Certificate {
                filed,
                identity,
                birth_rating,
                compatibility,
                rating,
            } => {
                let old = self.entries.get(filed).ok_or(PublicationError::Missing)?;
                if old.identity != *identity || old.birth_rating != *birth_rating {
                    return Err(PublicationError::Mismatch);
                }
                let mut next = old.clone();
                next.certificates.insert(*compatibility, *rating);
                (Some(old.clone()), next, None)
            }
        };
        let certificate_count = self
            .certificate_count
            .checked_sub(before.as_ref().map_or(0, |e| e.certificates.len()))
            .and_then(|n| n.checked_add(after.certificates.len()))
            .filter(|n| *n <= crate::MAX_RECORDS * 2)
            .ok_or(PublicationError::Capacity)?;
        Ok(Prepared {
            before,
            after,
            head,
            certificate_count,
        })
    }

    pub(crate) fn apply(&mut self, prepared: Prepared) {
        if let Some(old) = prepared.before {
            for compatibility in old.certificates.keys() {
                let key = (old.line, *compatibility);
                if let Some(candidates) = self.eligible.get_mut(&key) {
                    candidates.remove(&(old.at, old.sequence));
                    if candidates.is_empty() {
                        self.eligible.remove(&key);
                    }
                }
            }
        }
        let next = prepared.after;
        if !next.blocked {
            for (compatibility, rating) in &next.certificates {
                if rating.is_some() {
                    self.eligible
                        .entry((next.line, *compatibility))
                        .or_default()
                        .insert((next.at, next.sequence), next.filed);
                }
            }
        }
        if let Some(head) = prepared.head {
            self.heads.insert(next.line, head);
        }
        self.entries.insert(next.filed, next);
        self.certificate_count = prepared.certificate_count;
    }

    pub(crate) fn select(
        &self,
        line: &LineRecord,
        compatibility: &RatingCompatibility,
        now: WorldIcSeconds,
    ) -> Result<FiledRevisionId, PublicationError> {
        line.validate()?;
        let head = self.heads.get(&line.id).ok_or(PublicationError::Missing)?;
        if head.filed != line.head
            || head.sequence.checked_add(1) != Some(line.next_sequence)
            || self
                .entries
                .get(&head.filed)
                .is_none_or(|entry| entry.charter != line.charter || entry.line != line.id)
        {
            return Err(PublicationError::Mismatch);
        }
        // Persisted FiledRevision times predate versioned ClockStamp. Preserve
        // their existing absolute IC ordering; never compare invented epochs.
        self.eligible
            .get(&(line.id, compatibility.digest()))
            .and_then(|rows| rows.range(..=(now.get(), u64::MAX)).next_back())
            .map(|(_, filed)| *filed)
            .ok_or(PublicationError::Unavailable)
    }

    pub(crate) fn check_selected(
        &self,
        filed: &FiledRecord,
        compatibility: &RatingCompatibility,
        rating: &RatingRef,
    ) -> Result<(), PublicationError> {
        let entry = self
            .entries
            .get(&filed.content.id())
            .ok_or(PublicationError::Missing)?;
        entry.matches(filed)?;
        if entry.blocked || entry.certificates.get(&compatibility.digest()) != Some(&Some(*rating))
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
