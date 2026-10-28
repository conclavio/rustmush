//! Frozen real content/engine configuration and its registered definition fence.
//! Only module composition can register this value. A structural digest from a
//! command cannot construct RegisteredManifest or make an old rating current.

use super::{
    identity,
    plan::{STARTUP_MS, TRIAL_LIMIT_MS},
};
use crate::{
    arena::{input_tape::EngineStamp, replay_ground::ReplayGround, state::ArenaDefinition},
    config::TacsimConfig,
    definitions::{Catalog, content},
};
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, DefinitionValidationContext,
};
use mush_core::{CanonicalId, DefinitionDigest, ModError, Registrar};
use mush_roll::commerce::rating::{RatingCompatibility, RatingError};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{collections::BTreeMap, sync::Arc};

/// Owns the exact immutable resources used by every case, not just hashes of
/// today's lookup. Nothing here accepts an Arena or a live source-store handle.
pub(super) struct FrozenPolicy {
    pub(super) catalog: Arc<Catalog>,
    pub(super) cfg: TacsimConfig,
    pub(super) ground: Arc<ArenaDefinition>,
    manifest: Manifest,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u16,
    name: String,
    engine: EngineStamp,
    configuration: DefinitionDigest,
    ground: DefinitionDigest,
    layouts: DefinitionDigest,
    kernel_probes: Vec<(
        crate::arena::core::CoreClass,
        mush_roll::commerce::rating::KernelName,
        super::kernels::Recipe,
    )>,
    implementation: BTreeMap<String, DefinitionDigest>,
    definitions: Vec<DefinitionRef>,
}
/// Source-free registration result. Its fields are read-only and its sole
/// constructor executes the actual owner-stamped Registrar::definition path.
/// Keep this in the trusted composition graph; do not decode it from clients.
#[derive(Clone, Debug)]
pub(crate) struct RegisteredManifest {
    definition: DefinitionRef,
    compatibility: RatingCompatibility,
    captured_policy: DefinitionDigest,
}
impl RegisteredManifest {
    pub(crate) fn definition(&self) -> &DefinitionRef {
        &self.definition
    }
    pub(crate) fn compatibility(&self) -> &RatingCompatibility {
        &self.compatibility
    }
}
impl FrozenPolicy {
    fn descriptor(&self) -> Result<DefinitionDescriptor, ModError> {
        Ok(DefinitionDescriptor::from_serializable(
            DefinitionKind::new("registry_manifest").map_err(|e| ModError::msg(e.to_string()))?,
            DefinitionId::new("combat").map_err(|e| ModError::msg(e.to_string()))?,
            1,
            &self.manifest,
        )
        .map_err(|e| ModError::msg(e.to_string()))?
        .with_references(self.manifest.definitions.clone())
        .as_current())
    }

    /// Structural identity for the trusted composition graph. This does not
    /// create RegisteredManifest or a factory: actual registration below must
    /// install and validate exactly the same descriptor and content references.
    pub(super) fn definition_ref(&self) -> Result<DefinitionRef, ModError> {
        mush_core::mechanics::DefinitionRegistryBuilder::new()
            .register(
                mush_core::mechanics::OwnerId::new(crate::NS)
                    .map_err(|e| ModError::msg(e.to_string()))?,
                self.descriptor()?,
            )
            .map_err(|e| ModError::msg(e.to_string()))
    }

    pub(super) fn capture(
        catalog: Arc<Catalog>,
        records: &mush_atlas::record::Records,
        mut cfg: TacsimConfig,
    ) -> Result<Self, RatingError> {
        // These are manifest rules, not operator-selected match duration. The
        // complete resulting config is hashed and retained for reconstruction.
        cfg.match_.startup_ms = u32::try_from(STARTUP_MS).map_err(|_| RatingError::Bound)?;
        cfg.match_.max_duration_ms =
            u32::try_from(TRIAL_LIMIT_MS).map_err(|_| RatingError::Bound)?;
        let ground_id =
            CanonicalId::new("tacsim.arena.proving_ground").map_err(|_| RatingError::Invalid)?;
        let authored = catalog
            .arenas
            .get(&ground_id)
            .ok_or(RatingError::Unavailable)?;
        let ground = crate::definitions::from_pack(ground_id.as_str(), authored, records)
            .map_err(|_| RatingError::Invalid)?;
        let frozen_ground = ReplayGround::capture(&ground).map_err(|_| RatingError::Invalid)?;
        let mut definitions = Vec::new();
        for word in [
            "tacsim.arena.proving_ground",
            content::LINE_TANK,
            "tacsim.unit.heavy_mk1",
            content::RESOLUTION,
        ] {
            let id = CanonicalId::new(word).map_err(|_| RatingError::Invalid)?;
            definitions.push(
                catalog
                    .references
                    .get(&id)
                    .ok_or(RatingError::Unavailable)?
                    .clone(),
            );
        }
        definitions.sort();
        definitions.dedup();
        let manifest = Manifest {
            format: 1,
            name: "combat.v1".into(),
            engine: EngineStamp::current(),
            configuration: identity(
                "tacsim.registry.configuration.v1",
                &(&cfg.arena, &cfg.match_, &cfg.sensors, &cfg.minds),
                1024 * 1024,
            )?,
            ground: identity(
                "tacsim.registry.ground.v1",
                &frozen_ground,
                16 * 1024 * 1024,
            )?,
            layouts: source_hash(include_str!("../../../content/proving/scenarios.json")),
            kernel_probes: [
                crate::arena::core::CoreClass::I,
                crate::arena::core::CoreClass::II,
                crate::arena::core::CoreClass::III,
            ]
            .into_iter()
            .flat_map(|class| {
                super::plan::KERNELS
                    .into_iter()
                    .map(move |name| (class, name))
            })
            .map(|(class, name)| {
                Ok((
                    class,
                    name,
                    super::kernels::recipe(name, cfg.minds.classes.row(class))?,
                ))
            })
            .collect::<Result<Vec<_>, RatingError>>()?,
            implementation: implementation(),
            definitions,
        };
        manifest.validate()?;
        Ok(Self {
            catalog,
            cfg,
            ground: Arc::new(ground),
            manifest,
        })
    }
    pub(super) fn matches_registration(
        &self,
        registered: &RegisteredManifest,
    ) -> Result<bool, RatingError> {
        Ok(registered.captured_policy
            == identity(
                "tacsim.registry.captured_policy.v1",
                &self.manifest,
                32 * 1024,
            )?)
    }
    /// Must be called by Tacsim's actual registration callback. Roll receives
    /// the returned pair only through trusted composition and adds definition()
    /// to every authoritative action's exact definition guards.
    pub(super) fn register(&self, reg: &mut Registrar<'_>) -> Result<RegisteredManifest, ModError> {
        if reg.module() != crate::NS {
            return Err(ModError::msg("Registry manifest owner"));
        }
        let kind =
            DefinitionKind::new("registry_manifest").map_err(|e| ModError::msg(e.to_string()))?;
        let expected = self.manifest.clone();
        reg.definition_validator(
            kind.clone(),
            move |ctx: &DefinitionValidationContext<'_>| {
                let actual: Manifest = serde_json::from_value(ctx.definition().payload().clone())
                    .map_err(|e| e.to_string())?;
                actual.validate().map_err(|e| e.to_string())?;
                if actual != expected
                    || ctx.definition().references() != actual.definitions.as_slice()
                {
                    return Err(
                        "Registry manifest does not describe captured production resources".into(),
                    );
                }
                for reference in &actual.definitions {
                    ctx.get_exact(reference).map_err(|e| e.to_string())?;
                }
                Ok(())
            },
        )?;
        let definition = reg.definition(self.descriptor()?)?;
        let compatibility = RatingCompatibility::combat_v1(definition.digest());
        Ok(RegisteredManifest {
            definition,
            compatibility,
            captured_policy: identity(
                "tacsim.registry.captured_policy.v1",
                &self.manifest,
                32 * 1024,
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
        })
    }
}
impl Manifest {
    fn validate(&self) -> Result<(), RatingError> {
        if self.format != 1
            || self.name != "combat.v1"
            || self.engine != EngineStamp::current()
            || self.implementation != implementation()
            || self.definitions.len() != 4
            || self
                .definitions
                .windows(2)
                .any(|pair| matches!(pair,[a,b] if a>=b))
        {
            return Err(RatingError::Identity);
        }
        identity("tacsim.registry.manifest.shape.v1", self, 32 * 1024)?;
        Ok(())
    }
}
fn source_hash(source: &str) -> DefinitionDigest {
    DefinitionDigest::from_bytes(Sha256::digest(source.as_bytes()).into())
}
/// Pin actual charging/formula/API code, not a hand-copied numeric fee table.
/// This may conservatively invalidate on a comment-only implementation edit;
/// it cannot silently accept changed helper/native-rule fees under an old ABI.
fn implementation() -> BTreeMap<String, DefinitionDigest> {
    [
        ("api", include_str!("../api.rs")),
        ("registration", include_str!("../mod.rs")),
        ("fees", include_str!("../fees.rs")),
        ("kernels", include_str!("../kernels.rs")),
        ("stock_helpers", include_str!("../stock.rs")),
        ("rule_helper", include_str!("../rule_samples.rs")),
        (
            "native_program",
            include_str!("../../arena/profile_rules.rs"),
        ),
        ("runner", include_str!("../runner.rs")),
        ("sandbox", include_str!("../../../../mush-mind/src/lib.rs")),
        (
            "engine",
            include_str!("../../../../mush-mind/src/engine.rs"),
        ),
        (
            "sandbox_run",
            include_str!("../../../../mush-mind/src/run.rs"),
        ),
        (
            "memory",
            include_str!("../../../../mush-mind/src/memory.rs"),
        ),
        (
            "budget",
            include_str!("../../../../mush-mind/src/budget.rs"),
        ),
        ("meter", include_str!("meter.rs")),
        ("physical_observer", include_str!("meter/physical.rs")),
        ("schedule", include_str!("plan.rs")),
        ("subject", include_str!("subject.rs")),
        ("runtime", include_str!("runtime.rs")),
        ("trials", include_str!("trials.rs")),
        ("probes", include_str!("probes.rs")),
        ("kernel_probes", include_str!("kernels.rs")),
        (
            "public_picture",
            concat!(
                include_str!("../../arena/step.rs"),
                include_str!("../../arena/step/movement.rs"),
                include_str!("../../arena/step/sensors.rs"),
                include_str!("../../arena/step/weapons.rs"),
                include_str!("../../arena/step/frames.rs"),
            ),
        ),
        ("profile_probe", include_str!("profiles.rs")),
        ("ford_probe", include_str!("ford.rs")),
        ("order_loss_probe", include_str!("order_loss.rs")),
        ("worker", include_str!("machine.rs")),
    ]
    .into_iter()
    .map(|(name, source)| (name.into(), source_hash(source)))
    .collect()
}
