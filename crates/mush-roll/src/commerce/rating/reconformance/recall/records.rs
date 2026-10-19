use super::*;
use crate::commerce::{
    FiledRevisionId, RevisionIdentity, WorldIcSeconds,
    rating::{RatingRef, nonconformance::NonconformanceRef},
    records::{FiledRecord, RevisionStatus},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, GOD, ObjId};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Wire")]
pub(crate) struct MeasuredRecall {
    pub(crate) format: u16,
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) manifest: DefinitionRef,
    pub(crate) actor: ObjId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) birth_rating: RatingRef,
    pub(crate) rejection: NonconformanceRef,
    pub(crate) plan: DefinitionDigest,
    pub(crate) withdrawn_by: Option<CommitToken>,
    pub(crate) at_ic: ClockStamp,
    pub(crate) at_utc: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    manifest: DefinitionRef,
    actor: ObjId,
    filed: FiledRevisionId,
    revision: RevisionIdentity,
    birth_rating: RatingRef,
    rejection: NonconformanceRef,
    plan: DefinitionDigest,
    #[serde(deserialize_with = "required")]
    withdrawn_by: Option<CommitToken>,
    at_ic: ClockStamp,
    at_utc: i64,
}
fn required<'de, D: Deserializer<'de>>(d: D) -> Result<Option<CommitToken>, D::Error> {
    Option::deserialize(d)
}
impl TryFrom<Wire> for MeasuredRecall {
    type Error = PublicationError;
    fn try_from(w: Wire) -> Result<Self, Self::Error> {
        let value = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            manifest: w.manifest,
            actor: w.actor,
            filed: w.filed,
            revision: w.revision,
            birth_rating: w.birth_rating,
            rejection: w.rejection,
            plan: w.plan,
            withdrawn_by: w.withdrawn_by,
            at_ic: w.at_ic,
            at_utc: w.at_utc,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Record for MeasuredRecall {
    const FAMILY: &'static str = "roll_registry_measured_recalls";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        WorldIcSeconds::from_stamp(&self.at_ic).map_err(bad)?;
        if self.format != 1
            || self.actor != GOD
            || self.at_utc < 0
            || self.action != protocol::action(&self.manifest).map_err(bad)?
            || self.operation == self.rejection.job()
            || self.operation == self.birth_rating.job()
            || self.withdrawn_by == Some(self.operation)
        {
            return Err(PublicationError::Invalid);
        }
        let envelope = json!({"family":{"owner":crate::NS,"id":Self::FAMILY},
            "schema":{"id":format!("{}.value.v1",Self::FAMILY),"version":1},"key":self.key(),"value":self});
        mush_core::canonical_json_bytes_with_limit(&envelope, Self::MAX_BYTES)
            .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
impl MeasuredRecall {
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.registry.measured_recall.cause.v1",
            &json!(self),
        )
        .map_err(bad)
    }
    pub(crate) fn root(&self) -> Result<RequiredRoot, PublicationError> {
        Ok(RequiredRoot {
            anchor: RootAnchor::RegistryRecall(self.operation, self.digest()?),
            identity: self.revision,
        })
    }
    pub(crate) fn status(&self) -> RevisionStatus {
        RevisionStatus {
            format: 1,
            filed: self.filed,
            withdrawn_by: self.withdrawn_by,
            recalled_by: Some(self.operation),
        }
    }
    pub(crate) fn matches(&self, filed: &FiledRecord) -> Result<(), PublicationError> {
        self.validate()?;
        filed.validate()?;
        if filed.content.id() != self.filed
            || filed.content.revision() != self.revision
            || filed.rating != self.birth_rating
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}
