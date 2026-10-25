//! Immutable original-owner/personal-paid execution material; no holding/claim authority.
//!
//! The only external input is a Roll-owned projection from real typed records.
//! The closed reader must re-resolve that projection at checkout, and IC custody must commit its
//! exact claim. Source never enters a public listing or serialized projection.
//! Saved drafts continue through `draft_holdings` unchanged.

use mush_core::DefinitionDigest;
use mush_roll::commerce::personal_filed_reader::PersonalFiledHolding;
use mush_roll::commerce::rating::RatingClass;
use mush_roll::{Revision, RevisionAdapter as _, SourceAdapter};
use smol_str::SmolStr;
use std::sync::Arc;

use crate::arena::core::{ClassTable, CoreClass};
use crate::arena::profile_seal::{FiledCertificate, MindIdentity, MindOrigin, SealedCards};
use crate::arena::rules::RuleWeapon;
use crate::arena::runtime_card::{CardDefinition, ProfileDefinition};
use crate::arena::schema::{SchemaError, validate_envelope};
use crate::arena::state::Channel;

/// Server-private execution material. Unlike HeldMind, it carries a certified
/// minimum separately from the immutable declaration and cannot silently enter
/// the old `definition.declared_class == terms.rated` seal path.
#[derive(Clone)]
pub(crate) struct FiledExecutable {
    holding: PersonalFiledHolding,
    definition: Arc<CardDefinition>,
    source: Arc<str>,
    declared: CoreClass,
    minimum: CoreClass,
}
impl std::fmt::Debug for FiledExecutable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FiledExecutable")
            .field("filed", &self.holding.filed())
            .field("declared", &self.declared)
            .field("minimum", &self.minimum)
            .finish_non_exhaustive()
    }
}
impl FiledExecutable {
    pub(crate) fn resolve<S: SourceAdapter>(
        holding: PersonalFiledHolding,
        sources: &S,
    ) -> Result<Self, SchemaError> {
        let declared = class(holding.declared());
        let minimum = class(holding.minimum());
        if holding
            .paid_terms()
            .is_some_and(|terms| class(terms.class()) < minimum)
        {
            return Err(invalid(
                "class",
                "paid licence is below the certified minimum",
            ));
        }
        let (definition, source) = decode_holding(&holding, sources)?;
        Ok(Self {
            holding,
            definition: Arc::new(definition),
            source,
            declared,
            minimum,
        })
    }
    pub(crate) fn holding(&self) -> &PersonalFiledHolding {
        &self.holding
    }
    pub(crate) fn definition(&self) -> &CardDefinition {
        &self.definition
    }

    /// A point-in-time selection, obtained only from the opaque Roll projection.
    /// This does not grant a new IC seat claim or retain current owner authority;
    /// actual checkout must resolve the selected licence through Roll again.
    pub(crate) fn into_held(self) -> super::holdings::HeldMind {
        let certificate = FiledCertificate {
            revision_character: self.holding.identity().revision().key.character,
            revision_mind: self.holding.identity().revision().key.mind,
            revision_number: self.holding.identity().revision().key.number,
            revision_digest: self.holding.identity().revision().digest,
            source_digest: self.holding.identity().source(),
            rating_job: self.holding.rating().job(),
            rating_id: self.holding.rating().id().digest(),
            rating_schema: self.holding.rating().schema(),
            rating_digest: self.holding.rating().receipt_digest(),
            compatibility: self.holding.compatibility().digest(),
            definition: self.holding.definition(),
            declared: self.declared,
            minimum: self.minimum,
        };
        super::holdings::HeldMind {
            presets: self
                .definition
                .profiles
                .iter()
                .map(|profile| profile.name.clone())
                .collect(),
            definition: Some(self.definition),
            saved: None,
            identity: MindIdentity {
                mind: self.holding.name().into(),
                origin: MindOrigin::Filed {
                    charter: self.holding.charter().get().to_string().into(),
                    line: self.holding.line().get().to_string().into(),
                    revision: self.holding.sequence(),
                    licence: self.holding.licence().get().to_string().into(),
                    certificate: Box::new(certificate),
                },
                source_sha256: *self.holding.identity().source().as_bytes(),
            },
            rated: self.minimum,
            engine: crate::arena::mind::SealEngine::Scripted,
            source: Some(self.source),
        }
    }

    /// Internal runner installation only. This is not an authoring transport
    /// ACL: source display requires a separate current form/purpose check.
    /// A Sealed paid holding never grants that permission.
    pub(crate) fn source_for_execution(&self) -> Arc<str> {
        Arc::clone(&self.source)
    }

    /// Validate exact full profiles without changing source, declared schema
    /// limits or card digest. Missing/Class0 hardware selects the independent
    /// captured ROM outside this function; it never reinterprets these cards.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare_profiles(
        &self,
        hardware: CoreClass,
        cap: CoreClass,
        classes: &ClassTable,
        names: &[SmolStr],
        weapons: &[RuleWeapon],
        channels: &[Channel],
        floor: u32,
    ) -> Result<PreparedOriginalProfiles, SchemaError> {
        let Some(execution) = execution_class(self.minimum, hardware, cap)? else {
            return Ok(PreparedOriginalProfiles::RomRequired);
        };
        let cards = SealedCards::prepare(
            (*self.definition).clone(),
            names,
            &classes.row(execution),
            weapons,
            channels,
            floor,
        )?;
        Ok(PreparedOriginalProfiles::Scripted {
            class: execution,
            cards,
        })
    }
}

#[derive(Debug)]
pub(crate) enum PreparedOriginalProfiles {
    RomRequired,
    Scripted {
        class: CoreClass,
        cards: SealedCards,
    },
}

fn class(value: RatingClass) -> CoreClass {
    match value {
        RatingClass::I => CoreClass::I,
        RatingClass::II => CoreClass::II,
        RatingClass::III => CoreClass::III,
    }
}
fn execution_class(
    minimum: CoreClass,
    hardware: CoreClass,
    cap: CoreClass,
) -> Result<Option<CoreClass>, SchemaError> {
    if minimum == CoreClass::Zero {
        return Err(invalid("rating", "invalid certified class"));
    }
    if hardware == CoreClass::Zero || cap == CoreClass::Zero {
        return Ok(None);
    }
    if hardware < minimum || cap < minimum {
        return Err(invalid(
            "class",
            "fitted core or selected cap is below the certified minimum",
        ));
    }
    Ok(Some(minimum))
}

/// This pure decoder is deliberately not a constructor for a holding or seal.
/// A rating and immutable source must already have been resolved by their owners.
pub(crate) fn decode_source_access<S: SourceAdapter>(
    access: &mush_roll::commerce::paid_source_reader::PaidSourceAccess,
    sources: &S,
) -> Result<Arc<str>, SchemaError> {
    decode_archival_source(
        access.revision(),
        access.definition(),
        class(access.declared()),
        class(access.minimum()),
        sources,
    )
}

pub(crate) fn decode_dependent_source_access<S: SourceAdapter>(
    access: &mush_roll::commerce::dependent_source::DependentSourceAccess,
    sources: &S,
) -> Result<Arc<str>, SchemaError> {
    decode_archival_source(
        access.revision(),
        access.definition(),
        class(access.declared()),
        class(access.minimum()),
        sources,
    )
}

/// Source display has its own opaque access proofs above. Archival validity
/// does not require that an old program compile against today's game API, and
/// returning text does not produce an executable holding, claim or seal.
fn decode_archival_source<S: SourceAdapter>(
    revision: &Revision,
    expected_definition: DefinitionDigest,
    declared: CoreClass,
    minimum: CoreClass,
    sources: &S,
) -> Result<Arc<str>, SchemaError> {
    if revision.envelope().mind_abi == crate::minds::MIND_ABI {
        return decode_exact(revision, expected_definition, declared, minimum, sources)
            .map(|(_, source)| source);
    }
    let (_, source) = decode_material(revision, expected_definition, declared, minimum, sources)?;
    crate::minds::authoring::RetainedRevisionRules
        .validate(&source, revision.envelope())
        .map_err(|_| invalid("mind_abi", "retained source archive is not valid"))?;
    Ok(source)
}

/// A retained ABI is executable only behind the opaque owner observation.
/// Both owner and paid projections resolve the complete current certification
/// and its permanent roots before constructing this non-Deserialize value.
/// Checkout re-resolves it, and IC Open rechecks it in its actual owner union.
fn decode_holding<S: SourceAdapter>(
    holding: &PersonalFiledHolding,
    sources: &S,
) -> Result<(CardDefinition, Arc<str>), SchemaError> {
    let revision = holding.revision();
    if !holding
        .identity()
        .matches(revision)
        .map_err(|_| invalid("rating", "holding revision identity is invalid"))?
    {
        return Err(invalid(
            "rating",
            "holding names another immutable revision",
        ));
    }
    let (definition, source) = decode_material(
        revision,
        holding.definition(),
        class(holding.declared()),
        class(holding.minimum()),
        sources,
    )?;
    if revision.envelope().mind_abi != crate::minds::MIND_ABI {
        // A current certificate does not authorize an arbitrary historical ABI.
        // The known retained API4 metadata must match its immutable envelope and
        // the unchanged source must compile in the current bounded sandbox.
        crate::minds::authoring::validate_retained_registry(&source, revision.envelope())
            .map_err(|_| invalid("mind_abi", "retained executable is not compatible"))?;
    }
    Ok((definition, source))
}

/// Raw decoder callers receive no retained execution authority merely by
/// supplying a revision and digest. This current-only decoder grants no rights.
fn decode_exact<S: SourceAdapter>(
    revision: &Revision,
    expected_definition: DefinitionDigest,
    declared: CoreClass,
    minimum: CoreClass,
    sources: &S,
) -> Result<(CardDefinition, Arc<str>), SchemaError> {
    if revision.envelope().mind_abi != crate::minds::MIND_ABI {
        return Err(invalid(
            "mind_abi",
            "filed revision has another executable ABI",
        ));
    }
    decode_material(revision, expected_definition, declared, minimum, sources)
}

/// Pure byte/definition reconstruction; callers above retain their distinct
/// authority boundaries. Never returns a holding, claim or seal.
fn decode_material<S: SourceAdapter>(
    revision: &Revision,
    expected_definition: DefinitionDigest,
    declared: CoreClass,
    minimum: CoreClass,
    sources: &S,
) -> Result<(CardDefinition, Arc<str>), SchemaError> {
    revision
        .verify_source(sources)
        .map_err(|_| invalid("source", "immutable source is unavailable"))?;
    let envelope = revision.envelope();
    if declared == CoreClass::Zero
        || minimum < declared
        || CoreClass::parse(&envelope.declared_class) != Some(declared)
    {
        return Err(invalid(
            "rating",
            "certificate differs from the immutable declaration",
        ));
    }
    let decoded = validate_envelope(
        &envelope.schema,
        &envelope.presets,
        &envelope.profiles,
        &envelope.rules,
        declared,
    )?;
    let definition = CardDefinition {
        schema: decoded.schema,
        declared_class: declared,
        profiles: decoded
            .profiles
            .into_iter()
            .map(|(name, values)| ProfileDefinition {
                rules: decoded.rules.get(&name).cloned().unwrap_or_default(),
                name,
                values,
            })
            .collect(),
    };
    if definition.validate()? != expected_definition {
        return Err(invalid(
            "rating",
            "certificate names another complete card program",
        ));
    }
    let source = sources
        .read_source(revision.source())
        .map_err(|_| invalid("source", "immutable source is unavailable"))?;
    Ok((definition, Arc::from(source)))
}
fn invalid(field: &str, message: &str) -> SchemaError {
    SchemaError {
        field: field.into(),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_roll::{Envelope, RevisionAdapter as _, RollError, source_digest};
    use serde_json::json;

    struct Source {
        digest: DefinitionDigest,
        text: String,
    }
    impl SourceAdapter for Source {
        type Token = ();
        fn prepare_source(
            &self,
            _: &str,
            _: &str,
        ) -> Result<mush_roll::PreparedSource<Self::Token>, RollError> {
            Err(RollError::Missing)
        }
        fn read_source(&self, digest: DefinitionDigest) -> Result<&str, RollError> {
            if digest == self.digest {
                Ok(&self.text)
            } else {
                Err(RollError::Missing)
            }
        }
    }
    fn fixture() -> (Revision, Source, CardDefinition) {
        let text = "fn think(view, card) {}".to_string();
        let source = Source {
            digest: source_digest(&text),
            text,
        };
        let envelope = Envelope {
            schema: json!({"sections":[]}),
            presets: json!({}),
            profiles: json!({"Default":{}}),
            rules: json!({"Default":[]}),
            declared_class: "I".into(),
            mind_abi: crate::minds::MIND_ABI.into(),
        };
        let reference = envelope.digest(&source.text).unwrap();
        let revision = serde_json::from_value(json!({
            "format":1,"reference":{"key":{"character":7,"mind":1,"number":1},"digest":reference},
            "source":source.digest,"envelope":envelope,"notes":"",
        }))
        .unwrap();
        let definition = CardDefinition {
            schema: crate::arena::schema::CardSchema { sections: vec![] },
            declared_class: CoreClass::I,
            profiles: vec![ProfileDefinition {
                name: "Default".into(),
                values: Default::default(),
                rules: vec![],
            }],
        };
        (revision, source, definition)
    }
    #[test]
    fn certification_above_declaration_keeps_exact_source_schema_and_card_identity() {
        let (revision, source, definition) = fixture();
        let digest = definition.validate().unwrap();
        let (decoded, executable) =
            decode_exact(&revision, digest, CoreClass::I, CoreClass::II, &source).unwrap();
        assert_eq!(decoded, definition);
        assert_eq!(decoded.validate().unwrap(), digest);
        assert_eq!(executable.as_ref(), source.text);
        assert_eq!(
            execution_class(CoreClass::II, CoreClass::III, CoreClass::III).unwrap(),
            Some(CoreClass::II)
        );
        let cards = SealedCards::prepare(
            decoded,
            &[],
            &ClassTable::STOCK.row(CoreClass::II),
            &[],
            &[Channel::Helm, Channel::Sensors],
            0,
        )
        .unwrap();
        assert_eq!(cards.definition.declared_class, CoreClass::I);
        assert_eq!(cards.definition.validate().unwrap(), digest);
    }
    #[test]
    fn ordinary_class_caps_never_undercut_certification_and_rom_is_explicit() {
        for (hardware, cap) in [
            (CoreClass::I, CoreClass::III),
            (CoreClass::III, CoreClass::I),
        ] {
            assert!(execution_class(CoreClass::II, hardware, cap).is_err());
        }
        assert_eq!(
            execution_class(CoreClass::II, CoreClass::Zero, CoreClass::III).unwrap(),
            None
        );
        assert_eq!(
            execution_class(CoreClass::II, CoreClass::III, CoreClass::Zero).unwrap(),
            None
        );
        assert!(execution_class(CoreClass::Zero, CoreClass::III, CoreClass::III).is_err());
    }
    #[test]
    fn tampered_source_declaration_and_definition_fail_before_runner_installation() {
        let (revision, mut source, definition) = fixture();
        let digest = definition.validate().unwrap();
        assert!(decode_exact(&revision, digest, CoreClass::II, CoreClass::II, &source).is_err());
        assert!(decode_exact(&revision, digest, CoreClass::I, CoreClass::Zero, &source).is_err());
        assert!(
            decode_exact(
                &revision,
                DefinitionDigest::from_bytes([9; 32]),
                CoreClass::I,
                CoreClass::I,
                &source
            )
            .is_err()
        );
        source.text.push(' ');
        assert!(decode_exact(&revision, digest, CoreClass::I, CoreClass::I, &source).is_err());
    }

    #[test]
    fn raw_retained_revision_is_not_current_execution_authority() {
        let (revision, mut source, definition) = fixture();
        let mut value = serde_json::to_value(&revision).unwrap();
        value["envelope"]["mind_abi"] = json!("rhai=1.26.0;api=4;stock=2");
        let envelope: Envelope = serde_json::from_value(value["envelope"].clone()).unwrap();
        value["reference"]["digest"] =
            serde_json::to_value(envelope.digest(&source.text).unwrap()).unwrap();
        let retained: Revision = serde_json::from_value(value).unwrap();
        assert_ne!(retained.envelope().mind_abi, crate::minds::MIND_ABI);
        let digest = definition.validate().unwrap();
        assert!(decode_exact(&retained, digest, CoreClass::I, CoreClass::I, &source).is_err());
        // Reconstruction alone grants no rights. It preserves every immutable
        // byte; only decode_holding has the real current owner observation.
        let before = serde_json::to_vec(&retained).unwrap();
        let (actual, text) =
            decode_material(&retained, digest, CoreClass::I, CoreClass::II, &source).unwrap();
        assert_eq!(actual, definition);
        assert_eq!(text.as_ref(), source.text);
        assert_eq!(serde_json::to_vec(&retained).unwrap(), before);
        assert!(
            decode_material(
                &retained,
                DefinitionDigest::from_bytes([9; 32]),
                CoreClass::I,
                CoreClass::II,
                &source
            )
            .is_err()
        );
        assert!(decode_material(&retained, digest, CoreClass::II, CoreClass::II, &source).is_err());
        source.text.push(' ');
        assert!(decode_material(&retained, digest, CoreClass::I, CoreClass::II, &source).is_err());
    }

    #[test]
    fn retained_source_display_preserves_archive_without_execution_authority() {
        const OLD_ABI: &str = "rhai=1.26.0;api=4;stock=2";
        const SOURCE: &str = "const MIND = #{name: \"Retained\", api: 4, class: \"discretion\", role: \"combat\", core: \"I\"}; fn think(v,c) { this.marker = 73; }";
        let (base, _, definition) = fixture();
        let digest = definition.validate().unwrap();
        let retained = |text: &str, abi: &str| {
            let source = Source {
                digest: source_digest(text),
                text: text.into(),
            };
            let mut value = serde_json::to_value(&base).unwrap();
            value["envelope"]["mind_abi"] = json!(abi);
            let envelope: Envelope = serde_json::from_value(value["envelope"].clone()).unwrap();
            value["source"] = serde_json::to_value(source.digest).unwrap();
            value["reference"]["digest"] =
                serde_json::to_value(envelope.digest(text).unwrap()).unwrap();
            let revision: Revision = serde_json::from_value(value).unwrap();
            (revision, source)
        };
        let (revision, mut source) = retained(SOURCE, OLD_ABI);
        let before = serde_json::to_vec(&revision).unwrap();
        // This tests the private reconstruction boundary, not an access grant.
        // The public-to-crate wrappers require owner-created opaque ACL results.
        let text = decode_archival_source(&revision, digest, CoreClass::I, CoreClass::II, &source)
            .unwrap();
        assert_eq!(text.as_ref(), SOURCE);
        assert_eq!(serde_json::to_vec(&revision).unwrap(), before);
        assert!(decode_exact(&revision, digest, CoreClass::I, CoreClass::II, &source).is_err());
        assert!(
            crate::minds::authoring::RevisionRules
                .validate(SOURCE, revision.envelope())
                .is_err()
        );
        for (text, abi) in [
            (SOURCE.to_owned(), "rhai=1.26.0;api=3;stock=2"),
            (SOURCE.replace("api: 4", "api: 3"), OLD_ABI),
            (SOURCE.replace("fn think(v,c)", "fn think(v)"), OLD_ABI),
        ] {
            let (invalid, source) = retained(&text, abi);
            assert!(
                decode_archival_source(&invalid, digest, CoreClass::I, CoreClass::II, &source)
                    .is_err()
            );
        }
        assert!(
            decode_archival_source(
                &revision,
                DefinitionDigest::from_bytes([9; 32]),
                CoreClass::I,
                CoreClass::II,
                &source,
            )
            .is_err()
        );
        assert!(
            decode_archival_source(&revision, digest, CoreClass::II, CoreClass::II, &source)
                .is_err()
        );
        source.text.push(' ');
        assert!(
            decode_archival_source(&revision, digest, CoreClass::I, CoreClass::II, &source)
                .is_err()
        );
        let (current, source, _) = fixture();
        assert_eq!(
            decode_archival_source(&current, digest, CoreClass::I, CoreClass::II, &source)
                .unwrap()
                .as_ref(),
            source.text,
        );
    }
}
