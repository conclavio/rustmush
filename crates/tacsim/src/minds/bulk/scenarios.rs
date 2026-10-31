//! Production content expansion and character-owned immutable selection. A
//! cheap owner checkout freezes the selection, then a separate preparation
//! worker expands terrain/seals. There is no test helper or live arena input.
use super::{
    BulkError,
    frozen::FrozenStarts,
    invalid,
    model::{Artifact, MAX_ARTIFACT_BYTES, Opponent, Plan, Scenario, WorkKind},
    proving::FrozenProving,
};
use crate::{
    arena::{
        Arena, ArenaMode, Controller, CustodyOut, MatchMode, PartyId,
        input_tape::{EngineStamp, TapeHeader},
        profile_seal::MindOrigin,
    },
    arena_task::MatchId,
    config::TacsimConfig,
    definitions::{Catalog, content},
    match_::{SealTerms, draft_holdings::SavedMind, holdings::HeldMind},
};
use mush_core::{CanonicalId, ObjId};
use mush_roll::{RevisionKey, RevisionRef, RollError, RollStore, SourceAdapter};
use mush_sim::data::Pose;
use serde::Deserialize;
use smol_str::SmolStr;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub(in crate::minds) struct Candidate {
    pub(super) held: HeldMind,
    pub(super) profile: SmolStr,
    owned: Option<SavedMind>,
}
impl Candidate {
    pub(super) fn resolve<S: SourceAdapter>(
        roll: &RollStore,
        sources: &S,
        actor: ObjId,
        word: &str,
    ) -> Result<Self, RollError> {
        let (reference, profile) = word
            .split_once('/')
            .map_or((word, None), |(r, p)| (r, Some(p)));
        if ["standard", "doctrine"]
            .iter()
            .any(|w| reference.eq_ignore_ascii_case(w))
        {
            let held = HeldMind::doctrine();
            let profile = held
                .preset(profile.unwrap_or("Line"))
                .ok_or(RollError::Missing)?;
            return Ok(Self {
                held,
                profile,
                owned: None,
            });
        }
        let (name, version) = reference
            .split_once('@')
            .map_or((reference, None), |(a, b)| (a, Some(b)));
        let mind = roll.mind(actor, name)?;
        let pin = match version {
            None => None,
            Some(word) if word.starts_with("sha:") => {
                Some(roll.resolve_digest(actor, name, &word[4..])?)
            }
            Some(word) => {
                let number = word
                    .parse::<u64>()
                    .ok()
                    .filter(|n| *n > 0)
                    .or_else(|| mind.named().get(&word.to_ascii_lowercase()).copied())
                    .ok_or(RollError::Missing)?;
                Some(
                    roll.revision(
                        actor,
                        RevisionKey {
                            character: actor,
                            mind: mind.id(),
                            number,
                        },
                    )?
                    .reference(),
                )
            }
        };
        let saved = SavedMind::resolve(roll, sources, actor, name, pin)?;
        let choice = crate::match_::draft_holdings::SavedChoice::choose(roll, actor, name, pin)?;
        let held = HeldMind::from_saved(saved.clone(), choice);
        let profile = match profile {
            Some(p) => held.preset(p).ok_or(RollError::Missing)?,
            None => held.presets.first().cloned().ok_or(RollError::Missing)?,
        };
        Ok(Self {
            held,
            profile,
            owned: Some(saved),
        })
    }
    pub(super) fn revalidate<S: SourceAdapter>(
        &self,
        roll: &RollStore,
        sources: &S,
        actor: ObjId,
    ) -> Result<(), RollError> {
        if let Some(saved) = &self.owned {
            saved.revalidate(roll, sources, actor)?;
        }
        Ok(())
    }
    pub(super) fn reference(&self) -> Option<RevisionRef> {
        match self.owned.as_ref().map(|s| &s.origin) {
            Some(MindOrigin::Draft {
                character,
                mind,
                revision,
                digest,
            }) => Some(RevisionRef {
                key: RevisionKey {
                    character: *character,
                    mind: *mind,
                    number: *revision,
                },
                digest: *digest,
            }),
            _ => None,
        }
    }
}
/// Constructed from current authenticated actor plus strict line/typed fields.
/// Only saved revisions enter proving. Mutable drafts use the six quick checks.
pub(crate) struct Recipe {
    actor: ObjId,
    candidate: Candidate,
    opponent: Candidate,
    scenario: Scenario,
    opponent_kind: Opponent,
    per_side: u8,
    seeds: Vec<u64>,
}
impl Recipe {
    #[allow(clippy::too_many_arguments)]
    pub fn owned<S: SourceAdapter>(
        roll: &RollStore,
        sources: &S,
        actor: ObjId,
        candidate: &str,
        opponent: &str,
        scenario: Scenario,
        per_side: u8,
        games: u16,
        first_seed: u64,
    ) -> Result<Self, RollError> {
        if !(1..=6).contains(&per_side) || !(10..=100).contains(&games) || !games.is_multiple_of(2)
        {
            return Err(RollError::Missing);
        }
        let candidate = Candidate::resolve(roll, sources, actor, candidate)?;
        let (opponent, opponent_kind) = if opponent.eq_ignore_ascii_case("mirror") {
            (candidate.clone(), Opponent::Mirror)
        } else {
            let other = Candidate::resolve(roll, sources, actor, opponent)?;
            let kind = if other.held.is_doctrine() {
                Opponent::Doctrine
            } else {
                Opponent::Owned
            };
            (other, kind)
        };
        let seeds = (0..games / 2)
            .map(|n| {
                first_seed
                    .checked_add(u64::from(n))
                    .ok_or(RollError::Exhausted("seeds"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            actor,
            candidate,
            opponent,
            scenario,
            opponent_kind,
            per_side,
            seeds,
        })
    }
    /// Final owner/current-action recheck must run before publishing admission.
    pub fn revalidate<S: SourceAdapter>(
        &self,
        roll: &RollStore,
        sources: &S,
        actor: ObjId,
    ) -> Result<(), RollError> {
        if actor != self.actor {
            return Err(RollError::Missing);
        }
        self.candidate.revalidate(roll, sources, actor)?;
        self.opponent.revalidate(roll, sources, actor)
    }
    pub fn references(&self) -> Vec<RevisionRef> {
        let mut out = Vec::new();
        for r in [self.candidate.reference(), self.opponent.reference()]
            .into_iter()
            .flatten()
        {
            if !out.contains(&r) {
                out.push(r);
            }
        }
        out
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Content {
    format: u16,
    ground: String,
    chassis: BTreeMap<String, String>,
    scenarios: Vec<Layout>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    name: String,
    mode: String,
    red: Vec<[u16; 2]>,
    blue: Vec<[u16; 2]>,
}
pub(crate) struct ScenarioFactory {
    catalog: Arc<Catalog>,
    records: Arc<mush_atlas::record::Records>,
    cfg: TacsimConfig,
}
impl ScenarioFactory {
    /// Called on config/content composition. Resources are immutable snapshots;
    /// subsequent operator edits cannot change an already prepared recipe.
    pub fn new(
        catalog: Arc<Catalog>,
        records: Arc<mush_atlas::record::Records>,
        cfg: TacsimConfig,
    ) -> Self {
        Self {
            catalog,
            records,
            cfg,
        }
    }
    /// Worker-only. Expands complete content, binds the actual carried weapons,
    /// stations/classes and selected profiles with the production seal owner.
    pub fn prepare(&self, recipe: &Recipe) -> Result<FrozenProving, BulkError> {
        let content: Content =
            serde_json::from_str(include_str!("../../../content/proving/scenarios.json"))
                .map_err(|_| invalid("proving scenario content"))?;
        if content.format != 1 || content.scenarios.len() != 3 {
            return Err(invalid("proving scenario content version"));
        }
        let word = match recipe.scenario {
            Scenario::Duel => "duel",
            Scenario::Muster => "muster",
            Scenario::Hold => "hold",
        };
        let layout = content
            .scenarios
            .iter()
            .find(|r| r.name == word)
            .ok_or_else(|| invalid("proving scenario unavailable"))?;
        if (!layout.red.is_empty() && layout.red.len() != 6)
            || (!layout.blue.is_empty() && layout.blue.len() != 6)
        {
            return Err(invalid("proving deployment content"));
        }
        let ground_id = id(&content.ground)?;
        let authored = self
            .catalog
            .arenas
            .get(&ground_id)
            .ok_or_else(|| invalid("proving ground content missing"))?;
        let definition = crate::definitions::from_pack(ground_id.as_str(), authored, &self.records)
            .map_err(|e| invalid(e.to_string()))?;
        let mode = match layout.mode.as_str() {
            "skirmish" => MatchMode::Skirmish,
            "hold" => {
                if !definition
                    .modes
                    .iter()
                    .any(|m| matches!(m, ArenaMode::Hold(_)))
                {
                    return Err(invalid("proving HOLD mode missing"));
                }
                MatchMode::Hold
            }
            _ => return Err(invalid("proving mode content")),
        };
        let class = recipe.candidate.held.rated.max(recipe.opponent.held.rated);
        if class > self.cfg.minds.max_class {
            return Err(invalid("proving mind exceeds the operator class cap"));
        }
        let chassis = id(content
            .chassis
            .get(class.as_str())
            .ok_or_else(|| invalid("proving class chassis missing"))?)?;
        let spec = self
            .catalog
            .specs
            .get(&chassis)
            .ok_or_else(|| invalid("proving chassis content missing"))?;
        let binding = self
            .catalog
            .bindings
            .get(&chassis)
            .ok_or_else(|| invalid("proving chassis binding missing"))?;
        let resolution = self
            .catalog
            .resolutions
            .get(&id(content::RESOLUTION)?)
            .cloned()
            .ok_or_else(|| invalid("proving resolution content missing"))?;
        let mut starts = FrozenStarts::new();
        let mut native = None;
        for seed in &recipe.seeds {
            for swapped in [false, true] {
                let match_id = MatchId(starts.games.len() as u64 + 1);
                let mut units = Vec::new();
                let mut occupied = Vec::new();
                let mut sources = BTreeMap::new();
                for team in [PartyId::RED, PartyId::BLUE] {
                    let candidate = if (team == PartyId::RED) != swapped {
                        &recipe.candidate
                    } else {
                        &recipe.opponent
                    };
                    let profiles = if candidate.held.definition.is_some() {
                        Vec::new()
                    } else {
                        vec![(
                            candidate.profile.clone(),
                            crate::arena::card::preset(&candidate.profile, &self.cfg.arena.reflex)
                                .ok_or_else(|| invalid("proving doctrine profile"))?,
                        )]
                    };
                    let terms = SealTerms {
                        authored: candidate.held.definition.clone(),
                        selected_profiles: vec![candidate.profile.clone()],
                        classes: self.cfg.minds.classes,
                        rom_cfg: self.cfg.arena.reflex.clone(),
                        match_id,
                        principal: Some(crate::deliberation::order::CharacterRef::Character {
                            id: recipe.actor,
                        }),
                        card: crate::arena::card::line(&self.cfg.arena.reflex),
                        odds_floor_permille: self.cfg.minds.odds_floor_permille,
                        engine_for_minds: candidate.held.engine,
                        rated: candidate.held.rated,
                        class_cap: self.cfg.minds.max_class,
                        identity: candidate.held.identity.clone(),
                        profiles,
                        fallback: None,
                    };
                    let positions = if team == PartyId::RED {
                        &layout.red
                    } else {
                        &layout.blue
                    };
                    let authored_slots = authored
                        .tanks
                        .iter()
                        .filter(|u| u.team == team)
                        .collect::<Vec<_>>();
                    for n in 0..recipe.per_side {
                        let object =
                            ObjId((if team == PartyId::RED { 100 } else { 200 }) + u32::from(n));
                        let [x, y] = if positions.is_empty() {
                            let slot = authored_slots.get(usize::from(n)).ok_or_else(|| {
                                invalid("proving content has too few deployment slots")
                            })?;
                            [slot.x, slot.y]
                        } else {
                            *positions.get(usize::from(n)).ok_or_else(|| {
                                invalid("proving layout has too few deployment slots")
                            })?
                        };
                        let cell = i64::from(definition.terrain.cell_cm());
                        let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
                            .map_err(|e| invalid(e.to_string()))?;
                        let mut proposed = Pose::at(
                            i64::from(x) * cell + cell / 2,
                            i64::from(y) * cell + cell / 2,
                            0,
                            posture,
                        );
                        proposed.heading_mdeg = if team == PartyId::RED {
                            90_000
                        } else {
                            270_000
                        };
                        let pose = crate::match_::opening_pose(
                            &definition,
                            team,
                            Some(proposed),
                            &mut occupied,
                            crate::arena::state::CostRow::Tracked,
                        )
                        .ok_or_else(|| invalid("proving ground has no legal deployment cell"))?;
                        let cells = crate::definitions::to_tactical_unit(spec, binding, object)
                            .map_err(|e| invalid(e.to_string()))?;
                        let unit = crate::match_::custody_unit(
                            &self.catalog,
                            cells,
                            object,
                            team,
                            pose,
                            1,
                            Controller::Mind,
                            n,
                            &terms,
                        )
                        .map_err(|e| invalid(e.to_string()))?;
                        if unit.seal.engine == crate::arena::mind::SealEngine::Scripted {
                            let source = candidate
                                .held
                                .source
                                .as_ref()
                                .ok_or_else(|| invalid("owned sealed source missing"))?;
                            sources.insert(unit.seal.digest(), source.to_string());
                        }
                        units.push(unit);
                    }
                }
                let seals = units.iter().map(|u| u.seal.clone()).collect();
                let mut arena = Arena::new_with_minds(
                    CustodyOut {
                        definition: definition.clone(),
                        units,
                        pilots: Vec::new(),
                        resolution: resolution.clone(),
                        seed: *seed,
                        mode,
                        host: crate::arena::FieldHost::all_hostile(),
                    },
                    &self.cfg.arena,
                    &self.cfg.match_,
                    &self.cfg.sensors,
                    &self.cfg.minds,
                )
                .map_err(|e| invalid(format!("proving custody: {e:?}")))?;
                arena.clock.reanchor(0);
                let step = u32::try_from(arena.substep_len_ms())
                    .map_err(|_| invalid("native proving substep"))?;
                if native.replace(step).is_some_and(|old| old != step) {
                    return Err(invalid("paired proving cadence differs"));
                }
                let header = TapeHeader::capture(
                    &arena,
                    match_id.0,
                    seals,
                    &sources,
                    EngineStamp::current(),
                    MAX_ARTIFACT_BYTES,
                )
                .map_err(|e| invalid(e.to_string()))?;
                starts.push(*seed, swapped, header)?;
            }
        }
        let first = starts.header(0)?;
        use sha2::{Digest as _, Sha256};
        let hash = |bytes: &[u8]| crate::arena::digest_text(&Sha256::digest(bytes).into());
        let selection = |candidate: &Candidate| super::report::Selection {
            class: candidate.held.rated.as_str().to_owned(),
            profile: candidate.profile.to_string(),
            source_sha256: crate::arena::digest_text(&candidate.held.identity.source_sha256),
        };
        let context = super::report::Context {
            scenario: recipe.scenario,
            scenario_content_sha256: hash(include_bytes!(
                "../../../content/proving/scenarios.json"
            )),
            ground: content.ground.clone(),
            ground_sha256: hash(&super::encoding::encode(
                &first.ground,
                MAX_ARTIFACT_BYTES as usize,
            )?),
            config_sha256: hash(&super::encoding::encode(
                &(
                    &self.cfg.arena,
                    &self.cfg.match_,
                    &self.cfg.sensors,
                    &self.cfg.minds,
                ),
                1024 * 1024,
            )?),
            chassis: chassis.to_string(),
            startup_ms: self.cfg.match_.startup_ms,
            max_duration_ms: self.cfg.match_.max_duration_ms,
            native_substep_ms: native.ok_or_else(|| invalid("empty proving cadence"))?,
            candidate: selection(&recipe.candidate),
            opponent: selection(&recipe.opponent),
            pair_seeds: recipe.seeds.clone(),
        };
        context.validate()?;
        starts.context = Some(context);
        FrozenProving::prepare(
            Plan {
                registry: None,
                conduct: None,
                kind: WorkKind::Proving,
                scenario: recipe.scenario,
                opponent: recipe.opponent_kind,
                per_side: recipe.per_side,
                unit_count: u16::from(recipe.per_side) * 2,
                games: starts.games.len() as u16,
                seeds: recipe.seeds.clone(),
                limit_ms: self.cfg.match_.max_duration_ms,
                native_substep_ms: native.ok_or_else(|| invalid("empty proving batch"))?,
                start: Artifact {
                    sha256: [0; 32],
                    bytes: 1,
                },
                result_reservation_bytes: MAX_ARTIFACT_BYTES,
            },
            starts,
        )
    }
}
fn id(word: &str) -> Result<CanonicalId, BulkError> {
    CanonicalId::new(word).map_err(|e| invalid(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_atlas::{AtlasModule, AtlasState};
    use mush_content::{Appliers, ContentModule};
    use mush_testkit::TestWorld;

    #[test]
    fn m6_proving_frozen_openings_face_the_opposing_side() {
        let mut appliers = Appliers::new();
        mush_atlas::register_appliers(&mut appliers).unwrap();
        let atlas = mush_content::parse_pack(content::PROVING_GROUND_ATLAS_JSON).unwrap();
        let world = TestWorld::with_modules(vec![
            Box::new(AtlasModule::new()),
            Box::new(
                ContentModule::new()
                    .with_appliers(appliers)
                    .with_boot_packs(vec![atlas]),
            ),
        ]);
        let records = world
            .world()
            .module_state::<AtlasState>()
            .unwrap()
            .records
            .clone();
        let factory = ScenarioFactory::new(
            Arc::new(Catalog::build_game(&[], crate::NS).unwrap()),
            Arc::new(records),
            TacsimConfig::default(),
        );
        // Reopen actual compressed starts from the production factory. Both
        // paired sides and all shipped placement recipes must preserve facing;
        // checking only the eventual elevation would miss Pose::at's z argument.
        for scenario in [Scenario::Duel, Scenario::Muster, Scenario::Hold] {
            let recipe = Recipe::owned(
                &RollStore::default(),
                &crate::store::TacsimStore::default(),
                ObjId(99),
                "Standard/Line",
                "Standard/Line",
                scenario,
                1,
                10,
                1,
            )
            .unwrap();
            let mut prepared = factory.prepare(&recipe).unwrap();
            let starts = FrozenStarts::decode(prepared.bytes_mut().bytes()).unwrap();
            assert_eq!(starts.games.len(), 10);
            for index in 0..10 {
                let header = starts.header(index).unwrap();
                let arena = crate::minds::resimulation::open_frozen_header(&header).unwrap();
                assert_eq!(arena.units.len(), 2);
                for team in [PartyId::RED, PartyId::BLUE] {
                    let unit = arena.units.iter().find(|unit| unit.team == team).unwrap();
                    let expected = if team == PartyId::RED {
                        90_000
                    } else {
                        270_000
                    };
                    assert_eq!(
                        unit.pose.heading_mdeg, expected,
                        "{scenario:?} game{index} {team:?}"
                    );
                    assert_eq!(unit.desired_heading_mdeg, expected);
                    assert_eq!(unit.power, crate::arena::Power::Off);
                }
            }
        }
    }

    #[test]
    fn m6_frozen_v2_prepares_all_one_hundred_stock_muster_6v6_games_under_existing_cap() {
        // Real production content, atlas appliers, custody and ScenarioFactory;
        // this is exactly the public live-isolation request, without simulation.
        let mut appliers = Appliers::new();
        mush_atlas::register_appliers(&mut appliers).unwrap();
        let atlas = mush_content::parse_pack(content::PROVING_GROUND_ATLAS_JSON).unwrap();
        let world = TestWorld::with_modules(vec![
            Box::new(AtlasModule::new()),
            Box::new(
                ContentModule::new()
                    .with_appliers(appliers)
                    .with_boot_packs(vec![atlas]),
            ),
        ]);
        let records = world
            .world()
            .module_state::<AtlasState>()
            .unwrap()
            .records
            .clone();
        let factory = ScenarioFactory::new(
            Arc::new(Catalog::build_game(&[], crate::NS).unwrap()),
            Arc::new(records),
            TacsimConfig::default(),
        );
        let recipe = Recipe::owned(
            &RollStore::default(),
            &crate::store::TacsimStore::default(),
            ObjId(99),
            "Standard/Line",
            "Standard/Line",
            Scenario::Muster,
            6,
            100,
            1,
        )
        .unwrap();
        let mut prepared = factory
            .prepare(&recipe)
            .expect("advertised stock100 6v6 must prepare");
        assert_eq!(prepared.plan().games, 100);
        assert_eq!(prepared.plan().unit_count, 12);
        let bytes = prepared.bytes_mut().bytes();
        assert!(bytes.len() as u64 <= MAX_ARTIFACT_BYTES);
        let starts = FrozenStarts::decode(bytes).expect("stored body collection reopens");
        assert_eq!(starts.games.len(), 100);
        let packed = serde_json::to_value(&starts).unwrap();
        let mut raw_total = 0_u64;
        let mut largest_header = 0;
        for index in 0..100 {
            let header = starts.header(index).unwrap();
            assert_eq!(header.match_id, index as u64 + 1);
            assert_eq!(header.seals.len(), 12);
            assert!(
                header
                    .seals
                    .iter()
                    .all(|seal| seal.class == crate::arena::core::CoreClass::I
                        && seal.cards.selected.len() == 1
                        && seal.cards.selected.name(0) == Some("Line"))
            );
            raw_total += packed
                .get("games")
                .unwrap()
                .get(index)
                .unwrap()
                .get("raw_bytes")
                .unwrap()
                .as_u64()
                .unwrap();
            largest_header = largest_header.max(serde_json::to_vec(&header).unwrap().len());
            if matches!(index, 0 | 49 | 99) {
                let arena = crate::minds::resimulation::open_frozen_header(&header).unwrap();
                assert_eq!(arena.seed, index as u64 / 2 + 1);
                assert_eq!(arena.units.len(), 12);
                assert_eq!(
                    arena
                        .units
                        .iter()
                        .filter(|u| u.team == PartyId::RED)
                        .count(),
                    6
                );
                assert_eq!(
                    arena
                        .units
                        .iter()
                        .filter(|u| u.team == PartyId::BLUE)
                        .count(),
                    6
                );
            }
        }
        let repeated_format1_bytes = raw_total
            + serde_json::to_vec(packed.get("ground").unwrap())
                .unwrap()
                .len() as u64
            + serde_json::to_vec(packed.get("sources").unwrap())
                .unwrap()
                .len() as u64;
        assert!(
            repeated_format1_bytes > MAX_ARTIFACT_BYTES,
            "this fixture must exercise the formerly oversized raw collection"
        );
        eprintln!(
            "frozen_v2 stock100 muster6v6 artifact_bytes={} raw_game_bytes={} max_header_bytes={} cap={}",
            bytes.len(),
            raw_total,
            largest_header,
            MAX_ARTIFACT_BYTES
        );
    }
}
