//! Immutable term-expiry occurrences. Decoding never creates consent or authority.
use super::super::Holder;
use super::super::{
    CharterId, LicenceId, LineId, RevisionIdentity, TermGeneration, TermWindow, WorldIcSeconds,
};
use super::*;
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, GOD, ObjId};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum PersonalKind {
    Owner,
    Paid,
}

/// Historical acceptance and current subject authority are deliberately separate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AcceptedTerm {
    pub(super) kind: PersonalKind,
    pub(super) licence: LicenceId,
    pub(super) generation: TermGeneration,
    pub(super) birth: CommitToken,
    pub(super) term_operation: CommitToken,
    pub(super) term_digest: DefinitionDigest,
    pub(super) accepted_holder: Holder,
    pub(super) charter: CharterId,
    pub(super) line: LineId,
    pub(super) revision: RevisionIdentity,
    pub(super) window: TermWindow,
}
impl AcceptedTerm {
    pub(super) fn key(&self) -> String {
        format!("{}/{}", self.licence.key(), self.generation.key())
    }
    pub(super) fn validate(&self) -> Result<(), PublicationError> {
        if (self.generation.get() == 1) != (self.term_operation == self.birth) {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    pub(super) fn expired(&self, stamp: &ClockStamp) -> Result<(), PublicationError> {
        self.validate()?;
        let now = WorldIcSeconds::from_stamp(stamp).map_err(invalid)?;
        if now < self.window.grace_end() {
            return Err(PublicationError::Unavailable);
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Wire")]
pub(crate) struct LapseOccurrence {
    pub(super) format: u16,
    pub(super) operation: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) term: AcceptedTerm,
    pub(super) observed_head: TermGeneration,
    pub(super) actor: ObjId,
    pub(super) at_ic: ClockStamp,
    pub(super) at_utc: i64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    operation: CommitToken,
    action: DefinitionRef,
    term: AcceptedTerm,
    observed_head: TermGeneration,
    actor: ObjId,
    at_ic: ClockStamp,
    at_utc: i64,
}
impl TryFrom<Wire> for LapseOccurrence {
    type Error = PublicationError;
    fn try_from(w: Wire) -> Result<Self, Self::Error> {
        let value = Self {
            format: w.format,
            operation: w.operation,
            action: w.action,
            term: w.term,
            observed_head: w.observed_head,
            actor: w.actor,
            at_ic: w.at_ic,
            at_utc: w.at_utc,
        };
        value.validate()?;
        Ok(value)
    }
}
impl Record for LapseOccurrence {
    const FAMILY: &'static str = "roll_mind_lapse_occurrences";
    const MAX_BYTES: usize = 8192;
    fn key(&self) -> String {
        self.term.key()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        self.term.expired(&self.at_ic)?;
        if self.observed_head < self.term.generation
            || self.format != 1
            || self.actor != GOD
            || self.action != super::protocol::action().map_err(invalid)?
            || self.operation == self.term.birth
            || self.operation == self.term.term_operation
        {
            return Err(PublicationError::Invalid);
        }
        let envelope = serde_json::json!({"family":{"owner":crate::NS,"id":Self::FAMILY},"key":self.key(),
            "schema":{"id":format!("{}.value.v1",Self::FAMILY),"version":1},"value":self});
        mush_core::canonical_json_bytes_with_limit(&envelope, Self::MAX_BYTES)
            .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}
impl LapseOccurrence {
    pub(super) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.minds_lapse_occurrence.v1",
            &serde_json::to_value(self).map_err(invalid)?,
        )
        .map_err(invalid)
    }
    pub(super) fn matches(&self, term: &AcceptedTerm) -> Result<(), PublicationError> {
        self.validate()?;
        if &self.term != term {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub(super) fn root(&self) -> Result<super::super::roots::RequiredRoot, PublicationError> {
        Ok(super::super::roots::RequiredRoot {
            anchor: super::super::roots::RootAnchor::TermLapse(
                self.term.licence,
                self.term.generation,
                self.operation,
                self.digest()?,
            ),
            identity: self.term.revision,
        })
    }
}
pub(super) fn term_digest<T: Serialize>(
    kind: PersonalKind,
    value: &T,
) -> Result<DefinitionDigest, PublicationError> {
    DefinitionDigest::for_canonical_value(
        match kind {
            PersonalKind::Owner => "roll.minds_fact.accepted_owner_term.v1",
            PersonalKind::Paid => "roll.minds_fact.accepted_paid_term.v1",
        },
        &serde_json::to_value(value).map_err(invalid)?,
    )
    .map_err(invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    type Result<T = ()> = std::result::Result<T, Box<dyn std::error::Error>>;
    fn row() -> Result<LapseOccurrence> {
        let hash = DefinitionDigest::from_bytes([255; 32]);
        Ok(serde_json::from_value(serde_json::json!({
            "format":1,"operation":CommitToken::from_bytes([255;16]),"action":super::super::protocol::action()?,
            "term":{"kind":"paid","licence":u64::MAX.to_string(),"generation":u64::MAX.to_string(),"birth":CommitToken::from_bytes([253;16]),"term_operation":CommitToken::from_bytes([254;16]),"term_digest":hash,"accepted_holder":u32::MAX,"charter":u32::MAX.to_string(),"line":u64::MAX.to_string(),
                "revision":{"format":1,"revision":{"key":{"character":u32::MAX,"mind":u64::MAX,"number":u64::MAX},"digest":hash},"source":hash},
                "window":{"format":1,"eligibility_start":i64::MAX-100,"issued_at":i64::MAX-100,"term_end":i64::MAX-10,"grace_end":i64::MAX}},
            "observed_head":u64::MAX.to_string(),"actor":GOD,"at_ic":{"clock":{"kind":"world_ic","generation":u64::MAX},"at":i64::MAX},"at_utc":i64::MIN
        }))?)
    }
    #[test]
    fn committed_lapse_has_exact_closed_term_clock_root_and_full_envelope() -> Result {
        let r = row()?;
        r.validate()?;
        let value = serde_json::to_value(&r)?;
        check_eq!(serde_json::from_value::<LapseOccurrence>(value.clone())?, r);
        let envelope = serde_json::json!({"family":{"owner":"roll","id":LapseOccurrence::FAMILY},"schema":{"id":format!("{}.value.v1",LapseOccurrence::FAMILY),"version":1},"key":r.key(),"value":value});
        check!(
            mush_core::canonical_json_bytes_with_limit(&envelope, LapseOccurrence::MAX_BYTES)
                .is_ok()
        );
        for field in [
            "operation",
            "term",
            "observed_head",
            "actor",
            "at_ic",
            "at_utc",
        ] {
            let mut bad = value.clone();
            bad.as_object_mut().ok_or("object")?.remove(field);
            check!(
                serde_json::from_value::<LapseOccurrence>(bad).is_err(),
                "required {field}"
            );
        }
        for pointer in ["", "/term", "/term/window", "/term/revision", "/at_ic"] {
            let mut bad = value.clone();
            bad.pointer_mut(pointer)
                .and_then(serde_json::Value::as_object_mut)
                .ok_or("nested object")?
                .insert("permission".into(), serde_json::json!(true));
            check!(
                serde_json::from_value::<LapseOccurrence>(bad).is_err(),
                "closed {pointer}"
            );
        }
        let mut early = r.clone();
        early.at_ic.at -= 1;
        check!(early.validate().is_err());
        let mut actor = r.clone();
        actor.actor = ObjId(u32::MAX);
        check!(actor.validate().is_err());
        let mut clock = r.clone();
        clock.at_ic.clock = mush_core::ClockRef::WorldIc { generation: 0 };
        check!(clock.validate().is_err());
        let mut crossed = r.clone();
        crossed.term.term_digest = DefinitionDigest::from_bytes([1; 32]);
        check!(crossed.matches(&r.term).is_err());
        check_ne!(crossed.root()?.anchor, r.root()?.anchor);
        check_eq!(crossed.root()?.identity, r.root()?.identity);
        let wire = serde_json::to_string(&r)?;
        check!(
            serde_json::from_str::<LapseOccurrence>(&wire.replacen(
                "\"format\":1",
                "\"format\":1,\"format\":1",
                1
            ))
            .is_err()
        );
        Ok(())
    }
}
