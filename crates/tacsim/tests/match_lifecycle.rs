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

mod support;

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
/// boot packs on (the arena, its ground and its twelve tanks).
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
    /// **16b cut V.2**: every ledger chunk [`Rig::pump`] has handed the world,
    /// by match — the count the result document's `ledger.chunks` must equal.
    chunks_seen: BTreeMap<u64, u64>,
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
            chunks_seen: BTreeMap::new(),
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
                        let chunk_of = match &message {
                            tacsim::arena_task::ArenaOut::Ledger { match_id, .. } => {
                                Some(match_id.0)
                            }
                            _ => None,
                        };
                        if !tasks.stage(message) {
                            break;
                        }
                        if let Some(match_id) = chunk_of {
                            *self.chunks_seen.entry(match_id).or_default() += 1;
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

    /// **16b cut Q.8**: run one line as pilot `n` and answer what pilot `other`
    /// heard of it — the audience half of a sentence, which is the only way to
    /// tell an announced line from one said to the actor alone.
    fn say_overheard(&mut self, n: usize, line: &str, other: usize) -> String {
        let actor = self.pilot(n);
        let listener = self.pilot(other);
        let events = self.tw.run(actor, line);
        self.seen(listener, &events)
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
        self.end_match_returning(reason, &[]);
    }

    /// The same end, with the cells the arena hands back on it (16b cut D.3):
    /// what tells a **committed** check-in from a void one, which returns every
    /// unit to its check-out snapshot and leaves the same cleared marker behind.
    fn end_match_returning(
        &mut self,
        reason: tacsim::arena::EndReason,
        returned: &[tacsim::arena::damage::UnitReturn],
    ) {
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
            match_::finish(&mut ctx, match_id, end, "0".repeat(64), returned),
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

    // The sixth pod is the last one (16b cut Z.3); a seventh claim on that side
    // is refused by the pods that exist, not by the configured 12.
    let mut rig = Rig::boot(7);
    for n in 0..6 {
        rig.say(n, "sim team red");
    }
    let said = rig.say(6, "sim team red");
    assert!(said.contains("red side is full"), "{said}");
    assert!(said.contains("6 pilot(s)"), "{said}");
    assert_eq!(rig.centre().side_len(Team::Red), 6);
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
    assert!(
        said.contains("Only a side's lead or staff can start the match."),
        "{said}"
    );
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
    // 16b cut AA.2 (`B-78`): a dropped NON-lead moves neither lead.
    assert!(
        rig.centre().leads_side(Team::Red, account(0)),
        "red's lead is still its first claimant"
    );
    assert!(
        rig.centre().leads_side(Team::Blue, account(1)),
        "blue's lead is still its first claimant"
    );
}

/// The account the rig minted for pilot `n` ([`Rig::boot_on`]'s map) — the one
/// way to name a pilot whose slot the deadline has already dropped.
fn account(n: usize) -> AccountId {
    AccountId::mint(&format!("account-{n}"), n as i64).expect("an account id")
}

/// **The deadline drops a side's lead, and the side still has a lead** (16b cut
/// AA.2, 16c `B-78`).
///
/// Before AA.2 `board_deadline` dropped the unseated slots and kept `leads` as
/// it stood, so red's entry named an account with no slot: red's survivor read
/// `Only a side's first pilot can concede for it.` and the dropped lead read `You
/// hold no slot in the simulator.`, and nobody could concede for red for the
/// rest of the match. The OTHER side's lead starts it, because the trigger does
/// not depend on who typed `SIM START`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aa2_the_deadline_drops_a_lead_and_the_heir_can_concede() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team blue");
    let said = rig.say(2, "sim start");
    assert!(said.contains("boarding"), "blue's lead starts it: {said}");
    // Red's lead never boards.
    assert!(!rig.seat(1));
    assert!(!rig.seat(2));
    rig.tick_to(10);
    assert_eq!(rig.state(), MatchState::Running, "it ran with who was in");
    let centre = rig.centre();
    assert_eq!(centre.slots.len(), 2, "red's lead's slot is dropped");
    assert!(
        centre.leads_side(Team::Red, account(1)),
        "red's next claimant leads it"
    );
    assert!(
        !centre.leads_side(Team::Red, account(0)),
        "the dropped lead leads nothing"
    );
    assert!(
        centre.leads_side(Team::Blue, account(2)),
        "blue is untouched"
    );

    // The dropped lead first: holding no slot is holding no side.
    let said = rig.say(0, "sim concede");
    assert!(
        said.contains("You hold no slot in the simulator."),
        "{said}"
    );
    assert_eq!(rig.state(), MatchState::Running, "and nothing moved");

    // The heir may.
    let said = rig.say(1, "sim concede");
    assert!(said.contains("concedes for the red side"), "{said}");
    // The arena answers on its own thread; the centre lands in `ended` once the
    // check-in has run.
    for _ in 0..400 {
        if rig.state() == MatchState::Ended {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Ended, "a concede is not an abort");
    assert_eq!(
        rig.centre().outcome,
        Some(tacsim::match_::MatchOutcome::Blue)
    );
    assert_eq!(
        rig.centre().outcome_reason,
        Some(tacsim::arena::EndReason::Conceded { winner: Team::Blue })
    );
    let board = staff_board(&rig);
    assert!(
        board.contains("ended (match 1) — blue wins: red conceded"),
        "{board}"
    );
}

/// **Both leads dropped at once, and each side is led by its first seated
/// pilot** (16b cut AA.2, `B-78`): the rule runs once over the slots that are
/// left, not once per dropped slot.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aa2_a_double_drop_leaves_each_side_its_first_seated_pilot() {
    let mut rig = Rig::boot(4);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team blue");
    rig.say(3, "sim team blue");
    rig.say(0, "sim start");
    assert!(!rig.seat(1));
    assert!(!rig.seat(3));
    rig.tick_to(10);
    assert_eq!(rig.state(), MatchState::Running);
    let centre = rig.centre();
    assert_eq!(centre.slots.len(), 2, "both leads' slots are dropped");
    assert_eq!(centre.leads.len(), 2, "and both sides still have a lead");
    assert!(centre.leads_side(Team::Red, account(1)), "red's heir");
    assert!(centre.leads_side(Team::Blue, account(3)), "blue's heir");
}

/// **With the fill on, a side the deadline empties carries no lead** (16b cut
/// AA.2, `B-78`): the machines fly it, and nobody is left who could concede for
/// it — the entry goes with the last slot, as it does on a `SIM LEAVE`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aa2_with_the_fill_on_an_emptied_side_carries_no_lead() {
    let mut rig = fill_rig(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert!(
        !rig.seat(0),
        "one of two seated is not the trigger; the deadline is"
    );
    rig.tick_to(10);
    assert_eq!(rig.state(), MatchState::Running, "the machines fly blue");
    let centre = rig.centre();
    assert_eq!(centre.slots.len(), 1);
    assert!(
        !centre.leads.iter().any(|(side, _)| *side == Team::Blue),
        "an emptied side has no lead"
    );
    assert!(centre.leads_side(Team::Red, account(0)));
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
    // **16b cut D.3 (`T-60`)**: a claimant is refused, and the other claimant's
    // slot is still there — which is the whole of the row. Staff still may.
    let said = rig.say(0, "sim abort");
    assert!(
        said.contains("Nothing is running yet. `SIM LEAVE` gives your slot up."),
        "{said}"
    );
    assert_eq!(rig.state(), MatchState::Selection);
    assert_eq!(
        rig.centre().slots.len(),
        2,
        "one claimant dissolved a lobby"
    );

    let lobby = match_::centre_object(rig.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
    rig.tw
        .world_mut()
        .move_to(GOD, lobby)
        .expect("staff walk in");
    let said = rig.staff("sim abort");
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
    // **16b cut D.3 (`T-60`)**: a claimant's `SIM ABORT` no longer ends a
    // `Selection`, so the terminal centre this test needs is made by staff —
    // whose `SIM ABORT` in `Selection` is exactly what the row kept.
    let lobby = match_::centre_object(rig.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
    rig.tw
        .world_mut()
        .move_to(GOD, lobby)
        .expect("staff walk in");
    rig.staff("sim abort");
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
            assert!(!seen.contains(&tank), "twelve pods, twelve distinct tanks");
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
    assert_eq!(seen.len(), 12);
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
        tacsim::match_::ai_side_size(rig.tw.world(), &rig.centre(), &fill_config().match_),
        1,
        "one pilot alone is a one against one"
    );
    rig.say(1, "sim team red");
    assert_eq!(
        tacsim::match_::ai_side_size(rig.tw.world(), &rig.centre(), &fill_config().match_),
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
        tacsim::match_::ai_side_size(rig.tw.world(), &rig.centre(), &fill_config().match_),
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
    // Not a lead. **16b cut D.1** gave the set its own sentence: `NotLead` is
    // `SIM START`'s, and answering it here told somebody who asked for three
    // machines that they could not start a match.
    let said = rig.say(1, "sim ai 3");
    assert!(
        said.contains("Only a side's lead or staff can set the AI fill."),
        "{said}"
    );
    // Out of range, both ends: the centre has six pods a side (16b cut Z.3).
    let said = rig.say(0, "sim ai 0");
    assert!(said.contains("AI fill is 1 to 6 a side."), "{said}");
    let said = rig.say(0, "sim ai 7");
    assert!(said.contains("AI fill is 1 to 6 a side."), "{said}");
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

// ---------------------------------------------------------------------------
// 16b cut Q.8 — the unit type a slot fields
// ---------------------------------------------------------------------------

/// The canonical id of the unit definition one pod's tank is CURRENTLY built
/// from, resolved through the loaded catalog's own bindings — the same
/// comparison `reset_one` makes.
fn tank_schema(rig: &Rig, team: Team, index: u8) -> String {
    let w = rig.tw.world();
    let tank = match_::tank_for_slot(w, team, index).expect("the pod's tank is in the world");
    let held = mush_tactics::module::unit_of(w, tank).expect("the tank is a tactical unit");
    let store = w
        .module_state::<tacsim::TacsimStore>()
        .expect("the tacsim store");
    let catalog = store.definitions();
    catalog
        .bindings
        .iter()
        .find(|(_, binding)| binding.spec.definition == *held.schema())
        .map(|(id, _)| id.as_str().to_owned())
        .expect("the tank's schema is loaded")
}

/// One pod's tank's pose, for the "back at muster" half.
fn tank_pose(rig: &Rig, team: Team, index: u8) -> Option<(mush_sim::DomainId, mush_sim::Pose)> {
    let w = rig.tw.world();
    let tank = match_::tank_for_slot(w, team, index).expect("the pod's tank is in the world");
    mush_sim::pose_of(w, tank)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_sim_unit_lists_selects_and_clears_a_choice() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");

    // The bare read: the fielded catalog, and what this slot is flying. The
    // order is the catalog's own, which is canonical-id order, which is what
    // makes the sentence the same on every boot of the same content — and it is
    // pinned here because `docs/help/simulator.md` prints it.
    let said = rig.say(0, "sim unit");
    assert!(
        said.contains(
            "Fieldable units: biped_mk1, line_tank, line_tank_eccm, line_tank_ecm, \
             line_tank_laser, line_tank_thermal. Yours: line_tank."
        ),
        "{said}"
    );

    // The choice, announced the way a claim is, and on the board.
    let said = rig.say(0, "sim unit biped_mk1");
    assert!(said.contains("Slot red 1 will field biped_mk1."), "{said}");
    assert_eq!(
        rig.centre().slots[0].unit_type.as_deref(),
        Some("tacsim.unit.biped_mk1"),
        "the slot records the canonical id"
    );
    let said = rig.say(1, "sim status");
    assert!(
        said.contains("1 (RA): Pilot0 [biped_mk1]"),
        "the board shows a non-default choice: {said}"
    );
    let said = rig.say(0, "sim unit");
    assert!(said.contains("Yours: biped_mk1."), "{said}");

    // **Who hears what** (16b cut Q.8, fixer). The list is a read and goes to
    // the actor alone; the choice is announced the way a claim is, so the other
    // side reads it and can answer it. Neither audience has a test anywhere else
    // in this cut, and both are the kind of thing a later cut moves by accident.
    let overheard = rig.say_overheard(0, "sim unit", 1);
    assert!(
        !overheard.contains("Fieldable units:"),
        "the catalog listing is the asker's own: {overheard}"
    );
    let overheard = rig.say_overheard(0, "sim unit biped_mk1", 1);
    assert!(
        overheard.contains("Slot red 1 will field biped_mk1."),
        "the choice is announced to the centre: {overheard}"
    );

    // The pod's OWN type clears the choice rather than recording it.
    let said = rig.say(0, "sim unit line_tank");
    assert!(said.contains("Slot red 1 will field line_tank."), "{said}");
    assert_eq!(rig.centre().slots[0].unit_type, None);
    let said = rig.say(1, "sim status");
    assert!(
        said.contains("1 (RA): Pilot0,") || said.contains("1 (RA): Pilot0"),
        "{said}"
    );
    assert!(
        !said.contains('['),
        "a slot at its own type prints no bracket: {said}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_sim_unit_refuses_an_unknown_type_no_slot_and_a_wrong_state() {
    let mut rig = Rig::boot(2);

    // No slot at all, and a type named — the honest line in `Idle` and in a
    // terminal centre. **16b cut D.1** moved the BARE list in front of this gate
    // (it answers anybody in the centre), so it is the CHOICE that still needs a
    // slot to put a unit in.
    let said = rig.say(0, "sim unit biped_mk1");
    assert!(
        said.contains("Claim a slot first: SIM TEAM red|blue."),
        "{said}"
    );

    rig.say(0, "sim team red");
    let said = rig.say(0, "sim unit wibble");
    assert!(
        said.contains("No such unit type: wibble. SIM UNIT lists them."),
        "{said}"
    );
    assert_eq!(rig.centre().slots[0].unit_type, None, "and nothing moved");

    // A word that names a loaded definition that is not a UNIT is not a type
    // either: the list is the unit catalog's and nothing else's.
    let said = rig.say(0, "sim unit laser_mk1");
    assert!(said.contains("No such unit type: laser_mk1."), "{said}");

    // Past `Selection` it is too late.
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    let said = rig.say(0, "sim unit biped_mk1");
    assert!(
        said.contains(
            "The simulator centre is boarding; that is not something you can do to it now."
        ),
        "{said}"
    );
}

/// **A refit that cannot be made refuses the START and locks nothing** (16b cut
/// Q.8).
///
/// The tank is marked deployed, which is the one refusal `refit_unit` makes that
/// a test can arrange without breaking the world: the arena is the single writer
/// for a unit it holds, and the lobby has to stop rather than write anyway.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_a_refit_refusal_at_start_refuses_the_start_and_takes_no_lock() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");

    let tank = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    mush_sim::deployed::mark(
        rig.tw.world_mut(),
        tank,
        mush_sim::DeployedMark::new(99, "tacsim.arena.proving_ground"),
    );

    let said = rig.say(0, "sim start");
    // **16b cut Q.8, fixer (correctness Q8-6): the sentence's shape.** It names
    // the slot and the type and nothing else. `refit_unit`'s own reason is
    // `#57 is deployed; ...`, and this line goes to whoever typed `SIM START` —
    // who may be the other side's lead. The raw reason is in the server log.
    assert!(
        said.contains(
            "Slot red 1 cannot field biped_mk1: the simulator refused the refit. Staff should \
             read the server log."
        ),
        "the refusal names the slot and the type: {said}"
    );
    assert!(
        !said.contains("is deployed"),
        "the raw reason belongs in the log, not in the line: {said}"
    );
    assert!(
        !said.contains(&format!("#{}", tank.0)),
        "the refusal named the tank's object number: {said}"
    );
    assert_eq!(rig.state(), MatchState::Selection, "nothing moved");
    assert!(rig.centre().match_id.is_none(), "and no id was minted");
    assert!(
        rig.centre().slots.iter().all(|slot| slot.pod.is_none()),
        "no partial lock was taken"
    );
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.line_tank",
        "and nothing was written to the tank"
    );
}

/// **A match fought in a biped leaves the pod a `line_tank` at spec and at
/// muster** (16b cut Q.8), and a second reset writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_the_return_to_idle_puts_the_pods_own_tank_back() {
    let mut rig = Rig::boot(2);
    let authored = tank_pose(&rig, Team::Red, 1).expect("red 1's tank is placed");
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    rig.say(0, "sim start");

    // The refit happened at `SIM START`, before the pods locked.
    assert_eq!(tank_schema(&rig, Team::Red, 1), "tacsim.unit.biped_mk1");
    assert_eq!(
        tank_schema(&rig, Team::Blue, 1),
        "tacsim.unit.line_tank",
        "a slot that chose nothing is untouched"
    );

    assert!(!rig.seat(0));
    assert!(rig.seat(1), "the second seat fires the trigger");
    assert_eq!(rig.state(), MatchState::Running);
    rig.say(0, "sim abort");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");

    let said = rig.staff("@arena/reset");
    assert!(said.contains("is reset"), "{said}");
    assert!(
        said.contains("back to their own type"),
        "the staff line says what it put back: {said}"
    );
    assert_eq!(rig.state(), MatchState::Idle);
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.line_tank",
        "the pod's own tank is back"
    );
    let now = tank_pose(&rig, Team::Red, 1).expect("and it is still placed");
    assert_eq!(now.0, authored.0, "in its own domain");
    assert_eq!(
        (now.1.x_cm, now.1.y_cm, now.1.heading_mdeg),
        (authored.1.x_cm, authored.1.y_cm, authored.1.heading_mdeg),
        "on its authored cell"
    );

    // Idempotent and quiet: a second reset finds nothing at all.
    let said = rig.staff("@arena/reset");
    assert!(said.contains("already idle; nothing changed"), "{said}");
    assert!(!said.contains("back to their own type"), "{said}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_a_clean_boots_reset_is_still_silent() {
    let mut rig = Rig::boot(1);
    let said = rig.staff("@arena/reset");
    assert!(said.contains("already idle; nothing changed"), "{said}");
    assert!(!said.contains("back at spec"), "{said}");
    assert!(!said.contains("back to their own type"), "{said}");
}

/// **The board stops naming anybody's machine the moment the match is on** (16b
/// cut Q.8, fixer; doc 16 §1, §5).
///
/// `SIM STATUS` is the one selector that answers in every state, and the `SIM`
/// gate admits the twelve pods as well as the three rooms — so an ungated bracket
/// would let a seated pilot read the chassis of the contact their own sight
/// still renders `X1`, which is the single fact Q.1's provisional designators
/// and Q.2's classification clock exist to withhold. During `Selection` there is
/// no arena and no sensor to go round and both sides may read it; afterwards the
/// choice has been fitted to the tank and the row carries nothing the lobby
/// still needs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_the_board_prints_a_chosen_type_only_during_selection() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");

    // Selection: the opposing pilot reads it, deliberately.
    let said = rig.say(1, "sim status");
    assert!(said.contains("[biped_mk1]"), "{said}");

    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    let said = rig.say(1, "sim status");
    assert!(
        said.contains("1 (RA): Pilot0"),
        "the row itself is unchanged: {said}"
    );
    assert!(
        !said.contains("biped_mk1") && !said.contains('['),
        "the board named the other side's machine after the match began: {said}"
    );
    // And staff's own read of the same board says no more than a pilot's.
    let said = rig.staff("@arena/status");
    assert!(!said.contains("biped_mk1"), "{said}");

    assert!(!rig.seat(0));
    assert!(rig.seat(1));
    assert_eq!(rig.state(), MatchState::Running);
    let said = rig.say(1, "sim status");
    assert!(!said.contains("biped_mk1"), "{said}");
}

/// **The last `SIM LEAVE` is a return to `Idle`, and puts the tanks back** (16b
/// cut Q.8, fixer).
///
/// `lobby::leave` zeroes the centre in place when the last slot goes, and it was
/// the one path to `Idle` that never ran the tank reset — invisible until a
/// `SIM START` could leave a refitted tank standing in `Selection`. It can: the
/// refits are applied in slot order and the first refusal returns with the
/// earlier ones committed, which is the state this walks into and then out of.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_the_last_leave_puts_a_half_refitted_lobbys_tanks_back() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    rig.say(1, "sim unit biped_mk1");

    // Red 2's tank cannot be refitted, so the start refuses — after red 1's has
    // already been written.
    let second = match_::tank_for_slot(rig.tw.world(), Team::Red, 2).expect("red 2's tank");
    mush_sim::deployed::mark(
        rig.tw.world_mut(),
        second,
        mush_sim::DeployedMark::new(99, "tacsim.arena.proving_ground"),
    );
    let said = rig.say(0, "sim start");
    assert!(
        said.contains("Slot red 2 cannot field biped_mk1: "),
        "{said}"
    );
    assert_eq!(rig.state(), MatchState::Selection);
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.biped_mk1",
        "the refit that landed before the refusal stands"
    );

    // Everybody gives up on that lobby. The last one to go returns the centre to
    // `Idle`, and that is what puts the tanks back.
    for pilot in 0..3 {
        rig.say(pilot, "sim leave");
    }
    assert_eq!(rig.state(), MatchState::Idle);
    assert!(rig.centre().slots.is_empty());
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.line_tank",
        "the next pair does not inherit last lobby's biped"
    );

    // And a reset over it writes nothing — but it is **not** silent: red 2's
    // tank still carries the stale marker this test wedged it with, so `B-31`'s
    // middle headline is the true one and the row underneath it says why.
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("The simulator centre is idle, but not clean:"),
        "{said}"
    );
    assert!(
        said.contains("is still deployed and was left alone"),
        "{said}"
    );
    assert!(
        !said.contains("already idle; nothing changed"),
        "the headline contradicted the row under it: {said}"
    );
}

/// A stale `deployed` marker on one pod's tank: the one refusal `refit_unit`
/// makes that a test can arrange without breaking the world.
fn wedge(rig: &mut Rig, team: Team, index: u8) -> ObjId {
    let tank = match_::tank_for_slot(rig.tw.world(), team, index).expect("the pod's tank");
    mush_sim::deployed::mark(
        rig.tw.world_mut(),
        tank,
        mush_sim::DeployedMark::new(99, "tacsim.arena.proving_ground"),
    );
    tank
}

/// **A pod with no choice on it is put back to its authored type at `SIM
/// START`** (16b cut Q.8, fixer: conformance Q8-4).
///
/// The walk used to visit only the slots whose `Slot::unit_type` is `Some`, so a
/// tank refitted by an earlier refused start stood as whatever it had become —
/// and the board showed no bracket over it, because the slot had chosen nothing.
/// A pilot only has to change their mind for that to bite: `SIM UNIT line_tank`
/// clears the choice and does **not** clear the tank, and until this fix they
/// boarded the biped they had just said they did not want.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_sim_start_puts_a_pod_with_no_choice_back_to_its_authored_type() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red"); // red 1
    rig.say(1, "sim team red"); // red 2
    rig.say(2, "sim team blue"); // blue 1
    rig.say(0, "sim unit biped_mk1");
    rig.say(1, "sim unit biped_mk1");

    // Red 2 refuses, after red 1 has already been written.
    let second = wedge(&mut rig, Team::Red, 2);
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Selection);
    assert_eq!(tank_schema(&rig, Team::Red, 1), "tacsim.unit.biped_mk1");

    // Both of them change their minds. The board now says nothing about either
    // slot, and red 1's tank is still a biped.
    rig.say(0, "sim unit line_tank");
    rig.say(1, "sim unit line_tank");
    assert!(
        rig.centre()
            .slots
            .iter()
            .all(|slot| slot.unit_type.is_none()),
        "no slot carries a choice any more"
    );
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.biped_mk1",
        "and clearing a choice writes nothing to the tank"
    );

    mush_sim::deployed::clear(rig.tw.world_mut(), second);
    let said = rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding, "{said}");
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.line_tank",
        "the start put the pod with no choice on it back to its own machine"
    );
    assert_eq!(tank_schema(&rig, Team::Red, 2), "tacsim.unit.line_tank");
    assert_eq!(tank_schema(&rig, Team::Blue, 1), "tacsim.unit.line_tank");
}

/// **A leaver's refit does not reach the next claimant of their slot** (16b cut
/// Q.8, fixer: conformance Q8-4).
///
/// `Slot::unit_type` is cleared with the slot; the *tank* was not, and a
/// non-last `SIM LEAVE` runs no reset of its own. So the pilot who gave a lobby
/// up handed their biped to whoever took the slot next, with no bracket on the
/// board to say so. The vacating itself now puts that one pod back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_a_leavers_refit_does_not_reach_the_next_claimant() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red"); // red 1
    rig.say(1, "sim team red"); // red 2
    rig.say(2, "sim team blue"); // blue 1
    rig.say(0, "sim unit biped_mk1");
    rig.say(1, "sim unit biped_mk1");

    let second = wedge(&mut rig, Team::Red, 2);
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Selection);
    assert_eq!(tank_schema(&rig, Team::Red, 1), "tacsim.unit.biped_mk1");

    // Pilot 0 gives up. It is NOT the last slot, so the centre stays in
    // `Selection` and the whole-roster reset the last leave runs does not.
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Selection, "the lobby is still up");
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.line_tank",
        "the choice went with the pilot who made it"
    );

    // And the next claimant of red 1 chooses nothing and flies the pod's own
    // machine.
    mush_sim::deployed::clear(rig.tw.world_mut(), second);
    rig.say(1, "sim unit line_tank");
    rig.say(0, "sim team red");
    let mine = rig
        .centre()
        .slots
        .iter()
        .find(|slot| slot.pilot == rig.pilot(0))
        .map(|slot| (slot.team, slot.index, slot.unit_type.clone()))
        .expect("the re-claim took a slot");
    assert_eq!(
        mine,
        (Team::Red, 1, None),
        "the lowest free red slot, with no choice on it"
    );
    // Pilot 1 inherited the side's lead when pilot 0 left.
    let said = rig.say(1, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding, "{said}");
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.line_tank",
        "the new claimant boards the pod's own machine"
    );
}

/// **An AI-filled pod flies the pod's own machine after the pilot who chose one
/// left** (16b cut Q.8, fixer).
///
/// `lobby::run` fills the unclaimed pods — including the pods of the slots that
/// went during `Boarding` — with `tank_for_slot`, so a tank refitted at `SIM
/// START` and then abandoned was flown by a machine. That contradicts the third
/// bullet of `docs/help/simulator.md` ("the machines that fill the unclaimed
/// pods always fly the pod's own tank"), and there is no return to `Idle`
/// between the leave and the check-out to put it right.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_an_ai_filled_pod_flies_the_authored_type_after_a_chooser_left() {
    let mut rig = fill_rig(2);
    rig.say(0, "sim team red"); // red 1
    rig.say(1, "sim team red"); // red 2
    let said = rig.say(0, "sim ai 2");
    assert!(said.contains("AI fill set to 2 a side."), "{said}");
    rig.say(0, "sim unit biped_mk1");

    let said = rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding, "{said}");
    assert_eq!(tank_schema(&rig, Team::Red, 1), "tacsim.unit.biped_mk1");

    // The chooser gives up before boarding. Their pod is now one of the
    // unclaimed ones the fill takes.
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Boarding, "the match is still on");
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.line_tank",
        "the abandoned pod is back at its authored type before the check-out"
    );

    assert!(rig.seat(1), "the last seat fires the check-out");
    assert_eq!(rig.state(), MatchState::Running);
    assert!(
        rig.match_units().contains(&(Team::Red, 1)),
        "the fill really took the vacated pod: {:?}",
        rig.match_units()
    );
    assert_eq!(
        tank_schema(&rig, Team::Red, 1),
        "tacsim.unit.line_tank",
        "and the machine flies the pod's own tank"
    );
}

/// **`SIM UNIT`'s "claim a slot first" is only said where the claim is
/// accepted** (16b cut Q.8, fixer: correctness Q8-5).
///
/// Choosing a type needs a claimed slot, and the refusal is `SimError::NoSlot`
/// said as the thing to do next. `SIM TEAM` is accepted in `Idle`, in
/// `Selection` and on a terminal centre, and refused in `Boarding` and `Running`
/// — where the advice was therefore a lie, and the state refusal is what is
/// honest.
///
/// **16b cut D.1** moved the bare list in front of both gates (see
/// [`d1_the_two_bare_reads_answer_anybody_in_the_centre`]), so every line here
/// names a type.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_sim_unit_advises_sim_team_only_where_sim_team_is_accepted() {
    let mut rig = Rig::boot(3);

    // `Idle`, and then `Selection` with no slot of one's own: true both times.
    let said = rig.say(2, "sim unit biped_mk1");
    assert!(
        said.contains("Claim a slot first: SIM TEAM red|blue."),
        "{said}"
    );
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(2, "sim unit biped_mk1");
    assert!(
        said.contains("Claim a slot first: SIM TEAM red|blue."),
        "{said}"
    );

    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    let said = rig.say(2, "sim unit biped_mk1");
    assert!(
        said.contains(
            "The simulator centre is boarding; that is not something you can do to it now."
        ),
        "{said}"
    );
    assert!(
        !said.contains("SIM TEAM"),
        "it advised a verb that is refused here: {said}"
    );

    assert!(!rig.seat(0));
    assert!(rig.seat(1));
    assert_eq!(rig.state(), MatchState::Running);
    let said = rig.say(2, "sim unit biped_mk1");
    assert!(said.contains("The simulator centre is running;"), "{said}");
    assert!(!said.contains("SIM TEAM"), "{said}");

    // A terminal centre takes a claim — it clears the last match's board first —
    // so there the advice is true again.
    rig.end_match(tacsim::arena::EndReason::Abort);
    assert!(rig.state().is_terminal());
    let said = rig.say(2, "sim unit biped_mk1");
    assert!(
        said.contains("Claim a slot first: SIM TEAM red|blue."),
        "{said}"
    );
}

/// **The refit's document really crosses into `tactics`, under the module
/// identity production uses** (16b cut Q.8, fixer).
///
/// `refit_unit` writes through `Ctx::persist`, whose fast path is one namespace
/// equality and which consults the `tacsim → tactics` delegation table **only**
/// on a mismatch — so a call made under the tactics identity exercises no
/// delegation at all, and a refused crossing is a silent no-op that still leaves
/// the `UnitStore` RAM row (which every other assertion in this cut reads)
/// looking right. This is the production identity in the production
/// composition, where the delegation is really declared.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_a_refit_under_the_tacsim_identity_queues_the_tactics_document() {
    let mut rig = Rig::boot(1);
    let tank = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    let spec = {
        let id = mush_core::CanonicalId::new("tacsim.unit.biped_mk1").expect("a canonical id");
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .expect("the tacsim store")
            .definitions()
            .bindings
            .get(&id)
            .expect("the biped is in the fielded catalog")
            .spec
            .clone()
    };

    let mut out = Outbox::new();
    let engine = rig.tw.engine_mut();
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
    mush_tactics::module::refit_unit(&mut ctx, tank, &spec).expect("the refit lands");

    let ops: Vec<mush_core::ModuleOp> = out
        .take_deferred()
        .into_iter()
        .filter_map(|deferred| match deferred {
            mush_core::Deferred::Persist(op) => Some(op),
            _ => None,
        })
        .collect();
    assert_eq!(ops.len(), 1, "a refit is one document: {ops:?}");
    assert_eq!(
        ops[0].ns,
        mush_tactics::module::NS,
        "the delegation carried the write instead of dropping it"
    );
    let expected = format!("unit:{}", tank.0);
    assert!(
        matches!(&ops[0].op, mush_core::ModuleWrite::JsonSet { key, .. }
            if key.as_str() == expected),
        "the one op is this tank's record: {:?}",
        ops[0].op
    );
}

// ---------------------------------------------------------------------------
// 16b cut D.1 — the board and the two bare reads
// ---------------------------------------------------------------------------

/// The `red:` and `blue:` rows of one rendering, in that order.
fn sides(board: &str) -> (String, String) {
    let row = |head: &str| {
        board
            .lines()
            .find(|line| line.trim_start().starts_with(head))
            .unwrap_or_default()
            .to_owned()
    };
    (row("red:"), row("blue:"))
}

/// A rendering of the board as staff read it.
fn staff_board(rig: &Rig) -> String {
    match_::status_lines(rig.tw.world(), match_::BoardReader::Staff).join("\n")
}

/// **`B-01`: while a match runs a player reads their own side in full and every
/// other side as index, callsign and name alone** (16b cut D.1; doc 16 §1, §5).
///
/// The four readers, **both** states the rule applies in — `Running` and
/// `Stopping`, which is the whole of the gate — and the two before them that it
/// does not.
///
/// The terminal states are walked last and prove something weaker on purpose,
/// and the test says which: `release_rows` empties `centre.slots` at the end, so
/// a terminal board has no slot row at all for the clause to act on and what is
/// pinned there is the release, not the reader.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_the_board_withholds_the_other_sides_standing_while_a_match_runs() {
    let mut rig = Rig::boot(3);

    // (0) `Selection`: every reader gets the whole board, including the pilot
    // with no slot at all. The choice is a lobby negotiation and both sides may
    // read it.
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    for n in 0..3 {
        let said = rig.say(n, "sim status");
        assert!(
            said.contains("[biped_mk1]"),
            "reader {n} was refused the chassis bracket in Selection: {said}"
        );
    }

    // (1) `Boarding`: still the whole board — there is no arena yet, and
    // `seated` is lobby knowledge.
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    assert!(!rig.seat(0));
    for n in 0..3 {
        let said = rig.say(n, "sim status");
        assert!(
            said.contains(", seated"),
            "reader {n} was refused `seated` in Boarding: {said}"
        );
    }

    // (2) `Running`.
    assert!(rig.seat(1));
    assert_eq!(rig.state(), MatchState::Running);

    // (a) the red pilot: red in full, blue reduced.
    let said = rig.say(0, "sim status");
    let (red, blue) = sides(&said);
    assert!(red.contains(", pod ") && red.contains(", seated"), "{said}");
    for word in [", pod ", ", seated", ", left", "["] {
        assert!(
            !blue.contains(word),
            "a red pilot read `{word}` on the blue row: {said}"
        );
    }
    assert!(blue.contains("(BA): Pilot1"), "{said}");

    // (b) the blue pilot: the mirror image.
    let said = rig.say(1, "sim status");
    let (red, blue) = sides(&said);
    assert!(
        blue.contains(", pod ") && blue.contains(", seated"),
        "{said}"
    );
    for word in [", pod ", ", seated", ", left", "["] {
        assert!(
            !red.contains(word),
            "a blue pilot read `{word}` on the red row: {said}"
        );
    }
    assert!(red.contains("(RA): Pilot0"), "{said}");

    // (c) a player in the centre holding no slot: both sides reduced.
    let said = rig.say(2, "sim status");
    let (red, blue) = sides(&said);
    for row in [&red, &blue] {
        for word in [", pod ", ", seated", ", left", "["] {
            assert!(
                !row.contains(word),
                "a slotless reader read `{word}`: {said}"
            );
        }
    }
    assert!(
        red.contains("(RA): Pilot0") && blue.contains("(BA): Pilot1"),
        "{said}"
    );

    // (d) staff: both sides in full, in the same state.
    let board = staff_board(&rig);
    let (red, blue) = sides(&board);
    assert!(
        red.contains(", pod ") && red.contains(", seated"),
        "{board}"
    );
    assert!(
        blue.contains(", pod ") && blue.contains(", seated"),
        "{board}"
    );

    // (3) **A stop in flight** — the window `Stopping` is about. `SIM ABORT`
    // from a participant sends the stop and moves the **record** to `Stopping`;
    // the end lands when the arena answers and the egress is pumped, which this
    // test never does. The **centre** stays `Running` throughout — nothing in
    // the tree writes `MatchState::Stopping` to a centre (`lobby.rs` writes
    // `Selection`, `Boarding`, `Running` and whatever `release_rows` is given),
    // so the gate's `Stopping` arm is there for the day one does and cannot be
    // reached from here. What this block pins is the reachable half: the slots
    // are all still on the board, a stop is in flight, and the reader rule still
    // bites.
    rig.say(0, "sim abort");
    assert_eq!(
        rig.state(),
        MatchState::Running,
        "a centre took `Stopping`; the gate's second arm is reachable now and \
         this block should read the board in it"
    );
    let record = rig
        .centre()
        .match_id
        .and_then(|match_id| {
            rig.tw
                .world()
                .module_state::<tacsim::match_::MatchBook>()
                .and_then(|book| book.get(match_id).map(|record| record.state))
        })
        .expect("a running centre has a match on the book");
    assert_eq!(
        record,
        MatchState::Stopping,
        "the abort did not send the stop, so there is no stop in flight to read \
         the board under"
    );
    assert_eq!(
        rig.centre().slots.len(),
        2,
        "the stop released the rows, so there is nothing left to withhold"
    );
    let said = rig.say(0, "sim status");
    let (red, blue) = sides(&said);
    assert!(red.contains(", pod ") && red.contains(", seated"), "{said}");
    for word in [", pod ", ", seated", ", left", "["] {
        assert!(
            !blue.contains(word),
            "a red pilot read `{word}` on the blue row with a stop in flight: {said}"
        );
    }
    assert!(blue.contains("(BA): Pilot1"), "{said}");
    // And staff still read the whole of it.
    let board = staff_board(&rig);
    let (red, blue) = sides(&board);
    assert!(
        red.contains(", seated") && blue.contains(", seated"),
        "staff were reduced while a stop was in flight: {board}"
    );

    // (4) A terminal centre is public again: the match is over and there is
    // nothing left for a sensor to have earned. **What is pinned here is the
    // release**: `release_rows` clears `centre.slots`, so both sides read
    // `nobody` and the reader clause has no row to act on — a board that reduced
    // in every state would pass this block too, which is why (3) above is the
    // proof of the gate and this is not.
    rig.end_match(tacsim::arena::EndReason::Abort);
    assert!(rig.state().is_terminal());
    let board = staff_board(&rig);
    let (red, blue) = sides(&board);
    assert!(
        red.contains("nobody") && blue.contains("nobody"),
        "the end left a slot row on the board, so the terminal arm below is no \
         longer vacuous and must be written to test the reader: {board}"
    );
    for n in 0..3 {
        let said = rig.say(n, "sim status");
        assert_eq!(
            said.trim_end(),
            board.trim_end(),
            "reader {n} was given a different board from staff after the end"
        );
    }
}

/// **`B-28`: the two bare reads answer anybody standing in the centre** (16b cut
/// D.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_the_two_bare_reads_answer_anybody_in_the_centre() {
    let mut rig = fill_rig(3);

    // `SIM UNIT`, with no slot and in `Idle`: the catalog, and no `Yours:`.
    let said = rig.say(2, "sim unit");
    assert!(said.contains("Fieldable units: "), "{said}");
    assert!(
        !said.contains("Yours:"),
        "a reader with no slot was told what they are flying: {said}"
    );
    // `SIM AI`, from a non-lead with no slot, in `Idle`.
    let said = rig.say(2, "sim ai");
    assert!(said.contains("AI fill: "), "{said}");

    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    // A non-lead who holds a slot reads both, and the unit read names their own.
    let said = rig.say(1, "sim unit");
    assert!(said.contains("Fieldable units: "), "{said}");
    assert!(said.contains("Yours: line_tank."), "{said}");
    let said = rig.say(1, "sim ai");
    assert!(said.contains("AI fill: "), "{said}");
    // But setting is still the lead's, with its own sentence.
    let said = rig.say(1, "sim ai 3");
    assert!(
        said.contains("Only a side's lead or staff can set the AI fill."),
        "{said}"
    );

    // And in `Boarding`, where neither read was served at all before this cut.
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    for n in 0..3 {
        let said = rig.say(n, "sim unit");
        assert!(
            said.contains("Fieldable units: "),
            "reader {n} in Boarding: {said}"
        );
        let said = rig.say(n, "sim ai");
        assert!(said.contains("AI fill: "), "reader {n} in Boarding: {said}");
    }
}

/// **`B-30`: the claim line says what `SIM START` will actually do** (16b cut
/// D.1).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_the_claim_line_branches_on_the_ai_fill() {
    // With the fill off, the sentence is byte for byte what it always was.
    let mut rig = Rig::boot(2);
    let said = rig.say(0, "sim team red");
    assert!(
        said.contains(
            "Pilot0 takes red slot 1. `SIM START` begins the match once both sides have a pilot."
        ),
        "{said}"
    );

    // With it on, `SIM START` needs nobody on the other side.
    let mut rig = fill_rig(2);
    let said = rig.say(0, "sim team red");
    assert!(
        said.contains(
            "Pilot0 takes red slot 1. `SIM START` begins the match; the machines take the pods \
             nobody claims."
        ),
        "{said}"
    );
    assert!(
        !said.contains("once both sides have a pilot"),
        "the old advice is still on a server where it is false: {said}"
    );
}

/// **`B-29`: the board's clock is the arena's own, in `m:ss`** (16b cut D.1).
///
/// The formatting, at the edges nothing else exercises, and the `0:00` a board
/// prints before the first `Frames` message arrives.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d1_the_board_prints_the_arena_clock_in_minutes_and_seconds() {
    for (now_ms, text) in [
        (0_i64, "0:00"),
        (-1, "0:00"),
        (999, "0:00"),
        (1_000, "0:01"),
        (59_999, "0:59"),
        (60_000, "1:00"),
        (61_500, "1:01"),
        (3_600_000, "60:00"),
    ] {
        assert_eq!(match_::clock_text(now_ms), text, "{now_ms} ms");
    }

    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    // `Selection` has no match id, so there is no clock line at all.
    let said = rig.say(0, "sim status");
    assert!(!said.contains("arena time"), "{said}");

    rig.say(0, "sim start");
    assert!(!rig.seat(0));
    assert!(rig.seat(1));
    assert_eq!(rig.state(), MatchState::Running);
    let said = rig.say(0, "sim status");
    assert!(
        said.contains("  arena time 0:00"),
        "a board read before the first frame reads nought: {said}"
    );
    assert!(
        !said.contains(" ms"),
        "the raw milliseconds are gone from the board: {said}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut D.3 — the lifecycle's ends
// ---------------------------------------------------------------------------

/// The fill-on rig with `[tacsim.match].unattended_ms` short enough for a test to
/// walk past it (16b cut D.3, `B-33`).
fn unattended_config(ms: u32) -> TacsimConfig {
    let mut cfg = fill_config();
    cfg.match_.unattended_ms = ms;
    cfg
}

impl Rig {
    /// **The schema one pod's tank is fitted with**, as the board spells it.
    fn schema_of(&self, team: Team, index: u8) -> String {
        let tank = match_::tank_for_slot(self.tw.world(), team, index).expect("the pod's tank");
        mush_tactics::module::unit_of(self.tw.world(), tank)
            .expect("a tactical unit")
            .schema()
            .key()
            .id
            .as_str()
            .to_owned()
    }

    /// **Release one pilot's seat the way every route to a released seat does**
    /// (16b cut D.3): `Control { Released }` through
    /// [`tacsim::arena_bridge::send_control`], which is where `EXIT`, a `QUIT`
    /// and the linkdead grace running out all end up — and where
    /// `lobby::slot_left` is called, so this is the one seam the unattended
    /// stamp hangs off.
    fn release(&mut self, n: usize) {
        let pilot = self.pilot(n);
        let unit = self
            .centre()
            .slot_of_pilot(pilot)
            .and_then(|slot| slot.unit)
            .expect("a seated slot");
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
        let _ = tacsim::arena_bridge::send_control(
            &mut ctx,
            unit,
            tacsim::arena::state::Channel::Helm,
            tacsim::arena::intents::ControlState::Released,
            Some(pilot),
        );
    }

    /// Fire the module ticks due at `seconds` and answer what pilot `n` heard.
    fn tick_heard(&mut self, seconds: i64, n: usize) -> String {
        let actor = self.pilot(n);
        self.tw.set_now(seconds);
        let events = self.tw.fire_due(seconds.saturating_mul(1_000));
        self.seen(actor, &events)
    }

    /// **Strand one unit the way a refused check-in strands it** (16b cut D.3,
    /// `T-49`): a real `mush_tactics::custody::check_out`, which writes both
    /// halves of the marker — the tactics record's, which every ordinary
    /// mutation consults, and the simulation's, which the admission sites read.
    /// Planting only the second would leave a unit `@arena/release` could see and
    /// not check in.
    fn strand(&mut self, unit: ObjId, match_id: u64) {
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
        mush_tactics::custody::check_out(
            &mut ctx,
            unit,
            mush_sim::DeployedMark::new(match_id, "tacsim.arena.proving_ground"),
        )
        .expect("a check-out of a unit nothing holds");
    }

    /// **Give one pilot a cockpit billet on a unit the centre does not own** (16b
    /// cut D.3, `B-36`) — the two writes `BOARD` makes, without the pod and
    /// without the lobby: the `cockpit` station on the unit, and the seat.
    ///
    /// It is the state a staffer stationed in a machine of their own is in, and
    /// the one the narrowed [`waiting_to_start`] is about.
    fn station_on(&mut self, n: usize, unit: ObjId) {
        let actor = self.pilot(n);
        let station = mush_sim::StationId::new(tacsim::match_::cockpit::STATION)
            .expect("the cockpit station id");
        let mut out = Outbox::new();
        let engine = self.tw.engine_mut();
        let mut ctx = Ctx {
            w: &mut engine.world,
            actor,
            switches: SwitchSet::default(),
            args: ParsedArgs::default(),
            out: &mut out,
            reg: &engine.registry,
            module: Some(smol_str::SmolStr::new(tacsim::NS)),
            typed_state: mush_core::ModuleActionState::unavailable(),
        };
        mush_sim::declare_station(
            &mut ctx,
            unit,
            &station,
            tacsim::match_::cockpit::STATION_ROLE,
            &tacsim::match_::cockpit::cockpit_channels(),
            None,
            tacsim::match_::cockpit::STATION_PRIORITY,
        )
        .expect("a station on a unit nobody holds");
        mush_sim::request_seat_activation(
            &mut ctx,
            &mush_sim::SeatRequest::Activate {
                unit,
                station,
                npc: actor,
                granted: tacsim::match_::cockpit::cockpit_channels(),
                fallback: None,
            },
        )
        .expect("a vacant seat takes its holder");
    }

    /// Whether pilot `n` still holds the `cockpit` billet on `unit`.
    fn holds_billet(&self, n: usize, unit: ObjId) -> bool {
        let actor = self.pilot(n);
        self.tw
            .world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.assignments.get(&unit))
            .and_then(|host| host.seats.values().find(|seat| seat.actor == actor))
            .is_some()
    }

    /// The running match's task table.
    fn tasks(&self) -> tacsim::arena_bridge::ArenaTasks {
        self.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the registry")
            .tasks()
            .clone()
    }
}

/// Two pilots, one match, running.
fn duel(rig: &mut Rig) {
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert!(!rig.seat(0));
    assert!(rig.seat(1));
    assert_eq!(rig.state(), MatchState::Running);
}

// ---------------------------------------------------------------------------
// (1) `B-32`, `T-50`, `T-48`, `T-51` — one funnel for every end before `Running`
// ---------------------------------------------------------------------------

/// **A refused check-out calls the lobby off instead of stranding it** (16b cut
/// D.3, `B-32`, `T-50`).
///
/// Reached from a real `Boarding` with a really boarded pilot: the blue tank is
/// marked deployed to another match between the two seats, so the **second**
/// seat fires the `Boarding → Running` trigger and cut T.3's check-out refuses
/// it. The centre goes `Aborted`, both pods are free, both pilots are standing in
/// the team room with their billets back, the red pod's `SIM UNIT` refit is
/// undone, and a second crew can claim and board.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_a_refused_check_out_calls_the_lobby_off_and_frees_the_pods() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    assert_eq!(
        rig.schema_of(Team::Red, 1),
        "tacsim.unit.biped_mk1",
        "the start fitted the chosen chassis"
    );
    assert!(!rig.seat(0), "red is in and blue is not");

    // The blue tank is out on loan to somebody else, so the check-out refuses.
    let blue = match_::tank_for_slot(rig.tw.world(), Team::Blue, 1).expect("blue 1's tank");
    mush_sim::deployed::mark(
        rig.tw.world_mut(),
        blue,
        mush_sim::DeployedMark::new(99, "tacsim.arena.proving_ground"),
    );
    let pilot = rig.pilot(1);
    let pod = rig.pod(1).expect("a locked pod");
    let unit = match_::tank_of(rig.tw.world(), pod).expect("the pod's tank");
    let heard = {
        let match_id = rig
            .centre()
            .match_id
            .expect("a boarding centre has a match id");
        let mut out = Outbox::new();
        {
            let engine = rig.tw.engine_mut();
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
            assert!(
                match_::slot_seated(&mut ctx, match_id, pilot, unit).is_err(),
                "the check-out was supposed to refuse"
            );
        }
        rig.seen(pilot, &out.take())
    };
    assert!(
        heard.contains("could not start") && heard.contains("The pods are free again."),
        "{heard}"
    );
    assert_eq!(rig.state(), MatchState::Aborted, "the lobby was left stuck");
    assert!(rig.centre().slots.is_empty(), "the slots are given back");
    let red_pod = match_::pod_object(rig.tw.world(), Team::Red, 1).expect("red pod 1");
    assert_eq!(match_::pilot_of(rig.tw.world(), red_pod), None);
    assert!(
        !tacsim::match_::cockpit::in_a_pod(rig.tw.world(), rig.pilot(0)),
        "the boarded pilot was left shut in the pod"
    );
    assert!(
        match_::flying(rig.tw.world(), rig.pilot(0)).is_none(),
        "the boarded pilot kept the tank's billet"
    );
    assert_eq!(
        rig.schema_of(Team::Red, 1),
        "tacsim.unit.line_tank",
        "the called-off lobby left its biped standing in the pod"
    );

    // And a second crew really can use the centre again, once the stale marker
    // this test planted by hand is off the blue tank.
    mush_sim::deployed::clear(rig.tw.world_mut(), blue);
    rig.say(2, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(2, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert!(!rig.seat(2), "the pod took a pilot again");
}

/// **An early refusal of `run` calls the lobby off too** (16b cut D.3, `B-32`,
/// `T-50`).
///
/// [`tacsim::match_::sim_start`]'s own refusals happen before `Boarding`; these
/// are the three [`run`] itself makes, *after* the pods are locked and somebody is
/// sitting in one — and before this cut all three simply returned `Err`, leaving
/// the centre in `Boarding` with its deadline already cleared, its pods locked and
/// a pilot whose only way out was `SIM ABORT`.
///
/// `EmptySide` is the one of the three a lobby can really reach: with the fill on,
/// the check that a side has *somebody* is asked over the whole centre, so a side
/// with no claimant is legal and a side whose roster tanks are not in the world is
/// not. `NobodyPlaying` and `Unavailable` share this arm and cannot be reached
/// from a real `Boarding` — `settle_boarding` only calls `run` with every slot
/// seated, and `board_deadline` has called an empty centre off before it gets
/// there — so what is walked here is the arm, with the refusal that reaches it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_an_early_refusal_of_run_calls_the_lobby_off() {
    let mut rig = fill_rig(2);
    rig.say(0, "sim team red");
    rig.say(0, "sim unit biped_mk1");
    rig.say(0, "sim start");
    assert_eq!(
        rig.state(),
        MatchState::Boarding,
        "the fill starts one side"
    );

    // Blue's roster tanks leave the world between the lock and the seat, so the
    // fill finds nothing to field for that side and `ready` refuses.
    for index in 1..=match_::PODS_PER_SIDE {
        if let Some(tank) = match_::tank_for_slot(rig.tw.world(), Team::Blue, index) {
            rig.tw
                .world_mut()
                .destroy(tank)
                .expect("a tank nothing protects");
        }
    }
    rig.tw.world_mut().sweep();
    assert_eq!(
        match_::tank_for_slot(rig.tw.world(), Team::Blue, 1),
        None,
        "the blue side still has a tank to field"
    );

    // The seat fires the `Boarding → Running` trigger, and `run` refuses before
    // the check-out rather than at it.
    let pilot = rig.pilot(0);
    let pod = rig.pod(0).expect("a locked pod");
    let unit = match_::tank_of(rig.tw.world(), pod).expect("the pod's tank");
    let (refusal, heard) = {
        let match_id = rig
            .centre()
            .match_id
            .expect("a boarding centre has a match id");
        let mut out = Outbox::new();
        let refusal = {
            let engine = rig.tw.engine_mut();
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
            match_::slot_seated(&mut ctx, match_id, pilot, unit)
                .expect_err("the fill had no blue tank to field")
                .to_string()
        };
        (refusal, rig.seen(pilot, &out.take()))
    };
    assert!(
        refusal.contains("The blue side has no pilots; a match takes at least one a side."),
        "{refusal}"
    );
    assert!(
        heard.contains("Match 1 could not start:") && heard.contains("The pods are free again."),
        "{heard}"
    );

    // The same funnel, and the same freed centre: the state, the slots, the pod,
    // the pilot's billet and the red pod's refit.
    assert_eq!(rig.state(), MatchState::Aborted, "the lobby was left stuck");
    assert!(rig.centre().slots.is_empty(), "the slots are given back");
    let red_pod = match_::pod_object(rig.tw.world(), Team::Red, 1).expect("red pod 1");
    assert_eq!(match_::pilot_of(rig.tw.world(), red_pod), None);
    assert!(
        !tacsim::match_::cockpit::in_a_pod(rig.tw.world(), pilot),
        "the boarded pilot was left shut in the pod"
    );
    assert!(
        match_::flying(rig.tw.world(), pilot).is_none(),
        "the boarded pilot kept the tank's billet"
    );
    assert_eq!(
        rig.schema_of(Team::Red, 1),
        "tacsim.unit.line_tank",
        "the called-off lobby left its biped standing in the pod"
    );

    // And the centre is usable again: a second claim is accepted.
    let said = rig.say(1, "sim team red");
    assert!(said.contains("takes red slot 1"), "{said}");
    assert_eq!(rig.state(), MatchState::Selection);
}

/// **An end before `Running` puts a `SIM UNIT` refit back** (16b cut D.3, `B-32`;
/// 16b cut Q.8's `unfit_slot`).
///
/// The refit is *fitted* at `SIM START`, so a lobby that ends in `Boarding` would
/// otherwise leave a biped standing in a pod nobody chose it for — the next crew's
/// red 1 would climb into somebody else's machine. The control is the same abort
/// out of `Selection`, where nothing was fitted at all and the reset has nothing
/// to do.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_a_pre_running_end_puts_a_refitted_pod_back() {
    let mut rig = Rig::boot(2);

    // (a) The control: chosen, never fitted, and called off from `Selection`.
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    assert_eq!(
        rig.schema_of(Team::Red, 1),
        "tacsim.unit.line_tank",
        "`SIM UNIT` fitted a chassis in `Selection`"
    );
    let lobby = match_::centre_object(rig.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
    rig.tw
        .world_mut()
        .move_to(GOD, lobby)
        .expect("staff walk in");
    rig.staff("sim abort");
    assert_eq!(rig.state(), MatchState::Aborted);
    assert_eq!(rig.schema_of(Team::Red, 1), "tacsim.unit.line_tank");
    rig.staff("@arena/reset");

    // (b) The case: chosen, fitted by `SIM START`, and called off from
    // `Boarding` by a participant.
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    assert_eq!(
        rig.schema_of(Team::Red, 1),
        "tacsim.unit.biped_mk1",
        "the start never fitted the chosen chassis"
    );
    let said = rig.say(0, "sim abort");
    assert!(said.contains("calls the match off"), "{said}");
    assert_eq!(rig.state(), MatchState::Aborted);
    assert_eq!(
        rig.schema_of(Team::Red, 1),
        "tacsim.unit.line_tank",
        "the called-off lobby left its biped standing in the pod"
    );
}

/// **`T-48`: the last `SIM LEAVE` returns the centre to a `Centre::default()`**,
/// through the one writer of `Idle`.
///
/// The behaviour is unchanged and that is the point: `leave` used to zero the
/// struct in place, a second writer of a transition [`reset`] already owned, so
/// every field added to `Centre` had to be remembered in two places. What this
/// pins is the observable half — the centre really is default, in every field,
/// and the tanks are back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_the_last_leave_returns_the_centre_to_its_default() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    assert_ne!(rig.centre(), tacsim::match_::Centre::default());
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Selection, "one claimant is left");
    rig.say(1, "sim leave");
    assert_eq!(
        rig.centre(),
        tacsim::match_::Centre::default(),
        "the last leave left something on the centre"
    );
    assert_eq!(rig.schema_of(Team::Red, 1), "tacsim.unit.line_tank");
    // And a reset over it has nothing at all to say, which is only true of a
    // centre that really is default.
    let said = rig.staff("@arena/reset");
    assert!(said.contains("already idle; nothing changed"), "{said}");
}

/// **`T-51`: the fill's floor is held inside the pods that exist.**
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_the_ai_floor_is_clamped_to_the_pods_a_side_has() {
    // 16b cut Z.3: the size is read against a world, whose pods and tanks are
    // what `slots_per_side` clamps to; this one is whole, six a side.
    let rig = Rig::boot(0);
    let mut centre = tacsim::match_::Centre::default();
    assert_eq!(match_::largest_side(&centre), 1, "never zero");
    let cfg = tacsim::config::MatchConfig::default();
    for n in 0..12_u8 {
        centre.slots.push(tacsim::match_::Slot {
            team: Team::Red,
            index: n.saturating_add(1),
            account: mush_core::AccountId::mint("floor", i64::from(n)).expect("an account id"),
            pilot: ObjId(u32::from(n) + 1),
            pod: None,
            unit: None,
            left: false,
            dropped_at_ms: None,
            unit_type: None,
        });
    }
    assert_eq!(
        match_::largest_side(&centre),
        match_::PODS_PER_SIDE,
        "the floor ran past the pods that exist"
    );
    assert_eq!(
        match_::ai_side_size(rig.tw.world(), &centre, &cfg),
        match_::PODS_PER_SIDE,
        "and so did the size it is a floor under"
    );
}

// ---------------------------------------------------------------------------
// (2) `B-33` — `SIM CONCEDE`
// ---------------------------------------------------------------------------

/// **One unit the arena hands back damaged** (16b cut D.3, `B-33`), with the cell
/// it moved and the value it moved to.
///
/// The check-in's plan is a **diff** between the unit the world still holds and
/// the one the arena returns ([`tacsim::match_::returned_plan`]), so a returned
/// unit that is a copy of the held one commits nothing and proves nothing. This
/// lowers one scalar cell, which is the shape all of doc 16 §3.5's damage has.
fn damaged_return(
    rig: &Rig,
    unit: ObjId,
) -> (
    tacsim::arena::damage::UnitReturn,
    mush_tactics::CellAddr,
    i64,
) {
    let held = mush_tactics::module::unit_of(rig.tw.world(), unit)
        .expect("a unit")
        .clone();
    let (addr, hurt) = held
        .cells()
        .find(|(_, cell)| cell.current > cell.min)
        .map(|(addr, cell)| (addr.clone(), cell.current.saturating_sub(1).max(cell.min)))
        .expect("the stock tank has a cell that can be taken down");
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetCell {
            addr: addr.clone(),
            value: hurt,
        }],
        None,
        Some(smol_str::SmolStr::new_static("test:damage")),
    )
    .expect("one op is a plan");
    let (cells, _journal) = mush_tactics::apply(&held, &plan, 0).expect("the cell is in bounds");
    let pose = mush_sim::pose_of(rig.tw.world(), unit)
        .map(|(_, pose)| pose)
        .expect("a placed tank");
    (
        tacsim::arena::damage::UnitReturn {
            unit,
            cells,
            pose,
            shots: 4,
            hits: 2,
            wreck: false,
            disabled: false,
        },
        addr,
        hurt,
    )
}

/// **A side's lead concedes, and the other side wins** (16b cut D.3,
/// `B-33`).
///
/// The announcement, the board's new line, the state and the outcome — and that
/// it is **not** a void: the cells are committed, so a tank the match damaged is
/// still damaged on its own unit document afterwards, exactly as after an
/// elimination.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_a_lead_concedes_and_the_other_side_wins() {
    let mut rig = Rig::boot(2);
    duel(&mut rig);
    let red = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    let (damaged, addr, hurt) = damaged_return(&rig, red);
    let before = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .cell(&addr)
        .expect("the cell is on the held unit")
        .current;

    let said = rig.say(0, "sim concede");
    assert!(
        said.contains(
            "Pilot0 concedes for the red side. Match 1 goes to blue when the arena answers."
        ),
        "{said}"
    );
    // The stop is in flight; the end lands when the arena answers, which here is
    // the egress driven by hand, carrying the cells the loser's tank came back
    // with.
    rig.end_match_returning(
        tacsim::arena::EndReason::Conceded { winner: Team::Blue },
        &[damaged],
    );
    assert_eq!(rig.state(), MatchState::Ended, "a concede is not an abort");
    assert_eq!(
        rig.centre().outcome,
        Some(tacsim::match_::MatchOutcome::Blue)
    );
    assert_eq!(
        rig.centre().outcome_reason,
        Some(tacsim::arena::EndReason::Conceded { winner: Team::Blue })
    );
    // **The tanks stand as evidence**: a concede is a result and not a void, so
    // the check-in **committed** the arena's cells rather than restoring the
    // check-out snapshot over them. The marker alone cannot say which happened —
    // a void clears it too — so the damage is what is read.
    assert!(
        !mush_sim::deployed::is_deployed(rig.tw.world(), red),
        "the units are checked in"
    );
    assert_ne!(hurt, before, "the fixture did no damage at all");
    assert_eq!(
        mush_tactics::module::unit_of(rig.tw.world(), red)
            .expect("a unit")
            .cell(&addr)
            .map(|cell| cell.current),
        Some(hurt),
        "a concede voided the match's own evidence"
    );
    let board = staff_board(&rig);
    assert!(
        board.contains("ended (match 1) — blue wins: red conceded"),
        "{board}"
    );
}

/// **The board names the winner of a concede either way** (16b cut D.3, `B-33`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_the_board_names_the_winner_of_a_concede_either_way() {
    let mut rig = Rig::boot(2);
    duel(&mut rig);
    let said = rig.say(1, "sim concede");
    assert!(
        said.contains(
            "Pilot1 concedes for the blue side. Match 1 goes to red when the arena answers."
        ),
        "{said}"
    );
    rig.end_match(tacsim::arena::EndReason::Conceded { winner: Team::Red });
    assert_eq!(
        rig.centre().outcome,
        Some(tacsim::match_::MatchOutcome::Red)
    );
    let board = staff_board(&rig);
    assert!(
        board.contains("ended (match 1) — red wins: blue conceded"),
        "{board}"
    );
}

/// **`SIM CONCEDE`'s guards, in order** (16b cut D.3, `B-33`): `Running`, a slot,
/// and the side's lead.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_concede_is_the_leads_alone_and_only_while_a_match_runs() {
    let mut rig = Rig::boot(4);
    // Outside `Running`: the state refusal, in `Idle`, in `Selection` and in
    // `Boarding`.
    let said = rig.say(0, "sim concede");
    assert!(said.contains("The simulator centre is idle;"), "{said}");
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team blue");
    let said = rig.say(0, "sim concede");
    assert!(
        said.contains("The simulator centre is selection;"),
        "{said}"
    );
    rig.say(0, "sim start");
    let said = rig.say(0, "sim concede");
    assert!(said.contains("The simulator centre is boarding;"), "{said}");

    assert!(!rig.seat(0));
    assert!(!rig.seat(1));
    assert!(rig.seat(2));
    assert_eq!(rig.state(), MatchState::Running);

    // A player in the centre holding no slot.
    let said = rig.say(3, "sim concede");
    assert!(
        said.contains("You hold no slot in the simulator."),
        "{said}"
    );
    // A pilot who holds a slot but does not lead their side.
    let said = rig.say(1, "sim concede");
    assert!(
        said.contains("Only a side's lead can concede for it."),
        "{said}"
    );
    assert_eq!(rig.state(), MatchState::Running, "and nothing moved");
    // Their lead may.
    let said = rig.say(0, "sim concede");
    assert!(said.contains("concedes for the red side"), "{said}");
}

/// **A concede the world could not take says nothing about a winner** (16b cut
/// D.3, `B-33`).
///
/// [`tacsim::match_::sim_abort`]'s stop funnel finishes a match whose task is
/// already gone **on the spot**, as an `EndReason::Abort` — doc 16 §3.8's *a match
/// with no arena has no result to concede* — and says so to the centre. The
/// concede sentence over the top of that would tell everybody a side had won a
/// match whose board, record and result document all say `abort`, with no arena
/// left to come back and correct it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_a_concede_the_arena_cannot_take_claims_no_winner() {
    let mut rig = Rig::boot(2);
    duel(&mut rig);
    let match_id = rig.centre().match_id.expect("a running match");

    // The arena task goes, the way a panic or a lost supervisor loses one.
    let (_handle, join) = rig
        .tasks()
        .remove(match_id)
        .expect("the running match's task");
    join.abort();

    let heard = rig.say_overheard(0, "sim concede", 1);
    assert!(
        heard.contains("Match 1 is over (abort). Its units are checked in."),
        "{heard}"
    );
    assert!(
        !heard.contains("concedes for the"),
        "a match with no arena announced a winner: {heard}"
    );
    assert_eq!(rig.state(), MatchState::Aborted);
    assert_eq!(
        rig.centre().outcome,
        Some(tacsim::match_::MatchOutcome::Abort),
        "the record says abort and the announcement said blue won"
    );
}

// ---------------------------------------------------------------------------
// (3) `B-33` — the unattended match
// ---------------------------------------------------------------------------

/// **A match every pilot has left ends itself, once, after the interval** (16b
/// cut D.3, `B-33`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_a_match_every_pilot_left_ends_itself_after_the_interval() {
    let mut rig = Rig::boot_on(2, roster_with(unattended_config(10_000)));
    duel(&mut rig);
    assert_eq!(rig.centre().unattended_since_ms, None, "both are aboard");

    // One leaves: still attended.
    rig.release(0);
    assert_eq!(rig.centre().unattended_since_ms, None, "one is still in it");
    rig.tick_to(30);
    assert_eq!(rig.state(), MatchState::Running, "one pilot held the match");

    // The last one leaves: the clock starts, at the world second it happened.
    rig.release(1);
    assert_eq!(rig.centre().unattended_since_ms, Some(30_000));

    // Not before the interval.
    rig.tick_to(39);
    assert_eq!(rig.state(), MatchState::Running);
    assert_eq!(rig.centre().unattended_since_ms, Some(30_000));

    // And at it: the stop is sent, the stamp is cleared, and the centre says so.
    let heard = rig.tick_heard(40, 0);
    assert!(
        heard.contains("Match 1 ends: every pilot has left it."),
        "{heard}"
    );
    assert_eq!(rig.centre().unattended_since_ms, None, "the stamp is spent");

    // A further tick says nothing more: the stamp is what armed it.
    let heard = rig.tick_heard(41, 0);
    assert!(
        !heard.contains("every pilot has left it"),
        "it announced twice: {heard}"
    );

    // The arena answers, and the match is a `timeout` with the new reason on it.
    rig.end_match(tacsim::arena::EndReason::Unattended);
    assert_eq!(rig.state(), MatchState::Ended);
    assert_eq!(
        rig.centre().outcome,
        Some(tacsim::match_::MatchOutcome::Timeout)
    );
    let board = staff_board(&rig);
    assert!(
        board.contains("ended (match 1) — timeout: every pilot had left"),
        "{board}"
    );
    // And the next crew can open the next match.
    rig.say(0, "sim team red");
    assert_eq!(rig.state(), MatchState::Selection);
}

/// **`0` is off, a slotless staff match is never stamped, and a pilot who has
/// not given their seat up holds the match open** (16b cut D.3, `B-33`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_the_unattended_clock_is_off_at_zero_and_never_stamps_a_slotless_match() {
    // `0` is off. The stamp is still written — the key is read at the deadline,
    // so an operator who turns it off under a live match turns this off with it.
    let mut rig = Rig::boot_on(2, roster_with(unattended_config(0)));
    duel(&mut rig);
    rig.release(0);
    rig.release(1);
    assert_eq!(rig.centre().unattended_since_ms, Some(0));
    rig.tick_to(600);
    assert_eq!(
        rig.state(),
        MatchState::Running,
        "a zero interval ended a match"
    );

    // A staff `@arena/start` match has no slots at all, so nothing can leave it.
    let mut rig = Rig::boot_on(1, roster_with(unattended_config(10_000)));
    let red = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), Team::Blue, 1).expect("blue 1's tank");
    let said = rig.staff(&format!(
        "@arena/start tacsim.arena.proving_ground=#{}/#{}",
        red.0, blue.0
    ));
    assert!(said.contains("is running"), "{said}");
    assert!(rig.centre().slots.is_empty(), "the shortcut fields no pods");
    rig.tick_to(600);
    assert_eq!(rig.centre().unattended_since_ms, None);
    assert_eq!(rig.state(), MatchState::Running);

    // And a pilot who is still in their pod holds it open for ever. Nothing here
    // reads a tank's condition: `slot_left` is the only writer of the stamp and
    // its one input is whether a seat was GIVEN UP, so a pilot sitting in a wreck
    // is attending exactly as a pilot sitting in a whole tank is.
    let mut rig = Rig::boot_on(2, roster_with(unattended_config(10_000)));
    duel(&mut rig);
    rig.release(0);
    rig.tick_to(600);
    assert_eq!(rig.centre().unattended_since_ms, None);
    assert!(
        rig.centre()
            .slots
            .iter()
            .any(|slot| !slot.left && slot.is_seated()),
        "the seated pilot was marked left"
    );
    assert_eq!(rig.state(), MatchState::Running);
}

/// **A stop the arena will not take keeps the stamp, warns once, and is retried
/// until it lands** (16b cut D.3, `B-33`).
///
/// [`tacsim::arena_bridge::send_stop`] answers a momentarily full inbox with
/// `ControlRefusal::Busy` (16b cut M.1, `B-49`; `Gone` before it), and nothing
/// ever re-stamps — so clearing the stamp on a failed send would disarm the path
/// for the rest of the match, in exactly the long lagging fight that most needs
/// it.
///
/// The inbox is filled on a **current-thread** runtime, where the arena task only
/// advances at an `await`: the offers below therefore stay in the queue, and the
/// `yield_now` further down is what lets the task drain them. **16b cut M.1**
/// keeps `3 × units + 2` of the inbox free of intents so a control or a stop
/// always fits, so intents alone no longer fill it: the reserve is filled with
/// harmless controls after them.
///
/// `Centre::unattended_warned` is what makes the `warn` once-per-match, so it is
/// the flag and not the log line that is asserted here.
#[tokio::test(flavor = "current_thread")]
async fn d3_an_unattended_stop_the_arena_will_not_take_is_kept_and_retried() {
    let mut rig = Rig::boot_on(2, roster_with(unattended_config(10_000)));
    duel(&mut rig);
    let match_id = rig.centre().match_id.expect("a running match");
    rig.release(0);
    rig.release(1);
    assert_eq!(rig.centre().unattended_since_ms, Some(0));

    // Fill the task's bounded inbox. Its bound is
    // `intents.max_pending_per_unit × units + 3 × units + 2`, so this terminates.
    let tasks = rig.tasks();
    let mut offered = 0_usize;
    while offered < 1_000
        && tasks
            .offer(
                match_id,
                // A **harmless** message: an order stamped with an epoch the
                // arena will refuse at apply. Filling the inbox with stops would
                // end the match the moment the task drained one.
                tacsim::arena_task::ArenaMsg::Intent(tacsim::arena::intents::Intent {
                    unit: ObjId(0),
                    channel: tacsim::arena::state::Channel::Helm,
                    epoch: 0,
                    kind: tacsim::arena::intents::IntentKind::Startup,
                    seq: 0,
                }),
            )
            .is_ok()
    {
        offered += 1;
    }
    assert!(offered > 0, "the inbox took nothing at all");
    assert!(offered < 1_000, "the inbox never filled");
    // 16b cut M.1: and the reserve, with controls for a unit the arena does not
    // hold — `apply_control` raises that key's epoch and nothing else.
    let mut reserved = 0_usize;
    while reserved < 1_000
        && tasks
            .offer(
                match_id,
                tacsim::arena_task::ArenaMsg::Control(tacsim::arena::intents::Control {
                    unit: ObjId(0),
                    channel: tacsim::arena::state::Channel::Helm,
                    epoch: 1,
                    state: tacsim::arena::intents::ControlState::Live,
                    pilot: None,
                }),
            )
            .is_ok()
    {
        reserved += 1;
    }
    assert!(reserved > 0, "the reserve took no control");
    assert!(reserved < 1_000, "the reserve never filled");

    // The deadline fires and the offer is refused: nothing is said, and the
    // stamp survives so the next tick tries again.
    let heard = rig.tick_heard(10, 0);
    assert!(
        !heard.contains("every pilot has left it"),
        "a refused stop was announced: {heard}"
    );
    assert_eq!(
        rig.centre().unattended_since_ms,
        Some(0),
        "a refused send disarmed the path"
    );
    assert!(
        rig.centre().unattended_warned,
        "nothing armed the once-per-match warning"
    );

    // A second tick with the inbox still full warns no further: the flag is what
    // makes "once" once, and it is already set.
    let heard = rig.tick_heard(11, 0);
    assert!(!heard.contains("every pilot has left it"), "{heard}");
    assert_eq!(rig.centre().unattended_since_ms, Some(0));
    assert!(rig.centre().unattended_warned);
    assert_eq!(rig.state(), MatchState::Running);

    // The arena drains its inbox, and a later tick lands the stop.
    for _ in 0..256 {
        tokio::task::yield_now().await;
    }
    let heard = rig.tick_heard(12, 0);
    assert!(
        heard.contains("Match 1 ends: every pilot has left it."),
        "the retry never landed: {heard}"
    );
    assert_eq!(rig.centre().unattended_since_ms, None);
    assert!(!rig.centre().unattended_warned);

    // **And the stop the retry landed is still the unattended one.** The retry is
    // what a refactor would most easily route through `crate::match_::abort`,
    // whose no-task arm ends a match as an `Abort` and voids its cells — which
    // would leave the sentence above unchanged and the record saying something
    // else entirely. So the end is walked to the board.
    rig.end_match(tacsim::arena::EndReason::Unattended);
    assert_eq!(rig.state(), MatchState::Ended);
    assert_eq!(
        rig.centre().outcome,
        Some(tacsim::match_::MatchOutcome::Timeout)
    );
    assert_eq!(
        rig.centre().outcome_reason,
        Some(tacsim::arena::EndReason::Unattended)
    );
}

// ---------------------------------------------------------------------------
// (4) `B-35` — a refused check-in does not strand the seat
// ---------------------------------------------------------------------------

/// **A returned unit the tactics layer must refuse** (16b cut D.3, `B-35`): the
/// pod's own spec with every cell's ceiling raised tenfold and filled to it, so
/// `returned_plan`'s `SetCell` names a value the **held** unit's own bounds
/// forbid and `mush_tactics::unit::apply` refuses it — M32's refuse-not-clamp,
/// which is exactly the shape a check-in fails in.
fn over_full(rig: &Rig, unit: ObjId) -> mush_tactics::TacticalUnit {
    let mut spec = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the tacsim store")
        .definitions()
        .bindings
        .get(&mush_core::CanonicalId::new("tacsim.unit.line_tank").expect("a canonical id"))
        .expect("the stock tank is in the catalog")
        .spec
        .clone();
    for section in &mut spec.sections {
        for facet in &mut section.facets {
            for cell in &mut facet.cells {
                cell.max = cell.max.saturating_mul(10).max(1);
                cell.init = cell.max;
            }
        }
    }
    mush_tactics::TacticalUnit::from_spec(&spec, unit).expect("a doctored unit instantiates")
}

/// **A check-in the tactics layer refuses releases the seat and restores the
/// snapshot** (16b cut D.3, `B-35`).
///
/// The old arm logged and `continue`d *before* `release_seat`, so the pilot kept
/// the cockpit billet, the control lease and the focus on a tank that was no
/// longer in a match, and that pod was unboardable for the rest of the boot. The
/// new order is: release the seat, retry the return as a **void**, record
/// `custody_lost`, and never let one unit's failure skip another's release.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_a_refused_check_in_releases_the_seat_and_restores_the_snapshot() {
    let mut rig = Rig::boot(2);
    duel(&mut rig);
    let match_id = rig.centre().match_id.expect("a running match");
    let red = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), Team::Blue, 1).expect("blue 1's tank");
    let before = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    let refused = over_full(&rig, red);
    let pose = mush_sim::pose_of(rig.tw.world(), red)
        .map(|(_, pose)| pose)
        .expect("a placed tank");

    let mut out = Outbox::new();
    {
        let engine = rig.tw.engine_mut();
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
            reason: tacsim::arena::EndReason::Abort,
            t_ms: 12_000,
        };
        assert!(match_::finish(
            &mut ctx,
            match_id,
            end,
            "0".repeat(64),
            &[tacsim::arena::damage::UnitReturn {
                unit: red,
                cells: refused,
                pose,
                shots: 3,
                hits: 1,
                wreck: false,
                disabled: false,
            }],
        ));
    }

    // Both units are back: neither is stranded, so one unit's failure never
    // skipped the other's release.
    for unit in [red, blue] {
        assert!(
            !mush_sim::deployed::is_deployed(rig.tw.world(), unit),
            "#{} kept its marker",
            unit.0
        );
    }
    // The seat and the billet went with it — for the unit whose plan was refused,
    // which is the half that used to be skipped.
    assert!(
        match_::flying(rig.tw.world(), rig.pilot(0)).is_none(),
        "the refused unit's pilot still holds a station"
    );
    assert!(match_::flying(rig.tw.world(), rig.pilot(1)).is_none());
    // The void retry restored the check-out snapshot, so no cell of the arena's
    // reached the record.
    let after = mush_tactics::module::unit_of(rig.tw.world(), red)
        .expect("a unit")
        .clone();
    for (addr, cell) in before.cells() {
        assert_eq!(
            after.cell(addr).map(|held| held.current),
            Some(cell.current),
            "#{}'s {addr:?} was written by a refused return",
            red.0
        );
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the match still ended");
}

// ---------------------------------------------------------------------------
// (5) `T-49` — `@arena/release <unit>`
// ---------------------------------------------------------------------------

/// **`@arena/release`'s three answers, and a pod that can be boarded again**
/// (16b cut D.3, `T-49`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_arena_release_clears_a_stranded_marker_and_frees_the_pod() {
    let mut rig = Rig::boot(2);
    duel(&mut rig);
    let match_id = rig.centre().match_id.expect("a running match");
    let red = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");

    // (a) While the match is live, the verb refuses and names the match.
    let said = rig.staff(&format!("@arena/release #{}", red.0));
    assert!(
        said.contains(&format!(
            "#{} is in match {match_id}, which is still live; `@arena/abort` it first.",
            red.0
        )),
        "{said}"
    );
    assert!(mush_sim::deployed::is_deployed(rig.tw.world(), red));
    // **16b cut U.4**: and `@arena/status` does not call a live match's marker
    // stranded.
    let said = rig.staff("@arena/status");
    assert!(!said.contains("stranded"), "{said}");

    // The match ends, and a marker is left behind the way a refused check-in
    // leaves one: a real check-out, so BOTH halves of the marker are there — the
    // tactics record's and the simulation's — which is the state a stranded unit
    // is really in and the state the verb has to be able to undo.
    rig.end_match(tacsim::arena::EndReason::Abort);
    assert!(rig.state().is_terminal());
    rig.strand(red, match_id.0);

    // **16b cut U.4 (16c `T-67`)**: `@find` no longer lists a unit any marker
    // holds, so `@arena/status` is where staff read the dbref the repair takes.
    let said = rig.staff("@arena/status");
    assert!(
        said.contains(&format!(
            "#{} carries a stranded deployed marker (match {}); @arena/release #{}",
            red.0, match_id.0, red.0
        )),
        "{said}"
    );
    let said = rig.staff("@find Red Tank 1");
    assert!(
        said.contains("(0 found)") && !said.contains(&format!("#{}", red.0)),
        "{said}"
    );
    // A staff surface: a pilot asking is refused, and reads no dbref.
    let said = rig.say(0, "@arena/status");
    assert!(
        said.contains("Permission denied.")
            && !said.contains("stranded")
            && !said.contains(&format!("#{}", red.0)),
        "a pilot read the stranded line: {said}"
    );

    // (b) Now it is stranded, and the verb releases it.
    let said = rig.staff(&format!("@arena/release #{}", red.0));
    assert!(
        said.contains(&format!(
            "#{} is released: its check-out snapshot is restored and its seat is free.",
            red.0
        )),
        "{said}"
    );
    assert!(!mush_sim::deployed::is_deployed(rig.tw.world(), red));

    // (c) And a second release finds nothing to do.
    let said = rig.staff(&format!("@arena/release #{}", red.0));
    assert!(
        said.contains(&format!("#{} carries no deployed marker.", red.0)),
        "{said}"
    );
    let said = rig.staff("@arena/status");
    assert!(
        !said.contains("stranded"),
        "a released unit is not stranded: {said}"
    );

    // After it a pilot can board that pod again.
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    assert!(!rig.seat(0), "the pod took a pilot again");
}

// ---------------------------------------------------------------------------
// (6) `B-31` — `@arena/reset`'s three headlines
// ---------------------------------------------------------------------------

/// **The reset says which of three things it found** (16b cut D.3, `B-31`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_the_reset_has_a_headline_for_a_centre_that_is_idle_but_not_clean() {
    let mut rig = Rig::boot(2);
    // (a) Something changed.
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let lobby = match_::centre_object(rig.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
    rig.tw
        .world_mut()
        .move_to(GOD, lobby)
        .expect("staff walk in");
    rig.staff("sim abort");
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("The simulator centre is reset; its slots and pods are free."),
        "{said}"
    );

    // (b) Nothing at all.
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("The simulator centre is already idle; nothing changed."),
        "{said}"
    );

    // (c) Idle, and not clean: a roster tank carrying a stale marker is skipped,
    // so the pass writes nothing and has a great deal to say.
    let red = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    mush_sim::deployed::mark(
        rig.tw.world_mut(),
        red,
        mush_sim::DeployedMark::new(99, "tacsim.arena.proving_ground"),
    );
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("The simulator centre is idle, but not clean:"),
        "{said}"
    );
    assert!(
        said.contains(&format!(
            "#{} is still deployed and was left alone; `@arena/abort` its match first.",
            red.0
        )),
        "{said}"
    );
    assert!(
        !said.contains("already idle; nothing changed"),
        "the headline contradicted the row under it: {said}"
    );
}

// ---------------------------------------------------------------------------
// (7) `B-36` — which "you are not flying" line a pilot is owed
// ---------------------------------------------------------------------------

/// **`NOT_STARTED_YET` is `Boarding`'s alone, and a roster tank's alone** (16b
/// cut D.3, `B-36`).
///
/// The sentence promises *the match has not started yet*, and the old test was
/// "holds a cockpit billet on an undeployed unit" — true of a staffer sitting in
/// a unit of their own in an `Idle` centre, and of a pilot whose match ended an
/// hour ago. Both were told to wait for a match nobody was going to start.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn d3_the_not_started_yet_line_is_boardings_alone() {
    let mut rig = Rig::boot(2);
    // An `Idle` centre: nobody is waiting for anything.
    let said = rig.say(0, "status");
    assert!(
        said.contains(tacsim::match_::OUTSIDE_COCKPIT),
        "an idle centre answered the boarding line: {said}"
    );

    // In `Boarding`, with a real billet on a roster tank — a real `BOARD`, which
    // is the only thing that makes one — the boarding line.
    // The control-lease mirror is a coarse projection and
    // `request_seat_activation` fails closed on an unread one, so one second of
    // world time happens before anybody boards — exactly as on a live server
    // before its first player types anything.
    rig.tick_to(1);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    let red_room =
        match_::centre_object(rig.tw.world(), match_::RED_ROOM_ENTRY).expect("the red team room");
    let pilot = rig.pilot(0);
    rig.tw
        .world_mut()
        .move_to(pilot, red_room)
        .expect("a pilot walks into their team room");
    let said = rig.say(0, "board");
    assert!(said.contains("climb into"), "{said}");
    let said = rig.say(0, "status");
    assert!(
        said.contains(tacsim::match_::NOT_STARTED_YET),
        "a boarded pilot was told they were nowhere: {said}"
    );
    // And the pilot who has not boarded, who holds no billet at all, still gets
    // the other one.
    let said = rig.say(1, "status");
    assert!(said.contains(tacsim::match_::OUTSIDE_COCKPIT), "{said}");

    // **The case the row was written for, and the one that tells the two rules
    // apart.** A machine of staff's own, outside the centre's roster, with a real
    // cockpit billet on it and nothing deployed: the old rule asked only "holds a
    // cockpit billet on an undeployed unit" and answered *the match has not
    // started yet* — about a match this billet has nothing to do with. The centre
    // is in `Boarding` here on purpose, so the state half of the new rule is
    // satisfied and the **roster** half is the only thing refusing.
    let lobby = match_::centre_object(rig.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
    let own = rig
        .tw
        .world_mut()
        .create(mush_core::ObjKind::Thing, "Spare Tank", GOD, Some(lobby))
        .expect("staff makes a thing");
    let said = rig.staff(&format!("@unit/create #{}=tacsim.unit.line_tank", own.0));
    assert!(said.contains("is now a tactical unit"), "{said}");
    rig.station_on(1, own);
    assert!(
        !mush_sim::deployed::is_deployed(rig.tw.world(), own),
        "the spare is deployed, which the old rule excluded anyway"
    );
    let said = rig.say(1, "status");
    assert!(
        said.contains(tacsim::match_::OUTSIDE_COCKPIT),
        "a billet on a machine the centre does not field was told to wait for the \
         centre's match: {said}"
    );
    assert!(!said.contains(tacsim::match_::NOT_STARTED_YET), "{said}");

    // The match is called off: the billet is given back with the pod, and the
    // sentence a pilot who types a cockpit verb now reads is the other one —
    // where before this cut it was still "the match has not started yet", for as
    // long as anything held a billet on an undeployed unit. Pilot 1's billet on
    // the spare is still standing, so this is also the terminal half of the rule:
    // a billet nothing ejected, in a centre that is not `Boarding`.
    rig.tw
        .world_mut()
        .move_to(GOD, lobby)
        .expect("staff walk in");
    rig.staff("sim abort");
    assert!(rig.state().is_terminal());
    let said = rig.say(0, "status");
    assert!(
        said.contains(tacsim::match_::OUTSIDE_COCKPIT),
        "a terminal centre answered the boarding line: {said}"
    );
    // The spare's billet was never the centre's to give back, so it is still
    // held — and the terminal centre is what refuses now, not the missing seat.
    assert!(
        rig.holds_billet(1, own),
        "the eject swept a billet on a unit the centre does not own"
    );
    let said = rig.say(1, "status");
    assert!(
        said.contains(tacsim::match_::OUTSIDE_COCKPIT),
        "a terminal centre answered the boarding line to a billet-holder: {said}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut B.4 — the lead who leaves the tank (cut AA's residual)
// ---------------------------------------------------------------------------

/// **A lead who leaves the tank mid-match hands the side on** (16b cut B.4).
///
/// Before this cut `slot_left` set `left` and kept the lead, so no succession
/// ran: the side's next pilot, still fighting, read `Only a side's lead can
/// concede for it.` and the lead standing in the team room still held the
/// right. The release goes through [`Rig::release`] — `send_control`'s
/// `Released`, the seam `EXIT`, a `QUIT` and the linkdead grace all reach.
///
/// Red has three pilots, and the one claimed second leaves first: the heir
/// search must skip that earlier claimant, who has left, and not only the
/// outgoing lead — or the pilot standing in the team room would inherit the
/// right to concede for the side still fighting.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b4_slot_left_promotes_the_next_pilot_in_the_tank() {
    let mut rig = Rig::boot(4);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team red");
    rig.say(3, "sim team blue");
    rig.say(0, "sim start");
    assert!(!rig.seat(0));
    assert!(!rig.seat(1));
    assert!(!rig.seat(2));
    assert!(rig.seat(3));
    assert_eq!(rig.state(), MatchState::Running);
    assert!(rig.centre().leads_side(Team::Red, account(0)));

    // The second claimant leaves first: a non-lead leaving moves nothing.
    rig.release(1);
    assert!(
        rig.centre().leads_side(Team::Red, account(0)),
        "a non-lead leaving moves nothing"
    );

    rig.release(0);
    let centre = rig.centre();
    assert!(
        centre
            .slot_of_pilot(rig.pilot(0))
            .is_some_and(|slot| slot.left),
        "the lead's slot is marked left and stays on the board"
    );
    assert!(
        centre.leads_side(Team::Red, account(2)),
        "red's next pilot still in a tank leads it: {:?}",
        centre.leads
    );
    assert!(
        !centre.leads_side(Team::Red, account(1)),
        "the earlier claimant who left is skipped"
    );
    assert!(
        !centre.leads_side(Team::Red, account(0)),
        "the lead who left leads nothing"
    );
    assert!(centre.leads_side(Team::Blue, account(3)), "blue untouched");

    // The pilots who left are refused in the lead's words; the heir concedes.
    for n in [0, 1] {
        let said = rig.say(n, "sim concede");
        assert!(
            said.contains("Only a side's lead can concede for it."),
            "pilot {n}: {said}"
        );
    }
    assert_eq!(rig.state(), MatchState::Running, "and nothing moved");
    let said = rig.say(2, "sim concede");
    assert!(said.contains("concedes for the red side"), "{said}");
}

/// **A side whose pilots have all left has no lead** (16b cut B.4): nobody in
/// a tank can speak for it, so there is no entry at all — the same answer as a
/// side whose last slot went on a `SIM LEAVE`. The other side keeps its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn b4_a_side_whose_pilots_all_left_has_no_lead() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team blue");
    rig.say(0, "sim start");
    assert!(!rig.seat(0));
    assert!(!rig.seat(1));
    assert!(rig.seat(2));
    assert_eq!(rig.state(), MatchState::Running);

    rig.release(1);
    assert!(
        rig.centre().leads_side(Team::Red, account(0)),
        "a non-lead leaving moves nothing"
    );
    rig.release(0);
    let centre = rig.centre();
    assert!(
        !centre.leads.iter().any(|(side, _)| *side == Team::Red),
        "red has no lead once every red pilot has left: {:?}",
        centre.leads
    );
    assert!(centre.leads_side(Team::Blue, account(2)));
    assert_eq!(centre.slots.len(), 3, "every slot stays on the board");
    for n in [0, 1] {
        let said = rig.say(n, "sim concede");
        assert!(
            said.contains("Only a side's lead can concede for it."),
            "pilot {n}: {said}"
        );
    }
    assert_eq!(rig.state(), MatchState::Running);
}

// ---------------------------------------------------------------------------
// 16b cut M.1 — `B-49`: a busy arena is never an abort that voids
// ---------------------------------------------------------------------------

/// **A check-out of the running match's own tanks**, over the stand-in window —
/// what a replacement task for the same match id is built from. The arena it
/// builds is never stepped; it exists so its task can hold an inbox.
fn replacement_custody(rig: &Rig) -> tacsim::arena::state::CustodyOut {
    use tacsim::match_::{STAND_IN, custody_unit, resolution_for, stock_definition};

    let w = rig.tw.world();
    let catalog = w
        .module_state::<tacsim::TacsimStore>()
        .expect("the tacsim store")
        .definitions()
        .clone();
    let match_id = rig.centre().match_id.expect("a running match");
    let record = w
        .module_state::<tacsim::match_::MatchBook>()
        .and_then(|book| book.get(match_id))
        .expect("the running match's record")
        .clone();
    let cell = i64::from(tacsim::match_::ARENA_CELL_CM);
    let posture = mush_sim::data::passive_rules_value(serde_json::json!({}))
        .expect("the engine's own passive posture");
    let mut units = Vec::new();
    for (column, (team, unit)) in record
        .teams
        .iter()
        .flat_map(|roster| roster.units.iter().map(|unit| (roster.team, *unit)))
        .enumerate()
    {
        let cells = mush_tactics::module::unit_of(w, unit)
            .expect("the match's tank is a unit")
            .clone();
        let x = i64::try_from(column).unwrap_or(0) * 4 + 2;
        let pose = mush_sim::data::Pose {
            x_cm: x * cell + cell / 2,
            y_cm: 4 * cell + cell / 2,
            z_cm: 0,
            heading_mdeg: 0,
            posture: posture.clone(),
        };
        units.push(custody_unit(
            &catalog,
            cells,
            unit,
            team,
            pose,
            0,
            tacsim::arena::state::Controller::Pilot,
            0,
        ));
    }
    tacsim::arena::state::CustodyOut {
        definition: stock_definition(&smol_str::SmolStr::new_static(STAND_IN))
            .expect("the stand-in window"),
        units,
        pilots: Vec::new(),
        resolution: resolution_for(&catalog),
        seed: 2_396,
    }
}

/// **A busy arena is told to try again, and nothing changes** (16b cut M.1,
/// `B-49`).
///
/// A running task empties its inbox the moment a message arrives, so a full one
/// cannot be made on the live task. The running match's task is therefore
/// replaced, under the same id, by one spawned on a current-thread runtime this
/// test never drives: its inbox is filled with orders and then — because an
/// order may never take the `3 × units + 2` slots kept for controls, stops and
/// acks — with harmless controls, so a `Stop` meets a full inbox of a live task.
/// `SIM ABORT`, `SIM CONCEDE` and `@arena/abort` each say their try-again
/// sentence and leave the record `Running` with no result. Then the runtime goes,
/// taking the task with it and no result behind it, and `SIM ABORT` finishes the
/// match on the spot as it always did.
///
/// Before this cut every refused offer was `Gone`, so the first `SIM ABORT` here
/// would have finished the match with no units and restored every tank from its
/// check-out snapshot over the match's real damage.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m1_a_busy_arena_is_told_to_try_again_and_nothing_changes() {
    let mut rig = Rig::boot(2);
    duel(&mut rig);
    let match_id = rig.centre().match_id.expect("a running match");
    let tasks = rig.tasks();

    // The replacement, on a runtime nobody drives.
    let undriven = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime");
    let cfg = config();
    let (handle, join, _callsigns) = tacsim::arena_task::spawn_arena(
        undriven.handle(),
        match_id,
        replacement_custody(&rig),
        &cfg.arena,
        &cfg.match_,
        &cfg.sensors,
        tasks.egress_sender().expect("the egress sender"),
    )
    .expect("the replacement check-out is accepted");
    let (displaced, displaced_join) = tasks
        .insert(match_id, handle, join)
        .expect("the running match's own task is displaced");
    displaced.signal_stop();
    tokio::time::timeout(std::time::Duration::from_secs(5), displaced_join)
        .await
        .expect("the displaced task stops")
        .expect("the displaced task did not panic");

    // Fill it: orders until the inbox refuses one, then controls into the
    // reserve until it refuses those too.
    let mut orders = 0_usize;
    while orders < 1_000
        && tasks
            .offer(
                match_id,
                tacsim::arena_task::ArenaMsg::Intent(tacsim::arena::intents::Intent {
                    unit: ObjId(0),
                    channel: tacsim::arena::state::Channel::Helm,
                    epoch: 0,
                    kind: tacsim::arena::intents::IntentKind::Startup,
                    seq: 0,
                }),
            )
            .is_ok()
    {
        orders += 1;
    }
    assert!(
        orders > 0 && orders < 1_000,
        "the inbox took {orders} orders"
    );
    let mut controls = 0_usize;
    while controls < 1_000
        && tasks
            .offer(
                match_id,
                tacsim::arena_task::ArenaMsg::Control(tacsim::arena::intents::Control {
                    unit: ObjId(0),
                    channel: tacsim::arena::state::Channel::Helm,
                    epoch: 1,
                    state: tacsim::arena::intents::ControlState::Live,
                    pilot: None,
                }),
            )
            .is_ok()
    {
        controls += 1;
    }
    assert_eq!(controls, 8, "the reserve is `3 × units + 2` for two units");
    assert!(tasks.is_live(match_id), "the replacement task is live");

    let untouched = |rig: &Rig| {
        let record = rig
            .tw
            .world()
            .module_state::<tacsim::match_::MatchBook>()
            .and_then(|book| book.get(match_id))
            .expect("the record")
            .clone();
        assert_eq!(record.state, MatchState::Running, "the record moved");
        assert!(record.ended.is_none(), "a result was written");
        assert_eq!(rig.state(), MatchState::Running, "the centre moved");
    };

    // `SIM ABORT`: the sentence to the one who typed it, and nothing to the room.
    let heard = rig.say_overheard(1, "sim abort", 0);
    assert!(
        !heard.contains("aborts match"),
        "a busy abort was announced: {heard}"
    );
    let said = rig.say(1, "sim abort");
    assert!(
        said.contains("The arena is busy; try that again in a moment."),
        "{said}"
    );
    untouched(&rig);

    // `SIM CONCEDE`, from red's lead: the same sentence, and no concede.
    let said = rig.say(0, "sim concede");
    assert!(
        said.contains("The arena is busy; try that again in a moment."),
        "{said}"
    );
    assert!(!said.contains("concedes for the"), "{said}");
    untouched(&rig);

    // `@arena/abort`: the verb's own sentence.
    let said = rig.staff(&format!("@arena/abort {match_id}"));
    assert!(
        said.contains(&format!(
            "Match {match_id} is busy; try @arena/abort again in a moment."
        )),
        "{said}"
    );
    untouched(&rig);

    // The runtime goes, and the task with it — no result behind it.
    undriven.shutdown_background();
    for _ in 0..200 {
        if !tasks.is_live(match_id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(!tasks.is_live(match_id), "the undriven task's inbox closed");
    assert_eq!(
        tasks.send_stop(match_id, tacsim::arena::state::StopReason::Abort),
        Err(tacsim::arena_bridge::ControlRefusal::Gone),
        "a task that died without a result is `Gone`"
    );
    let said = rig.say(1, "sim abort");
    assert!(
        said.contains(&format!("Match {match_id} is over (abort).")),
        "a match with no task is finished on the spot: {said}"
    );
    assert_eq!(rig.state(), MatchState::Aborted);
}

// ---------------------------------------------------------------------------
// 16b cut U.4 (16c `T-52`) — `@arena/pose`
// ---------------------------------------------------------------------------

/// Claim, start and really `BOARD` both pods from their team rooms, which is what
/// puts each pilot on the arena's frame roster.
fn board_both(rig: &mut Rig) {
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    for (n, entry) in [(0, match_::RED_ROOM_ENTRY), (1, match_::BLUE_ROOM_ENTRY)] {
        let room = match_::centre_object(rig.tw.world(), entry).expect("a team room");
        let pilot = rig.pilot(n);
        rig.tw
            .world_mut()
            .move_to(pilot, room)
            .expect("a pilot walks into their team room");
        let said = rig.say(n, "board");
        assert!(said.contains("climb into"), "{said}");
    }
    assert_eq!(rig.state(), MatchState::Running);
}

/// **Where red slot 1's tank opened this match**: the arena cell and heading on
/// the first frame the arena sent its pilot in the centre's current match. The
/// tank is cold until somebody types `STARTUP`, so the first frame is the
/// opening pose.
async fn opening_of_red_one(rig: &mut Rig) -> ((i64, i64), u32) {
    let match_id = rig.centre().match_id.expect("a running centre has a match");
    let pilot = rig.pilot(0);
    for _ in 0..600 {
        rig.pump();
        let held = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(pilot))
            .filter(|held| held.match_id == match_id)
            .map(|held| {
                let pose = &held.frame.pose;
                ((pose.x_cm / 1_000, pose.y_cm / 1_000), pose.heading_mdeg)
            });
        if let Some(opening) = held {
            return opening;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("the arena sent red slot 1's pilot no frame");
}

/// **`@arena/pose`: an opening cell for the next match, and no later one** (16b
/// cut U.4, 16c `T-52`).
///
/// The override is set on an `Idle` centre, survives a lone claimant's `SIM
/// TEAM` and `SIM LEAVE` (both of which return the centre to `Idle` and rebuild
/// it), opens red slot 1 on its cell at its heading, and is gone once that match
/// ends — so the next match opens on the authored cell. One set on the terminal
/// centre between matches is the next match's in turn, and `@arena/reset` clears
/// one nobody has used. Every refusal says its own sentence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn u4_arena_pose_opens_a_slot_on_its_cell_for_the_next_match_and_no_later() {
    let mut rig = Rig::boot(2);
    // The control-lease mirror, read once before anybody boards.
    rig.tick_to(1);
    let red = match_::tank_for_slot(rig.tw.world(), Team::Red, 1).expect("red 1's tank");
    let authored = mush_sim::pose_of(rig.tw.world(), red)
        .expect("the applier placed it")
        .1;
    let authored_cell = (authored.x_cm / 1_000, authored.y_cm / 1_000);

    // The bare read, on nothing.
    let said = rig.staff("@arena/pose");
    assert!(said.contains("No opening cell is overridden."), "{said}");

    // A staff switch: a pilot is refused, moves nothing, and reads no list.
    let said = rig.say(0, "@arena/pose RA=90 97");
    assert!(said.contains("Permission denied."), "{said}");
    let said = rig.say(0, "@arena/pose");
    assert!(
        said.contains("Permission denied.") && !said.contains("overridden"),
        "{said}"
    );
    assert!(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .is_some_and(|book| book.opening_cells().is_empty()),
        "a pilot set an opening cell"
    );

    // Every refusal, in the order they are asked.
    let said = rig.staff("@arena/pose RZ=90 97");
    assert!(
        said.contains("There is no slot RZ on this centre."),
        "{said}"
    );
    let said = rig.staff("@arena/pose RA=300 97");
    assert!(
        said.contains("Cell (300, 97) is off the arena window."),
        "{said}"
    );
    let (bx, by) = {
        let definition = match_::definition_for(
            rig.tw.world(),
            &smol_str::SmolStr::new_static(match_::CENTRE_ARENA),
        )
        .expect("the centre's ground builds")
        .expect("the centre's arena");
        let terrain = &definition.terrain;
        (0..terrain.height())
            .flat_map(|y| (0..terrain.width()).map(move |x| (x, y)))
            .find(|(x, y)| !terrain.has_way_out(*x, *y))
            .expect("the proving ground has a cell nobody can open on")
    };
    let said = rig.staff(&format!("@arena/pose RA={bx} {by}"));
    assert!(
        said.contains(&format!(
            "Cell ({bx}, {by}) is impassable, or has no way out."
        )),
        "{said}"
    );
    let said = rig.staff("@arena/pose RA=90 97 360");
    assert!(said.contains("Heading is 0 to 359."), "{said}");
    let said = rig.staff("@arena/pose RA");
    assert!(
        said.contains(
            "Usage: @arena/pose <callsign>=<x> <y>[ <heading>] | <callsign>=off | (bare) list"
        ),
        "{said}"
    );

    // Set, case-folded; a second slot on the same cell is refused by name.
    let said = rig.staff("@arena/pose ra=90 97 90");
    assert!(
        said.contains("RA opens at cell (90, 97) heading 90 for the next match."),
        "{said}"
    );
    let said = rig.staff("@arena/pose RB=90 97");
    assert!(
        said.contains("Cell (90, 97) is already RA's opening cell."),
        "{said}"
    );
    let said = rig.staff("@arena/pose RB=107 97");
    assert!(
        said.contains("RB opens at cell (107, 97) heading 90 for the next match."),
        "red's own facing when no heading is given: {said}"
    );
    let said = rig.staff("@arena/pose RB=off");
    assert!(
        said.contains("RB opens at its authored cell again."),
        "{said}"
    );
    let said = rig.staff("@arena/pose BC=off");
    assert!(
        said.contains("BC opens at its authored cell again."),
        "off on a slot with no override says the same: {said}"
    );
    let said = rig.staff("@arena/pose");
    assert!(
        said.contains("Opening cells overridden for the next match:")
            && said.contains("  RA: cell (90, 97) heading 90")
            && !said.contains("RB:"),
        "{said}"
    );

    // A lone claimant who leaves returns the centre to `Idle` — and the centre is
    // rebuilt from its default — and the override stands.
    rig.say(0, "sim team red");
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Idle);
    let said = rig.staff("@arena/pose");
    assert!(said.contains("  RA: cell (90, 97) heading 90"), "{said}");

    // Match 1 honours it, and it cannot be changed under the running match.
    board_both(&mut rig);
    let said = rig.staff("@arena/pose RA=94 97");
    assert!(
        said.contains(
            "The simulator centre is running; set opening cells before the match starts."
        ),
        "{said}"
    );
    assert_eq!(
        opening_of_red_one(&mut rig).await,
        ((90, 97), 90_000),
        "red slot 1 opens on the cell staff chose"
    );
    rig.end_match(tacsim::arena::EndReason::Abort);
    assert!(rig.state().is_terminal());
    let said = rig.staff("@arena/pose");
    assert!(
        said.contains("No opening cell is overridden."),
        "the match that used it is over: {said}"
    );

    // Match 2 opens on the authored cell.
    rig.tick_to(2);
    board_both(&mut rig);
    assert_eq!(
        opening_of_red_one(&mut rig).await,
        (authored_cell, authored.heading_mdeg),
        "the next match is back on the authored cell"
    );
    rig.end_match(tacsim::arena::EndReason::Abort);

    // An override set on the terminal centre, between matches, is the next
    // match's.
    let said = rig.staff("@arena/pose RA=94 97 180");
    assert!(
        said.contains("RA opens at cell (94, 97) heading 180 for the next match."),
        "{said}"
    );
    rig.tick_to(3);
    board_both(&mut rig);
    assert_eq!(opening_of_red_one(&mut rig).await, ((94, 97), 180_000));
    rig.end_match(tacsim::arena::EndReason::Abort);

    // And `@arena/reset` clears one nobody used — and says so, rather than
    // "nothing changed", on a centre whose only state was the override.
    let said = rig.staff("@arena/pose RA=90 97");
    assert!(said.contains("RA opens at cell (90, 97)"), "{said}");
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("The simulator centre is reset; its slots and pods are free.")
            && said.contains("1 opening cell override(s) cleared."),
        "{said}"
    );
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("The simulator centre is already idle; nothing changed.")
            && !said.contains("override(s) cleared"),
        "a second reset has nothing to clear: {said}"
    );
    let said = rig.staff("@arena/pose RA=90 97");
    assert!(said.contains("RA opens at cell (90, 97)"), "{said}");
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("The simulator centre is reset; its slots and pods are free.")
            && said.contains("1 opening cell override(s) cleared.")
            && !said.contains("nothing changed"),
        "an override is the only thing an idle, clean centre held: {said}"
    );
    let said = rig.staff("@arena/pose");
    assert!(said.contains("No opening cell is overridden."), "{said}");
}

// ---------------------------------------------------------------------------
// 16b cut V.2 — the result names every unit and counts its tape
// ---------------------------------------------------------------------------

/// **The result document names every unit by the arena's own callsign and
/// counts the tape, and the book remembers it** (16b cut V.2).
///
/// A real boarded duel, ended by a real `SIM ABORT` through the arena's own `Ended`. The
/// callsign each pilot's tank carries **in the arena** is read off the frame the
/// arena sent that pilot — `Frame::callsign` is `UnitState::callsign`, the same
/// field `Arena::callsigns` reads — and the result document must carry exactly
/// it on the tank's unit row and on the pilot's row. `ledger.chunks` must equal
/// the chunks the rig handed the world, and the book's history ring must hold
/// the row the document reads as.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_the_result_names_every_unit_by_the_arenas_callsign_and_counts_its_tape() {
    let mut rig = Rig::boot(2);
    // The control-lease mirror, read once before anybody boards; then a real
    // `BOARD` on each side, which is what puts a pilot on the arena's roster.
    rig.tick_to(1);
    board_both(&mut rig);
    let match_id = rig.centre().match_id.expect("a running match");
    let _ = rig.tw.take_module_ops();

    // The arena's own callsigns, off the frames it sent.
    let mut flown: BTreeMap<u64, (u64, String)> = BTreeMap::new();
    for _ in 0..600 {
        rig.pump();
        for n in 0..2 {
            let pilot = rig.pilot(n);
            if let Some(held) = rig
                .tw
                .world()
                .module_state::<tacsim::arena_bridge::ArenaRegistry>()
                .and_then(|registry| registry.last_frame(pilot))
                .filter(|held| held.match_id == match_id)
            {
                flown.insert(
                    u64::from(pilot.0),
                    (
                        u64::from(held.frame.unit.0),
                        held.frame.callsign.to_string(),
                    ),
                );
            }
        }
        if flown.len() == 2 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(flown.len(), 2, "the arena sent both pilots a frame");
    assert_eq!(
        flown
            .values()
            .map(|(_, callsign)| callsign.as_str())
            .collect::<Vec<_>>(),
        vec!["RA", "BA"],
        "slot 1 of each side, by 16b cut Q.1's rule"
    );

    // The end, through the arena.
    let said = rig.say(1, "sim abort");
    assert!(said.contains("aborts match"), "{said}");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");

    let key = format!("tacsim:match:{}", match_id.0);
    let document = rig
        .tw
        .take_module_ops()
        .into_iter()
        .filter(|op| op.ns == tacsim::NS)
        .find_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet {
                key: written,
                value,
            } if written == key => Some(value),
            _ => None,
        })
        .expect("the check-in wrote the result document");

    // Every unit row, and every pilot row, carries the arena's callsign.
    let units = document["units"].as_array().expect("unit rows");
    assert_eq!(units.len(), 2, "{document}");
    for (unit, callsign) in flown.values() {
        let row = units
            .iter()
            .find(|row| row["unit"].as_u64() == Some(*unit))
            .expect("the tank has a unit row");
        assert_eq!(row["callsign"].as_str(), Some(callsign.as_str()), "{row}");
    }
    let mut pilot_rows = 0;
    for team in document["teams"].as_array().expect("teams") {
        for row in team["pilots"].as_array().expect("pilot rows") {
            pilot_rows += 1;
            let pilot = row["pilot"].as_u64().expect("a pilot id");
            let (unit, callsign) = flown.get(&pilot).expect("a pilot the arena framed");
            assert_eq!(row["unit"].as_u64(), Some(*unit), "{row}");
            assert_eq!(row["callsign"].as_str(), Some(callsign.as_str()), "{row}");
        }
    }
    assert_eq!(pilot_rows, 2);

    // The tape: every chunk the world was handed, and at least the opening one.
    let seen = rig.chunks_seen.get(&match_id.0).copied().unwrap_or(0);
    assert!(seen >= 1, "the arena flushed its tape before its end");
    assert_eq!(
        document["ledger"]["chunks"].as_u64(),
        Some(seen),
        "{document}"
    );
    assert_eq!(document["record_version"].as_u64(), Some(1));

    // The book remembers exactly that document.
    let expected = match_::HistoryRow::from_document(&document).expect("a result document");
    assert!(expected.named);
    let book = rig
        .tw
        .world()
        .module_state::<match_::MatchBook>()
        .expect("the book");
    assert_eq!(book.remembered(match_id), Some(&expected));
    assert_eq!(
        book.history().map(|row| row.id).collect::<Vec<_>>(),
        vec![match_id]
    );
    let bystander = match_::history_lines(book.history(), GOD);
    assert_eq!(
        bystander,
        vec![
            "The centre's last match:".to_owned(),
            format!(
                "  #{}  aborted  {}  1 v 1  {}",
                match_id.0,
                match_::duration_words(expected.arena_ms),
                match_::civil_utc(expected.ended_ms / 1_000)
            ),
        ]
    );

    // The authored arena's name, and the id itself for an arena with none.
    assert_eq!(
        match_::arena_label(rig.tw.world(), match_::CENTRE_ARENA),
        "The Proving Ground"
    );
    assert_eq!(
        match_::arena_label(rig.tw.world(), match_::STAND_IN),
        match_::STAND_IN
    );
}

/// The `tacsim` writes one `write_restart_abort` pushes, each as `(set|del, key)`
/// (16b cut V.2), through a real `Ctx` on the rig's own world.
fn restart_abort_writes(rig: &mut Rig, match_id: u64) -> Vec<(&'static str, String)> {
    let mut out = Outbox::new();
    let engine = rig.tw.engine_mut();
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
        .map(|op| {
            assert_eq!(op.ns, tacsim::NS, "{op:?}");
            match op.op {
                mush_core::ModuleWrite::JsonSet { key, .. } => ("set", key.to_string()),
                mush_core::ModuleWrite::Del { key } => ("del", key.to_string()),
                other => panic!("a result write is a set or a delete: {other:?}"),
            }
        })
        .collect()
}

/// **A row the history ring lets go takes its report documents with it, in the
/// same write** (16b cut V.2; the report documents are 16b cut V.3's, and this is
/// what keeps them bounded by the ring).
///
/// Driven through `write_restart_abort`, the result writer an integration test
/// can reach; `record::write_result` ends in the same private tail
/// (`remember_and_push`), so one statement covers both. Three cases: a newer
/// match pushes the oldest row out and deletes exactly that row's reports and
/// nobody else's; a match older than every row held falls straight off and,
/// having no reports, deletes nothing; a same-id rewrite replaces its row and
/// evicts nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v2_a_row_the_ring_evicts_has_its_report_documents_deleted_in_the_same_write() {
    let mut rig = Rig::boot(0);
    let row = |id: u64, reports: Vec<ObjId>| match_::HistoryRow {
        id: tacsim::arena_task::MatchId(id),
        outcome: match_::MatchOutcome::Abort,
        reason: smol_str::SmolStr::new("abort"),
        arena: smol_str::SmolStr::default(),
        started_ms: 0,
        ended_ms: 0,
        arena_ms: 0,
        log: String::new(),
        digest: String::new(),
        chunks: 0,
        seed: 0,
        units: Vec::new(),
        named: true,
        reports,
    };
    {
        let book = rig
            .tw
            .world_mut()
            .module_state_mut::<match_::MatchBook>()
            .expect("the book");
        for id in 101..=120_u64 {
            let reports = match id {
                101 => vec![ObjId(7), ObjId(9)],
                102 => vec![ObjId(11)],
                _ => Vec::new(),
            };
            assert!(book.remember(row(id, reports)).is_none(), "{id} fits");
        }
    }
    let ids = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .expect("the book")
            .history()
            .map(|row| row.id.0)
            .collect::<Vec<u64>>()
    };

    // (1) Match 121 pushes 101 out: its result, then 101's two reports — and
    //     not 102's, which the ring still holds.
    assert_eq!(
        restart_abort_writes(&mut rig, 121),
        vec![
            ("set", "tacsim:match:121".to_owned()),
            ("del", "tacsim:report:101:7".to_owned()),
            ("del", "tacsim:report:101:9".to_owned()),
        ]
    );
    assert_eq!(ids(&rig), (102..=121).rev().collect::<Vec<u64>>());

    // (2) Match 50 is older than every row held: it falls straight off the
    //     ring, and a row with no reports deletes nothing.
    assert_eq!(
        restart_abort_writes(&mut rig, 50),
        vec![("set", "tacsim:match:50".to_owned())]
    );
    assert_eq!(ids(&rig), (102..=121).rev().collect::<Vec<u64>>());

    // (3) Match 110 again replaces its own row: nothing is evicted, so 102's
    //     report stays.
    assert_eq!(
        restart_abort_writes(&mut rig, 110),
        vec![("set", "tacsim:match:110".to_owned())]
    );
    assert_eq!(ids(&rig), (102..=121).rev().collect::<Vec<u64>>());
}

// ---------------------------------------------------------------------------
// 16b cut V.3 — one report document per pilot, in the result's own action
// ---------------------------------------------------------------------------

/// **A finished match writes one report document per human pilot, and the
/// result names them** (16b cut V.3; 16c N2).
///
/// A real boarded duel: red's pilot types `STARTUP` and blue's does nothing, the
/// arena runs until red's frame reads power online, and a real `SIM ABORT` ends
/// it through the arena's own `Ended`. The check-in's writes must hold one
/// `tacsim:report:<match>:<unit>` document per pilot, written with the result,
/// and the result's `reports` must name both units in id order. Red's report
/// carries the one line its pilot did something for; blue's carries none; both
/// end on the abort, which the tape wrote and the folds folded as the chunks
/// passed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v3_a_finished_match_writes_one_report_per_pilot_and_the_result_names_them() {
    let mut rig = Rig::boot(2);
    rig.tick_to(1);
    board_both(&mut rig);
    let match_id = rig.centre().match_id.expect("a running match");
    let said = rig.say(0, "startup");
    assert!(!said.is_empty(), "the startup was answered");
    let red = rig.pilot(0);
    let mut online = false;
    for _ in 0..600 {
        rig.pump();
        online = rig
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.last_frame(red))
            .is_some_and(|held| {
                held.match_id == match_id && held.frame.power == tacsim::arena::state::Power::Online
            });
        if online {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(online, "red's power came online");
    let tanks: BTreeMap<u64, u64> = [0, 1]
        .into_iter()
        .filter_map(|n| {
            let pilot = rig.pilot(n);
            rig.tw
                .world()
                .module_state::<tacsim::arena_bridge::ArenaRegistry>()
                .and_then(|registry| registry.last_frame(pilot))
                .map(|held| (u64::from(pilot.0), u64::from(held.frame.unit.0)))
        })
        .collect();
    assert_eq!(tanks.len(), 2, "both pilots were framed");
    let _ = rig.tw.take_module_ops();

    let said = rig.say(1, "sim abort");
    assert!(said.contains("aborts match"), "{said}");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");

    let writes: BTreeMap<String, serde_json::Value> = rig
        .tw
        .take_module_ops()
        .into_iter()
        .filter(|op| op.ns == tacsim::NS)
        .filter_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => Some((key.to_string(), value)),
            _ => None,
        })
        .collect();
    let result = writes
        .get(&format!("tacsim:match:{}", match_id.0))
        .expect("the check-in wrote the result document");
    let mut units: Vec<u64> = tanks.values().copied().collect();
    units.sort_unstable();
    assert_eq!(result["reports"], serde_json::json!(units), "{result}");
    let row = match_::HistoryRow::from_document(result).expect("a result document");
    assert_eq!(
        row.reports
            .iter()
            .map(|unit| u64::from(unit.0))
            .collect::<Vec<_>>(),
        units
    );

    for (pilot, unit) in &tanks {
        let key = format!("tacsim:report:{}:{unit}", match_id.0);
        let document = writes
            .get(&key)
            .unwrap_or_else(|| panic!("no report document at {key}: {:?}", writes.keys()));
        assert_eq!(document["record"], serde_json::json!("tacsim.report"));
        assert_eq!(document["match"].as_u64(), Some(match_id.0));
        assert_eq!(document["unit"].as_u64(), Some(*unit));
        assert_eq!(document["pilot"].as_u64(), Some(*pilot));
        let report =
            match_::report::report_from_document(document).expect("a report document reads");
        assert!(!report.gap, "the rig handed the world the whole tape");
        assert_eq!(
            report
                .tail
                .last()
                .map(|line| line.ends_with("the match was aborted.")),
            Some(true),
            "{:?}",
            report.tail
        );
        if *pilot == u64::from(red.0) {
            assert_eq!(document["callsign"], serde_json::json!("RA"));
            assert!(
                report
                    .lines
                    .iter()
                    .any(|line| line.ends_with("  power online.")),
                "{:?}",
                report.lines
            );
        } else {
            assert_eq!(document["callsign"], serde_json::json!("BA"));
            assert!(report.lines.is_empty(), "{:?}", report.lines);
        }
    }
    // The fold is RAM on the record and was finished exactly once: the book's
    // record carries none after the check-in.
    assert!(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .and_then(|book| book.get(match_id))
            .is_none_or(|record| record.folds.is_empty()),
        "the folds were taken off the record"
    );
}

/// The one store read a step deferred: its correlation and its keys.
fn one_read(effects: &[mush_core::Effect]) -> (u64, Vec<smol_str::SmolStr>) {
    match effects {
        [
            mush_core::Effect::StoreRead {
                module,
                correlation,
                keys,
                ..
            },
        ] => {
            assert_eq!(module.as_str(), tacsim::NS);
            (*correlation, keys.clone())
        }
        other => panic!("expected exactly one StoreRead effect: {other:?}"),
    }
}

/// Serve a read from the persisted documents the way the world task's driver
/// does: one document per key, absent keys reported.
fn serve(
    kv: &BTreeMap<smol_str::SmolStr, serde_json::Value>,
    keys: Vec<smol_str::SmolStr>,
) -> mush_core::StoreReadResult {
    let mut docs = Vec::new();
    let mut absent = Vec::new();
    for key in keys {
        match kv.get(&key) {
            Some(value) => docs.push((key, value.clone())),
            None => absent.push(key),
        }
    }
    mush_core::StoreReadResult::Docs { docs, absent }
}

impl Rig {
    /// Run one line as `actor`, answering what it saw and the store reads the
    /// step deferred.
    fn read(&mut self, actor: ObjId, line: &str) -> (String, Vec<mush_core::Effect>) {
        let (events, effects) = self.tw.run_with_effects(actor, line);
        (self.seen(actor, &events), effects)
    }

    /// Re-enter one store read's result for `actor`, exactly as the world task
    /// routes it to the tacsim module's `StoreReadCompletion`.
    fn complete(
        &mut self,
        actor: ObjId,
        correlation: u64,
        result: &mush_core::StoreReadResult,
    ) -> String {
        let events = self.tw.apply(mush_core::WorldCommand::StoreReadDone {
            module: smol_str::SmolStr::new(tacsim::NS),
            actor,
            correlation,
            payload: result.to_payload(),
        });
        self.seen(actor, &events)
    }
}

/// **`SIM REPORT`'s refusals, in their order, and both reads through the
/// module's one `StoreReadCompletion`** (16b cut V.3; 16c N2).
///
/// Before anybody flew: the bare word has no match, and a word that is no
/// number is the usage line. While the match runs, its own id is `still
/// running` — checked before the ring, which never holds a live match, so a
/// reordered check would answer `not one of them` and fail here. After it: an
/// id the ring never held is `not one of them`, a player in the centre who
/// flew nothing `did not fly`, and the pilot's own read goes out as ONE store
/// read of ONE key, refuses a second while it is in flight, renders the
/// header and the tail when the documents the check-in wrote come back, is
/// spent by its completion (a replay renders nothing), and answers `still
/// being written` when the key is absent. Staff page the tape the check-in
/// appended: a page past its end is `no tape at chunk 16` under the header,
/// and the first page carries the seed and the abort, with no `Next:`
/// because the store returned fewer keys than it asked for.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn v3_sim_report_refuses_in_order_and_both_reads_complete_through_the_module() {
    let mut rig = Rig::boot(3);
    rig.tick_to(1);
    let said = rig.say(0, "sim report");
    assert!(
        said.contains("You have not flown a match the centre remembers."),
        "{said}"
    );
    let said = rig.say(0, "sim report twelve");
    assert!(said.contains("Usage: SIM REPORT [<match>]"), "{said}");

    board_both(&mut rig);
    let match_id = rig.centre().match_id.expect("a running match");
    let id = match_id.0;
    let said = rig.say(0, &format!("sim report {id}"));
    assert!(
        said.contains(&format!("Match {id} is still running.")),
        "{said}"
    );
    assert!(!said.contains("not one of them"), "{said}");

    let said = rig.say(1, "sim abort");
    assert!(said.contains("aborts match"), "{said}");
    for _ in 0..400 {
        if rig.state() == MatchState::Aborted {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
    }
    assert_eq!(rig.state(), MatchState::Aborted, "the check-in lands");
    // Every document the store would now hold, the last write of a key winning.
    let mut kv: BTreeMap<smol_str::SmolStr, serde_json::Value> = BTreeMap::new();
    for op in rig.tw.take_module_ops() {
        if op.ns != tacsim::NS {
            continue;
        }
        match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => {
                kv.insert(key, value);
            }
            mush_core::ModuleWrite::Del { key } => {
                kv.remove(&key);
            }
            mush_core::ModuleWrite::StreamAppend { .. } => {}
        }
    }

    let said = rig.say(0, "sim report 99");
    assert!(
        said.contains("The centre remembers its last 20 matches; match 99 is not one of them."),
        "{said}"
    );
    let said = rig.say(2, &format!("sim report {id}"));
    assert!(
        said.contains(&format!("You did not fly in match {id}.")),
        "{said}"
    );

    // The pilot's own read: one key, one read in flight.
    let red = rig.pilot(0);
    let (said, effects) = rig.read(red, "sim report");
    assert!(
        said.contains(&format!("Reading your report of match {id} …")),
        "{said}"
    );
    let (correlation, keys) = one_read(&effects);
    assert_eq!(keys.len(), 1, "{keys:?}");
    assert!(
        keys.iter()
            .all(|key| key.starts_with(&format!("tacsim:report:{id}:"))),
        "{keys:?}"
    );
    let (said, effects) = rig.read(red, "sim report");
    assert!(said.contains(match_::report::READ_IN_FLIGHT), "{said}");
    assert!(effects.is_empty(), "a refused read defers nothing");
    let whole = rig.complete(red, correlation, &serve(&kv, keys.clone()));
    for want in [
        format!("Match {id}"),
        "You flew RA: intact; 0 shots, 0 hits.".to_owned(),
        "You fired nothing.".to_owned(),
        "the match was aborted.".to_owned(),
    ] {
        assert!(whole.contains(&want), "{want:?} missing from:\n{whole}");
    }
    let replay = rig.complete(red, correlation, &serve(&kv, keys));
    assert!(
        replay.is_empty(),
        "a completion nobody waits on renders nothing: {replay}"
    );
    // A read that outran the commit.
    let (_, effects) = rig.read(red, "sim report");
    let (correlation, keys) = one_read(&effects);
    let early = rig.complete(red, correlation, &serve(&BTreeMap::new(), keys));
    assert!(
        early.contains(&format!(
            "Your report of match {id} is still being written; ask again in a moment."
        )),
        "{early}"
    );

    // Staff page the tape: past its end, then from its start.
    let (said, effects) = rig.read(GOD, &format!("@arena/report {id} 16"));
    assert!(
        said.contains(&format!("Reading chunks 16 to 31 of match {id} …")),
        "{said}"
    );
    let (correlation, keys) = one_read(&effects);
    let past = rig.complete(GOD, correlation, &serve(&kv, keys));
    assert!(
        past.contains(&format!(
            "Match {id} has no tape at chunk 16 on this server."
        )),
        "{past}"
    );
    assert!(past.contains("seed "), "the header follows: {past}");
    let (_, effects) = rig.read(GOD, &format!("@arena/report #{id}"));
    let (correlation, keys) = one_read(&effects);
    assert_eq!(keys.len(), mush_core::MAX_STORE_READ_KEYS, "{keys:?}");
    let page = rig.complete(GOD, correlation, &serve(&kv, keys));
    for want in [
        "seed ",
        "this page: chunks 0 to ",
        "match_started",
        "aborted",
    ] {
        assert!(page.contains(want), "{want:?} missing from:\n{page}");
    }
    assert!(
        !page.contains("Next: "),
        "the tape ended inside this page: {page}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut Z.3 — six a side
// ---------------------------------------------------------------------------

/// **Twelve pilots claim six a side, start, and board all twelve pods** (16b
/// cut Z.3; 16c N7): the thirteenth is refused by the pods that exist, every
/// pod locks to its own slot's tank, the board lists slots 5 and 6 by their
/// callsigns, and the last seat fires `Running`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z3_twelve_pilots_claim_six_a_side_start_and_board() {
    let mut rig = Rig::boot(13);
    for n in 0..6 {
        let said = rig.say(n, "sim team red");
        assert!(!said.contains("full"), "{said}");
    }
    for n in 6..12 {
        let said = rig.say(n, "sim team blue");
        assert!(!said.contains("full"), "{said}");
    }
    let said = rig.say(12, "sim team red");
    assert!(
        said.contains("The red side is full: it takes 6 pilot(s)."),
        "{said}"
    );
    let centre = rig.centre();
    assert_eq!(centre.side_len(Team::Red), 6);
    assert_eq!(centre.side_len(Team::Blue), 6);
    let board = rig.say(0, "sim status");
    for needle in [
        "5 (RE): Pilot4",
        "6 (RF): Pilot5",
        "5 (BE): Pilot10",
        "6 (BF): Pilot11",
    ] {
        assert!(board.contains(needle), "{needle}: {board}");
    }

    let said = rig.say(0, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert_eq!(rig.state(), MatchState::Boarding);
    let mut pods: Vec<ObjId> = Vec::new();
    let mut tanks: Vec<ObjId> = Vec::new();
    for n in 0..12 {
        let pod = rig.pod(n).expect("every claimed slot locks its pod");
        let (team, index) = match_::pod_slot(rig.tw.world(), pod).expect("a centre pod");
        let tank = match_::tank_of(rig.tw.world(), pod).expect("every pod has its tank");
        assert_eq!(
            match_::tank_for_slot(rig.tw.world(), team, index),
            Some(tank)
        );
        assert!(!pods.contains(&pod) && !tanks.contains(&tank));
        pods.push(pod);
        tanks.push(tank);
    }
    for n in 0..11 {
        assert!(!rig.seat(n), "pilot {n} is not the last to sit");
    }
    assert!(rig.seat(11), "the twelfth seat fires the match");
    assert_eq!(rig.state(), MatchState::Running);
    rig.staff("@arena/abort");
}

/// **The reset walks all twelve tanks, and its report names them** (16b cut
/// Z.3): with every roster tank carrying a stale marker the reset writes nothing
/// and says one row per tank, which is the reset reading the roster's twelve
/// slots and not a count of four a side.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z3_the_reset_walks_twelve_tanks() {
    let mut rig = Rig::boot(0);
    let mut tanks: Vec<ObjId> = Vec::new();
    for team in Team::ALL {
        for index in 1..=match_::PODS_PER_SIDE {
            let tank = match_::tank_for_slot(rig.tw.world(), team, index).expect("a slot tank");
            mush_sim::deployed::mark(
                rig.tw.world_mut(),
                tank,
                mush_sim::DeployedMark::new(99, "tacsim.arena.proving_ground"),
            );
            tanks.push(tank);
        }
    }
    assert_eq!(tanks.len(), 12);
    let said = rig.staff("@arena/reset");
    assert!(
        said.contains("The simulator centre is idle, but not clean:"),
        "{said}"
    );
    for tank in &tanks {
        assert!(
            said.contains(&format!(
                "#{} is still deployed and was left alone; `@arena/abort` its match first.",
                tank.0
            )),
            "#{}: {said}",
            tank.0
        );
    }
    for tank in tanks {
        mush_sim::deployed::clear(rig.tw.world_mut(), tank);
    }
}

/// A rig booted at the arena's **version 2** with the OOC pack's **version 7**
/// (16b cut Z.3): twelve pods, eight tanks, the fill on.
fn arena_v2_rig(pilots: usize) -> Rig {
    Rig::boot_on(
        pilots,
        support::roster_booting(fill_config(), support::boot_packs_at_arena_v2(true)),
    )
}

/// **A world holding twelve pods and the version-2 arena stays four a side,
/// and takes version 3 from the shelf** (16b cut Z.3; 16c N7).
///
/// Until the arena is taken the lobby's cap follows what exists — four a side,
/// `SIM AI 5` refused — and never hands out a slot whose tank is not there.
/// Then staff plan and apply `tacsim.pack.proving_ground` from the
/// `@atlas/pack` shelf with the centre idle: four tanks made, the eight kept by
/// object id, all twelve on their authored cells, the roster naming twelve
/// slots, and the lobby six a side.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z3_a_version_two_arena_is_four_a_side_until_it_takes_version_three() {
    let mut rig = arena_v2_rig(1);
    let ready = match_::centre_readiness(rig.tw.world());
    assert!(ready.rooms);
    assert_eq!((ready.pods, ready.tanks), (12, 8));
    assert_eq!(match_::fieldable_per_side(rig.tw.world()), 4);
    assert_eq!(
        match_::slots_per_side(rig.tw.world(), &fill_config().match_),
        4,
        "the configured 12 is held down to the tanks that exist"
    );
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim ai 5");
    assert!(said.contains("AI fill is 1 to 4 a side."), "{said}");
    let said = rig.say(0, "sim ai 4");
    assert!(said.contains("AI fill set to 4 a side."), "{said}");
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Idle);

    let before: Vec<(Team, u8, ObjId)> = Team::ALL
        .into_iter()
        .flat_map(|team| (1..=4_u8).map(move |index| (team, index)))
        .map(|(team, index)| {
            let tank = match_::tank_for_slot(rig.tw.world(), team, index).expect("a v2 slot");
            (team, index, tank)
        })
        .collect();
    let (planned, applied) =
        support::take_from_shelf(&mut rig.tw, GOD, "tacsim.pack.proving_ground");
    assert!(
        planned.contains("Plan for tacsim.pack.proving_ground@3: 0 create, 1 update, 0 retire"),
        "{planned}"
    );
    assert!(
        applied.contains("Applied tacsim.pack.proving_ground@3"),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(
        rig.tw.world().refused_namespace_writes(),
        Vec::new(),
        "the hand apply wrote across a namespace it may not"
    );

    for (team, index, tank) in &before {
        assert_eq!(
            match_::tank_for_slot(rig.tw.world(), *team, *index),
            Some(*tank),
            "{} slot {index} kept its object id",
            team.as_str()
        );
    }
    let arena = mush_core::CanonicalId::new(match_::CENTRE_ARENA).expect("an id");
    let roster = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .and_then(|store| store.arena_roster(&arena).cloned())
        .expect("the roster is written");
    assert_eq!(roster.units.len(), 12, "{:?}", roster.units);
    let mut made: Vec<ObjId> = Vec::new();
    for team in Team::ALL {
        for index in 1..=match_::PODS_PER_SIDE {
            let slot = format!("{}_{index}", team.as_str());
            let tank = match_::tank_for_slot(rig.tw.world(), team, index)
                .unwrap_or_else(|| panic!("{slot} resolves"));
            assert!(!made.contains(&tank), "{slot} is its own tank");
            made.push(tank);
            let (_, pose) = mush_sim::pose_of(rig.tw.world(), tank).expect("placed");
            assert_eq!(
                (pose.x_cm, pose.y_cm, pose.heading_mdeg),
                support::authored_pose_of(&slot),
                "{slot} stands on its authored cell"
            );
        }
    }
    for (name, team, index) in [
        ("Red Tank 5", Team::Red, 5),
        ("Red Tank 6", Team::Red, 6),
        ("Blue Tank 5", Team::Blue, 5),
        ("Blue Tank 6", Team::Blue, 6),
    ] {
        let tank = match_::tank_for_slot(rig.tw.world(), team, index).expect("made");
        assert_eq!(
            rig.tw
                .world()
                .get(tank)
                .map(|object| object.name.to_string()),
            Some(name.to_owned())
        );
    }

    // And the lobby is six a side.
    let ready = match_::centre_readiness(rig.tw.world());
    assert!(ready.is_whole(), "{ready:?}");
    assert_eq!(
        match_::slots_per_side(rig.tw.world(), &fill_config().match_),
        6
    );
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim ai 6");
    assert!(said.contains("AI fill set to 6 a side."), "{said}");

    // A second plan is nothing to do: the latch is at the shipped version.
    let said = rig.staff("@atlas/pack plan tacsim.pack.proving_ground");
    assert!(said.contains(" Nothing to do."), "{said}");
}

/// **Eight pods hold the lobby at four a side whatever the tanks say** (16b cut
/// Z.3; 16c N7): the pod half of [`match_::fieldable_per_side`].
///
/// A world still at `ooc_area@6` has eight pods. With the version-3 arena it
/// has twelve tanks (a staffer who took the arena pack and not the OOC pack),
/// and a cap that read the tanks alone would hand out slot 5 — and then every
/// `SIM START` would refuse, because the pod lookup runs before the unit check.
/// So the fifth claim is refused, `SIM AI 5` is refused, and four a side starts
/// and boards on pods 1 to 4. And dev1's world before the packs are taken
/// (eight pods, eight tanks) reads the same four.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z3_eight_pods_field_four_a_side_even_beside_twelve_tanks() {
    let mut rig = Rig::boot_on(
        5,
        support::roster_booting(fill_config(), support::boot_packs_at_ooc_v6(false)),
    );
    let ready = match_::centre_readiness(rig.tw.world());
    assert!(ready.rooms);
    assert_eq!((ready.pods, ready.tanks), (8, 12), "{ready:?}");
    assert!(!ready.is_whole());
    assert_eq!(match_::fieldable_per_side(rig.tw.world()), 4);
    assert_eq!(
        match_::slots_per_side(rig.tw.world(), &fill_config().match_),
        4,
        "the configured 12 is held down to the pods that exist"
    );
    for n in 0..4 {
        let said = rig.say(n, "sim team red");
        assert!(!said.contains("full"), "{said}");
    }
    let said = rig.say(4, "sim team red");
    assert!(
        said.contains("The red side is full: it takes 4 pilot(s)."),
        "{said}"
    );
    let said = rig.say(0, "sim ai 5");
    assert!(said.contains("AI fill is 1 to 4 a side."), "{said}");
    let said = rig.say(0, "sim ai 4");
    assert!(said.contains("AI fill set to 4 a side."), "{said}");
    let said = rig.say(0, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert_eq!(rig.state(), MatchState::Boarding);
    for n in 0..4 {
        let pod = rig.pod(n).expect("every claimed slot locks its pod");
        let (team, index) = match_::pod_slot(rig.tw.world(), pod).expect("a centre pod");
        assert!(team == Team::Red && index <= 4, "{team:?} {index}");
        assert!(match_::tank_of(rig.tw.world(), pod).is_some());
    }
    rig.staff("@arena/abort");

    // Eight pods and the version-2 arena's eight tanks: the same four.
    let before = Rig::boot_on(
        0,
        support::roster_booting(fill_config(), support::boot_packs_at_ooc_v6(true)),
    );
    let ready = match_::centre_readiness(before.tw.world());
    assert_eq!((ready.pods, ready.tanks), (8, 8), "{ready:?}");
    assert_eq!(match_::fieldable_per_side(before.tw.world()), 4);
}

/// **The game's two packs are on the shelf of a production composition** (16b
/// cut Z.3): `@atlas/pack plan` answers a plan for each — a no-op on a fresh
/// world — and never `is not on the shelf`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn z3_the_arena_pack_is_on_the_shelf() {
    let mut rig = Rig::boot(0);
    for pack in ["tacsim.pack.proving_ground", "tacsim.proving_ground"] {
        let said = rig.staff(&format!("@atlas/pack plan {pack}"));
        assert!(!said.contains("is not on the shelf"), "{pack}: {said}");
        assert!(
            said.contains(&format!("Plan for {pack}@")),
            "{pack}: {said}"
        );
        assert!(said.contains(" Nothing to do."), "{pack}: {said}");
    }
    let said = rig.staff("@atlas/pack list");
    assert!(said.contains("tacsim.pack.proving_ground"), "{said}");
    assert!(
        !said.contains("tacsim.pack.proving_ground (not on the shelf)"),
        "{said}"
    );
}
