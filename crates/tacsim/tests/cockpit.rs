//! **16b cut K.2 acceptance** — the cockpit, over the production composition
//! (16b §8 cut K, K2–K4; doc 16 §3.3, §3.6, §3.7, §3.8, §5).
//!
//! The rig is `crates/tacsim/tests/match_lifecycle.rs`'s, with cut K.1's stand-in
//! for a seat taken out: everything here boards through the real `BOARD` verb,
//! dispatched through the real command table against the real gate, and leaves
//! through the real `EXIT`.
//!
//! 1. `BOARD` puts the body in the pod, the station on the **tank**, the seat in
//!    the pilot's hands and the focus on it — and the second one fires cut K.1's
//!    `Boarding → Running` trigger;
//! 2. the eleven verbs are contextual: each refuses with its own one line where
//!    it does not mean anything;
//! 3. `HEADING` before `STARTUP` is refused with the cold tank's own word;
//! 4. `VIEW` draws the platform at the centre of a grid;
//! 5. `STATUS`, `CONTACTS` and `WEAPONSTATUS` render the last frame, and the
//!    frame carries doc 16 §5's shape;
//! 6. `EXIT` gives the seat, the lease and the focus back and puts the pilot in
//!    the team room — and during `Boarding` it un-seats the slot;
//! 7. the kernel's new linkdead hook reaches the arena as a raised epoch.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{AccountId, AccountIdentity, Audience, GOD, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use tacsim::TacsimConfig;
use tacsim::arena::state::{Channel, Team};
use tacsim::match_::{self, MatchState, cockpit};

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

/// A fast arena and a long enough boarding window that a test never races the
/// deadline it is not testing.
fn config() -> TacsimConfig {
    let mut cfg = TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    };
    cfg.arena.tempo.quiet_ms = 50;
    cfg.arena.tempo.alert_ms = 50;
    cfg.arena.tempo.engaged_ms = 50;
    cfg.arena.ledger.summary_ms = 100;
    cfg.match_.startup_ms = 200;
    cfg.match_.board_timeout_ms = 60_000;
    cfg
}

/// The production roster, composed as `mush_server::boot` composes it, with the
/// starter area (the centre) and the game's boot packs (the arena and its eight
/// tanks) on. The character sheet is left out for the reason
/// `match_lifecycle.rs` leaves it out: its chargen payload handler cannot be
/// registered twice in one process.
///
/// `cfg` is a parameter rather than [`config`] itself because one test wants a
/// startup window of its own; see
/// `an_order_while_power_is_coming_up_says_it_is_still_starting`.
fn roster_with(cfg: TacsimConfig) -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
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
    egress: Option<tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>>,
    now: i64,
}

impl Rig {
    fn boot(pilots: usize) -> Rig {
        Rig::boot_with(pilots, config())
    }

    fn boot_with(pilots: usize, cfg: TacsimConfig) -> Rig {
        let mut tw = TestWorld::with_modules(roster_with(cfg));
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
        let egress = tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
            .and_then(|tasks| tasks.take_egress());
        let mut rig = Rig {
            tw,
            pilots: who,
            egress,
            now: 0,
        };
        // **The control-lease mirror.** `request_seat_activation` fails closed on
        // an unread one (`SeatError::ControlUnknown`), and the mirror is
        // refreshed by `mush-sim`'s own coarse tick — so a world that has not
        // ticked once cannot seat anybody. One second of world time is the whole
        // of the arrangement, and it is what a live server does before its first
        // player types anything.
        rig.tick_to(1);
        rig
    }

    /// The world loop's `Wake::ArenaEgress` arm, by hand (see
    /// `match_lifecycle.rs`).
    fn pump(&mut self) {
        let Some(tasks) = self
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
        else {
            return;
        };
        let mut staged = 0_usize;
        if let Some(rx) = self.egress.as_mut() {
            while staged < tacsim::arena_bridge::MAX_EGRESS_PER_DRAIN {
                match rx.try_recv() {
                    Ok(message) => {
                        if !tasks.stage(message) {
                            break;
                        }
                        staged += 1;
                    }
                    Err(_) => break,
                }
            }
        }
        if staged == 0 {
            return;
        }
        self.tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
            actor: GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        });
        tasks.stamp_ledger_writes(1);
        tasks.ack_ledger_upto(1);
    }

    fn pilot(&self, n: usize) -> ObjId {
        *self.pilots.get(n).expect("that many pilots were booted")
    }

    fn say(&mut self, n: usize, line: &str) -> String {
        let actor = self.pilot(n);
        let events = self.tw.run(actor, line);
        self.tw.seen_by(&events, actor).join("\n")
    }

    /// Run one line as God (staff).
    fn staff(&mut self, line: &str) -> String {
        let events = self.tw.run(GOD, line);
        self.tw.seen_by(&events, GOD).join("\n")
    }

    /// Run one line and hand back every event it produced, so a test can look at
    /// a panel frame and not only at its text.
    fn events(&mut self, n: usize, line: &str) -> Vec<(Audience, OutputEvent)> {
        let actor = self.pilot(n);
        self.tw.run(actor, line)
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

    fn tank(&self, n: usize) -> ObjId {
        let pod = self.pod(n).expect("the pilot's pod is locked");
        match_::tank_of(self.tw.world(), pod).expect("the pod's tank is in the world")
    }

    fn location(&self, n: usize) -> Option<ObjId> {
        self.tw.world().get(self.pilot(n)).and_then(|o| o.location)
    }

    /// Walk one pilot into the room `entry` names.
    fn walk_to(&mut self, n: usize, entry: &str) {
        let pilot = self.pilot(n);
        let room = match_::centre_object(self.tw.world(), entry).expect("a centre room");
        self.tw
            .world_mut()
            .move_to(pilot, room)
            .expect("the centre's rooms take anybody");
    }

    fn tick_to(&mut self, seconds: i64) {
        self.now = seconds;
        self.tw.set_now(seconds);
        self.tw.fire_due(seconds.saturating_mul(1_000));
    }

    /// Two pilots, one a side, claimed, started and boarded — the state every
    /// cockpit test starts from. Answers what the second `BOARD` said.
    async fn fly(&mut self) -> String {
        self.say(0, "sim team red");
        self.say(1, "sim team blue");
        self.say(0, "sim start");
        self.walk_to(0, match_::RED_ROOM_ENTRY);
        self.walk_to(1, match_::BLUE_ROOM_ENTRY);
        self.say(0, "board");
        let said = self.say(1, "board");
        assert_eq!(self.state(), MatchState::Running, "{said}");
        said
    }

    /// Pump until the arena has sent that pilot a frame, or give up.
    async fn frame_for(&mut self, n: usize) -> bool {
        let pilot = self.pilot(n);
        for _ in 0..400 {
            self.pump();
            if self
                .tw
                .world()
                .module_state::<tacsim::arena_bridge::ArenaRegistry>()
                .and_then(|registry| registry.last_frame(pilot))
                .is_some()
            {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }

    /// Pump until every pilot's frame says power is online.
    async fn powered(&mut self, n: usize) -> bool {
        let pilot = self.pilot(n);
        for _ in 0..600 {
            self.pump();
            let online = self
                .tw
                .world()
                .module_state::<tacsim::arena_bridge::ArenaRegistry>()
                .and_then(|registry| registry.last_frame(pilot))
                .is_some_and(|held| held.frame.power == tacsim::arena::state::Power::Online);
            if online {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }

    /// The seats one pilot holds, as the simulation's own index has them.
    fn seats(&self, n: usize) -> Vec<(ObjId, String)> {
        self.tw
            .world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.derived.seats_by_actor.get(&self.pilot(n)))
            .map(|seats| {
                seats
                    .iter()
                    .map(|(unit, station)| (*unit, station.as_str().to_owned()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// **What the engine does to a module on a session transition**, by hand: the
    /// kernel's own dispatcher, on the point cut K.2 added. A `TestWorld` has no
    /// connections to drop, and the SSH lens (`mushd/tests/sim_cockpit.rs`) is
    /// where a real line is really cut; this is the unit-level proof that the
    /// hook is wired to the arena at all.
    ///
    /// Answers **what the hook emitted**, because the hook's outbox is this
    /// function's own and a caller that cannot read it can say nothing about the
    /// lines a reconnect prints.
    fn fire_conn(
        &mut self,
        who: ObjId,
        transition: mush_core::ConnTransition,
    ) -> Vec<(Audience, OutputEvent)> {
        let mut out = mush_core::Outbox::new();
        let engine = self.tw.engine_mut();
        mush_core::module::fire_conn_hook(
            &engine.registry,
            &mut engine.world,
            &mut out,
            who,
            transition,
            mush_core::action_state::TypedStateLens::Detached(
                mush_core::action_state::DetachedTypedState::Test,
            ),
        );
        out.take()
    }

    /// The arena's epoch mirror for one channel of one unit — the observable
    /// behind every `Control` the world sends, because `send_control` raises it
    /// before it offers the message.
    fn epoch(&self, unit: ObjId, channel: Channel) -> u64 {
        self.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.epoch_of(unit, channel))
            .unwrap_or_default()
    }

    /// Run one read against a `Ctx` built the way `mushd`'s own end-to-end tests
    /// build one, so a test can call a frame builder directly.
    fn with_ctx<T>(&mut self, n: usize, f: impl FnOnce(&mush_core::Ctx<'_>) -> T) -> T {
        let actor = self.pilot(n);
        let mut out = mush_core::Outbox::new();
        let engine = self.tw.engine_mut();
        let ctx = mush_core::Ctx {
            w: &mut engine.world,
            actor,
            switches: mush_core::SwitchSet::default(),
            args: mush_core::ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        f(&ctx)
    }

    /// The panel frame in a command's events, if it emitted one.
    fn panel(events: &[(Audience, OutputEvent)]) -> Option<serde_json::Value> {
        events.iter().rev().find_map(|(_, event)| match event {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::arena_bridge::FRAME_FEED =>
            {
                Some(data.clone())
            }
            _ => None,
        })
    }
}

// ---------------------------------------------------------------------------
// (1) `BOARD`
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn board_puts_the_body_in_the_pod_the_station_on_the_tank_and_the_seat_in_hand() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);

    let pod = rig.pod(0).expect("red pod 1 is locked to the first pilot");
    let tank = rig.tank(0);
    let said = rig.say(0, "board");
    assert!(said.contains("climb into"), "{said}");

    // The body is in the pod (doc 16 §3.7: the pod is where the body sits).
    assert_eq!(rig.location(0), Some(pod), "the pilot is inside the pod");

    // The station is on the TANK, and its console is the pod (doc 16 §3.3).
    let state = rig
        .tw
        .world()
        .module_state::<mush_sim::SpatialState>()
        .expect("the simulation is composed");
    let station = mush_sim::StationId::new(cockpit::STATION).expect("a station id");
    let spec = state
        .stations
        .get(&tank)
        .and_then(|host| host.stations.get(&station))
        .expect("the tank carries a cockpit");
    assert_eq!(spec.space, Some(pod), "its console is the pod");
    assert_eq!(spec.role.as_str(), cockpit::STATION_ROLE);
    for channel in Channel::ALL {
        assert!(
            mush_core::ChannelId::new(channel.as_str())
                .is_some_and(|id| spec.channels.contains(&id)),
            "the cockpit grants {}",
            channel.as_str()
        );
    }
    // The seat is the pilot's, and so is the focus.
    assert_eq!(
        rig.seats(0),
        vec![(tank, cockpit::STATION.to_owned())],
        "one billet, on the tank"
    );
    let focus = rig
        .tw
        .world()
        .module_state::<mush_sim::FocusState>()
        .and_then(|state| state.focus_of_actor(rig.pilot(0)))
        .expect("the account's foreground focus is the cockpit")
        .clone();
    assert_eq!(focus.unit, tank);
    assert_eq!(focus.station.as_str(), cockpit::STATION);
    // And the grant the arena will authorise against.
    let grants = rig.tw.world().command_subjects_of(rig.pilot(0));
    assert!(
        grants.iter().any(|grant| grant.subject == tank
            && grant.station.as_str() == cockpit::STATION
            && grant.channels.len() == 3),
        "the seat publishes one three-channel grant on the tank"
    );

    // One of two is not every one; the second fires the trigger.
    assert_eq!(rig.state(), MatchState::Boarding);
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    assert!(
        mush_sim::deployed::mark_of(rig.tw.world(), tank).is_some(),
        "the check-out ran inside the second BOARD"
    );
}

// ---------------------------------------------------------------------------
// (2) The gate: eleven verbs, one line each
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_cockpit_verb_is_refused_with_one_line_where_it_means_nothing() {
    let mut rig = Rig::boot(2);
    // In the lobby, flying nothing.
    for verb in cockpit::COCKPIT_VERBS {
        let said = rig.say(0, verb);
        assert!(
            said.contains(cockpit::OUTSIDE_COCKPIT),
            "{verb} in the lobby: {said}"
        );
    }
    let said = rig.say(0, "board");
    assert!(
        said.contains(cockpit::OUTSIDE_TEAM_ROOM),
        "BOARD in the lobby: {said}"
    );
    let said = rig.say(0, "exit");
    assert!(
        said.contains(cockpit::NOT_IN_A_POD),
        "EXIT in the lobby: {said}"
    );

    // In a team room with no slot: past the gate, refused by the verb.
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    let said = rig.say(0, "board");
    assert!(said.contains("hold no slot"), "{said}");

    // A slot on the other side.
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::BLUE_ROOM_ENTRY);
    let said = rig.say(0, "board");
    assert!(said.contains("slot is on the red side"), "{said}");

    // And, seated, a second BOARD.
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(0, "board");
    assert!(said.contains("already in your pod"), "{said}");
}

/// **The window between `BOARD` and the trigger says what it is** (16b cut F.1).
///
/// The K row's deviation 13 records the mechanism: the cockpit gate needs a
/// deployed unit, and nothing is deployed until the last pilot boards. What the
/// first pilot to board was *told* was `OUTSIDE_COCKPIT` — the sentence for a
/// lost seat — after a `BOARD` line that had just instructed them to type
/// `STARTUP`. In a two-player hand test one pilot always boards first, so this
/// fired on the very first run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn between_board_and_the_trigger_the_panel_says_it_is_waiting() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    let said = rig.say(0, "board");
    assert!(
        said.contains("The panel wakes when every pilot is seated"),
        "BOARD still promises a panel that is about to refuse: {said}"
    );
    assert!(
        !said.contains("STARTUP"),
        "and it no longer names a verb that is refused: {said}"
    );
    assert!(
        said.contains("leaving abandons the tank for the rest of it"),
        "and it says what EXIT costs once the match runs: {said}"
    );
    for verb in cockpit::COCKPIT_VERBS {
        let said = rig.say(0, verb);
        assert!(
            said.contains(cockpit::NOT_STARTED_YET),
            "{verb} between BOARD and the trigger: {said}"
        );
        assert!(
            !said.contains(cockpit::OUTSIDE_COCKPIT),
            "{verb} is still answered as a fault: {said}"
        );
    }
    // A pilot who holds no seat at all still gets the other sentence.
    let said = rig.say(1, "startup");
    assert!(said.contains(cockpit::OUTSIDE_COCKPIT), "{said}");
}

/// **A tank that is already starting up is told to wait** (16b cut F.1; doc 16
/// §3.3 "Startup").
///
/// The window under test is `match.startup_ms` wide, so this test gets **its own
/// configuration** with a long one rather than the file's 200 ms: at the shared
/// value a descheduled test task, or one `pump` that drains the `Off`, `Starting`
/// and `Online` frames together, steps straight over the window and the test
/// fails against correct code. Ten seconds of arena time is not a slower test —
/// nothing here waits for power to finish — it is a window wide enough that
/// losing the race is a real defect.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_order_while_power_is_coming_up_says_it_is_still_starting() {
    let mut cfg = config();
    cfg.match_.startup_ms = 10_000;
    let mut rig = Rig::boot_with(2, cfg);
    rig.fly().await;
    let said = rig.say(0, "heading 90");
    assert!(
        said.contains(cockpit::POWER_NOT_ONLINE),
        "a cold tank: {said}"
    );
    rig.say(0, "startup");
    assert!(rig.frame_for(0).await, "the arena never answered");
    // Between the accepted `STARTUP` and `match.startup_ms`, the answer is the
    // other one — the pilot has already done the thing the cold refusal asks for.
    for _ in 0..200 {
        rig.pump();
        let said = rig.say(0, "heading 90");
        if said.contains(cockpit::POWER_STARTING) {
            assert!(
                !said.contains(cockpit::POWER_NOT_ONLINE),
                "and not both: {said}"
            );
            return;
        }
        if said.contains("Steering to 90") {
            panic!(
                "power came online inside a ten-second startup, so the window was never the \
                 cockpit's to report"
            );
        }
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }
    panic!("the cockpit never reported a starting tank");
}

// ---------------------------------------------------------------------------
// (3) A cold tank
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn heading_and_speed_before_startup_are_refused_with_power_not_online() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let said = rig.say(0, "heading 90");
    assert!(
        said.contains(cockpit::POWER_NOT_ONLINE),
        "HEADING before STARTUP: {said}"
    );
    assert!(
        said.to_ascii_lowercase().contains("power not online"),
        "and it says so in the arena's own words: {said}"
    );
    let said = rig.say(0, "speed 20");
    assert!(said.contains(cockpit::POWER_NOT_ONLINE), "{said}");
    // The argument is checked for shape, and a bad one never reaches the arena.
    let said = rig.say(0, "heading 400");
    assert!(said.contains("HEADING <0-359>"), "{said}");
    let said = rig.say(0, "speed 900");
    assert!(said.contains("kph"), "{said}");

    // STARTUP is never refused, and power comes up behind it.
    let said = rig.say(0, "startup");
    assert!(said.contains("Startup sequence"), "{said}");
    assert!(rig.powered(0).await, "the arena brings power online");
    let said = rig.say(0, "heading 90");
    assert!(said.contains("Steering to 90"), "{said}");
    let said = rig.say(0, "speed 20");
    assert!(said.contains("Making 20 kph"), "{said}");
}

// ---------------------------------------------------------------------------
// (4) `VIEW`
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn view_draws_the_platform_at_the_centre_of_a_grid() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let events = rig.events(0, "view");
    let frame = Rig::panel(&events).expect("VIEW emits the cockpit frame");
    assert_eq!(
        frame
            .get("schema")
            .and_then(|schema| schema.get("shape"))
            .and_then(serde_json::Value::as_str),
        Some("grid"),
        "the shape the ANSI renderer draws: {frame}"
    );
    let width = frame.get("width").and_then(serde_json::Value::as_u64);
    let height = frame.get("height").and_then(serde_json::Value::as_u64);
    assert_eq!(width, Some(u64::from(cockpit::VIEW_COLUMNS)));
    assert_eq!(height, Some(u64::from(cockpit::VIEW_ROWS)));
    // **The window is centred on the platform and clamped to the arena**, and
    // `you` is the cell the renderer centres its own viewport on. A tank that has
    // just deployed is standing in its side's entry zone, six cells from the west
    // edge, so the horizontal half of the rule is the clamp: the origin is the
    // arena's own edge and the platform draws at its true column. The vertical
    // half is the centring, because 118,500 cm is the middle of a 2.56 km window.
    let cell = |key: &str, n: usize| {
        frame
            .get(key)
            .and_then(serde_json::Value::as_array)
            .and_then(|cell| cell.get(n).cloned())
            .and_then(|value| value.as_i64())
            .unwrap_or(-1)
    };
    let cell_cm = frame
        .get("cell_cm")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(1);
    let x_cm = frame
        .get("x_cm")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    let y_cm = frame
        .get("y_cm")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);
    assert_eq!(
        cell("you", 0),
        (x_cm - cell("origin", 0)) / cell_cm,
        "the platform draws at its own column of the window: {frame}"
    );
    assert_eq!(
        cell("you", 1),
        (y_cm - cell("origin", 1)) / cell_cm,
        "and at its own row: {frame}"
    );
    assert_eq!(cell("origin", 0), 0, "clamped at the arena's west edge");
    assert_eq!(
        cell("you", 1),
        i64::from(cockpit::VIEW_ROWS / 2),
        "and centred where the arena has room for it: {frame}"
    );
    let units = frame
        .get("units")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        units.len(),
        1,
        "one platform and no marker: 16b cut S.2 draws one per contact, and two tanks in \n         opposite entry zones of a 2.56 km arena hold none"
    );
    assert_eq!(
        units
            .first()
            .and_then(|unit| unit.get("you"))
            .and_then(serde_json::Value::as_bool),
        Some(true)
    );
    // Every row is run-length encoded terrain, and there are as many as the
    // window is tall.
    let rows = frame
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(rows.len(), usize::from(cockpit::VIEW_ROWS));
    let cells: u64 = rows
        .first()
        .and_then(serde_json::Value::as_array)
        .map(|runs| {
            runs.iter()
                .filter_map(|run| run.get(1).and_then(serde_json::Value::as_u64))
                .sum()
        })
        .unwrap_or_default();
    assert_eq!(
        cells,
        u64::from(cockpit::VIEW_COLUMNS),
        "a row's runs cover the window: {frame}"
    );
}

/// The other half of the window rule: a platform the arena has room around
/// draws **in the middle**.
///
/// A deployed tank is always in its entry zone, against one edge of the arena,
/// so the live verb above can only show the clamp. This drives the same builder
/// with a frame at the middle of the 2.56 km window — which is where a tank is
/// ninety seconds after `SPEED`, and a test that waited for it would be ninety
/// seconds long.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_window_is_centred_on_a_platform_the_arena_has_room_around() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let held = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(rig.pilot(0)))
        .cloned()
        .expect("a frame");
    let middle = i64::from(match_::ARENA_CELLS / 2) * i64::from(match_::ARENA_CELL_CM);
    let frame = tacsim::arena::step::Frame {
        pose: mush_sim::data::Pose {
            x_cm: middle,
            y_cm: middle,
            ..held.frame.pose.clone()
        },
        ..held.frame.clone()
    };
    let frame = rig.with_ctx(0, |ctx| cockpit::view_frame(ctx, held.match_id, &frame));
    let cell = |key: &str, n: usize| {
        frame
            .get(key)
            .and_then(serde_json::Value::as_array)
            .and_then(|cell| cell.get(n).cloned())
            .and_then(|value| value.as_i64())
            .unwrap_or(-1)
    };
    assert_eq!(
        (cell("you", 0), cell("you", 1)),
        (
            i64::from(cockpit::VIEW_COLUMNS / 2),
            i64::from(cockpit::VIEW_ROWS / 2)
        ),
        "the platform is in the middle of the window: {frame}"
    );
}

// ---------------------------------------------------------------------------
// (5) The reads, and the frame's shape
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_reads_render_the_last_frame_and_the_frame_carries_doc_16_s_shape() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    let said = rig.say(0, "status");
    assert!(said.contains("power off"), "cold at first: {said}");
    assert!(said.contains("the arena is running"), "{said}");

    let said = rig.say(0, "contacts");
    assert!(
        said.contains("No contacts."),
        "nothing is held: the two tanks open 240 cells apart and the stock sensor          reaches 60 (16b cut S.2 renders what a platform does hold): {said}"
    );

    let events = rig.events(0, "weaponstatus");
    let said = rig.tw.seen_by(&events, rig.pilot(0)).join("\n");
    assert!(said.contains("cannon"), "{said}");
    assert!(said.contains("machine_gun"), "{said}");
    assert!(said.contains("round(s)"), "{said}");

    // The frame itself: doc 16 §5's status block, with the two sections cuts S
    // and W fill already in place.
    let frame = Rig::panel(&events).expect("a read emits the frame it read");
    for key in [
        "match",
        "unit",
        "x_cm",
        "y_cm",
        "heading_mdeg",
        "speed_cms",
        "power",
        "blocked",
        "tempo",
        "contacts",
        "weapons",
    ] {
        assert!(frame.get(key).is_some(), "the frame carries {key}: {frame}");
    }
    assert_eq!(
        frame
            .get("contacts")
            .and_then(serde_json::Value::as_array)
            .map(Vec::len),
        Some(0),
        "empty for want of a contact, not for want of a renderer (16b cut S.2)"
    );
    let weapons = frame
        .get("weapons")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(weapons.len(), 2, "the stock tank's cannon and machine gun");
    assert!(
        weapons
            .iter()
            .any(|weapon| weapon.get("id").and_then(serde_json::Value::as_str) == Some("cannon")),
        "{frame}"
    );
    // `VIEW`'s frame is the same one with the window added.
    let plain = frame.get("schema").and_then(|schema| schema.get("shape"));
    assert_eq!(plain, None, "a pulse frame carries no grid: {frame}");
}

// ---------------------------------------------------------------------------
// (6) `LOCK` and `FIRE`, until cuts S and W
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lock_refuses_every_id_identically_and_fire_refuses_for_want_of_one() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let malformed = rig.say(0, "lock zzzz");
    let well_formed = rig.say(0, "lock 0123456789abcdef");
    let empty = rig.say(0, "lock");
    assert!(malformed.contains(cockpit::NO_SUCH_CONTACT), "{malformed}");
    assert_eq!(
        malformed, well_formed,
        "one byte-identical refusal for every id the platform does not hold"
    );
    assert_eq!(empty, well_formed);

    let said = rig.say(0, "fire");
    assert!(said.contains(cockpit::NO_LOCK), "{said}");
    let named = rig.say(0, "fire cannon");
    assert_eq!(named, said, "the default weapon IS the cannon");
    let said = rig.say(0, "fire trebuchet");
    assert!(said.contains("no weapon called"), "{said}");
}

/// **16b cut Q.1: the two read verbs, and what they refuse with.**
///
/// The two tanks open 240 cells apart against a sixty-cell sight, so nothing is
/// held — which is exactly the state the refusals are about. `RANGE` and
/// `BEARING` take a contact, so they get their own line for "no lock and no
/// name", and `FIRE`'s stays what it was: `FIRE` takes a **weapon** and never a
/// contact, so telling its pilot they could name one would be a lie.
///
/// Every shape of name the platform does not hold takes one path: a slot
/// designator whose tank is out there and undetected, a provisional number
/// nothing issued, a designator of nothing at all, and the sixteen digits that
/// stopped being typable with this cut.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_and_bearing_refuse_with_their_own_line_and_one_refusal_for_a_name() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    for verb in ["range", "bearing"] {
        let said = rig.say(0, verb);
        assert!(
            said.contains(cockpit::NO_LOCK_OR_NAME),
            "{verb} with neither a lock nor a name: {said}"
        );
        assert!(
            !said.contains(cockpit::NO_LOCK),
            "{verb} answered FIRE's line: {said}"
        );
    }
    // `FIRE`'s own line is untouched, and the two are different sentences.
    let said = rig.say(0, "fire");
    assert!(said.contains(cockpit::NO_LOCK), "{said}");
    assert_ne!(cockpit::NO_LOCK, cockpit::NO_LOCK_OR_NAME);

    let mut answers: Vec<String> = Vec::new();
    for verb in ["lock", "range", "bearing"] {
        let mut one: Vec<String> = Vec::new();
        for name in ["ba", "BA", "X1", "ZZ", "0123456789abcdef", "zzzz"] {
            let said = rig.say(0, &format!("{verb} {name}"));
            assert!(
                said.contains(cockpit::NO_SUCH_CONTACT),
                "{verb} {name}: {said}"
            );
            one.push(said);
        }
        let first = one.first().cloned().unwrap_or_default();
        for said in &one {
            assert_eq!(
                said, &first,
                "{verb} answered two different refusals for two names it does not hold"
            );
        }
        answers.push(first);
    }
    // And the three verbs answer the same sentence as one another.
    let first = answers.first().cloned().unwrap_or_default();
    for said in &answers {
        assert_eq!(said, &first, "the three verbs refuse differently");
    }
}

// ---------------------------------------------------------------------------
// (7) `EXIT`
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exit_gives_the_seat_back_and_stands_the_pilot_in_the_team_room() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pod = rig.pod(0).expect("a locked pod");
    let room = rig.tw.world().get(pod).and_then(|o| o.location);
    let tank = rig.tank(0);
    let before = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.epoch_of(tank, Channel::Fire))
        .unwrap_or_default();

    let said = rig.say(0, "exit");
    assert!(said.contains("climb out"), "{said}");
    assert_eq!(rig.location(0), room, "back in the team room");
    assert!(rig.seats(0).is_empty(), "the billet is given back");
    assert!(
        rig.tw
            .world()
            .module_state::<mush_sim::FocusState>()
            .and_then(|state| state.focus_of_actor(rig.pilot(0)))
            .is_none(),
        "and so is the focus"
    );
    let after = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.epoch_of(tank, Channel::Fire))
        .unwrap_or_default();
    assert!(
        after > before,
        "EXIT raised the arena's epoch, which is what purges a queued order \
         ({before} -> {after})"
    );
    // And the cockpit verbs mean nothing again.
    let said = rig.say(0, "status");
    assert!(said.contains(cockpit::OUTSIDE_COCKPIT), "{said}");
}

/// **`EXIT` mid-match stops the leak** (16b cut F.1; doc 16 §11's N.1
/// amendment, §3.6).
///
/// The 2026-09-18 review found two things behind the one-way leave, and this is
/// the witness for both:
///
/// * **the frames.** `Arena::pilots` was fixed at check-out and nothing pruned
///   it, so the arena went on building a `Frame` for a pilot who had given the
///   seat up — pose, heading, the weapons table and every contact's cell, for a
///   tank the reflex controller was driving. The world's own mirror of that is
///   `ArenaRegistry::last_frame`, written by `on_frames` for every pilot the
///   arena addresses: drained to nothing at `EXIT` and *still* nothing after a
///   window of pulses is the statement that the arena addressed them no more.
/// * **the binding.** `ArenaRegistry::pilots` survived `EXIT` too, so a reconnect
///   fired `Control { Live }` off the stale row and took the tank back off the
///   controller with nobody in the cockpit. Two witnesses, because the binding
///   being gone and no control being sent are two facts: the registry answers no
///   seat, and the arena's **epoch mirror does not move** across a `Connected`
///   and a `Revived` — a control cannot reach the arena without `send_control`
///   raising it first.
///
/// The arena-side half — that `StepOutput::frames` carries no row for a released
/// pilot, at three cadences — is `crates/tacsim/tests/arena.rs`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exit_mid_match_takes_the_pilot_off_the_frame_roster_and_off_the_binding() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    let tank = rig.tank(0);
    // The arena is talking to this pilot before they leave, so the assertion
    // below is about a stream that existed.
    assert!(
        rig.frame_for(0).await,
        "the arena never sent the pilot a frame at all"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.seat_of_pilot(pilot)),
        Some((rig.centre().match_id.expect("a running match"), tank)),
        "the check-out bound the pilot"
    );

    rig.say(0, "exit");

    // Drain everything the arena had already queued, and give it many pulses to
    // apply the `Released` — after this window nothing new can be in flight.
    for _ in 0..80 {
        rig.pump();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let settled = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot))
        .cloned();
    // The second window: whatever the ex-pilot's slot held after the first one is
    // what it still holds, because the arena builds them nothing.
    for _ in 0..80 {
        rig.pump();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let later = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot))
        .cloned();
    assert!(
        settled.is_none(),
        "EXIT left the ex-pilot's frame mirror in place"
    );
    assert!(
        later.is_none(),
        "the arena sent the ex-pilot another frame after they left"
    );

    // The binding is gone, so a reconnect resolves no seat and sends no `Live`.
    assert_eq!(
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.seat_of_pilot(pilot)),
        None,
        "the ex-pilot is still bound to the match"
    );
    // **And a reconnect sends no `Control { Live }`.** The observable is the
    // arena's own epoch mirror: `send_control` raises `(unit, channel)` before it
    // offers the message, so a `connected`/`revived` that resolved the stale
    // binding and called `control_all_channels(.., Live, ..)` would raise all
    // three by one. Unchanged across both transitions is the statement that no
    // control was sent at all — the same number
    // `linkdead_onset_and_revival_raise_the_arenas_epoch_without_releasing_the_seat`
    // reads the other way round. The hook's own lines are read too, from the
    // outbox `fire_conn` hands back, because the reclaim line is what a person
    // would have seen.
    let before: Vec<u64> = Channel::ALL.iter().map(|c| rig.epoch(tank, *c)).collect();
    let mut events = rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    events.extend(rig.fire_conn(pilot, mush_core::ConnTransition::Revived));
    let said = rig.tw.seen_by(&events, pilot).join("\n");
    let after: Vec<u64> = Channel::ALL.iter().map(|c| rig.epoch(tank, *c)).collect();
    assert_eq!(
        after, before,
        "a reconnect sent the arena a control off a binding that was given up"
    );
    assert!(
        !said.contains("back at your station"),
        "a reconnect reclaimed a seat that was given up: {said}"
    );
    // And the centre says so: the slot is `left`, not `seated`.
    let said = rig.say(0, "sim status");
    let red_row = said
        .lines()
        .find(|line| line.trim_start().starts_with("red:"))
        .unwrap_or_default()
        .to_owned();
    assert!(
        red_row.contains(", left"),
        "SIM STATUS still calls the ex-pilot seated: {said}"
    );
    assert!(
        !red_row.contains(", seated"),
        "SIM STATUS still calls the ex-pilot seated: {said}"
    );
    // And the pilot who stayed is still seated, so `left` is a fact about one
    // slot and not a word the renderer started printing everywhere.
    assert!(
        said.lines()
            .any(|line| line.trim_start().starts_with("blue:") && line.contains(", seated")),
        "{said}"
    );
}

/// **A pilot who QUITs is `left` too, and not `seated`** (16b cut F.1; doc 16
/// §3.8's "Disconnects").
///
/// `EXIT` is the route a reviewer thinks of and the rarest one in a playtest.
/// `SeatFacts::disconnected` — a clean `QUIT`, and the linkdead grace running out
/// — sends the **same** `Control { Released }` naming the same pilot, and since
/// this cut it does everything `EXIT` does to their standing: the arena drops
/// them from its frame roster, the registry drops the binding, and a relogin
/// takes nothing back. So `SIM STATUS` must not go on calling them `seated`
/// either, and the flag is written where the release is decided rather than in
/// one verb.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pilot_whose_session_ends_mid_match_is_left_on_the_board_too() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    assert!(
        rig.centre()
            .slot_of_pilot(pilot)
            .is_some_and(|slot| slot.is_seated() && !slot.left),
        "the slot starts seated and present"
    );

    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);

    assert_eq!(
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.seat_of_pilot(pilot)),
        None,
        "a QUIT left the pilot bound to the match"
    );
    let said = rig.say(1, "sim status");
    let red_row = said
        .lines()
        .find(|line| line.trim_start().starts_with("red:"))
        .unwrap_or_default()
        .to_owned();
    assert!(
        red_row.contains(", left"),
        "SIM STATUS calls a pilot who quit mid-match seated: {said}"
    );
    assert!(
        !red_row.contains(", seated"),
        "SIM STATUS calls a pilot who quit mid-match seated: {said}"
    );
    assert!(
        said.lines()
            .any(|line| line.trim_start().starts_with("blue:") && line.contains(", seated")),
        "and the pilot who stayed is still seated: {said}"
    );
}

/// **Doc 16 §3.8: the pods are released with the seats.** A pilot who never
/// climbed out is stood in the team room when the match ends, because the centre
/// says the pods are free and a body in one is the half of "free" that `BOARD`
/// created.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_match_that_ends_with_a_pilot_aboard_opens_the_hatch() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pod = rig.pod(0).expect("a locked pod");
    let room = rig.tw.world().get(pod).and_then(|o| o.location);
    assert_eq!(rig.location(0), Some(pod), "both pilots are still aboard");

    rig.say(0, "sim abort");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    assert_eq!(rig.location(0), room, "the hatch opened");
    assert!(rig.seats(0).is_empty(), "and the billet went back");
    // And the centre can run another match with the same pods and pilots.
    rig.staff("@arena/reset");
    assert_eq!(rig.state(), MatchState::Idle);
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    rig.fly().await;
    assert_eq!(
        rig.state(),
        MatchState::Running,
        "a second match seats them"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exit_while_boarding_unseats_the_slot_and_the_match_waits_again() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    assert!(
        rig.centre()
            .slot_of_pilot(rig.pilot(0))
            .is_some_and(tacsim::match_::Slot::is_seated),
        "the slot is seated"
    );
    rig.say(0, "exit");
    assert!(
        rig.centre()
            .slot_of_pilot(rig.pilot(0))
            .is_some_and(|slot| !slot.is_seated()),
        "and the trigger stops counting it"
    );
    assert_eq!(rig.state(), MatchState::Boarding, "nothing started");
    // The slot and its pod are still the pilot's: they may board again.
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
}

// ---------------------------------------------------------------------------
// (8) K4: the kernel's linkdead hook reaches the arena
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn linkdead_onset_and_revival_raise_the_arenas_epoch_without_releasing_the_seat() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let tank = rig.tank(0);
    let pilot = rig.pilot(0);
    let epoch = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.epoch_of(tank, Channel::Helm))
            .unwrap_or_default()
    };
    let before = epoch(&rig);

    rig.fire_conn(pilot, mush_core::ConnTransition::Linkdead);
    let onset = epoch(&rig);
    assert!(
        onset > before,
        "linkdead onset raises the epoch ({before} -> {onset})"
    );
    assert_eq!(
        rig.seats(0),
        vec![(tank, cockpit::STATION.to_owned())],
        "and keeps the seat: the tank steers on and fires nothing"
    );

    rig.fire_conn(pilot, mush_core::ConnTransition::Revived);
    let revived = epoch(&rig);
    assert!(
        revived > onset,
        "the reclaim is a NEW epoch ({onset} -> {revived})"
    );
    assert_eq!(
        rig.seats(0),
        vec![(tank, cockpit::STATION.to_owned())],
        "the seat was never given up"
    );
}

// ---------------------------------------------------------------------------
// (9) The verbs are the ones doc 16 §3.7 names
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_team_room_a_pilot_boards_from_is_the_one_their_slot_is_on() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    // Slot 1 of each side is pod 1 of that side's room.
    let red = match_::pod_object(rig.tw.world(), Team::Red, 1).expect("red pod 1");
    let blue = match_::pod_object(rig.tw.world(), Team::Blue, 1).expect("blue pod 1");
    assert_eq!(rig.pod(0), Some(red));
    assert_eq!(rig.pod(1), Some(blue));
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.location(0), Some(red));
    assert_eq!(rig.location(1), Some(blue));
    // Two pilots, two tanks, two sides.
    assert_ne!(rig.tank(0), rig.tank(1));
}
