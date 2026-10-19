//! Strict immutable re-conformance binding and current-attempt index.
//! Structural decoding cannot grant Registry authority or source access.
pub(crate) mod audit;
pub(crate) mod current;
pub(crate) mod fence;
pub(crate) mod history;
pub mod reader;
pub(crate) mod recall;

use mush_core::{CommitToken, DefinitionDigest};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;

use super::{RatingError, RatingRef};
use crate::commerce::{
    FiledRevisionId,
    records::{PublicationError, Record},
};

/// Present only on actual Registry-service admissions. Old jobs omit this field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "AdmissionWire")]
pub struct ReconformanceAdmission {
    format: u16,
    filed: FiledRevisionId,
    birth_rating: RatingRef,
    previous_attempt: Option<CommitToken>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdmissionWire {
    format: u16,
    filed: FiledRevisionId,
    birth_rating: RatingRef,
    #[serde(deserialize_with = "required_previous")]
    previous_attempt: Option<CommitToken>,
}
fn required_previous<'de, D: Deserializer<'de>>(d: D) -> Result<Option<CommitToken>, D::Error> {
    Option::<CommitToken>::deserialize(d)
}
impl TryFrom<AdmissionWire> for ReconformanceAdmission {
    type Error = RatingError;
    fn try_from(w: AdmissionWire) -> Result<Self, Self::Error> {
        let row = Self {
            format: w.format,
            filed: w.filed,
            birth_rating: w.birth_rating,
            previous_attempt: w.previous_attempt,
        };
        row.validate()?;
        Ok(row)
    }
}
impl ReconformanceAdmission {
    pub(crate) fn new(
        filed: FiledRevisionId,
        birth_rating: RatingRef,
        previous_attempt: Option<CommitToken>,
    ) -> Result<Self, RatingError> {
        let row = Self {
            format: 1,
            filed,
            birth_rating,
            previous_attempt,
        };
        row.validate()?;
        Ok(row)
    }
    pub(crate) fn validate(&self) -> Result<(), RatingError> {
        self.birth_rating.validate()?;
        if self.format != 1 || self.previous_attempt == Some(self.birth_rating.job()) {
            return Err(RatingError::Identity);
        }
        super::encoded(self, 1024)?;
        Ok(())
    }
    pub fn filed(&self) -> FiledRevisionId {
        self.filed
    }
    pub fn birth_rating(&self) -> RatingRef {
        self.birth_rating
    }
    pub fn previous_attempt(&self) -> Option<CommitToken> {
        self.previous_attempt
    }
}

/// Current attempt for exactly one immutable filed revision and compatibility.
/// Success/rejection is read from the actual job, never copied into a boolean.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "HeadWire")]
pub(crate) struct ReconformanceHead {
    pub(crate) format: u16,
    pub(crate) filed: FiledRevisionId,
    pub(crate) compatibility: DefinitionDigest,
    pub(crate) job: CommitToken,
    pub(crate) admitted_at_utc: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct HeadWire {
    format: u16,
    filed: FiledRevisionId,
    compatibility: DefinitionDigest,
    job: CommitToken,
    admitted_at_utc: i64,
}
impl TryFrom<HeadWire> for ReconformanceHead {
    type Error = PublicationError;
    fn try_from(w: HeadWire) -> Result<Self, Self::Error> {
        let row = Self {
            format: w.format,
            filed: w.filed,
            compatibility: w.compatibility,
            job: w.job,
            admitted_at_utc: w.admitted_at_utc,
        };
        row.validate()?;
        Ok(row)
    }
}
impl ReconformanceHead {
    pub(crate) fn key_for(
        filed: FiledRevisionId,
        compatibility: DefinitionDigest,
    ) -> Result<String, PublicationError> {
        DefinitionDigest::for_canonical_value(
            "roll.registry.reconformance.head-key.v1",
            &json!([filed, compatibility]),
        )
        .map(|d| d.to_string())
        .map_err(|_| PublicationError::Invalid)
    }
}
impl Record for ReconformanceHead {
    const FAMILY: &'static str = "roll_registry_reconformance_heads";
    const MAX_BYTES: usize = 4096;
    fn key(&self) -> String {
        // Both arguments have infallible finite canonical serializers. The
        // checked construction/validation path rejects any unexpected failure.
        Self::key_for(self.filed, self.compatibility).unwrap_or_default()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 || self.admitted_at_utc < 0 {
            return Err(PublicationError::Invalid);
        }
        let key = Self::key_for(self.filed, self.compatibility)?;
        let envelope = json!({"family":{"owner":"roll","id":Self::FAMILY},
            "schema":{"id":format!("{}.value.v1",Self::FAMILY),"version":1},
            "key":key,"value":self});
        mush_core::canonical_json_bytes_with_limit(&envelope, Self::MAX_BYTES)
            .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests;
