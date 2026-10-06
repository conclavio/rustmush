//! **The stock mind's content, compiled in** (17b cut M2.5; doc 17 §11
//! "Standard Doctrine", §13 "The digest", §17 "Three columns, two gates",
//! §22's M2 italics; D-UM-48).
//!
//! T2 of doc 17 §17's three columns: Standard Doctrine written in Rhai, the
//! combat role's stock mind. Two files under `crates/tacsim/content/minds/`,
//! `include_str!`ed here the way `crate::definitions::content` compiles the
//! game's packs in:
//!
//! * `standard.rhai` — the script: `crates/tacsim/src/arena/stock_mind.rs`'
//!   `decide_on_card` ported arm for arm onto the TacSim API ([`super`]),
//!   under the sandbox's binding limits (`switch` dispatch on the state and
//!   the objective, one helper `fn` per state arm and per T1 helper). **17b
//!   cut M2.5 ports the helm half** and the lock; **17b cut M2.6 the fire
//!   half** (`arming_for`'s slots on change with the slot number excluded,
//!   `engage_orders`' citation beside a fresh lock, the stands' calls).
//! * `standard.json` — its manifest ([`MindManifest`]): the name, **the role
//!   (`combat`, doc 17 §22's italic line: the manifest declares it)**, the
//!   API and stock versions it was written against, the class it is rated
//!   for and that class's op ceiling.
//!
//! **The digest** ([`STANDARD_SHA256`], doc 17 §13): the SHA-256 of the
//! script's bytes, lowercase, pinned by `crates/tacsim/tests/content.rs`, so
//! a change to the script is a reviewed diff of that string. It is the
//! compiled script's cache key ([`standard_digest`], `mush_mind::Sandbox`'s
//! `(Digest, abi)`). The persisted source store a sealed core names its mind
//! by — `tacsim:mindsrc:<sha256>` — is 17b cut M3's; nothing here writes it.
//!
//! **The runner** ([`standard_runner`]): a [`RhaiRunner`] over this script
//! on the process's one sandbox. A shipped script that does not compile is a
//! loud error for the caller ([`CompileError`]) — never a silent `NoMind`.
//! Nothing of production names it in this cut: `Arena::new` still installs
//! `default_mind_runner`, the native stock mind (T1); the fixtures and the
//! twin lanes set it.

use std::sync::Arc;

use mush_mind::{CompileError, Digest};
use serde::Deserialize;
use sha2::{Digest as _, Sha256};

use super::RhaiRunner;
use crate::arena::mind::MindRunner;
use crate::config::ArenaConfig;

/// **Standard Doctrine's source** (`content/minds/standard.rhai`).
pub const STANDARD_RHAI: &str = include_str!("../../content/minds/standard.rhai");

/// **Standard Doctrine's manifest** (`content/minds/standard.json`).
pub const STANDARD_MANIFEST_JSON: &str = include_str!("../../content/minds/standard.json");

/// **The SHA-256 of [`STANDARD_RHAI`]'s bytes**, 64 lowercase characters
/// (doc 17 §13) — pinned by `tests/content.rs`'s
/// `m2_the_standard_doctrine_source_is_pinned`, so a script change is a
/// reviewed re-bless of this string. **Re-blessed by 17b cut M2.6** with the
/// fire half written (`arming_for`, `aspect_mask`, `engage_orders`' citation
/// and the emission's `fire`) AND four cost-only rewrites of M2.5's helm half
/// (`read_notices`' kind read as one `switch`, `hostile_edges`' leash radii
/// read only under a leash, `stand` and `withdraw_stand` reading the think's
/// `p.dry`, `engage_orders` reading `p.dry` and the held row's range inline),
/// behaviour unchanged and pinned by Gate A on every lane; M2.5's helm half
/// was `95ee741e…0802ecd`.
pub const STANDARD_SHA256: &str =
    "8b3ec78c9d957865c8d6d7696fbf422ca8939d05a51370befcff621287d74996";

/// **The script's digest** — the SHA-256 of [`STANDARD_RHAI`], the key the
/// sandbox caches its compilation under (with `MIND_ABI`). Computed from the
/// bytes, so a stale [`STANDARD_SHA256`] never keys a changed script: the
/// pin test holds the two equal.
#[must_use]
pub fn standard_digest() -> Digest {
    Digest(Sha256::digest(STANDARD_RHAI.as_bytes()).into())
}

/// **A stock mind's manifest** (doc 17 §11, §13, §22's italics):
/// `standard.json`, decoded strictly.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MindManifest {
    /// The mind's name, as the card and the trace call it.
    pub name: String,
    /// The role it is the stock mind of (doc 17 §22: `combat`).
    pub role: String,
    /// The TacSim API version it was written against (`API_VERSION`).
    pub api: u32,
    /// The stock content version (`STOCK_VERSION`).
    pub stock: u32,
    /// The class it is rated for (D-UM-35's row: `I`).
    pub rated_class: String,
    /// That class's op ceiling per think (D-UM-35's `ops` column: 1,500).
    pub ops_ceiling: u64,
}

/// **Standard Doctrine's manifest, decoded** ([`STANDARD_MANIFEST_JSON`]).
///
/// # Errors
///
/// The decoder's error for a manifest that is not exactly the six fields —
/// a shipped file, so `tests/content.rs` holds it `Ok` and pins the values.
pub fn standard_manifest() -> Result<MindManifest, serde_json::Error> {
    serde_json::from_str(STANDARD_MANIFEST_JSON)
}

/// **A runner flying Standard Doctrine in Rhai** (T2): [`RhaiRunner::new`]
/// over [`STANDARD_RHAI`] and [`standard_digest`] on the match's doctrine
/// `cfg` (the stand-in a dropped hull falls back to, and `decide_ms()`).
///
/// # Errors
///
/// [`CompileError`] when the shipped script does not compile — loudly, for
/// the caller: a broken stock mind never flies as `NoMind`.
pub fn standard_runner(cfg: &ArenaConfig) -> Result<Arc<dyn MindRunner>, CompileError> {
    let runner = RhaiRunner::new(cfg, STANDARD_RHAI, standard_digest())?;
    Ok(Arc::new(runner))
}

/// **`stock_mind.rs`' unit fixtures, mirrored through both runners** (17b
/// cut M2.5; 17b §2's M2.5 item 4).
///
/// Here and not in `tests/minds_rhai.rs` because the fixtures build their
/// fields on the arena library's own test support (`test_support`'s 56-cell
/// window, its stock `line_tank` and `scout`), which only the crate's own
/// tests can reach: each mirror is the native fixture's body with its
/// `think` the TWIN's — the SAME job handed to `NativeStockMind` and to the
/// Rhai Standard Doctrine, T2's fault refused first, then the intents and
/// the memory bytes held equal, and T1's answer handed back so the native
/// fixture's assertions on the outcome — its intents and its memory — stand
/// beside the comparison. One is not carried over: `emission_holds` (the
/// emission order's check) is private to `stock_mind.rs`' tests, and the T1
/// answer it checks is the one the native fixture checks there. **The whole
/// intent list is compared, the fire half with it** (17b cut M2.6: the
/// slots, the disarm and the citation are ported, so the `Condition`/`Fire`
/// filter M2.5 held on an armed hull is gone). The Hold fixtures are not
/// mirrored: no Hold objective reaches a production think (the view carries
/// no match mode; M4).
///
/// Beside the mirrors, five fixtures of the twin's own hold the arms the
/// lanes reach seldom or never under Gate A: the order cap off the obeyed
/// order, the withdraw dwell and the rally (with a card's `rally_ms` 0, which
/// never rallies), the `Disabled` edge, the card's own terms (`prefer`'s
/// three weight triples, the presets' bands, `search_ms` and `hunt_after_ms`
/// 0 — T1 reads them through `StockCard::as_reflex`, T2 off the card, and
/// only a card that is not Line tells the two derivations apart), and the
/// script's clamps where T1 saturates.
///
/// **17b cut M2.6** adds the four M1b.2 fixtures of `stock_mind.rs` (the fire
/// half's): `arming_for`'s terms through T2's OWN `arming_for` beside T1's
/// on one view and card (`arming`, over the shipped script with a `think`
/// that calls it alone), and the admitting ceiling, the citation and the
/// removed-weapon residual through both runners on one job — here and not in
/// `tests/minds_rhai.rs` for the reason above.
#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use mush_core::ObjId;
    use smol_str::SmolStr;

    use super::*;
    use crate::arena::card::{StockCard, line, preset};
    use crate::arena::conditions::Arming;
    use crate::arena::damage::BLOCKED;
    use crate::arena::intents::{IntentKind, MotorGoal};
    use crate::arena::mind::{
        Memory, MindRunner, MindView, ThinkJob, ThinkResult, decode_memory, encode_memory,
    };
    use crate::arena::motor::fallback_waypoint;
    use crate::arena::perception::{Aspect, DriveOutcome, FrameNotice, FrameOrder, MindObjective};
    use crate::arena::reflex::{REFLEX_STAND_TICKS, ReflexState};
    use crate::arena::state::{
        Arena, ArenaDefinition, ArenaTerrain, Classification, ContactId, Controller, CustodyOut,
        CustodyUnit, MatchMode, Power, SeenPose, ShadowContact, Team,
    };
    use crate::arena::stock_mind::{NativeStockMind, StockMemory, health_permille, view_ranges};
    use crate::arena::test_support::{definition, ground, line_tank, pose, resolution, scout};
    use crate::config::{MatchConfig, SensorsConfig};
    use crate::deliberation::order::RangeBand;

    const RED: ObjId = ObjId(4);
    const BLUE: ObjId = ObjId(9);
    const BLUE2: ObjId = ObjId(11);
    /// A friendly line tank, the supporter's charge.
    const MATE: ObjId = ObjId(12);

    /// `stock_mind.rs`' `field`: the test definition's graph and zones on a
    /// 56-cell window of ground.
    fn field(units: Vec<CustodyUnit>) -> Arena {
        Arena::new(
            CustodyOut {
                definition: ArenaDefinition {
                    terrain: ArenaTerrain::uniform(56, 56, 1_000, ground())
                        .unwrap_or_else(|error| panic!("the field: {error:?}")),
                    ..definition()
                },
                units,
                pilots: Vec::new(),
                resolution: resolution(),
                seed: 2_350,
                mode: MatchMode::Skirmish,
            },
            &ArenaConfig::default(),
            &MatchConfig::default(),
            &SensorsConfig::default(),
        )
        .unwrap_or_else(|error| panic!("the field arena: {error:?}"))
    }

    /// A line tank flown by a mind.
    fn minded(id: ObjId, team: Team, x: i64, y: i64) -> CustodyUnit {
        let mut unit = line_tank(id, team, pose(x, y));
        unit.controller = Controller::Mind;
        unit
    }

    /// `viewer` holds `target` under `id`, classified, plotted where it stands.
    fn hold(arena: &mut Arena, viewer: ObjId, target: ObjId, id: ContactId) {
        let Some(state) = arena.unit(target) else {
            panic!("a fielded target");
        };
        let class = Classification::Classified { team: state.team };
        let plot = SeenPose::of(state);
        arena
            .contacts
            .insert(viewer, id, ShadowContact::sighted(target, class, 0, plot));
    }

    /// The hull's power, written.
    fn online(arena: &mut Arena, unit: ObjId) {
        if let Some(state) = arena.unit_mut(unit) {
            state.power = Power::Online;
        }
    }

    /// The view `unit`'s next think would read, off a clone.
    fn view_of(arena: &Arena, unit: ObjId) -> MindView {
        arena
            .clone()
            .mind_view(unit)
            .unwrap_or_else(|| panic!("#{} has a view", unit.0))
    }

    /// A standing order of `objective` for `red-1`, a minute long.
    fn order_of(objective: MindObjective) -> FrameOrder {
        FrameOrder {
            squad: SmolStr::new("red-1"),
            objective,
            hold_fire: false,
            max_range_band: None,
            expires_ms: 60_000,
        }
    }

    /// A patrolling memory with nothing seen and the hunt's clock just set.
    fn patrolling(t_ms: i64) -> StockMemory {
        StockMemory {
            state: ReflexState::Patrol,
            unheld_since_ms: Some(t_ms),
            ..StockMemory::default()
        }
    }

    /// Whether an intent is the fire half's.
    fn fire_half(kind: &IntentKind) -> bool {
        matches!(kind, IntentKind::Condition(_) | IntentKind::Fire { .. })
    }

    /// **Gate A on one job**: T2's fault refused first (with its trace), then
    /// its WHOLE intent list T1's — the fire half included (17b cut M2.6) —
    /// then its memory bytes T1's.
    fn compare(job: &ThinkJob<'_>, native: &ThinkResult, rhai: &ThinkResult) {
        assert_eq!(
            rhai.fault, None,
            "T2 faulted; trace {:#?}; view {:#?}",
            rhai.trace, job.view
        );
        assert_eq!(
            rhai.intents,
            native.intents,
            "the intents differ at t {} from {:?}",
            job.t_ms,
            StockMemory::from(&decode_memory(job.memory))
        );
        assert_eq!(
            String::from_utf8_lossy(&rhai.memory),
            String::from_utf8_lossy(&native.memory),
            "the memory bytes differ at t {}",
            job.t_ms
        );
    }

    /// The Rhai Standard Doctrine on the arena's doctrine.
    fn rhai_on(arena: &Arena) -> RhaiRunner {
        RhaiRunner::new(&arena.arena_cfg, STANDARD_RHAI, standard_digest())
            .unwrap_or_else(|error| panic!("Standard Doctrine compiles: {error}"))
    }

    /// **The mirrored `think`**: `stock_mind.rs`' (one think from `memory`
    /// on `view` at `t_ms`, landing one substep later, on Line, no Hold
    /// objective) as a job both runners think on, compared ([`compare`]);
    /// T1's intents answered and its memory written back.
    fn think(
        arena: &Arena,
        view: &MindView,
        memory: &mut StockMemory,
        t_ms: i64,
    ) -> Vec<IntentKind> {
        think_on(arena, view, memory, t_ms, &line(&arena.arena_cfg.reflex))
    }

    /// [`think`] on `card` (T1 on `card.as_reflex`, T2 reading the card).
    fn think_on(
        arena: &Arena,
        view: &MindView,
        memory: &mut StockMemory,
        t_ms: i64,
        card: &StockCard,
    ) -> Vec<IntentKind> {
        let view = Arc::new(view.clone());
        let bytes = encode_memory(&Memory::from(memory.clone()));
        let card = card.clone();
        let job = ThinkJob {
            unit: ObjId(view.unit),
            think_seq: 1,
            t_ms,
            lands_ms: t_ms.saturating_add(arena.substep_len_ms()),
            seed: arena.seed,
            dropped: false,
            view: &view,
            memory: &bytes,
            map: &arena.definition,
            card: &card,
        };
        let native = NativeStockMind::with(&arena.arena_cfg.reflex).think(&job);
        let rhai = rhai_on(arena).think(&job);
        compare(&job, &native, &rhai);
        *memory = StockMemory::from(&decode_memory(&native.memory));
        native.intents
    }

    /// Mirror of `m1a6_a_drive_no_route_notice_performs_the_give_ups_and_the_fallback`.
    #[test]
    fn m2_mirror_a_drive_no_route_notice_performs_the_give_ups_and_the_fallback() {
        let mut arena = field(vec![minded(RED, Team::Red, 3_500, 4_500)]);
        online(&mut arena, RED);
        let failed = (6, 4);
        let mut view = view_of(&arena, RED);
        view.notices = vec![FrameNotice::Drive {
            outcome: DriveOutcome::NoRoute,
        }];
        let start = StockMemory {
            state: ReflexState::Patrol,
            goal: Some(failed),
            waypoint: Some(1),
            last_plot: Some((failed, 9_000)),
            hunt: Some(failed),
            unheld_since_ms: Some(0),
            ..StockMemory::default()
        };
        assert_eq!(
            fallback_waypoint(
                &arena.definition,
                (3, 4),
                Some(failed),
                Some(1),
                None,
                RED.0
            ),
            (Some(0), Some(1), Some((1, 4)))
        );
        let mut memory = start.clone();
        let intents = think(&arena, &view, &mut memory, 10_000);
        assert_eq!(
            intents,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 1, y: 4 })]
        );
        assert_eq!(memory.last_plot, None);
        assert_eq!(memory.hunt, None);
        assert_eq!(memory.unheld_since_ms, Some(10_000));
        assert_eq!((memory.waypoint, memory.from_waypoint), (Some(0), Some(1)));
        assert_eq!(memory.goal, Some((1, 4)));
        assert_eq!(memory.ignore_until_ms, None);
        view.order = Some(order_of(MindObjective::Advance { to: failed }));
        let mut ordered = start;
        let _ = think(&arena, &view, &mut ordered, 10_000);
        assert_eq!(ordered.ignore_until_ms, Some(60_000));
        let mut quiet = StockMemory {
            state: ReflexState::Patrol,
            goal: Some((1, 4)),
            waypoint: Some(0),
            unheld_since_ms: Some(0),
            ..StockMemory::default()
        };
        assert_eq!(
            think(&arena, &view_of(&arena, RED), &mut quiet, 10_000),
            Vec::<IntentKind>::new()
        );
    }

    /// Mirror of `m1a6_a_drive_arrived_notice_is_arrival_wherever_the_fan_landed`.
    #[test]
    fn m2_mirror_a_drive_arrived_notice_is_arrival_wherever_the_fan_landed() {
        let mut arena = field(vec![minded(RED, Team::Red, 3_500, 4_500)]);
        online(&mut arena, RED);
        let named = (50, 40);
        let start = StockMemory {
            state: ReflexState::Patrol,
            goal: Some(named),
            hunt: Some(named),
            waypoint: Some(1),
            from_waypoint: Some(0),
            unheld_since_ms: Some(0),
            ..StockMemory::default()
        };
        let mut view = view_of(&arena, RED);
        let mut on = start.clone();
        assert_eq!(
            think(&arena, &view, &mut on, 5_000),
            Vec::<IntentKind>::new()
        );
        assert_eq!(on.hunt, Some(named));
        view.notices = vec![FrameNotice::Drive {
            outcome: DriveOutcome::Arrived,
        }];
        let mut arrived = start;
        let intents = think(&arena, &view, &mut arrived, 5_000);
        assert_eq!(arrived.hunt, None);
        assert_eq!(arrived.unheld_since_ms, Some(5_000));
        assert_eq!(arrived.waypoint, Some(0), "the nearest node, west");
        assert_eq!(
            intents,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 1, y: 4 })]
        );
    }

    /// **Both runners on one arena, compared on every think** — T1's answer
    /// flown (the stepping mirror's runner).
    #[derive(Debug)]
    struct Both {
        native: NativeStockMind,
        rhai: RhaiRunner,
        answers: Mutex<Vec<(MindView, ThinkResult)>>,
    }

    impl MindRunner for Both {
        fn think(&self, job: &ThinkJob<'_>) -> ThinkResult {
            let native = self.native.think(job);
            let rhai = self.rhai.think(job);
            compare(job, &native, &rhai);
            if let Ok(mut answers) = self.answers.lock() {
                answers.push(((**job.view).clone(), native.clone()));
            }
            native
        }
    }

    /// Mirror of `m1a6_an_unarmed_hull_emits_no_lock_and_no_fire`: a
    /// weaponless mind with two classified hostiles in its sight, stepped a
    /// scripted minute with both runners on every think.
    #[test]
    fn m2_mirror_an_unarmed_hull_emits_no_lock_and_no_fire() {
        let mut red = scout(RED, Team::Red, pose(20_500, 20_500));
        red.weapons.clear();
        red.controller = Controller::Mind;
        let mut arena = field(vec![
            red,
            line_tank(BLUE, Team::Blue, pose(20_500, 30_500)),
            line_tank(BLUE2, Team::Blue, pose(30_500, 20_500)),
        ]);
        let both = Arc::new(Both {
            native: NativeStockMind::with(&arena.arena_cfg.reflex),
            rhai: rhai_on(&arena),
            answers: Mutex::new(Vec::new()),
        });
        arena.mind_runner = both.clone();
        let mut wall = 0_i64;
        while arena.now_ms < 60_000 {
            let _ = arena.step(wall, &[], &[]);
            wall += 250;
        }
        let answers = both
            .answers
            .lock()
            .map(|answers| answers.clone())
            .unwrap_or_default();
        assert!(answers.len() >= 55, "{} thinks", answers.len());
        assert!(
            answers
                .iter()
                .any(|(_, result)| result.intents.contains(&IntentKind::Startup))
        );
        assert!(answers.iter().any(|(view, _)| {
            view.contacts
                .iter()
                .any(|row| matches!(row.class, Classification::Classified { team: Team::Blue }))
        }));
        assert!(
            answers.iter().any(|(_, result)| result
                .intents
                .iter()
                .any(|kind| matches!(kind, IntentKind::Goal(MotorGoal::Cell { .. })))),
            "it drives"
        );
        for (view, result) in &answers {
            assert!(view.weapons.is_empty());
            assert!(
                view.contacts
                    .iter()
                    .all(|row| row.odds.is_empty() && row.reads_dead.is_none())
            );
            assert!(
                !result
                    .intents
                    .iter()
                    .any(|kind| fire_half(kind) || matches!(kind, IntentKind::Lock { .. }))
            );
        }
    }

    /// Mirror of `m1a6_a_healthy_withdraw_with_no_order_returns_to_patrol`.
    #[test]
    fn m2_mirror_a_healthy_withdraw_with_no_order_returns_to_patrol() {
        let mut arena = field(vec![minded(RED, Team::Red, 20_500, 20_500)]);
        online(&mut arena, RED);
        let view = view_of(&arena, RED);
        let withdrawing = StockMemory {
            state: ReflexState::Withdraw,
            target: Some(ContactId(0x11)),
            ..StockMemory::default()
        };
        let mut expired = withdrawing.clone();
        let _ = think(&arena, &view, &mut expired, 1_000);
        assert_eq!(expired.state, ReflexState::Patrol);
        assert_eq!(expired.target, None);
        let mut ordered_view = view.clone();
        ordered_view.order = Some(order_of(MindObjective::Withdraw));
        let mut ordered = withdrawing.clone();
        let _ = think(&arena, &ordered_view, &mut ordered, 1_000);
        assert_eq!(ordered.state, ReflexState::Withdraw);
        let mut hurt_view = view;
        for section in &mut hurt_view.sections {
            if let Some((_, max)) = section.internals {
                section.internals = Some((0, max));
            }
            for facet in &mut section.facets {
                facet.armor = 0;
            }
        }
        assert!(health_permille(&hurt_view) < arena.arena_cfg.reflex.withdraw_permille);
        let mut hurt = withdrawing;
        let _ = think(&arena, &hurt_view, &mut hurt, 1_000);
        assert_eq!(hurt.state, ReflexState::Withdraw);
    }

    /// Mirror of `m1a6_a_supporter_stands_blind_rejoins_off_the_leash_and_keeps_station`
    /// (an armed hull: the arming and the shot compared with the rest).
    #[test]
    fn m2_mirror_a_supporter_stands_blind_rejoins_off_the_leash_and_keeps_station() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 30_500)),
            line_tank(MATE, Team::Red, pose(20_500, 50_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x11));
        let view = view_of(&arena, RED);
        let mut blind_view = view.clone();
        blind_view.order = Some(order_of(MindObjective::Support { of: None }));
        let mut blind = patrolling(0);
        let stood = think(&arena, &blind_view, &mut blind, 1_000);
        assert!(
            matches!(
                stood.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { .. },
                    IntentKind::Lock { .. },
                    IntentKind::Fire { slot: Some(1), .. }
                ]
            ),
            "{stood:?}"
        );
        assert_eq!(blind.target, None);
        assert_eq!(blind.state, ReflexState::Patrol);
        hold(&mut arena, RED, MATE, ContactId(0x99));
        let mut leashed_view = view_of(&arena, RED);
        leashed_view.order = Some(order_of(MindObjective::Support {
            of: Some(ContactId(0x99)),
        }));
        let mut leashed = StockMemory {
            state: ReflexState::Engage,
            target: Some(ContactId(0x11)),
            ..patrolling(0)
        };
        let back = think(&arena, &leashed_view, &mut leashed, 1_000);
        assert!(
            matches!(back.as_slice(), [IntentKind::Goal(MotorGoal::Cell { y, .. })] if *y > 20),
            "{back:?}"
        );
        assert_eq!(leashed.state, ReflexState::Patrol);
        assert_eq!(leashed.target, None);
        let mut close = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(MATE, Team::Red, pose(20_500, 23_500)),
        ]);
        online(&mut close, RED);
        hold(&mut close, RED, MATE, ContactId(0x99));
        let mut station_view = view_of(&close, RED);
        station_view.order = leashed_view.order.clone();
        let mut station = patrolling(0);
        let parked = think(&close, &station_view, &mut station, 1_000);
        assert_eq!(parked, vec![IntentKind::Goal(MotorGoal::Hold)]);
    }

    /// Mirror of `m1a6_a_screen_stands_on_a_row_inside_its_arc_and_not_outside`
    /// (the lock half, the arming and the shot).
    #[test]
    fn m2_mirror_a_screen_stands_on_a_row_inside_its_arc_and_not_outside() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 30_500)),
            line_tank(BLUE2, Team::Blue, pose(30_500, 20_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE2, ContactId(0x22));
        let mut view = view_of(&arena, RED);
        view.order = Some(order_of(MindObjective::Screen { facing_mdeg: 0 }));
        let mut memory = patrolling(0);
        let outside = think(&arena, &view, &mut memory, 1_000);
        assert_eq!(
            outside,
            vec![
                IntentKind::Goal(MotorGoal::Hold),
                IntentKind::Heading { mdeg: 0 }
            ]
        );
        assert_eq!(memory.target, None);
        assert_eq!(memory.state, ReflexState::Patrol);
        hold(&mut arena, RED, BLUE, ContactId(0x11));
        let mut view = view_of(&arena, RED);
        view.order = Some(order_of(MindObjective::Screen { facing_mdeg: 0 }));
        let mut memory = patrolling(0);
        let inside = think(&arena, &view, &mut memory, 1_000);
        assert!(
            matches!(
                inside.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { mdeg: 0 },
                    IntentKind::Lock { contact },
                    IntentKind::Fire { slot: Some(1), .. }
                ] if *contact == ContactId(0x11)
            ),
            "{inside:?}"
        );
        assert_eq!(memory.state, ReflexState::Patrol);
        assert_eq!(memory.last_plot.map(|(cell, _)| cell), Some((20, 30)));
    }

    /// Two hostile rows a view holds, tied (`stock_mind.rs`' `tied`).
    fn tied() -> (Arena, MindView) {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 30_500)),
            line_tank(BLUE2, Team::Blue, pose(30_500, 20_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x30));
        hold(&mut arena, RED, BLUE2, ContactId(0x10));
        let mut view = view_of(&arena, RED);
        for row in &mut view.contacts {
            row.range_m = 100;
            row.aspect = Aspect::Front;
        }
        (arena, view)
    }

    /// Mirror of `m1a6_an_engage_hint_outranks_a_better_score`.
    #[test]
    fn m2_mirror_an_engage_hint_outranks_a_better_score() {
        let (arena, mut view) = tied();
        for row in &mut view.contacts {
            if row.id == ContactId(0x10) {
                row.aspect = Aspect::Rear;
            }
        }
        let mut free = patrolling(0);
        let _ = think(&arena, &view, &mut free, 1_000);
        assert_eq!(free.target, Some(ContactId(0x10)));
        view.order = Some(order_of(MindObjective::Engage {
            target: Some(ContactId(0x30)),
        }));
        let mut told = patrolling(0);
        let intents = think(&arena, &view, &mut told, 1_000);
        assert_eq!(told.target, Some(ContactId(0x30)));
        assert!(intents.contains(&IntentKind::Lock {
            contact: ContactId(0x30)
        }));
    }

    /// Mirror of `m1a6_a_blocked_miss_notice_turns_engage_into_approach`
    /// (the lock half).
    #[test]
    fn m2_mirror_a_blocked_miss_notice_turns_engage_into_approach() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let mut view = view_of(&arena, RED);
        let ranges = view_ranges(&view, arena.arena_cfg.reflex.engage_band_permille, None);
        let between_m = (ranges.engage_cm + ranges.release_cm) / 200;
        for row in &mut view.contacts {
            row.range_m = between_m;
        }
        let engaged = StockMemory {
            state: ReflexState::Engage,
            target: Some(ContactId(0x77)),
            ..patrolling(0)
        };
        let mut standing = engaged.clone();
        let stand = think(&arena, &view, &mut standing, 1_000);
        assert_eq!(standing.state, ReflexState::Engage);
        assert!(stand.contains(&IntentKind::Goal(MotorGoal::Hold)));
        view.notices = vec![FrameNotice::Miss {
            contact: None,
            reason: SmolStr::new(BLOCKED),
        }];
        let mut blocked = engaged;
        let closed = think(&arena, &view, &mut blocked, 1_000);
        assert_eq!(blocked.state, ReflexState::Approach);
        assert!(!blocked.no_line);
        assert!(
            matches!(
                closed.as_slice(),
                [IntentKind::Goal(MotorGoal::Cell { .. })]
            ),
            "{closed:?}"
        );
    }

    /// **The order cap off the OBEYED order** (17b §5's feas F6; the brief's
    /// order-cap fixture): an armed red with a hostile 300 m out — inside the
    /// uncapped engage band, outside a `close` cap's — under an order that
    /// carries `max_range_band: close`:
    ///
    /// * an early-ended `Support` whose charge's row reads dead, and
    /// * an `Engage` whose deadline is the memory's `ignore_until_ms`,
    ///
    /// are each ended or ignored, so T1's ranges are UNCAPPED (`obeyed` is
    /// `None`) and it engages; the same `Engage` OBEYED (no
    /// `ignore_until_ms`) is capped and T1 keeps approaching — so the arm the
    /// `obey` local selects decides the think, and both runners agree on
    /// every one of the three, intent for intent (the fire half filtered)
    /// and byte for byte.
    #[test]
    fn m2_mirror_an_ignored_or_ended_order_does_not_cap_the_ranges() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 50_500)),
            line_tank(MATE, Team::Red, pose(30_500, 20_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        hold(&mut arena, RED, MATE, ContactId(0x99));
        let base = view_of(&arena, RED);
        let capped = |objective: MindObjective| FrameOrder {
            max_range_band: Some(RangeBand::Close),
            ..order_of(objective)
        };
        // The precondition: 300 m is inside the uncapped engage band and
        // outside the capped one.
        let band = arena.arena_cfg.reflex.engage_band_permille;
        let open = view_ranges(&base, band, None);
        let order = capped(MindObjective::Engage { target: None });
        let shut = view_ranges(&base, band, Some(&order));
        assert!(
            shut.engage_cm < 30_000 && 30_000 <= open.engage_cm,
            "{shut:?} {open:?}"
        );
        // (1) an early-ended Support: its charge's row reads dead.
        let mut ended_view = base.clone();
        ended_view.order = Some(capped(MindObjective::Support {
            of: Some(ContactId(0x99)),
        }));
        for row in &mut ended_view.contacts {
            if row.id == ContactId(0x99) {
                row.reads_dead = Some(true);
            }
        }
        let mut ended = patrolling(0);
        let first = think(&arena, &ended_view, &mut ended, 1_000);
        assert_eq!(ended.ignore_until_ms, Some(60_000), "the Support ended");
        assert_eq!(ended.state, ReflexState::Engage, "uncapped: {first:?}");
        // (2) an Engage whose deadline the memory already ignores.
        let mut ignored_view = base.clone();
        ignored_view.order = Some(order.clone());
        let mut ignored = StockMemory {
            ignore_until_ms: Some(60_000),
            ..patrolling(0)
        };
        let second = think(&arena, &ignored_view, &mut ignored, 1_000);
        assert_eq!(ignored.state, ReflexState::Engage, "uncapped: {second:?}");
        // (3) the same Engage obeyed: capped, still approaching.
        let mut obeyed = patrolling(0);
        let third = think(&arena, &ignored_view, &mut obeyed, 1_000);
        assert_eq!(obeyed.state, ReflexState::Approach, "capped: {third:?}");
        assert_ne!(third, second, "the cap decides the think");
    }

    /// `view` hurt past every card's `withdraw_permille`: every section's
    /// internals and every facet's plate at nought (the healthy-withdraw
    /// mirror's hurt view).
    fn hurt(view: &MindView) -> MindView {
        let mut hurt = view.clone();
        for section in &mut hurt.sections {
            if let Some((_, max)) = section.internals {
                section.internals = Some((0, max));
            }
            for facet in &mut section.facets {
                facet.armor = 0;
            }
        }
        hurt
    }

    /// **The withdraw dwell and the rally** (16b cut Q.7; the twin's own
    /// fixture — the lanes withdraw seldom and rally less): a hurt red home
    /// at its goal, a hostile 50 m off with a line, on each card below:
    ///
    /// 1. cornered: the stand on the row (`Hold`, the facing, the lock) and
    ///    the dwell `stand_until_ms = t + stand_ticks() × decide_ms()`;
    /// 2. a think later, the row gone: still dwelling, standing;
    /// 3. at the dwell's end: the dwell spent and the quiet clock stamped;
    /// 4. one millisecond short of `rally_ms` after it: still withdrawing;
    /// 5. at `rally_ms` exactly (`>=`): `Patrol`, `rallied`, the clock spent —
    ///    on Line (20 s) and on a card whose `rally_s` is 3; a card whose
    ///    `rally_ms` is 0 (Rearguard, Sentry) never rallies, an hour on.
    ///
    /// Every think is the twin's: both runners equal intent for intent (the
    /// fire half filtered) and byte for byte.
    #[test]
    fn m2_mirror_the_withdraw_dwell_and_the_rally() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 25_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x11));
        let cornered = hurt(&view_of(&arena, RED));
        assert!(health_permille(&cornered) < arena.arena_cfg.reflex.withdraw_permille);
        let mut quiet = cornered.clone();
        quiet.contacts.clear();
        let cfg = arena.arena_cfg.reflex.clone();
        let mut quick = line(&cfg);
        quick
            .set("rally_s", "3")
            .unwrap_or_else(|error| panic!("rally_s 3: {error}"));
        let never = |name: &str| preset(name, &cfg).unwrap_or_else(|| panic!("{name}"));
        let home = (20, 20);
        let dwell = REFLEX_STAND_TICKS * i64::from(cfg.decide_ms);
        for (card, rallies) in [
            (line(&cfg), true),
            (quick, true),
            (never("rearguard"), false),
            (never("sentry"), false),
        ] {
            let rally_ms = i64::from(card.rally_ms);
            let mut memory = StockMemory {
                state: ReflexState::Withdraw,
                goal: Some(home),
                ..StockMemory::default()
            };
            // 1. Cornered.
            let stand = think_on(&arena, &cornered, &mut memory, 1_000, &card);
            assert!(
                stand.contains(&IntentKind::Goal(MotorGoal::Hold))
                    && stand.contains(&IntentKind::Lock {
                        contact: ContactId(0x11)
                    }),
                "{stand:?}"
            );
            assert_eq!(memory.state, ReflexState::Withdraw);
            assert_eq!(memory.stand_until_ms, Some(1_000 + dwell));
            assert_eq!(memory.quiet_since_ms, None);
            // 2. Dwelling, the row gone.
            memory.goal = Some(home);
            let dwelling = think_on(&arena, &quiet, &mut memory, 1_000 + dwell - 1, &card);
            assert_eq!(dwelling, vec![IntentKind::Goal(MotorGoal::Hold)]);
            assert_eq!(memory.stand_until_ms, Some(1_000 + dwell));
            // 3. The dwell spent; the quiet clock stamped.
            let quiet_at = 1_000 + dwell;
            memory.goal = Some(home);
            let _ = think_on(&arena, &quiet, &mut memory, quiet_at, &card);
            assert_eq!(memory.stand_until_ms, None);
            assert_eq!(memory.quiet_since_ms, Some(quiet_at));
            assert_eq!(memory.state, ReflexState::Withdraw);
            // 4. One millisecond short of the rally (the card's `rally_ms`
            //    0 is never; an hour stands for it).
            let due = if rally_ms == 0 {
                quiet_at + 3_600_000
            } else {
                quiet_at + rally_ms
            };
            memory.goal = Some(home);
            let mut short = memory.clone();
            let _ = think_on(&arena, &quiet, &mut short, due - 1, &card);
            assert_eq!(short.state, ReflexState::Withdraw);
            assert!(!short.rallied);
            // 5. At it.
            let _ = think_on(&arena, &quiet, &mut memory, due, &card);
            if rallies {
                assert_eq!(memory.state, ReflexState::Patrol, "{card:?}");
                assert!(memory.rallied);
                assert_eq!(memory.quiet_since_ms, None);
            } else {
                assert_eq!(memory.state, ReflexState::Withdraw, "{card:?}");
                assert!(!memory.rallied);
                assert_eq!(memory.quiet_since_ms, Some(quiet_at));
            }
        }
    }

    /// **The `Disabled` edge rewrites the fourteen keys** (the twin's own
    /// fixture — a wreck thinks no more, so a lane reaches the edge only on
    /// an immobile unarmed hull): from a memory with every key set, an
    /// immobile view with no weapon row, and a wreck's view with its weapon
    /// rows, each answer nothing and leave `Disabled` over thirteen defaults
    /// — T2's bytes T1's.
    #[test]
    fn m2_mirror_the_disabled_edge_rewrites_the_fourteen_keys() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 25_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x11));
        let view = view_of(&arena, RED);
        let busy = StockMemory {
            state: ReflexState::Engage,
            target: Some(ContactId(0x11)),
            goal: Some((20, 25)),
            waypoint: Some(1),
            from_waypoint: Some(0),
            last_plot: Some(((20, 25), 900)),
            unheld_since_ms: Some(100),
            quiet_since_ms: Some(200),
            stand_until_ms: Some(3_000),
            hold_retry_ms: Some(400),
            rallied: true,
            hunt: Some((40, 40)),
            no_line: true,
            ignore_until_ms: Some(60_000),
        };
        let mut immobile = view.clone();
        immobile.immobile = true;
        immobile.weapons.clear();
        let mut wreck = view;
        wreck.wreck = true;
        assert!(!wreck.weapons.is_empty());
        for disabled in [immobile, wreck] {
            let mut memory = busy.clone();
            let intents = think(&arena, &disabled, &mut memory, 1_000);
            assert_eq!(intents, Vec::<IntentKind>::new());
            assert_eq!(
                memory,
                StockMemory {
                    state: ReflexState::Disabled,
                    ..StockMemory::default()
                }
            );
        }
    }

    /// **The card's own terms reach T2 as `StockCard::as_reflex` writes them
    /// for T1** (17b cut M1b.5; D-UM-43; the twin's own fixture — every lane
    /// flies Line, the identity anchor, so only a card that is not Line tells
    /// the script's derivation from T1's apart):
    ///
    /// * **`prefer`**: on two hostile rows whose ranges and aspects sweep a
    ///   grid, each of the five words and the six presets (their bands among
    ///   them) settles on the row T1 settles on — and `nearest` and `exposed`
    ///   part somewhere on the grid, so a wrong triple has a think to part on;
    /// * **`search_ms`**: a plot exactly `search_ms` old is searched and one a
    ///   millisecond older is dropped; a card whose `search_s` is 0 drops it;
    /// * **`hunt_after_ms`**: the hunt starts at exactly `hunt_after_ms`
    ///   unheld and not a millisecond before; Sentry's 0 never hunts.
    #[test]
    fn m2_mirror_the_card_reaches_t2_as_as_reflex_writes_it() {
        let (arena, base) = tied();
        let cfg = arena.arena_cfg.reflex.clone();
        let mut cards: Vec<StockCard> = Vec::new();
        for word in ["nearest", "exposed", "hurt", "balanced", "best_odds"] {
            let mut card = line(&cfg);
            card.set("prefer", word)
                .unwrap_or_else(|error| panic!("prefer {word}: {error}"));
            cards.push(card);
        }
        for name in [
            "line",
            "sentry",
            "sniper",
            "skirmisher",
            "hunter",
            "rearguard",
        ] {
            cards.push(preset(name, &cfg).unwrap_or_else(|| panic!("{name}")));
        }
        let mut parted = false;
        for near_m in [50_u64, 150, 400] {
            for far_m in [100_u64, 300, 900] {
                for aspect in [Aspect::Front, Aspect::Left, Aspect::Rear] {
                    let mut view = base.clone();
                    for row in &mut view.contacts {
                        if row.id == ContactId(0x10) {
                            row.range_m = near_m;
                        } else {
                            row.range_m = far_m;
                            row.aspect = aspect;
                        }
                    }
                    let mut picked: Vec<Option<ContactId>> = Vec::new();
                    for card in &cards {
                        let mut memory = patrolling(0);
                        let _ = think_on(&arena, &view, &mut memory, 1_000, card);
                        picked.push(memory.target);
                    }
                    parted |= picked.first() != picked.get(1);
                }
            }
        }
        assert!(parted, "nearest and exposed never parted on the grid");

        // The search and the hunt, alone on the field.
        let mut alone = field(vec![minded(RED, Team::Red, 20_500, 20_500)]);
        online(&mut alone, RED);
        let view = view_of(&alone, RED);
        let line_card = line(&cfg);
        let search_ms = i64::from(line_card.search_ms);
        let plotted = |at_ms: i64| StockMemory {
            last_plot: Some(((40, 40), at_ms)),
            ..patrolling(0)
        };
        let t = 100_000;
        let mut kept = plotted(t - search_ms);
        let searched = think_on(&alone, &view, &mut kept, t, &line_card);
        assert_eq!(
            searched,
            vec![IntentKind::Goal(MotorGoal::Cell { x: 40, y: 40 })]
        );
        let mut stale = plotted(t - search_ms - 1);
        let _ = think_on(&alone, &view, &mut stale, t, &line_card);
        assert_eq!(stale.last_plot, None);
        let mut off = line(&cfg);
        off.set("search_s", "0")
            .unwrap_or_else(|error| panic!("search_s 0: {error}"));
        let mut unsearched = plotted(t);
        let _ = think_on(&alone, &view, &mut unsearched, t, &off);
        assert_eq!(unsearched.last_plot, None);
        let hunt_after = i64::from(line_card.hunt_after_ms);
        let mut early = patrolling(0);
        let _ = think_on(&alone, &view, &mut early, hunt_after - 1, &line_card);
        assert_eq!(early.hunt, None);
        let mut due = patrolling(0);
        let _ = think_on(&alone, &view, &mut due, hunt_after, &line_card);
        assert!(due.hunt.is_some(), "{due:?}");
        let sentry = preset("sentry", &cfg).unwrap_or_else(|| panic!("sentry"));
        let mut never = patrolling(0);
        let _ = think_on(&alone, &view, &mut never, 10 * hunt_after, &sentry);
        assert_eq!(never.hunt, None);
    }

    /// **Where T1 saturates, T2 clamps** (the brief's `saturating_*` rule; the
    /// script head's "Saturation"): a Rhai INT overflow is a fault and a
    /// strike, so each site T1 saturates on answers T1's answer at the
    /// extremes an INT can hold — a row whose range reads `u64::MAX` metres
    /// (the getter's `i64::MAX`), and memory clocks at `i64::MIN` for the
    /// search's plot, the hunt's unheld clock and the rally's quiet clock.
    /// Before the clamps each of these thinks faulted `type` (an arithmetic
    /// overflow); now each is T1's, intent for intent and byte for byte.
    #[test]
    fn m2_mirror_where_t1_saturates_t2_clamps() {
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let mut far = view_of(&arena, RED);
        for row in &mut far.contacts {
            row.range_m = u64::MAX;
        }
        for state in [ReflexState::Approach, ReflexState::Engage] {
            let mut memory = StockMemory {
                state,
                target: Some(ContactId(0x77)),
                ..patrolling(0)
            };
            let _ = think(&arena, &far, &mut memory, 1_000);
            assert_eq!(memory.state, ReflexState::Approach);
        }
        let mut alone = field(vec![minded(RED, Team::Red, 20_500, 20_500)]);
        online(&mut alone, RED);
        let view = view_of(&alone, RED);
        let mut plotted = StockMemory {
            last_plot: Some(((40, 40), i64::MIN)),
            unheld_since_ms: Some(i64::MIN),
            ..patrolling(0)
        };
        let _ = think(&alone, &view, &mut plotted, 1_000);
        assert_eq!(plotted.last_plot, None, "the plot is past any search_ms");
        let mut quiet = StockMemory {
            state: ReflexState::Withdraw,
            goal: Some((20, 20)),
            quiet_since_ms: Some(i64::MIN),
            ..StockMemory::default()
        };
        let _ = think(&alone, &hurt(&view), &mut quiet, 1_000);
        assert_eq!(quiet.state, ReflexState::Patrol, "past any rally_ms");
        assert!(quiet.rallied);
    }

    // -----------------------------------------------------------------------
    // 17b cut M2.6: `stock_mind.rs`' four M1b.2 fixtures, mirrored
    // -----------------------------------------------------------------------

    /// **The Standard Doctrine's `arming_for` alone, as a think** (17b cut
    /// M2.6): the shipped script with its entry renamed (`standard_think`)
    /// and a `think` that calls the script's own `arming_for(view, card,
    /// this.ceiling)` — the memory carries the ceiling, an INT or `()` — so
    /// T1's free `arming_for` and T2's are held equal on one view and card
    /// ([`arming`]). 31 functions, under the 32.
    fn arming_alone() -> RhaiRunner {
        use sha2::{Digest as _, Sha256};
        let source = format!(
            "{}\nfn think(view, card) {{\n    arming_for(view, card, this.ceiling);\n}}\n",
            STANDARD_RHAI.replacen(
                "fn think(view, card) {",
                "fn standard_think(view, card) {",
                1
            )
        );
        let digest = Digest(Sha256::digest(source.as_bytes()).into());
        RhaiRunner::new(&crate::config::ArenaConfig::default(), &source, digest)
            .unwrap_or_else(|error| panic!("the arming script compiles: {error}"))
    }

    /// **T1's `arming_for` and T2's on one view** — the native function's
    /// answer and the script's own `arming_for` ([`arming_alone`]) on the
    /// same view, card and ceiling, held equal (T2's fault refused first);
    /// the native answer handed back.
    fn arming(
        arena: &Arena,
        view: &MindView,
        card: &StockCard,
        ceiling: Option<u64>,
    ) -> Vec<IntentKind> {
        use crate::arena::mind::MemValue;
        use crate::arena::stock_mind::arming_for;
        let native = arming_for(view, card, ceiling);
        let mut memory = Memory::new();
        memory.insert(
            "ceiling".to_owned(),
            ceiling.map_or(MemValue::Unit, |cm| {
                MemValue::Int(i64::try_from(cm).unwrap_or(i64::MAX))
            }),
        );
        let bytes = encode_memory(&memory);
        let view = Arc::new(view.clone());
        let job = ThinkJob {
            unit: ObjId(view.unit),
            think_seq: 1,
            t_ms: 1_000,
            lands_ms: 1_000_i64.saturating_add(arena.substep_len_ms()),
            seed: arena.seed,
            dropped: false,
            view: &view,
            memory: &bytes,
            map: &arena.definition,
            card,
        };
        let rhai = arming_alone().think(&job);
        assert_eq!(
            rhai.fault, None,
            "T2's arming_for faulted: {:?}",
            rhai.trace
        );
        assert_eq!(rhai.intents, native, "T2's arming_for on {ceiling:?}");
        native
    }

    /// `stock_mind.rs`' `row_like`: a weapon row like `like` under another id,
    /// reach and heat.
    fn row_like(
        like: &crate::arena::weapons::FrameWeapon,
        id: &str,
        reach_cm: u32,
        heat_per_shot: u32,
    ) -> crate::arena::weapons::FrameWeapon {
        crate::arena::weapons::FrameWeapon {
            id: SmolStr::new(id),
            bands_cm: vec![reach_cm / 2, reach_cm],
            heat_per_shot,
            ..like.clone()
        }
    }

    /// The `FireCondition` an `Arm` carries, or a panic.
    fn armed_terms(kind: &IntentKind) -> crate::arena::conditions::FireCondition {
        match kind {
            IntentKind::Condition(Arming::Arm(cond)) => cond.clone(),
            other => panic!("not an arming: {other:?}"),
        }
    }

    /// The one `Disarm { None }`.
    fn disarm() -> IntentKind {
        IntentKind::Condition(Arming::Disarm { weapon: None })
    }

    /// Mirror of `m1b2_arming_for_arms_the_first_two_weapons_whatever_the_heat`
    /// (17b cut M2.6): every `arming_for` of the native fixture through T2's
    /// own `arming_for` beside T1's ([`arming`]) — the hot laser armed with
    /// the cannon (no heat skip), the reserve as whole rounds, the reach
    /// under the ceiling, nothing said with nothing armed, one disarm beside
    /// an armed row, nothing on an unarmed hull — and once more on every
    /// preset, whose reserves and aspects Line's do not reach.
    #[test]
    fn m2_mirror_arming_for_arms_the_first_two_weapons_whatever_the_heat() {
        use crate::arena::conditions::{
            ASPECTS_ANY, FireCondition, STOCK_CONDITION_SLOTS, SlotTarget,
        };
        use crate::arena::stock_mind::would_overheat;
        use crate::arena::weapons::FrameArming;
        let mut arena = field(vec![minded(RED, Team::Red, 20_500, 20_500)]);
        online(&mut arena, RED);
        let mut view = view_of(&arena, RED);
        let Some(like) = view.weapons.first().cloned() else {
            panic!("the line tank's cannon row");
        };
        view.weapons = vec![
            row_like(&like, "laser", 60_000, 20),
            row_like(&like, "cannon", 250_000, 0),
            row_like(&like, "missiles", 300_000, 0),
        ];
        view.heat = Some(crate::arena::step::FrameHeat { heat: 60, max: 60 });
        let laser = view
            .weapons
            .first()
            .cloned()
            .unwrap_or_else(|| like.clone());
        assert!(would_overheat(&view, &laser), "the precondition: hot");
        let mut card = line(&arena.arena_cfg.reflex);
        card.reserve_pct = 10;
        let arms = arming(&arena, &view, &card, Some(80_000));
        assert_eq!(arms.len(), STOCK_CONDITION_SLOTS, "{arms:?}");
        let terms: Vec<FireCondition> = arms.iter().map(armed_terms).collect();
        assert_eq!(
            terms
                .iter()
                .map(|cond| (
                    cond.weapon.as_str(),
                    cond.target,
                    cond.min_rounds_left,
                    cond.max_range_m
                ))
                .collect::<Vec<_>>(),
            vec![
                ("laser", SlotTarget::BestHostile, like.rounds_max / 10, 600),
                ("cannon", SlotTarget::BestHostile, like.rounds_max / 10, 800),
            ]
        );
        let on_line = arming(&arena, &view, &line(&arena.arena_cfg.reflex), Some(80_000));
        assert!(
            on_line
                .iter()
                .map(armed_terms)
                .all(|cond| cond.min_permille == 100
                    && cond.aspects == ASPECTS_ANY
                    && cond.min_rounds_left == 0)
        );
        let line_card = line(&arena.arena_cfg.reflex);
        assert_eq!(arming(&arena, &view, &line_card, None), Vec::new());
        if let (Some(row), Some(cond)) = (view.weapons.get_mut(1), on_line.get(1).map(armed_terms))
        {
            row.armed = Some(FrameArming::of(2, &cond));
        }
        assert_eq!(arming(&arena, &view, &line_card, None), vec![disarm()]);
        // Every preset's discipline, armed fresh and against the slot shown
        // on Line's terms (each term but the number compared).
        for name in crate::arena::card::PRESETS {
            let Some(preset) = preset(name, &arena.arena_cfg.reflex) else {
                panic!("the preset {name}");
            };
            let _ = arming(&arena, &view, &preset, Some(80_000));
            let _ = arming(&arena, &view, &preset, Some(30_000));
        }
        view.weapons.clear();
        assert_eq!(arming(&arena, &view, &line_card, Some(80_000)), Vec::new());
        assert_eq!(
            arming(&arena, &view, &line_card, None),
            Vec::new(),
            "D-UM-48"
        );
    }

    /// Mirror of `m1b2_t1_arms_on_the_admitting_ceiling_and_cites_only_beside_a_fresh_lock`
    /// (17b cut M2.6): every think of the native fixture through both
    /// runners ([`think`]) — found: arm, lock, cite S1; held: arm and stand;
    /// a cycling gun: arm and lock; the view showing the slot as wanted: no
    /// `Condition`; on other terms: one `Arm`; `hold_fire`: the lock, and a
    /// disarm only beside an armed row; `Starting`: likewise; a withdraw
    /// stand en route arming on `stand_cm`.
    #[test]
    #[allow(clippy::too_many_lines)]
    fn m2_mirror_t1_arms_on_the_admitting_ceiling_and_cites_only_beside_a_fresh_lock() {
        use crate::arena::conditions::SlotTarget;
        use crate::arena::weapons::FrameArming;
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let view = view_of(&arena, RED);
        let cfg = &arena.arena_cfg.reflex;
        let ranges = view_ranges(&view, cfg.engage_band_permille, None);
        let reach = view
            .weapons
            .first()
            .and_then(|weapon| weapon.bands_cm.last().copied())
            .map_or(0, u64::from);
        let release_m = u32::try_from(ranges.release_cm.min(reach) / 100).unwrap_or(u32::MAX);
        let mut fight = StockMemory::default();
        let found = think(&arena, &view, &mut fight, 1_000);
        assert_eq!(fight.state, ReflexState::Engage);
        let [arm, lock, fire] = found.as_slice() else {
            panic!("{found:?}");
        };
        assert_eq!(armed_terms(arm).max_range_m, release_m);
        assert_eq!(armed_terms(arm).target, SlotTarget::BestHostile);
        assert_eq!(
            lock,
            &IntentKind::Lock {
                contact: ContactId(0x77)
            }
        );
        assert_eq!(
            fire,
            &IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: Some(1)
            }
        );
        let mut held_view = view.clone();
        held_view.lock = Some(ContactId(0x77));
        let mut holding = fight.clone();
        let stands = think(&arena, &held_view, &mut holding, 2_000);
        assert!(
            matches!(
                stands.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { .. }
                ]
            ),
            "{stands:?}"
        );
        let mut cycling = view.clone();
        for weapon in &mut cycling.weapons {
            weapon.ready = false;
            weapon.ready_in_ms = 4_000;
        }
        let locked = think(&arena, &cycling, &mut StockMemory::default(), 1_000);
        assert!(
            matches!(
                locked.as_slice(),
                [
                    IntentKind::Condition(Arming::Arm(_)),
                    IntentKind::Lock { .. }
                ]
            ),
            "{locked:?}"
        );
        let mut shown = held_view.clone();
        if let Some(row) = shown.weapons.first_mut() {
            row.armed = Some(FrameArming::of(1, &armed_terms(arm)));
        }
        let quietly = think(&arena, &shown, &mut fight.clone(), 2_000);
        assert!(
            matches!(
                quietly.as_slice(),
                [
                    IntentKind::Goal(MotorGoal::Hold),
                    IntentKind::Heading { .. }
                ]
            ),
            "{quietly:?}"
        );
        let mut narrower = shown.clone();
        if let Some(armed) = narrower
            .weapons
            .first_mut()
            .and_then(|row| row.armed.as_mut())
        {
            armed.max_range_m = armed.max_range_m.saturating_sub(100);
        }
        let rearmed = think(&arena, &narrower, &mut fight.clone(), 2_000);
        assert_eq!(rearmed.first().map(armed_terms), Some(armed_terms(arm)));
        // Every other term of the shown slot is compared too (the script's
        // `armed.target != "best"` and `armed.min_range_m != 0`, which no
        // stock hull's own arming reaches): a slot a pilot armed on the
        // lock, or with a floor of range, is re-armed on the wanted terms.
        for (term, edit) in [
            (
                "a lock target",
                (|armed: &mut FrameArming| armed.target = SlotTarget::Lock) as fn(&mut FrameArming),
            ),
            ("a contact target", |armed: &mut FrameArming| {
                armed.target = SlotTarget::Contact {
                    id: ContactId(0x77),
                };
            }),
            ("a range floor", |armed: &mut FrameArming| {
                armed.min_range_m = 1;
            }),
        ] {
            let mut other = shown.clone();
            if let Some(armed) = other.weapons.first_mut().and_then(|row| row.armed.as_mut()) {
                edit(armed);
            }
            let rearmed = think(&arena, &other, &mut fight.clone(), 2_000);
            assert_eq!(
                rearmed.first().map(armed_terms),
                Some(armed_terms(arm)),
                "{term}: re-armed on the wanted terms"
            );
        }
        // The slot NUMBER is not compared: shown as S2 on the wanted terms,
        // nothing to say.
        let mut renumbered = held_view.clone();
        if let Some(row) = renumbered.weapons.first_mut() {
            row.armed = Some(FrameArming::of(2, &armed_terms(arm)));
        }
        let unsaid = think(&arena, &renumbered, &mut fight.clone(), 2_000);
        assert!(
            !unsaid
                .iter()
                .any(|kind| matches!(kind, IntentKind::Condition(_))),
            "{unsaid:?}"
        );
        let mut held_fire = view.clone();
        held_fire.order = Some(FrameOrder {
            hold_fire: true,
            ..order_of(MindObjective::Engage { target: None })
        });
        let quiet = StockMemory::default();
        assert_eq!(
            think(&arena, &held_fire, &mut quiet.clone(), 1_000),
            vec![IntentKind::Lock {
                contact: ContactId(0x77)
            }]
        );
        if let Some(row) = held_fire.weapons.first_mut() {
            row.armed = Some(FrameArming::of(1, &armed_terms(arm)));
        }
        assert_eq!(
            think(&arena, &held_fire, &mut quiet.clone(), 1_000),
            vec![
                disarm(),
                IntentKind::Lock {
                    contact: ContactId(0x77)
                }
            ]
        );
        let mut starting = view.clone();
        starting.power = Power::Starting;
        assert_eq!(
            think(&arena, &starting, &mut StockMemory::default(), 1_000),
            Vec::new()
        );
        if let Some(row) = starting.weapons.first_mut() {
            row.armed = Some(FrameArming::of(1, &armed_terms(arm)));
        }
        assert_eq!(
            think(&arena, &starting, &mut StockMemory::default(), 1_000),
            vec![disarm()]
        );
        // Cold: `Startup`, then the disarm beside the armed row.
        let mut cold = starting.clone();
        cold.power = Power::Off;
        assert_eq!(
            think(&arena, &cold, &mut StockMemory::default(), 1_000),
            vec![IntentKind::Startup, disarm()]
        );
        let mut home = view.clone();
        home.order = Some(order_of(MindObjective::Withdraw));
        let stand_m = u32::try_from(ranges.stand_cm.min(reach) / 100).unwrap_or(u32::MAX);
        for row in &mut home.contacts {
            row.range_m = (ranges.stand_cm / 100).saturating_sub(10);
        }
        let mut withdrawing = StockMemory {
            state: ReflexState::Withdraw,
            goal: Some((1, 1)),
            ..StockMemory::default()
        };
        let cornered = think(&arena, &home, &mut withdrawing, 1_000);
        assert_eq!(withdrawing.state, ReflexState::Withdraw);
        let Some(first) = cornered.first() else {
            panic!("the cornered hull answers: {cornered:?}");
        };
        assert_eq!(armed_terms(first).max_range_m, stand_m, "{cornered:?}");
        assert!(stand_m < release_m);
    }

    /// Mirror of `m1b2_the_cited_shot_is_the_first_ready_armed_weapon_numbered_as_the_arena_does`
    /// (17b cut M2.6): the hot laser armed and skipped for the citation, the
    /// cannon cited S2; a third weapon, the only one ready, never cited —
    /// and the landing's countdown read against `lands_ms` off the view's
    /// clock (a gun ready one millisecond before the landing cited, one
    /// millisecond after it not).
    #[test]
    fn m2_mirror_the_cited_shot_is_the_first_ready_armed_weapon_numbered_as_the_arena_does() {
        use crate::arena::weapons::FrameWeapon;
        let mut arena = field(vec![
            minded(RED, Team::Red, 20_500, 20_500),
            line_tank(BLUE, Team::Blue, pose(20_500, 40_500)),
        ]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let mut view = view_of(&arena, RED);
        let Some(like) = view.weapons.first().cloned() else {
            panic!("the cannon row");
        };
        view.weapons = vec![
            row_like(&like, "laser", 250_000, 20),
            row_like(&like, "cannon", 250_000, 0),
        ];
        view.heat = Some(crate::arena::step::FrameHeat { heat: 60, max: 60 });
        let found = think(&arena, &view, &mut StockMemory::default(), 1_000);
        let weapons: Vec<SmolStr> = found
            .iter()
            .filter_map(|kind| match kind {
                IntentKind::Condition(Arming::Arm(cond)) => Some(cond.weapon.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(weapons, vec![SmolStr::new("laser"), SmolStr::new("cannon")]);
        assert_eq!(
            found.last(),
            Some(&IntentKind::Fire {
                weapon: SmolStr::new("cannon"),
                slot: Some(2)
            })
        );
        view.weapons = vec![
            row_like(&like, "laser", 250_000, 20),
            FrameWeapon {
                ready: false,
                ready_in_ms: 4_000,
                ..row_like(&like, "cannon", 250_000, 0)
            },
            row_like(&like, "missiles", 250_000, 0),
        ];
        let found = think(&arena, &view, &mut StockMemory::default(), 1_000);
        assert!(
            !found
                .iter()
                .any(|kind| matches!(kind, IntentKind::Fire { .. })),
            "{found:?}"
        );
        assert!(found.contains(&IntentKind::Lock {
            contact: ContactId(0x77)
        }));
        // Ready at the landing, not at the view: the cannon's countdown at
        // the milliseconds from the view's clock to `lands_ms`, and one past.
        let to_landing = 1_000_i64
            .saturating_add(arena.substep_len_ms())
            .saturating_sub(view.t_ms)
            .max(0);
        for (ready_in_ms, cited) in [(to_landing, true), (to_landing + 1, false)] {
            view.weapons = vec![FrameWeapon {
                ready: false,
                ready_in_ms,
                ..row_like(&like, "cannon", 250_000, 0)
            }];
            let found = think(&arena, &view, &mut StockMemory::default(), 1_000);
            assert_eq!(
                found
                    .iter()
                    .any(|kind| matches!(kind, IntentKind::Fire { .. })),
                cited,
                "ready in {ready_in_ms} against {to_landing}: {found:?}"
            );
        }
    }

    /// Mirror of `m1b2_residual_a_removed_weapon_parts_i_plus_one_from_the_arenas_number`
    /// (17b cut M2.6): the two guns armed through T2's `arming_for` beside
    /// T1's ([`arming`]), the turret's gun lost, the machine gun kept as S2 —
    /// nothing to re-arm on the view's S2 — and the fresh lock's think
    /// through both runners citing S2 off the view, the arena admitting it.
    #[test]
    fn m2_mirror_residual_a_removed_weapon_parts_i_plus_one_from_the_arenas_number() {
        use crate::arena::conditions::FireCondition;
        use crate::arena::resolution::Consequence;
        use crate::arena::weapons::FiredBy;
        let mut red = minded(RED, Team::Red, 20_500, 20_500);
        let mut mg = crate::arena::test_support::cannon();
        mg.id = SmolStr::new("machine_gun");
        mg.mount_section = SmolStr::new("hull");
        red.weapons.push(mg);
        let mut arena = field(vec![red, line_tank(BLUE, Team::Blue, pose(20_500, 40_500))]);
        online(&mut arena, RED);
        hold(&mut arena, RED, BLUE, ContactId(0x77));
        let card = line(&arena.arena_cfg.reflex);
        let view = view_of(&arena, RED);
        for kind in arming(&arena, &view, &card, Some(100_000)) {
            let IntentKind::Condition(arming) = kind else {
                panic!("an arming");
            };
            assert!(arena.resolve_arming(RED, &arming, 0).is_ok(), "{arming:?}");
        }
        assert_eq!(arena.conditions.slot_of(RED, "cannon"), Some(1));
        assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), Some(2));
        let mut journal = Vec::new();
        arena.apply_consequence(
            RED,
            &Consequence::RemoveWeapons {
                section: SmolStr::new("turret"),
            },
            0,
            None,
            &mut journal,
        );
        assert_eq!(
            arming(&arena, &view_of(&arena, RED), &card, Some(100_000)),
            Vec::new()
        );
        assert_eq!(arena.conditions.slot_of(RED, "machine_gun"), Some(2));
        let view = view_of(&arena, RED);
        let found = think(&arena, &view, &mut StockMemory::default(), 1_000);
        let rearmed: Vec<&FireCondition> = found
            .iter()
            .filter_map(|kind| match kind {
                IntentKind::Condition(Arming::Arm(cond)) => Some(cond),
                _ => None,
            })
            .collect();
        assert!(rearmed.iter().all(|cond| cond.weapon == "machine_gun"));
        for cond in &rearmed {
            assert_eq!(
                arena.resolve_arming(RED, &Arming::Arm((*cond).clone()), 0),
                Ok(Some(2))
            );
        }
        assert_eq!(
            found.last(),
            Some(&IntentKind::Fire {
                weapon: SmolStr::new("machine_gun"),
                slot: Some(2)
            })
        );
        assert_eq!(arena.resolve_lock(RED, ContactId(0x77), 0), Ok(()));
        assert!(
            arena
                .resolve_fire_by(RED, "machine_gun", 0, FiredBy::Mind { slot: Some(2) })
                .is_ok()
        );
    }
}
