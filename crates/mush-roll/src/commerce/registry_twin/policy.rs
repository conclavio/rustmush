//! Trusted composition identifies a real Registry authority; no actor is created.
use super::Review;
use crate::commerce::records::PublicationError;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::{DefinitionRef, ModError, ObjId};
use mush_org::{OrgId, OrgScopeId, SystemPrincipalId};
use serde::de::{DeserializeSeed, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, de};
use serde_json::json;
use std::marker::PhantomData;

fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}

/// Exact optional Registry scope and operators. Decoding this value is data,
/// never a grant; the live Org authorizer remains mandatory on every invocation.
///
/// One type for both sanctions: `RegistryRecallPolicy` is `RegistryPolicy<ExactRecall>` and
/// `RegistryRevocationPolicy` is `RegistryPolicy<TermRevocation>`. The type parameter is not
/// encoded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "PolicyWire")]
pub struct RegistryPolicy<K: Review> {
    pub(crate) format: u16,
    pub(crate) issuer: SystemPrincipalId,
    pub(crate) organization: OrgId,
    pub(crate) scope: OrgScopeId,
    pub(crate) operators: Vec<ObjId>,
    #[serde(skip)]
    review: PhantomData<K>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyWire {
    format: u16,
    issuer: SystemPrincipalId,
    organization: OrgId,
    scope: OrgScopeId,
    #[serde(deserialize_with = "operators")]
    operators: Vec<ObjId>,
}
fn operators<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ObjId>, D::Error> {
    bounded::<D, ObjId, 32>(d)
}
pub(crate) fn bounded<'de, D: Deserializer<'de>, T: Deserialize<'de>, const N: usize>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    struct Rows<T, const N: usize>(PhantomData<T>);
    struct Overflow;
    impl<'de> DeserializeSeed<'de> for Overflow {
        type Value = ();
        fn deserialize<D: Deserializer<'de>>(self, _: D) -> Result<(), D::Error> {
            Err(de::Error::custom("row bound"))
        }
    }
    impl<'de, T: Deserialize<'de>, const N: usize> Visitor<'de> for Rows<T, N> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "at most {N} rows")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            if seq.size_hint().is_some_and(|n| n > N) {
                return Err(de::Error::custom("row bound"));
            }
            let mut rows = Vec::with_capacity(seq.size_hint().unwrap_or(0).min(N));
            while rows.len() < N {
                match seq.next_element()? {
                    Some(row) => rows.push(row),
                    None => return Ok(rows),
                }
            }
            // Refuse the next element before decoding its payload.
            let _ = seq.next_element_seed(Overflow)?;
            Ok(rows)
        }
    }
    d.deserialize_seq(Rows::<T, N>(PhantomData))
}
impl<K: Review> TryFrom<PolicyWire> for RegistryPolicy<K> {
    type Error = PublicationError;
    fn try_from(w: PolicyWire) -> Result<Self, Self::Error> {
        let p = Self {
            format: w.format,
            issuer: w.issuer,
            organization: w.organization,
            scope: w.scope,
            operators: w.operators,
            review: PhantomData,
        };
        p.validate()?;
        Ok(p)
    }
}
impl<K: Review> RegistryPolicy<K> {
    /// Construct one explicit scope. Operators must already be sorted and unique.
    pub fn new(
        issuer: SystemPrincipalId,
        organization: OrgId,
        scope: OrgScopeId,
        operators: Vec<ObjId>,
    ) -> Result<Self, ModError> {
        let p = Self {
            format: 1,
            issuer,
            organization,
            scope,
            operators,
            review: PhantomData,
        };
        p.validate().map_err(error)?;
        Ok(p)
    }
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1
            || self.operators.is_empty()
            || self.operators.len() > 32
            || self
                .operators
                .windows(2)
                .any(|pair| matches!(pair, [a, b] if a >= b))
        {
            return Err(PublicationError::Invalid);
        }
        Ok(())
    }
    /// Exact action identity, independent of the current execution manifest.
    pub fn action(&self) -> Result<DefinitionRef, ModError> {
        let [_, action, _] = self.definitions()?;
        DefinitionRegistryBuilder::new()
            .register(owner()?, action)
            .map_err(error)
    }
    pub(crate) fn definitions(&self) -> Result<[DefinitionDescriptor; 3], ModError> {
        self.validate().map_err(error)?;
        let authority = descriptor(
            "registry_authority",
            K::AUTHORITY,
            serde_json::to_value(self).map_err(error)?,
        )?;
        let authority_ref = DefinitionRegistryBuilder::new()
            .register(owner()?, authority.clone())
            .map_err(error)?;
        let retention = super::super::authored_retirement::retention_reference()?;
        let action = descriptor("action", K::ACTION, K::action_value())?
            .with_references(vec![authority_ref, retention]);
        let outcome = descriptor("outcome", K::OUTCOME, json!({"format":1}))?;
        Ok([authority, action, outcome])
    }
}
fn owner() -> Result<OwnerId, ModError> {
    OwnerId::new(crate::NS).map_err(error)
}
fn descriptor(
    kind: &str,
    id: &str,
    value: serde_json::Value,
) -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new(kind).map_err(error)?,
        DefinitionId::new(id).map_err(error)?,
        1,
        value,
    )
    .map_err(error)
}
