//! Immutable execution material, captured from real owner data. No MatchId,
//! current eligibility cache or public source-display capability lives here.
use mush_core::participant::ParticipantError;
use mush_core::{DefinitionDigest, World};
use mush_roll::commerce::ic_claims::IcAdmissionFacts;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use smol_str::SmolStr;

use crate::arena::core::ClassTable;
use crate::arena::mind::{MindSeal, SealEngine, SortieId};
use crate::arena::rules::RuleWeapon;
use crate::config::{ArenaConfig, MindsConfig};
use crate::deliberation::order::CharacterRef;
use crate::match_::filed_holdings::FiledExecutable;
use crate::match_::seal::{SealPolicy, seal_with_policy};

fn bad(_: impl std::fmt::Display) -> ParticipantError {
    ParticipantError::refused("IC executable material is unavailable or changed")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Execution {
    pub(super) seal: MindSeal,
    source: String,
    arena: ArenaConfig,
    classes: ClassTable,
}
impl Execution {
    pub(super) fn capture(
        world: &World,
        facts: &IcAdmissionFacts,
        sortie: u64,
        profiles: &[SmolStr],
        arena: &ArenaConfig,
        minds: &MindsConfig,
    ) -> Result<Self, ParticipantError> {
        arena.validate().map_err(bad)?;
        minds.validate().map_err(bad)?;
        if sortie == 0 || profiles.len() > crate::arena::card::ProfileSet::MOST {
            return Err(bad("profile or sortie bound"));
        }
        for (kind, owner) in [
            (
                std::any::TypeId::of::<crate::store::TacsimStore>(),
                "tacsim",
            ),
            (std::any::TypeId::of::<mush_tactics::UnitStore>(), "tactics"),
        ] {
            if world
                .module_state_owner(kind)
                .is_none_or(|v| v.as_str() != owner)
            {
                return Err(bad("owner slot"));
            }
        }
        let store = world
            .module_state::<crate::store::TacsimStore>()
            .ok_or_else(|| bad("sources"))?;
        let unit = mush_tactics::module::unit_of(world, facts.unit()).ok_or_else(|| bad("unit"))?;
        let catalog = store.definitions();
        // weapon_summaries is intentionally total for OOC callers. IC must not
        // interpret an unknown/mismatched schema as an unarmed authored hull.
        let id = catalog
            .references
            .iter()
            .find(|(_, r)| *r == unit.schema())
            .map(|(id, _)| id)
            .ok_or_else(|| bad("unit schema"))?;
        if !catalog.specs.contains_key(id) || !catalog.bindings.contains_key(id) {
            return Err(bad("unit catalog"));
        }
        catalog
            .bindings
            .get(id)
            .ok_or_else(|| bad("unit binding"))?
            .spec
            .validate_instance(unit)
            .map_err(bad)?;
        let weapons = crate::match_::weapon_summaries(catalog, unit)
            .into_iter()
            .map(|w| RuleWeapon {
                id: w.id,
                rounds_max: w.count_per_load,
            })
            .collect::<Vec<_>>();
        let material = FiledExecutable::resolve(facts.holding().clone(), store).map_err(bad)?;
        let source = material.source_for_execution();
        let held = material.into_held();
        let definition = held
            .definition
            .as_deref()
            .ok_or_else(|| bad("definition"))?;
        let principal = Some(CharacterRef::Character {
            id: facts.dispatcher(),
        });
        // This stock value is unused by the authored-card branch, and supplies
        // no fallback identity. Class0 uses the independently captured ROM.
        let card = crate::arena::card::line(&arena.reflex);
        let fallback = None;
        let seal = seal_with_policy(
            facts.unit(),
            unit,
            &weapons,
            SortieId(sortie),
            &SealPolicy {
                authored: Some(definition),
                selected_profiles: profiles,
                classes: &minds.classes,
                rom_cfg: &arena.reflex,
                principal: &principal,
                card: &card,
                odds_floor_permille: minds.odds_floor_permille,
                engine_for_minds: held.engine,
                rated: held.rated,
                class_cap: minds.max_class,
                identity: &held.identity,
                profiles: &[],
                fallback: &fallback,
            },
        )
        .map_err(bad)?;
        let result = Self {
            seal,
            source: source.to_string(),
            arena: arena.clone(),
            classes: minds.classes,
        };
        result.validate()?;
        // Compile before admission, never after the first owner applies. The
        // compiler cannot publish and is not a substitute for certification.
        result.compile()?;
        Ok(result)
    }
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        self.arena.validate().map_err(bad)?;
        self.seal
            .verify(&self.classes, &self.seal.weapons)
            .map_err(bad)?;
        let digest: [u8; 32] = Sha256::digest(self.source.as_bytes()).into();
        if self.source.is_empty()
            || self.source.len() > mush_roll::MAX_SOURCE_BYTES
            || self.source.contains('\0')
            || self.seal.sortie.0 == 0
            || self.seal.requested.source_sha256 != digest
            || self.seal.requested.origin.certificate().is_none()
            || (self.seal.engine == SealEngine::Scripted && self.seal.source_sha256 != digest)
        {
            return Err(bad("immutable source"));
        }
        Ok(())
    }
    pub(super) fn compile(&self) -> Result<(), ParticipantError> {
        self.validate()?;
        self.runner().map(|_| ())
    }
    fn runner(
        &self,
    ) -> Result<std::sync::Arc<dyn crate::arena::mind::MindRunner>, ParticipantError> {
        self.validate()?;
        if self.seal.abi == crate::minds::MIND_ABI && self.seal.engine == SealEngine::Scripted {
            Ok(std::sync::Arc::new(
                crate::minds::RhaiRunner::new(
                    &self.arena,
                    &self.source,
                    mush_mind::Digest(self.seal.source_sha256),
                )
                .map_err(bad)?,
            ))
        } else {
            Ok(crate::arena::mind::default_mind_runner(&self.arena))
        }
    }
    pub(super) fn install(&self, arena: &mut crate::arena::Arena) -> Result<(), ParticipantError> {
        if arena.arena_cfg != self.arena
            || arena.classes != self.classes
            || arena
                .unit(self.seal.unit)
                .is_none_or(|u| u.seal != self.seal.seal_ref())
        {
            return Err(bad("runtime seal or captured configuration"));
        }
        // Prepare the current runner before touching runtime. This never changes the full
        // seal nor grants authority to mutate any physical owner or world row.
        let runner = self.runner()?;
        if self.seal.abi != crate::minds::MIND_ABI {
            let unit = self.seal.unit;
            let mind = arena
                .minds
                .get(&unit)
                .ok_or_else(|| bad("retained mind row"))?;
            if mind.seal_digest != self.seal.digest() {
                return Err(bad("retained mind seal"));
            }
            let first_drop = mind.dropped.is_none();
            let runtime = arena
                .profiles
                .get(&unit)
                .ok_or_else(|| bad("runtime profile"))?;
            if !first_drop && (!runtime.dropped || runtime.card.key() != runtime.rom.key()) {
                return Err(bad("retained ROM drop"));
            }
            if first_drop {
                // The full seal and captured ROM card stay immutable. The
                // native doctrine implementation is the current server's; no
                // old Rhai program is compiled or resumed (doc17 D-UM-23).
                let weapons = self
                    .seal
                    .weapons
                    .iter()
                    .map(|w| w.id.clone())
                    .collect::<Vec<_>>();
                let prepared = runtime
                    .prepare_rom(
                        unit,
                        &arena.classes.row(crate::arena::core::CoreClass::Zero),
                        &weapons,
                        &self.seal.channels,
                        arena.odds_floor_permille,
                    )
                    .map_err(bad)?;
                let hash = crate::arena::hash::state_hash(arena);
                let runtime = arena
                    .profiles
                    .get_mut(&unit)
                    .ok_or_else(|| bad("runtime profile"))?;
                // Journal capacity refuses before any runtime or memory change.
                // No intervening callback/mutation can change this exact token,
                // so commit's owner/generation check has the prepared preimage.
                arena
                    .ledger
                    .append(
                        arena.now_ms,
                        crate::arena::EventKind::MindDropped {
                            unit,
                            cause: "abi".into(),
                            fault: None,
                            ops: None,
                        },
                        Vec::new(),
                        hash,
                    )
                    .map_err(bad)?;
                runtime
                    .commit(&mut arena.conditions, prepared)
                    .map_err(bad)?;
                arena
                    .minds
                    .get_mut(&unit)
                    .ok_or_else(|| bad("retained mind row"))?
                    .dropped = Some(crate::arena::mind::DropCause::Abi);
            }
            // Neither an outstanding old-engine answer nor its sampled rules
            // may land after the upgrade. Landed memory bytes are untouched.
            arena.cancel_profile_think(unit);
        }
        arena.runners.insert(self.seal.unit, runner);
        Ok(())
    }
    pub(super) fn digest(&self) -> DefinitionDigest {
        DefinitionDigest::from_bytes(self.seal.digest())
    }
}

#[cfg(test)]
mod tests;
