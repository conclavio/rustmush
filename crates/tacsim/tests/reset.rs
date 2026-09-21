//! **16b cut F.1 acceptance: the release half** — the stock-tank reset and the
//! seat sweep, over the production composition (doc 16 §3.1 "Custody", §3.3 "The
//! stock tanks are persistent objects", §3.7, §3.8).
//!
//! The rig is `crates/tacsim/tests/cockpit.rs`'s: the production composition root
//! with the starter area (the centre's three rooms and eight pods) and the game's
//! own boot packs (the arena, its ground and its eight tanks) applied, and every
//! move made through the real command table against the real gate.
//!
//! What it proves, one test each:
//!
//! 1. a stock tank left as match one leaves it — a wrecked hull, spent rounds, a
//!    revoked power plant, parked where it died — is, after the reset, **equal to
//!    a fresh instantiation of its own spec** and standing on its authored cell,
//!    and a second reset writes nothing;
//! 2. a unit staff fielded through `@arena/start` is **not** reset;
//! 3. a roster tank that still carries a `deployed` marker is skipped and said
//!    so;
//! 4. an abort during `Boarding` with a pilot who **really boarded** leaves the
//!    pod boardable by a *different* pilot;
//! 5. so does the board deadline emptying a side;
//! 6. so does `SIM LEAVE` by a seated pilot;
//! 7. `@arena/reset` on an **already idle** centre sweeps a stranded body and
//!    billet — the state a restart leaves, which the old reset's first line
//!    returned on;
//! 8. the **boot pod sweep** — the half `boot_matches` arms rather than performs
//!    — gives a stranded pod and billet back on the module's first tick, once a
//!    boot, and leaves the tanks alone, because the reset is the boot pass's own
//!    work and a deferred one would undo whatever staff had just done.
//!
//! **16b cut F.4** adds one more to the deadline's pair: with the layer off, the
//! deadline drives nothing at all, because the tick that calls it is registered
//! whether or not the simulator is on.
//!
//! That a reset tank can be **destroyed again** is
//! `crates/tacsim/src/arena/damage.rs`'s
//! `a_tank_wrecked_and_then_reset_can_be_wrecked_again`, which drives the same
//! `spec_plan` through `plan_hit` with no world and no clock.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{AccountId, AccountIdentity, GOD, ObjId};
use mush_testkit::TestWorld;
use tacsim::TacsimConfig;
use tacsim::match_::{self, MatchState};
use tacsim::store::TacsimStore;

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

/// A fast arena and a boarding window long enough that only the test that wants
/// the deadline meets it.
fn config() -> TacsimConfig {
    let mut cfg = TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    };
    cfg.arena.tempo.quiet_ms = 50;
    cfg.arena.tempo.alert_ms = 50;
    cfg.arena.tempo.engaged_ms = 50;
    cfg.match_.startup_ms = 200;
    cfg.match_.board_timeout_ms = 60_000;
    cfg
}

/// The production roster, as `cockpit.rs` composes it (the character sheet is
/// left out because its chargen payload handler cannot be registered twice in one
/// process, and the sheet has nothing to do with the lobby).
fn roster() -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let cfg = config();
    let mut mods = comp::server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &onboarding,
        &game,
        comp::recovery_policy(&cfg.recovery),
    );
    mods.push(comp::worldgen_module_with(
        Arc::clone(&llm),
        mush_worldgen::WorldgenConfig::default(),
    ));
    mods.push(comp::npc_module_with(
        comp::npc_agent_adapter(),
        &mush_config::config::NpcCfg::default(),
    ));
    mods.push(comp::tacsim_module(cfg));
    mods.push(comp::content_module_with(true, true).expect("the content module composes"));
    mods
}

/// The boot-hydrated character → account map, as `mush-server` builds one.
#[derive(Default)]
struct Accounts(BTreeMap<ObjId, AccountId>);

impl AccountIdentity for Accounts {
    fn account_of(&self, character: ObjId) -> Option<AccountId> {
        self.0.get(&character).copied()
    }
}

struct Rig {
    tw: TestWorld,
    pilots: Vec<ObjId>,
}

impl Rig {
    fn boot(pilots: usize) -> Rig {
        let mut tw = TestWorld::with_modules(roster());
        let lobby = match_::centre_object(tw.world(), match_::LOBBY_ENTRY)
            .expect("the OOC pack at version 4 carries the simulator lobby");
        let mut map = Accounts::default();
        let mut who = Vec::new();
        for n in 0..pilots {
            let pilot = tw.spawn_player(&format!("Pilot{n}"));
            tw.world_mut()
                .move_to(pilot, lobby)
                .expect("a player walks into the lobby");
            map.0.insert(
                pilot,
                AccountId::mint(&format!("account-{n}"), n as i64).expect("an account id"),
            );
            who.push(pilot);
        }
        *tw.world_mut()
            .module_state_mut::<mush_sim::AccountOracle>()
            .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));
        let mut rig = Rig { tw, pilots: who };
        // The control-lease mirror is a coarse projection and
        // `request_seat_activation` fails closed on an unread one, so one second
        // of world time happens before anybody seats — exactly as on a live
        // server before its first player types anything.
        rig.tick_to(1);
        rig
    }

    fn pilot(&self, n: usize) -> ObjId {
        *self.pilots.get(n).expect("that many pilots were booted")
    }

    fn say(&mut self, n: usize, line: &str) -> String {
        let actor = self.pilot(n);
        let events = self.tw.run(actor, line);
        self.tw.seen_by(&events, actor).join("\n")
    }

    fn staff(&mut self, line: &str) -> String {
        let events = self.tw.run(GOD, line);
        self.tw.seen_by(&events, GOD).join("\n")
    }

    fn state(&self) -> MatchState {
        match_::centre_of(self.tw.world())
            .expect("the centre is on the match book")
            .state
    }

    fn centre(&self) -> tacsim::match_::Centre {
        match_::centre_of(self.tw.world())
            .expect("the centre is on the match book")
            .clone()
    }

    fn pod(&self, n: usize) -> Option<ObjId> {
        match_::pod_of(self.tw.world(), self.pilot(n))
    }

    fn location(&self, n: usize) -> Option<ObjId> {
        self.tw.world().get(self.pilot(n)).and_then(|o| o.location)
    }

    fn walk_to(&mut self, n: usize, entry: &str) {
        let pilot = self.pilot(n);
        let room = match_::centre_object(self.tw.world(), entry).expect("a centre room");
        self.tw
            .world_mut()
            .move_to(pilot, room)
            .expect("the centre's rooms take anybody");
    }

    fn tick_to(&mut self, seconds: i64) {
        self.tw.set_now(seconds);
        self.tw.fire_due(seconds.saturating_mul(1_000));
    }

    /// The tank behind one side's slot, through the arena's own roster.
    fn tank(&self, team: tacsim::arena::state::Team, index: u8) -> ObjId {
        match_::tank_for_slot(self.tw.world(), team, index)
            .expect("the slot's tank is in the world")
    }

    /// The billets held on one unit, as the simulation's own index has them.
    fn billets(&self, unit: ObjId) -> Vec<(String, ObjId)> {
        self.tw
            .world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.assignments.get(&unit))
            .map(|host| {
                host.seats
                    .iter()
                    .map(|(station, seat)| (station.as_str().to_owned(), seat.actor))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A pristine instantiation of the tank's own spec, for comparison.
    fn pristine(&self, unit: ObjId) -> mush_tactics::TacticalUnit {
        let held = mush_tactics::module::unit_of(self.tw.world(), unit)
            .expect("the roster tank is a tactical unit");
        let store = self
            .tw
            .world()
            .module_state::<TacsimStore>()
            .expect("the tacsim store");
        let catalog = store.definitions();
        let (entry, spec) = catalog
            .specs
            .iter()
            .find(|(entry, _)| {
                catalog
                    .bindings
                    .get(*entry)
                    .is_some_and(|binding| binding.spec.definition == *held.schema())
            })
            .expect("the tank's schema is loaded");
        let binding = catalog.bindings.get(entry).expect("its binding");
        tacsim::definitions::to_tactical_unit(spec, binding, unit).expect("it instantiates")
    }

    /// Run one closure against a real `Ctx`, the way `mushd`'s own end-to-end
    /// tests build one.
    fn with_ctx<T>(&mut self, f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T) -> T {
        let mut out = mush_core::Outbox::new();
        let engine = self.tw.engine_mut();
        let mut ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        f(&mut ctx)
    }

    /// **Leave a tank the way match one leaves it**: hull internals and front
    /// armour at the floor, the hull condition `wreck`, the three capability
    /// families a wreck loses revoked, and a magazine a third spent.
    ///
    /// Built as one plan and applied through `mush_tactics::module::apply_plan`,
    /// which is the production write path a match's own check-in takes. The same
    /// state is produced by a real kill over real SSH in
    /// `crates/mushd/tests/sim_rematch.rs`; this is the unit-level statement of
    /// what the reset has to undo.
    fn wreck(&mut self, unit: ObjId) {
        let pristine = self.pristine(unit);
        let mut ops: Vec<mush_tactics::UnitMutationOp> = Vec::new();
        for (addr, cell) in pristine.cells() {
            let floor = match addr {
                mush_tactics::CellAddr::Section { cell: id, .. }
                    if id.as_str() == tacsim::definitions::unit::INTERNALS_CELL =>
                {
                    Some(cell.min)
                }
                mush_tactics::CellAddr::Facet { cell: id, .. }
                    if id.as_str() == tacsim::definitions::unit::ARMOR_CELL =>
                {
                    Some(cell.min)
                }
                // A third of the load gone, so the reset has an ammunition claim
                // to answer as well as an armour one.
                mush_tactics::CellAddr::Component { cell: id, .. }
                    if id.as_str() == tacsim::definitions::unit::AMMO_CELL =>
                {
                    Some((cell.current / 3).max(cell.min))
                }
                _ => None,
            };
            if let Some(value) = floor
                && value != cell.current
            {
                ops.push(mush_tactics::UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value,
                });
            }
        }
        for (id, _) in pristine.sections() {
            if id.as_str() == "hull" {
                ops.push(mush_tactics::UnitMutationOp::SetSectionCondition {
                    section: id.clone(),
                    condition: mush_tactics::ConditionId::new("wreck").expect("a condition id"),
                });
            }
        }
        for key in pristine.capabilities() {
            let id = key.id.as_str();
            if id == "tacsim.power" || id == "tactics.mobility" || id.starts_with("tacsim.weapon.")
            {
                ops.push(mush_tactics::UnitMutationOp::RevokeCapability { key: key.clone() });
            }
        }
        assert!(!ops.is_empty(), "the fixture damaged nothing");
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:match-one")),
        )
        .expect("the damage fits one plan");
        self.with_ctx(|ctx| {
            mush_tactics::module::apply_plan(ctx, unit, &plan).expect("the damage applies");
        });
    }

    /// Where one object stands, as the simulation has it.
    fn pose(&self, unit: ObjId) -> Option<(mush_sim::DomainId, mush_sim::data::Pose)> {
        mush_sim::pose_of(self.tw.world(), unit)
    }
}

/// Two pilots claim, start and walk to their rooms; the first boards for real.
/// The centre is left `Boarding` with red pod 1 seated and blue unseated.
fn one_aboard(rig: &mut Rig) {
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    let said = rig.say(0, "board");
    assert!(said.contains("climb into"), "{said}");
    assert_eq!(rig.state(), MatchState::Boarding);
}

// ---------------------------------------------------------------------------
// (1) The tank reset
// ---------------------------------------------------------------------------

#[test]
fn a_wrecked_stock_tank_is_back_at_spec_and_at_muster_and_a_second_reset_says_nothing() {
    let mut rig = Rig::boot(1);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    let authored = rig.pose(red).expect("the applier placed it").1;
    rig.wreck(red);

    // And it is where it died, not where it mustered.
    let elsewhere = mush_sim::data::Pose {
        x_cm: authored.x_cm + 40_000,
        heading_mdeg: 123_000,
        ..authored.clone()
    };
    let domain = rig.pose(red).expect("placed").0;
    rig.with_ctx(|ctx| {
        mush_sim::place(ctx, red, domain, elsewhere).expect("staff moves a tank");
    });

    // The damage is real, and the reset has something to answer.
    let pristine = rig.pristine(red);
    let held = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    assert!(
        match_::spec_plan(&held, &pristine).is_some(),
        "the fixture left the tank at its spec"
    );
    assert!(
        tacsim::arena::damage::is_wreck(&held),
        "the fixture did not make a wreck"
    );

    let said = rig.staff("@arena/reset");
    assert!(said.contains("centre is reset"), "{said}");
    assert!(
        said.contains("1 tank(s) back at spec"),
        "the reset says what it repaired: {said}"
    );

    // **Equal to a fresh instantiation of its own spec**: no op is left to make.
    let after = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    let pristine = rig.pristine(red);
    assert_eq!(
        match_::spec_plan(&after, &pristine),
        None,
        "the reset left something to do"
    );
    assert!(
        !tacsim::arena::damage::is_wreck(&after),
        "it is not a wreck any more"
    );
    // Cells, conditions and the capabilities a wreck revokes, spelled out — the
    // three halves doc 16 §3.3 names, so a `spec_plan` that stopped emitting one
    // of them could not make this pass.
    for (addr, cell) in pristine.cells() {
        assert_eq!(
            after.cell(addr).map(|held| held.current),
            Some(cell.current),
            "{addr:?} is not back at its initial value"
        );
    }
    for (id, section) in pristine.sections() {
        assert_eq!(
            after.section(id).map(|held| held.condition.clone()),
            Some(section.condition.clone()),
            "section {} is not back to its initial condition",
            id.as_str()
        );
    }
    for key in pristine.capabilities() {
        assert!(
            after.capability(key).is_some(),
            "capability {} was not restored",
            key.id.as_str()
        );
    }

    // And it is back on its authored cell.
    let (_, standing) = rig.pose(red).expect("still placed");
    assert_eq!(
        (standing.x_cm, standing.y_cm, standing.heading_mdeg),
        (authored.x_cm, authored.y_cm, authored.heading_mdeg),
        "the tank is not back at its authored slot"
    );

    // A second reset writes nothing at all.
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("already idle; nothing changed"),
        "the second reset claimed to do something: {said}"
    );
    assert!(
        !said.contains("back at spec"),
        "and it rewrote a tank that was already at spec: {said}"
    );
}

/// **The boot pod sweep, and what it deliberately is not** (16b cut F.1; doc 16
/// §3.3's *every `Idle` transition*, of which a reboot performs one).
///
/// The boot pass owes the centre two things, and does them at two moments,
/// because they need different facts and carry different risks:
///
/// * the **stock-tank reset** happens inside `boot_matches` itself. It needs no
///   content latch — the roster document and the definitions are tacsim's own
///   state and `mush-sim` and `mush-tactics` are composed before it — and it
///   **writes** every roster tank's record *and its placement*, so a copy of it
///   running a second later would undo whatever staff had done in that second.
///   That is not hypothetical: through the gateway the arena story's opening line
///   is a `@spatial/place` of the two slot-1 tanks nose to nose, and a reset
///   deferred to the first tick put them back on their authored cells 2.4 km
///   apart, after which they never detected each other and the match never
///   started. `crates/mushd/tests/sim_rematch.rs`'s restart test is the end-to-end
///   proof of the boot reset: a spent magazine and a tank parked off its cell,
///   both put back across a real SSH reboot.
/// * the **pod sweep** cannot go with it: a pod is resolved through the content
///   latch, which is the content module's state and is still empty while tacsim's
///   `on_world_loaded` runs. So that half alone is armed, and the module's own
///   1 s tick performs it. Its window is harmless because it only takes back what
///   `BOARD` put in a pod, and `BOARD` fails closed on the control-lease mirror
///   until that same 1 Hz projection has been read once.
///
/// This is that tick: it gives a stranded pod and billet back, it leaves a
/// wrecked tank exactly where it stands, and it is owed once a boot.
#[test]
fn the_armed_boot_tick_sweeps_the_pods_once_and_leaves_the_tanks_to_the_boot_pass() {
    let mut rig = Rig::boot(2);
    one_aboard(&mut rig);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    let pod = rig.pod(0).expect("red pod 1 is locked to the first pilot");
    assert_eq!(rig.billets(red).len(), 1, "the pilot holds the cockpit");

    // **A restart, exactly** (test 7's line): the match book is RAM, so a reboot
    // starts from a fresh slot while the body in the pod and the billet on the
    // tank are both durable.
    *rig.tw
        .world_mut()
        .module_state_mut::<match_::MatchBook>()
        .expect("the match book") = match_::MatchBook::new();
    assert_eq!(rig.state(), MatchState::Idle);

    // Something for a reset to answer, so "the tick left the tanks alone" is a
    // claim and not a fact about a world nobody touched.
    let authored = rig.pose(red).expect("the applier placed it").1;
    rig.wreck(red);
    let elsewhere = mush_sim::data::Pose {
        y_cm: authored.y_cm + 40_000,
        ..authored.clone()
    };
    let domain = rig.pose(red).expect("placed").0;
    let died_at = elsewhere.y_cm;
    rig.with_ctx(|ctx| {
        mush_sim::place(ctx, red, domain, elsewhere).expect("the match left it where it died");
    });

    // A tick with nothing armed changes nothing: the sweep is a boot concern and
    // not something the boarding tick does every second.
    rig.tick_to(2);
    assert_eq!(
        rig.location(0),
        Some(pod),
        "an unarmed tick opened the pods"
    );

    // What `boot_matches` arms, and then the first tick of the boot.
    rig.tw
        .world_mut()
        .module_state_mut::<match_::MatchBook>()
        .expect("the match book")
        .arm_boot_sweep();
    rig.tick_to(3);

    assert_ne!(
        rig.location(0),
        Some(pod),
        "the boot sweep left the body shut in the pod"
    );
    assert_eq!(
        rig.billets(red),
        Vec::new(),
        "the boot sweep left the cockpit billet held against the next pilot"
    );

    // And it left the tank exactly as it stood. The reset is `boot_matches`' own
    // work, done before the front door serves a line; move it back into this tick
    // and the arena story through the gateway goes red.
    let held = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    assert!(
        match_::spec_plan(&held, &rig.pristine(red)).is_some(),
        "the boarding tick reset a tank"
    );
    let (_, standing) = rig.pose(red).expect("still placed");
    assert_eq!(standing.y_cm, died_at, "the boarding tick re-placed a tank");

    // What the boot pass itself calls, on the same state, does do it.
    let report = rig.with_ctx(match_::reset_stock_tanks);
    assert!(
        report.restored.contains(&red) && report.remustered.contains(&red),
        "{report:?}"
    );
    let after = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    assert_eq!(
        match_::spec_plan(&after, &rig.pristine(red)),
        None,
        "the boot pass's reset did not put the tank back at its spec"
    );
    let (_, standing) = rig.pose(red).expect("still placed");
    assert_eq!(
        (standing.x_cm, standing.y_cm, standing.heading_mdeg),
        (authored.x_cm, authored.y_cm, authored.heading_mdeg),
        "the boot pass's reset did not put the tank back at its authored slot"
    );

    // And the sweep is owed **once**: a pilot who boards later in the same boot
    // is not thrown out a second later, which is what stops a 1 Hz tick emptying
    // the pods for ever.
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    one_aboard(&mut rig);
    let pod = rig
        .pod(0)
        .expect("red pod 1 is locked to the first pilot again");
    rig.tick_to(4);
    assert_eq!(
        rig.location(0),
        Some(pod),
        "the boot sweep runs on every tick, not once a boot"
    );
}

#[test]
fn a_unit_staff_fielded_is_never_reset() {
    let mut rig = Rig::boot(1);
    // A tank of this staffer's own, outside the arena's roster, built from the
    // same shipped schema through the same verb a staffer would use.
    let own = rig
        .tw
        .world_mut()
        .create(mush_core::ObjKind::Thing, "Spare Tank", GOD, Some(GOD))
        .expect("staff makes a thing");
    let said = rig.staff(&format!("@unit/create #{}=tacsim.unit.line_tank", own.0));
    assert!(said.contains("is now a tactical unit"), "{said}");
    rig.wreck(own);
    let before = mush_tactics::module::unit_of(rig.tw.world(), own)
        .expect("a unit")
        .clone();

    rig.staff("@arena/reset");

    let after = mush_tactics::module::unit_of(rig.tw.world(), own)
        .expect("a unit")
        .clone();
    assert_eq!(
        before.revision(),
        after.revision(),
        "the reset wrote to a unit the centre does not own"
    );
    assert!(
        tacsim::arena::damage::is_wreck(&after),
        "and it is still the wreck staff left"
    );
}

// A real check-out spawns the arena's own tokio task, so this one needs a
// runtime; the rest of the file never reaches `Running`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deployed_roster_tank_is_skipped_and_said_so() {
    let mut rig = Rig::boot(2);
    // A real match, so the two slot-1 tanks really are checked out.
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    assert!(mush_sim::deployed::is_deployed(rig.tw.world(), red));

    // The staff verb refuses outright while the centre is live, so the claim is
    // about the function the `Idle` transition calls.
    let said = rig.staff("@arena/reset");
    assert!(said.contains("holding a match"), "{said}");
    let report = rig.with_ctx(match_::reset_stock_tanks);
    assert!(
        report.deployed.contains(&red),
        "a deployed tank was not reported skipped: {report:?}"
    );
    assert!(
        !report.restored.contains(&red) && !report.remustered.contains(&red),
        "a deployed tank was written: {report:?}"
    );
}

// ---------------------------------------------------------------------------
// (2) The seat sweep
// ---------------------------------------------------------------------------

/// After `rig`'s pilot 0 has been put out of red pod 1, a **different** pilot can
/// claim that slot and board that pod.
fn a_different_pilot_boards_red_pod_one(rig: &mut Rig, who: usize) {
    rig.walk_to(who, match_::LOBBY_ENTRY);
    let said = rig.say(who, "sim team red");
    assert!(said.contains("takes red slot 1"), "{said}");
    // Somebody has to be on the other side for the match to start boarding.
    rig.walk_to(1, match_::LOBBY_ENTRY);
    rig.say(1, "sim team blue");
    rig.say(who, "sim start");
    rig.walk_to(who, match_::RED_ROOM_ENTRY);
    let said = rig.say(who, "board");
    assert!(
        said.contains("climb into"),
        "the pod was still held against the next pilot: {said}"
    );
}

#[test]
fn an_abort_during_boarding_frees_the_pod_for_a_different_pilot() {
    let mut rig = Rig::boot(3);
    one_aboard(&mut rig);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    assert_eq!(rig.billets(red).len(), 1, "the pilot holds the cockpit");

    // Blue calls it off from the blue room.
    let said = rig.say(1, "sim abort");
    assert!(said.contains("calls the match off"), "{said}");
    assert_eq!(rig.state(), MatchState::Aborted);

    // The seated pilot is out, the billet is back, and the centre says nothing is
    // held.
    assert!(
        rig.location(0) != rig.pod(0) || rig.pod(0).is_none(),
        "the pilot is still shut in the pod"
    );
    assert_eq!(
        rig.billets(red),
        Vec::new(),
        "the pod's tank still holds the abandoned pilot's cockpit billet"
    );
    a_different_pilot_boards_red_pod_one(&mut rig, 2);
}

#[test]
fn the_board_deadline_emptying_a_side_frees_the_pod_for_a_different_pilot() {
    let mut rig = Rig::boot(3);
    one_aboard(&mut rig);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);

    // Blue never boards, the deadline passes, the match aborts.
    rig.tick_to(120);
    assert_eq!(
        rig.state(),
        MatchState::Aborted,
        "the deadline did not fire"
    );
    assert_eq!(
        rig.billets(red),
        Vec::new(),
        "the dropped match left the cockpit billet behind"
    );
    a_different_pilot_boards_red_pod_one(&mut rig, 2);
}

/// **With the layer off, the boarding deadline drives nothing** (16b cut F.4
/// residual).
///
/// Every route INTO `Boarding` asks `lobby::require_layer` first, and the match
/// book is RAM, so a running centre and a layer-off server cannot meet by
/// themselves: `[tacsim].enabled` is read once at boot and a restart starts the
/// centre `Idle`. What this test does by hand is therefore the only way to reach
/// the state — the flag going out from under a centre that is mid-boarding — and
/// it is worth pinning, because the boarding tick is registered unconditionally
/// and was the one thing still driving the simulator on a server whose simulator
/// is off: it would have dropped the unseated slots, ejected the seated pilot
/// and announced an abort to a room where every `SIM` line answers only that the
/// layer is off.
///
/// Its twin above — the same rig with the layer left on — is what proves the
/// deadline still fires when it should.
#[test]
fn f4_with_the_layer_off_the_board_deadline_drives_nothing() {
    let mut rig = Rig::boot(3);
    one_aboard(&mut rig);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    let pod = rig.pod(0).expect("red pod 1 is locked to the first pilot");
    let slots = rig.centre().slots.len();

    // The layer goes out from under it. `TacsimStore::default()` is the shape a
    // boot with `[tacsim].enabled = false` installs.
    *rig.tw
        .world_mut()
        .module_state_mut::<TacsimStore>()
        .expect("the tacsim slot") = TacsimStore::default();

    rig.tick_to(120);

    assert_eq!(
        rig.state(),
        MatchState::Boarding,
        "the deadline ran on a server with the layer off"
    );
    assert_eq!(
        rig.centre().slots.len(),
        slots,
        "the deadline dropped a slot on a server with the layer off"
    );
    assert_eq!(
        rig.location(0),
        Some(pod),
        "the deadline ejected a pilot on a server with the layer off"
    );
    assert_eq!(
        rig.billets(red).len(),
        1,
        "the deadline took the cockpit billet back on a server with the layer off"
    );
}

#[test]
fn sim_leave_by_a_seated_pilot_gives_the_seat_back() {
    let mut rig = Rig::boot(3);
    one_aboard(&mut rig);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    let pod = rig.pod(0).expect("red pod 1 is locked to the first pilot");
    assert_eq!(rig.location(0), Some(pod), "the pilot is inside the pod");

    // Typed from inside the pod, which the centre counts as the team room.
    let said = rig.say(0, "sim leave");
    assert!(said.contains("gives up red slot 1"), "{said}");
    assert_ne!(rig.location(0), Some(pod), "the pilot is still in the pod");
    assert_eq!(
        rig.billets(red),
        Vec::new(),
        "a seated leaver kept the tank's cockpit billet"
    );
    // The side emptied, so the match aborted; a different pilot takes the slot.
    a_different_pilot_boards_red_pod_one(&mut rig, 2);
}

// ---------------------------------------------------------------------------
// (3) Two players play again with no staff
// ---------------------------------------------------------------------------

#[test]
fn a_claim_on_a_terminal_centre_resets_it_and_claims_with_no_staff() {
    let mut rig = Rig::boot(2);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    rig.wreck(red);
    one_aboard(&mut rig);
    rig.say(1, "sim abort");
    assert_eq!(rig.state(), MatchState::Aborted, "a terminal centre");

    // The board says how it went **and** how to get past it.
    let said = rig.say(0, "sim status");
    assert!(said.contains("aborted"), "{said}");
    assert!(
        said.contains("`SIM TEAM red` or `SIM TEAM blue` opens the next match."),
        "a terminal centre is a dead end: {said}"
    );

    // And the claim itself does the staff verb's work on the way through.
    rig.walk_to(0, match_::LOBBY_ENTRY);
    let said = rig.say(0, "sim team red");
    assert!(
        !said.contains("that is not something you can do to it now"),
        "the terminal refusal is still what a player gets: {said}"
    );
    assert!(said.contains("takes red slot 1"), "{said}");
    // The whole sentence: nothing was skipped here, so the claim is entitled to
    // the muster half of it. When the report does carry a skipped or failed tank
    // the line stops at "cleared from the board" and a second one names the
    // count, because `@arena/reset` — the only surface that lists those rows — is
    // behind a staff login these two players may not have.
    assert!(
        said.contains("The last match is cleared from the board and the tanks are back at muster."),
        "the reset was not announced: {said}"
    );
    assert!(
        !said.contains("could not be put back"),
        "a clean reset claimed something was stuck: {said}"
    );
    assert_eq!(rig.state(), MatchState::Selection);

    // The tanks came with it.
    let after = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    let pristine = rig.pristine(red);
    assert_eq!(
        match_::spec_plan(&after, &pristine),
        None,
        "a player's claim did not reset the tanks"
    );
}

/// **A tank the reset could not put back is said so, to the players** (16b cut
/// F.1).
///
/// The state is the one `finish` logs and carries on from: a check-in that
/// refused (`CustodyError::Refused` or `Encode`) leaves that unit's `deployed`
/// marker on the record while the centre goes terminal anyway. `@arena/reset`
/// renders `ResetReport::deployed` and `::failed`, and it is staff-only — so on
/// the no-staff path those rows used to reach nothing but a `tracing::warn!`,
/// while the claim announced "the tanks are back at muster" over them and the
/// next match was fought on last match's wreck with no line connecting the two.
#[test]
fn a_claim_that_could_not_put_every_tank_back_says_so_rather_than_promising_muster() {
    let mut rig = Rig::boot(2);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    rig.wreck(red);
    one_aboard(&mut rig);
    // The marker a refused check-in leaves behind, planted **before** the end.
    // Only `@arena/release` and the boot reconciliation clear one, which is
    // exactly why neither reset may write over it — and since 16b cut D.3 the
    // pre-`Running` abort runs the tank pass itself (`call_off`), so a marker
    // planted after it would simply have found an already-reset tank.
    rig.with_ctx(|ctx| {
        mush_sim::deployed::mark(
            ctx.w,
            red,
            mush_sim::deployed::DeployedMark::new(99, "tacsim.arena.centre"),
        );
    });
    rig.say(1, "sim abort");
    assert_eq!(rig.state(), MatchState::Aborted, "a terminal centre");

    rig.walk_to(0, match_::LOBBY_ENTRY);
    let said = rig.say(0, "sim team red");
    assert!(
        said.contains("takes red slot 1"),
        "the claim was refused: {said}"
    );
    assert!(
        !said.contains("the tanks are back at muster"),
        "the claim promised a muster the reset did not do: {said}"
    );
    assert!(
        said.contains("1 tank(s) could not be put back"),
        "nothing told the players a tank was left as it stands: {said}"
    );
    // And the tank really was left alone — the reset never fights the arena for a
    // record it does not hold.
    let after = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    assert!(
        tacsim::arena::damage::is_wreck(&after),
        "a deployed tank was reset anyway"
    );
}

#[test]
fn the_other_sim_selectors_keep_their_state_guards_on_a_terminal_centre() {
    let mut rig = Rig::boot(2);
    one_aboard(&mut rig);
    rig.say(1, "sim abort");
    assert_eq!(rig.state(), MatchState::Aborted);

    // `SIM TEAM` is the one that opens the next match; the rest still refuse.
    for line in ["sim start", "sim leave", "sim abort"] {
        let said = rig.say(0, line);
        assert!(
            said.contains("aborted") || said.contains("hold no slot"),
            "{line} on a terminal centre: {said}"
        );
        assert_eq!(
            rig.state(),
            MatchState::Aborted,
            "{line} moved a terminal centre"
        );
    }
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    let said = rig.say(0, "board");
    assert!(said.contains("hold no slot"), "{said}");
}

#[test]
fn an_arena_reset_on_an_already_idle_centre_sweeps_a_stranded_body_and_billet() {
    let mut rig = Rig::boot(2);
    one_aboard(&mut rig);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    let pod = rig.pod(0).expect("red pod 1 is locked to the first pilot");
    assert_eq!(rig.billets(red).len(), 1);

    // **A restart, exactly**: the match book is RAM (D-16-3), so a reboot starts
    // from a fresh slot and the centre comes back `Idle` — while the body inside
    // the pod and the cockpit billet on the tank are both durable world state.
    // `crates/mushd/tests/arena_custody.rs` does this with a real server stop; one
    // line is the whole of what the restart does to the book.
    *rig.tw
        .world_mut()
        .module_state_mut::<match_::MatchBook>()
        .expect("the match book") = match_::MatchBook::new();
    assert_eq!(rig.state(), MatchState::Idle);
    assert_eq!(rig.location(0), Some(pod), "the body is still in the pod");

    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("1 pod(s) opened and 1 cockpit billet(s) given back"),
        "an idle centre with a stranded pilot reported nothing: {said}"
    );
    assert_ne!(rig.location(0), Some(pod), "the body is still in the pod");
    assert_eq!(
        rig.billets(red),
        Vec::new(),
        "the billet is still held against the next pilot"
    );
    assert!(rig.centre().slots.is_empty());

    // And the second run is the one that says nothing changed.
    let said = rig.staff("@arena/reset");
    assert!(said.contains("already idle; nothing changed"), "{said}");
}
