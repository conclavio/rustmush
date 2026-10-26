//! Measured source rejection is distinct from an interrupted or failed worker.
//! This bounded artifact records a real measured prefix through the highest
//! admitted class. Its codec proves consistency, never that execution occurred.
//! Only the actual quota/artifact union may publish it as Registry evidence.
use super::*;
use sha2::{Digest as _, Sha256};

mod publication;
pub use publication::{NonconformanceRef, RatingNonconformance};

/// Add new measured causes only with an actual worker measurement and verifier.
/// Compilation, engine errors, cancellation and resource exhaustion cannot be
/// converted to this cause by matching a generic error string.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NonconformanceCause {
    MaximumClassBudget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MeasuredNonconformance {
    format: u16,
    kind: NonconformanceKind,
    subject: RatingSubject,
    compatibility: RatingCompatibility,
    plan: DefinitionDigest,
    cause: NonconformanceCause,
    pages: Rows<PageContent, MAX_PAGES>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum NonconformanceKind {
    MeasuredNonconformance,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    kind: NonconformanceKind,
    subject: RatingSubject,
    compatibility: RatingCompatibility,
    plan: DefinitionDigest,
    cause: NonconformanceCause,
    pages: Rows<PageContent, MAX_PAGES>,
}
impl<'de> Deserialize<'de> for MeasuredNonconformance {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let value = Self {
            format: w.format,
            kind: w.kind,
            subject: w.subject,
            compatibility: w.compatibility,
            plan: w.plan,
            cause: w.cause,
            pages: w.pages,
        };
        value.validate().map_err(de::Error::custom)?;
        Ok(value)
    }
}
impl MeasuredNonconformance {
    pub fn maximum_class_budget(
        subject: RatingSubject,
        compatibility: RatingCompatibility,
        plan: DefinitionDigest,
        pages: Vec<PageContent>,
    ) -> Result<Self, RatingError> {
        let value = Self {
            format: 1,
            kind: NonconformanceKind::MeasuredNonconformance,
            subject,
            compatibility,
            plan,
            cause: NonconformanceCause::MaximumClassBudget,
            pages: Rows::new(pages)?,
        };
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RatingError> {
        self.subject.validate()?;
        self.compatibility.validate()?;
        if self.format != 1 || self.pages.0.is_empty() {
            return Err(RatingError::Invalid);
        }
        let subject = self.subject.digest()?;
        let mut prior = None;
        let mut count = 0usize;
        let mut current = self.subject.declared();
        let mut class_rows = Vec::new();
        for (ordinal, page) in self.pages.0.iter().enumerate() {
            page.validate()?;
            if usize::from(page.ordinal) != ordinal
                || page.subject != subject
                || page.compatibility != self.compatibility.digest()
                || (ordinal + 1 < self.pages.0.len() && page.rows.0.len() != MAX_PAGE_ROWS)
            {
                return Err(RatingError::Identity);
            }
            for row in &page.rows.0 {
                count = count.checked_add(1).ok_or(RatingError::Bound)?;
                if count > MAX_CASES || prior.is_some_and(|p| p >= &row.key) {
                    return Err(RatingError::Bound);
                }
                prior = Some(&row.key);
                if !matches!(row.key.case, CaseKind::Kernel { .. })
                    && row.binding.source != self.subject.revision().source()
                {
                    return Err(RatingError::Identity);
                }
                if row.key.class != current {
                    check_budget_class(current, &class_rows)?;
                    current = match current {
                        RatingClass::I => RatingClass::II,
                        RatingClass::II => RatingClass::III,
                        RatingClass::III => return Err(RatingError::Invalid),
                    };
                    if row.key.class != current {
                        return Err(RatingError::Incomplete);
                    }
                    class_rows.clear();
                }
                class_rows.push(row);
            }
        }
        if current != RatingClass::III {
            return Err(RatingError::Incomplete);
        }
        check_budget_class(current, &class_rows)?;
        encoded(self, MAX_RESULT_BYTES)?;
        Ok(())
    }
    pub fn subject(&self) -> &RatingSubject {
        &self.subject
    }
    pub fn compatibility(&self) -> &RatingCompatibility {
        &self.compatibility
    }
    pub fn plan(&self) -> DefinitionDigest {
        self.plan
    }
    pub fn cause(&self) -> NonconformanceCause {
        self.cause
    }
    pub fn pages(&self) -> &[PageContent] {
        &self.pages.0
    }
    /// Identity of the exact bytes returned by the real worker. Calling this
    /// function cannot authorize a recall or publish a result.
    pub fn artifact(&self) -> Result<execution::ArtifactIdentity, RatingError> {
        self.validate()?;
        let bytes = encoded(self, MAX_RESULT_BYTES)?;
        Ok(execution::ArtifactIdentity {
            sha256: DefinitionDigest::from_bytes(Sha256::digest(&bytes).into()),
            bytes: bytes.len() as u64,
        })
    }
}
fn check_budget_class(class: RatingClass, rows: &[&CaseResult]) -> Result<(), RatingError> {
    let summary = ClassSummary::from_cases(class, rows)?;
    if summary.outcome != ClassOutcome::BudgetExceeded
        || rows
            .last()
            .is_none_or(|r| !matches!(r.outcome, CaseOutcome::BudgetExceeded {}))
    {
        return Err(RatingError::Invalid);
    }
    Ok(())
}

/// A successful artifact retains its existing JSON byte shape. A rejection has
/// its own explicit kind and can never decode as a successful MeasuredRating.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MeasuredRegistryResult {
    Certified(execution::MeasuredRating),
    Nonconforming(MeasuredNonconformance),
}
impl MeasuredRegistryResult {
    pub fn artifact(&self) -> Result<execution::ArtifactIdentity, RatingError> {
        match self {
            Self::Certified(v) => v.artifact(),
            Self::Nonconforming(v) => v.artifact(),
        }
    }
}

#[cfg(test)]
mod tests;
