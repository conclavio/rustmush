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
use tacsim::arena::state::PartyId;
use tacsim::match_::{self, MatchState};

use crate::support;

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
    /// **16b cut AF.3**: the store as every write the rig has taken left it
    /// ([`Rig::pump_ops`]) — what [`Rig::reboot_mid_match`] boots from.
    image: StoreImage,
    /// **16b cut AF.3**: the arena clock of every `ArenaOut::Frames` [`Rig::pump`]
    /// has staged, as `(match, now_ms)`, in order.
    frames_seen: Vec<(u64, i64)>,
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
            image: StoreImage::new(),
            frames_seen: Vec::new(),
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
                        // 16b cut AF.3: the arena clock each pulse's frames carry.
                        if let tacsim::arena_task::ArenaOut::Frames {
                            match_id, now_ms, ..
                        } = &message
                        {
                            self.frames_seen.push((match_id.0, *now_ms));
                        }
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
    fn match_units(&self) -> Vec<(PartyId, u8)> {
        let w = self.tw.world();
        let Some(id) = match_::centre_of(w).and_then(|centre| centre.match_id) else {
            return Vec::new();
        };
        let registry = w.module_state::<tacsim::arena_bridge::ArenaRegistry>();
        let mut out = Vec::new();
        for team in PartyId::OOC {
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
        let red_pod = match_::pod_object(rig.tw.world(), PartyId::RED, 1).expect("red pod 1");
        let blue_pod = match_::pod_object(rig.tw.world(), PartyId::BLUE, 1).expect("blue pod 1");
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
    assert_eq!(centre.side_len(PartyId::RED), 3, "three accounts, three slots");
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
    assert_eq!(rig.centre().side_len(PartyId::RED), 6);
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
    let red_tank = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
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
        rig.centre().leads_side(PartyId::RED, account(0)),
        "red's lead is still its first claimant"
    );
    assert!(
        rig.centre().leads_side(PartyId::BLUE, account(1)),
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
        centre.leads_side(PartyId::RED, account(1)),
        "red's next claimant leads it"
    );
    assert!(
        !centre.leads_side(PartyId::RED, account(0)),
        "the dropped lead leads nothing"
    );
    assert!(
        centre.leads_side(PartyId::BLUE, account(2)),
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
        Some(tacsim::arena::EndReason::Conceded { winner: PartyId::BLUE })
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
    assert!(centre.leads_side(PartyId::RED, account(1)), "red's heir");
    assert!(centre.leads_side(PartyId::BLUE, account(3)), "blue's heir");
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
        !centre.leads.iter().any(|(side, _)| *side == PartyId::BLUE),
        "an emptied side has no lead"
    );
    assert!(centre.leads_side(PartyId::RED, account(0)));
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
    let red_tank = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
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
    let pod = match_::pod_object(rig.tw.world(), PartyId::BLUE, 2).expect("blue pod 2");
    let pilot = rig.pilot(0);
    rig.tw
        .world_mut()
        .move_to(pilot, pod)
        .expect("a pilot climbs into a pod");
    assert_eq!(
        match_::where_in_centre(rig.tw.world(), pilot),
        Some(match_::CentreRoom::Team(PartyId::BLUE))
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
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1).expect("blue 1's tank");
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
    for team in PartyId::OOC {
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
                    PartyId::RED => "Red",
                    PartyId::BLUE => "Blue",
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
    let pod = match_::pod_object(rig.tw.world(), PartyId::RED, 3).expect("red pod 3");
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

    let game = tacsim::GameEconomyConfig::default();
    let cfg = TacsimConfig::default();
    assert!(!cfg.enabled, "the documented default is off");
    // Reboots audit the foundry through the inventory owner, as production does.
    let (mut mods, inventory) = comp::domain_modules_with_economy(
        mush_economy::EconomyConfig::default(),
        mush_atlas::AtlasConfig::default(),
        &game,
    );
    mods.push(comp::npc_module(comp::npc_agent_adapter()));
    mods.push(comp::tacsim_module_wired(cfg, false, inventory));
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
        units.iter().filter(|(team, _)| *team == PartyId::BLUE).count(),
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

/// **The board's and the history's words for an elimination and a timeout**
/// (16b cut AP.2; review `F7`/`AP2-1`): the words `help simulator`'s end
/// paragraph quotes, pinned where they are printed.
///
/// This rig ends a match through `match_::finish` (`end_match`) and has no
/// arena to land a hit in, so it proves the WORDS and nothing of the disarmed
/// side's clause — that is the library's (`damage.rs`'s
/// `ap2_a_side_whose_guns_are_shot_away_is_out_while_the_other_carries_one`,
/// `weapons.rs`'s cross-tempo `ap2_a_disarming_hit_…`). An elimination has no
/// board arm of its own: `SIM STATUS` prints the outcome word, `— red`, and
/// `SIM HISTORY` `red won (blue eliminated)`. A timeout prints `— timeout` and
/// `timed out (the clock ran out)`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap2_an_elimination_end_prints_the_boards_and_the_historys_words() {
    let mut rig = fill_rig(1);
    rig.say(0, "sim team red");
    rig.say(0, "sim start");
    assert!(rig.seat(0), "the one pilot boards and the match runs");
    rig.end_match(tacsim::arena::EndReason::Elimination {
        winner: Some(PartyId::RED),
    });
    assert_eq!(rig.state(), MatchState::Ended);
    let said = rig.say(0, "sim status");
    assert!(
        said.lines()
            .any(|line| line.trim_end().ends_with("ended (match 1) — red")),
        "{said}"
    );
    let history = rig.say(0, "sim history");
    assert!(history.contains("red won (blue eliminated)"), "{history}");

    let mut rig = fill_rig(1);
    rig.say(0, "sim team red");
    rig.say(0, "sim start");
    assert!(rig.seat(0));
    rig.end_match(tacsim::arena::EndReason::Timeout);
    assert_eq!(rig.state(), MatchState::Ended);
    let said = rig.say(0, "sim status");
    assert!(
        said.lines()
            .any(|line| line.trim_end().ends_with("ended (match 1) — timeout")),
        "{said}"
    );
    let history = rig.say(0, "sim history");
    assert!(
        history.contains("timed out (the clock ran out)"),
        "{history}"
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
            .any(|(team, slot)| *team == PartyId::BLUE && *slot == 1),
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
fn tank_schema(rig: &Rig, team: PartyId, index: u8) -> String {
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
fn tank_pose(rig: &Rig, team: PartyId, index: u8) -> Option<(mush_sim::DomainId, mush_sim::Pose)> {
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
            "Fieldable units: biped_mk1, biped_mk2, drone_mk1, heavy_mk1, line_tank, \
             line_tank_eccm, line_tank_ecm, line_tank_laser, line_tank_thermal, scout_mk1, \
             target_mk1. Yours: line_tank."
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

/// **`SIM UNIT` never offers a hauler** (doc 18 cut IC2; doc 18 §4.2 schema
/// addition (1), D-18-29): the Dray is in the fielded catalog for the port code
/// and never in a lobby. The bare list is exactly the one
/// `q8_sim_unit_lists_selects_and_clears_a_choice` pins — unchanged by the Dray,
/// which is the proof — and naming it answers the unknown-type line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ic2_sim_unit_never_offers_a_hauler() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim unit");
    assert!(
        said.contains(
            "Fieldable units: biped_mk1, biped_mk2, drone_mk1, heavy_mk1, line_tank, \
             line_tank_eccm, line_tank_ecm, line_tank_laser, line_tank_thermal, scout_mk1, \
             target_mk1. Yours: line_tank."
        ),
        "{said}"
    );
    assert!(!said.contains("dray"), "{said}");
    let said = rig.say(0, "sim unit dray");
    assert!(
        said.contains("No such unit type: dray. SIM UNIT lists them."),
        "{said}"
    );
    assert_eq!(rig.centre().slots[0].unit_type, None, "and nothing moved");
}

/// **`SIM UNIT` never offers a terrain vehicle or a Dray-P** (doc 20 cut SM.3a;
/// D-20-6; reviewer F7): both are in the fielded catalog — the TV a `vehicle`,
/// the Dray-P a `hauler` — and neither is in a lobby. The bare list is
/// byte-identical to the one `ic2_sim_unit_never_offers_a_hauler` pins, which is
/// the proof that neither joined it, and naming either answers the
/// unknown-type line with the slot's choice unmoved.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sm3a_sim_unit_never_offers_a_vehicle() {
    let mut rig = Rig::boot(2);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim unit");
    assert!(
        said.contains(
            "Fieldable units: biped_mk1, biped_mk2, drone_mk1, heavy_mk1, line_tank, \
             line_tank_eccm, line_tank_ecm, line_tank_laser, line_tank_thermal, scout_mk1, \
             target_mk1. Yours: line_tank."
        ),
        "{said}"
    );
    assert!(!said.contains("tv_prospector"), "{said}");
    assert!(!said.contains("dray_p"), "{said}");
    for word in ["tv_prospector", "dray_p"] {
        let said = rig.say(0, &format!("sim unit {word}"));
        assert!(
            said.contains(&format!("No such unit type: {word}. SIM UNIT lists them.")),
            "{said}"
        );
        assert_eq!(rig.centre().slots[0].unit_type, None, "and nothing moved");
    }
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

    let tank = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
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
        tank_schema(&rig, PartyId::RED, 1),
        "tacsim.unit.line_tank",
        "and nothing was written to the tank"
    );
}

/// **A match fought in a biped leaves the pod a `line_tank` at spec and at
/// muster** (16b cut Q.8), and a second reset writes nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn q8_the_return_to_idle_puts_the_pods_own_tank_back() {
    let mut rig = Rig::boot(2);
    let authored = tank_pose(&rig, PartyId::RED, 1).expect("red 1's tank is placed");
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    rig.say(0, "sim start");

    // The refit happened at `SIM START`, before the pods locked.
    assert_eq!(tank_schema(&rig, PartyId::RED, 1), "tacsim.unit.biped_mk1");
    assert_eq!(
        tank_schema(&rig, PartyId::BLUE, 1),
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
        tank_schema(&rig, PartyId::RED, 1),
        "tacsim.unit.line_tank",
        "the pod's own tank is back"
    );
    let now = tank_pose(&rig, PartyId::RED, 1).expect("and it is still placed");
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
    let second = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 2).expect("red 2's tank");
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
        tank_schema(&rig, PartyId::RED, 1),
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
        tank_schema(&rig, PartyId::RED, 1),
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
fn wedge(rig: &mut Rig, team: PartyId, index: u8) -> ObjId {
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
    let second = wedge(&mut rig, PartyId::RED, 2);
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Selection);
    assert_eq!(tank_schema(&rig, PartyId::RED, 1), "tacsim.unit.biped_mk1");

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
        tank_schema(&rig, PartyId::RED, 1),
        "tacsim.unit.biped_mk1",
        "and clearing a choice writes nothing to the tank"
    );

    mush_sim::deployed::clear(rig.tw.world_mut(), second);
    let said = rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding, "{said}");
    assert_eq!(
        tank_schema(&rig, PartyId::RED, 1),
        "tacsim.unit.line_tank",
        "the start put the pod with no choice on it back to its own machine"
    );
    assert_eq!(tank_schema(&rig, PartyId::RED, 2), "tacsim.unit.line_tank");
    assert_eq!(tank_schema(&rig, PartyId::BLUE, 1), "tacsim.unit.line_tank");
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

    let second = wedge(&mut rig, PartyId::RED, 2);
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Selection);
    assert_eq!(tank_schema(&rig, PartyId::RED, 1), "tacsim.unit.biped_mk1");

    // Pilot 0 gives up. It is NOT the last slot, so the centre stays in
    // `Selection` and the whole-roster reset the last leave runs does not.
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Selection, "the lobby is still up");
    assert_eq!(
        tank_schema(&rig, PartyId::RED, 1),
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
        (PartyId::RED, 1, None),
        "the lowest free red slot, with no choice on it"
    );
    // Pilot 1 inherited the side's lead when pilot 0 left.
    let said = rig.say(1, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding, "{said}");
    assert_eq!(
        tank_schema(&rig, PartyId::RED, 1),
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
    assert_eq!(tank_schema(&rig, PartyId::RED, 1), "tacsim.unit.biped_mk1");

    // The chooser gives up before boarding. Their pod is now one of the
    // unclaimed ones the fill takes.
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Boarding, "the match is still on");
    assert_eq!(
        tank_schema(&rig, PartyId::RED, 1),
        "tacsim.unit.line_tank",
        "the abandoned pod is back at its authored type before the check-out"
    );

    assert!(rig.seat(1), "the last seat fires the check-out");
    assert_eq!(rig.state(), MatchState::Running);
    assert!(
        rig.match_units().contains(&(PartyId::RED, 1)),
        "the fill really took the vacated pod: {:?}",
        rig.match_units()
    );
    assert_eq!(
        tank_schema(&rig, PartyId::RED, 1),
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
    let tank = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
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
    fn schema_of(&self, team: PartyId, index: u8) -> String {
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
        rig.schema_of(PartyId::RED, 1),
        "tacsim.unit.biped_mk1",
        "the start fitted the chosen chassis"
    );
    assert!(!rig.seat(0), "red is in and blue is not");

    // The blue tank is out on loan to somebody else, so the check-out refuses.
    let blue = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1).expect("blue 1's tank");
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
    let red_pod = match_::pod_object(rig.tw.world(), PartyId::RED, 1).expect("red pod 1");
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
        rig.schema_of(PartyId::RED, 1),
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
        if let Some(tank) = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, index) {
            rig.tw
                .world_mut()
                .destroy(tank)
                .expect("a tank nothing protects");
        }
    }
    rig.tw.world_mut().sweep();
    assert_eq!(
        match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1),
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
    let red_pod = match_::pod_object(rig.tw.world(), PartyId::RED, 1).expect("red pod 1");
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
        rig.schema_of(PartyId::RED, 1),
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
        rig.schema_of(PartyId::RED, 1),
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
    assert_eq!(rig.schema_of(PartyId::RED, 1), "tacsim.unit.line_tank");
    rig.staff("@arena/reset");

    // (b) The case: chosen, fitted by `SIM START`, and called off from
    // `Boarding` by a participant.
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim unit biped_mk1");
    rig.say(0, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    assert_eq!(
        rig.schema_of(PartyId::RED, 1),
        "tacsim.unit.biped_mk1",
        "the start never fitted the chosen chassis"
    );
    let said = rig.say(0, "sim abort");
    assert!(said.contains("calls the match off"), "{said}");
    assert_eq!(rig.state(), MatchState::Aborted);
    assert_eq!(
        rig.schema_of(PartyId::RED, 1),
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
    assert_eq!(rig.schema_of(PartyId::RED, 1), "tacsim.unit.line_tank");
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
            team: PartyId::RED,
            index: n.saturating_add(1),
            account: mush_core::AccountId::mint("floor", i64::from(n)).expect("an account id"),
            pilot: ObjId(u32::from(n) + 1),
            pod: None,
            unit: None,
            left: false,
            dropped_at_ms: None,
            unit_type: None,
            released_by_resume: false,
            mind: None,
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
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
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
        tacsim::arena::EndReason::Conceded { winner: PartyId::BLUE },
        &[damaged],
    );
    assert_eq!(rig.state(), MatchState::Ended, "a concede is not an abort");
    assert_eq!(
        rig.centre().outcome,
        Some(tacsim::match_::MatchOutcome::Blue)
    );
    assert_eq!(
        rig.centre().outcome_reason,
        Some(tacsim::arena::EndReason::Conceded { winner: PartyId::BLUE })
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
    rig.end_match(tacsim::arena::EndReason::Conceded { winner: PartyId::RED });
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
    let match_id = rig.centre().match_id.expect("the running match");
    let _ = rig.pump_ops();
    let heard = rig.tick_heard(40, 0);
    assert!(
        heard.contains("Match 1 ends: every pilot has left it."),
        "{heard}"
    );
    assert_eq!(rig.centre().unattended_since_ms, None, "the stamp is spent");
    // **16b cut AF.2**: the stop's own action rewrites the live record parked
    // `stopping`, so a crash before the end is found by the boot as a stopping
    // match (voided) and never resumed as a running one.
    let writes = writes_of(rig.pump_ops());
    let stopping = set_of(&writes, &match_::live_key(match_id))
        .map(|document| match_::LiveRecord::from_document(&document))
        .expect("the unattended stop rewrote the live record")
        .expect("and it decodes");
    assert_eq!(stopping.record.state, MatchState::Stopping);
    assert_eq!(stopping.centre.match_id, Some(match_id));

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
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1).expect("blue 1's tank");
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
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1).expect("blue 1's tank");
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
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");

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
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
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
    assert!(rig.centre().leads_side(PartyId::RED, account(0)));

    // The second claimant leaves first: a non-lead leaving moves nothing.
    rig.release(1);
    assert!(
        rig.centre().leads_side(PartyId::RED, account(0)),
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
        centre.leads_side(PartyId::RED, account(2)),
        "red's next pilot still in a tank leads it: {:?}",
        centre.leads
    );
    assert!(
        !centre.leads_side(PartyId::RED, account(1)),
        "the earlier claimant who left is skipped"
    );
    assert!(
        !centre.leads_side(PartyId::RED, account(0)),
        "the lead who left leads nothing"
    );
    assert!(centre.leads_side(PartyId::BLUE, account(3)), "blue untouched");

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
        rig.centre().leads_side(PartyId::RED, account(0)),
        "a non-lead leaving moves nothing"
    );
    rig.release(0);
    let centre = rig.centre();
    assert!(
        !centre.leads.iter().any(|(side, _)| *side == PartyId::RED),
        "red has no lead once every red pilot has left: {:?}",
        centre.leads
    );
    assert!(centre.leads_side(PartyId::BLUE, account(2)));
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
        units.push(
            custody_unit(
                &catalog,
                cells,
                unit,
                team,
                pose,
                0,
                tacsim::arena::state::Controller::Pilot,
                0,
                &tacsim::match_::SealTerms::fixture(),
            )
            .expect("the fixture terms seal"),
        );
    }
    tacsim::arena::state::CustodyOut {
        definition: stock_definition(&smol_str::SmolStr::new_static(STAND_IN))
            .expect("the stand-in window"),
        units,
        pilots: Vec::new(),
        resolution: resolution_for(&catalog),
        seed: 2_396,
        mode: tacsim::arena::MatchMode::Skirmish,
        host: tacsim::arena::FieldHost::all_hostile(),
    }
    host: tacsim::arena::FieldHost::all_hostile(),
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
        &tacsim::config::MindsConfig::default(),
        &std::collections::BTreeMap::new(),
        tacsim::minds::MindPool::shared(2),
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
/// one nobody has used. Every refusal says its own sentence, after
/// `POSE refused. ` (16b cut AP.4), and no answer carries that prefix.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn u4_arena_pose_opens_a_slot_on_its_cell_for_the_next_match_and_no_later() {
    let mut rig = Rig::boot(2);
    // The control-lease mirror, read once before anybody boards.
    rig.tick_to(1);
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
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
        said.contains("POSE refused. There is no slot RZ on this centre."),
        "{said}"
    );
    let said = rig.staff("@arena/pose RA=300 97");
    assert!(
        said.contains("POSE refused. Cell (300, 97) is off the arena window."),
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
            "POSE refused. Cell ({bx}, {by}) is impassable, or has no way out."
        )),
        "{said}"
    );
    let said = rig.staff("@arena/pose RA=90 97 360");
    assert!(
        said.contains("POSE refused. Heading is 0 to 359."),
        "{said}"
    );
    let said = rig.staff("@arena/pose RA");
    assert!(
        said.contains(
            "POSE refused. Usage: @arena/pose <callsign>=<x> <y>[ <heading>] | <callsign>=off | (bare) list"
        ),
        "{said}"
    );

    // Set, case-folded; a second slot on the same cell is refused by name.
    let said = rig.staff("@arena/pose ra=90 97 90");
    assert!(
        said.contains("RA opens at cell (90, 97) heading 90 for the next match.")
            && !said.contains("POSE refused."),
        "{said}"
    );
    let said = rig.staff("@arena/pose RB=90 97");
    assert!(
        said.contains("POSE refused. Cell (90, 97) is already RA's opening cell."),
        "{said}"
    );
    let said = rig.staff("@arena/pose RB=107 97");
    assert!(
        said.contains("RB opens at cell (107, 97) heading 90 for the next match."),
        "red's own facing when no heading is given: {said}"
    );
    let said = rig.staff("@arena/pose RB=off");
    assert!(
        said.contains("RB opens at its authored cell again.") && !said.contains("POSE refused."),
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
            "POSE refused. The simulator centre is running; set opening cells before the match starts."
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

/// **A scout car posed on rubble opens in its zone, and the match starts** (16b
/// cut AE.1; 16c `B-58` and `T-14`).
///
/// `@arena/pose`'s world-side precheck knows no unit's row, so it asks the
/// tracked one (`ArenaTerrain::has_way_out`), and rubble is a tank's ground: the
/// override is set. The row is asked where the unit is known — `match_::start`
/// reads the fielded unit's own row (`CostRow::read`) and hands it to
/// `opening_pose`, which finds the cell no ground for a car and walks red's
/// entry zone instead, exactly as it does for a cell somebody else took. Asked
/// as a tank, the pose would have been kept and `Arena::new` would have refused
/// the car `CellImpassable`: no match at all.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ae1_a_scout_car_posed_on_rubble_opens_in_its_zone_and_the_match_starts() {
    use tacsim::arena::CostRow;

    let mut rig = Rig::boot(2);
    rig.tick_to(1);
    let definition = match_::definition_for(
        rig.tw.world(),
        &smol_str::SmolStr::new_static(match_::CENTRE_ARENA),
    )
    .expect("the centre's ground builds")
    .expect("the centre's arena");
    // The shipped rubble patch, which the world-side precheck admits and a car
    // cannot stand on.
    let rubble = (106_u16, 66_u16);
    assert_eq!(
        definition
            .terrain
            .get(rubble.0, rubble.1)
            .map(|cell| cell.class),
        Some(mush_atlas::TerrainClass::Rubble)
    );
    assert!(definition.terrain.has_way_out(rubble.0, rubble.1));
    assert!(
        !definition
            .terrain
            .has_way_out_for(CostRow::Wheeled, rubble.0, rubble.1)
    );
    let said = rig.staff("@arena/pose RA=106 66");
    assert!(
        said.contains("RA opens at cell (106, 66) heading 90 for the next match."),
        "{said}"
    );

    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim unit scout_mk1");
    assert!(said.contains("Slot red 1 will field scout_mk1."), "{said}");
    let said = rig.say(0, "sim start");
    assert!(
        said.contains("Match 1 is boarding."),
        "the start was refused: {said}"
    );
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
    assert_eq!(
        rig.state(),
        MatchState::Running,
        "the arena refused the check-out"
    );
    assert_eq!(
        tank_schema(&rig, PartyId::RED, 1),
        "tacsim.unit.scout_mk1",
        "red 1 fields the car"
    );

    let ((x, y), heading) = opening_of_red_one(&mut rig).await;
    assert_ne!((x, y), (106, 66), "the car opened on rubble");
    let zone = definition
        .entry_zones
        .iter()
        .find(|zone| zone.team == PartyId::RED)
        .expect("red's entry zone");
    let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else {
        panic!("the car opened off the window: ({x}, {y})");
    };
    assert!(
        (zone.x0..=zone.x1).contains(&x) && (zone.y0..=zone.y1).contains(&y),
        "the car opened outside red's zone: ({x}, {y})"
    );
    assert!(definition.terrain.has_way_out_for(CostRow::Wheeled, x, y));
    assert_eq!(heading, 90_000, "red's own facing");
    rig.end_match(tacsim::arena::EndReason::Abort);
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
    let _ = rig.pump_ops();

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
        .pump_ops()
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
        replay: false,
        mode: None,
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
    let _ = rig.pump_ops();

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
        .pump_ops()
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
    for op in rig.pump_ops() {
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
    assert_eq!(centre.side_len(PartyId::RED), 6);
    assert_eq!(centre.side_len(PartyId::BLUE), 6);
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
    for team in PartyId::OOC {
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

    let before: Vec<(PartyId, u8, ObjId)> = PartyId::OOC
        .into_iter()
        .flat_map(|team| (1..=4_u8).map(move |index| (team, index)))
        .map(|(team, index)| {
            let tank = match_::tank_for_slot(rig.tw.world(), team, index).expect("a v2 slot");
            (team, index, tank)
        })
        .collect();
    let (planned, applied) =
        support::take_from_shelf(&mut rig.tw, GOD, "tacsim.pack.proving_ground");
    // 16b cut O.1: the shipped arena is version 4, so the version 2 world
    // takes version 4 in one step (the four tanks are Z.3's, the Hold
    // declaration O.1's) — and version 5 since 16b cut AE.1 (The Bluff's key
    // spot).
    assert!(
        planned.contains("Plan for tacsim.pack.proving_ground@5: 0 create, 1 update, 0 retire"),
        "{planned}"
    );
    assert!(
        applied.contains("Applied tacsim.pack.proving_ground@5"),
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
    for team in PartyId::OOC {
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
        ("Red Tank 5", PartyId::RED, 5),
        ("Red Tank 6", PartyId::RED, 6),
        ("Blue Tank 5", PartyId::BLUE, 5),
        ("Blue Tank 6", PartyId::BLUE, 6),
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
        assert!(team == PartyId::RED && index <= 4, "{team:?} {index}");
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

// ---------------------------------------------------------------------------
// 16b cut O.1 — a standing world takes the Hold declaration
// ---------------------------------------------------------------------------

/// **A world booted at the arena's version 3 reads Hold at once and takes the
/// shipped version from the shelf** (16b cut O.1; `deploy/README.md`'s shelf
/// paragraph). The arena catalog is compiled in and rebuilt at every boot, so
/// `SIM MODE` lists `hold` and accepts it before the pack is taken; the shelf
/// then plans the arena entry as one update, applies it with the centre idle,
/// adopts the twelve standing tanks by roster and remakes none, and moves the
/// latch to the shipped version — 4 at O.1, **5 since 16b cut AE.1**, which is
/// one step from 3 as it was from 2.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o1_a_version_three_world_reads_hold_at_once_and_takes_version_five() {
    let mut rig = Rig::boot_on(
        1,
        support::roster_booting(fill_config(), support::boot_packs_at_arena_v3(true)),
    );
    let latch = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<mush_content::ContentState>()
            .expect("content")
            .latches
            .applied(
                &mush_core::CanonicalId::new(tacsim::definitions::content::PROVING_GROUND_PACK)
                    .expect("an id"),
            )
            .map(mush_content::Applied::version)
    };
    assert_eq!(latch(&rig), Some(3), "the world stands at version 3");
    let said = rig.say(0, "sim mode");
    assert!(
        said.contains("Mode: skirmish. The Proving Ground supports: skirmish, hold."),
        "{said}"
    );
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim mode hold");
    assert!(
        said.contains(
            "Mode set to hold: keep The Crossroads (B) uncontested for 120 seconds to win; \
             elimination still ends it."
        ),
        "{said}"
    );
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Idle);

    // A slot, its tank, and the tank's `(x_cm, y_cm, heading_mdeg)`.
    type Standing = (PartyId, u8, ObjId, (i64, i64, u32));
    let before: Vec<Standing> = PartyId::OOC
        .into_iter()
        .flat_map(|team| (1..=match_::PODS_PER_SIDE).map(move |index| (team, index)))
        .map(|(team, index)| {
            let tank = match_::tank_for_slot(rig.tw.world(), team, index).expect("a v3 slot");
            let (_, pose) = mush_sim::pose_of(rig.tw.world(), tank).expect("placed");
            (team, index, tank, (pose.x_cm, pose.y_cm, pose.heading_mdeg))
        })
        .collect();
    assert_eq!(before.len(), 12, "version 3 made twelve tanks");
    let (planned, applied) =
        support::take_from_shelf(&mut rig.tw, GOD, "tacsim.pack.proving_ground");
    assert!(
        planned.contains("Plan for tacsim.pack.proving_ground@5: 0 create, 1 update, 0 retire"),
        "{planned}"
    );
    assert!(
        applied.contains("Applied tacsim.pack.proving_ground@5"),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(latch(&rig), Some(5));
    for (team, index, tank, cell) in &before {
        assert_eq!(
            match_::tank_for_slot(rig.tw.world(), *team, *index),
            Some(*tank),
            "{} slot {index} kept its object",
            team.as_str()
        );
        let (_, pose) = mush_sim::pose_of(rig.tw.world(), *tank).expect("placed");
        assert_eq!(
            (pose.x_cm, pose.y_cm, pose.heading_mdeg),
            *cell,
            "{} slot {index} stands where it stood",
            team.as_str()
        );
    }
    // And still Hold on the latched version.
    let said = rig.say(0, "sim mode");
    assert!(said.contains("supports: skirmish, hold."), "{said}");
}

// ---------------------------------------------------------------------------
// 16b cut O.3 — the range reads as a range in the ring
// ---------------------------------------------------------------------------

/// **16b cut O.3 (16c N13; plan items 19, 39)**: a range's result reads back as
/// `mode: Some(Range)` and its ring line carries ` [range]` after the outcome
/// words — through O.1's tag, with no code of its own — so a range is never read
/// as a fight in `SIM HISTORY`.
#[test]
fn o3_a_range_result_reads_as_a_range_in_the_ring() {
    let document = serde_json::json!({
        "record": "tacsim.match",
        "record_version": 1,
        "match": 5,
        "arena": "tacsim.arena.proving_ground",
        "outcome": "abort",
        "reason": "abort",
        "mode": "range",
        "teams": [
            { "team": "red", "units": [101], "pilots": [{ "pilot": 7, "unit": 101, "callsign": "RA" }] },
            { "team": "blue", "units": [201], "pilots": [] },
        ],
        "units": [
            { "unit": 101, "callsign": "RA", "outcome": "intact", "shots": 2, "hits": 1 },
            { "unit": 201, "callsign": "BA", "outcome": "intact", "shots": 0, "hits": 0 },
        ],
        "ledger": { "log": "match.5", "digest": "ab", "chunks": 1 },
        "seed": 0,
        "started_ms": 1_790_258_000_000_i64,
        "ended_ms": 1_790_258_520_000_i64,
        "arena_ms": 30_000,
        "reports": [101],
        "replay": true,
    });
    let row = match_::HistoryRow::from_document(&document).expect("a result document");
    assert_eq!(row.mode, Some(tacsim::arena::MatchMode::Range));
    assert_eq!(row.mode_tag(), " [range]");
    let lines = match_::history_lines(std::iter::once(&row), ObjId(7));
    assert_eq!(
        lines,
        vec![
            "The centre's last match:".to_owned(),
            "  #5  aborted [range]  30 s  1 v 1  2026-09-24 14:02 UTC  (you flew RA)".to_owned(),
        ]
    );
}

// ---------------------------------------------------------------------------
// 16b cut O.3 — the range's boarding gates (plan item 13)
// ---------------------------------------------------------------------------

/// A fill-off centre with its two claimants both on red and `SIM RANGE 1` set
/// and started: boarding, with the one target on blue's pod 1.
fn o3_range_boarding(rig: &mut Rig) {
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    let said = rig.say(0, "sim range 1");
    assert!(
        said.contains("Range set: 1 target stands on the blue side;"),
        "{said}"
    );
    let said = rig.say(0, "sim start");
    assert!(
        said.contains("boarding"),
        "a range starts with the fill off: {said}"
    );
    assert_eq!(rig.state(), MatchState::Boarding);
}

/// **A shooter leaving or getting out during boarding is not an empty side on a
/// range** (16b cut O.3; plan item 13), with the fill OFF — the shipped default
/// and the only configuration where `settle_boarding`'s range arm is not the
/// fill's own.
///
/// Before O.3 the fill-off branch asked every side whether it had a slot, and
/// under a range blue never does: its pods are the targets'. So an `EXIT`
/// (`slot_vacated`) or a `SIM LEAVE` during boarding would call the match off
/// with `the blue side has nobody left to fly it` — the N13 bug in a new place.
/// Here the one who got out keeps boarding open, the one who left lets the
/// boarded one's match run, and nothing is ever aborted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o3_a_range_survives_a_leave_and_an_exit_during_boarding_with_the_fill_off() {
    let mut rig = Rig::boot(2);
    o3_range_boarding(&mut rig);
    assert!(!rig.seat(0), "one of two seated is not the trigger");

    // `EXIT` during boarding: the slot is unseated, the centre is not empty.
    rig.vacate(0);
    assert_eq!(
        rig.state(),
        MatchState::Boarding,
        "an EXIT on a range called the match off"
    );

    // Pilot0 boards again; Pilot1 gives up the slot, which leaves everybody who
    // is left seated — so the match runs, with red alone against its target.
    assert!(!rig.seat(0), "Pilot1 is still unseated");
    let heard = rig.say_overheard(1, "sim leave", 0);
    assert!(
        !heard.contains("aborted") && !heard.contains("nobody left to fly it"),
        "{heard}"
    );
    assert_eq!(rig.state(), MatchState::Running, "{heard}");
    let units = rig.match_units();
    assert_eq!(
        units,
        vec![(PartyId::RED, 1), (PartyId::BLUE, 1)],
        "the boarded shooter against the one target"
    );
}

/// **The boarding deadline on a range drops the shooter who never boarded and
/// runs with the one who did — and aborts only a centre that boarded nobody**
/// (16b cut O.3; plan item 13), with the fill OFF.
///
/// Before O.3 the deadline's fill-off branch found blue — the targets' side,
/// which never holds a slot — empty and aborted with `the blue side boarded
/// nobody`, whoever had boarded on red.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o3_the_boarding_deadline_on_a_range_asks_the_shooters_alone() {
    let mut rig = Rig::boot(2);
    o3_range_boarding(&mut rig);
    assert!(!rig.seat(0));
    let heard = rig.tick_heard(10, 0);
    assert!(
        heard.contains("Pilot1 never boarded; red slot 2 is dropped."),
        "{heard}"
    );
    assert!(!heard.contains("aborted"), "{heard}");
    assert_eq!(rig.state(), MatchState::Running, "it ran with who was in");
    assert_eq!(rig.centre().slots.len(), 1);
    assert_eq!(rig.match_units(), vec![(PartyId::RED, 1), (PartyId::BLUE, 1)]);

    // Nobody boards at all: that is an abort, in the centre's words and not in
    // a side's.
    let mut rig = Rig::boot(2);
    o3_range_boarding(&mut rig);
    let match_id = rig.centre().match_id.expect("a boarding match id");
    let heard = rig.tick_heard(10, 0);
    assert!(
        heard.contains(&format!("Match {match_id} is aborted: nobody boarded.")),
        "{heard}"
    );
    assert!(!heard.contains("side boarded nobody"), "{heard}");
    assert_eq!(rig.state(), MatchState::Aborted);
}

/// **`unattended` ends a range as it ends any match** (16b cut O.3; plan item
/// 13's "unattended applies as today"), with the fill OFF: the one shooter gives
/// the seat up, the clock starts though the target is still in its pod — a
/// target is nobody's seat — and at the interval the match is stopped, and the
/// ring reads the `timeout` as a range's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn o3_a_range_every_shooter_left_ends_itself_after_the_interval() {
    let mut cfg = config();
    cfg.match_.unattended_ms = 10_000;
    let mut rig = Rig::boot_on(1, roster_with(cfg));
    rig.say(0, "sim team red");
    rig.say(0, "sim range 1");
    rig.say(0, "sim start");
    assert!(rig.seat(0), "the one shooter boards and the range runs");
    assert_eq!(rig.state(), MatchState::Running);
    rig.tick_to(20);
    assert_eq!(
        rig.centre().unattended_since_ms,
        None,
        "the shooter is aboard"
    );

    rig.release(0);
    assert_eq!(rig.centre().unattended_since_ms, Some(20_000));
    rig.tick_to(29);
    assert_eq!(rig.state(), MatchState::Running, "not before the interval");
    let heard = rig.tick_heard(30, 0);
    assert!(
        heard.contains("Match 1 ends: every pilot has left it."),
        "{heard}"
    );
    rig.end_match(tacsim::arena::EndReason::Unattended);
    assert_eq!(rig.state(), MatchState::Ended);
    assert_eq!(
        rig.centre().outcome,
        Some(tacsim::match_::MatchOutcome::Timeout)
    );
    let history = rig.say(0, "sim history");
    assert!(
        history.contains("#1  timed out (every pilot had left) [range]"),
        "{history}"
    );
}

// ---------------------------------------------------------------------------
// 16b cut AE.1 — a standing world takes The Bluff
// ---------------------------------------------------------------------------

/// **A world at the atlas's version 1 and the arena's version 4 takes The Bluff
/// from the shelf** (16b cut AE.1; `deploy/README.md`'s shelf paragraph, whose
/// quoted plan line this pins).
///
/// The arena catalog is compiled in and rebuilt at every boot, so before either
/// pack is taken `SIM UNIT` already lists the ten fieldable types and the
/// definition a start would build already carries five key spots, the fifth
/// lettered `F` — `VIEW`'s legend and its off-window line, rendered from that
/// definition by the cockpit's own functions, both name `F The Bluff` — over
/// flat ground, because the ground is the atlas pack's.
/// Then, with the centre idle, the atlas plan for `tacsim.proving_ground` is the
/// shelf's version — 3 since doc 21 cut G.7b, the showcase relief, so the plan
/// is the four heightfield quadrants and nothing else (at AE.1 it was version
/// 2's one, `field.q1_0`, the quadrant the bluff stands in) — and its apply
/// replaces them in place; the arena plan is one
/// update, and its apply moves the latch to 5 and adopts the twelve standing
/// tanks by roster, remaking none (each slot's object is the one it had, and
/// the world still carries twelve units). The definition the next start builds stands
/// the top of The Bluff at 500 cm.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ae1_a_version_four_world_takes_the_bluff_from_the_shelf() {
    let mut rig = Rig::boot_on(
        1,
        support::roster_booting(fill_config(), support::boot_packs_at_arena_v4(true)),
    );
    let latch = |rig: &Rig, pack: &str| {
        rig.tw
            .world()
            .module_state::<mush_content::ContentState>()
            .expect("content")
            .latches
            .applied(&mush_core::CanonicalId::new(pack).expect("an id"))
            .map(mush_content::Applied::version)
    };
    let arena_pack = tacsim::definitions::content::PROVING_GROUND_PACK;
    let atlas_pack = tacsim::definitions::content::PROVING_GROUND_ATLAS_PACK;
    assert_eq!(
        latch(&rig, arena_pack),
        Some(4),
        "the arena stands at version 4"
    );
    assert_eq!(
        latch(&rig, atlas_pack),
        Some(1),
        "and the ground at version 1"
    );
    let next_ground = |rig: &Rig| {
        match_::definition_for(
            rig.tw.world(),
            &smol_str::SmolStr::new(match_::PROVING_GROUND),
        )
        .expect("the proving ground's ground builds")
        .expect("the proving ground is shipped")
    };

    // Compiled in: the eleven types (16b cut AE.3's drone among them), and the
    // fifth spot over flat ground.
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim unit");
    assert!(
        said.contains(
            "Fieldable units: biped_mk1, biped_mk2, drone_mk1, heavy_mk1, line_tank, \
             line_tank_eccm, line_tank_ecm, line_tank_laser, line_tank_thermal, scout_mk1, \
             target_mk1."
        ),
        "{said}"
    );
    rig.say(0, "sim leave");
    assert_eq!(rig.state(), MatchState::Idle);
    let before = next_ground(&rig);
    assert_eq!(before.key_spots.len(), 5, "the catalog's five spots");
    let fifth = before.key_spots.get(4).expect("the fifth spot");
    assert_eq!(
        (fifth.name.as_str(), fifth.x, fifth.y),
        ("The Bluff", 144, 111)
    );
    assert_eq!(match_::cockpit::spot_letter(4), Some('F'));
    // And what `VIEW` draws of it, rendered from that very definition by the
    // cockpit's own two functions: a window over the whole ground puts `F` in
    // the legend as The Bluff, and the red muster's window (x 0..38, which
    // draws none of the five) names it last on its off-window line — 138 cells
    // east and 14 north of (6, 97), 1390 m at 084°.
    let (_, legend, drawn) = match_::cockpit::spot_markers(&before.key_spots, (0, 0), (256, 256));
    assert!(
        legend.contains(&serde_json::json!(["F", "The Bluff"])),
        "the legend does not letter the fifth spot: {legend:?}"
    );
    assert_eq!(
        drawn,
        vec![0, 1, 2, 3, 4],
        "every spot is on the whole ground"
    );
    let mut muster = mush_sim::data::Pose::at(
        6 * 1_000 + 500,
        97 * 1_000 + 500,
        0,
        mush_sim::data::passive_rules_value(serde_json::json!({})).expect("an empty posture blob"),
    );
    muster.heading_mdeg = 90_000;
    let off_window = match_::cockpit::off_window_spots_line(
        &before.key_spots,
        &[],
        &muster,
        before.terrain.cell_cm(),
    )
    .expect("the red muster's window draws no spot");
    assert!(
        off_window.ends_with("; F The Bluff, 1390 m at 084°."),
        "{off_window}"
    );
    assert_ne!(
        before.terrain.get(144, 111).map(|cell| cell.elev_cm),
        Some(500),
        "the ground under F is still version 1's"
    );

    // A slot, its tank, and the tank's `(x_cm, y_cm, heading_mdeg)`.
    type Standing = (PartyId, u8, ObjId, (i64, i64, u32));
    let standing: Vec<Standing> = PartyId::OOC
        .into_iter()
        .flat_map(|team| (1..=match_::PODS_PER_SIDE).map(move |index| (team, index)))
        .map(|(team, index)| {
            let tank = match_::tank_for_slot(rig.tw.world(), team, index).expect("a v4 slot");
            let (_, pose) = mush_sim::pose_of(rig.tw.world(), tank).expect("placed");
            (team, index, tank, (pose.x_cm, pose.y_cm, pose.heading_mdeg))
        })
        .collect();
    assert_eq!(standing.len(), 12, "version 4 made twelve tanks");
    // Every object that carries a tactical unit: on this world, the twelve
    // stock tanks and nothing else, so "none remade" is a count as well as an
    // object id a slot.
    let units = |rig: &Rig| {
        rig.tw
            .world()
            .iter()
            .filter(|object| !object.flags.contains(mush_core::Flags::GOING))
            .filter(|object| mush_tactics::module::unit_of(rig.tw.world(), object.id).is_some())
            .count()
    };
    assert_eq!(units(&rig), 12, "twelve tanks and no other unit");

    // The ground first: the shelf's version 3 (doc 21 cut G.7b), the four
    // heightfield quadrants and nothing else.
    let (planned, applied) = support::take_from_shelf(&mut rig.tw, GOD, atlas_pack);
    assert!(
        planned.contains(G7B_ATLAS_PLAN_LINE),
        "the atlas plan is the four quadrants: {planned}"
    );
    for quadrant in ["field.q0_0", "field.q0_1", "field.q1_0", "field.q1_1"] {
        assert!(
            planned.contains(&format!(" update   {quadrant} (heightfield)")),
            "{quadrant}: {planned}"
        );
    }
    assert!(
        applied.contains(&format!("Applied {atlas_pack}@3")),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(latch(&rig, atlas_pack), Some(3));

    // Then the arena: one update, the latch to 5, the twelve adopted.
    let (planned, applied) = support::take_from_shelf(&mut rig.tw, GOD, arena_pack);
    assert!(
        planned.contains("Plan for tacsim.pack.proving_ground@5: 0 create, 1 update, 0 retire"),
        "{planned}"
    );
    assert!(
        applied.contains("Applied tacsim.pack.proving_ground@5"),
        "{applied}"
    );
    assert!(!applied.contains("STOPPED"), "{applied}");
    assert_eq!(latch(&rig, arena_pack), Some(5));
    assert_eq!(
        rig.tw.world().refused_namespace_writes(),
        Vec::new(),
        "the two applies wrote across a namespace they may not"
    );
    assert_eq!(units(&rig), 12, "the arena apply made no thirteenth unit");
    for (team, index, tank, cell) in &standing {
        assert_eq!(
            match_::tank_for_slot(rig.tw.world(), *team, *index),
            Some(*tank),
            "{} slot {index} kept its object",
            team.as_str()
        );
        let (_, pose) = mush_sim::pose_of(rig.tw.world(), *tank).expect("placed");
        assert_eq!(
            (pose.x_cm, pose.y_cm, pose.heading_mdeg),
            *cell,
            "{} slot {index} stands where it stood",
            team.as_str()
        );
    }

    // The next start fights on The Bluff.
    let after = next_ground(&rig);
    assert_eq!(after.key_spots.len(), 5);
    assert_eq!(
        after.terrain.get(144, 111).map(|cell| cell.elev_cm),
        Some(500)
    );
    assert_eq!(
        after.terrain.get(144, 119).map(|cell| cell.elev_cm),
        Some(270)
    );
}

/// The atlas plan line `deploy/README.md` quotes (doc 21 cut G.7b; at 16b cut
/// AE.1 it was version 2's `0 create, 1 update, 0 retire, 5 unchanged`).
const G7B_ATLAS_PLAN_LINE: &str =
    "Plan for tacsim.proving_ground@3: 0 create, 4 update, 0 retire, 2 unchanged";

// ---------------------------------------------------------------------------
// 16b cut AE.3 — a side needs a gun
// ---------------------------------------------------------------------------

/// The `NoGun` sentence for red, as `run` announces a `ready` refusal.
const AE3_RED_NO_GUN: &str = "Match 1 could not start: The red side fields no gun; a side needs \
                              a machine that carries one. The pods are free again.";

/// **A boarding drop that leaves a side a lone drone calls the match off**
/// (16b cut AE.3; plan §6 AE-X3).
///
/// `SIM START` passes its courtesy — red claims a drone and a tank, and the
/// tank carries a gun — but red's tank pilot never boards, the deadline drops
/// that slot, and the side `ready()` builds is the drone alone. `ready()` is
/// where the sides are final, so that is where `NoGun` binds, and `run` calls
/// the lobby off through the one funnel with the refusal in its sentence; the
/// pods and the slots are free again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ae3_a_boarding_drop_that_leaves_a_lone_drone_calls_the_match_off() {
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim unit drone_mk1");
    assert!(said.contains("Slot red 1 will field drone_mk1."), "{said}");
    rig.say(1, "sim team red");
    rig.say(2, "sim team blue");
    let said = rig.say(0, "sim start");
    assert!(
        said.contains("boarding"),
        "red fields a tank beside its drone, so the courtesy passes: {said}"
    );
    assert_eq!(
        rig.schema_of(PartyId::RED, 1),
        "tacsim.unit.drone_mk1",
        "the refit put the drone in red pod 1"
    );
    // Red's drone and blue's tank board; red's tank pilot never does.
    assert!(!rig.seat(0));
    assert!(!rig.seat(2));
    let heard = rig.tick_heard(10, 0);
    assert!(heard.contains(AE3_RED_NO_GUN), "{heard}");
    assert_eq!(rig.state(), MatchState::Aborted, "the lobby is called off");
    assert!(rig.centre().slots.is_empty(), "the pods are free again");
    let red_pod = match_::pod_object(rig.tw.world(), PartyId::RED, 1).expect("red pod 1");
    assert_eq!(match_::pilot_of(rig.tw.world(), red_pod), None);
    assert!(
        !tacsim::match_::cockpit::in_a_pod(rig.tw.world(), rig.pilot(0)),
        "the drone's pilot was left shut in the pod"
    );
    let drone = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    assert!(
        mush_sim::deployed::mark_of(rig.tw.world(), drone).is_none(),
        "nothing was checked out"
    );
}

/// **With the fill on, a lone drone pilot starts when the fill brings a gun**
/// (16b cut AE.3).
///
/// `SIM START`'s courtesy walks the pods the fill will add
/// (`lobby::fill_pods`, the walk `ready()` makes). At the fill's own size — the
/// larger side, one — the fill adds nothing to red's side beside its one
/// claimed drone, so the courtesy refuses at once with the `NoGun` sentence and
/// nothing is written. Asked for two a side (`SIM AI 2`), the fill brings red a
/// tank from its lowest unclaimed pod, the start is accepted, and the match
/// runs with the fill's tank beside the drone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ae3_with_the_fill_on_a_lone_drone_starts_beside_the_fills_tank() {
    let mut rig = fill_rig(1);
    rig.say(0, "sim team red");
    rig.say(0, "sim unit drone_mk1");
    let said = rig.say(0, "sim start");
    assert!(
        said.contains("The red side fields no gun; a side needs a machine that carries one."),
        "a fill of one a side brings red nothing beside its drone: {said}"
    );
    assert_eq!(rig.state(), MatchState::Selection, "nothing was written");
    assert!(rig.centre().match_id.is_none());
    // Before the first write means before the refit too: red pod 1's tank is
    // still the pod's own, not the drone the slot chose.
    assert_eq!(
        rig.schema_of(PartyId::RED, 1),
        "tacsim.unit.line_tank",
        "the refused start refitted red pod 1"
    );

    let said = rig.say(0, "sim ai 2");
    assert!(said.contains('2'), "{said}");
    let said = rig.say(0, "sim start");
    assert!(said.contains("boarding"), "the fill brings a gun: {said}");
    assert!(rig.seat(0), "the one pilot's seat starts it");
    assert_eq!(rig.state(), MatchState::Running);
    assert_eq!(
        rig.match_units(),
        vec![
            (PartyId::RED, 1),
            (PartyId::RED, 2),
            (PartyId::BLUE, 1),
            (PartyId::BLUE, 2)
        ],
        "the drone, the fill's tank beside it, and two machines a side"
    );
    assert_eq!(rig.schema_of(PartyId::RED, 1), "tacsim.unit.drone_mk1");
    assert_eq!(rig.schema_of(PartyId::RED, 2), "tacsim.unit.line_tank");
}

/// A centre whose `[tacsim.arena].max_units_by_class` is `caps` (16b cut
/// AE.3), the fill off as [`config`] ships it.
fn ae3_capped_rig(pilots: usize, caps: &[(tacsim::definitions::UnitClass, u32)]) -> Rig {
    let mut cfg = config();
    cfg.arena.max_units_by_class = caps.iter().copied().collect();
    Rig::boot_on(pilots, roster_with(cfg))
}

/// **The lobby's cap is per side, counts a slot by its effective type, and
/// reads the operator's number** (16b cut AE.3; doc 16 §2 W2; plan Q-AE4).
///
/// 1. At the shipped default a red pilot and a blue pilot BOTH field a drone —
///    the count is the caller's own side's — and a second red pilot is the one
///    refused, with the cap's sentence, his slot keeping what it had.
/// 2. A slot that chose nothing counts as its pod's own tank
///    (`authored_unit_type`): under `armour = 1` a red pilot who never typed
///    `SIM UNIT` already fields red's one armoured machine, so his teammate's
///    `SIM UNIT line_tank` is refused and a walker is not.
/// 3. `drone = 0` forbids the class outright, in the cap-0 sentence.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ae3_the_lobby_cap_is_per_side_by_effective_type_and_reads_the_config() {
    use tacsim::definitions::UnitClass;

    // (1) Per side, not per map.
    let mut rig = Rig::boot(3);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(2, "sim team red");
    let said = rig.say(0, "sim unit drone_mk1");
    assert!(said.contains("Slot red 1 will field drone_mk1."), "{said}");
    let said = rig.say(1, "sim unit drone_mk1");
    assert!(
        said.contains("Slot blue 1 will field drone_mk1."),
        "blue's drone is blue's first: {said}"
    );
    let said = rig.say(2, "sim unit drone_mk1");
    assert!(
        said.contains("A side fields at most 1 drone; pick another type."),
        "red's second drone: {said}"
    );
    assert!(!said.contains("will field"), "{said}");
    let board = rig.say(0, "sim status");
    assert_eq!(
        board.matches("[drone_mk1]").count(),
        2,
        "one drone a side: {board}"
    );

    // (2) The effective type: a slot with no choice is its pod's tank.
    let mut rig = ae3_capped_rig(2, &[(UnitClass::Armour, 1)]);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    let said = rig.say(1, "sim unit line_tank");
    assert!(
        said.contains("A side fields at most 1 armoured machine; pick another type."),
        "red slot 1 flies its pod's line tank without having chosen it: {said}"
    );
    let said = rig.say(1, "sim unit biped_mk1");
    assert!(
        said.contains("Slot red 2 will field biped_mk1."),
        "a walker is another class: {said}"
    );

    // (3) Zero forbids the class.
    let mut rig = ae3_capped_rig(1, &[(UnitClass::Drone, 0)]);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim unit drone_mk1");
    assert!(
        said.contains("This server fields no drones; pick another type."),
        "{said}"
    );
    let board = rig.say(0, "sim status");
    assert!(!board.contains("[drone_mk1]"), "{board}");
}

// ---------------------------------------------------------------------------
// 16b cut AF.2 (16c N16) — the two documents of a running match
// ---------------------------------------------------------------------------

/// One module write, as `(set|del, key, value)`.
type Write = (&'static str, String, Option<serde_json::Value>);

/// The module writes a list of ops carries, in order.
fn writes_of(ops: Vec<mush_core::ModuleOp>) -> Vec<Write> {
    ops.into_iter()
        .filter_map(|op| match op.op {
            mush_core::ModuleWrite::JsonSet { key, value } => {
                Some(("set", key.to_string(), Some(value)))
            }
            mush_core::ModuleWrite::Del { key } => Some(("del", key.to_string(), None)),
            mush_core::ModuleWrite::StreamAppend { .. } => None,
        })
        .collect()
}

/// The value one `set` of `key` wrote, if the writes hold one.
fn set_of(writes: &[Write], key: &str) -> Option<serde_json::Value> {
    writes
        .iter()
        .rev()
        .find(|(op, at, _)| *op == "set" && at == key)
        .and_then(|(_, _, value)| value.clone())
}

/// Whether the writes delete `key`.
fn deletes(writes: &[Write], key: &str) -> bool {
    writes.iter().any(|(op, at, _)| *op == "del" && at == key)
}

impl Rig {
    /// **`BOARD` from the pilot's team room, and the writes of that one
    /// action** (16b cut AF.2): whatever was queued before is discarded first,
    /// so what comes back is the check-out's own action when this `BOARD` fires
    /// the trigger. A real `BOARD`, because a real seat is what gives a pilot a
    /// debrief fold.
    fn board_writes(&mut self, n: usize, entry: &str) -> (String, Vec<Write>) {
        let room = match_::centre_object(self.tw.world(), entry).expect("a team room");
        let pilot = self.pilot(n);
        self.tw
            .world_mut()
            .move_to(pilot, room)
            .expect("a pilot walks into their team room");
        let _ = self.pump_ops();
        let said = self.say(n, "board");
        (said, writes_of(self.pump_ops()))
    }

    /// **One pump, and the writes of the one action it applied** (16b cut
    /// AF.2), with the wall time the world spent on it: whatever was queued
    /// before is discarded first, so what comes back is this pump's alone.
    fn pump_writes(&mut self) -> (Vec<Write>, f64) {
        let _ = self.pump_ops();
        let at = std::time::Instant::now();
        self.pump();
        let ms = at.elapsed().as_secs_f64() * 1_000.0;
        (writes_of(self.pump_ops()), ms)
    }

    /// **Hand the world one egress message no running task would send now**
    /// (16b cut AF.2): staged on the registry's tasks and drained by the egress
    /// wake, as [`Rig::pump`] does with what a task sent, and the module writes
    /// that one action carried.
    fn deliver_writes(&mut self, message: tacsim::arena_task::ArenaOut) -> Vec<Write> {
        let tasks = self
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
            .expect("the arena registry");
        assert!(tasks.stage(message), "the egress stage takes the message");
        let _ = self.pump_ops();
        self.tw.apply(mush_core::WorldCommand::ModuleAsyncDone {
            module: smol_str::SmolStr::new_static(tacsim::arena_bridge::MODULE),
            actor: GOD,
            correlation: 0,
            payload: tacsim::arena_bridge::EGRESS_PAYLOAD.to_owned(),
        });
        writes_of(self.pump_ops())
    }
}

/// **The two documents ride the arena writer lane beside the match, and go with
/// its result** (16b cut AF.2; 16c N16; the brief's test 4).
///
/// - **At check-out**: the `BOARD` that fires `Boarding → Running` writes
///   `tacsim:live:<id>` in its own action, beside the tank's deployed marker,
///   and the record inside is `running` with a fold per pilot and the replay
///   fold.
/// - **At a snapshot**: the pump that hands the world the arena's `Snapshot`
///   writes `tacsim:snapshot:<id>` and `tacsim:live:<id>` in that one action,
///   the live record's `at_ms` the snapshot's own; the snapshot document is the
///   arena's (`tacsim.snapshot`, no `match` key).
/// - **`@arena/status`**: staff read `last snapshot: at N ms of arena time`;
///   a pilot's `@arena/status` is refused byte for byte as it was before any
///   snapshot existed (the observer rule's assertion for the new line).
/// - **At the abort**: the live record is rewritten `stopping` in the abort's
///   own action, before any end.
/// - **At the end**: the action that writes the result deletes both.
/// - **After the end**: a `Snapshot` the world drains for the finished match is
///   dropped, and neither document comes back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn af2_the_two_documents_ride_the_lane_and_go_with_the_result() {
    let mut rig = Rig::boot(2);
    // The control-lease mirror, read once before anybody boards.
    rig.tick_to(1);
    let refused = rig.say(0, "@arena/status");
    assert!(refused.contains("Permission denied."), "{refused}");
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    let (said, _) = rig.board_writes(0, match_::RED_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    let (said, checkout) = rig.board_writes(1, match_::BLUE_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    assert_eq!(
        rig.state(),
        MatchState::Running,
        "the second BOARD fires the check-out"
    );
    let match_id = rig.centre().match_id.expect("a running match");
    let live_key = match_::live_key(match_id).to_string();
    let snapshot_key = match_::snapshot_key(match_id).to_string();

    // (1) At check-out, in the check-out's own action.
    let document = set_of(&checkout, &live_key)
        .unwrap_or_else(|| panic!("the check-out wrote no live record: {checkout:?}"));
    assert!(
        checkout
            .iter()
            .any(|(op, key, _)| *op == "set" && key.starts_with("unit:")),
        "the markers ride the same action: {:?}",
        checkout
            .iter()
            .map(|(op, key, _)| (op, key))
            .collect::<Vec<_>>()
    );
    let live = match_::LiveRecord::from_document(&document).expect("the live record decodes");
    assert_eq!(live.match_id, match_id);
    assert_eq!(live.record.state, MatchState::Running);
    assert_eq!(live.record.folds.len(), 2, "a fold per pilot");
    assert!(live.record.replay.is_some(), "the replay fold");
    assert_eq!((live.at_ms, live.resumed_from_ms), (0, None));
    // The round trip holds on what the world wrote, and a moved byte is refused.
    assert_eq!(
        live.to_document().as_ref().ok(),
        Some(&document),
        "the document is its own round trip"
    );
    let mut moved = document.clone();
    if let Some(state) = moved["state"]
        .as_str()
        .map(|state| state.replacen("\"id\":", "\"id\": ", 1))
    {
        moved["state"] = serde_json::Value::from(state);
    }
    assert!(matches!(
        match_::LiveRecord::from_document(&moved),
        Err(match_::LiveRecordError::DigestMismatch { .. })
    ));

    // (2) At a snapshot: both documents in the one action of one pump.
    let mut taken: Option<(Vec<Write>, serde_json::Value)> = None;
    for _ in 0..400 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let (writes, _) = rig.pump_writes();
        if let Some(snapshot) = set_of(&writes, &snapshot_key) {
            taken = Some((writes, snapshot));
            break;
        }
    }
    let (writes, snapshot) = taken.expect("the arena snapshotted its opening pulse");
    assert_eq!(snapshot["record"], "tacsim.snapshot");
    assert!(snapshot.get("match").is_none(), "the key names the match");
    let head = tacsim::arena::Snapshot::head(&snapshot).expect("the head reads");
    let live = set_of(&writes, &live_key)
        .map(|document| match_::LiveRecord::from_document(&document))
        .expect("the live record rides the snapshot's action")
        .expect("and decodes");
    assert_eq!(
        live.at_ms, head.at_ms,
        "the two documents are of one moment"
    );
    assert_eq!(live.record.state, MatchState::Running);
    assert_eq!(live.centre.match_id, Some(match_id));
    assert_eq!(live.centre.state, MatchState::Running);

    // (3) `@arena/status`, staff and pilot.
    let status = rig.staff("@arena/status");
    assert!(
        status.contains(&format!(
            "\n  last snapshot: at {} ms of arena time",
            head.at_ms
        )),
        "{status}"
    );
    assert_eq!(
        rig.say(0, "@arena/status"),
        refused,
        "a pilot's refusal is the same bytes with a snapshot written"
    );

    // (4) The abort parks the record, and the live record says so at once.
    let _ = rig.pump_ops();
    let said = rig.say(1, "sim abort");
    assert!(said.contains("aborts match"), "{said}");
    let writes = writes_of(rig.pump_ops());
    let stopping = set_of(&writes, &live_key)
        .map(|document| match_::LiveRecord::from_document(&document))
        .expect("the abort rewrote the live record")
        .expect("and it decodes");
    assert_eq!(stopping.record.state, MatchState::Stopping);
    assert!(
        !writes
            .iter()
            .any(|(_, key, _)| key == &match_::result_key(match_id).to_string()),
        "no result yet"
    );

    // (5) The end deletes both, in the result's own action.
    let result_key = match_::result_key(match_id).to_string();
    let mut ended: Option<Vec<Write>> = None;
    for _ in 0..400 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let (writes, _) = rig.pump_writes();
        if set_of(&writes, &result_key).is_some() {
            ended = Some(writes);
            break;
        }
    }
    let writes = ended.expect("the check-in lands");
    assert!(deletes(&writes, &live_key), "{writes:?}");
    assert!(deletes(&writes, &snapshot_key), "{writes:?}");
    assert_eq!(rig.state(), MatchState::Aborted);

    // (6) A snapshot the world reads AFTER the result — one a task put on the
    // shared egress before it went — writes nothing: the match is not held
    // live, and writing would re-create the two documents its result deleted,
    // to lie there until the next boot (and, from 16b cut AF.3, to be read by
    // it as a running match that has a result).
    let late = tacsim::arena::Snapshot {
        at_ms: head.at_ms,
        state_hash: head.state_hash,
        ledger_seq: head.ledger_seq,
        chunk_seq: head.chunk_seq,
        arena: head.arena.clone(),
        definition_digest: head.definition_digest.clone(),
        document: snapshot,
    };
    let writes = rig.deliver_writes(tacsim::arena_task::ArenaOut::Snapshot {
        match_id,
        snapshot: Arc::new(late),
    });
    assert!(
        !writes
            .iter()
            .any(|(_, key, _)| key == &live_key || key == &snapshot_key),
        "a snapshot of a finished match wrote: {writes:?}"
    );
}

/// **The bytes `value` takes inside another document's JSON string** (16b cut
/// AF.2): its compact JSON, escaped once more — what a fold line or a replay
/// entry costs inside the live record's `state`, which the store holds as one
/// string, every `"` a `\"`.
fn escaped_len(value: &serde_json::Value) -> usize {
    let inner = serde_json::to_string(value).unwrap_or_default();
    serde_json::to_string(&serde_json::Value::String(inner))
        .map_or(0, |outer| outer.len().saturating_sub(2))
}

/// **The summed [`escaped_len`] of an array's items, one comma each.**
fn escaped_items(value: &serde_json::Value) -> (usize, usize) {
    value.as_array().map_or((0, 0), |items| {
        (
            items.iter().map(|item| escaped_len(item) + 1).sum(),
            items.len(),
        )
    })
}

/// **The summed compact length of an array's items, one comma each** — the
/// measure `ReplayFold`'s byte budget counts its document's entries in.
fn compact_items(value: &serde_json::Value) -> usize {
    value.as_array().map_or(0, |items| {
        items
            .iter()
            .map(|item| serde_json::to_string(item).map_or(0, |text| text.len() + 1))
            .sum()
    })
}

/// **What the world pays per snapshot, at the first and the last snapshot of a
/// minute of fire** (16b cut AF.2; the brief's item 6): twelve tanks — two
/// pilots and ten machines of the AI fill — the machines posed astride the
/// proving ground's crossroads fifty-four cells apart, inside the authored
/// sight, so they fight from the first passes, and each pilot's tank posed
/// eight cells in front of the OTHER side's line and powered up, so the
/// machines see it, shoot it, and its debrief fold fills with the hit lines
/// the live record carries; sixty arena seconds on tokio's paused clock.
///
/// At the opening snapshot and at the snapshot at 60,000 ms it prints
/// `af2_world_side:` — the live record's bytes **as stored** and its
/// serialisation time (the work `write_live` does, timed on the decoded
/// record), the snapshot document's bytes as stored, and the wall time of the
/// one pump that carried the snapshot through `on_snapshot` on the world loop —
/// and after them `af2_world_side_ceiling:`, an **estimate** of the live
/// record's ceiling: the last document as stored, plus twenty-four folds at
/// [`tacsim::match_::report::REPORT_MAX_LINES`] lines at the measured bytes a
/// kept line costs inside the stored `state` string, plus the replay fold at
/// [`tacsim::match_::replay::REPLAY_MAX_BYTES`] scaled by the measured ratio of
/// the fold's serde form, escaped, to the compact arrays that budget counts
/// (the larger of the shots' and the keyframes' ratios).
///
/// Asserted: the match fought, every snapshot came with its live record, the
/// pilots' folds filled, and every live record the world wrote re-encodes, off
/// its own decode, to the very document written — the round trip on folds and
/// a replay fold that carry data.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn af2_the_world_side_cost_at_the_first_and_the_last_snapshot_of_a_minute_of_fire() {
    let mut rig = fill_rig(2);
    // The control-lease mirror, read once before anybody boards.
    rig.tick_to(1);
    for (letter, row) in ["A", "B", "C", "D", "E", "F"]
        .iter()
        .zip([97, 95, 99, 93, 101, 91])
    {
        for said in [
            rig.staff(&format!("@arena/pose R{letter}=102 {row} 90")),
            rig.staff(&format!("@arena/pose B{letter}=156 {row} 270")),
        ] {
            assert!(said.contains("opens at cell"), "{said}");
        }
    }
    // The pilots' own tanks (RA and BA), each in front of the other side's
    // machines and facing them, so the machines' fire lands on them.
    for said in [
        rig.staff("@arena/pose RA=148 97 90"),
        rig.staff("@arena/pose BA=110 97 270"),
    ] {
        assert!(said.contains("opens at cell"), "{said}");
    }
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let said = rig.say(0, "sim ai 6");
    assert!(said.contains('6'), "{said}");
    rig.say(0, "sim start");
    rig.board_writes(0, match_::RED_ROOM_ENTRY);
    rig.board_writes(1, match_::BLUE_ROOM_ENTRY);
    assert_eq!(rig.state(), MatchState::Running);
    assert_eq!(rig.match_units().len(), 12, "six a side");
    // Both pilots power up, so their tanks run hot and the machines see them.
    for n in 0..2 {
        let said = rig.say(n, "startup");
        assert!(!said.is_empty(), "STARTUP answers");
    }
    let match_id = rig.centre().match_id.expect("a running match");
    let live_key = match_::live_key(match_id).to_string();
    let snapshot_key = match_::snapshot_key(match_id).to_string();

    let mut first: Option<String> = None;
    let mut last: Option<(i64, String, serde_json::Value, match_::LiveRecord)> = None;
    let mut seen = 0_u32;
    for _ in 0..2_000 {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (writes, pump_ms) = rig.pump_writes();
        let Some(snapshot) = set_of(&writes, &snapshot_key) else {
            continue;
        };
        seen += 1;
        let document =
            set_of(&writes, &live_key).expect("every snapshot rides with its live record");
        let live = match_::LiveRecord::from_document(&document).expect("the live record decodes");
        let at = std::time::Instant::now();
        let encoded = live.to_document().expect("the live record serialises");
        let live_encode_ms = at.elapsed().as_secs_f64() * 1_000.0;
        // The round trip on real data: nothing a derive carries is dropped or
        // altered between what the world wrote and what the boot would read.
        assert_eq!(
            encoded, document,
            "the live record at {} ms re-encodes to the document written",
            live.at_ms
        );
        let live_bytes = serde_json::to_string(&document).map_or(0, |text| text.len());
        let snapshot_bytes = serde_json::to_string(&snapshot).map_or(0, |text| text.len());
        let lines: usize = live
            .record
            .folds
            .values()
            .map(|fold| fold.clone().finish().lines.len())
            .sum();
        let shots = live.record.replay.as_ref().map_or(0, |fold| {
            fold.clone()
                .finish(match_::MatchOutcome::Draw, "")
                .shots
                .len()
        });
        let line = |word: &str| {
            format!(
                "af2_world_side: at={word} at_ms={} live_bytes={live_bytes} live_encode_ms={live_encode_ms:.3} snapshot_bytes={snapshot_bytes} snapshot_set_ms={pump_ms:.3} folds={} fold_lines={lines} replay_shots={shots}",
                live.at_ms,
                live.record.folds.len()
            )
        };
        if first.is_none() {
            first = Some(line("first"));
        }
        let at_ms = live.at_ms;
        last = Some((at_ms, line("last"), document, live));
        if at_ms >= 60_000 {
            break;
        }
    }
    let first = first.expect("the opening snapshot");
    let (at_ms, last_line, document, live) = last.expect("a last snapshot");
    assert!(at_ms >= 60_000, "the minute ran: last snapshot at {at_ms}");
    assert!(seen >= 13, "a snapshot every five arena seconds: {seen}");
    println!("{first}");
    println!("{last_line}");
    // The fight happened: shots are on the replay fold the live record carries,
    // and the pilots' folds carry more than their `power online.` lines.
    let fold = live.record.replay.clone().expect("the replay fold");
    let replay = fold.clone().finish(match_::MatchOutcome::Draw, "");
    assert!(!replay.shots.is_empty(), "the minute had no fire");
    let fold_lines: usize = live
        .record
        .folds
        .values()
        .map(|fold| fold.clone().finish().lines.len())
        .sum();
    assert!(
        fold_lines >= 10,
        "the pilots' folds filled: {fold_lines} lines"
    );

    // The ceiling, an estimate. (a) One kept report line's bytes inside the
    // stored `state` string, measured over every line the folds hold.
    let state: serde_json::Value = document["state"]
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .expect("the state parses");
    let (line_bytes, line_count) = state["record"]["folds"]
        .as_object()
        .into_iter()
        .flat_map(|folds| folds.values())
        .map(|fold| escaped_items(&fold["lines"]))
        .fold((0_usize, 0_usize), |(bytes, count), (b, c)| {
            (bytes + b, count + c)
        });
    let per_line = line_bytes.checked_div(line_count).unwrap_or(0);
    // (b) The replay fold's serde form, escaped, against the compact arrays
    // `REPLAY_MAX_BYTES` budgets (`ReplayFold::encoded_bytes`): per shot and
    // per keyframe, the larger ratio taken.
    let held = &state["record"]["replay"]["replay"];
    let written = replay.to_document();
    let (shot_bytes, shot_count) = escaped_items(&held["shots"]);
    let (frame_bytes, frame_count) = escaped_items(&held["keyframes"]);
    assert_eq!(
        shot_count,
        replay.shots.len(),
        "the state's shots are the fold's"
    );
    assert_eq!(frame_count, replay.keyframes.len(), "and its keyframes");
    #[allow(clippy::cast_precision_loss)]
    let ratio = |held: usize, compact: usize| {
        if compact == 0 {
            0.0
        } else {
            held as f64 / compact as f64
        }
    };
    let shot_ratio = ratio(shot_bytes, compact_items(&written["shots"]));
    let frame_ratio = ratio(frame_bytes, compact_items(&written["keyframes"]));
    let replay_ratio = shot_ratio.max(frame_ratio);
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let replay_ceiling =
        (tacsim::match_::replay::REPLAY_MAX_BYTES as f64 * replay_ratio).ceil() as usize;
    let live_bytes = serde_json::to_string(&document).map_or(0, |text| text.len());
    let ceiling =
        live_bytes + 24 * tacsim::match_::report::REPORT_MAX_LINES * per_line + replay_ceiling;
    println!(
        "af2_world_side_ceiling: estimate folds=24 lines_per_fold={} bytes_per_line_stored={per_line} (over {line_count} lines) replay_max_bytes={} replay_ratio_shots={shot_ratio:.2} replay_ratio_keyframes={frame_ratio:.2} replay_stored_bytes~{replay_ceiling} live_stored_bytes~{ceiling}",
        tacsim::match_::report::REPORT_MAX_LINES,
        tacsim::match_::replay::REPLAY_MAX_BYTES
    );
}

// ---------------------------------------------------------------------------
// 16b cut AF.3 (16c N16) — the boot resumes a running match
// ---------------------------------------------------------------------------

/// **The store, as one process's writes left it** (16b cut AF.3): per module
/// namespace, per key, the last document written — a `JsonSet` replaces, a
/// `Del` removes, a `StreamAppend` is not a document. Cumulative across every
/// drain a test makes, so a document written before a mid-test drain is not
/// lost to the reboot ([`Rig::pump_ops`]).
type StoreImage = BTreeMap<smol_str::SmolStr, BTreeMap<smol_str::SmolStr, serde_json::Value>>;

/// Fold `ops` into `image`, in order.
fn fold_ops(image: &mut StoreImage, ops: &[mush_core::ModuleOp]) {
    for op in ops {
        let docs = image.entry(op.ns.clone()).or_default();
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
}

impl Rig {
    /// **Take the module writes queued since the last take, fold them into the
    /// store image and hand them back** (16b cut AF.3) — what every test uses
    /// instead of a bare `take_module_ops`, so [`Rig::reboot_mid_match`] boots
    /// from every document this world has written.
    fn pump_ops(&mut self) -> Vec<mush_core::ModuleOp> {
        let ops = self.tw.take_module_ops();
        fold_ops(&mut self.image, &ops);
        ops
    }

    /// **The store image** (16b cut AF.3), for a test to edit between the
    /// shutdown and the boot of [`Rig::reboot_mid_match`] — through
    /// [`Rig::reboot_mid_match_editing`].
    fn store_image(&mut self) -> &mut StoreImage {
        &mut self.image
    }

    /// **A restart of the server half with a match running, in process** (16b
    /// cut AF.3; the brief's item 11), answering the writes the boot made.
    async fn reboot_mid_match(&mut self) -> Vec<Write> {
        self.reboot_mid_match_editing(|_| {}).await
    }

    /// [`Rig::reboot_mid_match`], with `edit` run on the store image after the
    /// old process has stopped and before the new one boots:
    ///
    /// 1. every live task of the old registry is stopped and joined
    ///    (`ArenaTasks::shutdown`, as `mush_server::world_task::arena_shutdown`
    ///    does); whatever the old tasks had put on the egress and the world had
    ///    not drained yet is **dropped on purpose** — a process that crashed
    ///    never handled it, and the stop signal sends no `Ended` — and the writes
    ///    the world had already queued are folded into the image;
    /// 2. edit and normalize the retained image after the old tasks stop;
    /// 3. register the actual roster afresh over the same World and configured
    ///    Tacsim owner, then run every module's boot and restore the account map;
    /// 4. claim the fresh registry's egress and return the real boot writes.
    ///
    /// The split-server fixture retains its clock and connection flags. No
    /// reconnect or tick consumes the resume/hatch notices under assertion.
    async fn reboot_mid_match_editing(&mut self, edit: impl FnOnce(&mut StoreImage)) -> Vec<Write> {
        let cfg = self
            .tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .expect("the configured Tacsim owner")
            .config()
            .clone();
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
        edit(self.store_image());
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
        let (empty, registry) = mush_testkit::boot_world();
        let world = std::mem::replace(self.tw.world_mut(), empty);
        *self.tw.engine_mut() = mush_core::Engine::with_modules(world, registry, roster_with(cfg))
            .expect("the same production roster registers afresh");
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
        self.egress = self
            .tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .map(|registry| registry.tasks().clone())
            .and_then(|tasks| tasks.take_egress());
        assert!(self.egress.is_some(), "the fresh process owns arena egress");
        writes_of(self.pump_ops())
    }

    /// The tacsim document the image holds under `key`.
    fn image_doc(&self, key: &str) -> Option<serde_json::Value> {
        self.image
            .get(tacsim::NS)
            .and_then(|docs| docs.get(key))
            .cloned()
    }

    /// The match book, read.
    fn book(&self) -> &match_::MatchBook {
        self.tw
            .world()
            .module_state::<match_::MatchBook>()
            .expect("the match book")
    }

    /// The arena registry, read.
    fn registry(&self) -> &tacsim::arena_bridge::ArenaRegistry {
        self.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .expect("the arena registry")
    }
}

/// One captured event: its level and its fields.
type CapturedEvent = (tracing::Level, BTreeMap<String, String>);

/// **Every event the tracing layer is handed, with its fields** (16b cut
/// AF.3): the rig's own subscriber, set as the thread's default for one test,
/// so a boot's error line and its `reason` field can be read.
#[derive(Clone, Default)]
struct Captured(Arc<std::sync::Mutex<Vec<CapturedEvent>>>);

/// The field visitor [`Captured`] records with: strings as they are, the rest
/// by their `Debug`.
struct FieldsOf<'a>(&'a mut BTreeMap<String, String>);

impl tracing::field::Visit for FieldsOf<'_> {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }

    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }
}

impl tracing::Subscriber for Captured {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut fields = BTreeMap::new();
        event.record(&mut FieldsOf(&mut fields));
        if let Ok(mut held) = self.0.lock() {
            held.push((*event.metadata().level(), fields));
        }
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

impl Captured {
    /// The reason the boot's one void line gave for `match_id`, if it gave one.
    fn void_reason(&self, match_id: u64) -> Option<String> {
        let held = self.0.lock().ok()?;
        held.iter()
            .filter(|(level, _)| *level == tracing::Level::ERROR)
            .map(|(_, fields)| fields)
            .find(|fields| {
                fields
                    .get("message")
                    .is_some_and(|message| message.contains("could not resume"))
                    && fields.get("match_id").map(String::as_str)
                        == Some(match_id.to_string().as_str())
            })
            .and_then(|fields| fields.get("reason").cloned())
    }

    /// How many events whose message holds `needle` were seen.
    fn said(&self, needle: &str) -> usize {
        self.0.lock().map_or(0, |held| {
            held.iter()
                .filter(|(_, fields)| {
                    fields
                        .get("message")
                        .is_some_and(|message| message.contains(needle))
                })
                .count()
        })
    }
}

/// `[tacsim]` for the resume's tests: [`config`] with a snapshot every arena
/// second and a position summary every 20 ms, so a 16 KiB chunk closes in a
/// couple of seconds and a snapshot follows it within one more.
fn af3_config() -> TacsimConfig {
    let mut cfg = config();
    cfg.arena.snapshot_ms = 1_000;
    cfg.arena.ledger.summary_ms = 20;
    cfg
}

/// A rig on [`af3_config`].
fn af3_rig(pilots: usize) -> Rig {
    Rig::boot_on(pilots, roster_with(af3_config()))
}

/// **A running lobby match**: pilots 0 (red) and 1 (blue) claimed, started and
/// boarded through the real `BOARD`. Answers the match id.
fn af3_running(rig: &mut Rig) -> tacsim::arena_task::MatchId {
    // The control-lease mirror, read once before anybody boards.
    rig.tick_to(1);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    let (said, _) = rig.board_writes(0, match_::RED_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    let (said, _) = rig.board_writes(1, match_::BLUE_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    assert_eq!(rig.state(), MatchState::Running);
    rig.centre().match_id.expect("a running match")
}

/// Pump until the image holds a snapshot of `match_id` taken after at least
/// `chunks` of its chunks had closed, and answer its head.
async fn af3_snapshot_after(
    rig: &mut Rig,
    match_id: tacsim::arena_task::MatchId,
    chunks: u64,
) -> tacsim::arena::SnapshotHead {
    let key = match_::snapshot_key(match_id).to_string();
    for _ in 0..3_000 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
        let _ = rig.pump_ops();
        if let Some(head) = rig
            .image_doc(&key)
            .and_then(|document| tacsim::arena::Snapshot::head(&document).ok())
            .filter(|head| head.chunk_seq >= chunks)
        {
            return head;
        }
    }
    panic!("no snapshot of match {match_id} after {chunks} chunk(s)");
}

/// The chunk documents of `match_id`'s log the image holds, by index.
fn af3_log_docs(
    rig: &Rig,
    match_id: tacsim::arena_task::MatchId,
) -> BTreeMap<u64, serde_json::Value> {
    let log = tacsim::arena_bridge::match_log_id(match_id);
    rig.image
        .get(tacsim::NS)
        .map(|docs| {
            docs.iter()
                .filter_map(|(key, value)| {
                    let (id, index) = mush_core::parse_chunk_key(key)?;
                    (id == log).then(|| (index, value.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The world-ledger chunk index the log's next append opens: the tail's own
/// when it is unsealed, the one after when it is sealed, 0 for no log.
fn af3_next_chunk(docs: &BTreeMap<u64, serde_json::Value>) -> u64 {
    docs.iter().next_back().map_or(0, |(index, doc)| {
        if doc.get("sealed").and_then(serde_json::Value::as_bool) == Some(true) {
            index + 1
        } else {
            *index
        }
    })
}

/// A document's state string edited by `edit` and its digest stamped again —
/// the live record's and the snapshot's, which are stamped the same way.
fn af3_restamp(document: &mut serde_json::Value, edit: impl FnOnce(&str) -> String) {
    use sha2::Digest as _;
    let state = document["state"]
        .as_str()
        .map(edit)
        .expect("a state string");
    document["state_digest"] = serde_json::Value::String(tacsim::arena::snapshot::digest_text(
        &sha2::Sha256::digest(state.as_bytes()).into(),
    ));
    document["state"] = serde_json::Value::String(state);
}

/// The keys a list of writes touched, for a failure message.
fn keys_of(writes: &[Write]) -> Vec<&str> {
    writes.iter().map(|(_, key, _)| key.as_str()).collect()
}

/// **17b cut M1b.1 — the configured floor reaches the arena through the
/// production path** (doc 17 §5, D-UM-17 as amended: *captured into the arena
/// at check-out*). `[tacsim.minds].odds_floor_permille = 50` on the world's
/// config, a lobby match started and boarded through the real verbs: the
/// `open_match` → `lobby::minds_config` → `spawn_arena` chain captures 50 (not
/// the default 100) into the running arena, which its first snapshot carries.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m1b1_open_match_captures_the_configured_floor_into_the_arena() {
    let mut cfg = af3_config();
    cfg.minds.odds_floor_permille = 50;
    assert_ne!(
        cfg.minds.odds_floor_permille,
        tacsim::config::MindsConfig::default().odds_floor_permille,
        "the configured floor is not the default"
    );
    let mut rig = Rig::boot_on(2, roster_with(cfg));
    let match_id = af3_running(&mut rig);
    let head = af3_snapshot_after(&mut rig, match_id, 0).await;
    let document = rig
        .image_doc(&match_::snapshot_key(match_id))
        .expect("the snapshot is in the store");
    let ground = match_::definition_for(rig.tw.world(), &head.arena)
        .expect("the ground builds")
        .expect("and is loaded");
    let arena = tacsim::arena::Snapshot::decode(&document, ground).expect("the snapshot decodes");
    assert_eq!(
        arena.odds_floor_permille, 50,
        "the floor the match was opened under"
    );
}

/// **The boot resumes a match from its two documents** (16b cut AF.3; the
/// brief's test 1), over the in-process reboot rig: a lobby match with two
/// seated pilots, snapshotted after a chunk had closed, and the server half
/// restarted under it.
///
/// After the boot: both units are still deployed; the record is `running` with
/// its folds; the centre is `running` on the match with every seated slot
/// dropped, left and `released_by_resume`, the leads resettled (every slot has
/// left, so no side has one) and the unattended clock stamped; the task is
/// live; the world's epoch mirror is the snapshot's table plus one on every
/// channel of a seat it held; the boot wrote the `resumed` marker into
/// `log:match.<id>` at the log's next chunk (never chunk 0, and no earlier chunk
/// document rewritten), the live record with `resumed_from_ms` and no result
/// document; the task's first frames carry an arena clock at or past the
/// snapshot's moment. Then an abort ends it through `finish` with an ordinary
/// result: `ledger.chunks` at least the chunks the live record counted, a
/// replay, a report per pilot, and both documents deleted.
///
/// **Two more arms**, on a second match: a live record shaped as a 16b cut AF.2
/// running half wrote it (no `released_by_resume` key anywhere) still resumes —
/// the promise the field's `#[serde(default)]` keeps — and with the
/// `tacsim:match:seq` document gone the book's next id is still above the
/// resumed match's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn af3_a_boot_resumes_a_match_from_its_two_documents() {
    let mut rig = af3_rig(2);
    let match_id = af3_running(&mut rig);
    let id = match_id.0;
    let head = af3_snapshot_after(&mut rig, match_id, 1).await;
    let tanks: Vec<ObjId> = rig
        .centre()
        .slots
        .iter()
        .filter_map(|slot| slot.unit)
        .collect();
    assert_eq!(tanks.len(), 2);
    let live_key = match_::live_key(match_id).to_string();
    let live_before = rig
        .image_doc(&live_key)
        .map(|document| match_::LiveRecord::from_document(&document))
        .expect("the live record is in the store")
        .expect("and decodes");
    let snapshot_doc = rig
        .image_doc(&match_::snapshot_key(match_id))
        .expect("the snapshot is in the store");
    let log_before = af3_log_docs(&rig, match_id);
    let next_chunk = af3_next_chunk(&log_before);
    assert!(
        next_chunk > 0,
        "a chunk closed before the snapshot: {:?}",
        log_before.keys()
    );

    let writes = rig.reboot_mid_match().await;

    // Custody: both units are still out on loan and still refused.
    for tank in &tanks {
        assert!(
            mush_tactics::custody::marker_of(rig.tw.world(), *tank).is_some(),
            "#{} lost its marker",
            tank.0
        );
        assert!(mush_sim::deployed::is_deployed(rig.tw.world(), *tank));
    }
    // The record, as the live record carried it.
    let record = rig
        .book()
        .get(match_id)
        .cloned()
        .expect("the record is back");
    assert_eq!(record.state, MatchState::Running);
    assert_eq!(record.folds.len(), 2, "a fold per pilot");
    assert!(record.replay.is_some(), "the replay fold");
    // The centre, and every seat released to its sealed mind.
    let centre = rig.centre();
    assert_eq!(centre.state, MatchState::Running);
    assert_eq!(centre.match_id, Some(match_id));
    for slot in centre.slots.iter().filter(|slot| slot.is_seated()) {
        assert!(slot.is_dropped(), "{slot:?}");
        assert!(slot.left && slot.released_by_resume, "{slot:?}");
    }
    assert!(
        centre.leads.is_empty(),
        "every slot left: {:?}",
        centre.leads
    );
    assert!(centre.unattended_since_ms.is_some(), "the unattended clock");
    assert!(centre.board_deadline_ms.is_none());
    // The task.
    assert!(rig.registry().tasks().is_live(match_id), "the task is live");
    // The epoch mirror: the snapshot's table, plus one on a released seat.
    let ground = match_::definition_for(rig.tw.world(), &head.arena)
        .expect("the ground builds")
        .expect("and is loaded");
    let decoded =
        tacsim::arena::Snapshot::decode(&snapshot_doc, ground).expect("the snapshot decodes");
    assert_eq!(
        decoded.pilots.len(),
        2,
        "both seats were held at the snapshot"
    );
    for unit in &decoded.units {
        let seated = decoded.pilots.iter().any(|seat| seat.unit == unit.id);
        for channel in tacsim::arena::state::Channel::ALL {
            let table = decoded.epochs.current(unit.id, channel);
            assert_eq!(
                rig.registry().epoch_of(unit.id, channel),
                table + u64::from(seated),
                "#{} {channel:?}",
                unit.id.0
            );
        }
    }
    // The marker, at the log's next chunk; no earlier chunk touched.
    let log = tacsim::arena_bridge::match_log_id(match_id);
    let marker_key = mush_core::chunk_key(&log, next_chunk).to_string();
    let marked = set_of(&writes, &marker_key)
        .unwrap_or_else(|| panic!("no write of {marker_key}: {:?}", keys_of(&writes)));
    let marker = marked["entries"]
        .as_array()
        .and_then(|entries| entries.last())
        .map(|entry| entry["value"].clone())
        .expect("the chunk holds an entry");
    assert_eq!(
        marker["resumed"]["from_chunk"],
        serde_json::json!(head.chunk_seq)
    );
    assert_eq!(marker["resumed"]["at_ms"], serde_json::json!(head.at_ms));
    for index in 0..next_chunk {
        let key = mush_core::chunk_key(&log, index).to_string();
        assert!(set_of(&writes, &key).is_none(), "{key} was rewritten");
    }
    // The live record, rewritten with the resume's memory; no result.
    let live = set_of(&writes, &live_key)
        .map(|document| match_::LiveRecord::from_document(&document))
        .expect("the boot rewrote the live record")
        .expect("and it decodes");
    assert_eq!(live.resumed_from_ms, Some(head.at_ms));
    assert_eq!(live.at_ms, head.at_ms);
    assert!(live.centre.slots.iter().all(|slot| slot.released_by_resume));
    let result_key = match_::result_key(match_id).to_string();
    assert!(
        set_of(&writes, &result_key).is_none(),
        "a resumed match has no result"
    );
    // The task's first frames.
    let seen = rig.frames_seen.len();
    let mut first: Option<i64> = None;
    for _ in 0..400 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
        let _ = rig.pump_ops();
        if let Some((_, now_ms)) = rig
            .frames_seen
            .iter()
            .skip(seen)
            .find(|(held, _)| *held == id)
        {
            first = Some(*now_ms);
            break;
        }
    }
    let first = first.expect("the resumed task sent a frame");
    assert!(
        first >= head.at_ms,
        "{first} is before the snapshot at {}",
        head.at_ms
    );

    // An abort ends it with an ordinary result.
    let said = rig.staff(&format!("@arena/abort {id}"));
    assert!(said.contains("is stopping"), "{said}");
    let mut ended: Option<Vec<Write>> = None;
    for _ in 0..600 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let (writes, _) = rig.pump_writes();
        if set_of(&writes, &result_key).is_some() {
            ended = Some(writes);
            break;
        }
    }
    let writes = ended.expect("the check-in lands");
    let result = set_of(&writes, &result_key).expect("the result");
    assert_eq!(result["reason"], "abort", "{result}");
    assert!(
        result["ledger"]["chunks"].as_u64().unwrap_or(0) >= live_before.chunks_appended,
        "{result}"
    );
    assert_eq!(result["replay"], true, "{result}");
    for tank in &tanks {
        let report = match_::report_key(match_id, *tank).to_string();
        assert!(set_of(&writes, &report).is_some(), "{report}");
    }
    assert!(deletes(&writes, &live_key));
    assert!(deletes(&writes, &match_::snapshot_key(match_id)));
    for tank in &tanks {
        assert!(mush_tactics::custody::marker_of(rig.tw.world(), *tank).is_none());
    }

    // (Two more arms) An AF.2-shaped live record, and no counter document.
    let mut rig = af3_rig(2);
    let match_id = af3_running(&mut rig);
    let _ = af3_snapshot_after(&mut rig, match_id, 0).await;
    let key = match_::live_key(match_id).to_string();
    let writes = rig
        .reboot_mid_match_editing(|image| {
            let docs = image.entry(smol_str::SmolStr::new(tacsim::NS)).or_default();
            if let Some(document) = docs.get_mut(key.as_str()) {
                af3_restamp(document, |state| {
                    state.replace(",\"released_by_resume\":false", "")
                });
                assert!(
                    !document["state"]
                        .as_str()
                        .is_some_and(|state| state.contains("released_by_resume")),
                    "an AF.2-shaped record"
                );
            }
            docs.remove(match_::SEQ_KEY);
        })
        .await;
    assert!(rig.registry().tasks().is_live(match_id), "it resumed");
    assert_eq!(
        rig.book().get(match_id).map(|record| record.state),
        Some(MatchState::Running)
    );
    assert!(set_of(&writes, &match_::result_key(match_id)).is_none());
    assert!(
        rig.book().peek_seq() > match_id.0,
        "the next id {} is not above the resumed {}",
        rig.book().peek_seq(),
        match_id.0
    );
    let said = rig.staff(&format!("@arena/abort {}", match_id.0));
    assert!(said.contains("is stopping"), "{said}");
}

/// **One running lobby match for a resume test** (16b cut AF.3): the rig, the
/// match, the head of the snapshot the image holds (taken after at least
/// `chunks` closed chunks), the two tanks, and — when `moved` — red's tank
/// placed three cells east of its muster during `Boarding`, so the check-out
/// takes it there and only the boot's tank reset can put it back. `pods` is
/// each pilot and the pod their body sat in when the snapshot was taken.
struct Af3Match {
    rig: Rig,
    match_id: tacsim::arena_task::MatchId,
    head: tacsim::arena::SnapshotHead,
    tanks: Vec<ObjId>,
    moved: Option<(ObjId, mush_sim::data::Pose)>,
    pods: Vec<(ObjId, ObjId)>,
}

async fn af3_match(chunks: u64, moved: bool) -> Af3Match {
    let mut rig = af3_rig(2);
    rig.tick_to(1);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    let moved = if moved {
        let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red's tank");
        let (domain, pose) = mush_sim::pose_of(rig.tw.world(), red).expect("placed");
        let said = rig.staff(&format!(
            "@spatial/place/domain #{}={},{{\"x_cm\":{},\"y_cm\":{},\"heading_mdeg\":{}}}",
            red.0,
            domain.0,
            pose.x_cm + 3_000,
            pose.y_cm,
            pose.heading_mdeg
        ));
        assert!(said.contains("Placed"), "the placement was refused: {said}");
        let (_, at) = mush_sim::pose_of(rig.tw.world(), red).expect("placed");
        Some((red, at))
    } else {
        None
    };
    let (said, _) = rig.board_writes(0, match_::RED_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    let (said, _) = rig.board_writes(1, match_::BLUE_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    assert_eq!(rig.state(), MatchState::Running);
    let match_id = rig.centre().match_id.expect("a running match");
    let tanks: Vec<ObjId> = rig
        .centre()
        .slots
        .iter()
        .filter_map(|slot| slot.unit)
        .collect();
    let head = af3_snapshot_after(&mut rig, match_id, chunks).await;
    let pods: Vec<(ObjId, ObjId)> = (0..2)
        .map(|n| (rig.pilot(n), rig.pod(n).expect("a boarded pilot's pod")))
        .collect();
    for (pilot, pod) in &pods {
        assert_eq!(
            rig.tw
                .world()
                .get(*pilot)
                .and_then(|object| object.location),
            Some(*pod),
            "the pilot sits in the pod when the server stops"
        );
    }
    Af3Match {
        rig,
        match_id,
        head,
        tanks,
        moved,
        pods,
    }
}

impl Rig {
    /// One closure against a real `Ctx` under the tacsim module's identity.
    fn with_ctx<T>(&mut self, f: impl FnOnce(&mut Ctx<'_>) -> T) -> T {
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
        f(&mut ctx)
    }

    /// The billets held on `unit`.
    fn billets(&self, unit: ObjId) -> usize {
        self.tw
            .world()
            .module_state::<mush_sim::SpatialState>()
            .and_then(|state| state.assignments.get(&unit))
            .map_or(0, |host| host.seats.len())
    }
}

/// The tacsim namespace of a store image.
fn tacsim_docs(image: &mut StoreImage) -> &mut BTreeMap<smol_str::SmolStr, serde_json::Value> {
    image.entry(smol_str::SmolStr::new(tacsim::NS)).or_default()
}

/// Keep each refusal case's owned world on the heap. The lazy future also
/// keeps construction out of the enclosing sixteen-case test future.
#[inline(never)]
fn af3_match_boxed(
    chunks: u64,
    moved: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Box<Af3Match>>>> {
    Box::pin(async move { Box::new(af3_match(chunks, moved).await) })
}

/// What a void's reason must be.
enum Reason {
    /// These words exactly.
    Exact(String),
    /// Words that begin so (the rest carries a digest or a count).
    Prefix(&'static str),
}

/// **Reboot `m`'s server half with `edit` applied to the store, and hold the
/// boot to a void by name** (16b cut AF.3): the reason the one error line
/// gives; the units restored — no marker, not deployed, `@unit/damage` accepted
/// — with no billet left on them; the `aborted(restart)` result; both
/// documents deleted; no live task; and every roster tank at spec and at
/// muster (a second reset changes nothing, and a tank moved off its muster
/// before the check-out is back on it).
async fn af3_void(m: &mut Af3Match, edit: impl FnOnce(&mut StoreImage), reason: Reason) {
    let captured = Captured::default();
    let writes = {
        let _guard = tracing::subscriber::set_default(captured.clone());
        m.rig.reboot_mid_match_editing(edit).await
    };
    let id = m.match_id;
    let said = captured.void_reason(id.0);
    match reason {
        Reason::Exact(words) => assert_eq!(said.as_deref(), Some(words.as_str())),
        Reason::Prefix(words) => assert!(
            said.as_deref().is_some_and(|said| said.starts_with(words)),
            "{said:?} does not begin {words:?}"
        ),
    }
    // 16b cut U.4 (`B-37`) across a void: a voided match's centre is not
    // `Running`, so the boot's sweep keeps nothing — every pilot is put out of
    // the pod the match locked, and owed the hatch line.
    let owed = m.rig.book().hatch_notices().to_vec();
    for (pilot, pod) in &m.pods {
        assert_ne!(
            m.rig
                .tw
                .world()
                .get(*pilot)
                .and_then(|object| object.location),
            Some(*pod),
            "a voided match left #{} shut in its pod",
            pilot.0
        );
        assert!(
            owed.contains(&(*pilot, *pod)),
            "no hatch line is owed to #{}: {owed:?}",
            pilot.0
        );
    }
    for tank in &m.tanks {
        assert!(
            mush_tactics::custody::marker_of(m.rig.tw.world(), *tank).is_none(),
            "#{} kept its marker",
            tank.0
        );
        assert!(!mush_sim::deployed::is_deployed(m.rig.tw.world(), *tank));
        assert_eq!(m.rig.billets(*tank), 0, "#{} kept a billet", tank.0);
        let said = m
            .rig
            .staff(&format!("@unit/damage #{}={{\"ops\":[]}}", tank.0));
        assert!(said.contains("Applied 0 op(s)"), "{said}");
    }
    let result = set_of(&writes, &match_::result_key(id))
        .unwrap_or_else(|| panic!("no result: {:?}", keys_of(&writes)));
    assert_eq!(result["outcome"], "abort", "{result}");
    assert_eq!(result["reason"], "restart", "{result}");
    assert!(deletes(&writes, &match_::live_key(id)));
    assert!(deletes(&writes, &match_::snapshot_key(id)));
    assert!(
        !m.rig.registry().tasks().is_live(id),
        "a voided match has no task"
    );
    assert_ne!(m.rig.centre().state, MatchState::Running);
    let again = m
        .rig
        .with_ctx(|ctx| match_::reset_stock_tanks(ctx, &std::collections::BTreeSet::new()));
    assert!(
        !again.changed() && again.deployed.is_empty(),
        "the boot left a tank off its spec or its muster: {again:?}"
    );
    if let Some((tank, at)) = &m.moved {
        let (_, now) = mush_sim::pose_of(m.rig.tw.world(), *tank).expect("placed");
        assert_ne!(
            (now.x_cm, now.y_cm),
            (at.x_cm, at.y_cm),
            "the moved tank was not put back at muster"
        );
    }
}

/// The snapshot document the image holds for `m`, edited through its parsed
/// state and stamped again (so the edit is past the digest check).
fn af3_state_edit(document: &mut serde_json::Value, edit: impl FnOnce(&mut serde_json::Value)) {
    af3_restamp(document, |state| {
        let mut parsed: serde_json::Value = serde_json::from_str(state).expect("a JSON state");
        edit(&mut parsed);
        serde_json::to_string(&parsed).expect("a state encodes")
    });
}

/// **A snapshot document decoded on its ground, edited as an arena and
/// encoded again** (the fill decision's review, 2026-10-09): the lawful way
/// to rewrite what a snapshot carries, because [`tacsim::arena::Snapshot::encode`]
/// stamps the state hash and the state digest itself, so the boot's decode
/// reads the result as one the task wrote — a field the hash folds (a mind
/// row's `seal_digest`) included, where [`af3_state_edit`] restamps the
/// digest alone. The unedited arena's re-encode is asserted byte for byte
/// first (`tests/arena.rs`' encode, decode and encode again).
fn af3_reencode(
    document: &mut serde_json::Value,
    ground: tacsim::arena::state::ArenaDefinition,
    edit: impl FnOnce(&mut tacsim::arena::state::Arena),
) {
    let mut arena =
        tacsim::arena::Snapshot::decode(document, ground).expect("the snapshot decodes");
    let again = tacsim::arena::Snapshot::encode(&arena).expect("the decoded arena encodes");
    assert_eq!(
        again.document, *document,
        "the unedited arena encodes to the document it was decoded from"
    );
    edit(&mut arena);
    *document = tacsim::arena::Snapshot::encode(&arena)
        .expect("the edited arena encodes")
        .document;
}

/// **A match the boot cannot resume is voided by name, one reason at a time**
/// (16b cut AF.3; the brief's test 2), over the in-process reboot rig and the
/// store image of test 1 edited once per reason — every reason the in-process
/// rig can reach. (The layer off and `snapshot_ms = 0` rebuild the store slot
/// from the module's own frozen config, which this rig cannot change between
/// boots; they are `mushd`'s `arena_custody.rs`' arms.)
///
/// Each arm: the error line's reason is the closed list's word for its case;
/// the units are restored and at spec and at muster (the phase-B arms too:
/// red's tank is moved off its muster before the check-out, and only the
/// boot's reset puts it back); the seats are released; the `aborted(restart)`
/// document is written; both documents are deleted; no task runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn af3_a_boot_voids_a_match_it_cannot_resume_for_each_reason() {
    // No snapshot document.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::snapshot_key(m.match_id).to_string();
    af3_void(
        &mut m,
        |image| {
            tacsim_docs(image).remove(key.as_str());
        },
        Reason::Exact("no snapshot document".to_owned()),
    )
    .await;

    // No live record.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    af3_void(
        &mut m,
        |image| {
            tacsim_docs(image).remove(key.as_str());
        },
        Reason::Exact("no live record".to_owned()),
    )
    .await;

    // The live record's state edited without a new digest.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                let moved = document["state"]
                    .as_str()
                    .map(|state| state.replacen("\"id\":", "\"id\": ", 1))
                    .expect("a state");
                document["state"] = serde_json::Value::String(moved);
            }
        },
        Reason::Prefix("the live record does not decode: state digest mismatch: recorded "),
    )
    .await;

    // The live record parked `stopping` (an abort before the restart).
    let mut m = af3_match_boxed(0, true).await;
    let said = m.rig.staff(&format!("@arena/abort {}", m.match_id.0));
    assert!(said.contains("is stopping"), "{said}");
    af3_void(
        &mut m,
        |_| {},
        Reason::Exact("the live record is not running (state stopping)".to_owned()),
    )
    .await;

    // The check-out's own live record beside the opening snapshot (16b cut
    // AF.2's carried constraint: `at_ms` is 0 for both, so only the centre
    // tells them apart): its centre still `boarding`.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                af3_state_edit(document, |state| {
                    state["centre"]["state"] = serde_json::json!("boarding");
                });
            }
        },
        Reason::Exact("the live record is not running (state boarding)".to_owned()),
    )
    .await;

    // The snapshot's state edited without a new digest (phase B's decode).
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::snapshot_key(m.match_id).to_string();
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                let moved = document["state"]
                    .as_str()
                    .map(|state| state.replacen("\"now_ms\":", "\"now_ms\": ", 1))
                    .expect("a state");
                document["state"] = serde_json::Value::String(moved);
            }
        },
        Reason::Prefix("the snapshot does not decode: state digest mismatch: recorded "),
    )
    .await;

    // Edited and stamped again: the hash refuses it (phase B).
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::snapshot_key(m.match_id).to_string();
    let recorded = m.head.state_hash;
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                af3_state_edit(document, |state| {
                    let now = state["now_ms"].as_i64().unwrap_or(0);
                    state["now_ms"] = serde_json::json!(now + 1);
                });
            }
        },
        Reason::Prefix("state hash mismatch (recorded "),
    )
    .await;
    assert!(
        m.head.state_hash == recorded,
        "the head is the unedited snapshot's"
    );

    // Another ground on the head.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::snapshot_key(m.match_id).to_string();
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                document["arena_definition_digest"] = serde_json::json!("f".repeat(64));
            }
        },
        Reason::Exact("the snapshot and the record name different grounds".to_owned()),
    )
    .await;

    // Another locomotion table.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::snapshot_key(m.match_id).to_string();
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                document["physics_digest"] = serde_json::json!("0".repeat(64));
            }
        },
        Reason::Exact(format!(
            "the ground's movement prices changed (recorded {}, loaded {})",
            "0".repeat(64),
            mush_atlas::physics::LOCOMOTION_DIGEST
        )),
    )
    .await;

    // The two documents from two moments.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    let at_ms = m.head.at_ms;
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                document["at_ms"] = serde_json::json!(at_ms + 1_000);
            }
        },
        Reason::Exact(format!(
            "the live record and the snapshot are from different moments (live at {} ms, \
             snapshot at {at_ms} ms)",
            at_ms + 1_000
        )),
    )
    .await;

    // A marker on a unit the record does not field: red's second pod's tank,
    // marked as the match's with its own cells as the snapshot.
    let mut m = af3_match_boxed(0, true).await;
    let stray = match_::tank_for_slot(m.rig.tw.world(), PartyId::RED, 2).expect("red's second tank");
    let (id, arena) = (m.match_id.0, m.head.arena.clone());
    af3_void(
        &mut m,
        |image| {
            let docs = image
                .entry(smol_str::SmolStr::new(mush_tactics::module::NS))
                .or_default();
            if let Some(document) = docs.get_mut(format!("unit:{}", stray.0).as_str()) {
                let unit = document["unit"].clone();
                document["deployed"] = serde_json::json!({
                    "mark": { "match_id": id, "arena": arena.as_str() },
                    "snapshot": unit,
                });
            }
        },
        Reason::Exact("the units still checked out are not the record's roster".to_owned()),
    )
    .await;
    assert!(
        mush_tactics::custody::marker_of(m.rig.tw.world(), stray).is_none(),
        "the stray marker is restored with the match's"
    );

    // Already resumed from this very snapshot: the resume-once rule.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    let at_ms = m.head.at_ms;
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                document["resumed_from_ms"] = serde_json::json!(at_ms);
            }
        },
        Reason::Exact(format!(
            "the server stopped again before this match's next snapshot (already resumed \
             from {at_ms} ms)"
        )),
    )
    .await;

    // A breach in the resumed log: its last chunk is not a chunk.
    let mut m = af3_match_boxed(1, true).await;
    let last = af3_log_docs(&m.rig, m.match_id)
        .keys()
        .next_back()
        .copied()
        .expect("a chunk document");
    let key = mush_core::chunk_key(&tacsim::arena_bridge::match_log_id(m.match_id), last);
    af3_void(
        &mut m,
        |image| {
            tacsim_docs(image).insert(key, serde_json::json!("not a chunk"));
        },
        Reason::Prefix("the ledger tail could not be restored: "),
    )
    .await;

    // The tape behind the snapshot: every chunk document from the one holding
    // the snapshot's last closed chunk on is lost.
    let mut m = af3_match_boxed(1, true).await;
    let chunk_seq = m.head.chunk_seq;
    let lost: Vec<smol_str::SmolStr> = af3_log_docs(&m.rig, m.match_id)
        .into_iter()
        .filter(|(_, document)| {
            document["entries"].as_array().is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["value"]["chunk"].as_u64() >= Some(chunk_seq))
            })
        })
        .map(|(index, _)| {
            mush_core::chunk_key(&tacsim::arena_bridge::match_log_id(m.match_id), index)
        })
        .collect();
    assert!(!lost.is_empty());
    af3_void(
        &mut m,
        |image| {
            for key in &lost {
                tacsim_docs(image).remove(key);
            }
        },
        Reason::Prefix("the tape is behind the snapshot (last durable chunk "),
    )
    .await;

    // Another snapshot format.
    let mut m = af3_match_boxed(0, true).await;
    let key = match_::snapshot_key(m.match_id).to_string();
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                // 16b cut AI.1: against the constant, so the next bump
                // breaks neither arm.
                document["format"] =
                    serde_json::json!(tacsim::arena::snapshot::SNAPSHOT_FORMAT + 1);
            }
        },
        Reason::Exact(format!(
            "the snapshot does not decode: snapshot format {} is not {}",
            tacsim::arena::snapshot::SNAPSHOT_FORMAT + 1,
            tacsim::arena::snapshot::SNAPSHOT_FORMAT
        )),
    )
    .await;

    // Phase B: the ground the two documents name is not the one loaded — the
    // replay's own mechanism (a foreign digest recorded), on both documents so
    // phase A's agreement check passes.
    let mut m = af3_match_boxed(0, true).await;
    let (live_key, snapshot_key) = (
        match_::live_key(m.match_id).to_string(),
        match_::snapshot_key(m.match_id).to_string(),
    );
    let real = m.head.definition_digest.clone();
    let foreign = "0".repeat(64);
    let arena = m.head.arena.clone();
    let real_digest = real.clone();
    af3_void(
        &mut m,
        |image| {
            let docs = tacsim_docs(image);
            if let Some(document) = docs.get_mut(snapshot_key.as_str()) {
                document["arena_definition_digest"] = serde_json::json!(foreign.clone());
            }
            if let Some(document) = docs.get_mut(live_key.as_str()) {
                af3_restamp(document, |state| state.replace(&real, &foreign));
            }
        },
        Reason::Exact(format!(
            "the ground {arena} is not the ground loaded on this server (recorded {}, loaded \
             {})",
            "0".repeat(64),
            real_digest
        )),
    )
    .await;
}

/// **The ledger tail is restored before the first append** (16b cut AF.3; the
/// brief's test 4). Without it the registry's fresh ledger would open
/// `log:match.<id>` at chunk 0 again and overwrite it.
///
/// - The log's durable tail is an **unsealed** chunk `n` holding `k` entries —
///   here the real tail followed by one small abandoned chunk the old server
///   wrote after its snapshot, unsealed: the resume marker lands in chunk `n` as
///   entry `k`, and no earlier chunk is written. (A sealed tail — every real
///   16 KiB chunk — puts it in `n + 1`: test 1.)
/// - A breach in another, finished match's log (a chunk that is not a chunk)
///   voids nothing: the resume inspects its own log's documents alone.
/// - A breach in the resumed log voids with `the ledger tail could not be
///   restored`, and a tail that does not reach the snapshot's chunk voids with
///   `the tape is behind the snapshot`: test 2's arms, by the same helper.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn af3_the_ledger_tail_is_restored_before_the_first_append() {
    let mut m = af3_match(1, false).await;
    let log = tacsim::arena_bridge::match_log_id(m.match_id);
    let docs = af3_log_docs(&m.rig, m.match_id);
    let next = af3_next_chunk(&docs);
    let last_seq = docs
        .values()
        .next_back()
        .and_then(|document| document["entries"].as_array())
        .and_then(|entries| entries.last())
        .and_then(|entry| entry["seq"].as_u64())
        .expect("the tail's last entry");
    let abandoned = serde_json::json!({
        "chunk": next,
        "sealed": false,
        "entries": [{
            "seq": last_seq + 1,
            "value": { "chunk": m.head.chunk_seq + 5, "bytes": 0, "lines": "" },
        }],
    });
    let key = mush_core::chunk_key(&log, next);
    let foreign = mush_core::chunk_key("match.999", 0);
    let writes = m
        .rig
        .reboot_mid_match_editing(|image| {
            let docs = tacsim_docs(image);
            docs.insert(key.clone(), abandoned);
            docs.insert(foreign, serde_json::json!("not a chunk"));
        })
        .await;
    assert!(
        m.rig.registry().tasks().is_live(m.match_id),
        "another log's breach voided nothing"
    );
    let written = set_of(&writes, key.as_str())
        .unwrap_or_else(|| panic!("no write of {key}: {:?}", keys_of(&writes)));
    let entries = written["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 2, "the marker is entry 1 of chunk {next}");
    assert_eq!(entries[1]["seq"], serde_json::json!(last_seq + 2));
    assert_eq!(
        entries[1]["value"]["resumed"]["from_chunk"],
        serde_json::json!(m.head.chunk_seq)
    );
    assert_eq!(written["sealed"], false);
    for index in 0..next {
        let earlier = mush_core::chunk_key(&log, index).to_string();
        assert!(
            set_of(&writes, &earlier).is_none(),
            "{earlier} was rewritten"
        );
    }
    let said = m.rig.staff(&format!("@arena/abort {}", m.match_id.0));
    assert!(said.contains("is stopping"), "{said}");

    // The two voiding arms, as test 2 runs them.
    let mut m = af3_match(1, false).await;
    let last = af3_log_docs(&m.rig, m.match_id)
        .keys()
        .next_back()
        .copied()
        .expect("a chunk document");
    let key = mush_core::chunk_key(&tacsim::arena_bridge::match_log_id(m.match_id), last);
    af3_void(
        &mut m,
        |image| {
            tacsim_docs(image).insert(key, serde_json::json!("not a chunk"));
        },
        Reason::Prefix("the ledger tail could not be restored: "),
    )
    .await;
    let mut m = af3_match(1, false).await;
    let chunk_seq = m.head.chunk_seq;
    let log = tacsim::arena_bridge::match_log_id(m.match_id);
    let keys: Vec<smol_str::SmolStr> = af3_log_docs(&m.rig, m.match_id)
        .into_keys()
        .map(|index| mush_core::chunk_key(&log, index))
        .collect();
    af3_void(
        &mut m,
        |image| {
            for key in &keys {
                tacsim_docs(image).remove(key);
            }
        },
        Reason::Exact(format!(
            "the tape is behind the snapshot (last durable chunk 0, snapshot at chunk {})",
            chunk_seq
        )),
    )
    .await;
}

/// **The snapshot of `m`, decoded on its own ground, handed `controls` in one
/// pulse at its own moment (no arena time passes), and encoded again** — a
/// consistent document, its digest and its hash the changed arena's own: the
/// decode-encode twin the brief's test 5 builds its snapshots with.
fn af3_twin(m: &Af3Match, controls: &[tacsim::arena::intents::Control]) -> serde_json::Value {
    let document = m
        .rig
        .image_doc(&match_::snapshot_key(m.match_id))
        .expect("the snapshot");
    let ground = match_::definition_for(m.rig.tw.world(), &m.head.arena)
        .expect("the ground builds")
        .expect("and is loaded");
    let mut arena = tacsim::arena::Snapshot::decode(&document, ground).expect("it decodes");
    let _ = arena.resume_at(0);
    let _ = arena.step(0, controls, &[]);
    assert_eq!(arena.now_ms, m.head.at_ms, "no arena time passed");
    assert_eq!(arena.ledger.last_closed_seq(), m.head.chunk_seq);
    tacsim::arena::Snapshot::encode(&arena)
        .expect("the twin encodes")
        .document
}

/// The `Released` a seat's drop sends: every channel of `unit`, one epoch past
/// the snapshot's table, naming `pilot`.
fn af3_released(m: &Af3Match, pilot: ObjId, unit: ObjId) -> Vec<tacsim::arena::intents::Control> {
    let document = m
        .rig
        .image_doc(&match_::snapshot_key(m.match_id))
        .expect("the snapshot");
    let ground = match_::definition_for(m.rig.tw.world(), &m.head.arena)
        .expect("the ground builds")
        .expect("and is loaded");
    let arena = tacsim::arena::Snapshot::decode(&document, ground).expect("it decodes");
    tacsim::arena::state::Channel::ALL
        .into_iter()
        .map(|channel| tacsim::arena::intents::Control {
            unit,
            channel,
            epoch: arena.epochs.current(unit, channel) + 1,
            state: tacsim::arena::intents::ControlState::Released,
            pilot: Some(pilot),
        })
        .collect()
}

/// The live record of `m` with its centre's slots edited by `edit`.
fn af3_edit_slots(document: &mut serde_json::Value, edit: impl Fn(&mut serde_json::Value)) {
    af3_state_edit(document, |state| {
        if let Some(slots) = state["centre"]["slots"].as_array_mut() {
            for slot in slots {
                edit(slot);
            }
        }
    });
}

/// **A seat the world saw leave or reclaim after the snapshot is reconciled**
/// (16b cut AF.3; the brief's test 5): the world's slots and the arena's
/// roster disagree in both directions by up to one snapshot period, and the
/// resume settles each.
///
/// - **(a)** the live record has red's slot `left` (an `EXIT` the world handled
///   after the snapshot's pulse) while the snapshot still seats its pilot: the
///   resume releases the seat at the arena anyway — a watcher's first picture
///   has every tank on the reflex controller — and `BOARD` answers `LeftForGood`:
///   an `EXIT` is one-way.
/// - **(b)** the live record has red's slot seated and un-left (a reclaim the
///   world handled after the snapshot's pulse) while the snapshot does NOT hold
///   its pilot (the twin: the drop's `Released` taken at the snapshot's own
///   moment): after the boot the slot is dropped, no `AlreadyAboard`, and
///   `BOARD` after `reclaim_min_ms` takes the tank back.
/// - **(c)** every slot left and dropped before the snapshot: nothing marks a
///   slot at the boot, so the unattended clock is restamped at the boot's own
///   moment, and with nobody back the record parks `stopping` once
///   `unattended_ms` of world time has passed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn af3_a_seat_the_world_saw_leave_or_reclaim_after_the_snapshot_is_reconciled() {
    // (a) An `EXIT` after the snapshot's pulse.
    let mut m = af3_match(0, false).await;
    let red_pilot = m.rig.pilot(0);
    let key = match_::live_key(m.match_id).to_string();
    let _ = m
        .rig
        .reboot_mid_match_editing(|image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                af3_edit_slots(document, |slot| {
                    if slot["pilot"] == serde_json::json!(red_pilot.0) {
                        slot["left"] = serde_json::json!(true);
                    }
                });
            }
        })
        .await;
    let red = m
        .rig
        .centre()
        .slot_of_pilot(red_pilot)
        .cloned()
        .expect("red's slot");
    assert!(red.left && red.dropped_at_ms.is_none(), "{red:?}");
    assert!(!red.released_by_resume, "an EXIT is not the resume's");
    let said = m.rig.staff(&format!("@arena/watch {}", m.match_id.0));
    assert!(!said.is_empty(), "{said}");
    let mut reflex = false;
    for _ in 0..400 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        m.rig.pump();
        let _ = m.rig.pump_ops();
        if let Some(picture) = m
            .rig
            .registry()
            .last_observer(m.match_id)
            .filter(|picture| picture.now_ms > m.head.at_ms)
        {
            // 17b cut M4.3: a released seat is a mind's.
            reflex = picture
                .units
                .iter()
                .all(|unit| unit.controller == tacsim::arena::state::Controller::Mind);
            break;
        }
    }
    assert!(reflex, "every seat the snapshot held is a mind's");
    m.rig.tick_to(30);
    let said = m.rig.say(0, "board");
    assert!(
        said.contains("for the rest of the match; it cannot be taken back."),
        "{said}"
    );

    // (b) A reclaim after the snapshot's pulse.
    let mut m = af3_match(0, false).await;
    let red_pilot = m.rig.pilot(0);
    let red_tank = m
        .rig
        .centre()
        .slot_of_pilot(red_pilot)
        .and_then(|slot| slot.unit)
        .expect("red's tank");
    let twin = af3_twin(&m, &af3_released(&m, red_pilot, red_tank));
    let key = match_::snapshot_key(m.match_id).to_string();
    let _ = m
        .rig
        .reboot_mid_match_editing(|image| {
            tacsim_docs(image).insert(smol_str::SmolStr::new(&key), twin);
        })
        .await;
    assert!(m.rig.registry().tasks().is_live(m.match_id), "it resumed");
    let red = m
        .rig
        .centre()
        .slot_of_pilot(red_pilot)
        .cloned()
        .expect("red's slot");
    assert!(red.is_dropped() && red.released_by_resume, "{red:?}");
    m.rig.tick_to(3);
    let said = m.rig.say(0, "board");
    assert!(said.contains("is still being handed over"), "{said}");
    assert!(!said.contains("already in your pod"), "{said}");
    m.rig.tick_to(30);
    let said = m.rig.say(0, "board");
    assert!(said.contains("is yours again where it stands."), "{said}");

    // (c) Everybody gone before the snapshot.
    let mut m = af3_match(0, false).await;
    let mut controls = Vec::new();
    for slot in m.rig.centre().slots {
        let unit = slot.unit.expect("seated");
        controls.extend(af3_released(&m, slot.pilot, unit));
    }
    let twin = af3_twin(&m, &controls);
    let (live_key, snapshot_key) = (
        match_::live_key(m.match_id).to_string(),
        match_::snapshot_key(m.match_id).to_string(),
    );
    let _ = m
        .rig
        .reboot_mid_match_editing(|image| {
            let docs = tacsim_docs(image);
            docs.insert(smol_str::SmolStr::new(&snapshot_key), twin);
            if let Some(document) = docs.get_mut(live_key.as_str()) {
                af3_edit_slots(document, |slot| {
                    slot["left"] = serde_json::json!(true);
                    slot["dropped_at_ms"] = serde_json::json!(500);
                });
            }
        })
        .await;
    let centre = m.rig.centre();
    assert!(centre.slots.iter().all(|slot| slot.is_dropped()));
    assert!(
        centre.slots.iter().all(|slot| !slot.released_by_resume),
        "the resume released nobody"
    );
    assert_eq!(
        centre.unattended_since_ms,
        Some(1_000),
        "restamped at the boot's own moment"
    );
    let unattended_s = i64::from(tacsim::config::MatchConfig::default().unattended_ms) / 1_000;
    m.rig.tick_to(1 + unattended_s - 1);
    assert_eq!(
        m.rig.book().get(m.match_id).map(|record| record.state),
        Some(MatchState::Running),
        "not before its time"
    );
    m.rig.tick_to(2 + unattended_s);
    assert_eq!(
        m.rig.book().get(m.match_id).map(|record| record.state),
        Some(MatchState::Stopping),
        "a match nobody came back to parks"
    );
}

/// **The boot's tank reset keeps a resumed match's tanks, quietly, and resets
/// a match voided in phase B** (16b cut AF.3; the brief's test 14 — here and
/// not in `tests/reset.rs`, because the in-process reboot of a running match is
/// this file's rig).
///
/// After a resume: the boot said no `a roster tank is still deployed` line,
/// and the reset the boot ran, asked again with the resumed tanks kept, lists
/// nothing under `deployed` and changes nothing; asked with nothing kept it
/// lists both — the keep is what spared them. After a phase-B void: the tank
/// moved off its muster before the check-out is back on it (test 2's phase-B
/// arm asserts the same through `af3_void`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn af3_the_boot_reset_keeps_a_resumed_match_s_tanks_quietly_and_resets_a_voided_one() {
    let mut m = af3_match(0, false).await;
    let captured = Captured::default();
    {
        let _guard = tracing::subscriber::set_default(captured.clone());
        let _ = m.rig.reboot_mid_match().await;
    }
    assert!(m.rig.registry().tasks().is_live(m.match_id), "it resumed");
    assert_eq!(
        captured.said("a roster tank is still deployed at an Idle transition"),
        0,
        "a resumed tank is skipped without a word"
    );
    let keep: std::collections::BTreeSet<ObjId> = m.tanks.iter().copied().collect();
    let kept = m.rig.with_ctx(|ctx| match_::reset_stock_tanks(ctx, &keep));
    assert!(kept.deployed.is_empty(), "{kept:?}");
    assert!(!kept.changed(), "{kept:?}");
    let unkept = m
        .rig
        .with_ctx(|ctx| match_::reset_stock_tanks(ctx, &std::collections::BTreeSet::new()));
    let mut listed = unkept.deployed.clone();
    listed.sort_unstable();
    let mut tanks = m.tanks.clone();
    tanks.sort_unstable();
    assert_eq!(listed, tanks, "unkept, the resumed tanks are the fault");

    // A phase-B void: the ground moved under the documents.
    let mut m = af3_match(0, true).await;
    let (live_key, snapshot_key) = (
        match_::live_key(m.match_id).to_string(),
        match_::snapshot_key(m.match_id).to_string(),
    );
    let real = m.head.definition_digest.clone();
    let foreign = "0".repeat(64);
    af3_void(
        &mut m,
        |image| {
            let docs = tacsim_docs(image);
            if let Some(document) = docs.get_mut(snapshot_key.as_str()) {
                document["arena_definition_digest"] = serde_json::json!(foreign.clone());
            }
            if let Some(document) = docs.get_mut(live_key.as_str()) {
                af3_restamp(document, |state| state.replace(&real, &foreign));
            }
        },
        Reason::Prefix("the ground "),
    )
    .await;
}

// ---------------------------------------------------------------------------
// 16b cut AH.1 (16c N18) — the lobby and boarding hints, and `@tutorial`
// ---------------------------------------------------------------------------

use tacsim::match_::tutorial::{HINT_LINES, HINT_PREFIX, NO_ACCOUNT, SET_OFF, SET_ON, USAGE};

impl Rig {
    /// **16b cut AH.1**: [`Rig::boot_on`] with some pilots SHARING an account —
    /// for each `(a, b)`, pilot `b` is mapped to pilot `a`'s account, which is
    /// what an alt made with `@newchar` is — and GOD on an account of its own,
    /// so staff can walk the lobby as a person does.
    fn boot_sharing(
        pilots: usize,
        shared: &[(usize, usize)],
        modules: Vec<Box<dyn mush_core::GameModule>>,
    ) -> Rig {
        let mut rig = Rig::boot_on(pilots, modules);
        let mut map = Accounts::default();
        for n in 0..pilots {
            map.0.insert(rig.pilot(n), account(n));
        }
        for (a, b) in shared {
            map.0.insert(rig.pilot(*b), account(*a));
        }
        map.0
            .insert(GOD, AccountId::mint("boss", 0).expect("an account id"));
        *rig.tw
            .world_mut()
            .module_state_mut::<mush_sim::AccountOracle>()
            .expect("the simulation is composed") = mush_sim::AccountOracle::new(Arc::new(map));
        rig
    }

    /// **A staff `@teleport` of pilot `n` into the lobby** — a real
    /// `WorldEvent::Moved`, which the rig's own `move_to` placement is not — and
    /// everything pilot `n` saw of it.
    fn teleport_in(&mut self, n: usize) -> String {
        let pilot = self.pilot(n);
        let lobby = match_::centre_object(self.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
        let events = self
            .tw
            .run(GOD, &format!("@teleport #{}=#{}", pilot.0, lobby.0));
        self.seen(pilot, &events)
    }

    /// Out to Limbo and back in by staff: the lobby point's occasion, once.
    fn out_and_in(&mut self, n: usize) -> String {
        self.send_to_limbo(n);
        self.teleport_in(n)
    }

    /// **End the match from the lobby, as staff**: GOD walks in (a `move_to`,
    /// so no lobby point is spent on him) and types `SIM ABORT`, and the rig
    /// pumps until the centre is terminal.
    fn end_by_staff(&mut self) {
        let lobby = match_::centre_object(self.tw.world(), match_::LOBBY_ENTRY).expect("the lobby");
        self.tw
            .world_mut()
            .move_to(GOD, lobby)
            .expect("staff walk in");
        let said = self.staff("sim abort");
        assert!(said.contains("calls the match off"), "{said}");
        for _ in 0..50 {
            if self.state().is_terminal() {
                return;
            }
            self.pump();
        }
        assert!(
            self.state().is_terminal(),
            "the match ended: {:?}",
            self.state()
        );
    }
}

/// Every `Hint:` line one pilot read in one step's output.
fn hints_in(said: &str) -> Vec<&str> {
    said.lines()
        .map(str::trim)
        .filter(|line| line.starts_with(HINT_PREFIX))
        .collect()
}

/// **16b cut AH.1**: the lobby hint is read once, on a real entry, the boarding
/// hint once at `SIM START`, and an alt on the same account reads neither.
///
/// The `@tutorial` counts read `of 6` (were `of 5` at 16b cut AH.1): the table
/// holds six since `HintPoint::Mind`, said at a match's end (17b cut M5.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ah1_the_lobby_and_boarding_hints_are_read_once_per_account_and_an_alt_reads_none() {
    let mut rig = Rig::boot_sharing(3, &[(1, 2)], roster_with(fill_config()));
    let said = rig.out_and_in(1);
    assert_eq!(hints_in(&said), vec![HINT_LINES[0]], "{said}");
    let said = rig.out_and_in(1);
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "once: {said}");
    let said = rig.say(1, "sim team red");
    assert!(said.contains("red slot 1"), "{said}");
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "{said}");
    let said = rig.say(1, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert_eq!(hints_in(&said), vec![HINT_LINES[1]], "{said}");
    rig.end_by_staff();

    // Pilot 2 is pilot 1's alt: the same person, who has read both.
    let said = rig.out_and_in(2);
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "an alt: {said}");
    let said = rig.say(2, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 2 of 7."),
        "{said}"
    );
    // Pilot 0 is somebody else, and has read nothing.
    let said = rig.say(0, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 0 of 7."),
        "{said}"
    );
}

/// **16b cut AH.1**: `off` silences the points not yet reached without
/// spending them, and `on` starts them again without re-arming one read.
///
/// The `@tutorial` counts read `of 6` (were `of 5` at 16b cut AH.1): the table
/// holds six since `HintPoint::Mind`, said at a match's end (17b cut M5.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ah1_off_silences_the_unreached_and_on_does_not_rearm_the_read() {
    let mut rig = Rig::boot_sharing(2, &[], roster_with(fill_config()));
    let said = rig.out_and_in(1);
    assert_eq!(hints_in(&said), vec![HINT_LINES[0]], "{said}");
    let said = rig.say(1, "@tutorial off");
    assert!(said.contains(SET_OFF), "{said}");
    let said = rig.say(1, "@tutorial OFF");
    assert!(
        said.contains(SET_OFF),
        "a switch that changes nothing still answers: {said}"
    );
    assert_eq!(hints_in(&rig.out_and_in(1)), Vec::<&str>::new());
    let said = rig.say(1, "sim team red");
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "{said}");
    let said = rig.say(1, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "off: {said}");
    rig.end_by_staff();

    let said = rig.say(1, "@tutorial on");
    assert!(said.contains(SET_ON), "{said}");
    assert_eq!(
        hints_in(&rig.out_and_in(1)),
        Vec::<&str>::new(),
        "read once stays read"
    );
    let said = rig.say(1, "sim team red");
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "{said}");
    let said = rig.say(1, "sim start");
    assert_eq!(
        hints_in(&said),
        vec![HINT_LINES[1]],
        "a point refused by off was not spent: {said}"
    );
    let said = rig.say(1, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 2 of 7."),
        "{said}"
    );
    let said = rig.say(1, "@tutorial sideways");
    assert!(said.contains(USAGE), "{said}");
    rig.end_by_staff();
}

/// **16b cut AH.1**: the lobby point waits for a centre that would take a
/// claim, for an account that holds no slot, and for a live layer.
///
/// The `@tutorial` counts read `of 6` (were `of 5` at 16b cut AH.1): the table
/// holds six since `HintPoint::Mind`, said at a match's end (17b cut M5.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ah1_the_lobby_hint_waits_for_a_claimable_centre_a_free_account_and_a_live_layer() {
    // (i) The centre: boarding takes no claim, a terminal centre does.
    let mut rig = Rig::boot_sharing(3, &[], roster_with(fill_config()));
    rig.say(1, "sim team red");
    let said = rig.say(1, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert_eq!(rig.state(), MatchState::Boarding);
    let said = rig.out_and_in(2);
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "boarding: {said}");
    let said = rig.say(2, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 0 of 7."),
        "{said}"
    );
    rig.end_by_staff();
    let said = rig.out_and_in(2);
    assert_eq!(hints_in(&said), vec![HINT_LINES[0]], "terminal: {said}");

    // (ii) The claim: a slot holder walking back in is not told to claim.
    let mut rig = Rig::boot_sharing(2, &[], roster_with(fill_config()));
    let said = rig.say(1, "sim team red");
    assert!(said.contains("red slot 1"), "{said}");
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "{said}");
    let said = rig.out_and_in(1);
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "holds a slot: {said}");
    let said = rig.say(1, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 0 of 7."),
        "{said}"
    );
    let said = rig.say(1, "sim leave");
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "{said}");
    let said = rig.out_and_in(1);
    assert_eq!(
        hints_in(&said),
        vec![HINT_LINES[0]],
        "after SIM LEAVE: {said}"
    );

    // (iii) The layer: off, nothing is said, and the verb still answers.
    let mut rig = Rig::boot_on(1, roster_with_the_layer_off());
    let said = rig.out_and_in(0);
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "layer off: {said}");
    let said = rig.say(0, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 0 of 7."),
        "{said}"
    );
}

/// **16b cut AH.1**: a character the account oracle does not map reads no
/// hint, and the verb says why.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ah1_an_unmapped_character_reads_no_hint() {
    let mut rig = Rig::boot(1);
    let stray = rig.tw.spawn_player("Stray");
    rig.pilots.push(stray);
    let said = rig.out_and_in(1);
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "{said}");
    let said = rig.say(1, "@tutorial");
    assert!(said.contains(NO_ACCOUNT), "{said}");
    let said = rig.say(1, "@tutorial off");
    assert!(said.contains(NO_ACCOUNT), "{said}");
}

/// **16b cut AH.1**: a hint waits for a live connection. A pilot with none —
/// linkdead, still `is_connected` — is neither told nor marked, and reads the
/// point at its next occasion with a connection back.
///
/// The `@tutorial` counts read `of 6` (were `of 5` at 16b cut AH.1): the table
/// holds six since `HintPoint::Mind`, said at a match's end (17b cut M5.6).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ah1_a_hint_waits_for_a_live_connection() {
    let mut rig = Rig::boot(2);
    let p1 = rig.pilot(1);
    assert!(
        rig.tw.world().has_live_conns(p1),
        "a spawned player is attached"
    );
    rig.tw.world_mut().set_live_conns(p1, 0);
    assert!(rig.tw.world().is_connected(p1), "the linkdead shape");
    let said = rig.out_and_in(1);
    assert_eq!(hints_in(&said), Vec::<&str>::new(), "{said}");
    let said = rig.say(1, "@tutorial");
    assert!(
        said.contains("Hints are on; you have read 0 of 7."),
        "{said}"
    );
    rig.tw.world_mut().set_live_conns(p1, 1);
    let said = rig.out_and_in(1);
    assert_eq!(hints_in(&said), vec![HINT_LINES[0]], "{said}");
}

// ---------------------------------------------------------------------------
// 17b cut M3.1 — a live world takes the cores at the next reset
// ---------------------------------------------------------------------------

/// **A roster tank without a core is refitted with one at the reset** (17b
/// cut M3.1; doc 17 §9, D-UM-33; 16b cut Q.8's refit rule). A live world
/// whose roster tank was made before the cores — the line tank of its
/// version 2, hand-rolled here: the bound spec with the core component and
/// the `tacsim.core` capability taken out, under a version-2 definition — is
/// put back to the CURRENT definition on the way into `Idle` and at boot by
/// `match_::reset_stock_tanks`: the schema is not the loaded binding's, so the
/// tank is refitted (`ResetReport::refitted`), and afterwards it points at the
/// version-3 definition and carries the line core (Class II, governed), read
/// live off the unit. A second reset changes nothing. (**17b cut M3.6**: the
/// loaded definition is version 4 now — the turret's `destroy_core` — and the
/// refit is to it; was "the version-3 definition".)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_a_roster_tank_without_a_core_is_refitted_with_one_at_the_reset() {
    use tacsim::arena::core::{CoreClass, CoreHardware, read_core};
    let mut rig = Rig::boot(1);
    let tank = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let binding = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .expect("the tacsim store")
        .definitions()
        .bindings
        .get(&mush_core::CanonicalId::new(tacsim::definitions::content::LINE_TANK).expect("an id"))
        .expect("the line tank is loaded")
        .clone();
    assert_eq!(
        binding.spec.definition.version(),
        4,
        "the loaded line tank is the version-4 definition"
    );

    // The version-2 tank: no core component, no core capability.
    let mut stale = binding.spec.clone();
    stale.definition = mush_tactics::DefinitionRef::new(
        binding.spec.definition.key().clone(),
        2,
        mush_core::DefinitionDigest::from_bytes([2_u8; 32]),
    )
    .expect("version 2 is not zero");
    stale
        .components
        .retain(|component| component.id.as_str() != "core");
    stale
        .capabilities
        .retain(|init| init.key.id.as_str() != tacsim::definitions::unit::CORE_CAPABILITY);
    let old = mush_tactics::TacticalUnit::from_spec(&stale, tank).expect("the old tank");
    assert_eq!(read_core(&old), None, "the version-2 tank had no core");
    rig.tw
        .world_mut()
        .module_state_mut::<mush_tactics::module::UnitStore>()
        .expect("the tactics store")
        .units
        .insert(tank, old);

    let report =
        rig.with_ctx(|ctx| match_::reset_stock_tanks(ctx, &std::collections::BTreeSet::new()));
    assert!(
        report.refitted >= 1,
        "the stale tank was refitted: {report:?}"
    );
    assert!(report.failed.is_empty(), "{report:?}");
    let now = mush_tactics::module::unit_of(rig.tw.world(), tank).expect("a unit");
    assert_eq!(
        now.schema(),
        &binding.spec.definition,
        "the current definition"
    );
    assert_eq!(
        read_core(now),
        Some(CoreHardware {
            component: smol_str::SmolStr::new("core"),
            class: CoreClass::II,
            governed: true,
        }),
        "the refitted tank carries the line core"
    );
    let again =
        rig.with_ctx(|ctx| match_::reset_stock_tanks(ctx, &std::collections::BTreeSet::new()));
    assert_eq!(again.refitted, 0, "{again:?}");
    assert!(
        !again.changed(),
        "a second reset changes nothing: {again:?}"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M3.3 — the seal and the record (doc 17 §12, §13; D-UM-45, D-UM-46)
// ---------------------------------------------------------------------------

/// The engine ABI the resume refusals rewrite a seal to: the rhai version
/// before this build's.
const FOREIGN_ABI: &str = "rhai=1.25.0;api=1;stock=1";

/// **`start` seals every unit and the record keeps one seal per unit** (17b
/// cut M3.3; doc 17 §13 "Where it goes", D-UM-46), through the real lobby with
/// the AI fill on: one red pilot, two a side. The pilot's hull's warrant
/// principal is the pilot; red's fill hull's is red's side lead (the same
/// pilot — the character who set what the fill flies); blue has no claimed
/// slot and so no lead, and its two fill hulls name nobody. Every seal is the
/// stock one (the reflex engine, `MIND_ABI`, the stock source, the Line card
/// at the configured floor, the sortie `in_match(match, unit)`), and the
/// live record the check-out wrote carries the same map. A staff
/// `@arena/start` match seals its unseated hulls with no principal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_start_writes_one_seal_per_unit_and_the_live_record_carries_them() {
    use tacsim::arena::state::Channel;
    use tacsim::arena::{CoreClass, SortieId};
    use tacsim::deliberation::CharacterRef;
    let mut rig = fill_rig(1);
    rig.tick_to(1);
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim ai 2");
    assert!(said.contains("AI fill set to 2 a side."), "{said}");
    rig.say(0, "sim start");
    let (said, writes) = rig.board_writes(0, match_::RED_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    assert_eq!(rig.state(), MatchState::Running);
    let match_id = rig.centre().match_id.expect("a running match");
    let pilot = rig.pilot(0);
    let record = rig.book().get(match_id).cloned().expect("the record");
    let units = record.units();
    assert_eq!(units.len(), 4, "two a side");
    assert_eq!(
        record.minds.keys().copied().collect::<Vec<ObjId>>(),
        {
            let mut sorted = units.clone();
            sorted.sort_unstable();
            sorted
        },
        "one seal per unit, keyed by unit"
    );
    let piloted = record
        .teams
        .iter()
        .flat_map(|side| side.pilots.iter())
        .map(|(_, unit)| *unit)
        .collect::<Vec<ObjId>>();
    assert_eq!(piloted.len(), 1);
    for side in &record.teams {
        for unit in &side.units {
            let sealed = record.minds.get(unit).expect("a seal");
            let seal = &sealed.seal;
            assert_eq!(sealed.sortie, seal.sortie);
            assert_eq!(seal.sortie, SortieId::in_match(match_id.0, *unit));
            assert_eq!(seal.unit, *unit);
            // 17b cut M4.3: every hull on the production fill's engine.
            assert_eq!(
                seal.engine,
                tacsim::match_::PRODUCTION_FILL_ENGINE,
                "#{}",
                unit.0
            );
            assert_eq!(seal.abi.as_str(), tacsim::minds::MIND_ABI);
            assert_eq!(
                tacsim::arena::digest_text(&seal.source_sha256),
                tacsim::minds::content::STANDARD_SHA256
            );
            assert_eq!(seal.odds_floor_permille, 100);
            assert_eq!(seal.mind.as_str(), "Standard Doctrine");
            assert_eq!(seal.origin.licence_label(), "doctrine");
            assert_eq!(seal.core_class, CoreClass::II, "a line tank's core");
            assert_eq!(seal.class, CoreClass::I);
            assert!(seal.channels.contains(&Channel::Fire));
            let principal = match side.team {
                PartyId::RED => Some(CharacterRef::Character { id: pilot }),
                _ => None,
            };
            assert_eq!(
                seal.principal,
                principal,
                "#{} on {:?} (piloted: {})",
                unit.0,
                side.team,
                piloted.contains(unit)
            );
        }
    }
    // The live record the check-out wrote carries the map, whole.
    let live = set_of(&writes, &match_::live_key(match_id))
        .map(|document| match_::LiveRecord::from_document(&document))
        .expect("the check-out wrote the live record")
        .expect("and it decodes");
    assert_eq!(live.record.minds, record.minds);
    let said = rig.staff(&format!("@arena/abort {}", match_id.0));
    assert!(said.contains("is stopping"), "{said}");

    // A staff `@arena/start`: nobody chose the unseated hulls' mind.
    let mut rig = Rig::boot(1);
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1).expect("blue 1's tank");
    let said = rig.staff(&format!(
        "@arena/start {}=#{}/#{}",
        match_::CENTRE_ARENA,
        red.0,
        blue.0
    ));
    assert!(said.contains("is running on"), "{said}");
    let match_id = rig.centre().match_id.expect("a running match");
    let record = rig.book().get(match_id).cloned().expect("the record");
    assert_eq!(record.minds.len(), 2);
    for unit in [red, blue] {
        let seal = &record.minds.get(&unit).expect("a seal").seal;
        assert_eq!(seal.principal, None, "a staff match tells nobody");
        assert_eq!(seal.sortie, SortieId::in_match(match_id.0, unit));
    }
    rig.staff("@arena/abort");
}

/// **A second pilot's hull names its own pilot, and the fill names the lead**
/// (17b cut M3.3; doc 17 §13, D-UM-46): two humans on red — pilot 0 claims
/// first and so leads the side, pilot 1 takes the second slot — and the AI
/// fill at three a side, so red fields one machine beside them. Each piloted
/// hull's principal is ITS OWN seated pilot (never the side's lead), red's
/// fill hull's is the lead, and blue, which nobody claimed, names nobody.
/// This is the case that tells the two rules of `start` apart: with one
/// human a side the lead and the pilot are one character.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_a_second_pilots_hull_names_its_own_pilot_and_the_fill_names_the_lead() {
    use tacsim::deliberation::CharacterRef;
    let mut rig = fill_rig(2);
    rig.tick_to(1);
    let said = rig.say(0, "sim team red");
    assert!(said.contains("red slot 1"), "{said}");
    let said = rig.say(1, "sim team red");
    assert!(said.contains("red slot 2"), "{said}");
    let said = rig.say(0, "sim ai 3");
    assert!(said.contains("AI fill set to 3 a side."), "{said}");
    let said = rig.say(0, "sim start");
    assert!(said.contains("boarding"), "{said}");
    // The real `BOARD`, which seats each pilot on its hull's stations (the
    // `seat` stand-in fires the trigger without seating anyone).
    let (said, _) = rig.board_writes(0, match_::RED_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    assert_eq!(
        rig.state(),
        MatchState::Boarding,
        "one of two is not every one"
    );
    let (said, _) = rig.board_writes(1, match_::RED_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    assert_eq!(rig.state(), MatchState::Running);
    let match_id = rig.centre().match_id.expect("a running match");
    let (lead, second) = (rig.pilot(0), rig.pilot(1));
    let record = rig.book().get(match_id).cloned().expect("the record");
    let hull_of = |pilot: ObjId| {
        record
            .teams
            .iter()
            .flat_map(|side| side.pilots.iter())
            .find(|(seated, _)| *seated == pilot)
            .map(|(_, unit)| *unit)
            .expect("the pilot flies a hull")
    };
    let (lead_hull, second_hull) = (hull_of(lead), hull_of(second));
    assert_ne!(lead_hull, second_hull);
    let principal_of = |unit: ObjId| {
        record
            .minds
            .get(&unit)
            .map(|sealed| sealed.seal.principal.clone())
            .expect("a seal")
    };
    assert_eq!(
        principal_of(second_hull),
        Some(CharacterRef::Character { id: second }),
        "the second pilot's hull names the second pilot, not the lead"
    );
    assert_eq!(
        principal_of(lead_hull),
        Some(CharacterRef::Character { id: lead })
    );
    let mut fills = 0_usize;
    for side in &record.teams {
        for unit in &side.units {
            if *unit == lead_hull || *unit == second_hull {
                continue;
            }
            fills += 1;
            let expected = match side.team {
                PartyId::RED => Some(CharacterRef::Character { id: lead }),
                _ => None,
            };
            assert_eq!(
                principal_of(*unit),
                expected,
                "fill #{} on {:?}",
                unit.0,
                side.team
            );
        }
    }
    assert_eq!(fills, 4, "one red fill hull and three blue");
    let said = rig.staff(&format!("@arena/abort {}", match_id.0));
    assert!(said.contains("is stopping"), "{said}");
}

/// **A live record written before 17b cut M3.3 decodes with no seals, and
/// the boot voids it by name over a scripted seal** (`#[serde(default)]`
/// under `LIVE_FORMAT` 1, the `leaders` precedent; 17b cut M4.3, re-pointed
/// by the fill decision of 2026-10-09): the running match's live record with
/// its `minds` key taken out of the state reads back an empty map. The seal
/// checks have nothing to refuse, but the snapshot's units carry the seal
/// references the check-out wrote — rewritten here to the `scripted` engine
/// on every unit, as every production check-out wrote them from the flip to
/// the fill decision (EXPLICITLY since the decision, whose production seal is
/// `native`) — and a record that names no seal names no source for them, so
/// the installer refuses the first unit by name and the match is voided
/// `aborted(restart)` with that sentence. **The production snapshot as the
/// check-out now writes it** — every reference on `PRODUCTION_FILL_ENGINE`,
/// `native` — under the same seal-less record resumes: a native reference
/// runs compiled code and names no source, so nothing is left to refuse.
/// (Before the flip every production seal was `reflex` and such a record
/// resumed; a snapshot older than M3.2's format 8 is refused at its head
/// either way.)
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m4_3_a_pre_m3_live_record_decodes_with_no_seals_and_is_voided_by_name() {
    // Scripted references, no record to name their source: voided by name.
    let mut m = af3_match(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    let snapshot_key = match_::snapshot_key(m.match_id).to_string();
    let mut document = m.rig.image_doc(&key).expect("the live record");
    af3_state_edit(&mut document, |state| {
        let removed = state["record"]
            .as_object_mut()
            .and_then(|record| record.remove("minds"));
        assert!(removed.is_some_and(|minds| minds.as_object().is_some_and(|m| m.len() == 2)));
    });
    let live = match_::LiveRecord::from_document(&document).expect("a pre-M3 record decodes");
    assert!(live.record.minds.is_empty());
    let older = document.clone();
    af3_void(
        &mut m,
        |image| {
            tacsim_docs(image).insert(smol_str::SmolStr::new(&key), older);
            if let Some(snapshot) = tacsim_docs(image).get_mut(snapshot_key.as_str()) {
                af3_state_edit(snapshot, |state| {
                    let rows = state["units"].as_array_mut().expect("the snapshot's units");
                    assert_eq!(rows.len(), 2);
                    for row in rows {
                        assert_eq!(
                            row["seal"]["engine"],
                            serde_json::json!(tacsim::match_::PRODUCTION_FILL_ENGINE.as_str()),
                            "the check-out's own reference, before the rewrite"
                        );
                        row["seal"]["engine"] = serde_json::json!("scripted");
                    }
                });
            }
        },
        Reason::Prefix("the arena could not be opened: a sealed mind cannot be run: #"),
    )
    .await;

    // The production's own references (native), the same seal-less record:
    // resumes.
    let mut rig = af3_rig(2);
    let match_id = af3_running(&mut rig);
    let _ = af3_snapshot_after(&mut rig, match_id, 0).await;
    let key = match_::live_key(match_id).to_string();
    let snapshot_key = match_::snapshot_key(match_id).to_string();
    let state: serde_json::Value = rig
        .image_doc(&snapshot_key)
        .and_then(|document| document.get("state").cloned())
        .and_then(|state| state.as_str().map(serde_json::from_str))
        .expect("a snapshot")
        .expect("a JSON state");
    let rows = state["units"].as_array().expect("the snapshot's units");
    assert_eq!(rows.len(), 2);
    for row in rows {
        assert_eq!(
            row["seal"]["engine"],
            serde_json::json!("native"),
            "the check-out's own reference, untouched"
        );
    }
    let writes = rig
        .reboot_mid_match_editing(|image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                af3_state_edit(document, |state| {
                    let removed = state["record"]
                        .as_object_mut()
                        .and_then(|record| record.remove("minds"));
                    assert!(
                        removed
                            .is_some_and(|minds| minds.as_object().is_some_and(|m| m.len() == 2))
                    );
                });
            }
        })
        .await;
    assert!(rig.registry().tasks().is_live(match_id), "it resumed");
    assert!(set_of(&writes, &match_::result_key(match_id)).is_none());
    let said = rig.staff(&format!("@arena/abort {}", match_id.0));
    assert!(said.contains("is stopping"), "{said}");
}

/// **A floor the Line card sits under refuses the check-out by name, before
/// any write** (17b cut M3.3; D-UM-17, refuse-not-clamp): with
/// `[tacsim.minds].odds_floor_permille = 200` on the world's config, the
/// lobby's `Boarding → Running` trigger is refused with
/// `StartError::CardUnlawful`'s sentence naming red's first pod (`RA`), the
/// pods are freed, and nothing was written: no record on the book, no live
/// record, no `deployed` marker. A staff `@arena/start` is refused the same
/// way, naming the tank by its world name (it has no pod).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_a_card_under_the_floor_refuses_the_check_out_by_name_before_any_write() {
    let mut cfg = config();
    cfg.minds.odds_floor_permille = 200;
    let mut rig = Rig::boot_on(2, roster_with(cfg.clone()));
    rig.tick_to(1);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    let match_id = rig.centre().match_id.expect("a match in boarding");
    let (said, _) = rig.board_writes(0, match_::RED_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    let (said, writes) = rig.board_writes(1, match_::BLUE_ROOM_ENTRY);
    // The boarding pilot is answered by `SimError::Start` over the check-out's
    // own sentence, byte for byte.
    assert!(
        said.contains(
            "The match could not start: RA's card asks for odds under the Compact floor of \
             20%; the match cannot start."
        ),
        "{said}"
    );
    // The call-off left the centre `Aborted` with every slot and pod free.
    assert_eq!(rig.state(), MatchState::Aborted);
    assert!(rig.centre().slots.is_empty(), "the slots are given back");
    for (n, team) in [(0, PartyId::RED), (1, PartyId::BLUE)] {
        let pod = match_::pod_object(rig.tw.world(), team, 1).expect("pod 1");
        assert_eq!(
            match_::pilot_of(rig.tw.world(), pod),
            None,
            "{team:?} pod 1"
        );
        assert!(
            !tacsim::match_::cockpit::in_a_pod(rig.tw.world(), rig.pilot(n)),
            "pilot {n} is out of the pod"
        );
    }
    assert!(rig.book().get(match_id).is_none(), "no record was inserted");
    assert!(
        set_of(&writes, &match_::live_key(match_id)).is_none(),
        "no live record: {:?}",
        keys_of(&writes)
    );
    for team in [PartyId::RED, PartyId::BLUE] {
        let tank = match_::tank_for_slot(rig.tw.world(), team, 1).expect("a tank");
        assert!(mush_tactics::custody::marker_of(rig.tw.world(), tank).is_none());
        assert!(!mush_sim::deployed::is_deployed(rig.tw.world(), tank));
    }

    // The centre's call-off line, read off the seat funnel itself (the
    // `ready` call-off tests' way: `board`'s refusal hands the pilot the
    // error alone): the check-out's sentence once, with its own full stop
    // (`lobby::run` trims it, so the line never reads `start.. The pods`).
    let mut rig = Rig::boot_on(2, roster_with(cfg.clone()));
    rig.tick_to(1);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    rig.say(0, "sim start");
    let match_id = rig.centre().match_id.expect("a match in boarding");
    let (said, _) = rig.board_writes(0, match_::RED_ROOM_ENTRY);
    assert!(said.contains("climb into"), "{said}");
    let pilot = rig.pilot(1);
    let unit =
        match_::tank_of(rig.tw.world(), rig.pod(1).expect("a locked pod")).expect("the pod's tank");
    let heard = {
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
                "the floor refuses the check-out"
            );
        }
        rig.seen(pilot, &out.take())
    };
    assert!(
        heard.contains(&format!(
            "Match {match_id} could not start: RA's card asks for odds under the Compact \
             floor of 20%; the match cannot start. The pods are free again."
        )),
        "{heard}"
    );
    assert_eq!(rig.state(), MatchState::Aborted);
    assert!(rig.centre().slots.is_empty(), "the slots are given back");

    // The staff shortcut, on a fresh centre under the same floor.
    let mut rig = Rig::boot_on(1, roster_with(cfg));
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let blue = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1).expect("blue 1's tank");
    let name = mush_core::output::display_name(rig.tw.world(), red);
    let said = rig.staff(&format!(
        "@arena/start {}=#{}/#{}",
        match_::CENTRE_ARENA,
        red.0,
        blue.0
    ));
    assert!(
        said.contains(&format!(
            "{name}'s card asks for odds under the Compact floor of 20%; the match cannot start."
        )),
        "{said}"
    );
    assert!(!mush_sim::deployed::is_deployed(rig.tw.world(), red));
    assert!(!mush_sim::deployed::is_deployed(rig.tw.world(), blue));
    assert_ne!(rig.state(), MatchState::Running);
}

/// **The stock mind's source is stored once, immutably** (17b cut M3.3; doc
/// 17 §13: *sources live immutably at `tacsim:mindsrc:<sha256>`*): the first
/// boot writes `{sha256, abi, bytes}` under the script's digest and the store
/// answers the bytes; a second boot over that document writes nothing and
/// still answers; a boot over a tampered document (its bytes edited, or its
/// `sha256` not its key's) skips it loudly, does NOT rewrite it, and answers
/// nothing for the digest.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_the_stock_source_is_stored_once_and_never_rewritten() {
    let digest =
        tacsim::arena::digest_from_text(tacsim::minds::content::STANDARD_SHA256).expect("pinned");
    let key = format!("tacsim:mindsrc:{}", tacsim::minds::content::STANDARD_SHA256);
    let mut rig = Rig::boot(0);
    let first = writes_of(rig.pump_ops());
    let document = set_of(&first, &key)
        .unwrap_or_else(|| panic!("the first boot stores the source: {:?}", keys_of(&first)));
    assert_eq!(
        document,
        serde_json::json!({
            "sha256": tacsim::minds::content::STANDARD_SHA256,
            "abi": tacsim::minds::MIND_ABI,
            "bytes": tacsim::minds::content::STANDARD_RHAI,
        })
    );
    let held = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .and_then(|store| store.mind_source(&digest).map(str::to_owned))
    };
    assert_eq!(
        held(&rig).as_deref(),
        Some(tacsim::minds::content::STANDARD_RHAI)
    );

    // A second boot over it: nothing written, the same bytes held.
    let second = rig.reboot_mid_match().await;
    assert!(set_of(&second, &key).is_none(), "{:?}", keys_of(&second));
    assert_eq!(
        held(&rig).as_deref(),
        Some(tacsim::minds::content::STANDARD_RHAI)
    );
    assert!(
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .is_some_and(|store| store.has_mind_source(&digest))
    );

    // Tampered: its bytes edited, then its `sha256` not its key's.
    for tamper in [
        serde_json::json!({
            "sha256": tacsim::minds::content::STANDARD_SHA256,
            "abi": tacsim::minds::MIND_ABI,
            "bytes": "// not the stock mind",
        }),
        serde_json::json!({
            "sha256": "0".repeat(64),
            "abi": tacsim::minds::MIND_ABI,
            "bytes": tacsim::minds::content::STANDARD_RHAI,
        }),
    ] {
        let captured = Captured::default();
        let writes = {
            let _guard = tracing::subscriber::set_default(captured.clone());
            let doc = tamper.clone();
            let key = key.clone();
            rig.reboot_mid_match_editing(move |image| {
                tacsim_docs(image).insert(smol_str::SmolStr::new(&key), doc);
            })
            .await
        };
        assert!(
            set_of(&writes, &key).is_none(),
            "a tampered source is never rewritten: {:?}",
            keys_of(&writes)
        );
        assert_eq!(held(&rig), None, "{tamper}");
        assert_eq!(
            captured.said("a mind source's bytes do not hash to its key")
                + captured.said("a mind source disagrees with its own key"),
            1,
            "skipped loudly: {tamper}"
        );
    }
}

/// **A resume across another `MIND_ABI` is refused by name and voided — on a
/// reflex seal too** (17b cut M3.3; doc 17 §13, D-UM-23's R-B clause, D-16-3):
/// a lobby match of two pilots (both hulls sealed on the production fill's
/// engine, 17b cut M4.3), its live record's seals rewritten to
/// [`FOREIGN_ABI`], is voided by the boot with `ResumeRefusal::MindAbi`'s
/// sentence naming the first unit — the void of every other reason, with no
/// tape line written for it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_a_resume_across_another_abi_is_refused_by_name_on_a_production_seal() {
    let mut m = af3_match(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    let record = m
        .rig
        .book()
        .get(m.match_id)
        .cloned()
        .expect("the running record");
    let first = *record.minds.keys().next().expect("a seal");
    assert!(
        record
            .minds
            .values()
            .all(|sealed| sealed.seal.engine == tacsim::match_::PRODUCTION_FILL_ENGINE),
        "every hull of a two-pilot match is sealed on the production engine"
    );
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                af3_state_edit(document, |state| {
                    if let Some(minds) = state["record"]["minds"].as_object_mut() {
                        for sealed in minds.values_mut() {
                            sealed["seal"]["abi"] = serde_json::json!(FOREIGN_ABI);
                        }
                    }
                });
            }
        },
        Reason::Exact(format!(
            "#{}'s mind was sealed on engine {FOREIGN_ABI} and this server runs {}",
            first.0,
            tacsim::minds::MIND_ABI
        )),
    )
    .await;
}

/// **A scripted seal whose source the store lacks is refused by name and
/// voided** (17b cut M3.3; doc 17 §13): one seal of the live record rewritten
/// to the `scripted` engine over a digest no `tacsim:mindsrc:` document holds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_a_scripted_seal_whose_source_is_missing_is_refused_by_name() {
    let mut m = af3_match(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    let first = *m
        .rig
        .book()
        .get(m.match_id)
        .expect("the running record")
        .minds
        .keys()
        .next()
        .expect("a seal");
    let missing = "0".repeat(64);
    let written = missing.clone();
    af3_void(
        &mut m,
        |image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                af3_state_edit(document, |state| {
                    let sealed = &mut state["record"]["minds"][first.0.to_string()]["seal"];
                    sealed["engine"] = serde_json::json!("scripted");
                    sealed["source_sha256"] = serde_json::json!(written);
                });
            }
        },
        // 17b cut M4.3: the stock source is compiled in, so the refusal
        // names both places a source may be.
        Reason::Exact(format!(
            "#{}'s mind source {missing} is neither this server's Standard Doctrine nor in its \
             store",
            first.0
        )),
    )
    .await;
}

/// **Only a scripted seal needs its source; since the fill decision the
/// production seal is not one** (17b cut M3.3 as 17b cut M4.3 re-pointed it,
/// and as the fill decision of 2026-10-09 re-points it again; doc 17 §13).
/// Every seal of the live record rewritten to the `scripted` engine over a
/// digest neither Standard Doctrine's nor any `tacsim:mindsrc:` document's,
/// and each unit's snapshot reference to that rewritten seal's own (the
/// `scripted` engine and its digest; [`af3_reencode`]) — the shape every
/// production check-out wrote, in both places, from the flip to the
/// decision, written EXPLICITLY since: the boot voids the match by name on
/// the first unit, before any tape line. The record as the production check-out
/// now writes it — every engine `native` (`PRODUCTION_FILL_ENGINE`) in the
/// record's seals and in the snapshot's seal reference on each unit alike,
/// untouched — with every source rewritten to that missing digest resumes:
/// the native and reflex engines run compiled code and never read a source,
/// so a store that lost (or never had) a document voids no match that does
/// not run one.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m4_3_a_scripted_seal_missing_its_source_is_refused_and_a_native_one_resumes() {
    assert_eq!(
        tacsim::match_::PRODUCTION_FILL_ENGINE,
        tacsim::arena::SealEngine::Native,
        "the production fill runs no source (the fill decision, 2026-10-09); re-read this test \
         if it moves"
    );
    let missing = "0".repeat(64);

    // Scripted, source missing: voided by name.
    let mut m = af3_match(0, true).await;
    let key = match_::live_key(m.match_id).to_string();
    let record = m
        .rig
        .book()
        .get(m.match_id)
        .cloned()
        .expect("the running record");
    assert!(!record.minds.is_empty());
    assert!(
        record
            .minds
            .values()
            .all(|sealed| sealed.seal.engine == tacsim::arena::SealEngine::Native),
        "every hull of a lobby match is sealed on the production engine"
    );
    let units: Vec<ObjId> = record.minds.keys().copied().collect();
    let first = *units.first().expect("a seal");
    let written = missing.clone();
    let rewritten = units.clone();
    // The snapshot's half of the pre-decision shape: each unit's reference
    // is the rewritten seal's own (the `scripted` engine and its digest).
    let scripted: BTreeMap<ObjId, tacsim::arena::MindSeal> = record
        .minds
        .iter()
        .map(|(unit, sealed)| {
            (
                *unit,
                tacsim::arena::MindSeal {
                    engine: tacsim::arena::SealEngine::Scripted,
                    source_sha256: [0; 32],
                    ..sealed.seal.clone()
                },
            )
        })
        .collect();
    let snapshot_key = match_::snapshot_key(m.match_id).to_string();
    let ground = match_::definition_for(m.rig.tw.world(), &m.head.arena)
        .expect("the ground builds")
        .expect("and is loaded");
    af3_void(
        &mut m,
        |image| {
            let docs = tacsim_docs(image);
            af3_state_edit(
                docs.get_mut(key.as_str()).expect("the live record"),
                |state| {
                    for unit in &rewritten {
                        let sealed = &mut state["record"]["minds"][unit.0.to_string()]["seal"];
                        assert_eq!(sealed["engine"], serde_json::json!("native"));
                        sealed["engine"] = serde_json::json!("scripted");
                        sealed["source_sha256"] = serde_json::json!(written);
                    }
                },
            );
            af3_reencode(
                docs.get_mut(snapshot_key.as_str()).expect("the snapshot"),
                ground,
                |arena| {
                    assert!(
                        arena.minds.is_empty(),
                        "a seated hull has no mind row until its seat is released"
                    );
                    for (unit, seal) in &scripted {
                        let state = arena.unit_mut(*unit).expect("the unit");
                        let was = record.minds.get(unit).expect("its seal");
                        assert_eq!(state.seal, was.seal.seal_ref(), "#{}", unit.0);
                        assert_eq!(state.seal.engine, tacsim::arena::SealEngine::Native);
                        state.seal = seal.seal_ref();
                    }
                },
            );
        },
        Reason::Exact(format!(
            "#{}'s mind source {missing} is neither this server's Standard Doctrine nor in its \
             store",
            first.0
        )),
    )
    .await;

    // Native as the check-out wrote it, source missing: resumes. The record's
    // seals keep the engine the production check-out wrote, as does the
    // snapshot's seal reference on each unit (what the installer reads: the
    // arena keeps the seal it was checked out under, never re-derived from
    // the record) — read here, never rewritten.
    let mut rig = af3_rig(2);
    let match_id = af3_running(&mut rig);
    let _ = af3_snapshot_after(&mut rig, match_id, 0).await;
    let record = rig.book().get(match_id).cloned().expect("the record");
    let key = match_::live_key(match_id).to_string();
    let snapshot_key = match_::snapshot_key(match_id).to_string();
    let units: Vec<ObjId> = record.minds.keys().copied().collect();
    let state: serde_json::Value = rig
        .image_doc(&snapshot_key)
        .and_then(|document| document.get("state").cloned())
        .and_then(|state| state.as_str().map(serde_json::from_str))
        .expect("a snapshot")
        .expect("a JSON state");
    let rows = state["units"].as_array().expect("the snapshot's units");
    assert_eq!(rows.len(), units.len());
    for row in rows {
        assert_eq!(row["seal"]["engine"], serde_json::json!("native"));
    }
    let writes = rig
        .reboot_mid_match_editing(|image| {
            if let Some(document) = tacsim_docs(image).get_mut(key.as_str()) {
                af3_state_edit(document, |state| {
                    for unit in &units {
                        let sealed = &mut state["record"]["minds"][unit.0.to_string()]["seal"];
                        assert_eq!(sealed["engine"], serde_json::json!("native"));
                        sealed["source_sha256"] = serde_json::json!(missing);
                    }
                });
            }
        })
        .await;
    assert!(rig.registry().tasks().is_live(match_id), "it resumed");
    assert!(set_of(&writes, &match_::result_key(match_id)).is_none());
    let said = rig.staff(&format!("@arena/abort {}", match_id.0));
    assert!(said.contains("is stopping"), "{said}");
}

/// **An unchanged record resumes, and the arena keeps the seal it was checked
/// out under** (17b cut M3.3; doc 17 §13; re-pointed by the fill decision of
/// 2026-10-09): the arena's `UnitState.seal` rides the snapshot and is never
/// re-derived from the world's record — the snapshot the boot decodes, and
/// the first one the resumed task writes, both carry each unit's seal
/// reference with the record's digest and sortie.
///
/// **The production upgrade path**: a match checked out between 17b cut M4.3
/// and the fill decision sealed every hull `scripted` on the stock source, in
/// its record and in its snapshot's references alike, and the deploy that
/// carries the decision resumes it — until 17b cut M4.6's format 9, which
/// voids every older snapshot (this test writes its snapshot at this build's
/// format, so what it pins is the scripted seal's resume path, which a
/// fixture's scripted seal still takes). Each hull of this lobby match — checked
/// out `native` (`PRODUCTION_FILL_ENGINE`), asserted — is made that shape
/// GENUINELY before the reboot: its record seal rewritten to the `scripted`
/// engine on the stock `source_sha256` (compiled in, so the resume refusal
/// and the installer both find it), and its snapshot reference to that
/// seal's own [`tacsim::arena::MindSeal::seal_ref`] (the engine and the new
/// digest), the snapshot decoded and encoded again ([`af3_reencode`]). The
/// boot resumes it — live, no result — on the scripted seals: the record it
/// puts back names them, the snapshot it decoded carries their references,
/// and the next snapshot the resumed task writes carries the same scripted
/// references, equal to the record's (a scripted reference whose source the
/// installer could not find would have voided the resume), with the mind
/// row the boot's release lays on each hull reading its scripted seal's
/// digest. Without the snapshot's half the references stay `native`,
/// nothing is installed and those asserts fail (the review's mutation
/// check).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_an_unchanged_record_resumes_with_the_seal_it_was_checked_out_under() {
    use tacsim::arena::{MindSeal, SealEngine, SealRef};
    let mut rig = af3_rig(2);
    let match_id = af3_running(&mut rig);
    let head = af3_snapshot_after(&mut rig, match_id, 0).await;
    let record = rig.book().get(match_id).cloned().expect("the record");
    let snapshot_key = match_::snapshot_key(match_id).to_string();
    let ground = match_::definition_for(rig.tw.world(), &head.arena)
        .expect("the ground builds")
        .expect("and is loaded");
    let seals_of = |rig: &Rig| {
        let document = rig.image_doc(&snapshot_key).expect("a snapshot");
        let arena = tacsim::arena::Snapshot::decode(&document, ground.clone())
            .expect("the snapshot decodes");
        (
            arena.now_ms,
            arena
                .units
                .iter()
                .map(|unit| (unit.id, unit.seal.clone()))
                .collect::<Vec<_>>(),
            arena
                .minds
                .iter()
                .map(|(unit, row)| (*unit, row.seal_digest))
                .collect::<Vec<_>>(),
        )
    };
    let check = |record: &match_::MatchRecord, seals: &[(ObjId, SealRef)]| {
        assert_eq!(seals.len(), record.minds.len());
        for (unit, seal) in seals {
            let sealed = record.minds.get(unit).expect("a recorded seal");
            assert_eq!(seal.digest, sealed.seal.digest(), "#{}", unit.0);
            assert_eq!(seal.sortie, sealed.sortie, "#{}", unit.0);
            assert_eq!(seal.engine, sealed.seal.engine, "#{}", unit.0);
            assert!(!seal.closed);
        }
    };
    let (before_ms, before, rows) = seals_of(&rig);
    check(&record, &before);
    assert!(
        rows.is_empty(),
        "a seated hull has no mind row until its seat is released"
    );

    // Every hull as a check-out between 17b cut M4.3 and the fill decision
    // sealed it: `scripted` on the stock source, in both places.
    let scripted: BTreeMap<ObjId, MindSeal> = record
        .minds
        .iter()
        .map(|(unit, sealed)| {
            assert_eq!(
                sealed.seal.engine,
                tacsim::match_::PRODUCTION_FILL_ENGINE,
                "#{} as the production check-out sealed it",
                unit.0
            );
            assert_eq!(sealed.seal.engine, SealEngine::Native);
            assert_eq!(
                sealed.seal.source_sha256,
                tacsim::minds::standard_digest().0,
                "#{} names the stock source",
                unit.0
            );
            let seal = MindSeal {
                engine: SealEngine::Scripted,
                ..sealed.seal.clone()
            };
            assert_ne!(
                seal.digest(),
                sealed.seal.digest(),
                "the engine is folded into the seal's digest"
            );
            (*unit, seal)
        })
        .collect();
    let expected: Vec<(ObjId, SealRef)> = before
        .iter()
        .map(|(unit, _)| (*unit, scripted[unit].seal_ref()))
        .collect();
    let key = match_::live_key(match_id).to_string();
    let at = snapshot_key.clone();
    let checked_out = record.clone();
    let rewritten = scripted.clone();
    let on = ground.clone();
    let writes = rig
        .reboot_mid_match_editing(move |image| {
            let docs = tacsim_docs(image);
            af3_state_edit(
                docs.get_mut(key.as_str()).expect("the live record"),
                |state| {
                    for unit in rewritten.keys() {
                        let sealed = &mut state["record"]["minds"][unit.0.to_string()]["seal"];
                        assert_eq!(sealed["engine"], serde_json::json!("native"));
                        assert_eq!(
                            sealed["source_sha256"],
                            serde_json::json!(tacsim::minds::content::STANDARD_SHA256)
                        );
                        sealed["engine"] = serde_json::json!("scripted");
                    }
                },
            );
            af3_reencode(
                docs.get_mut(at.as_str()).expect("the snapshot"),
                on,
                |arena| {
                    assert_eq!(arena.now_ms, before_ms, "the snapshot the check read");
                    for (unit, seal) in &rewritten {
                        let state = arena.unit_mut(*unit).expect("the unit");
                        let was = checked_out.minds.get(unit).expect("its seal");
                        assert_eq!(state.seal, was.seal.seal_ref(), "#{}", unit.0);
                        state.seal = seal.seal_ref();
                    }
                },
            );
        })
        .await;
    assert!(rig.registry().tasks().is_live(match_id), "it resumed");
    assert!(set_of(&writes, &match_::result_key(match_id)).is_none());
    // The record the boot put back names the scripted seals, and the snapshot
    // it decoded carries their references.
    let resumed = rig
        .book()
        .get(match_id)
        .cloned()
        .expect("the record is back");
    for (unit, seal) in &scripted {
        assert_eq!(
            resumed.minds.get(unit).map(|sealed| &sealed.seal),
            Some(seal),
            "#{} resumed on its scripted seal",
            unit.0
        );
    }
    let (decoded_ms, decoded, rows) = seals_of(&rig);
    assert_eq!(decoded_ms, before_ms);
    check(&resumed, &decoded);
    assert_eq!(decoded, expected);
    assert!(rows.is_empty());
    // The first snapshot the resumed task writes carries the same seals.
    let mut after = None;
    for _ in 0..3_000 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        rig.pump();
        let _ = rig.pump_ops();
        let (now_ms, seals, rows) = seals_of(&rig);
        if now_ms > before_ms {
            after = Some((seals, rows));
            break;
        }
    }
    let (after, rows) = after.expect("the resumed task snapshots again");
    check(&resumed, &after);
    // The boot released every seat to its sealed mind ("to doctrine" here
    // until 17b cut M5.7), and the rows the release laid
    // read their digest off the scripted references (`Arena::adopt_mind`).
    assert_eq!(
        rows,
        scripted
            .iter()
            .map(|(unit, seal)| (*unit, seal.digest()))
            .collect::<Vec<_>>(),
        "each released hull's mind row carries its scripted seal's digest"
    );
    assert!(
        after
            .iter()
            .all(|(_, seal)| seal.engine == SealEngine::Scripted),
        "every unit resumed on the scripted seal it was checked out under: {after:?}"
    );
    assert_eq!(
        after, expected,
        "the arena kept the seals it was checked out under"
    );
    let said = rig.staff(&format!("@arena/abort {}", match_id.0));
    assert!(said.contains("is stopping"), "{said}");
}

// ---------------------------------------------------------------------------
// 17b cut M3.5: the pool at check-out (doc 17 §9, D-UM-36)
// ---------------------------------------------------------------------------

/// **`n` more stock tanks in the world**, each made a tactical unit through
/// `@unit/create` and placed in the proving ground's domain on red slot 1's
/// pose — `match_::start` relocates every unit into its side's entry zone
/// (`opening_pose`), so the shared pose is never where one opens.
fn spare_hulls(rig: &mut Rig, n: usize) -> Vec<ObjId> {
    let anchor = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    let (domain, pose) = mush_sim::pose_of(rig.tw.world(), anchor).expect("the tank is placed");
    let mut hulls = Vec::new();
    for k in 0..n {
        let thing = rig
            .tw
            .world_mut()
            .create(
                mush_core::ObjKind::Thing,
                &format!("Spare Hull {k}"),
                GOD,
                Some(ObjId(0)),
            )
            .expect("a thing");
        let said = rig.staff(&format!(
            "@unit/create #{}={}",
            thing.0,
            tacsim::definitions::content::LINE_TANK
        ));
        assert!(said.contains("is now a tactical unit"), "{said}");
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
        mush_sim::window::place(&mut ctx, thing, domain, pose.clone()).expect("the hull is placed");
        hulls.push(thing);
    }
    hulls
}

/// **Twelve hulls a side** for `match_::start`: the side's six shipped tanks
/// then six of `spares`, the first `classed` of each red twelve fielded with
/// a Class III preview, every other by the fill rule. The Class III gate then
/// installs real cores and selections with `install_class_iii_minds`.
fn twelve_a_side(
    rig: &Rig,
    spares: &[ObjId],
    classed: usize,
    class: tacsim::arena::core::CoreClass,
) -> Vec<match_::Side> {
    let mut sides = Vec::new();
    for (team, extra) in [
        (PartyId::RED, spares.get(..6).unwrap_or_default()),
        (PartyId::BLUE, spares.get(6..12).unwrap_or_default()),
    ] {
        let mut units: Vec<ObjId> = (1..=6)
            .map(|slot| match_::tank_for_slot(rig.tw.world(), team, slot).expect("a shipped tank"))
            .collect();
        units.extend_from_slice(extra);
        let fielded = units
            .into_iter()
            .enumerate()
            .map(|(at, unit)| match_::Fielded {
                mind_class: (team == PartyId::RED && at < classed).then_some(class),
                ..match_::Fielded::unslotted(unit)
            })
            .collect();
        sides.push(match_::Side {
            team,
            units: fielded,
            minds: BTreeMap::new(),
        });
    }
    sides
}

/// Give the pool fixture real Class III custody: a governed heavy core and
/// Load III's actual source/profile selection. A caller's preview class is not
/// authority, so deliberately understate it after preparing the real choice.
fn install_class_iii_minds(rig: &mut Rig, sides: &mut [match_::Side]) {
    use tacsim::arena::core::CoreClass;
    let _provider = holdings_on(rig);
    let spec = {
        let id = mush_core::CanonicalId::new(tacsim::definitions::content::HEAVY_MK1)
            .expect("the heavy definition id");
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .expect("the source catalog")
            .definitions()
            .bindings
            .get(&id)
            .expect("the heavy is fieldable")
            .spec
            .clone()
    };
    for side in sides {
        for fielded in &mut side.units {
            if fielded.mind_class != Some(CoreClass::III) {
                continue;
            }
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
            mush_tactics::module::refit_unit(&mut ctx, fielded.unit, &spec)
                .expect("the pool fixture has a genuine governed Class III core");
            side.minds.insert(
                fielded.unit,
                match_::holdings::SealChoice {
                    held: support::holdings::load_iii(),
                    profiles: Vec::new(),
                    chose: true,
                    substituted: false,
                },
            );
            fielded.mind_class = Some(CoreClass::Zero);
        }
    }
}

/// `match_::start` as staff's `@arena/start` calls it — the proving ground,
/// a skirmish, the mind fill (17b cut M4.3) — under `cfg`'s blocks.
fn start_as_staff(
    rig: &mut Rig,
    cfg: &TacsimConfig,
    match_id: u64,
    sides: &[match_::Side],
) -> Result<tacsim::arena_task::MatchId, match_::StartError> {
    start_as_staff_queued(rig, cfg, match_id, sides).0
}

/// [`start_as_staff`], answering too how much deferred work the call left
/// queued (17b cut M5.2's review) — every document write among it — so a
/// test tells a refusal taken before the first write (none) from one rolled
/// back after the check-out wrote.
fn start_as_staff_queued(
    rig: &mut Rig,
    cfg: &TacsimConfig,
    match_id: u64,
    sides: &[match_::Side],
) -> (
    Result<tacsim::arena_task::MatchId, match_::StartError>,
    usize,
) {
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
    let started = match_::start(
        &mut ctx,
        tacsim::arena_task::MatchId(match_id),
        &smol_str::SmolStr::new_static(match_::PROVING_GROUND),
        sides,
        &cfg.arena,
        &cfg.match_,
        &cfg.sensors,
        tacsim::arena::MatchMode::Skirmish,
    );
    (started, out.deferred().len())
}

/// **THE GATE FIXTURE: an over-subscribed roster is refused by name at
/// check-out** (17b cut M3.5; doc 17 §9, D-UM-36: *check-out refuses an
/// over-subscribed roster by name as the last gate*).
///
/// At the stock pool the arena's check-out gate cannot refuse (twelve Class
/// III a side load 216,000 against a half of 238,000, and a thirteenth is
/// `TooManyUnits` — `match_::pool`'s head), so this runs under the world's
/// `[tacsim.minds].pool_base_ops_s = 300_000`: 276,000 at 24 units, a half of
/// 138,000. Twelve a side (the six shipped tanks and six spare stock tanks
/// each) through `match_::start` as staff calls it. M6 installs actual Load III
/// selections and heavy cores, with a III fill cap, before checkout; preview
/// fields are deliberately understated and cannot reduce the true seal charge:
///
/// * eight of red's twelve fielded with a Class III mind: refused
///   `StartError::Pool` on the EIGHTH — `would 18,000`, `half 138,000`,
///   `spoken 126,000` (seven Class III before it), said in the pool's
///   sentence behind the refused hull's name (17b cut M5.1: the hull named,
///   `Spare Hull 1`, in the owner's OQ-11 wording; behind the side, `red: This
///   side would think …`, at 17b cut M3.5) — before any write: no unit
///   deployed, no record in the book;
/// * seven Class III (and five fill hulls at Class I's 1,500 — 17b cut
///   M4.3's `fill_class`: 133,500 a side): admitted, and the match opens;
/// * on a fresh world under the same pool, twelve a side by the fill rule
///   (18,000 a side): admitted;
/// * and at the STOCK pool, on a fresh world, twelve Class III a side —
///   216,000 a side: admitted (the declared fact).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_an_over_subscribed_roster_is_refused_by_name_at_check_out() {
    use tacsim::arena::core::CoreClass;
    use tacsim::match_::pool::PoolRefusal;
    let mut lowered = config();
    lowered.minds.pool_base_ops_s = 300_000;
    lowered.minds.fill_class = CoreClass::III;
    assert_eq!(tacsim::match_::pool::pool_for(&lowered.minds, 24), 276_000);

    let mut rig = Rig::boot_on(0, roster_with(lowered.clone()));
    let spares = spare_hulls(&mut rig, 12);
    let mut sides = twelve_a_side(&rig, &spares, 8, CoreClass::III);
    install_class_iii_minds(&mut rig, &mut sides);
    let refused = start_as_staff(&mut rig, &lowered, 91, &sides);
    let expected = PoolRefusal {
        would: 18_000,
        half: 138_000,
        spoken: 126_000,
    };
    // Red's eighth hull: its six tanks, then its second spare.
    let eighth = *spares.get(1).expect("red's second spare");
    assert_eq!(
        refused,
        Err(match_::StartError::Pool {
            team: "red",
            unit: eighth,
            callsign: "Spare Hull 1".to_owned(),
            refusal: expected,
        })
    );
    assert_eq!(
        refused.map_err(|error| error.to_string()),
        Err(
            "Spare Hull 1: This tank would think 18,000 ops a second; its side's half of the \
             centre's pool is 138,000 and 126,000 are spoken for."
                .to_owned()
        )
    );
    for side in &sides {
        for fielded in &side.units {
            assert!(
                !mush_sim::deployed::is_deployed(rig.tw.world(), fielded.unit),
                "#{} was checked out",
                fielded.unit.0
            );
        }
    }
    assert!(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .and_then(|book| book.get(tacsim::arena_task::MatchId(91)))
            .is_none(),
        "no record was written"
    );

    // Seven Class III: admitted, and the match opens.
    let mut sides = twelve_a_side(&rig, &spares, 7, CoreClass::III);
    install_class_iii_minds(&mut rig, &mut sides);
    let opened = start_as_staff(&mut rig, &lowered, 92, &sides);
    assert_eq!(opened, Ok(tacsim::arena_task::MatchId(92)), "seven pass");

    // A fresh world under the same pool: twelve a side by the fill rule.
    let mut rig = Rig::boot_on(0, roster_with(lowered.clone()));
    let spares = spare_hulls(&mut rig, 12);
    let sides = twelve_a_side(&rig, &spares, 0, CoreClass::III);
    let opened = start_as_staff(&mut rig, &lowered, 93, &sides);
    assert_eq!(
        opened,
        Ok(tacsim::arena_task::MatchId(93)),
        "the fill passes"
    );

    // The stock pool: twelve Class III a side are admitted.
    let mut stock = config();
    assert_eq!(
        stock.minds,
        tacsim::config::MindsConfig::default(),
        "the stock keys"
    );
    // Permit the selected III minds; all pool arithmetic remains stock.
    stock.minds.fill_class = CoreClass::III;
    let mut rig = Rig::boot_on(0, roster_with(stock.clone()));
    let spares = spare_hulls(&mut rig, 12);
    let mut sides = twelve_a_side(&rig, &spares, 12, CoreClass::III);
    for fielded in sides.iter_mut().flat_map(|side| side.units.iter_mut()) {
        fielded.mind_class = Some(CoreClass::III);
    }
    install_class_iii_minds(&mut rig, &mut sides);
    let opened = start_as_staff(&mut rig, &stock, 94, &sides);
    assert_eq!(
        opened,
        Ok(tacsim::arena_task::MatchId(94)),
        "twelve Class III a side fit the stock pool"
    );
}

/// **Pilot `n` walks to `unit` and takes its cockpit** — `Rig::station_on`
/// proves reach from the room the unit stands in.
fn seat_beside(rig: &mut Rig, n: usize, unit: ObjId) {
    let room = rig
        .tw
        .world()
        .get(unit)
        .and_then(|thing| thing.location)
        .expect("the hull stands somewhere");
    let pilot = rig.pilot(n);
    rig.tw
        .world_mut()
        .move_to(pilot, room)
        .expect("a pilot walks to the hull");
    rig.station_on(n, unit);
}

/// **The fill rule decides the pool, and the pool is the LAST gate** (17b cut
/// M3.5; doc 17 §9, D-UM-36), under a pool small enough that the fill's
/// class is what tips a side — **17b cut M4.3**: `[tacsim.minds].fill_class`,
/// Class I's 1,500 — `[tacsim.minds]` at base 33,024, per unit 1, floor 1 —
/// 33,000 at 24 units, a half of 16,500, so eleven fill hulls (16,500) fit
/// and a twelfth (18,000) does not.
///
/// * twelve a side, nobody seated, every hull by the fill rule: red is
///   refused on its TWELFTH hull (`Spare Hull 5`) — `would 1,500`, `spoken
///   16,500` — so an unseated hull loads the fill's Class I;
/// * **17b cut M5.1** (the design note's M5.1 item 7; doc 17 §24's carried
///   "seated hulls at the pool"): a pilot stationed on red's seventh hull, a
///   spare (the two writes `BOARD` makes), and another on blue's: red is
///   refused exactly as before — a seated hull is admitted at `fill_class`
///   too, the class it is sealed and, released, flown at. At 17b cut M3.5
///   the seat on red's hull loaded nothing, so red was admitted and blue
///   refused (`fill_refused("blue")`), and a seat a side opened the match;
/// * eleven a side, the two seats among them (22 units: 33,002, a half of
///   16,501): the match opens;
/// * and on a fresh world whose roster is BOTH over the pool and carries a
///   card under `odds_floor_permille = 200`, check-out is refused for the
///   card (`StartError::CardUnlawful`), never the pool: the pool gate comes
///   after every other refusal of check-out's block (doc 17 §9: "the last
///   gate").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m3_the_fill_rule_decides_the_pool_and_the_pool_is_the_last_gate() {
    use tacsim::arena::core::CoreClass;
    use tacsim::match_::pool::PoolRefusal;
    let mut tiny = config();
    tiny.minds.pool_base_ops_s = 33_024;
    tiny.minds.pool_per_unit_ops_s = 1;
    tiny.minds.pool_floor_ops_s = 1;
    assert_eq!(tacsim::match_::pool::pool_for(&tiny.minds, 24), 33_000);
    let fill_refused = |team: &'static str, unit: ObjId, callsign: &str| {
        Err(match_::StartError::Pool {
            team,
            unit,
            callsign: callsign.to_owned(),
            refusal: PoolRefusal {
                would: 1_500,
                half: 16_500,
                spoken: 16_500,
            },
        })
    };

    let mut rig = Rig::boot_on(2, roster_with(tiny.clone()));
    // The control-lease mirror is read on the first second of world time, and
    // `request_seat_activation` fails closed before it (as in `d3_`).
    rig.tick_to(1);
    let spares = spare_hulls(&mut rig, 12);
    let sides = twelve_a_side(&rig, &spares, 0, CoreClass::III);
    let red_twelfth = *spares.get(5).expect("red's last spare");
    assert_eq!(
        start_as_staff(&mut rig, &tiny, 95, &sides),
        fill_refused("red", red_twelfth, "Spare Hull 5"),
        "twelve unseated hulls load the fill's Class I each"
    );

    // Red's first spare (its seventh hull) and blue's: a seat on each. 17b cut
    // M5.1: a seated hull loads `fill_class` too, so the seats change nothing.
    seat_beside(&mut rig, 0, *spares.first().expect("red's first spare"));
    seat_beside(&mut rig, 1, *spares.get(6).expect("blue's first spare"));
    assert_eq!(
        start_as_staff(&mut rig, &tiny, 96, &sides),
        fill_refused("red", red_twelfth, "Spare Hull 5"),
        "a seated hull loads the fill's Class I, so red is refused as before the seats"
    );
    for side in &sides {
        for fielded in &side.units {
            assert!(
                !mush_sim::deployed::is_deployed(rig.tw.world(), fielded.unit),
                "#{} was checked out",
                fielded.unit.0
            );
        }
    }

    // Eleven a side, the seats among them: 22 units, a half of 16,501.
    assert_eq!(tacsim::match_::pool::pool_for(&tiny.minds, 22), 33_002);
    let eleven: Vec<match_::Side> = sides
        .iter()
        .map(|side| match_::Side {
            units: side.units.get(..11).unwrap_or_default().to_vec(),
            ..side.clone()
        })
        .collect();
    assert_eq!(
        start_as_staff(&mut rig, &tiny, 97, &eleven),
        Ok(tacsim::arena_task::MatchId(97)),
        "eleven a side, a seated hull on each: the match opens"
    );

    // Over the pool AND under the Compact floor: the card is refused first.
    let mut both = tiny.clone();
    both.minds.odds_floor_permille = 200;
    let mut rig = Rig::boot_on(0, roster_with(both.clone()));
    let spares = spare_hulls(&mut rig, 12);
    let sides = twelve_a_side(&rig, &spares, 0, CoreClass::III);
    let refused = start_as_staff(&mut rig, &both, 98, &sides);
    let red_first = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("red 1's tank");
    assert!(
        matches!(
            refused,
            Err(match_::StartError::CardUnlawful { unit, floor: 200, .. }) if unit == red_first
        ),
        "the pool is the last gate: {refused:?}"
    );
}

// ---------------------------------------------------------------------------
// 17b cut M5.1: the choice in the lobby — `SIM MIND`, `SIM AI MIND`, the
// holdings seam, the fit, the pool and the selection fog
// ---------------------------------------------------------------------------

/// The test provider on `rig`'s world, through the production seam
/// (`tests/support/holdings.rs`): Standard Doctrine, `Fixture I` and `Load
/// III` for every character.
fn holdings_on(rig: &mut Rig) -> Arc<support::holdings::TestHoldings> {
    let setup = support::holdings::seed_fixture_revisions(rig.tw.world_mut());
    fold_ops(&mut rig.image, &setup);
    support::holdings::install(rig.tw.world_mut())
}

/// God in the simulator lobby, so a staff `SIM` line is of the centre.
fn god_in_the_lobby(rig: &mut Rig) {
    let lobby =
        match_::centre_object(rig.tw.world(), match_::LOBBY_ENTRY).expect("the simulator lobby");
    rig.tw
        .world_mut()
        .move_to(GOD, lobby)
        .expect("God walks into the lobby");
}

impl Rig {
    /// **17b cut M5.1** (its review): run one line as pilot `n` and answer
    /// both what that pilot saw and what pilot `other` heard of it — the
    /// reply and its audience from the one run.
    fn say_heard(&mut self, n: usize, line: &str, other: usize) -> (String, String) {
        let actor = self.pilot(n);
        let listener = self.pilot(other);
        let events = self.tw.run(actor, line);
        (self.seen(actor, &events), self.seen(listener, &events))
    }

    /// **17b cut M5.1** (its review): the sides the check-out would be handed
    /// now — `lobby::ready`'s, through its test hook — with each hull's
    /// `Fielded::mind_class` and each side's `Side::minds`; nothing is run,
    /// written or said.
    fn ready_sides(&mut self) -> Result<Vec<match_::Side>, match_::SimError> {
        let mut out = Outbox::new();
        let sides = {
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
            match_::lobby::ready_sides_for_tests(&mut ctx)
        };
        assert!(out.take().is_empty(), "the hook says nothing");
        sides
    }
}

/// Side `team` of what [`Rig::ready_sides`] answered.
fn side_of(sides: &[match_::Side], team: PartyId) -> &match_::Side {
    sides
        .iter()
        .find(|side| side.team == team)
        .expect("the check-out fields both sides")
}

/// The sentence `SIM MIND`'s bare read prints first for a slot that chose
/// nothing.
const UNCHOSEN: &str = "Yours: none; a tank you leave flies Standard Doctrine on Line.";

/// Every stock preset, as a `Fits` line lists them.
const SIX: &str = "Line, Sentry, Sniper, Skirmisher, Hunter or Rearguard";

/// **17b cut M5.1** (D-UM-49; the owner's OQ-2): the bare `SIM MIND` lists
/// what the CHARACTER holds and what fits its slot's chassis, to the actor
/// alone — under `DoctrineOnly`, Standard Doctrine alone.
///
/// * no slot: `You hold Standard Doctrine (six presets); claim a slot and SIM
///   MIND lists what fits its core.`;
/// * a `line_tank` slot (a governed Class II core): `Yours: none; …`, the
///   `Fits line_tank (II)` line — up to two profiles, the count read at the
///   class Standard Doctrine is sealed at (I), not the core's — and the pool
///   line and exact sealed class (one fielded hull gives a half of 249,500;
///   dormant pods do not enter the pool);
/// * a `drone_mk1` slot (a governed Class I core, no gun): the same lines on
///   its own chassis — every preset is lawful on it;
/// * another pilot hears none of it;
/// * with the test provider installed, `Fixture I` fits the `line_tank`, and
///   `Load III`, rated above the line core, has no `Fits` line;
/// * (17b cut M5.1's review) Bo, in the lobby, hears nothing of Ada's set or
///   clear, and of `SIM UNIT`'s re-fit of her standing choice only the
///   type it announces — no mind, no preset.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_sim_mind_lists_only_what_the_character_holds_and_what_fits() {
    let mut rig = Rig::boot(2);
    assert_eq!(
        rig.say(0, "sim mind"),
        "You hold Standard Doctrine (six presets); claim a slot and SIM MIND lists what fits its \
         core."
    );
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim mind");
    assert_eq!(
        said,
        [
            UNCHOSEN.to_owned(),
            format!("Fits line_tank (II): Standard Doctrine, up to two of {SIX}."),
            "Your mind will seal at Class I.".to_owned(),
            "Red's half of the centre's pool is 249,500 ops a second and 1,500 are spoken for."
                .to_owned(),
        ]
        .join("\n")
    );
    assert!(
        !said.contains("Fixture") && !said.contains("Load"),
        "Standard Doctrine is all a character holds: {said}"
    );
    let heard = rig.say_overheard(0, "sim mind", 1);
    assert!(heard.is_empty(), "the read is the actor's alone: {heard}");

    rig.say(1, "sim team blue");
    let said = rig.say(1, "sim unit drone_mk1");
    assert!(said.contains("Slot blue 1 will field drone_mk1."), "{said}");
    assert_eq!(
        rig.say(1, "sim mind"),
        [
            UNCHOSEN.to_owned(),
            format!("Fits drone_mk1 (I): Standard Doctrine, up to two of {SIX}."),
            "Your mind will seal at Class I.".to_owned(),
            "Blue's half of the centre's pool is 249,000 ops a second and 1,500 are spoken for."
                .to_owned(),
        ]
        .join("\n")
    );

    let _provider = holdings_on(&mut rig);
    assert_eq!(
        rig.say(0, "sim mind"),
        [
            UNCHOSEN.to_owned(),
            format!("Fits line_tank (II): Standard Doctrine, up to two of {SIX}."),
            "Fits line_tank (II): Fixture I, up to two of Line or Sniper.".to_owned(),
            "Your mind will seal at Class I.".to_owned(),
            "Red's half of the centre's pool is 249,000 ops a second and 1,500 are spoken for."
                .to_owned(),
        ]
        .join("\n"),
        "Load III is rated above the line core, so it fits nothing here"
    );

    // 17b cut M5.1's review: nothing about a choice is told to anybody else
    // (doc 17 §12's fog) — the set, `SIM UNIT`'s re-fit of the standing
    // choice (the one choice-adjacent verb that announces: the type, never
    // the mind) and the clear, each overheard by Bo in the lobby.
    let (said, heard) = rig.say_heard(0, "sim mind Standard PROFILE Sniper, Line", 1);
    assert_eq!(
        said,
        "Your slot will carry Standard Doctrine (Sniper, Line)."
    );
    assert!(heard.is_empty(), "a choice is told to nobody else: {heard}");
    let (said, heard) = rig.say_heard(0, "sim unit line_tank_thermal", 1);
    assert!(
        said.contains("Slot red 1 will field line_tank_thermal."),
        "{said}"
    );
    assert!(
        heard.contains("Slot red 1 will field line_tank_thermal."),
        "SIM UNIT announces the type: {heard}"
    );
    for word in ["mind", "Doctrine", "Sniper"] {
        assert!(!heard.contains(word), "the re-fit names no mind: {heard}");
    }
    assert!(
        rig.say(0, "sim mind")
            .starts_with("Yours: Standard Doctrine (Sniper, Line).\n"),
        "the standing choice is fitted to the new chassis"
    );
    let (said, heard) = rig.say_heard(0, "sim mind none", 1);
    assert_eq!(said, tacsim::match_::lobby::MIND_CLEARED);
    assert!(heard.is_empty(), "a clear is told to nobody else: {heard}");
}

/// **17b cut M5.1** (doc 17 §22's italic: *a fixture that a mind the
/// character does not hold is refused*, *a fixture that the sim seal takes no
/// seat and posts nothing*): `SIM MIND` naming a mind the character does not
/// hold is refused in one sentence to the actor alone, against
/// `DoctrineOnly` (`Sentinel`, and the test provider's two minds, which no
/// production character holds) and against the test provider (`Sentinel`);
/// neither a refusal nor an accepted choice queues one module write; and the
/// check-out of a match with a chosen mind writes exactly the keys the
/// check-out of one without writes (`board_writes`).
///
/// Also the provider's own pins: `Fixture I`'s and `Load III`'s digests, and
/// `install_holdings` putting `Fixture I`'s source in the store's RAM map,
/// where a seal's digest finds it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_mind_the_character_does_not_hold_is_refused_and_posts_nothing() {
    use support::holdings::{
        FIXTURE_I_RHAI, FIXTURE_I_SHA256_HEX, LOAD_III_RHAI, LOAD_III_SHA256_HEX, digest,
        digest_hex,
    };
    assert_eq!(digest_hex(FIXTURE_I_RHAI), FIXTURE_I_SHA256_HEX);
    assert_eq!(digest_hex(LOAD_III_RHAI), LOAD_III_SHA256_HEX);

    /// The `(op, key)` list of the second `BOARD`'s check-out, sorted.
    fn checkout_keys(rig: &mut Rig) -> Vec<(&'static str, String)> {
        rig.say(0, "sim start");
        rig.board_writes(0, match_::RED_ROOM_ENTRY);
        let (said, writes) = rig.board_writes(1, match_::BLUE_ROOM_ENTRY);
        assert!(said.contains("climb into"), "{said}");
        assert_eq!(rig.state(), MatchState::Running, "the check-out ran");
        let mut keys: Vec<(&'static str, String)> =
            writes.into_iter().map(|(op, key, _)| (op, key)).collect();
        keys.sort();
        keys
    }

    let mut rig = Rig::boot(2);
    rig.tick_to(1);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    let _ = rig.pump_ops();
    for word in ["Sentinel", "Fixture I", "Load III"] {
        assert_eq!(
            rig.say(0, &format!("sim mind {word}")),
            "No such owned mind or revision."
        );
    }
    let heard = rig.say_overheard(0, "sim mind Sentinel", 1);
    assert!(heard.is_empty(), "a refusal is the actor's alone: {heard}");
    assert!(rig.pump_ops().is_empty(), "a refusal writes nothing");
    assert_eq!(
        rig.centre()
            .slots
            .first()
            .and_then(|slot| slot.mind.clone()),
        None
    );
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Sniper, Line"),
        "Your slot will carry Standard Doctrine (Sniper, Line)."
    );
    assert!(rig.pump_ops().is_empty(), "a choice writes nothing");

    let _provider = holdings_on(&mut rig);
    assert_eq!(
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .and_then(|store| store.mind_source(&digest(FIXTURE_I_RHAI))),
        Some(FIXTURE_I_RHAI),
        "the provider's source is in the store's RAM map"
    );
    assert_eq!(
        rig.say(0, "sim mind Sentinel"),
        "No such owned mind or revision."
    );
    assert_eq!(
        rig.say(0, "sim mind fixture i"),
        "Your slot will carry Fixture I (Line)."
    );
    assert!(rig.pump_ops().is_empty(), "a choice writes nothing");
    let chosen = checkout_keys(&mut rig);

    let mut plain = Rig::boot(2);
    plain.tick_to(1);
    plain.say(0, "sim team red");
    plain.say(1, "sim team blue");
    let unchosen = checkout_keys(&mut plain);
    assert!(!chosen.is_empty());
    assert_eq!(
        chosen, unchosen,
        "a chosen mind adds no write to the check-out"
    );
}

/// The later choice is re-admitted against the complete, actually fielded
/// roster. M6 also admits claims, so no setup relies on an unchecked claim
/// creating an oversized side. Coreless defaults cost ROM whether explicitly
/// chosen or not; clearing a choice cannot spuriously raise that cost.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_later_of_sim_unit_and_sim_mind_is_refused_with_the_arithmetic() {
    use tacsim::arena::core::CoreClass;
    let mut lowered = config();
    lowered.minds.pool_base_ops_s = 3_800;
    lowered.minds.pool_per_unit_ops_s = 0;
    lowered.minds.pool_floor_ops_s = 1;
    assert_eq!(tacsim::match_::pool::pool_for(&lowered.minds, 2), 3_800);

    // Unit first: Bo's coreless hull (400) leaves room for Ada's default I
    // (1,500), but not her later selected II (6,000).
    let mut rig = Rig::boot_on(2, roster_with(lowered.clone()));
    class_two_provider(&mut rig);
    assert!(rig.say(1, "sim team red").contains("takes red slot 1"));
    assert!(
        rig.say(1, "sim unit target_mk1")
            .contains("will field target_mk1.")
    );
    assert!(rig.say(0, "sim team red").contains("takes red slot 2"));
    assert!(
        rig.say(0, "sim unit line_tank")
            .contains("will field line_tank.")
    );
    let before = rig.centre();
    assert_eq!(
        rig.say(0, "sim mind Fixture II"),
        "This tank would think 6,000 ops a second; its side's half of the centre's pool is \
         1,900 and 400 are spoken for."
    );
    assert_eq!(rig.centre(), before, "a refused mind changes no selection");

    // Mind first: a II mind may be selected on a coreless hull (ROM), but
    // moving it onto a genuine II core must charge II's complete ceiling.
    let mut rig = Rig::boot_on(2, roster_with(lowered.clone()));
    class_two_provider(&mut rig);
    assert!(rig.say(0, "sim team red").contains("takes red slot 1"));
    assert!(
        rig.say(0, "sim unit target_mk1")
            .contains("will field target_mk1.")
    );
    assert_eq!(
        rig.say(0, "sim mind Fixture II"),
        "Your slot will carry Fixture II (Line)."
    );
    assert!(rig.say(1, "sim team red").contains("takes red slot 2"));
    let before = rig.centre();
    assert_eq!(
        rig.say(0, "sim unit line_tank"),
        "This tank would think 6,000 ops a second; its side's half of the centre's pool is \
         1,900 and 0 are spoken for."
    );
    assert_eq!(
        rig.centre(),
        before,
        "a refused chassis keeps its mind and coreless type"
    );
    assert_eq!(preview_class(&rig, PartyId::RED, 1), CoreClass::Zero);

    // The same exact ROM charge holds for an explicitly chosen Standard and
    // for the unchosen default. Clearing is allowed and does not invent I.
    assert_eq!(
        rig.say(0, "sim mind Standard"),
        "Your slot will carry Standard Doctrine (Line)."
    );
    assert_eq!(
        rig.say(0, "sim mind none"),
        tacsim::match_::lobby::MIND_CLEARED
    );
    assert_eq!(preview_class(&rig, PartyId::RED, 1), CoreClass::Zero);
    assert!(
        rig.centre()
            .slots
            .iter()
            .find(|slot| slot.pilot == rig.pilot(0))
            .expect("Ada's slot")
            .mind
            .is_none()
    );
    let before = rig.centre();
    assert_eq!(
        rig.say(0, "sim unit line_tank"),
        "This tank would think 1,500 ops a second; its side's half of the centre's pool is \
         1,900 and 1,500 are spoken for."
    );
    assert_eq!(
        rig.centre(),
        before,
        "even the default is re-admitted on a new chassis"
    );
    let said = rig.say(0, "sim mind");
    assert!(
        said.ends_with(
            "Red's half of the centre's pool is 1,900 ops a second and 1,900 are spoken for."
        ),
        "{said}"
    );

    // Claims use the same complete admission and cannot create the invalid
    // oversubscribed premise the pre-M6 version of this test relied upon.
    let mut rig = Rig::boot_on(2, roster_with(lowered));
    assert!(rig.say(0, "sim team red").contains("takes red slot 1"));
    let before = rig.centre();
    assert_eq!(
        rig.say(1, "sim team red"),
        "This tank would think 1,500 ops a second; its side's half of the centre's pool is \
         1,900 and 1,500 are spoken for."
    );
    assert_eq!(
        rig.centre(),
        before,
        "a refused claim publishes no roster change"
    );
}

/// **The over-class refusal at the choice** (17b cut M5.1; D-UM-41, the
/// owner's OQ-10 (d), the design note's C-24): `Load III`, rated III, on a
/// `line_tank` (a governed Class II core) is refused — `Load III thinks at
/// Class III; this line_tank's core is Class II and is governed.` — and on a
/// chassis with no core (`target_mk1`, which flies the ROM) it is never
/// over-class; the later `SIM UNIT` back onto the line core is refused for
/// it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_an_over_class_mind_is_refused_at_the_choice() {
    let mut rig = Rig::boot(1);
    let _provider = holdings_on(&mut rig);
    rig.say(0, "sim team red");
    let over = "Load III thinks at Class III; this line_tank's core is Class II and is governed.";
    assert_eq!(rig.say(0, "sim mind Load III"), over);
    rig.say(0, "sim unit target_mk1");
    assert_eq!(
        rig.say(0, "sim mind load iii"),
        "Your slot will carry Load III (Line)."
    );
    assert_eq!(rig.say(0, "sim unit line_tank"), over);
}

/// **A profile under the operator's Compact floor is refused as typed** (17b
/// cut M5.1; D-UM-17: refuse, never clamp) on an armed `line_tank`, under
/// `[tacsim.minds].odds_floor_permille = 400`: Skirmisher (35%), Hunter (30%)
/// and Line alone (10%) are refused by name, the first under the floor in the
/// order typed; Sniper and Rearguard are sealed; and the bare read's `Fits`
/// line lists only the presets the floor admits.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_profile_under_the_floor_is_refused_as_typed() {
    let mut floor = config();
    floor.minds.odds_floor_permille = 400;
    let mut rig = Rig::boot_on(1, roster_with(floor));
    rig.say(0, "sim team red");
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Skirmisher"),
        "Skirmisher fires at 35% and better; this server's Compact floor is 40%, so it cannot be \
         sealed."
    );
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Sniper, hunter"),
        "Hunter fires at 30% and better; this server's Compact floor is 40%, so it cannot be \
         sealed."
    );
    assert_eq!(
        rig.say(0, "sim mind Standard"),
        "Line fires at 10% and better; this server's Compact floor is 40%, so it cannot be sealed."
    );
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Sniper, Rearguard"),
        "Your slot will carry Standard Doctrine (Sniper, Rearguard)."
    );
    let said = rig.say(0, "sim mind");
    assert!(
        said.contains(
            "Fits line_tank (II): Standard Doctrine, up to two of Sentry, Sniper or Rearguard."
        ),
        "{said}"
    );
}

/// **An unarmed hull takes any preset under any floor** (17b cut M5.1; the
/// design note's C-23; R-60, R-61): on a `drone_mk1` — no weapons station —
/// under the highest floor the keys allow (95%), every preset is lawful and
/// listed, `Hunter, Line` is sealed; and the later `SIM UNIT` onto an armed
/// chassis is refused for the floor it now binds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_an_unarmed_hull_takes_any_preset_under_any_floor() {
    let mut floor = config();
    floor.minds.odds_floor_permille = 950;
    let mut rig = Rig::boot_on(1, roster_with(floor));
    rig.say(0, "sim team red");
    let said = rig.say(0, "sim unit drone_mk1");
    assert!(said.contains("Slot red 1 will field drone_mk1."), "{said}");
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Hunter, Line"),
        "Your slot will carry Standard Doctrine (Hunter, Line)."
    );
    let said = rig.say(0, "sim mind");
    assert!(
        said.contains(&format!(
            "Fits drone_mk1 (I): Standard Doctrine, up to two of {SIX}."
        )),
        "{said}"
    );
    assert_eq!(
        rig.say(0, "sim unit line_tank"),
        "Hunter fires at 30% and better; this server's Compact floor is 95%, so it cannot be \
         sealed."
    );
}

/// **The sealed class caps the profile count** (17b cut M5.1; the owner's
/// OQ-3: N from the mind's sealed class, Class I → 2): on a `line_tank`
/// Standard Doctrine seals at Class I — not the core's II — so three profiles
/// are refused. Under M6 a coreless `target_mk1` may retain a requested profile
/// set, but actual checkout substitutes the complete singleton Class 0 ROM;
/// the selected Sniper/Line set never becomes its executable card set. Also
/// cover an unknown profile, a duplicate, a revision pin and the usage lines.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_sealed_class_caps_the_profile_count() {
    let mut rig = fill_rig(1);
    rig.say(0, "sim team red");
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Sniper, Line, Hunter"),
        "Standard Doctrine thinks at Class I on this core, which seals two profiles at most."
    );
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Brawler"),
        format!("Standard Doctrine has no profile called Brawler: {SIX}.")
    );
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Sniper, sniper"),
        "Sniper is named twice; a profile is sealed once."
    );
    assert_eq!(
        rig.say(0, "sim mind Standard @3"),
        "No such owned mind or revision."
    );
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE"),
        "Which profiles? `SIM MIND Standard PROFILE Sniper, Line`."
    );
    assert_eq!(
        rig.say(0, "sim mind PROFILE Sniper"),
        "Which mind? `SIM MIND Standard`, or `SIM MIND none`."
    );
    assert_eq!(
        rig.say(0, "sim mind doctrine profile Sniper, Line"),
        "Your slot will carry Standard Doctrine (Sniper, Line)."
    );
    rig.say(0, "sim mind none");
    let said = rig.say(0, "sim unit target_mk1");
    assert!(said.contains("will field target_mk1."), "{said}");
    assert_eq!(
        rig.say(0, "sim mind Standard PROFILE Sniper, Line"),
        "Your slot will carry Standard Doctrine (Sniper, Line)."
    );
    let said = rig.say(0, "sim mind");
    assert!(
        said.contains("Yours: Standard Doctrine (Sniper, Line).")
            && said.contains("Your mind will seal at Class 0."),
        "{said}"
    );
    // One armed fill beside the coreless pilot keeps both sides fieldable.
    rig.say(0, "sim ai 2");
    let said = rig.say(0, "sim start");
    assert!(said.contains("boarding"), "{said}");
    let unit = match_::tank_of(rig.tw.world(), rig.pod(0).expect("the pilot's pod"))
        .expect("the coreless tank");
    assert!(rig.seat(0));
    assert_eq!(rig.state(), MatchState::Running);
    let seal = sealed_of(&rig, unit);
    assert_eq!(seal.core, None);
    assert_eq!(seal.class, tacsim::arena::core::CoreClass::Zero);
    assert_eq!(seal.engine, tacsim::arena::SealEngine::Native);
    assert_eq!(seal.fallback.as_deref(), Some(match_::NO_CORE_FALLBACK));
    assert_eq!(
        seal.cards, seal.rom,
        "actual execution uses the captured ROM"
    );
    assert_eq!(seal.cards.selected.names().collect::<Vec<_>>(), ["Line"]);
}

/// **`SIM AI MIND` is the lead's, and reads to the lead alone** (17b cut
/// M5.1; doc 17 §12, §19; D9, D-UM-49; the design note's §2.4), on a server
/// with the fill on:
///
/// * red's second claimant may not set red's fill (`NotLeadForAi`); its lead
///   may, with ONE profile, from its own holdings — `Red's fill will carry
///   Standard Doctrine (Hunter).`, said to the lead alone;
/// * the bare read: the lead reads red's choice and blue's `nothing chosen`;
///   red's other claimant `Red's fill carries a mind.`;
/// * staff lead no side and hold no minds: a set with no side asks which,
///   `Fixture I` is refused with the staff sentence, and `SIM AI MIND blue
///   Standard PROFILE Sentry` sets blue's; staff read both;
/// * (17b cut M5.1's review) a side word alone reads that side's line as
///   far as the reader may know it, and the lead's clear is told to nobody
///   else;
/// * with the test provider, the lead may choose `Fixture I` for red's fill.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_sim_ai_mind_is_the_leads_and_reads_to_the_lead_alone() {
    let mut rig = fill_rig(2);
    god_in_the_lobby(&mut rig);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    assert_eq!(
        rig.say(1, "sim ai mind Standard PROFILE Hunter"),
        "Only a side's lead or staff can set the AI fill."
    );
    assert_eq!(
        rig.say(0, "sim ai mind Standard PROFILE Hunter, Sniper"),
        "The fill flies one profile; nobody aboard switches it."
    );
    let heard = rig.say_overheard(0, "sim ai mind Standard PROFILE Hunter", 1);
    assert!(
        heard.is_empty(),
        "the choice is told to nobody else: {heard}"
    );
    assert_eq!(
        rig.say(0, "sim ai mind Standard PROFILE Hunter"),
        "Red's fill will carry Standard Doctrine (Hunter)."
    );
    assert_eq!(
        rig.say(0, "sim ai mind"),
        "Red's fill: Standard Doctrine (Hunter).\nBlue's fill: nothing chosen."
    );
    assert_eq!(
        rig.say(1, "sim ai mind"),
        "Red's fill carries a mind.\nBlue's fill: nothing chosen."
    );
    assert_eq!(
        rig.staff("sim ai mind Standard PROFILE Sentry"),
        "Which side's fill? SIM AI MIND red Standard sets red's."
    );
    assert_eq!(
        rig.staff("sim ai mind blue Fixture I"),
        "Staff hold no minds; SIM AI MIND Standard sets the fill's doctrine and preset."
    );
    assert_eq!(
        rig.staff("sim ai mind blue Standard PROFILE Sentry"),
        "Blue's fill will carry Standard Doctrine (Sentry)."
    );
    assert_eq!(
        rig.staff("sim ai mind"),
        "Red's fill: Standard Doctrine (Hunter).\nBlue's fill: Standard Doctrine (Sentry)."
    );
    assert_eq!(
        rig.say(0, "sim ai mind"),
        "Red's fill: Standard Doctrine (Hunter).\nBlue's fill carries a mind."
    );
    // 17b cut M5.1's review: a side word alone reads that side's line —
    // never a mind called `red` — as far as the reader may know it.
    assert_eq!(
        rig.say(0, "sim ai mind red"),
        "Red's fill: Standard Doctrine (Hunter)."
    );
    assert_eq!(
        rig.say(1, "sim ai mind BLUE"),
        "Blue's fill carries a mind."
    );
    assert_eq!(
        rig.staff("sim ai mind blue"),
        "Blue's fill: Standard Doctrine (Sentry)."
    );
    // And a clear is told to nobody else either.
    let (said, heard) = rig.say_heard(0, "sim ai mind none", 1);
    assert_eq!(
        said,
        "Red's fill: nothing chosen; it flies Standard Doctrine on Line."
    );
    assert!(heard.is_empty(), "a clear is told to nobody else: {heard}");
    let _provider = holdings_on(&mut rig);
    assert_eq!(
        rig.say(0, "sim ai mind Fixture I"),
        "Red's fill will carry Fixture I (Line)."
    );
    assert_eq!(
        rig.staff("sim ai mind red Fixture I"),
        "Staff hold no minds; SIM AI MIND Standard sets the fill's doctrine and preset.",
        "staff hold no minds whatever a provider holds"
    );
}

/// **The bare reads follow the state** (17b cut M5.1; the design note's §2.4,
/// row by row), on a server with the fill on: Ada (red's lead, who chooses
/// her slot's mind and her fill's), Bo (blue's lead, who chooses nothing),
/// Cy (a bystander holding no slot) and staff — in `Idle`, `Selection`,
/// `Boarding` and `Running`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_bare_reads_follow_the_state() {
    let (ada, bo, cy) = (0, 1, 2);
    let mut rig = fill_rig(3);
    god_in_the_lobby(&mut rig);
    let hold = "You hold Standard Doctrine (six presets); claim a slot and SIM MIND lists what fits its \
         core.";
    let unchosen_fill = "Red's fill: nothing chosen; it flies Standard Doctrine on Line.\nBlue's \
                         fill: nothing chosen; it flies Standard Doctrine on Line.";
    let nothing = "Red's fill: nothing chosen.\nBlue's fill: nothing chosen.";

    // Idle.
    for n in [ada, bo, cy] {
        assert_eq!(rig.say(n, "sim mind"), hold, "idle, pilot {n}");
        assert_eq!(rig.say(n, "sim ai mind"), nothing, "idle, pilot {n}");
    }
    assert_eq!(rig.staff("sim ai mind"), unchosen_fill, "idle, staff");

    // Selection.
    rig.say(ada, "sim team red");
    rig.say(bo, "sim team blue");
    rig.say(ada, "sim mind Standard PROFILE Sniper, Line");
    rig.say(ada, "sim ai mind Standard PROFILE Hunter");
    let said = rig.say(ada, "sim mind");
    assert!(
        said.starts_with("Yours: Standard Doctrine (Sniper, Line).\nFits line_tank (II):"),
        "{said}"
    );
    assert!(rig.say(bo, "sim mind").starts_with(UNCHOSEN));
    assert_eq!(rig.say(cy, "sim mind"), hold);
    assert_eq!(
        rig.say(ada, "sim ai mind"),
        "Red's fill: Standard Doctrine (Hunter).\nBlue's fill: nothing chosen."
    );
    assert_eq!(
        rig.say(bo, "sim ai mind"),
        "Red's fill carries a mind.\nBlue's fill: nothing chosen; it flies Standard Doctrine on \
         Line."
    );
    assert_eq!(
        rig.say(cy, "sim ai mind"),
        "Red's fill carries a mind.\nBlue's fill: nothing chosen."
    );
    assert_eq!(
        rig.staff("sim ai mind"),
        "Red's fill: Standard Doctrine (Hunter).\nBlue's fill: nothing chosen; it flies Standard \
         Doctrine on Line."
    );

    // Boarding, then Running: the same reads.
    rig.say(ada, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    for running in [false, true] {
        if running {
            assert!(!rig.seat(ada));
            assert!(rig.seat(bo));
            assert_eq!(rig.state(), MatchState::Running);
        }
        let state = if running { "running" } else { "boarding" };
        assert_eq!(
            rig.say(ada, "sim mind"),
            "Yours: Standard Doctrine (Sniper, Line); selection is closed.",
            "{state}"
        );
        assert_eq!(
            rig.say(bo, "sim mind"),
            "Yours: none; selection is closed.",
            "{state}"
        );
        assert_eq!(
            rig.say(cy, "sim mind"),
            "You hold Standard Doctrine (six presets); selection is closed.",
            "{state}"
        );
        assert_eq!(
            rig.say(ada, "sim ai mind"),
            "Red's fill: Standard Doctrine (Hunter).",
            "{state}: the setter reads her own"
        );
        for n in [bo, cy] {
            assert_eq!(
                rig.say(n, "sim ai mind"),
                "The fills are not shown once a match starts.",
                "{state}, pilot {n}"
            );
        }
        assert_eq!(
            rig.staff("sim ai mind"),
            "Red's fill: Standard Doctrine (Hunter).\nBlue's fill: nothing chosen; it flies \
             Standard Doctrine on Line.",
            "{state}, staff"
        );
    }
}

/// **A change of lead clears the side's fill choice** (17b cut M5.1; the
/// design note's §2.4, the adversarial review's B4): red's lead sets red's
/// fill and leaves with `SIM LEAVE`; red's next claimant leads and reads
/// `Red's fill: nothing chosen; it flies Standard Doctrine on Line.` — no new
/// lead reads or inherits another account's choice — while a staff choice
/// for blue stays; and the check-out's `Side::minds` holds no red fill
/// choice: read through `lobby::ready`'s test hook once the new lead is
/// seated and blue's pilot is not, every red hull — the lead's and the fill's
/// — is `SealChoice::unchosen()`, with every fielded hull's exact Class I
/// preview (including the seated pilot's default), and blue's fill hull
/// carries the staff's Standard Doctrine (Sentry), chosen, at Class I (17b
/// cut M5.1's review: the test read `Centre::fill_minds` after `Running`
/// before it, the input and not the check-out's sides).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_change_of_lead_clears_the_sides_fill_choice() {
    use tacsim::arena::core::CoreClass;
    use tacsim::match_::holdings::{HeldMind, SealChoice};
    let mut rig = fill_rig(3);
    god_in_the_lobby(&mut rig);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(2, "sim team blue");
    assert_eq!(
        rig.say(0, "sim ai mind Standard PROFILE Hunter"),
        "Red's fill will carry Standard Doctrine (Hunter)."
    );
    rig.staff("sim ai mind blue Standard PROFILE Sentry");
    assert_eq!(rig.centre().fill_minds.len(), 2);
    rig.say(0, "sim leave");
    assert!(
        rig.centre().leads_side(PartyId::RED, account(1)),
        "red's next claimant leads"
    );
    assert_eq!(
        rig.say(1, "sim ai mind"),
        "Red's fill: nothing chosen; it flies Standard Doctrine on Line.\nBlue's fill carries a \
         mind."
    );
    let fills = rig.centre().fill_minds;
    assert_eq!(fills.len(), 1, "{fills:?}");
    assert!(
        fills
            .iter()
            .all(|fill| fill.team == PartyId::BLUE && fill.by == match_::lobby::FillSetter::Staff),
        "the staff choice stays: {fills:?}"
    );
    rig.say(1, "sim ai 2");
    let said = rig.say(1, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert!(!rig.seat(1), "blue's pilot is not seated yet");

    let sides = rig.ready_sides().expect("the check-out would run");
    let red = side_of(&sides, PartyId::RED);
    assert_eq!(red.units.len(), 2, "the new lead's hull and one fill hull");
    assert_eq!(
        red.minds.keys().copied().collect::<Vec<ObjId>>(),
        {
            let mut units: Vec<ObjId> = red.units.iter().map(|fielded| fielded.unit).collect();
            units.sort();
            units
        },
        "one choice per red hull"
    );
    for fielded in &red.units {
        assert_eq!(
            fielded.mind_class,
            Some(CoreClass::I),
            "every fielded hull previews the actual sealed class, including an unchosen pilot"
        );
        assert_eq!(
            red.minds.get(&fielded.unit),
            Some(&SealChoice::unchosen()),
            "slot {}: no red fill choice reaches the check-out",
            fielded.slot
        );
    }
    let blue = side_of(&sides, PartyId::BLUE);
    assert_eq!(blue.units.len(), 2, "blue's pilot's hull and one fill hull");
    let pilot_tank = match_::tank_of(rig.tw.world(), rig.pod(2).expect("blue's locked pod"))
        .expect("the pod's tank");
    for fielded in &blue.units {
        if fielded.unit == pilot_tank {
            assert_eq!(fielded.mind_class, Some(CoreClass::I));
            assert_eq!(blue.minds.get(&fielded.unit), Some(&SealChoice::unchosen()));
        } else {
            assert_eq!(fielded.mind_class, Some(CoreClass::I), "the staff's fill");
            assert_eq!(
                blue.minds.get(&fielded.unit),
                Some(&SealChoice {
                    held: HeldMind::doctrine(),
                    profiles: vec![smol_str::SmolStr::new_static("Sentry")],
                    chose: true,
                    substituted: false,
                }),
                "the staff choice reaches the check-out"
            );
        }
    }
    assert!(rig.seat(2));
    assert_eq!(rig.state(), MatchState::Running);
}

/// **The check-out reads each hull's choice** (17b cut M5.1's review; the
/// design note's M5.1 item 6): what `lobby::ready` writes, read through its
/// test hook with Ada seated and Bo not, on a server with the fill on and the
/// test provider installed:
///
/// * Ada's slot chose `Fixture I` (Sniper) on its `line_tank`: **since 17b
///   cut M5.2** (Q-UM-15, the owner's OQ-5) humans hold both sides, so her
///   hull's `Side::minds` entry is the substitution — Standard Doctrine on
///   Line, `chose` and `substituted` — and its `Fielded::mind_class` the
///   class Standard Doctrine seals at there (I). Was `Fixture I` as the
///   provider holds it, its profiles, never `substituted`, at 17b cut M5.1,
///   which built no substitution; the unsubstituted seal of `Fixture I` is
///   `m5_check_out_seals_the_slots_choice_and_the_leads_fill`'s, on a match
///   with humans on one side;
/// * red's lead chose its fill Standard Doctrine (Hunter): under `SIM AI 3`
///   each of red's two fill hulls carries that choice, chosen, at Class I;
/// * Bo chose nothing, and blue's fill was chosen by nobody: each blue hull
///   is `SealChoice::unchosen()`, with each unclaimed fill hull explicitly
///   previewed at Class I;
/// * one entry per fielded hull, keyed by the hull — and the check-out then
///   runs on them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_check_out_reads_each_hulls_choice() {
    use smol_str::SmolStr;
    use tacsim::arena::core::CoreClass;
    use tacsim::match_::holdings::{HeldMind, SealChoice};
    let (ada, bo) = (0, 1);
    let mut rig = fill_rig(2);
    let _provider = holdings_on(&mut rig);
    rig.say(ada, "sim team red");
    rig.say(bo, "sim team blue");
    assert_eq!(
        rig.say(ada, "sim mind Fixture I PROFILE Sniper"),
        "Your slot will carry Fixture I (Sniper)."
    );
    assert_eq!(
        rig.say(ada, "sim ai mind Standard PROFILE Hunter"),
        "Red's fill will carry Standard Doctrine (Hunter)."
    );
    rig.say(ada, "sim ai 3");
    let said = rig.say(ada, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert!(!rig.seat(ada), "Bo is not seated yet");

    let sides = rig.ready_sides().expect("the check-out would run");
    let tank_of = |rig: &Rig, n: usize| {
        match_::tank_of(rig.tw.world(), rig.pod(n).expect("a locked pod")).expect("its tank")
    };
    // 17b cut M5.2 (Q-UM-15): Ada's `Fixture I (Sniper)` is substituted —
    // Standard Doctrine on Line — with humans on both sides.
    assert!(rig.centre().doctrine_only, "humans on both sides");
    for (team, pilot, chosen) in [
        (PartyId::RED, tank_of(&rig, ada), Some(HeldMind::doctrine())),
        (PartyId::BLUE, tank_of(&rig, bo), None),
    ] {
        let side = side_of(&sides, team);
        assert_eq!(
            side.units.len(),
            3,
            "{team:?}: a pilot's hull and two fill hulls"
        );
        let mut units: Vec<ObjId> = side.units.iter().map(|fielded| fielded.unit).collect();
        units.sort();
        assert_eq!(
            side.minds.keys().copied().collect::<Vec<ObjId>>(),
            units,
            "{team:?}: one choice per hull, keyed by the hull"
        );
        for fielded in &side.units {
            let (class, choice) = match (fielded.unit == pilot, &chosen, team) {
                (true, Some(held), _) => (
                    Some(CoreClass::I),
                    SealChoice {
                        held: held.clone(),
                        profiles: Vec::new(),
                        chose: true,
                        substituted: true,
                    },
                ),
                (false, _, PartyId::RED) => (
                    Some(CoreClass::I),
                    SealChoice {
                        held: HeldMind::doctrine(),
                        profiles: vec![SmolStr::new_static("Hunter")],
                        chose: true,
                        substituted: false,
                    },
                ),
                (false, _, PartyId::BLUE) => (Some(CoreClass::I), SealChoice::unchosen()),
                _ => (None, SealChoice::unchosen()),
            };
            assert_eq!(
                fielded.mind_class, class,
                "{team:?} slot {}: the admitted class",
                fielded.slot
            );
            assert_eq!(
                side.minds.get(&fielded.unit),
                Some(&choice),
                "{team:?} slot {}: the choice",
                fielded.slot
            );
        }
    }
    assert!(rig.seat(bo));
    assert_eq!(
        rig.state(),
        MatchState::Running,
        "the check-out ran on them"
    );
}

/// Checkout charges the actual sealed ROM class on a coreless target,
/// independently of whether its pilot explicitly chose Standard. Preview,
/// ready sides and the final custody seal must agree under the same low pool.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_check_out_admits_a_chosen_hull_at_the_class_its_mind_seals_at() {
    use tacsim::arena::core::CoreClass;
    let (ada, cy) = (0, 1);
    let mut lowered = fill_config();
    lowered.minds.pool_base_ops_s = 3_812;
    lowered.minds.pool_per_unit_ops_s = 1;
    lowered.minds.pool_floor_ops_s = 1;
    assert_eq!(tacsim::match_::pool::pool_for(&lowered.minds, 4), 3_808);
    for ada_chooses in [true, false] {
        let mut rig = Rig::boot_on(2, roster_with(lowered.clone()));
        rig.tick_to(1);
        for (n, team) in [(ada, "red"), (cy, "blue")] {
            assert!(rig.say(n, &format!("sim team {team}")).contains("takes"));
            let said = rig.say(n, "sim unit target_mk1");
            assert!(said.contains("will field target_mk1."), "{said}");
            if n == cy || ada_chooses {
                assert_eq!(
                    rig.say(n, "sim mind Standard"),
                    "Your slot will carry Standard Doctrine (Line)."
                );
            }
        }
        let said = rig.say(ada, "sim ai 2");
        assert!(!said.contains("would think"), "{said}");
        for team in PartyId::OOC {
            assert_eq!(preview_class(&rig, team, 1), CoreClass::Zero);
            assert_eq!(preview_class(&rig, team, 2), CoreClass::I);
        }
        let said = rig.say(ada, "sim start");
        assert!(said.contains("boarding"), "{said}");
        assert!(!rig.seat(ada), "Cy is not seated yet");
        let sides = rig.ready_sides().expect("the exact roster fits");
        assert_eq!(
            side_of(&sides, PartyId::RED)
                .units
                .iter()
                .map(|fielded| (fielded.slot, fielded.mind_class))
                .collect::<Vec<_>>(),
            [(1, Some(CoreClass::Zero)), (2, Some(CoreClass::I))],
            "chosen and default pilots expose the same exact sealed class"
        );
        assert!(
            rig.seat(cy),
            "both chosen and unchosen ROM rosters check out"
        );
        assert_eq!(rig.state(), MatchState::Running);
        // Before Cy seated, the prospective blue side filled other unclaimed
        // pods. Compare custody against the final fully seated roster instead.
        let final_sides = rig.ready_sides().expect("the fully seated roster fits");
        let id = rig.centre().match_id.expect("running match");
        let record = rig.book().get(id).expect("the immutable match record");
        assert_eq!(record.minds.len(), 4, "one seal for every actual hull");
        for side in &final_sides {
            assert_eq!(
                side.units
                    .iter()
                    .map(|fielded| (fielded.slot, fielded.mind_class))
                    .collect::<Vec<_>>(),
                [(1, Some(CoreClass::Zero)), (2, Some(CoreClass::I))],
                "each final side has its coreless pilot and one Class I fill"
            );
            let mut load = 0;
            for fielded in &side.units {
                let seal = &record
                    .minds
                    .get(&fielded.unit)
                    .expect("each hull is sealed")
                    .seal;
                let expected = if fielded.slot == 1 {
                    CoreClass::Zero
                } else {
                    CoreClass::I
                };
                assert_eq!(seal.class, expected, "the actual seal determines admission");
                load += lowered.minds.classes.ceiling_per_s(seal.class);
            }
            assert_eq!(load, 1_900);
            assert!(load <= 1_904);
        }
    }
}

/// **The board names a slot's mind to its own character alone in
/// selection** (17b cut M5.1; the design note's §2.4; the
/// `q8_the_board_prints_a_chosen_type_only_during_selection` analogue), on a
/// server with the fill on: Ada chooses her slot's mind and red's fill's; Bo
/// (blue) and Cy (no slot) read `, a mind` against her row and `red fill: a
/// mind`; Bo's own unchosen row prints nothing to anybody; staff read both
/// names. From `SIM START` on nobody reads a mind on the board, staff
/// included.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_board_names_a_slots_mind_to_its_own_character_alone_in_selection() {
    let (ada, bo, cy) = (0, 1, 2);
    let mut rig = fill_rig(3);
    rig.say(ada, "sim team red");
    rig.say(bo, "sim team blue");
    rig.say(ada, "sim mind Standard PROFILE Sniper, Line");
    rig.say(ada, "sim ai mind Standard PROFILE Hunter");

    let board = rig.say(ada, "sim status");
    assert!(
        board.contains("  red: 1 (RA): Pilot0, mind Standard Doctrine (Sniper, Line)\n"),
        "{board}"
    );
    assert!(
        board.contains("  red fill: Standard Doctrine (Hunter)\n"),
        "{board}"
    );
    assert!(board.contains("  blue: 1 (BA): Pilot1\n"), "{board}");
    for n in [bo, cy] {
        let board = rig.say(n, "sim status");
        assert!(
            board.contains("  red: 1 (RA): Pilot0, a mind\n"),
            "{n}: {board}"
        );
        assert!(board.contains("  red fill: a mind\n"), "{n}: {board}");
        assert!(board.contains("  blue: 1 (BA): Pilot1\n"), "{n}: {board}");
        assert!(
            !board.contains("Sniper") && !board.contains("Hunter") && !board.contains("Doctrine"),
            "{n}: {board}"
        );
    }
    let staff = rig.staff("@arena/status");
    assert!(
        staff.contains("1 (RA): Pilot0, mind Standard Doctrine (Sniper, Line)"),
        "{staff}"
    );
    assert!(
        staff.contains("red fill: Standard Doctrine (Hunter)"),
        "{staff}"
    );

    rig.say(ada, "sim start");
    assert_eq!(rig.state(), MatchState::Boarding);
    // After the start no row names a mind by ANY word — not `mind`, not
    // `Doctrine`, not one of the six presets — for any reader, the slot's
    // own pilot and staff included (the review of 17b cut M5.7: a row that
    // named the mind to its own pilot without the word `mind` passed).
    let names_no_mind = |who: &str, board: &str| {
        let words: Vec<&str> = board
            .split(|c: char| !c.is_ascii_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        for name in std::iter::once("Doctrine").chain(tacsim::arena::card::PRESETS) {
            assert!(!words.contains(&name), "{who} reads {name}: {board}");
        }
    };
    for n in [ada, bo, cy] {
        let board = rig.say(n, "sim status");
        assert!(!board.contains("mind"), "{n}: {board}");
        names_no_mind(&n.to_string(), &board);
        assert!(
            !board.contains("fill:") || board.contains("AI fill:"),
            "{n}: {board}"
        );
    }
    let staff = rig.staff("@arena/status");
    assert!(
        !staff.contains("mind") && !staff.contains("Sniper"),
        "{staff}"
    );
    names_no_mind("staff", &staff);
}

/// **Every `SIM MIND` line is under 120 columns on every fieldable type**
/// (17b cut M5.1; the design note's M5.1 item 3, the adversarial review's
/// B10): with the test provider installed, a pilot fields every type `SIM
/// UNIT` lists, in turn, and every line of the bare `SIM MIND` on it is under
/// 120 columns and carries no sixteen-hex-digit word; then every sentence of item
/// 3 is measured at its widest — every type word in the `Fits` line at the
/// widest class word and count word with all six presets (and, since the
/// review, in its `(ROM)` form and the coreless profile-count refusal), the
/// widest preset pair (`Skirmisher, Rearguard`), six-figure pool terms, the
/// widest fill preset, every refusal, and the last BOARDer's reply when the
/// check-out's pool refuses.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_every_sim_mind_line_is_under_120_columns_on_every_fieldable_type() {
    use smol_str::SmolStr;
    use tacsim::arena::core::CoreClass;
    use tacsim::match_::lobby::{self, SimError};
    fn sixteen_hex(line: &str) -> bool {
        line.split(|c: char| !c.is_ascii_alphanumeric())
            .any(|word| word.len() >= 16 && word.chars().all(|c| c.is_ascii_hexdigit()))
    }
    let mut measured: Vec<String> = Vec::new();
    let mut rig = Rig::boot(1);
    let _provider = holdings_on(&mut rig);
    let held = rig.say(0, "sim mind");
    assert_eq!(
        held,
        "You hold Standard Doctrine, Fixture I and Load III; claim a slot and SIM MIND lists \
         what fits its core.",
        "several minds by their names"
    );
    measured.push(held);
    rig.say(0, "sim team red");
    let list = rig.say(0, "sim unit");
    let types: Vec<String> = list
        .strip_prefix("Fieldable units: ")
        .and_then(|rest| rest.split_once(". Yours:"))
        .map(|(types, _)| types.split(", ").map(str::to_owned).collect())
        .unwrap_or_default();
    assert_eq!(types.len(), 11, "{list}");
    for word in &types {
        let said = rig.say(0, &format!("sim unit {word}"));
        assert!(said.contains("will field"), "{word}: {said}");
        let read = rig.say(0, "sim mind");
        assert!(read.contains("Fits "), "{word}: {read}");
        measured.extend(read.lines().map(str::to_owned));
    }
    let six: Vec<SmolStr> = [
        "Line",
        "Sentry",
        "Sniper",
        "Skirmisher",
        "Hunter",
        "Rearguard",
    ]
    .into_iter()
    .map(SmolStr::new_static)
    .collect();
    let widest_pair = [
        SmolStr::new_static("Skirmisher"),
        SmolStr::new_static("Rearguard"),
    ];
    let pair = lobby::mind_words("Standard Doctrine", &widest_pair);
    for word in &types {
        measured.push(lobby::fits_line(
            word,
            Some(CoreClass::III),
            "Standard Doctrine",
            3,
            &six,
        ));
        // 17b cut M5.1's review: a chassis with no core flies the ROM, and
        // its `Fits` bracket and profile-count refusal say so.
        measured.push(lobby::fits_line(word, None, "Standard Doctrine", 3, &six));
        measured.push(
            SimError::TooManyProfiles {
                mind: "Standard Doctrine".to_owned(),
                class: CoreClass::Zero,
                most: 3,
                rom: Some(word.clone()),
            }
            .to_string(),
        );
        measured.push(
            SimError::OverClass {
                mind: "Standard Doctrine".to_owned(),
                rated: CoreClass::III,
                chassis: word.clone(),
                core: CoreClass::III,
            }
            .to_string(),
        );
    }
    for team in [PartyId::RED, PartyId::BLUE] {
        measured.push(lobby::pool_line(team, 999_999, 999_999));
    }
    measured.extend([
        lobby::MIND_UNCHOSEN.to_owned(),
        lobby::MIND_CLEARED.to_owned(),
        lobby::FILLS_HIDDEN.to_owned(),
        format!("Yours: {pair}."),
        format!("Yours: {pair}; selection is closed."),
        "Yours: none; selection is closed.".to_owned(),
        format!("Your slot will carry {pair}."),
        "You hold Standard Doctrine (six presets); claim a slot and SIM MIND lists what fits its \
         core."
            .to_owned(),
        format!(
            "You hold {}; claim a slot and SIM MIND lists what fits its core.",
            lobby::holdings_words(&[
                tacsim::match_::holdings::HeldMind::doctrine(),
                support::holdings::fixture_i(),
                support::holdings::load_iii(),
            ])
        ),
        "Blue's fill will carry Standard Doctrine (Skirmisher).".to_owned(),
        "Blue's fill: Standard Doctrine (Skirmisher).".to_owned(),
        "Blue's fill: nothing chosen; it flies Standard Doctrine on Line.".to_owned(),
        "Blue's fill carries a mind.".to_owned(),
        "AI fill: none on a range.".to_owned(),
        "Which profiles? `SIM MIND Standard PROFILE Sniper, Line`.".to_owned(),
        "Which mind? `SIM MIND Standard`, or `SIM MIND none`.".to_owned(),
        "Which profile? `SIM AI MIND Standard PROFILE Hunter`.".to_owned(),
        "Which mind? `SIM AI MIND Standard`, or `SIM AI MIND none`.".to_owned(),
        format!(
            "Match 999 could not start: {}. The pods are free again.",
            SimError::MindNotHeld {
                team: "blue",
                index: Some(6),
            }
            .to_string()
            .trim_end_matches('.')
        ),
        format!(
            "Match 999 could not start: {}. The pods are free again.",
            SimError::MindNotHeld {
                team: "blue",
                index: None,
            }
            .to_string()
            .trim_end_matches('.')
        ),
    ]);
    for error in [
        SimError::NoSuchMind {
            word: "Sentinel".to_owned(),
        },
        SimError::NoRevisions,
        SimError::NoSuchProfile {
            mind: "Standard Doctrine".to_owned(),
            word: "Brawler".to_owned(),
            presets: "Line, Sentry, Sniper, Skirmisher, Hunter or Rearguard".to_owned(),
        },
        SimError::ProfileTwice {
            profile: "Skirmisher".to_owned(),
        },
        SimError::TooManyProfiles {
            mind: "Standard Doctrine".to_owned(),
            class: CoreClass::III,
            most: 3,
            rom: None,
        },
        SimError::UnderFloor {
            profile: "Skirmisher".to_owned(),
            odds_permille: 950,
            floor_permille: 950,
        },
        SimError::ProfilePair {
            profile: "Rearguard".to_owned(),
            fight_home: "77.5".to_owned(),
            break_off: "70".to_owned(),
        },
        SimError::FillOneProfile,
        SimError::StaffHoldNoMinds,
        SimError::FillWhichSide,
        SimError::Pool(tacsim::match_::pool::PoolRefusal {
            would: 999_999,
            half: 999_999,
            spoken: 999_999,
        }),
        // 17b cut M5.1's review: what the last BOARDer reads when the
        // check-out's pool refuses — the call-off's short form, as one
        // sentence (`lobby::run`), at six-figure terms.
        SimError::Start(format!(
            "{}.",
            tacsim::match_::StartError::Pool {
                team: "blue",
                unit: ObjId(999_999),
                callsign: "Spare Hull 999".to_owned(),
                refusal: tacsim::match_::pool::PoolRefusal {
                    would: 999_999,
                    half: 999_999,
                    spoken: 999_999,
                },
            }
            .call_off_words(rig.tw.world(), &[])
        )),
    ] {
        measured.push(error.to_string());
    }
    for line in &measured {
        assert!(
            line.chars().count() < 120,
            "{} columns: {line}",
            line.chars().count()
        );
        assert!(!sixteen_hex(line), "a sixteen-hex-digit word: {line}");
    }
    assert!(measured.len() > 60, "{}", measured.len());
}

/// **`SIM MIND` and `SIM AI MIND` reuse the lobby's refusals** (17b cut M5.1;
/// the design note's §2.4 and M5.1 item 2): a `SIM MIND` set in `SIM UNIT`'s
/// order — no slot in `Idle` and `Selection` is `NoSlotToField`, outside
/// `Selection` `WrongState` — and `SIM AI MIND` in `SIM AI`'s — the fill off
/// is `AiOff` for every form, a range reads `AI fill: none on a range.` and
/// refuses a set with `AiOnRange` (never `AiOff`), outside `Selection` a set
/// is `WrongState`, and a pilot who leads no side is `NotLeadForAi`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_sim_mind_and_sim_ai_mind_reuse_the_lobby_refusals() {
    let no_slot = "Claim a slot first: SIM TEAM red|blue.";
    // The fill off.
    let mut rig = Rig::boot(3);
    assert_eq!(rig.say(0, "sim mind Standard"), no_slot, "idle");
    for line in ["sim ai mind", "sim ai mind Standard", "sim ai mind none"] {
        assert_eq!(rig.say(0, line), "AI fill is off on this server.", "{line}");
    }
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    assert_eq!(rig.say(2, "sim mind Standard"), no_slot, "selection");
    assert_eq!(rig.say(2, "sim mind none"), no_slot, "selection");
    rig.say(0, "sim start");
    let boarding = "The simulator centre is boarding; that is not something you can do to it now.";
    assert_eq!(rig.say(0, "sim mind Standard"), boarding);
    assert_eq!(rig.say(0, "sim mind none"), boarding);
    assert_eq!(rig.say(2, "sim mind Standard"), boarding, "no slot");

    // The fill on.
    let mut rig = fill_rig(2);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    assert_eq!(
        rig.say(1, "sim ai mind Standard"),
        "Only a side's lead or staff can set the AI fill."
    );
    rig.say(0, "sim range 1");
    assert_eq!(rig.say(0, "sim ai mind"), "AI fill: none on a range.");
    for line in ["sim ai mind Standard", "sim ai mind none"] {
        assert_eq!(
            rig.say(0, line),
            "The range fields no machines; SIM RANGE sets its targets.",
            "{line}"
        );
    }
    rig.say(0, "sim mode skirmish");
    rig.say(0, "sim start");
    assert_eq!(
        rig.say(0, "sim ai mind Standard"),
        "The simulator centre is boarding; that is not something you can do to it now."
    );
}

/// **A holding lost before the check-out calls the match off by slot** (17b
/// cut M5.1; the design note's M5.1 item 6): Ada chooses `Fixture I` (the
/// test provider's) on her `line_tank`; the provider drops it between the
/// choice and the check-out; the last seat fires the check-out, which refuses
/// with `SimError::MindNotHeld` and calls the match off through `run`'s
/// `ready` arm — `Match 1 could not start: Slot red 1's chosen mind is no
/// longer held by its pilot. The pods are free again.` — said to the centre,
/// naming the slot and never the mind (the `d3_`/`ae3_` call-off pattern:
/// the seat reported through `match_::slot_seated`, what the pilot heard read
/// off its own outbox) — read by the seating pilot and, since the review, by
/// Ada in the lobby.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_holding_lost_before_check_out_calls_the_match_off_by_slot() {
    let mut rig = Rig::boot(2);
    let provider = holdings_on(&mut rig);
    rig.say(0, "sim team red");
    rig.say(1, "sim team blue");
    assert_eq!(
        rig.say(0, "sim mind Fixture I"),
        "Your slot will carry Fixture I (Line)."
    );
    rig.say(0, "sim start");
    assert!(!rig.seat(0), "red is in and blue is not");
    provider.drop_fixture_i();
    let pilot = rig.pilot(1);
    let pod = rig.pod(1).expect("a locked pod");
    let unit = match_::tank_of(rig.tw.world(), pod).expect("the pod's tank");
    let match_id = rig
        .centre()
        .match_id
        .expect("a boarding centre has a match id");
    let mut out = Outbox::new();
    let refused = {
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
    };
    assert_eq!(
        refused,
        Err(match_::SimError::MindNotHeld {
            team: "red",
            index: Some(1),
        })
    );
    let events = out.take();
    let call_off = "Match 1 could not start: Slot red 1's chosen mind is no longer held by its \
                    pilot. The pods are free again.";
    // The pilot whose seat fired the check-out, and (17b cut M5.1's review)
    // Ada — the centre, to whom the call-off is said.
    for n in [1, 0] {
        let heard = rig.seen(rig.pilot(n), &events);
        assert!(heard.contains(call_off), "pilot {n}: {heard}");
        assert!(
            !heard.contains("Fixture"),
            "pilot {n}: the call-off names no mind: {heard}"
        );
    }
    assert_eq!(rig.state(), MatchState::Aborted);
    assert!(rig.centre().slots.is_empty(), "the slots are given back");
    assert_eq!(
        match_::SimError::MindNotHeld {
            team: "red",
            index: None,
        }
        .to_string(),
        "The red fill's chosen mind is no longer held by its lead."
    );
}

// ---------------------------------------------------------------------------
// 17b cut M5.2: the seal carries the choice — the check-out reads
// `Side::minds`, Q-UM-15's substitution, the seal line
// ---------------------------------------------------------------------------

/// **Board pilot `n`** from their team room's `entry` and answer what each
/// pilot of `heard` saw of that one `BOARD` (the actor's own reply among
/// them when `n` is listed).
fn board_heard(rig: &mut Rig, n: usize, entry: &str, heard: &[usize]) -> Vec<String> {
    let room = match_::centre_object(rig.tw.world(), entry).expect("a team room");
    let pilot = rig.pilot(n);
    rig.tw
        .world_mut()
        .move_to(pilot, room)
        .expect("a pilot walks into their team room");
    let _ = rig.pump_ops();
    let events = rig.tw.run(pilot, "board");
    heard
        .iter()
        .map(|other| rig.seen(rig.pilot(*other), &events))
        .collect()
}

/// The seal the running match's record keeps for `unit`.
fn sealed_of(rig: &Rig, unit: ObjId) -> tacsim::arena::MindSeal {
    let match_id = rig.centre().match_id.expect("a match id");
    rig.book()
        .get(match_id)
        .and_then(|record| record.minds.get(&unit))
        .map(|sealed| sealed.seal.clone())
        .expect("a seal per unit")
}

/// Re-admit the full sealed definition, verify its opening canonical payload and
/// digest, and read each selected profile's exact stock adapter in sealed order.
fn sealed_stock_cards(seal: &tacsim::arena::MindSeal) -> Vec<(String, tacsim::arena::StockCard)> {
    use sha2::{Digest as _, Sha256};
    let mut catalog = tacsim::arena::runtime_card::CardCatalog::default();
    let (profiles, opening) = seal
        .cards
        .admit(&mut catalog)
        .expect("sealed definition admits");
    assert_eq!(opening.canonical_bytes(), Some(seal.card.as_slice()));
    assert_eq!(
        seal.card_digest,
        <[u8; 32]>::from(Sha256::digest(&seal.card))
    );
    profiles
        .names()
        .enumerate()
        .map(|(index, name)| {
            let key = profiles
                .key(u8::try_from(index).expect("bounded profile count"))
                .expect("selected key");
            let card = catalog.resolve(key).expect("selected complete profile");
            (
                name.to_owned(),
                card.stock()
                    .expect("fixture uses the full stock adapter")
                    .clone(),
            )
        })
        .collect()
}

/// The units of `team` in the running match's record.
fn units_of(rig: &Rig, team: PartyId) -> Vec<ObjId> {
    let match_id = rig.centre().match_id.expect("a match id");
    rig.book()
        .get(match_id)
        .and_then(|record| record.teams.iter().find(|side| side.team == team))
        .map(|side| side.units.clone())
        .expect("the side's roster")
}

/// **The check-out seals each slot's choice and the lead's fill** (17b cut
/// M5.2; the design note's item 10), on a server with the fill on and the
/// test provider installed, Ada and Bo both on red (so humans hold one side
/// and Q-UM-15 substitutes nothing), `SIM AI 3`:
///
/// * Ada's `Standard PROFILE Sniper, Line`: her hull's seal names Standard
///   Doctrine on the production engine, its card Sniper's (the first, which
///   flies at the release), its set `[Sniper, Line]` each with its card's
///   digest, Class I, no fallback;
/// * Bo's `Fixture I PROFILE Sniper` on his `line_tank`: sealed on its OWN
///   identity (`Fixture I`, its charter, line, revision and source digest),
///   on its own engine (`Scripted`), rated I on the governed Class II core —
///   Class I — its set `[Sniper]` with Sniper's card digest, unsubstituted
///   (17b cut M5.2's review: the non-Doctrine choice with a profile, from
///   the lobby to the seal, which M5.1's `m5_the_check_out_reads_each_hulls_choice`
///   pinned before Q-UM-15 substituted it there); and its source found, so
///   the arena opened on it (`InstallError::NoSource` would have called the
///   match off) — the store holds it;
/// * red's fill hull: the lead's `Standard PROFILE Hunter`, set `[Hunter]`;
/// * blue's three fill hulls, chosen by nobody: the default seal, Standard
///   Doctrine on Line, an explicit singleton set;
/// * the world's mirror holds each hull's first sealed card
///   (`CardSource::CheckOut`): Ada's and Bo's Sniper, the fill's Hunter,
///   blue's Line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_check_out_seals_the_slots_choice_and_the_leads_fill() {
    use tacsim::arena::SealEngine;
    use tacsim::arena::core::CoreClass;
    use tacsim::arena_bridge::CardSource;
    let (ada, bo) = (0, 1);
    let cfg = fill_config();
    let reflex = cfg.arena.reflex.clone();
    let preset = |name: &str| tacsim::arena::preset(name, &reflex).expect("a preset");
    let mut rig = fill_rig(2);
    let _provider = holdings_on(&mut rig);
    rig.say(ada, "sim team red");
    rig.say(bo, "sim team red");
    assert_eq!(
        rig.say(ada, "sim mind Standard PROFILE Sniper, Line"),
        "Your slot will carry Standard Doctrine (Sniper, Line)."
    );
    assert_eq!(
        rig.say(bo, "sim mind Fixture I PROFILE Sniper"),
        "Your slot will carry Fixture I (Sniper)."
    );
    assert_eq!(
        rig.say(ada, "sim ai mind Standard PROFILE Hunter"),
        "Red's fill will carry Standard Doctrine (Hunter)."
    );
    rig.say(ada, "sim ai 3");
    let said = rig.say(ada, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert!(!rig.centre().doctrine_only, "humans hold one side");
    let tank_of = |rig: &Rig, n: usize| {
        match_::tank_of(rig.tw.world(), rig.pod(n).expect("a locked pod")).expect("its tank")
    };
    let (ada_tank, bo_tank) = (tank_of(&rig, ada), tank_of(&rig, bo));
    assert!(!rig.seat(ada));
    assert!(rig.seat(bo));
    assert_eq!(rig.state(), MatchState::Running, "the arena opened on them");

    let seal = sealed_of(&rig, ada_tank);
    assert_eq!(seal.mind.as_str(), "Standard Doctrine");
    assert_eq!(seal.engine, tacsim::match_::PRODUCTION_FILL_ENGINE);
    assert_eq!(seal.class, CoreClass::I);
    assert_eq!(seal.fallback, None);
    assert_eq!(
        sealed_stock_cards(&seal),
        vec![
            ("Sniper".to_owned(), preset("Sniper")),
            ("Line".to_owned(), preset("Line")),
        ]
    );

    let fixture = support::holdings::fixture_i();
    let seal = sealed_of(&rig, bo_tank);
    assert_eq!(seal.mind, fixture.identity.mind);
    assert_eq!(seal.origin, fixture.identity.origin);
    assert_eq!(seal.source_sha256, fixture.identity.source_sha256);
    assert_eq!(seal.engine, SealEngine::Scripted, "on its own engine");
    assert_eq!((seal.core_class, seal.class), (CoreClass::II, CoreClass::I));
    assert_eq!(
        sealed_stock_cards(&seal),
        vec![("Sniper".to_owned(), preset("Sniper"))],
        "its chosen profile, unsubstituted"
    );
    assert_eq!(
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .and_then(|store| store.mind_source(&fixture.identity.source_sha256)),
        Some(support::holdings::FIXTURE_I_RHAI),
        "its source found"
    );

    let red = units_of(&rig, PartyId::RED);
    let fill: Vec<ObjId> = red
        .iter()
        .copied()
        .filter(|unit| *unit != ada_tank && *unit != bo_tank)
        .collect();
    assert_eq!(fill.len(), 1, "red's one fill hull");
    let match_id = rig.centre().match_id.expect("a match id");
    let mirror_card = |rig: &Rig, unit: ObjId| {
        rig.tw
            .world()
            .module_state::<tacsim::arena_bridge::ArenaRegistry>()
            .and_then(|registry| registry.mirror(match_id))
            .and_then(|mirror| mirror.cards.get(&unit).cloned())
            .expect("a mirror card")
    };
    for unit in &fill {
        let seal = sealed_of(&rig, *unit);
        assert_eq!(seal.mind.as_str(), "Standard Doctrine");
        assert_eq!(
            seal.cards
                .selected
                .names()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            vec!["Hunter".to_owned()]
        );
        assert_eq!(
            mirror_card(&rig, *unit),
            (preset("Hunter"), 0, CardSource::CheckOut)
        );
    }
    assert_eq!(
        mirror_card(&rig, ada_tank),
        (preset("Sniper"), 0, CardSource::CheckOut)
    );
    assert_eq!(
        mirror_card(&rig, bo_tank),
        (preset("Sniper"), 0, CardSource::CheckOut)
    );
    let blue = units_of(&rig, PartyId::BLUE);
    assert_eq!(blue.len(), 3);
    for unit in blue {
        let seal = sealed_of(&rig, unit);
        assert_eq!(seal.mind.as_str(), "Standard Doctrine");
        assert_eq!(
            sealed_stock_cards(&seal),
            vec![("Line".to_owned(), preset("Line"))],
            "the default seal"
        );
        assert_eq!(
            mirror_card(&rig, unit),
            (preset("Line"), 0, CardSource::CheckOut)
        );
    }
}

/// **A chosen mind's source is installed where the seal is built** (17b cut
/// M5.2; the orchestrator's item carried from 17b cut M5.1): under a
/// provider whose `sources()` is EMPTY ([`support::holdings::NoSources`]) —
/// so `install_holdings` put nothing in the store — Ada, alone on red with
/// the fill on, chooses `Fixture I`; the store holds no `Fixture I` source
/// until the check-out, which installs her holding's source before her seal
/// is used: the arena opens on her scripted seal (no `InstallError::NoSource`
/// call-off) and the store then holds it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_check_out_installs_a_chosen_minds_source() {
    use support::holdings::{FIXTURE_I_RHAI, digest};
    let mut rig = fill_rig(1);
    support::holdings::install_without_sources(rig.tw.world_mut());
    let held = |rig: &Rig| {
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .and_then(|store| store.mind_source(&digest(FIXTURE_I_RHAI)))
            .is_some()
    };
    assert!(!held(&rig), "the provider installed nothing");
    rig.say(0, "sim team red");
    assert_eq!(
        rig.say(0, "sim mind Fixture I"),
        "Your slot will carry Fixture I (Line)."
    );
    let said = rig.say(0, "sim start");
    assert!(said.contains("boarding"), "{said}");
    assert!(!held(&rig), "nothing before the check-out");
    assert!(rig.seat(0));
    assert_eq!(rig.state(), MatchState::Running, "the arena opened on it");
    assert!(held(&rig), "the check-out installed it");
    let tank = match_::tank_of(rig.tw.world(), rig.pod(0).expect("a pod")).expect("a tank");
    assert_eq!(
        sealed_of(&rig, tank).engine,
        tacsim::arena::SealEngine::Scripted
    );
}

/// **The seal takes no seat and posts nothing** (17b cut M5.2; the design
/// note's gate evidence): Ada and Bo on red with the fill on; the
/// check-out's `(op, key)` write list when Bo's slot chose `Fixture I` — a
/// chosen mind actually sealed (humans hold one side) — equals the list when
/// nobody chose anything, key for key, apart from M6's two exact Roll
/// revision-retention roots, whose full values are checked below. Tacsim's own
/// family is still the live record alone: the seal is `MatchRecord::minds`, in
/// the live record, no document of its own and no `tacsim:mindsrc:` (a
/// holding's source goes to the store's RAM map); and no seat is taken (the
/// record's pilots are the same).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_sim_seal_takes_no_seat_and_posts_nothing() {
    /// The sorted write list after checking and removing only the two
    /// required revision roots, plus the record's pilots.
    fn checkout(chose: bool) -> (Vec<(&'static str, String)>, usize) {
        let mut rig = fill_rig(2);
        rig.tick_to(1);
        let _provider = holdings_on(&mut rig);
        rig.say(0, "sim team red");
        rig.say(1, "sim team red");
        if chose {
            assert_eq!(
                rig.say(1, "sim mind Fixture I"),
                "Your slot will carry Fixture I (Line)."
            );
        }
        let _ = rig.pump_ops();
        rig.say(0, "sim start");
        rig.board_writes(0, match_::RED_ROOM_ENTRY);
        let (said, writes) = rig.board_writes(1, match_::RED_ROOM_ENTRY);
        assert!(said.contains("climb into"), "{said}");
        assert_eq!(rig.state(), MatchState::Running, "the check-out ran");
        let match_id = rig.centre().match_id.expect("a match id");
        let record = rig.book().get(match_id).cloned().expect("the record");
        let pilots = record.teams.iter().map(|side| side.pilots.len()).sum();
        let mut retention_keys = Vec::new();
        if chose {
            let tank = match_::tank_of(rig.tw.world(), rig.pod(1).expect("a pod")).expect("a tank");
            assert_eq!(sealed_of(&rig, tank).mind.as_str(), "Fixture I");
            let origin = sealed_of(&rig, tank).origin;
            assert_eq!(origin, support::holdings::fixture_i().identity.origin);
            let tacsim::arena::profile_seal::MindOrigin::Draft {
                character,
                mind,
                revision,
                digest,
            } = origin
            else {
                panic!("the actual fixture seal retains its immutable draft revision");
            };
            let revision = mush_roll::RevisionRef {
                key: mush_roll::RevisionKey {
                    character,
                    mind,
                    number: revision,
                },
                digest,
            };
            for kind in [
                mush_roll::ReferenceKind::OpenSeal,
                mush_roll::ReferenceKind::MatchRecord,
            ] {
                let key = mush_roll::ReferenceKey {
                    owner: tacsim::NS.into(),
                    kind,
                    record: format!("match/{}", match_id.0),
                };
                let stored_key =
                    format!("reference:{}:{}:{}", key.owner, kind.as_str(), key.record);
                assert_eq!(
                    writes.iter().filter(|(_, at, _)| at == &stored_key).count(),
                    1,
                    "each required root is written exactly once"
                );
                assert_eq!(
                    set_of(&writes, &stored_key),
                    Some(serde_json::json!({
                        "format": mush_roll::FORMAT,
                        "key": key,
                        "revisions": [revision],
                    })),
                    "the root retains only the actual sealed revision"
                );
                assert_eq!(
                    rig.tw
                        .world()
                        .module_state::<mush_roll::RollStore>()
                        .expect("the actual Roll owner")
                        .reference_keys(tacsim::NS, kind),
                    vec![key],
                    "the same root is published in the owner"
                );
                retention_keys.push(stored_key);
            }
        }
        let mut keys: Vec<(&'static str, String)> =
            writes.into_iter().map(|(op, key, _)| (op, key)).collect();
        for key in retention_keys {
            let index = keys
                .iter()
                .position(|(op, at)| *op == "set" && at == &key)
                .expect("the exact validated retention write");
            keys.remove(index);
        }
        keys.sort();
        (keys, pilots)
    }
    let (chosen, seated) = checkout(true);
    let (unchosen, plain) = checkout(false);
    assert!(!chosen.is_empty());
    assert_eq!(
        chosen, unchosen,
        "apart from exact revision retention, a sealed choice adds no write"
    );
    assert_eq!((seated, plain), (2, 2), "no seat taken");
    // tacsim's own family: the live record alone, which carries the seals
    // (`MatchRecord::minds`) — no seal document, and no `tacsim:mindsrc:`
    // (a holding's source goes to the store's RAM map, never a document).
    let ours: Vec<&str> = chosen
        .iter()
        .map(|(_, key)| key.as_str())
        .filter(|key| key.starts_with("tacsim:"))
        .collect();
    assert_eq!(ours, vec!["tacsim:live:1"], "{chosen:?}");
}

/// **A range's check-out seals its coreless target** (17b cut M5.2; C-24:
/// a hull with no core is never over-class), with the fill off and both
/// claimants on red under `SIM RANGE 1`: blue's one target, `target_mk1`
/// (no core), is sealed — Standard Doctrine rated I over no core, never
/// refused `OverClass` — at Class 0, governed, flying the ROM (`no core:
/// the ROM`), and the match runs.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_range_check_out_seals_its_coreless_target() {
    use tacsim::arena::core::CoreClass;
    let mut rig = Rig::boot(2);
    o3_range_boarding(&mut rig);
    assert!(!rig.seat(0));
    assert!(rig.seat(1));
    assert_eq!(rig.state(), MatchState::Running, "never over-class");
    let targets = units_of(&rig, PartyId::BLUE);
    assert_eq!(targets.len(), 1);
    let seal = sealed_of(&rig, targets[0]);
    assert_eq!(seal.core, None, "target_mk1 has no core");
    assert_eq!(
        (seal.core_class, seal.class),
        (CoreClass::Zero, CoreClass::Zero)
    );
    assert!(seal.governed);
    assert_eq!(seal.mind.as_str(), "Standard Doctrine");
    assert_eq!(
        seal.fallback.as_deref(),
        Some(tacsim::match_::NO_CORE_FALLBACK)
    );
}

/// **One shipped tank a side** for `match_::start` as staff calls it
/// ([`start_as_staff_queued`]), red's carrying `choice` in `Side::minds` as
/// `lobby::ready` would hand it (17b cut M5.2's review: the seal's backstops
/// reached through the check-out itself, where no lobby choice can).
fn one_a_side_choosing(
    rig: &Rig,
    choice: match_::holdings::SealChoice,
) -> (Vec<match_::Side>, ObjId) {
    let red = match_::tank_for_slot(rig.tw.world(), PartyId::RED, 1).expect("a shipped tank");
    let blue = match_::tank_for_slot(rig.tw.world(), PartyId::BLUE, 1).expect("a shipped tank");
    let sides = vec![
        match_::Side {
            team: PartyId::RED,
            units: vec![match_::Fielded::unslotted(red)],
            minds: BTreeMap::from([(red, choice)]),
        },
        match_::Side {
            team: PartyId::BLUE,
            units: vec![match_::Fielded::unslotted(blue)],
            minds: BTreeMap::new(),
        },
    ];
    (sides, red)
}

/// Nothing deployed and no record for `match_id`: a refused check-out.
fn nothing_checked_out(rig: &Rig, sides: &[match_::Side], match_id: u64) {
    for side in sides {
        for fielded in &side.units {
            assert!(
                !mush_sim::deployed::is_deployed(rig.tw.world(), fielded.unit),
                "#{} was checked out",
                fielded.unit.0
            );
        }
    }
    assert!(
        rig.tw
            .world()
            .module_state::<match_::MatchBook>()
            .and_then(|book| book.get(tacsim::arena_task::MatchId(match_id)))
            .is_none(),
        "no record was written"
    );
}

/// **An over-class mind is refused at the check-out, by hull** (17b cut
/// M5.2; doc 17 §9, D-UM-41; the design note's C-24; built against the
/// fixture mind `Load III`, the owner's OQ-10 (d); 17b cut M5.2's review: the
/// mapping from the seal's `OverClass` to the call-off had no test) — the
/// seal's backstop reached through `match_::start` itself: red's shipped
/// line tank (a governed Class II core) handed `Load III` (rated III) in
/// `Side::minds`, as no lobby choice can (`SIM MIND` refuses it first):
/// refused `StartError::MindOverClass` naming the hull, III over II, before
/// the first write — nothing queued, nothing deployed, no record; its long
/// form names both classes (the log's and the fixtures'), and the call-off's
/// words name the hull alone, `{callsign}'s mind is over its core's class`
/// — no mind and no class — under 120 columns in the lobby's call-off line.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_an_over_class_mind_is_refused_at_the_check_out_by_hull() {
    use tacsim::arena::core::CoreClass;
    let cfg = config();
    let mut rig = Rig::boot_on(0, roster_with(cfg.clone()));
    let choice = match_::holdings::SealChoice {
        held: support::holdings::load_iii(),
        profiles: Vec::new(),
        chose: true,
        substituted: false,
    };
    let (sides, red) = one_a_side_choosing(&rig, choice);
    let (refused, queued) = start_as_staff_queued(&mut rig, &cfg, 93, &sides);
    let Err(error @ match_::StartError::MindOverClass { .. }) = refused else {
        panic!("refused over-class: {refused:?}");
    };
    let match_::StartError::MindOverClass {
        unit,
        callsign,
        rated,
        core,
    } = error.clone()
    else {
        unreachable!("matched above");
    };
    assert_eq!((unit, rated, core), (red, CoreClass::III, CoreClass::II));
    assert_eq!(
        error.to_string(),
        format!(
            "{callsign}'s mind is rated Class III, over its core's Class II; the match cannot start."
        )
    );
    let words = error.call_off_words(rig.tw.world(), &sides);
    assert_eq!(words, format!("{callsign}'s mind is over its core's class"));
    assert!(
        !words.contains("Load") && !words.contains("Class"),
        "{words}"
    );
    let call_off = format!("Match 93 could not start: {words}. The pods are free again.");
    assert!(call_off.chars().count() < 120, "{call_off}");
    assert_eq!(queued, 0, "refused before the first write");
    nothing_checked_out(&rig, &sides, 93);
}

/// **A card the seal refuses is called off by hull, naming no profile** (17b
/// cut M5.2's review; doc 17 §11, D-UM-43; C-4: a chosen preset is its
/// pilot's alone) — through `match_::start` itself: red's shipped tank
/// handed Standard Doctrine `PROFILE Rearguard` in `Side::minds` over a
/// doctrine the config admits (`withdraw_permille` 800 over
/// `stand_min_permille` 750: `fight_home_pct` 75) under which Rearguard's
/// `break_off_pct` 70 breaks the survival pair. No lobby choice reaches it —
/// `SIM MIND` fits the same pair under the same frozen `[tacsim]` block —
/// but the seal checks every sealed profile, so the check-out has its own
/// word: refused `StartError::CardRefused` naming the hull and the pair,
/// before the first write; its long form names Rearguard (the log's), and
/// the call-off's words — said to the whole centre and to the last `BOARD`er
/// — name the hull alone, `{callsign}'s card could not be sealed`, never the
/// profile, and fit under 120 columns where the long form does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_card_the_seal_refuses_is_called_off_by_hull_naming_no_profile() {
    let mut cfg = config();
    cfg.arena.reflex.withdraw_permille = 800;
    cfg.arena.reflex.stand_min_permille = 750;
    assert_eq!(
        tacsim::config::ArenaConfig {
            reflex: cfg.arena.reflex.clone(),
            ..tacsim::config::ArenaConfig::default()
        }
        .validate(),
        Ok(()),
        "the config admits the pair"
    );
    let mut rig = Rig::boot_on(0, roster_with(cfg.clone()));
    let choice = match_::holdings::SealChoice {
        held: match_::holdings::HeldMind::doctrine(),
        profiles: vec![smol_str::SmolStr::new_static("Rearguard")],
        chose: true,
        substituted: false,
    };
    let (sides, red) = one_a_side_choosing(&rig, choice);
    let (refused, queued) = start_as_staff_queued(&mut rig, &cfg, 94, &sides);
    let Err(error @ match_::StartError::CardRefused { .. }) = refused else {
        panic!("refused for the card: {refused:?}");
    };
    let match_::StartError::CardRefused {
        unit,
        callsign,
        refusal,
    } = error.clone()
    else {
        unreachable!("matched above");
    };
    assert_eq!(unit, red);
    assert_eq!(
        refusal,
        tacsim::arena::CardError::Pair {
            slider: "Rearguard",
            fight_home: "75".to_owned(),
            break_off: "70".to_owned(),
        }
    );
    let long = error.to_string();
    assert_eq!(
        long,
        format!(
            "{callsign}'s card could not be sealed: Rearguard would put fight_home_pct 75 above \
             break_off_pct 70; fight_home_pct is at most break_off_pct; the match cannot start."
        )
    );
    let words = error.call_off_words(rig.tw.world(), &sides);
    assert_eq!(words, format!("{callsign}'s card could not be sealed"));
    assert!(!words.contains("Rearguard"), "{words}");
    let call_off = format!("Match 94 could not start: {words}. The pods are free again.");
    assert!(call_off.chars().count() < 120, "{call_off}");
    assert_eq!(queued, 0, "refused before the first write");
    nothing_checked_out(&rig, &sides, 94);
}

/// **A scripted choice whose source the store lacks under its identity is
/// refused before the first write** (17b cut M5.2's review of the carried
/// install) — through `match_::start` itself: red's shipped line tank handed
/// `Fixture I` whose held `source` is NOT its identity's text but Load
/// III's (a provider whose source and identity disagree), on a world where
/// no provider installed anything: the check-out installs the text it was
/// handed (the store then holds Load III's, never Fixture I's) and refuses
/// the seal, which names Fixture I's digest, with the refusal `open_match`
/// would have given after the check-out wrote — `InstallError::NoSource`
/// for the hull — before any write: nothing queued, nothing deployed; the
/// call-off's words `{callsign}'s sealed mind has no source on this
/// server`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_a_scripted_choice_with_no_source_under_its_identity_is_refused_before_any_write() {
    use support::holdings::{FIXTURE_I_RHAI, LOAD_III_RHAI, digest};
    let cfg = config();
    let mut rig = Rig::boot_on(0, roster_with(cfg.clone()));
    let mismatched = match_::holdings::HeldMind {
        source: Some(Arc::from(LOAD_III_RHAI)),
        ..support::holdings::fixture_i()
    };
    let choice = match_::holdings::SealChoice {
        held: mismatched,
        profiles: Vec::new(),
        chose: true,
        substituted: false,
    };
    let (sides, red) = one_a_side_choosing(&rig, choice);
    let (refused, queued) = start_as_staff_queued(&mut rig, &cfg, 95, &sides);
    let Err(
        error @ match_::StartError::Open(tacsim::arena_bridge::OpenMatchError::Mind(
            tacsim::minds::InstallError::NoSource { unit, .. },
        )),
    ) = refused
    else {
        panic!("refused for the source: {refused:?}");
    };
    assert_eq!(unit, red);
    let held = |text: &str| {
        rig.tw
            .world()
            .module_state::<tacsim::TacsimStore>()
            .and_then(|store| store.mind_source(&digest(text)))
            .is_some()
    };
    assert!(held(LOAD_III_RHAI), "the text it was handed, installed");
    assert!(!held(FIXTURE_I_RHAI), "never the identity's");
    let words = error.call_off_words(rig.tw.world(), &sides);
    assert!(
        words.ends_with("'s sealed mind has no source on this server"),
        "{words}"
    );
    assert_eq!(queued, 0, "refused before the first write");
    nothing_checked_out(&rig, &sides, 95);
}

/// **Humans on both sides: a non-Doctrine choice is substituted, and said
/// so once** (17b cut M5.2; doc 17 §21's Q-UM-15 at its default, the owner's
/// OQ-5; the design note's C-7 and item 11), on a server with the fill on and
/// the test provider installed, `SIM AI 2`: Ada (red) chooses `Fixture I`
/// for her slot and, as red's lead, `Fixture I PROFILE Sniper` for red's
/// fill; Bo (blue) chooses nothing and types `SIM START`:
///
/// * the centre is `doctrine_only` from `SIM START`, and Ada — and Ada
///   alone, though both her choices are substituted — hears `Humans on the
///   other side: your fill and any released hull fly Standard Doctrine.`
///   ONCE; Bo, who typed `SIM START`, hears nothing of it;
/// * her hull is sealed Standard Doctrine on Line, and red's fill hull
///   Standard Doctrine at Sniper (the lead's chosen profile, one of
///   Doctrine's presets), each with the fallback `humans on the other side`;
/// * blue's hulls are the default seal, no fallback;
/// * Ada's seal line says the substitution:
///   `Sealed for this sortie: Standard Doctrine (Line), Class I, for humans
///   on the other side; MIND reads it.` (103).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_humans_on_both_sides_substitute_a_non_doctrine_choice_and_say_so_once() {
    let (ada, bo) = (0, 1);
    let sentence =
        "Humans on the other side: your fill and any released hull fly Standard Doctrine.";
    assert_eq!(sentence.chars().count(), 80);
    let mut rig = fill_rig(2);
    rig.tick_to(1);
    let _provider = holdings_on(&mut rig);
    rig.say(ada, "sim team red");
    rig.say(bo, "sim team blue");
    rig.say(ada, "sim mind Fixture I");
    assert_eq!(
        rig.say(ada, "sim ai mind Fixture I PROFILE Sniper"),
        "Red's fill will carry Fixture I (Sniper)."
    );
    rig.say(ada, "sim ai 2");
    let (to_bo, to_ada) = rig.say_heard(bo, "sim start", ada);
    assert!(to_bo.contains("boarding"), "{to_bo}");
    assert!(rig.centre().doctrine_only, "humans on both sides");
    assert_eq!(
        to_ada.matches(sentence).count(),
        1,
        "once, to Ada: {to_ada}"
    );
    assert!(!to_bo.contains("Humans on the other side"), "{to_bo}");
    let tank_of = |rig: &Rig, n: usize| {
        match_::tank_of(rig.tw.world(), rig.pod(n).expect("a locked pod")).expect("its tank")
    };
    let ada_tank = tank_of(&rig, ada);
    assert!(!rig.seat(bo));
    let heard = board_heard(&mut rig, ada, match_::RED_ROOM_ENTRY, &[ada, bo]);
    assert_eq!(rig.state(), MatchState::Running, "{heard:?}");
    assert!(
        heard[0].contains(
            "Sealed for this sortie: Standard Doctrine (Line), Class I, for humans on the other \
             side; MIND reads it."
        ),
        "{}",
        heard[0]
    );
    assert!(!heard[1].contains("Sealed for this sortie"), "{}", heard[1]);
    let seal = sealed_of(&rig, ada_tank);
    assert_eq!(seal.mind.as_str(), "Standard Doctrine");
    assert_eq!(
        seal.cards.selected.names().collect::<Vec<_>>(),
        vec!["Line"],
        "on Line"
    );
    assert_eq!(
        seal.fallback.as_deref(),
        Some(tacsim::match_::HUMANS_FALLBACK)
    );
    let fill: Vec<ObjId> = units_of(&rig, PartyId::RED)
        .into_iter()
        .filter(|unit| *unit != ada_tank)
        .collect();
    assert_eq!(fill.len(), 1);
    let seal = sealed_of(&rig, fill[0]);
    assert_eq!(seal.mind.as_str(), "Standard Doctrine");
    assert_eq!(
        seal.cards
            .selected
            .names()
            .map(str::to_owned)
            .collect::<Vec<_>>(),
        vec!["Sniper".to_owned()],
        "at the lead's chosen profile"
    );
    assert_eq!(
        seal.fallback.as_deref(),
        Some(tacsim::match_::HUMANS_FALLBACK)
    );
    for unit in units_of(&rig, PartyId::BLUE) {
        let seal = sealed_of(&rig, unit);
        assert_eq!(seal.fallback, None);
        assert_eq!(
            seal.cards.selected.names().collect::<Vec<_>>(),
            vec!["Line"]
        );
    }
}

/// **The seal line reaches the choosing pilot alone, before `Good
/// hunting.`** (17b cut M5.2; doc 17 §12; the design note's D7, C-18 and
/// item 12): Ada (red) chooses `Standard PROFILE Skirmisher, Rearguard` (the
/// widest preset pair); Cy, her red teammate, chose nothing and boards
/// FIRST, so he sits in his own red pod; Ada boards second; Bo (blue), who
/// chose nothing, boards LAST — so the check-out's actor is Bo, and a line
/// written to the actor would reach him and not her; and Di, who claimed no
/// slot, stands in the red team room. Ada reads `Sealed for this sortie:
/// Standard Doctrine (Skirmisher, Rearguard), Class I; MIND reads it.` (90)
/// once, before `Match 1 is running. Good hunting.`, which stays the
/// check-out's last line; Cy, Bo and Di — the teammate, the actor and the
/// red room's observer, each of whom hears `Good hunting.` — read no seal
/// line and no profile of hers (17b cut M5.2's review: a line said to
/// Ada's side or her room instead of to her would reach Cy or Di). An
/// unchosen slot's seal is the default, and not said (C-18).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m5_the_seal_line_reaches_the_choosing_pilot_alone_before_good_hunting() {
    let (ada, bo, cy, di) = (0, 1, 2, 3);
    let line = "Sealed for this sortie: Standard Doctrine (Skirmisher, Rearguard), Class I; MIND \
                reads it.";
    assert!(line.chars().count() < 120);
    let mut rig = Rig::boot(4);
    rig.tick_to(1);
    rig.say(ada, "sim team red");
    rig.say(cy, "sim team red");
    rig.say(bo, "sim team blue");
    assert_eq!(
        rig.say(ada, "sim mind Standard PROFILE Skirmisher, Rearguard"),
        "Your slot will carry Standard Doctrine (Skirmisher, Rearguard)."
    );
    rig.say(ada, "sim start");
    let red_room =
        match_::centre_object(rig.tw.world(), match_::RED_ROOM_ENTRY).expect("the red room");
    let observer = rig.pilot(di);
    rig.tw
        .world_mut()
        .move_to(observer, red_room)
        .expect("Di walks into the red room");
    let early = board_heard(&mut rig, cy, match_::RED_ROOM_ENTRY, &[cy, ada, di]);
    let early = board_heard(&mut rig, ada, match_::RED_ROOM_ENTRY, &[ada, cy, di])
        .into_iter()
        .chain(early)
        .collect::<Vec<_>>();
    assert!(
        early.iter().all(|heard| !heard.contains("Sealed")),
        "nothing before the check-out: {early:?}"
    );
    let heard = board_heard(&mut rig, bo, match_::BLUE_ROOM_ENTRY, &[ada, bo, cy, di]);
    assert_eq!(rig.state(), MatchState::Running);
    let to_ada = &heard[0];
    assert_eq!(to_ada.matches(line).count(), 1, "{to_ada}");
    let hunting = "Match 1 is running. Good hunting.";
    let sealed_at = to_ada.find(line).expect("the seal line");
    let hunting_at = to_ada.find(hunting).expect("the check-out's last line");
    assert!(sealed_at < hunting_at, "before Good hunting: {to_ada}");
    assert!(
        to_ada.trim_end().ends_with(hunting),
        "Good hunting stays last: {to_ada}"
    );
    for (who, other) in [("Bo", &heard[1]), ("Cy", &heard[2]), ("Di", &heard[3])] {
        assert!(other.contains(hunting), "{who} hears the centre: {other}");
        assert!(!other.contains("Sealed for this sortie"), "{who}: {other}");
        assert!(!other.contains("Skirmisher"), "{who}: {other}");
        assert!(!other.contains("Rearguard"), "{who}: {other}");
    }
}

// 17d M6.2 class admission through the real selector and shipped Class II core.
// The authored save/seal acceptance also runs this path with the saved provider;
// this narrow provider isolates cap, authority and roster arithmetic behavior.
struct ClassTwoHoldings;

impl tacsim::match_::holdings::Holdings for ClassTwoHoldings {
    fn held(
        &self,
        _world: &mush_core::World,
        _character: ObjId,
    ) -> Vec<tacsim::match_::holdings::HeldMind> {
        let mut mind = support::holdings::fixture_i();
        mind.identity.mind = "Fixture II".into();
        mind.identity.origin = support::holdings::fixture_origin(
            3,
            tacsim::arena::core::CoreClass::II,
            support::holdings::FIXTURE_I_RHAI,
        );
        mind.rated = tacsim::arena::core::CoreClass::II;
        vec![
            tacsim::match_::holdings::HeldMind::doctrine(),
            mind,
            support::holdings::load_iii(),
        ]
    }

    fn sources(&self) -> Vec<Arc<str>> {
        vec![
            Arc::from(support::holdings::FIXTURE_I_RHAI),
            Arc::from(support::holdings::LOAD_III_RHAI),
        ]
    }
}

fn class_two_provider(rig: &mut Rig) {
    let setup = support::holdings::seed_fixture_revisions(rig.tw.world_mut());
    fold_ops(&mut rig.image, &setup);
    tacsim::match_::holdings::install_holdings(rig.tw.world_mut(), Arc::new(ClassTwoHoldings));
}

fn preview_class(rig: &Rig, team: PartyId, index: u8) -> tacsim::arena::core::CoreClass {
    tacsim::match_::lobby::class_preview_for_tests(rig.tw.world())
        .expect("the complete roster fits")
        .into_iter()
        .find(|row| row.0 == team && row.1 == index)
        .expect("the hull is fielded")
        .2
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m6_pilot_and_fill_caps_are_independent_and_do_not_raise_a_minds_rating() {
    use tacsim::arena::core::CoreClass;
    let mut cfg = config();
    cfg.match_.ai_fill = true;
    cfg.minds.fill_class = CoreClass::I;
    cfg.minds.max_class = CoreClass::III;
    let mut rig = Rig::boot_on(2, roster_with(cfg));
    class_two_provider(&mut rig);
    rig.say(0, "sim team red");
    assert_eq!(
        rig.say(0, "sim mind Fixture II"),
        "Your slot will carry Fixture II (Line)."
    );
    rig.say(0, "sim ai 2");
    assert_eq!(
        preview_class(&rig, PartyId::RED, 1),
        CoreClass::II,
        "pilot is independent of fill default I"
    );
    assert_eq!(
        preview_class(&rig, PartyId::RED, 2),
        CoreClass::I,
        "default Standard is rated I"
    );
    rig.say(0, "sim ai class III");
    assert_eq!(
        preview_class(&rig, PartyId::RED, 2),
        CoreClass::I,
        "raising a cap cannot raise Standard's rating"
    );
    rig.say(0, "sim ai class I");
    assert!(rig.say(0, "sim ai mind Fixture II").contains("Fixture II"));
    assert_eq!(preview_class(&rig, PartyId::RED, 2), CoreClass::I);
    rig.say(0, "sim ai class II");
    assert_eq!(preview_class(&rig, PartyId::RED, 1), CoreClass::II);
    assert_eq!(preview_class(&rig, PartyId::RED, 2), CoreClass::II);
    assert_eq!(
        preview_class(&rig, PartyId::BLUE, 1),
        CoreClass::I,
        "other side keeps its own default"
    );
    assert!(
        rig.say(0, "sim mind")
            .contains("Your mind will seal at Class II.")
    );
    rig.say(1, "sim team red");
    assert_eq!(
        rig.say(1, "sim ai class 0"),
        "Only your side's lead can set its AI class cap."
    );
    rig.say(0, "sim leave");
    assert!(
        rig.centre().fill_classes.is_empty(),
        "the new lead does not inherit the old choice"
    );
    assert!(
        rig.centre().fill_minds.is_empty(),
        "the prior lead's mind is cleared too"
    );
    assert!(
        rig.say(1, "sim ai class 0")
            .contains("AI class cap is now 0")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m6_operator_cap_cannot_hide_over_class_and_the_rom_exception_remains() {
    use tacsim::arena::core::CoreClass;
    let mut cfg = config();
    cfg.match_.ai_fill = true;
    cfg.minds.max_class = CoreClass::I;
    let mut rig = Rig::boot_on(1, roster_with(cfg));
    class_two_provider(&mut rig);
    rig.say(0, "sim team red");
    let refused = rig.say(0, "sim mind Load III");
    assert!(
        refused.contains("Class III") && refused.contains("core is Class II"),
        "{refused}"
    );
    rig.say(0, "sim mind Fixture II");
    assert_eq!(preview_class(&rig, PartyId::RED, 1), CoreClass::I);
    rig.say(0, "sim unit target_mk1");
    assert!(
        rig.say(0, "sim mind Load III")
            .starts_with("Your slot will carry")
    );
    assert_eq!(preview_class(&rig, PartyId::RED, 1), CoreClass::Zero);
    assert!(
        rig.say(0, "sim unit line_tank")
            .contains("core is Class II")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m6_class_and_roster_changes_refuse_before_publishing_an_oversized_pool() {
    use tacsim::arena::core::CoreClass;
    let mut cfg = config();
    cfg.match_.ai_fill = true;
    cfg.minds.pool_base_ops_s = 16_000;
    cfg.minds.pool_per_unit_ops_s = 0;
    cfg.minds.pool_floor_ops_s = 1;
    let mut rig = Rig::boot_on(3, roster_with(cfg));
    class_two_provider(&mut rig);
    rig.say(0, "sim team red");
    rig.say(0, "sim mind Fixture II");
    rig.say(0, "sim ai 2");
    rig.say(0, "sim ai mind Fixture II");
    let before = rig.centre();
    let refused = rig.say(0, "sim ai class II");
    assert!(
        refused.contains("8,000") && refused.contains("6,000"),
        "{refused}"
    );
    assert_eq!(
        rig.centre(),
        before,
        "refused cap does not mutate the centre"
    );
    assert_eq!(preview_class(&rig, PartyId::RED, 2), CoreClass::I);
    rig.say(1, "sim team red");
    let before = rig.centre();
    let refused = rig.say(2, "sim team red");
    assert!(
        refused.contains("8,000"),
        "claim re-admits every hull: {refused}"
    );
    assert_eq!(
        rig.centre(),
        before,
        "refused roster does not mutate the centre"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m6_a_lead_may_leave_when_losing_their_cap_makes_the_roster_too_large() {
    use tacsim::arena::core::CoreClass;
    let mut cfg = config();
    cfg.match_.ai_fill = true;
    cfg.minds.pool_base_ops_s = 16_000;
    cfg.minds.pool_per_unit_ops_s = 0;
    cfg.minds.pool_floor_ops_s = 1;
    let mut rig = Rig::boot_on(2, roster_with(cfg));
    class_two_provider(&mut rig);
    rig.say(0, "sim team red");
    rig.say(0, "sim unit target_mk1");
    rig.say(1, "sim team red");
    rig.say(1, "sim mind Fixture II");
    rig.say(0, "sim ai 3");
    rig.say(0, "sim ai class 0");
    assert_eq!(preview_class(&rig, PartyId::RED, 3), CoreClass::Zero);
    let leaver = rig.pilot(0);
    let (left, lead_heard) = rig.say_heard(0, "sim leave", 1);
    assert!(left.contains("gives up red slot 1"), "{left}");
    assert!(!rig.centre().slots.iter().any(|slot| slot.pilot == leaver));
    assert!(rig.centre().fill_classes.is_empty());
    assert!(
        lead_heard.contains("admission needs review"),
        "{lead_heard}"
    );
    assert_eq!(preview_class(&rig, PartyId::RED, 3), CoreClass::I);
    let refused = rig.say(1, "sim start");
    assert!(
        refused.contains("8,000"),
        "the new roster cannot check out: {refused}"
    );
    assert_eq!(rig.centre().state, match_::MatchState::Selection);
    assert!(
        rig.centre().match_id.is_none(),
        "refusal precedes match id and pod locks"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m6_rechecking_a_teammates_lost_mind_does_not_disclose_their_choice() {
    let mut cfg = config();
    cfg.match_.ai_fill = true;
    let mut rig = Rig::boot_on(2, roster_with(cfg));
    let provider = holdings_on(&mut rig);
    rig.say(0, "sim team red");
    rig.say(1, "sim team red");
    rig.say(0, "sim mind Fixture I");
    provider.drop_fixture_i();
    let before = rig.centre();
    let refused = rig.say(1, "sim mind Standard PROFILE Hunter");
    assert_eq!(
        refused,
        "Another hull cannot be sealed; its pilot or side lead must review its choices."
    );
    assert!(!refused.contains("Fixture"));
    assert_eq!(
        rig.centre(),
        before,
        "a failed complete preview publishes no partial choice"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn m6_range_targets_preview_the_refitted_rom_class_and_checkout_uses_it() {
    use tacsim::arena::core::CoreClass;
    let mut cfg = config();
    cfg.match_.ai_fill = true;
    cfg.minds.pool_base_ops_s = 3_200;
    cfg.minds.pool_per_unit_ops_s = 0;
    cfg.minds.pool_floor_ops_s = 1;
    let mut rig = Rig::boot_on(1, roster_with(cfg));
    rig.say(0, "sim team red");
    // A previous fill selection has no authority over a range's target chassis.
    god_in_the_lobby(&mut rig);
    let selected = rig.staff("sim ai mind blue Standard PROFILE Hunter");
    assert!(
        selected.contains("Standard Doctrine (Hunter)"),
        "{selected}"
    );
    let said = rig.say(0, "sim range 2");
    assert!(
        !said.contains("would think"),
        "target ROM loads fit: {said}"
    );
    assert_eq!(preview_class(&rig, PartyId::RED, 1), CoreClass::I);
    assert_eq!(preview_class(&rig, PartyId::BLUE, 1), CoreClass::Zero);
    assert_eq!(preview_class(&rig, PartyId::BLUE, 2), CoreClass::Zero);
    rig.say(0, "sim start");
    assert!(
        rig.seat(0),
        "the range checks out under the same lowered pool"
    );
    let id = rig.centre().match_id.expect("running range");
    let record = rig.book().get(id).expect("range record");
    let targets = record
        .teams
        .iter()
        .find(|side| side.team == PartyId::BLUE)
        .expect("target side");
    assert_eq!(targets.units.len(), 2);
    for unit in &targets.units {
        assert_eq!(
            record.minds.get(unit).expect("target seal").seal.class,
            CoreClass::Zero
        );
    }
}

#[path = "match_lifecycle/resume_watch.rs"]
mod resume_watch;
