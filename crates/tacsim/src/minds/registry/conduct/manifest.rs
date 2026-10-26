//! Frozen data for private role arenas. The manifest is only structural data;
//! actual owner registration and acknowledged work establish authority.
use super::super::{identity, manifest::FrozenPolicy};
use super::{Case, cargo::CargoSpec};
use crate::{
    arena::{input_tape::EngineStamp, replay_ground::ReplayGround},
    config::{LoadedPack, TacsimConfig},
    definitions::{Catalog, GoodsTable},
    port::capacity::Compartment,
};
use mush_core::{CanonicalId, DefinitionDigest, DefinitionRef};
use mush_roll::commerce::rating::RatingError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

const SOURCE: &str = include_str!("../../../../content/proving/conduct.json");
const CARRIER: &str = include_str!("../../../../content/units/conduct_carrier.json");
const GOODS: &str = include_str!("../../../../content/market/goods.json");

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Recipe {
    format: u16,
    name: String,
    escort: CanonicalId,
    carrier: CanonicalId,
    core: CanonicalId,
    ground: CanonicalId,
    cargo: CargoSpec,
    seeds: Vec<u64>,
    maximum_ms: u32,
    roles: Vec<String>,
    cases: Vec<Case>,
    escort_cases: Vec<Case>,
    transit_cases: Vec<Case>,
    note: String,
}
impl Recipe {
    fn validate(&self) -> Result<(), RatingError> {
        if self.format != 1
            || self.name != "safe_conduct.ground.v1"
            || self.maximum_ms != 120_000
            || self.seeds != [2350, 2351, 2352, 2353, 2354]
            || self.roles != ["Escort", "Transit"]
            || self.cases.as_slice() != Case::ALL
            || self.escort_cases.as_slice() != Case::for_role(super::Role::Escort)
            || self.transit_cases.as_slice() != Case::for_role(super::Role::Transit)
            || self.note.len() > 512
        {
            return Err(RatingError::Identity);
        }
        self.cargo.validate()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u16,
    recipe: Recipe,
    combat_policy: DefinitionRef,
    engine: EngineStamp,
    configuration: DefinitionDigest,
    ground: DefinitionDigest,
    goods: DefinitionDigest,
    implementation: Vec<DefinitionDigest>,
    definitions: Vec<DefinitionRef>,
}

/// Retained data, not executable policy or a permission to create a worker.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RetainedPolicy {
    manifest: Manifest,
    goods: GoodsTable,
}
impl RetainedPolicy {
    pub(super) fn validate(&self, expected: DefinitionDigest) -> Result<(), RatingError> {
        let m = &self.manifest;
        m.recipe.validate()?;
        // Validate the retained table through its ordinary checked loader. Do
        // not replace an old table with today's content or mechanics registry.
        let goods = GoodsTable::load(
            "retained Conduct goods",
            &serde_json::to_string(&self.goods).map_err(|_| RatingError::Invalid)?,
        )
        .map_err(|_| RatingError::Invalid)?;
        if m.format != 1
            || m.implementation.len() != 15
            || m.definitions.len() != 6
            || m.definitions
                .windows(2)
                .any(|p| !matches!(p, [a,b] if a < b))
            || !m.definitions.contains(&m.combat_policy)
            || identity("tacsim.conduct.goods.v1", &goods, 1024 * 1024)? != m.goods
            || identity("tacsim.conduct.manifest.v1", m, 32 * 1024)? != expected
        {
            return Err(RatingError::Identity);
        }
        Ok(())
    }
    pub(super) fn cargo(
        &self,
        policy: DefinitionDigest,
        seed: u64,
    ) -> Result<super::cargo::CargoReport, RatingError> {
        let spec = &self.manifest.recipe.cargo;
        let compartment = Compartment {
            id: spec.compartment.clone(),
            accepts: spec.accepts.clone(),
            limits: spec.limits.clone(),
        };
        super::cargo::Cargo::load(
            spec,
            &compartment,
            &self.goods,
            mush_core::ObjId(101),
            (129, 97),
            policy,
            seed,
        )?
        .report()
    }
    pub(super) fn descriptor(&self) -> Result<mush_core::DefinitionDescriptor, RatingError> {
        mush_core::DefinitionDescriptor::new(
            mush_core::DefinitionKind::new("conduct_manifest").map_err(|_| RatingError::Invalid)?,
            mush_core::DefinitionId::new("safe_conduct").map_err(|_| RatingError::Invalid)?,
            1,
            serde_json::to_value(&self.manifest).map_err(|_| RatingError::Invalid)?,
        )
        .map(|d| d.with_references(self.manifest.definitions.clone()))
        .map_err(|_| RatingError::Invalid)
    }
    pub(super) fn definition_ref(&self) -> Result<DefinitionRef, RatingError> {
        mush_core::DefinitionRegistryBuilder::new()
            .register(
                mush_core::OwnerId::new("tacsim").map_err(|_| RatingError::Invalid)?,
                self.descriptor()?,
            )
            .map_err(|_| RatingError::Invalid)
    }
}

pub(super) struct Policy {
    pub(super) physics: FrozenPolicy,
    pub(super) goods: GoodsTable,
    compartment: Compartment,
    recipe: Recipe,
    manifest: Manifest,
    digest: DefinitionDigest,
}
impl Policy {
    pub(super) fn retained(&self) -> RetainedPolicy {
        RetainedPolicy {
            manifest: self.manifest.clone(),
            goods: self.goods.clone(),
        }
    }
    pub(super) fn descriptor(&self) -> Result<mush_core::DefinitionDescriptor, RatingError> {
        self.retained().descriptor()
    }
    pub(super) fn definition_ref(&self) -> Result<DefinitionRef, RatingError> {
        self.retained().definition_ref()
    }
    /// The role content enters the same strict pack loader as ordinary units.
    /// This is opt-in; it changes neither the default catalogue nor Dray.
    pub(super) fn carrier_pack() -> Result<LoadedPack, RatingError> {
        Ok(LoadedPack {
            source: "builtin:conduct_carrier".into(),
            pack: crate::pack::load_pack(CARRIER).map_err(|_| RatingError::Invalid)?,
        })
    }
    pub(super) fn capture(
        catalog: Arc<Catalog>,
        records: &mush_atlas::record::Records,
        cfg: TacsimConfig,
    ) -> Result<Self, RatingError> {
        if SOURCE.len() > 16 * 1024 {
            return Err(RatingError::Bound);
        }
        let recipe: Recipe = serde_json::from_str(SOURCE).map_err(|_| RatingError::Invalid)?;
        recipe.validate()?;
        let physics = FrozenPolicy::capture(catalog, records, cfg)?;
        if physics.ground.id != recipe.ground.as_str() {
            return Err(RatingError::Identity);
        }
        // Capacity comes from the same bound ground hull that the native arena
        // will instantiate. A scenario cannot retain an independently authored
        // capacity after a carrier definition or compartment changes.
        let compartment = carrier_compartment(&physics.catalog, &recipe)?;
        let mut definitions = [
            &recipe.escort,
            &recipe.carrier,
            &recipe.core,
            &recipe.ground,
        ]
        .into_iter()
        .map(|id| {
            physics
                .catalog
                .references
                .get(id)
                .cloned()
                .ok_or(RatingError::Unavailable)
        })
        .collect::<Result<Vec<_>, _>>()?;
        definitions.push(
            physics
                .catalog
                .references
                .get(
                    &CanonicalId::new(crate::definitions::content::RESOLUTION)
                        .map_err(|_| RatingError::Invalid)?,
                )
                .cloned()
                .ok_or(RatingError::Unavailable)?,
        );
        // Reuse the complete existing native/sandbox implementation manifest,
        // not merely its display version or a subset of role helper sources.
        let combat_policy = physics.definition_ref().map_err(|_| RatingError::Invalid)?;
        definitions.push(combat_policy.clone());
        definitions.sort();
        definitions.dedup();
        let goods = GoodsTable::load("conduct cargo", GOODS).map_err(|_| RatingError::Invalid)?;
        let ground = ReplayGround::capture(&physics.ground).map_err(|_| RatingError::Invalid)?;
        let manifest = Manifest {
            format: 1,
            recipe: recipe.clone(),
            combat_policy,
            engine: EngineStamp::current(),
            configuration: identity(
                "tacsim.conduct.config.v1",
                &(
                    &physics.cfg.arena,
                    &physics.cfg.match_,
                    &physics.cfg.sensors,
                    &physics.cfg.minds,
                ),
                1024 * 1024,
            )?,
            ground: identity("tacsim.conduct.ground.v1", &ground, 16 * 1024 * 1024)?,
            goods: identity("tacsim.conduct.goods.v1", &goods, 1024 * 1024)?,
            implementation: [
                include_str!("trial.rs"),
                include_str!("observation.rs"),
                include_str!("cargo.rs"),
                include_str!("mod.rs"),
                include_str!("manifest.rs"),
                include_str!("run.rs"),
                include_str!("../runtime.rs"),
                include_str!("../subject.rs"),
                include_str!("../meter.rs"),
                include_str!("../../../arena/stock_mind.rs"),
                include_str!("../../../arena/intents.rs"),
                include_str!("../../../arena/motor.rs"),
                include_str!("worker.rs"),
                include_str!("history.rs"),
                include_str!("../../bulk/conduct_budget.rs"),
            ]
            .into_iter()
            .map(|source| identity("tacsim.conduct.implementation.v1", &source, 1024 * 1024))
            .collect::<Result<Vec<_>, _>>()?,
            definitions,
        };
        let digest = identity("tacsim.conduct.manifest.v1", &manifest, 32 * 1024)?;
        Ok(Self {
            physics,
            goods,
            compartment,
            recipe,
            manifest,
            digest,
        })
    }
    pub(super) fn digest(&self) -> DefinitionDigest {
        self.digest
    }
    pub(super) fn escort(&self) -> &CanonicalId {
        &self.recipe.escort
    }
    pub(super) fn carrier(&self) -> &CanonicalId {
        &self.recipe.carrier
    }
    pub(super) fn cargo(&self) -> &CargoSpec {
        &self.recipe.cargo
    }
    pub(super) fn compartment(&self) -> &Compartment {
        &self.compartment
    }
    pub(super) fn seeds(&self) -> &[u64] {
        &self.recipe.seeds
    }
    pub(super) fn maximum_ms(&self) -> u32 {
        self.recipe.maximum_ms
    }
    #[cfg(test)]
    pub(super) fn references(&self) -> &[DefinitionRef] {
        &self.manifest.definitions
    }
}

fn carrier_compartment(catalog: &Catalog, recipe: &Recipe) -> Result<Compartment, RatingError> {
    use crate::arena::core::{CoreClass, read_core};
    use crate::definitions::{UnitClass, to_tactical_unit};
    let spec = catalog
        .specs
        .get(&recipe.carrier)
        .ok_or(RatingError::Unavailable)?;
    spec.check(recipe.carrier.as_str())
        .map_err(|_| RatingError::Invalid)?;
    let binding = catalog
        .bindings
        .get(&recipe.carrier)
        .ok_or(RatingError::Unavailable)?;
    let core = catalog
        .cores
        .get(&recipe.core)
        .ok_or(RatingError::Unavailable)?;
    let stowage = spec
        .ground_stowage
        .get(&recipe.cargo.compartment)
        .ok_or(RatingError::Identity)?;
    let rules = binding.spec.rules.data();
    if spec.class_of() != UnitClass::Armour
        || !spec.class_of().fieldable()
        || spec.mobility.tracked.is_none()
        || spec.logistics.is_some()
        || spec.interior.is_some()
        || spec.weapons().next().is_some()
        || spec.cores().collect::<Vec<_>>() != [&recipe.core]
        || core.class != CoreClass::II
        || !core.governed
        || stowage.accepts != recipe.cargo.accepts
        || stowage.limits != recipe.cargo.limits
        || catalog.references.get(&recipe.carrier) != Some(&binding.spec.definition)
        || rules.get("ground_stowage") != Some(&serde_json::json!(spec.ground_stowage))
        || rules.get("components") != Some(&serde_json::json!(spec.components))
        || rules.get("mobility") != Some(&serde_json::json!(spec.mobility))
    {
        return Err(RatingError::Identity);
    }
    let unit =
        to_tactical_unit(spec, binding, mush_core::ObjId(101)).map_err(|_| RatingError::Invalid)?;
    let actual = read_core(&unit).ok_or(RatingError::Identity)?;
    if actual.class != CoreClass::II
        || !actual.governed
        || UnitClass::read(&unit) != UnitClass::Armour
    {
        return Err(RatingError::Identity);
    }
    Ok(Compartment::from_stowage(
        &recipe.cargo.compartment,
        stowage,
    ))
}
