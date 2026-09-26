//! **16b cut F.1 acceptance: the release half** — the stock-tank reset and the
//! seat sweep, over the production composition (doc 16 §3.1 "Custody", §3.3 "The
//! stock tanks are persistent objects", §3.7, §3.8).
//!
//! The rig is `crates/tacsim/tests/cockpit.rs`'s: the production composition root
//! with the starter area (the centre's three rooms and twelve pods) and the game's
//! own boot packs (the arena, its ground and its twelve tanks) applied, and every
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
//! 8. the **boot pod sweep** gives a stranded pod and billet back **inside the
//!    boot**, before any tick — from tacsim's `on_all_loaded`, the kernel's
//!    post-boot hook (16b cut U.4) — and the module's first tick says the hatch
//!    line and the room to that body, once a boot; a body whose session is not
//!    attached yet is told on the first tick after it is, within
//!    `match_::HATCH_HOLD_S`, and never after it;
//! 9. **16b cut V.2**: the boot reads the result documents it is handed onto the
//!    match book's history ring — the newest twenty, newest first whatever the
//!    order they arrive in, a document whose `match` disagrees with its key left
//!    out — and `SIM HISTORY` prints what it read.
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

    /// **A restart of the server half, in place** (16b cut U.4).
    ///
    /// The match book is RAM (D-16-3) and starts fresh. Everything else comes
    /// back the way a real boot brings it back: every module is handed the
    /// documents this world has written so far and the engine's boot runs again
    /// over the same objects — every `on_world_loaded`, then every
    /// `on_all_loaded`. The account oracle is the host's post-composition wiring,
    /// so it is wired again exactly as [`Rig::boot`] wired it.
    fn reboot(&mut self) {
        self.reboot_with(Vec::new());
    }

    /// [`Rig::reboot`], with `seeded` written into the store before it — each
    /// `(key, document)` a `tacsim` document some earlier process left behind
    /// (16b cut V.2: the result documents the history ring is filled from).
    fn reboot_with(&mut self, seeded: Vec<(smol_str::SmolStr, serde_json::Value)>) {
        *self
            .tw
            .world_mut()
            .module_state_mut::<match_::MatchBook>()
            .expect("the match book") = match_::MatchBook::new();
        let mut kv: BTreeMap<smol_str::SmolStr, Vec<(smol_str::SmolStr, serde_json::Value)>> =
            BTreeMap::new();
        kv.entry(smol_str::SmolStr::new(tacsim::NS))
            .or_default()
            .extend(seeded);
        for op in self.tw.take_module_ops() {
            let entry = kv.entry(op.ns.clone()).or_default();
            match op.op {
                mush_core::ModuleWrite::JsonSet { key, value } => {
                    entry.retain(|(k, _)| *k != key);
                    entry.push((key, value));
                }
                mush_core::ModuleWrite::Del { key } => entry.retain(|(k, _)| *k != key),
                mush_core::ModuleWrite::StreamAppend { .. } => {}
            }
        }
        let data: std::collections::HashMap<smol_str::SmolStr, mush_core::ModuleBootData> = kv
            .into_iter()
            .map(|(ns, entries)| {
                (
                    ns,
                    mush_core::ModuleBootData {
                        kv: entries,
                        ..Default::default()
                    },
                )
            })
            .collect();
        self.tw.modules_loaded(data);
        let mut map = Accounts::default();
        for (n, pilot) in self.pilots.iter().enumerate() {
            map.0.insert(
                *pilot,
                AccountId::mint(&format!("account-{n}"), n as i64).expect("an account id"),
            );
        }
        *self
            .tw
            .world_mut()
            .module_state_mut::<mush_sim::AccountOracle>()
            .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));
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

/// **The boot opens the pods before any tick, and the first tick says so** (16b
/// cut U.4, 16c `B-37`; 16b cut F.1; doc 16 §3.3's *every `Idle` transition*, of
/// which a reboot performs one).
///
/// The boot owes the centre two things:
///
/// * the **stock-tank reset**, inside `boot_matches` itself. It needs no content
///   latch — the roster document and the definitions are tacsim's own state and
///   `mush-sim` and `mush-tactics` are composed before it — and it **writes**
///   every roster tank's record *and its placement*, so a copy of it running a
///   second later would undo whatever staff had done in that second (through the
///   gateway the arena story's opening line is a `@spatial/place` of the two
///   slot-1 tanks nose to nose). `crates/mushd/tests/sim_rematch.rs`'s restart
///   test is the end-to-end proof of it.
/// * the **pod sweep**, which needs the pods, which resolve through the content
///   latch the content module fills after tacsim's own load. Cut F.1 had to arm
///   it and let the module's first 1 s tick perform it; since cut U.4 it runs in
///   tacsim's `on_all_loaded`, which the kernel calls once every module has
///   loaded — still inside the boot, so nothing a player types can land first.
///   Giving a seat back is a `Revoke`, which never reads the control-lease
///   mirror, so it needs no tick either.
///
/// What the boot cannot do is **speak**: its output reaches no session. So the
/// sweep says nothing and leaves each `(body, pod)` on the match book, and the
/// first tick tells that body its hatch is open and shows it the room — and
/// leaves a tank staff wrecked and moved after the boot exactly where it stands.
/// Once a boot: a pilot who boards again later is neither thrown out nor told
/// again.
#[test]
fn the_boot_opens_the_pods_before_any_tick_and_the_first_tick_says_the_hatch() {
    let mut rig = Rig::boot(2);
    one_aboard(&mut rig);
    let pilot = rig.pilot(0);
    let red = rig.tank(tacsim::arena::state::Team::Red, 1);
    let pod = rig.pod(0).expect("red pod 1 is locked to the first pilot");
    let team_room = rig
        .tw
        .world()
        .get(pod)
        .and_then(|object| object.location)
        .expect("the pod stands in the red room");
    assert_eq!(rig.billets(red).len(), 1, "the pilot holds the cockpit");

    // A tick with no boot behind it opens nothing: the sweep is a boot concern
    // and not something the boarding tick does every second.
    rig.tick_to(2);
    assert_eq!(rig.location(0), Some(pod), "a tick opened the pods");

    // **A restart, exactly**: the match book is RAM (D-16-3) and starts fresh;
    // everything else comes back from the documents this world has written — the
    // body placed inside the pod, the billet on the tank — through every module's
    // own boot, and then every module's `on_all_loaded`.
    rig.reboot();

    // (1) Before any tick: out of the pod and the billet given back.
    assert_eq!(
        rig.location(0),
        Some(team_room),
        "the boot left the body shut in the pod"
    );
    assert_eq!(
        rig.billets(red),
        Vec::new(),
        "the boot left the cockpit billet held against the next pilot"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .map(|book| book.hatch_notices().to_vec()),
        Some(vec![(pilot, pod)]),
        "the sweep owes that body one hatch line, and said nothing at boot"
    );

    // Something for a reset to answer, done after the boot and before its first
    // tick, so "the tick that drains the hatch lines leaves the tanks alone" is a
    // claim and not a fact about a world nobody touched. The reset is the boot
    // pass's own work, done before the front door serves a line; move it (or any
    // tank write) into this tick and a staff `@spatial/place` typed in the first
    // second is undone — the arena story through the gateway goes red.
    let authored = rig.pose(red).expect("the applier placed it").1;
    rig.wreck(red);
    let elsewhere = mush_sim::data::Pose {
        y_cm: authored.y_cm + 40_000,
        ..authored.clone()
    };
    let domain = rig.pose(red).expect("placed").0;
    let died_at = elsewhere.y_cm;
    rig.with_ctx(|ctx| {
        mush_sim::place(ctx, red, domain, elsewhere).expect("staff moved it after the boot");
    });

    // (2) The first tick says it: the hatch line, then the room the body is in.
    rig.tw.set_now(3);
    let events = rig.tw.fire_due(3_000);
    let heard = rig.tw.seen_by(&events, pilot).join("\n");
    let pod_name = mush_core::output::display_name(rig.tw.world(), pod);
    let room_name = mush_core::output::display_name(rig.tw.world(), team_room);
    assert!(
        heard.contains(&format!("The hatch of {pod_name} swings open.")),
        "the first tick did not say the hatch line: {heard}"
    );
    assert!(
        heard.contains(&room_name),
        "the first tick did not show the room: {heard}"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .is_some_and(|book| book.hatch_notices().is_empty()),
        "the hatch lines are owed once"
    );

    // And that tick left the tank exactly as staff left it.
    let held = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    assert!(
        match_::spec_plan(&held, &rig.pristine(red)).is_some(),
        "the first tick after the boot reset a tank"
    );
    let (_, standing) = rig.pose(red).expect("still placed");
    assert_eq!(
        standing.y_cm, died_at,
        "the first tick after the boot re-placed a tank"
    );

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

    // (3) Once a boot: a pilot who boards later is not thrown out a second later,
    // and nobody is told a hatch opened.
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    one_aboard(&mut rig);
    let pod = rig
        .pod(0)
        .expect("red pod 1 is locked to the first pilot again");
    rig.tw.set_now(4);
    let events = rig.tw.fire_due(4_000);
    let heard = rig.tw.seen_by(&events, pilot).join("\n");
    assert_eq!(
        rig.location(0),
        Some(pod),
        "the boot sweep ran again on a later tick"
    );
    assert!(
        !heard.contains("swings open"),
        "a later tick said a hatch line: {heard}"
    );
}

/// **The hatch line waits for the pilot's session, and no longer than the
/// hold** (16b cut U.4, 16c `B-37`).
///
/// On the split server the world learns a surviving SSH session is attached
/// only when the gateway's re-announce reaches it, which nothing orders against
/// the module's first tick. A restarted world's connection map is empty — it is
/// RAM — so the pilot here is marked not connected across the reboot, the way
/// the fresh server half sees it. The first tick says nothing and holds the
/// line; the tick after the re-attach says it, once. A pilot who is not back
/// within `match_::HATCH_HOLD_S` is never told.
#[test]
fn the_hatch_line_waits_for_the_pilots_session_and_no_longer_than_the_hold() {
    for reattached in [true, false] {
        let mut rig = Rig::boot(2);
        one_aboard(&mut rig);
        let pilot = rig.pilot(0);
        let pod = rig.pod(0).expect("red pod 1 is locked to the first pilot");
        let pod_name = mush_core::output::display_name(rig.tw.world(), pod);
        let hatch = format!("The hatch of {pod_name} swings open.");

        rig.tw.world_mut().set_connected(pilot, false);
        rig.reboot();
        assert_ne!(rig.location(0), Some(pod), "the boot opened the pod");

        // The first tick: nobody attached to tell, so the line is held.
        rig.tw.set_now(3);
        let events = rig.tw.fire_due(3_000);
        assert!(
            !rig.tw
                .seen_by(&events, pilot)
                .join("\n")
                .contains("swings open"),
            "a line was said to a body with no session"
        );
        let book = rig
            .tw
            .world()
            .module_state::<match_::MatchBook>()
            .expect("the match book");
        assert_eq!(book.hatch_notices(), [(pilot, pod)], "the line is held");
        assert_eq!(
            book.hatch_hold_until_s(),
            Some(3 + match_::HATCH_HOLD_S),
            "held from the first tick that held it"
        );

        if reattached {
            // The re-announce lands a second later: the next tick says it.
            rig.tw.world_mut().set_connected(pilot, true);
            rig.tw.set_now(4);
            let events = rig.tw.fire_due(4_000);
            let heard = rig.tw.seen_by(&events, pilot).join("\n");
            assert!(
                heard.contains(&hatch),
                "the re-attached pilot was not told: {heard}"
            );
        } else {
            // Nobody comes back within the hold: the line is dropped at it.
            let last = 3 + match_::HATCH_HOLD_S;
            rig.tw.set_now(last - 1);
            let _ = rig.tw.fire_due((last - 1) * 1_000);
            assert!(
                rig.tw
                    .world()
                    .module_state::<match_::MatchBook>()
                    .is_some_and(|book| book.hatch_notices().len() == 1),
                "the line is still held inside the hold"
            );
            rig.tw.set_now(last);
            let _ = rig.tw.fire_due(last * 1_000);
            rig.tw.world_mut().set_connected(pilot, true);
            rig.tw.set_now(last + 1);
            let events = rig.tw.fire_due((last + 1) * 1_000);
            assert!(
                !rig.tw
                    .seen_by(&events, pilot)
                    .join("\n")
                    .contains("swings open"),
                "a line was said past the hold"
            );
        }
        let book = rig
            .tw
            .world()
            .module_state::<match_::MatchBook>()
            .expect("the match book");
        assert!(book.hatch_notices().is_empty(), "the line is owed once");
        assert_eq!(
            book.hatch_hold_until_s(),
            None,
            "the hold ends with the list"
        );
    }
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

// ---------------------------------------------------------------------------
// 16b cut V.2 — the history ring is filled from the boot documents
// ---------------------------------------------------------------------------

/// One finished match's result document, in the shape `record::result_document`
/// writes (its own round trip is pinned in-source by
/// `v2_a_result_document_round_trips_into_a_history_row`): red's `unit`, flown
/// by `pilot` as `RA`, beat blue's `unit + 1`.
fn finished_result(id: u64, pilot: ObjId, unit: u32) -> serde_json::Value {
    let ended_ms = 1_700_000_000_000_i64 + i64::try_from(id).expect("a small id") * 60_000;
    serde_json::json!({
        "record": "tacsim.match",
        "record_version": 1,
        "match": id,
        "arena": match_::CENTRE_ARENA,
        "arena_definition_digest": "",
        "outcome": "red",
        "reason": "elimination",
        "teams": [
            { "team": "red", "units": [unit], "pilots": [
                { "pilot": pilot.0, "unit": unit, "callsign": "RA" },
            ] },
            { "team": "blue", "units": [unit + 1], "pilots": [] },
        ],
        "units": [
            { "unit": unit, "callsign": "RA", "outcome": "intact", "shots": 3, "hits": 2 },
            { "unit": unit + 1, "callsign": "BA", "outcome": "destroyed", "shots": 1, "hits": 0 },
        ],
        "ledger": { "log": format!("tacsim.match.{id}"), "digest": "ab", "chunks": 4 },
        "seed": 7,
        "started_ms": ended_ms - 272_000,
        "ended_ms": ended_ms,
        "arena_ms": 272_000,
    })
}

/// **The boot reads the result documents onto the history ring** (16b cut V.2;
/// 16c N2): twenty-three finished matches left in the store in no particular
/// order come back as the newest twenty, in descending id order; a document
/// whose own `match` disagrees with its key, and a document that is no result,
/// are left out; and the boot writes no result of its own for any of them — so
/// every row the ring holds, and every row `SIM HISTORY` prints, was READ.
///
/// The live proof, across a real reboot over SSH with one restart abort and one
/// finished match in the store, is `crates/mushd/tests/sim_rematch.rs`'s restart
/// test; this one is the ordering and the filter, which that test's two rows
/// cannot reach.
#[test]
fn v2_the_boot_fills_the_history_ring_from_the_result_documents_newest_first() {
    let mut rig = Rig::boot(1);
    let pilot = rig.pilot(0);
    let order: [u64; 23] = [
        7, 23, 1, 15, 2, 22, 9, 3, 21, 4, 20, 5, 19, 6, 18, 8, 17, 10, 16, 11, 14, 12, 13,
    ];
    let mut seeded: Vec<(smol_str::SmolStr, serde_json::Value)> = order
        .iter()
        .map(|id| {
            let unit = 500 + u32::try_from(*id).expect("a small id") * 2;
            (
                match_::result_key(tacsim::arena_task::MatchId(*id)),
                finished_result(*id, pilot, unit),
            )
        })
        .collect();
    // A document under one match's key that says it is another match's.
    seeded.push((
        match_::result_key(tacsim::arena_task::MatchId(40)),
        finished_result(50, pilot, 900),
    ));
    // And a document under a result key that is no result at all.
    seeded.push((
        match_::result_key(tacsim::arena_task::MatchId(30)),
        serde_json::json!({ "record": "tacsim.other", "match": 30 }),
    ));
    rig.reboot_with(seeded);

    // The boot wrote no result: nothing was running, so every row below is one
    // it read.
    let written: Vec<String> = rig
        .tw
        .take_module_ops()
        .into_iter()
        .filter_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet { key, .. }
                if match_::match_id_of_key(key.as_str()).is_some() =>
            {
                Some(key.to_string())
            }
            _ => None,
        })
        .collect();
    assert_eq!(written, Vec::<String>::new(), "the boot wrote a result");

    let book = rig
        .tw
        .world()
        .module_state::<match_::MatchBook>()
        .expect("the match book");
    assert_eq!(
        book.history().map(|row| row.id.0).collect::<Vec<_>>(),
        (4..=23).rev().collect::<Vec<u64>>(),
        "the newest twenty, newest first, whatever order the store handed them in"
    );
    assert_eq!(
        book.history().count(),
        match_::HISTORY_DEPTH,
        "the ring holds its depth"
    );
    for id in [50, 40, 30, 3, 2, 1] {
        assert!(
            book.remembered(tacsim::arena_task::MatchId(id)).is_none(),
            "match {id} is not a row the ring should hold"
        );
    }
    let newest = book
        .remembered(tacsim::arena_task::MatchId(23))
        .expect("the newest match");
    assert_eq!(
        Some(newest),
        match_::HistoryRow::from_document(&finished_result(23, pilot, 546)).as_ref(),
        "the row is the document, read"
    );

    // And `SIM HISTORY`, typed in the lobby, prints what was read.
    let said = rig.say(0, "sim history");
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some("The centre's last 20 matches, newest first:"),
        "{said}"
    );
    assert_eq!(lines.len(), 21, "{said}");
    assert!(
        lines.get(1).is_some_and(|row| row
            .starts_with("  #23  red won (blue eliminated)  4 m 32 s  1 v 1  ")
            && row.ends_with("  (you flew RA)")),
        "{said}"
    );
    assert!(
        lines.last().is_some_and(|row| row.starts_with("  #4  ")),
        "{said}"
    );
}

/// **A result written before the debrief existed has nothing to read** (16b cut
/// V.3; 16c N2): a remembered match whose result names no report — the shape
/// every result had before this cut — is refused in one sentence, bare or by
/// id, and no store read goes out; a restart abort of the same age is refused
/// as a restart first, because its rows name no pilot.
#[test]
fn v3_a_result_from_before_reports_existed_is_refused_without_a_read() {
    let mut rig = Rig::boot(1);
    let pilot = rig.pilot(0);
    let mut restarted = finished_result(6, pilot, 600);
    restarted["outcome"] = serde_json::json!("abort");
    restarted["reason"] = serde_json::json!("restart");
    restarted["teams"][0]["pilots"] = serde_json::json!([]);
    rig.reboot_with(vec![
        (
            match_::result_key(tacsim::arena_task::MatchId(5)),
            finished_result(5, pilot, 500),
        ),
        (
            match_::result_key(tacsim::arena_task::MatchId(6)),
            restarted,
        ),
    ]);
    for line in ["sim report", "sim report 5", "sim report #5"] {
        let (events, effects) = rig.tw.run_with_effects(pilot, line);
        let said = rig.tw.seen_by(&events, pilot).join("\n");
        assert!(
            said.contains(
                "Match 5 was recorded before reports existed; there is nothing to read of it."
            ),
            "{line}: {said}"
        );
        assert!(effects.is_empty(), "{line}: a refusal reads nothing");
    }
    let said = rig.say(0, "sim report 6");
    assert!(
        said.contains("The server restarted during match 6; there is no debrief of it."),
        "{said}"
    );
}
