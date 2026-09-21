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
use tacsim::match_::{self, BoardReader, MatchState, cockpit};

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
    /// **16b cut Q.3**: everything the rig has ever emitted, in order.
    ///
    /// Two of this cut's claims are about what was **never** sent — no
    /// `arena.map` frame unasked, one cleared cockpit per pilot and not two —
    /// and an absence can only be asserted against a whole record.
    log: Vec<(Audience, OutputEvent)>,
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
            log: Vec::new(),
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
        let out = self.tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
            actor: GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        });
        self.log.extend(out);
        tasks.stamp_ledger_writes(1);
        tasks.ack_ledger_upto(1);
    }

    fn pilot(&self, n: usize) -> ObjId {
        *self.pilots.get(n).expect("that many pilots were booted")
    }

    /// **The frame the world's mirror holds for that pilot** (16b cut Q.6), cloned
    /// so a test can hold one pulse's frame and wait for the next.
    ///
    /// The mirror only moves on [`Rig::pump`], which is the whole reason this
    /// exists: an order typed with [`Rig::say`] is forwarded as an intent and
    /// resolved a pulse later, so a frame read without pumping first is the frame
    /// from *before* the order and says nothing about it.
    fn frame(&self, n: usize) -> Option<tacsim::arena::step::Frame> {
        self.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(self.pilot(n)))
            .map(|held| held.frame.clone())
    }

    /// The same, insisting there is one.
    fn held_frame(&self, n: usize) -> tacsim::arena::step::Frame {
        self.frame(n).expect("the arena has spoken to that pilot")
    }

    /// **Hand the world ONE frame through the real egress seam** (16b cut D.1).
    ///
    /// [`Rig::pump`] moves whatever the live arena task has produced; this stages
    /// a frame a test composed and fires the same re-entry, so `on_frames` runs
    /// the production path over it — the notices, the mirror, the hint's two
    /// frame-borne ends. Nothing is drained from the task's own egress, so the
    /// staged frame is the last one `on_frames` sees.
    ///
    /// It is the arena's own message on the arena's own seam: no fixture stands
    /// in for a module (16b §0.4), and there is no other way to put a platform
    /// with no weapon left, or a refused `STARTUP`, in front of the cockpit.
    fn push_frame(&mut self, n: usize, frame: tacsim::arena::step::Frame) {
        let pilot = self.pilot(n);
        let match_id = self.centre().match_id.expect("a running match");
        let Some(tasks) = self
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
        else {
            panic!("the arena registry is composed");
        };
        assert!(
            tasks.stage(tacsim::arena_task::ArenaOut::Frames {
                match_id,
                frames: vec![(pilot, frame)],
                now_ms: 0,
            }),
            "the egress staging table took the frame"
        );
        let out = self.tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
            actor: GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        });
        self.log.extend(out);
    }

    /// **Whether that pilot's `Starting` hint stands now** (16b cut D.1).
    fn hinted(&self, n: usize) -> bool {
        let pilot = self.pilot(n);
        let now = self.tw.world().now();
        self.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .is_some_and(|registry| registry.starting_hint(pilot, now))
    }

    /// Stamp that pilot's hint to expire at an absolute world second (16b cut
    /// D.1), so a test can reach the expiry without sleeping.
    fn hint_until(&mut self, n: usize, until_s: i64) {
        let pilot = self.pilot(n);
        if let Some(registry) = self
            .tw
            .world_mut()
            .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
        {
            registry.hint_starting(pilot, until_s);
        }
    }

    fn say(&mut self, n: usize, line: &str) -> String {
        let actor = self.pilot(n);
        let events = self.events_of(actor, line);
        self.tw.seen_by(&events, actor).join("\n")
    }

    /// Run one line as God (staff).
    fn staff(&mut self, line: &str) -> String {
        let events = self.events_of(GOD, line);
        self.tw.seen_by(&events, GOD).join("\n")
    }

    /// Run one line and hand back every event it produced, so a test can look at
    /// a panel frame and not only at its text.
    fn events(&mut self, n: usize, line: &str) -> Vec<(Audience, OutputEvent)> {
        let actor = self.pilot(n);
        self.events_of(actor, line)
    }

    /// One line as `actor`, recorded into [`Rig::log`].
    fn events_of(&mut self, actor: ObjId, line: &str) -> Vec<(Audience, OutputEvent)> {
        let events = self.tw.run(actor, line);
        self.log.extend(events.iter().cloned());
        events
    }

    /// **16b cut Q.3**: every panel frame of `feed` the rig has ever emitted,
    /// with the audience it went to.
    fn panels(&self, feed: &str) -> Vec<(Audience, serde_json::Value)> {
        self.log
            .iter()
            .filter_map(|(audience, event)| match event {
                OutputEvent::Panel {
                    feed: sent, data, ..
                } if sent.as_str() == feed => Some((audience.clone(), data.clone())),
                _ => None,
            })
            .collect()
    }

    /// **16b cut Q.3**: one registered query, answered for `actor` exactly as
    /// the engine's `Inbound::Query` handling answers it.
    fn query(&self, actor: ObjId, feed: &str) -> serde_json::Value {
        let (_owner, handler) = self
            .tw
            .registry()
            .query_handler(feed)
            .unwrap_or_else(|| panic!("{feed} has a registered query handler"));
        match handler.query(self.tw.world(), actor, &serde_json::json!({})) {
            Ok(OutputEvent::Panel {
                feed: answered,
                kind,
                data,
            }) => {
                assert_eq!(answered.as_str(), feed, "answered on its own feed");
                assert_eq!(kind, mush_proto::PanelFrameKind::Keyframe);
                data
            }
            other => panic!("{feed} must answer a keyframe, got {other:?}"),
        }
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

    /// **Stand one unit on an arena cell** (16b cut Q.3), through the staff
    /// placement verb the arena story opens with — the only way a test puts two
    /// tanks inside each other's sensors without driving them there for ninety
    /// seconds. The arena takes the placement at check-out, so it is done while
    /// the centre is `Boarding`.
    fn place(&mut self, unit: ObjId, cell: (i64, i64), heading_mdeg: u32) {
        let domain = mush_sim::pose_of(self.tw.world(), unit)
            .expect("the tank is placed on the proving ground")
            .0;
        let said = self.staff(&format!(
            "@spatial/place/domain #{}={},{{\"x_cm\":{},\"y_cm\":{},\"heading_mdeg\":{heading_mdeg}}}",
            unit.0,
            domain.0,
            cell.0 * 1_000 + 500,
            cell.1 * 1_000 + 500
        ));
        assert!(said.contains("Placed"), "the placement was refused: {said}");
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
        let out = self.tw.fire_due(seconds.saturating_mul(1_000));
        self.log.extend(out);
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

/// **`VIEW`'s payload is what it always was, apart from `retain`** (16b cut
/// Q.3).
///
/// The cut split the window out into the pure [`cockpit::view_window`] the
/// fixtures call, and claimed the live surface did not move. That claim is two
/// things, and both are pinned here:
///
/// * the live payload is [`cockpit::view_window`]'s, with **one** field added —
///   the pilot's own tank's display name, which a window with no world cannot
///   ask for and which [`cockpit::view_window`] therefore leaves out rather than
///   filling with a callsign the server never sends;
/// * the `schema` object is the pre-cut one plus `retain`, and nothing else.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_live_window_is_the_pure_one_plus_the_tank_s_own_name() {
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
    let ground = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.arena_of(held.match_id))
        .cloned()
        .expect("the running match's ground");
    let unit = held.frame.unit;
    let name = mush_core::output::display_name(rig.tw.world(), unit);

    let live = rig.with_ctx(0, |ctx| {
        cockpit::view_frame(ctx, held.match_id, &held.frame)
    });
    let mut pure = cockpit::view_window(
        cockpit::status_frame(held.match_id, &held.frame),
        ground.id.as_str(),
        &ground.terrain,
        &held.frame,
    );

    // The pure window says nothing about what the tank is called, and the live
    // one says the name it has always said — never the callsign.
    let you = pure
        .get("units")
        .and_then(serde_json::Value::as_array)
        .and_then(|units| units.first())
        .cloned()
        .expect("the platform's own marker is first");
    assert_eq!(
        you.get("you").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(
        you.get("name").is_none(),
        "a pure window invents no name: {you}"
    );
    let live_you = live
        .get("units")
        .and_then(serde_json::Value::as_array)
        .and_then(|units| units.first())
        .cloned()
        .expect("the live payload's own marker is first too");
    assert_eq!(
        live_you.get("name").and_then(serde_json::Value::as_str),
        Some(name.as_str()),
        "the live own marker carries the tank's display name: {live_you}"
    );
    assert_ne!(
        live_you.get("name").and_then(serde_json::Value::as_str),
        Some(held.frame.callsign.to_string().as_str()),
        "and not the callsign the cockpit frame already carries"
    );

    // With that one field put back, the two payloads are the same payload.
    if let Some(marker) = pure
        .get_mut("units")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|units| units.first_mut())
        .and_then(serde_json::Value::as_object_mut)
    {
        marker.insert("name".to_owned(), serde_json::json!(name));
    }
    assert_eq!(live, pure, "the live window is the pure one, plus the name");

    // And the schema is the one cut K.2 wrote, plus the marker the session ring
    // reads — the whole of what this cut changed about what `VIEW` sends.
    assert_eq!(
        live.get("schema"),
        Some(&serde_json::json!({
            "id": "arena.cockpit",
            "version": 1,
            "shape": "grid",
            "retain": "latest",
        }))
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
    // **16b cut D.1** (`T-59`): the pilot's own alert state, where the arena's
    // one global tempo word used to be. The two tanks open 240 cells apart and
    // the stock sight reaches 60, so this platform holds nothing.
    assert!(said.contains("scope clear"), "{said}");
    assert!(
        !said.contains("the arena is running"),
        "the arena's global tempo word is not the pilot's own state: {said}"
    );

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
    // **16b cut D.3 (`T-60`)**: an empty argument is not an id that failed to
    // resolve, it is a pilot who has not said which contact — so it is usage and
    // not the one refusal. Every NON-EMPTY word still takes the one path, which
    // is what keeps `LOCK BA` against an unheld `BA` indistinguishable from
    // `LOCK ZZ`.
    assert!(
        empty.contains("LOCK <contact>; CONTACTS lists them."),
        "{empty}"
    );
    assert!(
        !empty.contains(cockpit::NO_SUCH_CONTACT),
        "a bare LOCK still answered about the scope: {empty}"
    );

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
///
/// **16b cut Q.2** put `SCAN` on the same two rules. It matters more there than
/// on the other two: `SCAN` of a contact the platform holds has three different
/// answers of its own (unidentified, no reading, a reading), so a fourth answer
/// for a contact it does *not* hold would be the one place the refusals could say
/// which tanks are out there.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn range_and_bearing_refuse_with_their_own_line_and_one_refusal_for_a_name() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    // 16b cut Q.2 put `SCAN` on the same three rules: the named contact, else the
    // lock, else this one line.
    for verb in ["range", "bearing", "scan"] {
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
    for verb in ["lock", "range", "bearing", "scan"] {
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

/// **A lock taken on `X1` is still a lock when the row becomes `BA`, at the
/// cockpit** (16b cut Q.6; the owner's ruling of 2026-09-20).
///
/// The owner's words: *"new contact X1 until identified is fine, but if someone
/// locks X1 and it becomes BA then their lock should continue, since it is the
/// same contact even though the letters changed."* `weapons.rs` proves the arena
/// keeps the lock; this proves the **cockpit** does not need it re-taken, which
/// is where a pilot would find out.
///
/// Three claims, in the order a pilot meets them:
///
/// 1. the rename does not disturb the lock — the frame's opaque `lock` is the
///    same sixteen digits before and after, and nothing says the lock broke;
/// 2. **both spellings still resolve to it**: `LOCK BA` (the new name) and
///    `LOCK X1` (the number the platform gave it first, which `ContactRow::alias`
///    keeps) are the same order on the same contact, and `resolve_lock` is
///    idempotent, so neither drops it and neither takes it again. Each repeat is
///    given a pulse before its frame is read — the order is an intent and the
///    mirror only moves on [`Rig::pump`] — because a frame read straight after
///    `say` is the frame from *before* the order and would make the claim true
///    without testing it. The ledger half of the same claim, that a repeat
///    journals no second `locked` line, is `weapons.rs`'s (5b);
/// 3. the four read verbs that fall back to the lock — `RANGE`, `BEARING`,
///    `WEAPONSTATUS` and `SCAN`, each with no argument — answer **for that
///    contact under its new name**, so a pilot who locked `X1` reads `BA`
///    everywhere the moment the platform tells it apart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_lock_taken_on_a_provisional_number_still_answers_to_both_names() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    // Fourteen cells apart up the middle, as `VIEW`'s own fixture stands them:
    // well inside the stock sixty-cell sight and inside the half-reach a `SCAN`
    // needs, so the only thing this test waits for is the classification clock.
    let middle = i64::from(match_::ARENA_CELLS / 2);
    rig.place(rig.tank(0), (middle, middle), 0);
    rig.place(rig.tank(1), (middle, middle + 14), 180_000);
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    rig.say(0, "startup");
    rig.say(1, "startup");
    assert!(rig.powered(0).await, "the tank wakes up");

    // Pump until the platform holds the other tank and has **not** yet told it
    // apart: that window is `classify_after_ms` long and it is the only moment
    // this test can take its lock in.
    let mut provisional = None;
    for _ in 0..900 {
        rig.pump();
        let row = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(rig.pilot(0)))
            .map(|held| cockpit::frame_contacts(&held.frame))
            .unwrap_or_default()
            .first()
            .cloned();
        if let Some(row) = row
            && row.class.as_str() == "unknown"
        {
            provisional = Some(row);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let provisional = provisional.expect("the platform holds an unidentified return");
    let number = provisional.designator.to_string();
    assert_eq!(number, "X1", "the first return this platform ever held");
    assert_eq!(
        provisional.alias, None,
        "while unknown, the number IS the name"
    );

    let said = rig.say(0, &format!("lock {number}"));
    assert!(said.contains(&format!("Locking {number}.")), "{said}");
    // The order is forwarded as an intent and resolved in the arena's next pulse
    // phase 4, so the lock reaches the world's mirror a pulse later.
    let mut locked_on = None;
    for _ in 0..300 {
        rig.pump();
        locked_on = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(rig.pilot(0)))
            .and_then(|held| cockpit::frame_lock(&held.frame));
        if locked_on.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        locked_on,
        Some(provisional.id),
        "the lock landed on the unidentified return"
    );

    // Now let the classification clock run out.
    let mut named = None;
    for _ in 0..900 {
        rig.pump();
        let row = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(rig.pilot(0)))
            .map(|held| cockpit::frame_contacts(&held.frame))
            .unwrap_or_default()
            .first()
            .cloned();
        if let Some(row) = row
            && row.class.as_str() != "unknown"
        {
            named = Some(row);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let named = named.expect("the return is told apart");

    // (1) The same contact, under a new name, with the lock untouched.
    assert_eq!(named.id, provisional.id, "a rename is not a new contact");
    assert_eq!(
        named.designator.to_string(),
        "BA",
        "the other side's first slot"
    );
    assert_eq!(
        named.alias,
        Some(1),
        "and the number it was held under is kept"
    );
    let still = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(rig.pilot(0)))
        .and_then(|held| cockpit::frame_lock(&held.frame));
    assert_eq!(still, locked_on, "the rename moved the lock");
    assert_eq!(
        still,
        Some(named.id),
        "and it is on the row that was renamed"
    );

    // (2) Both spellings, and neither is a second lock. `resolve_lock` journals
    //     nothing for a lock already held, so what a repeat may not do is change
    //     the frame's lock or say the lock broke.
    //
    //     Each repeat is given a pulse before its frame is read. The order is
    //     forwarded as an intent and resolved in the arena's next pulse, so a
    //     frame read straight after `say` is the one from before the order and
    //     would make the assertion below true by construction.
    let spoken = rig.log.len();
    for word in ["BA", "ba", &number, "x1"] {
        let said = rig.say(0, &format!("lock {word}"));
        assert!(
            !said.contains(cockpit::NO_SUCH_CONTACT),
            "LOCK {word} was refused after the rename: {said}"
        );
        assert!(said.contains("Locking BA."), "LOCK {word}: {said}");
        let before = rig.frame(0).expect("a seated pilot holds a frame");
        let mut pulsed = false;
        for _ in 0..600 {
            rig.pump();
            if rig.frame(0).is_some_and(|now| !now.same_as(&before)) {
                pulsed = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(pulsed, "no pulse reached the mirror after LOCK {word}");
        let held = rig.frame(0).and_then(|frame| cockpit::frame_lock(&frame));
        assert_eq!(held, still, "LOCK {word} re-took or dropped the lock");
    }
    // And the pilot was never told the lock went. A repeat **does** answer with
    // the hit chance — phase 4 stages the `Locked` notice on every accepted order,
    // which is where `Locked on BA, 88% to hit.` comes from (16b cut F.2) — so the
    // line that would mean something here is the one that says the lock broke, and
    // `weapons.rs`'s claim (5b) is what proves the repeat journalled no second
    // lock.
    let told: Vec<String> = rig
        .log
        .iter()
        .skip(spoken)
        .filter_map(|(_, event)| match event {
            OutputEvent::Sys(text) => Some(text.to_plain_text()),
            _ => None,
        })
        .collect();
    assert!(
        !told.iter().any(|line| line.contains("broken")),
        "a repeated LOCK broke the lock: {told:?}"
    );
    assert!(
        told.iter().any(|line| line.starts_with("Locked on BA,")),
        "a repeated LOCK did not answer with the lock it already held: {told:?}"
    );

    // (3) The four reads that fall back to the lock, each with no argument.
    for verb in ["range", "bearing", "weaponstatus", "scan"] {
        let said = rig.say(0, verb);
        assert!(
            said.contains("BA"),
            "{verb} did not answer for the locked contact under its new name: {said}"
        );
        assert!(
            !said.contains(cockpit::NO_LOCK_OR_NAME) && !said.contains(cockpit::NO_LOCK),
            "{verb} answered as though there were no lock: {said}"
        );
        assert!(
            !said.contains(&named.id.to_string()),
            "{verb} printed the sixteen digits: {said}"
        );
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
    // And the centre says so: the slot is `left`, not `seated`. Read by the
    // ex-pilot themselves, whose own side this is — **16b cut D.1** reduced every
    // OTHER side's rows while a match runs, so the blue row below is read off the
    // staff board instead.
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
    let staff = tacsim::match_::status_lines(rig.tw.world(), BoardReader::Staff).join("\n");
    assert!(
        staff
            .lines()
            .any(|line| line.trim_start().starts_with("blue:") && line.contains(", seated")),
        "{staff}"
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
    // **16b cut D.1**: the red row is read off the staff board, because the blue
    // pilot who is still in the match is not entitled to red's standing while it
    // runs — which is the whole of `B-01`, and is pinned in
    // `match_lifecycle.rs`. What this test is about is that the release wrote
    // `left` at all, and that the renderer prints it.
    let staff = tacsim::match_::status_lines(rig.tw.world(), BoardReader::Staff).join("\n");
    let red_row = staff
        .lines()
        .find(|line| line.trim_start().starts_with("red:"))
        .unwrap_or_default()
        .to_owned();
    assert!(
        red_row.contains(", left"),
        "SIM STATUS calls a pilot who quit mid-match seated: {staff}"
    );
    assert!(
        !red_row.contains(", seated"),
        "SIM STATUS calls a pilot who quit mid-match seated: {staff}"
    );
    let said = rig.say(1, "sim status");
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

// ---------------------------------------------------------------------------
// (10) The two feeds (16b cut Q.3; doc 16 §3.2, §5)
// ---------------------------------------------------------------------------

/// **The map is the answer to a question and nothing else.**
///
/// A seated pilot who asks gets the ground; a bystander, a staffer and a pilot
/// who has typed `EXIT` get the same two keys, which say nothing about any
/// match. And in a whole match nobody is ever *sent* one unasked — the one-shot
/// push of the biggest message at the least reliable moment is exactly what this
/// design refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_arena_map_is_pulled_by_a_seated_pilot_and_pushed_to_nobody() {
    let mut rig = Rig::boot(3);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    // (a) A seated pilot gets the whole ground.
    let map = rig.query(rig.pilot(0), tacsim::arena_bridge::MAP_FEED);
    assert_eq!(
        map.get("schema").and_then(|s| s.get("id")),
        Some(&serde_json::json!("arena.map"))
    );
    assert_eq!(
        map.get("width").and_then(serde_json::Value::as_u64),
        Some(u64::from(match_::ARENA_CELLS)),
        "the whole arena and not a window: {map}"
    );
    assert_eq!(
        map.get("cell_cm").and_then(serde_json::Value::as_u64),
        Some(u64::from(match_::ARENA_CELL_CM))
    );
    assert!(map.get("units").is_none(), "no unit of any kind: {map}");
    assert!(map.get("seated").is_none(), "a seated pilot is not told so");

    // (b) A bystander in the lobby, and staff, get the unseated answer — which
    // does not even say whether a match is running.
    let unseated = cockpit::map_unseated();
    assert_eq!(
        rig.query(rig.pilot(2), tacsim::arena_bridge::MAP_FEED),
        unseated,
        "a bystander"
    );
    assert_eq!(
        rig.query(GOD, tacsim::arena_bridge::MAP_FEED),
        unseated,
        "and staff, who are not flying either"
    );

    // (c) And a pilot who has left.
    rig.say(0, "exit");
    assert_eq!(
        rig.query(rig.pilot(0), tacsim::arena_bridge::MAP_FEED),
        unseated,
        "the binding went with the seat, so the answer did too"
    );

    // (d) Nothing on that feed was ever pushed, to anybody, at any point.
    assert!(
        rig.panels(tacsim::arena_bridge::MAP_FEED).is_empty(),
        "the map is pulled, never pushed: {:?}",
        rig.panels(tacsim::arena_bridge::MAP_FEED)
    );
}

/// The cockpit query answers the world's own mirror — the same frame `STATUS`
/// and `CONTACTS` render — and answers anybody else the cleared frame.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_cockpit_query_answers_the_mirror_and_an_unseated_asker_nothing() {
    let mut rig = Rig::boot(3);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    let held = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(rig.pilot(0)))
        .cloned()
        .expect("the mirror holds a frame");
    let answered = rig.query(rig.pilot(0), tacsim::arena_bridge::FRAME_FEED);
    assert_eq!(
        answered,
        cockpit::status_frame(held.match_id, &held.frame),
        "the query is a re-render of the mirror and never a second model"
    );
    assert_eq!(
        answered.get("notices"),
        Some(&serde_json::json!([])),
        "the mirror is stored with its notices cleared, so a resync says nothing twice"
    );

    let cleared = cockpit::cockpit_cleared();
    assert_eq!(
        rig.query(rig.pilot(2), tacsim::arena_bridge::FRAME_FEED),
        cleared
    );
    assert_eq!(rig.query(GOD, tacsim::arena_bridge::FRAME_FEED), cleared);

    // **And a pilot who has left is a bystander from the next line of code**,
    // even though a pulse that was already staged when she typed `EXIT` is
    // drained after it. Seated is the registry's binding and never the mirror: a
    // query is answerable from the lobby at any time, and answering an ex-pilot
    // out of a stale mirror would hand her a whole tank's contact list, locks,
    // bearings and ranges.
    rig.say(0, "exit");
    for _ in 0..40 {
        rig.pump();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        rig.query(rig.pilot(0), tacsim::arena_bridge::FRAME_FEED),
        cleared,
        "the binding went with the seat, so the answer did too"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(rig.pilot(0)))
            .is_none(),
        "and no pulse in flight wrote the mirror again"
    );
}

/// **The cleared cockpit**: one frame to the pilot who left, none to anybody
/// else, and one to whoever is still aboard when the match ends.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pilot_who_stops_flying_is_sent_one_cleared_cockpit_and_nobody_else_is() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    assert!(rig.frame_for(1).await, "and one to the other pilot");

    let cleared = cockpit::cockpit_cleared();
    let count = |rig: &Rig, who: ObjId| {
        rig.panels(tacsim::arena_bridge::FRAME_FEED)
            .into_iter()
            .filter(|(audience, data)| *audience == Audience::Actor(who) && *data == cleared)
            .count()
    };
    assert_eq!(count(&rig, rig.pilot(0)), 0, "nothing cleared while flying");

    // `EXIT` clears the leaver's panel, once — three channels release one seat,
    // and only the first of them unbinds anybody.
    let said = rig.say(0, "exit");
    assert!(
        said.contains("climb out"),
        "the ordinary line is unchanged: {said}"
    );
    assert_eq!(count(&rig, rig.pilot(0)), 1, "exactly one, at the EXIT");
    assert_eq!(
        count(&rig, rig.pilot(1)),
        0,
        "and nothing at all to the pilot who is still in her tank"
    );

    // **And it stays cleared.** The arena drops her from its roster at the start
    // of its next pulse, so a batch staged before the `EXIT` is drained after
    // it; a live frame pushed then would be the LAST thing on her feed and —
    // the retain marker being what it is — the only thing left in her session's
    // ring to reconnect into.
    let last_to = |rig: &Rig, who: ObjId| {
        rig.panels(tacsim::arena_bridge::FRAME_FEED)
            .into_iter()
            .filter(|(audience, _)| *audience == Audience::Actor(who))
            .map(|(_, data)| data)
            .next_back()
    };
    for _ in 0..40 {
        rig.pump();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        last_to(&rig, rig.pilot(0)),
        Some(cleared.clone()),
        "the last cockpit frame the leaver was sent is the cleared one"
    );
    assert_eq!(
        count(&rig, rig.pilot(0)),
        1,
        "and still exactly one of them"
    );

    // The match ends: the pilot still aboard is cleared too, and the leaver is
    // not cleared a second time.
    rig.staff("@arena/abort");
    for _ in 0..400 {
        rig.pump();
        if count(&rig, rig.pilot(1)) > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        count(&rig, rig.pilot(1)),
        1,
        "once, at the end of the match"
    );
    assert_eq!(
        count(&rig, rig.pilot(0)),
        1,
        "and still only once for the leaver"
    );
}

/// **What `VIEW` actually sends, in order** (16b cut Q.3; doc 16 §3.6).
///
/// The sentence itself is a pure function and is pinned as one below; this is
/// the verb around it, which the pure test cannot see: that the panel goes
/// **first** and the line after it (a pilot is not told about a map before being
/// shown it), that the line rides the arena's own notice tag, and that it is the
/// only thing this cut added to what a line client is sent.
///
/// The two tanks open 240 cells apart, so the scene is built: they are stood
/// fourteen cells apart north-south, which is inside the stock sensor's sixty
/// and outside the twenty rows a terminal has.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn view_follows_its_panel_with_the_line_about_what_the_window_could_not_fit() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    // Up the middle of the proving ground, on the road the two entry zones face
    // each other along, and fourteen cells apart: past the ten rows the window
    // has north of the platform, well inside the sixty cells a stock sensor
    // reaches.
    let middle = i64::from(match_::ARENA_CELLS / 2);
    rig.place(rig.tank(0), (middle, middle), 0);
    rig.place(rig.tank(1), (middle, middle + 14), 180_000);
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    rig.say(0, "startup");
    rig.say(1, "startup");
    assert!(rig.powered(0).await, "the tank wakes up");

    // Pump until the platform holds the other tank.
    let mut held = 0;
    for _ in 0..600 {
        rig.pump();
        held = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(rig.pilot(0)))
            .map_or(0, |frame| frame.frame.contacts.len());
        if held > 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(held, 1, "the platform holds the tank fourteen cells north");

    let events = rig.events(0, "view");
    let panel = events.iter().position(|(_, event)| {
        matches!(event, OutputEvent::Panel { feed, data, .. }
            if feed.as_str() == tacsim::arena_bridge::FRAME_FEED
                && data.get("schema").and_then(|schema| schema.get("shape"))
                    == Some(&serde_json::json!("grid")))
    });
    let beyond = "1 contact plots beyond this window; CONTACTS lists it.";
    let line = events.iter().position(|(_, event)| {
        matches!(event, OutputEvent::Sys(text)
            if text.to_plain_text() == beyond
                && text
                    .spans()
                    .iter()
                    .any(|span| span.tag.as_deref() == Some("arena")))
    });
    let (Some(panel), Some(line)) = (panel, line) else {
        panic!("VIEW owes a grid panel and the one line: {events:?}");
    };
    assert!(
        panel < line,
        "the panel goes first and the line about it after: {events:?}"
    );
    assert_eq!(
        rig.tw
            .seen_by(&events, rig.pilot(0))
            .iter()
            .filter(|said| said.as_str() == beyond)
            .count(),
        1,
        "said once"
    );
}

// ---------------------------------------------------------------------------
// (11) The canonical fixtures (16b cut Q.3; doc 16 §5)
// ---------------------------------------------------------------------------
//
// Doc 16 §5 asks for renderer parity, and three renderers each pinned to a
// golden of its own is regression and not parity. So there is ONE scene, built
// below by the three pure builders, written to `web/src/lib/fixtures/cockpit/`,
// and read by everybody: Rust `include_str!`s it (as `mush-net-ssh`'s TUI reads
// `run-allowlist.json`), vitest reads it natively, and there are no copies to
// drift.
//
// The scene is deliberately awkward. The pilot stands near the SOUTH-WEST corner
// so the window clamps on two edges at once; one contact stands on a building
// cell inside the window; one plots outside it; one carries a reading and one
// does not. `view_grid.txt` — pinned in `mush-ansi/tests/golden.rs` — is what a
// line client draws of it, and it is what 16b cut Q.4's and Q.5's compositions of
// `arena_map.json` + `cockpit_frame.json` at exactly 78 by 20 must equal byte for
// byte. That equality, and not three separate goldens, is what makes the three
// renderers one renderer.
//
// Re-bless with `TACSIM_BLESS=1 cargo test -p tacsim --test cockpit`.

mod fixture {
    use std::path::PathBuf;

    use mush_atlas::TerrainClass;
    use mush_core::ObjId;
    use mush_sim::data::Pose;
    use smol_str::SmolStr;
    use tacsim::arena::clock::Tempo;
    use tacsim::arena::condition::{Condition, ConditionBand};
    use tacsim::arena::perception::{FrameCondition, FrameContact, FrameNotice, Named, NoticeKind};
    use tacsim::arena::state::{
        ArenaTerrain, Classification, ContactId, Designator, Power, Team, TerrainCell,
    };
    use tacsim::arena::step::{Frame, FrameFacet, FrameHeat, FrameSection};
    use tacsim::arena::weapons::{FrameBand, FrameWeapon};
    use tacsim::arena_task::MatchId;
    use tacsim::match_::cockpit;

    /// The env var that rewrites the fixtures instead of comparing against them.
    pub const BLESS: &str = "TACSIM_BLESS";

    /// The one match id the fixtures are stamped with.
    pub const MATCH: MatchId = MatchId(7);

    /// The synthetic ground's id, as the payloads' `surface` carries it.
    pub const SURFACE: &str = "fixture_yard";

    /// The ground is 96 by 64 cells of ten metres.
    pub const WIDTH: u16 = 96;
    /// As [`WIDTH`], up the ground.
    pub const HEIGHT: u16 = 64;

    /// The pilot's own unit.
    const UNIT: ObjId = ObjId(101);

    /// The pilot's cell, near the south-west corner: 78 by 20 around it clamps
    /// to the arena's own west edge AND its own south edge.
    const YOU: (u16, u16) = (6, 4);

    /// The four contacts' row keys. Opaque and never printed; distinct and
    /// ascending, because a frame's contact list is in contact-id order.
    const HOSTILE: ContactId = ContactId(0x1111_1111_1111_1111);
    const FRIEND: ContactId = ContactId(0x2222_2222_2222_2222);
    const ON_A_ROOF: ContactId = ContactId(0x3333_3333_3333_3333);
    const OFF_WINDOW: ContactId = ContactId(0x4444_4444_4444_4444);

    /// The fixture ground: five terrain classes, a building block and a pond.
    ///
    /// NOT the shipped proving ground. A fixture built on shipped content would
    /// move whenever the content did, and what these files pin is the *shape* of
    /// a payload, not the ground the game happens to ship.
    #[must_use]
    pub fn terrain() -> ArenaTerrain {
        let cell = |class: TerrainClass, move_cost: u16| TerrainCell {
            class,
            elev_cm: 0,
            move_cost,
        };
        let mut cells =
            vec![cell(TerrainClass::Grass, 120); usize::from(WIDTH) * usize::from(HEIGHT)];
        let mut put = |x: u16, y: u16, what: TerrainCell| {
            let index = usize::from(y) * usize::from(WIDTH) + usize::from(x);
            if let Some(slot) = cells.get_mut(index) {
                *slot = what;
            }
        };
        // An east-west road across the whole ground, ten cells up.
        for x in 0..WIDTH {
            put(x, 10, cell(TerrainClass::Road, 60));
        }
        // A pond north-east of the muster corner, inside the window.
        for y in 2..6 {
            for x in 20..30 {
                put(x, y, cell(TerrainClass::Water, 0));
            }
        }
        // A building, inside the window, with a contact standing on it.
        for y in 6..9 {
            for x in 12..16 {
                put(x, y, cell(TerrainClass::Wall, 0));
            }
        }
        // Woods well north of the window: on the map, off the view — which is
        // what makes the two legends differ.
        for y in 20..30 {
            for x in 40..60 {
                put(x, y, cell(TerrainClass::WoodsLight, 200));
            }
        }
        ArenaTerrain::new(WIDTH, HEIGHT, 1_000, cells).expect("the fixture grid is 96 by 64")
    }

    /// The opaque posture blob every `Pose` carries, empty.
    fn posture() -> mush_sim::RulesValue {
        let reference = mush_core::DefinitionRef::new(
            mush_core::DefinitionKey::new(
                mush_core::OwnerId::new("tacsim").expect("a canonical owner"),
                mush_core::DefinitionKind::new("schema").expect("a canonical kind"),
                mush_core::DefinitionId::new("tacsim.arena.fixture").expect("a canonical id"),
            ),
            1,
            mush_core::DefinitionDigest::from_bytes([0_u8; 32]),
        )
        .expect("version 1 is not zero");
        mush_sim::RulesValue::new(reference, serde_json::json!({})).expect("an empty posture blob")
    }

    /// `ggyrd`: two whole sections, one hurt, one nearly gone, one destroyed.
    fn reading() -> Condition {
        Condition::default()
            .with_band(0, ConditionBand::Green)
            .with_band(1, ConditionBand::Green)
            .with_band(2, ConditionBand::Yellow)
            .with_band(3, ConditionBand::Red)
            .with_band(4, ConditionBand::Destroyed)
    }

    /// **The one scene.**
    ///
    /// Red slot 1, `RA`, at the south-west corner, heading north-east, moving,
    /// warm, one track hurt and one destroyed, a cannon loaded and a machine gun
    /// cycling, locked on `BA`, four contacts and two notices.
    ///
    /// **16b cut D.1**: the platform carries the shipped 600 m sight, so two of
    /// its four contacts read `far` — the band is thirds of the lesser of the
    /// sight and the fired weapon's reach.
    #[must_use]
    pub fn frame() -> Frame {
        Frame {
            unit: UNIT,
            callsign: Designator::Slot {
                team: Team::Red,
                slot: 1,
            },
            cell: Some(YOU),
            team: Team::Red,
            pose: Pose {
                x_cm: i64::from(YOU.0) * 1_000 + 500,
                y_cm: i64::from(YOU.1) * 1_000 + 500,
                z_cm: 0,
                heading_mdeg: 45_000,
                posture: posture(),
            },
            speed_cms: 1_200,
            power: Power::Online,
            blocked: false,
            // 16b cut D.2: the pilot's own two limits.
            wreck: false,
            immobile: false,
            tempo: Tempo::Engaged,
            // **16b cut D.1**: the SHIPPED sight — 60 cells at the fixture's own
            // 1,000 cm cell, 600 m — because the README's rule is that the scene
            // never carries a value the server does not send. A longer sight
            // would have kept the goldens still and would be a sensor the game
            // cannot field.
            sight_cm: 60_000,
            contacts: vec![
                // Classified hostile, read two seconds ago, and the lock.
                FrameContact {
                    id: HOSTILE,
                    designator: Designator::Slot {
                        team: Team::Blue,
                        slot: 1,
                    },
                    alias: Some(1),
                    class: Classification::Classified { team: Team::Blue },
                    bearing_mdeg: 45_000,
                    range_cm: 43_000,
                    age_ms: 1_200,
                    cell: Some((36, 14)),
                    condition: Some(FrameCondition {
                        table: 0,
                        bands: reading(),
                        age_ms: 2_000,
                    }),
                },
                // Classified friendly off the mesh: no reading at all, which is
                // what keeps its chassis out of `scan_tables`.
                FrameContact {
                    id: FRIEND,
                    designator: Designator::Slot {
                        team: Team::Red,
                        slot: 2,
                    },
                    alias: None,
                    class: Classification::Classified { team: Team::Red },
                    bearing_mdeg: 270_000,
                    range_cm: 12_000,
                    age_ms: 400,
                    cell: Some((2, 9)),
                    condition: None,
                },
                // Unknown, standing ON the building block inside the window: its
                // marker overwrites the wall glyph.
                FrameContact {
                    id: ON_A_ROOF,
                    designator: Designator::Provisional(3),
                    alias: None,
                    class: Classification::Unknown,
                    bearing_mdeg: 315_000,
                    range_cm: 22_000,
                    age_ms: 3_500,
                    cell: Some((14, 7)),
                    condition: None,
                },
                // Unknown, plotted north of the window: on `CONTACTS`, not on
                // the map, and the reason `VIEW` says its one line.
                FrameContact {
                    id: OFF_WINDOW,
                    designator: Designator::Provisional(4),
                    alias: None,
                    class: Classification::Unknown,
                    bearing_mdeg: 30_000,
                    range_cm: 58_000,
                    age_ms: 6_000,
                    cell: Some((50, 33)),
                    condition: None,
                },
            ],
            scan_tables: vec![vec![
                SmolStr::new_static("engine"),
                SmolStr::new_static("hull"),
                SmolStr::new_static("left_track"),
                SmolStr::new_static("right_track"),
                SmolStr::new_static("turret"),
            ]],
            notices: vec![
                FrameNotice::Contact {
                    kind: NoticeKind::Acquired,
                    contact: Named {
                        id: OFF_WINDOW,
                        designator: Designator::Provisional(4),
                    },
                    class: Classification::Unknown,
                    was: None,
                },
                FrameNotice::TargetSectionLost {
                    contact: Named {
                        id: HOSTILE,
                        designator: Designator::Slot {
                            team: Team::Blue,
                            slot: 1,
                        },
                    },
                    section: SmolStr::new_static("turret"),
                },
            ],
            lock: Some(HOSTILE),
            weapons: vec![
                FrameWeapon {
                    id: SmolStr::new_static("cannon"),
                    ready: true,
                    ready_in_ms: 0,
                    rounds: 14,
                    rounds_max: 20,
                    range_band: Some(FrameBand {
                        index: 1,
                        of: 2,
                        max_range_cm: 100_000,
                        dispersion_mrad: 4,
                    }),
                    p_permille: Some(624),
                    bands_cm: vec![100_000, 300_000],
                    // 16b cut D.2: the stock tank's guns make no heat.
                    heat_per_shot: 0,
                },
                FrameWeapon {
                    id: SmolStr::new_static("machine_gun"),
                    ready: false,
                    ready_in_ms: 900,
                    rounds: 380,
                    rounds_max: 500,
                    range_band: None,
                    p_permille: Some(210),
                    bands_cm: vec![40_000],
                    heat_per_shot: 0,
                },
            ],
            sections: vec![
                FrameSection {
                    id: SmolStr::new_static("engine"),
                    condition: SmolStr::new_static("intact"),
                    internals: Some((40, 40)),
                    facets: Vec::new(),
                },
                FrameSection {
                    id: SmolStr::new_static("hull"),
                    condition: SmolStr::new_static("intact"),
                    internals: Some((60, 60)),
                    facets: vec![
                        FrameFacet {
                            id: SmolStr::new_static("front"),
                            armor: 55,
                            armor_max: 80,
                        },
                        FrameFacet {
                            id: SmolStr::new_static("rear"),
                            armor: 30,
                            armor_max: 30,
                        },
                    ],
                },
                // Damaged.
                FrameSection {
                    id: SmolStr::new_static("left_track"),
                    condition: SmolStr::new_static("intact"),
                    internals: Some((6, 24)),
                    facets: vec![FrameFacet {
                        id: SmolStr::new_static("left"),
                        armor: 4,
                        armor_max: 24,
                    }],
                },
                // Destroyed.
                FrameSection {
                    id: SmolStr::new_static("right_track"),
                    condition: SmolStr::new_static("destroyed"),
                    internals: Some((0, 24)),
                    facets: vec![FrameFacet {
                        id: SmolStr::new_static("right"),
                        armor: 0,
                        armor_max: 24,
                    }],
                },
                FrameSection {
                    id: SmolStr::new_static("turret"),
                    condition: SmolStr::new_static("intact"),
                    internals: Some((30, 30)),
                    facets: vec![FrameFacet {
                        id: SmolStr::new_static("top"),
                        armor: 18,
                        armor_max: 20,
                    }],
                },
            ],
            heat: Some(FrameHeat { heat: 40, max: 100 }),
        }
    }

    /// The three payloads of the one scene, through the three pure builders.
    #[must_use]
    pub fn payloads() -> (serde_json::Value, serde_json::Value, serde_json::Value) {
        let frame = frame();
        let terrain = terrain();
        let cockpit = cockpit::status_frame(MATCH, &frame);
        let map = cockpit::map_frame(MATCH, SURFACE, &terrain);
        let view = cockpit::view_window(cockpit.clone(), SURFACE, &terrain, &frame);
        (cockpit, map, view)
    }

    /// One payload, as a fixture file holds it: pretty JSON with a trailing
    /// newline, so a diff of a fixture is readable.
    #[must_use]
    pub fn text(value: &serde_json::Value) -> String {
        let mut out = serde_json::to_string_pretty(value).expect("a payload encodes");
        out.push('\n');
        out
    }

    /// **The grid shape's decode**, as `mush_ansi::panel::expand_rows` does it:
    /// `height` rows of `width` glyphs, run-length pairs expanded, short rows
    /// padded, then the markers overlaid in order with the last write winning.
    ///
    /// Re-stated here rather than imported, because the game crate depends on no
    /// renderer and must not start. It is the same eighteen lines, and
    /// `mush-ansi/tests/golden.rs` drives the REAL one over the very files this
    /// test writes — so a drift between the two is a failing golden.
    #[must_use]
    pub fn expand_rows(data: &serde_json::Value) -> Vec<Vec<char>> {
        let num = |key: &str| {
            data.get(key)
                .and_then(serde_json::Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .unwrap_or(0)
        };
        let glyph_of = |value: &serde_json::Value| {
            value
                .as_str()
                .and_then(|text| text.chars().next())
                .unwrap_or('?')
        };
        let (width, height) = (num("width"), num("height"));
        let empty = Vec::new();
        let raw = data
            .get("rows")
            .and_then(serde_json::Value::as_array)
            .unwrap_or(&empty);
        let mut rows: Vec<Vec<char>> = Vec::with_capacity(height);
        for y in 0..height {
            let mut row: Vec<char> = Vec::with_capacity(width);
            if let Some(runs) = raw.get(y).and_then(serde_json::Value::as_array) {
                for run in runs {
                    let Some(pair) = run.as_array() else { continue };
                    let glyph = pair.first().map_or(' ', &glyph_of);
                    let n = pair
                        .get(1)
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|n| usize::try_from(n).ok())
                        .unwrap_or(1);
                    for _ in 0..n {
                        if row.len() >= width {
                            break;
                        }
                        row.push(glyph);
                    }
                }
            }
            row.resize(width, ' ');
            rows.push(row);
        }
        if let Some(units) = data.get("units").and_then(serde_json::Value::as_array) {
            for unit in units {
                let Some(cell) = unit.get("cell").and_then(serde_json::Value::as_array) else {
                    continue;
                };
                let coord = |n: usize| {
                    cell.get(n)
                        .and_then(serde_json::Value::as_u64)
                        .and_then(|v| usize::try_from(v).ok())
                };
                let (Some(x), Some(y)) = (coord(0), coord(1)) else {
                    continue;
                };
                let glyph = unit.get("glyph").map_or('o', &glyph_of);
                if let Some(slot) = rows.get_mut(y).and_then(|row| row.get_mut(x)) {
                    *slot = glyph;
                }
            }
        }
        rows
    }

    /// The directory the four files live in, from this crate's manifest.
    #[must_use]
    pub fn dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/src/lib/fixtures/cockpit")
    }

    /// Compare `actual` with the committed `name`, or rewrite it under
    /// [`BLESS`] — the way the content digest pins are re-blessed.
    pub fn pinned(name: &str, committed: &str, actual: &str) {
        if std::env::var(BLESS).is_ok() {
            std::fs::create_dir_all(dir()).expect("the fixture directory");
            std::fs::write(dir().join(name), actual).expect("the fixture is writable");
            return;
        }
        assert_eq!(
            committed, actual,
            "web/src/lib/fixtures/cockpit/{name} is out of date; re-bless with \
             `{BLESS}=1 cargo test -p tacsim --test cockpit`"
        );
    }
}

/// The three payloads of the one scene, byte for byte.
#[test]
fn the_canonical_cockpit_fixtures_are_what_the_builders_produce() {
    let (cockpit, map, view) = fixture::payloads();
    fixture::pinned(
        "cockpit_frame.json",
        include_str!("../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"),
        &fixture::text(&cockpit),
    );
    fixture::pinned(
        "arena_map.json",
        include_str!("../../../web/src/lib/fixtures/cockpit/arena_map.json"),
        &fixture::text(&map),
    );
    fixture::pinned(
        "view_frame.json",
        include_str!("../../../web/src/lib/fixtures/cockpit/view_frame.json"),
        &fixture::text(&view),
    );
}

/// What the scene was built to exercise, asserted on the payloads rather than
/// left to a reader of the JSON: the clamp on two edges, a marker on a building,
/// a contact off the window, a reading and a chassis that is only published
/// because of it, and nothing about any unit on the map.
#[test]
fn the_fixture_scene_exercises_the_clamp_the_building_and_the_off_window_contact() {
    let (cockpit, map, view) = fixture::payloads();
    let pair = |value: &serde_json::Value, key: &str| -> (i64, i64) {
        let cell = value
            .get(key)
            .and_then(serde_json::Value::as_array)
            .unwrap_or_else(|| panic!("{key} is a pair: {value}"))
            .clone();
        (
            cell.first()
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(-1),
            cell.get(1)
                .and_then(serde_json::Value::as_i64)
                .unwrap_or(-1),
        )
    };
    // The window clamps on BOTH the west and the south edge, so the platform
    // draws at its own cell rather than in the middle.
    assert_eq!(pair(&view, "origin"), (0, 0));
    assert_eq!(pair(&view, "you"), (6, 4));
    assert_eq!(
        view.get("width").and_then(serde_json::Value::as_u64),
        Some(u64::from(cockpit::VIEW_COLUMNS))
    );
    assert_eq!(
        view.get("height").and_then(serde_json::Value::as_u64),
        Some(u64::from(cockpit::VIEW_ROWS))
    );
    // Three of the four contacts are drawn; the fourth is beyond the window.
    let units = view
        .get("units")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(units.len(), 4, "the platform and three markers: {view}");
    assert_eq!(
        units
            .first()
            .and_then(|unit| unit.get("you"))
            .and_then(serde_json::Value::as_bool),
        Some(true),
        "the pilot's own marker is FIRST: the marker order is the rule every \
         compositor follows"
    );
    let drawn: Vec<&str> = units
        .iter()
        .skip(1)
        .filter_map(|unit| unit.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(drawn, vec!["BA", "RB", "X3"], "in frame order: {view}");
    // `X3` stands on a wall cell, and its marker wins the cell.
    let on_a_roof = units
        .iter()
        .find(|unit| unit.get("id").and_then(serde_json::Value::as_str) == Some("X3"))
        .expect("X3 is drawn");
    assert_eq!(
        on_a_roof.get("cell"),
        Some(&serde_json::json!([14, 7])),
        "relative to the window's own corner, which is the arena's here"
    );
    let rows = fixture::expand_rows(&view);
    assert_eq!(
        rows.get(7).and_then(|row| row.get(14)).copied(),
        Some('?'),
        "an unknown contact's glyph overwrites the building it is standing on"
    );
    assert_eq!(
        rows.get(7).and_then(|row| row.get(13)).copied(),
        Some('#'),
        "and the building is still there beside it"
    );
    // The reading, and the chassis it is the price of.
    let hostile = cockpit
        .get("contacts")
        .and_then(serde_json::Value::as_array)
        .and_then(|rows| rows.first())
        .expect("the first contact");
    assert_eq!(
        hostile.get("condition").and_then(|c| c.get("bands")),
        Some(&serde_json::json!("ggyrd"))
    );
    assert_eq!(
        cockpit.get("scan_tables"),
        Some(&serde_json::json!([[
            "engine",
            "hull",
            "left_track",
            "right_track",
            "turret"
        ]])),
        "one table, for the one contact that carries a reading"
    );
    // The map is the ground and nothing else.
    assert!(map.get("units").is_none(), "the map names no unit: {map}");
    assert!(map.get("you").is_none(), "nor the asker: {map}");
    assert_eq!(map.get("origin_cell"), Some(&serde_json::json!([0, 0])));
    assert_eq!(
        map.get("width").and_then(serde_json::Value::as_u64),
        Some(u64::from(fixture::WIDTH))
    );
    assert_eq!(
        map.get("height").and_then(serde_json::Value::as_u64),
        Some(u64::from(fixture::HEIGHT))
    );
    // The map's legend carries the woods the window cannot see.
    let map_legend = map
        .get("legend")
        .map(ToString::to_string)
        .unwrap_or_default();
    let view_legend = view
        .get("legend")
        .map(ToString::to_string)
        .unwrap_or_default();
    assert!(map_legend.contains("woods_light"), "{map_legend}");
    assert!(!view_legend.contains("woods_light"), "{view_legend}");
}

/// Every keyframe either arena feed sends carries the retain marker, and the
/// grid-shaped one carries it too — or a typed `VIEW` would re-create the ring
/// flood one frame at a time.
#[test]
fn both_feeds_ask_the_session_ring_to_keep_only_the_latest() {
    let (cockpit, map, view) = fixture::payloads();
    for (what, payload, shape) in [
        ("cockpit_frame.json", &cockpit, None),
        ("arena_map.json", &map, Some("terrain")),
        ("view_frame.json", &view, Some("grid")),
    ] {
        let schema = payload.get("schema").expect("a schema");
        assert_eq!(
            schema.get("retain").and_then(serde_json::Value::as_str),
            // `mush_session::RETAIN_LATEST`, spelled out: the game crate does
            // not depend on the gateway's session manager, and the string IS the
            // contract between them.
            Some("latest"),
            "{what} must carry the retain marker: {schema}"
        );
        // The shape is what decides whether a line client prints anything at
        // all: `mush_ansi::panel` draws `grid` and nothing else, so a pulse
        // frame and a whole-arena map are both silent on a terminal.
        assert_eq!(
            schema.get("shape").and_then(serde_json::Value::as_str),
            shape,
            "{what}: {schema}"
        );
    }
}

/// The map of the SHIPPED proving ground fits in one message, and expanding it
/// gives back the authored terrain cell for cell.
///
/// The live ground and not [`match_::stock_definition`]'s uniform stand-in: the
/// bound is about what the content pack actually paints, and a flat window would
/// run-length-encode to nothing and prove nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_shipped_arena_s_map_is_small_enough_to_send_and_expands_to_the_ground() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let match_id = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(rig.pilot(0)))
        .map(|held| held.match_id)
        .expect("a frame names its match");
    let definition = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.arena_of(match_id))
        .cloned()
        .expect("the running match's ground");
    let terrain = &definition.terrain;
    let map = cockpit::map_frame(match_id, definition.id.as_str(), terrain);
    let bytes = serde_json::to_vec(&map).expect("the map encodes").len();
    assert!(
        bytes < 256 * 1024,
        "the shipped arena's map is {bytes} bytes, past the 256 KiB bound a \
         pulled keyframe is held to"
    );
    // The renderer's own decode, and it reads `width`/`height` off the payload —
    // so this is the round trip a client makes. (`mush-ansi/tests/golden.rs`
    // drives the real `mush_ansi::panel::expand_rows` over the committed
    // fixtures; this crate depends on no renderer.)
    let rows = fixture::expand_rows(&map);
    assert_eq!(rows.len(), usize::from(terrain.height()));
    for y in 0..terrain.height() {
        let row = rows.get(usize::from(y)).expect("a row per cell row");
        assert_eq!(row.len(), usize::from(terrain.width()));
        for x in 0..terrain.width() {
            let expected = terrain
                .get(x, y)
                .map_or(mush_atlas::TerrainClass::Void, |cell| cell.class)
                .glyph();
            assert_eq!(
                row.get(usize::from(x)).copied(),
                Some(expected),
                "cell ({x}, {y}) of the shipped ground"
            );
        }
    }
    // And nothing about any unit is in it, on the live ground as on the fixture.
    assert!(map.get("units").is_none(), "the shipped map names no unit");
}

/// `VIEW`'s one honest line: nothing when the window holds everything, the
/// singular when one contact is beyond it, the plural when several are.
#[test]
fn view_says_how_many_contacts_plot_beyond_its_window() {
    let terrain = fixture::terrain();
    let mut frame = fixture::frame();
    let window = |frame: &tacsim::arena::step::Frame| {
        cockpit::view_window(
            cockpit::status_frame(fixture::MATCH, frame),
            fixture::SURFACE,
            &terrain,
            frame,
        )
    };
    // The scene as it stands: one contact north of the window.
    assert_eq!(
        cockpit::beyond_window_line(&frame, &window(&frame)),
        Some("1 contact plots beyond this window; CONTACTS lists it.".to_owned())
    );
    // Push a second one off the window, and a third off the arena entirely (a
    // plot with no cell at all) — three beyond, one line.
    if let Some(contact) = frame.contacts.get_mut(0) {
        contact.cell = Some((36, 40));
    }
    if let Some(contact) = frame.contacts.get_mut(1) {
        contact.cell = None;
    }
    assert_eq!(
        cockpit::beyond_window_line(&frame, &window(&frame)),
        Some("3 contacts plot beyond this window; CONTACTS lists them.".to_owned())
    );
    // Bring them all inside: no line at all.
    for (n, contact) in frame.contacts.iter_mut().enumerate() {
        contact.cell = Some((u16::try_from(n).unwrap_or(0) + 20, 5));
    }
    assert_eq!(cockpit::beyond_window_line(&frame, &window(&frame)), None);
    // And a payload with no window in it owes nothing.
    assert_eq!(
        cockpit::beyond_window_line(&frame, &cockpit::status_frame(fixture::MATCH, &frame)),
        None
    );
}

/// The answers the two feeds give somebody who is not flying, and the cleared
/// cockpit a panel is sent when its pilot stops being one. One payload, twice.
#[test]
fn an_unseated_asker_and_a_cleared_cockpit_read_the_same() {
    assert_eq!(
        cockpit::cockpit_cleared(),
        serde_json::json!({
            "schema": { "id": "arena.cockpit", "version": 1, "retain": "latest" },
            "seated": false,
        })
    );
    assert_eq!(
        cockpit::map_unseated(),
        serde_json::json!({
            "schema": {
                "id": "arena.map",
                "version": 1,
                "shape": "terrain",
                "retain": "latest",
            },
            "seated": false,
        })
    );
}

// ---------------------------------------------------------------------------
// 16b cut D.1 — the panel tells the truth
// ---------------------------------------------------------------------------

/// **`B-25` and `B-19`: the pilot's band word, and the range floor** (16b cut
/// D.1). Pure: no world, no arena.
#[test]
fn d1_contact_band_is_thirds_of_the_lesser_of_the_sight_and_the_reach() {
    use tacsim::match_::cockpit::{CONTACT_RANGE_STEP_M, LOCK_RANGE_STEP_M, contact_band};

    // The sight is the lesser: thirds of it, at each edge.
    let bands = [150_000_u32, 300_000];
    assert_eq!(contact_band(0, &bands, 60_000), "close");
    assert_eq!(contact_band(20_000, &bands, 60_000), "close");
    assert_eq!(contact_band(20_001, &bands, 60_000), "effective");
    assert_eq!(contact_band(40_000, &bands, 60_000), "effective");
    assert_eq!(contact_band(40_001, &bands, 60_000), "far");
    // A plot a teammate passed you from past your own sight is `far` and not
    // `beyond`: the gun still reaches it.
    assert_eq!(contact_band(90_000, &bands, 60_000), "far");
    // Past the last band nothing the platform fires reaches it.
    assert_eq!(contact_band(300_000, &bands, 60_000), "far");
    assert_eq!(contact_band(300_001, &bands, 60_000), "beyond");
    // The WEAPON is the lesser: thirds of the last band.
    assert_eq!(contact_band(100_000, &bands, 900_000), "close");
    assert_eq!(contact_band(100_001, &bands, 900_000), "effective");
    assert_eq!(contact_band(200_001, &bands, 900_000), "far");
    // A platform with no sensor at all bands against the weapon alone.
    assert_eq!(contact_band(100_000, &bands, 0), "close");
    assert_eq!(contact_band(200_001, &bands, 0), "far");
    // A platform with nothing that fires has nothing in range.
    assert_eq!(contact_band(0, &[], 60_000), "beyond");
    assert_eq!(contact_band(u64::MAX, &[u32::MAX], 60_000), "beyond");

    // **The shipped numbers**: the stock cannon's bands and the stock sight.
    let shipped = [150_000_u32, 300_000];
    for (range_m, word) in [
        (200_u64, "close"),
        (201, "effective"),
        (400, "effective"),
        (401, "far"),
        (900, "far"),
    ] {
        assert_eq!(
            contact_band(range_m * 100, &shipped, 60_000),
            word,
            "{range_m} m on the shipped tank"
        );
    }

    // `B-19`: a contact is never `0 m` away.
    assert_eq!(cockpit::range_m_rounded(0, CONTACT_RANGE_STEP_M), 50);
    assert_eq!(cockpit::range_m_rounded(2_400, CONTACT_RANGE_STEP_M), 50);
    assert_eq!(cockpit::range_m_rounded(2_500, CONTACT_RANGE_STEP_M), 50);
    assert_eq!(cockpit::range_m_rounded(2_600, CONTACT_RANGE_STEP_M), 50);
    assert_eq!(cockpit::range_m_rounded(0, LOCK_RANGE_STEP_M), 10);
    assert_eq!(cockpit::range_m_rounded(400, LOCK_RANGE_STEP_M), 10);
}

/// **`T-59`: the scope word is the pilot's own, and the frame carries the
/// sight** (16b cut D.1), over the canonical scene's raw payload.
#[test]
fn d1_the_frame_carries_the_sight_and_the_pilots_own_scope() {
    use tacsim::arena::state::{Classification, Team};

    let (frame, _, _) = fixture::payloads();
    assert_eq!(
        frame.get("sight_m").and_then(serde_json::Value::as_u64),
        Some(600),
        "the platform's own nominal sight, metres: {frame}"
    );
    assert_eq!(
        frame.get("scope").and_then(serde_json::Value::as_str),
        Some("hostile"),
        "the scene holds a classified blue contact: {frame}"
    );
    assert_eq!(
        frame.get("tempo").and_then(serde_json::Value::as_str),
        Some("engaged"),
        "the arena's cadence stays on the wire for a panel's timers: {frame}"
    );

    // The three words, off three scopes.
    let mut scene = fixture::frame();
    assert_eq!(cockpit::own_scope(&scene), "hostile");
    // Nothing but classified friends.
    scene
        .contacts
        .retain(|held| held.class == Classification::Classified { team: Team::Red });
    assert!(
        !scene.contacts.is_empty(),
        "the friend is still in the list"
    );
    assert_eq!(cockpit::own_scope(&scene), "clear");
    // One unknown return.
    let mut scene = fixture::frame();
    scene
        .contacts
        .retain(|held| held.class == Classification::Unknown);
    assert!(!scene.contacts.is_empty());
    assert_eq!(cockpit::own_scope(&scene), "unidentified");
    // A wreck's pilot with an empty scope reads `clear` whatever the arena is
    // doing elsewhere, which is the whole of `T-59`.
    let mut scene = fixture::frame();
    scene.contacts.clear();
    for tempo in [
        tacsim::arena::clock::Tempo::Quiet,
        tacsim::arena::clock::Tempo::Alert,
        tacsim::arena::clock::Tempo::Engaged,
    ] {
        scene.tempo = tempo;
        assert_eq!(cockpit::own_scope(&scene), "clear", "{tempo:?}");
        let payload = cockpit::status_frame(tacsim::arena_task::MatchId(7), &scene);
        assert_eq!(
            payload.get("scope").and_then(serde_json::Value::as_str),
            Some("clear")
        );
    }
}

/// **`B-02`: an empty weapon list on a live frame is a fact, not a gap** (16b cut
/// D.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_an_empty_weapon_list_on_a_live_frame_is_a_fact() {
    let mut rig = Rig::boot(2);
    rig.fly().await;

    // With NO frame at all, the world record is still read: a pilot who has just
    // boarded is told the cannon exists.
    let said = rig.say(0, "weaponstatus");
    assert!(said.contains("cannon"), "{said}");
    assert!(!said.contains("carries no weapon"), "{said}");

    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let mut frame = rig.held_frame(0);
    assert!(!frame.weapons.is_empty(), "the stock tank starts armed");
    frame.weapons.clear();
    rig.push_frame(0, frame);

    let said = rig.say(0, "weaponstatus");
    assert!(
        said.contains("This unit carries no weapon."),
        "the frame's empty list fell back to the pre-match world record: {said}"
    );
    assert!(
        !said.contains("round(s)"),
        "a weapon the platform no longer carries was printed: {said}"
    );
    // And `FIRE` refuses in the cockpit, one pulse earlier than the arena would.
    let said = rig.say(0, "fire");
    assert!(
        said.contains("That unit carries no weapon called `cannon`."),
        "{said}"
    );
    assert!(!said.contains("Firing"), "{said}");
}

/// **`B-06`: `STARTUP` knows what it already did, and the hint has four ends**
/// (16b cut D.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_startup_reads_the_mirror_first_and_the_hint_ends_four_ways() {
    use tacsim::arena::intents::{IntentKind, Refusal};
    use tacsim::arena::perception::{FrameNotice, RefusalDetail};
    use tacsim::arena::state::Power;

    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    // A frame to compose the rest of this test's frames from. It says nothing
    // about the hint: what stamps one is the verb below.
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    assert!(!rig.hinted(0), "a frame on its own stamped a startup hint");

    // (0) Cold, and the arena has said nothing about the power yet: the intent
    // goes and the hint is stamped.
    let said = rig.say(0, "startup");
    assert!(
        said.contains("Startup sequence begun; power comes online shortly."),
        "{said}"
    );
    assert!(rig.hinted(0), "the hint was not stamped");

    // (1) And every motion verb now says WAIT rather than telling the pilot to
    // type the thing they have just typed.
    let said = rig.say(0, "heading 90");
    assert!(said.contains(cockpit::POWER_STARTING), "{said}");
    let said = rig.say(0, "startup");
    assert!(said.contains(cockpit::POWER_STARTING), "{said}");
    assert!(
        !said.contains("Startup sequence begun"),
        "a second STARTUP said the sequence had begun again: {said}"
    );

    // (2) A frame carrying `Off` and no refusal LEAVES it: it may be a batch
    // staged before the intent arrived.
    let mut frame = rig.held_frame(0);
    frame.power = Power::Off;
    frame.notices.clear();
    rig.push_frame(0, frame.clone());
    assert!(rig.hinted(0), "an ordinary cold frame cleared the hint");
    let said = rig.say(0, "heading 90");
    assert!(said.contains(cockpit::POWER_STARTING), "{said}");

    // (3) A refusal for ANOTHER order leaves it too.
    let refusal = |order: &str| FrameNotice::Refused {
        order: smol_str::SmolStr::new(order),
        refusal: Refusal::PowerNotOnline,
        weapon: None,
        detail: RefusalDetail::None,
    };
    let mut cold = frame.clone();
    cold.notices = vec![refusal("fire")];
    rig.push_frame(0, cold);
    assert!(rig.hinted(0), "a refusal of another order cleared the hint");

    // (4) **Refused**: a frame carrying the arena's refusal of THIS order clears
    // it, so the pilot is not told to hold on for six seconds after reading
    // `startup refused`.
    let mut cold = frame.clone();
    cold.notices = vec![refusal(IntentKind::Startup.as_str())];
    rig.push_frame(0, cold);
    assert!(!rig.hinted(0), "the refusing frame did not clear the hint");
    let said = rig.say(0, "heading 90");
    assert!(said.contains(cockpit::POWER_NOT_ONLINE), "{said}");
    assert!(
        !said.contains(cockpit::POWER_STARTING),
        "a refused STARTUP still told the pilot to hold on: {said}"
    );

    // (5) **Confirmed**: any frame whose power is not `Off` clears it.
    rig.say(0, "startup");
    assert!(rig.hinted(0));
    let mut warm = frame.clone();
    warm.power = Power::Starting;
    rig.push_frame(0, warm);
    assert!(!rig.hinted(0), "a `starting` frame did not clear the hint");
    let said = rig.say(0, "startup");
    assert!(said.contains(cockpit::POWER_STARTING), "{said}");

    let mut hot = frame.clone();
    hot.power = Power::Online;
    rig.push_frame(0, hot);
    let said = rig.say(0, "startup");
    assert!(said.contains("Power is already online."), "{said}");
    assert!(
        !said.contains("Startup sequence begun"),
        "STARTUP on a running tank promised a `Power online.` that will never come: {said}"
    );

    // (6) **Expired**: a hint stamped in the past is not a hint.
    rig.hint_until(0, -1);
    assert!(!rig.hinted(0), "an expired hint still stands");

    // (7) **Unbound**: the seat ends, and the optimism goes with it.
    rig.hint_until(0, i64::MAX);
    assert!(rig.hinted(0));
    if let Some(registry) = rig
        .tw
        .world_mut()
        .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
    {
        registry.unbind_pilot(pilot);
    }
    assert!(!rig.hinted(0), "unbinding the pilot left the hint behind");
}

/// **`B-06`, the hint's fourth end by its OTHER door: the match's own rows go**
/// (16b cut D.1).
///
/// [`ArenaRegistry::unbind_pilot`] is pinned in the test above; this is
/// [`ArenaRegistry::untrack`], which is what runs when a pilot's station has
/// already gone — a check-in that could not release the seat, and every pilot on
/// a void — and it is written on a pilot with a hint and **no remembered frame at
/// all**, which is the whole of what the sweep has to get right. `untrack` reads
/// the match's pilots off its own roster *before* emptying it; the obvious
/// refactor is to key the sweep on `last_frames`, which sits two lines away and
/// is empty here, and that would leave this hint behind for the next match's
/// pilot to read as a cold tank coming up.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_untracking_a_match_drops_a_hint_no_frame_ever_confirmed() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    // Nothing has been pumped, so the registry remembers no frame for anybody:
    // a hint is all there is to find this pilot by.
    assert!(
        rig.frame(0).is_none(),
        "a frame arrived before the test could stamp a hint without one"
    );
    rig.hint_until(0, i64::MAX);
    assert!(rig.hinted(0));

    let match_id = rig
        .centre()
        .match_id
        .expect("a running centre has a match id");
    if let Some(registry) = rig
        .tw
        .world_mut()
        .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
    {
        // The roster is what the sweep reads, and this is the row it has to find
        // the hint by — there is no frame to find it by.
        assert!(
            registry.pilots_of(match_id).contains(&pilot),
            "the pilot is not on the match's roster, so the sweep below would \
             pass without doing anything"
        );
        let _ = registry.untrack(match_id);
    }
    assert!(
        !rig.hinted(0),
        "untracking the match left behind a hint no frame had ever confirmed"
    );
}

/// **`T-59`: each of `STATUS`'s three scope sentences, through the verb** (16b
/// cut D.1).
///
/// [`cockpit::own_scope`]'s three bare words are pinned over the canonical scene
/// elsewhere; what this pins is the mapping `STATUS` puts them through, whose
/// third arm is a catch-all — so swapping the two named arms, or dropping one of
/// them into the catch-all, reads `scope clear` to a pilot holding a hostile and
/// nothing else in the suite would say so.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_status_prints_the_sentence_for_each_scope() {
    use tacsim::arena::state::Classification;

    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let base = rig.held_frame(0);
    let scene = fixture::frame();

    // (a) an empty scope.
    let mut frame = base.clone();
    frame.contacts.clear();
    rig.push_frame(0, frame);
    let said = rig.say(0, "status");
    assert!(said.contains("; scope clear"), "{said}");

    // (b) returns, none of them resolved.
    let mut frame = base.clone();
    frame.contacts = scene
        .contacts
        .iter()
        .filter(|held| held.class == Classification::Unknown)
        .cloned()
        .collect();
    assert!(!frame.contacts.is_empty(), "the scene holds an unknown");
    rig.push_frame(0, frame);
    let said = rig.say(0, "status");
    assert!(
        said.contains("; scope holds an unidentified contact"),
        "{said}"
    );
    assert!(!said.contains("scope clear"), "{said}");

    // (c) one of them is theirs.
    let mut frame = base.clone();
    frame.contacts = scene.contacts.clone();
    assert_eq!(
        cockpit::own_scope(&frame),
        "hostile",
        "the scene's contacts are read against this platform's own team"
    );
    rig.push_frame(0, frame);
    let said = rig.say(0, "status");
    assert!(said.contains("; scope holds a hostile"), "{said}");
    assert!(!said.contains("unidentified"), "{said}");
}

/// **`B-01`: the withheld fact is the other side's STANDING, and it really is
/// withheld** (16b cut D.1).
///
/// The reduced row is compared byte for byte across the one change that moves a
/// side's standing while a match runs — a pilot typing `EXIT` — because "no
/// `seated` in the output" would pass just as well on a board that never said
/// `seated` about anybody.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_the_other_sides_row_does_not_move_when_that_side_loses_a_pilot() {
    let mut rig = Rig::boot(2);
    rig.fly().await;

    let blue_row = |said: &str| {
        said.lines()
            .find(|line| line.trim_start().starts_with("blue:"))
            .unwrap_or_default()
            .to_owned()
    };
    let before = blue_row(&rig.say(0, "sim status"));
    assert!(before.contains("(BA): Pilot1"), "{before}");

    // The blue pilot climbs out, which mid-match is final.
    rig.say(1, "exit");
    assert!(
        rig.centre()
            .slots
            .iter()
            .any(|slot| slot.team == tacsim::arena::state::Team::Blue && slot.left),
        "the EXIT did not mark the blue slot as gone"
    );

    // Staff can see it. The red pilot cannot, and reads exactly what they read
    // before.
    let staff = tacsim::match_::status_lines(rig.tw.world(), BoardReader::Staff).join("\n");
    assert!(blue_row(&staff).contains(", left"), "{staff}");
    let after = blue_row(&rig.say(0, "sim status"));
    assert_eq!(
        before, after,
        "the red pilot's reading of the blue row moved when blue lost its pilot"
    );
}

// ---------------------------------------------------------------------------
// 16b cut D.2 — refusals reach the pilot
// ---------------------------------------------------------------------------

/// **`B-18`**: `(FRIENDLY)` follows the designator on the three lines that name
/// one, it is read off the frame's own row, and an unidentified return never
/// carries it. Nothing is refused.
#[test]
fn d2_a_friendly_contact_is_marked_on_the_lock_the_lock_line_and_the_hit() {
    use tacsim::arena::perception::{FrameNotice, Named};
    let frame = fixture::frame();
    let rows = cockpit::frame_contacts(&frame);
    let row = |friendly: bool| {
        rows.iter()
            .find(|row| row.friendly == friendly && row.class != "unknown")
            .cloned()
            .unwrap_or_else(|| panic!("a {friendly} classified row"))
    };
    let unknown = rows
        .iter()
        .find(|row| row.class == "unknown")
        .cloned()
        .unwrap_or_else(|| panic!("an unidentified row"));
    let friend = row(true);
    let hostile = row(false);
    let named = |row: &cockpit::ContactRow| Named {
        id: row.id,
        designator: row.designator,
    };

    // (1) The hit line.
    let hit = |row: &cockpit::ContactRow| {
        cockpit::notice_line(
            &frame,
            &FrameNotice::Hit {
                contact: Some(named(row)),
                section: Some(smol_str::SmolStr::new("turret")),
                spent: false,
            },
            &rows,
        )
    };
    assert_eq!(
        hit(&friend),
        format!("Hit on {} (FRIENDLY): turret.", friend.designator)
    );
    assert_eq!(
        hit(&hostile),
        format!("Hit on {}: turret.", hostile.designator)
    );
    assert_eq!(
        hit(&unknown),
        format!("Hit on {}: turret.", unknown.designator),
        "an unidentified return has no side, so it is never a friend"
    );

    // (2) The row's own `friendly`, which is the word the `LOCK` echo is built
    //     from. The echo itself is a separate path — `lock` reads `row.friendly`
    //     where these two read `friendly_mark` — so it is driven as a verb in
    //     `d2_the_lock_echo_marks_a_friendly_and_never_a_blip` rather than
    //     claimed here.
    assert!(friend.friendly && !hostile.friendly && !unknown.friendly);

    // (3) `lock_line`, so `WEAPONSTATUS` says it too.
    let mut locked = frame.clone();
    locked.lock = Some(friend.id);
    let line = cockpit::lock_line(&locked).unwrap_or_else(|| panic!("a lock line"));
    assert!(
        line.starts_with(&format!("Locked on {} (FRIENDLY)", friend.designator)),
        "{line}"
    );
    assert!(line.contains("% to hit"), "{line}");
    let mut locked = frame.clone();
    locked.lock = Some(hostile.id);
    let line = cockpit::lock_line(&locked).unwrap_or_else(|| panic!("a lock line"));
    assert!(!line.contains("FRIENDLY"), "{line}");

    // (4) And the notice object carries the same fact, so a panel needs no table
    // of its own.
    let mut marked = frame.clone();
    marked.notices = vec![FrameNotice::Locked {
        contact: named(&friend),
    }];
    let payload = cockpit::status_frame(fixture::MATCH, &marked);
    let notice = payload
        .get("notices")
        .and_then(serde_json::Value::as_array)
        .and_then(|list| list.first().cloned())
        .unwrap_or_else(|| panic!("one notice"));
    assert_eq!(
        notice.get("friendly").and_then(serde_json::Value::as_bool),
        Some(true)
    );
}

/// **`B-18`, the `LOCK` echo itself**: the third of the three friendly lines, and
/// the only one with an implementation of its own — `lock` builds its word from
/// the resolved row's `friendly`, where `notice_line` and `lock_line` both go
/// through `friendly_mark`. So it is driven as a verb, against a real world and
/// a frame pushed through the real bridge.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d2_the_lock_echo_marks_a_friendly_and_never_a_blip() {
    use tacsim::arena::perception::FrameContact;
    use tacsim::arena::state::{Classification, ContactId, Designator, Team};
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    // A classified return on the pilot's **own** side, one on the other side and
    // an unidentified blip, so one frame carries all three answers.
    let mut seen = rig.held_frame(0);
    seen.notices.clear();
    let other = if seen.team == Team::Red {
        Team::Blue
    } else {
        Team::Red
    };
    let row = |id: u64, designator: Designator, class: Classification| FrameContact {
        id: ContactId(id),
        designator,
        alias: None,
        class,
        bearing_mdeg: 45_000,
        range_cm: 12_000,
        age_ms: 0,
        cell: None,
        condition: None,
    };
    let friend = row(
        0xa1a1_a1a1_a1a1_a1a1,
        Designator::Slot {
            team: seen.team,
            slot: 2,
        },
        Classification::Classified { team: seen.team },
    );
    let hostile = row(
        0xb2b2_b2b2_b2b2_b2b2,
        Designator::Slot {
            team: other,
            slot: 1,
        },
        Classification::Classified { team: other },
    );
    let blip = row(
        0xc3c3_c3c3_c3c3_c3c3,
        Designator::Provisional(9),
        Classification::Unknown,
    );
    seen.contacts = vec![friend, hostile, blip];
    rig.push_frame(0, seen);

    let said = rig.say(0, &format!("lock {}", friend.designator));
    assert!(
        said.contains(&format!("Locking {} (FRIENDLY).", friend.designator)),
        "the echo did not mark a contact on the pilot's own side: {said}"
    );
    for row in [&hostile, &blip] {
        let said = rig.say(0, &format!("lock {}", row.designator));
        assert!(
            said.contains(&format!("Locking {}.", row.designator)),
            "{said}"
        );
        assert!(
            !said.contains("FRIENDLY"),
            "{} was marked a friend: {said}",
            row.designator
        );
    }
}

/// **`B-07`**: the notice object carries what the sentence says — seven keys
/// beside the five cut F.2 gave it, so a panel composes the line from fields and
/// never from `text`.
#[test]
fn d2_the_notice_object_carries_the_sentence_s_own_parts() {
    use tacsim::arena::intents::Refusal;
    use tacsim::arena::motion::BlockedCause;
    use tacsim::arena::perception::{FrameNotice, RefusalDetail};
    let frame = fixture::frame();
    let object = |notice: FrameNotice| -> serde_json::Value {
        let mut one = frame.clone();
        one.notices = vec![notice];
        cockpit::status_frame(fixture::MATCH, &one)
            .get("notices")
            .and_then(serde_json::Value::as_array)
            .and_then(|list| list.first().cloned())
            .unwrap_or_else(|| panic!("one notice"))
    };
    let key = |value: &serde_json::Value, name: &str| value.get(name).cloned().unwrap_or_default();

    // A refusal: the order, the weapon, the word and the numbers.
    let refused = object(FrameNotice::Refused {
        order: smol_str::SmolStr::new("fire"),
        refusal: Refusal::Overheated,
        weapon: Some(smol_str::SmolStr::new("laser")),
        detail: RefusalDetail::Overheated { heat: 60, max: 60 },
    });
    assert_eq!(key(&refused, "kind"), serde_json::json!("refused"));
    assert_eq!(key(&refused, "order"), serde_json::json!("fire"));
    assert_eq!(key(&refused, "weapon"), serde_json::json!("laser"));
    assert_eq!(key(&refused, "refusal"), serde_json::json!("overheated"));
    assert_eq!(
        key(&refused, "detail"),
        serde_json::json!({ "heat": 60, "max": 60 })
    );
    assert_eq!(key(&refused, "cause"), serde_json::Value::Null);
    assert_eq!(key(&refused, "friendly"), serde_json::json!(false));
    assert_eq!(key(&refused, "spent"), serde_json::json!(false));
    assert_eq!(
        key(&refused, "text"),
        serde_json::json!("FIRE laser refused. Overheated: 60 of 60. Let it cool.")
    );

    // A cycling refusal's own number.
    let cycling = object(FrameNotice::Refused {
        order: smol_str::SmolStr::new("fire"),
        refusal: Refusal::Cycling,
        weapon: Some(smol_str::SmolStr::new("cannon")),
        detail: RefusalDetail::Cycling { secs: 3 },
    });
    assert_eq!(key(&cycling, "detail"), serde_json::json!({ "secs": 3 }));

    // A block: the cause, and no refusal.
    let held = object(FrameNotice::Blocked {
        cause: BlockedCause::TooSteep,
    });
    assert_eq!(key(&held, "cause"), serde_json::json!("too_steep"));
    assert_eq!(key(&held, "refusal"), serde_json::Value::Null);
    assert_eq!(key(&held, "order"), serde_json::Value::Null);
    assert_eq!(key(&held, "detail"), serde_json::Value::Null);

    // The victim's own two, which name nothing at all.
    for (notice, word, text) in [
        (
            FrameNotice::OwnDestroyed,
            "destroyed",
            cockpit::OWN_DESTROYED,
        ),
        (FrameNotice::OwnDisabled, "disabled", cockpit::OWN_DISABLED),
    ] {
        let own = object(notice);
        assert_eq!(key(&own, "kind"), serde_json::json!(word));
        assert_eq!(key(&own, "text"), serde_json::json!(text));
        for empty in [
            "contact",
            "designator",
            "section",
            "cause",
            "refusal",
            "order",
            "weapon",
        ] {
            assert_eq!(key(&own, empty), serde_json::Value::Null, "{word}.{empty}");
        }
    }

    // The spent pair.
    let struck = object(FrameNotice::StruckSpent {
        section: smol_str::SmolStr::new("left_track"),
    });
    assert_eq!(key(&struck, "kind"), serde_json::json!("struck_spent"));
    assert_eq!(key(&struck, "spent"), serde_json::json!(true));
    assert_eq!(
        key(&struck, "section"),
        serde_json::Value::Null,
        "the victim's own section is not the gunner's key"
    );
    assert_eq!(
        key(&struck, "text"),
        serde_json::json!("Hit on the left_track, which is already destroyed.")
    );

    // None of the seven new keys can carry a sixteen-digit contact id or an
    // object number, on any notice of the one scene.
    let payload = cockpit::status_frame(fixture::MATCH, &fixture::frame());
    let notices = payload
        .get("notices")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(!notices.is_empty());
    for notice in notices.iter().chain([&refused, &cycling, &held, &struck]) {
        for name in [
            "cause", "refusal", "order", "weapon", "detail", "friendly", "spent",
        ] {
            let word = serde_json::to_string(&key(notice, name)).unwrap_or_default();
            for part in word.split(|c: char| !c.is_ascii_hexdigit()) {
                assert!(
                    part.len() != 16 || u64::from_str_radix(part, 16).is_err(),
                    "{name} carried a sixteen-digit contact id: {word}"
                );
            }
            assert!(
                !word.contains("101"),
                "{name} carried the platform's object number: {word}"
            );
        }
    }

    // And the frame's own two new top-level keys, and the weapon row's.
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    assert_eq!(payload.get("wreck"), Some(&serde_json::json!(false)));
    assert_eq!(payload.get("immobile"), Some(&serde_json::json!(false)));
    let weapons = payload
        .get("weapons")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(!weapons.is_empty());
    for weapon in &weapons {
        assert_eq!(
            weapon.get("heat_per_shot"),
            Some(&serde_json::json!(0)),
            "a stock tank's guns make no heat"
        );
    }
}

/// **`B-03`**: the cockpit answers a dead tank and an immobile one itself,
/// before the power check, and `STATUS` carries the mark afterwards.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d2_the_cockpit_refuses_a_dead_tank_and_an_immobile_one() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    // A tank whose motion is gone still sees, locks and shoots: only the two
    // motion verbs answer.
    let mut stuck = rig.held_frame(0);
    stuck.notices.clear();
    stuck.immobile = true;
    stuck.power = tacsim::arena::state::Power::Online;
    rig.push_frame(0, stuck);
    for verb in ["heading 90", "speed 10"] {
        let said = rig.say(0, verb);
        assert!(said.contains(cockpit::IMMOBILE), "{verb}: {said}");
    }
    let said = rig.say(0, "startup");
    assert!(!said.contains(cockpit::IMMOBILE), "{said}");

    // A wreck answers every order, before the power check — so a pilot whose
    // tank has just died is never told to wait for a startup.
    let mut dead = rig.held_frame(0);
    dead.notices.clear();
    dead.wreck = true;
    dead.immobile = true;
    dead.power = tacsim::arena::state::Power::Off;
    rig.push_frame(0, dead);
    for verb in ["startup", "heading 90", "speed 10", "lock BA", "fire"] {
        let said = rig.say(0, verb);
        assert!(said.contains(cockpit::DESTROYED), "{verb}: {said}");
        assert!(
            !said.contains(cockpit::POWER_NOT_ONLINE) && !said.contains(cockpit::POWER_STARTING),
            "{verb} was answered about the power: {said}"
        );
    }
    // And `STATUS` says so beside the speed.
    let said = rig.say(0, "status");
    assert!(said.contains(", DESTROYED"), "{said}");
}

/// **`B-16`, the cockpit's own third of the zero-heat rule**: a cooked platform
/// refuses the gun that makes heat and lets the gun that makes none through.
///
/// The engine's clause is pinned in `tests/extensions.rs` and the controller's in
/// `arena::reflex`'s own tests; this is the one of the three that lives in the
/// cockpit's pre-check, and it is asserted here against a real world, the real
/// verb and a frame pushed through the real bridge. The cold gun is proved
/// through the check rather than past it: with a lock but no arena behind it the
/// next thing it meets is `Cycling`/`Empty`/the power, never the heat sentence.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d2_a_cooked_platform_refuses_the_hot_gun_and_not_the_cold_one() {
    use tacsim::arena::state::ContactId;
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    // A lock, so `FIRE` reaches the heat check at all, and a rack of one gun that
    // makes heat and one that makes none, on a platform at its ceiling.
    let mut cooked = rig.held_frame(0);
    cooked.notices.clear();
    cooked.power = tacsim::arena::state::Power::Online;
    cooked.lock = Some(ContactId(0x1234_5678_9abc_def0));
    cooked.heat = Some(tacsim::arena::step::FrameHeat { heat: 60, max: 60 });
    for weapon in &mut cooked.weapons {
        weapon.ready = true;
        weapon.ready_in_ms = 0;
        weapon.rounds = weapon.rounds.max(1);
        weapon.heat_per_shot = if weapon.id == "cannon" { 20 } else { 0 };
    }
    assert_eq!(cooked.weapons.len(), 2, "the stock tank's two guns");
    rig.push_frame(0, cooked);

    let said = rig.say(0, "fire cannon");
    assert!(
        said.contains("FIRE cannon refused. Overheated: 60 of 60. Let it cool."),
        "the gun that makes heat is refused at the ceiling: {said}"
    );
    let said = rig.say(0, "fire mg");
    assert!(
        !said.contains("Overheated"),
        "a gun that makes no heat was held back by the platform's heat: {said}"
    );
}

/// **16b cut D.1's hint, end to end** (`B-03`): an accepted `STARTUP` stamps the
/// hint, the arena's own refusal of that order clears it, and the next `HEADING`
/// is told the truth rather than told to hold on.
///
/// The refusing frame is composed with the arena's **own** order word
/// ([`IntentKind::as_str`], which `Arena::stage_refusal` is given in both pulse
/// phases) and the arena's **own** sentence ([`cockpit::notice_line`]); that the
/// arena stages exactly this notice for a wrecked tank is pinned in
/// `arena::step`'s `d2_phase_two_names_the_order_it_refused`.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d2_an_arena_refused_startup_clears_the_hint_and_the_next_order_tells_the_truth() {
    use tacsim::arena::intents::{IntentKind, Refusal};
    use tacsim::arena::perception::{FrameNotice, RefusalDetail};
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    let said = rig.say(0, "startup");
    assert!(said.contains("Startup sequence begun"), "{said}");
    assert!(rig.hinted(0), "the accepted order stamped no hint");
    // While the hint stands the pilot is told to wait, which is true.
    let said = rig.say(0, "heading 90");
    assert!(said.contains(cockpit::POWER_STARTING), "{said}");

    // The arena refuses that very order: the tank died between the pilot's last
    // frame and the apply, so the local pre-check had nothing to read.
    let refusal = FrameNotice::Refused {
        order: smol_str::SmolStr::new(IntentKind::Startup.as_str()),
        refusal: Refusal::Wreck,
        weapon: None,
        detail: RefusalDetail::None,
    };
    let mut refused = rig.held_frame(0);
    refused.power = tacsim::arena::state::Power::Off;
    refused.notices = vec![refusal.clone()];
    assert_eq!(
        cockpit::notice_line(&refused, &refusal, &[]),
        "STARTUP refused. Your tank is a wreck.",
        "the sentence the pilot reads with it"
    );
    rig.push_frame(0, refused);
    assert!(!rig.hinted(0), "the refusing frame did not clear the hint");

    let said = rig.say(0, "heading 90");
    assert!(said.contains(cockpit::POWER_NOT_ONLINE), "{said}");
    assert!(
        !said.contains(cockpit::POWER_STARTING),
        "a refused STARTUP still told the pilot to hold on: {said}"
    );
}

/// **`B-17`**: `FIRE mg` names the coaxial, `WEAPONSTATUS` advertises the short
/// name, and a word two weapons answer to is refused saying which.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn d2_fire_takes_the_short_name_and_weaponstatus_advertises_it() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let said = rig.say(0, "weaponstatus");
    assert!(said.contains("machine_gun (mg)"), "{said}");
    assert!(
        !said.contains("cannon ("),
        "a one-word id has no short name: {said}"
    );
    // With no lock, `FIRE mg` gets as far as the lock check — which is the proof
    // that the word resolved to a weapon at all, since an unknown one is refused
    // before it.
    let said = rig.say(0, "fire mg");
    assert!(
        said.contains("FIRE machine_gun refused."),
        "the short name did not resolve: {said}"
    );
    assert!(said.contains(cockpit::NO_LOCK), "{said}");
    let said = rig.say(0, "fire can");
    assert!(said.contains("FIRE cannon refused."), "{said}");
    let said = rig.say(0, "fire railgun");
    assert!(
        said.contains("no weapon called `railgun`") && !said.contains("refused."),
        "a weapon that is not there has none to name: {said}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut D.3 (`T-60`) — four slips from the hand-played transcript
// ---------------------------------------------------------------------------

/// **`SPEED`'s usage names both ceilings, and enforces both** (16b cut D.3,
/// `T-60`).
///
/// The transcript's own complaint: `That unit makes 54 kph; SPEED 0 to 54.` over
/// a verb that accepted `SPEED -10` — a range the refusal named and did not
/// enforce at either end. It compared the **magnitude**, so `SPEED -54` went
/// through on a tank whose reverse ceiling is half its forward one and the arena
/// silently clamped it at apply. The stock tank makes 1,500 cm/s ahead and
/// `reverse_factor_pct` of 50, so the two numbers are 54 and 27.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_speed_names_and_enforces_the_reverse_ceiling() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the tank never came online");

    const USAGE: &str = "That unit makes 54 kph ahead and 27 astern; SPEED -27 to 54.";
    // Past the forward ceiling.
    let said = rig.say(0, "speed 999");
    assert!(said.contains(USAGE), "{said}");
    // Past the REVERSE ceiling, which is the half the old sentence lied about.
    let said = rig.say(0, "speed -54");
    assert!(said.contains(USAGE), "{said}");
    let said = rig.say(0, "speed -28");
    assert!(said.contains(USAGE), "{said}");
    // And both ends of the range it names are accepted.
    let said = rig.say(0, "speed 54");
    assert!(said.contains("Making 54 kph."), "{said}");
    let said = rig.say(0, "speed -27");
    assert!(said.contains("Making -27 kph."), "{said}");
    let said = rig.say(0, "speed -10");
    assert!(said.contains("Making -10 kph."), "{said}");
    // A word that is not a number is still the other usage line.
    let said = rig.say(0, "speed abc");
    assert!(
        said.contains("SPEED <kph>; a negative speed reverses."),
        "{said}"
    );
}

/// **`STATUS` brackets the fitted chassis when it is not the pod's own** (16b cut
/// D.3, `T-60`).
///
/// A pilot who typed `SIM UNIT biped_mk1` read `Red Tank 1: power online, …` all
/// match: the object's name is pack content and a pod's tank keeps the pod's own
/// name whatever is fitted to it. The name stays — renaming a persistent object
/// per match is a much worse answer — and the type goes in the bracket the board
/// already uses for exactly this.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_status_brackets_a_refitted_chassis_and_nothing_else() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim unit biped_mk1");
    assert!(said.contains("will field biped_mk1"), "{said}");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
    assert!(rig.frame_for(0).await, "the refitted pilot got no frame");
    assert!(rig.frame_for(1).await, "the stock pilot got no frame");

    let said = rig.say(0, "status");
    assert!(
        said.contains("[biped_mk1]: power "),
        "the refitted chassis is not on the panel: {said}"
    );
    // And the pilot who chose nothing reads byte for byte the line cut D.1
    // wrote: no bracket at all.
    let said = rig.say(1, "status");
    assert!(
        said.contains("Blue Tank 1: power "),
        "a stock tank grew a bracket: {said}"
    );

    // **A designator that is not this pod's brackets nothing.** `resolve_callsigns`
    // gives EVERY unit a `Designator::Slot` — a staff `@arena/start`'s two and the
    // harness's three hundred included — so the designator alone says nothing
    // about which pod a tank came out of. Here the refitted red tank is handed a
    // frame carrying **blue 1's** slot: without the second question the line would
    // bracket its chassis against the authored type of a pod it was never in.
    let mut stray = rig.held_frame(0);
    stray.callsign = tacsim::arena::state::Designator::Slot {
        team: Team::Blue,
        slot: 1,
    };
    rig.push_frame(0, stray);
    let said = rig.say(0, "status");
    assert!(
        said.contains("Red Tank 1: power "),
        "a unit was bracketed against a pod it was never in: {said}"
    );
    assert!(!said.contains("[biped_mk1]"), "{said}");
}

/// **`EXIT` says one line to the person leaving** (16b cut D.3, `T-60`).
///
/// `put_out` moves the body into the team room and the room line was then fanned
/// out to it, so the pilot read their own `You climb out of …` and the room's
/// `Pilot0 climbs out of …` one after the other. The room still hears it; the
/// actor does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_exit_says_one_line_to_the_pilot_and_one_to_the_room() {
    let mut rig = Rig::boot(3);
    rig.fly().await;
    // A bystander standing in the red team room, so the room half has an
    // audience that is not the actor.
    rig.walk_to(2, match_::RED_ROOM_ENTRY);

    let actor = rig.pilot(0);
    let watcher = rig.pilot(2);
    let events = rig.events_of(actor, "exit");
    let mine = rig.tw.seen_by(&events, actor).join("\n");
    let theirs = rig.tw.seen_by(&events, watcher).join("\n");

    assert!(mine.contains("You climb out of"), "{mine}");
    assert!(
        !mine.contains("climbs out of"),
        "the pilot read the room's line about themselves too: {mine}"
    );
    assert_eq!(
        mine.matches("climb out of").count(),
        1,
        "more than one line about leaving: {mine}"
    );
    assert!(
        theirs.contains("Pilot0 climbs out of"),
        "the room was not told: {theirs}"
    );
    assert!(
        !theirs.contains("You climb out of"),
        "the room read the pilot's own line: {theirs}"
    );
}
