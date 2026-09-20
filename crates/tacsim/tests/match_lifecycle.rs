//! **16b cut K.1 acceptance** — doc 16 §3.8's machine, walked (16b §8 cut K, K1;
//! doc 16 §3.7 "Pods", §3.8 "Lifecycle", §5).
//!
//! Every claim of the lifetime half of cut K, over the **production composition
//! root** with the shipped content applied — the same rig
//! `crates/tacsim/tests/definitions.rs` boots, plus `[game].starter_area` so the
//! OOC pack's simulator centre is in the world:
//!
//! 1. two pilots walk `Selection → Boarding → Running → Aborted → Idle`, twice
//!    in a row, and the second run is what proves the reset;
//! 2. one account cannot hold two slots and a third account cannot take one that
//!    is held;
//! 3. `SIM START` with an empty side refuses;
//! 4. a boarding that times out drops the unseated and aborts when that empties
//!    a side;
//! 5. `SIM ABORT` from each state releases exactly what that state held;
//! 6. `SIM` outside the centre is refused with one line;
//! 7. `@arena/reset` after an end returns the centre to `Idle`, and a second
//!    reset says nothing changed.
//!
//! # How a pilot is seated here
//!
//! Cut K.2 builds `BOARD`. Until it exists, nothing puts a character in a pod and
//! reports the seat, so this file calls the game crate's own
//! [`tacsim::match_::slot_seated`] directly, with a `Ctx` built the way
//! `mushd`'s own end-to-end tests build one. That is the **only** seam this file
//! reaches through: every other transition is a real `SIM` line dispatched
//! through the real command table, against the real gate.

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    AccountId, AccountIdentity, Audience, Ctx, GOD, ObjId, Outbox, ParsedArgs, SwitchSet,
};
use mush_proto::OutputEvent;
use mush_testkit::TestWorld;
use tacsim::TacsimConfig;
use tacsim::arena::state::Team;
use tacsim::match_::{self, MatchState};

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

/// `[tacsim].enabled`, with a fast arena and a short boarding window so a test
/// runs in milliseconds rather than in the two minutes doc 16 ships.
fn config() -> TacsimConfig {
    let mut cfg = TacsimConfig {
        enabled: true,
        ..TacsimConfig::default()
    };
    cfg.arena.tempo.quiet_ms = 100;
    cfg.arena.tempo.alert_ms = 100;
    cfg.arena.tempo.engaged_ms = 100;
    cfg.match_.startup_ms = 200;
    cfg.match_.board_timeout_ms = 5_000;
    cfg
}

/// The production roster, composed exactly as `mush_server::boot` composes it,
/// with the starter area on (the centre is in the OOC pack) and the game's own
/// boot packs on (the arena, its ground and its eight tanks).
///
/// The character sheet is left out for the same reason
/// `crates/tacsim/tests/definitions.rs` leaves it out of its reboot roster: it
/// registers a `pilot.chargen` job payload handler that cannot be registered
/// twice in one process, and several of these tests boot more than one world.
/// The sheet has nothing to do with the lobby.
fn roster() -> Vec<Box<dyn mush_core::GameModule>> {
    roster_with(config())
}

/// **16b cut Q.7**: the same roster on a named `[tacsim]`, so a test can boot a
/// centre with `[tacsim.match].ai_fill` on.
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

/// One world with the centre in it and `n` pilots standing in the lobby, each on
/// an account of its own.
struct Rig {
    tw: TestWorld,
    pilots: Vec<ObjId>,
    /// The arena's egress, taken once, exactly as `mush_server::world_task`
    /// takes it at its first iteration. A `TestWorld` has no loop, so
    /// [`Rig::pump`] is the loop's `Wake::ArenaEgress` arm, written out.
    egress: Option<tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>>,
}

impl Rig {
    fn boot(pilots: usize) -> Rig {
        Rig::boot_on(pilots, roster())
    }

    /// **16b cut Q.7**: the same rig on a named roster, so a test can boot a
    /// centre with the AI fill on.
    fn boot_on(pilots: usize, modules: Vec<Box<dyn mush_core::GameModule>>) -> Rig {
        let mut tw = TestWorld::with_modules(modules);
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
        Rig {
            tw,
            pilots: who,
            egress,
        }
    }

    /// **The world loop's `Wake::ArenaEgress` arm**, by hand: drain whatever the
    /// arena task has pushed, park it in the shared table, and step the world
    /// once with the module's own re-entry token. Without it a `TestWorld` never
    /// hears the arena's `Ended` and a check-in never runs.
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
        // The ledger writes the step queued would ride the next batch on a real
        // server; here nothing commits, so they are stamped and acknowledged at
        // once, which is what keeps a short match from backing its egress up.
        tasks.stamp_ledger_writes(1);
        tasks.ack_ledger_upto(1);
    }

    fn pilot(&self, n: usize) -> ObjId {
        *self.pilots.get(n).expect("that many pilots were booted")
    }

    /// Run one line as a pilot and return everything that pilot saw.
    fn say(&mut self, n: usize, line: &str) -> String {
        let actor = self.pilot(n);
        let events = self.tw.run(actor, line);
        self.seen(actor, &events)
    }

    /// Run one line as God (staff).
    fn staff(&mut self, line: &str) -> String {
        let events = self.tw.run(GOD, line);
        self.seen(GOD, &events)
    }

    fn seen(&self, viewer: ObjId, events: &[(Audience, OutputEvent)]) -> String {
        self.tw.seen_by(events, viewer).join("\n")
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

    /// The object one pilot's locked pod is.
    fn pod(&self, n: usize) -> Option<ObjId> {
        match_::pod_of(self.tw.world(), self.pilot(n))
    }

    /// Move a pilot out of the centre entirely, so the out-of-place refusal can
    /// be seen to bite.
    fn send_to_limbo(&mut self, n: usize) {
        let pilot = self.pilot(n);
        self.tw
            .world_mut()
            .move_to(pilot, mush_core::LIMBO)
            .expect("Limbo takes everybody");
    }

    /// **Seat one pilot**, standing in for cut K.2's `BOARD` (see this file's
    /// header). Answers whether this seating fired `Boarding → Running`.
    fn seat(&mut self, n: usize) -> bool {
        let pilot = self.pilot(n);
        let pod = self.pod(n).expect("the pilot's pod is locked");
        let unit = match_::tank_of(self.tw.world(), pod).expect("the pod's tank is in the world");
        let match_id = self
            .centre()
            .match_id
            .expect("a boarding centre has a match id");
        let mut out = Outbox::new();
        let engine = self.tw.engine_mut();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: pilot,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        match_::slot_seated(&mut ctx, match_id, pilot, unit).expect("the seat is reported")
    }

    /// **Un-seat one pilot**, standing in for cut K.2's `EXIT` during
    /// `Boarding`.
    fn vacate(&mut self, n: usize) {
        let pilot = self.pilot(n);
        let match_id = self
            .centre()
            .match_id
            .expect("a boarding centre has a match id");
        let mut out = Outbox::new();
        let engine = self.tw.engine_mut();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: pilot,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        match_::slot_vacated(&mut ctx, match_id, pilot).expect("the vacancy is reported");
    }

    /// **End the running match the way the egress does** (16b cut Q.7):
    /// `arena_bridge`'s `Ended` arm reduced to the one call that puts the reason
    /// on the board. Nothing is returned, because what this is for is the
    /// centre's half and not the cells.
    fn end_match(&mut self, reason: tacsim::arena::EndReason) {
        let match_id = self
            .centre()
            .match_id
            .expect("a running centre has a match id");
        let mut out = Outbox::new();
        let engine = self.tw.engine_mut();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor: GOD,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        let end = tacsim::arena::MatchEnd {
            reason,
            t_ms: 12_000,
        };
        assert!(
            match_::finish(&mut ctx, match_id, end, "0".repeat(64), &[]),
            "the match had a live record to check in"
        );
    }

    /// **Which pods the running match actually fielded** (16b cut Q.7), as
    /// `(team, pod index)` — the AI fill's units never enter `centre.slots`, so
    /// the board cannot answer this and the registry's own unit binding can.
    fn match_units(&self) -> Vec<(Team, u8)> {
        let w = self.tw.world();
        let Some(id) = match_::centre_of(w).and_then(|centre| centre.match_id) else {
            return Vec::new();
        };
        let registry = w.module_state::<tacsim::arena_bridge::ArenaRegistry>();
        let mut out = Vec::new();
        for team in Team::ALL {
            for slot in 1..=match_::PODS_PER_SIDE {
                if let Some(tank) = match_::tank_for_slot(w, team, slot)
                    && registry.and_then(|registry| registry.match_of_unit(tank)) == Some(id)
                {
                    out.push((team, slot));
                }
            }
        }
        out
    }

    /// Fire the module ticks due at `seconds`, which is what closes boarding.
    fn tick_to(&mut self, seconds: i64) {
        self.tw.set_now(seconds);
        self.tw.fire_due(seconds.saturating_mul(1_000));
    }
}

// ---------------------------------------------------------------------------
// (1) The whole walk, twice
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_pilots_walk_the_whole_machine_twice_in_a_row() {
    let mut rig = Rig::boot(2);
    for run in 1..=2 {
        assert_eq!(rig.state(), MatchState::Idle, "run {run} starts idle");

        // Idle -> Selection.
        let said = rig.say(0, "sim team red");
        assert!(said.contains("red slot 1"), "run {run}: {said}");
        assert_eq!(rig.state(), MatchState::Selection);
        rig.say(1, "sim team blue");
        assert_eq!(rig.centre().slots.len(), 2, "run {run}");

        // The status a pilot reads, anywhere in the centre.
        let status = rig.say(0, "sim status");
        assert!(status.contains("selection"), "run {run}: {status}");
        assert!(status.contains("Pilot0"), "run {run}: {status}");
        assert!(status.contains("Pilot1"), "run {run}: {status}");

        // Selection -> Boarding: the pods lock, slot n to pod n.
        let said = rig.say(0, "sim start");
        assert!(said.contains("boarding"), "run {run}: {said}");
        assert_eq!(rig.state(), MatchState::Boarding);
        let red_pod = match_::pod_object(rig.tw.world(), Team::Red, 1).expect("red pod 1");
        let blue_pod = match_::pod_object(rig.tw.world(), Team::Blue, 1).expect("blue pod 1");
        assert_eq!(rig.pod(0), Some(red_pod), "run {run}");
        assert_eq!(rig.pod(1), Some(blue_pod), "run {run}");
        assert_eq!(
            match_::pilot_of(rig.tw.world(), red_pod),
            Some(rig.pilot(0)),
            "run {run}"
        );

        // Boarding -> Running: the named trigger, when every claimed slot is
        // seated and not before.
        assert!(!rig.seat(0), "run {run}: one of two is not every one");
        assert_eq!(rig.state(), MatchState::Boarding);
        assert!(rig.seat(1), "run {run}: the second seat fires the trigger");
        assert_eq!(rig.state(), MatchState::Running);

        // The units the arena took are the tanks of the locked pods.
        let red_tank = match_::tank_of(rig.tw.world(), red_pod).expect("red pod 1's tank");
        assert!(
            mush_sim::deployed::mark_of(rig.tw.world(), red_tank).is_some(),
            "run {run}: the tank is checked out"
        );

        // Running -> Aborted, by a participant.
        let said = rig.say(1, "sim abort");
        assert!(said.contains("aborts match"), "run {run}: {said}");

        // The arena answers on its own thread; the centre lands in `aborted`
        // once the check-in has run, and the units are back.
        for _ in 0..400 {
            if rig.state() == MatchState::Aborted {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            rig.pump();
        }
        assert_eq!(
            rig.state(),
            MatchState::Aborted,
            "run {run}: the check-in lands"
        );
        assert!(
            mush_sim::deployed::mark_of(rig.tw.world(), red_tank).is_none(),
            "run {run}: custody is back"
        );
        assert!(
            rig.centre().slots.is_empty(),
            "run {run}: the pods are free"
        );

        // Aborted -> Idle, which is the reset, and which run 2 then proves.
        let said = rig.staff("@arena/reset");
        assert!(said.contains("is reset"), "run {run}: {said}");
        assert_eq!(rig.state(), MatchState::Idle);
    }
}

// ---------------------------------------------------------------------------
// (2) One slot per account, and one account per slot
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_account_takes_one_slot_and_a_taken_slot_is_not_taken_twice() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team red");
    let centre = rig.centre();
    assert_eq!(centre.side_len(Team::Red), 3, "three accounts, three slots");
    let indices: Vec<u8> = centre.slots.iter().map(|slot| slot.index).collect();
    assert_eq!(indices, vec![1, 2, 3], "the lowest free slot each time");

    // The same account again, on either side.
    let said = rig.say(0, "sim team blue");
    assert!(said.contains("already hold red slot 1"), "{said}");
    assert_eq!(rig.centre().slots.len(), 3, "and nothing was written");

    // The fourth pod is the last one; a fifth claim on that side is refused by
    // the pods that exist, not by the configured 12.
    let mut rig = Rig::boot(5);
    for n in 0..4 {
        rig.say(n, "sim team red");
    }
    let said = rig.say(4, "sim team red");
    assert!(said.contains("red side is full"), "{said}");
    assert!(said.contains("4 pilot(s)"), "{said}");
    assert_eq!(rig.centre().side_len(Team::Red), 4);
}

// ---------------------------------------------------------------------------
// (3) `SIM START` with an empty side
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_with_an_empty_side_refuses_and_mints_no_match() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    let said = rig.say(0, "sim start");
    assert!(said.contains("blue side has no pilots"), "{said}");
    assert_eq!(rig.state(), MatchState::Selection, "nothing moved");
    assert!(rig.centre().match_id.is_none(), "and no id was minted");

    // Both sides manned, and only a lead may start it.
    rig.say(1, "sim leave");
    rig.say(1, "sim team blue");
    let said = rig.say(1, "sim start");
    assert!(said.contains("boarding"), "the blue lead starts it: {said}");
    assert_eq!(rig.state(), MatchState::Boarding);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn only_a_sides_first_pilot_or_staff_may_start_it() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(2, "sim team blue");

    // Pilot2 claimed blue second, so it leads nothing.
    let said = rig.say(2, "sim start");
    assert!(said.contains("first pilot or staff"), "{said}");
    assert_eq!(rig.state(), MatchState::Selection);

    // The blue lead leaves; the next blue claimant inherits the lead, so a side
    // is never left with nobody who can start it.
    rig.say(1, "sim leave");
    let said = rig.say(2, "sim start");
    assert!(said.contains("boarding"), "the heir starts it: {said}");
    assert_eq!(rig.state(), MatchState::Boarding);
}

// ---------------------------------------------------------------------------
// (4) The boarding deadline
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_boarding_that_times_out_drops_the_unseated_and_aborts_when_a_side_empties() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);

    // Red seats; blue never does. The deadline drops blue's slot, which empties
    // the blue side, and an empty side aborts (doc 16 §3.8).
    assert!(!rig.seat(0));
    rig.tick_to(10);
    assert_eq!(rig.state(), MatchState::Aborted);
    assert!(rig.centre().slots.is_empty(), "the pods are free again");
    let red_tank = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    assert!(
        mush_sim::deployed::mark_of(rig.tw.world(), red_tank).is_none(),
        "nothing was ever checked out, so nothing is deployed"
    );

    // And the tick is idle on a centre that is not boarding: firing it again
    // changes nothing.
    rig.tick_to(20);
    assert_eq!(rig.state(), MatchState::Aborted);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_boarding_that_times_out_with_both_sides_seated_runs_with_who_is_in() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(2, "sim team blue");
    rig.say(0, "sim start");
    assert!(!rig.seat(0));
    assert!(!rig.seat(1));
    // Pilot2 holds blue slot 2 and never boards.
    rig.tick_to(10);
    assert_eq!(rig.state(), MatchState::Running, "one blue is a blue side");
    assert_eq!(rig.centre().slots.len(), 2, "the unseated slot is dropped");
    assert!(
        rig.centre()
            .slots
            .iter()
            .all(|slot| slot.pilot != rig.pilot(2))
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pilot_who_steps_back_out_of_a_pod_is_not_seated_any_more() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(2, "sim team blue");
    rig.say(0, "sim start");

    // Two of three board; the third's seat is what the trigger waits on.
    assert!(!rig.seat(0));
    assert!(!rig.seat(1));
    // Blue 1 steps back out before blue 2 is in: the trigger must not fire when
    // blue 2 boards, because blue 1 is no longer in a pod.
    rig.vacate(1);
    assert!(!rig.seat(2), "one of the three is out of its pod again");
    assert_eq!(rig.state(), MatchState::Boarding);
    assert!(rig.seat(1), "and it fires when they are all back in");
    assert_eq!(rig.state(), MatchState::Running);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_leave_during_boarding_fires_the_trigger_or_aborts_the_match() {
    // The leave that completes the boarding.
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(2, "sim team blue");
    rig.say(0, "sim start");
    assert!(!rig.seat(0));
    assert!(!rig.seat(1));
    rig.say(2, "sim leave");
    assert_eq!(
        rig.state(),
        MatchState::Running,
        "everybody left is seated, so the trigger owes an answer now"
    );

    // The leave that empties a side.
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert!(!rig.seat(0));
    rig.say(1, "sim leave");
    assert_eq!(rig.state(), MatchState::Aborted);
    assert!(rig.centre().slots.is_empty());
}

// ---------------------------------------------------------------------------
// (5) `SIM ABORT` from each state
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_from_selection_releases_slots_and_nothing_else() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim abort");
    assert!(said.contains("calls the match off"), "{said}");
    assert_eq!(rig.state(), MatchState::Aborted);
    assert!(rig.centre().slots.is_empty());
    assert!(
        rig.centre().match_id.is_none(),
        "there was never a match to have an id"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_from_boarding_releases_the_slots_and_the_pods() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    let locked = rig.pod(0).expect("a pod is locked in boarding");
    assert_eq!(match_::pilot_of(rig.tw.world(), locked), Some(rig.pilot(0)));

    rig.say(1, "sim abort");
    assert_eq!(rig.state(), MatchState::Aborted);
    assert_eq!(
        match_::pilot_of(rig.tw.world(), locked),
        None,
        "the pod is free"
    );
    let red_tank = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    assert!(
        mush_sim::deployed::mark_of(rig.tw.world(), red_tank).is_none(),
        "doc 16 §3.8: an abort before Running has no custody to return"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_bystander_cannot_abort_and_staff_can() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(2, "sim abort");
    assert!(said.contains("hold no slot"), "{said}");
    assert_eq!(rig.state(), MatchState::Selection, "and it is untouched");

    // God is in Limbo, not in the centre, so even staff are refused there —
    // the gate is about the room, not about the power.
    let said = rig.staff("sim abort");
    assert!(said.contains(match_::OUTSIDE_CENTRE), "{said}");

    let lobby = match_::centre_object(rig.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
    rig.tw
        .world_mut()
        .move_to(GOD, lobby)
        .expect("staff walk in");
    let said = rig.staff("sim abort");
    assert!(said.contains("calls the match off"), "{said}");
    assert_eq!(rig.state(), MatchState::Aborted);
}

// ---------------------------------------------------------------------------
// (6) Where `SIM` works
//
// 16b cut F.4 deleted the command gate that used to answer this; `sim_command`
// checks the location itself now, so the answers below are the verb's own.
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sim_outside_the_centre_is_refused_with_one_line() {
    let mut rig = Rig::boot(1);
    rig.send_to_limbo(0);
    // Every spelling, including the two that do not parse: with the gate gone,
    // the verb's own location check has to run BEFORE the parser, or `sim
    // wibble` answers with a usage block and a bare `sim team` asks "Which
    // side?" to a player standing in Limbo.
    for line in [
        "sim",
        "sim status",
        "sim team red",
        "sim/status",
        "sim wibble",
        "sim team",
        "sim/team",
        "sim start",
        "sim leave",
        "sim abort",
    ] {
        let said = rig.say(0, line);
        assert!(
            said.contains(match_::OUTSIDE_CENTRE),
            "`{line}` outside the centre: {said}"
        );
        assert_eq!(
            said.lines()
                .filter(|row| row.contains(match_::OUTSIDE_CENTRE))
                .count(),
            1,
            "exactly one line: {said}"
        );
        for other in ["Usage:", "Which side?", "Simulator centre"] {
            assert!(
                !said.contains(other),
                "`{line}` outside the centre also said `{other}`: {said}"
            );
        }
    }
    assert_eq!(rig.state(), MatchState::Idle, "and nothing was claimed");

    // A world that HAS a centre is what this refusal is for; a world without one
    // gets the true answer instead of a corridor that is not there.
    assert!(
        match_::centre_rooms(rig.tw.world()).is_some(),
        "the rig's OOC pack carries the centre"
    );

    // A pilot **inside a pod** is in that pod's team room as far as the check is
    // concerned — which is the seat cut K.2's `BOARD` leaves them in, and the
    // one place `SIM ABORT` most needs to work from.
    let pod = match_::pod_object(rig.tw.world(), Team::Blue, 2).expect("blue pod 2");
    let pilot = rig.pilot(0);
    rig.tw
        .world_mut()
        .move_to(pilot, pod)
        .expect("a pilot climbs into a pod");
    assert_eq!(
        match_::where_in_centre(rig.tw.world(), pilot),
        Some(match_::CentreRoom::Team(Team::Blue))
    );
    let said = rig.say(0, "sim status");
    assert!(!said.contains(match_::OUTSIDE_CENTRE), "{said}");

    // The same verb, in a team room, is the lobby's own.
    let red = match_::centre_object(rig.tw.world(), match_::RED_ROOM_ENTRY).expect("the red room");
    let pilot = rig.pilot(0);
    rig.tw
        .world_mut()
        .move_to(pilot, red)
        .expect("the pilot walks in");
    let said = rig.say(0, "sim status");
    assert!(!said.contains(match_::OUTSIDE_CENTRE), "{said}");
    assert!(said.contains("Simulator centre"), "{said}");
}

// ---------------------------------------------------------------------------
// (7) The staff reset, and the staff shortcut
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reset_after_an_end_is_idempotent_and_says_so() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim abort");
    assert_eq!(rig.state(), MatchState::Aborted);

    let said = rig.staff("@arena/reset");
    assert!(said.contains("is reset"), "{said}");
    assert_eq!(rig.state(), MatchState::Idle);

    let said = rig.staff("@arena/reset");
    assert!(said.contains("already idle; nothing changed"), "{said}");
    assert_eq!(rig.state(), MatchState::Idle);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_staff_shortcut_and_the_lobby_read_one_state() {
    let mut rig = Rig::boot(1);
    let red = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), Team::Blue, 1).expect("blue 1's tank");
    let said = rig.staff(&format!(
        "@arena/start {}=#{}/#{}",
        match_::CENTRE_ARENA,
        red.0,
        blue.0
    ));
    assert!(said.contains("is running on"), "{said}");
    assert_eq!(
        rig.state(),
        MatchState::Running,
        "the shortcut goes through the machine"
    );

    // And `@arena/status` prints the centre the lobby would print.
    let said = rig.staff("@arena/status");
    assert!(said.contains("Simulator centre: running"), "{said}");

    // A second shortcut is refused while the first is checked out.
    let said = rig.staff(&format!(
        "@arena/start {}=#{}/#{}",
        match_::CENTRE_ARENA,
        red.0,
        blue.0
    ));
    assert!(said.contains("holding a match"), "{said}");
    // As is a lobby claim.
    let said = rig.say(0, "sim team red");
    assert!(said.contains("centre is running"), "{said}");

    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("holding a match"),
        "a reset never drops custody: {said}"
    );
    rig.staff("@arena/abort");
}

// ---------------------------------------------------------------------------
// The content ladder itself
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn every_pod_resolves_to_its_own_tank_by_id_and_never_by_name() {
    let rig = Rig::boot(0);
    let mut seen: Vec<ObjId> = Vec::new();
    for team in Team::ALL {
        for index in 1..=match_::PODS_PER_SIDE {
            let pod = match_::pod_object(rig.tw.world(), team, index)
                .unwrap_or_else(|| panic!("{} pod {index}", team.as_str()));
            assert_eq!(match_::pod_slot(rig.tw.world(), pod), Some((team, index)));
            let tank = match_::tank_of(rig.tw.world(), pod)
                .unwrap_or_else(|| panic!("{} pod {index}'s tank", team.as_str()));
            assert!(!seen.contains(&tank), "eight pods, eight distinct tanks");
            seen.push(tank);
            // The shipped pack names them so; the resolution above used the
            // slot id, so a rename here changes the label and nothing else.
            let name = rig
                .tw
                .world()
                .get(tank)
                .map(|object| object.name.to_string())
                .unwrap_or_default();
            let expected = format!(
                "{} Tank {index}",
                match team {
                    Team::Red => "Red",
                    Team::Blue => "Blue",
                }
            );
            assert_eq!(name, expected);
        }
    }
    assert_eq!(seen.len(), 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_renamed_tank_is_still_its_slots_tank() {
    let mut rig = Rig::boot(0);
    let pod = match_::pod_object(rig.tw.world(), Team::Red, 3).expect("red pod 3");
    let tank = match_::tank_of(rig.tw.world(), pod).expect("red 3's tank");
    rig.tw
        .world_mut()
        .set_name(tank, "Old Faithful")
        .expect("staff rename a tank");
    assert_eq!(
        match_::tank_of(rig.tw.world(), pod),
        Some(tank),
        "the slot id is the identity, not the label"
    );
}

// ---------------------------------------------------------------------------
// The layer switch (16b cut F.4)
// ---------------------------------------------------------------------------

/// The shipped **off** shape: the module composes, the verbs register, and the
/// game's own boot packs do not ride along — which is what
/// `mush_server::boot::game_modules` does with `[tacsim].enabled = false`.
///
/// The centre's rooms and pods still exist: they are in the starter area, which
/// is a different key. That is the whole trap this test is about.
fn roster_with_the_layer_off() -> Vec<Box<dyn mush_core::GameModule>> {
    use tacsim::composition as comp;

    let llm = Arc::new(
        mush_llm::LlmService::from_config(&mush_llm::LlmConfig::default())
            .expect("a disabled LLM service composes"),
    );
    let onboarding = mush_core::OnboardingConfig::default();
    let game = tacsim::GameEconomyConfig::default();
    let cfg = TacsimConfig::default();
    assert!(!cfg.enabled, "the documented default is off");
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
    // `game_content` false: the starter area, and none of the game's packs.
    mods.push(comp::content_module_with(true, false).expect("the content module composes"));
    mods
}

/// **With the layer off, every lobby verb refuses at the word the player
/// typed** (16b cut F.4).
///
/// Before this cut the verbs registered unconditionally and then checked
/// nothing: two players in a centre with no arena behind it could both claim a
/// slot, `SIM STATUS` rendered a healthy board, and only `SIM START` refused —
/// with "staff must re-apply the arena", naming a repair no verb in the tree can
/// perform. Every refusal below is the first thing the verb does.
#[test]
fn f4_with_the_layer_off_the_lobby_refuses_out_loud() {
    const OFF: &str = "The TacSim layer is off on this server";

    let mut tw = TestWorld::with_modules(roster_with_the_layer_off());
    // The centre is here — rooms, pods and all — because it is starter-area
    // content and not configuration.
    let lobby = match_::centre_object(tw.world(), match_::LOBBY_ENTRY)
        .expect("the OOC pack carries the simulator lobby whatever the layer says");
    let red_room =
        match_::centre_object(tw.world(), match_::RED_ROOM_ENTRY).expect("and the red room");
    let pilot = tw.spawn_player("Pilot0");
    tw.world_mut()
        .move_to(pilot, lobby)
        .expect("a player walks into the lobby");
    let mut map = Accounts::default();
    map.0.insert(
        pilot,
        AccountId::mint("account-off", 1).expect("an account id"),
    );
    *tw.world_mut()
        .module_state_mut::<mush_sim::AccountOracle>()
        .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));

    let said = |tw: &mut TestWorld, line: &str| -> String {
        let events = tw.run(pilot, line);
        tw.seen_by(&events, pilot).join("\n")
    };

    // The claim, which is where the old path let the player through.
    let seen = said(&mut tw, "SIM TEAM red");
    assert!(seen.contains(OFF), "SIM TEAM: {seen}");
    // The board, which used to render an idle centre over nothing.
    let seen = said(&mut tw, "SIM STATUS");
    assert!(seen.contains(OFF), "SIM STATUS: {seen}");
    // The other three selectors, each at its own first line.
    for line in ["SIM LEAVE", "SIM START", "SIM ABORT"] {
        let seen = said(&mut tw, line);
        assert!(seen.contains(OFF), "{line}: {seen}");
    }
    // And boarding, from the room boarding happens in.
    tw.world_mut()
        .move_to(pilot, red_room)
        .expect("a player walks into the red room");
    let seen = said(&mut tw, "BOARD");
    assert!(seen.contains(OFF), "BOARD: {seen}");

    // Nothing was claimed on the way: the centre never left `Idle`.
    assert_eq!(
        match_::centre_of(tw.world()).map(|centre| centre.state),
        Some(MatchState::Idle)
    );
    assert!(
        match_::centre_of(tw.world()).is_some_and(|centre| centre.slots.is_empty()),
        "a slot was claimed on a server with no arena"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Q.7 — one human against the machines
// ---------------------------------------------------------------------------

/// The same short config with `[tacsim.match].ai_fill` **on**.
fn fill_config() -> TacsimConfig {
    let mut cfg = config();
    cfg.match_.ai_fill = true;
    cfg
}

/// A centre on a server that fields machines.
fn fill_rig(pilots: usize) -> Rig {
    Rig::boot_on(pilots, roster_with(fill_config()))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_with_the_fill_on_one_claimed_slot_starts_a_match() {
    // This reverses the ruling `lobby::run` used to carry. It was written when
    // no human could be alone in a centre; the owner has asked for exactly one
    // human against the machines, and what is left of the old rule is that a
    // match still needs somebody playing it.
    let mut rig = fill_rig(1);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim start");
    assert!(said.contains("boarding"), "one pilot starts it: {said}");
    assert_eq!(rig.state(), MatchState::Boarding);
    assert!(rig.centre().match_id.is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_with_the_fill_off_an_empty_side_is_still_refused_byte_for_byte() {
    let mut rig = Rig::boot(1);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim start");
    assert!(
        said.contains("The blue side has no pilots; a match takes at least one a side."),
        "{said}"
    );
    assert_eq!(rig.state(), MatchState::Selection);
    assert!(rig.centre().match_id.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_the_default_fill_is_the_larger_side_and_sim_ai_sets_it() {
    let mut rig = fill_rig(3);
    rig.say(0, "sim team red");
    assert_eq!(
        tacsim::match_::ai_side_size(&rig.centre(), &fill_config().match_),
        1,
        "one pilot alone is a one against one"
    );
    rig.say(1, "sim team red");
    assert_eq!(
        tacsim::match_::ai_side_size(&rig.centre(), &fill_config().match_),
        2,
        "two friends on red get two machines on blue"
    );

    // The board says so, in the one line the slots cannot.
    let said = rig.say(0, "sim status");
    assert!(
        said.contains("  AI fill: each side fields 2; machines take the unclaimed pods."),
        "{said}"
    );

    // And the lead can ask for more.
    let said = rig.say(0, "sim ai 3");
    assert!(said.contains("AI fill set to 3 a side."), "{said}");
    assert_eq!(rig.centre().ai_side_size, Some(3));
    let said = rig.say(0, "sim ai");
    assert!(said.contains("AI fill: 3 a side."), "{said}");
    let said = rig.say(0, "sim status");
    assert!(
        said.contains("  AI fill: each side fields 3; machines take the unclaimed pods."),
        "{said}"
    );
}

/// **The size somebody set is a floor and not a cap** (16b cut Q.7).
///
/// `SIM AI` refuses a size below what is claimed, but claiming goes on for the
/// whole of `Selection`: a lead who sized the fill and then watched two friends
/// claim would otherwise start a 3 v 1 under a board line reading `each side
/// fields 1`, which is the one thing the refusal exists to prevent.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_a_claim_after_sim_ai_raises_the_fill_rather_than_splitting_the_sides() {
    let mut rig = fill_rig(3);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim ai 1");
    assert!(said.contains("AI fill set to 1 a side."), "{said}");
    assert_eq!(rig.centre().ai_side_size, Some(1));

    rig.say(1, "sim team red");
    rig.say(2, "sim team red");
    assert_eq!(
        tacsim::match_::ai_side_size(&rig.centre(), &fill_config().match_),
        3,
        "the three that turned up are what both sides field"
    );
    let said = rig.say(0, "sim status");
    assert!(
        said.contains("  AI fill: each side fields 3; machines take the unclaimed pods."),
        "the board says the number the check-out will use: {said}"
    );

    rig.say(0, "sim start");
    for n in 0..3 {
        rig.seat(n);
    }
    assert_eq!(rig.state(), MatchState::Running);
    let units = rig.match_units();
    assert_eq!(units.len(), 6, "three a side: {units:?}");
    assert_eq!(
        units.iter().filter(|(team, _)| *team == Team::Blue).count(),
        3,
        "blue was filled to the size red brought: {units:?}"
    );
}

/// **A dry field is a draw the board can explain** (16b cut Q.7).
///
/// [`tacsim::arena::EndReason::Spent`] maps to `MatchOutcome::Draw`, which is
/// the result document's own word (doc 16 §4) — so the board carries the arena's
/// reason beside it, and this is the one place that sentence is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_a_spent_end_is_a_draw_the_board_explains() {
    let mut rig = fill_rig(1);
    rig.say(0, "sim team red");
    rig.say(0, "sim start");
    assert!(rig.seat(0), "the one pilot boards and the match runs");
    rig.end_match(tacsim::arena::EndReason::Spent);
    assert_eq!(rig.state(), MatchState::Ended);
    assert_eq!(
        rig.centre().outcome_reason,
        Some(tacsim::arena::EndReason::Spent),
        "the reason is written after the rows are released, not before"
    );
    let said = rig.say(0, "sim status");
    assert!(
        said.contains("— draw: both sides are out of ammunition"),
        "{said}"
    );

    // And a draw that is not a dry field is the bare word, so the sentence
    // above is about the reason and not about the outcome.
    let mut rig = fill_rig(1);
    rig.say(0, "sim team red");
    rig.say(0, "sim start");
    assert!(rig.seat(0));
    rig.end_match(tacsim::arena::EndReason::Elimination { winner: None });
    let said = rig.say(0, "sim status");
    assert!(said.contains("— draw"), "{said}");
    assert!(
        !said.contains("out of ammunition"),
        "a mutual elimination is not a dry field: {said}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_sim_ai_refuses_by_the_sentence_for_each_reason() {
    let mut rig = fill_rig(3);
    // Outside `Selection` there is nothing to size.
    let said = rig.say(0, "sim ai 2");
    assert!(
        said.contains("that is not something you can do to it now"),
        "{said}"
    );

    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    // Not a lead.
    let said = rig.say(1, "sim ai 3");
    assert!(
        said.contains("Only a side's first pilot or staff can start the match."),
        "{said}"
    );
    // Out of range, both ends: the centre has four pods a side.
    let said = rig.say(0, "sim ai 0");
    assert!(said.contains("AI fill is 1 to 4 a side."), "{said}");
    let said = rig.say(0, "sim ai 5");
    assert!(said.contains("AI fill is 1 to 4 a side."), "{said}");
    // And never below what a side already holds.
    let said = rig.say(0, "sim ai 1");
    assert!(
        said.contains("AI fill cannot be below the larger side (2)."),
        "{said}"
    );
    assert_eq!(rig.centre().ai_side_size, None, "nothing was written");
    // A word that is not a number is the usage line and never a silent read.
    let said = rig.say(0, "sim ai four");
    assert!(said.contains("How many a side?"), "{said}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_with_the_fill_off_sim_ai_says_so_and_the_board_says_nothing() {
    let mut rig = Rig::boot(1);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim ai");
    assert!(said.contains("AI fill is off on this server."), "{said}");
    let said = rig.say(0, "sim status");
    assert!(
        !said.contains("AI fill:"),
        "a board on a server with no fill says nothing about one: {said}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_a_solo_match_fields_one_machine_and_its_pod_keeps_its_designator() {
    let mut rig = fill_rig(1);
    rig.say(0, "sim team red");
    rig.say(0, "sim start");
    assert!(rig.seat(0), "the one pilot boards");
    assert_eq!(rig.state(), MatchState::Running);
    let units = rig.match_units();
    assert_eq!(units.len(), 2, "one a side: {units:?}");
    // The machine took blue's LOWEST unclaimed pod, so its designator is `BA`.
    assert!(
        units
            .iter()
            .any(|(team, slot)| *team == Team::Blue && *slot == 1),
        "the fill takes the lowest unclaimed pod: {units:?}"
    );
    let said = rig.say(0, "sim status");
    assert!(
        said.contains("  AI fill: each side fields 1; machines take the unclaimed pods."),
        "the board says it while the match runs: {said}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q7_the_boarding_deadline_runs_with_one_seated_pilot_and_aborts_with_none() {
    // One of two boards; the other never does. With the fill on that is a match,
    // because somebody is playing it.
    let mut rig = fill_rig(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert!(
        !rig.seat(0),
        "one of two seated is not the trigger; the deadline is"
    );
    assert_eq!(rig.state(), MatchState::Boarding, "still waiting on blue");
    rig.tick_to(6);
    assert_eq!(rig.state(), MatchState::Running, "it ran with who was in");

    // Nobody at all boards, and it is an abort — the half of the old rule that
    // survives.
    let mut rig = fill_rig(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    rig.tick_to(6);
    assert_eq!(rig.state(), MatchState::Aborted);
    let said = rig.say(0, "sim status");
    assert!(said.contains("aborted"), "{said}");
}
