//! Retained attachment evidence, not a capability or a fact result.
use super::*;
use serde::{Deserialize, Deserializer, Serialize, de};

const MAX_MARKER_BYTES: usize = 16_384;

/// A bounded historical authorization binding. Public decoding is data only:
/// every new attachment re-authorizes and replaces it before publication.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Wire")]
pub struct FactConsentV1 {
    format: u16,
    contract: crate::ContractId,
    generation: u32,
    authorized_by: ObjId,
    surface_digest: [u8; 32],
    references: Vec<ExternalRef>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    contract: crate::ContractId,
    generation: u32,
    authorized_by: ObjId,
    surface_digest: [u8; 32],
    #[serde(deserialize_with = "bounded_refs")]
    references: Vec<ExternalRef>,
}
fn bounded_refs<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<ExternalRef>, D::Error> {
    struct Refs;
    impl<'de> de::Visitor<'de> for Refs {
        type Value = Vec<ExternalRef>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a bounded canonical list of fact references")
        }
        fn visit_seq<A: de::SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
            if a.size_hint()
                .is_some_and(|n| n > crate::contract::MAX_CONDITIONS)
            {
                return Err(de::Error::custom("too many fact references"));
            }
            let mut refs = Vec::new();
            while let Some(r) = a.next_element::<ExternalRef>()? {
                if refs.len() == crate::contract::MAX_CONDITIONS || !bounded(&r) {
                    return Err(de::Error::custom("invalid fact reference"));
                }
                refs.push(r);
            }
            Ok(refs)
        }
    }
    d.deserialize_seq(Refs)
}
fn bounded(r: &ExternalRef) -> bool {
    !r.provider.is_empty()
        && r.provider.len() <= 64
        && !r.kind.is_empty()
        && r.kind.len() <= 128
        && !r.id.is_empty()
        && r.id.len() <= 512
}
impl TryFrom<Wire> for FactConsentV1 {
    type Error = &'static str;
    fn try_from(w: Wire) -> Result<Self, Self::Error> {
        let v = Self {
            format: w.format,
            contract: w.contract,
            generation: w.generation,
            authorized_by: w.authorized_by,
            surface_digest: w.surface_digest,
            references: w.references,
        };
        v.validate().then_some(v).ok_or("invalid fact consent")
    }
}
impl FactConsentV1 {
    fn validate(&self) -> bool {
        self.format == 1
            && self.contract.0 != u64::MAX
            && self.generation <= crate::contract::MAX_AMENDMENTS
            && !self.references.is_empty()
            && self.references.len() <= crate::contract::MAX_CONDITIONS
            && self.references.iter().all(bounded)
            && self.references.windows(2).all(|p| p.first() < p.get(1))
            && serde_json::to_vec(self).is_ok_and(|b| b.len() <= MAX_MARKER_BYTES)
    }
    pub(super) fn matches(&self, c: &EconomicContract, fact: &ExternalRef) -> bool {
        self.validate()
            && self.contract == c.id
            && self.generation == c.amendments
            && self.surface_digest == surface_digest(c)
            && self.references.contains(fact)
    }
    pub(super) const fn actor(&self) -> ObjId {
        self.authorized_by
    }
}
fn surface_digest(c: &EconomicContract) -> [u8; 32] {
    let mut surface = c.consent.clone();
    surface.fact_consent = None;
    crate::consent_hash(&c.issuer, &surface)
}

/// Synchronous preflight retained across the amendment's own refund. It has no
/// decoder/constructor outside this owner and cannot authorize another surface.
pub(crate) struct FactAttachment {
    contract: crate::ContractId,
    generation: u32,
    surface_digest: [u8; 32],
    marker: Option<FactConsentV1>,
}
impl FactAttachment {
    pub(crate) fn apply(&self, c: &mut EconomicContract) -> Result<(), crate::ContractError> {
        if c.id != self.contract
            || c.amendments != self.generation
            || surface_digest(c) != self.surface_digest
        {
            return Err(crate::ContractError::NoStanding);
        }
        c.consent.fact_consent = self.marker.clone();
        c.consent_hash = crate::consent_hash(&c.issuer, &c.consent);
        Ok(())
    }
    pub(crate) fn matches(&self, c: &EconomicContract) -> bool {
        self.contract == c.id
            && self.generation == c.amendments
            && self.surface_digest == surface_digest(c)
            && self.marker == c.consent.fact_consent
    }
}

pub(crate) fn prepare_attachment(
    ctx: &Ctx<'_>,
    c: &EconomicContract,
) -> Result<FactAttachment, crate::ContractError> {
    let providers = ctx
        .w
        .module_state::<crate::EconomyStore>()
        .ok_or(crate::ContractError::NoStanding)?
        .facts();
    let mut refs = Vec::new();
    for condition in &c.consent.conditions {
        if let crate::TypedCondition::Fact { fact } = condition
            && providers
                .iter()
                .any(|p| p.provider() == fact.provider.as_str() && p.requires_scope(fact))
        {
            if !bounded(fact) || !permitted(ctx, c, fact, FactPurpose::Attach) {
                return Err(crate::ContractError::NoStanding);
            }
            refs.push(fact.clone());
        }
    }
    refs.sort();
    refs.dedup();
    let digest = surface_digest(c);
    let marker = if refs.is_empty() {
        None
    } else {
        let marker = FactConsentV1 {
            format: 1,
            contract: c.id,
            generation: c.amendments,
            authorized_by: ctx
                .dispatch_actor()
                .ok_or(crate::ContractError::NoStanding)?,
            surface_digest: digest,
            references: refs,
        };
        if !marker.validate() {
            return Err(crate::ContractError::NoStanding);
        }
        Some(marker)
    };
    Ok(FactAttachment {
        contract: c.id,
        generation: c.amendments,
        surface_digest: digest,
        marker,
    })
}

pub(crate) fn stamp_attachment(
    ctx: &Ctx<'_>,
    c: &mut EconomicContract,
) -> Result<(), crate::ContractError> {
    prepare_attachment(ctx, c)?.apply(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn marker() -> FactConsentV1 {
        FactConsentV1 {
            format: 1,
            contract: crate::ContractId(0),
            generation: 0,
            authorized_by: ObjId(5),
            surface_digest: [9; 32],
            references: vec![ExternalRef {
                provider: "minds".into(),
                kind: "term".into(),
                id: "7/1".into(),
                revision: Some(1),
            }],
        }
    }
    #[test]
    fn fact_marker_codec_is_closed_bounded_and_canonical() {
        let actual = marker();
        let value = serde_json::to_value(&actual).expect("marker");
        assert_eq!(
            serde_json::from_value::<FactConsentV1>(value.clone()).expect("roundtrip"),
            actual
        );
        for (field, replacement) in [
            ("format", serde_json::json!(2)),
            ("contract", serde_json::json!(u64::MAX)),
            ("generation", serde_json::json!(33)),
            ("references", serde_json::json!([])),
            ("extra", serde_json::json!(true)),
        ] {
            let mut bad = value.clone();
            bad.as_object_mut()
                .expect("object")
                .insert(field.into(), replacement);
            assert!(serde_json::from_value::<FactConsentV1>(bad).is_err());
        }
        let mut missing = value.clone();
        missing
            .as_object_mut()
            .expect("object")
            .remove("authorized_by");
        assert!(serde_json::from_value::<FactConsentV1>(missing).is_err());
        let mut duplicate = marker();
        duplicate
            .references
            .push(duplicate.references.first().expect("ref").clone());
        assert!(
            serde_json::from_value::<FactConsentV1>(
                serde_json::to_value(duplicate).expect("codec")
            )
            .is_err()
        );
        let mut wide = marker();
        wide.references.first_mut().expect("ref").id = "x".repeat(513).into();
        assert!(
            serde_json::from_value::<FactConsentV1>(serde_json::to_value(wide).expect("codec"))
                .is_err()
        );
        let refs = vec![
            marker().references.first().expect("ref").clone();
            crate::contract::MAX_CONDITIONS + 1
        ];
        let mut overflow = value;
        overflow.as_object_mut().expect("object").insert(
            "references".into(),
            serde_json::to_value(refs).expect("codec"),
        );
        assert!(serde_json::from_value::<FactConsentV1>(overflow).is_err());
    }
}
