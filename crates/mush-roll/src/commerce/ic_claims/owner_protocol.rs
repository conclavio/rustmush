//! Closed, type-selected Tacsim participant contract. Only actual outputs from
//! PrepareCtx::prepare_owner are consumed by IcWriter. There is no output data
//! constructor, decoder, callback writer or caller-provided currentness flag.

use mush_core::participant::OwnerParticipant;
use mush_core::{ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_sim::ic_exclusion::IcEventRef;
use mush_tactics::UnitMutationPlan;
use smol_str::SmolStr;

use super::IcAdmissionFacts;

/// Data describing a genuine retained event, never an event capability. The
/// fixed owner may expose this only after it consumes its opaque host event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IcCloseKind {
    ReturnedToBase { base: ObjId },
    PoweredDownInBase { base: ObjId },
    Destroyed,
    RecoveredToBase { base: ObjId },
}

// Reuse the existing closed historical wire rather than deriving a second
// codec for the public data view. This creates no opaque event authority.
impl serde::Serialize for IcCloseKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use super::records::CloseKind;
        let wire = match *self {
            Self::ReturnedToBase { base } => CloseKind::ReturnedToBase { base },
            Self::PoweredDownInBase { base } => CloseKind::PoweredDownInBase { base },
            Self::Destroyed => CloseKind::Destroyed {},
            Self::RecoveredToBase { base } => CloseKind::RecoveredToBase { base },
        };
        serde::Serialize::serialize(&wire, serializer)
    }
}
impl<'de> serde::Deserialize<'de> for IcCloseKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use super::records::CloseKind;
        Ok(
            match <CloseKind as serde::Deserialize>::deserialize(deserializer)? {
                CloseKind::ReturnedToBase { base } => Self::ReturnedToBase { base },
                CloseKind::PoweredDownInBase { base } => Self::PoweredDownInBase { base },
                CloseKind::Destroyed {} => Self::Destroyed,
                CloseKind::RecoveredToBase { base } => Self::RecoveredToBase { base },
            },
        )
    }
}

/// Implemented by the actual Tacsim opaque owner output. Knowledge of these
/// getters does not construct that output or supply a typed preparation grant.
pub trait IcSealOutput {
    fn action(&self) -> &DefinitionRef;
    fn commit(&self) -> CommitToken;
    fn open(&self) -> CommitToken;
    fn unit(&self) -> ObjId;
    fn sortie(&self) -> u64;
    fn transition(&self) -> u64;
    fn admission_digest(&self) -> DefinitionDigest;
    fn seal_digest(&self) -> DefinitionDigest;
    fn at_ic(&self) -> &ClockStamp;
    fn close_event(&self) -> Option<&IcEventRef>;
    fn close_kind(&self) -> Option<IcCloseKind>;
    fn conditions(&self) -> Option<&UnitMutationPlan>;
    fn closing_unit_digest(&self) -> Option<DefinitionDigest>;
}

/// Trusted concrete adapter selection. The real Tacsim module supplies this
/// implementation, its exact descriptor and actual participant grants. Start
/// and Close are opaque, non-wire host capabilities, not user-selected reasons.
pub trait IcSealParticipant: OwnerParticipant
where
    Self::Output: IcSealOutput,
{
    type Start: Send + Sync + 'static;
    type Close: Send + Sync + 'static;
    fn opening_unit(start: &Self::Start) -> mush_core::ObjId;

    fn opening(
        start: Self::Start,
        facts: IcAdmissionFacts,
        profiles: Vec<SmolStr>,
    ) -> Self::Request;
    fn closing(event: Self::Close) -> Self::Request;
    fn closing_open(event: &Self::Close) -> CommitToken;
    /// The selected concrete owner reads its own actual loaded book. Retained
    /// Roll views are comparison data; they do not create host capabilities.
    fn audit_claims(
        world: &mush_core::World,
        claims: &[super::IcClaimHistory],
        next_transition: u64,
    ) -> Result<(), mush_core::ModError>;
}

#[cfg(test)]
mod close_data_tests {
    use super::IcCloseKind;
    #[test]
    fn public_close_data_uses_exact_existing_closed_history_wire()
    -> Result<(), Box<dyn std::error::Error>> {
        use serde_json::json;
        for (data, wire) in [
            (IcCloseKind::Destroyed, json!({"kind":"destroyed"})),
            (
                IcCloseKind::ReturnedToBase {
                    base: mush_core::ObjId(7),
                },
                json!({"kind":"returned_to_base","base":7}),
            ),
            (
                IcCloseKind::PoweredDownInBase {
                    base: mush_core::ObjId(7),
                },
                json!({"kind":"powered_down_in_base","base":7}),
            ),
            (
                IcCloseKind::RecoveredToBase {
                    base: mush_core::ObjId(7),
                },
                json!({"kind":"recovered_to_base","base":7}),
            ),
        ] {
            assert_eq!(serde_json::to_value(data)?, wire);
            assert_eq!(serde_json::from_value::<IcCloseKind>(wire.clone())?, data);
            let mut extra = wire;
            extra["authority"] = json!(true);
            assert!(serde_json::from_value::<IcCloseKind>(extra).is_err());
        }
        assert!(serde_json::from_value::<IcCloseKind>(json!({"kind":"expired"})).is_err());
        Ok(())
    }
}
