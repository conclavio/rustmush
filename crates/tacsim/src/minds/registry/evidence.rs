//! Complete measured-prefix aggregation. A lower class may end on an actual
//! budget fault; a passing class must cover its exact entire frozen schedule.
//! No persistence/authority or public statistics constructor lives here.
use super::{plan::Schedule, subject::PreparedSubject};
use mush_roll::commerce::rating::execution::MeasuredRating;
use mush_roll::commerce::rating::nonconformance::{MeasuredNonconformance, MeasuredRegistryResult};
use mush_roll::commerce::rating::{
    Applicability, CaseBinding, CaseKind, CaseOutcome, CaseResult, CertificateContent,
    ClassSummary, MAX_CASES, MAX_PAGE_ROWS, PageContent, ProbeName, RatingClass,
    RatingCompatibility, RatingError, RatingResultRef,
};

pub(super) enum CaseDisposition {
    Next,
    Promote,
    Certified,
    Nonconforming,
}
pub(super) struct Evidence {
    rows: Vec<CaseResult>,
    next: usize,
    finished: bool,
}
impl Evidence {
    pub(super) fn new() -> Self {
        Self {
            rows: Vec::new(),
            next: 0,
            finished: false,
        }
    }
    /// `binding` is captured from the actual prepared case's runtime cards and
    /// concrete programs; caller data cannot name another source/program here.
    pub(super) fn push(
        &mut self,
        schedule: &Schedule,
        row: CaseResult,
        binding: &CaseBinding,
    ) -> Result<CaseDisposition, RatingError> {
        let expected = schedule
            .cases()
            .get(self.next)
            .ok_or(RatingError::Incomplete)?;
        if self.finished
            || self.rows.len() == MAX_CASES
            || row.key() != &expected.key
            || row.binding() != binding
        {
            return Err(RatingError::Identity);
        }
        let wanted = match expected.key.kind() {
            CaseKind::Kernel { .. } => Applicability::EngineBoundary,
            CaseKind::Probe { name, .. }
                if matches!(
                    name,
                    ProbeName::EmptyUnarmed
                        | ProbeName::Hostile450Unarmed
                        | ProbeName::HurtTwoUnarmed
                        | ProbeName::OrderLossUnarmed
                        | ProbeName::DensityUnarmed
                        | ProbeName::FordMobile
                        | ProbeName::FordDisabled
                ) =>
            {
                Applicability::Unarmed
            }
            _ => Applicability::Armed,
        };
        if row.coverage().applicability() != wanted
            || row.meter().thinks() > expected.maximum_thinks
        {
            return Err(RatingError::Identity);
        }
        let next = self.next.checked_add(1).ok_or(RatingError::Bound)?;
        let (next, finished, disposition) =
            if matches!(row.outcome(), CaseOutcome::BudgetExceeded {}) {
                let next = schedule
                    .cases()
                    .iter()
                    .enumerate()
                    .skip(next)
                    .find(|(_, case)| case.key.class() > expected.key.class())
                    .map(|(index, _)| index);
                match next {
                    Some(next) => (next, false, CaseDisposition::Promote),
                    None if expected.key.class() == RatingClass::III
                        && !matches!(expected.key.kind(), CaseKind::Kernel { .. }) =>
                    {
                        (self.next, true, CaseDisposition::Nonconforming)
                    }
                    None => return Err(RatingError::Invalid),
                }
            } else if schedule
                .cases()
                .get(next)
                .is_none_or(|case| case.key.class() != expected.key.class())
            {
                (next, true, CaseDisposition::Certified)
            } else {
                (next, false, CaseDisposition::Next)
            };
        // Every check above precedes changing the accepted evidence prefix.
        self.rows.push(row);
        self.next = next;
        self.finished = finished;
        Ok(disposition)
    }
    pub(super) fn finish(
        self,
        subject: &PreparedSubject,
        compatibility: &RatingCompatibility,
        schedule: &Schedule,
    ) -> Result<MeasuredRegistryResult, RatingError> {
        if !self.finished {
            return Err(RatingError::Incomplete);
        }
        let minimum = self
            .rows
            .last()
            .ok_or(RatingError::Incomplete)?
            .key()
            .class();
        let classes = [RatingClass::I, RatingClass::II, RatingClass::III]
            .into_iter()
            .filter(|class| *class >= subject.subject().declared() && *class <= minimum)
            .map(|class| {
                ClassSummary::from_cases(
                    class,
                    &self
                        .rows
                        .iter()
                        .filter(|row| row.key().class() == class)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        let subject_hash = subject.subject().digest()?;
        let contents = self
            .rows
            .chunks(MAX_PAGE_ROWS)
            .enumerate()
            .map(|(ordinal, rows)| {
                PageContent::new(
                    subject_hash,
                    compatibility.digest(),
                    u16::try_from(ordinal).map_err(|_| RatingError::Bound)?,
                    rows.to_vec(),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        if self
            .rows
            .last()
            .is_some_and(|row| matches!(row.outcome(), CaseOutcome::BudgetExceeded {}))
        {
            return MeasuredNonconformance::maximum_class_budget(
                subject.subject().clone(),
                compatibility.clone(),
                schedule.digest()?,
                contents,
            )
            .map(MeasuredRegistryResult::Nonconforming);
        }
        let result = RatingResultRef::from_pages(&contents)?;
        let content = CertificateContent::new(
            subject.subject().clone(),
            compatibility.clone(),
            schedule.digest()?,
            subject.aliases().to_vec(),
            classes,
            minimum,
            result.clone(),
        )?;
        MeasuredRating::new(content, contents).map(MeasuredRegistryResult::Certified)
    }
}
