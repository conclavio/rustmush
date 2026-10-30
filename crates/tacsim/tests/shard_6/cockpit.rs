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
//! 2. the nineteen verbs are contextual: each refuses with its own one line where
//!    it does not mean anything;
//! 3. `HEADING` before `STARTUP` is refused with the cold tank's own word;
//! 4. `VIEW` draws the platform at the centre of a grid;
//! 5. `STATUS`, `CONTACTS` and `WEAPONSTATUS` render the last frame, and the
//!    frame carries doc 16 §5's shape;
//! 6. `EXIT` gives the seat, the lease and the focus back and puts the pilot in
//!    the team room — and during `Boarding` it un-seats the slot;
//! 7. the kernel's new linkdead hook reaches the arena as a raised epoch.

use crate::support;

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{AccountId, AccountIdentity, Audience, GOD, ObjId};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use tacsim::TacsimConfig;
use tacsim::arena::state::{Channel, PartyId};
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

    let game = tacsim::GameEconomyConfig::default();
    // Reboots audit the foundry through the inventory owner, as production does.
    let (mut mods, inventory) = comp::domain_modules_with_economy(
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &game,
    );
    mods.push(comp::npc_module(comp::npc_agent_adapter()));
    // Match production: load the lane catalog when applying the game boot packs.
    mods.push(comp::tacsim_module_wired(cfg, true, inventory));
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
    /// **The latest arena clock a staged `ArenaOut::Frames` carried** (16b cut
    /// U.2), recorded by [`Rig::pump`] as it stages each message.
    ///
    /// Since 16c `T-30` a pulse that changed nothing a pilot can read sends that
    /// pilot nothing, so "the mirror's frame changed" is no longer a signal that
    /// a pulse ran; this is. The task publishes a `Frames` whenever a pulse
    /// carries a frame **or** the arena second has moved (16b cut D.1), so the
    /// clock reaches here at least once an arena second whatever the frames do.
    staged_now_ms: i64,
    /// **The match ledger as it left the arena** (16b cut Y.1): every line of
    /// every closed chunk [`Rig::pump`] has staged, in order — the `ordered`
    /// lines an `ORDER` journals are read here.
    tape: Vec<tacsim::arena::ledger::LedgerEvent>,
    /// **The arena time of every staff picture a staged `Frames` carried**
    /// (16b cut I.4), recorded by [`Rig::pump`] — what "afterwards the staged
    /// frames carry no picture" is read off.
    pictures: Vec<i64>,
    /// **16b cut AF.3**: the store as every write the rig has taken left it
    /// ([`Rig::pump_ops`]) — what [`Rig::reboot_mid_match`] boots from.
    image: StoreImage,
}

impl Rig {
    fn boot(pilots: usize) -> Rig {
        Rig::boot_with(pilots, config())
    }

    fn boot_with(pilots: usize, cfg: TacsimConfig) -> Rig {
        Rig::boot_on(pilots, roster_with(cfg))
    }

    fn boot_on(pilots: usize, mods: Vec<Box<dyn mush_core::GameModule>>) -> Rig {
        let mut tw = TestWorld::with_modules(mods);
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
            staged_now_ms: 0,
            tape: Vec::new(),
            pictures: Vec::new(),
            image: StoreImage::new(),
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
                        // 16b cut U.2: the arena's clock, as each pulse's message
                        // carries it — see `Rig::staged_now_ms`.
                        let clock = match &message {
                            tacsim::arena_task::ArenaOut::Frames { now_ms, .. } => Some(*now_ms),
                            _ => None,
                        };
                        // 16b cut I.4: the staff picture, as each message carries it.
                        if let tacsim::arena_task::ArenaOut::Frames {
                            observer: Some(picture),
                            ..
                        } = &message
                        {
                            self.pictures.push(picture.now_ms);
                        }
                        // 16b cut Y.1: the tape, as each closed chunk carries it.
                        if let tacsim::arena_task::ArenaOut::Ledger { chunk, .. } = &message {
                            self.tape.extend(
                                String::from_utf8_lossy(&chunk.bytes)
                                    .lines()
                                    .filter_map(|line| serde_json::from_str(line).ok()),
                            );
                        }
                        if !tasks.stage(message) {
                            break;
                        }
                        if let Some(now_ms) = clock {
                            self.staged_now_ms = self.staged_now_ms.max(now_ms);
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
        self.push_frame_at(n, frame, 0);
    }

    /// [`Rig::push_frame`], with the message's arena clock at `now_ms` (16b cut
    /// AP.1): what `on_frames` stamps the held frame's `arena_ms` with, and what
    /// it raises the mirror's `now_ms` to.
    fn push_frame_at(&mut self, n: usize, frame: tacsim::arena::step::Frame, now_ms: i64) {
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
                now_ms,
                // 16b cut I.4: a pilot's frame, and no staff picture.
                observer: None,
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

    /// **Move that pilot's held frame's receipt back by `ms`** (16b cut AP.1)
    /// through the registry's own test aid, so a test reaches a weapon's ready
    /// instant on the aged mirror without sleeping.
    fn age_last_frame(&mut self, n: usize, ms: u64) {
        let pilot = self.pilot(n);
        self.tw
            .world_mut()
            .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the arena registry is composed")
            .age_last_frame(pilot, std::time::Duration::from_millis(ms));
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
    /// connections to drop, and the SSH lens (`mushd/tests/sim_lenses/sim_cockpit.rs`) is
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

    /// **The same with a mutable `Ctx`** (16b cut Y.1), for a test that calls a
    /// world-side function that writes — `forward_intent`, `forward_order` —
    /// directly. What it emits is recorded into [`Rig::log`].
    fn with_ctx_mut<T>(&mut self, n: usize, f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T) -> T {
        let actor = self.pilot(n);
        let mut out = mush_core::Outbox::new();
        let answer = {
            let engine = self.tw.engine_mut();
            let mut ctx = mush_core::Ctx {
                w: &mut engine.world,
                actor,
                switches: mush_core::SwitchSet::default(),
                args: mush_core::ParsedArgs::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some(smol_str::SmolStr::new(tacsim::NS)),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            f(&mut ctx)
        };
        self.log.extend(out.take());
        answer
    }

    /// **`reds` pilots on red and `blues` on blue, each side topped up to
    /// `fielded` by the AI fill, started and boarded** (16b cut Y.1). Pilot `0`
    /// claims first and leads red; the side's slots follow claim order, so red
    /// pilot `n` sits in slot `n + 1`.
    async fn fly_with_ai(&mut self, reds: usize, blues: usize, fielded: u8) {
        for n in 0..reds {
            self.say(n, "sim team red");
        }
        for n in reds..reds + blues {
            self.say(n, "sim team blue");
        }
        let said = self.say(0, &format!("sim ai {fielded}"));
        assert!(!said.is_empty(), "SIM AI answers");
        self.say(0, "sim start");
        for n in 0..reds {
            self.walk_to(n, match_::RED_ROOM_ENTRY);
            self.say(n, "board");
        }
        for n in reds..reds + blues {
            self.walk_to(n, match_::BLUE_ROOM_ENTRY);
            self.say(n, "board");
        }
        assert_eq!(self.state(), MatchState::Running);
    }

    /// The running match's record: its squads and every unit's callsign (16b
    /// cut Y.1).
    fn record(&self) -> tacsim::match_::MatchRecord {
        let match_id = self.centre().match_id.expect("a running match");
        self.tw
            .world()
            .module_state::<tacsim::match_::MatchBook>()
            .and_then(|book| book.get(match_id))
            .cloned()
            .expect("the match is on the book")
    }

    /// Pump until the tape holds a line `pick` answers, or give up (16b cut
    /// Y.1). A chunk closes at 16 KiB, which the fast summaries this rig runs
    /// fill in a few seconds.
    async fn tape_until(
        &mut self,
        pick: impl Fn(&tacsim::arena::ledger::LedgerEvent) -> bool,
    ) -> bool {
        for _ in 0..3_000 {
            self.pump();
            if self.tape.iter().any(&pick) {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        false
    }

    /// Pump until the tape holds at least `want` lines `pick` answers, and
    /// answer how many it holds then (16b cut Y.1). An order lands on every unit
    /// of a squad in one pulse, but a chunk may close between two of its lines,
    /// so a count is waited for rather than read off the first line's chunk.
    async fn tape_count_until(
        &mut self,
        pick: impl Fn(&tacsim::arena::ledger::LedgerEvent) -> bool,
        want: usize,
    ) -> usize {
        for _ in 0..3_000 {
            self.pump();
            if self.tape.iter().filter(|event| pick(event)).count() >= want {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        self.tape.iter().filter(|event| pick(event)).count()
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
// (2) The gate: nineteen verbs (the seventeen selectors, BOARD and EXIT; eighteen and
// sixteen until 17b cut M5.4's MIND), one line each
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
        &ground.key_spots,
        &held.frame,
    );

    // The pure window says nothing about what the tank is called, and the live
    // one says the name it has always said — never the callsign. **16b cut
    // V.1**: the own marker is the one flagged `you`, not the first — the red
    // muster's window (x 0..38) holds none of the shipped spots, so here it
    // happens to be first as well; the moved pose below is where it is not.
    let you = own_marker(&pure).expect("the platform's own marker is in the window");
    assert_eq!(
        you.get("you").and_then(serde_json::Value::as_bool),
        Some(true)
    );
    assert!(
        you.get("name").is_none(),
        "a pure window invents no name: {you}"
    );
    let live_you = own_marker(&live).expect("the live payload's own marker is there too");
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
    name_own_marker(&mut pure, &name);
    assert_eq!(live, pure, "the live window is the pure one, plus the name");

    // **16b cut V.1**: the same frame stood four cells west of The Ford (64, 97),
    // so the window's first marker is the spot and the own marker follows it.
    // The live payload names the tank on the `you` marker and on nothing else —
    // taking `units.first()` would have put the tank's name on the ford and left
    // the pilot's own marker bare.
    let mut near = held.frame.clone();
    near.pose.x_cm = 60 * 1_000 + 500;
    near.pose.y_cm = 97 * 1_000 + 500;
    let live_near = rig.with_ctx(0, |ctx| cockpit::view_frame(ctx, held.match_id, &near));
    let mut pure_near = cockpit::view_window(
        cockpit::status_frame(held.match_id, &near),
        ground.id.as_str(),
        &ground.terrain,
        &ground.key_spots,
        &near,
    );
    let units = live_near
        .get("units")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let first = units.first().expect("the window draws the ford");
    assert_eq!(
        (
            first.get("id").and_then(serde_json::Value::as_str),
            first.get("name").and_then(serde_json::Value::as_str),
            first.get("spot").and_then(serde_json::Value::as_bool),
        ),
        (Some("A"), Some("The Ford"), Some(true)),
        "the ford is drawn first, under its own name: {live_near}"
    );
    let you_at = units
        .iter()
        .position(|unit| unit.get("you").and_then(serde_json::Value::as_bool) == Some(true))
        .expect("the own marker");
    assert_eq!(you_at, 1, "the own marker follows the one spot drawn");
    assert_eq!(
        own_marker(&live_near)
            .as_ref()
            .and_then(|you| you.get("name"))
            .and_then(serde_json::Value::as_str),
        Some(name.as_str()),
        "the tank's name is on the pilot's own marker: {live_near}"
    );
    name_own_marker(&mut pure_near, &name);
    assert_eq!(
        live_near, pure_near,
        "with a spot in the window, the live window is still the pure one plus the name"
    );

    // And the schema is the one cut K.2 wrote, plus the marker the session ring
    // reads — the whole of what this cut changed about what `VIEW` sends. 17b
    // cut M1a.3 bumped the version to 2 (doc 17 §3), 17b cut M1b.3 to 3.
    assert_eq!(
        live.get("schema"),
        Some(&serde_json::json!({
            "id": "arena.cockpit",
            "version": 8,
            "shape": "grid",
            "retain": "latest",
        }))
    );
}

/// The window's own marker — the one flagged `you` (16b cut V.1: a spot may be
/// drawn before it, so it is found by its flag and never by its place).
fn own_marker(window: &serde_json::Value) -> Option<serde_json::Value> {
    window
        .get("units")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .find(|unit| unit.get("you").and_then(serde_json::Value::as_bool) == Some(true))
        .cloned()
}

/// Put the tank's display name on a pure window's own marker, which is the one
/// field the live window adds.
fn name_own_marker(window: &mut serde_json::Value, name: &str) {
    if let Some(marker) = window
        .get_mut("units")
        .and_then(serde_json::Value::as_array_mut)
        .and_then(|units| {
            units
                .iter_mut()
                .find(|unit| unit.get("you").and_then(serde_json::Value::as_bool) == Some(true))
        })
        .and_then(serde_json::Value::as_object_mut)
    {
        marker.insert("name".to_owned(), serde_json::json!(name));
    }
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
        // 16b cut Y.4: the catch-up report's key, `null` on every frame but a
        // reclaim's first.
        "catchup",
    ] {
        assert!(frame.get(key).is_some(), "the frame carries {key}: {frame}");
    }
    assert_eq!(
        frame.get("catchup"),
        Some(&serde_json::Value::Null),
        "a read verb's frame is the mirror's, which never carries a catch-up"
    );
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
    //
    //     **16b cut U.2** (16c `T-30`): the pulse is recognised by the arena's own
    //     clock on a staged `Frames`, not by the mirror's frame changing. The
    //     pair is parked in continuous view, so the plot is re-taken every pass
    //     and its age never leaves the first second; a frame that differs only
    //     in milliseconds is no longer sent, and the old wait (`!now.same_as(&
    //     before)`) had become a wait for the lock's range step to cross 648 ms.
    //     The clock moves on every pulse that carries a frame and at least once
    //     an arena second regardless (16b cut D.1).
    let spoken = rig.log.len();
    for word in ["BA", "ba", &number, "x1"] {
        let said = rig.say(0, &format!("lock {word}"));
        assert!(
            !said.contains(cockpit::NO_SUCH_CONTACT),
            "LOCK {word} was refused after the rename: {said}"
        );
        assert!(said.contains("Locking BA."), "LOCK {word}: {said}");
        let before = rig.staged_now_ms;
        let mut pulsed = false;
        for _ in 0..600 {
            rig.pump();
            if rig.staged_now_ms > before {
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
    // **16b cut B.1**: the two lines are told apart on the wire by their tag. The
    // answer to the typed line (`Locking BA.`) is a reply and keeps the arena's
    // category; `Locked on BA, …` is the `Sys` copy of a notice the frame after it
    // carries, and wears the notice tag — so a panel ring that takes the replies
    // leaves it out and composes it from the frame instead, once.
    let tags_of = |needle: &str| -> Vec<Option<String>> {
        rig.log
            .iter()
            .skip(spoken)
            .filter_map(|(_, event)| match event {
                OutputEvent::Sys(text) if text.to_plain_text().starts_with(needle) => Some(
                    text.spans()
                        .iter()
                        .map(|span| span.tag.as_deref().map(str::to_owned))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect()
    };
    let reply = tags_of("Locking BA.");
    let sentence = tags_of("Locked on BA,");
    assert!(!reply.is_empty() && !sentence.is_empty(), "{told:?}");
    assert!(
        reply
            .iter()
            .all(|tag| tag.as_deref() == Some(tacsim::arena_bridge::NOTICE_CATEGORY)),
        "a reply is tagged with the arena's category: {reply:?}"
    );
    assert!(
        sentence
            .iter()
            .all(|tag| tag.as_deref() == Some(tacsim::arena_bridge::NOTICE_SENTENCE_TAG)),
        "a notice's own sentence is tagged as one: {sentence:?}"
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
    // **16b cut Y.3**: nor is an `EXIT` a drop, so the reconnect does not
    // offer `BOARD` a tank the pilot left for good. (`flying on doctrine`
    // until 17b cut M5.3, which made the reconnect line one word for every
    // sealed mind.)
    assert!(
        !said.contains("flying on its mind"),
        "a reconnect after an EXIT offered the tank back: {said}"
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

/// **A pilot who QUITs is out of the cockpit too, and not `seated`** (16b cut
/// F.1; doc 16 §3.8's "Disconnects") — **and since 16b cut Y.3 the board says
/// the machine is flying it for them**.
///
/// `EXIT` is the route a reviewer thinks of and the rarest one in a playtest.
/// `SeatFacts::disconnected` — a clean `QUIT`, and the linkdead grace running out
/// — sends the **same** `Control { Released }` naming the same pilot: the arena
/// drops them from its frame roster and the registry drops the binding. So `SIM
/// STATUS` must not go on calling them `seated` either, and the flag is written
/// where the release is decided rather than in one verb. Cut Y.3 split the
/// word: a drop can be taken back (`BOARD`), so the row reads `flown by its
/// mind` rather than F.1's `left`, which is now an `EXIT`'s alone. (Was `flown
/// by doctrine`, and this test `…_reads_as_flown_by_doctrine`, until 17b cut
/// M5.3: the drop clause is one word for every sealed mind, C-11 and D11.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pilot_whose_session_ends_mid_match_reads_as_flown_by_its_mind() {
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
        red_row.contains(", flown by its mind"),
        "SIM STATUS does not say the machine flies a quit pilot's tank: {staff}"
    );
    assert!(
        !red_row.contains(", seated") && !red_row.contains(", left"),
        "SIM STATUS calls a pilot who quit mid-match seated, or gone for good: {staff}"
    );
    // 16b cut Y.3: `left` keeps its meaning — out of the cockpit, by any
    // route — and the drop's own stamp is what makes the slot reclaimable.
    assert!(
        rig.centre()
            .slot_of_pilot(pilot)
            .is_some_and(|slot| slot.left && slot.dropped_at_ms.is_some()),
        "a drop marks the slot left AND dropped"
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
    let red = match_::pod_object(rig.tw.world(), PartyId::RED, 1).expect("red pod 1");
    let blue = match_::pod_object(rig.tw.world(), PartyId::BLUE, 1).expect("blue pod 1");
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
// `arena_map.json` + `cockpit_frame.json` at exactly 39 by 20 must equal byte for
// byte. That equality, and not three separate goldens, is what makes the three
// renderers one renderer.
//
// Re-bless with `TACSIM_BLESS=1 cargo test -p tacsim --test shard_6 cockpit::`.

mod fixture {
    use super::m6_frame;
    use std::path::PathBuf;

    use mush_core::ObjId;
    use mush_sim::data::Pose;
    use smol_str::SmolStr;
    use tacsim::arena::clock::Tempo;
    use tacsim::arena::condition::{Condition, ConditionBand};
    use tacsim::arena::conditions::{FireCondition, Rule, SlotTarget, aspect_mask};
    use tacsim::arena::core::CoreClass;
    use tacsim::arena::perception::{
        Aspect, FrameCondition, FrameContact, FrameNotice, FrameOdds, Named, NoticeKind,
    };
    use tacsim::arena::state::{Classification, ContactId, Designator, Power, PartyId};
    use tacsim::arena::step::{Frame, FrameFacet, FrameHeat, FrameMind, FrameSection};
    use tacsim::arena::weapons::{FrameArming, FrameBand, FrameRule, FrameWeapon};
    use tacsim::arena_task::MatchId;
    use tacsim::match_::cockpit;

    /// The env var that rewrites the fixtures instead of comparing against them.
    pub const BLESS: &str = "TACSIM_BLESS";

    /// The one match id the fixtures are stamped with.
    pub const MATCH: MatchId = MatchId(7);

    /// **16b cut I.1** moved the yard — its id, its size, its terrain and its
    /// three spots — into `support::yard`, because `tests/replay.rs` blesses the
    /// replay fixture set over the same ground. Re-exported here so every
    /// `fixture::terrain()` below reads as it did.
    pub use crate::support::yard::{HEIGHT, SURFACE, WIDTH, spots, terrain};

    /// The pilot's own unit.
    const UNIT: ObjId = ObjId(101);

    /// The pilot's cell, near the south-west corner: 39 by 20 around it clamps
    /// to the arena's own west edge AND its own south edge.
    const YOU: (u16, u16) = (6, 4);

    /// The four contacts' row keys. Opaque and never printed; distinct and
    /// ascending, because a frame's contact list is in contact-id order.
    const HOSTILE: ContactId = ContactId(0x1111_1111_1111_1111);
    const FRIEND: ContactId = ContactId(0x2222_2222_2222_2222);
    const ON_A_ROOF: ContactId = ContactId(0x3333_3333_3333_3333);
    const OFF_WINDOW: ContactId = ContactId(0x4444_4444_4444_4444);

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
            unit_type: "tacsim.unit.line_tank".into(),
            side_unit_types: vec!["tacsim.unit.line_tank".into()],
            sensor_height_cm: 150,
            callsign: Designator::Slot {
                team: PartyId::RED,
                slot: 1,
            },
            cell: Some(YOU),
            mode: tacsim::arena::state::MatchMode::Skirmish,
            objective: None,
            motor: None,
            own_last_hit_ms: None,
            team: PartyId::RED,
            pose: Pose {
                x_cm: i64::from(YOU.0) * 1_000 + 500,
                y_cm: i64::from(YOU.1) * 1_000 + 500,
                z_cm: 0,
                heading_mdeg: 45_000,
                posture: posture(),
            },
            speed_cms: 1_200,
            power: Power::Online,
            held: false,
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
            // **16b cut D.4**: the fixture's pilot stands in no bubble, so its
            // own receiver is clean and the payload's `receiver` key is `null`.
            // The README's rule is that the scene never carries a value the
            // server does not send, and a degraded receiver would have put a
            // second `STATUS` line and a fourth panel row into every golden.
            receiver: None,
            // **16b cut O.2**: and it carries no jamming pod — a stock tank —
            // so the payload's `ecm` key is `null` and no golden's scope row
            // grows a suffix.
            ecm: None,
            contacts: vec![
                // Classified hostile, read two seconds ago, and the lock.
                FrameContact {
                    id: HOSTILE,
                    designator: Designator::Slot {
                        team: PartyId::BLUE,
                        slot: 1,
                    },
                    alias: Some(1),
                    class: Classification::Classified { party: PartyId::BLUE },
                    moving: false,
                    bearing_mdeg: 45_000,
                    range_cm: 43_000,
                    age_ms: 1_200,
                    cell: Some((36, 14)),
                    condition: Some(FrameCondition {
                        table: 0,
                        bands: reading(),
                        age_ms: 2_000,
                    }),
                    // **17b cut M1a.3**: seen broadside, a reading whose
                    // destroyed turret is not the chassis's fatal hull, and the
                    // fatal words beside it.
                    aspect: Aspect::Left,
                    facing: tacsim::arena::perception::ContactFacing::Left,
                    reads_dead: Some(false),
                    fatal: Some(vec![SmolStr::new_static("hull")]),
                    unit_type: Some("tacsim.unit.line_tank".into()),
                },
                // Classified friendly off the mesh: no reading at all, which is
                // what keeps its chassis out of `scan_tables`.
                FrameContact {
                    id: FRIEND,
                    designator: Designator::Slot {
                        team: PartyId::RED,
                        slot: 2,
                    },
                    alias: None,
                    class: Classification::Classified { party: PartyId::RED },
                    moving: false,
                    bearing_mdeg: 270_000,
                    range_cm: 12_000,
                    age_ms: 400,
                    cell: Some((2, 9)),
                    condition: None,
                    // 17b cut M1a.3: nose-on; no reading, so no fatal words.
                    aspect: Aspect::Front,
                    facing: tacsim::arena::perception::ContactFacing::Front,
                    reads_dead: Some(false),
                    fatal: None,
                    unit_type: Some("tacsim.unit.line_tank".into()),
                },
                // Unknown, standing ON the building block inside the window: its
                // marker overwrites the wall glyph.
                FrameContact {
                    id: ON_A_ROOF,
                    designator: Designator::Provisional(3),
                    alias: None,
                    class: Classification::Unknown,
                    moving: false,
                    bearing_mdeg: 315_000,
                    range_cm: 22_000,
                    age_ms: 3_500,
                    cell: Some((14, 7)),
                    condition: None,
                    // 17b cut M1a.3: an aspect is told for an unclassified
                    // return — the plot's heading — and no odds ever are.
                    aspect: Aspect::Front,
                    facing: tacsim::arena::perception::ContactFacing::Front,
                    reads_dead: Some(false),
                    fatal: None,
                    unit_type: None,
                },
                // Unknown, plotted north of the window: on `CONTACTS`, not on
                // the map, and the reason `VIEW` says its one line.
                FrameContact {
                    id: OFF_WINDOW,
                    designator: Designator::Provisional(4),
                    alias: None,
                    class: Classification::Unknown,
                    moving: false,
                    bearing_mdeg: 30_000,
                    range_cm: 58_000,
                    age_ms: 6_000,
                    cell: Some((50, 33)),
                    condition: None,
                    aspect: Aspect::Rear,
                    facing: tacsim::arena::perception::ContactFacing::Rear,
                    reads_dead: Some(false),
                    fatal: None,
                    unit_type: None,
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
                            team: PartyId::BLUE,
                            slot: 1,
                        },
                    },
                    section: SmolStr::new_static("turret"),
                },
            ],
            lock: Some(HOSTILE),
            rules: vec![FrameRule {
                slot: 1,
                rule: Rule::Fire {
                    condition: FireCondition {
                        weapon: "cannon".into(),
                        target: SlotTarget::Lock,
                        min_permille: 600,
                        aspects: aspect_mask(&[Aspect::Left, Aspect::Right, Aspect::Rear]),
                        min_rounds_left: 8,
                        min_range_m: 0,
                        max_range_m: u32::MAX,
                    },
                    predicates: Vec::new(),
                    implicit: true,
                    independent: true,
                },
            }],
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
                    // **17b cut M1b.3**: armed in S1 on the lock — at 60 per
                    // cent or better, on a flank or the rear, keeping eight
                    // rounds back (`S1: >=60% side/rear keeps 8`).
                    armed: Some(FrameArming {
                        slot: 1,
                        target: SlotTarget::Lock,
                        min_permille: 600,
                        aspects: aspect_mask(&[Aspect::Left, Aspect::Right, Aspect::Rear]),
                        keeps: 8,
                        min_range_m: 0,
                        max_range_m: u32::MAX,
                    }),
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
                    // 17b cut M1b.3: no slot holds it — `"armed": null`.
                    armed: None,
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
            // **16b cut Y.4**: the scene is an ordinary pulse, not the first
            // frame after a reclaim, so the payload's `catchup` key is `null`.
            // `catchup_lines.json` carries the report's own shapes.
            catchup: None,
            // **17b cut M1a.3**, re-cut by **17b cut M1b.3** (Q2): the odds
            // rows are the lock first and then the nearest classified
            // contacts of ANOTHER side — so `BA` alone, led by the cannon, the
            // weapon a bare `FIRE` fires. `BA`'s figures are the lock's own
            // (`p_permille` on the weapon rows above), because the lock is on
            // `BA` and both are one reading of one plot. No odds for `RB`
            // (red's own: the rows never carry a friend's, though it is nearer)
            // and none for `X3` or `X4` (not told apart). The frame the arena
            // now builds for this scene (`Arena::frame_odds`).
            odds: vec![
                FrameOdds {
                    contact: HOSTILE,
                    weapon: SmolStr::new_static("cannon"),
                    p_permille: 624,
                },
                FrameOdds {
                    contact: HOSTILE,
                    weapon: SmolStr::new_static("machine_gun"),
                    p_permille: 210,
                },
            ],
            // A piloted hull is under no standing order on this surface.
            order: None,
            t_ms: 61_250,
            // **17b cut M1b.3**: the shipped Compact floor, 10 per cent.
            odds_floor_permille: 100,
            // **17b cut M5.4** (doc 17 §12): the pilot chose Standard Doctrine
            // on Sniper and Line and has not switched; the mind is rated I on
            // the line tank's Class II core and stands, dormant under the seat.
            mind: Some(FrameMind {
                doctrine: true,
                profiles: m6_frame(&["Sniper", "Line"]).profiles,
                active: 0,
                class: CoreClass::I,
                core_class: CoreClass::II,
                switch_at_ms: None,
                off: false,
                ..m6_frame(&["Sniper", "Line"])
            }),
            // **Doc 21 cut G.12a** (§7.3): weapons free, and the line tank's
            // ceilings — 54 kph ahead, 27 astern (`SPEED`'s refusal's figures).
            weapons_free: true,
            max_speed_cms: 1_500,
            reverse_cms: 750,
        }
    }

    /// The three payloads of the one scene, through the three pure builders.
    #[must_use]
    pub fn payloads() -> (serde_json::Value, serde_json::Value, serde_json::Value) {
        let frame = frame();
        let terrain = terrain();
        let spots = spots();
        let cockpit = cockpit::status_frame(MATCH, &frame);
        let map = cockpit::map_frame(MATCH, SURFACE, &terrain, &spots);
        let view = cockpit::view_window(cockpit.clone(), SURFACE, &terrain, &spots, &frame);
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
             `{BLESS}=1 cargo test -p tacsim --test shard_6 cockpit::`"
        );
    }
}

/// The three payloads of the one scene, byte for byte.
#[test]
fn the_canonical_cockpit_fixtures_are_what_the_builders_produce() {
    let (cockpit, map, view) = fixture::payloads();
    fixture::pinned(
        "cockpit_frame.json",
        include_str!("../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"),
        &fixture::text(&cockpit),
    );
    fixture::pinned(
        "arena_map.json",
        include_str!("../../../../web/src/lib/fixtures/cockpit/arena_map.json"),
        &fixture::text(&map),
    );
    fixture::pinned(
        "view_frame.json",
        include_str!("../../../../web/src/lib/fixtures/cockpit/view_frame.json"),
        &fixture::text(&view),
    );
}

/// **16b cut AG.1**: the brief of the one scene, as `brief_lines` says it over
/// the very window `view_frame.json` pins — one more reading of the one scene,
/// and read by this test alone (the brief has one renderer; neither the TUI nor
/// the web reads it). Joined by newlines with a trailing one, as the file holds
/// it.
fn fixture_brief() -> String {
    let (_cockpit, _map, view) = fixture::payloads();
    let lines = cockpit::brief_lines(
        &fixture::frame(),
        &fixture::terrain(),
        &fixture::spots(),
        &view,
        &BTreeMap::new(),
    );
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// **16b cut AG.1**: `view_brief.txt` is what `brief_lines` produces over the
/// fixture scene, byte for byte (re-bless with `TACSIM_BLESS=1`).
#[test]
fn the_brief_golden_is_what_brief_lines_produces() {
    fixture::pinned(
        "view_brief.txt",
        include_str!("../../../../web/src/lib/fixtures/cockpit/view_brief.txt"),
        &fixture_brief(),
    );
}

/// **17b cut M1a.2: the move changed no byte** (doc 17 §3 "One formatter",
/// D-UM-4; 17b §0 item 3). The row formatter left `match_::cockpit` for
/// `arena::view`, and the two sentences a contact is printed in are pinned
/// here against what HEAD printed before the move, in two halves because they
/// are two sentences:
///
/// - **`CONTACTS`** prints [`cockpit::ContactRow::render`]: the four rows of
///   the fixture scene, as LITERALS read at HEAD (`10a5a0ec`) before the move —
///   no committed fixture carries them, so they are held here. **17b cut
///   M1a.3** grew each row by its two new columns (doc 17 §3: the aspect's side
///   word, then the main gun's odds or a dash), so the literals are now the rows
///   as that cut prints them, and each is the M1a.2 row plus its tail;
/// - **`VIEW BRIEF`** prints [`cockpit::contact_sentence`], which stayed in the
///   cockpit: the same four rows through it equal the four contact lines of
///   `view_brief.txt`, byte for byte.
///
/// Both read [`cockpit::frame_contacts_held`] with an empty range memory, as
/// `brief_lines` does for the golden — the re-exported path, which is also the
/// one the bridge, the read verbs and the bot's contract test resolve. And
/// the re-export IS the new home, not a copy beside it: the rows read through
/// `tacsim::arena::view` and `tacsim::arena` are typed as `arena::view`'s
/// `ContactRow` and equal the cockpit's, so a local copy pasted back into the
/// cockpit (two formatters that could drift, which D-UM-4 forbids) stops this
/// compiling.
#[test]
fn m1a2_the_rows_render_as_before() {
    const CONTACTS_SINCE_M1A3: [&str; 4] = [
        "BA  blue  bearing 045° (rel 000°)  far  450 m  1 s  side  cannon 62%",
        "RB  red  bearing 270° (rel 225°)  close  100 m  0 s  front  -",
        "X3  unknown  bearing 315° (rel 270°)  effective  200 m  3 s  front  -",
        "X4  unknown  bearing 030° (rel 345°)  far  600 m  6 s  rear  -",
    ];
    let frame = fixture::frame();
    let rows = cockpit::frame_contacts_held(&frame, &BTreeMap::new());
    // (a) `CONTACTS`' rows, as 17b cut M1a.3 renders them.
    let rendered: Vec<String> = rows.iter().map(cockpit::ContactRow::render).collect();
    assert_eq!(
        rendered, CONTACTS_SINCE_M1A3,
        "the CONTACTS rows moved a byte"
    );
    // The memory-less reading is the same rows (`frame_contacts` is
    // `frame_contacts_held` with no memory).
    assert_eq!(cockpit::frame_contacts(&frame), rows);
    // One function, not two: the cockpit's rows are `arena::view`'s type, and
    // the view's home and the arena's re-export read the same rows.
    let via_view: Vec<tacsim::arena::view::ContactRow> = rows.clone();
    assert_eq!(
        tacsim::arena::view::frame_contacts_held(&frame, &BTreeMap::new()),
        via_view
    );
    let via_arena: Vec<tacsim::arena::view::ContactRow> =
        tacsim::arena::frame_contacts_held(&frame, &BTreeMap::new());
    assert_eq!(via_arena, rows);
    // (b) The brief's sentence over the same rows, against the golden's own
    // contact lines — the ones that open with a designator and a comma.
    let golden = include_str!("../../../../web/src/lib/fixtures/cockpit/view_brief.txt");
    let brief: Vec<&str> = golden
        .lines()
        .filter(|line| {
            rows.iter()
                .any(|row| line.starts_with(&format!("{}, ", row.designator)))
        })
        .collect();
    assert_eq!(
        brief.len(),
        4,
        "the golden carries four contact lines: {brief:#?}"
    );
    let said: Vec<String> = rows.iter().map(cockpit::contact_sentence).collect();
    assert_eq!(said, brief, "the brief's contact sentences moved a byte");
    assert_eq!(
        brief.first().copied(),
        Some("BA, blue: bearing 045° (rel 000°), far, 450 m, 1 s old.")
    );
}

/// **16b cut AG.1: the brief's order** — the header, the eight bearings in
/// compass order, the contacts in frame order, the beyond-window line, the
/// spots the legend names in the window, the spots off it and (16b cut AP.4)
/// `End of brief.` — seventeen lines on the scene; and THIRTEEN with the
/// scene's contacts cleared, over a window rebuilt from that cleared frame (a
/// payload the server could send): `No contacts in the window.` (the brief's
/// own sentence since AP.4, never `CONTACTS`' `No contacts.`), no beyond line,
/// `B` The Roof named in the window now that nothing covers it, and the end
/// line last.
#[test]
fn ag1_brief_lines_is_the_header_the_eight_rays_the_contacts_and_the_spots_in_that_order() {
    let (_cockpit, _map, view) = fixture::payloads();
    let lines = cockpit::brief_lines(
        &fixture::frame(),
        &fixture::terrain(),
        &fixture::spots(),
        &view,
        &BTreeMap::new(),
    );
    assert_eq!(lines.len(), 17, "{lines:#?}");
    let line = |n: usize| lines.get(n).map_or("", String::as_str);
    assert!(line(0).starts_with("Window around RA:"), "{}", line(0));
    for (n, (bearing, _, _)) in cockpit::BRIEF_BEARINGS.iter().enumerate() {
        assert!(
            line(n + 1).starts_with(&format!("{bearing}:")),
            "line {} is not {bearing}: {}",
            n + 1,
            line(n + 1)
        );
    }
    let contacts: Vec<String> = cockpit::frame_contacts(&fixture::frame())
        .iter()
        .map(cockpit::contact_sentence)
        .collect();
    assert_eq!(contacts.len(), 4);
    for (n, sentence) in contacts.iter().enumerate() {
        assert_eq!(line(9 + n), sentence, "the contacts in frame order");
    }
    assert_eq!(
        line(13),
        "1 contact plots beyond this window; CONTACTS lists it."
    );
    assert!(line(14).starts_with("In the window:"), "{}", line(14));
    assert!(line(15).starts_with("Off the window:"), "{}", line(15));
    assert_eq!(line(16), cockpit::BRIEF_END_LINE);
    assert_eq!(cockpit::BRIEF_END_LINE, "End of brief.");

    let mut cleared = fixture::frame();
    cleared.contacts.clear();
    let window = cockpit::view_window(
        cockpit::status_frame(fixture::MATCH, &cleared),
        fixture::SURFACE,
        &fixture::terrain(),
        &fixture::spots(),
        &cleared,
    );
    let quiet = cockpit::brief_lines(
        &cleared,
        &fixture::terrain(),
        &fixture::spots(),
        &window,
        &BTreeMap::new(),
    );
    assert_eq!(quiet.len(), 13, "13 + C lines at C = 0: {quiet:#?}");
    let quiet_line = |n: usize| quiet.get(n).map_or("", String::as_str);
    assert_eq!(quiet_line(0), line(0), "the same window, the same header");
    for n in 1..=8 {
        assert_eq!(quiet_line(n), line(n), "the ground does not move");
    }
    assert_eq!(quiet_line(9), "No contacts in the window.");
    assert_eq!(quiet_line(9), cockpit::NO_CONTACTS_IN_WINDOW);
    assert_eq!(
        quiet_line(10),
        "In the window: A The Ford, 190 m at 093°; B The Roof, 90 m at 069°."
    );
    assert_eq!(
        quiet_line(11),
        "Off the window: C The Woods, 490 m at 064°."
    );
    assert_eq!(quiet_line(12), "End of brief.");
}

/// **16b cut AG.1: the brief names only what the grid drew**, checked over the
/// fixture payload and the golden's own lines: every class word maps back to a
/// class the legend lists; every designator is a marker the window drew, or the
/// one contact the beyond line counts (`X4`); the in-window spot has its legend
/// entry, the off-window spot no marker at all, and the covered `B` is in
/// neither; and nothing is an object number.
#[test]
fn ag1_the_brief_names_only_what_the_grid_drew() {
    let (_cockpit, _map, view) = fixture::payloads();
    let brief = include_str!("../../../../web/src/lib/fixtures/cockpit/view_brief.txt");
    let lines: Vec<&str> = brief.lines().collect();
    assert_eq!(
        lines.len(),
        17,
        "the golden is the seventeen lines: {brief}"
    );
    assert_eq!(lines.last(), Some(&"End of brief."), "{brief}");
    let array = |key: &str| {
        view.get(key)
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let legend = array("legend");
    let units = array("units");

    // The ground: every class word is a class the legend names by its label.
    for line in lines.iter().skip(1).take(8) {
        let (_, said) = line.split_once(": ").expect("a bearing and its ground");
        for part in said.split(", then ") {
            let word = if let Some((_, class)) = part.split_once(" m of ") {
                class
            } else if let Some((class, _)) = part.split_once(" to ") {
                class
            } else {
                // The tail: `beyond the window.` or `the edge of the arena.`
                assert!(
                    part == "beyond the window." || part == "the edge of the arena.",
                    "{line}"
                );
                continue;
            };
            let class = mush_atlas::TerrainClass::ALL
                .into_iter()
                .find(|class| cockpit::class_prose(*class) == word)
                .unwrap_or_else(|| panic!("{word:?} is no class's prose: {line}"));
            assert!(
                legend
                    .iter()
                    .any(|entry| entry.get(1).and_then(serde_json::Value::as_str)
                        == Some(class.label())),
                "{word:?} ({}) is not in the window's legend: {line}",
                class.label()
            );
        }
    }

    // The contacts: a drawn marker's id, or counted by the beyond line.
    let ids: Vec<&str> = units
        .iter()
        .filter_map(|unit| unit.get("id").and_then(serde_json::Value::as_str))
        .collect();
    let undrawn: Vec<&str> = lines
        .iter()
        .filter(|line| line.contains(": bearing "))
        .filter_map(|line| line.split_once(", ").map(|(designator, _)| designator))
        .filter(|designator| !ids.contains(designator))
        .collect();
    assert_eq!(undrawn, vec!["X4"], "exactly one contact is off the window");
    assert!(
        lines.contains(&"1 contact plots beyond this window; CONTACTS lists it."),
        "{brief}"
    );

    // The spots: the named one in the window, the undrawn one off it.
    let in_window = lines
        .iter()
        .find(|line| line.starts_with("In the window:"))
        .expect("the in-window line");
    let off_window = lines
        .iter()
        .find(|line| line.starts_with("Off the window:"))
        .expect("the off-window line");
    assert_eq!(*in_window, "In the window: A The Ford, 190 m at 093°.");
    assert!(legend.contains(&serde_json::json!(["A", "The Ford"])));
    assert_eq!(*off_window, "Off the window: C The Woods, 490 m at 064°.");
    assert!(
        !units.iter().any(|unit| {
            unit.get("spot").and_then(serde_json::Value::as_bool) == Some(true)
                && unit.get("id").and_then(serde_json::Value::as_str) == Some("C")
        }),
        "C is not drawn"
    );
    for line in [in_window, off_window] {
        assert!(
            !line.contains("B The Roof"),
            "the covered B is named: {line}"
        );
    }

    // No object number, no sixteen digits.
    for token in brief.split_whitespace() {
        let token = token.trim_matches(|c: char| ",.;:()°".contains(c));
        assert!(!token.contains('#'), "an object number: {token}");
        assert!(
            !(token.len() == 16 && token.chars().all(|c| c.is_ascii_hexdigit())),
            "a sixteen-digit word: {token}"
        );
    }
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
    // **16b cut V.1**: and two of the three spots, which go down FIRST.
    let units = view
        .get("units")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        units.len(),
        6,
        "two spot markers, the platform and three contact markers: {view}"
    );
    assert_eq!(
        units.first(),
        Some(&serde_json::json!({
            "id": "A",
            "name": "The Ford",
            "cell": [25, 3],
            "glyph": "A",
            "you": false,
            "spot": true,
        })),
        "the first spot is FIRST: the marker order (spots, `@`, contacts) is the \
         rule every compositor follows"
    );
    let own = units
        .iter()
        .position(|unit| unit.get("you").and_then(serde_json::Value::as_bool) == Some(true))
        .expect("the pilot's own marker is in the window");
    assert_eq!(
        own, 2,
        "the pilot's own marker follows the two spot markers: {view}"
    );
    let drawn: Vec<&str> = units
        .iter()
        .skip(own + 1)
        .filter_map(|unit| unit.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(drawn, vec!["BA", "RB", "X3"], "in frame order: {view}");
    // `B` The Roof is placed — it is in `units` — and then covered by `X3`,
    // which stands on the same cell; so it is named in no legend. `C` The Woods
    // is off the window and is not a marker at all.
    let roof = units
        .iter()
        .find(|unit| unit.get("id").and_then(serde_json::Value::as_str) == Some("B"))
        .expect("B is placed before X3 covers it");
    assert_eq!(roof.get("cell"), Some(&serde_json::json!([14, 7])));
    assert!(
        !units
            .iter()
            .any(|unit| unit.get("id").and_then(serde_json::Value::as_str) == Some("C")),
        "C is off the window: {view}"
    );
    let legend = view
        .get("legend")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !legend
            .iter()
            .any(|entry| entry.get(0).and_then(serde_json::Value::as_str) == Some("B")),
        "a spot a contact covers is named nowhere: {view}"
    );
    let unknown = legend
        .iter()
        .position(|entry| entry == &serde_json::json!(["?", "contact"]))
        .expect("the unknown contact's glyph is in the legend");
    assert_eq!(
        legend.last(),
        Some(&serde_json::json!(["A", "The Ford"])),
        "the spots are named LAST: {view}"
    );
    assert!(unknown < legend.len() - 1, "after the contacts: {view}");
    // The map carries all three, in authored order, in arena cells.
    assert_eq!(
        map.get("spots"),
        Some(&serde_json::json!([
            { "letter": "A", "name": "The Ford", "cell": [25, 3] },
            { "letter": "B", "name": "The Roof", "cell": [14, 7] },
            { "letter": "C", "name": "The Woods", "cell": [50, 25] },
        ])),
        "{map}"
    );
    // And the contacts' own line still counts contacts: X4 alone.
    assert_eq!(
        cockpit::beyond_window_line(&fixture::frame(), &view),
        Some("1 contact plots beyond this window; CONTACTS lists it.".to_owned())
    );
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

/// **16b cut V.1: the pilot's own `@` owns a spot's cell exactly as a contact
/// does**, on the line client's legend — the case the TUI and the web each pin
/// (`tui_golden.rs`, `cockpit.test.ts`) and the fixture, whose covered spot is
/// under a contact, cannot. The fixture frame with `B` Home Plate put on the
/// pilot's own cell (6, 4): the spot is placed in `units` before the `@` and
/// then covered, so the grid draws `@` there and the legend names `A` and not
/// `B` — the three renderers name the same spots.
#[test]
fn v1_a_spot_under_the_pilot_s_own_marker_is_placed_and_named_nowhere() {
    let frame = fixture::frame();
    let spots = {
        let mut spots = fixture::spots();
        spots.truncate(1);
        spots.push(tacsim::arena::state::KeySpot {
            name: smol_str::SmolStr::new("Home Plate"),
            x: 6,
            y: 4,
        });
        spots
    };
    let view = cockpit::view_window(
        cockpit::status_frame(fixture::MATCH, &frame),
        fixture::SURFACE,
        &fixture::terrain(),
        &spots,
        &frame,
    );
    let units = view
        .get("units")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let home = units
        .iter()
        .position(|unit| unit.get("id").and_then(serde_json::Value::as_str) == Some("B"))
        .expect("the spot on the pilot's cell is placed");
    let own = units
        .iter()
        .position(|unit| unit.get("you").and_then(serde_json::Value::as_bool) == Some(true))
        .expect("the pilot's own marker");
    assert!(home < own, "the spot goes down before the `@`: {view}");
    assert_eq!(
        units.get(home).and_then(|unit| unit.get("cell")),
        units.get(own).and_then(|unit| unit.get("cell")),
        "on the very cell the `@` is drawn on"
    );
    let legend = view
        .get("legend")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        !legend
            .iter()
            .any(|entry| entry.get(0).and_then(serde_json::Value::as_str) == Some("B")),
        "a spot the `@` covers is named nowhere: {view}"
    );
    assert_eq!(
        legend.last(),
        Some(&serde_json::json!(["A", "The Ford"])),
        "the ford still owns its cell and is named last: {view}"
    );
    let rows = fixture::expand_rows(&view);
    assert_eq!(
        rows.get(4).and_then(|row| row.get(6)).copied(),
        Some('@'),
        "the grid draws the pilot, not the spot"
    );
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

/// **The seat, resolved once** (16b cut U.4, 16c `T-53`).
///
/// `flying` answers the whole [`match_::Seat`] — the driver, the unit and the
/// station — and every cockpit verb reads the cockpit as `seat.driver` while it
/// answers the typist. For a human flying their own seat the driver **is** the
/// typist, the unit is the pod's tank, and the station is the cockpit. The case
/// where they differ — a human driving an NPC's seat — is
/// [`a_human_driving_an_npc_s_seat_reads_the_npc_s_cockpit`].
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn flying_names_the_seat_a_human_pilot_drives_themselves() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    for n in 0..2 {
        let pilot = rig.pilot(n);
        let seat =
            match_::flying(rig.tw.world(), pilot).expect("a boarded pilot of a running match");
        assert_eq!(
            seat,
            match_::Seat {
                driver: pilot,
                unit: rig.tank(n),
                station: mush_sim::StationId::new(match_::STATION).expect("the cockpit station"),
            },
            "pilot {n}'s seat is their own"
        );
    }
    // The verbs read it: STATUS answers the pilot, off the pilot's own frame.
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let said = rig.say(0, "status");
    assert!(said.contains("Callsign RA."), "{said}");
}

/// **A human driving an NPC's seat reads the NPC's cockpit** (16b cut U.4, 16c
/// `T-53`).
///
/// An NPC claims red slot 1 and boards it like anybody; then a human's account
/// takes that seat as a proxy (the `ActiveFocus` `+seat/activate` installs,
/// seeded through `mush-sim`'s `testkit` door as `mush-propagation`'s proxy-seat
/// tests seed it). The arena keys the cockpit — the last frame, the power hint,
/// the pilot binding — on the NPC, so the human's `STATUS` must read the NPC's
/// frame. Before the seat was resolved once, the unit came through the proxy and
/// the frame was looked up under the typist: the human read their own empty
/// mirror about the NPC's tank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_human_driving_an_npc_s_seat_reads_the_npc_s_cockpit() {
    // Pilot1 flies blue; Pilot2 is the human who will drive the NPC's seat.
    let mut rig = Rig::boot(3);
    let human = rig.pilot(2);
    let lobby = match_::centre_object(rig.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
    let npc = rig
        .tw
        .world_mut()
        .create(mush_core::ObjKind::Npc, "Ace", GOD, Some(lobby))
        .expect("an NPC walks into the lobby");
    // The NPC is on an account of its own (it takes the seat before any proxy
    // does); the humans keep theirs.
    let mut map = Accounts::default();
    for (n, pilot) in rig.pilots.iter().enumerate() {
        map.0.insert(
            *pilot,
            AccountId::mint(&format!("account-{n}"), n as i64).expect("an account id"),
        );
    }
    map.0.insert(
        npc,
        AccountId::mint("account-npc", 99).expect("an account id"),
    );
    let human_account = map.0.get(&human).copied().expect("the human's account");
    *rig.tw
        .world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));

    let npc_says = |rig: &mut Rig, line: &str| {
        let events = rig.events_of(npc, line);
        rig.tw.seen_by(&events, npc).join("\n")
    };
    npc_says(&mut rig, "sim team red");
    rig.say(1, "sim team blue");
    npc_says(&mut rig, "sim start");
    let red_room =
        match_::centre_object(rig.tw.world(), match_::RED_ROOM_ENTRY).expect("the red room");
    rig.tw
        .world_mut()
        .move_to(npc, red_room)
        .expect("the centre's rooms take anybody");
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    let said = npc_says(&mut rig, "board");
    let pod = match_::pod_of(rig.tw.world(), npc)
        .unwrap_or_else(|| panic!("the NPC's pod is not locked: {said}"));
    let tank = match_::tank_of(rig.tw.world(), pod).expect("the pod's tank");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    let station = mush_sim::StationId::new(match_::STATION).expect("the cockpit station");
    assert_eq!(
        match_::flying(rig.tw.world(), npc),
        Some(match_::Seat {
            driver: npc,
            unit: tank,
            station: station.clone(),
        }),
        "the NPC flies its own seat"
    );

    // The arena speaks to the NPC.
    let mut heard = false;
    for _ in 0..400 {
        rig.pump();
        heard = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(npc))
            .is_some();
        if heard {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(heard, "the arena sends the NPC a frame");

    // The human's account takes the seat: `actor` the NPC, `driver` the human,
    // `proxy` — exactly what `+seat/activate` installs over the NPC's own focus.
    let mut focus = rig
        .tw
        .world()
        .module_state::<mush_sim::FocusState>()
        .and_then(|state| state.focus_of_actor(npc))
        .cloned()
        .expect("the NPC's boarding focused the cockpit");
    focus.account = human_account;
    focus.driver = human;
    focus.proxy = true;
    rig.tw
        .world_mut()
        .module_state_mut::<mush_sim::FocusState>()
        .expect("the focus slot")
        .claim_for_test(focus);
    assert_eq!(
        mush_sim::driving_actor(rig.tw.world(), human),
        npc,
        "the human drives the NPC"
    );

    // One resolution names both: the NPC's seat, with the NPC as its driver.
    assert_eq!(
        match_::flying(rig.tw.world(), human),
        Some(match_::Seat {
            driver: npc,
            unit: tank,
            station,
        }),
        "the human's seat is the NPC's"
    );

    // And the verb reads it: the human's STATUS is the NPC's cockpit, addressed
    // to the human.
    let theirs = npc_says(&mut rig, "status");
    let said = rig.say(2, "status");
    assert!(said.contains("Callsign RA."), "{said}");
    assert_eq!(
        said, theirs,
        "the driver read something other than the NPC's own cockpit"
    );
}

/// The map of the SHIPPED proving ground expands to the authored terrain cell
/// for cell.
///
/// The live ground and not [`match_::stock_definition`]'s uniform stand-in: the
/// round trip is about what the content pack actually paints, and a flat window
/// would run-length-encode to nothing and prove nothing. **16b cut U.4** moved
/// the size half to `tests/definitions.rs`, beside the rule that now enforces it
/// at apply.
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
    let map = cockpit::map_frame(
        match_id,
        definition.id.as_str(),
        terrain,
        &definition.key_spots,
    );
    // 16b cut U.4 (16c `T-57`): the size bound is a rule now, enforced when the
    // arena is applied (`definitions::arena_map_within_bound`), and its test is
    // `tests/definitions.rs`'. What is left here is the round trip.
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
            &fixture::spots(),
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
            "schema": { "id": "arena.cockpit", "version": 8, "retain": "latest" },
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
    use tacsim::arena::state::{Classification, PartyId};

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
        .retain(|held| held.class == Classification::Classified { party: PartyId::RED });
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
    // And `FIRE` refuses in the cockpit, one pulse earlier than the arena would —
    // since 16b cut AE.3 with `WEAPONSTATUS`'s own sentence, since a bare `FIRE`
    // names no gun (it read `That unit carries no weapon called `cannon`.`).
    let said = rig.say(0, "fire");
    assert!(said.contains("This unit carries no weapon."), "{said}");
    assert!(!said.contains("called `cannon`"), "{said}");
    assert!(!said.contains("Firing"), "{said}");
    // A named weapon on the empty list still names what the pilot typed.
    let said = rig.say(0, "fire cannon");
    assert!(
        said.contains("That unit carries no weapon called `cannon`."),
        "{said}"
    );
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
            .any(|slot| slot.team == tacsim::arena::state::PartyId::BLUE && slot.left),
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
    use tacsim::arena::state::{Classification, ContactId, Designator, PartyId};
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    // A classified return on the pilot's **own** side, one on the other side and
    // an unidentified blip, so one frame carries all three answers.
    let mut seen = rig.held_frame(0);
    seen.notices.clear();
    let other = if seen.team == PartyId::RED {
        PartyId::BLUE
    } else {
        PartyId::RED
    };
    let row = |id: u64, designator: Designator, class: Classification| FrameContact {
        id: ContactId(id),
        designator,
        alias: None,
        class,
        moving: false,
        bearing_mdeg: 45_000,
        range_cm: 12_000,
        age_ms: 0,
        cell: None,
        condition: None,
        aspect: tacsim::arena::perception::Aspect::Front,
        facing: tacsim::arena::perception::ContactFacing::Front,
        reads_dead: None,
        fatal: None,
        unit_type: None,
    };
    let friend = row(
        0xa1a1_a1a1_a1a1_a1a1,
        Designator::Slot {
            team: seen.team,
            slot: 2,
        },
        Classification::Classified { party: seen.team },
    );
    let hostile = row(
        0xb2b2_b2b2_b2b2_b2b2,
        Designator::Slot {
            team: other,
            slot: 1,
        },
        Classification::Classified { party: other },
    );
    let blip = row(
        0xc3c3_c3c3_c3c3_c3c3,
        Designator::Provisional(9),
        Classification::Unknown,
    );
    seen.contacts = vec![friend.clone(), hostile.clone(), blip.clone()];
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
    // **16b cut B.1** (`T-86`): the victim's own section IS on the `section`
    // key now — the pilot's own tank, outside the observer rule — and `kind`
    // says whose it is; it names no contact. The stronger pin, over all three
    // victim kinds, is `b1_the_victims_section_and_the_old_name_are_published`.
    assert_eq!(
        key(&struck, "section"),
        serde_json::json!("left_track"),
        "the victim's own section rides the `section` key, and `kind` says whose"
    );
    assert_eq!(key(&struck, "contact"), serde_json::Value::Null);
    assert_eq!(key(&struck, "designator"), serde_json::Value::Null);
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
            "cause", "refusal", "order", "weapon", "detail", "friendly", "spent", "was", "section",
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
    // 16b cut Y.1: `ORDER` too, and before its authority — this rig fields no
    // machine, so a check that came after it would answer `No such squad.`.
    for verb in [
        "startup",
        "heading 90",
        "speed 10",
        "lock BA",
        "fire",
        "order red-1 hold",
    ] {
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
// Doc 21 §7.3 (2026-10-27) — the picture's keys order QUIET
// ---------------------------------------------------------------------------

/// **`HEADING … QUIET` and `SPEED … QUIET` order without an acknowledgement**
/// (doc 21 §7.3, the owner's first playtest of G.12: a held `w` read `Making 54
/// kph.` in the comms every quarter second). The word is case-insensitive and
/// trailing; a refusal — the usage, the ceilings — is still spoken, and the
/// plain verb still acknowledges.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quiet_orders_are_taken_without_an_acknowledgement() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the tank never came online");

    let said = rig.say(0, "speed 20 QUIET");
    assert!(!said.contains("Making"), "{said}");
    assert!(!said.contains("SPEED <kph>"), "{said}");
    let said = rig.say(0, "heading 90 quiet");
    assert!(!said.contains("Steering"), "{said}");
    assert!(!said.contains("HEADING <0-359>"), "{said}");
    // Refusals speak whichever way the order came.
    let said = rig.say(0, "speed 999 quiet");
    assert!(said.contains("That unit makes 54 kph ahead"), "{said}");
    let said = rig.say(0, "heading 400 quiet");
    assert!(said.contains("HEADING <0-359>"), "{said}");
    let said = rig.say(0, "heading quiet");
    assert!(said.contains("HEADING <0-359>"), "{said}");
    // The plain verb is as it was.
    let said = rig.say(0, "speed 54");
    assert!(said.contains("Making 54 kph."), "{said}");
    let said = rig.say(0, "heading 180");
    assert!(said.contains("Steering to 180°."), "{said}");
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
        team: PartyId::BLUE,
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

// ---------------------------------------------------------------------------
// 16b cut D.4 — a lock's range is as fine as the plot allows, and a pilot reads
// their own receiver (`T-20`, `B-20`, `B-22`)
// ---------------------------------------------------------------------------

/// **The step a lock's range is rounded to, by the age of the plot it is read
/// off** (16b cut D.4, `T-20`; 16c section 5 Q28).
///
/// The table, at the two ends and at both sides of each boundary the formula
/// has: the floor a fresh plot keeps, the millisecond it stops keeping it, the
/// one- and two-second plots, the ceiling and the millisecond it is reached at,
/// a plot far past it, and a negative age — which nothing produces and which
/// must not underflow into a fifty-metre step.
#[test]
fn d4_the_lock_range_step_is_the_drift_of_the_plot_rounded_up_to_ten() {
    for (age_ms, step) in [
        (0_i64, 10_u64),
        (647, 10),
        (648, 20),
        (1_000, 20),
        (2_000, 40),
        (2_999, 50),
        (3_000, 50),
        (60_000, 50),
        (-1_000, 10),
    ] {
        assert_eq!(
            cockpit::lock_range_step_m(age_ms),
            step,
            "a plot {age_ms} ms old"
        );
    }
    // The two ends are the two constants, so the floor and the ceiling are the
    // named ones and not numbers that happen to coincide with them.
    assert_eq!(cockpit::lock_range_step_m(0), cockpit::LOCK_RANGE_STEP_M);
    assert_eq!(
        cockpit::lock_range_step_m(i64::MAX),
        cockpit::CONTACT_RANGE_STEP_M
    );
    assert_eq!(cockpit::PLOT_DRIFT_M_PER_S, 17);
}

/// **The lock's range is the plot's own age against the plot's own range** (16b
/// cut D.4, `T-20`), and the step is derivable from what the pilot already
/// holds — which is the whole defence of reading the plot's age.
///
/// Over the **raw frame JSON**: the row's `age_ms` is the only input
/// `lock_range_step_m` has, so the step publishes nothing the pilot has not
/// already been sent — not the tempo, not `sensors.pass_ms`, not anything else.
/// `lock_range_m` is then that step against the centimetres the same row was
/// built from, and `RANGE` prints the same number off the same function.
///
/// The **metres** are the one thing a client must not recompute, and the last
/// block says why: the row's own `range_m` is already rounded to the scope's
/// 50 m, so re-rounding it to the lock's step gives the coarse figure back. That
/// is exactly the two-distances-for-one-lock defect `T-20` exists to forbid,
/// and the frame forbids it by publishing `lock_range_m` itself.
///
/// Read for a fresh plot and for two stale ones, because they take different
/// branches of the clamp.
#[test]
fn d4_the_locks_range_reads_the_plots_own_age_and_is_published_not_recomputed() {
    for age_ms in [0_i64, 1_200, 4_000] {
        let mut frame = fixture::frame();
        if let Some(held) = frame.contacts.first_mut() {
            held.age_ms = age_ms;
        }
        let payload = cockpit::status_frame(fixture::MATCH, &frame);
        let row = payload
            .get("contacts")
            .and_then(serde_json::Value::as_array)
            .and_then(|rows| rows.first())
            .cloned()
            .unwrap_or_else(|| panic!("the locked contact's own row: {payload}"));
        // Everything below comes off the JSON the pilot was sent.
        let wire_age = row
            .get("age_ms")
            .and_then(serde_json::Value::as_i64)
            .unwrap_or_else(|| panic!("the row's age: {row}"));
        let wire_metres = row
            .get("range_m")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_else(|| panic!("the row's range: {row}"));
        assert_eq!(wire_age, age_ms);
        // The step comes off the wire; the centimetres are the row's own, which
        // is what `status_frame` itself rounds.
        let derived = cockpit::range_m_rounded(
            frame
                .contacts
                .first()
                .map(|held| held.range_cm)
                .unwrap_or_default(),
            cockpit::lock_range_step_m(wire_age),
        );
        assert_eq!(
            payload
                .get("lock_range_m")
                .and_then(serde_json::Value::as_u64),
            Some(derived),
            "the lock's range is not the plot's own age against its own range: {payload}"
        );
        // The row's own metres are the scope's coarse figure, locked row or not.
        assert_eq!(wire_metres % cockpit::CONTACT_RANGE_STEP_M, 0);

        // And `RANGE` says the same number: one function, so the verb, the frame
        // and the two panels' lock lines cannot give two distances.
        let rows = cockpit::frame_contacts(&frame);
        let locked = rows
            .iter()
            .find(|row| Some(row.id) == frame.lock)
            .cloned()
            .unwrap_or_else(|| panic!("the locked row"));
        assert!(
            cockpit::range_line(&frame, &locked, 1_000).contains(&format!(": {derived} m")),
            "{}",
            cockpit::range_line(&frame, &locked, 1_000)
        );
    }

    // And the hazard, named: a client that rebuilt the lock's metres out of the
    // row's own `range_m` would draw a second distance for one lock. On the
    // shared scene's 1,200 ms plot the row publishes 450 m, the lock publishes
    // 440, and re-rounding 450 to the 20 m step the same row's age asks for
    // gives 460 — three figures for one contact. That is why the metres are
    // published once and never recomputed.
    let frame = fixture::frame();
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    let coarse = payload
        .get("contacts")
        .and_then(serde_json::Value::as_array)
        .and_then(|rows| rows.first())
        .and_then(|row| row.get("range_m"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| panic!("the locked row's coarse metres: {payload}"));
    assert_eq!(coarse, 450);
    assert_eq!(
        payload
            .get("lock_range_m")
            .and_then(serde_json::Value::as_u64),
        Some(440)
    );
    assert_eq!(
        cockpit::range_m_rounded(
            coarse.saturating_mul(100),
            cockpit::lock_range_step_m(1_200)
        ),
        460,
        "the coarse metres re-rounded to the lock's own step are a third figure"
    );
}

/// **The canonical scene's lock line moves, and by exactly one step** (16b cut
/// D.4, `T-20`): `BA` stands at 43,000 cm on a plot 1,200 ms old, which is 20 m
/// of drift and therefore a 20 m step, and 430 rounds half up to 440.
///
/// It is pinned here as well as in the goldens because the goldens are text and
/// this is the arithmetic behind the one line of them that moved.
#[test]
fn d4_the_shared_scenes_lock_reads_four_hundred_and_forty_metres() {
    let frame = fixture::frame();
    let held = frame
        .contacts
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("the scene's lock"));
    assert_eq!((held.range_cm, held.age_ms), (43_000, 1_200));
    assert_eq!(cockpit::lock_range_step_m(held.age_ms), 20);
    assert_eq!(
        cockpit::status_frame(fixture::MATCH, &frame)
            .get("lock_range_m")
            .and_then(serde_json::Value::as_u64),
        Some(440)
    );
}

/// **The canonical scene's pilot stands in no bubble** (16b cut D.4, `B-22`), so
/// the payload's new key is present and `null` — a panel that reads it has to be
/// able to tell "clean" from "this server does not send it".
#[test]
fn d4_the_frame_carries_a_receiver_key_and_the_clean_scene_is_null() {
    let frame = fixture::frame();
    assert_eq!(frame.receiver, None);
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    assert_eq!(
        payload.get("receiver"),
        Some(&serde_json::Value::Null),
        "the key is missing rather than null: {payload}"
    );

    // With a fold on it: three integers and nothing else at all.
    let mut jammed = frame.clone();
    jammed.receiver = Some(tacsim::arena::perception::Receiver {
        budget: 15,
        budget_max: 40,
        reach_permille: 600,
    });
    assert!(
        !frame.same_as(&jammed, 1_000),
        "a receiver that changed degree sent no frame"
    );
    let payload = cockpit::status_frame(fixture::MATCH, &jammed);
    assert_eq!(
        payload.get("receiver"),
        Some(&serde_json::json!({
            "budget": 15,
            "budget_max": 40,
            "reach_percent": 60,
        })),
        "{payload}"
    );
}

/// **The three new sentences, verbatim** (16b cut D.4), through the one renderer
/// every surface composes from.
#[test]
fn d4_the_new_sentences_are_these_sentences() {
    use tacsim::arena::perception::{FrameNotice, Named, NoticeKind};
    use tacsim::arena::state::{Classification, Designator, PartyId};

    let frame = fixture::frame();
    let rows = cockpit::frame_contacts(&frame);

    // `B-20`: the regression, under both names.
    let regressed = FrameNotice::Contact {
        kind: NoticeKind::Unidentified,
        contact: Named {
            id: frame.lock.unwrap_or_else(|| panic!("the scene's lock")),
            designator: Designator::Provisional(2),
        },
        class: Classification::Unknown,
        was: Some(Designator::Slot {
            team: PartyId::BLUE,
            slot: 1,
        }),
    };
    assert_eq!(
        cockpit::notice_line(&frame, &regressed, &rows),
        "BA is unidentified again: X2."
    );
    assert_eq!(NoticeKind::Unidentified.as_str(), "unidentified");

    // `B-22`: the pilot's own machine, both ways.
    assert_eq!(
        cockpit::notice_line(&frame, &FrameNotice::Receiver { degraded: true }, &rows),
        "Receiver degraded: something is jamming the sight."
    );
    assert_eq!(
        cockpit::notice_line(&frame, &FrameNotice::Receiver { degraded: false }, &rows),
        "Receiver clear."
    );
    assert_eq!(
        cockpit::RECEIVER_DEGRADED,
        "Receiver degraded: something is jamming the sight."
    );
    assert_eq!(cockpit::RECEIVER_CLEAR, "Receiver clear.");
    assert_eq!(NoticeKind::ReceiverDegraded.as_str(), "receiver_degraded");
    assert_eq!(NoticeKind::ReceiverClear.as_str(), "receiver_clear");

    // **16b cut O.2** (16c N12): the pilot's own pod, both ways — and what
    // switching it off costs, said in the line.
    let on = FrameNotice::Emcon {
        on: true,
        radius_m: 60,
    };
    let off = FrameNotice::Emcon {
        on: false,
        radius_m: 60,
    };
    assert_eq!(
        cockpit::notice_line(&frame, &on, &rows),
        "EMCON on: the pod radiates; every sight inside 60 m is jammed, your own among them."
    );
    assert_eq!(
        cockpit::notice_line(&frame, &off, &rows),
        "EMCON off: the pod is silent; your own sight clears, and nothing hides you."
    );
    assert_eq!(on.kind(), NoticeKind::EmconOn);
    assert_eq!(off.kind(), NoticeKind::EmconOff);
    assert_eq!(NoticeKind::EmconOn.as_str(), "emcon_on");
    assert_eq!(NoticeKind::EmconOff.as_str(), "emcon_off");
    // Neither names a contact, a designator or a section.
    for notice in [&on, &off] {
        assert_eq!(notice.contact(), None);
        assert_eq!(notice.designator(), None);
        assert_eq!(notice.section(), None);
    }
}

/// **`STATUS`'s receiver line and `RANGE`'s step, through the real verbs** (16b
/// cut D.4, `B-22` and `T-20`).
///
/// The line appears only for a platform whose own last pass ran under
/// interference, it sits after the heat line, and it carries the platform's own
/// three integers and nothing whatever about the emitter.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d4_status_prints_the_receiver_and_range_reads_the_plots_own_step() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let base = rig.held_frame(0);
    let scene = fixture::frame();

    // A clean receiver says nothing at all.
    let mut frame = base.clone();
    frame.contacts = scene.contacts.clone();
    frame.lock = scene.lock;
    frame.receiver = None;
    rig.push_frame(0, frame.clone());
    let said = rig.say(0, "status");
    assert!(!said.contains("receiver"), "{said}");

    // A degraded one is one line, after the heat line where there is one.
    frame.receiver = Some(tacsim::arena::perception::Receiver {
        budget: 15,
        budget_max: 40,
        reach_permille: 600,
    });
    rig.push_frame(0, frame.clone());
    let said = rig.say(0, "status");
    assert!(
        said.contains("  receiver degraded: 15 of 40 through cover, 60% reach"),
        "{said}"
    );
    // No source, no bearing, no side, no cell and no object number on it.
    let line = said
        .lines()
        .find(|line| line.contains("receiver degraded"))
        .unwrap_or_default()
        .to_owned();
    for leak in [
        "BA", "RB", "X1", "bearing", "blue", "red", "#", "pod", "jammer",
    ] {
        assert!(
            !line.contains(leak),
            "the receiver line leaked {leak}: {line}"
        );
    }

    // And `RANGE` on the lock reads the step the plot's own age allows. The
    // scene's `BA` is 43,000 cm on a 1,200 ms plot: 20 m of drift, so 440 m.
    let said = rig.say(0, "range ba");
    assert!(said.contains("Range to BA: 440 m"), "{said}");

    // A stale plot of the same contact reads at the scope's own step.
    if let Some(held) = frame.contacts.first_mut() {
        held.age_ms = 4_000;
    }
    rig.push_frame(0, frame);
    let said = rig.say(0, "range ba");
    assert!(said.contains("Range to BA: 450 m"), "{said}");
}

/// **16b cut B.1** (`T-86`): the notice object carries the old name on the two
/// lines that quote one, and the victim's own section on the victim's three
/// kinds — so neither panel has to parse either back out of `text`.
///
/// The stronger pin that replaced cut D.2's "the victim's own section is not the
/// gunner's key": on `struck`, `struck_spent` and `section_lost` the published
/// `section` is one of the pilot's OWN frame's `sections[].id`, and the notice
/// names no contact — no `contact`, no `designator`, no `was` — so it is never a
/// contact's. No reader highlights by this key alone; `kind` says whose it is.
#[test]
fn b1_the_victims_section_and_the_old_name_are_published() {
    use tacsim::arena::perception::{FrameNotice, Named, NoticeKind};
    use tacsim::arena::state::{Classification, ContactId, Designator, PartyId};

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
    let blue_one = Designator::Slot {
        team: PartyId::BLUE,
        slot: 1,
    };
    let hostile = ContactId(0x1111_1111_1111_1111);

    // `X2 identified: BA` — the provisional number it was told apart from.
    let classified = FrameNotice::Contact {
        kind: NoticeKind::Classified,
        contact: Named {
            id: hostile,
            designator: blue_one,
        },
        class: Classification::Classified { party: PartyId::BLUE },
        was: Some(Designator::Provisional(2)),
    };
    assert_eq!(classified.was(), Some(Designator::Provisional(2)));
    let published = object(classified);
    assert_eq!(key(&published, "kind"), serde_json::json!("classified"));
    assert_eq!(key(&published, "designator"), serde_json::json!("BA"));
    assert_eq!(key(&published, "was"), serde_json::json!("X2"));
    assert_eq!(key(&published, "section"), serde_json::Value::Null);

    // `BA unidentified again: X2` — the name a flicker cost it.
    let unidentified = FrameNotice::Contact {
        kind: NoticeKind::Unidentified,
        contact: Named {
            id: hostile,
            designator: Designator::Provisional(2),
        },
        class: Classification::Unknown,
        was: Some(blue_one),
    };
    let published = object(unidentified);
    assert_eq!(key(&published, "kind"), serde_json::json!("unidentified"));
    assert_eq!(key(&published, "designator"), serde_json::json!("X2"));
    assert_eq!(key(&published, "was"), serde_json::json!("BA"));

    // Every notice of the one scene carries the key, `null` where no old name
    // is quoted — present, so a panel can tell "none" from "not sent".
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    let notices = payload
        .get("notices")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(!notices.is_empty());
    for notice in &notices {
        assert_eq!(
            notice.get("was"),
            Some(&serde_json::Value::Null),
            "{notice}"
        );
    }

    // The victim's three: the section is the pilot's own, and nothing names a
    // contact.
    let own: Vec<serde_json::Value> = payload
        .get("sections")
        .and_then(serde_json::Value::as_array)
        .map(|sections| {
            sections
                .iter()
                .filter_map(|section| section.get("id").cloned())
                .collect()
        })
        .unwrap_or_default();
    assert!(!own.is_empty());
    let section = |name: &str| smol_str::SmolStr::new(name);
    for (notice, word, expected) in [
        (
            FrameNotice::Struck {
                section: section("hull"),
                armor_section: section("hull"),
                facet: section("front"),
                armor_left: 55,
                armor_max: 80,
                internals_left: 60,
                internals_max: 60,
            },
            "struck",
            "hull",
        ),
        (
            FrameNotice::StruckSpent {
                section: section("right_track"),
            },
            "struck_spent",
            "right_track",
        ),
        (
            FrameNotice::SectionLost {
                section: section("right_track"),
                consequences: vec![section("immobile")],
            },
            "section_lost",
            "right_track",
        ),
    ] {
        assert_eq!(
            notice.section().map(smol_str::SmolStr::as_str),
            Some(expected)
        );
        assert_eq!(notice.was(), None);
        let published = object(notice);
        assert_eq!(key(&published, "kind"), serde_json::json!(word));
        let said = key(&published, "section");
        assert_eq!(said, serde_json::json!(expected), "{word}");
        assert!(
            own.contains(&said),
            "{word}'s section {said} is not one of the pilot's own sections {own:?}"
        );
        for empty in ["contact", "designator", "was"] {
            assert_eq!(
                key(&published, empty),
                serde_json::Value::Null,
                "{word}.{empty}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 16b cut B.4 — the range memory (`T-60`) and the scan width (`B-26`)
// ---------------------------------------------------------------------------

/// The fixture scene's contact whose designator reads `named`.
fn b4_contact_id(
    frame: &tacsim::arena::step::Frame,
    named: &str,
) -> tacsim::arena::state::ContactId {
    frame
        .contacts
        .iter()
        .find(|held| held.designator.to_string() == named)
        .map(|held| held.id)
        .unwrap_or_else(|| panic!("the fixture scene holds {named}"))
}

/// **16b cut B.4** (`T-60`): the memory the bridge stores on its mirror is one
/// row per contact on the frame, each at the figure its row publishes — held
/// against the previous map — and a contact gone from the frame is gone from
/// the map, so a re-acquired id starts afresh.
#[test]
fn b4_settle_ranges_holds_each_row_and_drops_a_vanished_one() {
    let mut frame = fixture::frame();
    let friend = b4_contact_id(&frame, "RB");
    let hostile = b4_contact_id(&frame, "BA");
    // RB's plot sits at 431 m, past the plain 425 m edge.
    if let Some(held) = frame.contacts.iter_mut().find(|held| held.id == friend) {
        held.range_cm = 43_100;
    }
    // No memory: every row rounds plainly.
    let first = cockpit::settle_ranges(&frame, &BTreeMap::new());
    assert_eq!(first.len(), frame.contacts.len(), "one entry per row");
    assert_eq!(first.get(&friend), Some(&450));
    // A memory of 400 holds RB at 400; BA's 430 m against a 400 held holds too.
    let held = BTreeMap::from([(friend, 400_u64), (hostile, 400_u64)]);
    let second = cockpit::settle_ranges(&frame, &held);
    assert_eq!(second.get(&friend), Some(&400), "held across the edge");
    assert_eq!(second.get(&hostile), Some(&400));
    // And the rows the frame's panel payload publishes are those figures.
    let payload = cockpit::status_frame_held(fixture::MATCH, &frame, &second);
    let published = |named: &str| {
        payload
            .get("contacts")
            .and_then(serde_json::Value::as_array)
            .and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("designator").and_then(serde_json::Value::as_str) == Some(named)
                })
            })
            .and_then(|row| row.get("range_m"))
            .and_then(serde_json::Value::as_u64)
    };
    assert_eq!(published("RB"), Some(400));
    assert_eq!(
        cockpit::status_frame(fixture::MATCH, &frame)
            .get("contacts")
            .and_then(serde_json::Value::as_array)
            .and_then(|rows| {
                rows.iter().find(|row| {
                    row.get("designator").and_then(serde_json::Value::as_str) == Some("RB")
                })
            })
            .and_then(|row| row.get("range_m"))
            .and_then(serde_json::Value::as_u64),
        Some(450),
        "the memoryless form is the old rounding, which the fixtures keep"
    );
    // A contact that vanishes from the frame is dropped from the memory.
    frame.contacts.retain(|held| held.id != friend);
    let third = cockpit::settle_ranges(&frame, &second);
    assert!(!third.contains_key(&friend), "a vanished row is forgotten");
    assert_eq!(third.len(), frame.contacts.len());
}

/// **16b cut B.4** (`T-60`): `RANGE` on an unlocked row prints the figure the
/// row carries — the held one — and does not recompute it from the plot, so
/// `RANGE`, `CONTACTS` and both panels read one distance; the lock alone is
/// recomputed at its own age-derived step.
#[test]
fn b4_range_prints_an_unlocked_rows_held_figure_and_recomputes_only_the_lock() {
    let mut frame = fixture::frame();
    let friend = b4_contact_id(&frame, "RB");
    let hostile = b4_contact_id(&frame, "BA");
    if let Some(held) = frame.contacts.iter_mut().find(|held| held.id == friend) {
        held.range_cm = 43_100;
    }
    let memory = BTreeMap::from([(friend, 400_u64), (hostile, 400_u64)]);
    let rows = cockpit::frame_contacts_held(&frame, &memory);
    let row = |named: &str| {
        rows.iter()
            .find(|row| row.designator.to_string() == named)
            .cloned()
            .unwrap_or_else(|| panic!("{named}'s row"))
    };
    // RB is not the lock: a plot at 431 m with 400 held prints 400 m, which is
    // what `CONTACTS` prints about the same row.
    let rb = row("RB");
    assert_eq!(rb.range_m, 400);
    assert_eq!(
        cockpit::range_line(&frame, &rb, 1_000),
        "Range to RB: 400 m."
    );
    assert!(rb.render().contains("400 m"), "{}", rb.render());
    // BA is the lock: its row holds 400 on the scope, but `RANGE` recomputes
    // from the plot at the lock's own step — 430 m on a 1.2 s plot is 440 at
    // 20 m — the frame's `lock_range_m`, and no memory.
    let ba = row("BA");
    assert_eq!(ba.range_m, 400);
    let line = cockpit::range_line(&frame, &ba, 1_000);
    assert!(line.starts_with("Range to BA: 440 m"), "{line}");
    assert_eq!(
        cockpit::status_frame_held(fixture::MATCH, &frame, &memory)
            .get("lock_range_m")
            .and_then(serde_json::Value::as_u64),
        Some(440)
    );
}

/// **16b cut B.4** (`B-26`): a wrapped `SCAN` run never passes 76 columns, the
/// comma it takes on a wrap included. The two cells here fill a run to exactly
/// 76 before a third arrives — which the old `+ 2` test let stand, and then
/// pushed the wrap's comma onto for 77 — so the run must wrap one cell earlier.
#[test]
fn b4_a_scan_run_that_would_reach_77_wraps_at_76_or_less() {
    use smol_str::SmolStr;
    let mut frame = fixture::frame();
    // `ggyrd`: two 30-character names reading `green` make cells of 36 each,
    // and `  ` + 36 + `, ` + 36 is 76 exactly.
    let long = |n: u8| SmolStr::new(format!("{:a<29}{n}", ""));
    frame.scan_tables = vec![vec![
        long(1),
        long(2),
        SmolStr::new_static("left_track"),
        SmolStr::new_static("right_track"),
        SmolStr::new_static("turret"),
    ]];
    assert_eq!(long(1).len(), 30);
    let rows = cockpit::frame_contacts(&frame);
    let ba = rows
        .iter()
        .find(|row| row.designator.to_string() == "BA")
        .cloned()
        .expect("BA's row");
    let lines = cockpit::scan_lines(&frame, &ba);
    for run in lines.iter().skip(1) {
        assert!(
            run.len() <= 76,
            "a wrapped run ran past 76 columns: {} ({run:?})",
            run.len()
        );
        assert!(run.starts_with("  "), "{run:?}");
    }
    assert_eq!(
        lines.get(1).map(String::as_str),
        Some(format!("  {} green,", long(1)).as_str()),
        "the second cell no longer fits beside the first once the comma is counted"
    );
    // Every section is named once.
    let joined = lines.join(" ");
    for name in ["left_track", "right_track", "turret"] {
        assert_eq!(joined.matches(name).count(), 1, "{joined}");
    }
}

/// The arena-cockpit panel payloads in `events`.
fn b4_cockpit_panels(events: &[(Audience, OutputEvent)]) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter_map(|(_, event)| match event {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::arena_bridge::FRAME_FEED =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

/// **16b cut B.4** (`B-23`): `VIEW` with a frame held but no ground to draw it
/// on says so in one sentence and pushes nothing — it used to push the bare
/// status frame, which declares no shape and a line client prints as a blank.
///
/// The frame here is the arena's own, stood off the terrain and handed back
/// through the real egress seam, which is one of the two ways
/// `view_frame_held` answers a bare frame (the other is no arena in the
/// registry; both reach the one inline shape test). The no-frame sentence and
/// the drawn window either side of it are pinned too, so the test fails if the
/// shape test is dropped (a blank panel again) or inverted (a dark sentence on
/// a window that could be drawn).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b4_view_says_the_window_is_dark_when_there_is_no_ground_under_it() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);

    // No frame yet: the sentence that always was, and nothing pushed.
    let events = rig.events(0, "view");
    assert_eq!(
        rig.tw.seen_by(&events, pilot),
        vec!["The window is dark: the panel has told you nothing yet.".to_owned()]
    );
    assert!(b4_cockpit_panels(&events).is_empty(), "{events:?}");

    // A frame on the ground: the window is drawn, and the dark sentence is not
    // said.
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = rig.held_frame(0);
    let events = rig.events(0, "view");
    let panels = b4_cockpit_panels(&events);
    assert_eq!(panels.len(), 1, "{events:?}");
    assert_eq!(
        panels
            .first()
            .and_then(|payload| payload.pointer("/schema/shape")),
        Some(&serde_json::json!("grid"))
    );
    assert!(
        !rig.tw
            .seen_by(&events, pilot)
            .iter()
            .any(|said| said.contains("window is dark")),
        "{events:?}"
    );

    // The same frame stood off the terrain: one sentence, no panel of any shape.
    let mut off = frame;
    off.pose.x_cm = -1;
    rig.push_frame(0, off);
    let events = rig.events(0, "view");
    assert_eq!(
        rig.tw.seen_by(&events, pilot),
        vec!["The window is dark: there is no ground under the panel to draw.".to_owned()]
    );
    assert!(
        b4_cockpit_panels(&events).is_empty(),
        "nothing is pushed when nothing can be drawn: {events:?}"
    );
}

/// **16b cut B.4** (`T-60`), through the bridge: two successive frames handed to
/// `on_frames` over the real egress seam. The second plot crosses the plain
/// 425 m edge by six metres, and the panel the pulse publishes, `CONTACTS` and
/// `RANGE` off the mirror all keep the held 400 m; a plot past the hysteresis
/// band moves all three to 450 together.
///
/// It is what the pure `settle_ranges` tests cannot see: that `on_frames` reads
/// the previous mirror's memory, stores the settled map on the new one, and
/// publishes with it, and that the read verbs render the mirror's memory rather
/// than rounding afresh.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b4_the_bridge_holds_a_range_across_the_edge_from_pulse_to_pulse() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let base = rig.held_frame(0);
    let scene = fixture::frame();
    let friend = b4_contact_id(&scene, "RB");
    let mut frame = base;
    frame.contacts = scene.contacts.clone();
    frame.lock = scene.lock;

    let published_rb = |rig: &Rig| {
        rig.panels(tacsim::arena_bridge::FRAME_FEED)
            .last()
            .and_then(|(_, payload)| {
                payload
                    .get("contacts")
                    .and_then(serde_json::Value::as_array)
                    .and_then(|rows| {
                        rows.iter().find(|row| {
                            row.get("designator").and_then(serde_json::Value::as_str) == Some("RB")
                        })
                    })
                    .and_then(|row| row.get("range_m"))
                    .and_then(serde_json::Value::as_u64)
            })
    };
    let contacts_rb = |said: &str| {
        said.lines()
            .find(|line| line.contains("RB"))
            .unwrap_or_default()
            .to_owned()
    };
    let at = |mut frame: tacsim::arena::step::Frame, range_cm: u64| {
        if let Some(held) = frame.contacts.iter_mut().find(|held| held.id == friend) {
            held.range_cm = range_cm;
        }
        frame
    };

    // 424 m: 400 on every surface.
    rig.push_frame(0, at(frame.clone(), 42_400));
    assert_eq!(published_rb(&rig), Some(400));
    // 431 m, past the plain 425 m edge but inside the band: still 400, where
    // the memoryless rounding says 450.
    rig.push_frame(0, at(frame.clone(), 43_100));
    assert_eq!(
        published_rb(&rig),
        Some(400),
        "the pulse's panel holds the figure it last published"
    );
    let said = rig.say(0, "contacts");
    assert!(contacts_rb(&said).contains("400 m"), "{said}");
    let said = rig.say(0, "range rb");
    assert!(said.contains("Range to RB: 400 m."), "{said}");
    // 432 m is past the six-metre band: 450 on every surface together.
    rig.push_frame(0, at(frame.clone(), 43_200));
    assert_eq!(published_rb(&rig), Some(450));
    let said = rig.say(0, "contacts");
    assert!(contacts_rb(&said).contains("450 m"), "{said}");
    let said = rig.say(0, "range rb");
    assert!(said.contains("Range to RB: 450 m."), "{said}");
    // And held there on the way back down to 419 m, which rounds plainly to 400.
    rig.push_frame(0, at(frame, 41_900));
    assert_eq!(published_rb(&rig), Some(450));
}

/// **16b cut B.4** (`B-38`, the ejection half): the two ejections show the
/// pilot the room they are put out into, after the hatch line that explains
/// it — `SIM LEAVE` by a pilot seated during `Boarding` (`eject_pilot`) and the
/// centre-wide sweep (`eject_all`, here run by the end of an aborted match;
/// the staff reset runs the same sweep).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b4_an_ejected_pilot_is_shown_the_room_after_the_hatch_line() {
    let hatch_then_room = |events: &[(Audience, OutputEvent)], pilot: ObjId, room: ObjId| {
        let hatch = events.iter().position(|(audience, event)| {
            *audience == Audience::Actor(pilot)
                && matches!(event, OutputEvent::Sys(text)
                    if text.to_plain_text().starts_with("The hatch of "))
        });
        let view = events.iter().position(|(audience, event)| {
            *audience == Audience::Actor(pilot)
                && matches!(event, OutputEvent::Room(view) if view.id == room.to_string())
        });
        match (hatch, view) {
            (Some(hatch), Some(view)) => hatch < view,
            _ => false,
        }
    };
    let room_of = |rig: &Rig, pod: ObjId| {
        rig.tw
            .world()
            .get(pod)
            .and_then(|object| object.location)
            .expect("the pod stands in the team room")
    };

    // `SIM LEAVE` from inside the pod, while the centre is `Boarding` (the one
    // state that verb and a seat meet in).
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.say(0, "board");
    let pod = rig.pod(0).expect("a locked pod");
    assert_eq!(rig.location(0), Some(pod), "aboard");
    let room = room_of(&rig, pod);
    let events = rig.events(0, "sim leave");
    assert_eq!(rig.location(0), Some(room), "the hatch opened");
    assert!(
        hatch_then_room(&events, rig.pilot(0), room),
        "the team room after the hatch line: {events:?}"
    );

    // The sweep, for both pilots of a match that ends.
    rig.staff("@arena/reset");
    assert_eq!(rig.state(), MatchState::Idle);
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    rig.fly().await;
    let rooms = [
        room_of(&rig, rig.pod(0).expect("a locked pod")),
        room_of(&rig, rig.pod(1).expect("a locked pod")),
    ];
    // A running match ends through the arena (the check-in lands a pulse
    // later), so the sweep's events are read off the rig's whole record from
    // the mark on.
    let mark = rig.log.len();
    rig.say(0, "sim abort");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    let events = rig.log.split_off(mark);
    for (n, room) in rooms.into_iter().enumerate() {
        assert_eq!(rig.location(n), Some(room), "pilot {n}'s hatch opened");
        assert!(
            hatch_then_room(&events, rig.pilot(n), room),
            "pilot {n}'s team room after the hatch line: {events:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// 16b cut V.1: the named ground on the read verbs
// ---------------------------------------------------------------------------

/// The shipped proving ground's spots, as the running match's definition
/// carries them.
fn shipped_spots(rig: &Rig) -> (Vec<tacsim::arena::state::KeySpot>, u16) {
    let registry = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .expect("the arena registry is composed");
    let match_id = rig.centre().match_id.expect("a running match");
    let ground = registry
        .arena_of(match_id)
        .expect("the running match's ground");
    (ground.key_spots.clone(), ground.terrain.cell_cm())
}

/// **16b cut V.1: `RANGE`, `BEARING` and `STATUS` name the ground**, over the
/// real composition and the shipped proving ground.
///
/// The red pilot's tank opens on (6, 97), the ford on (64, 97): fifty-eight
/// cells due east on the same road, 580 m at 090°. North Barn on (106, 194) is
/// 100 cells east and 97 north, `isqrt(100,000² + 97,000²)` = 139,316 cm (1,390
/// m) at `atan2(100, 97)` = 45.87°, so `045°`, and off a nose pointing east
/// (090°) that is `rel 315°`. The numbers are computed from the pilot's own
/// frame through the pure helpers as well, so the test pins both the arithmetic
/// and the sentence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_range_bearing_and_status_name_the_shipped_ground() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = rig.held_frame(0);
    let (spots, cell_cm) = shipped_spots(&rig);
    assert_eq!(
        spots
            .iter()
            .map(|spot| spot.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "The Ford",
            "The Crossroads",
            "North Barn",
            "East Compound",
            "The Bluff"
        ],
        "the shipped five, in authored order (the fifth is 16b cut AE.1's)"
    );
    assert_eq!(frame.cell, Some((6, 97)), "red slot 1 musters on the road");

    // Each answer is the sentence, then the frame it read re-pushed (which the
    // rig prints as `[panel arena.cockpit]`): the sentence is the first line.
    let first = |said: String| said.lines().next().unwrap_or_default().to_owned();
    let said = first(rig.say(0, "RANGE The Ford"));
    assert_eq!(said, "Range to The Ford (A): 580 m.");
    assert_eq!(
        first(rig.say(0, "range a")),
        said,
        "the map letter is the same spot"
    );
    assert_eq!(
        first(rig.say(0, "range THE FORD")),
        said,
        "and case does not matter"
    );
    let ford = spots.first().expect("the ford");
    assert_eq!(
        cockpit::range_m_rounded(
            cockpit::spot_range_cm(&frame.pose, ford, cell_cm),
            cockpit::SPOT_RANGE_STEP_M
        ),
        580
    );

    let said = first(rig.say(0, "BEARING north barn"));
    assert_eq!(said, "Bearing to North Barn (C): 045° (rel 315°).");
    let barn = spots.get(2).expect("the barn");
    assert_eq!(
        cockpit::spot_bearing_mdeg(&frame.pose, barn, cell_cm) / 1_000,
        45
    );

    let status = rig.say(0, "status");
    assert!(
        status.contains("  nearest spot: The Ford (A), 580 m at 090°"),
        "{status}"
    );
    // After the position line and before anything else.
    let lines: Vec<&str> = status.lines().collect();
    let at = lines
        .iter()
        .position(|line| line.starts_with("  at "))
        .expect("the position line");
    assert_eq!(
        lines.get(at + 1).copied(),
        Some("  nearest spot: The Ford (A), 580 m at 090°"),
        "{status}"
    );
}

/// **16b cut V.1: a word that is neither a held contact nor a spot is the one
/// refusal**, and `SCAN` and `LOCK` never take a spot.
///
/// `RANGE ZZ` (a designator of nothing), `RANGE The Fjord` (a spot that does not
/// exist) and `RANGE BA` (a tank out there and undetected) are byte-identical to
/// each other and to `LOCK ZZ`, so the second resolution arm tells a pilot
/// nothing the first did not. `SCAN The Ford` and `LOCK The Ford` are refused
/// exactly as they always were: ground is not a contact.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_a_word_that_names_nothing_is_still_the_one_refusal() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let lock = rig.say(0, "LOCK ZZ");
    assert_eq!(lock, cockpit::NO_SUCH_CONTACT);
    for line in [
        "RANGE ZZ",
        "RANGE The Fjord",
        "RANGE BA",
        "BEARING The Fjord",
        "BEARING BA",
        "RANGE the",
        "RANGE d",
        "SCAN The Ford",
        "LOCK The Ford",
        "SCAN a",
        "LOCK a",
    ] {
        assert_eq!(
            rig.say(0, line),
            lock,
            "{line} answered something other than the one refusal"
        );
    }
}

/// **16b cut V.1: a spot named before the arena has spoken answers
/// `NO_FRAME_YET`**, and a contact word still answers the one refusal.
///
/// Straight after the second `BOARD` the match is running and nothing has been
/// pumped, so the world holds no frame for the pilot: the spot is real (the
/// ground is surveyed) and the pilot's own position is what is missing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_a_spot_named_before_the_first_frame_says_the_panel_is_dark() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame(0).is_none(), "nothing has been pumped yet");
    assert_eq!(rig.say(0, "RANGE The Ford"), cockpit::NO_FRAME_YET);
    assert_eq!(rig.say(0, "BEARING a"), cockpit::NO_FRAME_YET);
    assert_eq!(rig.say(0, "RANGE ZZ"), cockpit::NO_SUCH_CONTACT);
    assert_eq!(rig.say(0, "RANGE BA"), cockpit::NO_SUCH_CONTACT);
}

/// **16b cut V.1: `VIEW` at the muster names the ground off its window**, after
/// the panel, and says nothing about any unit.
///
/// The red muster's 39 by 20 window is columns 0 to 38 and rows 87 to 106, and
/// all five shipped spots are east of it, so all five are listed, in authored
/// order, each with the range and bearing from the pilot's own pose.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_view_at_the_muster_names_every_spot_off_its_window() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = rig.held_frame(0);
    let (spots, cell_cm) = shipped_spots(&rig);
    let events = rig.events(0, "view");
    let view = Rig::panel(&events).expect("VIEW pushes the window");
    // The window drew no spot, and its own units are the pilot alone.
    let units = view
        .get("units")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        units.len(),
        1,
        "the pilot's own marker and nothing else: {view}"
    );
    let said = rig.tw.seen_by(&events, rig.pilot(0)).join("\n");
    let expected = cockpit::off_window_spots_line(&spots, &[], &frame.pose, cell_cm)
        .expect("five spots, none drawn");
    // 16b cut AE.1: The Bluff (144, 111) is 138 cells east and 14 north of the
    // muster's (6, 97): `isqrt` 138,708 cm (1,390 m to the ten) at
    // `atan2(138, 14)` = 84.2°, so 084°.
    assert_eq!(
        expected,
        "Off the window: A The Ford, 580 m at 090°; B The Crossroads, 1230 m at 090°; \
         C North Barn, 1390 m at 045°; E East Compound, 2080 m at 075°; \
         F The Bluff, 1390 m at 084°."
    );
    assert!(said.contains(&expected), "{said}");
    // Nothing about any unit: not the other tank's name, pilot or callsign.
    for leak in ["Blue Tank", "Pilot1", "BA"] {
        assert!(
            !expected.contains(leak) && !said.contains(leak),
            "VIEW leaked {leak:?}: {said}"
        );
    }
}

/// **16b cut V.1: a spot inside the window is drawn and is not listed off it**,
/// over the real composition.
///
/// The red tank is stood on (60, 97) while the centre is `Boarding`, four cells
/// west of The Ford (64, 97), so `VIEW`'s window holds the ford and none of the
/// other four. What that proves, which the muster test cannot (its window
/// draws no spot at all):
///
/// * the live panel's first marker is the `A` spot and the pilot's own marker —
///   found by its flag — follows it and carries the tank's display name, so
///   `view_frame_held` names the pilot and not the ford;
/// * the off-window line is read against the drawing's own spots: it names B, C,
///   E and (since 16b cut AE.1) F, and not A. From (60,500, 97,500) cm: The
///   Crossroads (129, 97) is
///   69,000 cm due east, 690 m at 090°; North Barn (106, 194) is dx 46,000,
///   dy 97,000, `isqrt` 107,354 cm (1,070 m) at `atan2(46, 97)` = 25.4°, so
///   025°; East Compound (208, 148) is dx 148,000, dy 51,000, `isqrt` 156,541
///   cm (1,570 m) at `atan2(148, 51)` = 71.0°, so 070°.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v1_view_beside_the_ford_draws_it_and_names_only_the_others_off_the_window() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.place(rig.tank(0), (60, 97), 90_000);
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = rig.held_frame(0);
    let (spots, cell_cm) = shipped_spots(&rig);
    let name = mush_core::output::display_name(rig.tw.world(), frame.unit);

    let events = rig.events(0, "view");
    let view = Rig::panel(&events).expect("VIEW pushes the window");
    let units = view
        .get("units")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let spot_ids: Vec<&str> = units
        .iter()
        .filter(|unit| unit.get("spot").and_then(serde_json::Value::as_bool) == Some(true))
        .filter_map(|unit| unit.get("id").and_then(serde_json::Value::as_str))
        .collect();
    assert_eq!(
        spot_ids,
        vec!["A"],
        "the window draws the ford alone: {view}"
    );
    assert_eq!(
        units
            .first()
            .and_then(|first| first.get("name"))
            .and_then(serde_json::Value::as_str),
        Some("The Ford"),
        "the spot is drawn first, under its own name: {view}"
    );
    let you = own_marker(&view).expect("the pilot's own marker");
    assert_eq!(
        units.iter().position(|unit| unit == &you),
        Some(1),
        "the own marker follows the spot"
    );
    assert_eq!(
        you.get("name").and_then(serde_json::Value::as_str),
        Some(name.as_str()),
        "the tank's name is on the pilot's own marker, not on the ford: {view}"
    );

    let said = rig.tw.seen_by(&events, rig.pilot(0)).join("\n");
    let expected = cockpit::off_window_spots_line(&spots, &[0], &frame.pose, cell_cm)
        .expect("five spots undrawn");
    // 16b cut AE.1: The Bluff (144, 111) is dx 84,000, dy 14,000 from (60, 97):
    // `isqrt` 85,158 cm (850 m) at `atan2(84, 14)` = 80.5°, so 080°.
    assert_eq!(
        expected,
        "Off the window: B The Crossroads, 690 m at 090°; C North Barn, 1070 m at 025°; \
         E East Compound, 1570 m at 070°; F The Bluff, 850 m at 080°."
    );
    // The one off-window line `VIEW` said is that sentence exactly: the drawn
    // ford is not in it.
    let off: Vec<&str> = said
        .lines()
        .filter(|line| line.starts_with("Off the window:"))
        .collect();
    assert_eq!(
        off,
        vec![expected.as_str()],
        "a drawn spot is never listed off the window: {said}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut AG.1: `VIEW BRIEF`, the window in sentences
// ---------------------------------------------------------------------------

/// The lines a command said to that pilot, without the panel frames the rig
/// prints as `[panel …]`.
fn ag1_said(rig: &Rig, events: &[(Audience, OutputEvent)]) -> Vec<String> {
    rig.tw
        .seen_by(events, rig.pilot(0))
        .into_iter()
        .flat_map(|said| said.lines().map(str::to_owned).collect::<Vec<String>>())
        .filter(|line| !line.starts_with("[panel "))
        .collect()
}

/// Whether any panel in a command's events is the grid (`schema.shape`).
fn ag1_pushed_a_grid(events: &[(Audience, OutputEvent)]) -> bool {
    events.iter().any(|(_, event)| match event {
        OutputEvent::Panel { data, .. } => {
            data.pointer("/schema/shape")
                .and_then(serde_json::Value::as_str)
                == Some("grid")
        }
        _ => false,
    })
}

/// **16b cut AG.1: `VIEW BRIEF` at the muster** — the header naming the
/// pilot's own callsign, the eight bearings in order, `No contacts in the
/// window.` (16b cut AP.4; the blue tank is 2,430 m away against a 600 m
/// sight), no in-window spot (the red window draws none of the five), the
/// off-window line `VIEW` itself says and `End of brief.`; then the status
/// keyframe re-pushed and no grid, and nothing about any unit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ag1_view_brief_at_the_muster_is_the_window_in_sentences() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = rig.held_frame(0);
    let (spots, cell_cm) = shipped_spots(&rig);
    let events = rig.events(0, "view brief");
    let said = ag1_said(&rig, &events);
    let joined = said.join("\n");
    let line = |n: usize| said.get(n).map_or("", String::as_str);
    assert!(
        line(0).starts_with(&format!("Window around {}:", frame.callsign)),
        "{joined}"
    );
    for (n, (bearing, _, _)) in cockpit::BRIEF_BEARINGS.iter().enumerate() {
        assert!(
            line(n + 1).starts_with(&format!("{bearing}:")),
            "line {} is not {bearing}: {joined}",
            n + 1
        );
    }
    assert_eq!(line(9), "No contacts in the window.", "{joined}");
    assert!(
        !said.iter().any(|line| line.starts_with("In the window:")),
        "the muster window names no spot: {joined}"
    );
    let off = cockpit::off_window_spots_line(&spots, &[], &frame.pose, cell_cm)
        .expect("the shipped spots, none drawn");
    assert_eq!(line(10), off, "{joined}");
    assert_eq!(line(11), "End of brief.", "{joined}");
    assert_eq!(said.len(), 12, "nothing else is said: {joined}");
    // The status keyframe, as `CONTACTS` re-pushes it; never the grid.
    let panel = Rig::panel(&events).expect("the brief re-pushes the status keyframe");
    assert!(
        panel.pointer("/schema/shape").is_none(),
        "the re-pushed frame is the status keyframe: {panel}"
    );
    assert!(!ag1_pushed_a_grid(&events), "the brief pushes no grid");
    // Nothing about any unit: not the other tank's name, pilot or callsign.
    for leak in ["Blue Tank", "Pilot1", "BA"] {
        assert!(
            !joined.contains(leak),
            "VIEW BRIEF leaked {leak:?}: {joined}"
        );
    }
}

/// **16b cut AG.1: beside the ford, the brief names it in the window** —
/// the `v1_view_beside_the_ford_…` pose, four cells west of The Ford on
/// (64, 97): 4,000 cm due east, `40 m at 090°` — and the other four off it,
/// exactly as `VIEW`'s own off-window line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ag1_view_brief_beside_the_ford_names_it_in_the_window() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.place(rig.tank(0), (60, 97), 90_000);
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = rig.held_frame(0);
    let (spots, cell_cm) = shipped_spots(&rig);
    let events = rig.events(0, "VIEW BRIEF");
    let said = ag1_said(&rig, &events);
    let joined = said.join("\n");
    let in_window: Vec<&str> = said
        .iter()
        .map(String::as_str)
        .filter(|line| line.starts_with("In the window:"))
        .collect();
    assert_eq!(
        in_window,
        vec!["In the window: A The Ford, 40 m at 090°."],
        "{joined}"
    );
    let expected = cockpit::off_window_spots_line(&spots, &[0], &frame.pose, cell_cm)
        .expect("four spots undrawn");
    let off: Vec<&str> = said
        .iter()
        .map(String::as_str)
        .filter(|line| line.starts_with("Off the window:"))
        .collect();
    assert_eq!(off, vec![expected.as_str()], "{joined}");
    assert!(!ag1_pushed_a_grid(&events), "the brief pushes no grid");
}

/// **16b cut AG.1: a second word on `VIEW` is one refusal** — a word of
/// nothing, the slot designator of the tank out there (undetected), a
/// provisional number, a designator of nothing and sixteen digits each answer
/// exactly the usage line and push nothing — and a bare `VIEW` still draws.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ag1_a_second_word_on_view_is_refused_and_bare_view_still_draws() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    for line in [
        "view nonsense",
        "view ba",
        "view x1",
        "view zz",
        "view ffffffffffffffff",
    ] {
        let events = rig.events(0, line);
        assert_eq!(
            rig.tw.seen_by(&events, rig.pilot(0)).join("\n"),
            "VIEW [BRIEF]; BRIEF says the window in sentences.",
            "{line}"
        );
        assert!(Rig::panel(&events).is_none(), "{line} pushed a frame");
    }
    assert_eq!(
        cockpit::VIEW_USAGE,
        "VIEW [BRIEF]; BRIEF says the window in sentences."
    );
    let events = rig.events(0, "view");
    assert!(
        ag1_pushed_a_grid(&events),
        "a bare VIEW still draws the map"
    );
}

/// **16b cut AG.1: before the first frame the brief is the dark window** —
/// `VIEW`'s own line, and nothing else (no frame to re-push).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ag1_view_brief_before_the_first_frame_says_the_window_is_dark() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame(0).is_none(), "nothing has been pumped yet");
    let events = rig.events(0, "view brief");
    assert_eq!(
        rig.tw.seen_by(&events, rig.pilot(0)),
        vec!["The window is dark: the panel has told you nothing yet.".to_owned()]
    );
    assert!(Rig::panel(&events).is_none(), "nothing is pushed");
}

// ---------------------------------------------------------------------------
// 16b cut Y.1 — `ORDER`
// ---------------------------------------------------------------------------

/// The tail every acknowledgement ends with.
const Y1_TAIL: &str = " It stands for 10 minutes or until your next order to it.";

/// [`config`] with the AI fill on, so a side has machines to order.
fn y1_config() -> TacsimConfig {
    let mut cfg = config();
    cfg.match_.ai_fill = true;
    cfg
}

/// **16b cut Y.1**: the side's lead orders the side's machines, and every
/// refusal is one line.
///
/// `SIM AI 3` with one pilot a side fields two machines on each, in one squad
/// (`red-1`, `blue-1`): a side of at most `[tacsim.arena].squad_size` machines —
/// four at the shipped value — is one squad by 16b cut Z.1's dealing. The red
/// lead's `ORDER red-1 HOLD` acknowledges in the
/// doctrine's words and `@arena/status` counts it; the ordinal and `ALL` reach
/// the same squad; `blue-1` (the other side's), `red-9` (nothing) and `2` (the
/// own side's second squad, which does not exist) all answer one byte-identical
/// `No such squad.`; a target or a spot the panel does not know is byte for
/// byte `LOCK ZZ`'s refusal; a known spot is named with its letter; and a bare
/// or unknown objective is the usage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y1_the_lead_orders_the_sides_machines_and_every_squad_refusal_is_one_line() {
    let mut rig = Rig::boot_with(2, y1_config());
    rig.fly_with_ai(1, 1, 3).await;
    let squads: Vec<String> = rig
        .record()
        .squads
        .iter()
        .map(|roster| roster.id.to_string())
        .collect();
    assert_eq!(squads, vec!["red-1", "blue-1"], "one squad of two a side");

    let said = rig.say(0, "ORDER red-1 HOLD");
    assert_eq!(
        said,
        format!("Ordered red-1 to hold: it stops where it stands and fires nothing.{Y1_TAIL}")
    );
    let status = rig.staff("@arena/status");
    assert!(
        status.contains("  orders: 1 by pilots, 0 by lance leaders"),
        "{status}"
    );

    let said = rig.say(0, "ORDER 1 WITHDRAW HOLD FIRE");
    assert_eq!(
        said,
        format!(
            "Ordered red-1 to withdraw, holding fire: it breaks for its entry zone and fires \
             nothing.{Y1_TAIL}"
        )
    );
    let said = rig.say(0, "order all engage");
    assert_eq!(
        said,
        format!("Ordered red-1 to engage: it closes and fires on what it finds.{Y1_TAIL}")
    );

    // One sentence for every squad the pilot may not order.
    for line in ["ORDER blue-1 HOLD", "ORDER red-9 HOLD", "ORDER 2 HOLD"] {
        assert_eq!(rig.say(0, line), cockpit::NO_SUCH_SQUAD, "{line}");
    }

    // A target or a spot the panel does not know: `LOCK ZZ`'s refusal, byte for
    // byte.
    let lock = rig.say(0, "LOCK ZZ");
    assert_eq!(lock, cockpit::NO_SUCH_CONTACT);
    for line in [
        "ORDER red-1 ENGAGE ZZ",
        "ORDER red-1 ENGAGE BA",
        "ORDER red-1 ADVANCE The Fjord",
    ] {
        assert_eq!(rig.say(0, line), lock, "{line}");
    }
    let said = rig.say(0, "ORDER red-1 ADVANCE The Ford");
    assert_eq!(
        said,
        format!(
            "Ordered red-1 to advance on The Ford (A): it drives there and fights nothing on \
             the way.{Y1_TAIL}"
        )
    );
    for line in ["ORDER red-1", "ORDER red-1 DANCE", "ORDER"] {
        assert_eq!(rig.say(0, line), cockpit::ORDER_USAGE, "{line}");
    }
    let status = rig.staff("@arena/status");
    assert!(
        status.contains("  orders: 4 by pilots, 0 by lance leaders"),
        "every order that reached a squad is counted, and no refusal is: {status}"
    );
}

/// **16b cut Z.1 (16c N6): the squad size reaches the lobby's grouping.**
///
/// The Y.1 lead test's field at `[tacsim.arena].squad_size = 1`: `SIM AI 3` with
/// one pilot a side puts two machines on each, and at one machine a squad they
/// are `red-1` and `red-2`, so the lead's `ORDER ALL HOLD` names both squads in
/// one acknowledgement and the tape carries two `ordered` lines, one per squad —
/// the record's grouping and the arena's read the same key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z1_a_squad_size_of_one_makes_the_leads_all_two_squads() {
    let mut cfg = y1_config();
    cfg.arena.squad_size = 1;
    let mut rig = Rig::boot_with(2, cfg);
    rig.fly_with_ai(1, 1, 3).await;
    let squads: Vec<String> = rig
        .record()
        .squads
        .iter()
        .map(|roster| roster.id.to_string())
        .collect();
    assert_eq!(
        squads,
        vec!["red-1", "red-2", "blue-1", "blue-2"],
        "one machine a squad"
    );

    let said = rig.say(0, "ORDER ALL HOLD");
    assert_eq!(
        said,
        format!(
            "Ordered red-1 and red-2 to hold: it stops where it stands and fires nothing.{Y1_TAIL}"
        )
    );
    let issuer = rig.tank(0);
    let holds = |event: &tacsim::arena::ledger::LedgerEvent| {
        y1_ordered_by(event, issuer).is_some_and(|(_, _, objective, _)| objective == "hold")
    };
    assert_eq!(rig.tape_count_until(holds, 2).await, 2);
    let mut reached: Vec<String> = rig
        .tape
        .iter()
        .filter_map(|event| y1_ordered_by(event, issuer))
        .map(|(_, squad, _, _)| squad)
        .collect();
    reached.sort();
    assert_eq!(
        reached,
        vec!["red-1".to_string(), "red-2".to_string()],
        "two ordered lines, one per squad"
    );
}

/// **16b cut Y.1 (16c Q38, the cut Y plan's decision 2)**: anyone but the lead
/// orders one tank — their pod pair's, when a machine flies it.
///
/// Three red pilots in slots 1–3 and `SIM AI 4`: red's one machine flies pod 4
/// (`RD`) and is `red-1` on its own. Slot 3's pilot (pair 4) is answered by the
/// wingman's callsign and the tape's `ordered` line names that one unit; slot
/// 2's pilot (pair 1, a human) has no wingman and reads `No such squad.` for
/// `red-1` and `You have no squad to order.` for `ALL`; and the lead still
/// orders the squad by its name.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y1_a_pilot_orders_their_pod_pair_and_nothing_else() {
    let mut rig = Rig::boot_with(4, y1_config());
    rig.fly_with_ai(3, 1, 4).await;
    let record = rig.record();
    let wingman = record
        .callsigns
        .iter()
        .find(|(_, callsign)| callsign.to_string() == "RD")
        .map(|(unit, _)| *unit)
        .expect("red pod 4 is flown by a machine");
    let red: Vec<&tacsim::deliberation::SquadRoster> = record
        .squads
        .iter()
        .filter(|roster| roster.id.team == PartyId::RED)
        .collect();
    assert_eq!(red.len(), 1);
    assert_eq!(red[0].units, vec![wingman], "red's one machine is red-1");

    let said = rig.say(2, "ORDER red-1 HOLD");
    assert_eq!(
        said,
        format!("Ordered RD to hold: it stops where it stands and fires nothing.{Y1_TAIL}")
    );
    let issuer = rig.tank(2);
    let from_slot_three = |event: &tacsim::arena::ledger::LedgerEvent| {
        matches!(
            &event.kind,
            tacsim::arena::ledger::EventKind::Ordered { issuer: Some(by), .. } if *by == issuer
        )
    };
    assert!(
        rig.tape_until(from_slot_three).await,
        "the order was never journalled"
    );
    let lines: Vec<ObjId> = rig
        .tape
        .iter()
        .filter_map(|event| match &event.kind {
            tacsim::arena::ledger::EventKind::Ordered {
                unit,
                issuer: Some(by),
                squad,
                objective,
                ..
            } if *by == issuer => {
                assert_eq!(squad.as_str(), "red-1");
                assert_eq!(objective.as_str(), "hold");
                Some(*unit)
            }
            _ => None,
        })
        .collect();
    assert_eq!(lines, vec![wingman], "one line, on the wingman alone");
    assert_eq!(rig.say(2, "ORDER blue-1 HOLD"), cockpit::NO_SUCH_SQUAD);

    // Slot 2's pair is slot 1, a human.
    assert_eq!(rig.say(1, "ORDER red-1 HOLD"), cockpit::NO_SUCH_SQUAD);
    assert_eq!(rig.say(1, "ORDER ALL HOLD"), cockpit::NO_SQUAD_TO_ORDER);

    // The lead orders the squad by its name.
    let said = rig.say(0, "ORDER red-1 HOLD");
    assert_eq!(
        said,
        format!("Ordered red-1 to hold: it stops where it stands and fires nothing.{Y1_TAIL}")
    );
}

/// **16b cut Y.1 — `authorise_seat`, pinned for the first time.** Steps (1)–(4)
/// of `forward_intent` moved into one function `ORDER` shares, and no test
/// named a refusal arm before. From the seat, `HEADING 90` still steers and
/// `forward_order` is admitted; with the pilot climbed out into the team room,
/// both `forward_intent` and `forward_order` answer `NotFocused` and offer
/// nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y1_authorise_seat_refuses_a_pilot_out_of_the_seat_and_the_seat_still_steers() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the arena brings power online");
    let said = rig.say(0, "heading 90");
    assert!(said.contains("Steering to 90"), "{said}");

    let tank = rig.tank(0);
    let pilot = rig.pilot(0);
    let told = |issuer: ObjId| tacsim::deliberation::Order {
        squad: tacsim::deliberation::SquadId {
            team: PartyId::RED,
            index: 1,
        },
        objective: tacsim::deliberation::Objective::Hold,
        target: None,
        horizon_ms: 600_000,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer: Some(issuer),
        // 17b cut L.1: the seat's character, as the cockpit's own `ORDER` writes.
        by: tacsim::deliberation::CharacterRef::Character { id: pilot },
        spot: None,
    };
    // From the seat: authorised, and nothing to offer to (this match fields no
    // machine).
    let seated = rig.with_ctx_mut(0, |ctx| {
        tacsim::arena_bridge::forward_order(ctx, pilot, tank, &[], told(tank))
    });
    assert_eq!(seated, Ok(0));

    // Stand the pilot in the team room. Through `EXIT`, the one route that
    // gives the seat and its focus back: a body merely moved out of the pod
    // (`mush_sim::unplace` and a `move_to`) keeps its focus, and the world's
    // four checks still pass for it. The tank stays in the match, so step 1
    // still finds it and step 2 is the one that answers.
    let said = rig.say(0, "exit");
    assert!(!said.is_empty(), "EXIT answers");
    let room = match_::centre_object(rig.tw.world(), match_::RED_ROOM_ENTRY).expect("red's room");
    assert_eq!(
        rig.location(0),
        Some(room),
        "the pilot stands in the team room"
    );
    let steer = rig.with_ctx_mut(0, |ctx| {
        tacsim::arena_bridge::forward_intent(
            ctx,
            pilot,
            tank,
            tacsim::arena::intents::IntentKind::Heading { mdeg: 180_000 },
        )
    });
    assert_eq!(steer, Err(tacsim::arena_bridge::IntentRefusal::NotFocused));
    let order = rig.with_ctx_mut(0, |ctx| {
        tacsim::arena_bridge::forward_order(ctx, pilot, tank, &[tank], told(tank))
    });
    assert_eq!(order, Err(tacsim::arena_bridge::IntentRefusal::NotFocused));
}

/// One `ordered` line off the tape, as `(unit, squad, objective, target)`, when
/// `issuer` wrote it (16b cut Y.1).
fn y1_ordered_by(
    event: &tacsim::arena::ledger::LedgerEvent,
    issuer: ObjId,
) -> Option<(
    ObjId,
    String,
    String,
    Option<tacsim::arena::state::ContactId>,
)> {
    match &event.kind {
        tacsim::arena::ledger::EventKind::Ordered {
            unit,
            squad,
            issuer: Some(by),
            objective,
            target,
            ..
        } if *by == issuer => Some((*unit, squad.to_string(), objective.to_string(), *target)),
        _ => None,
    }
}

/// **16b cut Y.1 (16c Q38, the cut Y plan's decision 2; review Y-08)**: a
/// non-lead reaches their pod pair's tank **and not the rest of its squad**,
/// and the answer goes to the issuer alone.
///
/// [`y1_a_pilot_orders_their_pod_pair_and_nothing_else`] fields one red
/// machine, so its squad and its wingman are the same one unit and cannot tell
/// a one-unit fan-out from a whole-roster one. Here red pilots claim slots 1, 2
/// and 3, slot 2 is given up (`SIM LEAVE` keeps the other indices) and its
/// pilot goes blue, so `SIM AI 4` flies red pods 2 and 4 — `RB` and `RD`, one
/// squad of two. Slot 3's pilot (pair 4) orders `red-1` and then `ALL`: each
/// journals exactly one `ordered` line, on `RD`; the lead's `ORDER red-1 HOLD`
/// journals two, one per unit. Every event either order produced is addressed
/// to its issuer and nobody else hears it: no room line, no side line, no
/// radio line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y1_a_non_lead_reaches_their_wingman_and_not_the_rest_of_its_squad() {
    let mut rig = Rig::boot_with(3, y1_config());
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team red");
    rig.say(1, "sim leave");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim ai 4");
    assert!(!said.is_empty(), "SIM AI answers");
    rig.say(0, "sim start");
    for n in [0, 2] {
        rig.walk_to(n, match_::RED_ROOM_ENTRY);
        rig.say(n, "board");
    }
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
    assert_eq!(
        rig.centre()
            .slot_of_pilot(rig.pilot(2))
            .map(|slot| slot.index),
        Some(3),
        "the third red claimant keeps slot 3"
    );

    let record = rig.record();
    let unit_of = |word: &str| {
        record
            .callsigns
            .iter()
            .find(|(_, callsign)| callsign.to_string() == word)
            .map(|(unit, _)| *unit)
            .unwrap_or_else(|| panic!("{word} is fielded"))
    };
    let (rb, rd) = (unit_of("RB"), unit_of("RD"));
    let red: Vec<&tacsim::deliberation::SquadRoster> = record
        .squads
        .iter()
        .filter(|roster| roster.id.team == PartyId::RED)
        .collect();
    assert_eq!(red.len(), 1, "fewer than four machines are one squad");
    let mut roster = red[0].units.clone();
    roster.sort();
    let mut both = vec![rb, rd];
    both.sort();
    assert_eq!(roster, both, "red-1 is RB and RD");

    // Slot 3 orders its pair, by the squad's word.
    let (lead, third, blue) = (rig.pilot(0), rig.pilot(2), rig.pilot(1));
    let events = rig.events(2, "ORDER red-1 HOLD");
    assert_eq!(
        rig.tw.seen_by(&events, third).join("\n"),
        format!("Ordered RD to hold: it stops where it stands and fires nothing.{Y1_TAIL}")
    );
    assert!(
        events
            .iter()
            .all(|(audience, _)| *audience == Audience::Actor(third)),
        "the answer is the issuer's alone: {events:?}"
    );
    for other in [lead, blue] {
        assert!(
            rig.tw.seen_by(&events, other).is_empty(),
            "nobody else hears an order"
        );
    }
    let issuer = rig.tank(2);
    let holds = |event: &tacsim::arena::ledger::LedgerEvent| {
        y1_ordered_by(event, issuer).is_some_and(|(_, _, objective, _)| objective == "hold")
    };
    assert_eq!(rig.tape_count_until(holds, 1).await, 1);

    // The lead's order to the same squad reaches both machines. Waiting for
    // its two lines also closes the question of slot 3's: they were journalled
    // a pulse earlier, so every one of them is on the tape by now.
    let events = rig.events(0, "ORDER red-1 HOLD");
    assert_eq!(
        rig.tw.seen_by(&events, lead).join("\n"),
        format!("Ordered red-1 to hold: it stops where it stands and fires nothing.{Y1_TAIL}")
    );
    assert!(
        events
            .iter()
            .all(|(audience, _)| *audience == Audience::Actor(lead)),
        "the lead's answer is the lead's alone: {events:?}"
    );
    assert!(rig.tw.seen_by(&events, third).is_empty());
    let by_lead = rig.tank(0);
    let from_lead =
        |event: &tacsim::arena::ledger::LedgerEvent| y1_ordered_by(event, by_lead).is_some();
    assert_eq!(rig.tape_count_until(from_lead, 2).await, 2);
    let mut reached: Vec<ObjId> = rig
        .tape
        .iter()
        .filter_map(|event| y1_ordered_by(event, by_lead))
        .map(|(unit, squad, _, _)| {
            assert_eq!(squad, "red-1");
            unit
        })
        .collect();
    reached.sort();
    assert_eq!(
        reached, both,
        "the lead's order lands on every unit of red-1"
    );
    let from_third: Vec<ObjId> = rig
        .tape
        .iter()
        .filter_map(|event| y1_ordered_by(event, issuer))
        .map(|(unit, _, _, _)| unit)
        .collect();
    assert_eq!(from_third, vec![rd], "slot 3's order landed on RD alone");

    // `ALL` from slot 3 is the same one tank.
    let said = rig.say(2, "ORDER ALL WITHDRAW");
    assert_eq!(
        said,
        format!(
            "Ordered RD to withdraw: it breaks for its entry zone and still fights inside its \
             bands.{Y1_TAIL}"
        )
    );
    let withdraws = |event: &tacsim::arena::ledger::LedgerEvent| {
        y1_ordered_by(event, issuer).is_some_and(|(_, _, objective, _)| objective == "withdraw")
    };
    assert_eq!(rig.tape_count_until(withdraws, 1).await, 1);
    // One more of the lead's, so the pulse that carried slot 3's `ALL` is
    // wholly on the tape before it is counted.
    rig.say(0, "ORDER red-1 HOLD");
    assert_eq!(rig.tape_count_until(from_lead, 4).await, 4);
    let withdrawn: Vec<ObjId> = rig
        .tape
        .iter()
        .filter(|event| withdraws(event))
        .filter_map(|event| y1_ordered_by(event, issuer))
        .map(|(unit, _, _, _)| unit)
        .collect();
    assert_eq!(
        withdrawn,
        vec![rd],
        "ALL from a non-lead is their wingman alone"
    );
}

/// **16b cut Y.1 — `ENGAGE <contact>` end to end**: the hint is the pilot's
/// own row's opaque id, stamped with the pilot's own tank as its issuer, and
/// an own-side row is no target.
///
/// The red lead's tank is stood fourteen cells south of the blue pilot's (as
/// the lock test stands them) with red's first machine (`RB`) four cells to its
/// west; `ORDER ALL HOLD` keeps red's machines where they stand until the
/// lead's panel holds both, classified. Then:
///
/// * `ORDER red-1 ENGAGE BA` answers with the hint, and both of red-1's
///   `ordered` lines carry `target` = the id of the lead's own `BA` row and the
///   lead's tank as `issuer` — the arena resolves it on that scope;
/// * `ORDER red-1 ENGAGE RB` — a row the lead's panel classes as red — is a
///   plain engage, answered and journalled as one (`target` none): the
///   machines never fire on their own side, so nothing may promise they will.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y1_an_engage_carries_the_pilots_own_row_and_an_own_side_row_is_no_target() {
    let mut rig = Rig::boot_with(2, y1_config());
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim ai 3");
    assert!(!said.is_empty(), "SIM AI answers");
    rig.say(0, "sim start");
    let middle = i64::from(match_::ARENA_CELLS / 2);
    rig.place(rig.tank(0), (middle, middle), 0);
    rig.place(rig.tank(1), (middle, middle + 14), 180_000);
    let rb =
        match_::centre::tank_for_slot(rig.tw.world(), PartyId::RED, 2).expect("red pod 2 has a tank");
    rig.place(rb, (middle - 4, middle), 0);
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    let said = rig.say(0, "ORDER ALL HOLD");
    assert!(said.starts_with("Ordered red-1 to hold"), "{said}");
    rig.say(0, "startup");
    rig.say(1, "startup");
    assert!(rig.powered(0).await, "the tank wakes up");

    // Until the lead's panel holds BA and RB, each under its callsign.
    let lead = rig.pilot(0);
    let mut held = None;
    for _ in 0..1_500 {
        rig.pump();
        let rows = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(lead))
            .map(|held| cockpit::frame_contacts(&held.frame))
            .unwrap_or_default();
        let row = |word: &str| {
            rows.iter()
                .find(|row| row.designator.to_string() == word)
                .cloned()
        };
        if let (Some(ba), Some(own)) = (row("BA"), row("RB")) {
            held = Some((ba, own));
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let (ba, own) = held.expect("the lead's panel holds BA and RB");
    assert!(!ba.friendly, "BA is blue");
    assert!(own.friendly, "RB is the lead's own side");

    let said = rig.say(0, "ORDER red-1 ENGAGE BA");
    assert_eq!(
        said,
        format!(
            "Ordered red-1 to engage BA: it closes and fires on BA where it can see it, else on \
             what it finds.{Y1_TAIL}"
        )
    );
    let issuer = rig.tank(0);
    let engaged = |event: &tacsim::arena::ledger::LedgerEvent| {
        y1_ordered_by(event, issuer).is_some_and(|(_, _, objective, _)| objective == "engage")
    };
    assert_eq!(
        rig.tape_count_until(engaged, 2).await,
        2,
        "one line per unit of red-1"
    );
    for (_, squad, _, target) in rig
        .tape
        .iter()
        .filter(|event| engaged(event))
        .filter_map(|event| y1_ordered_by(event, issuer))
    {
        assert_eq!(squad, "red-1");
        assert_eq!(
            target,
            Some(ba.id),
            "the hint is the lead's own row's id, never an object id"
        );
    }

    let said = rig.say(0, "ORDER red-1 ENGAGE RB");
    assert_eq!(
        said,
        format!("Ordered red-1 to engage: it closes and fires on what it finds.{Y1_TAIL}"),
        "an own-side row is no target"
    );
    assert_eq!(rig.tape_count_until(engaged, 4).await, 4);
    let bare = rig
        .tape
        .iter()
        .filter(|event| engaged(event))
        .filter_map(|event| y1_ordered_by(event, issuer))
        .filter(|(_, _, _, target)| target.is_none())
        .count();
    assert_eq!(bare, 2, "the own-side hint is dropped, not journalled");
}

// ---------------------------------------------------------------------------
// 16b cut Y.2 — the span and the rate (doc 16 §2 W8)
// ---------------------------------------------------------------------------

/// [`y1_config`] with `[tacsim.command]` lowered to a span of one squad and two
/// orders a minute, so both bind in a handful of lines.
fn y2_config() -> TacsimConfig {
    let mut cfg = y1_config();
    cfg.command.max_span = 1;
    cfg.command.orders_per_minute = 2;
    cfg
}

/// **16b cut Y.2 (doc 16 §2 W8): the span is read off the world's own watch.**
///
/// This field is two a side, which is one squad a side (a side of at most
/// `[tacsim.arena].squad_size` machines — four — is one squad; 16b cut Z.1), so
/// the lead's second squad is a live watch seeded on the registry — which is
/// where the span is read, and the only place. (Since 16b cut Z.3's six a side
/// the centre can make two squads for real — five machines deal as three and
/// two — and
/// [`z3_one_pilot_and_six_a_side_make_two_squads_and_the_span_refuses_all`]
/// reaches the `ORDER ALL` span sentence that way, end to end, with no seeded
/// watch. This test keeps the seeded one because it is the only way to hold a
/// *live* order on a squad the pilot did not order through the verb.) With a span of one and that watch standing, `ORDER red-1 HOLD` and
/// `ORDER ALL HOLD` (which names the lead's one squad, so one at a time would
/// not pass either) are each refused with the wait sentence and reach no unit;
/// a third refusal at a rate of two a minute still answers the span, so a span
/// refusal spends no stamp. Once the seeded watch has run out in world time the
/// same order is taken, and
/// re-ordering the squad the pilot already holds is taken too — a replacement
/// never counts against the span.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y2_the_span_counts_the_other_squads_under_a_live_order_and_waits_for_one_to_run_out() {
    let mut rig = Rig::boot_with(2, y2_config());
    rig.fly_with_ai(1, 1, 3).await;
    let match_id = rig.centre().match_id.expect("a running match");
    let issuer = rig.tank(0);
    let second = tacsim::deliberation::SquadId {
        team: PartyId::RED,
        index: 2,
    };
    let expires_ms = rig.now.saturating_mul(1_000).saturating_add(600_000);
    // 17b cut L.1: the issuing pilot's character is the watch's author.
    let by = tacsim::deliberation::CharacterRef::Character { id: rig.pilot(0) };
    rig.with_ctx_mut(0, |ctx| {
        ctx.w
            .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the registry is composed")
            .note_human_order(match_id, second, "hold".into(), issuer, by, expires_ms);
    });
    let live = |rig: &Rig, at_s: i64| {
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.live_orders_of(match_id, issuer, at_s.saturating_mul(1_000)))
            .unwrap_or_default()
    };
    assert_eq!(
        live(&rig, rig.now).into_iter().collect::<Vec<_>>(),
        vec![second]
    );

    assert_eq!(
        rig.say(0, "ORDER red-1 HOLD"),
        "You hold orders on 1 squad(s) already; wait for one to run out, or RELEASE one."
    );
    // `ALL` names the lead's one squad here, so ordering it on its own would be
    // refused too: the answer is to wait, never "one at a time".
    assert_eq!(
        rig.say(0, "ORDER ALL HOLD"),
        "You hold orders on 1 squad(s) already; wait for one to run out, or RELEASE one."
    );
    // The span is checked before the rate stamp is spent: two refusals at a rate
    // of two a minute leave the third answering the span, not the rate.
    assert_eq!(
        rig.say(0, "ORDER red-1 HOLD"),
        "You hold orders on 1 squad(s) already; wait for one to run out, or RELEASE one.",
        "a span refusal spent no stamp"
    );
    let status = rig.staff("@arena/status");
    assert!(
        status.contains("  orders: 1 by pilots, 0 by lance leaders"),
        "a refused order reached no squad; the one counted is the seeded watch: {status}"
    );

    // The seeded watch runs out in world time; the span is free again.
    let past = rig.now.saturating_add(601);
    rig.tick_to(past);
    assert!(live(&rig, past).is_empty(), "the seeded watch has run out");
    assert_eq!(
        rig.say(0, "ORDER red-1 HOLD"),
        format!("Ordered red-1 to hold: it stops where it stands and fires nothing.{Y1_TAIL}")
    );
    // A replacement on the squad the pilot already holds is not a second squad.
    assert_eq!(
        rig.say(0, "ORDER red-1 WITHDRAW"),
        format!(
            "Ordered red-1 to withdraw: it breaks for its entry zone and still fights inside \
             its bands.{Y1_TAIL}"
        )
    );
    assert_eq!(
        live(&rig, past)
            .into_iter()
            .map(|squad| squad.to_string())
            .collect::<Vec<_>>(),
        vec!["red-1".to_string()]
    );
}

/// **16b cut Y.2 (doc 16 §2 W8's human trigger): the rate is a window of world
/// seconds, per issuer, emptied with the match.**
///
/// At two a minute the third `ORDER` inside one minute is refused, and sixty
/// one seconds on it is taken. `ORDER ALL` spends one stamp like any other.
/// Then the match is aborted and checked in (`ArenaRegistry::untrack`), and a
/// second match on the same rig, flown in the same world minute by the same
/// pilot in the same tank, starts at nought.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y2_the_rate_refuses_the_third_order_in_a_minute_and_a_new_match_starts_at_nought() {
    let mut rig = Rig::boot_with(2, y2_config());
    rig.fly_with_ai(1, 1, 3).await;
    let hold =
        format!("Ordered red-1 to hold: it stops where it stands and fires nothing.{Y1_TAIL}");
    let full = "You have given 2 orders in the last minute; wait.";

    // A refusal before the rate spends nothing: two of them first, and the
    // pilot still has both orders of the minute.
    assert_eq!(rig.say(0, "ORDER blue-1 HOLD"), cockpit::NO_SUCH_SQUAD);
    assert_eq!(rig.say(0, "ORDER blue-1 HOLD"), cockpit::NO_SUCH_SQUAD);
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), hold);
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), hold);
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), full);
    assert_eq!(rig.say(0, "ORDER ALL HOLD"), full, "ALL is one order");
    // And one after the window is full is still its own sentence, not the rate's.
    assert_eq!(rig.say(0, "ORDER blue-1 HOLD"), cockpit::NO_SUCH_SQUAD);

    let later = rig.now.saturating_add(61);
    rig.tick_to(later);
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), hold);
    assert_eq!(
        rig.say(0, "ORDER ALL HOLD"),
        hold.clone(),
        "ALL reaches red-1 alone"
    );
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), full);

    // End the match; the check-in untracks it.
    let tank = rig.tank(0);
    rig.say(0, "sim abort");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    rig.staff("@arena/reset");
    assert_eq!(rig.state(), MatchState::Idle);
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    rig.fly_with_ai(1, 1, 3).await;
    assert_eq!(rig.now, later, "the same world second as the refusal");
    assert_eq!(rig.tank(0), tank, "the same tank, the same issuer");
    assert_eq!(
        rig.say(0, "ORDER red-1 HOLD"),
        hold,
        "a new match starts at nought"
    );
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), hold);
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), full);
}

// ---------------------------------------------------------------------------
// 17b cut L.1: the principal (doc 17 §4 "A mind never orders"; D-UM-52, D-UM-53)
// ---------------------------------------------------------------------------

/// A squad order for `squad`, authored by `by` in `issuer`'s scope — a `hold`
/// standing ten minutes (17b cut L.1).
fn l1_order(
    squad: tacsim::deliberation::SquadId,
    issuer: Option<ObjId>,
    by: tacsim::deliberation::CharacterRef,
) -> tacsim::deliberation::Order {
    tacsim::deliberation::Order {
        squad,
        objective: tacsim::deliberation::Objective::Hold,
        target: None,
        horizon_ms: 600_000,
        constraints: tacsim::deliberation::Constraints::default(),
        ttl_ms: 600_000,
        issued_ms: 0,
        issuer,
        by,
        spot: None,
    }
}

/// **17b cut L.1 — the admission is the cockpit's own authority rule** (D-UM-52:
/// *an order whose `by` names no character entitled to order that squad is
/// refused at the world and bridge admission*; the rule is `command_of`'s, 16c
/// Q38 as 16b cut Y.1 built it).
///
/// [`y1_a_non_lead_reaches_their_wingman_and_not_the_rest_of_its_squad`]'s
/// field: red's lead (pilot 0), red's slot-3 pilot (pilot 2) whose wingman is
/// `RD`, blue's pilot (pilot 1), and a fourth player who never joined; red-1 is
/// `RB` and `RD`. Through `send_order` — the one wall the typed path, the
/// leader's path and any direct caller meet — a room, the outsider, the other
/// side's pilot, and the non-lead for `RB` (red-1's, but not their wingman) are
/// each `NoCharacter`; the non-lead for `RD` and the lead for either are
/// admitted and the lead's `ordered` line lands; a persona of another squad,
/// and red-1's own persona with an issuer, are `NoCharacter`; red-1's own
/// persona in the commander's scope is admitted. The unit is part of the grant:
/// the lead's red-1 order onto the lead's own tank or a blue machine, and
/// red-1's persona onto a blue machine, are `NoCharacter`. A match with no
/// record is `Unavailable`; and once the record names no leader for red-1, its
/// persona is `NoCharacter` too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_send_order_refuses_an_order_nobody_entitled_gave() {
    use tacsim::arena_bridge::IntentRefusal;
    use tacsim::deliberation::{CharacterRef, SquadId};
    let mut rig = Rig::boot_with(4, y1_config());
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team red");
    rig.say(1, "sim leave");
    rig.say(1, "sim team blue");
    assert!(!rig.say(0, "sim ai 4").is_empty(), "SIM AI answers");
    rig.say(0, "sim start");
    for n in [0, 2] {
        rig.walk_to(n, match_::RED_ROOM_ENTRY);
        rig.say(n, "board");
    }
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);

    let record = rig.record();
    let match_id = record.id;
    let unit_of = |word: &str| {
        record
            .callsigns
            .iter()
            .find(|(_, callsign)| callsign.to_string() == word)
            .map(|(unit, _)| *unit)
            .unwrap_or_else(|| panic!("{word} is fielded"))
    };
    let (rb, rd) = (unit_of("RB"), unit_of("RD"));
    let red_one = SquadId {
        team: PartyId::RED,
        index: 1,
    };
    assert!(
        record
            .squads
            .iter()
            .any(|roster| roster.id == red_one && roster.holds(rb) && roster.holds(rd)),
        "red-1 is RB and RD"
    );
    let blue_one = SquadId {
        team: PartyId::BLUE,
        index: 1,
    };
    let (lead, blue, third, outsider) = (rig.pilot(0), rig.pilot(1), rig.pilot(2), rig.pilot(3));
    let room = match_::centre_object(rig.tw.world(), match_::RED_ROOM_ENTRY).expect("red's room");
    let (lead_tank, third_tank) = (rig.tank(0), rig.tank(2));
    let character = |id: ObjId| CharacterRef::Character { id };
    let persona = |squad: SquadId| CharacterRef::Persona { squad };
    let send = |rig: &mut Rig, unit: ObjId, order: tacsim::deliberation::Order| {
        rig.with_ctx_mut(0, |ctx| {
            tacsim::arena_bridge::send_order(ctx, match_id, unit, order)
        })
    };
    let refused = Err(IntentRefusal::NoCharacter);

    // Nobody entitled: a room, a player not in the match, the other side's
    // pilot, and a non-lead for a squad unit that is not their wingman.
    for (who, by, unit) in [
        ("a room", character(room), rb),
        ("a player not in the match", character(outsider), rb),
        ("the other side's pilot", character(blue), rb),
        ("the non-lead, not their wingman", character(third), rb),
    ] {
        assert_eq!(
            send(&mut rig, unit, l1_order(red_one, Some(lead_tank), by)),
            refused,
            "{who}"
        );
    }
    // A valid principal cannot borrow the commander's or another unit's
    // contact scope through the direct bridge entry.
    for issuer in [None, Some(third_tank), Some(rb)] {
        assert_eq!(
            send(&mut rig, rb, l1_order(red_one, issuer, character(lead))),
            refused,
            "the lead's issuer must be their actual occupied tank"
        );
    }
    assert_eq!(
        send(
            &mut rig,
            rd,
            l1_order(red_one, Some(lead_tank), character(third))
        ),
        refused,
        "the wingman grant does not borrow the lead's contact scope"
    );
    // Entitled: the non-lead for their own wingman, the lead for any red unit.
    assert_eq!(
        send(
            &mut rig,
            rd,
            l1_order(red_one, Some(third_tank), character(third))
        ),
        Ok(()),
        "the non-lead orders their wingman"
    );
    for unit in [rb, rd] {
        assert_eq!(
            send(
                &mut rig,
                unit,
                l1_order(red_one, Some(lead_tank), character(lead))
            ),
            Ok(()),
            "the lead orders any unit of the side's squads"
        );
    }
    let from_lead = |event: &tacsim::arena::ledger::LedgerEvent| {
        y1_ordered_by(event, lead_tank).is_some_and(|(_, squad, _, _)| squad == "red-1")
    };
    assert_eq!(
        rig.tape_count_until(from_lead, 2).await,
        2,
        "the lead's order lands"
    );

    // The personas: another squad's, and red-1's own with an issuer, refused;
    // red-1's own in the commander's scope admitted.
    assert_eq!(
        send(&mut rig, rb, l1_order(red_one, None, persona(blue_one))),
        refused,
        "another squad's persona"
    );
    assert_eq!(
        send(
            &mut rig,
            rb,
            l1_order(red_one, Some(lead_tank), persona(red_one))
        ),
        refused,
        "a leader's issuer stays None"
    );
    assert_eq!(
        send(&mut rig, rb, l1_order(red_one, None, persona(red_one))),
        Ok(()),
        "red-1's own lance-leader orders red-1"
    );

    // The unit is part of the grant: an order written for red-1 reaches only
    // red-1's machines — the lead's own tank and a blue machine are outside it,
    // for the lead's character and for red-1's persona alike.
    let blue_machine = record
        .squads
        .iter()
        .find(|roster| roster.id.team == PartyId::BLUE)
        .and_then(|roster| roster.units.first().copied())
        .expect("blue fields machines");
    for (who, unit) in [
        ("the lead, onto their own tank", lead_tank),
        ("the lead, onto a blue machine", blue_machine),
    ] {
        assert_eq!(
            send(
                &mut rig,
                unit,
                l1_order(red_one, Some(lead_tank), character(lead))
            ),
            refused,
            "{who}: no roster of the grant holds it"
        );
    }
    assert_eq!(
        send(
            &mut rig,
            blue_machine,
            l1_order(red_one, None, persona(red_one))
        ),
        refused,
        "red-1's lance-leader orders red-1's machines and no other's"
    );

    // No record for the match: the admission cannot read a rule, so the order
    // is `Unavailable`, not a principal's refusal.
    let gone = tacsim::arena_task::MatchId(match_id.0 + 1_000);
    assert_eq!(
        rig.with_ctx_mut(0, |ctx| {
            tacsim::arena_bridge::send_order(
                ctx,
                gone,
                rb,
                l1_order(red_one, Some(lead_tank), character(lead)),
            )
        }),
        Err(IntentRefusal::Unavailable),
        "a match with no record"
    );

    // The same real egress wreck fact that stops typed ORDER also stops a
    // direct send_order, for both a lead and an otherwise entitled wingman.
    for (pilot, tank, author, unit) in [(0, lead_tank, lead, rb), (2, third_tank, third, rd)] {
        let mut dead = rig.held_frame(pilot);
        dead.notices.clear();
        dead.wreck = true;
        rig.push_frame(pilot, dead);
        assert_eq!(
            send(
                &mut rig,
                unit,
                l1_order(red_one, Some(tank), character(author))
            ),
            refused,
            "a destroyed pilot cannot bypass typed ORDER's alive check"
        );
    }

    // The record is the persona's warrant: with red-1's row gone from
    // `leaders`, red-1's own persona in the commander's scope is refused.
    if let Some(record) = rig
        .tw
        .world_mut()
        .module_state_mut::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get_mut(match_id))
    {
        record.leaders.retain(|leader| leader.squad != red_one);
    }
    assert_eq!(
        send(&mut rig, rb, l1_order(red_one, None, persona(red_one))),
        refused,
        "a persona the record names as nobody's leader orders nothing"
    );
}

/// **17b cut L.1 — a typed `ORDER` carries the seat's character** (D-UM-52):
/// the red lead's `ORDER red-1 HOLD`, acknowledged in HEAD's exact words, lands
/// on both of red-1's machines with `issuer` = the lead's tank on each hull's
/// `ordered` tape line, the world's watch names the author (`order_author`:
/// the lead's character) and the issuer, and the shield stands: a signal for
/// red-1 takes no in-flight slot, while another squad's signal in the same
/// call takes exactly one. **17b cut M4.3**: red-1's machines are minds, so
/// the snapshot the arena writes every second carries each hull's order on
/// its mind row as the `FrameOrder` the mind reads (the squad and the
/// objective; a mind's order names no author — D-UM-52's `by` is the
/// world's watch's and the tape's), where the reflex row carried the
/// `StandingOrder` whole.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_a_typed_order_carries_the_seats_character() {
    use tacsim::deliberation::CharacterRef;
    let mut cfg = y1_config();
    cfg.arena.snapshot_ms = 1_000;
    let mut rig = Rig::boot_with(2, cfg);
    rig.fly_with_ai(1, 1, 3).await;
    let record = rig.record();
    let match_id = record.id;
    let roster = record
        .squads
        .iter()
        .find(|roster| roster.id.to_string() == "red-1")
        .cloned()
        .expect("red fields red-1");
    let (pilot, tank) = (rig.pilot(0), rig.tank(0));

    assert_eq!(
        rig.say(0, "ORDER red-1 HOLD"),
        format!("Ordered red-1 to hold: it stops where it stands and fires nothing.{Y1_TAIL}"),
        "the acknowledgement is HEAD's, byte for byte"
    );
    let by_pilot = |event: &tacsim::arena::ledger::LedgerEvent| {
        y1_ordered_by(event, tank).is_some_and(|(_, _, objective, _)| objective == "hold")
    };
    assert_eq!(
        rig.tape_count_until(by_pilot, roster.units.len()).await,
        roster.units.len()
    );
    let landed_ms = rig
        .tape
        .iter()
        .filter(|event| by_pilot(event))
        .map(|event| event.t_ms)
        .max()
        .expect("the ordered lines");

    // The world's watch.
    let author = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| {
            registry
                .order_author(match_id, roster.id)
                .zip(registry.order_issuer(match_id, roster.id))
        });
    assert_eq!(
        author,
        Some((CharacterRef::Character { id: pilot }, tank)),
        "the watch's author is the seat's character, its issuer the seat's tank"
    );

    // The shield: a signal for red-1 is not spent.
    let signal = tacsim::arena::command::SquadSignal {
        squad: roster.id,
        commander: roster.commander,
        team: roster.id.team,
        reason: tacsim::arena::command::SignalReason::NoOrder,
        at_ms: 0,
        digest: tacsim::arena::command::SquadDigest::default(),
        contacts: Vec::new(),
        bands_cm: Vec::new(),
        sight_cm: 0,
        default_ttl_ms: 30_000,
        health_permille: 1_000,
        states: [0; 7],
        cell: None,
        window: (16, 16),
        members: Vec::new(),
        objective: None,
    };
    let counts = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.deliberation_counts(match_id))
    };
    // The match's own opening signals may already hold slots (no provider
    // answers in this rig), so the shield is read as "no slot more" — beside a
    // squad no pilot ordered, in the same call, which does take one: the cap is
    // not what kept red-1 from being asked.
    let other = record
        .squads
        .iter()
        .find(|other| other.id != roster.id)
        .cloned()
        .expect("the match fields a second squad");
    let other_signal = tacsim::arena::command::SquadSignal {
        squad: other.id,
        commander: other.commander,
        team: other.id.team,
        ..signal.clone()
    };
    let before = counts(&rig).expect("the registry");
    assert!(
        before.2 < tacsim::deliberation::MAX_IN_FLIGHT_PER_MATCH,
        "a slot is free: {before:?}"
    );
    rig.with_ctx_mut(0, |ctx| {
        tacsim::arena_bridge::on_deliberate(ctx, match_id, &[signal.clone(), other_signal]);
    });
    assert_eq!(
        counts(&rig),
        Some((before.0, before.1, before.2 + 1)),
        "a squad under the pilot's order is not asked about; the unordered one is"
    );

    // The arena's own standing orders, as its snapshot carries them.
    let head = rig.af3_snapshot_at_or_after(landed_ms).await;
    assert_eq!(head.format, tacsim::arena::snapshot::SNAPSHOT_FORMAT);
    let key = match_::snapshot_key(match_id).to_string();
    let state: serde_json::Value = rig
        .image
        .get(tacsim::NS)
        .and_then(|docs| docs.get(key.as_str()))
        .and_then(|document| document.get("state"))
        .and_then(serde_json::Value::as_str)
        .and_then(|text| serde_json::from_str(text).ok())
        .expect("the snapshot's state");
    for unit in &roster.units {
        let order = state
            .get("minds")
            .and_then(|minds| minds.get(unit.0.to_string()))
            .and_then(|row| row.get("order"))
            .filter(|order| !order.is_null())
            .unwrap_or_else(|| panic!("{unit:?}'s mind carries a standing order"));
        assert_eq!(
            order.get("squad"),
            Some(&serde_json::json!("red-1")),
            "{unit:?}: the order is red-1's"
        );
        // `MindObjective` is tagged `objective`, so the row's `objective` key
        // holds `{"objective": "hold"}`.
        assert_eq!(
            order
                .get("objective")
                .and_then(|objective| objective.get("objective")),
            Some(&serde_json::json!("hold")),
            "{unit:?}: the objective the lead typed"
        );
        // The author rides the tape's `ordered` line for this hull: the
        // issuer is the seat's tank (asserted above through `y1_ordered_by`).
        assert!(
            rig.tape.iter().any(|event| matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::Ordered { unit: on, issuer: Some(by), .. }
                    if on == unit && *by == tank
            )),
            "{unit:?}: the tape names the seat's tank as the issuer"
        );
    }
}

/// **17b cut L.1 — `NoCharacter` is never a cockpit word.** The refusal renders,
/// through `CockpitError::Order`'s `Refused: {0}.`, as one plain sentence, so a
/// future caller has one; but no `ORDER` a pilot can type reaches it: the
/// cockpit names the seat's character and only the units `command_of` gave it,
/// and every refusal through the real composition is one of the four pinned
/// words.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_no_character_is_never_a_cockpit_word() {
    assert_eq!(
        cockpit::CockpitError::Order(tacsim::arena_bridge::IntentRefusal::NoCharacter).to_string(),
        "Refused: no character entitled to order that squad gave that order."
    );
    // The field of `y1_a_pilot_orders_their_pod_pair_and_nothing_else`: red's
    // lead (pilot 0), slot 2 (pilot 1, pair 1, a human: no wingman), slot 3
    // (pilot 2, wingman RD), and blue (pilot 3).
    let mut rig = Rig::boot_with(4, y1_config());
    rig.fly_with_ai(3, 1, 4).await;
    let pinned = [
        cockpit::NO_SUCH_SQUAD,
        cockpit::NO_SQUAD_TO_ORDER,
        cockpit::NO_SUCH_FRIENDLY,
        cockpit::NO_SUCH_CONTACT,
    ];
    let mut seen = std::collections::BTreeSet::new();
    for (n, line) in [
        (0, "ORDER blue-1 HOLD"),
        (0, "ORDER red-9 HOLD"),
        (0, "ORDER red-1 ENGAGE ZZ"),
        (0, "ORDER red-1 SUPPORT BZ"),
        (1, "ORDER red-1 HOLD"),
        (1, "ORDER ALL HOLD"),
        (2, "ORDER blue-1 HOLD"),
        (2, "ORDER red-1 ENGAGE ZZ"),
        (2, "ORDER red-1 SUPPORT BA"),
        (3, "ORDER red-1 HOLD"),
    ] {
        let said = rig.say(n, line);
        assert!(
            pinned.contains(&said.as_str()),
            "pilot {n}'s {line:?} answered {said:?}, which is not a pinned refusal"
        );
        assert!(!said.contains("no character"), "{said}");
        seen.insert(said);
    }
    assert_eq!(
        seen.len(),
        pinned.len(),
        "every pinned word was reached: {seen:?}"
    );
}

/// **17b cut L.1 — a match of nothing but pilots mints no leader** (D-UM-53):
/// no squad, so no persona — the record's `leaders` is empty beside its empty
/// `squads` (doc 16 §1's A1 story).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn l1_a_pilots_only_match_mints_no_leader() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let record = rig.record();
    assert!(record.squads.is_empty(), "no machine is fielded");
    assert!(record.leaders.is_empty(), "so no lance-leader is minted");
}

// ---------------------------------------------------------------------------
// 16b cut Y.3: the reclaim (16c `N4`; doc 07 §1.3 "Reclaiming control")
// ---------------------------------------------------------------------------

/// The red row of the staff board — the whole row whoever is flying.
fn y3_red_row(rig: &Rig) -> String {
    tacsim::match_::status_lines(rig.tw.world(), BoardReader::Staff)
        .into_iter()
        .find(|line| line.trim_start().starts_with("red:"))
        .unwrap_or_default()
}

/// The registry's seat for a pilot, if it holds one.
fn y3_seat_of(rig: &Rig, pilot: ObjId) -> Option<(tacsim::arena_task::MatchId, ObjId)> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.seat_of_pilot(pilot))
}

/// The account a pilot's slot is held by.
fn y3_account(rig: &Rig, pilot: ObjId) -> AccountId {
    rig.centre()
        .slot_of_pilot(pilot)
        .map(|slot| slot.account)
        .expect("the pilot holds a slot")
}

/// **A dropped pilot is told the machine has their tank, and `BOARD` takes it
/// back where it stands, at a new epoch** (16b cut Y.3; 16c `N4`; doc 07 §1.3).
///
/// The drop is `fire_conn(Disconnected)`, which runs `SeatFacts::disconnected`
/// and mush-sim's own hook exactly as production does: the board reads `flown
/// by its mind`, nobody is flying and the registry has no seat. The reconnect
/// says so in one line; a `BOARD` inside `reclaim_min_ms` is told to wait; past
/// it, a `BOARD` from inside the pod the drop left the pilot in takes the tank
/// back — bound again, every channel's epoch raised, a frame, a `HEADING`
/// taken — and a second `BOARD` is `AlreadyAboard`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y3_a_dropped_pilot_is_told_and_boards_again_from_the_pod() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    let tank = rig.tank(0);
    let pod = rig.pod(0).expect("a locked pod");
    let match_id = rig.centre().match_id.expect("a running match");
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the arena brings power online");

    // The drop. (17b cut M5.3: `, flown by doctrine` and `… is flying on
    // doctrine; …` until that cut — one word for every sealed mind now.)
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    let row = y3_red_row(&rig);
    assert!(row.contains(", flown by its mind"), "{row}");
    assert!(
        !row.contains(", seated") && !row.contains(", left"),
        "{row}"
    );
    assert!(
        cockpit::flying(rig.tw.world(), pilot).is_none(),
        "the dropped pilot is still flying"
    );
    assert_eq!(y3_seat_of(&rig, pilot), None, "the registry kept the seat");
    assert_eq!(
        rig.location(0),
        Some(pod),
        "a drop leaves the body in the pod"
    );
    // The observer rule (doc 16 §1, §5; 16b cut D.1): the other side's pilot
    // reads red's row reduced — nothing about who is flying it.
    let theirs = rig.say(1, "sim status");
    let red_to_blue = theirs
        .lines()
        .find(|line| line.trim_start().starts_with("red:"))
        .unwrap_or_default();
    assert!(
        !red_to_blue.contains("flown by") && !red_to_blue.contains("left"),
        "the other side read the drop: {theirs}"
    );

    // The reconnect: one line, and no reclaim of its own.
    let events = rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    let said = rig.tw.seen_by(&events, pilot).join("\n");
    assert!(
        said.contains(&format!(
            "{} is flying on its mind; BOARD takes it back.",
            mush_core::output::display_name(rig.tw.world(), tank)
        )),
        "{said}"
    );
    assert!(!said.contains("back at your station"), "{said}");

    // Too soon.
    let said = rig.say(0, "board");
    assert!(
        said.contains(&format!(
            "{} is still being handed over; try again in 5 second(s).",
            mush_core::output::display_name(rig.tw.world(), tank)
        )),
        "{said}"
    );
    assert_eq!(
        y3_seat_of(&rig, pilot),
        None,
        "a refused BOARD bound nothing"
    );

    // Past the floor, from inside the pod.
    let before: Vec<u64> = Channel::ALL.iter().map(|c| rig.epoch(tank, *c)).collect();
    let now = rig.now;
    rig.tick_to(now + 5);
    let events = rig.events(0, "board");
    let said = rig.tw.seen_by(&events, pilot).join("\n");
    assert!(
        said.contains(&format!(
            "You climb back into {}: {} is yours again where it stands.",
            mush_core::output::display_name(rig.tw.world(), pod),
            mush_core::output::display_name(rig.tw.world(), tank)
        )),
        "{said}"
    );
    assert_eq!(y3_seat_of(&rig, pilot), Some((match_id, tank)));
    let after: Vec<u64> = Channel::ALL.iter().map(|c| rig.epoch(tank, *c)).collect();
    for (was, now) in before.iter().zip(&after) {
        assert!(
            now > was,
            "the reclaim is a NEW epoch: {before:?} -> {after:?}"
        );
    }
    assert!(
        cockpit::flying(rig.tw.world(), pilot).is_some(),
        "the reclaim did not give the focus back"
    );
    assert!(
        rig.centre()
            .slot_of_pilot(pilot)
            .is_some_and(|slot| !slot.left && slot.dropped_at_ms.is_none() && slot.is_seated()),
        "the slot is back in its cockpit"
    );
    assert!(y3_red_row(&rig).contains(", seated"));
    // The arena speaks to the pilot again, and takes an order.
    assert!(rig.frame_for(0).await, "no frame after the reclaim");
    assert!(rig.powered(0).await, "the tank came back where it stands");
    let said = rig.say(0, "heading 90");
    assert!(said.contains("Steering to 90"), "{said}");

    // And a second `BOARD` is the seated pilot's.
    let said = rig.say(0, "board");
    assert!(said.contains("You are already in your pod."), "{said}");
}

/// **`EXIT` is one-way whatever came before it** (16b cut Y.3; doc 16 §11's N.1
/// amendment), both ways round: a drop, a reconnect in the pod and an `EXIT`;
/// and an `EXIT` and then a `QUIT`. Either way `BOARD` is refused for the rest of
/// the match, and the slot carries no drop stamp.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y3_an_exit_is_final_before_or_after_a_drop() {
    let left_for_good = |rig: &Rig, n: usize| {
        format!(
            "You left {} for the rest of the match; it cannot be taken back.",
            mush_core::output::display_name(rig.tw.world(), rig.tank(n))
        )
    };
    // (a) drop -> reconnect -> EXIT -> BOARD from the team room.
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    let said = rig.say(0, "exit");
    assert!(said.contains("you cannot take it back"), "{said}");
    assert!(
        rig.centre()
            .slot_of_pilot(pilot)
            .is_some_and(|slot| slot.left && slot.dropped_at_ms.is_none()),
        "the EXIT did not make the leave final"
    );
    assert!(y3_red_row(&rig).contains(", left"), "{}", y3_red_row(&rig));
    let now = rig.now;
    rig.tick_to(now + 10);
    let said = rig.say(0, "board");
    assert!(said.contains(&left_for_good(&rig, 0)), "{said}");
    assert_eq!(y3_seat_of(&rig, pilot), None);

    // (b) EXIT -> QUIT -> BOARD.
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    let said = rig.say(0, "exit");
    assert!(said.contains("you cannot take it back"), "{said}");
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    assert!(
        rig.centre()
            .slot_of_pilot(pilot)
            .is_some_and(|slot| slot.left && slot.dropped_at_ms.is_none()),
        "a QUIT after an EXIT turned it into a drop"
    );
    // And the reconnect after it is silent: nothing to take back.
    let events = rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    let said = rig.tw.seen_by(&events, pilot).join("\n");
    assert!(
        !said.contains("flying on its mind") && !said.contains("back at your station"),
        "a reconnect after an EXIT and a QUIT offered the tank back: {said}"
    );
    let now = rig.now;
    rig.tick_to(now + 10);
    let said = rig.say(0, "board");
    assert!(said.contains(&left_for_good(&rig, 0)), "{said}");
}

/// **A reclaimed lead does not take the side back** (16b cut Y.3; 16c Q42's
/// default): the red lead drops, the heir leads, the lead reclaims and the heir
/// still leads — and the heir's `SIM CONCEDE` is accepted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y3_a_reclaimed_lead_leaves_the_heir_leading() {
    let mut rig = Rig::boot_with(3, y1_config());
    rig.fly_with_ai(2, 1, 2).await;
    let (lead, heir) = (rig.pilot(0), rig.pilot(1));
    let (lead_account, heir_account) = (y3_account(&rig, lead), y3_account(&rig, heir));
    assert!(rig.centre().leads_side(PartyId::RED, lead_account));

    rig.fire_conn(lead, mush_core::ConnTransition::Disconnected);
    assert!(
        rig.centre().leads_side(PartyId::RED, heir_account),
        "the heir did not take the lead"
    );
    let now = rig.now;
    rig.tick_to(now + 5);
    let said = rig.say(0, "board");
    assert!(said.contains("is yours again where it stands"), "{said}");
    assert!(
        rig.centre().leads_side(PartyId::RED, heir_account),
        "the reclaimed pilot took the lead back"
    );
    assert!(!rig.centre().leads_side(PartyId::RED, lead_account));
    let said = rig.say(1, "sim concede");
    assert!(
        said.contains("Pilot1 concedes for the red side."),
        "the heir's concede was refused: {said}"
    );
}

/// **A reclaim clears the unattended clock** (16b cut Y.3; 16b cut D.3's
/// `B-33`): a lone pilot of an AI-filled match drops, the clock starts, they
/// reclaim, the clock and its warning are cleared — and the interval passing
/// ends nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y3_a_reclaim_clears_the_unattended_clock() {
    let mut cfg = y1_config();
    cfg.match_.unattended_ms = 10_000;
    let mut rig = Rig::boot_with(1, cfg);
    rig.fly_with_ai(1, 0, 1).await;
    let pilot = rig.pilot(0);
    let account = y3_account(&rig, pilot);
    assert_eq!(rig.centre().unattended_since_ms, None);
    assert!(rig.centre().leads_side(PartyId::RED, account));

    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    assert!(
        rig.centre().unattended_since_ms.is_some(),
        "the lone pilot's drop did not start the clock"
    );
    // Every red claimant has left its tank, so red has no lead at all
    // (`resettle_leads` on the drop) ...
    assert!(
        !rig.centre()
            .leads
            .iter()
            .any(|(side, _)| *side == PartyId::RED),
        "a side whose only pilot dropped kept a lead"
    );
    let now = rig.now;
    rig.tick_to(now + 5);
    let said = rig.say(0, "board");
    assert!(said.contains("is yours again where it stands"), "{said}");
    assert_eq!(rig.centre().unattended_since_ms, None, "the clock survived");
    assert!(!rig.centre().unattended_warned);
    // ... and the first who comes back leads it again: the reclaim's own
    // `resettle_leads` (Q42's default keeps a recorded heir; there is none).
    assert!(
        rig.centre().leads_side(PartyId::RED, account),
        "the lone pilot came back and leads nothing"
    );
    // Past the interval the match runs on: no stop was asked for.
    let now = rig.now;
    rig.tick_to(now + 30);
    assert_eq!(rig.state(), MatchState::Running);
    assert_eq!(
        rig.record().state,
        MatchState::Running,
        "the unattended stop was sent under a pilot who came back"
    );
    // And the side's word is the reclaimed pilot's again: in a match nobody
    // else can end, their concede is accepted.
    let said = rig.say(0, "sim concede");
    assert!(
        said.contains("Pilot0 concedes for the red side."),
        "the reclaimed lone pilot's concede was refused: {said}"
    );
}

/// **A dropped pilot stood in the team room reclaims from there** (16b cut
/// Y.3): the boot sweep's shape — the placement released and the body moved —
/// and the team room is told the pilot climbed back in. The reclaim's own room
/// check comes first: `board`'s top gate lets any team room through, and from
/// the OTHER side's room the reclaim is refused, binds nothing and moves nobody.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y3_a_dropped_pilot_in_the_team_room_boards_back_in_and_the_room_hears_it() {
    let mut rig = Rig::boot(3);
    rig.fly().await;
    let pilot = rig.pilot(0);
    let pod = rig.pod(0).expect("a locked pod");
    // A bystander in the red room, who holds no slot.
    rig.walk_to(2, match_::RED_ROOM_ENTRY);
    let bystander = rig.pilot(2);
    let team_room =
        match_::centre_object(rig.tw.world(), match_::RED_ROOM_ENTRY).expect("the red room");
    let their_room =
        match_::centre_object(rig.tw.world(), match_::BLUE_ROOM_ENTRY).expect("the blue room");

    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    rig.with_ctx_mut(0, |ctx| {
        mush_sim::unplace(ctx, pilot, "test").expect("the pod placement releases");
        ctx.w
            .move_to(pilot, their_room)
            .expect("the body walks into the other side's team room");
    });
    assert_eq!(rig.location(0), Some(their_room));

    // Past the floor, from the wrong team room: refused, as `Boarding` refuses it.
    let now = rig.now;
    rig.tick_to(now + 5);
    let said = rig.say(0, "board");
    assert!(said.contains(cockpit::OUTSIDE_TEAM_ROOM), "{said}");
    assert_eq!(
        y3_seat_of(&rig, pilot),
        None,
        "a reclaim from the other side's room bound the pilot"
    );
    assert_eq!(
        rig.location(0),
        Some(their_room),
        "a refused reclaim moved the body"
    );
    assert!(
        rig.centre()
            .slot_of_pilot(pilot)
            .is_some_and(|slot| slot.is_dropped()),
        "a refused reclaim touched the slot"
    );

    // Into their own team room, and the reclaim goes through.
    rig.with_ctx_mut(0, |ctx| {
        ctx.w
            .move_to(pilot, team_room)
            .expect("the body walks into the team room");
    });
    assert_eq!(rig.location(0), Some(team_room));
    let events = rig.events(0, "board");
    let said = rig.tw.seen_by(&events, pilot).join("\n");
    assert!(said.contains("is yours again where it stands"), "{said}");
    assert_eq!(rig.location(0), Some(pod), "the body is back in the pod");
    let heard = rig.tw.seen_by(&events, bystander).join("\n");
    assert!(
        heard.contains(&format!(
            "Pilot0 climbs back into {}.",
            mush_core::output::display_name(rig.tw.world(), pod)
        )),
        "the team room was not told: {heard}"
    );
    assert!(
        cockpit::flying(rig.tw.world(), pilot).is_some(),
        "the reclaim from the team room did not seat the pilot"
    );
}

/// **A reclaim into a match already stopping is refused** (16b cut Y.3): a `SIM
/// ABORT` parks the record in `Stopping` while the centre is still `Running`
/// until the arena answers, and `BOARD` says the centre is stopping.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y3_a_reclaim_while_the_stop_is_in_flight_is_refused() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    let now = rig.now;
    rig.tick_to(now + 5);
    rig.say(1, "sim abort");
    assert_eq!(
        rig.record().state,
        MatchState::Stopping,
        "the stop is in flight"
    );
    assert_eq!(rig.state(), MatchState::Running, "the centre has not heard");
    let said = rig.say(0, "board");
    assert!(
        said.contains(
            "The simulator centre is stopping; that is not something you can do to it now."
        ),
        "{said}"
    );
    assert_eq!(
        y3_seat_of(&rig, pilot),
        None,
        "the refused reclaim bound nothing"
    );
}

/// **`BOARD` reads `[tacsim.match].reclaim_min_ms` and counts the wait up**
/// (16b cut Y.3): at 2,500 ms a `BOARD` in the drop's own second has 2.5 s to
/// go and says three, two seconds later it has half a second and says one —
/// never "0" — and a second after that the tank is back. Every other reclaim
/// test runs at the default 5,000, a whole number of world seconds, so this is
/// the one that would see the field ignored or the wait rounded down. The
/// reclaim is from the pod the drop left the pilot in, so the body does not
/// move and the red room hears nothing of it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y3_the_reclaim_floor_is_the_configured_one_rounded_up() {
    let mut cfg = config();
    cfg.match_.reclaim_min_ms = 2_500;
    let mut rig = Rig::boot_with(3, cfg);
    rig.fly().await;
    let pilot = rig.pilot(0);
    let tank = rig.tank(0);
    // A bystander in the red room, who holds no slot.
    rig.walk_to(2, match_::RED_ROOM_ENTRY);
    let bystander = rig.pilot(2);
    let wait = |rig: &Rig, secs: i64| {
        format!(
            "{} is still being handed over; try again in {secs} second(s).",
            mush_core::output::display_name(rig.tw.world(), tank)
        )
    };

    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    let dropped = rig.now;
    let said = rig.say(0, "board");
    assert!(said.contains(&wait(&rig, 3)), "{said}");
    rig.tick_to(dropped + 2);
    let said = rig.say(0, "board");
    assert!(said.contains(&wait(&rig, 1)), "{said}");
    assert_eq!(
        y3_seat_of(&rig, pilot),
        None,
        "a refused BOARD bound nothing"
    );
    rig.tick_to(dropped + 3);
    let events = rig.events(0, "board");
    let said = rig.tw.seen_by(&events, pilot).join("\n");
    assert!(said.contains("is yours again where it stands"), "{said}");
    assert!(
        cockpit::flying(rig.tw.world(), pilot).is_some(),
        "the reclaim did not seat the pilot"
    );
    let heard = rig.tw.seen_by(&events, bystander).join("\n");
    assert!(
        !heard.contains("climbs back into"),
        "a reclaim from inside the pod was announced to the room: {heard}"
    );
}

/// **`reclaim_min_ms = 0` reclaims in the drop's own second** (16b cut Y.3):
/// the `Released` and the reclaim's `Live` reach the arena task back to back,
/// through the real composition, and the task's buffer applies them in epoch
/// order — the pilot is on the frame roster again and the arena speaks to them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y3_a_zero_floor_reclaims_in_the_drops_own_second() {
    let mut cfg = config();
    cfg.match_.reclaim_min_ms = 0;
    let mut rig = Rig::boot_with(2, cfg);
    rig.fly().await;
    let pilot = rig.pilot(0);
    let tank = rig.tank(0);
    let match_id = rig.centre().match_id.expect("a running match");

    let dropped = rig.now;
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    let said = rig.say(0, "board");
    assert_eq!(rig.now, dropped, "the same world second as the drop");
    assert!(said.contains("is yours again where it stands"), "{said}");
    assert_eq!(y3_seat_of(&rig, pilot), Some((match_id, tank)));
    assert!(
        y3_red_row(&rig).contains(", seated"),
        "{}",
        y3_red_row(&rig)
    );
    assert!(
        rig.frame_for(0).await,
        "no frame after a same-second reclaim"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Y.4: the catch-up report (16c `N4`; doc 07 §1.3 step 5)
// ---------------------------------------------------------------------------

/// Every seated `arena.cockpit` keyframe the rig has sent `pilot` since `from`,
/// in order.
fn y4_cockpit_frames(rig: &Rig, pilot: ObjId, from: usize) -> Vec<serde_json::Value> {
    rig.log
        .iter()
        .skip(from)
        .filter_map(|(audience, event)| match event {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::arena_bridge::FRAME_FEED
                    && *audience == Audience::Actor(pilot)
                    && data.get("seated").and_then(serde_json::Value::as_bool) != Some(false) =>
            {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

/// Every `Sys` line the rig has sent `pilot` since `from`, as its plain text
/// and its span tags.
fn y4_sys_lines(rig: &Rig, pilot: ObjId, from: usize) -> Vec<(String, Vec<Option<String>>)> {
    rig.log
        .iter()
        .skip(from)
        .filter_map(|(audience, event)| match event {
            OutputEvent::Sys(text) if *audience == Audience::Actor(pilot) => Some((
                text.to_plain_text(),
                text.spans()
                    .iter()
                    .map(|span| span.tag.as_deref().map(str::to_owned))
                    .collect(),
            )),
            _ => None,
        })
        .collect()
}

/// Pump until the rig has sent `pilot` at least `want` seated cockpit frames
/// since `from`, and answer them.
async fn y4_frames_until(
    rig: &mut Rig,
    pilot: ObjId,
    from: usize,
    want: usize,
) -> Vec<serde_json::Value> {
    for _ in 0..600 {
        rig.pump();
        if y4_cockpit_frames(rig, pilot, from).len() >= want {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    y4_cockpit_frames(rig, pilot, from)
}

/// The mirror's own frame's catch-up for pilot `n`, read through a `Ctx` the
/// way a verb reads it: `None` for no mirrored frame at all.
fn y4_mirror_catchup(rig: &mut Rig, n: usize) -> Option<Option<tacsim::arena::Catchup>> {
    let pilot = rig.pilot(n);
    rig.with_ctx(n, |ctx| {
        ctx.w
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(pilot))
            .map(|held| held.frame.catchup.clone())
    })
}

/// **A reclaim's first frame carries the catch-up once, and the line client
/// reads it once** (16b cut Y.4; doc 07 §1.3 step 5), over the production
/// composition: `BOARD` past the grace, the arena's `Live`, the report built,
/// the frame, `on_frames`' one tagged `Sys` block before the frame's notices,
/// the mirror storing it cleared.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y4_a_reclaim_s_first_frame_carries_the_catch_up_and_says_it_once() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the arena brings power online");

    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    let now = rig.now;
    rig.tick_to(now + 5);
    let mark = rig.log.len();
    let said = rig.say(0, "board");
    assert!(said.contains("is yours again where it stands"), "{said}");

    // The first seated cockpit frame after the reclaim carries the report.
    let frames = y4_frames_until(&mut rig, pilot, mark, 1).await;
    let first = frames.first().cloned().expect("no frame after the reclaim");
    let catchup = first.get("catchup").cloned().unwrap_or_default();
    assert!(catchup.is_object(), "the reclaim's first frame: {first}");
    assert!(
        catchup
            .get("away_s")
            .and_then(serde_json::Value::as_i64)
            .is_some(),
        "{catchup}"
    );
    assert_eq!(
        catchup
            .pointer("/machine/state")
            .and_then(serde_json::Value::as_str),
        Some("withdraw"),
        "a released tank is the machine's in Withdraw: {catchup}"
    );

    // The reads, at once — the tank stands still, so the mirror is still the
    // frame that carried the report: it holds it cleared, and neither read
    // re-says it or pushes it again.
    y4_reads_never_resay(&mut rig, "the reclaim's own frame");

    // The next frame does not carry it. An order is what makes a next frame
    // at all.
    let said = rig.say(0, "heading 180");
    assert!(said.contains("Steering to 180"), "{said}");
    let frames = y4_frames_until(&mut rig, pilot, mark, 2).await;
    assert!(
        frames.len() >= 2,
        "two frames after the reclaim: {frames:?}"
    );
    for later in frames.iter().skip(1) {
        assert_eq!(
            later.get("catchup"),
            Some(&serde_json::Value::Null),
            "a later frame carried the report again: {later}"
        );
    }

    // The line client: exactly one block, tagged as a frame's own sentence.
    let sys = y4_sys_lines(&rig, pilot, mark);
    let blocks: Vec<&(String, Vec<Option<String>>)> = sys
        .iter()
        .filter(|(text, _)| text.starts_with("Back aboard RA"))
        .collect();
    assert_eq!(blocks.len(), 1, "one block: {sys:?}");
    let (block, tags) = blocks.first().copied().cloned().unwrap_or_default();
    assert!(
        tags.iter()
            .all(|tag| tag.as_deref() == Some(tacsim::arena_bridge::NOTICE_SENTENCE_TAG)),
        "the block is tagged as a frame's own sentence: {tags:?}"
    );
    assert!(
        block.contains("\n  The machine was withdrawing toward its entry zone under no order."),
        "{block}"
    );
    for line in [
        "\n  Position: ",
        "\n  Contacts: ",
        "\n  Sections: ",
        "\n  Rounds spent: ",
        "\n  Power online.",
    ] {
        assert!(
            block.contains(line),
            "{line:?} is not in the block:\n{block}"
        );
    }

    // **Before the frame's notice lines.** The still reclaim frame above need
    // carry no notice at all, so the order is proven on `on_frames`' own path
    // over a frame that carries both: the live frame, with the report the
    // reclaim was sent and a refusal beside it.
    use tacsim::arena::intents::Refusal;
    use tacsim::arena::perception::{FrameNotice, RefusalDetail};
    let mut both = rig.held_frame(0);
    both.catchup = Some(y4_decode(&catchup));
    both.notices = vec![FrameNotice::Refused {
        order: smol_str::SmolStr::new("fire"),
        refusal: Refusal::PowerNotOnline,
        weapon: None,
        detail: RefusalDetail::None,
    }];
    let notice = cockpit::notice_lines(&both);
    assert_eq!(notice.len(), 1, "the refusal says one line: {notice:?}");
    let pushed = rig.log.len();
    rig.push_frame(0, both);
    let sys = y4_sys_lines(&rig, pilot, pushed);
    let at = sys
        .iter()
        .position(|(text, _)| text.starts_with("Back aboard RA"));
    let refusal = sys
        .iter()
        .position(|(text, _)| Some(text) == notice.first());
    assert!(
        at.is_some() && refusal.is_some() && at < refusal,
        "the block is said before the frame's notice lines: {sys:?}"
    );
    assert_eq!(sys.len(), 2, "the block and the one notice: {sys:?}");

    // And that frame, too, is remembered without it.
    y4_reads_never_resay(&mut rig, "a frame carrying a report and a notice");
}

/// **The mirror holds no catch-up, and `STATUS` and `VIEW` re-say none** (16b
/// cut Y.4): read straight after a frame that carried one, so it is that
/// frame's `remembered` copy the reads render.
fn y4_reads_never_resay(rig: &mut Rig, after: &str) {
    let pilot = rig.pilot(0);
    assert_eq!(
        y4_mirror_catchup(rig, 0),
        Some(None),
        "the mirror kept the report after {after}"
    );
    let spoken = rig.log.len();
    for verb in ["status", "view"] {
        let events = rig.events(0, verb);
        let text = rig.tw.seen_by(&events, pilot).join("\n");
        assert!(
            !text.contains("Back aboard"),
            "{verb} re-said the report after {after}: {text}"
        );
        let frame = Rig::panel(&events).expect("a read pushes its frame");
        assert_eq!(
            frame.get("catchup"),
            Some(&serde_json::Value::Null),
            "{verb}'s frame carried the report after {after}"
        );
    }
    assert!(
        !y4_sys_lines(rig, pilot, spoken)
            .iter()
            .any(|(text, _)| text.contains("Back aboard")),
        "the report was said again after {after}"
    );
}

/// **A revival inside the grace reads no catch-up, and leaves no baseline for
/// a later drop** (16b cut Y.4): `Linkdead` then `Connected` sends a `Live`
/// that finds the pilot still on the roster — no block, no `catchup` — and
/// the snapshot the `Linkdead` took is spent.
///
/// The `Connected` waits until the arena has run a pulse after the `Linkdead`
/// was offered: the task buffers controls until its pulse and a `Live` at a
/// higher epoch displaces a standing `Linkdead` (`buffer_control`), so the two
/// back to back would never reach `apply_control` as two and no snapshot would
/// ever be taken. The rig cannot reach into the arena task, so the second half
/// is read off the one place a surviving baseline would show: a later drop
/// and reclaim, measured from **that** drop and not from the `Linkdead` eight
/// arena seconds and more before it. (The arena's own
/// `a_revival_inside_the_grace_spends_the_snapshot_and_builds_nothing` reads
/// `at_release` directly.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn y4_a_revival_inside_the_grace_reads_no_catch_up_and_leaves_no_baseline() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the arena brings power online");

    let mark = rig.log.len();
    rig.pump();
    let linkdead_at = rig.staged_now_ms;
    let out = rig.fire_conn(pilot, mush_core::ConnTransition::Linkdead);
    rig.log.extend(out);
    // The arena takes the `Linkdead` in a pulse of its own. The staged clock
    // lags the arena's by at most the one arena second a `Frames` is published
    // in whatever the frames do, so a clock two seconds on is a pulse that ran
    // after the offer — the one that took the snapshot.
    for _ in 0..1_000 {
        rig.pump();
        if rig.staged_now_ms >= linkdead_at + 2_000 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        rig.staged_now_ms >= linkdead_at + 2_000,
        "the arena did not run a pulse after the Linkdead"
    );
    let out = rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    rig.log.extend(out);
    // The seat is the pilot's again: an order turns the tank, so frames flow.
    let said = rig.say(0, "heading 180");
    assert!(said.contains("Steering to 180"), "{said}");
    // Eight arena seconds and more from the `Linkdead`, with frames flowing:
    // a baseline that survived the revival would make the later report count
    // from a moment at least six seconds before its drop.
    for _ in 0..2_000 {
        rig.pump();
        if rig.staged_now_ms >= linkdead_at + 8_000 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        rig.staged_now_ms >= linkdead_at + 8_000,
        "the arena did not run on"
    );
    let frames = y4_cockpit_frames(&rig, pilot, mark);
    assert!(!frames.is_empty(), "the revived pilot's frames flow");
    assert!(
        frames
            .iter()
            .all(|frame| frame.get("catchup") == Some(&serde_json::Value::Null)),
        "a revival carried a report: {frames:?}"
    );
    assert!(
        !y4_sys_lines(&rig, pilot, mark)
            .iter()
            .any(|(text, _)| text.contains("Back aboard")),
        "a revival was told a report"
    );

    // Now a real drop, and a reclaim: the report counts from this drop.
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    let now = rig.now;
    rig.tick_to(now + 5);
    let mark = rig.log.len();
    let said = rig.say(0, "board");
    assert!(said.contains("is yours again where it stands"), "{said}");
    let frames = y4_frames_until(&mut rig, pilot, mark, 1).await;
    let away_s = frames
        .first()
        .and_then(|frame| frame.pointer("/catchup/away_s"))
        .and_then(serde_json::Value::as_i64)
        .expect("the reclaim's report");
    assert!(
        away_s < 4,
        "the report counted from the revived Linkdead, not from the drop: {away_s} s"
    );
}

/// A designator as a frame prints it, read back (the fixture's names).
fn y4_designator(word: &str) -> tacsim::arena::state::Designator {
    use tacsim::arena::state::Designator;
    if let Some(number) = word.strip_prefix('X') {
        return Designator::Provisional(number.parse().expect("a provisional number"));
    }
    let mut letters = word.chars();
    let team = match letters.next() {
        Some('R') => PartyId::RED,
        Some('B') => PartyId::BLUE,
        other => panic!("no side is lettered {other:?}"),
    };
    let slot = letters.fold(0_u16, |slot, letter| {
        slot * 26 + (u16::from(u8::try_from(letter).expect("ASCII")) - u16::from(b'A') + 1)
    });
    Designator::Slot { team, slot }
}

/// The fixture's wire object, decoded into the arena's report.
fn y4_decode(wire: &serde_json::Value) -> tacsim::arena::Catchup {
    use serde_json::Value;
    let int = |v: &Value, key: &str| v.get(key).and_then(Value::as_i64).expect("an integer");
    let text = |v: &Value, key: &str| {
        smol_str::SmolStr::new(v.get(key).and_then(Value::as_str).expect("a string"))
    };
    let cell = |v: &Value, key: &str| -> Option<(u16, u16)> {
        let pair = v.get(key)?.as_array()?;
        Some((
            u16::try_from(pair.first()?.as_u64()?).ok()?,
            u16::try_from(pair.get(1)?.as_u64()?).ok()?,
        ))
    };
    let power = |v: &Value, key: &str| -> tacsim::arena::state::Power {
        serde_json::from_value(v.get(key).cloned().unwrap_or_default()).expect("a power word")
    };
    let names = |key: &str| -> Vec<tacsim::arena::state::Designator> {
        wire.get(key)
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(Value::as_str)
                    .map(y4_designator)
                    .collect()
            })
            .unwrap_or_default()
    };
    let count = |key: &str| u32::try_from(int(wire, key)).expect("a count");
    let words = |v: &Value| tacsim::arena::SectionWords {
        condition: text(v, "condition"),
        armour: int(v, "armour"),
        internals: int(v, "internals"),
    };
    let list = |key: &str| {
        wire.get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    tacsim::arena::Catchup {
        away_ms: int(wire, "away_s") * 1_000,
        cell_then: cell(wire, "cell_then"),
        cell_now: cell(wire, "cell_now"),
        power_then: power(wire, "power_then"),
        power_now: power(wire, "power_now"),
        wreck: wire.get("wreck").and_then(Value::as_bool).expect("a flag"),
        contacts_new: names("contacts_new"),
        contacts_lost: names("contacts_lost"),
        contacts_more: count("contacts_more"),
        sections: list("sections")
            .iter()
            .map(|s| {
                (
                    text(s, "id"),
                    words(s.get("then").expect("then")),
                    words(s.get("now").expect("now")),
                )
            })
            .collect(),
        sections_more: count("sections_more"),
        rounds_spent: list("rounds_spent")
            .iter()
            .map(|r| {
                (
                    text(r, "weapon"),
                    u32::try_from(int(r, "rounds")).expect("rounds"),
                )
            })
            .collect(),
        rounds_more: count("rounds_more"),
        machine: wire.get("machine").filter(|m| m.is_object()).map(|m| {
            tacsim::arena::MachineWords {
                state: text(m, "state"),
                goal: cell(m, "goal"),
                objective: m
                    .get("objective")
                    .and_then(Value::as_str)
                    .map(smol_str::SmolStr::new),
            }
        }),
    }
}

/// **The parity fixture is what the server sends and says** (16b cut Y.4):
/// every case of the hand-written `catchup_lines.json`, put on the canonical
/// scene's frame, comes out of `status_frame` as exactly the fixture's object,
/// and `catchup_lines` says its lines — the Rust reader of the one file the TUI
/// and the web are pinned to as well.
#[test]
fn y4_the_catch_up_fixture_is_the_wire_s_object_and_its_lines() {
    let cases: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../web/src/lib/fixtures/cockpit/catchup_lines.json"
    ))
    .expect("catchup_lines.json is an array");
    // Was 9 until 17b cut M5.3, which added the tenth case (D14: the machine
    // `flying on its own mind`).
    assert_eq!(cases.len(), 10);
    for case in &cases {
        let wire = case.get("catchup").expect("a catchup object");
        let report = y4_decode(wire);
        let lines: Vec<String> = case
            .get("lines")
            .and_then(serde_json::Value::as_array)
            .map(|lines| {
                lines
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let callsign = case
            .get("callsign")
            .and_then(serde_json::Value::as_str)
            .map(y4_designator)
            .expect("a callsign");
        assert_eq!(cockpit::catchup_lines(callsign, &report), lines);
        let mut frame = fixture::frame();
        frame.catchup = Some(report);
        let payload = cockpit::status_frame(fixture::MATCH, &frame);
        assert_eq!(payload.get("catchup"), Some(wire), "the wire's own object");
    }
}

// ---------------------------------------------------------------------------
// 16b cut Z.2 — SCREEN, SUPPORT and RELEASE (16c N6, Q44)
// ---------------------------------------------------------------------------

/// One `order_released` line off the tape, as `(unit, squad)`, when `issuer`'s
/// release wrote it (16b cut Z.2).
fn z2_released_by(
    event: &tacsim::arena::ledger::LedgerEvent,
    issuer: ObjId,
) -> Option<(ObjId, String)> {
    match &event.kind {
        tacsim::arena::ledger::EventKind::OrderReleased {
            unit,
            squad,
            issuer: Some(by),
        } if *by == issuer => Some((*unit, squad.to_string())),
        _ => None,
    }
}

/// The squads the world's watch says `issuer` holds a live order on now.
fn z2_live(rig: &Rig, issuer: ObjId) -> Vec<String> {
    let match_id = rig.centre().match_id.expect("a running match");
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| {
            registry
                .live_orders_of(match_id, issuer, rig.now.saturating_mul(1_000))
                .into_iter()
                .map(|squad| squad.to_string())
                .collect()
        })
        .unwrap_or_default()
}

/// **16b cut Z.2 (16c N6, Q44): the lead screens, supports and releases, and
/// every callsign refusal is one line.**
///
/// `SIM AI 3` with one pilot a side: red is `RA` (the lead) and the machines
/// `RB` and `RC` in `red-1`; red pod `D` is empty. A `RELEASE` to a squad under
/// no order is acknowledged and journals nothing. `SCREEN 90` answers
/// verbatim and the tape's `ordered` lines read `screen`; `SUPPORT RA` — the
/// lead's own tank — answers with the 150 m leash; the other side's callsign,
/// a word that is no callsign, a provisional and an empty pod's callsign are
/// one byte-identical sentence; `RELEASE` answers verbatim, journals one
/// `order_released` per machine and leaves the lead holding no squad. The
/// grammar's new usage lines are `ORDER_USAGE`, and `blue-1` is `No such
/// squad.` before any callsign is looked at.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z2_the_lead_screens_supports_and_releases_and_every_callsign_refusal_is_one_line() {
    let mut rig = Rig::boot_with(2, y1_config());
    rig.fly_with_ai(1, 1, 3).await;
    let issuer = rig.tank(0);
    // 17b cut M5.3: `to doctrine: it fights on its own reflexes …` until that
    // cut — a release names no mind, one word for every sealed mind.
    let release = "Released red-1 to its mind: it fights on its own until it is ordered again.";

    // Nothing is under an order yet: acknowledged, and nothing to journal.
    //
    // A release shares `Order`'s buffer slot (`continuous_code` 2), so an order
    // typed in the same pulse would replace it before the arena saw it. The
    // arena is let run on (the staged clock two seconds past the offer, 16b cut
    // Y.4's wait: the clock lags the arena's by at most the one arena second a
    // `Frames` is published in) before the next order, so the release really is
    // applied to machines under no order — and the tape below counts what it
    // journalled.
    rig.pump();
    let offered_at = rig.staged_now_ms;
    assert_eq!(rig.say(0, "ORDER red-1 RELEASE"), release);
    for _ in 0..1_000 {
        rig.pump();
        if rig.staged_now_ms >= offered_at + 2_000 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        rig.staged_now_ms >= offered_at + 2_000,
        "the arena did not run a pulse after the release"
    );

    assert_eq!(
        rig.say(0, "ORDER red-1 SCREEN 90"),
        format!(
            "Ordered red-1 to screen 090°: it holds where it stands, faces 090° and fires on \
             what comes inside its arc without giving chase.{Y1_TAIL}"
        )
    );
    let screens = |event: &tacsim::arena::ledger::LedgerEvent| {
        y1_ordered_by(event, issuer).is_some_and(|(_, _, objective, _)| objective == "screen")
    };
    assert_eq!(rig.tape_count_until(screens, 2).await, 2);

    // 17b cut M5.6 (OQ-6; STOP D, the text made true): `… and fires first on
    // what RA is fighting.` until that cut.
    assert_eq!(
        rig.say(0, "ORDER red-1 SUPPORT ra"),
        format!(
            "Ordered red-1 to support RA: it keeps within 150 m of RA and fights what it finds \
             there.{Y1_TAIL}"
        )
    );
    // The other side's, nonsense, a provisional, an empty pod: one sentence.
    for line in [
        "ORDER red-1 SUPPORT BA",
        "ORDER red-1 SUPPORT ZZ",
        "ORDER red-1 SUPPORT X1",
        "ORDER red-1 SUPPORT RD",
    ] {
        assert_eq!(rig.say(0, line), cockpit::NO_SUCH_FRIENDLY, "{line}");
    }
    // Authority first: the other side's squad is refused before the callsign.
    assert_eq!(
        rig.say(0, "ORDER blue-1 SUPPORT RA"),
        cockpit::NO_SUCH_SQUAD
    );
    for line in [
        "ORDER red-1 SCREEN 360",
        "ORDER red-1 SCREEN",
        "ORDER red-1 SCREEN east",
        "ORDER red-1 SUPPORT",
        "ORDER red-1 SUPPORT RA RB",
        "ORDER red-1 RELEASE HOLD FIRE",
    ] {
        assert_eq!(rig.say(0, line), cockpit::ORDER_USAGE, "{line}");
    }
    assert_eq!(z2_live(&rig, issuer), vec!["red-1".to_string()]);

    assert_eq!(rig.say(0, "ORDER red-1 RELEASE"), release);
    assert!(
        z2_live(&rig, issuer).is_empty(),
        "the release gave the squad back"
    );
    let released =
        |event: &tacsim::arena::ledger::LedgerEvent| z2_released_by(event, issuer).is_some();
    assert_eq!(rig.tape_count_until(released, 2).await, 2);
    // One more order and its two lines, so every pulse before it is on the tape.
    rig.say(0, "ORDER red-1 HOLD");
    let holds = |event: &tacsim::arena::ledger::LedgerEvent| {
        y1_ordered_by(event, issuer).is_some_and(|(_, _, objective, _)| objective == "hold")
    };
    assert_eq!(rig.tape_count_until(holds, 2).await, 2);
    let lines: Vec<(ObjId, String)> = rig
        .tape
        .iter()
        .filter_map(|event| z2_released_by(event, issuer))
        .collect();
    assert_eq!(
        lines.len(),
        2,
        "one line per machine of the one real release; the release to a squad under no \
         order journalled nothing: {lines:?}"
    );
    assert!(lines.iter().all(|(_, squad)| squad == "red-1"));
}

/// **16b cut Z.2 (16c Q44): a release frees the releaser's own span, and only
/// what its issuer holds.**
///
/// A span of one. The lead orders `red-1`, and a second squad's watch is seeded
/// under the lead as Y.2's test seeds one. `RELEASE` of `red-1` frees `red-1`
/// only, so a new `HOLD` is still refused with the wait sentence; a release of
/// the seeded squad noted for another tank drops nothing, and noted for the
/// lead's own drops it — and the same order is taken.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z2_a_release_frees_the_span_it_was_given_and_nothing_else() {
    let mut cfg = y1_config();
    cfg.command.max_span = 1;
    let mut rig = Rig::boot_with(2, cfg);
    rig.fly_with_ai(1, 1, 3).await;
    let match_id = rig.centre().match_id.expect("a running match");
    let issuer = rig.tank(0);
    let hold =
        format!("Ordered red-1 to hold: it stops where it stands and fires nothing.{Y1_TAIL}");
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), hold);
    let second = tacsim::deliberation::SquadId {
        team: PartyId::RED,
        index: 2,
    };
    let expires_ms = rig.now.saturating_mul(1_000).saturating_add(600_000);
    // 17b cut L.1: the issuing pilot's character is the watch's author.
    let by = tacsim::deliberation::CharacterRef::Character { id: rig.pilot(0) };
    rig.with_ctx_mut(0, |ctx| {
        ctx.w
            .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the registry is composed")
            .note_human_order(match_id, second, "hold".into(), issuer, by, expires_ms);
    });
    assert_eq!(
        z2_live(&rig, issuer),
        vec!["red-1".to_string(), "red-2".to_string()]
    );

    // 17b cut M5.3: `to doctrine: … own reflexes …` until that cut.
    assert_eq!(
        rig.say(0, "ORDER red-1 RELEASE"),
        "Released red-1 to its mind: it fights on its own until it is ordered again."
    );
    assert_eq!(
        z2_live(&rig, issuer),
        vec!["red-2".to_string()],
        "the release freed red-1 alone"
    );
    assert_eq!(
        rig.say(0, "ORDER red-1 HOLD"),
        "You hold orders on 1 squad(s) already; wait for one to run out, or RELEASE one."
    );

    // A release noted for another tank leaves the watch; the issuer's own drops it.
    let note = |rig: &mut Rig, releaser: ObjId| {
        rig.with_ctx_mut(0, |ctx| {
            ctx.w
                .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
                .expect("the registry is composed")
                .note_order_released(match_id, second, releaser);
        });
    };
    note(&mut rig, ObjId(issuer.0.saturating_add(1)));
    assert_eq!(z2_live(&rig, issuer), vec!["red-2".to_string()]);
    note(&mut rig, issuer);
    assert!(z2_live(&rig, issuer).is_empty());
    assert_eq!(rig.say(0, "ORDER red-1 HOLD"), hold);
}

/// **16b cut Z.2 (16c Q44): a wingman's release is their one machine's, and
/// the lead's order stands.**
///
/// The Y.1 non-lead field: red pods 2 and 4 flown by machines (`RB`, `RD`, one
/// squad), slot 3's pilot commanding `RD`. The lead orders `red-1`; slot 3's
/// `ORDER 1 RELEASE` answers by the wingman's callsign and journals one
/// `order_released` line, on `RD` alone; the lead's watch — its issuer and the
/// lead's live squads — is unchanged.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z2_a_wingmans_release_is_their_one_machine_and_the_leads_order_stands() {
    let mut rig = Rig::boot_with(3, y1_config());
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team red");
    rig.say(1, "sim leave");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim ai 4");
    assert!(!said.is_empty(), "SIM AI answers");
    rig.say(0, "sim start");
    for n in [0, 2] {
        rig.walk_to(n, match_::RED_ROOM_ENTRY);
        rig.say(n, "board");
    }
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
    let match_id = rig.centre().match_id.expect("a running match");
    let rd = rig
        .record()
        .callsigns
        .iter()
        .find(|(_, callsign)| callsign.to_string() == "RD")
        .map(|(unit, _)| *unit)
        .expect("RD is fielded");
    let red_one = tacsim::deliberation::SquadId {
        team: PartyId::RED,
        index: 1,
    };
    let (lead, third) = (rig.tank(0), rig.tank(2));

    assert_eq!(
        rig.say(0, "ORDER red-1 HOLD"),
        format!("Ordered red-1 to hold: it stops where it stands and fires nothing.{Y1_TAIL}")
    );
    let from_lead =
        |event: &tacsim::arena::ledger::LedgerEvent| y1_ordered_by(event, lead).is_some();
    assert_eq!(rig.tape_count_until(from_lead, 2).await, 2);

    // 17b cut M5.3: `to doctrine: … own reflexes …` until that cut.
    assert_eq!(
        rig.say(2, "ORDER 1 RELEASE"),
        "Released RD to its mind: it fights on its own until it is ordered again."
    );
    let watch = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.order_issuer(match_id, red_one));
    assert_eq!(watch, Some(lead), "the lead's watch stands");
    assert_eq!(z2_live(&rig, lead), vec!["red-1".to_string()]);
    let released =
        |event: &tacsim::arena::ledger::LedgerEvent| z2_released_by(event, third).is_some();
    assert_eq!(rig.tape_count_until(released, 1).await, 1);
    // One more of the lead's, so the release's pulse is wholly on the tape.
    rig.say(0, "ORDER red-1 HOLD");
    assert_eq!(rig.tape_count_until(from_lead, 4).await, 4);
    let lines: Vec<(ObjId, String)> = rig
        .tape
        .iter()
        .filter_map(|event| z2_released_by(event, third))
        .collect();
    assert_eq!(
        lines,
        vec![(rd, "red-1".to_string())],
        "one line, on the wingman alone"
    );
}

/// **16b cut Z.2 (16c Q44): a release is an `ORDER` and spends a stamp.** At
/// two orders a minute, two releases are taken and the third is the rate's
/// sentence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z2_a_release_spends_a_rate_stamp() {
    let mut rig = Rig::boot_with(2, y2_config());
    rig.fly_with_ai(1, 1, 3).await;
    // 17b cut M5.3: `to doctrine: … own reflexes …` until that cut.
    let release = "Released red-1 to its mind: it fights on its own until it is ordered again.";
    assert_eq!(rig.say(0, "ORDER red-1 RELEASE"), release);
    assert_eq!(rig.say(0, "ORDER red-1 RELEASE"), release);
    assert_eq!(
        rig.say(0, "ORDER red-1 RELEASE"),
        "You have given 2 orders in the last minute; wait."
    );
}

// ---------------------------------------------------------------------------
// 16b cut Z.3 — six a side makes two squads
// ---------------------------------------------------------------------------

/// **Pods pair 1↔2, 3↔4 and 5↔6** (16b cut Z.3's six a side): the wingman rule
/// reaches the new pair, and nothing pairs across it.
#[test]
fn z3_slot_five_pairs_with_six() {
    assert_eq!(cockpit::pair(5), 6);
    assert_eq!(cockpit::pair(6), 5);
    for slot in 1..=match_::PODS_PER_SIDE {
        let other = cockpit::pair(slot);
        assert!((1..=match_::PODS_PER_SIDE).contains(&other), "{slot}");
        assert_eq!(cockpit::pair(other), slot, "{slot} and {other} are a pair");
        assert_eq!(
            (slot - 1) / 2,
            (other - 1) / 2,
            "{slot} pairs inside its two"
        );
    }
}

/// **One pilot and `SIM AI 6` make two squads of machines, and the span
/// refuses `ORDER ALL` over them** (16b cut Z.3; the cut Y plan's decision 2
/// reached end to end).
///
/// With six pods a side the fill fields five machines beside the one pilot on
/// red — dealt `red-1` (three) and `red-2` (two) by 16b cut Z.1 — and six on
/// blue, `blue-1` and `blue-2`, three each. The lead's `ORDER ALL HOLD` names
/// both of its squads in one acknowledgement; and on a server whose
/// `[tacsim.command].max_span` is one, the same line is refused with the
/// `ORDER ALL` span sentence, verbatim — the refusal Y.2 could only reach
/// through a seeded watch while the centre had four pods a side.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z3_one_pilot_and_six_a_side_make_two_squads_and_the_span_refuses_all() {
    let mut rig = Rig::boot_with(1, y1_config());
    rig.fly_with_ai(1, 0, 6).await;
    let record = rig.record();
    let squads: Vec<(String, usize)> = record
        .squads
        .iter()
        .map(|roster| (roster.id.to_string(), roster.units.len()))
        .collect();
    assert_eq!(
        squads,
        vec![
            ("red-1".to_owned(), 3),
            ("red-2".to_owned(), 2),
            ("blue-1".to_owned(), 3),
            ("blue-2".to_owned(), 3),
        ],
        "five machines on red deal three and two; six on blue, three and three"
    );
    // The fill walked pods 5 and 6 on both sides.
    let centre = rig.centre();
    assert_eq!(centre.slots.len(), 1, "one claimed slot");
    let said = rig.say(0, "ORDER ALL HOLD");
    assert_eq!(
        said,
        format!(
            "Ordered red-1 and red-2 to hold: it stops where it stands and fires nothing.{Y1_TAIL}"
        )
    );

    let mut cfg = y1_config();
    cfg.command.max_span = 1;
    let mut rig = Rig::boot_with(1, cfg);
    rig.fly_with_ai(1, 0, 6).await;
    assert_eq!(
        rig.say(0, "ORDER ALL HOLD"),
        "ORDER ALL would put 2 squads under you; your span is 1. Order them one at a time."
    );
    // One at a time does pass.
    assert_eq!(
        rig.say(0, "ORDER red-2 HOLD"),
        format!("Ordered red-2 to hold: it stops where it stands and fires nothing.{Y1_TAIL}")
    );
}

// ---------------------------------------------------------------------------
// 16b cut Z.4 (16c N8): the side nets
// ---------------------------------------------------------------------------

/// The channel id of one side's net, which the boot made.
fn z4_net(rig: &Rig, team: PartyId) -> mush_comsys::ChannelId {
    mush_comsys::api::channel_id_by_name(rig.tw.world(), match_::net_name(team))
        .expect("the boot made both nets")
}

/// Who is on one side's net, in player order.
fn z4_members(rig: &Rig, team: PartyId) -> Vec<ObjId> {
    mush_comsys::api::members_of(rig.tw.world(), z4_net(rig, team))
}

/// One pilot's membership of one side's net, as `(alias, title)`.
fn z4_on(rig: &Rig, n: usize, team: PartyId) -> Option<(String, Option<String>)> {
    let net = z4_net(rig, team);
    rig.tw
        .world()
        .module_state::<mush_comsys::ComsysState>()
        .and_then(|state| state.members.get(&(rig.pilot(n), net)))
        .map(|member| (member.alias.to_string(), member.title.clone()))
}

/// Every plain line [`Rig::log`] holds for `who`, in order.
fn z4_lines(rig: &Rig, who: ObjId) -> Vec<String> {
    rig.log
        .iter()
        .filter(|(audience, _)| *audience == Audience::Actor(who))
        .map(|(_, event)| mush_testkit::text_of(event))
        .collect()
}

/// How many times `who` has been sent exactly `line`.
fn z4_count(rig: &Rig, who: ObjId, line: &str) -> usize {
    z4_lines(rig, who)
        .iter()
        .filter(|seen| *seen == line)
        .count()
}

/// Every `Channel` frame in `events`, as `(to, channel, title, actor, text)`.
fn z4_channel_frames(
    events: &[(Audience, OutputEvent)],
) -> Vec<(ObjId, String, Option<String>, String, String)> {
    events
        .iter()
        .filter_map(|(audience, event)| match (audience, event) {
            (
                Audience::Actor(to),
                OutputEvent::Channel {
                    channel,
                    title,
                    actor,
                    text,
                    ..
                },
            ) => Some((
                *to,
                channel.to_string(),
                title.clone(),
                actor.name.clone(),
                text.to_plain_text(),
            )),
            _ => None,
        })
        .collect()
}

/// Put `pilot` on `team`'s net under `alias`, the way a missed path would have
/// left one there.
fn z4_plant(rig: &mut Rig, n: usize, team: PartyId, alias: &str) {
    let net = z4_net(rig, team);
    let pilot = rig.pilot(n);
    let joined = rig.with_ctx_mut(n, |ctx| {
        mush_comsys::api::force_join(ctx, pilot, net, alias)
    });
    assert!(joined, "the plant took");
}

const RED_JOINED: &str = "You have been joined to channel RedNet (alias: ra).";
const BLUE_JOINED: &str = "You have been joined to channel BlueNet (alias: ba).";
const RED_REMOVED: &str = "You have been removed from channel RedNet.";
const BLUE_REMOVED: &str = "You have been removed from channel BlueNet.";

/// **The join lands at the `Boarding → Running` trigger, not at `SIM START`**
/// (16b cut Z.4): nobody is on a net after `SIM START` or after the first
/// `BOARD`; the last `BOARD` puts each pilot on their side's net with their
/// callsign lower-cased as the alias and the callsign as the title, and each
/// reads comsys's own join line once, just ahead of `Good hunting` — which stays
/// the check-out's last line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_the_last_board_puts_each_pilot_on_their_sides_net_under_their_callsign() {
    let mut rig = Rig::boot(2);
    assert!(z4_members(&rig, PartyId::RED).is_empty());
    assert!(z4_members(&rig, PartyId::BLUE).is_empty());
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert!(
        z4_members(&rig, PartyId::RED).is_empty(),
        "SIM START joins nobody"
    );
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    assert!(
        z4_members(&rig, PartyId::RED).is_empty(),
        "nor does the first BOARD"
    );
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");

    assert_eq!(z4_members(&rig, PartyId::RED), vec![rig.pilot(0)]);
    assert_eq!(z4_members(&rig, PartyId::BLUE), vec![rig.pilot(1)]);
    assert_eq!(
        z4_on(&rig, 0, PartyId::RED),
        Some(("ra".to_owned(), Some("RA".to_owned())))
    );
    assert_eq!(
        z4_on(&rig, 1, PartyId::BLUE),
        Some(("ba".to_owned(), Some("BA".to_owned())))
    );
    assert_eq!(z4_count(&rig, rig.pilot(0), RED_JOINED), 1);
    assert_eq!(z4_count(&rig, rig.pilot(1), BLUE_JOINED), 1);
    assert_eq!(z4_count(&rig, rig.pilot(0), BLUE_JOINED), 0);
    // In the last `BOARD`'s own answer, just ahead of the line that says the
    // match is running (which goes to the centre's rooms, so it is read off what
    // the pilot saw rather than off the actor-addressed log).
    let hunting = said.find("Good hunting").expect("the start is announced");
    let joined = said.find(BLUE_JOINED).expect("the join is said");
    assert!(joined < hunting, "{said}");
}

/// **A line on the net reaches its side and nobody else** (16b cut Z.4): `ra
/// contact bearing 090` from red is one `Channel` frame, to the red pilot, on
/// `RedNet`, carrying the callsign as the title — and nothing at all to blue; a
/// pose rides the same frame; and blue cannot list the net or read who is on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_a_line_on_the_net_reaches_its_side_and_nobody_else() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let (red, blue) = (rig.pilot(0), rig.pilot(1));

    let events = rig.events(0, "ra contact bearing 090");
    assert_eq!(
        z4_channel_frames(&events),
        vec![(
            red,
            "RedNet".to_owned(),
            Some("RA".to_owned()),
            "Pilot0".to_owned(),
            "contact bearing 090".to_owned()
        )]
    );
    let events = rig.events(0, "ra :waves");
    let frames = z4_channel_frames(&events);
    assert_eq!(frames.len(), 1, "{frames:?}");
    assert!(events.iter().any(|(_, event)| matches!(
        event,
        OutputEvent::Channel {
            kind: mush_proto::NarrativeKind::Pose,
            ..
        }
    )));
    // Blue's own net is blue's.
    let events = rig.events(1, "ba copy");
    assert_eq!(
        z4_channel_frames(&events)
            .iter()
            .map(|frame| frame.0)
            .collect::<Vec<_>>(),
        vec![blue]
    );
    // Nothing of red's net ever reached blue.
    assert!(
        !z4_channel_frames(&rig.log)
            .iter()
            .any(|frame| frame.0 == blue && frame.1 == "RedNet"),
        "a red line reached blue"
    );
    // And blue cannot find it to ask.
    let said = rig.say(1, "clist");
    assert!(!said.contains("RedNet"), "{said}");
    assert!(said.contains("BlueNet"), "{said}");
    assert_eq!(rig.say(1, "@channel/who RedNet"), "No such channel.");
    assert_eq!(rig.say(1, "addcom rn=RedNet"), "No such channel.");
}

/// **`EXIT` takes the pilot off their net, once** (16b cut Z.4), and what blue
/// can read about red's net — nothing — reads the same before and after.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_exit_takes_the_pilot_off_their_net_once() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let before = (rig.say(1, "clist"), rig.say(1, "@channel/who RedNet"));
    let said = rig.say(0, "exit");
    assert!(said.contains(RED_REMOVED), "{said}");
    assert!(said.contains("You climb out of"), "{said}");
    assert_eq!(z4_count(&rig, rig.pilot(0), RED_REMOVED), 1);
    assert!(z4_members(&rig, PartyId::RED).is_empty());
    assert_eq!(z4_members(&rig, PartyId::BLUE), vec![rig.pilot(1)]);
    let after = (rig.say(1, "clist"), rig.say(1, "@channel/who RedNet"));
    assert_eq!(before, after, "the other side reads nothing of the leave");
    // Off the net, the alias means nothing any more.
    let events = rig.events(0, "ra anybody");
    assert!(z4_channel_frames(&events).is_empty());
}

/// **The check-in takes everybody off, and a rematch joins them again** (16b cut
/// Z.4): the end hands every seat back through the one seam, so each pilot reads
/// the removal line once; the next match joins the same pilots under the same
/// callsigns.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_the_check_in_takes_everybody_off_and_a_rematch_joins_them_again() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    rig.say(0, "sim abort");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    assert!(z4_members(&rig, PartyId::RED).is_empty());
    assert!(z4_members(&rig, PartyId::BLUE).is_empty());
    assert_eq!(z4_count(&rig, rig.pilot(0), RED_REMOVED), 1);
    assert_eq!(z4_count(&rig, rig.pilot(1), BLUE_REMOVED), 1);

    rig.staff("@arena/reset");
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    rig.fly().await;
    assert_eq!(
        z4_on(&rig, 0, PartyId::RED),
        Some(("ra".to_owned(), Some("RA".to_owned())))
    );
    assert_eq!(
        z4_on(&rig, 1, PartyId::BLUE),
        Some(("ba".to_owned(), Some("BA".to_owned())))
    );
    assert_eq!(z4_count(&rig, rig.pilot(0), RED_JOINED), 2, "one per match");
}

/// **A machine-only side has nobody on its net, and a stale member is swept**
/// (16b cut Z.4): a bystander planted on red's net and a red pilot planted on
/// blue's before the check-out are both gone after it, the red pilot is on red's
/// net under their own callsign, and blue — flown by machines alone — has nobody.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_a_machine_only_side_has_nobody_and_a_stale_member_is_swept() {
    let mut rig = Rig::boot_with(3, y1_config());
    z4_plant(&mut rig, 2, PartyId::RED, "zz");
    z4_plant(&mut rig, 0, PartyId::BLUE, "yy");
    assert_eq!(z4_members(&rig, PartyId::RED), vec![rig.pilot(2)]);
    rig.fly_with_ai(1, 0, 1).await;
    assert_eq!(z4_members(&rig, PartyId::RED), vec![rig.pilot(0)]);
    assert!(
        z4_members(&rig, PartyId::BLUE).is_empty(),
        "the machines' side has nobody on its net"
    );
    assert_eq!(z4_count(&rig, rig.pilot(2), RED_REMOVED), 1);
    assert_eq!(z4_count(&rig, rig.pilot(0), BLUE_REMOVED), 1);
    assert_eq!(
        z4_on(&rig, 0, PartyId::RED),
        Some(("ra".to_owned(), Some("RA".to_owned())))
    );
}

/// **A pilot already on their net under another alias is joined again under
/// their callsign** (16b cut Z.4): the forced join leaves an existing member as
/// it is, so the join repairs it by taking them off first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_a_pilot_on_their_net_under_a_stale_alias_is_joined_again_as_their_callsign() {
    let mut rig = Rig::boot(2);
    z4_plant(&mut rig, 0, PartyId::RED, "rb");
    rig.fly().await;
    assert_eq!(
        z4_on(&rig, 0, PartyId::RED),
        Some(("ra".to_owned(), Some("RA".to_owned())))
    );
    assert_eq!(z4_count(&rig, rig.pilot(0), RED_REMOVED), 1);
    assert_eq!(z4_count(&rig, rig.pilot(0), RED_JOINED), 1);
}

/// **A staff `@arena/start` match joins nobody** (16b cut Z.4): it shares
/// `match_::start` with the lobby and never passes through the lobby's
/// check-out, which is where the join is.
///
/// The case that matters is a **human seated by staff**: nothing inside `start`
/// tells a lobby pilot from a staff-seated one (both are checked out with their
/// unit, and every unit carries a `Designator::Slot`), so a join moved into
/// `start` or `lobby::adopt` would put this pilot on `RedNet`. Pilot0 is seated
/// on red 1's tank the way a staffer's `@spatial/station` and `@spatial/assign`
/// seat one — the cockpit station declared and a seat activated through the
/// simulation's own API, no pod and no lobby — and is checked out with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_a_staff_started_match_joins_nobody() {
    let mut rig = Rig::boot(2);
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1).expect("blue 1's tank");
    let pilot = rig.pilot(0);
    // A console with no space of its own is reached from the room the tank
    // stands in, so the staffer's pilot walks there first.
    let bay = rig
        .tw
        .world()
        .get(red)
        .and_then(|tank| tank.location)
        .expect("the tank stands somewhere");
    rig.tw
        .world_mut()
        .move_to(pilot, bay)
        .expect("a pilot walks up to the tank");
    rig.with_ctx_mut(0, |ctx| {
        let station =
            mush_sim::StationId::new(match_::cockpit::STATION).expect("the cockpit station id");
        mush_sim::declare_station(
            ctx,
            red,
            &station,
            match_::cockpit::STATION_ROLE,
            &match_::cockpit::cockpit_channels(),
            None,
            match_::cockpit::STATION_PRIORITY,
        )
        .expect("a station on a tank nobody holds");
        mush_sim::request_seat_activation(
            ctx,
            &mush_sim::SeatRequest::Activate {
                unit: red,
                station,
                npc: pilot,
                granted: match_::cockpit::cockpit_channels(),
                fallback: None,
            },
        )
        .expect("a vacant seat takes its holder");
    });
    assert_eq!(
        rig.seats(0),
        vec![(red, match_::cockpit::STATION.to_owned())],
        "the staff seat took"
    );
    let said = rig.staff(&format!(
        "@arena/start {}=#{}/#{}",
        match_::CENTRE_ARENA,
        red.0,
        blue.0
    ));
    assert!(said.contains("is running on"), "{said}");
    // The seated human is really in the match: the arena sends them frames.
    assert!(rig.frame_for(0).await, "the staff-seated pilot is bound");
    assert!(z4_members(&rig, PartyId::RED).is_empty());
    assert!(z4_members(&rig, PartyId::BLUE).is_empty());
    assert_eq!(z4_on(&rig, 0, PartyId::RED), None);
    assert_eq!(z4_count(&rig, pilot, RED_JOINED), 0);
    assert!(
        !rig.log
            .iter()
            .any(|(_, event)| mush_testkit::text_of(event).contains("You have been joined")),
        "nobody was told of a join"
    );
    rig.staff("@arena/abort");
}

/// **A drop past the grace keeps the pilot on their net, through the reclaim**
/// (16b cut Z.4): the drop does not hand the seat back, so nothing leaves, and
/// the reclaim needs no rejoin — no removal line, and no second join line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_a_drop_past_the_grace_stays_on_the_net_through_the_reclaim() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    assert_eq!(
        z4_members(&rig, PartyId::RED),
        vec![pilot],
        "a drop is not a leave"
    );
    let now = rig.now;
    rig.tick_to(now + 5);
    let said = rig.say(0, "board");
    assert!(said.contains("is yours again where it stands"), "{said}");
    assert_eq!(
        z4_on(&rig, 0, PartyId::RED),
        Some(("ra".to_owned(), Some("RA".to_owned())))
    );
    assert_eq!(z4_count(&rig, pilot, RED_REMOVED), 0);
    assert_eq!(z4_count(&rig, pilot, RED_JOINED), 1);
}

/// **A pilot who dropped and never came back is still taken off at the
/// check-in** (16b cut Z.4): the drop keeps the billet (the seat hook releases
/// the focus, not the seat), so the check-in's `release_seat` still reaches
/// `hand_seat_back` for them and the net is empty after the match — nobody left
/// on it to hear the next match's red side.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z4_a_pilot_who_dropped_and_never_reclaimed_is_taken_off_at_the_check_in() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    let now = rig.now;
    rig.tick_to(now + 5);
    assert_eq!(
        z4_members(&rig, PartyId::RED),
        vec![pilot],
        "past the grace, still on the net"
    );
    // Blue calls it off; the dropped red pilot never boards again.
    rig.say(1, "sim abort");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    assert!(
        z4_members(&rig, PartyId::RED).is_empty(),
        "the dropped pilot is off red's net"
    );
    assert!(z4_members(&rig, PartyId::BLUE).is_empty());
    assert_eq!(z4_count(&rig, pilot, RED_REMOVED), 1);
    assert_eq!(z4_count(&rig, rig.pilot(1), BLUE_REMOVED), 1);
}

// ---------------------------------------------------------------------------
// 16b cut I.1 — the replay fold and `SIM REPLAY` in line mode (16c N9, Q37)
// ---------------------------------------------------------------------------

/// The pilot index [`i1_finished`] boots as the bystander: in the lobby, no slot.
const I1_BYSTANDER: usize = 2;

/// **A finished match** (16b cut I.1): pilots 0 and 1 fly, the arena runs long
/// enough for several keyframes at the rig's 100 ms summaries, and a real `SIM
/// ABORT` ends it through the arena's own `Ended`. Answers the match id.
async fn i1_finished(rig: &mut Rig) -> tacsim::arena_task::MatchId {
    rig.fly().await;
    let match_id = rig.centre().match_id.expect("a running match");
    // Several summaries' worth of arena time. The tape reaches the world a
    // closed chunk at a time (16 KiB), and the task sends every chunk it still
    // holds before its `Ended`, so the clock is what is waited on here.
    let start_ms = rig.staged_now_ms;
    for _ in 0..600 {
        rig.pump();
        if rig.staged_now_ms >= start_ms + 700 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        rig.staged_now_ms >= start_ms + 700,
        "the arena clock reached only {} ms",
        rig.staged_now_ms
    );
    rig.say(0, "sim abort");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    match_id
}

/// A read of the match book.
fn i1_book<T>(rig: &Rig, read: impl FnOnce(&match_::MatchBook) -> T) -> T {
    read(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .expect("the match book"),
    )
}

/// A match's record, as the book still holds it after its check-in.
fn i1_record(rig: &Rig, match_id: tacsim::arena_task::MatchId) -> match_::MatchRecord {
    i1_book(rig, |book| book.get(match_id).cloned()).expect("the book holds the record")
}

/// The replay the ring holds for `match_id`.
fn i1_replay(rig: &Rig, match_id: tacsim::arena_task::MatchId) -> Arc<match_::replay::Replay> {
    i1_book(rig, |book| book.replay(match_id)).expect("the ring holds the replay")
}

/// Every `arena.replay` panel in `events`.
fn i1_windows(events: &[(Audience, OutputEvent)]) -> Vec<serde_json::Value> {
    events
        .iter()
        .filter_map(|(_, event)| match event {
            OutputEvent::Panel { feed, data, .. } if feed.as_str() == "arena.replay" => {
                Some(data.clone())
            }
            _ => None,
        })
        .collect()
}

/// `SIM REPLAY <rest>` as pilot `n`: `(the lines they read, the windows pushed)`.
fn i1_replay_as(rig: &mut Rig, n: usize, rest: &str) -> (String, Vec<serde_json::Value>) {
    let actor = rig.pilot(n);
    let events = rig.events_of(actor, &format!("sim replay {rest}"));
    (
        rig.tw.seen_by(&events, actor).join("\n"),
        i1_windows(&events),
    )
}

/// The window corner `replay_window` clamps to around a cell (39 by 20 on the
/// shipped 256 by 256 ground).
fn i1_corner(cell: (u16, u16)) -> serde_json::Value {
    let x0 = cell.0.saturating_sub(19).min(256 - 39);
    let y0 = cell.1.saturating_sub(10).min(256 - 20);
    serde_json::json!([x0, y0])
}

/// A roster entry's cell at one keyframe.
fn i1_cell_at(replay: &match_::replay::Replay, index: usize, at: u16) -> (u16, u16) {
    replay
        .keyframes
        .get(index)
        .and_then(|keyframe| keyframe.units.get(usize::from(at)))
        .copied()
        .flatten()
        .map(|unit| (unit.x, unit.y))
        .expect("the unit is in the keyframe")
}

/// **`SIM REPLAY` in line mode** (16b cut I.1; 16c N9, Q37): the check-in
/// writes the document and holds it on the ring with its ground; a bystander
/// with no slot is shown the header, ONE `grid` panel on `arena.replay` that
/// carries `retain: latest` and `you: null`, and `Next:`; `next`, `prev`,
/// `last`, a time and a callsign step and centre it; a pilot who flew is centred
/// on their own tank; the two queries answer by the actor's cursor.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i1_sim_replay_prints_steps_centres_and_the_queries_read_the_cursor() {
    let mut rig = Rig::boot(3);
    let match_id = i1_finished(&mut rig).await;
    let id = match_id.0;

    // The check-in's writes: the replay document, then the result that says so.
    let writes: Vec<(String, serde_json::Value)> = rig
        .pump_ops()
        .into_iter()
        .filter(|op| op.ns == tacsim::NS)
        .filter_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => Some((key.to_string(), value)),
            _ => None,
        })
        .collect();
    let at = |key: &str| writes.iter().position(|(held, _)| held == key);
    let replay_at = at(&format!("tacsim:replay:{id}")).expect("the check-in wrote the replay");
    let result_at = at(&format!("tacsim:match:{id}")).expect("the check-in wrote the result");
    assert!(
        replay_at < result_at,
        "the replay rides the result's batch, first"
    );
    let (_, result) = writes.get(result_at).expect("the result");
    assert_eq!(result["replay"], serde_json::json!(true));
    let replay = i1_replay(&rig, match_id);
    let (_, document) = writes.get(replay_at).expect("the replay");
    println!("i1 replay document: {document}");
    assert_eq!(
        document,
        &replay.to_document(),
        "the ring holds what was written"
    );
    assert!(
        replay.keyframes.len() >= 5,
        "{} keyframes",
        replay.keyframes.len()
    );
    assert_eq!(
        replay.keyframes.first().map(|keyframe| keyframe.t_ms),
        Some(0)
    );
    // **`start`'s seeding, against the arena's own record** (the fixer's T5):
    // the rig's tanks stand idle, so keyframe 0 — the check-out poses `start`
    // captured before the custody crossed the seam — is exactly where the
    // arena's first position summary (keyframe 1) finds them.
    assert_eq!(
        replay
            .keyframes
            .first()
            .map(|keyframe| keyframe.units.clone()),
        replay
            .keyframes
            .get(1)
            .map(|keyframe| keyframe.units.clone()),
        "keyframe 0 is the opening the arena's own first summary confirms"
    );
    assert!(
        replay
            .keyframes
            .first()
            .is_some_and(|keyframe| keyframe.units.iter().all(Option::is_some)),
        "every unit is in keyframe 0"
    );
    // And the roster: every unit the record names, once, each on the side its
    // slot callsign says (`RA` red, `BA` blue) — not the side `start` read.
    let callsigns: Vec<(String, &str)> = replay
        .roster
        .iter()
        .map(|unit| (unit.callsign.to_string(), unit.team.as_str()))
        .collect();
    assert_eq!(
        callsigns,
        vec![("RA".to_owned(), "red"), ("BA".to_owned(), "blue")],
        "the roster in unit-id order, each on its own side"
    );
    assert_eq!(
        replay.roster.len(),
        i1_record(&rig, match_id).callsigns.len()
    );
    assert_eq!(replay.reason.as_str(), "abort");
    // The ground, held under the digest, before `untrack` dropped it.
    let digest = i1_record(&rig, match_id).arena_definition_digest;
    assert_eq!(replay.arena_digest, digest);
    assert!(i1_book(&rig, |book| book.ground(&digest).is_some()));
    assert_eq!(i1_book(&rig, match_::MatchBook::ground_count), 1);

    // Before anybody types it: no replay open.
    let bystander = rig.pilot(I1_BYSTANDER);
    let asked = rig.query(bystander, "arena.replay");
    assert_eq!(asked["available"], serde_json::json!(false));
    assert_eq!(
        asked["reason"],
        serde_json::json!("You have no replay open; type SIM REPLAY <match>.")
    );
    assert_eq!(
        rig.query(bystander, "arena.map")["seated"],
        serde_json::json!(false)
    );

    // A bare `SIM REPLAY <id>` by the bystander: keyframe 0, around the first
    // roster entry.
    let first = replay.roster.first().expect("a roster").callsign.clone();
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &id.to_string());
    let header = format!(
        "Replay of match {id} at 0:00, keyframe 1 of {} ({}, aborted): both sides as the arena \
         recorded them every 100 ms, around {first}.",
        replay.keyframes.len(),
        match_::duration_words(replay.end_ms)
    );
    assert!(said.lines().any(|line| line == header), "{said}");
    assert!(
        said.lines()
            .any(|line| line == format!("Next: SIM REPLAY {id} next")),
        "{said}"
    );
    assert_eq!(windows.len(), 1, "one panel: {windows:?}");
    let window = windows.first().expect("the window");
    assert_eq!(window["schema"]["shape"], serde_json::json!("grid"));
    assert_eq!(window["schema"]["retain"], serde_json::json!("latest"));
    assert_eq!(window["you"], serde_json::Value::Null);
    assert_eq!(window["match"], serde_json::json!(id));
    assert_eq!(window["keyframe"]["index"], serde_json::json!(0));
    assert_eq!(window["origin_cell"], i1_corner(i1_cell_at(&replay, 0, 0)));
    let cursor = i1_book(&rig, |book| book.replay_cursor(bystander)).expect("a cursor");
    assert_eq!(
        (cursor.match_id, cursor.index, cursor.focus),
        (match_id, 0, 0)
    );

    // The queries now answer the replay and its ground to this actor.
    let asked = rig.query(bystander, "arena.replay");
    assert_eq!(asked["schema"]["shape"], serde_json::json!("replay"));
    assert_eq!(asked["record"], serde_json::json!("tacsim.replay"));
    assert_eq!(asked["match"], serde_json::json!(id));
    assert_eq!(asked["keyframes"], replay.to_document()["keyframes"]);
    let map = rig.query(bystander, "arena.map");
    assert_eq!(map["match"], serde_json::json!(id));
    assert_eq!(map["schema"]["shape"], serde_json::json!("terrain"));
    // …and nothing to a pilot who typed nothing.
    let blue = rig.pilot(1);
    assert_eq!(
        rig.query(blue, "arena.replay")["reason"],
        serde_json::json!("You have no replay open; type SIM REPLAY <match>.")
    );
    assert_eq!(
        rig.query(blue, "arena.map")["seated"],
        serde_json::json!(false)
    );
    // …and the centre's own sentence to somebody outside it.
    let wanderer = rig.tw.spawn_player("Wanderer");
    let asked = rig.query(wanderer, "arena.replay");
    assert_eq!(asked["available"], serde_json::json!(false));
    assert_eq!(asked["reason"], serde_json::json!(match_::OUTSIDE_CENTRE));

    // `next`, `prev`, `last`, a time: the cursor steps.
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} next"));
    assert!(said.contains(", keyframe 2 of "), "{said}");
    assert_eq!(
        windows.first().map(|w| w["keyframe"]["index"].clone()),
        Some(serde_json::json!(1))
    );
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("#{id} prev"));
    assert!(said.contains(", keyframe 1 of "), "{said}");
    let last = replay.keyframes.len() - 1;
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} last"));
    assert!(
        said.contains(&format!(", keyframe {} of {}", last + 1, last + 1)),
        "{said}"
    );
    assert!(
        !said.contains("Next:"),
        "the last keyframe owes no Next: {said}"
    );
    assert_eq!(
        windows.first().map(|w| w["keyframe"]["index"].clone()),
        Some(serde_json::json!(last))
    );
    // The end of the tape: refused, and the cursor stays where it was.
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} next"));
    assert_eq!(said, format!("That is the last keyframe of match {id}."));
    assert!(windows.is_empty());
    assert_eq!(
        i1_book(&rig, |book| book.replay_cursor(bystander)).map(|cursor| cursor.index),
        Some(last)
    );
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} 0:00"));
    assert!(
        said.contains(", keyframe 1 of "),
        "a time: the first keyframe at or after it: {said}"
    );
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} prev"));
    assert_eq!(said, format!("That is the first keyframe of match {id}."));
    assert!(i1_book(&rig, |book| book.replay_cursor(bystander)).is_some());
    // A time past the tape: the last keyframe.
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} 59:59"));
    assert!(
        said.contains(&format!(", keyframe {} of", last + 1)),
        "{said}"
    );

    // A callsign centres the window on that unit.
    let (other, other_at) = replay
        .roster
        .iter()
        .enumerate()
        .skip(1)
        .map(|(at, unit)| (unit.callsign.to_string(), u16::try_from(at).unwrap_or(0)))
        .next()
        .expect("a second roster entry");
    let (said, windows) = i1_replay_as(
        &mut rig,
        I1_BYSTANDER,
        &format!("{id} first {}", other.to_ascii_lowercase()),
    );
    assert!(said.contains(&format!(", around {other}.")), "{said}");
    let window = windows.first().expect("the window");
    assert_eq!(
        window["origin_cell"],
        i1_corner(i1_cell_at(&replay, 0, other_at))
    );
    assert_eq!(window["you"], serde_json::Value::Null);
    // …and the focus sticks with the cursor through a step.
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} next"));
    assert!(said.contains(&format!(", around {other}.")), "{said}");

    // A pilot who flew is centred on their own tank.
    let record = i1_record(&rig, match_id);
    let flown = record
        .teams
        .iter()
        .flat_map(|roster| roster.pilots.iter())
        .find(|(pilot, _)| *pilot == rig.pilot(0))
        .map(|(_, unit)| *unit)
        .expect("pilot 0 flew a tank");
    let own = record.callsign_word(flown);
    let (said, _) = i1_replay_as(&mut rig, 0, &id.to_string());
    assert!(said.contains(&format!(", around {own}.")), "{said}");

    // The refusals, verbatim.
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} zz"));
    assert_eq!(said, format!("No unit of match {id} flew as ZZ."));
    assert_eq!(
        i1_book(&rig, |book| book.replay_cursor(bystander)),
        None,
        "a refused SIM REPLAY drops the cursor"
    );
    assert_eq!(
        rig.query(bystander, "arena.replay")["reason"],
        serde_json::json!("You have no replay open; type SIM REPLAY <match>.")
    );
    assert_eq!(
        rig.query(bystander, "arena.map")["seated"],
        serde_json::json!(false)
    );
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, "");
    assert_eq!(
        said,
        "Usage: SIM REPLAY <match> [<m:ss> | next | prev] [<callsign>]"
    );
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &format!("{id} 0:01 0:02"));
    assert_eq!(
        said,
        "Usage: SIM REPLAY <match> [<m:ss> | next | prev] [<callsign>]"
    );
    // A forgotten id drops the cursor too.
    i1_replay_as(&mut rig, I1_BYSTANDER, &id.to_string());
    assert!(i1_book(&rig, |book| book.replay_cursor(bystander)).is_some());
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, "99");
    assert_eq!(
        said,
        "The centre remembers its last 20 matches; match 99 is not one of them."
    );
    assert_eq!(i1_book(&rig, |book| book.replay_cursor(bystander)), None);
    assert_eq!(
        rig.query(bystander, "arena.map")["seated"],
        serde_json::json!(false)
    );

    // A restart-abort row: the restart sentence.
    rig.with_ctx_mut(0, |ctx| {
        match_::write_restart_abort(ctx, tacsim::arena_task::MatchId(id + 50), &[ObjId(500)]);
    });
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &(id + 50).to_string());
    assert_eq!(
        said,
        format!(
            "The server restarted during match {}; there is no replay of it.",
            id + 50
        )
    );
    // A row written before replays existed: the before-replays sentence.
    let mut old = i1_book(&rig, |book| book.remembered(match_id).cloned()).expect("the row");
    old.id = tacsim::arena_task::MatchId(id + 40);
    old.replay = false;
    rig.tw
        .world_mut()
        .module_state_mut::<match_::MatchBook>()
        .expect("the book")
        .remember(old);
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &(id + 40).to_string());
    assert_eq!(
        said,
        format!(
            "Match {} was recorded before replays existed; there is nothing to watch of it.",
            id + 40
        )
    );

    // A second match RUNNING: its one refusal, and the cursor dropped by it.
    i1_replay_as(&mut rig, I1_BYSTANDER, &id.to_string());
    assert!(i1_book(&rig, |book| book.replay_cursor(bystander)).is_some());
    rig.staff("@arena/reset");
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    rig.fly().await;
    let running = rig.centre().match_id.expect("the second match");
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &running.0.to_string());
    assert_eq!(said, format!("Match {} is still running.", running.0));
    assert!(windows.is_empty());
    assert_eq!(i1_book(&rig, |book| book.replay_cursor(bystander)), None);
    assert_eq!(
        rig.query(bystander, "arena.replay")["reason"],
        serde_json::json!("You have no replay open; type SIM REPLAY <match>.")
    );
    assert_eq!(
        rig.query(bystander, "arena.map")["seated"],
        serde_json::json!(false)
    );
    // A seated pilot still reads their own match's map, whatever else is open.
    assert_eq!(
        rig.query(rig.pilot(0), "arena.map")["match"],
        serde_json::json!(running.0)
    );
}

/// The `tacsim` writes one `write_restart_abort` pushes, as `(set|del, key)`,
/// through a real `Ctx` on the rig's own world — `match_lifecycle.rs`'s
/// `restart_abort_writes`, which an integration test cannot import.
fn i1_restart_abort(rig: &mut Rig, match_id: u64) -> Vec<(&'static str, String)> {
    let mut out = mush_core::Outbox::new();
    let engine = rig.tw.engine_mut();
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
    match_::write_restart_abort(
        &mut ctx,
        tacsim::arena_task::MatchId(match_id),
        &[ObjId(500)],
    );
    out.take_deferred()
        .into_iter()
        .filter_map(|deferred| match deferred {
            mush_core::Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .filter_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet { key, .. } => Some(("set", key.to_string())),
            mush_core::ModuleWrite::Del { key } => Some(("del", key.to_string())),
            _ => None,
        })
        .collect()
}

/// **The ring bounds the replays and their grounds** (16b cut I.1): two
/// remembered matches on one ground hold ONE ground; the eviction of their
/// rows deletes each replay document in the same write, forgets the replay and
/// every cursor on it, and drops the ground once no remembered replay names it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i1_the_ring_evicts_replays_cursors_and_grounds() {
    let mut rig = Rig::boot(3);
    let first = i1_finished(&mut rig).await;
    rig.staff("@arena/reset");
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    let second = i1_finished(&mut rig).await;
    assert!(i1_book(&rig, |book| book.replay(first).is_some()
        && book.replay(second).is_some()));
    assert_eq!(
        i1_book(&rig, match_::MatchBook::ground_count),
        1,
        "two matches on one ground hold one copy of it"
    );
    let bystander = rig.pilot(I1_BYSTANDER);
    i1_replay_as(&mut rig, I1_BYSTANDER, &first.0.to_string());
    i1_replay_as(&mut rig, 0, &first.0.to_string());
    assert!(i1_book(&rig, |book| book.replay_cursor(bystander)).is_some());

    // Eighteen newer results fill the ring; the nineteenth pushes `first` out.
    let base = second.0 + 100;
    for n in 0..18 {
        let writes = i1_restart_abort(&mut rig, base + n);
        assert!(writes.iter().all(|(what, _)| *what == "set"), "{writes:?}");
    }
    assert!(
        i1_book(&rig, |book| book.replay(first).is_some()),
        "twenty rows, both held"
    );
    let writes = i1_restart_abort(&mut rig, base + 18);
    assert!(
        writes.contains(&("del", format!("tacsim:replay:{}", first.0))),
        "the eviction deleted the replay document in the same write: {writes:?}"
    );
    assert!(i1_book(&rig, |book| book.replay(first).is_none()));
    assert_eq!(i1_book(&rig, |book| book.replay_cursor(bystander)), None);
    assert_eq!(i1_book(&rig, |book| book.replay_cursor(rig.pilot(0))), None);
    assert_eq!(
        i1_book(&rig, match_::MatchBook::ground_count),
        1,
        "the second match still names the ground"
    );
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &first.0.to_string());
    assert_eq!(
        said,
        format!(
            "The centre remembers its last 20 matches; match {} is not one of them.",
            first.0
        )
    );
    // And the next pushes `second` out: no replay names the ground any more.
    let writes = i1_restart_abort(&mut rig, base + 19);
    assert!(writes.contains(&("del", format!("tacsim:replay:{}", second.0))));
    assert!(i1_book(&rig, |book| book.replay(second).is_none()));
    assert_eq!(i1_book(&rig, match_::MatchBook::ground_count), 0);
}

/// **A replay on a ground that is not the one loaded** (16b cut I.1): a replay
/// hand-edited on the book to name a foreign digest, its ground not held,
/// answers the ground sentence; the build is tried once and remembered in the
/// mismatch set, so a second ask answers the same without building again; and
/// the replay's own ground, dropped as a boot drops it, is built back by the
/// first `SIM REPLAY` and makes the map answerable.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i1_a_foreign_ground_is_refused_and_built_at_most_once() {
    let mut rig = Rig::boot(3);
    let match_id = i1_finished(&mut rig).await;
    let id = match_id.0;
    let replay = i1_replay(&rig, match_id);
    let digest = replay.arena_digest.clone();

    // As after a boot: the ground is not held until somebody asks.
    {
        let book = rig
            .tw
            .world_mut()
            .module_state_mut::<match_::MatchBook>()
            .expect("the book");
        book.drop_ground(&digest);
    }
    assert_eq!(i1_book(&rig, match_::MatchBook::ground_count), 0);
    let bystander = rig.pilot(I1_BYSTANDER);
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &id.to_string());
    assert!(said.contains("both sides"), "{said}");
    assert_eq!(windows.len(), 1);
    assert!(
        i1_book(&rig, |book| book.ground(&digest).is_some()),
        "built once, kept"
    );
    assert_eq!(
        rig.query(bystander, "arena.map")["match"],
        serde_json::json!(id)
    );

    // A foreign digest: the ground loaded is not the ground it names.
    let foreign = "0".repeat(64);
    {
        let mut edited = (*replay).clone();
        edited.arena_digest.clone_from(&foreign);
        let book = rig
            .tw
            .world_mut()
            .module_state_mut::<match_::MatchBook>()
            .expect("the book");
        book.hold_replay(match_id, Arc::new(edited));
        book.drop_ground(&digest);
    }
    let sentence = format!("The ground of match {id} is not the ground loaded on this server.");
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &id.to_string());
    assert_eq!(said, sentence);
    assert!(windows.is_empty());
    assert!(i1_book(&rig, |book| book.ground_mismatched(&foreign)));
    assert_eq!(i1_book(&rig, |book| book.replay_cursor(bystander)), None);
    assert_eq!(
        i1_book(&rig, match_::MatchBook::ground_count),
        0,
        "a mismatch keeps nothing"
    );
    // Asked again: the mismatch set answers, and nothing is built.
    let (said, _) = i1_replay_as(&mut rig, I1_BYSTANDER, &id.to_string());
    assert_eq!(said, sentence);
    assert_eq!(i1_book(&rig, match_::MatchBook::ground_count), 0);
    assert_eq!(
        rig.query(bystander, "arena.map")["seated"],
        serde_json::json!(false)
    );

    // **The mark alone refuses** (the fixer's T2): the replay put back under
    // its REAL digest — whose build would succeed, as the first half proved —
    // with that digest marked mismatched and its ground not held. Only the
    // mismatch set's short-circuit can answer the ground sentence here; a
    // build would find the loaded ground equal and keep it.
    {
        let book = rig
            .tw
            .world_mut()
            .module_state_mut::<match_::MatchBook>()
            .expect("the book");
        book.hold_replay(match_id, Arc::clone(&replay));
        book.drop_ground(&digest);
        book.mark_ground_mismatch(&digest);
    }
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &id.to_string());
    assert_eq!(said, sentence, "a marked digest is never built again");
    assert!(windows.is_empty());
    assert!(
        i1_book(&rig, |book| book.ground(&digest).is_none()),
        "nothing was built under the marked digest"
    );
    assert_eq!(i1_book(&rig, match_::MatchBook::ground_count), 0);
}

// ---------------------------------------------------------------------------
// 16b cut I.3 — `@arena/tempo` and `@arena/place` (16c N10)
// ---------------------------------------------------------------------------

/// [`config`] with every tempo at 500 ms — the shipped `engaged_ms`, so the
/// floor's bounds read `500 to 10000 ms in steps of 500` and every wake of a
/// 5,000 ms floor moves exactly `4 × 500` = 2,000 ms of arena time, whatever
/// tempo the match is in (a Rig match is not reliably `Engaged`, and this makes
/// the three tempos one).
fn i3_config() -> TacsimConfig {
    let mut cfg = config();
    cfg.arena.tempo.quiet_ms = 500;
    cfg.arena.tempo.alert_ms = 500;
    cfg.arena.tempo.engaged_ms = 500;
    cfg
}

/// The running match's id.
fn i3_match(rig: &Rig) -> u64 {
    rig.centre().match_id.expect("a running match").0
}

/// The registry's mirror of `match_id`'s staff floor.
fn i3_mirrored_floor(rig: &Rig, match_id: u64) -> Option<u32> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(tacsim::arena_task::MatchId(match_id)))
        .and_then(|mirror| mirror.tempo_floor_ms)
}

/// The arena clock the rig has seen at `at` of tokio time, after pumping.
async fn i3_clock_at(rig: &mut Rig, at: tokio::time::Instant) -> i64 {
    tokio::time::sleep_until(at).await;
    rig.pump();
    rig.staged_now_ms
}

/// **16b cut I.3**: `@arena/tempo` sets, reads and lifts a floor, answers each
/// refusal in its own sentence, writes the mirror only after the task took the
/// message, and `@arena/status` prints the floor on a line of its own while
/// one stands.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn i3_arena_tempo_sets_reads_lifts_and_refuses() {
    let mut rig = Rig::boot_with(2, i3_config());
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let id = i3_match(&rig);

    // The refusals, verbatim.
    let usage = "Usage: @arena/tempo <match>=<period_ms> | <match>=off | <match>";
    assert_eq!(rig.staff("@arena/tempo"), usage);
    assert_eq!(rig.staff(&format!("@arena/tempo {id}=fast")), usage);
    let bounds = "A tempo floor is 500 to 10000 ms in steps of 500, or off.";
    for wrong in ["750", "0", "250", "10500", "20000"] {
        assert_eq!(
            rig.staff(&format!("@arena/tempo {id}={wrong}")),
            bounds,
            "{wrong}"
        );
    }
    assert_eq!(rig.staff("@arena/tempo 99=5000"), "There is no match 99.");

    // The bare read, with nothing standing, and a status with no floor line.
    assert_eq!(
        rig.staff(&format!("@arena/tempo {id}")),
        format!("Match {id} has no tempo floor.")
    );
    assert!(!rig.staff("@arena/status").contains("tempo floor"));

    // A refused offer leaves the mirror as it was: fill the task's inbox to its
    // reserve (this runtime has not yielded, so the task has drained nothing).
    let tasks = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .expect("the registry");
    let tank = rig.tank(0);
    let mut seq = 1_000_000_u64;
    while tasks
        .offer(
            tacsim::arena_task::MatchId(id),
            tacsim::arena_task::ArenaMsg::Intent(tacsim::arena::intents::Intent {
                unit: tank,
                channel: Channel::Helm,
                epoch: rig.epoch(tank, Channel::Helm),
                kind: tacsim::arena::intents::IntentKind::Speed { cms: 0 },
                seq,
            }),
        )
        .is_ok()
    {
        seq += 1;
        assert!(seq < 1_010_000, "the inbox never filled");
    }
    assert_eq!(
        rig.staff(&format!("@arena/tempo {id}=5000")),
        "That match is busy; try again in a moment."
    );
    assert_eq!(
        i3_mirrored_floor(&rig, id),
        None,
        "a refused offer wrote the mirror"
    );
    assert!(!rig.staff("@arena/status").contains("tempo floor"));
    // Let the task drain its inbox.
    tokio::time::sleep(std::time::Duration::from_millis(600)).await;
    rig.pump();

    // The set, and what it says: every tempo is 500 ms here, so all three
    // tempos move 2,000 ms of arena time a 5,000 ms wake.
    let floored = format!(
        "Match {id} under a floor of 5000 ms: engaged 2000 ms of arena time a wake every 5000 \
         ms, alert 2000 every 5000, quiet 2000 every 5000; orders wait for the next wake. \
         @arena/tempo {id}=off lifts it."
    );
    assert_eq!(rig.staff(&format!("@arena/tempo {id}=5000")), floored);
    assert_eq!(i3_mirrored_floor(&rig, id), Some(5_000));
    assert_eq!(
        rig.staff(&format!("@arena/tempo {id}")),
        floored,
        "the read"
    );
    let status = rig.staff("@arena/status");
    assert!(status.contains("\n  tempo floor: 5000 ms"), "{status}");
    // The match row itself is untouched: `mushd`'s support still parses it.
    let row = status
        .lines()
        .find(|line| line.starts_with(&format!("match {id}:")))
        .unwrap_or_else(|| panic!("the match row: {status}"));
    assert!(row.contains(", clock "), "{row}");

    // The lift.
    assert_eq!(
        rig.staff(&format!("@arena/tempo {id}=OFF")),
        format!("Match {id} runs at its own tempo again.")
    );
    assert_eq!(i3_mirrored_floor(&rig, id), None);
    assert_eq!(
        rig.staff(&format!("@arena/tempo {id}")),
        format!("Match {id} has no tempo floor.")
    );
    assert!(!rig.staff("@arena/status").contains("tempo floor"));

    // A match that is stopping has no wake to floor, nor one that has ended.
    rig.staff(&format!("@arena/abort {id}"));
    assert_eq!(
        rig.staff(&format!("@arena/tempo {id}=5000")),
        format!("Match {id} is stopping.")
    );
    for _ in 0..400 {
        rig.pump();
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    assert_eq!(
        rig.staff(&format!("@arena/tempo {id}")),
        format!("Match {id} is aborted.")
    );
    // Nobody but the caller heard of any of it.
    for n in 0..2 {
        let heard = rig.tw.seen_by(&rig.log, rig.pilot(n)).join("\n");
        assert!(
            !heard.contains("floor") && !heard.contains("tempo"),
            "pilot {n} was told: {heard}"
        );
    }
}

/// **16b cut I.3**: under a 5,000 ms floor the task wakes every 5,000 ms of
/// tokio time and each wake moves 2,000 ms of arena time; the set, a lift and
/// an abort each take effect within a millisecond (the deadline pull-in); and
/// nothing lags — no `Lagged` notice, no `lagged` line. The tape carries the
/// three changes.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn i3_a_floor_slows_the_task_and_a_lift_and_an_abort_are_prompt() {
    use tokio::time::{Duration, Instant};
    let mut rig = Rig::boot_with(2, i3_config());
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let id = i3_match(&rig);
    tokio::time::sleep(Duration::from_millis(3_000)).await;
    rig.pump();

    // The set, and its pull-in pulse a millisecond later.
    let t0 = Instant::now();
    assert!(
        rig.staff(&format!("@arena/tempo {id}=5000"))
            .contains("under a floor of 5000 ms")
    );
    let pulled = i3_clock_at(&mut rig, t0 + Duration::from_millis(3)).await;
    // Nothing moves until the floor's first wake, a whole floor later.
    let before = i3_clock_at(&mut rig, t0 + Duration::from_millis(4_999)).await;
    assert_eq!(before, pulled, "the task woke inside the floor");
    let first = i3_clock_at(&mut rig, t0 + Duration::from_millis(5_003)).await;
    assert!(first > before, "no wake at the floor");
    let still = i3_clock_at(&mut rig, t0 + Duration::from_millis(9_999)).await;
    assert_eq!(still, first, "the task woke inside the floor");
    let second = i3_clock_at(&mut rig, t0 + Duration::from_millis(10_003)).await;
    assert_eq!(second - first, 2_000, "one wake, the clamp of arena time");
    let third = i3_clock_at(&mut rig, t0 + Duration::from_millis(15_003)).await;
    assert_eq!(third - second, 2_000);

    // The lift, two seconds after the last floored wake: its pull-in pulse
    // runs within a millisecond and moves the arena — and is not lag, though
    // its interval began two seconds of wall ago under the floor.
    let t1 = t0 + Duration::from_millis(17_003);
    tokio::time::sleep_until(t1).await;
    rig.pump();
    assert_eq!(rig.staged_now_ms, third, "nothing woke before the lift");
    assert_eq!(
        rig.staff(&format!("@arena/tempo {id}=off")),
        format!("Match {id} runs at its own tempo again.")
    );
    let lifted = i3_clock_at(&mut rig, t1 + Duration::from_millis(2)).await;
    assert_eq!(
        lifted - third,
        2_000,
        "the lift's pulse, within a millisecond"
    );
    // And it runs at its own 500 ms again: in five seconds of wall it moves as
    // many of arena time (the rig sees the clock a whole arena second at a
    // time), where a floored match would have moved two.
    let running = i3_clock_at(&mut rig, t1 + Duration::from_millis(5_000)).await;
    assert!(running - lifted >= 4_000, "{running} after {lifted}");

    // A floor again, and an abort under it: the record parks within a
    // millisecond rather than a floor later.
    assert!(
        rig.staff(&format!("@arena/tempo {id}=10000"))
            .contains("under a floor of 10000 ms")
    );
    let t2 = Instant::now();
    tokio::time::sleep_until(t2 + Duration::from_millis(5)).await;
    rig.pump();
    let t3 = Instant::now();
    rig.staff(&format!("@arena/abort {id}"));
    tokio::time::sleep_until(t3 + Duration::from_millis(2)).await;
    rig.pump();
    assert_eq!(
        rig.state(),
        MatchState::Aborted,
        "the abort waited out the floor"
    );

    // Nothing lagged.
    let lag_notices = rig
        .tw
        .seen_by(&rig.log, GOD)
        .into_iter()
        .filter(|line| line.contains("is lagging"))
        .count();
    assert_eq!(lag_notices, 0, "a slowed match raised a Lagged notice");
    assert!(
        !rig.tape.iter().any(|event| event.kind.as_str() == "lagged"),
        "a slowed match journalled lag"
    );
    let floors: Vec<Option<u32>> = rig
        .tape
        .iter()
        .filter_map(|event| match event.kind {
            tacsim::arena::ledger::EventKind::TempoFloor { period_ms } => Some(period_ms),
            _ => None,
        })
        .collect();
    assert_eq!(floors, vec![Some(5_000), None, Some(10_000)]);
}

/// A passable cell with a way out within three cells of `near`, held by nobody
/// in `taken` — where a place is accepted.
fn i3_open_cell(rig: &Rig, near: (u16, u16), taken: &[(u16, u16)]) -> (u16, u16) {
    let registry = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .expect("the registry");
    let ground = registry
        .arena_of(tacsim::arena_task::MatchId(i3_match(rig)))
        .expect("the running match's ground");
    for dy in -3_i32..=3 {
        for dx in -3_i32..=3 {
            let (Ok(x), Ok(y)) = (
                u16::try_from(i32::from(near.0) + dx),
                u16::try_from(i32::from(near.1) + dy),
            ) else {
                continue;
            };
            if (x, y) != near && !taken.contains(&(x, y)) && ground.terrain.has_way_out(x, y) {
                return (x, y);
            }
        }
    }
    panic!("no open cell near {near:?}");
}

/// **16b cut I.3**: `@arena/place` moves a piloted tank — the pilot's next
/// frame stands on the placed cell and carries no notice — and the tape says
/// `placed`; a place the arena refuses is `place_refused` on the tape and
/// nothing at all to the pilot; the world's own refusals answer in
/// `@arena/pose`'s and `@arena/intent`'s sentences and reach no tape.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn i3_arena_place_moves_a_tank_says_nothing_and_the_tape_records_it() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    assert!(rig.frame_for(1).await, "the arena sends the other a frame");
    let tank = rig.tank(0);
    let other = rig
        .held_frame(1)
        .cell
        .expect("the other tank is on the window");
    let from = rig.held_frame(0).cell.expect("the tank is on the window");
    let to = i3_open_cell(&rig, from, &[other]);

    // The world's refusals, verbatim, and the usage.
    assert_eq!(
        rig.staff(&format!("@arena/place #{}=1", tank.0)),
        "Usage: @arena/place <unit>=<x>,<y>"
    );
    assert_eq!(
        rig.staff(&format!("@arena/place #{}=9999,5", tank.0)),
        "Cell (9999, 5) is off the arena window."
    );
    let ground = {
        let registry = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the registry");
        registry
            .arena_of(tacsim::arena_task::MatchId(i3_match(&rig)))
            .expect("the ground")
            .terrain
            .clone()
    };
    let walled = (0..ground.height())
        .flat_map(|y| (0..ground.width()).map(move |x| (x, y)))
        .find(|(x, y)| !ground.has_way_out(*x, *y))
        .expect("the proving ground has a cell no tank may stand on");
    assert_eq!(
        rig.staff(&format!(
            "@arena/place #{}={} {}",
            tank.0, walled.0, walled.1
        )),
        format!(
            "Cell ({}, {}) is impassable, or has no way out.",
            walled.0, walled.1
        )
    );
    assert_eq!(
        rig.staff(&format!("@arena/place #{}=3,3", rig.pilot(0).0)),
        "That unit is not in a running match."
    );

    // The place.
    let heard_from = rig.log.len();
    assert_eq!(
        rig.staff(&format!("@arena/place #{}={},{}", tank.0, to.0, to.1)),
        format!(
            "Placing #{} at cell ({}, {}): the arena answers at its next pulse and the tape \
             records it.",
            tank.0, to.0, to.1
        )
    );
    let mut moved = None;
    for _ in 0..400 {
        rig.pump();
        if let Some(frame) = rig.frame(0).filter(|frame| frame.cell == Some(to)) {
            moved = Some(frame);
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let moved = moved.expect("the pilot's frame never reached the placed cell");
    assert!(
        moved.notices.is_empty(),
        "the place was said: {:?}",
        moved.notices
    );
    assert!(
        rig.tape_until(|event| event.kind.as_str() == "placed")
            .await,
        "the tape never carried the place"
    );

    // A place the arena refuses: onto the other tank.
    assert!(
        rig.staff(&format!("@arena/place #{}={},{}", tank.0, other.0, other.1))
            .starts_with("Placing")
    );
    assert!(
        rig.tape_until(|event| event.kind.as_str() == "place_refused")
            .await,
        "the tape never carried the refusal"
    );
    let refused = rig
        .tape
        .iter()
        .find(|event| event.kind.as_str() == "place_refused")
        .map(|event| event.kind.clone());
    assert_eq!(
        refused,
        Some(tacsim::arena::ledger::EventKind::PlaceRefused {
            unit: tank,
            x: other.0,
            y: other.1,
            reason: smol_str::SmolStr::new_static("occupied"),
        })
    );
    assert_eq!(
        rig.frame(0).and_then(|frame| frame.cell),
        Some(to),
        "the refused place moved the tank"
    );

    // The pilot heard nothing of either: no line, no notice, no refusal.
    let since = rig.log.get(heard_from..).unwrap_or_default().to_vec();
    let heard = rig.tw.seen_by(&since, rig.pilot(0)).join("\n");
    for word in ["place", "Placing", "in_flight", "refused", "occupied"] {
        assert!(!heard.contains(word), "the pilot heard `{word}`: {heard}");
    }
    for (_, event) in &since {
        if let OutputEvent::Panel { feed, data, .. } = event
            && feed.as_str() == tacsim::arena_bridge::FRAME_FEED
        {
            let notices = data
                .get("notices")
                .and_then(serde_json::Value::as_array)
                .map_or(0, Vec::len);
            assert_eq!(notices, 0, "a cockpit frame carried a notice: {data}");
        }
    }
    // Exactly the two places the arena received are on the tape: the world's
    // refusals never reached it.
    assert_eq!(
        rig.tape
            .iter()
            .filter(|event| matches!(event.kind.as_str(), "placed" | "place_refused"))
            .count(),
        2
    );

    // A match that is stopping takes no place (the fixer's R2): its `Stop` is
    // already on the way, and the world says so rather than promising a line
    // on a tape the arena may already have closed. The unit is still bound
    // until the check-in, so this is the book's word and not the intent
    // sentence.
    let id = i3_match(&rig);
    rig.staff(&format!("@arena/abort {id}"));
    assert_eq!(
        rig.staff(&format!("@arena/place #{}={},{}", tank.0, from.0, from.1)),
        format!("Match {id} is stopping.")
    );
}

// ---------------------------------------------------------------------------
// 16b cut I.4 — `@arena/watch` and the staff picture (16c N10)
// ---------------------------------------------------------------------------

/// `@arena/watch`'s refusal for anybody who flies or is about to, verbatim.
const I4_FLYING: &str = "You are flying, or about to; EXIT and leave your slot before you watch.";

/// What a watching staffer is told, verbatim.
fn i4_watching(id: u64) -> String {
    format!(
        "You are watching match {id}: its picture reaches your panel at most once an arena \
         second on arena.observer, and screen 0 draws it; the line client prints none of it. \
         @arena/watch off stops it."
    )
}

/// Give a character `SIM_ADMIN`.
fn i4_make_staff(rig: &mut Rig, who: ObjId) {
    let object = rig
        .tw
        .world_mut()
        .get_mut(who)
        .expect("the character exists");
    object.powers |= mush_core::Powers::SIM_ADMIN;
}

/// A new staffer with no account, no slot and no seat.
fn i4_staffer(rig: &mut Rig, name: &str) -> ObjId {
    let staffer = rig.tw.spawn_player(name);
    i4_make_staff(rig, staffer);
    staffer
}

/// One line as `who`, and what `who` read.
fn i4_as(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let events = rig.events_of(who, line);
    rig.tw.seen_by(&events, who).join("\n")
}

/// Every `arena.observer` panel in `rig.log` from `from`, with its audience.
fn i4_pictures(rig: &Rig, from: usize) -> Vec<(Audience, serde_json::Value)> {
    rig.log
        .get(from..)
        .unwrap_or_default()
        .iter()
        .filter_map(|(audience, event)| match event {
            OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == tacsim::arena_bridge::OBSERVER_FEED =>
            {
                Some((audience.clone(), data.clone()))
            }
            _ => None,
        })
        .collect()
}

/// Whether `who` was sent a cleared picture in `pictures`.
fn i4_cleared(pictures: &[(Audience, serde_json::Value)], who: ObjId) -> bool {
    pictures.iter().any(|(audience, data)| {
        *audience == Audience::Actor(who) && data["watching"] == serde_json::json!(false)
    })
}

/// Pump for `arena_ms` of arena time.
async fn i4_run_for(rig: &mut Rig, arena_ms: i64) {
    let start = rig.staged_now_ms;
    for _ in 0..2_000 {
        rig.pump();
        if rig.staged_now_ms >= start + arena_ms {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!(
        "the arena clock reached only {} ms of {arena_ms}",
        rig.staged_now_ms - start
    );
}

/// The registry's answer to "what does `who` watch".
fn i4_watching_of(rig: &Rig, who: ObjId) -> Option<u64> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.watching(who))
        .map(|match_id| match_id.0)
}

/// **16b cut I.4**: `@arena/watch <match>` binds God and his panel is sent the
/// field at most once an arena second, naming both tanks by callsign and object
/// number (a staff surface); no pilot is sent any of it; the query and the map
/// answer the watcher; `off` sends the cleared picture and afterwards the
/// arena's `Frames` carry no picture at all. Every reply verbatim.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i4_arena_watch_sends_the_field_at_most_once_an_arena_second_and_off_ends_it() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let id = rig.centre().match_id.expect("a running match").0;

    assert_eq!(rig.staff("@arena/watch"), "You are watching nothing.");
    assert_eq!(rig.staff("@arena/watch off"), "You are watching nothing.");
    assert_eq!(
        rig.staff("@arena/watch twelve"),
        "Usage: @arena/watch <match> | off"
    );
    assert_eq!(
        rig.staff(&format!("@arena/watch {id}=1")),
        "Usage: @arena/watch <match> | off"
    );
    assert_eq!(rig.staff("@arena/watch 99"), "There is no match 99.");
    // A pilot is not staff, and the verb's own gate says so first.
    assert_eq!(
        rig.say(1, &format!("@arena/watch {id}")),
        "Permission denied."
    );

    let from = rig.log.len();
    assert_eq!(rig.staff(&format!("@arena/watch {id}")), i4_watching(id));
    assert_eq!(rig.staff("@arena/watch"), i4_watching(id), "the bare read");
    // Watching again is not a second watch.
    assert_eq!(rig.staff(&format!("@arena/watch {id}")), i4_watching(id));
    i4_run_for(&mut rig, 3_500).await;

    let pictures = i4_pictures(&rig, from);
    assert!(
        pictures
            .iter()
            .all(|(audience, _)| *audience == Audience::Actor(GOD)),
        "a picture went to somebody else: {pictures:?}"
    );
    let seconds: Vec<i64> = pictures
        .iter()
        .filter_map(|(_, data)| data["now_ms"].as_i64())
        .map(|now_ms| now_ms.div_euclid(1_000))
        .collect();
    assert!(seconds.len() >= 3, "{seconds:?}");
    assert!(
        seconds.windows(2).all(|pair| pair[0] < pair[1]),
        "two pictures in one arena second: {seconds:?}"
    );
    let (_, picture) = pictures.last().cloned().expect("a picture");
    assert_eq!(picture["schema"]["id"], serde_json::json!("arena.observer"));
    assert_eq!(picture["match"], serde_json::json!(id));
    let rows = picture["units"].as_array().cloned().unwrap_or_default();
    for (callsign, tank) in [("RA", rig.tank(0)), ("BA", rig.tank(1))] {
        assert!(
            rows.iter()
                .any(|row| row[0] == serde_json::json!(callsign)
                    && row[5] == serde_json::json!(tank.0)),
            "the picture names {callsign} as #{}: {picture}",
            tank.0
        );
    }
    // Nobody but the watcher heard a word of it: no pilot line, no pilot panel.
    for n in 0..2 {
        let heard = rig.tw.seen_by(&rig.log, rig.pilot(n)).join("\n");
        assert!(!heard.contains("watching"), "pilot {n}: {heard}");
    }

    // The query answers the watcher the picture and a pilot nothing; the map
    // answers the watcher the watched ground.
    let asked = rig.query(GOD, "arena.observer");
    assert_eq!(asked["match"], serde_json::json!(id), "{asked}");
    assert!(asked["units"].is_array());
    assert_eq!(
        rig.query(rig.pilot(0), "arena.observer"),
        match_::observer::observer_cleared()
    );
    assert_eq!(
        rig.query(GOD, "arena.map")["match"],
        serde_json::json!(id),
        "the watcher's map is the watched ground"
    );

    // `off`: the cleared picture, and then no picture leaves the arena at all.
    let from = rig.log.len();
    // The testkit prints the cleared keyframe as `[panel arena.observer]`; the
    // sentence is the reply.
    let said = rig.staff("@arena/watch off");
    let lines: Vec<&str> = said
        .lines()
        .filter(|line| !line.starts_with("[panel "))
        .collect();
    assert_eq!(
        lines,
        vec![format!("You stop watching match {id}.")],
        "{said}"
    );
    assert!(i4_cleared(&i4_pictures(&rig, from), GOD));
    assert_eq!(rig.staff("@arena/watch"), "You are watching nothing.");
    assert_eq!(
        rig.query(GOD, "arena.observer"),
        match_::observer::observer_cleared()
    );
    assert_eq!(
        rig.query(GOD, "arena.map")["seated"],
        serde_json::json!(false)
    );
    // A pulse already staged before the task heard the `Watch` may still carry
    // one; after an arena second none does.
    i4_run_for(&mut rig, 1_000).await;
    let settled = rig.pictures.len();
    let after = rig.log.len();
    i4_run_for(&mut rig, 3_000).await;
    assert_eq!(
        rig.pictures.len(),
        settled,
        "the arena still builds a picture nobody watches"
    );
    assert!(
        i4_pictures(&rig, after).is_empty(),
        "a picture was sent after off"
    );
}

/// **16b cut I.4**: `@arena/watch` refuses anybody who flies or is about to —
/// a slot holder who has not boarded, a seated pilot, an `EXIT`ed pilot whose
/// slot is kept, and another character of a flying pilot's account — in one
/// sentence, before it even reads the match; a staffer with no slot and no
/// seat watches.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i4_arena_watch_refuses_anybody_who_flies_or_is_about_to() {
    let mut rig = Rig::boot(3);
    for n in 0..3 {
        let pilot = rig.pilot(n);
        i4_make_staff(&mut rig, pilot);
    }
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    // Boarding: a slot, no seat — and no running match either, which is
    // refused later than the slot.
    assert_eq!(rig.state(), MatchState::Boarding);
    assert_eq!(rig.say(0, "@arena/watch 1"), I4_FLYING);
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
    let id = rig.centre().match_id.expect("a running match").0;

    // Seated.
    assert_eq!(rig.say(0, &format!("@arena/watch {id}")), I4_FLYING);
    // `EXIT`ed: the seat is gone and the slot is kept.
    rig.say(1, "exit");
    assert!(
        rig.centre().slot_of_pilot(rig.pilot(1)).is_some(),
        "an EXIT keeps the slot"
    );
    assert_eq!(rig.say(1, &format!("@arena/watch {id}")), I4_FLYING);
    // Another character of pilot 0's account.
    let alt = rig.tw.spawn_player("Alt0");
    i4_make_staff(&mut rig, alt);
    let mut map = Accounts::default();
    for (n, pilot) in rig.pilots.iter().enumerate() {
        map.0.insert(
            *pilot,
            AccountId::mint(&format!("account-{n}"), n as i64).expect("an account id"),
        );
    }
    map.0.insert(
        alt,
        AccountId::mint("account-0", 0).expect("pilot 0's account"),
    );
    *rig.tw
        .world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));
    assert_eq!(
        i4_as(&mut rig, alt, &format!("@arena/watch {id}")),
        I4_FLYING
    );
    assert_eq!(i4_watching_of(&rig, alt), None);

    // A staffer with no slot, no seat and an account of their own watches.
    assert_eq!(rig.say(2, &format!("@arena/watch {id}")), i4_watching(id));
    assert_eq!(i4_watching_of(&rig, rig.pilot(2)), Some(id));
}

/// **16b cut I.4**: eight staffers watch; a ninth is refused; and every picture
/// — the one a pulse builds, told apart by its `now_ms` — is sent to exactly the
/// eight, once each: never more than eight events for a pulse, and none of the
/// eight skipped. Counted per picture and not per `Rig::pump` drain: the fan-out
/// runs at the drain, so a test thread stalled past an arena second hands one
/// drain two pictures, sixteen events, and no pulse the more for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i4_eight_watch_a_ninth_is_refused_and_no_pulse_sends_more_than_eight() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let id = rig.centre().match_id.expect("a running match").0;
    let staffers: Vec<ObjId> = (0..9)
        .map(|n| i4_staffer(&mut rig, &format!("Staffer{n}")))
        .collect();
    for staffer in staffers.iter().take(8) {
        assert_eq!(
            i4_as(&mut rig, *staffer, &format!("@arena/watch {id}")),
            i4_watching(id)
        );
    }
    let ninth = *staffers.last().expect("nine");
    assert_eq!(
        i4_as(&mut rig, ninth, &format!("@arena/watch {id}")),
        format!("Match {id} already has 8 watchers.")
    );
    assert_eq!(i4_watching_of(&rig, ninth), None);
    // A watcher asking again is not a ninth.
    assert_eq!(
        i4_as(&mut rig, staffers[0], &format!("@arena/watch {id}")),
        i4_watching(id)
    );

    let from = rig.log.len();
    i4_run_for(&mut rig, 3_000).await;
    let eight: std::collections::BTreeSet<ObjId> = staffers.iter().take(8).copied().collect();
    let mut by_picture: std::collections::BTreeMap<i64, Vec<ObjId>> =
        std::collections::BTreeMap::new();
    for (audience, data) in i4_pictures(&rig, from) {
        let Audience::Actor(watcher) = audience else {
            panic!("a picture went to {audience:?}, not to one watcher");
        };
        let now_ms = data["now_ms"]
            .as_i64()
            .expect("a picture, not a cleared frame");
        by_picture.entry(now_ms).or_default().push(watcher);
    }
    assert!(
        by_picture.len() >= 2,
        "{} pictures in three arena seconds",
        by_picture.len()
    );
    for (now_ms, audiences) in &by_picture {
        assert_eq!(
            audiences.len(),
            8,
            "the picture at {now_ms} ms was sent {} times",
            audiences.len()
        );
        let reached: std::collections::BTreeSet<ObjId> = audiences.iter().copied().collect();
        assert_eq!(reached, eight, "the picture at {now_ms} ms");
    }
    // Nobody but the eight was ever sent one.
    for (audience, _) in i4_pictures(&rig, 0) {
        assert!(
            matches!(audience, Audience::Actor(who) if eight.contains(&who)),
            "{audience:?}"
        );
    }
}

/// **16b cut I.4**: a watch ends with its watcher's connection, with the
/// staffer's power (at the next picture — the query already answers nothing),
/// and with the match (`finish` unbinds every watcher and sends each the
/// cleared picture — two are still bound when it runs, so neither the first
/// nor the last alone passes).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i4_a_disconnect_a_lost_power_and_the_end_of_the_match_each_end_a_watch() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let id = rig.centre().match_id.expect("a running match").0;
    let gone = i4_staffer(&mut rig, "Gone");
    let demoted = i4_staffer(&mut rig, "Demoted");
    let stays = i4_staffer(&mut rig, "Stays");
    let also = i4_staffer(&mut rig, "Also");
    for staffer in [gone, demoted, stays, also] {
        assert_eq!(
            i4_as(&mut rig, staffer, &format!("@arena/watch {id}")),
            i4_watching(id)
        );
    }
    i4_run_for(&mut rig, 1_100).await;

    // A disconnect.
    let said = rig.fire_conn(gone, mush_core::ConnTransition::Disconnected);
    let cleared = said.iter().any(|(audience, event)| {
        *audience == Audience::Actor(gone)
            && matches!(event, OutputEvent::Panel { feed, data, .. }
                if feed.as_str() == "arena.observer"
                    && data["watching"] == serde_json::json!(false))
    });
    assert!(cleared, "the disconnect cleared the panel: {said:?}");
    assert_eq!(i4_watching_of(&rig, gone), None);

    // A lost power: the query answers nothing at once; the next picture
    // unwatches with the cleared frame.
    rig.tw
        .world_mut()
        .get_mut(demoted)
        .expect("the staffer")
        .powers = mush_core::Powers::empty();
    assert_eq!(
        rig.query(demoted, "arena.observer"),
        match_::observer::observer_cleared()
    );
    assert_eq!(
        rig.query(demoted, "arena.map")["seated"],
        serde_json::json!(false)
    );
    let from = rig.log.len();
    i4_run_for(&mut rig, 1_100).await;
    let since = i4_pictures(&rig, from);
    assert!(i4_cleared(&since, demoted), "{since:?}");
    assert!(
        !since
            .iter()
            .any(|(audience, data)| *audience == Audience::Actor(demoted)
                && data.get("units").is_some()),
        "a demoted staffer was sent a picture"
    );
    assert_eq!(i4_watching_of(&rig, demoted), None);
    for watcher in [stays, also] {
        assert_eq!(i4_watching_of(&rig, watcher), Some(id));
        assert!(
            since
                .iter()
                .any(|(audience, data)| *audience == Audience::Actor(watcher)
                    && data.get("units").is_some()),
            "the remaining watchers still see the field"
        );
    }

    // The end of the match.
    let from = rig.log.len();
    rig.staff(&format!("@arena/abort {id}"));
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    let since = i4_pictures(&rig, from);
    for watcher in [stays, also] {
        assert!(i4_cleared(&since, watcher), "{watcher:?}: {since:?}");
        let cleared_at = since
            .iter()
            .rposition(|(audience, _)| *audience == Audience::Actor(watcher))
            .expect("the watcher's last event");
        assert_eq!(
            since
                .get(cleared_at)
                .map(|(_, data)| data["watching"].clone()),
            Some(serde_json::json!(false)),
            "the cleared picture is the last thing {watcher:?} was sent"
        );
        assert_eq!(i4_watching_of(&rig, watcher), None);
    }
    let empty = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.watchers_of(tacsim::arena_task::MatchId(id)));
    assert_eq!(empty, Some(Vec::new()));
    assert_eq!(
        i4_as(&mut rig, stays, "@arena/watch"),
        "You are watching nothing."
    );
}

/// **16b cut I.4**: a watch closes the staffer's replay (the cursor goes),
/// `SIM REPLAY` refuses a watcher, and the map answers the watched ground.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn i4_a_watch_closes_the_replay_and_sim_replay_refuses_a_watcher() {
    let mut rig = Rig::boot(3);
    let finished = i1_finished(&mut rig).await;
    let bystander = rig.pilot(I1_BYSTANDER);
    let _ = i1_replay_as(&mut rig, I1_BYSTANDER, &finished.0.to_string());
    assert!(i1_book(&rig, |book| book.replay_cursor(bystander)).is_some());

    // A second match.
    rig.staff("@arena/reset");
    rig.walk_to(0, match_::LOBBY_ENTRY);
    rig.walk_to(1, match_::LOBBY_ENTRY);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let id = rig.centre().match_id.expect("a running match").0;
    assert_ne!(id, finished.0);

    i4_make_staff(&mut rig, bystander);
    assert_eq!(
        rig.say(I1_BYSTANDER, &format!("@arena/watch {id}")),
        i4_watching(id)
    );
    assert_eq!(
        i1_book(&rig, |book| book.replay_cursor(bystander)),
        None,
        "a watch closes the replay"
    );
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &finished.0.to_string());
    assert_eq!(
        said,
        format!("You are watching match {id}; @arena/watch off first.")
    );
    assert!(windows.is_empty());
    assert_eq!(i1_book(&rig, |book| book.replay_cursor(bystander)), None);
    assert_eq!(
        rig.query(bystander, "arena.map")["match"],
        serde_json::json!(id),
        "the watcher's map is the watched ground, not the replay's"
    );
    i4_run_for(&mut rig, 1_100).await;
    let asked = rig.query(bystander, "arena.observer");
    assert_eq!(asked["match"], serde_json::json!(id), "{asked}");
    // And after `off`, `SIM REPLAY` is the replay again.
    rig.say(I1_BYSTANDER, "@arena/watch off");
    let (said, windows) = i1_replay_as(&mut rig, I1_BYSTANDER, &finished.0.to_string());
    assert!(
        said.contains(&format!("Replay of match {}", finished.0)),
        "{said}"
    );
    assert_eq!(windows.len(), 1);
}

// ---------------------------------------------------------------------------
// 16b cut O.1 — `SIM MODE` and the Hold win (16c N11)
// ---------------------------------------------------------------------------

/// What `SIM MODE` answers on the shipped ground at a skirmish, verbatim — with
/// 16b cut O.3's tail, which points at the one mode the list does not carry.
const O1_BARE_SKIRMISH: &str = "Mode: skirmish. The Proving Ground supports: skirmish, hold. SIM \
                                RANGE sets a range.";

/// What it answers once the mode is Hold, verbatim (16b cut O.3's tail last).
const O1_BARE_HOLD: &str = "Mode: hold. The Proving Ground supports: skirmish, hold. Hold: keep \
                            The Crossroads (B) uncontested for 120 seconds to win. SIM RANGE \
                            sets a range.";

/// The announcement of a set to Hold, verbatim.
const O1_SET_HOLD: &str = "Mode set to hold: keep The Crossroads (B) uncontested for 120 seconds \
                           to win; elimination still ends it.";

/// The board's line under Hold, verbatim.
const O1_BOARD_LINE: &str = "  mode: hold \u{2014} keep The Crossroads (B) uncontested for 120 s";

/// **16b cut O.1**: `SIM MODE` reads for anybody and sets for a lead or staff
/// in `Selection`, every reply and refusal verbatim; the set is announced in
/// every centre room; the board prints the rule in `Selection` and nothing at a
/// skirmish; the check-out carries the mode onto the record; and the return to
/// `Idle` clears it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o1_sim_mode_reads_sets_refuses_announces_and_is_cleared_at_idle() {
    let mut rig = Rig::boot(3);
    // The bare read, in `Idle`, slot or no slot.
    assert_eq!(rig.say(2, "sim mode"), O1_BARE_SKIRMISH);
    // A set outside `Selection`.
    assert_eq!(
        rig.say(0, "sim mode hold"),
        "The simulator centre is idle; that is not something you can do to it now."
    );
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(2, "sim team red");
    // A claimant who leads nothing, and an unknown word.
    assert_eq!(
        rig.say(2, "sim mode hold"),
        "Only a side's lead or staff can set the mode."
    );
    assert_eq!(
        rig.say(0, "sim mode capture"),
        "No such mode: capture. SIM MODE lists them."
    );
    // No board line at a skirmish.
    let board = lobby_board(&rig, 2);
    assert!(
        !board.iter().any(|line| line.contains("mode:")),
        "{board:?}"
    );
    // The set, case-folded, heard in every room of the centre.
    let events = rig.events(0, "SIM MODE Hold");
    let rooms: Vec<ObjId> = [
        match_::LOBBY_ENTRY,
        match_::RED_ROOM_ENTRY,
        match_::BLUE_ROOM_ENTRY,
    ]
    .into_iter()
    .map(|entry| match_::centre_object(rig.tw.world(), entry).expect("a centre room"))
    .collect();
    for room in &rooms {
        assert!(
            events.iter().any(|(audience, event)| matches!(
                audience,
                Audience::Room { room: heard, .. } if heard == room
            ) && format!("{event:?}").contains(O1_SET_HOLD)),
            "room {room} did not hear the set: {events:?}"
        );
    }
    assert_eq!(rig.centre().mode, tacsim::arena::MatchMode::Hold);
    // The read now carries the rule, for anybody; the board says it too.
    assert_eq!(rig.say(2, "sim mode"), O1_BARE_HOLD);
    let board = lobby_board(&rig, 2);
    assert!(board.iter().any(|line| line == O1_BOARD_LINE), "{board:?}");
    // The other side's lead may set it back, and the first lead again.
    assert_eq!(
        rig.say(1, "sim mode skirmish"),
        "Mode set to skirmish: eliminate the other side to win."
    );
    assert_eq!(rig.centre().mode, tacsim::arena::MatchMode::Skirmish);
    assert_eq!(rig.say(0, "sim mode hold"), O1_SET_HOLD);
    assert_eq!(rig.centre().mode, tacsim::arena::MatchMode::Hold);

    // The check-out carries it onto the record.
    rig.say(0, "sim start");
    assert_eq!(
        rig.say(0, "sim mode skirmish"),
        "The simulator centre is boarding; that is not something you can do to it now."
    );
    let board = lobby_board(&rig, 2);
    assert!(board.iter().any(|line| line == O1_BOARD_LINE), "{board:?}");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.walk_to(2, match_::RED_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    rig.say(2, "board");
    assert_eq!(rig.state(), MatchState::Running);
    assert_eq!(rig.record().mode, tacsim::arena::MatchMode::Hold);
    // The bare read answers mid-match too.
    assert_eq!(rig.say(1, "sim mode"), O1_BARE_HOLD);

    // The return to `Idle` clears it.
    rig.say(0, "sim abort");
    for _ in 0..400 {
        if rig.state().is_terminal() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert!(rig.state().is_terminal());
    rig.staff("@arena/reset");
    assert_eq!(rig.state(), MatchState::Idle);
    assert_eq!(rig.centre().mode, tacsim::arena::MatchMode::Skirmish);
    assert_eq!(rig.say(2, "sim mode"), O1_BARE_SKIRMISH);
}

/// The board as pilot `n` reads it.
fn lobby_board(rig: &Rig, n: usize) -> Vec<String> {
    match_::status_lines(rig.tw.world(), BoardReader::Player(rig.pilot(n)))
}

/// **16b cut O.1 (plan item 30)**: `match_::start` refuses a mode the ground
/// does not declare — the stand-in window declares none — right after the
/// definition is resolved, and writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o1_start_refuses_a_mode_the_ground_does_not_declare() {
    let mut rig = Rig::boot(1);
    let tank = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let refused = rig.with_ctx_mut(0, |ctx| {
        match_::start(
            ctx,
            tacsim::arena_task::MatchId(99),
            &smol_str::SmolStr::new(match_::STAND_IN),
            &[match_::Side {
                team: PartyId::RED,
                units: vec![match_::Fielded::unslotted(tank)],
                minds: std::collections::BTreeMap::new(),
            }],
            &tacsim::config::ArenaConfig::default(),
            &tacsim::config::MatchConfig::default(),
            &tacsim::config::SensorsConfig::default(),
            tacsim::arena::MatchMode::Hold,
        )
    });
    let Err(error) = refused else {
        panic!("a hold started on a ground that declares none");
    };
    assert_eq!(
        error,
        match_::StartError::ModeUnsupported {
            mode: "hold",
            arena: smol_str::SmolStr::new(match_::STAND_IN),
        }
    );
    assert_eq!(
        error.to_string(),
        "arena tacsim.arena.stand_in does not support hold"
    );
    assert!(
        !mush_sim::deployed::is_deployed(rig.tw.world(), tank),
        "nothing was checked out"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .and_then(|book| book.get(tacsim::arena_task::MatchId(99)))
            .is_none(),
        "no record was written"
    );
}

/// **16b cut O.1 (plan item 26): the win itself**, under paused time — the
/// arena is a tokio task and `Rig::tick_to` moves only the world's clock, so
/// paused time is what advances it and 120 arena seconds cost no wall time.
///
/// Red's tank is posed on The Crossroads and blue's stands on its authored
/// muster; both are boarded and started up; and the pump waits on the terminal
/// head, never a fixed count. The head reads `red wins: held The Crossroads`,
/// the result and the replay documents carry `"mode": "hold"`, `SIM HISTORY`
/// tags the row, the report's header tags it, and a restart row written after
/// carries no mode and reads with no tag. The record is what is asserted —
/// never the arena's own `HoldState`, which the rig cannot see.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn o1_a_hold_is_won_and_every_record_says_hold() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    assert_eq!(rig.say(0, "sim mode hold"), O1_SET_HOLD);
    let said = rig.staff("@arena/pose RA=129 97 90");
    assert!(
        said.contains("RA opens at cell (129, 97) heading 90 for the next match."),
        "{said}"
    );
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
    let match_id = rig.centre().match_id.expect("a running match");
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    rig.say(0, "startup");
    rig.say(1, "startup");
    assert!(
        rig.powered(0).await && rig.powered(1).await,
        "both started up"
    );
    // The pump, waiting on the terminal head: 120 arena seconds and a margin
    // is 13,000 ten-millisecond sleeps at the most.
    for _ in 0..13_000 {
        rig.pump();
        if rig.state().is_terminal() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        rig.state(),
        MatchState::Ended,
        "no end by {} arena ms",
        rig.staged_now_ms
    );
    assert!(
        rig.staged_now_ms >= 120_000,
        "the match ended before a hold could: {} ms",
        rig.staged_now_ms
    );
    let board = lobby_board(&rig, 1);
    assert_eq!(
        board.first().map(String::as_str),
        Some(
            format!(
                "Simulator centre: ended (match {match_id}) \u{2014} red wins: held The Crossroads"
            )
            .as_str()
        ),
        "{board:?}"
    );

    // The documents.
    let writes: Vec<(String, serde_json::Value)> = rig
        .pump_ops()
        .into_iter()
        .filter(|op| op.ns == tacsim::NS)
        .filter_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => Some((key.to_string(), value)),
            _ => None,
        })
        .collect();
    let document = |key: String| {
        writes
            .iter()
            .find(|(held, _)| *held == key)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{key} was written"))
    };
    let result = document(format!("tacsim:match:{}", match_id.0));
    assert_eq!(result["mode"], serde_json::json!("hold"));
    assert_eq!(result["outcome"], serde_json::json!("red"));
    assert_eq!(result["reason"], serde_json::json!("hold"));
    let replay = document(format!("tacsim:replay:{}", match_id.0));
    assert_eq!(replay["mode"], serde_json::json!("hold"));

    // The ring and the report's header.
    let history = rig.say(1, "sim history");
    assert!(
        history.contains(&format!(
            "#{}  red won (red held the key spot) [hold]",
            match_id.0
        )),
        "{history}"
    );
    let row = rig
        .tw
        .world()
        .module_state::<match_::MatchBook>()
        .and_then(|book| book.remembered(match_id).cloned())
        .expect("the ring holds the match");
    assert_eq!(row.mode, Some(tacsim::arena::MatchMode::Hold));
    let header = match_::report::header_lines(
        &row,
        "The Proving Ground",
        match_::report::Reader::Staff,
        None,
        None,
    );
    assert!(
        header
            .first()
            .is_some_and(|line| line.contains("red won (red held the key spot) [hold], ")),
        "{header:?}"
    );

    // A restart row knows no mode and says none.
    let tank = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    rig.with_ctx_mut(0, |ctx| {
        match_::write_restart_abort(ctx, tacsim::arena_task::MatchId(77), &[tank]);
    });
    let restart = rig
        .tw
        .world()
        .module_state::<match_::MatchBook>()
        .and_then(|book| book.remembered(tacsim::arena_task::MatchId(77)).cloned())
        .expect("the ring holds the restart row");
    assert_eq!(restart.mode, None);
    let history = rig.say(1, "sim history");
    let line = history
        .lines()
        .find(|line| line.contains("#77 "))
        .unwrap_or_else(|| panic!("{history}"));
    assert!(
        line.contains("aborted (the server restarted)") && !line.contains('['),
        "{line}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut O.2 — `EMCON`, the jamming pod's switch (16c N12)
// ---------------------------------------------------------------------------

/// **The pod's switch, through the real verb** (16b cut O.2; 16c N12).
///
/// Pilot 0 chose `line_tank_ecm` through `SIM UNIT`, so the arena fields the
/// pod-carrying tank in red slot 1; pilot 1 flies the stock tank. The dark
/// panel before the first frame, bare and with a word, forwarding nothing; every
/// reply and refusal verbatim; the frame's `ecm` on both (the pod's tank `on`, the
/// stock one `null`); the `STATUS` line; the notice lines on the next frame;
/// a stock tank's `EMCON off` refused locally with no intent forwarded; and
/// **`EMCON off` then `EMCON on` typed back to back, both accepted with true
/// replies** — the second read off the bridge's hint and not the stale mirror —
/// the pod on at the frame that follows and the tape carrying both lines, in
/// order. Then one real flip, the hint read before the mirror again, the tape's
/// line and both reports' projections.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o2_emcon_switches_the_pod_from_the_cockpit_and_says_what_it_costs() {
    use tacsim::match_::report::{Reader, ReportRules, Roster, fold};

    const RULES: ReportRules = ReportRules {
        designate_on_detection: false,
    };
    const OFF_LINE: &str =
        "EMCON off: the pod is silent; your own sight clears, and nothing hides you.";
    const ON_LINE: &str =
        "EMCON on: the pod radiates; every sight inside 60 m is jammed, your own among them.";

    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim unit line_tank_ecm");
    assert!(said.contains("will field line_tank_ecm"), "{said}");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
    let hint = |rig: &Rig, n: usize| -> Option<bool> {
        let pilot = rig.pilot(n);
        let now = rig.tw.world().now();
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.emcon_hint(pilot, now))
    };

    // Before the first frame (16b cut V.1's precedent: nothing has been pumped,
    // so the world holds no frame): the panel is dark, bare or with a word, and
    // nothing is forwarded — without a frame there is nothing to say whether
    // this tank carries a pod, or which way its switch stands.
    assert!(rig.frame(0).is_none(), "nothing has been pumped yet");
    assert_eq!(rig.say(0, "emcon"), cockpit::NO_FRAME_YET);
    assert_eq!(rig.say(0, "emcon off"), cockpit::NO_FRAME_YET);
    assert_eq!(rig.say(0, "EMCON ON"), cockpit::NO_FRAME_YET);
    assert_eq!(hint(&rig, 0), None, "a dark panel's EMCON stamped a hint");

    assert!(rig.frame_for(0).await, "the pod's pilot got no frame");
    assert!(rig.frame_for(1).await, "the stock pilot got no frame");
    let ecm_tank = rig.tank(0);
    let stock_tank = rig.tank(1);
    let emcon_lines = |rig: &Rig| -> Vec<(ObjId, bool)> {
        rig.tape
            .iter()
            .filter_map(|event| match event.kind {
                tacsim::arena::ledger::EventKind::Emcon { unit, on } => Some((unit, on)),
                _ => None,
            })
            .collect()
    };

    // The frame: the pod's tank carries its switch, the stock tank `null`.
    assert_eq!(rig.held_frame(0).ecm, Some(true), "every pod checks out on");
    assert_eq!(rig.held_frame(1).ecm, None);
    let panel = Rig::panel(&rig.events(0, "status")).expect("STATUS re-pushes the frame");
    assert_eq!(panel.get("ecm"), Some(&serde_json::json!("on")));
    let panel = Rig::panel(&rig.events(1, "status")).expect("STATUS re-pushes the frame");
    assert_eq!(panel.get("ecm"), Some(&serde_json::Value::Null));

    // Bare reads, and the STATUS line.
    let said = rig.say(0, "emcon");
    assert!(said.contains("EMCON on: the pod radiates."), "{said}");
    let said = rig.say(1, "emcon");
    assert!(said.contains("This tank carries no jamming pod."), "{said}");
    let said = rig.say(0, "status");
    assert!(said.contains("  emcon on: the pod radiates"), "{said}");
    let said = rig.say(1, "status");
    assert!(!said.to_lowercase().contains("emcon"), "{said}");

    // The refusals: already on, a bad word, a stock tank — none forwards.
    let said = rig.say(0, "emcon ON");
    assert!(said.contains("EMCON is already on."), "{said}");
    assert_eq!(hint(&rig, 0), None, "an order already true stamps nothing");
    let said = rig.say(0, "emcon sideways");
    assert!(said.contains("EMCON on|off."), "{said}");
    let said = rig.say(1, "emcon off");
    assert!(said.contains("This tank carries no jamming pod."), "{said}");
    assert_eq!(
        hint(&rig, 1),
        None,
        "the stock tank's refusal stamped nothing"
    );

    // Off then on, back to back: the mirror still says `on` for the second,
    // and the hint is what makes its reply true.
    let said = rig.say(0, "emcon off");
    assert!(
        said.contains("EMCON going off; the pod is silent from the next pulse."),
        "{said}"
    );
    assert_eq!(hint(&rig, 0), Some(false));
    let said = rig.say(0, "emcon on");
    assert!(
        said.contains("EMCON coming on; the pod radiates from the next pulse."),
        "{said}"
    );
    assert_eq!(hint(&rig, 0), Some(true));
    let lines = rig
        .tape_count_until(|event| event.kind.as_str() == "emcon", 2)
        .await;
    assert_eq!(lines, 2, "both orders were resolved and journalled");
    assert_eq!(
        emcon_lines(&rig),
        vec![(ecm_tank, false), (ecm_tank, true)],
        "the tape carries both switches, in the order they were typed"
    );
    // The frames that carried the two notices were staged before the chunk
    // that closed over the two lines, so the mirror is past them.
    assert_eq!(rig.held_frame(0).ecm, Some(true), "the pod ended on");
    assert_eq!(
        hint(&rig, 0),
        None,
        "the frame that agreed cleared the hint"
    );
    let told = rig.tw.seen_by(&rig.log, rig.pilot(0)).join("\n");
    assert!(told.contains(OFF_LINE), "{told}");
    assert!(told.contains(ON_LINE), "{told}");

    // One real flip: the reply, then the hint answering before the mirror.
    let said = rig.say(0, "emcon off");
    assert!(
        said.contains("EMCON going off; the pod is silent from the next pulse."),
        "{said}"
    );
    let said = rig.say(0, "emcon off");
    assert!(said.contains("EMCON is already off."), "{said}");
    let lines = rig
        .tape_count_until(|event| event.kind.as_str() == "emcon", 3)
        .await;
    assert_eq!(lines, 3, "one more line, and not two");
    assert_eq!(emcon_lines(&rig).last(), Some(&(ecm_tank, false)));
    assert_eq!(rig.held_frame(0).ecm, Some(false));
    let said = rig.say(0, "status");
    assert!(said.contains("  emcon off: the pod is silent"), "{said}");
    let said = rig.say(0, "emcon");
    assert!(said.contains("EMCON off: the pod is silent."), "{said}");
    let panel = Rig::panel(&rig.events(0, "status")).expect("STATUS re-pushes the frame");
    assert_eq!(panel.get("ecm"), Some(&serde_json::json!("off")));
    // And with the hint cleared, the mirror answers.
    assert_eq!(hint(&rig, 0), None);
    let said = rig.say(0, "emcon off");
    assert!(said.contains("EMCON is already off."), "{said}");

    // The stock tank's pod never moved: no line names it.
    assert!(
        emcon_lines(&rig).iter().all(|(unit, _)| *unit == ecm_tank),
        "{:?}",
        emcon_lines(&rig)
    );
    assert_ne!(ecm_tank, stock_tank);
    // **And its local refusal forwarded nothing** — which the missing hint
    // above cannot say on its own, since the hint is stamped only after a
    // forward succeeds. An `emcon` intent that had reached the arena would have
    // come back, pulses ago now, as `Refusal::NoJammer` on the stock pilot's
    // frame and been told as the arena's own refusal line; the tape has been
    // pumped well past that, and the stock pilot was told neither it nor any
    // EMCON sentence but the local refusals.
    let stock_told = rig.tw.seen_by(&rig.log, rig.pilot(1)).join("\n");
    assert!(
        !stock_told.contains("EMCON refused"),
        "the stock tank's EMCON off reached the arena: {stock_told}"
    );
    assert!(
        !stock_told.contains("EMCON going") && !stock_told.contains("EMCON coming"),
        "{stock_told}"
    );

    // The tape, folded: the pilot's own switches on the pod's pilot's report
    // and nothing on the other's; the staff fold's field for each.
    let roster = Roster::from_record(&rig.record());
    let own = fold(&rig.tape, &roster, Reader::Pilot { unit: ecm_tank }, RULES);
    let switched: Vec<&String> = own
        .lines
        .iter()
        .filter(|line| line.contains("switched EMCON"))
        .collect();
    assert_eq!(switched.len(), 3, "{:?}", own.lines);
    assert!(
        switched
            .first()
            .is_some_and(|line| line.ends_with("  you switched EMCON off.")),
        "{switched:?}"
    );
    assert!(
        switched
            .get(1)
            .is_some_and(|line| line.ends_with("  you switched EMCON on.")),
        "{switched:?}"
    );
    let other = fold(
        &rig.tape,
        &roster,
        Reader::Pilot { unit: stock_tank },
        RULES,
    );
    assert!(
        other
            .lines
            .iter()
            .chain(other.tail.iter())
            .all(|line| !line.contains("EMCON")),
        "another pilot's switch reached this pilot's report: {:?}",
        other.lines
    );
    let staff = fold(&rig.tape, &roster, Reader::Staff, RULES);
    let fields: Vec<&String> = staff
        .lines
        .iter()
        .filter(|line| line.contains("emcon unit="))
        .collect();
    assert_eq!(fields.len(), 3, "{:?}", staff.lines);
    assert!(
        fields
            .first()
            .is_some_and(|line| line.ends_with(&format!("(#{}) off", ecm_tank.0))),
        "{fields:?}"
    );
    assert!(
        fields
            .get(1)
            .is_some_and(|line| line.ends_with(&format!("(#{}) on", ecm_tank.0))),
        "{fields:?}"
    );
}

/// **The pod-switch hint's frame-borne ends** (16b cut O.2), in D.1's shape
/// for the `STARTUP` hint.
///
/// The hint is stamped with the switch the pilot last forwarded, and a frame
/// ends it only two ways: its `ecm` **agrees** with that switch (the arena
/// confirmed it), or it carries the arena's `Refused` of an **`emcon`** order.
/// A frame whose `ecm` still disagrees may be a batch staged before the intent
/// arrived, and a refusal of another order is not this one's, so both leave it
/// standing — the first is what makes `EMCON off` then `EMCON on` inside one
/// pulse two true replies. Composed frames through [`Rig::push_frame`], the real
/// egress seam; the stamp through the registry, so no intent is in flight.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o2_the_emcon_hint_ends_only_on_agreement_or_its_own_refusal() {
    use tacsim::arena::intents::{IntentKind, Refusal};
    use tacsim::arena::perception::{FrameNotice, RefusalDetail};

    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let hint = |rig: &Rig| -> Option<bool> {
        let now = rig.tw.world().now();
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.emcon_hint(pilot, now))
    };
    let stamp = |rig: &mut Rig, on: bool| {
        if let Some(registry) = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
        {
            registry.hint_emcon(pilot, on, i64::MAX);
        }
    };
    assert_eq!(hint(&rig), None, "a frame on its own stamped an EMCON hint");

    // The scene: a pod radiating, as the mirror holds it.
    let mut frame = rig.held_frame(0);
    frame.ecm = Some(true);
    frame.notices.clear();
    rig.push_frame(0, frame.clone());
    stamp(&mut rig, false);
    assert_eq!(hint(&rig), Some(false));
    // Read before the mirror: the switch just sent, not the stale `on`.
    let said = rig.say(0, "emcon off");
    assert!(said.contains("EMCON is already off."), "{said}");

    // (1) A frame whose `ecm` still disagrees, and no refusal, LEAVES it.
    rig.push_frame(0, frame.clone());
    assert_eq!(
        hint(&rig),
        Some(false),
        "a frame staged before the intent arrived cleared the hint"
    );

    // (2) A refusal of ANOTHER order leaves it too.
    let refusal = |order: &str, refusal: Refusal| FrameNotice::Refused {
        order: smol_str::SmolStr::new(order),
        refusal,
        weapon: None,
        detail: RefusalDetail::None,
    };
    let mut other = frame.clone();
    other.notices = vec![refusal("fire", Refusal::PowerNotOnline)];
    rig.push_frame(0, other);
    assert_eq!(
        hint(&rig),
        Some(false),
        "a refusal of another order cleared the hint"
    );

    // (3) **Refused**: the arena's refusal of an `emcon` order clears it, so the
    // pilot who reads `EMCON refused.` is not then told `already off`.
    let mut refused = frame.clone();
    refused.notices = vec![refusal(
        IntentKind::Emcon { on: false }.as_str(),
        Refusal::NoJammer,
    )];
    rig.push_frame(0, refused);
    assert_eq!(
        hint(&rig),
        None,
        "the refusing frame did not clear the hint"
    );
    let said = rig.say(0, "emcon");
    assert!(said.contains("EMCON on: the pod radiates."), "{said}");

    // (4) **Confirmed**: a frame whose `ecm` agrees clears it — either way.
    stamp(&mut rig, false);
    let mut silent = frame.clone();
    silent.ecm = Some(false);
    rig.push_frame(0, silent.clone());
    assert_eq!(
        hint(&rig),
        None,
        "the agreeing frame did not clear the hint"
    );
    stamp(&mut rig, true);
    rig.push_frame(0, silent);
    assert_eq!(
        hint(&rig),
        Some(true),
        "a disagreeing `off` frame cleared an `on` hint"
    );
    rig.push_frame(0, frame);
    assert_eq!(
        hint(&rig),
        None,
        "the agreeing `on` frame did not clear the hint"
    );
}

/// **The pod-switch hint's other door: the match's own rows go** (16b cut O.2),
/// on the precedent of `d1_untracking_a_match_drops_a_hint_no_frame_ever_confirmed`
/// — a pilot with a hint and **no remembered frame at all**, so a sweep keyed on
/// the frames the registry holds would find nothing to clear.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o2_untracking_a_match_drops_an_emcon_hint_no_frame_ever_confirmed() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    assert!(
        rig.frame(0).is_none(),
        "a frame arrived before the test could stamp a hint without one"
    );
    let match_id = rig
        .centre()
        .match_id
        .expect("a running centre has a match id");
    let now = rig.tw.world().now();
    let Some(registry) = rig
        .tw
        .world_mut()
        .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
    else {
        panic!("the arena registry is composed");
    };
    registry.hint_emcon(pilot, false, i64::MAX);
    assert_eq!(registry.emcon_hint(pilot, now), Some(false));
    assert!(
        registry.pilots_of(match_id).contains(&pilot),
        "the pilot is not on the match's roster, so the sweep below would pass \
         without doing anything"
    );
    let _ = registry.untrack(match_id);
    assert_eq!(
        registry.emcon_hint(pilot, now),
        None,
        "untracking the match left behind an EMCON hint no frame had confirmed"
    );
}

/// **Both blessed fixtures carry `"ecm": null`** (16b cut O.2): the scene's
/// platform carries no pod, so the key is on the frame and empty — which is
/// what keeps every golden's scope row without a suffix.
#[test]
fn o2_both_blessed_fixtures_carry_ecm_null() {
    for (name, text) in [
        (
            "cockpit_frame.json",
            include_str!("../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"),
        ),
        (
            "view_frame.json",
            include_str!("../../../../web/src/lib/fixtures/cockpit/view_frame.json"),
        ),
    ] {
        let frame: serde_json::Value = serde_json::from_str(text).expect("a fixture reads");
        assert_eq!(
            frame.get("ecm"),
            Some(&serde_json::Value::Null),
            "{name} does not carry `ecm` as null"
        );
    }
    let (cockpit, _, view) = fixture::payloads();
    assert_eq!(cockpit.get("ecm"), Some(&serde_json::Value::Null));
    assert_eq!(view.get("ecm"), Some(&serde_json::Value::Null));
}

/// **The hand-written ring rows are the server's own wire shape** (16b cut
/// O.2): the two `emcon` notices `status_frame` publishes on the scene's frame
/// — the kind, the pod's radius on `detail.radius_m` and every other key
/// `null` or `false` — equal the `notice` objects of their rows in
/// `notice_lines.json` byte for byte, and the frame's `ecm` is the switch.
#[test]
fn o2_the_emcon_notice_rows_are_what_status_frame_publishes() {
    use tacsim::arena::perception::FrameNotice;

    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../web/src/lib/fixtures/cockpit/notice_lines.json"
    ))
    .expect("notice_lines.json is an array");
    for (on, kind) in [(true, "emcon_on"), (false, "emcon_off")] {
        let mut frame = fixture::frame();
        frame.ecm = Some(on);
        frame.notices = vec![FrameNotice::Emcon { on, radius_m: 60 }];
        let payload = cockpit::status_frame(fixture::MATCH, &frame);
        assert_eq!(
            payload.get("ecm"),
            Some(&serde_json::json!(if on { "on" } else { "off" }))
        );
        let published = payload
            .pointer("/notices/0")
            .cloned()
            .unwrap_or_else(|| panic!("{kind}: no notice on the frame: {payload}"));
        let committed = rows
            .iter()
            .find_map(|row| {
                row.get("notice")
                    .filter(|notice| notice.get("kind") == Some(&serde_json::json!(kind)))
            })
            .cloned()
            .unwrap_or_else(|| panic!("notice_lines.json has no {kind} row"));
        assert_eq!(published, committed, "{kind}");
    }
}

// ---------------------------------------------------------------------------
// 16b cut O.3 — `SIM RANGE` and the range's targets (16c N13)
// ---------------------------------------------------------------------------

/// What the bare `SIM RANGE` answers when no range is set, verbatim.
const O3_NOT_SET: &str = "The range is not set; SIM RANGE <n> sets it in selection.";

/// The announcement of `SIM RANGE 2` with red claimed, verbatim.
const O3_SET_TWO: &str = "Range set: 2 targets stand on the blue side; the match ends when every \
                          target is out of action, or on SIM ABORT.";

/// The board's line under that range, verbatim.
const O3_BOARD_LINE: &str = "  mode: range \u{2014} 2 targets on the blue side";

/// **The invariant** `mode == Range ⇔ range_targets.is_some()` on the centre as
/// it stands (16b cut O.3; plan item 21), asserted after every write.
fn o3_invariant(rig: &Rig) {
    let centre = rig.centre();
    assert_eq!(
        centre.mode == tacsim::arena::MatchMode::Range,
        centre.range_targets.is_some(),
        "mode {:?} with range_targets {:?}",
        centre.mode,
        centre.range_targets
    );
}

/// The unit schema the tank in `team`'s pod `index` holds now.
fn o3_schema(rig: &Rig, team: PartyId, index: u8) -> String {
    let tank = match_::tank_for_slot(rig.tw.world(), team, index).expect("that pod's tank");
    mush_tactics::module::unit_of(rig.tw.world(), tank)
        .map(|unit| unit.schema().key().id.as_str().to_owned())
        .unwrap_or_default()
}

/// Pump until the centre is terminal, or give up.
async fn o3_until_terminal(rig: &mut Rig) {
    for _ in 0..400 {
        rig.pump();
        if rig.state().is_terminal() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(rig.state().is_terminal(), "the check-in lands");
}

/// **16b cut O.3**: `SIM RANGE` on a server with the fill OFF (the rig's
/// default and the shipped one) — every reply and refusal verbatim, the bare
/// read in `Idle`, in `Selection` by the lead (a read, not a set) and in
/// `Running`; `SIM MODE range` refused; the invariant after every write; the
/// announcement in every centre room; the board line; a claim on the targets'
/// side refused and the claim advice under a range; `SIM MODE hold` leaving the
/// range; `SIM START` accepted with the fill off; the targets' pods refitted to
/// `target_mk1` and nothing else refitted; every target the reflex
/// controller's; the record's mode; `SIM HISTORY`'s and the replay's ` [range]`;
/// the pilot's header line with no shots carrying no rate; and the `Idle` reset
/// putting the targets' pods back to `line_tank`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o3_sim_range_reads_sets_refuses_and_starts_with_the_fill_off() {
    let mut rig = Rig::boot(3);
    o3_invariant(&rig);
    let per_side = {
        let cfg = config();
        match_::slots_per_side(rig.tw.world(), &cfg.match_)
    };
    // The bare read in `Idle`, slot or no slot; a set outside `Selection`; a
    // word that is not a number.
    assert_eq!(rig.say(2, "sim range"), O3_NOT_SET);
    assert_eq!(
        rig.say(0, "sim range 2"),
        "The simulator centre is idle; that is not something you can do to it now."
    );
    assert_eq!(
        rig.say(0, "sim range four"),
        "How many targets? `SIM RANGE 4`."
    );
    o3_invariant(&rig);

    // Both sides claimed: the range needs a side nobody claimed.
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    assert_eq!(
        rig.say(0, "sim range 2"),
        "The range is one side against the targets; every slot must be on one side."
    );
    o3_invariant(&rig);
    rig.say(1, "sim leave");
    rig.say(2, "sim team red");
    // A claimant who leads nothing; the lead's bare word reads and sets nothing.
    assert_eq!(
        rig.say(2, "sim range 2"),
        "Only a side's lead or staff can set the range."
    );
    assert_eq!(rig.say(0, "sim range"), O3_NOT_SET);
    o3_invariant(&rig);
    // The count's bounds.
    let bounds = format!("The range fields 1 to {per_side} targets.");
    assert_eq!(rig.say(0, "sim range 0"), bounds);
    assert_eq!(rig.say(0, &format!("sim range {}", per_side + 1)), bounds);
    // `SIM MODE range` is the range's to set, and no one else's.
    assert_eq!(
        rig.say(0, "sim mode range"),
        "SIM RANGE <n> sets the range."
    );
    o3_invariant(&rig);
    assert_eq!(rig.centre().mode, tacsim::arena::MatchMode::Skirmish);

    // The set, heard in every room of the centre.
    let events = rig.events(0, "SIM RANGE 2");
    for entry in [
        match_::LOBBY_ENTRY,
        match_::RED_ROOM_ENTRY,
        match_::BLUE_ROOM_ENTRY,
    ] {
        let room = match_::centre_object(rig.tw.world(), entry).expect("a centre room");
        assert!(
            events.iter().any(|(audience, event)| matches!(
                audience,
                Audience::Room { room: heard, .. } if *heard == room
            ) && format!("{event:?}").contains(O3_SET_TWO)),
            "room {room} did not hear the set: {events:?}"
        );
    }
    o3_invariant(&rig);
    assert_eq!(rig.centre().mode, tacsim::arena::MatchMode::Range);
    assert_eq!(rig.centre().range_targets, Some(2));
    // The read, for anybody; the mode's read; the board.
    assert_eq!(
        rig.say(2, "sim range"),
        "Range: 2 targets on the blue side."
    );
    assert_eq!(
        rig.say(2, "sim mode"),
        "Mode: range. The Proving Ground supports: skirmish, hold. SIM RANGE sets a range."
    );
    let board = lobby_board(&rig, 2);
    assert!(board.iter().any(|line| line == O3_BOARD_LINE), "{board:?}");
    assert!(
        !board.iter().any(|line| line.contains("AI fill:")),
        "{board:?}"
    );
    // A claim on the targets' side, refused; a claim on the shooters', advised.
    assert_eq!(
        rig.say(1, "sim team blue"),
        "The range puts targets on the blue side; claim a red slot."
    );
    let said = rig.say(1, "sim team red");
    assert!(
        said.contains(
            "Pilot1 takes red slot 3. `SIM START` begins the match; the targets stand on the \
             blue side."
        ),
        "{said}"
    );
    // The fill is off here, and `SIM AI` says so first, range or not.
    assert_eq!(rig.say(1, "sim ai"), "AI fill is off on this server.");
    // `SIM MODE hold` leaves the range, and `SIM RANGE 2` takes it back.
    rig.say(0, "sim mode hold");
    o3_invariant(&rig);
    assert_eq!(rig.centre().mode, tacsim::arena::MatchMode::Hold);
    assert_eq!(rig.say(2, "sim range"), O3_NOT_SET);
    assert_eq!(rig.say(0, "sim range 2"), O3_SET_TWO);
    o3_invariant(&rig);

    // `SIM START`, with the fill off: accepted, and the targets' pods refitted.
    let said = rig.say(0, "sim start");
    assert!(said.contains("is boarding"), "{said}");
    assert_eq!(rig.state(), MatchState::Boarding);
    assert_eq!(o3_schema(&rig, PartyId::BLUE, 1), "tacsim.unit.target_mk1");
    assert_eq!(o3_schema(&rig, PartyId::BLUE, 2), "tacsim.unit.target_mk1");
    assert_eq!(o3_schema(&rig, PartyId::BLUE, 3), "tacsim.unit.line_tank");
    assert_eq!(o3_schema(&rig, PartyId::RED, 4), "tacsim.unit.line_tank");
    let board = lobby_board(&rig, 2);
    assert!(board.iter().any(|line| line == O3_BOARD_LINE), "{board:?}");
    for n in 0..3 {
        rig.walk_to(n, match_::RED_ROOM_ENTRY);
        rig.say(n, "board");
    }
    assert_eq!(rig.state(), MatchState::Running);
    assert_eq!(
        rig.say(1, "sim range"),
        "Range: 2 targets on the blue side."
    );
    let board = lobby_board(&rig, 1);
    assert!(board.iter().any(|line| line == O3_BOARD_LINE), "{board:?}");

    // The record: a range; red its three claimed tanks; blue the two targets,
    // with no pilot and in a squad — the reflex controller's.
    let record = rig.record();
    assert_eq!(record.mode, tacsim::arena::MatchMode::Range);
    let targets: Vec<ObjId> = (1..=2)
        .map(|index| match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, index).expect("a pod"))
        .collect();
    let side = |team: PartyId| {
        record
            .teams
            .iter()
            .find(|roster| roster.team == team)
            .cloned()
            .expect("the side")
    };
    assert_eq!(side(PartyId::RED).units.len(), 3);
    assert_eq!(side(PartyId::BLUE).units, targets);
    assert!(side(PartyId::BLUE).pilots.is_empty(), "no target is seated");
    let squadded: Vec<ObjId> = record
        .squads
        .iter()
        .flat_map(|roster| roster.units.iter().copied())
        .collect();
    assert_eq!(
        squadded, targets,
        "every target and nothing else is a machine"
    );

    // The end, by `SIM ABORT`: the ring and the replay say it was a range.
    let match_id = rig.centre().match_id.expect("a running match");
    rig.say(0, "sim abort");
    o3_until_terminal(&mut rig).await;
    // The terminal centre still holds the range (only the returns to `Idle`
    // clear it) but no slot to read a side off: the bare read names the count
    // alone, and the board prints no range line.
    o3_invariant(&rig);
    assert_eq!(rig.say(2, "sim range"), "Range: 2 targets.");
    let board = lobby_board(&rig, 2);
    assert!(
        !board.iter().any(|line| line.contains("mode: range")),
        "{board:?}"
    );
    let history = rig.say(2, "sim history");
    assert!(
        history.contains(&format!("#{}  aborted [range]  ", match_id.0)),
        "{history}"
    );
    let replay = rig.say(2, &format!("sim replay {}", match_id.0));
    assert!(replay.contains("aborted [range])"), "{replay}");
    let row = rig
        .tw
        .world()
        .module_state::<match_::MatchBook>()
        .and_then(|book| book.remembered(match_id).cloned())
        .expect("the ring holds the match");
    assert_eq!(row.mode, Some(tacsim::arena::MatchMode::Range));
    let tank = side(PartyId::RED).units[0];
    let header = match_::report::header_lines(
        &row,
        "The Proving Ground",
        match_::report::Reader::Pilot { unit: tank },
        None,
        None,
    );
    assert!(
        header
            .iter()
            .any(|line| line.starts_with("You flew R") && line.ends_with("0 shots, 0 hits.")),
        "no shot, no rate: {header:?}"
    );
    assert!(
        header
            .first()
            .is_some_and(|line| line.contains("aborted [range], ")),
        "{header:?}"
    );

    // The return to `Idle`: the range goes, and the targets' pods get their
    // own tanks back.
    rig.staff("@arena/reset");
    assert_eq!(rig.state(), MatchState::Idle);
    o3_invariant(&rig);
    assert_eq!(rig.centre().mode, tacsim::arena::MatchMode::Skirmish);
    assert_eq!(o3_schema(&rig, PartyId::BLUE, 1), "tacsim.unit.line_tank");
    assert_eq!(o3_schema(&rig, PartyId::BLUE, 2), "tacsim.unit.line_tank");
    assert_eq!(rig.say(2, "sim range"), O3_NOT_SET);
}

/// **16b cut O.3, the fill ON** (plan items 13, 21, 39): `SIM AI` reads and
/// refuses honestly on a range, the board prints no `AI fill:` line under it,
/// the claim advice names the targets and not the machines, and at `SIM START`
/// the shooters' own unclaimed pods are **not** fielded — the fill takes no pod
/// on a range — while the targets are.
///
/// The lead asks for three a side **before** the range is set, so the stored
/// size (3) outlives the set and exceeds red's two claims: `ready()`'s fill loop
/// would put a machine in red's pod 3 were its range guard missing, and the
/// record below would hold three red units and a red squad.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o3_the_range_with_the_fill_on_fields_no_machines_and_says_so() {
    let mut rig = Rig::boot_with(2, y1_config());
    rig.say(0, "sim team red");
    assert_eq!(rig.say(0, "sim ai"), "AI fill: 1 a side.");
    let said = rig.say(0, "sim ai 3");
    assert!(said.contains("AI fill set to 3 a side."), "{said}");
    assert_eq!(rig.centre().ai_side_size, Some(3));
    assert!(
        lobby_board(&rig, 1)
            .iter()
            .any(|line| line.contains("AI fill:")),
        "the fill's line, before the range"
    );
    assert_eq!(
        rig.say(0, "sim range 1"),
        "Range set: 1 target stands on the blue side; the match ends when every target is out \
         of action, or on SIM ABORT."
    );
    o3_invariant(&rig);
    assert_eq!(rig.say(1, "sim ai"), "AI fill: none on a range.");
    assert_eq!(
        rig.say(0, "sim ai 4"),
        "The range fields no machines; SIM RANGE sets its targets."
    );
    assert_eq!(
        rig.centre().ai_side_size,
        Some(3),
        "the refused set stored nothing"
    );
    let board = lobby_board(&rig, 1);
    assert!(
        !board.iter().any(|line| line.contains("AI fill:")),
        "{board:?}"
    );
    assert!(
        board
            .iter()
            .any(|line| line == "  mode: range \u{2014} 1 target on the blue side"),
        "{board:?}"
    );
    let said = rig.say(1, "sim team red");
    assert!(
        said.contains("the targets stand on the blue side.") && !said.contains("machines"),
        "{said}"
    );
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    for n in 0..2 {
        rig.walk_to(n, match_::RED_ROOM_ENTRY);
        rig.say(n, "board");
    }
    assert_eq!(rig.state(), MatchState::Running);
    let record = rig.record();
    let side = |team: PartyId| {
        let mut units = record
            .teams
            .iter()
            .find(|roster| roster.team == team)
            .map(|roster| roster.units.clone())
            .unwrap_or_default();
        units.sort();
        units
    };
    let pod = |team: PartyId, index: u8| {
        match_::tank_for_slot(rig.tw.world(), team, index).expect("that pod's tank")
    };
    let mut claimed = vec![pod(PartyId::RED, 1), pod(PartyId::RED, 2)];
    claimed.sort();
    assert_eq!(
        side(PartyId::RED),
        claimed,
        "the two claimed and no machine, though the fill was sized to three"
    );
    let unclaimed = pod(PartyId::RED, 3);
    assert!(
        record
            .teams
            .iter()
            .all(|roster| !roster.units.contains(&unclaimed)),
        "red's unclaimed pod 3 was fielded: {:?}",
        record.teams
    );
    let target = pod(PartyId::BLUE, 1);
    assert_eq!(side(PartyId::BLUE), vec![target]);
    assert_eq!(o3_schema(&rig, PartyId::BLUE, 1), "tacsim.unit.target_mk1");
    let squadded: Vec<ObjId> = record
        .squads
        .iter()
        .flat_map(|roster| roster.units.iter().copied())
        .collect();
    assert_eq!(
        squadded,
        vec![target],
        "the target is the only machine: {:?}",
        record.squads
    );
    assert_eq!(rig.say(1, "sim ai"), "AI fill: none on a range.");
    rig.say(0, "sim abort");
    o3_until_terminal(&mut rig).await;
}

/// **16b cut O.3: the range's own end**, under paused time (plan item 26's
/// pattern) — one pilot, `SIM RANGE 1`, the shooter posed four cells behind
/// the target with both facing east, so every round lands on the rear band.
/// Started with the fill off, started up, locked on `BA` and fired until the
/// board's head is terminal: the head reads the elimination, the result and
/// replay documents carry `"mode": "range"`, `SIM HISTORY` tags the row, and
/// the pilot's own header line carries the hit rate off the real counters.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn o3_a_range_ends_when_its_target_is_out_of_action() {
    let mut rig = Rig::boot(1);
    rig.say(0, "sim team red");
    rig.say(0, "sim range 1");
    for (callsign, x) in [("BA", 130), ("RA", 126)] {
        let said = rig.staff(&format!("@arena/pose {callsign}={x} 97 90"));
        assert!(
            said.contains(&format!("{callsign} opens at cell ({x}, 97) heading 90")),
            "{said}"
        );
    }
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.say(0, "board");
    assert_eq!(rig.state(), MatchState::Running);
    let match_id = rig.centre().match_id.expect("a running match");
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "started up");
    // The target, told apart: `BA`.
    let mut locked = false;
    for _ in 0..2_000 {
        rig.pump();
        let told = rig.frame(0).is_some_and(|frame| {
            frame
                .contacts
                .iter()
                .any(|contact| contact.designator.to_string() == "BA")
        });
        if told {
            let said = rig.say(0, "lock BA");
            assert!(said.contains("Locking BA."), "{said}");
            locked = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(locked, "the target was never told apart");
    // Fire once an arena second until the board's head is terminal.
    for tick in 0..20_000_u32 {
        rig.pump();
        if rig.state().is_terminal() {
            break;
        }
        if tick % 100 == 0 {
            rig.say(0, "fire");
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        rig.state(),
        MatchState::Ended,
        "no end by {} arena ms",
        rig.staged_now_ms
    );
    let board = lobby_board(&rig, 0);
    assert_eq!(
        board.first().map(String::as_str),
        Some(format!("Simulator centre: ended (match {match_id}) \u{2014} red").as_str()),
        "{board:?}"
    );
    let writes: Vec<(String, serde_json::Value)> = rig
        .pump_ops()
        .into_iter()
        .filter(|op| op.ns == tacsim::NS)
        .filter_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => Some((key.to_string(), value)),
            _ => None,
        })
        .collect();
    let document = |key: String| {
        writes
            .iter()
            .find(|(held, _)| *held == key)
            .map(|(_, value)| value.clone())
            .unwrap_or_else(|| panic!("{key} was written"))
    };
    let result = document(format!("tacsim:match:{}", match_id.0));
    assert_eq!(result["mode"], serde_json::json!("range"));
    assert_eq!(result["outcome"], serde_json::json!("red"));
    assert_eq!(result["reason"], serde_json::json!("elimination"));
    let replay = document(format!("tacsim:replay:{}", match_id.0));
    assert_eq!(replay["mode"], serde_json::json!("range"));
    let history = rig.say(0, "sim history");
    assert!(
        history.contains(&format!(
            "#{}  red won (blue eliminated) [range]",
            match_id.0
        )),
        "{history}"
    );
    let row = rig
        .tw
        .world()
        .module_state::<match_::MatchBook>()
        .and_then(|book| book.remembered(match_id).cloned())
        .expect("the ring holds the match");
    let flown = row
        .units
        .iter()
        .find(|unit| unit.callsign == "RA")
        .cloned()
        .expect("RA's row");
    assert!(
        flown.shots >= 1 && flown.hits >= 1,
        "{} shots, {} hits",
        flown.shots,
        flown.hits
    );
    let per_cent = flown.hits * 100 / flown.shots;
    let header = match_::report::header_lines(
        &row,
        "The Proving Ground",
        match_::report::Reader::Pilot { unit: flown.unit },
        None,
        None,
    );
    assert!(
        header
            .iter()
            .any(|line| line.starts_with("You flew RA: ")
                && line.ends_with(&format!(" ({per_cent}%)."))),
        "{header:?}"
    );
}

/// **16b cut AE.2**: a miss on a target the platform no longer holds reads
/// `Miss.` and nothing more — the arena stages it with no reason, because
/// `moved`, `blocked` and `cover` are facts about where the target is that the
/// platform's sensors have not earned once the contact is gone. A held
/// contact's miss renders as it always did.
#[test]
fn ae2_an_unheld_miss_reads_miss_and_nothing_more() {
    use tacsim::arena::perception::{FrameNotice, Named};
    use tacsim::arena::state::Designator;

    let frame = fixture::frame();
    let rows = cockpit::frame_contacts(&frame);
    let render = |notice: FrameNotice| cockpit::notice_line(&frame, &notice, &rows);
    assert_eq!(
        render(FrameNotice::Miss {
            launch_cell: None,
            contact: None,
            reason: smol_str::SmolStr::new_static(""),
        }),
        "Miss."
    );
    let held = frame.lock.unwrap_or_else(|| panic!("the scene's lock"));
    assert_eq!(
        render(FrameNotice::Miss {
            launch_cell: None,
            contact: Some(Named {
                id: held,
                designator: Designator::Provisional(2),
            }),
            reason: smol_str::SmolStr::new(tacsim::arena::damage::MOVED),
        }),
        "Miss on X2: the target moved."
    );
}

// ---------------------------------------------------------------------------
// 16b cut AF.3 (16c N16) — the pilots come back through the reclaim
// ---------------------------------------------------------------------------

/// **The store, as one process's writes left it** (16b cut AF.3): per module
/// namespace, per key, the last document written. `match_lifecycle.rs` owns
/// the rig this copies the steps of ([`Rig::reboot_mid_match`]).
type StoreImage = BTreeMap<smol_str::SmolStr, BTreeMap<smol_str::SmolStr, serde_json::Value>>;

impl Rig {
    /// Take the module writes queued since the last take, fold them into the
    /// store image and hand them back (16b cut AF.3).
    fn pump_ops(&mut self) -> Vec<mush_core::ModuleOp> {
        let ops = self.tw.take_module_ops();
        for op in &ops {
            let docs = self.image.entry(op.ns.clone()).or_default();
            match &op.op {
                mush_core::ModuleWrite::JsonSet { key, value } => {
                    docs.insert(key.clone(), value.clone());
                }
                mush_core::ModuleWrite::Del { key } => {
                    docs.remove(key);
                }
                mush_core::ModuleWrite::StreamAppend { .. } => {}
            }
        }
        ops
    }

    /// Restart the server half over the retained module image with fresh owner
    /// slots, including the one-shot BulkService. Stop/join old arena tasks and
    /// discard undrained egress as a crashed process would. The real config,
    /// account map and previously connected pilots survive this split-server
    /// fixture; a pilot deliberately disconnected before boot stays disconnected.
    /// Boot/reannouncement output is discarded, while the actual deferred-resume
    /// fence keeps notices owed to the existing first-tick assertions.
    async fn reboot_mid_match(&mut self) {
        let cfg = self
            .tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .expect("the configured Tacsim owner")
            .config()
            .clone();
        let connected: Vec<ObjId> = self
            .pilots
            .iter()
            .copied()
            .filter(|pilot| self.tw.world().is_connected(*pilot))
            .collect();
        let old = self
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
            .expect("the arena registry");
        old.shutdown(tacsim::arena_bridge::JOIN_TIMEOUT).await;
        if let Some(rx) = self.egress.as_mut() {
            while rx.try_recv().is_ok() {}
        }
        let _ = self.pump_ops();
        let data: std::collections::HashMap<smol_str::SmolStr, mush_core::ModuleBootData> = self
            .image
            .iter()
            .map(|(ns, docs)| {
                let mut data = mush_core::ModuleBootData {
                    kv: docs
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                    ..Default::default()
                };
                data.normalize();
                (ns.clone(), data)
            })
            .collect();
        let old_world = std::mem::replace(&mut self.tw, TestWorld::new());
        self.tw = old_world
            .reboot_into(roster_with(cfg), data)
            .expect("actual fresh production boot");
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
        for pilot in &connected {
            self.tw.connect(*pilot);
        }
        for pilot in &self.pilots {
            assert_eq!(
                self.tw.world().is_connected(*pilot),
                connected.contains(pilot),
                "the split-server fixture preserves each pilot's session presence"
            );
        }
        self.egress = self
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
            .and_then(|tasks| tasks.take_egress());
        assert!(self.egress.is_some(), "the fresh process owns arena egress");
        let _ = self.pump_ops();
    }

    /// Pump until the image holds a snapshot of the running match taken at or
    /// after arena millisecond `at_least`, and answer its head.
    async fn af3_snapshot_at_or_after(&mut self, at_least: i64) -> tacsim::arena::SnapshotHead {
        let match_id = self.centre().match_id.expect("a running match");
        let key = match_::snapshot_key(match_id).to_string();
        for _ in 0..3_000 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            self.pump();
            let _ = self.pump_ops();
            if let Some(head) = self
                .image
                .get(tacsim::NS)
                .and_then(|docs| docs.get(key.as_str()))
                .and_then(|document| tacsim::arena::Snapshot::head(document).ok())
                .filter(|head| head.at_ms >= at_least)
            {
                return head;
            }
        }
        panic!("no snapshot at or after {at_least} ms");
    }

    /// The arena clock the world's mirror holds for the running match.
    fn af3_arena_ms(&self) -> i64 {
        let match_id = self.centre().match_id.expect("a running match");
        self.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.mirror(match_id))
            .map_or(0, |mirror| mirror.now_ms)
    }
}

/// The `Sys` lines one actor was sent since `from` in the rig's log.
fn af3_said(rig: &Rig, actor: ObjId, from: usize) -> Vec<String> {
    rig.log
        .iter()
        .skip(from)
        .filter_map(|(audience, event)| match event {
            OutputEvent::Sys(text) if *audience == Audience::Actor(actor) => {
                Some(text.to_plain_text())
            }
            _ => None,
        })
        .collect()
}

/// `away_s` off a cockpit frame's catch-up.
fn af3_away_s(frame: &serde_json::Value) -> i64 {
    frame
        .pointer("/catchup/away_s")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_else(|| panic!("the frame carries no catch-up: {frame}"))
}

/// `[tacsim]` for the cockpit's resume tests: this file's [`config`] with a
/// snapshot every arena second.
fn af3_config() -> TacsimConfig {
    let mut cfg = config();
    cfg.arena.snapshot_ms = 1_000;
    cfg
}

/// **The pilots come back through the reclaim after a resume** (16b cut AF.3;
/// the brief's test 6), over this file's rig rebooted mid-match. Four pilots:
/// red 1 (pilot 0) and blue 1 (pilot 1) seated when the server stops, red 2
/// (pilot 2) who typed `EXIT` before the snapshot, blue 2 (pilot 3) who
/// dropped (a `QUIT`) before it. Every session survives the restart, as the
/// split's do. Red 1's tank opens 62 cells west of blue's on the east-west
/// road, out of every blue sight, and is driven into them only after blue 2's
/// drop: the contact blue 2's tank gains meanwhile.
///
/// - Before any tick, a reconnect says nothing to a pilot the resume owes the
///   two lines (the tick is the path that says them); `STATUS` answers `You
///   are not at the station of a deployed unit.`; the board reads `flown by
///   its mind` on every slot the resume released, and the other side's reduced
///   row is byte-identical to the one it read before the restart (Y.3's,
///   withholding it); `BOARD` inside `reclaim_min_ms` answers `HandingOver`
///   with the seconds.
/// - Blue 1 takes the tank back before the tick fires (the world's clock moved
///   with no tick), and the tick then says nothing to them.
/// - The first boarding tick says red 1 the resume line then Y.3's line (`…
///   is flying on its mind; …`, `… on doctrine; …` until 17b cut M5.3), once,
///   and sends red 1's attached session exactly one cleared cockpit
///   keyframe (the abandoned timeline's last panel); a later tick says nothing,
///   and a login after it reads Y.3's line alone.
/// - Red 1 past the floor: `You climb back into … is yours again where it
///   stands.`; the first frame's `catchup` counts `away` from the snapshot's
///   moment and the line client reads `Back aboard RA after …`; `HEADING 90` is
///   taken; an `EXIT` afterwards is one-way.
/// - Red 2 reads `LeftForGood`.
/// - Blue 2 reconnects to Y.3's line alone — no resume line — and after the
///   floor takes the tank back with a catch-up counted from their own drop
///   (the carried `at_release`), which lists red 1's tank as gained: it was
///   gained after the drop and before the snapshot, so a baseline rebuilt at
///   the snapshot's moment would list nothing new.
/// - A second rig, the pilot away across the restart: the first tick holds the
///   owed lines, the hold runs out unsaid, a later login says both, once, the
///   tick after it nothing, and a login after that Y.3's line alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn af3_the_pilots_board_the_resumed_match_through_the_reclaim() {
    let mut rig = Rig::boot_with(4, af3_config());
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(2, "sim team red");
    rig.say(3, "sim team blue");
    // Red 1's tank opens 62 cells west of blue's muster, on the road: 620 m,
    // past the 600 m every blue sight reaches.
    let said = rig.staff("@arena/pose RA=187 97 90");
    assert!(
        said.contains("RA opens at cell (187, 97) heading 90 for the next match."),
        "{said}"
    );
    rig.say(0, "sim start");
    for (n, room) in [
        (0, match_::RED_ROOM_ENTRY),
        (1, match_::BLUE_ROOM_ENTRY),
        (2, match_::RED_ROOM_ENTRY),
        (3, match_::BLUE_ROOM_ENTRY),
    ] {
        rig.walk_to(n, room);
        rig.say(n, "board");
    }
    assert_eq!(rig.state(), MatchState::Running);
    let match_id = rig.centre().match_id.expect("a running match");
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the arena brings power online");
    let (red1, blue1, red2, blue2) = (rig.pilot(0), rig.pilot(1), rig.pilot(2), rig.pilot(3));
    let (tank0, pod0) = (rig.tank(0), rig.pod(0).expect("red 1's pod"));
    // Before the snapshot: red 2 climbs out for good, blue 2 drops holding no
    // contact.
    let said = rig.say(2, "exit");
    assert!(!said.is_empty(), "{said}");
    // Blue's two tanks hold each other as contacts — a mate is a contact the
    // sight has classified as its own side — so blue 2 drops once blue 1's
    // frame holds its mate classified and nothing west of blue's entry zone:
    // blue 2's tank then holds blue 1's (the contact held at the drop and at
    // the reclaim alike) and red 1's tank is in no blue sight.
    let mut settled = false;
    for _ in 0..1_000 {
        rig.pump();
        if rig.frame(1).is_some_and(|frame| {
            frame.contacts.iter().any(|contact| {
                contact.class
                    == tacsim::arena::state::Classification::Classified { party: PartyId::BLUE }
            }) && frame
                .contacts
                .iter()
                .all(|contact| contact.cell.is_some_and(|(x, _)| x >= 240))
        }) {
            settled = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        settled,
        "blue's sights never settled on their own side alone: {:?}",
        rig.frame(1).map(|frame| frame.contacts)
    );
    rig.fire_conn(blue2, mush_core::ConnTransition::Disconnected);
    let drop_ms = rig.af3_arena_ms();
    // A contact gained meanwhile: red 1 drives east along the road into blue's
    // sights and stops some 46 cells short of blue's tanks — outside the band a
    // withdrawing machine turns to fight in — and blue 1's own frame says when
    // blue's sights hold it; blue 2's tank, two rows from blue 1's, holds it
    // too.
    let said = rig.say(0, "speed 54");
    assert!(said.contains("Making 54 kph"), "{said}");
    let mut gained = false;
    for _ in 0..3_000 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
        // Red 1's tank, and not a blue return: west of blue's entry zone.
        gained = gained
            || rig.frame(1).is_some_and(|frame| {
                frame
                    .contacts
                    .iter()
                    .any(|contact| contact.cell.is_some_and(|(x, _)| x < 240))
            });
        if gained
            && rig
                .frame(0)
                .and_then(|frame| frame.cell)
                .is_some_and(|(x, _)| x >= 203)
        {
            break;
        }
    }
    assert!(gained, "blue's sights never held red 1's tank");
    let said = rig.say(0, "speed 0");
    assert!(!said.is_empty(), "{said}");
    let seen_ms = rig.af3_arena_ms();
    let head = rig
        .af3_snapshot_at_or_after((seen_ms + 2_000).max(drop_ms + 2_000))
        .await;
    // What the other side reads of red, before the restart.
    let theirs = rig.say(1, "sim status");
    let red_row_before = theirs
        .lines()
        .find(|line| line.trim_start().starts_with("red:"))
        .unwrap_or_else(|| panic!("no red row: {theirs}"))
        .to_owned();

    rig.reboot_mid_match().await;
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .is_some_and(|registry| registry.tasks().is_live(match_id)),
        "it resumed"
    );
    for (n, pilot) in [(0, red1), (1, blue1)] {
        let slot = rig.centre().slot_of_pilot(pilot).cloned().expect("a slot");
        assert!(
            slot.is_dropped() && slot.released_by_resume,
            "{n}: {slot:?}"
        );
        assert_eq!(
            rig.location(n),
            rig.pod(n),
            "{n}: the resume left the body in the pod"
        );
    }
    let blue2_slot = rig.centre().slot_of_pilot(blue2).cloned().expect("a slot");
    assert!(blue2_slot.is_dropped() && !blue2_slot.released_by_resume);
    let red2_slot = rig.centre().slot_of_pilot(red2).cloned().expect("a slot");
    assert!(red2_slot.left && red2_slot.dropped_at_ms.is_none());

    // Before any tick: a reconnect says nothing to a pilot the tick owes.
    let events = rig.fire_conn(red1, mush_core::ConnTransition::Connected);
    let said = rig.tw.seen_by(&events, red1).join("\n");
    assert!(
        !said.contains("resumed") && !said.contains("flying on"),
        "the reconnect said what the tick owes: {said}"
    );
    let said = rig.say(0, "status");
    assert!(
        said.contains("You are not at the station of a deployed unit."),
        "{said}"
    );
    let board = tacsim::match_::status_lines(rig.tw.world(), BoardReader::Staff).join("\n");
    assert_eq!(
        board.matches(", flown by its mind").count(),
        3,
        "red 1, blue 1 and blue 2: {board}"
    );
    let theirs = rig.say(1, "sim status");
    let red_to_blue = theirs
        .lines()
        .find(|line| line.trim_start().starts_with("red:"))
        .unwrap_or_else(|| panic!("no red row: {theirs}"));
    assert_eq!(
        red_to_blue, red_row_before,
        "the other side's reduced row moved across the resume: {theirs}"
    );
    assert!(
        !red_to_blue.contains("flown by") && !red_to_blue.contains("left"),
        "the other side read the resume: {theirs}"
    );
    let tank0_name = mush_core::output::display_name(rig.tw.world(), tank0);
    let said = rig.say(0, "board");
    assert!(
        said.contains(&format!(
            "{tank0_name} is still being handed over; try again in 5 second(s)."
        )),
        "{said}"
    );

    // Blue 1 takes the tank back before the tick fires.
    let now = rig.now;
    rig.now = now + 6;
    rig.tw.set_now(now + 6);
    let said = rig.say(1, "board");
    assert!(said.contains("is yours again where it stands."), "{said}");

    // The first tick: red 1 reads the two lines once, and one cleared panel.
    let mark = rig.log.len();
    rig.tick_to(now + 6);
    let lines = af3_said(&rig, red1, mark);
    let on_its_mind = format!("{tank0_name} is flying on its mind; BOARD takes it back.");
    assert_eq!(
        lines,
        vec![match_::RESUME_LINE.to_owned(), on_its_mind.clone()],
        "the tick's two lines, in order"
    );
    assert!(
        af3_said(&rig, blue1, mark).is_empty(),
        "a pilot who took the tank back is told nothing"
    );
    let cleared_to_red1 = rig.log[mark..]
        .iter()
        .filter(|(audience, event)| {
            *audience == Audience::Actor(red1)
                && matches!(
                    event,
                    OutputEvent::Panel { feed, data, .. }
                        if feed.as_str() == tacsim::arena_bridge::FRAME_FEED
                            && *data == cockpit::cockpit_cleared()
                )
        })
        .count();
    assert_eq!(
        cleared_to_red1, 1,
        "one cleared keyframe to red 1's session"
    );
    let mark = rig.log.len();
    rig.tick_to(now + 7);
    assert!(af3_said(&rig, red1, mark).is_empty(), "said once");
    // And a login after the tick said them reads Y.3's line alone.
    let events = rig.fire_conn(red1, mush_core::ConnTransition::Connected);
    assert_eq!(
        rig.tw.seen_by(&events, red1),
        vec![on_its_mind.clone()],
        "the resume line is said once, whichever path said it"
    );

    // Red 1 takes the tank back: the reclaim and the catch-up.
    let mark = rig.log.len();
    let events = rig.events(0, "board");
    let said = rig.tw.seen_by(&events, red1).join("\n");
    assert!(
        said.contains(&format!(
            "You climb back into {}: {tank0_name} is yours again where it stands.",
            mush_core::output::display_name(rig.tw.world(), pod0)
        )),
        "{said}"
    );
    let frames = y4_frames_until(&mut rig, red1, mark, 1).await;
    let first = frames.first().cloned().expect("a frame after the reclaim");
    let away = af3_away_s(&first);
    let since_snapshot = (rig.af3_arena_ms() - head.at_ms) / 1_000;
    assert!(
        (since_snapshot - 1..=since_snapshot + 1).contains(&away),
        "away {away} s is counted from the snapshot's moment ({since_snapshot} s)"
    );
    assert!(
        af3_said(&rig, red1, mark)
            .iter()
            .any(|line| line.starts_with("Back aboard RA after")),
        "the line client reads the catch-up"
    );
    let said = rig.say(0, "heading 90");
    assert!(said.contains("Steering to 90"), "{said}");
    let said = rig.say(0, "exit");
    assert!(!said.is_empty(), "{said}");
    let said = rig.say(0, "board");
    assert!(
        said.contains(&format!(
            "You left {tank0_name} for the rest of the match; it cannot be taken back."
        )),
        "{said}"
    );

    // Red 2 left before the snapshot, for good.
    let said = rig.say(2, "board");
    assert!(
        said.contains("for the rest of the match; it cannot be taken back."),
        "{said}"
    );

    // Blue 2 dropped before the snapshot: Y.3's line alone, and a catch-up
    // from their own drop.
    let tank3 = rig.tank(3);
    let events = rig.fire_conn(blue2, mush_core::ConnTransition::Connected);
    let said = rig.tw.seen_by(&events, blue2).join("\n");
    assert!(
        said.contains(&format!(
            "{} is flying on its mind; BOARD takes it back.",
            mush_core::output::display_name(rig.tw.world(), tank3)
        )),
        "{said}"
    );
    assert!(!said.contains(match_::RESUME_LINE), "{said}");
    let mark = rig.log.len();
    let said = rig.say(3, "board");
    assert!(said.contains("is yours again where it stands."), "{said}");
    let frames = y4_frames_until(&mut rig, blue2, mark, 1).await;
    let first = frames.first().cloned().expect("a frame after the reclaim");
    let away = af3_away_s(&first);
    let since_drop = (rig.af3_arena_ms() - drop_ms) / 1_000;
    assert!(
        away >= since_drop - 1,
        "away {away} s is counted from the drop ({since_drop} s), not the snapshot"
    );
    assert!(
        away > (rig.af3_arena_ms() - head.at_ms) / 1_000,
        "away {away} s is longer than the time since the snapshot"
    );
    // The contact gained meanwhile: red 1's tank came into blue's sights after
    // blue 2's drop and before the snapshot, so it is new against the carried
    // `at_release` row — and would not be against a row rebuilt at the
    // snapshot's moment.
    let catchup = first.get("catchup").cloned().unwrap_or_default();
    let gained: Vec<String> = catchup["contacts_new"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    assert!(
        !gained.is_empty(),
        "no contact gained meanwhile in blue 2's catch-up: {catchup}"
    );
    // Every gained name is held on the frame, and one of them is red 1's tank:
    // the return west of blue's entry zone.
    let held: Vec<(String, Option<u64>)> = first["contacts"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    let name = row["designator"].as_str()?.to_owned();
                    Some((
                        name,
                        row.pointer("/cell/0").and_then(serde_json::Value::as_u64),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    for name in &gained {
        assert!(
            held.iter().any(|(held, _)| held == name),
            "{name} is gained but not held on the frame: {first}"
        );
    }
    assert!(
        held.iter()
            .any(|(name, x)| gained.contains(name) && x.is_some_and(|x| x < 240)),
        "red 1's tank is not among the contacts gained meanwhile: {catchup} / {first}"
    );
    // And blue 1's tank, held at the drop and at the reclaim, is not new: the
    // carried row kept what blue 2's tank held, not only when it dropped.
    assert!(
        held.iter()
            .filter(|(name, _)| gained.contains(name))
            .all(|(_, x)| x.is_some_and(|x| x < 240)),
        "a contact held since before the drop reads as gained: {catchup} / {first}"
    );

    // A second rig, the pilot away across the restart (a fresh server half's
    // connection map is empty): the first tick holds the owed lines, the hold
    // runs out unsaid, and a later login says both, once.
    let mut rig = Rig::boot_with(2, af3_config());
    rig.fly().await;
    let _ = rig.af3_snapshot_at_or_after(0).await;
    let pilot = rig.pilot(0);
    let tank = rig.tank(0);
    rig.tw.world_mut().set_connected(pilot, false);
    rig.reboot_mid_match().await;
    let owed = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .is_some_and(|book| book.resume_lines_deferred(pilot))
    };
    let now = rig.now;
    let mark = rig.log.len();
    rig.tick_to(now + 1);
    assert!(
        af3_said(&rig, pilot, mark).is_empty(),
        "nobody attached to tell"
    );
    assert!(owed(&rig), "the first tick holds the lines");
    assert_eq!(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .and_then(match_::MatchBook::resume_hold_until_s),
        Some(now + 1 + match_::HATCH_HOLD_S),
        "held from the first tick that held them"
    );
    rig.tick_to(now + match_::HATCH_HOLD_S);
    assert!(owed(&rig), "still held inside the hold");
    rig.tick_to(now + 1 + match_::HATCH_HOLD_S);
    assert!(!owed(&rig), "dropped unsaid past the hold");
    assert!(
        af3_said(&rig, pilot, mark).is_empty(),
        "nothing was said to a body with no session"
    );
    rig.tw.world_mut().set_connected(pilot, true);
    let on_its_mind = format!(
        "{} is flying on its mind; BOARD takes it back.",
        mush_core::output::display_name(rig.tw.world(), tank)
    );
    let events = rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    assert_eq!(
        rig.tw.seen_by(&events, pilot),
        vec![match_::RESUME_LINE.to_owned(), on_its_mind.clone()],
        "a later login says the two lines"
    );
    let mark = rig.log.len();
    let now = rig.now;
    rig.tick_to(now + 1);
    assert!(
        af3_said(&rig, pilot, mark).is_empty(),
        "and the tick nothing"
    );
    let events = rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    assert_eq!(
        rig.tw.seen_by(&events, pilot),
        vec![on_its_mind],
        "a login after that reads Y.3's line alone"
    );
}

// ---------------------------------------------------------------------------
// 16b cut AH.1 (16c N18) — the three frame-borne hints
// ---------------------------------------------------------------------------

use tacsim::match_::tutorial::{HINT_LINES, HINT_PREFIX, HintPoint, SET_ON, frame_points};

impl Rig {
    /// **16b cut AH.1**: [`Rig::push_frame`] for an actor outside
    /// [`Rig::pilots`]'s order — the same staging of one `ArenaOut::Frames`
    /// through the real egress seam, addressed to `actor`.
    fn push_frame_for(&mut self, actor: ObjId, frame: tacsim::arena::step::Frame) {
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
                frames: vec![(actor, frame)],
                now_ms: 0,
                observer: None,
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

    /// **16b cut AH.1**: rebind one pilot the way a reclaim does — the seat
    /// dropped (and with it the held frame) and taken again — so the pilot's
    /// next frame is a first frame of the match.
    fn rebind(&mut self, pilot: ObjId, match_id: tacsim::arena_task::MatchId, unit: ObjId) {
        let registry = self
            .tw
            .world_mut()
            .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the arena registry is composed");
        registry.unbind_pilot(pilot);
        registry.bind_pilot(pilot, match_id, unit);
    }
}

/// **16b cut AH.1**: every hint in `log` is `Sys` output on the arena's reply
/// category — never the notice tag both panel rings drop — and no panel payload
/// carries one: a hint never rides a frame.
fn audit_hints(log: &[(Audience, OutputEvent)]) {
    let mut hints = 0_usize;
    for (audience, event) in log {
        match event {
            OutputEvent::Panel { feed, data, .. } => {
                let payload = serde_json::to_string(data).expect("a panel payload serializes");
                assert!(
                    !payload.contains("Hint:"),
                    "a hint rode the {feed} panel to {audience:?}"
                );
            }
            OutputEvent::Sys(text) if text.to_plain_text().starts_with(HINT_PREFIX) => {
                hints += 1;
                let tags: Vec<Option<&str>> = text
                    .spans()
                    .iter()
                    .map(|span| span.tag.as_deref())
                    .collect();
                assert_eq!(
                    tags.first().copied().flatten(),
                    Some(tacsim::arena_bridge::NOTICE_CATEGORY),
                    "a hint is a reply on the arena's category: {tags:?}"
                );
                assert!(
                    !tags.contains(&Some(tacsim::arena_bridge::NOTICE_SENTENCE_TAG)),
                    "a hint is never a frame's own notice: {tags:?}"
                );
            }
            _ => {}
        }
    }
    assert!(hints > 0, "the audit read at least one hint");
}

/// What one actor was sent in `log`, in order: `Some(text)` for a `Sys` line,
/// `None` for an `arena.cockpit` keyframe.
fn ah1_stream(log: &[(Audience, OutputEvent)], actor: ObjId) -> Vec<Option<String>> {
    log.iter()
        .filter(|(audience, _)| *audience == Audience::Actor(actor))
        .filter_map(|(_, event)| match event {
            OutputEvent::Sys(text) => Some(Some(text.to_plain_text())),
            OutputEvent::Panel { feed, .. }
                if feed.as_str() == tacsim::arena_bridge::FRAME_FEED =>
            {
                Some(None)
            }
            _ => None,
        })
        .collect()
}

/// Every hint one actor was sent in `log`, in order.
fn ah1_hints(log: &[(Audience, OutputEvent)], actor: ObjId) -> Vec<String> {
    ah1_stream(log, actor)
        .into_iter()
        .flatten()
        .filter(|line| line.starts_with(HINT_PREFIX))
        .collect()
}

/// Where `line` sits in `stream`, insisting it is there.
fn ah1_at(stream: &[Option<String>], line: &str) -> usize {
    stream
        .iter()
        .position(|said| said.as_deref() == Some(line))
        .unwrap_or_else(|| panic!("{line:?} was not said: {stream:?}"))
}

/// A `Contact` notice classifying one contact as `team`'s.
fn ah1_classified(team: PartyId) -> tacsim::arena::perception::FrameNotice {
    use tacsim::arena::perception::{FrameNotice, Named, NoticeKind};
    use tacsim::arena::state::{Classification, ContactId, Designator};
    FrameNotice::Contact {
        kind: NoticeKind::Classified,
        contact: Named {
            id: ContactId(0x5555_5555_5555_5555),
            designator: Designator::Slot { team, slot: 1 },
        },
        class: Classification::Classified { party: team },
        was: None,
    }
}

/// **16b cut AH.1**: the boarding hint at `SIM START`, the panel hint on the
/// first frame AFTER its keyframe, and the power hint after `Power online.` and
/// after that step's keyframe — each once, each on the reply tag.
///
/// The `@tutorial` counts read `of 6` (were `of 5` at 16b cut AH.1): the table
/// holds six since `HintPoint::Mind`, said at a match's end (17b cut M5.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ah1_the_first_frame_and_power_online_each_say_their_hint_once_after_the_keyframe() {
    let mut rig = Rig::boot(2);
    let p0 = rig.pilot(0);
    rig.fly().await;
    assert_eq!(
        ah1_hints(&rig.log, p0),
        vec![HINT_LINES[1]],
        "SIM START said the boarding hint, and the move_to placement no lobby hint"
    );
    let mark = rig.log.len();
    assert!(rig.frame_for(0).await, "the arena spoke to pilot 0");
    let stream = ah1_stream(&rig.log[mark..], p0);
    let panel_hint = ah1_at(&stream, HINT_LINES[2]);
    assert!(
        stream[..panel_hint].contains(&None),
        "the panel hint follows the first keyframe: {stream:?}"
    );
    assert_eq!(
        ah1_hints(&rig.log[mark..], p0),
        vec![HINT_LINES[2]],
        "{stream:?}"
    );

    let mark = rig.log.len();
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the power came up");
    let stream = ah1_stream(&rig.log[mark..], p0);
    let online = ah1_at(&stream, "Power online.");
    let power_hint = ah1_at(&stream, HINT_LINES[3]);
    assert!(online < power_hint, "{stream:?}");
    assert!(
        stream[online..power_hint].contains(&None),
        "the power hint follows that step's keyframe: {stream:?}"
    );
    assert_eq!(
        ah1_hints(&rig.log[mark..], p0),
        vec![HINT_LINES[3]],
        "{stream:?}"
    );

    let mark = rig.log.len();
    for _ in 0..5 {
        rig.pump();
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(ah1_hints(&rig.log[mark..], p0), Vec::<String>::new());
    let said = rig.say(0, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 3 of 7."),
        "boarding, panel and power: {said}"
    );
    audit_hints(&rig.log);
}

/// **16b cut AH.1**: the contact hint waits for an ENEMY told apart, in a unit
/// with a gun, with power up — and the panel hint for a first frame with power
/// off; a bound seat that holds no centre slot reads neither.
///
/// The `@tutorial` counts read `of 6` (were `of 5` at 16b cut AH.1): the table
/// holds six since `HintPoint::Mind`, said at a match's end (17b cut M5.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ah1_the_contact_hint_waits_for_an_enemy_a_gun_and_power_and_the_panel_hint_for_power_off()
{
    use tacsim::arena::state::Power;

    let mut rig = Rig::boot_with(3, config());
    let (p0, p2) = (rig.pilot(0), rig.pilot(2));
    let said = rig.say(0, "@tutorial off");
    assert!(said.contains("Hints are off"), "{said}");
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena spoke to pilot 0");
    assert_eq!(
        ah1_hints(&rig.log, p0),
        Vec::<String>::new(),
        "off: no boarding and no panel hint"
    );
    let said = rig.say(0, "@tutorial on");
    assert!(said.contains(SET_ON), "{said}");
    let said = rig.say(0, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 0 of 7."),
        "{said}"
    );

    // From here on nothing is pumped: every frame on_frames reads is staged.
    let held = rig.held_frame(0);
    assert!(!held.weapons.is_empty(), "the stock tank carries its guns");
    let own = held.team;
    let other = match own {
        PartyId::RED => PartyId::BLUE,
        PartyId::BLUE => PartyId::RED,
    };
    let frame = |power: Power, gun: bool, notices: Vec<tacsim::arena::perception::FrameNotice>| {
        let mut frame = held.clone();
        frame.power = power;
        if !gun {
            frame.weapons.clear();
        }
        frame.notices = notices;
        frame
    };
    let quiet = |rig: &mut Rig, frame: tacsim::arena::step::Frame, why: &str| {
        let mark = rig.log.len();
        rig.push_frame(0, frame);
        assert_eq!(
            ah1_hints(&rig.log[mark..], p0),
            Vec::<String>::new(),
            "{why}"
        );
    };
    quiet(
        &mut rig,
        frame(Power::Online, true, vec![ah1_classified(own)]),
        "a friendly told apart",
    );
    quiet(
        &mut rig,
        frame(Power::Online, false, vec![ah1_classified(other)]),
        "no gun to FIRE",
    );
    quiet(
        &mut rig,
        frame(Power::Off, true, vec![ah1_classified(other)]),
        "FIRE would answer Power not online",
    );
    let mark = rig.log.len();
    rig.push_frame(0, frame(Power::Online, true, vec![ah1_classified(other)]));
    let stream = ah1_stream(&rig.log[mark..], p0);
    let contact_hint = ah1_at(&stream, HINT_LINES[4]);
    assert!(
        stream[..contact_hint].contains(&None),
        "after that step's keyframe: {stream:?}"
    );
    assert_eq!(ah1_hints(&rig.log[mark..], p0), vec![HINT_LINES[4]]);
    quiet(
        &mut rig,
        frame(Power::Online, true, vec![ah1_classified(other)]),
        "once",
    );

    // A reclaim's first frame: the held frame gone, the doctrine's power up.
    let match_id = rig.centre().match_id.expect("a running match");
    rig.rebind(p0, match_id, held.unit);
    quiet(
        &mut rig,
        frame(Power::Online, true, vec![]),
        "a first frame with power up is no panel point",
    );
    let said = rig.say(0, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 1 of 7."),
        "{said}"
    );
    rig.rebind(p0, match_id, held.unit);
    let mark = rig.log.len();
    rig.push_frame(0, frame(Power::Off, true, vec![]));
    assert_eq!(ah1_hints(&rig.log[mark..], p0), vec![HINT_LINES[2]]);

    // The seat gate: a bound seat that holds no slot of the centre's.
    if !rig.tw.world().has_live_conns(p2) {
        rig.tw.world_mut().set_live_conns(p2, 1);
    }
    assert!(rig.tw.world().has_live_conns(p2));
    assert!(
        rig.centre().slot_of_pilot(p2).is_none(),
        "pilot 2 claimed nothing"
    );
    rig.tw
        .world_mut()
        .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
        .expect("the arena registry is composed")
        .bind_pilot(p2, match_id, held.unit);
    let mark = rig.log.len();
    rig.push_frame_for(p2, frame(Power::Online, true, vec![ah1_classified(other)]));
    rig.rebind(p2, match_id, held.unit);
    rig.push_frame_for(p2, frame(Power::Off, true, vec![]));
    assert!(
        ah1_stream(&rig.log[mark..], p2).contains(&None),
        "the seat was sent its frames"
    );
    assert_eq!(
        ah1_hints(&rig.log[mark..], p2),
        Vec::<String>::new(),
        "no slot, no hint"
    );
    let events = rig.events_of(p2, "@tutorial");
    assert_eq!(
        rig.tw.seen_by(&events, p2),
        vec!["Hints are on; you have read 0 of 7.".to_owned()]
    );
    audit_hints(&rig.log);
}

/// **16b cut AH.1**: the committed cockpit scene earns exactly what its own
/// fields earn — `cockpit_frame.json` is power `online`, a red unit with two
/// weapons, and two notices (`acquired` of an unknown `X4`, and
/// `target_section_lost` on `BA`), none a classification and none
/// `power_online` — so no point; and the same scene as a first frame with its
/// power off and no notices earns the panel point alone.
#[test]
fn ah1_the_fixture_frame_yields_the_points_its_own_fields_earn() {
    use tacsim::arena::state::Power;

    let scene = fixture::frame();
    assert_eq!(scene.power, Power::Online);
    assert_eq!(scene.team, PartyId::RED);
    assert_eq!(scene.weapons.len(), 2);
    assert_eq!(scene.notices.len(), 2);
    assert_eq!(frame_points(false, &scene), Vec::<HintPoint>::new());
    assert_eq!(
        frame_points(true, &scene),
        Vec::<HintPoint>::new(),
        "power is up"
    );
    let mut cold = fixture::frame();
    cold.power = Power::Off;
    cold.notices = vec![];
    assert_eq!(frame_points(true, &cold), vec![HintPoint::Panel]);
    assert_eq!(frame_points(false, &cold), Vec::<HintPoint>::new());
}

/// `SIM ABORT` from pilot `n`, then pump until the check-in has landed.
async fn m5_abort_and_check_in(rig: &mut Rig, n: usize) {
    rig.say(n, "sim abort");
    for _ in 0..400 {
        if rig.state().is_terminal() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert!(rig.state().is_terminal(), "the check-in lands");
}

/// **17b cut M5.6 (the design note's item 2)**: the sixth hint,
/// `HintPoint::Mind`, is said by `match_::finish` — once per ACCOUNT, to each
/// pilot of the match with a live connection, as the check-in's last line to
/// them — and to nobody else: a linkdead pilot is neither told nor marked and reads
/// it at the end of their next match; a pilot who read it reads it never
/// again; a bystander and staff, who flew nothing, are never told.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_mind_hint_is_said_at_a_matchs_end_once_per_account_to_a_live_pilot() {
    let mut rig = Rig::boot(3);
    let (p0, p1, p2) = (rig.pilot(0), rig.pilot(1), rig.pilot(2));
    rig.fly().await;
    rig.tw.world_mut().set_live_conns(p1, 0);
    let mark = rig.log.len();
    m5_abort_and_check_in(&mut rig, 0).await;
    let stream = ah1_stream(&rig.log[mark..], p0);
    let hatch = stream
        .iter()
        .position(|said| {
            said.as_deref()
                .is_some_and(|line| line.contains("swings open"))
        })
        .unwrap_or_else(|| panic!("the pod was opened: {stream:?}"));
    let hint = ah1_at(&stream, HINT_LINES[5]);
    assert!(hatch < hint, "after the lobby's release: {stream:?}");
    assert_eq!(hint, stream.len() - 1, "the match's last line: {stream:?}");
    assert_eq!(
        ah1_hints(&rig.log[mark..], p0)
            .into_iter()
            .filter(|line| line == HINT_LINES[5])
            .count(),
        1,
        "once"
    );
    for (who, name) in [(p1, "linkdead"), (p2, "a bystander"), (GOD, "staff")] {
        assert!(
            !ah1_hints(&rig.log[mark..], who).contains(&HINT_LINES[5].to_owned()),
            "{name}"
        );
    }

    // The next match: pilot 1 is live again and reads the point that waited;
    // pilot 0 has read it.
    rig.staff("@arena/reset");
    rig.tw.world_mut().set_live_conns(p1, 1);
    rig.fly().await;
    let mark = rig.log.len();
    m5_abort_and_check_in(&mut rig, 1).await;
    assert!(
        ah1_hints(&rig.log[mark..], p1).contains(&HINT_LINES[5].to_owned()),
        "the point waited for a live connection"
    );
    assert!(
        !ah1_hints(&rig.log[mark..], p0).contains(&HINT_LINES[5].to_owned()),
        "once per account"
    );
    audit_hints(&rig.log);
}

// ---------------------------------------------------------------------------
// 16b cut AP.1: the aged mirror (16c `B-86`)
// ---------------------------------------------------------------------------

/// **A frame the arena could have sent**: pilot 0's own held frame with power
/// online, the golden scene's contacts and its lock on one of them, and the cannon
/// two seconds into nothing — `ready: false, ready_in_ms: 2_000`, a round in the
/// breech. Everything `FIRE` pre-checks passes but the cycle.
fn ap1_locked_and_cycling(rig: &Rig) -> tacsim::arena::step::Frame {
    let scene = fixture::frame();
    let mut frame = rig.held_frame(0);
    frame.notices.clear();
    frame.catchup = None;
    frame.power = tacsim::arena::state::Power::Online;
    frame.contacts = scene.contacts.clone();
    frame.lock = scene.lock;
    assert!(
        frame
            .contacts
            .iter()
            .any(|contact| Some(contact.id) == frame.lock),
        "the lock is one of the frame's own contacts"
    );
    let mut cannons = 0;
    for weapon in &mut frame.weapons {
        if weapon.id == "cannon" {
            weapon.ready = false;
            weapon.ready_in_ms = 2_000;
            weapon.rounds = weapon.rounds.max(1);
            cannons += 1;
        }
    }
    assert_eq!(cannons, 1, "the stock tank carries one cannon");
    frame
}

/// **`B-86`, the forward**: a `FIRE` typed after the gun's true ready instant and
/// before the ready frame lands is forwarded, not refused in the cockpit.
///
/// The ready instant is reached by moving the held frame's receipt back
/// (`ArenaRegistry::age_last_frame`), not by sleeping. The arena's own answer a
/// pulse later (it holds no lock for this pilot) is not this test's business.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ap1_a_fire_typed_after_the_true_ready_instant_and_before_the_ready_frame_is_forwarded() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = ap1_locked_and_cycling(&rig);
    rig.push_frame(0, frame);

    let said = rig.say(0, "fire");
    assert!(
        said.contains("FIRE cannon refused. Still cycling: 2 s to go."),
        "a fresh frame's countdown is refused as it stands: {said}"
    );
    rig.age_last_frame(0, 2_000);
    let said = rig.say(0, "fire");
    assert!(
        said.contains("Firing cannon."),
        "a gun ready by the wall clock was refused by the cockpit: {said}"
    );
    assert!(!said.contains("Still cycling"), "{said}");
}

/// **`B-86`, the read verb and the panel**: `WEAPONSTATUS` prints the aged
/// remainder and counts down to `ready`, while the keyframe it re-pushes keeps the
/// frame's own figures — which the TUI and the web print as they stand until the
/// next frame; neither counts down.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ap1_weaponstatus_prints_the_aged_remainder_and_the_keyframe_does_not() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = ap1_locked_and_cycling(&rig);
    rig.push_frame(0, frame);

    let said = rig.say(0, "weaponstatus");
    assert!(said.contains("cannon  cycling (2 s)"), "{said}");
    rig.age_last_frame(0, 1_000);
    let said = rig.say(0, "weaponstatus");
    assert!(said.contains("cannon  cycling (1 s)"), "{said}");
    rig.age_last_frame(0, 1_000);
    let events = rig.events(0, "weaponstatus");
    let said = rig.tw.seen_by(&events, rig.pilot(0)).join("\n");
    assert!(said.contains("cannon  ready"), "{said}");
    assert!(!said.contains("cannon  cycling"), "{said}");

    // The keyframe beside the lines: the frame's own countdown, unaged.
    let panel = Rig::panel(&events).expect("WEAPONSTATUS re-pushes the keyframe");
    let cannon = panel["weapons"]
        .as_array()
        .and_then(|weapons| weapons.iter().find(|weapon| weapon["id"] == "cannon"))
        .expect("the keyframe carries the cannon");
    assert_eq!(cannon["ready_in_ms"], 2000, "{cannon}");
    assert_eq!(cannon["ready"], false, "{cannon}");
}

/// **`B-86`, the floor rule both ways**: under a staff tempo floor the countdown
/// stays the frame's own; after the lift a frame built under the floor STILL
/// does, because its `arena_ms` is at or before the lift's stamp; and the first
/// frame built past the lift ages again.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ap1_a_staff_tempo_floor_and_a_frame_built_under_one_leave_the_remainder_unaged() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let match_id = rig.centre().match_id.expect("a running match");
    let frame = ap1_locked_and_cycling(&rig);
    rig.push_frame(0, frame.clone());
    rig.age_last_frame(0, 2_000);

    // A floor stands: the wall clock runs ahead of the arena's, so no aging.
    rig.tw
        .world_mut()
        .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
        .expect("the arena registry is composed")
        .note_tempo_floor(match_id, Some(5_000));
    let said = rig.say(0, "fire");
    assert!(
        said.contains("FIRE cannon refused. Still cycling: 2 s to go."),
        "a floored countdown was aged: {said}"
    );

    // Lifted: the stamp is the mirror's clock, and the held frame predates it.
    let pilot = rig.pilot(0);
    let lifted_at = {
        let registry = rig
            .tw
            .world_mut()
            .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the arena registry is composed");
        registry.note_tempo_floor(match_id, None);
        let mirror = registry.mirror(match_id).expect("the match is mirrored");
        assert_eq!(mirror.tempo_floor_ms, None);
        let at = mirror.floor_lifted_at_ms.expect("a lift is stamped");
        assert_eq!(at, mirror.now_ms, "the lift stamps the mirror's own clock");
        let held = registry.last_frame(pilot).expect("the pilot holds a frame");
        assert!(
            held.arena_ms <= at,
            "the held frame was built under the floor"
        );
        at
    };
    let said = rig.say(0, "fire");
    assert!(
        said.contains("FIRE cannon refused. Still cycling: 2 s to go."),
        "a frame built under a floor since lifted was aged: {said}"
    );

    // The first frame past the lift ages again.
    rig.push_frame_at(0, frame, lifted_at + 1_000);
    rig.age_last_frame(0, 2_000);
    let said = rig.say(0, "fire");
    assert!(
        said.contains("Firing cannon."),
        "a frame built past the lift was left unaged: {said}"
    );

    // And a new floor clears the stamp.
    let registry = rig
        .tw
        .world_mut()
        .module_state_mut::<tacsim::arena_bridge::ArenaRegistry>()
        .expect("the arena registry is composed");
    registry.note_tempo_floor(match_id, Some(5_000));
    assert_eq!(
        registry
            .mirror(match_id)
            .and_then(|mirror| mirror.floor_lifted_at_ms),
        None
    );
}

// ---------------------------------------------------------------------------
// 16b cut AP.3: `WATCH`, a pushed digest for line mode (16c `N19`)
// ---------------------------------------------------------------------------

/// **The scene's digest, pinned** (16b cut AP.3): `watch_lines` over the one
/// fixture scene with its own weapon rows, the ranges its keyframe carries
/// (settled against no memory, as `status_frame` reads them) and the shipped
/// one-second sensor pass.
///
/// `RA` at 1,200 cm/s (43 kph) heading 045°; the cannon loaded and the machine
/// gun 900 ms from ready (`1 s`, rounded up); the lock on `BA`, whose 430 m
/// plot rounds to 450 and whose bare-`FIRE` chance is the cannon's 624 per
/// mille. The contacts line does not fit: `BA` (stale at 1,200 ms) and `RB`
/// (fresh at 400 ms, so no age) take 74 columns, and `X3`'s entry would carry
/// it to 124, so the line ends `; +2 more` for `X3` and `X4`.
#[test]
fn ap3_watch_lines_over_the_fixture_scene() {
    let frame = fixture::frame();
    let ranges = cockpit::settle_ranges(&frame, &BTreeMap::new());
    let lines = cockpit::watch_lines(&frame, &ranges, &cockpit::frame_weapons(&frame), 1_000);
    assert_eq!(
        lines,
        vec![
            "Watch RA: online, 43 kph, heading 045°".to_owned(),
            "Watch guns: cannon ready, machine_gun (mg) cycling (1 s); lock BA, 450 m, 62% to hit"
                .to_owned(),
            "Watch contacts: BA blue 045° far 450 m, plotted 1 s ago; RB red 270° close 100 m; \
             +2 more"
                .to_owned(),
        ]
    );
    for line in &lines {
        assert!(line.chars().count() <= cockpit::WATCH_COLUMNS, "{line}");
    }
}

/// Every digest block the rig has pushed `pilot` since `from`: a `Sys` to that
/// actor whose first span wears the notice tag and whose text opens `Watch `.
fn ap3_blocks(rig: &Rig, pilot: ObjId, from: usize) -> Vec<String> {
    rig.log
        .iter()
        .skip(from)
        .filter_map(|(audience, event)| match event {
            OutputEvent::Sys(text) if *audience == Audience::Actor(pilot) => {
                let tagged = text.spans().first().and_then(|span| span.tag.as_deref())
                    == Some(tacsim::arena_bridge::NOTICE_SENTENCE_TAG);
                let plain = text.to_plain_text();
                (tagged && plain.starts_with("Watch ")).then_some(plain)
            }
            _ => None,
        })
        .collect()
}

/// What `pilot` was sent since `from`, in order: `K` for a cockpit keyframe,
/// `W` for a digest block, nothing for anything else.
fn ap3_order(rig: &Rig, pilot: ObjId, from: usize) -> String {
    rig.log
        .iter()
        .skip(from)
        .filter(|(audience, _)| *audience == Audience::Actor(pilot))
        .filter_map(|(_, event)| match event {
            OutputEvent::Panel { feed, .. }
                if feed.as_str() == tacsim::arena_bridge::FRAME_FEED =>
            {
                Some('K')
            }
            OutputEvent::Sys(text) if text.to_plain_text().starts_with("Watch ") => Some('W'),
            _ => None,
        })
        .collect()
}

/// The frame the registry holds for `pilot`, whole.
fn ap3_held(rig: &Rig, pilot: ObjId) -> tacsim::arena_bridge::CockpitFrame {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.last_frame(pilot).cloned())
        .expect("the pilot holds a frame")
}

/// **`N19`, the switch and the push** (16b cut AP.3): bare `WATCH` reads off;
/// `WATCH ON` answers and says the held frame's block at once, on AP.1's aged
/// rows; a frame that changes what the block prints pushes exactly one block,
/// after its keyframe; a frame that moves only the cell or the tempo pushes
/// none; a plot crossing the sensor pass pushes one; `WATCH OFF` stops it; a
/// word that is neither is the usage line; the pilot who never watched is
/// pushed nothing; and the switch goes with the seat, so a reclaimed seat
/// starts off.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ap3_watch_on_pushes_one_block_per_changed_block_and_off_stops_it() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let pilot = rig.pilot(0);
    let pass_ms = i64::from(config().sensors.pass_ms);
    let frame = ap1_locked_and_cycling(&rig);
    rig.push_frame(0, frame.clone());
    rig.age_last_frame(0, 1_000);

    // Off until typed.
    let said = rig.say(0, "watch");
    assert_eq!(said, cockpit::WATCH_IS_OFF);

    // On: the reply, then the held frame's block at once, on the aged rows —
    // while the keyframe it was read from still carries its own countdown.
    let from = rig.log.len();
    let said = rig.say(0, "WATCH ON");
    assert!(said.starts_with(cockpit::WATCHING), "{said}");
    let blocks = ap3_blocks(&rig, pilot, from);
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    let held = ap3_held(&rig, pilot);
    let aged = cockpit::aged_weapons(&held, std::time::Instant::now(), false);
    let expected = cockpit::watch_lines(&held.frame, &held.ranges, &aged, pass_ms).join("\n");
    assert_eq!(blocks.first(), Some(&expected));
    assert!(expected.contains("cannon cycling (1 s)"), "{expected}");
    let cannon = held
        .frame
        .weapons
        .iter()
        .find(|weapon| weapon.id == "cannon")
        .map(|weapon| weapon.ready_in_ms);
    assert_eq!(
        cannon,
        Some(2_000),
        "the keyframe's own countdown is unaged"
    );
    assert!(
        (1..=3).contains(&expected.lines().count()),
        "one to three lines: {expected}"
    );

    // A frame that moved the speed: its keyframe, then exactly one block —
    // the frame as sent, with its own rows.
    let mut faster = frame.clone();
    faster.speed_cms = frame.speed_cms + 500;
    let from = rig.log.len();
    rig.push_frame(0, faster.clone());
    assert_eq!(ap3_order(&rig, pilot, from), "KW");
    let held = ap3_held(&rig, pilot);
    assert_eq!(
        ap3_blocks(&rig, pilot, from),
        vec![
            cockpit::watch_lines(
                &held.frame,
                &held.ranges,
                &cockpit::frame_weapons(&held.frame),
                pass_ms
            )
            .join("\n")
        ],
        "the bridge's block is the frame as sent"
    );

    // The same frame one cell on, and then at another tempo: keyframes only.
    let mut moved = faster.clone();
    moved.cell = moved.cell.map(|(x, y)| (x + 1, y));
    moved.pose.x_cm += 1_000;
    let from = rig.log.len();
    rig.push_frame(0, moved.clone());
    assert_eq!(
        ap3_order(&rig, pilot, from),
        "K",
        "a moved cell pushed a block"
    );
    let mut retimed = moved.clone();
    retimed.tempo = match moved.tempo {
        tacsim::arena::clock::Tempo::Quiet => tacsim::arena::clock::Tempo::Engaged,
        _ => tacsim::arena::clock::Tempo::Quiet,
    };
    let from = rig.log.len();
    rig.push_frame(0, retimed.clone());
    assert_eq!(
        ap3_order(&rig, pilot, from),
        "K",
        "a tempo flip pushed a block"
    );

    // A held contact's plot crossing the pass: one block, saying so.
    let mut stale = retimed.clone();
    let fresh = stale
        .contacts
        .iter_mut()
        .find(|contact| contact.age_ms <= pass_ms)
        .expect("the scene holds a fresh plot");
    fresh.age_ms = pass_ms + 500;
    let entry_head = format!("{} ", fresh.designator);
    let from = rig.log.len();
    rig.push_frame(0, stale.clone());
    assert_eq!(ap3_order(&rig, pilot, from), "KW");
    let blocks = ap3_blocks(&rig, pilot, from);
    let entry = blocks
        .first()
        .and_then(|block| {
            block
                .lines()
                .find(|line| line.starts_with("Watch contacts: "))
        })
        .and_then(|line| line.strip_prefix("Watch contacts: "))
        .and_then(|list| {
            list.split("; ")
                .find(|entry| entry.starts_with(&entry_head))
        })
        .map(str::to_owned);
    assert!(
        entry
            .as_deref()
            .is_some_and(|entry| entry.ends_with(" m, plotted 1 s ago")),
        "{blocks:?}"
    );

    // Off: the reply, and a changed frame pushes nothing.
    let said = rig.say(0, "watch off");
    assert_eq!(said, cockpit::WATCH_OFF);
    let mut slower = stale.clone();
    slower.speed_cms = 0;
    let from = rig.log.len();
    rig.push_frame(0, slower);
    assert_eq!(ap3_order(&rig, pilot, from), "K", "a block after WATCH OFF");
    assert_eq!(rig.say(0, "watch"), cockpit::WATCH_IS_OFF);

    // A word that is neither.
    let said = rig.say(0, "watch sideways");
    assert_eq!(said, cockpit::WATCH_USAGE);

    // The pilot who never typed it was pushed nothing, ever.
    let other = rig.pilot(1);
    assert!(
        !rig.log.iter().any(|(audience, event)| {
            *audience == Audience::Actor(other)
                && matches!(event, OutputEvent::Sys(text)
                    if text.to_plain_text().starts_with("Watch "))
        }),
        "a pilot who never watched was pushed a block"
    );

    // The switch is the seat's: on, then the drop (`Released` unbinds the
    // pilot) and the reclaim, and the reclaimed seat starts off.
    assert!(rig.say(0, "watch on").starts_with(cockpit::WATCHING));
    assert_eq!(rig.say(0, "watch"), cockpit::WATCH_IS_ON);
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    assert!(
        !rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .is_some_and(|registry| registry.line_watching(pilot)),
        "the drop left the switch on"
    );
    rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    let now = rig.now;
    rig.tick_to(now + 5);
    let said = rig.say(0, "board");
    assert!(said.contains("You climb back into"), "{said}");
    assert_eq!(rig.say(0, "watch"), cockpit::WATCH_IS_OFF);
}

/// **`N19`, the block `WATCH ON` says is the one compared against** (16b cut
/// AP.3): with every gun ready the aged rows are the frame's own, so the block
/// the verb says at once is the block the bridge would build from the next
/// frame that moves only the cell — and that frame pushes its keyframe and no
/// block. Were the verb not to record what it said, the pilot would read the
/// same lines twice.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn ap3_the_block_watch_on_says_is_recorded_and_not_said_again() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let pilot = rig.pilot(0);
    let pass_ms = i64::from(config().sensors.pass_ms);
    let mut frame = ap1_locked_and_cycling(&rig);
    for weapon in &mut frame.weapons {
        weapon.ready = true;
        weapon.ready_in_ms = 0;
        weapon.rounds = weapon.rounds.max(1);
    }
    rig.push_frame(0, frame.clone());
    rig.age_last_frame(0, 1_000);

    let from = rig.log.len();
    let said = rig.say(0, "watch on");
    assert!(said.starts_with(cockpit::WATCHING), "{said}");
    let blocks = ap3_blocks(&rig, pilot, from);
    assert_eq!(blocks.len(), 1, "{blocks:?}");
    let held = ap3_held(&rig, pilot);
    let as_sent = cockpit::watch_lines(
        &held.frame,
        &held.ranges,
        &cockpit::frame_weapons(&held.frame),
        pass_ms,
    )
    .join("\n");
    assert_eq!(
        blocks.first(),
        Some(&as_sent),
        "with every gun ready the aged block is the frame's own"
    );

    // The same frame one cell on: the keyframe, and no block.
    let mut moved = frame.clone();
    moved.cell = moved.cell.map(|(x, y)| (x + 1, y));
    moved.pose.x_cm += 1_000;
    let from = rig.log.len();
    rig.push_frame(0, moved);
    assert_eq!(
        ap3_order(&rig, pilot, from),
        "K",
        "the block WATCH ON said was said again"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M1a.3 — the frame grows (doc 17 §3, D-UM-4, D-UM-5)
// ---------------------------------------------------------------------------

/// **17b cut M1a.3**: the fixture scene's keyframe carries what a mind reads —
/// version 2; per contact the facet word, `reads_dead`, the odds (default
/// weapon first, `[]` for an unclassified row) and the fatal words beside a
/// reading; per weapon its band edges; at the top `t_ms` and `order: null` —
/// and the scene's `CONTACTS` rows end in the side word and the main gun's
/// odds or a dash. The JSON round-trips: `cockpit_frame.json` as committed is
/// what the builder writes.
#[test]
fn m1a3_the_keyframe_carries_aspect_odds_reads_dead_order_and_t_ms() {
    let frame = fixture::frame();
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    let committed: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
    ))
    .expect("the fixture is JSON");
    assert_eq!(payload, committed, "the fixture round-trips with the keys");
    // DT.1b adds opaque standing-order identity at version 8.
    assert_eq!(cockpit::FRAME_VERSION, 8);
    assert_eq!(
        payload.pointer("/schema/version"),
        Some(&serde_json::json!(8))
    );
    assert_eq!(payload.get("t_ms"), Some(&serde_json::json!(61_250)));
    assert_eq!(payload.get("order"), Some(&serde_json::Value::Null));
    let contacts = payload
        .get("contacts")
        .and_then(serde_json::Value::as_array)
        .expect("contacts");
    let by = |designator: &str| {
        contacts
            .iter()
            .find(|contact| contact.get("designator") == Some(&serde_json::json!(designator)))
            .cloned()
            .unwrap_or_else(|| panic!("{designator}"))
    };
    let ba = by("BA");
    assert_eq!(ba.get("aspect"), Some(&serde_json::json!("left")));
    assert_eq!(ba.get("reads_dead"), Some(&serde_json::json!(false)));
    assert_eq!(ba.get("fatal"), Some(&serde_json::json!(["hull"])));
    assert_eq!(
        ba.get("odds"),
        Some(&serde_json::json!([
            { "weapon": "cannon", "p_permille": 624, "percent": 62 },
            { "weapon": "machine_gun", "p_permille": 210, "percent": 21 },
        ]))
    );
    let x3 = by("X3");
    assert_eq!(
        x3.get("odds"),
        Some(&serde_json::json!([])),
        "never unclassified"
    );
    assert_eq!(x3.get("fatal"), Some(&serde_json::Value::Null));
    assert_eq!(x3.get("aspect"), Some(&serde_json::json!("front")));
    assert_eq!(
        payload.pointer("/weapons/0/bands_cm"),
        Some(&serde_json::json!([100_000, 300_000]))
    );
    // The scene's `CONTACTS` rows, verbatim.
    let rows: Vec<String> = cockpit::frame_contacts(&frame)
        .iter()
        .map(cockpit::ContactRow::render)
        .collect();
    assert_eq!(
        rows,
        [
            "BA  blue  bearing 045° (rel 000°)  far  450 m  1 s  side  cannon 62%",
            "RB  red  bearing 270° (rel 225°)  close  100 m  0 s  front  -",
            "X3  unknown  bearing 315° (rel 270°)  effective  200 m  3 s  front  -",
            "X4  unknown  bearing 030° (rel 345°)  far  600 m  6 s  rear  -",
        ]
    );
}

/// **17b cut M1a.3** (doc 17 §22's italic line): a hull with no weapons
/// station — no gun aboard, or its turret shot off — carries no `weapons` key
/// and no contact carries `odds` or `reads_dead`; the lock, the heat, the scan
/// tables and the fatal words stay; and its `CONTACTS` rows end at the aspect.
#[test]
fn m1a3_an_unarmed_hull_drops_the_weapons_block_and_its_rows_end_at_the_aspect() {
    let mut frame = fixture::frame();
    frame.weapons.clear();
    frame.odds.clear();
    for row in &mut frame.contacts {
        row.reads_dead = None;
    }
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    let object = payload.as_object().expect("an object");
    assert!(!object.contains_key("weapons"), "{payload}");
    for key in [
        "lock",
        "lock_designator",
        "lock_range_m",
        "lock_p_permille",
        "lock_percent",
        "heat",
        "scan_tables",
        "t_ms",
        "order",
    ] {
        assert!(object.contains_key(key), "{key} stays");
    }
    for contact in payload
        .get("contacts")
        .and_then(serde_json::Value::as_array)
        .expect("contacts")
    {
        let contact = contact.as_object().expect("a contact object");
        assert!(!contact.contains_key("odds"), "{contact:?}");
        assert!(!contact.contains_key("reads_dead"), "{contact:?}");
        assert!(contact.contains_key("aspect"), "{contact:?}");
        assert!(contact.contains_key("fatal"), "{contact:?}");
    }
    let rows: Vec<String> = cockpit::frame_contacts(&frame)
        .iter()
        .map(cockpit::ContactRow::render)
        .collect();
    assert_eq!(
        rows.first().map(String::as_str),
        Some("BA  blue  bearing 045° (rel 000°)  beyond  450 m  1 s  side"),
        "no gun: no reach, so `beyond`, and nothing after the aspect"
    );
    assert!(
        rows.iter().all(|row| row.ends_with("  side")
            || row.ends_with("  front")
            || row.ends_with("  rear")),
        "{rows:?}"
    );
}

/// **17b cut M1a.3**: a standing order as the keyframe spells it — the
/// objective's word, `target` and `of` as the designators of the hull's own
/// rows (`null` when the order names a row the frame does not carry), the
/// payloads, the constraints, and the time left measured from the frame's own
/// `t_ms`, never below nought.
#[test]
fn m1a3_the_keyframe_spells_an_order_with_designators_and_the_time_left() {
    use smol_str::SmolStr;
    use tacsim::arena::perception::{FrameOrder, MindObjective};
    use tacsim::arena::state::ContactId;
    use tacsim::deliberation::order::RangeBand;
    let mut frame = fixture::frame();
    let hostile = frame.contacts.first().map(|row| row.id).expect("BA's row");
    frame.order = Some(FrameOrder {
        identity: None,
        squad: SmolStr::new_static("red-1"),
        objective: MindObjective::Engage {
            target: Some(hostile),
        },
        hold_fire: false,
        max_range_band: Some(RangeBand::Effective),
        expires_ms: 115_500,
    });
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    assert_eq!(
        payload.get("order"),
        Some(&serde_json::json!({
            "squad": "red-1",
            "identity": null,
            "objective": "engage",
            "target": "BA",
            "to": null,
            "facing_mdeg": null,
            "of": null,
            "hold_fire": false,
            "max_range_band": "effective",
            "expires_in_ms": 54_250,
        }))
    );
    let order = |objective: MindObjective, expires_ms: i64| FrameOrder {
        identity: None,
        squad: SmolStr::new_static("red-1"),
        objective,
        hold_fire: true,
        max_range_band: None,
        expires_ms,
    };
    frame.order = Some(order(MindObjective::Advance { to: (3, 4) }, 0));
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    assert_eq!(
        payload.pointer("/order/to"),
        Some(&serde_json::json!([3, 4]))
    );
    assert_eq!(
        payload.pointer("/order/expires_in_ms"),
        Some(&serde_json::json!(0)),
        "an expired deadline reads nought, never negative"
    );
    frame.order = Some(order(
        MindObjective::Support {
            of: Some(ContactId(0xdead)),
        },
        70_000,
    ));
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    assert_eq!(
        payload.pointer("/order/objective"),
        Some(&serde_json::json!("support"))
    );
    assert_eq!(
        payload.pointer("/order/of"),
        Some(&serde_json::Value::Null),
        "a row the frame does not carry is named by nothing"
    );
    frame.order = Some(order(
        MindObjective::Screen {
            facing_mdeg: 90_000,
        },
        70_000,
    ));
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    assert_eq!(
        payload.pointer("/order/facing_mdeg"),
        Some(&serde_json::json!(90_000))
    );
}

// ---------------------------------------------------------------------------
// 17b cut M1a.4 — `DRIVE`, the sixteenth selector (doc 17 §4 D-UM-10)
// ---------------------------------------------------------------------------

/// **The verb word collides with no registered command** (17b cut M1a.4; the
/// check this cut adds for every new verb word). Doc 17 named it `GOTO`, and
/// core's baseline `goto` — with its aliases `go` and `move` — holds that word:
/// `Registry::insert_entry` refuses a collision, so a `goto` selector would
/// have failed the tacsim module's registration and every composed server at
/// boot. On the registry this rig composes, the three still answer core's own
/// command, `drive` answers the cockpit's (a module's verb, so no kernel word),
/// and no cockpit selector is a kernel or host word at all.
///
/// **The prefix shift, pinned as a deliberate fact**: the selectors register
/// by prefix, and core's `drop` does too, so `dr` — core `drop`'s shortest
/// prefix before this cut — is now ambiguous between `drive` and `drop`, and
/// answers neither; `dro` is still `drop`, and `dri` is `DRIVE`'s. Declared in
/// the `drive` stanza of `legacy-handlers.toml` (16b cut AP.4 made `o` an
/// exact-only name for the same kind of shift; whether `dr` deserves one is the
/// lead's call).
#[test]
fn m1a4_drive_collides_with_no_registered_command() {
    let mut rig = Rig::boot(1);
    let registry = &rig.tw.engine_mut().registry;
    for word in ["goto", "go", "move"] {
        assert_eq!(
            registry.kernel_command_named(word),
            Some("goto"),
            "{word} is core's command"
        );
    }
    assert_eq!(registry.kernel_command_named("drive"), None);
    let drive = registry
        .spec_of("drive")
        .expect("the cockpit registers drive");
    assert_eq!(drive.name.as_str(), "drive");
    assert!(drive.help.starts_with("DRIVE <spot|x,y>"), "{}", drive.help);
    assert_eq!(
        registry.compatibility_of("drive"),
        Some(mush_core::CommandCompatibility::Authoritative)
    );
    for verb in cockpit::COCKPIT_VERBS {
        assert_eq!(
            registry.kernel_command_named(verb),
            None,
            "the cockpit's {verb} shadows a kernel or host word"
        );
        assert!(registry.spec_of(verb).is_some(), "{verb} is registered");
    }
    // The prefix shift `drive` makes.
    let named = |word: &str| {
        registry
            .lookup(word)
            .map(|(spec, _)| spec.name.to_string())
            .map_err(|err| match err {
                mush_core::LookupErr::NotFound => Vec::new(),
                mush_core::LookupErr::Ambiguous(names) => {
                    let mut names: Vec<String> = names.iter().map(ToString::to_string).collect();
                    names.sort();
                    names
                }
            })
    };
    assert_eq!(
        named("dr"),
        Err(vec!["drive".to_owned(), "drop".to_owned()]),
        "dr is ambiguous between DRIVE and core's drop"
    );
    assert_eq!(named("dro"), Ok("drop".to_owned()));
    assert_eq!(named("dri"), Ok("drive".to_owned()));
    assert_eq!(named("drive"), Ok("drive".to_owned()));
}

// ---------------------------------------------------------------------------
// 17b cut M5.4 — `MIND`, the seventeenth selector (doc 17 §12 "The `MIND` verb")
// ---------------------------------------------------------------------------

/// **The verb word collides with no registered command, and the prefix shift
/// is pinned** (17b cut M5.4; the M5 design note's M5.4 item 7; the check
/// 17b cut M1a.4 added for every new verb word). `mind` is no kernel or host
/// word; the cockpit registers it, `authoritative` with the rest of the
/// cockpit, with its help line. Core's lookup tries exact names and aliases
/// first and then prefixes of bare primary names only — `@` and `+` names
/// and aliases never prefix-match — and this production roster has no other
/// bare primary verb starting with `m` (its m-names are `+map`, `+market`,
/// `+move`, `+muster`, `@macro`, `@mail`, `@market` and `@mind`; `move` is
/// only `goto`'s alias). So `m`, `mi` and `min`, which found no command
/// before this cut, are `MIND`'s, and no verb loses a prefix: the staff
/// verb `@mind` and the `move` alias still answer as they did. Declared in
/// the `mind` stanza of `legacy-handlers.toml`.
#[test]
fn m5_mind_collides_with_no_registered_command() {
    let mut rig = Rig::boot(1);
    let registry = &rig.tw.engine_mut().registry;
    assert_eq!(registry.kernel_command_named("mind"), None);
    let mind = registry
        .spec_of("mind")
        .expect("the cockpit registers mind");
    assert_eq!(mind.name.as_str(), "mind");
    assert!(
        mind.help.starts_with("MIND [PROFILE <name>|RULES]"),
        "{}",
        mind.help
    );
    assert_eq!(
        registry.compatibility_of("mind"),
        Some(mush_core::CommandCompatibility::Authoritative)
    );
    let named = |word: &str| {
        registry
            .lookup(word)
            .map(|(spec, _)| spec.name.to_string())
            .map_err(|err| match err {
                mush_core::LookupErr::NotFound => Vec::new(),
                mush_core::LookupErr::Ambiguous(names) => {
                    let mut names: Vec<String> = names.iter().map(ToString::to_string).collect();
                    names.sort();
                    names
                }
            })
    };
    for word in ["m", "mi", "min", "mind", "MIND", "Mi"] {
        assert_eq!(named(word), Ok("mind".to_owned()), "{word}");
    }
    // Nothing else lost a word: the staff verb and the alias are where they were.
    assert_eq!(named("@mind"), Ok("@mind".to_owned()));
    assert_eq!(named("move"), Ok("goto".to_owned()));
    assert_eq!(
        named("minds"),
        Err(Vec::new()),
        "a prefix, never a longer word"
    );
}

/// Every line `pilot` was sent from `mark` on, as the client reads them.
fn m1a4_said_since(rig: &Rig, pilot: ObjId, mark: usize) -> Vec<String> {
    rig.tw.seen_by(&rig.log[mark..], pilot)
}

/// **`DRIVE` resolves a spot by name and by letter, a cell, `HOLD` and `OFF`,
/// and refuses the rest in one sentence** (17b cut M1a.4; doc 17 §4 D-UM-10;
/// doc 16 §3.7), over the real composition, the shipped proving ground and the
/// real arena task.
///
/// Cold, it is refused in `HEADING`'s own words. Powered: every reply
/// verbatim, the usage line for a bare `DRIVE`, the off-window line, and the
/// ONE byte-identical refusal for `BA`, `X2`, `nonsense`, `12` and `12,` — a
/// contact's designator is not a spot, so nothing about any contact is read.
/// Then the forwarded order is proved by its effect: a `DRIVE` to a cell four
/// cells along the road is a `Goal { Cell }` on the helm channel at the seat's
/// epoch, or the arena would have refused it (a wrong channel or epoch is
/// refused silently, a wrong kind would not drive) — and the arena's motor
/// drives the tank there and says `DRIVE: arrived.`; a `HEADING` then takes the
/// helm back. Last, frames pushed through the real egress seam: a wreck, an
/// immobile tank and a cold one are refused locally in `HEADING`'s words.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m1a4_drive_resolves_a_spot_a_letter_a_cell_hold_and_off_and_refuses_the_rest() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let pilot = rig.pilot(0);

    // Cold: refused as `HEADING` is, before the word is read.
    let cold = rig.say(0, "drive a");
    assert!(cold.contains(cockpit::POWER_NOT_ONLINE), "{cold}");
    assert_eq!(cold, rig.say(0, "heading 90"), "HEADING's own words");
    assert!(
        rig.say(0, "drive nonsense")
            .contains(cockpit::POWER_NOT_ONLINE)
    );

    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the power came up");

    // The usage line, and the replies.
    assert_eq!(rig.say(0, "drive"), cockpit::DRIVE_USAGE);
    let (spots, _) = shipped_spots(&rig);
    let ford = spots.first().expect("the ground names a spot").name.clone();
    let named = format!("Steering for {ford} (A).");
    assert_eq!(rig.say(0, &format!("drive {ford}")), named);
    assert_eq!(
        rig.say(0, &format!("DRIVE {}", ford.to_uppercase())),
        named,
        "case-folded"
    );
    assert_eq!(rig.say(0, "drive a"), named, "by its map letter");
    assert_eq!(rig.say(0, "drive 12,34"), "Steering for cell 12,34.");
    assert_eq!(rig.say(0, "drive 12 34"), "Steering for cell 12,34.");
    assert_eq!(rig.say(0, "drive hold"), cockpit::DRIVE_HOLDING);
    assert_eq!(rig.say(0, "DRIVE OFF"), cockpit::DRIVE_RELEASED);
    assert_eq!(cockpit::DRIVE_HOLDING, "Holding here.");
    assert_eq!(cockpit::DRIVE_RELEASED, "DRIVE off; the helm is yours.");

    // Off the window: the proving ground is 256 cells a side.
    for word in ["256,3", "3,256", "70000,1"] {
        assert_eq!(
            rig.say(0, &format!("drive {word}")),
            cockpit::DRIVE_OFF_WINDOW,
            "{word}"
        );
    }
    // Anything else: ONE sentence, byte for byte, and never an echo.
    for word in ["BA", "X2", "nonsense", "12", "12,"] {
        let said = rig.say(0, &format!("drive {word}"));
        assert_eq!(said, cockpit::DRIVE_NOT_A_GOAL, "{word}");
    }
    assert_eq!(
        cockpit::DRIVE_NOT_A_GOAL,
        "DRIVE refused. That is not a spot, a cell, HOLD or OFF."
    );

    // Stop where it stands, then drive four cells along the road it faces.
    assert_eq!(rig.say(0, "drive hold"), cockpit::DRIVE_HOLDING);
    let from = rig.held_frame(0).cell.expect("the tank is on the window");
    let to = (from.0.saturating_add(4), from.1);
    let mark = rig.log.len();
    assert_eq!(
        rig.say(0, &format!("drive {},{}", to.0, to.1)),
        format!("Steering for cell {},{}.", to.0, to.1)
    );
    let mut arrived = false;
    for _ in 0..4_000 {
        rig.pump();
        if m1a4_said_since(&rig, pilot, mark)
            .iter()
            .any(|line| line.contains(cockpit::DRIVE_ARRIVED))
        {
            arrived = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let told = m1a4_said_since(&rig, pilot, mark);
    assert!(arrived, "the motor never said it arrived: {told:?}");
    assert!(
        !told.iter().any(|line| line.contains("DRIVE refused")),
        "the arena refused the goal: {told:?}"
    );
    let at = rig.held_frame(0).cell.expect("on the window");
    assert!(
        at.0.abs_diff(to.0).max(at.1.abs_diff(to.1)) <= 1,
        "arrived at {at:?} for {to:?}"
    );
    // And the helm is the pilot's again the moment they type one.
    assert_eq!(rig.say(0, "heading 90"), "Steering to 90°.");

    // Local refusals off the held frame, in `HEADING`'s words and order.
    let mut stuck = rig.held_frame(0);
    stuck.notices.clear();
    stuck.immobile = true;
    stuck.power = tacsim::arena::state::Power::Online;
    rig.push_frame(0, stuck);
    let said = rig.say(0, "drive a");
    assert!(said.contains(cockpit::IMMOBILE), "{said}");
    assert_eq!(said, rig.say(0, "heading 90"));
    let mut dead = rig.held_frame(0);
    dead.notices.clear();
    dead.wreck = true;
    dead.immobile = true;
    dead.power = tacsim::arena::state::Power::Off;
    rig.push_frame(0, dead);
    let said = rig.say(0, "drive a");
    assert!(said.contains(cockpit::DESTROYED), "{said}");
    assert_eq!(said, rig.say(0, "heading 90"));
}

/// **The two `DRIVE` ring rows are the server's own wire shape** (17b cut
/// M1a.4): each notice `status_frame` publishes for the motor's two answers —
/// the kind, the sentence, and every other key `null` or `false` — equals the
/// `notice` object of its row in `notice_lines.json` byte for byte, so the
/// TUI's ring and the web's are pinned against what the server sends.
#[test]
fn m1a4_the_drive_notice_rows_are_what_status_frame_publishes() {
    use tacsim::arena::perception::{DriveOutcome, FrameNotice};

    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../web/src/lib/fixtures/cockpit/notice_lines.json"
    ))
    .expect("notice_lines.json is an array");
    for (outcome, kind, text) in [
        (
            DriveOutcome::Arrived,
            "drive_arrived",
            cockpit::DRIVE_ARRIVED,
        ),
        (
            DriveOutcome::NoRoute,
            "drive_no_route",
            cockpit::DRIVE_NO_ROUTE,
        ),
    ] {
        let mut frame = fixture::frame();
        frame.notices = vec![FrameNotice::Drive { outcome }];
        let payload = cockpit::status_frame(fixture::MATCH, &frame);
        let published = payload
            .pointer("/notices/0")
            .cloned()
            .unwrap_or_else(|| panic!("{kind}: no notice on the frame: {payload}"));
        assert_eq!(published.get("text"), Some(&serde_json::json!(text)));
        let committed = rows
            .iter()
            .find(|row| {
                row.get("notice")
                    .is_some_and(|notice| notice.get("kind") == Some(&serde_json::json!(kind)))
            })
            .cloned()
            .unwrap_or_else(|| panic!("notice_lines.json has no {kind} row"));
        assert_eq!(committed.get("notice"), Some(&published), "{kind}");
        assert_eq!(
            committed.get("line"),
            Some(&serde_json::json!(text)),
            "{kind}"
        );
        assert_eq!(cockpit::notice_lines(&frame), vec![text.to_owned()]);
    }
}

/// **The `profile` refusal's ring row is the server's own wire shape** (17b
/// cut M5.2; the design note's M5.2 "What moves", the wire): the notice
/// `status_frame` publishes for the arena's refusal of a `MIND PROFILE` —
/// `order: "mind"`, `refusal: "profile"`, no weapon, no detail — equals the
/// `notice` object of its row in `notice_lines.json` byte for byte, its
/// sentence is `MIND refused. That profile cannot be switched to now.`
/// (composed through `order_refused` from the intent's word, no renderer
/// special-cased), and the line client prints exactly that.
#[test]
fn m5_the_profile_refusal_row_is_what_status_frame_publishes() {
    use tacsim::arena::intents::{IntentKind, Refusal};
    use tacsim::arena::perception::{FrameNotice, RefusalDetail};

    let text = "MIND refused. That profile cannot be switched to now.";
    let rows: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../web/src/lib/fixtures/cockpit/notice_lines.json"
    ))
    .expect("notice_lines.json is an array");
    let mut frame = fixture::frame();
    frame.notices = vec![FrameNotice::Refused {
        order: IntentKind::Profile { index: 1 }.as_str().into(),
        refusal: Refusal::Profile,
        weapon: None,
        detail: RefusalDetail::None,
    }];
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    let published = payload
        .pointer("/notices/0")
        .cloned()
        .unwrap_or_else(|| panic!("no notice on the frame: {payload}"));
    assert_eq!(published.get("text"), Some(&serde_json::json!(text)));
    let committed = rows
        .iter()
        .find(|row| {
            row.get("notice")
                .is_some_and(|notice| notice.get("refusal") == Some(&serde_json::json!("profile")))
        })
        .cloned()
        .expect("notice_lines.json has a profile row");
    assert_eq!(committed.get("notice"), Some(&published));
    assert_eq!(
        committed.get("line"),
        Some(&serde_json::json!(
            "MIND refused: that profile cannot be switched to now"
        ))
    );
    assert_eq!(cockpit::notice_lines(&frame), vec![text.to_owned()]);
}

// ---------------------------------------------------------------------------
// 17b cut M1b.3 — the arming and the floor on the frame, Q2's odds rows
// (doc 17 §3, §5, D-UM-4, D-UM-5, D-UM-17)
// ---------------------------------------------------------------------------

/// **17b cut M1b.3**: the fixture scene's keyframe at `FRAME_VERSION` 3 — the
/// cannon's `armed` object key by key (the slot, the target's KIND, the odds
/// per mille and per cent, the facet words of the mask, the rounds kept, and
/// `null` for each unbound range), the machine gun's `null`, the top-level
/// `odds_floor_permille`. A `FrameArming` built from each `SlotTarget` writes
/// `lock`, `best` or `contact` and never an id; a bounded range writes its
/// metres. `Frame::same_as` differs on an arming change alone and not on the
/// floor. And the ONE formatter's line for the scene's cannon is the line the
/// web's view model and the TUI's golden print under the cannon's row: three
/// renderers, one shape.
#[test]
fn m1b3_the_keyframe_carries_the_arming_the_floor_and_an_unarmed_row_null() {
    use tacsim::arena::conditions::{ASPECTS_ANY, SlotTarget};
    use tacsim::arena::state::ContactId;
    use tacsim::arena::weapons::FrameArming;
    let frame = fixture::frame();
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    assert_eq!(cockpit::FRAME_VERSION, 8);
    assert_eq!(
        payload.pointer("/schema/version"),
        Some(&serde_json::json!(8))
    );
    assert_eq!(
        payload.pointer("/weapons/0/armed"),
        Some(&serde_json::json!({
            "slot": 1,
            "target": "lock",
            "min_permille": 600,
            "percent": 60,
            "aspects": ["left", "right", "rear"],
            "keeps": 8,
            "min_range_m": null,
            "max_range_m": null,
        }))
    );
    assert_eq!(
        payload.pointer("/weapons/1/armed"),
        Some(&serde_json::Value::Null),
        "an unarmed row is null, not absent"
    );
    assert_eq!(
        payload.get("odds_floor_permille"),
        Some(&serde_json::json!(100))
    );
    // Each target's word, and never a designator or an id.
    let of = |target: SlotTarget| FrameArming {
        slot: 2,
        target,
        min_permille: 450,
        aspects: ASPECTS_ANY,
        keeps: 0,
        min_range_m: 200,
        max_range_m: 600,
    };
    for (target, word) in [
        (SlotTarget::Lock, "lock"),
        (SlotTarget::BestHostile, "best"),
        (
            SlotTarget::Contact {
                id: ContactId(0x1111_1111_1111_1111),
            },
            "contact",
        ),
    ] {
        let object = cockpit::arming_json(&of(target));
        assert_eq!(object.get("target"), Some(&serde_json::json!(word)));
        assert_eq!(
            object,
            serde_json::json!({
                "slot": 2,
                "target": word,
                "min_permille": 450,
                "percent": 45,
                "aspects": ["front", "left", "right", "rear"],
                "keeps": 0,
                "min_range_m": 200,
                "max_range_m": 600,
            }),
            "every key, and nothing else"
        );
        let text = object.to_string();
        assert!(!text.contains("1111111111111111"), "an id leaked: {text}");
        assert!(!text.contains("BA"), "a designator leaked: {text}");
    }
    // `same_as`: the arming alone re-sends the frame; the floor never does.
    let mut rearmed = fixture::frame();
    if let Some(gun) = rearmed.weapons.first_mut()
        && let Some(armed) = gun.armed.as_mut()
    {
        armed.min_permille = 700;
    }
    assert!(
        !frame.same_as(&rearmed, 1_000),
        "an arming change is a frame"
    );
    let mut cleared = fixture::frame();
    if let Some(gun) = cleared.weapons.first_mut() {
        gun.armed = None;
    }
    assert!(!frame.same_as(&cleared, 1_000), "a disarm is a frame");
    let mut floored = fixture::frame();
    floored.odds_floor_permille = 50;
    assert!(frame.same_as(&floored, 1_000), "the floor is not compared");
    // The weapon convenience field still has its short formatter. M5rules
    // readers show the shared ordered-rule summary above the weapons instead.
    let armed = frame
        .weapons
        .first()
        .and_then(|gun| gun.armed.as_ref())
        .expect("the scene's cannon is armed");
    let words = cockpit::arming_words(armed);
    assert_eq!(words, "S1: >=60% side/rear keeps 8");
    let web = include_str!("../../../../web/src/lib/fixtures/cockpit/web_view_model.txt");
    let lines: Vec<&str> = web.lines().collect();
    assert!(lines.contains(&"rules: S1 fire; MIND RULES"));
    assert!(lines.iter().any(|line| line.starts_with("cannon  ")));
    let tui = include_str!("../../../mush-net-ssh/tests/golden/tui_cockpit_120x34.txt");
    assert!(
        tui.lines()
            .any(|line| line.contains("rules: S1 fire; MIND RULES")),
        "the TUI's weapons pane prints the ordered-rule summary"
    );
}

/// **The words of every term** (17b cut M1b.3): `arming_words` over the
/// target, the odds, the facets, the range and the reserve — the lines doc 17
/// §5 and 17b's plan quote, verbatim.
#[test]
fn m1b3_arming_words_says_every_term() {
    use tacsim::arena::conditions::{ASPECTS_ANY, SlotTarget, aspect_mask};
    use tacsim::arena::perception::Aspect;
    use tacsim::arena::state::ContactId;
    use tacsim::arena::weapons::FrameArming;
    let flank_rear = aspect_mask(&[Aspect::Left, Aspect::Right, Aspect::Rear]);
    let base = FrameArming {
        slot: 1,
        target: SlotTarget::Lock,
        min_permille: 600,
        aspects: ASPECTS_ANY,
        keeps: 0,
        min_range_m: 0,
        max_range_m: u32::MAX,
    };
    for (armed, words) in [
        (
            FrameArming {
                aspects: flank_rear,
                keeps: 8,
                ..base.clone()
            },
            "S1: >=60% side/rear keeps 8",
        ),
        (
            FrameArming {
                aspects: flank_rear,
                keeps: 8,
                min_range_m: 200,
                max_range_m: 600,
                ..base.clone()
            },
            "S1: >=60% side/rear 200-600m keeps 8",
        ),
        // T1's slot: the best hostile at Line's floor, under its ceiling.
        (
            FrameArming {
                target: SlotTarget::BestHostile,
                min_permille: 100,
                max_range_m: 420,
                ..base.clone()
            },
            "S1: best >=10% under 420m",
        ),
        (
            FrameArming {
                slot: 2,
                target: SlotTarget::Contact { id: ContactId(7) },
                ..base.clone()
            },
            "S2: contact >=60%",
        ),
        (
            FrameArming {
                min_range_m: 300,
                ..base.clone()
            },
            "S1: >=60% past 300m",
        ),
        (
            FrameArming {
                aspects: aspect_mask(&[Aspect::Front]),
                ..base.clone()
            },
            "S1: >=60% front",
        ),
        (
            FrameArming {
                aspects: aspect_mask(&[Aspect::Right]),
                ..base.clone()
            },
            "S1: >=60% side",
        ),
        (
            FrameArming {
                aspects: aspect_mask(&[Aspect::Front, Aspect::Left, Aspect::Rear]),
                ..base.clone()
            },
            "S1: >=60% front/side/rear",
        ),
        (
            FrameArming {
                aspects: 0,
                ..base.clone()
            },
            "S1: >=60%",
        ),
        (base.clone(), "S1: >=60%"),
    ] {
        assert_eq!(cockpit::arming_words(&armed), words, "{armed:?}");
    }
}

/// One frame of `unit` off a BUILT arena (17b cut M1b.3): a fresh pilot seated
/// on it for one pulse at the arena's own wall reference, so no arena time
/// passes and nothing hand-held is re-sighted.
fn m1b3_frame_of(
    arena: &mut tacsim::arena::state::Arena,
    unit: ObjId,
    pilot: u32,
) -> tacsim::arena::step::Frame {
    let pilot = ObjId(pilot);
    arena.seat_pilot(pilot, unit, 1);
    let out = arena.step(0, &[], &[]);
    arena.drop_pilot(pilot, unit);
    out.frames
        .into_iter()
        .find(|(seat, _)| *seat == pilot)
        .map(|(_, frame)| frame)
        .expect("a seated pilot is sent a frame")
}

/// **Q2, on a built arena** (17b cut M1b.3; the lead's ruling on M1a's
/// residual (1)): `Arena::frame_odds` seats the hull's LOCK first when it is a
/// classified contact of another side, then the nearest other classified
/// hostiles, the first `STOCK_ODDS_ROWS`; never a row of the hull's own side
/// and never an unclassified one.
///
/// The proving ground with red's four tanks and blue's four, posed by hand on
/// one line north and east of red 1: two wingmen at 100 m and 150 m, an
/// unclassified blue at 200 m, and three classified blues at 300 m, 400 m and
/// 500 m. Red 1 holds every one of them. Locked on the FARTHEST hostile, that
/// one takes row one and the nearest hostile row two, and every other row
/// prints the dash — both friends, the unidentified return, and the hostile
/// past the two; row one's cannon figure is the keyframe's `lock_p_permille`.
/// With no lock, the two nearest hostiles. With a FRIENDLY lock the `LOCK`
/// line still quotes the lock's own aim (marked friendly) and the rows are the
/// two nearest hostiles.
#[test]
fn m1b3_the_odds_rows_seat_the_lock_first_and_never_a_friend() {
    use tacsim::arena::state::{Classification, ContactId, SeenPose, ShadowContact};
    use tacsim::arena::view::OddsCell;
    let world = support::booted_world();
    let mut arena = support::shipped_arena(
        &world,
        &[
            &[
                support::Muster::piloted(PartyId::RED, 1),
                support::Muster::piloted(PartyId::RED, 2),
                support::Muster::piloted(PartyId::RED, 3),
            ],
            &[
                support::Muster::piloted(PartyId::BLUE, 1),
                support::Muster::piloted(PartyId::BLUE, 2),
                support::Muster::piloted(PartyId::BLUE, 3),
                support::Muster::piloted(PartyId::BLUE, 4),
            ],
        ],
    );
    let red = support::tank(&world, PartyId::RED, 1);
    let (x0, y0) = (128_500_i64, 128_500_i64);
    // (unit, row, dx, dy, classified) — every row red 1 holds.
    let rb = ContactId(0x0B);
    let rc = ContactId(0x0C);
    let x5 = ContactId(0x05);
    let ba = ContactId(0x1A);
    let bb = ContactId(0x1B);
    let bc = ContactId(0x1C);
    let field = [
        (support::tank(&world, PartyId::RED, 2), rb, 0, 10_000, true),
        (support::tank(&world, PartyId::RED, 3), rc, 15_000, 0, true),
        (support::tank(&world, PartyId::BLUE, 4), x5, 0, 20_000, false),
        (support::tank(&world, PartyId::BLUE, 1), ba, 0, 30_000, true),
        (support::tank(&world, PartyId::BLUE, 2), bb, 40_000, 0, true),
        (support::tank(&world, PartyId::BLUE, 3), bc, 0, 50_000, true),
    ];
    if let Some(state) = arena.unit_mut(red) {
        state.pose.x_cm = x0;
        state.pose.y_cm = y0;
        state.power = tacsim::arena::state::Power::Online;
    }
    for (unit, row, dx, dy, classified) in field {
        let team = {
            let state = arena.unit_mut(unit).expect("a fielded tank");
            state.pose.x_cm = x0 + dx;
            state.pose.y_cm = y0 + dy;
            state.team
        };
        let plot = SeenPose::of(arena.unit(unit).expect("a fielded tank"));
        let class = if classified {
            Classification::Classified { party: team }
        } else {
            Classification::Unknown
        };
        arena
            .contacts
            .insert(red, row, ShadowContact::sighted(unit, class, 0, plot));
    }
    let rows_of = |frame: &tacsim::arena::step::Frame| {
        let mut rows: Vec<ContactId> = Vec::new();
        for odds in &frame.odds {
            if !rows.contains(&odds.contact) {
                rows.push(odds.contact);
            }
        }
        rows
    };
    let cell_of = |frame: &tacsim::arena::step::Frame, id: ContactId| {
        cockpit::frame_contacts(frame)
            .into_iter()
            .find(|row| row.id == id)
            .unwrap_or_else(|| panic!("row {id} on the frame"))
    };

    // Locked on the farthest hostile: it takes row one, the nearest row two.
    if let Some(state) = arena.unit_mut(red) {
        state.lock = Some(bc);
    }
    let frame = m1b3_frame_of(&mut arena, red, 9_101);
    assert_eq!(
        arena.frame_odds(red),
        frame.odds,
        "the frame's odds are frame_odds'"
    );
    assert_eq!(
        rows_of(&frame),
        vec![bc, ba],
        "the lock first, then the nearest"
    );
    for quiet in [rb, rc, x5, bb] {
        let row = cell_of(&frame, quiet);
        assert_eq!(row.odds, OddsCell::Dash, "{quiet}");
        assert!(row.render().ends_with("  -"), "{}", row.render());
    }
    for loud in [bc, ba] {
        assert!(matches!(cell_of(&frame, loud).odds, OddsCell::Odds(..)));
    }
    // Row one's cannon figure is the lock's own: the keyframe's
    // `lock_p_permille`, one reading of one plot.
    let main = tacsim::arena::view::fired_by_default(&frame.weapons).expect("a main gun");
    let first = frame.odds.first().expect("row one");
    assert_eq!(first.contact, bc);
    assert_eq!(first.weapon, main.id, "led by the weapon a bare FIRE fires");
    assert_eq!(Some(first.p_permille), main.p_permille);
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    assert_eq!(
        payload.get("lock_p_permille"),
        Some(&serde_json::json!(first.p_permille))
    );

    // No lock: the two nearest hostiles, and the wingmen nearer than both
    // carry nothing.
    if let Some(state) = arena.unit_mut(red) {
        state.lock = None;
    }
    let frame = m1b3_frame_of(&mut arena, red, 9_102);
    assert_eq!(rows_of(&frame), vec![ba, bb], "the two nearest hostiles");
    for quiet in [rb, rc, x5, bc] {
        assert_eq!(cell_of(&frame, quiet).odds, OddsCell::Dash, "{quiet}");
    }

    // A friendly lock: no row of its own, the two nearest hostiles — and the
    // `LOCK` line still quotes the lock's own aim.
    if let Some(state) = arena.unit_mut(red) {
        state.lock = Some(rb);
    }
    let frame = m1b3_frame_of(&mut arena, red, 9_103);
    assert_eq!(
        rows_of(&frame),
        vec![ba, bb],
        "a friend's lock takes no row"
    );
    assert_eq!(cell_of(&frame, rb).odds, OddsCell::Dash);
    let aim = arena.aim_at(red, rb).expect("the friend is held");
    let main = tacsim::arena::view::fired_by_default(&frame.weapons).expect("a main gun");
    let own = arena.resolve_seen(red, &aim, &main.id).p_permille;
    assert_eq!(main.p_permille, Some(own), "the lock's own aim");
    let lock = cockpit::lock_line(&frame).expect("a lock line");
    assert!(lock.contains("(FRIENDLY)"), "{lock}");
    assert!(lock.contains(&format!("{}%", own / 10)), "{lock}");
}

/// **The match's floor rides the frame** (17b cut M1b.3; doc 17 §5 as
/// amended, D-UM-4, D-UM-17): an arena checked out at a floor of 50 per mille
/// and one at the default 100 — the keyframe's top-level `odds_floor_permille`
/// and a mind's `MindView.odds_floor_permille` read the arena's captured
/// figure; the fixture scene's is 100.
#[test]
fn m1b3_the_frame_carries_the_match_floor() {
    let world = support::booted_world();
    for floor in [50_u32, 100] {
        let mut arena = support::shipped_arena(
            &world,
            &[
                &[
                    support::Muster::piloted(PartyId::RED, 1),
                    support::Muster::mind(PartyId::RED, 2),
                ],
                &[support::Muster::piloted(PartyId::BLUE, 1)],
            ],
        );
        assert_eq!(arena.odds_floor_permille, 100, "Arena::new's default");
        // What `spawn_arena` writes from `[tacsim.minds]` at check-out.
        arena.odds_floor_permille = floor;
        let red = support::tank(&world, PartyId::RED, 1);
        let mind = support::tank(&world, PartyId::RED, 2);
        let frame = m1b3_frame_of(&mut arena, red, 9_201);
        assert_eq!(frame.odds_floor_permille, floor);
        let payload = cockpit::status_frame(fixture::MATCH, &frame);
        assert_eq!(
            payload.get("odds_floor_permille"),
            Some(&serde_json::json!(floor))
        );
        let view = arena.mind_view(mind).expect("a mind's view");
        assert_eq!(view.odds_floor_permille, floor);
    }
    assert_eq!(fixture::frame().odds_floor_permille, 100);
    let committed: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../web/src/lib/fixtures/cockpit/cockpit_frame.json"
    ))
    .expect("the fixture is JSON");
    assert_eq!(
        committed.get("odds_floor_permille"),
        Some(&serde_json::json!(100))
    );
}

// ---------------------------------------------------------------------------
// 17b cut M1b.4 — the pilot's surface: `FIRE <weapon> WHEN …`, `FIRE … OFF`,
// `WEAPONSTATUS`' arming line and the two slot sentences (doc 17 §5, D-UM-14,
// D-UM-17 as amended, D-UM-18's slot-fitting subset)
// ---------------------------------------------------------------------------

/// The fixture scene's row of `id`, as `FIRE` would choose it.
fn m1b4_row(frame: &tacsim::arena::step::Frame, id: &str) -> cockpit::WeaponRow {
    cockpit::frame_weapons(frame)
        .into_iter()
        .find(|row| row.id == id)
        .unwrap_or_else(|| panic!("the scene carries {id}"))
}

/// The floor sentence, as the cockpit and the arena both print it.
const M1B4_FLOOR: &str =
    "FIRE cannon refused. The Compact floor is 10%; a condition may not ask for less.";

/// The outside sentence's said-half.
const M1B4_OUTSIDE: &str =
    "That condition is not one this core takes; help cockpit lists the supported native rules.";

/// The gate's rule's reply.
const M1B4_GATE_REPLY: &str = "Arming cannon: fires your lock when your chance to hit is 60% or \
                               better and it shows its side or rear.";

/// **The pure grammar** (17b cut M1b.4; D-UM-18's slot-fitting subset):
/// `parse_arming` over every one of the eight terms and every value shape —
/// the compiled `FireCondition` per case — the floor refused `Unlawful` at 5
/// and 9 on a floor-100 frame and accepted at 10 and 100, a floor-50 frame
/// accepting 5, the ONE outside sentence for everything else (101, `OR`, a
/// fifth condition, each word this core does not take, a malformed or missing
/// number, a conflicting repeat, another gun's odds), `band = …` compiled to
/// exactly the metres `contact_band` cuts on the fixture frame, and the
/// English written back for the gate's rule verbatim, a bounded range, a
/// reserve and a rule with no odds term (which asks for the floor).
///
/// **17b cut M2.2, the lead's ruling (c)**: the two range terms compile
/// STRICT — `range < 600` stores `max_range_m 599`, `range > 200` stores
/// `min_range_m 201` (M1b.4 stored 600 and 200) — and the English is written
/// back off the rule as typed (`parse_arming`'s `ArmingEcho`), so every
/// reply below is M1b.4's byte for byte.
#[test]
fn m1b4_fire_when_compiles_the_eight_terms_and_refuses_the_rest_in_one_sentence() {
    use tacsim::arena::conditions::{ASPECTS_ANY, FireCondition, SlotTarget, aspect_mask};
    use tacsim::arena::perception::Aspect;
    let frame = fixture::frame();
    assert_eq!(frame.odds_floor_permille, 100);
    let cannon = m1b4_row(&frame, "cannon");
    let mg = m1b4_row(&frame, "machine_gun");
    let base = FireCondition {
        weapon: "cannon".into(),
        target: SlotTarget::Lock,
        min_permille: 100,
        aspects: ASPECTS_ANY,
        min_rounds_left: 0,
        min_range_m: 0,
        max_range_m: u32::MAX,
    };
    let flank_rear = aspect_mask(&[Aspect::Left, Aspect::Right, Aspect::Rear]);
    let parse = |row: &cockpit::WeaponRow, rule: &str| {
        cockpit::parse_arming(row, rule, &frame).map(|(cond, _)| cond)
    };
    // Every term, every spelling, compiled.
    for (rule, want) in [
        (
            "odds >= 60",
            FireCondition {
                min_permille: 600,
                ..base.clone()
            },
        ),
        (
            "ODDS>=60",
            FireCondition {
                min_permille: 600,
                ..base.clone()
            },
        ),
        (
            "odds cannon >= 60",
            FireCondition {
                min_permille: 600,
                ..base.clone()
            },
        ),
        (
            "odds can >= 60",
            FireCondition {
                min_permille: 600,
                ..base.clone()
            },
        ),
        (
            "odds >= 100",
            FireCondition {
                min_permille: 1_000,
                ..base.clone()
            },
        ),
        (
            "odds >= 10",
            FireCondition {
                min_permille: 100,
                ..base.clone()
            },
        ),
        (
            "aspect = side,rear",
            FireCondition {
                aspects: flank_rear,
                ..base.clone()
            },
        ),
        (
            "aspect=side , rear",
            FireCondition {
                aspects: flank_rear,
                ..base.clone()
            },
        ),
        (
            "aspect = front",
            FireCondition {
                aspects: aspect_mask(&[Aspect::Front]),
                ..base.clone()
            },
        ),
        (
            "aspect = side,side",
            FireCondition {
                aspects: aspect_mask(&[Aspect::Left, Aspect::Right]),
                ..base.clone()
            },
        ),
        // 17b cut M2.2, ruling (c): strict — 600 -> 599 and 200 -> 201.
        (
            "range < 600 m",
            FireCondition {
                max_range_m: 599,
                ..base.clone()
            },
        ),
        (
            "range<600m",
            FireCondition {
                max_range_m: 599,
                ..base.clone()
            },
        ),
        (
            "range > 200",
            FireCondition {
                min_range_m: 201,
                ..base.clone()
            },
        ),
        (
            "range > 200 m AND range < 600 m",
            FireCondition {
                min_range_m: 201,
                max_range_m: 599,
                ..base.clone()
            },
        ),
        (
            "band = close",
            FireCondition {
                max_range_m: 200,
                ..base.clone()
            },
        ),
        (
            "band = effective",
            FireCondition {
                min_range_m: 201,
                max_range_m: 400,
                ..base.clone()
            },
        ),
        (
            "BAND = FAR",
            FireCondition {
                min_range_m: 401,
                max_range_m: 3_000,
                ..base.clone()
            },
        ),
        (
            "rounds > 8",
            FireCondition {
                min_rounds_left: 8,
                ..base.clone()
            },
        ),
        (
            "rounds > 25%",
            FireCondition {
                min_rounds_left: 5,
                ..base.clone()
            },
        ),
        (
            "rounds > 25 %",
            FireCondition {
                min_rounds_left: 5,
                ..base.clone()
            },
        ),
        (
            "rounds > 20",
            FireCondition {
                min_rounds_left: 20,
                ..base.clone()
            },
        ),
        ("rounds > 0", base.clone()),
        ("reserve", base.clone()),
        ("target = identified", base.clone()),
        ("lock", base.clone()),
        (
            "odds >= 60 AND odds >= 60",
            FireCondition {
                min_permille: 600,
                ..base.clone()
            },
        ),
        (
            "odds >= 60 and aspect = side,rear AND range < 600 m AND rounds > 8",
            FireCondition {
                min_permille: 600,
                aspects: flank_rear,
                max_range_m: 599,
                min_rounds_left: 8,
                ..base.clone()
            },
        ),
    ] {
        assert_eq!(parse(&cannon, rule), Ok(want), "{rule}");
    }
    // The machine gun's own bands and load: `band` is this weapon's.
    assert_eq!(
        parse(&mg, "band = effective").map(|c| (c.weapon, c.min_range_m, c.max_range_m)),
        Ok(("machine_gun".into(), 134, 266))
    );
    assert_eq!(
        parse(&mg, "rounds > 10%").map(|c| c.min_rounds_left),
        Ok(50)
    );
    assert_eq!(parse(&mg, "odds mg >= 50").map(|c| c.min_permille), Ok(500));
    // The band's cuts are `contact_band`'s, metre for metre, on this frame.
    let bands = &frame.weapons[0].bands_cm;
    let cuts = cockpit::band_bounds_m(frame.sight_cm, bands).expect("the cannon has bands");
    assert_eq!(cuts, [(0, 200), (201, 400), (401, 3_000)]);
    for (word, (nearest, furthest)) in ["close", "effective", "far"].into_iter().zip(cuts) {
        for metres in [nearest.max(1), furthest] {
            assert_eq!(
                cockpit::contact_band(u64::from(metres) * 100, bands, frame.sight_cm),
                word,
                "{metres} m"
            );
        }
    }
    assert_eq!(
        cockpit::contact_band(3_001 * 100, bands, frame.sight_cm),
        "beyond"
    );
    assert_eq!(cockpit::band_bounds_m(frame.sight_cm, &[]), None);
    // The floor: refused AS TYPED, refuse-not-clamp.
    for rule in ["odds >= 5", "odds >= 9", "odds cannon >= 1"] {
        let refused = parse(&cannon, rule);
        assert_eq!(
            refused,
            Err(cockpit::CockpitError::Unlawful { floor_pct: 10 }),
            "{rule}"
        );
        assert_eq!(
            cockpit::order_refused("fire", Some("cannon"), &refused.unwrap_err().to_string()),
            M1B4_FLOOR
        );
    }
    // The cockpit's floor sentence IS the arena's landing refusal's.
    assert_eq!(
        cockpit::notice_line(
            &frame,
            &tacsim::arena::perception::FrameNotice::Refused {
                order: "fire".into(),
                refusal: tacsim::arena::intents::Refusal::Unlawful,
                weapon: Some("cannon".into()),
                detail: tacsim::arena::perception::RefusalDetail::Floor { permille: 100 },
            },
            &cockpit::frame_contacts(&frame),
        ),
        M1B4_FLOOR
    );
    // A frame at a floor of 50 per mille takes 5.
    let mut low = fixture::frame();
    low.odds_floor_permille = 50;
    assert_eq!(
        cockpit::parse_arming(&cannon, "odds >= 5", &low).map(|(c, _)| c.min_permille),
        Ok(50)
    );
    assert_eq!(
        cockpit::parse_arming(&cannon, "odds >= 4", &low),
        Err(cockpit::CockpitError::Unlawful { floor_pct: 5 })
    );
    assert_eq!(
        cockpit::parse_arming(&cannon, "aspect = rear", &low).map(|(c, _)| c.min_permille),
        Ok(50),
        "no odds term asks for the frame's floor"
    );
    // Everything else: ONE sentence.
    for rule in [
        "",
        "   ",
        "odds >= 101",
        "odds >= 0",
        "odds >= sixty",
        "odds >= 6.5",
        "odds >= -5",
        "odds >=",
        "odds > 60",
        "odds = 60",
        "odds mg >= 50",
        "odds ghost >= 50",
        "odds >= 60 OR aspect = rear",
        "odds >= 60 AND",
        "AND odds >= 60",
        "odds >= 60 AND aspect = rear AND range < 600 AND rounds > 2 AND lock",
        "odds >= 60 AND odds >= 70",
        "aspect = side AND aspect = rear",
        "aspect =",
        "aspect = top",
        "aspect = side,",
        "aspect = ,side",
        "aspect = side rear",
        "range < 0",
        "range < 10001",
        "range <= 600",
        "range < 600 km",
        "band = beyond",
        "band = effective AND range < 300",
        "rounds > 21",
        "rounds > 101%",
        "rounds > -1",
        "rounds >= 8",
        "target = hostile",
        "target = BA",
        "hit within 5 s",
        "hit",
        "hostiles",
        "order",
        "own",
        "heat < 50",
        "no lock",
        "lock BA",
        "LOCK X2",
        "reserve 5",
    ] {
        assert_eq!(
            parse(&cannon, rule),
            Err(cockpit::CockpitError::NotThisCore),
            "{rule:?}"
        );
    }
    assert_eq!(cockpit::CockpitError::NotThisCore.to_string(), M1B4_OUTSIDE);
    // The grammar is read whole before the floor: malformed AND under the
    // floor is the outside sentence.
    assert_eq!(
        parse(&cannon, "odds >= 5 AND hit"),
        Err(cockpit::CockpitError::NotThisCore)
    );
    // The English written back — 17b cut M2.2: off the rule as typed.
    let reply = |rule: &str| {
        let (cond, echo) =
            cockpit::parse_arming(&cannon, rule, &frame).unwrap_or_else(|e| panic!("{rule}: {e}"));
        cockpit::arming_reply(&cond.weapon, &echo)
    };
    assert_eq!(reply("odds >= 60 AND aspect = side,rear"), M1B4_GATE_REPLY);
    assert_eq!(
        reply("odds >= 45 AND range > 200 m AND range < 600 m"),
        "Arming cannon: fires your lock when your chance to hit is 45% or better and it is \
         between 200 and 600 m."
    );
    assert_eq!(
        reply("range < 600"),
        "Arming cannon: fires your lock when your chance to hit is 10% or better and it is \
         under 600 m."
    );
    assert_eq!(
        reply("range > 200"),
        "Arming cannon: fires your lock when your chance to hit is 10% or better and it is past \
         200 m."
    );
    assert_eq!(
        reply("band = effective"),
        "Arming cannon: fires your lock when your chance to hit is 10% or better and it is \
         between 201 and 400 m."
    );
    assert_eq!(
        reply("reserve"),
        "Arming cannon: fires your lock when your chance to hit is 10% or better."
    );
    assert_eq!(
        reply("aspect = front, rear AND rounds > 8"),
        "Arming cannon: fires your lock when your chance to hit is 10% or better and it shows \
         its front or rear and cannon keeps 8 round(s)."
    );
    assert_eq!(
        reply("target = identified AND lock"),
        "Arming cannon: fires your lock when your chance to hit is 10% or better."
    );
    for rule in [
        "odds >= 60",
        "aspect = side,rear",
        "band = far",
        "rounds > 8",
    ] {
        let said = reply(rule);
        assert!(
            !said.contains("S1") && !said.contains("S2"),
            "no slot number: {said}"
        );
    }
}

/// **The range terms are strict, the reply reads the figure typed** (17b cut
/// M2.2, the lead's ruling (c); doc 17 §5): on the fixture frame,
/// `range < 600` stores `max_range_m 599` and replies `it is under 600 m`;
/// `range > 200` stores `min_range_m 201` and replies `it is past 200 m`;
/// both store `(201, 599)` and reply `it is between 200 and 600 m`; `band =
/// close` stores `(0, 200)` and replies `it is under 200 m` (a band is fixed
/// metres, untouched); `range < 1` stores 0. `WEAPONSTATUS`'s arming line
/// (`arming_words`, untouched) prints the STORED figure — `S1: >=10% under
/// 599m`, the last metre that fires. **Declared effect 1**: `range < 200 AND
/// band = close` — both 200 under M1b.4 — now reads 199 against the band's
/// 200 and is refused as a differing duplicate (one figure per term), in the
/// one outside sentence; and its mirror at the band's lower edge, `range >
/// 200 AND band = effective` — 200 against 201 under M1b.4, refused — now
/// reads 201 against 201 and is TAKEN (`range > 201` with it is refused).
/// And `help cockpit` says all of it.
#[test]
fn m2_a_range_term_is_strict_and_the_reply_reads_the_typed_figure() {
    use tacsim::arena::conditions::{ASPECTS_ANY, FireCondition, SlotTarget};
    use tacsim::arena::weapons::FrameArming;
    let frame = fixture::frame();
    let cannon = m1b4_row(&frame, "cannon");
    let armed = |rule: &str| {
        let (cond, echo) =
            cockpit::parse_arming(&cannon, rule, &frame).unwrap_or_else(|e| panic!("{rule}: {e}"));
        let said = cockpit::arming_reply(&cond.weapon, &echo);
        ((cond.min_range_m, cond.max_range_m), said, cond)
    };
    let tail = |clause: &str| {
        format!("Arming cannon: fires your lock when your chance to hit is 10% or better{clause}.")
    };
    for (rule, stored, clause) in [
        ("range < 600", (0, 599), " and it is under 600 m"),
        ("range > 200", (201, u32::MAX), " and it is past 200 m"),
        (
            "range > 200 AND range < 600",
            (201, 599),
            " and it is between 200 and 600 m",
        ),
        ("band = close", (0, 200), " and it is under 200 m"),
        ("range < 1", (0, 0), " and it is under 1 m"),
        (
            "range > 10000",
            (10_001, u32::MAX),
            " and it is past 10000 m",
        ),
    ] {
        let (bounds, said, _) = armed(rule);
        assert_eq!(bounds, stored, "{rule}: the stored bounds");
        assert_eq!(said, tail(clause), "{rule}: the reply");
    }
    // The echo itself, field by field, for the window.
    let (_, echo) = cockpit::parse_arming(&cannon, "range > 200 AND range < 600", &frame)
        .expect("the window compiles");
    assert_eq!(
        echo,
        cockpit::ArmingEcho {
            min_permille: Some(100),
            aspects: None,
            under_m: Some(600),
            past_m: Some(200),
            band: None,
            keeps: None,
        }
    );
    // `WEAPONSTATUS` prints what the slot stores: the last metre that fires.
    let (_, _, cond) = armed("range < 600");
    assert_eq!(
        cond,
        FireCondition {
            weapon: "cannon".into(),
            target: SlotTarget::Lock,
            min_permille: 100,
            aspects: ASPECTS_ANY,
            min_rounds_left: 0,
            min_range_m: 0,
            max_range_m: 599,
        }
    );
    assert_eq!(
        cockpit::arming_words(&FrameArming::of(1, &cond)),
        "S1: >=10% under 599m"
    );
    let (_, _, cond) = armed("range > 200");
    assert_eq!(
        cockpit::arming_words(&FrameArming::of(1, &cond)),
        "S1: >=10% past 201m"
    );
    // Declared effect 1: a band and a range term that disagree by the strict
    // metre are refused — M1b.4 read both as 200 and took the rule.
    for rule in [
        "range < 200 AND band = close",
        "band = close AND range < 200",
    ] {
        assert_eq!(
            cockpit::parse_arming(&cannon, rule, &frame),
            Err(cockpit::CockpitError::NotThisCore),
            "{rule}"
        );
    }
    // One figure for the term: the band's 200 typed as `range < 201` agrees.
    let (bounds, said, _) = armed("range < 201 AND band = close");
    assert_eq!(bounds, (0, 200));
    assert_eq!(said, tail(" and it is under 201 m"));
    // The lower edge flips the other way: `band = effective` is (201, 400) on
    // this frame, so `range > 200` — M1b.4's 200 against the band's 201,
    // refused — now stores the band's own 201 and is TAKEN, the reply reading
    // the typed 200; one metre further, `range > 201`, differs and is refused.
    let (bounds, said, _) = armed("band = effective");
    assert_eq!(bounds, (201, 400));
    assert_eq!(said, tail(" and it is between 201 and 400 m"));
    let (bounds, said, _) = armed("range > 200 AND band = effective");
    assert_eq!(bounds, (201, 400));
    assert_eq!(said, tail(" and it is between 200 and 400 m"));
    for rule in [
        "range > 201 AND band = effective",
        "band = effective AND range > 201",
    ] {
        assert_eq!(
            cockpit::parse_arming(&cannon, rule, &frame),
            Err(cockpit::CockpitError::NotThisCore),
            "{rule}"
        );
    }
    // The compatibility adapter above retains its captured-metre bounds and
    // echo. Actual pilot rules now keep typed strict predicates and reevaluate
    // bands against the current plot, as the native-rule help explains.
    let help: String = include_str!("../../../../docs/help/cockpit.md")
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ");
    for words in [
        "range < 600 m strict rounded metres; range > 200 m is also available",
        "Range comparisons use the same rounded metres as the plot.",
        "A band is evaluated again when the rule runs; it is not frozen to the metres shown when you typed the rule.",
        "`MIND RULES [slot]` reads their complete terms.",
        "A full table is refused by the arena; `MIND RULE <slot> OFF` frees a slot.",
    ] {
        assert!(help.contains(words), "help cockpit: {words:?}");
    }
}

/// **`WEAPONSTATUS`' arming line** (17b cut M1b.4; doc 17 §5 as built for the
/// 120-column floor): the compatibility `arming_words` formatter over every
/// shape, including the fixture's `  S1: >=60% side/rear keeps 8` and T1's
/// `  S1: best >=10% under 420m`. The live verb reads the full native rule
/// after its unchanged weapon rows. Every published line fits 120 columns.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m1b4_weaponstatus_prints_the_arming_line_under_the_row() {
    use tacsim::arena::conditions::{ASPECTS_ANY, SlotTarget, aspect_mask};
    use tacsim::arena::perception::Aspect;
    use tacsim::arena::weapons::FrameArming;
    let scene = fixture::frame();
    let armed = scene
        .weapons
        .first()
        .and_then(|gun| gun.armed.clone())
        .expect("the scene's cannon is armed");
    assert_eq!(
        format!("  {}", cockpit::arming_words(&armed)),
        "  S1: >=60% side/rear keeps 8"
    );
    let t1 = FrameArming {
        slot: 1,
        target: SlotTarget::BestHostile,
        min_permille: 100,
        aspects: ASPECTS_ANY,
        keeps: 0,
        min_range_m: 0,
        max_range_m: 420,
    };
    assert_eq!(
        format!("  {}", cockpit::arming_words(&t1)),
        "  S1: best >=10% under 420m"
    );

    // The widest of each, over every weapon a shipped unit mounts.
    let units = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("content/units");
    let mut ids: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&units).expect("the shipped units") {
        let path = entry.expect("an entry").path();
        let text = std::fs::read_to_string(&path).expect("a unit pack");
        let pack: serde_json::Value = serde_json::from_str(&text).expect("a JSON pack");
        let mut stack = vec![pack];
        while let Some(value) = stack.pop() {
            match value {
                serde_json::Value::Object(map) => {
                    if map.get("role").and_then(|r| r.get("kind"))
                        == Some(&serde_json::json!("weapon"))
                        && let Some(id) = map.get("id").and_then(serde_json::Value::as_str)
                    {
                        ids.push(id.to_owned());
                    }
                    stack.extend(map.into_iter().map(|(_, v)| v));
                }
                serde_json::Value::Array(list) => stack.extend(list),
                _ => {}
            }
        }
    }
    ids.sort();
    ids.dedup();
    assert!(ids.contains(&"machine_gun".to_owned()), "{ids:?}");
    let widest_row = |id: &str| cockpit::WeaponRow {
        id: id.into(),
        ready: false,
        ready_in_ms: 999_000,
        rounds: 99_999,
        rounds_max: 99_999,
        range_band: Some(tacsim::arena::weapons::FrameBand {
            index: 9,
            of: 9,
            max_range_cm: 1_000_000,
            dispersion_mrad: 99,
        }),
        p_permille: Some(1_000),
        heat_per_shot: 0,
        armed: None,
    };
    for id in &ids {
        let rows = vec![widest_row(id)];
        let row = rows[0].render(&rows);
        assert!(
            row.chars().count() < 120,
            "{} columns: {row}",
            row.chars().count()
        );
    }
    let widest_arming = FrameArming {
        slot: 2,
        target: SlotTarget::BestHostile,
        min_permille: 1_000,
        aspects: aspect_mask(&[Aspect::Front, Aspect::Left, Aspect::Rear]),
        keeps: 99_999,
        min_range_m: 10_000,
        max_range_m: 10_000,
    };
    let line = format!("  {}", cockpit::arming_words(&widest_arming));
    assert_eq!(
        line,
        "  S2: best >=100% front/side/rear 10000-10000m keeps 99999"
    );
    assert!(line.chars().count() < 120, "{line}");
    let brief = FrameArming {
        slot: 2,
        target: SlotTarget::BestHostile,
        min_permille: 950,
        aspects: aspect_mask(&[Aspect::Front, Aspect::Left]),
        keeps: 1_999,
        min_range_m: 1_000,
        max_range_m: 3_000,
    };
    assert_eq!(
        format!("  {}", cockpit::arming_words(&brief)),
        "  S2: best >=95% front/side 1000-3000m keeps 1999",
        "the plan's longest shipped arming line"
    );

    // The real verb, over a frame through the real egress seam.
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let mut frame = rig.held_frame(0);
    frame.notices.clear();
    let cannon = frame
        .weapons
        .iter()
        .position(|gun| gun.id == "cannon")
        .expect("a stock tank carries the cannon");
    assert!(frame.weapons.iter().all(|gun| gun.armed.is_none()));
    let unarmed = rig.say(0, "weaponstatus");
    assert!(
        !unarmed.lines().any(|line| line.starts_with("  S")),
        "an unarmed row prints no line: {unarmed}"
    );
    frame.weapons[cannon].armed = Some(armed.clone());
    frame.rules = scene.rules.clone();
    rig.push_frame(0, frame.clone());
    let said = rig.say(0, "weaponstatus");
    let lines: Vec<&str> = said.lines().collect();
    let row = lines
        .iter()
        .position(|line| line.starts_with("cannon  "))
        .unwrap_or_else(|| panic!("the cannon's row: {said}"));
    let rule_start = lines
        .iter()
        .position(|line| line.starts_with("S1:"))
        .unwrap_or_else(|| panic!("the complete rule: {said}"));
    assert!(
        rule_start > row,
        "the complete rule follows the weapon rows: {said}"
    );
    assert!(
        said.split_whitespace().collect::<Vec<_>>().join(" ").contains(
            "S1: cannon fires at your identified lock when its chance to hit is at least 60% and the target shows its side or rear and cannon keeps 8 round(s)"
        ),
        "{said}"
    );
    assert_eq!(
        &lines[..rule_start],
        unarmed
            .lines()
            .filter(|line| !line.starts_with("[panel "))
            .collect::<Vec<_>>(),
        "the weapon rows are unchanged"
    );
    for line in &lines {
        assert!(line.chars().count() < 120, "{line}");
    }
}

/// Pump until `done` holds of the rig, or fail saying `what`.
async fn m1b4_until(rig: &mut Rig, what: &str, done: impl Fn(&Rig) -> bool) {
    for _ in 0..1_500 {
        rig.pump();
        if done(rig) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let heard = rig.tw.seen_by(&rig.log, rig.pilot(0));
    let tail = heard
        .get(heard.len().saturating_sub(12)..)
        .unwrap_or_default();
    panic!("never: {what}; pilot 0 last heard {tail:#?}");
}

/// Let a whole arena second pass, so the next orders meet a fresh seat window
/// (`max_pending_per_unit` a second; 16b cut M.1).
async fn m1b4_next_second(rig: &mut Rig) {
    let from = rig.staged_now_ms;
    m1b4_until(rig, "an arena second", |rig| {
        rig.staged_now_ms >= from + 1_000
    })
    .await;
}

/// The slot `weapon` is armed in on that pilot's mirror, and its odds term.
fn m1b4_armed(rig: &Rig, n: usize, weapon: &str) -> Option<(u8, u16)> {
    rig.frame(n)?
        .weapons
        .iter()
        .find(|gun| gun.id == weapon)?
        .armed
        .as_ref()
        .map(|armed| (armed.slot, armed.min_permille))
}

/// **`FIRE <weapon|ALL> OFF` clears and says so** (17b cut M1b.4): the two
/// replies are unconditional — the same bytes with an arming on the mirror
/// and with none — bare `FIRE OFF` is `ALL`'s, a word after `OFF` is the
/// outside refusal and an unknown gun `FIRE`'s own; the forwarded disarm
/// clears the frame's `armed` to `null`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m1b4_fire_off_clears_and_says_so() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let one = "Cleared: cannon fires only when you say.";
    let all = "Cleared: your guns fire only when you say.";
    assert_eq!(cockpit::CLEARED_ALL, all);
    // Nothing armed.
    assert_eq!(rig.say(0, "fire cannon off"), one);
    assert_eq!(rig.say(0, "FIRE CAN OFF"), one, "the gun as FIRE names it");
    assert_eq!(rig.say(0, "fire all off"), all);
    assert_eq!(rig.say(0, "FIRE OFF"), all);
    assert_eq!(
        rig.say(0, "fire cannon off now"),
        format!("FIRE cannon refused. {M1B4_OUTSIDE}")
    );
    assert_eq!(
        rig.say(0, "fire all off now"),
        format!("FIRE refused. {M1B4_OUTSIDE}")
    );
    assert_eq!(
        rig.say(0, "fire ghost off"),
        "That unit carries no weapon called `ghost`."
    );
    // Armed, then cleared: the same bytes.
    m1b4_next_second(&mut rig).await;
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 60"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 60%"
    );
    m1b4_until(&mut rig, "the cannon armed", |rig| {
        m1b4_armed(rig, 0, "cannon").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "cannon"), Some((1, 600)));
    assert_eq!(rig.say(0, "fire cannon off"), one);
    m1b4_until(&mut rig, "the cannon cleared", |rig| {
        rig.frame(0)
            .is_some_and(|frame| frame.weapons.iter().all(|gun| gun.armed.is_none()))
    })
    .await;
    m1b4_next_second(&mut rig).await;
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 60"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 60%"
    );
    assert_eq!(
        rig.say(0, "fire mg when odds >= 30"),
        "Queued: machine_gun fires at your identified lock when its chance to hit is at least 30%"
    );
    m1b4_until(&mut rig, "both armed", |rig| {
        m1b4_armed(rig, 0, "cannon").is_some() && m1b4_armed(rig, 0, "machine_gun").is_some()
    })
    .await;
    m1b4_next_second(&mut rig).await;
    assert_eq!(rig.say(0, "fire all off"), all);
    m1b4_until(&mut rig, "every slot cleared", |rig| {
        rig.frame(0)
            .is_some_and(|frame| frame.weapons.iter().all(|gun| gun.armed.is_none()))
    })
    .await;
    // The keyframe's `armed` is `null` again.
    let frame = rig.held_frame(0);
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    for gun in payload["weapons"].as_array().cloned().unwrap_or_default() {
        assert_eq!(gun["armed"], serde_json::Value::Null, "{gun}");
    }
}

// ---------------------------------------------------------------------------
// Doc 21 cut G.12a: the master gate and the volley (§7.3)
// ---------------------------------------------------------------------------

/// **A frame the arena could have sent for a volley** (doc 21 cut G.12a):
/// pilot 0's held frame with power online and the golden scene's contacts,
/// lock and weapons — the cannon ready with a band on the lock, the machine
/// gun (reach 400 m) with none — and the gun made ready, so the reach alone
/// decides it.
fn g12a_volley_frame(rig: &Rig) -> tacsim::arena::step::Frame {
    let scene = fixture::frame();
    let mut frame = rig.held_frame(0);
    frame.notices.clear();
    frame.catchup = None;
    frame.power = tacsim::arena::state::Power::Online;
    frame.contacts = scene.contacts.clone();
    frame.lock = scene.lock;
    frame.weapons = scene.weapons.clone();
    for weapon in &mut frame.weapons {
        weapon.ready = true;
        weapon.ready_in_ms = 0;
    }
    assert_eq!(
        frame
            .weapons
            .iter()
            .map(|weapon| (weapon.id.as_str(), weapon.range_band.is_some()))
            .collect::<Vec<_>>(),
        vec![("cannon", true), ("machine_gun", false)],
        "the roster: the cannon reaches the lock, the machine gun does not"
    );
    frame
}

/// **Bare `FIRE ALL` is one volley** (doc 21 cut G.12a, §7.3; the `g` key):
/// each weapon in reach fires as `FIRE <weapon>` does and each other one says
/// why it did not, in the roster's order, one line a weapon; nothing in
/// reach is the first weapon's out-of-reach refusal; no lock is `FIRE ALL`'s
/// no-lock refusal; and `FIRE ALL OFF` / `FIRE ALL WHEN …` keep their arms.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn g12a_fire_all_fires_each_weapon_in_reach_once_and_says_the_rest() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");

    // No lock on the arena's own first frame.
    assert_eq!(
        rig.say(0, "fire all"),
        format!("FIRE ALL refused. {}", cockpit::NO_LOCK)
    );

    let frame = g12a_volley_frame(&rig);
    rig.push_frame(0, frame.clone());
    let said = rig.say(0, "FIRE ALL");
    let lines: Vec<&str> = said
        .lines()
        .filter(|line| !line.starts_with("[panel "))
        .collect();
    assert_eq!(lines.len(), 2, "one line a weapon: {said}");
    assert_eq!(lines[0], "Firing cannon.", "{said}");
    assert!(
        lines[1].starts_with("FIRE machine_gun refused. Out of reach: the lock is ")
            && lines[1].ends_with(" m off and it reaches 400 m."),
        "the out-of-reach gun is skipped with its line: {said}"
    );
    assert!(lines[1].starts_with("FIRE machine_gun refused. "));
    assert!(lines[1].contains(cockpit::OUT_OF_REACH));

    // A cycling cannon is skipped with FIRE's own words; with nothing left that
    // fires, the volley is refused in the first in-reach weapon's words.
    let mut cycling = frame.clone();
    for weapon in &mut cycling.weapons {
        if weapon.id == "cannon" {
            weapon.ready = false;
            weapon.ready_in_ms = 2_000;
        }
    }
    rig.push_frame(0, cycling);
    assert_eq!(
        rig.say(0, "fire all"),
        "FIRE cannon refused. Still cycling: 2 s to go."
    );

    // Nothing in reach: the first weapon's out-of-reach line.
    let mut far = frame.clone();
    for weapon in &mut far.weapons {
        weapon.range_band = None;
    }
    rig.push_frame(0, far);
    let said = rig.say(0, "fire all");
    assert!(
        said.starts_with("FIRE cannon refused. Out of reach: the lock is ")
            && said.ends_with(" m off and it reaches 3000 m."),
        "{said}"
    );

    // The other ALL forms keep their own arms.
    assert_eq!(rig.say(0, "fire all off"), cockpit::CLEARED_ALL);
}

/// **Bare `FIRE HOLD` / `FIRE FREE` and the hold rule parse apart** (doc 21
/// cut G.12a): the bare words are the gate's two unconditional replies, the
/// same on a repeat; `FIRE cannon HOLD WHEN …` is still a hold rule (its
/// `Queued:` readback, never the gate's line); and `WEAPONSTATUS` and `MIND
/// RULES` say the gate only while the held frame says weapons hold.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn g12a_bare_fire_hold_and_a_hold_rule_parse_apart() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    assert_eq!(
        cockpit::WEAPONS_HOLD,
        "Weapons hold: your rules stand but fire only when you say."
    );
    assert_eq!(cockpit::WEAPONS_FREE, "Weapons free.");
    assert_eq!(rig.say(0, "fire hold"), cockpit::WEAPONS_HOLD);
    assert_eq!(rig.say(0, "FIRE HOLD"), cockpit::WEAPONS_HOLD, "idempotent");
    assert_eq!(rig.say(0, "FIRE FREE"), cockpit::WEAPONS_FREE);
    assert_eq!(
        rig.say(0, "fire  free "),
        cockpit::WEAPONS_FREE,
        "idempotent"
    );
    let said = rig.say(0, "FIRE cannon HOLD WHEN rounds>0");
    assert!(said.starts_with("Queued:"), "a hold rule: {said}");
    assert!(
        !said.contains(cockpit::WEAPONS_HOLD) && !said.contains(cockpit::WEAPONS_FREE),
        "{said}"
    );

    // The listings: nothing at weapons free, one line at weapons hold.
    let mut frame = rig.held_frame(0);
    frame.notices.clear();
    assert!(frame.weapons_free, "every seat starts weapons free");
    rig.push_frame(0, frame.clone());
    assert!(
        !rig.say(0, "weaponstatus")
            .contains(cockpit::WEAPONS_HOLD_LINE)
    );
    assert!(
        !rig.say(0, "mind rules")
            .contains(cockpit::WEAPONS_HOLD_LINE)
    );
    frame.weapons_free = false;
    rig.push_frame(0, frame);
    let said = rig.say(0, "weaponstatus");
    assert!(said.contains(cockpit::WEAPONS_HOLD_LINE), "{said}");
    let said = rig.say(0, "mind rules");
    assert!(said.contains(cockpit::WEAPONS_HOLD_LINE), "{said}");
}

/// **`FIRE HOLD` lands at a pulse and the frame says so** (doc 21 cut G.12a;
/// G.12's acceptance, "`f` flips `weapons_free` in the frame"): the arena's
/// own next frame carries `weapons_free: false`, and `FIRE FREE` puts it
/// back; the keyframe publishes the three new fields.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn g12a_fire_hold_flips_weapons_free_on_the_arenas_frame() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    assert!(rig.held_frame(0).weapons_free, "free at the start (Q-21-9)");
    assert_eq!(rig.say(0, "fire hold"), cockpit::WEAPONS_HOLD);
    m1b4_until(&mut rig, "the arena holds the gate", |rig| {
        rig.frame(0).is_some_and(|frame| !frame.weapons_free)
    })
    .await;
    let payload = cockpit::status_frame(fixture::MATCH, &rig.held_frame(0));
    assert_eq!(payload["weapons_free"], false, "{payload}");
    assert_eq!(payload["max_speed_kph"], 54, "{payload}");
    assert_eq!(payload["reverse_kph"], 27, "{payload}");
    m1b4_next_second(&mut rig).await;
    assert_eq!(rig.say(0, "fire free"), cockpit::WEAPONS_FREE);
    m1b4_until(&mut rig, "the arena frees the gate", |rig| {
        rig.frame(0).is_some_and(|frame| frame.weapons_free)
    })
    .await;
}

/// **The two sentences render from their fields** (17b cut M1b.4; D-UM-14):
/// `notice_line` over a slot shot with and without a contact (the first
/// letter upper-cased on `cannon` and `machine_gun`), every `HeldWhy` with
/// the range's three shapes, and the `unlawful` refusal with its floor; the
/// keyframe's notice object per kind; and `notice_lines.json`'s five slot
/// cases ARE what `status_frame` publishes for them, each `line` the
/// line-mode sentence less its full stop.
#[test]
fn m1b4_the_sentences_render_from_their_fields() {
    use tacsim::arena::conditions::HeldWhy;
    use tacsim::arena::intents::Refusal;
    use tacsim::arena::perception::{Aspect, FrameNotice, Named, RefusalDetail};
    let mut frame = fixture::frame();
    let rows = cockpit::frame_contacts(&frame);
    let ba = rows
        .iter()
        .find(|row| row.designator.to_string() == "BA")
        .map(|row| Named {
            id: row.id,
            designator: row.designator,
        })
        .expect("the scene's BA");
    let held = |why: HeldWhy| FrameNotice::SlotHeld {
        slot: 1,
        weapon: "cannon".into(),
        why,
    };
    let fired = FrameNotice::SlotFired {
        slot: 1,
        weapon: "cannon".into(),
        contact: Some(ba),
        p_permille: 624,
        aspect: Aspect::Left,
    };
    let fired_blind = FrameNotice::SlotFired {
        slot: 2,
        weapon: "machine_gun".into(),
        contact: None,
        p_permille: 210,
        aspect: Aspect::Rear,
    };
    let odds = held(HeldWhy::Odds {
        permille: 540,
        floor: 600,
    });
    let aspect = held(HeldWhy::Aspect {
        contact: ba,
        aspect: Aspect::Front,
    });
    let unlawful = FrameNotice::Refused {
        order: "fire".into(),
        refusal: Refusal::Unlawful,
        weapon: Some("cannon".into()),
        detail: RefusalDetail::Floor { permille: 100 },
    };
    for (notice, said) in [
        (&fired, "Cannon fires at BA by S1 (62%, side)."),
        (&fired_blind, "Machine_gun fires by S2 (21%, rear)."),
        (&odds, "S1 held the cannon: 54% under your 60%."),
        (&aspect, "S1 held the cannon: BA shows its front."),
        (
            &held(HeldWhy::Range {
                contact: ba,
                range_m: 650,
                min_m: 200,
                max_m: 600,
            }),
            "S1 held the cannon: BA at 650 m, outside 200 to 600.",
        ),
        (
            &held(HeldWhy::Range {
                contact: ba,
                range_m: 650,
                min_m: 0,
                max_m: 600,
            }),
            "S1 held the cannon: BA at 650 m, outside under 600.",
        ),
        (
            &held(HeldWhy::Range {
                contact: ba,
                range_m: 150,
                min_m: 200,
                max_m: u32::MAX,
            }),
            "S1 held the cannon: BA at 150 m, outside past 200.",
        ),
        (
            &held(HeldWhy::Rounds {
                rounds: 7,
                keeps: 8,
            }),
            "S1 held the cannon: 7 rounds, keeps 8.",
        ),
        (&unlawful, M1B4_FLOOR),
    ] {
        assert_eq!(cockpit::notice_line(&frame, notice, &rows), said);
    }
    // The keyframe's objects, and the committed ring cases.
    let cases = [fired, fired_blind, odds, aspect, unlawful];
    frame.notices = cases.to_vec();
    let payload = cockpit::status_frame(fixture::MATCH, &frame);
    let published = payload["notices"].as_array().cloned().unwrap_or_default();
    assert_eq!(published.len(), cases.len());
    let keys = |object: &serde_json::Value| -> Vec<String> {
        object
            .as_object()
            .map(|map| map.keys().cloned().collect())
            .unwrap_or_default()
    };
    let common = [
        "cause",
        "contact",
        "designator",
        "detail",
        "friendly",
        "kind",
        "order",
        "refusal",
        "section",
        "spent",
        "text",
        "was",
        "weapon",
    ];
    let mut fired_keys: Vec<&str> = common.to_vec();
    fired_keys.extend(["aspect", "percent", "slot"]);
    fired_keys.sort_unstable();
    let mut held_keys: Vec<&str> = common.to_vec();
    held_keys.push("slot");
    held_keys.sort_unstable();
    assert_eq!(keys(&published[0]), fired_keys);
    assert_eq!(keys(&published[2]), held_keys);
    assert_eq!(keys(&published[4]), common.to_vec());
    for object in &published {
        let text = object.to_string();
        assert!(!text.contains("\"101\""), "no object number: {text}");
    }
    let committed: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../web/src/lib/fixtures/cockpit/notice_lines.json"
    ))
    .expect("notice_lines.json is an array");
    for object in &published {
        let row = committed
            .iter()
            .find(|row| row.get("notice") == Some(object))
            .unwrap_or_else(|| panic!("notice_lines.json has no row for {object}"));
        let text = object["text"].as_str().expect("a sentence");
        assert_eq!(
            row.get("line"),
            Some(&serde_json::json!(text.strip_suffix('.').unwrap_or(text))),
            "{object}"
        );
    }
}

/// **The floor, refused as typed and at the landing** (17b cut M1b.4; doc 17
/// §5, D-UM-17 as amended; the gate's item), through the real `FIRE` handler
/// on a running rig: `FIRE cannon WHEN odds >= 5` answers the floor sentence
/// from the cockpit and forwards nothing — the arena never says it again; the
/// same arming offered straight into the match's inbox is refused by the
/// arena's own landing in the SAME bytes, its keyframe notice carrying
/// `detail.floor_permille` 100; `odds >= 10` arms. A rig checked out at a
/// floor of 50 per mille takes `odds >= 5` at the cockpit AND at the arena;
/// and the cockpit reads the frame's captured floor and never the config — a
/// held frame saying 50 on a config of 100 takes it too.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m1b4_fire_when_under_the_floor_is_refused_unlawful_as_typed_and_at_the_landing() {
    use tacsim::arena::conditions::{ASPECTS_ANY, Arming, FireCondition, SlotTarget};
    use tacsim::arena::intents::{Intent, IntentKind};
    let mut rig = Rig::boot(2);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let pilot = rig.pilot(0);
    let floor_lines = |rig: &Rig, mark: usize| {
        m1a4_said_since(rig, pilot, mark)
            .into_iter()
            .filter(|line| line.contains("Compact floor"))
            .collect::<Vec<String>>()
    };

    // As typed: the cockpit's sentence, and nothing forwarded — had it been,
    // the arena's landing would say it a second time.
    let mark = rig.log.len();
    assert_eq!(rig.say(0, "FIRE cannon WHEN odds >= 5"), M1B4_FLOOR);
    let settle = rig.staged_now_ms;
    m1b4_until(&mut rig, "two arena seconds", |rig| {
        rig.staged_now_ms >= settle + 2_000
    })
    .await;
    assert_eq!(floor_lines(&rig, mark), vec![M1B4_FLOOR.to_owned()]);
    assert!(m1b4_armed(&rig, 0, "cannon").is_none());

    // The backstop: the same arming straight into the inbox.
    let tasks = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .expect("the registry");
    let match_id = rig.centre().match_id.expect("a running match");
    let tank = rig.tank(0);
    let mark = rig.log.len();
    let panels_before = rig.panels(tacsim::arena_bridge::FRAME_FEED).len();
    assert!(
        tasks
            .offer(
                match_id,
                tacsim::arena_task::ArenaMsg::Intent(Intent {
                    unit: tank,
                    channel: Channel::Fire,
                    epoch: rig.epoch(tank, Channel::Fire),
                    kind: IntentKind::Condition(Arming::Arm(FireCondition {
                        weapon: "cannon".into(),
                        target: SlotTarget::Lock,
                        min_permille: 50,
                        aspects: ASPECTS_ANY,
                        min_rounds_left: 0,
                        min_range_m: 0,
                        max_range_m: u32::MAX,
                    })),
                    seq: 9_100_001,
                }),
            )
            .is_ok(),
        "the inbox took the arming"
    );
    m1b4_until(&mut rig, "the arena's refusal", |rig| {
        !floor_lines(rig, mark).is_empty()
    })
    .await;
    assert_eq!(
        floor_lines(&rig, mark),
        vec![M1B4_FLOOR.to_owned()],
        "the arena's landing says the cockpit's bytes"
    );
    let notice = rig
        .panels(tacsim::arena_bridge::FRAME_FEED)
        .into_iter()
        .skip(panels_before)
        .flat_map(|(_, data)| data["notices"].as_array().cloned().unwrap_or_default())
        .find(|notice| notice["refusal"] == "unlawful")
        .expect("the keyframe carries the refusal");
    assert_eq!(
        notice["detail"],
        serde_json::json!({ "floor_permille": 100 })
    );
    assert_eq!(notice["order"], "fire");
    assert_eq!(notice["weapon"], "cannon");
    assert!(m1b4_armed(&rig, 0, "cannon").is_none());

    // At the floor: armed.
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 10"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 10%"
    );
    m1b4_until(&mut rig, "the cannon armed", |rig| {
        m1b4_armed(rig, 0, "cannon").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "cannon"), Some((1, 100)));

    // The frame's captured floor, never the config: a held frame at 50 on a
    // rig whose `[tacsim.minds]` says 100 takes 5 at the cockpit.
    let mut low = rig.held_frame(0);
    low.notices.clear();
    low.odds_floor_permille = 50;
    rig.push_frame(0, low);
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 5"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 5%"
    );

    // A match checked out at 50: the cockpit and the arena both take 5.
    let mut cfg = config();
    cfg.minds.odds_floor_permille = 50;
    let mut rig = Rig::boot_with(2, cfg);
    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    assert_eq!(rig.held_frame(0).odds_floor_permille, 50);
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 5"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 5%"
    );
    assert_eq!(
        rig.say(0, "fire mg when odds >= 4"),
        "FIRE machine_gun refused. The Compact floor is 5%; a condition may not ask for less."
    );
    m1b4_until(&mut rig, "the cannon armed at 50", |rig| {
        m1b4_armed(rig, 0, "cannon").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "cannon"), Some((1, 50)));
}

/// **The arming through the composition** (17b cut M1b.4): on a pod fitted
/// with the authored heavy (three guns) the gate's rule forwards one
/// `Condition` and replies verbatim; the next frame's `armed.slot` is 1, a
/// second arming on the machine gun lands as 2, and a re-arm of an armed gun
/// replaces it in place. The existing `FIRE` lines are untouched; the outside
/// rule is the one sentence; `FIRE cannon OFF` replies and the frame's
/// `armed` goes `null`.
///
/// **17b cut M2.2, the lead's ruling (c)**: the cockpit's count pre-check is
/// gone and the arena's phase-4 `NoSlot` is the one wall — two armings typed
/// inside one pulse are both accepted by the cockpit, and a `FIRE mg OFF`
/// with an arming typed straight after it, inside one pulse, against a mirror
/// that still shows the machine gun armed, is accepted too.
///
/// **17b cut M3.2** (D-UM-16, the plan's F5): the heavy's command core
/// (Class III) holds SIX slots, so its third gun is ACCEPTED — `Arming
/// missiles: …` and the missiles in `S3` on `WEAPONSTATUS`, with no count
/// refusal — where M1b.4 to M3.1 pinned the two-line count transcript (the
/// acceptance, then the arena's `FIRE missiles refused. Every slot is armed;
/// FIRE <weapon> OFF frees one.`). That transcript, the race on a full table
/// and the freed slot moved to the arena, on a heavy whose core is degraded to
/// Class I (two slots): `tests/conditions.rs`'
/// `m3_the_count_is_told_in_two_lines_on_a_degraded_heavy`. Here the race's
/// two armings both land (the machine gun back in S2, the missiles re-armed in
/// place in S3), and the arming typed straight after `FIRE mg OFF` re-arms the
/// missiles in place in S3 while S2 is freed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m1b4_fire_when_arms_slots_in_the_arenas_numbers_and_the_count_is_one_sentence() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim unit heavy_mk1");
    assert!(said.contains("will field heavy_mk1"), "{said}");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);

    // Before the first frame an arming has no captured floor or weapon rows.
    // Native-rule commands wait for that frame; immediate FIRE keeps its own
    // no-lock answer. No rule can be queued from an absent projection.
    assert!(rig.frame(0).is_none(), "nothing has been pumped yet");
    let dark = format!("FIRE cannon refused. {}", cockpit::NO_LOCK);
    assert_eq!(rig.say(0, "fire cannon"), dark);
    assert_eq!(
        rig.say(0, "FIRE cannon WHEN odds >= 60"),
        cockpit::NO_FRAME_YET
    );
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 5"),
        cockpit::NO_FRAME_YET
    );
    assert_eq!(
        rig.say(0, "fire ghost when odds >= 60"),
        cockpit::NO_FRAME_YET
    );
    assert_eq!(
        rig.say(0, "fire all when odds >= 60"),
        cockpit::NO_FRAME_YET,
        "ALL is a native-rule expansion, which also needs a captured frame"
    );

    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let pilot = rig.pilot(0);
    let guns: Vec<String> = rig
        .held_frame(0)
        .weapons
        .iter()
        .map(|gun| gun.id.to_string())
        .collect();
    assert_eq!(guns, ["cannon", "machine_gun", "missiles"]);
    let count = "Every rule slot is occupied; MIND RULE <slot> OFF frees one.";

    // Every existing `FIRE` line, byte for byte (no lock here).
    let no_lock = format!("FIRE cannon refused. {}", cockpit::NO_LOCK);
    assert_eq!(rig.say(0, "fire cannon"), no_lock);
    assert_eq!(rig.say(0, "fire"), no_lock, "the bare FIRE is the cannon");
    assert_eq!(
        rig.say(0, "fire mg"),
        format!("FIRE machine_gun refused. {}", cockpit::NO_LOCK)
    );
    // Doc 21 cut G.12a: bare `FIRE ALL` is the volley now, refused for want
    // of a lock in `FIRE`'s own words (it was an unknown weapon `all`).
    assert_eq!(
        rig.say(0, "fire all"),
        format!("FIRE ALL refused. {}", cockpit::NO_LOCK)
    );
    assert_eq!(
        rig.say(0, "fire m"),
        "`m` could be machine_gun or missiles; say more."
    );

    // The gate's rule.
    assert_eq!(
        rig.say(0, "FIRE cannon WHEN odds >= 60 AND aspect = side,rear"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 60% and the target shows its side or\nrear"
    );
    m1b4_until(&mut rig, "the cannon in S1", |rig| {
        m1b4_armed(rig, 0, "cannon").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "cannon"), Some((1, 600)));
    let said = rig.say(0, "weaponstatus");
    assert!(
        said.contains("S1: cannon fires at your identified lock"),
        "{said}"
    );
    assert!(said.contains("the target shows its side or rear"), "{said}");
    // The second gun takes S2.
    assert_eq!(
        rig.say(0, "fire mg when odds >= 30"),
        "Queued: machine_gun fires at your identified lock when its chance to hit is at least 30%"
    );
    m1b4_until(&mut rig, "the machine gun in S2", |rig| {
        m1b4_armed(rig, 0, "machine_gun").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));
    // 17b cut M3.2 (D-UM-16): the command core holds six, so the third is
    // ACCEPTED and lands in S3 — one line, and no count refusal (the two-line
    // transcript lives on the degraded heavy in `tests/conditions.rs`).
    m1b4_next_second(&mut rig).await;
    let mark = rig.log.len();
    let accepted =
        "Queued: missiles fires at your identified lock when its chance to hit is at least 40%";
    assert_eq!(rig.say(0, "fire missiles when odds >= 40"), accepted);
    let full = format!("FIRE missiles refused. {count}");
    m1b4_until(&mut rig, "the missiles in S3", |rig| {
        m1b4_armed(rig, 0, "missiles").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "missiles"), Some((3, 400)));
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));
    let said = rig.say(0, "weaponstatus");
    assert!(
        said.contains(
            "S3: missiles fires at your identified lock when its chance to hit is at least 40%"
        ),
        "the full S3 rule follows the weapon rows: {said}"
    );
    let heard = m1a4_said_since(&rig, pilot, mark);
    assert_eq!(
        heard
            .iter()
            .filter(|said| said.contains("missiles refused"))
            .cloned()
            .collect::<Vec<String>>(),
        Vec::<String>::new(),
        "no count refusal: {heard:#?}"
    );
    assert!(!heard.contains(&full));
    // A re-arm is not a new slot.
    m1b4_next_second(&mut rig).await;
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 70"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 70%"
    );
    m1b4_until(&mut rig, "the cannon re-armed in place", |rig| {
        m1b4_armed(rig, 0, "cannon") == Some((1, 700))
    })
    .await;
    // The outside rule: one sentence, nothing forwarded.
    assert_eq!(
        rig.say(0, "FIRE cannon WHEN hit within 5 s"),
        "FIRE cannon refused. That condition needs a thinking mind; a pilot's mind is dormant."
    );

    m1b4_next_second(&mut rig).await;
    // The race: S2 freed, and two armings typed inside one pulse past a
    // mirror that has not moved. Both pass the cockpit (ruling (c)); on the
    // command core's six slots (17b cut M3.2) the arena gives the machine gun
    // S2 again and re-arms the missiles in place in S3 at their NEW figure
    // (45%, so the second arming of the pulse is proved to land) — no count
    // refusal. (The race on a FULL table, refused, is
    // `m3_the_count_is_two_lines_through_the_verb_on_a_two_slot_heavy`'s.)
    assert_eq!(
        rig.say(0, "fire mg off"),
        "Cleared: machine_gun fires only when you say."
    );
    m1b4_until(&mut rig, "S2 free", |rig| {
        m1b4_armed(rig, 0, "machine_gun").is_none()
    })
    .await;
    m1b4_next_second(&mut rig).await;
    let mark = rig.log.len();
    assert_eq!(
        rig.say(0, "fire mg when odds >= 30"),
        "Queued: machine_gun fires at your identified lock when its chance to hit is at least 30%"
    );
    assert_eq!(
        rig.say(0, "fire missiles when odds >= 45"),
        "Queued: missiles fires at your identified lock when its chance to hit is at least 45%"
    );
    m1b4_until(
        &mut rig,
        "the machine gun in S2 again, the missiles re-armed",
        |rig| {
            m1b4_armed(rig, 0, "machine_gun").is_some()
                && m1b4_armed(rig, 0, "missiles") == Some((3, 450))
        },
    )
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));
    assert_eq!(m1b4_armed(&rig, 0, "missiles"), Some((3, 450)), "in place");
    assert!(
        !m1a4_said_since(&rig, pilot, mark).contains(&full),
        "no count refusal"
    );

    // Off: the reply, and the frame's `null`.
    m1b4_next_second(&mut rig).await;
    assert_eq!(
        rig.say(0, "FIRE cannon OFF"),
        "Cleared: cannon fires only when you say."
    );
    m1b4_until(&mut rig, "the cannon cleared", |rig| {
        m1b4_armed(rig, 0, "cannon").is_none()
    })
    .await;
    let payload = cockpit::status_frame(fixture::MATCH, &rig.held_frame(0));
    assert_eq!(payload["weapons"][0]["id"], "cannon");
    assert_eq!(payload["weapons"][0]["armed"], serde_json::Value::Null);
    assert_eq!(
        payload["weapons"][1]["armed"]["slot"], 2,
        "S2 kept its number"
    );

    // 17b cut M2.2, ruling (c): the cannon back in S1, then `FIRE mg OFF`
    // and the missiles typed straight after it, inside one pulse, against a
    // mirror that still shows the machine gun armed — both accepted; the
    // arena frees S2 and (17b cut M3.2, six slots) re-arms the missiles in
    // place in S3. (The arming that TAKES the freed slot on a full table is
    // `tests/conditions.rs`' degraded heavy's.)
    m1b4_next_second(&mut rig).await;
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 60"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 60%"
    );
    m1b4_until(&mut rig, "the cannon back in S1", |rig| {
        m1b4_armed(rig, 0, "cannon").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "cannon"), Some((1, 600)));
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));
    m1b4_next_second(&mut rig).await;
    let mark = rig.log.len();
    assert_eq!(
        rig.say(0, "fire mg off"),
        "Cleared: machine_gun fires only when you say."
    );
    // Nothing pumped since: the mirror still shows the machine gun in S2,
    // which is what M1b.4's pre-check counted.
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));
    assert_eq!(
        rig.say(0, "fire missiles when odds >= 20"),
        "Queued: missiles fires at your identified lock when its chance to hit is at least 20%"
    );
    m1b4_until(&mut rig, "the missiles re-armed in S3, S2 freed", |rig| {
        m1b4_armed(rig, 0, "missiles") == Some((3, 200))
            && m1b4_armed(rig, 0, "machine_gun").is_none()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "missiles"), Some((3, 200)));
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), None);
    assert!(
        !m1a4_said_since(&rig, pilot, mark).contains(&full),
        "no count refusal"
    );
}

/// **17b cut M3.2: the count is two lines THROUGH THE VERB on a two-slot
/// heavy** (17b cut M2.2's ruling (c); D-UM-16; the plan's F5). The shipped
/// heavy's command core holds six since 17b cut M3.2, so
/// [`m1b4_fire_when_arms_slots_in_the_arenas_numbers_and_the_count_is_one_sentence`]
/// can no longer fill its table; this match is checked out under a
/// `[tacsim.minds].classes` table whose Class II and III rows carry two slots
/// (the class table is the production lever that sizes the row,
/// `Arena::slot_capacity`), so the same heavy, flown by the same verbs over
/// the real composition, holds two. With the cannon and the machine gun
/// armed, `FIRE missiles WHEN odds >= 40` is ACCEPTED by the cockpit — no
/// count is read off the mirror (ruling (c)) — and refused by the arena a
/// pulse later: every line the pilot hears about the missiles is exactly the
/// acceptance, then the arena's count, in that order. The race on a full
/// table (S2 freed, then the machine gun and the missiles typed inside one
/// pulse past an unmoved mirror): both accepted, the machine gun retakes S2
/// and the missiles are refused in the same words. The freed slot (the table
/// full again, then `FIRE mg OFF` and the missiles typed straight after it
/// against a mirror still showing the machine gun): both accepted, and the
/// missiles take the freed S2 with no count refusal.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m3_the_count_is_two_lines_through_the_verb_on_a_two_slot_heavy() {
    use tacsim::arena::core::{ClassTable, CoreClass};
    let mut cfg = config();
    cfg.minds.classes = serde_json::from_value(serde_json::json!({
        "II": { "slots": 2 },
        "III": { "slots": 2 }
    }))
    .expect("a two-slot table");
    assert_eq!(cfg.minds.classes.row(CoreClass::III).slots, 2);
    assert_ne!(cfg.minds.classes, ClassTable::STOCK);
    let mut rig = Rig::boot_with(2, cfg);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim unit heavy_mk1");
    assert!(said.contains("will field heavy_mk1"), "{said}");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running);
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let pilot = rig.pilot(0);
    let count = "Every rule slot is occupied; MIND RULE <slot> OFF frees one.";
    let full = format!("FIRE missiles refused. {count}");

    // The table filled: the cannon in S1, the machine gun in S2.
    assert_eq!(
        rig.say(0, "fire cannon when odds >= 60"),
        "Queued: cannon fires at your identified lock when its chance to hit is at least 60%"
    );
    m1b4_until(&mut rig, "the cannon in S1", |rig| {
        m1b4_armed(rig, 0, "cannon").is_some()
    })
    .await;
    assert_eq!(
        rig.say(0, "fire mg when odds >= 30"),
        "Queued: machine_gun fires at your identified lock when its chance to hit is at least 30%"
    );
    m1b4_until(&mut rig, "the machine gun in S2", |rig| {
        m1b4_armed(rig, 0, "machine_gun").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "cannon"), Some((1, 600)));
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));

    // The third: the cockpit's acceptance, then the arena's count.
    m1b4_next_second(&mut rig).await;
    let mark = rig.log.len();
    let accepted =
        "Queued: missiles fires at your identified lock when its chance to hit is at least 40%";
    assert_eq!(rig.say(0, "fire missiles when odds >= 40"), accepted);
    m1b4_until(&mut rig, "the arena's count on a full table", |rig| {
        m1a4_said_since(rig, pilot, mark).contains(&full)
    })
    .await;
    let heard = m1a4_said_since(&rig, pilot, mark);
    assert_eq!(
        heard
            .iter()
            .filter(|said| said.contains("missiles"))
            .cloned()
            .collect::<Vec<String>>(),
        vec![accepted.to_owned(), full.clone()],
        "the acceptance, then the arena's refusal: two lines and no more"
    );
    assert_eq!(m1b4_armed(&rig, 0, "missiles"), None);
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));

    // The race on a full table: S2 freed, then two armings in one pulse.
    m1b4_next_second(&mut rig).await;
    assert_eq!(
        rig.say(0, "fire mg off"),
        "Cleared: machine_gun fires only when you say."
    );
    m1b4_until(&mut rig, "S2 free", |rig| {
        m1b4_armed(rig, 0, "machine_gun").is_none()
    })
    .await;
    m1b4_next_second(&mut rig).await;
    let mark = rig.log.len();
    assert_eq!(
        rig.say(0, "fire mg when odds >= 30"),
        "Queued: machine_gun fires at your identified lock when its chance to hit is at least 30%"
    );
    assert_eq!(rig.say(0, "fire missiles when odds >= 40"), accepted);
    m1b4_until(&mut rig, "the arena's count in the race", |rig| {
        m1a4_said_since(rig, pilot, mark).contains(&full)
    })
    .await;
    m1b4_until(&mut rig, "the machine gun in S2 again", |rig| {
        m1b4_armed(rig, 0, "machine_gun").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));
    assert_eq!(m1b4_armed(&rig, 0, "missiles"), None);

    // The freed slot: `FIRE mg OFF` and the missiles in one pulse, past a
    // mirror that still shows the machine gun in S2.
    m1b4_next_second(&mut rig).await;
    let mark = rig.log.len();
    assert_eq!(
        rig.say(0, "fire mg off"),
        "Cleared: machine_gun fires only when you say."
    );
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), Some((2, 300)));
    assert_eq!(
        rig.say(0, "fire missiles when odds >= 20"),
        "Queued: missiles fires at your identified lock when its chance to hit is at least 20%"
    );
    m1b4_until(&mut rig, "the missiles in the freed S2", |rig| {
        m1b4_armed(rig, 0, "missiles").is_some()
    })
    .await;
    assert_eq!(m1b4_armed(&rig, 0, "missiles"), Some((2, 200)));
    assert_eq!(m1b4_armed(&rig, 0, "machine_gun"), None);
    assert_eq!(m1b4_armed(&rig, 0, "cannon"), Some((1, 600)));
    assert!(
        !m1a4_said_since(&rig, pilot, mark).contains(&full),
        "no count refusal"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M1b.5 — `@mind/live` and the fill controller (doc 17 §11, §22's M1b;
// D-UM-17 as amended, D-UM-43, D-UM-44)
// ---------------------------------------------------------------------------

/// [`af3_config`] with the AI fill on (17b cut M1b.5; from 17b cut M4.3 the
/// fill is a stock mind on Line with no knob to set): every unseated hull a
/// mind, and a snapshot an arena second.
fn m1b5_mind_config() -> TacsimConfig {
    let mut cfg = af3_config();
    cfg.match_.ai_fill = true;
    cfg
}

/// The units the AI fill fielded in the running match — every unit no pilot
/// was checked out on — in id order.
fn m1b5_fill_units(rig: &Rig) -> Vec<ObjId> {
    let record = rig.record();
    let mut fill: Vec<ObjId> = record
        .teams
        .iter()
        .flat_map(|team| {
            team.units
                .iter()
                .copied()
                .filter(|unit| !team.pilots.iter().any(|(_, seated)| seated == unit))
                .collect::<Vec<ObjId>>()
        })
        .collect();
    fill.sort_unstable();
    fill
}

/// The registry's controller of `unit`, as `@mind/live` reads it.
fn m1b5_controller(rig: &Rig, unit: ObjId) -> Option<tacsim::arena::state::Controller> {
    let match_id = rig.centre().match_id.expect("a running match");
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .and_then(|mirror| mirror.controllers.get(&unit).copied())
}

/// The head line of a read-back.
fn m1b5_head(said: &str) -> String {
    said.lines().next().unwrap_or_default().to_owned()
}

/// **`@mind/live` reads, sets and refuses by name** (17b cut M1b.5; doc 17 §11,
/// §22's M1b; D-UM-17 as amended, D-UM-43, D-UM-44), through the composition
/// on a running match whose AI fill is the lab's `mind`, as staff:
///
/// * the bare read-back of a fill hull opens `(as checked out)` on Line with
///   the five sections (the band in per cent, the `[M2]` marks, the fire
///   discipline live on an armed hull);
/// * `=Sniper` replies the offer and reads back `(sent at N ms, not yet on
///   the tape)`; once the chunk holding the arena's `mind_card` line closes —
///   a line naming `Sniper` and carrying the whole card — the head reads `(on
///   the tape at N ms)`;
/// * `band_pct 72` is the step's sentence, `min_odds 5` the floor's sentence
///   naming the MATCH's floor (10 per cent), `stance fast` the choices,
///   `foo 1` and `=berserker` the unknown sentence; a bare `@mind` or another
///   switch, a slider with no value (`=min_odds`) and a preset with one
///   (`=sniper 5`) the usage;
/// * a pilot's hull is `not flown by a mind`; after its pilot drops it is its
///   sealed mind's (17b cut M4.3; the reflex machine's before it), which
///   `=sniper` sets, and the board reads `flown by its mind` for that slot
///   (`flown by doctrine` until 17b cut M5.3);
/// * a pilot is refused by the gate;
/// * after a restart of the server half resumes the match, the head is
///   `not read since the resume`, `min_odds 5` still meets the floor's
///   sentence (the resumed mirror's floor is the decoded arena's), and a
///   preset is offered — its read-back's fire discipline live, the resumed
///   mirror's armed set being the decoded arena's — and lands again;
///
/// and on a second rig booted at the default floor of 100 per mille, checked
/// out with its store's `[tacsim.minds]` set to 50 and then set back to 100, `min_odds 5` passes the verb's
/// floor pre-check (the slider's own range answers it) and `min_odds 10` is
/// sent, before and after a restart of the server half: the captured floor,
/// never the live config.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m1b5_mind_set_reads_sets_and_refuses_by_name() {
    let mut rig = Rig::boot_with(2, m1b5_mind_config());
    rig.fly_with_ai(1, 1, 2).await;
    let fill = m1b5_fill_units(&rig);
    assert_eq!(fill.len(), 2, "one fill hull a side: {fill:?}");
    let mind = *fill.first().expect("a fill hull");
    let name = mush_core::output::display_name(rig.tw.world(), mind);
    let id = mind.0;

    // The read-back, as checked out.
    let said = rig.staff(&format!("@mind/live #{id}"));
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some(format!("Mind card on #{id} ({name}): Line (as checked out)").as_str()),
        "{said}"
    );
    assert_eq!(lines.len(), 6, "{said}");
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("  posture          stance hold [M2]  band 70%")),
        "{said}"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("  fire discipline  min_odds 10%  aspect any")),
        "{said}"
    );

    // A preset: sent, then on the tape.
    let said = rig.staff(&format!("@mind/live #{id}=sniper"));
    let mut lines = said.lines();
    assert_eq!(
        lines.next(),
        Some(
            format!("Mind card on #{id}: Sniper sent; the arena takes it at its next pulse.")
                .as_str()
        ),
        "{said}"
    );
    let head = lines.next().unwrap_or_default();
    assert!(
        head.starts_with(&format!("Mind card on #{id} ({name}): Sniper (sent at "))
            && head.ends_with(" ms, not yet on the tape)"),
        "{said}"
    );
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, name, .. }
                    if *unit == mind && name == "Sniper"
            )
        })
        .await;
    assert!(landed, "the arena's mind_card line names Sniper");
    let line = rig
        .tape
        .iter()
        .find(|event| matches!(&event.kind, tacsim::arena::ledger::EventKind::MindCard { unit, .. } if *unit == mind))
        .cloned()
        .expect("the line");
    let tacsim::arena::ledger::EventKind::MindCard {
        stock: Some(card),
        changed,
        ..
    } = &line.kind
    else {
        panic!("a mind_card line");
    };
    assert_eq!(
        card,
        &tacsim::arena::preset("sniper", &config().arena.reflex).expect("sniper"),
        "the whole card"
    );
    assert_eq!(
        changed
            .iter()
            .map(smol_str::SmolStr::as_str)
            .collect::<Vec<&str>>(),
        vec!["band_pct", "min_odds", "reserve_pct"]
    );
    assert_eq!(
        m1b5_head(&rig.staff(&format!("@mind/live #{id}"))),
        format!(
            "Mind card on #{id} ({name}): Sniper (on the tape at {} ms)",
            line.t_ms
        )
    );

    // Refused by name.
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=band_pct 72")),
        "@mind/live: band_pct is 30 to 100 in steps of 5 (it is 72)."
    );
    // 17b cut M2.2, the lead's ruling (b): the doctrine pair, in the same wrap
    // (the card is Sniper here, whose pair is Line's: break_off 35, fight_home
    // 17.5).
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=break_off_pct 10")),
        "@mind/live: break_off_pct would put fight_home_pct 17.5 above break_off_pct 10; \
         fight_home_pct is at most break_off_pct (it is 10)."
    );
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=fight_home_pct 40")),
        "@mind/live: fight_home_pct would put fight_home_pct 40 above break_off_pct 35; \
         fight_home_pct is at most break_off_pct (it is 40)."
    );
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=min_odds 5")),
        "@mind/live: min_odds is 10% to 95% in steps of 5; 10% is the Compact floor (it is 5)."
    );
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=stance fast")),
        "@mind/live: stance is hold, close, orbit, hunt or screen (it is fast)."
    );
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=foo 1")),
        "@mind/live: no slider or preset named \"foo\"."
    );
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=berserker")),
        "@mind/live: no slider or preset named \"berserker\"."
    );
    let usage = "Usage: @mind/live <unit>[=<preset> | =<slider> <value>]";
    // M7 leads the workshop help with free teaching pages. Private authoring
    // must remain discoverable; live card reads still require staff-only /live,
    // even when the bare dispatcher's argument is a dbref.
    let author_help = rig.staff(&format!("@mind #{id}"));
    assert_eq!(
        author_help.lines().next(),
        Some(
            "@mind/doctrine [Line|Sentry|Sniper|Skirmisher|Hunter|Rearguard] (free teaching pages)"
        ),
        "bare @mind must retain free teaching discovery"
    );
    assert!(
        author_help
            .lines()
            .any(|line| line == "@mind/new <name> [FROM Standard|<owned mind>]"),
        "bare @mind must retain private authoring discovery"
    );
    assert!(
        author_help.contains(usage.trim_start_matches("Usage: ")),
        "{author_help}"
    );
    // 17g C2a: the author's commerce commands are discoverable too; 17g C2c
    // adds `@mind/publish`.
    for line in [
        "@mind/rate <name>@<rev>; @mind/file <name>@<rev>[=<charter name>] (Registry rating, then filing)",
        "@mind/update <filed> <name>@<rev>; @mind/withdraw <filed>; @mind/attest <filed> [org=<org>]",
        "@mind/fork <licence>=<name> (from an Open licence; @mind/localcopy copies your own drafts)",
        "@mind/publish <filed> form=sealed|open|open-resale [royalty=<per mille>] [transfer=no] (at a counter)",
    ] {
        assert!(
            author_help.lines().any(|help| help == line),
            "{line} in {author_help}"
        );
    }
    assert_eq!(rig.staff("@mind/live"), usage);
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=min_odds 60 70")),
        usage
    );
    // A slider with no value, and a preset with one: the usage, never the
    // unknown sentence (each IS a slider or a preset).
    assert_eq!(rig.staff(&format!("@mind/live #{id}=min_odds")), usage);
    assert_eq!(rig.staff(&format!("@mind/live #{id}=sniper 5")), usage);
    // A slider set: sent, and the scratch card is the tape's Sniper edited.
    let said = rig.staff(&format!("@mind/live #{id}=MIN_ODDS 70"));
    assert!(
        said.starts_with(&format!(
            "Mind card on #{id}: min_odds 70 sent; the arena takes it at its next pulse.\nMind \
             card on #{id} ({name}): Sniper, edited (min_odds 70) (sent at "
        )),
        "{said}"
    );

    // A pilot's hull, and the same hull once its pilot drops.
    let tank = rig.tank(0);
    assert_eq!(
        m1b5_controller(&rig, tank),
        Some(tacsim::arena::state::Controller::Pilot)
    );
    assert_eq!(
        rig.staff(&format!("@mind/live #{}", tank.0)),
        format!("@mind/live: #{} is not flown by a mind.", tank.0)
    );
    rig.fire_conn(rig.pilot(0), mush_core::ConnTransition::Disconnected);
    // 17b cut M4.3: a released seat is a mind's — the one sealed on the hull
    // at check-out, whose card `@mind/live` now reads and sets.
    assert_eq!(
        m1b5_controller(&rig, tank),
        Some(tacsim::arena::state::Controller::Mind),
        "a released seat is a mind's"
    );
    let said = rig.staff(&format!("@mind/live #{}=sniper", tank.0));
    assert!(
        said.starts_with(&format!(
            "Mind card on #{}: Sniper sent; the arena takes it at its next pulse.",
            tank.0
        )),
        "{said}"
    );
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, name, .. }
                    if *unit == tank && name == "Sniper"
            )
        })
        .await;
    assert!(
        landed,
        "the arena applied the preset to the released seat's mind"
    );
    assert!(
        y3_red_row(&rig).contains(", flown by its mind"),
        "{}",
        y3_red_row(&rig)
    );
    // Not staff.
    assert_eq!(
        rig.say(1, &format!("@mind/live #{id}=sniper")),
        "Permission denied."
    );

    // A restart of the server half: not read since the resume.
    let _ = rig.af3_snapshot_at_or_after(0).await;
    let match_id = rig.centre().match_id.expect("a running match");
    rig.reboot_mid_match().await;
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .is_some_and(|registry| registry.tasks().is_live(match_id)),
        "it resumed"
    );
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}")),
        format!(
            "Mind card on #{id} ({name}): not read since the resume \u{2014} the tape's last \
             mind_card line names it; a preset resets it."
        )
    );
    // The resumed mirror's floor is the decoded arena's own (100 here): the
    // verb's floor pre-check still answers, naming 10 per cent.
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=min_odds 5")),
        "@mind/live: min_odds is 10% to 95% in steps of 5; 10% is the Compact floor (it is 5)."
    );
    let said = rig.staff(&format!("@mind/live #{id}=rearguard"));
    assert!(
        said.starts_with(&format!("Mind card on #{id}: Rearguard sent;")),
        "{said}"
    );
    // The resumed mirror's armed set is the decoded arena's: the fill hull's
    // fire discipline is live, not inert.
    assert!(
        said.lines()
            .any(|line| line.starts_with("  fire discipline  min_odds 50%  aspect any")),
        "{said}"
    );
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, name, .. }
                    if *unit == mind && name == "Rearguard"
            )
        })
        .await;
    assert!(landed, "the resumed arena takes a preset");

    // The captured floor, never the live config. The rig boots on the
    // default floor of 100 (what a restart reloads), the store is set to 50
    // for the check-out, which the arena captures, and set back to 100 after.
    let mut rig = Rig::boot_with(2, m1b5_mind_config());
    if let Some(store) = rig.tw.world_mut().module_state_mut::<tacsim::TacsimStore>() {
        store.replace_minds_config_for_tests(tacsim::config::MindsConfig {
            odds_floor_permille: 50,
            ..tacsim::config::MindsConfig::default()
        });
    }
    rig.fly_with_ai(1, 1, 2).await;
    let mind = *m1b5_fill_units(&rig).first().expect("a fill hull");
    let id = mind.0;
    if let Some(store) = rig.tw.world_mut().module_state_mut::<tacsim::TacsimStore>() {
        store.replace_minds_config_for_tests(tacsim::config::MindsConfig {
            odds_floor_permille: 100,
            ..tacsim::config::MindsConfig::default()
        });
    }
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=min_odds 5")),
        "@mind/live: min_odds is 10 to 95 in steps of 5 (it is 5).",
        "the floor pre-check passed at the match's 5%: the slider's range answers"
    );
    assert!(
        rig.staff(&format!("@mind/live #{id}=min_odds 10"))
            .starts_with(&format!("Mind card on #{id}: min_odds 10 sent;")),
    );
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, stock: Some(card), .. }
                    if *unit == mind && card.min_odds_permille == 100
            )
        })
        .await;
    assert!(landed, "min_odds 10 lands on the arena's row");
    // And across a restart: the resumed mirror takes the decoded arena's
    // captured 5 per cent, never the live block's 10 (the restart reloads the
    // boot's 100), so `min_odds 5` is again the slider's range and not the
    // floor.
    let _ = rig.af3_snapshot_at_or_after(0).await;
    let match_id = rig.centre().match_id.expect("a running match");
    rig.reboot_mid_match().await;
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .is_some_and(|registry| registry.tasks().is_live(match_id)),
        "it resumed"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .map(|store| store.config().minds.odds_floor_permille),
        Some(100),
        "the live block says 100 after the restart"
    );
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=min_odds 5")),
        "@mind/live: min_odds is 10 to 95 in steps of 5 (it is 5).",
        "the resumed match is held to its captured 5%, not the live 10%"
    );
}

/// **`@mind/live` refuses a preset that breaks the doctrine pair, by the
/// preset's name** (17b cut M2.2, the lead's ruling (b): the presets
/// pair-valid; doc 17 §11, D-UM-43, D-UM-44): on a doctrine the config admits
/// with `fight_home` 75 under `break_off` 80, `=rearguard` — `break_off_pct`
/// 70 under Line's `fight_home_pct` 75 — answers `Pair`'s sentence naming
/// `Rearguard` in the verb's wrap and sends nothing, while `=sniper`, which
/// leaves the pair alone, is sent. The arena's own refusal of the same card
/// is `tests/minds.rs`' `m2_the_arena_refuses_a_preset_that_breaks_the_pair_by_name`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m2_mind_set_refuses_a_preset_that_breaks_the_pair_by_name() {
    let mut cfg = m1b5_mind_config();
    cfg.arena.reflex.withdraw_permille = 800;
    cfg.arena.reflex.stand_min_permille = 750;
    let mut rig = Rig::boot_with(2, cfg);
    rig.fly_with_ai(1, 1, 2).await;
    let fill = m1b5_fill_units(&rig);
    let mind = *fill.first().expect("a fill hull");
    let id = mind.0;
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}=rearguard")),
        "@mind/live: Rearguard would put fight_home_pct 75 above break_off_pct 70; \
         fight_home_pct is at most break_off_pct."
    );
    let name = mush_core::output::display_name(rig.tw.world(), mind);
    assert_eq!(
        m1b5_head(&rig.staff(&format!("@mind/live #{id}"))),
        format!("Mind card on #{id} ({name}): Line (as checked out)"),
        "nothing was sent"
    );
    let said = rig.staff(&format!("@mind/live #{id}=sniper"));
    assert_eq!(
        said.lines().next(),
        Some(
            format!("Mind card on #{id}: Sniper sent; the arena takes it at its next pulse.")
                .as_str()
        ),
        "{said}"
    );
}

/// **The fill is a mind sealed at `fill_class`** (17b cut M4.3; the M4
/// row's (iii); doc 16 §6's AI fill): on the shipped config the fill hulls
/// are [`Controller::Mind`] at check-out and dealt into squads (17b cut L.2's
/// `is_machine`), the seated hulls the pilots', every record seal Class I on
/// `PRODUCTION_FILL_ENGINE` (a seated hull's too, so a release hands it to a
/// mind of the same class on the same engine), the mirror holds a card for
/// every hull, the board says what it said (`AI fill: each side fields 2;
/// machines take the unclaimed pods.`, and `flown by its mind` for a dropped
/// pilot's slot — `flown by doctrine` until 17b cut M5.3, which made the drop
/// clause one word for every sealed mind), and a fill hull takes a card.
///
/// [`Controller::Mind`]: tacsim::arena::state::Controller::Mind
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m4_3_the_fill_is_a_mind_sealed_at_fill_class() {
    use tacsim::arena::core::CoreClass;
    use tacsim::arena::state::Controller;
    let mut rig = Rig::boot_with(2, y1_config());
    rig.fly_with_ai(1, 1, 2).await;
    let fill = m1b5_fill_units(&rig);
    assert_eq!(fill.len(), 2, "{fill:?}");
    for unit in &fill {
        assert_eq!(
            m1b5_controller(&rig, *unit),
            Some(Controller::Mind),
            "#{}",
            unit.0
        );
    }
    for n in 0..2 {
        assert_eq!(
            m1b5_controller(&rig, rig.tank(n)),
            Some(Controller::Pilot),
            "pilot {n}'s hull"
        );
    }
    // Every seal of the record: Class I on the fill's engine, seated or not.
    let record = rig.record();
    assert!(!record.minds.is_empty());
    for (unit, sealed) in &record.minds {
        assert_eq!(sealed.seal.class, CoreClass::I, "#{}", unit.0);
        assert_eq!(
            sealed.seal.engine,
            tacsim::match_::PRODUCTION_FILL_ENGINE,
            "#{}",
            unit.0
        );
    }
    // The mirror holds a card for every hull (a released seat reads one).
    let match_id = rig.centre().match_id.expect("a running match");
    let carded = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .map(|mirror| mirror.cards.len())
        .unwrap_or_default();
    assert_eq!(carded, record.minds.len(), "a card for every hull");
    // Dealt into squads, like the reflex fill was.
    let squads = record.squads;
    for unit in &fill {
        assert!(
            squads.iter().any(|squad| squad.units.contains(unit)),
            "#{} in a squad: {squads:?}",
            unit.0
        );
    }
    // The board says what it said.
    let board = tacsim::match_::status_lines(rig.tw.world(), BoardReader::Staff).join("\n");
    assert!(
        board.contains("AI fill: each side fields 2; machines take the unclaimed pods."),
        "{board}"
    );
    rig.fire_conn(rig.pilot(0), mush_core::ConnTransition::Disconnected);
    let row = y3_red_row(&rig);
    assert!(row.contains(", flown by its mind"), "{row}");
    // A fill hull takes a card.
    let id = fill.first().map_or(0, |unit| unit.0);
    assert!(
        rig.staff(&format!("@mind/live #{id}"))
            .contains("Line (as checked out)"),
    );
}

// ---------------------------------------------------------------------------
// 17b cut M3.5: a re-seat is a seat activation inside the sortie (doc 17 §12)
// ---------------------------------------------------------------------------

/// Every unit's seal reference (`UnitState::seal`) in the running match's
/// latest snapshot in the rig's image, by unit id.
fn m3_snapshot_seals(rig: &Rig) -> BTreeMap<u64, serde_json::Value> {
    let match_id = rig.centre().match_id.expect("a running match");
    let key = match_::snapshot_key(match_id).to_string();
    let state: serde_json::Value = rig
        .image
        .get(tacsim::NS)
        .and_then(|docs| docs.get(key.as_str()))
        .and_then(|document| document.get("state"))
        .and_then(serde_json::Value::as_str)
        .and_then(|text| serde_json::from_str(text).ok())
        .expect("the snapshot's state");
    state
        .get("units")
        .and_then(serde_json::Value::as_array)
        .expect("the snapshot's units")
        .iter()
        .filter_map(|unit| {
            Some((
                unit.get("id").and_then(serde_json::Value::as_u64)?,
                unit.get("seal")?.clone(),
            ))
        })
        .collect()
}

/// **A reclaim re-seals nothing** (17b cut M3.5; doc 17 §12 "Release and
/// possess": *a re-seat is a seat activation inside the same sortie, never a
/// new seal*), over the real rig: a lone pilot of an AI-filled match drops
/// and `BOARD`s back — the world's `slot_reclaimed`, a `Live` control at a new
/// epoch over the hull the machine flew meanwhile. The record's
/// `MatchRecord::minds` is the check-out's, seal for seal, after the reclaim;
/// and the arena's own seal reference of every unit (`UnitState::seal`, read
/// off a snapshot taken after the reclaim) is the one a snapshot carried
/// before the drop — each the record's seal's reference.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_a_reclaim_re_seals_nothing() {
    let mut cfg = y1_config();
    cfg.arena.snapshot_ms = 1_000;
    let mut rig = Rig::boot_with(1, cfg);
    rig.fly_with_ai(1, 0, 1).await;
    let pilot = rig.pilot(0);
    let tank = rig.tank(0);
    let minds = rig.record().minds;
    assert!(minds.contains_key(&tank), "the pilot's hull is sealed");
    let _ = rig.af3_snapshot_at_or_after(0).await;
    let before = m3_snapshot_seals(&rig);
    assert_eq!(before.len(), minds.len(), "a seal reference per unit");
    for (unit, row) in &minds {
        let reference = serde_json::to_value(row.seal.seal_ref()).expect("a reference");
        assert_eq!(
            before.get(&u64::from(unit.0)),
            Some(&reference),
            "#{}: the arena's reference is the record's seal's",
            unit.0
        );
    }

    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    let now = rig.now;
    rig.tick_to(now + 5);
    let said = rig.say(0, "board");
    assert!(said.contains("is yours again where it stands"), "{said}");
    assert_eq!(rig.record().minds, minds, "the record's seals are unmoved");
    let after_ms = rig.af3_arena_ms();
    let _ = rig.af3_snapshot_at_or_after(after_ms + 1).await;
    assert_eq!(
        m3_snapshot_seals(&rig),
        before,
        "the arena's seal references are unmoved"
    );
    assert_eq!(rig.record().minds, minds);
}

/// **A released mind hull is its mind's again, on the world's mirror as in
/// the arena** (17b cut M3.5; doc 17 §12 "Release and possess"), over the
/// real rig with the lab's `mind` fill: a `Live` control on a fill hull the
/// stock mind flies (the possess) makes the mirror read the pilot's and
/// `@mind/live` refuse it as `not flown by a mind`; the `Released` that ends
/// the seat hands it back — the mirror reads `Controller::Mind` again
/// (M3.5's `MatchMirror::minded` rule, the set 17b cut M4.6a deleted: every
/// released hull reads its mind's since 17b cut M4.3), and `@mind/live` offers
/// a preset that the arena itself applies (a `mind_card` line naming it on the
/// tape: the arena, too, handed the hull back to its mind and not to the
/// reflex machine). The
/// pilot's own hull, which carries no mind row, still goes to the reflex
/// machine (`m1b5_mind_set_reads_sets_and_refuses_by_name`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_a_released_mind_hull_is_its_mind_s_again_on_the_mirror() {
    use tacsim::arena::intents::ControlState;
    use tacsim::arena::state::{Channel, Controller};
    let mut rig = Rig::boot_with(2, m1b5_mind_config());
    rig.fly_with_ai(1, 1, 2).await;
    let mind = *m1b5_fill_units(&rig).first().expect("a fill hull");
    let id = mind.0;
    assert_eq!(m1b5_controller(&rig, mind), Some(Controller::Mind));

    rig.with_ctx_mut(0, |ctx| {
        tacsim::arena_bridge::send_control(ctx, mind, Channel::Helm, ControlState::Live, None)
    })
    .expect("the possess is offered");
    assert_eq!(m1b5_controller(&rig, mind), Some(Controller::Pilot));
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}")),
        format!("@mind/live: #{id} is not flown by a mind.")
    );

    rig.with_ctx_mut(0, |ctx| {
        tacsim::arena_bridge::send_control(ctx, mind, Channel::Helm, ControlState::Released, None)
    })
    .expect("the release is offered");
    assert_eq!(
        m1b5_controller(&rig, mind),
        Some(Controller::Mind),
        "a released mind hull is its mind's"
    );
    let said = rig.staff(&format!("@mind/live #{id}=sniper"));
    assert!(
        said.starts_with(&format!(
            "Mind card on #{id}: Sniper sent; the arena takes it at its next pulse."
        )),
        "{said}"
    );
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, name, .. }
                    if *unit == mind && name == "Sniper"
            )
        })
        .await;
    assert!(landed, "the arena applied the card: the hull is its mind's");
}

// ---------------------------------------------------------------------------
// 17b cut M5.4: `MIND` and `MIND PROFILE` through the seat
// ---------------------------------------------------------------------------

/// **Two pilots flown, red's slot sealed on Sniper and Line** (17b cut M5.4):
/// red claims and chooses `SIM MIND Standard PROFILE Sniper, Line` in
/// Selection, blue claims, the match starts and both board; red's first frame
/// is held. Nobody types `STARTUP`: both tanks stand cold where the check-out
/// put them.
async fn m5_sniper_line() -> Rig {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Sniper, Line"),
        "Your slot will carry Standard Doctrine (Sniper, Line)."
    );
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    assert!(rig.frame_for(0).await, "the arena sends red its frame");
    rig
}

/// Pump until `done` holds of the rig, or fail saying `what` (17b cut M5.4) —
/// [`m1b4_until`] with room for the ten seconds a switch waits.
async fn m5_until(rig: &mut Rig, what: &str, done: impl Fn(&Rig) -> bool) {
    for _ in 0..4_000 {
        rig.pump();
        if done(rig) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let heard = rig.tw.seen_by(&rig.log, rig.pilot(0));
    let tail = heard
        .get(heard.len().saturating_sub(12)..)
        .unwrap_or_default();
    panic!("never: {what}; pilot 0 last heard {tail:#?}");
}

/// **The mirror's arena clock** (`MatchMirror::now_ms`), what the cockpit
/// reads a mind's time against.
fn m5_now_ms(rig: &Rig) -> i64 {
    let match_id = rig.centre().match_id.expect("a running match");
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .map(|mirror| mirror.now_ms)
        .expect("the match is mirrored")
}

/// Red's held frame's `mind`.
fn m5_mind(rig: &Rig) -> tacsim::arena::step::FrameMind {
    rig.held_frame(0)
        .mind
        .expect("a sealed hull's frame carries its mind")
}

/// The tape's `mind_card` lines for `unit`, in order: the millisecond, the
/// card's name, the sliders it moved and the card.
fn m5_cards(rig: &Rig, unit: ObjId) -> Vec<(i64, String, Vec<String>, tacsim::arena::StockCard)> {
    rig.tape
        .iter()
        .filter_map(|event| match &event.kind {
            tacsim::arena::ledger::EventKind::MindCard {
                unit: carded,
                name,
                changed,
                stock: Some(card),
                ..
            } if *carded == unit => Some((
                event.t_ms,
                name.to_string(),
                changed.iter().map(ToString::to_string).collect(),
                card.clone(),
            )),
            _ => None,
        })
        .collect()
}

/// The mirror's copy of `unit`'s card, as `@mind/live` reads it.
fn m5_mirror_card(
    rig: &Rig,
    unit: ObjId,
) -> Option<(
    tacsim::arena::StockCard,
    i64,
    tacsim::arena_bridge::CardSource,
)> {
    let match_id = rig.centre().match_id.expect("a running match");
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .and_then(|mirror| mirror.cards.get(&unit).cloned())
}

/// **`MIND PROFILE` switches a sealed profile, at most once in ten seconds**
/// (17b cut M5.4; doc 17 §11, §12, D-UM-44; the M5 design note's M5.4 items
/// 4 and 5), over the production composition, on a COLD tank (no `STARTUP`:
/// a switch asks no power, 17b cut M5.2):
///
/// * bare `MIND` prints [`cockpit::mind_lines`]' block off the held frame —
///   Sniper flown, Line the other, every channel the pilot's;
/// * `MIND PROFILE line` is acknowledged, staged through the seat on helm,
///   and the next frame's `mind` reads Line and the ABSOLUTE moment of the
///   next switch, which the keyframe the bridge pushes carries as
///   `next_switch_ms`;
/// * inside the ten seconds the cockpit refuses in one line each — the
///   countdown read against the mirror's arena clock — and the profile
///   already flown and a preset not sealed; and the arena refuses a switch
///   the cockpit never pre-checked (offered straight through the seat's own
///   forward) in its one generic sentence, the profile unmoved;
/// * the tape holds the switch as one `mind_card` line, named Line, at the
///   millisecond the next switch counts from;
/// * once the mirror's clock passes the ten seconds the next switch is taken
///   (back to Sniper), and ten seconds on, a third (Line again);
/// * after `EXIT` the released hull flies the card the pilot switched to: a
///   staff slider set lands on Line's card, never the first sealed profile's.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m5_mind_profile_switches_a_sealed_profile_at_most_once_in_ten_seconds() {
    use tacsim::arena::intents::IntentKind;
    use tacsim::arena::state::Power;
    let mut rig = m5_sniper_line().await;
    let pilot = rig.pilot(0);
    let tank = rig.tank(0);
    assert_eq!(rig.held_frame(0).power, Power::Off, "a cold tank");
    let mind = m5_mind(&rig);
    assert!(mind.doctrine);
    assert_eq!(
        mind.profiles.names().collect::<Vec<_>>(),
        ["Sniper", "Line"]
    );
    assert_eq!((mind.active, mind.switch_at_ms, mind.off), (0, None, false));

    // Bare `MIND`: the block, off the held frame.
    let now = m5_now_ms(&rig);
    let said = rig.say(0, "mind");
    // The block, then the frame it read (the line client marks the panel).
    let mut block = cockpit::mind_lines(&mind, true, now);
    block.push("[panel arena.cockpit]".to_owned());
    assert_eq!(said, block.join("\n"));
    assert_eq!(
        said.lines().nth(1),
        Some(
            "Profile: Sniper, 1 of 2 (also Line); MIND PROFILE Line switches, once in ten seconds."
        )
    );
    assert_eq!(
        said.lines().nth(2),
        Some("Channels: you hold helm, sensors and fire; the mind is dormant while you fly.")
    );
    assert_eq!(
        rig.say(0, "mind rules"),
        "Current station rules (manual changes are not saved):\nNone armed.\n\
         [panel arena.cockpit]"
    );
    assert_eq!(
        rig.say(0, "mind trace 3"),
        "MIND TRACE refused. No mind trace is available."
    );

    // The switch, from the seat on helm.
    let mark = rig.log.len();
    assert_eq!(
        rig.say(0, "mind profile line"),
        "Profile Line: the mind will fly it when you leave the seat."
    );
    m5_until(&mut rig, "the switch on red's frame", |rig| {
        rig.frame(0)
            .and_then(|frame| frame.mind)
            .is_some_and(|mind| mind.active == 1)
    })
    .await;
    let at = m5_mind(&rig)
        .switch_at_ms
        .expect("a switch stamps the next one's moment");
    let pushed = y4_cockpit_frames(&rig, pilot, mark);
    let last = pushed.last().expect("the bridge pushed the switched frame");
    assert_eq!(last["mind"]["active"], serde_json::json!(1), "{last}");
    assert_eq!(
        last["mind"]["next_switch_ms"],
        serde_json::json!(at),
        "{last}"
    );
    assert_eq!(
        last["mind"]["profiles"],
        serde_json::json!(["Sniper", "Line"])
    );

    // Inside the ten seconds: the cockpit's own pre-checks, one line each,
    // the countdown against the mirror's arena clock.
    let now = m5_now_ms(&rig);
    assert!(now < at, "the window is open: {now} against {at}");
    let secs = (at - now + 999) / 1_000;
    assert_eq!(
        rig.say(0, "mind profile sniper"),
        format!("MIND refused. One switch in ten seconds; the next in {secs} s.")
    );
    assert_eq!(
        rig.say(0, "mind profile LINE"),
        "MIND refused. Line is already the profile."
    );
    assert_eq!(
        rig.say(0, "mind profile hunter"),
        "MIND refused. Hunter is not sealed on this mind; it carries Sniper and Line."
    );
    // The arena's own check: a switch the cockpit never pre-checked, offered
    // through the seat's own forward inside the ten seconds.
    let mark = rig.log.len();
    rig.with_ctx_mut(0, |ctx| {
        tacsim::arena_bridge::forward_intent(ctx, pilot, tank, IntentKind::Profile { index: 0 })
    })
    .expect("the seat forwards the switch");
    m5_until(&mut rig, "the arena's refusal", |rig| {
        m1a4_said_since(rig, pilot, mark)
            .iter()
            .any(|line| line == "MIND refused. That profile cannot be switched to now.")
    })
    .await;
    assert_eq!(m5_mind(&rig).active, 1, "the profile did not move");

    // The tape: one `mind_card` line, the switch, at the stamp's moment.
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, .. } if *unit == tank
            )
        })
        .await;
    assert!(landed, "the switch is on the tape");
    let cards = m5_cards(&rig, tank);
    let (t_ms, name, _, card) = cards.first().cloned().expect("the switch's line");
    assert_eq!((t_ms, name.as_str()), (at - 10_000, "Line"));
    assert_eq!(card, tacsim::arena::line(&config().arena.reflex));

    // Past the ten seconds, on the mirror's clock: the next is taken.
    m5_until(&mut rig, "the ten seconds", |rig| m5_now_ms(rig) >= at).await;
    assert_eq!(
        rig.say(0, "mind profile sniper"),
        "Profile Sniper: the mind will fly it when you leave the seat."
    );
    m5_until(&mut rig, "the second switch", |rig| {
        rig.frame(0)
            .and_then(|frame| frame.mind)
            .is_some_and(|mind| mind.active == 0)
    })
    .await;
    let again = m5_mind(&rig).switch_at_ms.expect("the second stamp");
    assert!(again >= at + 10_000, "{again} against {at}");
    m5_until(&mut rig, "ten seconds more", |rig| m5_now_ms(rig) >= again).await;
    assert_eq!(
        rig.say(0, "mind profile line"),
        "Profile Line: the mind will fly it when you leave the seat."
    );
    m5_until(&mut rig, "the third switch", |rig| {
        rig.frame(0)
            .and_then(|frame| frame.mind)
            .is_some_and(|mind| mind.active == 1)
    })
    .await;

    // The release flies the switched card: a staff slider set after `EXIT`
    // lands on Line's card (the first sealed profile was Sniper).
    rig.say(0, "exit");
    m5_until(&mut rig, "the release", |rig| {
        m1b5_controller(rig, tank) == Some(tacsim::arena::state::Controller::Mind)
    })
    .await;
    let said = rig.staff(&format!("@mind/live #{}=band_pct 80", tank.0));
    assert!(
        said.starts_with(&format!(
            "Mind card on #{}: band_pct 80 sent; the arena takes it at its next pulse.",
            tank.0
        )),
        "{said}"
    );
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, name, .. }
                    if *unit == tank && name == "Line, edited (band_pct 80)"
            )
        })
        .await;
    assert!(landed, "the staff set lands on the released hull");
    let mut expected = tacsim::arena::line(&config().arena.reflex);
    expected
        .set("band_pct", "80")
        .expect("band_pct 80 is on Line's scale");
    let (_, name, changed, card) = m5_cards(&rig, tank)
        .last()
        .cloned()
        .expect("the staff set's line");
    assert_eq!(changed, ["band_pct"]);
    assert_eq!(card, expected, "the row flew Line, the switched card");
    assert_eq!(name, "Line, edited (band_pct 80)");
}

/// **A switch on a stationary hull sends one frame, and `MIND` reads it**
/// (17b cut M5.4; the M5 design note's M5.4 item 2, the feasibility review's
/// A4): red's cold tank stands where the check-out put it and its pilot is
/// sent nothing for two arena seconds; then `MIND PROFILE Line` — and in the
/// three arena seconds after it, exactly ONE keyframe reaches the pilot, its
/// `mind.active` moved to Line, its `next_switch_ms` the switch's absolute
/// moment; then bare `MIND` prints the new profile, the countdown read
/// against the mirror's arena clock. Without `Frame::same_as`' comparison of
/// the mind the pilot would have been sent nothing, and every reader of the
/// held frame would read Sniper.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m5_a_switch_on_a_stationary_hull_sends_one_frame_and_mind_reads_it() {
    let mut rig = m5_sniper_line().await;
    let pilot = rig.pilot(0);
    let from = rig.staged_now_ms;
    m5_until(&mut rig, "two arena seconds", |rig| {
        rig.staged_now_ms >= from + 2_000
    })
    .await;
    let quiet = rig.log.len();
    let from = rig.staged_now_ms;
    m5_until(&mut rig, "two quiet arena seconds", |rig| {
        rig.staged_now_ms >= from + 2_000
    })
    .await;
    assert!(
        y4_cockpit_frames(&rig, pilot, quiet).is_empty(),
        "a parked, cold hull is sent nothing"
    );

    let mark = rig.log.len();
    assert_eq!(
        rig.say(0, "mind profile Line"),
        "Profile Line: the mind will fly it when you leave the seat."
    );
    let from = rig.staged_now_ms;
    m5_until(&mut rig, "three arena seconds", |rig| {
        rig.staged_now_ms >= from + 3_000
    })
    .await;
    let frames = y4_cockpit_frames(&rig, pilot, mark);
    assert_eq!(frames.len(), 1, "one frame for the switch: {frames:#?}");
    let mind = m5_mind(&rig);
    assert_eq!(mind.active, 1);
    let at = mind.switch_at_ms.expect("the switch's stamp");
    let sent = frames.first().expect("the one frame");
    assert_eq!(sent["mind"]["active"], serde_json::json!(1));
    assert_eq!(sent["mind"]["next_switch_ms"], serde_json::json!(at));

    // `MIND` reads it, against the mirror's clock.
    let now = m5_now_ms(&rig);
    assert!(now < at, "inside the ten seconds: {now} against {at}");
    let secs = (at - now + 999) / 1_000;
    let said = rig.say(0, "mind");
    assert_eq!(
        said.lines().nth(1),
        Some(
            format!(
                "Profile: Line, 2 of 2 (also Sniper); one switch in ten seconds, the next in \
                 {secs} s."
            )
            .as_str()
        ),
        "{said}"
    );
    let mut block = cockpit::mind_lines(&mind, true, now);
    block.push("[panel arena.cockpit]".to_owned());
    assert_eq!(said, block.join("\n"));
}

/// **A staff read-back follows a pilot's switch** (17b cut M5.4; doc 17 §11,
/// D14 of the M5 design note; `@mind/live`, 17b cut M1b.5): red's hull opens
/// the match with the world's mirror holding its first sealed profile's card
/// — Sniper, `as checked out` at nought; its pilot switches to Line from the
/// seat, and the switch's `mind_card` line, absorbed off the tape as a staff
/// edit's is, leaves the mirror on Line. After `EXIT` the hull is its mind's,
/// so staff's bare `@mind/live #<tank>` reads `Line (on the tape at N ms)`
/// with Line's five sections, and a staff slider set is validated and read
/// back on a scratch card made from Line — `Line, edited (band_pct 80)`,
/// never Sniper — and lands on the arena's row as that very card.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m5_a_staff_read_back_follows_a_pilots_switch() {
    use tacsim::arena_bridge::CardSource;
    let mut rig = m5_sniper_line().await;
    let tank = rig.tank(0);
    let id = tank.0;
    let reflex = config().arena.reflex;
    let sniper = tacsim::arena::preset("Sniper", &reflex).expect("a preset");
    let line = tacsim::arena::line(&reflex);
    assert_eq!(
        m5_mirror_card(&rig, tank),
        Some((sniper, 0, CardSource::CheckOut)),
        "the check-out's card: the first sealed profile"
    );
    // A seated hull is not staff's to set.
    assert_eq!(
        rig.staff(&format!("@mind/live #{id}")),
        format!("@mind/live: #{id} is not flown by a mind.")
    );

    assert_eq!(
        rig.say(0, "mind profile line"),
        "Profile Line: the mind will fly it when you leave the seat."
    );
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, .. } if *unit == tank
            )
        })
        .await;
    assert!(landed, "the switch is on the tape");
    let (t_ms, ..) = m5_cards(&rig, tank).first().cloned().expect("the line");
    assert_eq!(
        m5_mirror_card(&rig, tank),
        Some((line.clone(), t_ms, CardSource::OnTape)),
        "the mirror follows the switch"
    );

    rig.say(0, "exit");
    m5_until(&mut rig, "the release", |rig| {
        m1b5_controller(rig, tank) == Some(tacsim::arena::state::Controller::Mind)
    })
    .await;
    let name = mush_core::output::display_name(rig.tw.world(), tank);
    let said = rig.staff(&format!("@mind/live #{id}"));
    let mut want = vec![format!(
        "Mind card on #{id} ({name}): Line (on the tape at {t_ms} ms)"
    )];
    want.extend(line.read_back(&reflex, true));
    assert_eq!(said, want.join("\n"));

    // A staff slider set after the release: validated and read back on Line.
    let mut edited = line.clone();
    edited
        .set("band_pct", "80")
        .expect("band_pct 80 is on Line's scale");
    let said = rig.staff(&format!("@mind/live #{id}=band_pct 80"));
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some(
            format!("Mind card on #{id}: band_pct 80 sent; the arena takes it at its next pulse.")
                .as_str()
        )
    );
    assert!(
        lines.get(1).is_some_and(|head| head.starts_with(&format!(
            "Mind card on #{id} ({name}): Line, edited (band_pct 80) (sent at "
        ))),
        "{said}"
    );
    assert_eq!(
        lines.get(2..).map(<[&str]>::to_vec),
        Some(
            edited
                .read_back(&reflex, true)
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
        )
    );
    assert!(
        !said.contains("Sniper"),
        "nothing off the check-out's card: {said}"
    );
    let landed = rig
        .tape_until(|event| {
            matches!(
                &event.kind,
                tacsim::arena::ledger::EventKind::MindCard { unit, name, .. }
                    if *unit == tank && name == "Line, edited (band_pct 80)"
            )
        })
        .await;
    assert!(landed, "the staff set lands");
    let (_, _, changed, card) = m5_cards(&rig, tank).last().cloned().expect("its line");
    assert_eq!(changed, ["band_pct"]);
    assert_eq!(card, edited, "the arena's row is the read-back's card");
}

/// **`MIND` reads the SEAL, so after Q-UM-15's substitution it names what
/// flies** (17b cut M5.4; the orchestrator's addendum, M5.2's D26; doc 17
/// §21's Q-UM-15 at its default, the owner's OQ-5): with the test provider
/// installed through the production seam (`install_holdings`), red's pilot
/// chooses `Fixture I` — a mind that is not Standard Doctrine — and blue's
/// claims, so there are humans on both sides and the check-out seals red's
/// hull Standard Doctrine on Line. From the seat, bare `MIND` reads that
/// seal off the frame — `Mind: Standard Doctrine, …` and the set of one —
/// never the choice (no `Fixture I`, no `your own mind`, no Sniper), and
/// `MIND PROFILE Sniper` is refused as a profile the sealed mind does not
/// carry.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m5_mind_reads_the_seal_and_after_a_substitution_names_what_flies() {
    let mut rig = Rig::boot(2);
    let _provider = support::holdings::install(rig.tw.world_mut());
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    assert_eq!(
        rig.say(0, "sim mind Fixture I PROFILE Sniper"),
        "Your slot will carry Fixture I (Sniper)."
    );
    rig.say(0, "sim start");
    assert!(rig.centre().doctrine_only, "humans on both sides");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    assert!(rig.frame_for(0).await, "the arena sends red its frame");

    let mind = m5_mind(&rig);
    assert!(mind.doctrine, "the substitution is what flies");
    assert_eq!(mind.profiles.names().collect::<Vec<_>>(), ["Line"]);
    let said = rig.say(0, "mind");
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(
        lines.first().copied(),
        Some(cockpit::mind_head_line(&mind).as_str())
    );
    assert!(
        lines.first().is_some_and(
            |head| head.starts_with("Mind: Standard Doctrine, sealed for this sortie;")
        ),
        "{said}"
    );
    assert_eq!(
        lines.get(1).copied(),
        Some("Profile: Line, 1 of 1; no other is sealed to switch to.")
    );
    for leak in ["Fixture", "your own mind", "Sniper"] {
        assert!(!said.contains(leak), "MIND names the choice: {said}");
    }
    assert_eq!(
        rig.say(0, "mind profile sniper"),
        "MIND refused. Sniper is not sealed on this mind; it carries Line."
    );
}

/// **Every `MIND` form answers through the verb, its guards in their order**
/// (17b cut M5.4; the M5 design note's M5.4 items 4-6; the review's finding
/// that the dispatch was pinned only through its composed strings), over the
/// production composition, red's hull sealed as every unchosen slot is
/// (Standard Doctrine on Line alone):
///
/// * before the arena's first frame (16b cut V.1's precedent, `EMCON`'s):
///   bare `MIND` and `MIND RULES` say [`cockpit::NO_FRAME_YET`] and push
///   nothing, `MIND PROFILE line` says it after `MIND refused.`, and `MIND
///   TRACE` is refused in its own form;
/// * on the sealed frame: `MIND PROFILE` with no name and a form `MIND`
///   does not take each say [`cockpit::MIND_USAGE`]; `MIND RULES` validates
///   its optional slot, and a word no preset answers is refused without
///   an echo; the profile already flown is refused; `MI` and `MIN` (the
///   prefix shift) dispatch to the same answers as `MIND`;
/// * on a frame that carries no `mind` (a hull no seal reached), bare `MIND`
///   says [`cockpit::NO_SEAL`] with the frame and `MIND PROFILE` says it
///   after the prefix; live pilot rules remain readable through `MIND RULES`;
/// * on a wreck the wreck is refused first, in `MIND refused.`'s prefix —
///   before the no-seal check and before a switch the frame would allow
///   (D-B3) — while bare `MIND` still reads the block, as every read verb
///   answers a wreck.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn m5_every_mind_form_answers_through_the_verb_in_its_order() {
    let mut rig = Rig::boot(2);
    rig.fly().await;

    // Before the first frame: the panel is dark, and nothing is pushed.
    assert!(rig.frame(0).is_none(), "nothing has been pumped yet");
    assert_eq!(rig.say(0, "mind"), cockpit::NO_FRAME_YET);
    assert_eq!(rig.say(0, "mind rules"), cockpit::NO_FRAME_YET);
    assert_eq!(
        rig.say(0, "mind profile line"),
        "MIND refused. The panel has told you nothing yet."
    );
    assert_eq!(
        rig.say(0, "mind trace"),
        "MIND TRACE refused. No mind trace is available."
    );

    // The sealed frame: every form `MIND` does not take is the usage.
    assert!(rig.frame_for(0).await, "the arena sends red its frame");
    let mind = m5_mind(&rig);
    assert_eq!(
        mind.profiles.names().collect::<Vec<_>>(),
        vec!["Line"],
        "an unchosen slot's seal"
    );
    for form in [
        "mind profile",
        "MIND PROFILE",
        "mind bogus",
        "mind profiles line",
    ] {
        assert_eq!(rig.say(0, form), cockpit::MIND_USAGE, "{form}");
    }
    assert_eq!(
        rig.say(0, "mind rules now"),
        "MIND RULE refused. A rule slot is a whole number from 1 to 6."
    );
    assert_eq!(
        rig.say(0, "mind profile berserker"),
        "MIND refused. No profile of that name is sealed on this mind; it carries Line."
    );
    assert_eq!(
        rig.say(0, "mind profile line"),
        "MIND refused. Line is already the profile."
    );
    let block = rig.say(0, "mind");
    assert!(
        block.starts_with("Mind: Standard Doctrine, sealed for this sortie;"),
        "{block}"
    );
    for prefix in ["mi", "MIN"] {
        assert_eq!(rig.say(0, prefix), block, "{prefix}");
    }
    assert_eq!(
        rig.say(0, "mi profile line"),
        "MIND refused. Line is already the profile."
    );

    // A frame no seal reached: `NO_SEAL`, with the frame for the reads.
    let mut unsealed = rig.held_frame(0);
    unsealed.notices.clear();
    unsealed.mind = None;
    rig.push_frame(0, unsealed);
    let no_seal = format!("{}\n[panel arena.cockpit]", cockpit::NO_SEAL);
    assert_eq!(rig.say(0, "mind"), no_seal);
    assert_eq!(
        rig.say(0, "mind rules"),
        "Current station rules (manual changes are not saved):\nNone armed.\n[panel arena.cockpit]",
        "live pilot rules are readable independently of a sealed mind"
    );
    assert_eq!(
        rig.say(0, "mind profile line"),
        "MIND refused. This hull carries no seal."
    );

    // A wreck: refused first, before the no-seal check …
    let mut dead = rig.held_frame(0);
    dead.notices.clear();
    dead.wreck = true;
    rig.push_frame(0, dead.clone());
    let wrecked = "MIND refused. Your tank is destroyed and takes no orders. EXIT leaves the pod.";
    assert_eq!(rig.say(0, "mind profile line"), wrecked);
    // … and before a switch the frame would allow; bare `MIND` still reads.
    let two = tacsim::arena::step::FrameMind {
        profiles: m6_frame(&["Sniper", "Line"]).profiles,
        ..mind.clone()
    };
    dead.mind = Some(two.clone());
    rig.push_frame(0, dead.clone());
    assert_eq!(rig.say(0, "mind profile line"), wrecked);
    let mut block = cockpit::mind_lines(&two, !dead.weapons.is_empty(), 0);
    block.push("[panel arena.cockpit]".to_owned());
    assert_eq!(rig.say(0, "mind"), block.join("\n"));
}

// ---------------------------------------------------------------------------
// 17b cut M5.6: the `Your mind` block's spans, on the arena's own clock
// ---------------------------------------------------------------------------

use tacsim::match_::report::stamp;

/// **The world's copy of the arena's own clock for the centre's match**
/// (`MatchMirror::now_ms`).
fn m5_arena_clock(rig: &Rig) -> i64 {
    let match_id = rig.centre().match_id.expect("a running match");
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .map(|mirror| mirror.now_ms)
        .expect("the registry mirrors the running match")
}

/// Pump until the mirror's clock reads past `after`, or give up.
async fn m5_clock_past(rig: &mut Rig, after: i64) -> i64 {
    for _ in 0..600 {
        rig.pump();
        let now = m5_arena_clock(rig);
        if now > after {
            return now;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the arena's clock never passed {after} ms");
}

/// **One pilot's `Your mind` block as their report would print it now**: the
/// debrief fold on the match's record, finished on a copy.
fn m5_block_now(rig: &Rig, n: usize) -> Vec<String> {
    let match_id = rig.centre().match_id.expect("a running match");
    let tank = rig.tank(n);
    rig.tw
        .world()
        .module_state::<match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .and_then(|record| record.folds.get(&tank).cloned())
        .expect("a pilot's debrief fold")
        .finish()
        .mind
        .map(|block| block.lines)
        .unwrap_or_default()
}

/// **One pilot's spans as the world told them, to the millisecond** (17b cut
/// M5.6's review, R4/R28): `(from_ms, to_ms)` off the debrief fold's own live
/// record (`mind.spans`), where the block prints them only to the second.
fn m5_spans_now(rig: &Rig, n: usize) -> Vec<(i64, Option<i64>)> {
    let match_id = rig.centre().match_id.expect("a running match");
    let tank = rig.tank(n);
    let fold = rig
        .tw
        .world()
        .module_state::<match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .and_then(|record| record.folds.get(&tank).cloned())
        .expect("a pilot's debrief fold");
    let record = serde_json::to_value(&fold).expect("a fold writes");
    record["mind"]["spans"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|span| {
            (
                span["from_ms"].as_i64().expect("a span opens"),
                span["to_ms"].as_i64(),
            )
        })
        .collect()
}

/// **The ledger's last stamp for the centre's match** (`MatchMirror::arena_ms`)
/// — the number the spans must never read (the ruling at 17b cut M5.4's close).
fn m5_ledger_ms(rig: &Rig) -> i64 {
    let match_id = rig.centre().match_id.expect("a running match");
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(match_id))
        .map(|mirror| mirror.arena_ms)
        .expect("the registry mirrors the running match")
}

/// **17b cut M5.6** (the owner's OQ-9 (ii); the design note's M5.6 item 1, as
/// the orchestrator ruled its clock at 17b cut M5.4's close): the spans of the
/// `Your mind` block are opened where the slot is marked out of its cockpit
/// (`lobby::slot_left`, every `Released` that names a pilot) and closed where
/// a `BOARD` takes the tank back (`lobby::slot_reclaimed`), each at the
/// arena's own clock as the world last heard it — `MatchMirror::now_ms`, never
/// the ledger's last stamp — on the pilot's own fold alone: a drop, a reclaim
/// past `reclaim_min_ms`, then an `EXIT`, on a slot that chose no mind (the
/// block printed because the mind flew it). The other pilot, seated
/// throughout and choosing nothing, has no block. (17b cut M5.6's review,
/// R4/R28: each span's edges are read to the millisecond off the fold's own
/// record against `MatchMirror::now_ms`, and the ledger's `arena_ms` is read
/// and shown apart at the reclaim, not left to a comment.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_drop_a_reclaim_and_an_exit_open_and_close_the_minds_spans_on_the_arena_clock() {
    let mut rig = Rig::boot(2);
    rig.fly().await;
    let pilot = rig.pilot(0);
    rig.say(0, "startup");
    assert!(rig.powered(0).await, "the arena brings power online");
    assert!(
        m5_block_now(&rig, 0).is_empty(),
        "no block before a release"
    );

    // The drop: the span opens at the mirror's clock as it stands — to the
    // millisecond, off the fold's own record.
    let dropped_at = m5_arena_clock(&rig);
    rig.fire_conn(pilot, mush_core::ConnTransition::Disconnected);
    assert_eq!(m5_spans_now(&rig, 0), vec![(dropped_at, None)]);
    let block = m5_block_now(&rig, 0);
    assert_eq!(
        block.get(1),
        Some(&format!(
            "  It flew your tank from {} to the end, while your line was down.",
            stamp(dropped_at)
        )),
        "{block:#?}"
    );

    // The reclaim, past the floor and once the arena's clock has passed its
    // first second — so the stamp reads neither the world's clock (`rig.now`,
    // five seconds on) nor the ledger's last stamp (nought on a quiet opening).
    rig.fire_conn(pilot, mush_core::ConnTransition::Connected);
    let now = rig.now;
    rig.tick_to(now + 5);
    let reclaimed_at = m5_clock_past(&mut rig, dropped_at.max(999)).await;
    assert_ne!(
        stamp(reclaimed_at),
        stamp(rig.now.saturating_mul(1_000)),
        "the arena's clock and the world's read apart here"
    );
    let ledger_at = m5_ledger_ms(&rig);
    assert_ne!(
        stamp(ledger_at),
        stamp(reclaimed_at),
        "the ledger's last stamp and the arena's clock read apart here"
    );
    let said = rig.say(0, "board");
    assert!(said.contains("is yours again where it stands"), "{said}");
    assert_eq!(
        m5_spans_now(&rig, 0),
        vec![(dropped_at, Some(reclaimed_at))],
        "closed on `MatchMirror::now_ms` ({reclaimed_at} ms), never the ledger's {ledger_at} ms"
    );
    let block = m5_block_now(&rig, 0);
    assert_eq!(
        block.get(1),
        Some(&format!(
            "  It flew your tank from {} to {}, while your line was down.",
            stamp(dropped_at),
            stamp(reclaimed_at)
        )),
        "{block:#?}"
    );

    // The `EXIT`: a second span, to the end — one drop and one climb-out, so
    // the line carries no cause clause.
    assert!(rig.frame_for(0).await, "a frame after the reclaim");
    let exited_at = m5_clock_past(&mut rig, reclaimed_at).await;
    let said = rig.say(0, "exit");
    assert!(said.contains("you cannot take it back"), "{said}");
    assert_eq!(
        m5_spans_now(&rig, 0),
        vec![(dropped_at, Some(reclaimed_at)), (exited_at, None)]
    );
    let block = m5_block_now(&rig, 0);
    assert_eq!(
        block.first().map(String::as_str),
        Some("Your mind: Standard Doctrine (Line)."),
        "{block:#?}"
    );
    assert_eq!(
        block.get(1),
        Some(&format!(
            "  It flew your tank from {} to {} and from {} to the end.",
            stamp(dropped_at),
            stamp(reclaimed_at),
            stamp(exited_at)
        )),
        "{block:#?}"
    );
    assert!(
        block
            .get(2)
            .is_some_and(|line| line.starts_with("  Its shots: ") || line == "  It fired nothing."),
        "{block:#?}"
    );

    // The other pilot: seated throughout, nothing chosen, no block.
    assert!(m5_block_now(&rig, 1).is_empty());
}

/// **17b cut M5.6's review** (R25; the design note's M5.6 gate: "a chosen `SIM
/// MIND Standard` with no `PROFILE`, never released, prints it"): the check-out
/// seed, end to end — the choice typed in Selection, sealed by `SIM START`'s
/// check-out (`match_::start`'s `own_minds`, off `Side::minds` and the custody
/// unit's seal) and read off the pilot's own debrief fold: red, who chose
/// `Standard PROFILE Sniper, Line` and never left the seat, reads their block;
/// blue, seated throughout and choosing nothing, reads none. A chosen hull
/// seals byte for byte as an unchosen one, so only this seed can tell them
/// apart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_chosen_mind_is_named_in_the_pilots_block_from_the_check_out() {
    let rig = m5_sniper_line().await;
    assert_eq!(
        m5_block_now(&rig, 0),
        vec![
            "Your mind: Standard Doctrine (Sniper, Line).",
            "  It never flew: you flew the tank the whole match.",
        ]
    );
    assert!(
        m5_block_now(&rig, 1).is_empty(),
        "an unchosen pilot, seated throughout, reads none"
    );
}

/// **17b cut M5.6's review** (R25): the bare choice — `SIM MIND Standard`, no
/// `PROFILE` — is named too, on Line, though its seal is the default's byte for
/// byte; and blue, who chose nothing, still reads none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_bare_standard_choice_is_named_on_line_in_the_pilots_block() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    assert_eq!(
        rig.say(0, "sim mind Standard"),
        "Your slot will carry Standard Doctrine (Line)."
    );
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.walk_to(1, match_::BLUE_ROOM_ENTRY);
    rig.say(0, "board");
    let said = rig.say(1, "board");
    assert_eq!(rig.state(), MatchState::Running, "{said}");
    assert!(rig.frame_for(0).await, "the arena sends red its frame");
    assert_eq!(
        m5_block_now(&rig, 0),
        vec![
            "Your mind: Standard Doctrine (Line).",
            "  It never flew: you flew the tank the whole match.",
        ]
    );
    assert!(m5_block_now(&rig, 1).is_empty());
}

mod m5rules_station {
    use super::*;
    use tacsim::arena::conditions::Rule;

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn every_native_predicate_and_selector_round_trips_through_the_station() {
        let mut rig = Rig::boot(2);
        rig.fly().await;
        assert!(rig.frame_for(0).await);
        let normalized = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
        // These are prospective rules: absent target readings do not prevent
        // admission, and a cold hull with no lock supplies no hidden target.
        // Each command replaces the same Fire slot through the real verb,
        // then reads the authoritative frame back through MIND RULES.
        for (command, words) in [
            (
                "FIRE cannon WHEN odds mg>=50 AND aspect=side,rear",
                "machine_gun's chance to hit is at least 50% and the target shows its side or rear",
            ),
            (
                "FIRE cannon WHEN range>200m AND range<600m",
                "its rounded range is more than 200 m and its rounded range is less than 600 m",
            ),
            (
                "FIRE cannon WHEN band=effective",
                "its range band is effective",
            ),
            (
                "FIRE cannon WHEN rounds<25%",
                "the selected weapon has less than 25% of its full load",
            ),
            (
                "FIRE cannon WHEN rounds mg>3",
                "machine_gun has more than 3 rounds",
            ),
            (
                "FIRE cannon WHEN reserve",
                "the selected weapon has more than 0 rounds",
            ),
            (
                "FIRE cannon WHEN target=identified AND target=moving AND target=hurt",
                "the target is identified and the target is moving and the target is hurt",
            ),
            ("FIRE cannon WHEN target=stopped", "the target is stopped"),
            (
                "FIRE cannon WHEN target turret=red",
                "the earned turret reading is red",
            ),
            (
                "FIRE cannon WHEN own turret<40% AND heat>80%",
                "your turret internals are below 40% of their maximum and your heat is above 80% of its maximum",
            ),
            ("FIRE cannon WHEN lock", "you have an identified lock"),
            (
                "FIRE cannon HOLD WHEN nolock",
                "you have no identified lock",
            ),
            (
                "FIRE cannon WHEN target=best_hostile",
                "cannon fires at your identified hostile lock",
            ),
            (
                "FIRE cannon HOLD WHEN target=stopped AND rounds>0",
                "cannon holds fire when the target is stopped",
            ),
        ] {
            m1b4_next_second(&mut rig).await;
            let said = rig.say(0, &format!("MIND RULE 1 {command}"));
            assert!(normalized(&said).contains(words), "{command}: {said}");
            m1b4_until(&mut rig, command, |rig| {
                rig.frame(0).is_some_and(|frame| {
                    frame.rules.iter().any(|entry| {
                        entry.slot == 1
                            && entry.rule.channel() == Channel::Fire
                            && tacsim::arena::rules::rule_words(&entry.rule, None).contains(words)
                    })
                })
            })
            .await;
            let said = rig.say(0, "MIND RULES 1");
            assert!(normalized(&said).contains(words), "{command}: {said}");
            assert!(
                said.lines()
                    .all(|line| line.len() <= cockpit::WATCH_COLUMNS),
                "{said}"
            );
        }
        for (selector, words) in [
            ("nearest", "lock the nearest identified hostile"),
            (
                "best_odds",
                "lock the identified hostile with the best hit chance",
            ),
            (
                "weakest",
                "lock the identified hostile with the most observed damage",
            ),
            (
                "flank",
                "lock the identified hostile showing the most exposed face",
            ),
            (
                "newest",
                "lock the identified hostile with the freshest plot",
            ),
        ] {
            m1b4_next_second(&mut rig).await;
            let command = format!("MIND RULE 2 LOCK {selector} WHEN nolock");
            assert!(rig.say(0, &command).contains(words), "{command}");
            m1b4_until(&mut rig, &command, |rig| {
                rig.frame(0).is_some_and(|frame| {
                    frame.rules.iter().any(|entry| {
                        entry.slot == 2
                            && entry.rule.channel() == Channel::Sensors
                            && tacsim::arena::rules::rule_words(&entry.rule, None)
                                .starts_with(words)
                    })
                })
            })
            .await;
            let said = rig.say(0, "MIND RULES 2");
            assert!(
                said.contains(words) && said.contains("you have no identified lock"),
                "{said}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_contact_citation_accepted_from_a_stale_mirror_is_refused_at_landing() {
        let mut rig = Rig::boot(2);
        rig.fly().await;
        assert!(rig.frame_for(0).await);
        // Model the world mirror trailing contact expiry in the arena. The
        // real station still resolves the permitted name and queues its intent.
        let mut frame = rig.held_frame(0);
        let mut contact = fixture::frame().contacts[0].clone();
        contact.id = tacsim::arena::state::ContactId(0x0bad_cafe);
        contact.class = tacsim::arena::state::Classification::Classified { party: PartyId::BLUE };
        let name = contact.designator.to_string();
        frame.contacts = vec![contact];
        rig.push_frame(0, frame);
        let panels_before = rig.panels(tacsim::arena_bridge::FRAME_FEED).len();
        let said = rig.say(0, &format!("MIND RULE 1 FIRE cannon WHEN target={name}"));
        assert!(said.contains("fires"), "the mirrored name parsed: {said}");
        m1b4_until(
            &mut rig,
            "the stale citation refused by the authoritative arena",
            |rig| {
                rig.panels(tacsim::arena_bridge::FRAME_FEED)
                    .into_iter()
                    .skip(panels_before)
                    .flat_map(|(_, data)| data["notices"].as_array().cloned().unwrap_or_default())
                    .any(|notice| notice["refusal"] == "unlawful")
            },
        )
        .await;
        let refusal = rig
            .panels(tacsim::arena_bridge::FRAME_FEED)
            .into_iter()
            .skip(panels_before)
            .flat_map(|(_, data)| data["notices"].as_array().cloned().unwrap_or_default())
            .find(|notice| notice["refusal"] == "unlawful")
            .expect("the landing refusal");
        assert!(
            refusal["detail"].is_null(),
            "classification loss must not claim the odds were below the Compact floor: {refusal}"
        );
        assert!(rig.frame(0).expect("fresh frame").rules.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn explicit_rules_round_trip_through_real_station_commands() {
        let mut rig = Rig::boot(2);
        rig.fly().await;
        assert!(rig.frame_for(0).await);
        assert!(rig.frame_for(1).await);
        let said = rig.say(
            0,
            "MIND RULE 1 FIRE cannon HOLD WHEN heat>80% AND target=moving",
        );
        assert!(said.contains("holds fire"), "{said}");
        let said = rig.say(0, "MIND RULE 2 LOCK nearest WHEN no lock");
        assert!(said.contains("nearest"), "{said}");
        m1b4_until(&mut rig, "two different-channel native rules", |rig| {
            rig.frame(0).is_some_and(|frame| frame.rules.len() == 2)
        })
        .await;
        let frame = rig.frame(0).expect("own frame");
        assert!(matches!(&frame.rules[0].rule, Rule::HoldFire { .. }));
        assert!(matches!(&frame.rules[1].rule, Rule::Lock { .. }));
        assert_eq!(frame.rules[0].rule.channel(), Channel::Fire);
        assert_eq!(frame.rules[1].rule.channel(), Channel::Sensors);
        let said = rig.say(0, "MIND RULES");
        assert!(said.contains("manual changes are not saved"), "{said}");
        assert!(
            said.contains("holds fire") && said.contains("nearest"),
            "{said}"
        );
        let said = rig.say(0, "MIND RULES 1");
        assert!(
            said.contains("holds fire") && !said.contains("nearest"),
            "{said}"
        );
        let other = rig.say(1, "MIND RULES");
        assert!(
            !other.contains("holds fire") && !other.contains("nearest"),
            "{other}"
        );
        let said = rig.say(0, "FIRE cannon WHEN hostiles>=2");
        assert!(
            said.contains("thinking mind") && said.contains("dormant"),
            "{said}"
        );
        let said = rig.say(0, "FIRE cannon WHEN target=ZZ");
        assert!(said.contains("not on your plot"), "{said}");

        m1b4_next_second(&mut rig).await;
        let said = rig.say(
            0,
            "MIND RULE 1 FIRE cannon WHEN target=stopped AND rounds>25%",
        );
        assert!(said.contains("fires"), "{said}");
        m1b4_until(
            &mut rig,
            "slot one replaced without moving slot two",
            |rig| {
                rig.frame(0).is_some_and(|frame| {
                    frame.rules.len() == 2
                        && matches!(&frame.rules[0].rule, Rule::Fire { .. })
                        && matches!(&frame.rules[1].rule, Rule::Lock { .. })
                })
            },
        )
        .await;
        let said = rig.say(0, "MIND RULE 2 OFF");
        assert!(said.contains('2'), "{said}");
        m1b4_until(&mut rig, "Sensors rule removed by its own channel", |rig| {
            rig.frame(0)
                .is_some_and(|frame| frame.rules.len() == 1 && frame.rules[0].slot == 1)
        })
        .await;
        let said = rig.say(0, "MIND RULES 2");
        assert!(said.contains("No active rule"), "{said}");

        m1b4_next_second(&mut rig).await;
        // The production line core grants four slots. Leave exactly one free
        // before asking ALL to reserve two, so this tests atomic refusal.
        let _ = rig.say(0, "MIND RULE 3 LOCK nearest WHEN no lock");
        let _ = rig.say(0, "MIND RULE 4 FIRE cannon HOLD WHEN rounds>0");
        m1b4_until(&mut rig, "only one of four slots free", |rig| {
            rig.frame(0).is_some_and(|frame| frame.rules.len() == 3)
        })
        .await;
        let before = rig.frame(0).expect("own frame").rules.clone();
        let _ = rig.say(0, "FIRE ALL HOLD WHEN rounds>0");
        m1b4_next_second(&mut rig).await;
        assert_eq!(
            rig.frame(0).expect("own frame").rules,
            before,
            "a batch that cannot fit changes no existing rule"
        );

        // OFF removes both Fire actions but keeps Sensors authority intact.
        assert_eq!(rig.say(0, "FIRE ALL OFF"), cockpit::CLEARED_ALL);
        m1b4_until(
            &mut rig,
            "the Sensors rule survives clearing all guns",
            |rig| {
                rig.frame(0).is_some_and(|frame| {
                    frame.rules.len() == 1
                        && frame.rules[0].slot == 3
                        && matches!(&frame.rules[0].rule, Rule::Lock { .. })
                })
            },
        )
        .await;
        m1b4_next_second(&mut rig).await;
        let said = rig.say(0, "FIRE ALL WHEN rounds>0");
        assert_eq!(
            said.lines()
                .filter(|line| line.starts_with("Queued:"))
                .count(),
            2,
            "one readback for each gun: {said}"
        );
        m1b4_until(
            &mut rig,
            "ALL atomically fills the two free gun slots",
            |rig| {
                rig.frame(0).is_some_and(|frame| {
                    frame.rules.len() == 3
                        && frame.rules[0].slot == 1
                        && matches!(&frame.rules[0].rule, Rule::Fire { .. })
                        && frame.rules[1].slot == 2
                        && matches!(&frame.rules[1].rule, Rule::Fire { .. })
                        && frame.rules[2].slot == 3
                        && matches!(&frame.rules[2].rule, Rule::Lock { .. })
                })
            },
        )
        .await;
        let said = rig.say(0, "MIND RULES");
        assert!(
            said.contains("cannon fires")
                && said.contains("machine_gun fires")
                && said.contains("nearest"),
            "{said}"
        );
        assert!(
            said.lines()
                .all(|line| line.len() <= cockpit::WATCH_COLUMNS),
            "full readback stays within the terminal floor: {said}"
        );
    }
}

/// Build actual immutable cards for renderer fixtures; names never reconstruct runtime data.
fn m6_frame(names: &[&str]) -> tacsim::arena::step::FrameMind {
    use tacsim::arena::core::{ClassTable, CoreClass};
    use tacsim::arena::profile_seal::SealedCards;
    use tacsim::arena::runtime_card::{CardCatalog, stock_definition};
    let cfg = tacsim::config::ReflexConfig::default();
    let selected = names
        .iter()
        .map(|name| {
            (
                smol_str::SmolStr::new(*name),
                tacsim::arena::preset(name, &cfg).expect("fixture preset"),
            )
        })
        .collect::<Vec<_>>();
    let definition = stock_definition(&selected, CoreClass::III).expect("full cards");
    let cards = SealedCards::prepare(
        definition,
        &[],
        &ClassTable::STOCK.row(CoreClass::III),
        &[],
        &[
            tacsim::arena::Channel::Helm,
            tacsim::arena::Channel::Sensors,
        ],
        100,
    )
    .expect("sealed cards");
    let mut catalog = CardCatalog::default();
    let (_, card) = cards.admit(&mut catalog).expect("catalog admission");
    // Retain the complete selected order for profile renderer tests.
    let names = names
        .iter()
        .map(|name| smol_str::SmolStr::new(*name))
        .collect::<Vec<_>>();
    let profiles =
        tacsim::arena::runtime_card::RuntimeProfiles::bind(&catalog, card.key().definition, &names)
            .expect("selected profiles");
    tacsim::arena::step::FrameMind {
        name: smol_str::SmolStr::new_static("Standard Doctrine"),
        doctrine: true,
        profiles,
        card,
        program: cards.programs[0].clone(),
        active: 0,
        class: CoreClass::I,
        core_class: CoreClass::II,
        switch_at_ms: None,
        off: false,
    }
}

// ---------------------------------------------------------------------------
// 16c Q-PT1 (2026-10-27) — the seat says it is a range
// ---------------------------------------------------------------------------

/// **A range reads as a range from the seat** (16c `Q-PT1`; the 2026-10-27
/// dev2 playtest in doc 16c §4, where four range targets took a fight's
/// shape): on `SIM RANGE 1` the pilot's keyframe carries `"mode": "range"`,
/// `STATUS` opens `Callsign RA. On the range: targets on the blue side.` and
/// `WATCH`'s header ends `, RANGE` — and a skirmish's frame says none of it.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn q_pt1_a_range_says_so_from_the_seat() {
    let mut rig = Rig::boot(1);
    rig.say(0, "sim team red");
    rig.say(0, "sim range 1");
    rig.say(0, "sim start");
    rig.walk_to(0, match_::RED_ROOM_ENTRY);
    rig.say(0, "board");
    assert_eq!(rig.state(), MatchState::Running);
    let match_id = rig.centre().match_id.expect("a running match");
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = rig.frame(0).expect("a held frame");
    assert!(cockpit::on_range(&frame), "the frame's match is a range");
    assert_eq!(
        cockpit::status_frame(match_id, &frame)["mode"],
        serde_json::json!("range"),
        "the keyframe carries the mode"
    );
    let said = rig.say(0, "status");
    assert!(
        said.contains("Callsign RA. On the range: targets on the blue side."),
        "{said}"
    );
    let watch = cockpit::watch_lines(
        &frame,
        &BTreeMap::new(),
        &cockpit::frame_weapons(&frame),
        1_000,
    );
    assert!(
        watch.first().is_some_and(|line| line.ends_with(", RANGE")),
        "{watch:?}"
    );

    // The skirmish fixture: the same two readers print nothing of it.
    let skirmish = fixture::frame();
    assert!(!cockpit::on_range(&skirmish));
    assert_eq!(cockpit::range_sentence(&skirmish), "");
    let watch = cockpit::watch_lines(
        &skirmish,
        &BTreeMap::new(),
        &cockpit::frame_weapons(&skirmish),
        1_000,
    );
    assert!(
        watch.first().is_some_and(|line| !line.contains("RANGE")),
        "{watch:?}"
    );
}

// ---------------------------------------------------------------------------
// Doc 21 cut G.14 — one health word a contact
// ---------------------------------------------------------------------------

/// **Doc 21 cut G.14**: each keyframe contact carries `health` — the one word
/// the picture gauges the hull by — summed from the bands the platform already
/// holds, and `null` wherever it holds no reading. On the fixture scene BA's
/// `ggyrd` (a red track, a destroyed turret, the fatal hull green) reads
/// `crippled`; RB (classified, never read), X3 and X4 (unidentified) read
/// `null`. The word follows the reading: all green `intact`, a yellow
/// `damaged`, a destroyed engine `crippled` (the chassis survives it), the
/// hull destroyed `wreck`; an own kill is `wreck` with no reading at all; and
/// an unarmed hull, whose rows carry no `reads_dead`, reads the wreck off the
/// same destroyed hull. Never a number: the key is a string or null on every
/// row, and nothing else about the contact moves.
#[test]
fn g14_each_contact_carries_one_health_word_or_null() {
    use tacsim::arena::condition::{Condition, ConditionBand, ConditionTier};
    use tacsim::arena::perception::FrameCondition;

    let contact = |frame: &tacsim::arena::step::Frame, designator: &str| -> serde_json::Value {
        cockpit::status_frame(fixture::MATCH, frame)
            .get("contacts")
            .and_then(serde_json::Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("designator") == Some(&serde_json::json!(designator)))
                    .cloned()
            })
            .unwrap_or_else(|| panic!("{designator} is on the frame"))
    };
    let health = |frame: &tacsim::arena::step::Frame, designator: &str| {
        contact(frame, designator)
            .get("health")
            .cloned()
            .unwrap_or_else(|| panic!("{designator} carries the key"))
    };
    let frame = fixture::frame();
    assert_eq!(health(&frame, "BA"), serde_json::json!("crippled"));
    assert_eq!(
        health(&frame, "RB"),
        serde_json::Value::Null,
        "classified, never read"
    );
    assert_eq!(
        health(&frame, "X3"),
        serde_json::Value::Null,
        "unidentified"
    );
    assert_eq!(
        health(&frame, "X4"),
        serde_json::Value::Null,
        "unidentified"
    );

    // The word follows BA's reading.
    let all = |band: ConditionBand| {
        (0..5).fold(Condition::default(), |bands, index| {
            bands.with_band(index, band)
        })
    };
    let read = |bands: Condition| {
        let mut frame = fixture::frame();
        let ba = frame
            .contacts
            .iter_mut()
            .find(|held| held.designator.to_string() == "BA")
            .expect("BA");
        ba.condition = Some(FrameCondition {
            table: 0,
            bands,
            age_ms: 2_000,
        });
        frame
    };
    assert_eq!(
        health(&read(all(ConditionBand::Green)), "BA"),
        serde_json::json!("intact")
    );
    assert_eq!(
        health(
            &read(all(ConditionBand::Green).with_band(2, ConditionBand::Yellow)),
            "BA"
        ),
        serde_json::json!("damaged")
    );
    assert_eq!(
        health(
            &read(all(ConditionBand::Green).with_band(0, ConditionBand::Destroyed)),
            "BA"
        ),
        serde_json::json!("crippled"),
        "the engine gone: the chassis survives it"
    );
    assert_eq!(
        health(
            &read(all(ConditionBand::Green).with_band(3, ConditionBand::Red)),
            "BA"
        ),
        serde_json::json!("crippled"),
        "a red track"
    );
    let hull_gone = read(all(ConditionBand::Green).with_band(1, ConditionBand::Destroyed));
    assert_eq!(
        health(&hull_gone, "BA"),
        serde_json::json!("wreck"),
        "the hull is the chassis's fatal section"
    );

    // The own kill: `wreck` with no reading at all.
    let mut killed = fixture::frame();
    for held in &mut killed.contacts {
        if held.designator.to_string() == "BA" {
            held.condition = None;
            held.fatal = None;
            held.reads_dead = Some(true);
        }
    }
    killed.scan_tables.clear();
    let ba = contact(&killed, "BA");
    assert_eq!(ba.get("health"), Some(&serde_json::json!("wreck")));
    assert_eq!(ba.get("condition"), Some(&serde_json::Value::Null));

    // An unarmed hull: no `reads_dead` on any row, and the same reading reads
    // the same wreck through the fatal words.
    let mut unarmed = hull_gone;
    unarmed.weapons.clear();
    unarmed.odds.clear();
    for held in &mut unarmed.contacts {
        held.reads_dead = None;
    }
    let ba = contact(&unarmed, "BA");
    assert!(
        !ba.as_object()
            .expect("a contact")
            .contains_key("reads_dead")
    );
    assert_eq!(ba.get("health"), Some(&serde_json::json!("wreck")));

    // The pure summary, and its words.
    assert_eq!(Condition::default().tier(0), ConditionTier::Intact);
    assert_eq!(
        all(ConditionBand::Yellow).tier(0b10),
        ConditionTier::Damaged,
        "a fatal mask over sections that are not destroyed is nothing"
    );
    assert_eq!(
        all(ConditionBand::Destroyed).tier(0),
        ConditionTier::Crippled
    );
    assert_eq!(
        all(ConditionBand::Destroyed).tier(0b1_0000),
        ConditionTier::Wreck
    );
    assert_eq!(
        [
            ConditionTier::Intact,
            ConditionTier::Damaged,
            ConditionTier::Crippled,
            ConditionTier::Wreck
        ]
        .map(|tier| tier.as_str()),
        ["intact", "damaged", "crippled", "wreck"]
    );

    // Never a number, on any row of any hull.
    for frame in [&frame, &killed, &unarmed] {
        for row in cockpit::status_frame(fixture::MATCH, frame)
            .get("contacts")
            .and_then(serde_json::Value::as_array)
            .expect("contacts")
        {
            assert!(
                matches!(
                    row.get("health"),
                    Some(serde_json::Value::Null | serde_json::Value::String(_))
                ),
                "{row}"
            );
        }
    }
}

/// A real connected practice match: two red players, one red machine and a
/// blue player. Connections matter independently from the assigned cockpit.
async fn dt_live_rig() -> Rig {
    let mut rig = Rig::boot_with(3, y1_config());
    for n in 0..3 {
        let actor = rig.pilot(n);
        let events = rig.tw.connect(actor);
        rig.log.extend(events);
    }
    rig.fly_with_ai(2, 1, 3).await;
    for n in 0..3 {
        assert!(rig.frame_for(n).await, "each assigned pilot gets a frame");
    }
    rig
}

fn dt_live_projection(rig: &Rig, n: usize) -> serde_json::Value {
    rig.query(rig.pilot(n), tacsim::arena_bridge::FRAME_FEED)
        .get("detachment")
        .cloned()
        .expect("the active group's frame carries its roster")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dt_live_opt_in_emits_versioned_roster_and_player_wilco_stays_advisory() {
    let mut rig = dt_live_rig().await;
    assert!(
        rig.query(rig.pilot(0), tacsim::arena_bridge::FRAME_FEED)
            .get("detachment")
            .is_none(),
        "an ordinary practice match has not opted into detachment command"
    );
    assert!(rig.say(0, "+sortie").contains("+sortie/practice"));

    let events = rig.events(0, "+sortie/practice");
    let pushed = Rig::panel(&events).expect("opt-in pushes the production cockpit frame");
    let roster = pushed
        .get("detachment")
        .expect("the pushed frame has the roster");
    assert_eq!(roster, &dt_live_projection(&rig, 0));
    for field in ["id", "revision", "term"] {
        assert!(
            roster[field]
                .as_str()
                .is_some_and(|value| value.parse::<u64>().is_ok()),
            "{field} preserves its full integer as a wire string"
        );
    }
    assert_eq!(roster["leader_slot"], 1);
    let slots = roster["slots"].as_array().expect("stable numbered slots");
    assert_eq!(slots.len(), 3);
    assert_eq!(
        slots
            .iter()
            .map(|slot| slot["slot"].as_u64())
            .collect::<Vec<_>>(),
        [Some(1), Some(2), Some(3)]
    );
    assert_eq!(
        slots
            .iter()
            .map(|slot| slot["callsign"].as_str())
            .collect::<Vec<_>>(),
        [Some("RA"), Some("RB"), Some("RC")]
    );
    assert_eq!(slots[0]["controller"], "player");
    assert_eq!(slots[1]["controller"], "player");
    assert_eq!(slots[2]["controller"], "automated");
    assert!(
        slots[2]["pilot"].is_null(),
        "a machine has no invented persona"
    );
    assert!(
        roster["all_order_prefill"]
            .as_str()
            .expect("ALL prefill")
            .ends_with(" ALL ")
    );
    assert!(roster["my_order"].is_null());
    assert!(
        rig.query(rig.pilot(2), tacsim::arena_bridge::FRAME_FEED)
            .get("detachment")
            .is_none(),
        "opting in red does not expose or activate blue"
    );

    let player_unit = rig.tank(1);
    let before = rig.held_frame(1);
    let epochs = Channel::ALL.map(|channel| rig.epoch(player_unit, channel));
    let order = format!(
        "{}WITHDRAW",
        slots[1]["order_prefill"].as_str().expect("player prefill")
    );
    let delivery = rig.events(0, &order);
    assert!(
        rig.tw
            .seen_by(&delivery, rig.pilot(1))
            .iter()
            .any(|line| line.contains("Order from") && line.contains("WITHDRAW")),
        "the connected player receives the advisory order"
    );
    let recipient = dt_live_projection(&rig, 1);
    assert_eq!(recipient["my_order"]["objective"], "WITHDRAW");
    assert!(recipient["my_order"]["sequence"].is_string());
    assert!(recipient["my_order"]["response"].is_null());
    let wilco = recipient["wilco_command"]
        .as_str()
        .expect("receipt's fenced response");
    assert_eq!(
        rig.say(1, wilco),
        "WILCO recorded. Your controls remain yours.\n[panel arena.cockpit]"
    );
    assert_eq!(dt_live_projection(&rig, 1)["my_order"]["response"], "wilco");
    assert_eq!(
        Channel::ALL.map(|channel| rig.epoch(player_unit, channel)),
        epochs
    );

    // A machine order after the acknowledgement provides a real arena barrier:
    // its journal line proves the task has processed the earlier host offers.
    let machine_order = format!(
        "{}HOLD",
        slots[2]["order_prefill"].as_str().expect("machine prefill")
    );
    assert!(rig.say(0, &machine_order).starts_with("Order sent;"));
    assert!(
        rig.tape_until(|event| matches!(
            &event.kind,
            tacsim::arena::ledger::EventKind::DetachmentCommanded { .. }
        ))
        .await,
        "the machine command reaches the actual arena task"
    );
    assert!(!rig.tape.iter().any(|event| matches!(&event.kind, tacsim::arena::ledger::EventKind::DetachmentCommanded { unit, .. } if *unit == player_unit)), "neither the player advisory nor WILCO actuates a controller");
    let after = rig.held_frame(1);
    assert_eq!(after.pose, before.pose);
    assert_eq!(after.speed_cms, before.speed_cms);
    assert_eq!(after.power, before.power);
    assert!(after.order.is_none());
    assert_eq!(
        Channel::ALL.map(|channel| rig.epoch(player_unit, channel)),
        epochs
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dt_live_controller_delivery_is_journalled_and_stale_or_legacy_orders_refuse() {
    let mut rig = dt_live_rig().await;
    assert!(rig.say(0, "+sortie/practice").contains("leader slot 1"));
    let roster = dt_live_projection(&rig, 0);
    let id = roster["id"]
        .as_str()
        .expect("id")
        .parse::<u64>()
        .expect("numeric id");
    let revision = roster["revision"]
        .as_str()
        .expect("revision")
        .parse::<u64>()
        .expect("numeric revision");
    let term = roster["term"]
        .as_str()
        .expect("term")
        .parse::<u64>()
        .expect("numeric term");
    let machine = rig
        .record()
        .callsigns
        .iter()
        .find_map(|(unit, callsign)| (callsign.to_string() == "RC").then_some(*unit))
        .expect("red's third slot is its machine");
    let issuer = rig.pilot(0);
    let issuer_unit = rig.tank(0);
    let order = format!("ORDER DT:{id}:{revision}:{term} 3 HOLD");
    assert_eq!(
        rig.say(0, &order),
        "Order sent; delivery and acknowledgement are reported separately.\n[panel arena.cockpit]"
    );
    let accepted = |event: &tacsim::arena::ledger::LedgerEvent| {
        matches!(
            &event.kind,
            tacsim::arena::ledger::EventKind::DetachmentCommanded { unit, command }
                if *unit == machine && command.issuer == issuer
                    && command.issuer_unit == issuer_unit
                    && command.authority.detachment == id
                    && command.authority.revision == revision
                    && command.authority.term == term
                    && command.authority_epoch > 0
                    && command.sequence == 1
                    && command.expires_at_ms.is_none()
                    && command.directive.is_some_and(|directive| directive.objective == tacsim::deliberation::Objective::Hold)
        )
    };
    assert!(
        rig.tape_until(accepted).await,
        "the delivered command carries the admitted fences into the arena ledger"
    );

    assert_eq!(
        rig.say(
            0,
            &format!(
                "ORDER DT:{id}:{}:{term} 3 WITHDRAW",
                revision.wrapping_add(1)
            )
        ),
        "Detachment changed; refresh the roster before ordering."
    );
    assert_eq!(
        rig.say(
            0,
            &format!("ORDER DT:{id}:{revision}:{} 3 WITHDRAW", term + 1)
        ),
        "Detachment changed; refresh the roster before ordering."
    );
    assert_eq!(
        rig.say(1, &order),
        "Only the current detachment leader may issue orders."
    );
    for command in ["ORDER red-1 WITHDRAW", "ORDER red-1 RELEASE"] {
        let said = rig.say(0, command);
        assert!(
            said.starts_with("Refused:"),
            "legacy command cannot bypass active detachment ownership: {said}"
        );
    }
    // The next valid command is sequence two: every rejected path left the
    // detachment journal and sequence allocator untouched.
    assert!(
        rig.say(0, &format!("ORDER DT:{id}:{revision}:{term} 3 RELEASE"))
            .starts_with("Order sent;")
    );
    assert!(
        rig.tape_until(|event| matches!(
            &event.kind,
            tacsim::arena::ledger::EventKind::DetachmentCommanded { unit, command }
                if *unit == machine && command.sequence == 2 && command.directive.is_none()
        ))
        .await
    );
    let commands = rig
        .tape
        .iter()
        .filter_map(|event| match &event.kind {
            tacsim::arena::ledger::EventKind::DetachmentCommanded { unit, command }
                if *unit == machine =>
            {
                Some(command.sequence)
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(commands, [1, 2]);
}

/// **Doc 30 cut WG.4**: open `proving_ground.draft.json`, move `"The Ford"` from
/// `(64, 97)` to `(80, 97)`, rebuild the two packs with [`tacsim::ground::build`],
/// boot a world from the rebuilt packs, verify [`tacsim::ground::draft_from_world`]
/// reassembles the edited draft from live tiles with `provenance.source == Pack`,
/// and run the live `RANGE The Ford` command in a started match.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wg4_rebuilt_ground_draft_moves_the_ford_and_range_reads_the_moved_spot() {
    let mut draft = tacsim::ground::proving_ground_draft().expect("shipped draft");
    let ford = draft
        .key_spots
        .iter_mut()
        .find(|s| s.name == "The Ford")
        .expect("The Ford");
    assert_eq!((ford.x, ford.y), (64, 97));
    ford.x = 80;
    draft.arena_version = 6;
    let built = tacsim::ground::build(&draft).expect("rebuilt packs");

    let mut cfg = config();
    cfg.packs.push(tacsim::PackSource {
        name: "rebuilt_arena.json".to_owned(),
        bytes: built.arena_json.clone(),
    });
    let mut boot = mush_atlas::starter::packs().expect("starter packs");
    let mut game = tacsim::definitions::content::boot_packs().expect("game packs");
    let _old_arena = game.pop().expect("arena pack");
    let _old_atlas = game.pop().expect("atlas pack");
    boot.extend(game);
    boot.push(mush_content::load_pack(&built.atlas_json).expect("rebuilt atlas pack"));
    boot.push(mush_content::load_pack(&built.arena_json).expect("rebuilt arena pack"));

    let mut rig = Rig::boot_on(2, support::roster_booting(cfg, boot));

    let reassembled = tacsim::ground::draft_from_world(rig.tw.world(), "proving_ground")
        .expect("draft_from_world reassembles from live tiles");
    assert_eq!(
        reassembled.provenance.source,
        tacsim::ground::GroundSource::Pack
    );
    assert_eq!(
        reassembled.provenance.inputs,
        vec![
            "tacsim.proving_ground:region.proving_ground".to_owned(),
            "tacsim.proving_ground:tacsim.arena.proving_ground".to_owned()
        ]
    );
    let reassembled_ford = reassembled
        .key_spots
        .iter()
        .find(|s| s.name == "The Ford")
        .expect("The Ford in reassembled draft");
    assert_eq!((reassembled_ford.x, reassembled_ford.y), (80, 97));

    rig.fly().await;
    assert!(rig.frame_for(0).await, "the arena sends a frame");
    let frame = rig.held_frame(0);
    assert_eq!(frame.cell, Some((6, 97)), "red slot 1 musters on (6, 97)");

    let first = |said: String| said.lines().next().unwrap_or_default().to_owned();
    // From (6, 97) to (80, 97) is 74 cells = 740 m (was 580 m at (64, 97)).
    assert_eq!(
        first(rig.say(0, "RANGE The Ford")),
        "Range to The Ford (A): 740 m."
    );
}
