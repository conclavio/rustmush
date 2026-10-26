//! Exact source and full-program expansion on the preparation worker. This
//! accepts an already-authorized immutable owner checkout, never a live World.
//! The caller must retain its root until final worker destruction/publication.

use std::{collections::BTreeMap, sync::Arc};

use mush_core::DefinitionDigest;
use mush_mind::{Digest, EngineLimits, Sandbox};
use mush_roll::{
    Revision, RevisionAdapter,
    commerce::{
        RevisionIdentity,
        rating::{AliasKind, RatingClass, RatingError, RatingSubject, VariantAlias},
    },
};
use serde::Serialize;

use crate::{
    arena::{
        core::CoreClass,
        runtime_card::{CardDefinition, ProfileDefinition},
        schema::validate_envelope,
    },
    config::ArenaConfig,
    minds::{RhaiRunner, authoring::RevisionRules},
};

use super::identity;

const MAX_EXPANSION_BYTES: usize = 4 * 1024 * 1024;
pub(super) const MAX_VARIANTS: usize = 12;
pub(super) const MAX_PAIRS: usize = 36;

/// A complete immutable definition, including every saved program. The recipe
/// is part of identity: an opening-profile trial and a cycling preset trial do
/// not deduplicate merely because their first cards happen to be equal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Variant {
    pub(super) definition: CardDefinition,
    pub(super) initial: u8,
    pub(super) cycle: bool,
}

/// A direct probe targets one actual saved program with one complete card.
/// It retains the full definition so profile() inside the source has exactly
/// the same available names/programs as its corresponding whole-match trial.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct Pair {
    pub(super) variant: u8,
    pub(super) profile: u8,
}

#[derive(Debug)]
pub(super) struct PreparedSubject {
    subject: RatingSubject,
    variants: Vec<Variant>,
    pairs: Vec<Pair>,
    aliases: Vec<VariantAlias>,
    runner: Arc<RhaiRunner>,
}

impl PreparedSubject {
    /// No normalization or mutable alias resolution occurs here. Both immutable
    /// digests must match the exact source bytes supplied by the source owner.
    pub(super) fn prepare(
        revision: &Revision,
        source: &str,
        arena: &ArenaConfig,
    ) -> Result<Self, RatingError> {
        Self::prepare_inner(revision, source, arena, false)
    }
    pub(super) fn prepare_reconformance(
        revision: &Revision,
        source: &str,
        arena: &ArenaConfig,
    ) -> Result<Self, RatingError> {
        Self::prepare_inner(revision, source, arena, true)
    }
    fn prepare_inner(
        revision: &Revision,
        source: &str,
        arena: &ArenaConfig,
        reconformance: bool,
    ) -> Result<Self, RatingError> {
        let revision_identity =
            RevisionIdentity::capture(revision).map_err(|_| RatingError::Invalid)?;
        let envelope = revision.envelope();
        if mush_roll::source_digest(source) != revision.source()
            || envelope.digest(source).map_err(|_| RatingError::Invalid)?
                != revision.reference().digest
        {
            return Err(RatingError::Identity);
        }
        if reconformance {
            crate::minds::authoring::validate_retained_registry(source, envelope)
                .map_err(|_| RatingError::Invalid)?;
        } else {
            RevisionRules
                .validate(source, envelope)
                .map_err(|_| RatingError::Invalid)?;
        }
        let class = CoreClass::parse(&envelope.declared_class).ok_or(RatingError::Invalid)?;
        let declared = rating_class(class)?;
        let checked = validate_envelope(
            &envelope.schema,
            &envelope.presets,
            &envelope.profiles,
            &envelope.rules,
            class,
        )
        .map_err(|_| RatingError::Invalid)?;
        let definition = CardDefinition {
            schema: checked.schema,
            declared_class: class,
            profiles: checked
                .profiles
                .into_iter()
                .map(|(name, values)| {
                    let rules = checked.rules.get(&name).cloned().unwrap_or_default();
                    ProfileDefinition {
                        name,
                        values,
                        rules,
                    }
                })
                .collect(),
        };
        let base_digest = definition.validate().map_err(|_| RatingError::Invalid)?;
        if definition.profiles.is_empty()
            || definition.profiles.len() > 4
            || checked.presets.len() > 8
        {
            return Err(RatingError::Bound);
        }
        let mut variants = Vec::new();
        let mut seen = BTreeMap::new();
        let mut aliases = Vec::new();
        let mut pairs = Vec::new();
        for (index, profile) in definition.profiles.iter().enumerate() {
            let index = u8::try_from(index).map_err(|_| RatingError::Bound)?;
            let variant = admit(
                &mut variants,
                &mut seen,
                Variant {
                    definition: definition.clone(),
                    initial: index,
                    cycle: false,
                },
            )?;
            aliases.push(VariantAlias::new(
                AliasKind::Profile,
                profile.name.to_string(),
                variant,
            )?);
            pairs.push(Pair {
                variant,
                profile: index,
            });
        }
        for (name, values) in checked.presets {
            let mut preset_definition = definition.clone();
            for profile in &mut preset_definition.profiles {
                profile.values.clone_from(&values);
            }
            // Programs are deliberately untouched. A HOLD-only profile and a
            // FIRE profile with the same slider values remain different probes.
            preset_definition
                .validate()
                .map_err(|_| RatingError::Invalid)?;
            let variant = admit(
                &mut variants,
                &mut seen,
                Variant {
                    definition: preset_definition,
                    initial: 0,
                    cycle: true,
                },
            )?;
            aliases.push(VariantAlias::new(
                AliasKind::Preset,
                name.to_string(),
                variant,
            )?);
            for index in 0..definition.profiles.len() {
                pairs.push(Pair {
                    variant,
                    profile: u8::try_from(index).map_err(|_| RatingError::Bound)?,
                });
            }
        }
        if pairs.len() > MAX_PAIRS {
            return Err(RatingError::Bound);
        }
        // Duplicate preset aliases still each name a covered pair. No name is
        // silently dropped; whole trials may share an identical full recipe.
        let expanded = identity(
            "tacsim.registry.expansion.v1",
            &(&variants, &pairs, &aliases),
            MAX_EXPANSION_BYTES,
        )?;
        let envelope_digest = identity(
            "tacsim.registry.envelope.v1",
            envelope,
            mush_roll::MAX_ENVELOPE_BYTES,
        )?;
        let subject = RatingSubject::new(
            revision_identity,
            envelope_digest,
            mush_roll::commerce::records::schema_line_digest(envelope)
                .map_err(|_| RatingError::Invalid)?,
            base_digest,
            expanded,
            declared,
        );
        let sandbox = Arc::new(Sandbox::new(
            &EngineLimits::default(),
            crate::minds::register_api,
        ));
        let runner = Arc::new(
            RhaiRunner::on(
                sandbox,
                arena,
                source,
                Digest(*revision.source().as_bytes()),
            )
            .map_err(|_| RatingError::Invalid)?,
        );
        Ok(Self {
            subject,
            variants,
            pairs,
            aliases,
            runner,
        })
    }

    pub(super) fn subject(&self) -> &RatingSubject {
        &self.subject
    }
    pub(super) fn variants(&self) -> &[Variant] {
        &self.variants
    }
    pub(super) fn pairs(&self) -> &[Pair] {
        &self.pairs
    }
    pub(super) fn aliases(&self) -> &[VariantAlias] {
        &self.aliases
    }
    pub(super) fn runner(&self) -> Arc<RhaiRunner> {
        Arc::clone(&self.runner)
    }
}

fn admit(
    variants: &mut Vec<Variant>,
    seen: &mut BTreeMap<DefinitionDigest, u8>,
    variant: Variant,
) -> Result<u8, RatingError> {
    let digest = identity("tacsim.registry.variant.v1", &variant, 256 * 1024)?;
    if let Some(index) = seen.get(&digest) {
        if variants.get(usize::from(*index)) != Some(&variant) {
            return Err(RatingError::Identity);
        }
        return Ok(*index);
    }
    if variants.len() == MAX_VARIANTS {
        return Err(RatingError::Bound);
    }
    let index = u8::try_from(variants.len()).map_err(|_| RatingError::Bound)?;
    variants.push(variant);
    seen.insert(digest, index);
    Ok(index)
}

pub(super) fn rating_class(class: CoreClass) -> Result<RatingClass, RatingError> {
    match class {
        CoreClass::I => Ok(RatingClass::I),
        CoreClass::II => Ok(RatingClass::II),
        CoreClass::III => Ok(RatingClass::III),
        CoreClass::Zero => Err(RatingError::Invalid),
    }
}
