//! Durable measured rejection. This header and every original measured page
//! reconstruct the exact worker artifact; an error string is never evidence.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "HeaderWire")]
pub struct RatingNonconformance {
    format: u16,
    kind: NonconformanceKind,
    id: RatingId,
    subject: RatingSubject,
    compatibility: RatingCompatibility,
    plan: DefinitionDigest,
    cause: NonconformanceCause,
    result: RatingResultRef,
    publication: RatingPublication,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeaderWire {
    format: u16,
    kind: NonconformanceKind,
    id: RatingId,
    subject: RatingSubject,
    compatibility: RatingCompatibility,
    plan: DefinitionDigest,
    cause: NonconformanceCause,
    result: RatingResultRef,
    publication: RatingPublication,
}
impl TryFrom<HeaderWire> for RatingNonconformance {
    type Error = RatingError;
    fn try_from(w: HeaderWire) -> Result<Self, Self::Error> {
        let row = Self {
            format: w.format,
            kind: w.kind,
            id: w.id,
            subject: w.subject,
            compatibility: w.compatibility,
            plan: w.plan,
            cause: w.cause,
            result: w.result,
            publication: w.publication,
        };
        row.validate()?;
        Ok(row)
    }
}
impl RatingNonconformance {
    fn content_id(&self) -> Result<RatingId, RatingError> {
        Ok(RatingId(digest(
            "tacsim.registry.nonconformance.content.v1",
            &(
                &self.subject,
                &self.compatibility,
                self.plan,
                self.cause,
                &self.result,
            ),
            MAX_RECEIPT_BYTES - TYPED_ENVELOPE_RESERVE,
        )?))
    }
    pub(crate) fn validate(&self) -> Result<(), RatingError> {
        self.subject.validate()?;
        self.compatibility.validate()?;
        self.result.validate()?;
        self.publication.validate()?;
        if self.format != FORMAT || self.id != self.content_id()? {
            return Err(RatingError::Identity);
        }
        encoded(self, MAX_RECEIPT_BYTES - TYPED_ENVELOPE_RESERVE)?;
        Ok(())
    }
    pub fn id(&self) -> RatingId {
        self.id
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
    pub fn result(&self) -> &RatingResultRef {
        &self.result
    }
    pub fn publication(&self) -> &RatingPublication {
        &self.publication
    }
    pub fn reference(&self) -> Result<NonconformanceRef, RatingError> {
        self.validate()?;
        Ok(NonconformanceRef {
            job: self.publication.job(),
            id: self.id,
            schema: FORMAT,
            receipt_digest: digest(
                "tacsim.registry.nonconformance.receipt.v1",
                self,
                MAX_RECEIPT_BYTES - TYPED_ENVELOPE_RESERVE,
            )?,
        })
    }
    pub fn verify_pages(&self, pages: &[RatingPage]) -> Result<(), RatingError> {
        MeasuredNonconformance::from_receipt(self, pages).map(|_| ())
    }
}

/// Distinct typed locator; resolving a certificate never reads this family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RefWire")]
pub struct NonconformanceRef {
    job: CommitToken,
    id: RatingId,
    schema: u16,
    receipt_digest: DefinitionDigest,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RefWire {
    job: CommitToken,
    id: RatingId,
    schema: u16,
    receipt_digest: DefinitionDigest,
}
impl TryFrom<RefWire> for NonconformanceRef {
    type Error = RatingError;
    fn try_from(w: RefWire) -> Result<Self, Self::Error> {
        if w.schema != FORMAT {
            return Err(RatingError::Invalid);
        }
        Ok(Self {
            job: w.job,
            id: w.id,
            schema: w.schema,
            receipt_digest: w.receipt_digest,
        })
    }
}
impl NonconformanceRef {
    pub fn job(&self) -> CommitToken {
        self.job
    }
    pub fn id(&self) -> RatingId {
        self.id
    }
}

impl MeasuredNonconformance {
    pub fn publish_parts(
        self,
        publication: RatingPublication,
    ) -> Result<(RatingNonconformance, Vec<RatingPage>), RatingError> {
        self.validate()?;
        let result = RatingResultRef::from_pages(&self.pages.0)?;
        let mut receipt = RatingNonconformance {
            format: FORMAT,
            kind: NonconformanceKind::MeasuredNonconformance,
            id: RatingId(DefinitionDigest::from_bytes([0; 32])),
            subject: self.subject,
            compatibility: self.compatibility,
            plan: self.plan,
            cause: self.cause,
            result,
            publication,
        };
        receipt.id = receipt.content_id()?;
        receipt.validate()?;
        let pages = self
            .pages
            .0
            .into_iter()
            .map(|page| {
                RatingPage::new(
                    receipt.publication.job(),
                    receipt.id,
                    receipt.result.id(),
                    page,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        receipt.verify_pages(&pages)?;
        Ok((receipt, pages))
    }
    pub(crate) fn from_receipt(
        receipt: &RatingNonconformance,
        pages: &[RatingPage],
    ) -> Result<Self, RatingError> {
        receipt.validate()?;
        if pages.len() != receipt.result.pages().len() {
            return Err(RatingError::Incomplete);
        }
        let mut content = Vec::with_capacity(pages.len());
        let mut bytes = 0usize;
        for (page, wanted) in pages.iter().zip(receipt.result.pages()) {
            page.validate()?;
            bytes = bytes
                .checked_add(
                    encoded(page, MAX_PAGE_BYTES - TYPED_ENVELOPE_RESERVE)?.len()
                        + TYPED_ENVELOPE_RESERVE,
                )
                .ok_or(RatingError::Bound)?;
            if bytes > MAX_RESULT_BYTES
                || page.job != receipt.publication.job()
                || page.rating != receipt.id
                || page.result != receipt.result.id()
                || page.content.ordinal != wanted.ordinal()
                || page.content.digest()? != wanted.digest()
                || page.content.rows.0.len() != usize::from(wanted.rows)
            {
                return Err(RatingError::Identity);
            }
            content.push(page.content.clone());
        }
        if RatingResultRef::from_pages(&content)? != receipt.result {
            return Err(RatingError::Identity);
        }
        // The measured codec checks exact source/manifest, all admitted classes,
        // complete ordered pages, and the real final maximum-class budget row.
        match receipt.cause {
            NonconformanceCause::MaximumClassBudget => Self::maximum_class_budget(
                receipt.subject.clone(),
                receipt.compatibility.clone(),
                receipt.plan,
                content,
            ),
        }
    }
}
