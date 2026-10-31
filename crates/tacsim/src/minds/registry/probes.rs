//! The six single-picture and two full-density combat.v1 recipes. Their own fresh arena binds the
//! complete exact selected program and executes actual collection, settlement,
//! station landing and native evaluation. Multi-transition recipes live in the
//! same driver once their closed witness state machines are added; no absent
//! recipe may silently return a conformance row.
use super::{
    identity,
    manifest::FrozenPolicy,
    plan::{Case, Physics},
    runtime,
    subject::PreparedSubject,
};
use crate::{
    arena::{
        Arena, ArenaDefinition, ArenaTerrain, Classification, ContactId, Controller, CustodyOut,
        CustodyUnit, EntryZone, MatchMode, PartyId, Power, ShadowContact, TerrainCell, Waypoint,
        Waypoints, core::CoreClass, mind::SealEngine, motor::MotorCounters,
        profile_seal::SealedCards, state::SeenPose,
    },
    arena_task::MatchId,
    match_::{SealIdentity, SealTerms},
};
use mush_core::{CanonicalId, ObjId};
use mush_roll::commerce::rating::{
    Applicability, CaseKind, CaseOutcome, CaseResult, Coverage, ProbeName, RatingClass, RatingError,
};
use mush_sim::data::Pose;
use mush_tactics::{CellAddr, UnitMutationOp, UnitMutationPlan};
use smol_str::SmolStr;
use std::collections::BTreeMap;

const OWN: ObjId = ObjId(1);
const SEED: u64 = 0x7265_6769_7374_7279;
#[derive(Clone, Copy)]
enum Picture {
    Empty,
    Hostile,
    HurtTwo,
    OrderLoss,
    Density(usize),
}
#[derive(Clone, Copy)]
struct Recipe {
    picture: Picture,
    armed: bool,
}
impl Recipe {
    fn single(name: ProbeName, rows: usize) -> Result<Self, RatingError> {
        let (picture, armed) = match name {
            ProbeName::EmptyArmed => (Picture::Empty, true),
            ProbeName::EmptyUnarmed => (Picture::Empty, false),
            ProbeName::Hostile450Armed => (Picture::Hostile, true),
            ProbeName::Hostile450Unarmed => (Picture::Hostile, false),
            ProbeName::HurtTwoArmed => (Picture::HurtTwo, true),
            ProbeName::HurtTwoUnarmed => (Picture::HurtTwo, false),
            ProbeName::DensityArmed if (1..=32).contains(&rows) => (Picture::Density(rows), true),
            ProbeName::DensityUnarmed if (1..=32).contains(&rows) => {
                (Picture::Density(rows), false)
            }
            _ => return Err(RatingError::Incomplete),
        };
        Ok(Self { picture, armed })
    }
    const fn contacts(self) -> usize {
        match self.picture {
            Picture::Empty => 0,
            Picture::Hostile => 1,
            Picture::HurtTwo | Picture::OrderLoss => 2,
            Picture::Density(rows) => rows,
        }
    }
}
pub(super) struct Single {
    arena: Arena,
    case: Case,
    installed: runtime::Installed,
    recipe: Recipe,
    done: bool,
}
impl Single {
    pub(super) fn open(
        policy: &FrozenPolicy,
        subject: &PreparedSubject,
        case: &Case,
    ) -> Result<Self, RatingError> {
        let CaseKind::Probe { pair, name } = case.key.kind() else {
            return Err(RatingError::Invalid);
        };
        let class = core(case.key.class());
        let recipe = Recipe::single(name, policy.cfg.minds.classes.row(class).rows)?;
        let pair = subject
            .pairs()
            .get(usize::from(pair))
            .ok_or(RatingError::Identity)?;
        let variant = subject
            .variants()
            .get(usize::from(pair.variant))
            .ok_or(RatingError::Identity)?;
        let (mut arena, rom) = opening(policy, recipe, class)?;
        if physics(&arena)? != case.physics || case.maximum_thinks == 0 {
            return Err(RatingError::Identity);
        }
        let installed = runtime::install(
            &mut arena,
            subject,
            variant,
            pair.profile,
            class,
            &BTreeMap::from([(OWN, rom)]),
            1,
        )?;
        arena.traces.set_enabled(true);
        Ok(Self {
            arena,
            case: case.clone(),
            installed,
            recipe,
            done: false,
        })
    }
    pub(super) fn physics(
        policy: &FrozenPolicy,
        name: ProbeName,
        class: CoreClass,
    ) -> Result<Physics, RatingError> {
        let (arena, _) = opening(
            policy,
            Recipe::single(name, policy.cfg.minds.classes.row(class).rows)?,
            class,
        )?;
        physics(&arena)
    }
    pub(super) fn binding(&self) -> &mush_roll::commerce::rating::CaseBinding {
        &self.installed.binding
    }
    pub(super) fn advance(
        &mut self,
        allowance_ms: u64,
    ) -> Result<super::trials::Slice, RatingError> {
        if self.done || allowance_ms < u64::from(self.case.physics.native_ms) {
            return Err(RatingError::Invalid);
        }
        let mut counters = MotorCounters::default();
        let sequence = self
            .arena
            .minds
            .get(&OWN)
            .ok_or(RatingError::Incomplete)?
            .think_seq;
        self.arena.collect_think(OWN, sequence, 0, &mut counters);
        let think = self.arena.collected.pop().ok_or(RatingError::Incomplete)?;
        if !self.arena.collected.is_empty()
            || think.unit != OWN
            || think.t_ms != 0
            || think.lands_ms != i64::from(self.case.physics.native_ms)
        {
            return Err(RatingError::Identity);
        }
        // This is the actual captured public picture that the real runner will
        // read. Its observable facts, not hidden target rows, prove the recipe.
        let view = &think.view;
        if view.contacts.len() != self.recipe.contacts()
            || view.weapons.is_empty() == self.recipe.armed
            || view.power != Power::Online
        {
            return Err(RatingError::Incomplete);
        }
        let own_sections = view
            .sections
            .iter()
            .map(|row| (row.id.clone(), row.internals))
            .collect::<Vec<_>>();
        if matches!(self.recipe.picture, Picture::HurtTwo)
            && !own_sections
                .iter()
                .any(|(_, value)| value.is_some_and(|(now, full)| now > 0 && now < full))
        {
            return Err(RatingError::Incomplete);
        }
        if view.contacts.iter().any(|row| {
            row.friendly
                || !matches!(
                    row.class,
                    Classification::Classified {
                        party: PartyId::BLUE
                    }
                )
        }) {
            return Err(RatingError::Incomplete);
        }
        let facts = view
            .contacts
            .iter()
            .map(|c| (c.id, c.range_m, c.class, &c.odds, c.reads_dead))
            .collect::<Vec<_>>();
        if !matches!(self.recipe.picture, Picture::Empty)
            && !view.contacts.iter().any(|c| c.range_m == 450)
        {
            return Err(RatingError::Incomplete);
        }
        if matches!(self.recipe.picture, Picture::Density(_)) {
            let expected_odds = if self.recipe.armed {
                self.arena
                    .classes
                    .row(core(self.case.key.class()))
                    .odds_rows
                    .min(self.recipe.contacts())
            } else {
                0
            };
            let actual_odds = view
                .contacts
                .iter()
                .filter(|row| !row.odds.is_empty())
                .count();
            if actual_odds != expected_odds
                || (!self.recipe.armed && view.contacts.iter().any(|row| row.reads_dead.is_some()))
            {
                return Err(RatingError::Incomplete);
            }
        }
        let picture = identity(
            "tacsim.registry.single.picture.v1",
            &(
                view.t_ms,
                view.cell,
                &facts,
                &own_sections,
                view.weapons.len(),
            ),
            16 * 1024,
        )?;
        let answer = think.think().result;
        let settled = self.arena.settle_collected(&think, answer);
        self.arena.now_ms = think.lands_ms;
        self.arena
            .mind_land(OWN, think.think_seq, think.lands_ms, &mut counters);
        let measured = self.installed.meter.measurement()?;
        let budget = measured.summary.only_budget_faults();
        if measured.summary.thinks() != 1
            || (!measured.summary.clean() && !budget)
            || settled.overflows != 0
            || settled.dropped != 0
            || (!budget && settled.faults != 0)
        {
            return Err(RatingError::Invalid);
        }
        let state = self.arena.minds.get(&OWN).ok_or(RatingError::Incomplete)?;
        if state.dropped.is_some() || (!budget && state.strikes != 0) {
            return Err(RatingError::Invalid);
        }
        let trace = self
            .arena
            .traces
            .recent(OWN, Some(1))
            .pop()
            .ok_or(RatingError::Incomplete)?;
        if trace.think_seq != think.think_seq
            || !trace.settled
            || trace.diagnostics.omitted_intents() != 0
            || trace.rom
            || trace.rom_drop.is_some()
        {
            return Err(RatingError::Invalid);
        }
        if !budget && !matches!(trace.outcome, crate::arena::author_trace::Outcome::Applied) {
            return Err(RatingError::Incomplete);
        }
        let refusals = u32::try_from(
            trace
                .intents
                .iter()
                .filter(|i| {
                    matches!(
                        i.outcome,
                        crate::arena::author_trace::Outcome::Refused { .. }
                    )
                })
                .count(),
        )
        .map_err(|_| RatingError::Bound)?;
        let coverage = Coverage::new(
            if self.recipe.armed {
                Applicability::Armed
            } else {
                Applicability::Unarmed
            },
            1,
            1,
            identity(
                "tacsim.registry.single.coverage.v1",
                &(
                    picture,
                    measured.witness,
                    &measured.seen,
                    crate::arena::state_hash(&self.arena),
                ),
                16 * 1024,
            )?,
        )?;
        let result = CaseResult::new(
            self.case.key.clone(),
            self.installed.binding.clone(),
            measured.summary,
            coverage,
            refusals,
            if budget {
                CaseOutcome::BudgetExceeded {}
            } else {
                CaseOutcome::Conformant {}
            },
            None,
        )?;
        self.done = true;
        Ok(super::trials::Slice {
            simulated_ms: u64::from(self.case.physics.native_ms),
            result: Some(result),
        })
    }
}
/// Fresh real armed hull for the separately identified trusted engine probes.
/// It has no candidate invocation or invented saved source identity.
pub(super) fn kernel_opening(
    policy: &FrozenPolicy,
    class: CoreClass,
) -> Result<(Arena, SealedCards), RatingError> {
    opening(
        policy,
        Recipe {
            picture: Picture::Empty,
            armed: true,
        },
        class,
    )
}
fn opening(
    policy: &FrozenPolicy,
    recipe: Recipe,
    class: CoreClass,
) -> Result<(Arena, SealedCards), RatingError> {
    opening_on(policy, recipe, class, ground()?, [5000, 5000], 0)
}

/// The shipped ford, with an actually mobile hull whose weapon station is absent.
pub(super) fn ford_opening(
    policy: &FrozenPolicy,
    class: CoreClass,
) -> Result<(Arena, SealedCards), RatingError> {
    if !policy
        .ground
        .key_spots
        .iter()
        .any(|spot| spot.name == "The Ford" && (spot.x, spot.y) == (64, 97))
    {
        return Err(RatingError::Identity);
    }
    opening_on(
        policy,
        Recipe {
            picture: Picture::Empty,
            armed: false,
        },
        class,
        (*policy.ground).clone(),
        [60_500, 97_500],
        90_000,
    )
}
/// A last-known classified plot behind a fixed impassable ridge. Its actual
/// sensor expiry and actual fired shot establish loss and no-line; no target
/// truth is copied into a candidate view after this closed opening fixture.
pub(super) fn loss_opening(
    policy: &FrozenPolicy,
    class: CoreClass,
    armed: bool,
) -> Result<(Arena, SealedCards), RatingError> {
    let mut definition = ground()?;
    let mut cells = definition.terrain.cells().to_vec();
    for y in 0..64_usize {
        let cell = cells.get_mut(y * 64 + 6).ok_or(RatingError::Incomplete)?;
        // A 100 m ridge is representable in terrain i16 centimetres and
        // occludes the fixed ground-level hulls on either side.
        cell.elev_cm = 10_000;
        cell.move_cost = 0;
    }
    definition.id = "tacsim.arena.registry_order_loss".into();
    definition.terrain =
        ArenaTerrain::new(64, 64, 1000, cells).map_err(|_| RatingError::Invalid)?;
    let (mut arena, rom) = opening_on(
        policy,
        Recipe {
            picture: Picture::OrderLoss,
            armed,
        },
        class,
        definition,
        [5_500, 5_500],
        90_000,
    )?;
    // A real partial load, established before the private arena runs. The
    // immutable full-load denominators remain the actual hardware's capacity.
    // The recipe spends these rounds through ordinary accepted Seat shots.
    let own = arena.unit_mut(OWN).ok_or(RatingError::Incomplete)?;
    for count in own.ammo.values_mut() {
        *count = 1;
    }
    Ok((arena, rom))
}
fn opening_on(
    policy: &FrozenPolicy,
    recipe: Recipe,
    class: CoreClass,
    definition: ArenaDefinition,
    own_position: [i64; 2],
    heading_mdeg: u32,
) -> Result<(Arena, SealedCards), RatingError> {
    let terms = SealTerms {
        authored: None,
        selected_profiles: Vec::new(),
        classes: policy.cfg.minds.classes,
        rom_cfg: policy.cfg.arena.reflex.clone(),
        match_id: MatchId(1),
        principal: None,
        card: crate::arena::card::line(&policy.cfg.arena.reflex),
        odds_floor_permille: policy.cfg.minds.odds_floor_permille,
        engine_for_minds: SealEngine::Native,
        rated: CoreClass::Zero,
        class_cap: CoreClass::Zero,
        identity: SealIdentity::doctrine(),
        profiles: Vec::new(),
        fallback: None,
    };
    let mut own = hull(
        policy,
        OWN,
        PartyId::RED,
        own_position,
        class,
        matches!(recipe.picture, Picture::HurtTwo | Picture::OrderLoss),
        Controller::Mind,
        &terms,
    )?;
    own.pose.heading_mdeg = heading_mdeg;
    if !recipe.armed {
        own.weapons.clear();
        own.reseal(&terms).map_err(|_| RatingError::Invalid)?;
    }
    let rom = own.seal.rom.clone();
    let mut units = vec![own];
    for n in 0..recipe.contacts() {
        let position = if matches!(recipe.picture, Picture::OrderLoss) {
            [
                7_500,
                5_500 + i64::try_from(n).map_err(|_| RatingError::Bound)? * 2_000,
            ]
        } else if matches!(recipe.picture, Picture::Density(_)) {
            [
                50_000,
                5_000 + i64::try_from(n).map_err(|_| RatingError::Bound)? * 1_000,
            ]
        } else if n == 0 {
            [50_000, 5_000]
        } else {
            [50_000, 30_000]
        };
        units.push(hull(
            policy,
            ObjId(u32::try_from(n + 2).map_err(|_| RatingError::Bound)?),
            PartyId::BLUE,
            position,
            class,
            false,
            Controller::Pilot,
            &terms,
        )?);
    }
    let resolution = policy
        .catalog
        .resolutions
        .get(&id(crate::definitions::content::RESOLUTION)?)
        .cloned()
        .ok_or(RatingError::Unavailable)?;
    let mut arena = Arena::new_with_minds(
        CustodyOut {
            definition,
            units,
            pilots: Vec::new(),
            resolution,
            seed: SEED,
            mode: MatchMode::Skirmish,
            host: crate::arena::FieldHost::all_hostile(),
        },
        &policy.cfg.arena,
        &policy.cfg.match_,
        &policy.cfg.sensors,
        &policy.cfg.minds,
    )
    .map_err(|_| RatingError::Invalid)?;
    arena.clock.reanchor(0);
    for unit in &mut arena.units {
        unit.power = Power::Online;
    }
    for n in 0..recipe.contacts() {
        let target = ObjId(u32::try_from(n + 2).map_err(|_| RatingError::Bound)?);
        let state = arena.unit(target).ok_or(RatingError::Incomplete)?;
        let contact = ShadowContact::sighted(
            target,
            Classification::Classified { party: state.team },
            0,
            SeenPose::of(state),
        );
        arena.contacts.insert(
            OWN,
            ContactId(u64::try_from(n + 1).map_err(|_| RatingError::Bound)?),
            contact,
        );
    }
    Ok((arena, rom))
}
#[allow(clippy::too_many_arguments)] // fixed private recipe facts; no caller authority
fn hull(
    policy: &FrozenPolicy,
    object: ObjId,
    team: PartyId,
    position: [i64; 2],
    class: CoreClass,
    hurt: bool,
    controller: Controller,
    terms: &SealTerms,
) -> Result<CustodyUnit, RatingError> {
    let entry = id(if class == CoreClass::III {
        "tacsim.unit.heavy_mk1"
    } else {
        crate::definitions::content::LINE_TANK
    })?;
    let spec = policy
        .catalog
        .specs
        .get(&entry)
        .ok_or(RatingError::Unavailable)?;
    let binding = policy
        .catalog
        .bindings
        .get(&entry)
        .ok_or(RatingError::Unavailable)?;
    let cells = crate::definitions::to_tactical_unit(spec, binding, object)
        .map_err(|_| RatingError::Invalid)?;
    let ops = cells
        .cells()
        .filter_map(|(addr, value)| match addr {
            CellAddr::Section { cell, .. } if hurt && cell.as_str() == "internals" => {
                Some((addr, value))
            }
            CellAddr::Facet { cell, .. } if hurt && cell.as_str() == "armor" => Some((addr, value)),
            _ => None,
        })
        .map(|(addr, value)| UnitMutationOp::SetCell {
            addr: addr.clone(),
            value: value.min + (value.max - value.min) / 5,
        })
        .collect();
    let mutation = UnitMutationPlan::new(ops, None, Some("registry_picture".into()))
        .map_err(|_| RatingError::Invalid)?;
    let (cells, _) = mush_tactics::apply(&cells, &mutation, 0).map_err(|_| RatingError::Invalid)?;
    let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
        .map_err(|_| RatingError::Invalid)?;
    let [x, y] = position;
    crate::match_::custody_unit(
        &policy.catalog,
        cells,
        object,
        team,
        Pose::at(x, y, 0, posture),
        1,
        controller,
        0,
        terms,
    )
    .map_err(|_| RatingError::Invalid)
}
fn ground() -> Result<ArenaDefinition, RatingError> {
    let terrain = ArenaTerrain::uniform(
        64,
        64,
        1000,
        TerrainCell {
            class: mush_atlas::TerrainClass::Ground,
            elev_cm: 0,
            move_cost: 100,
        },
    )
    .map_err(|_| RatingError::Invalid)?;
    Ok(ArenaDefinition {
        id: "tacsim.arena.registry_pictures".into(),
        terrain,
        entry_zones: vec![
            EntryZone {
                team: PartyId::RED,
                x0: 0,
                y0: 0,
                x1: 4,
                y1: 63,
            },
            EntryZone {
                team: PartyId::BLUE,
                x0: 59,
                y0: 0,
                x1: 63,
                y1: 63,
            },
        ],
        key_spots: Vec::new(),
        waypoints: Waypoints::new(
            vec![
                Waypoint {
                    id: "west".into(),
                    x: 5,
                    y: 5,
                },
                Waypoint {
                    id: "east".into(),
                    x: 50,
                    y: 5,
                },
            ],
            &[(SmolStr::new_static("west"), SmolStr::new_static("east"))],
        ),
        modes: Vec::new(),
        homes: Vec::new(),
    })
}
fn physics(arena: &Arena) -> Result<Physics, RatingError> {
    Ok(Physics {
        units: u16::try_from(arena.units.len()).map_err(|_| RatingError::Bound)?,
        native_ms: u32::try_from(arena.substep_len_ms()).map_err(|_| RatingError::Bound)?,
    })
}
fn core(class: RatingClass) -> CoreClass {
    match class {
        RatingClass::I => CoreClass::I,
        RatingClass::II => CoreClass::II,
        RatingClass::III => CoreClass::III,
    }
}
fn id(word: &str) -> Result<CanonicalId, RatingError> {
    CanonicalId::new(word).map_err(|_| RatingError::Invalid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_roll::{Envelope, Revision};
    type TestResult = Result<(), Box<dyn std::error::Error>>;
    #[test]
    fn order_loss_ridge_is_representable_impassable_and_occludes_the_fixed_hulls() -> TestResult {
        use mush_atlas::{Blocker, Eye, Sight};
        let policy = super::super::tests::frozen_policy()?;
        for armed in [false, true] {
            let (arena, _) = loss_opening(&policy, CoreClass::I, armed)?;
            let terrain = &arena.definition.terrain;
            let from = terrain
                .cell_of(&arena.unit(OWN).ok_or("own hull")?.pose)
                .ok_or("own cell")?;
            let to = terrain
                .cell_of(&arena.unit(ObjId(2)).ok_or("target hull")?.pose)
                .ok_or("target cell")?;
            check_eq!((from, to), ((5, 5), (7, 5)));
            let ridge = terrain.get(6, 5).ok_or("ridge")?;
            check_eq!((ridge.elev_cm, ridge.move_cost), (10_000, 0));
            check_eq!(
                terrain.sight(Eye::standing(from), Eye::standing(to)).sight,
                Sight::BlockedBy(Blocker::Elevation {
                    at: (6, 5),
                    elev_cm: 10_000
                })
            );
        }
        Ok(())
    }

    #[test]
    fn all_six_actual_pictures_run_saved_fire_program_including_inert_unarmed_rows() -> TestResult {
        let policy = super::super::tests::frozen_policy()?;
        let source = "fn think(v,c) { stock::rules(); helm(90000); this.seen = 1; }";
        let revision = super::super::tests::revision(source)?;
        let mut envelope: Envelope = revision.envelope().clone();
        let fire = crate::minds::workshop::author_rules::parse("FIRE ALL WHEN odds >= 60", 100)?;
        envelope.rules = serde_json::json!({"Alpha":[fire],"Beta":[]});
        let mut encoded = serde_json::to_value(&revision)?;
        *encoded.get_mut("envelope").ok_or("envelope")? = serde_json::to_value(&envelope)?;
        *encoded
            .get_mut("reference")
            .and_then(|r| r.get_mut("digest"))
            .ok_or("digest")? = serde_json::to_value(envelope.digest(source)?)?;
        let revision: Revision = serde_json::from_value(encoded)?;
        let subject = PreparedSubject::prepare(&revision, source, &policy.cfg.arena)?;
        for name in [
            ProbeName::EmptyArmed,
            ProbeName::EmptyUnarmed,
            ProbeName::Hostile450Armed,
            ProbeName::Hostile450Unarmed,
            ProbeName::HurtTwoArmed,
            ProbeName::HurtTwoUnarmed,
        ] {
            let case = Case {
                key: mush_roll::commerce::rating::CaseKey::new(
                    RatingClass::I,
                    CaseKind::Probe { pair: 0, name },
                )?,
                physics: Single::physics(&policy, name, CoreClass::I)?,
                maximum_ms: 5000,
                maximum_thinks: 1,
            };
            let mut probe = Single::open(&policy, &subject, &case)?;
            let program = &probe.arena.profiles.get(&OWN).ok_or("runtime")?.program;
            check!(
                !program.rows.is_empty(),
                "an absent station never erases the authored program"
            );
            check!(
                program
                    .rows
                    .iter()
                    .all(|row| row.inert != probe.recipe.armed)
            );
            let result = probe
                .advance(u64::from(case.physics.native_ms))?
                .result
                .ok_or("actual row")?;
            check!(matches!(result.outcome(), CaseOutcome::Conformant {}));
            check_eq!(result.meter().thinks(), 1);
            check!(result.meter().max_fees() > crate::minds::fees::FEE_THINK);
            check!(result.coverage().complete());
            let state = probe.arena.minds.get(&OWN).ok_or("state")?;
            let memory: serde_json::Value = serde_json::from_slice(&state.memory)?;
            check_eq!(
                memory.get("seen").and_then(serde_json::Value::as_u64),
                Some(1)
            );
            check_eq!(state.strikes, 0);
            check!(state.dropped.is_none());
            check!(
                probe.advance(u64::from(case.physics.native_ms)).is_err(),
                "single case cannot be counted twice"
            );
        }
        Ok(())
    }
    #[test]
    fn wider_public_odds_follow_effective_seal_and_remain_cockpit_equal() -> TestResult {
        let policy = super::super::tests::frozen_policy()?;
        let source = "fn think(v,c) { stock::rules(); }";
        let revision = super::super::tests::revision(source)?;
        let subject = PreparedSubject::prepare(&revision, source, &policy.cfg.arena)?;
        for (class, expected) in [
            (RatingClass::I, 2),
            (RatingClass::II, 4),
            (RatingClass::III, 8),
        ] {
            let name = ProbeName::DensityArmed;
            let case = Case {
                key: mush_roll::commerce::rating::CaseKey::new(
                    class,
                    CaseKind::Probe { pair: 0, name },
                )?,
                physics: Single::physics(&policy, name, core(class))?,
                maximum_ms: 20_000,
                maximum_thinks: 16,
            };
            let mut probe = Single::open(&policy, &subject, &case)?;
            let frame = probe
                .arena
                .frame_for(OWN, probe.arena.clock.tempo())
                .ok_or("frame")?;
            let view = probe.arena.mind_view(OWN).ok_or("view")?;
            check_eq!(
                view.contacts.len(),
                policy.cfg.minds.classes.row(core(class)).rows
            );
            check_eq!(
                view.contacts.iter().filter(|c| !c.odds.is_empty()).count(),
                expected
            );
            for row in &view.contacts {
                let public = frame
                    .odds
                    .iter()
                    .filter(|o| o.contact == row.id)
                    .map(|o| (o.weapon.clone(), o.p_permille))
                    .collect::<Vec<_>>();
                check_eq!(row.odds, public);
            }
            // Authored declaration remains I; the fitted/capped effective seal
            // controls the public projection, never source metadata.
            check_eq!(subject.subject().declared(), RatingClass::I);
            check_eq!(
                probe
                    .advance(u64::from(case.physics.native_ms))?
                    .result
                    .ok_or("density result")?
                    .meter()
                    .thinks(),
                1
            );
            if class == RatingClass::III {
                probe.arena.unit_mut(OWN).ok_or("hull")?.seal.class = CoreClass::I;
                check_eq!(
                    probe
                        .arena
                        .frame_odds(OWN)
                        .iter()
                        .map(|r| r.contact)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    2
                );
                probe.arena.unit_mut(OWN).ok_or("hull")?.seal.class = CoreClass::III;
                probe.arena.minds.get_mut(&OWN).ok_or("mind")?.dropped =
                    Some(crate::arena::mind::DropCause::Strikes);
                check_eq!(probe.arena.mind_class(OWN), CoreClass::Zero);
                check_eq!(
                    probe
                        .arena
                        .frame_odds(OWN)
                        .iter()
                        .map(|r| r.contact)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    2
                );
                probe.arena.minds.get_mut(&OWN).ok_or("mind")?.dropped = None;
                // Configured projection limits remain authoritative as well.
                probe.arena.classes =
                    serde_json::from_value(serde_json::json!({"III":{"odds_rows":3}}))?;
                check_eq!(
                    probe
                        .arena
                        .frame_odds(OWN)
                        .iter()
                        .map(|r| r.contact)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    3
                );
                check_eq!(
                    probe
                        .arena
                        .mind_view(OWN)
                        .ok_or("custom view")?
                        .contacts
                        .iter()
                        .filter(|c| !c.odds.is_empty())
                        .count(),
                    3
                );
                probe.arena.classes = policy.cfg.minds.classes;
                let saved_unit = probe.arena.unit(OWN).ok_or("hull")?.unit.clone();
                let key = saved_unit
                    .capabilities()
                    .find(|key| key.id.as_str() == crate::definitions::unit::CORE_CAPABILITY)
                    .cloned()
                    .ok_or("fitted core")?;
                let plan = UnitMutationPlan::new(
                    vec![UnitMutationOp::RevokeCapability { key }],
                    None,
                    Some("registry_projection_test".into()),
                )?;
                let (damaged, _) = mush_tactics::apply(&saved_unit, &plan, 0)?;
                probe.arena.unit_mut(OWN).ok_or("hull")?.unit = damaged;
                check_eq!(probe.arena.mind_class(OWN), CoreClass::Zero);
                check_eq!(
                    probe
                        .arena
                        .frame_odds(OWN)
                        .iter()
                        .map(|r| r.contact)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    2
                );
                probe.arena.unit_mut(OWN).ok_or("hull")?.unit = saved_unit;
                probe.arena.unit_mut(OWN).ok_or("hull")?.seal = crate::arena::mind::SealRef::none();
                check_eq!(probe.arena.mind_class(OWN), CoreClass::Zero);
                check_eq!(
                    probe
                        .arena
                        .frame_odds(OWN)
                        .iter()
                        .map(|r| r.contact)
                        .collect::<std::collections::BTreeSet<_>>()
                        .len(),
                    2
                );
            }
        }
        Ok(())
    }
}
