//! **Doc 45 unit AP.8 — a detachment whose leader is gone** (Q-45-13; doc 44 Q-44-18, Q-44-38,
//! Q-44-46), over the composed world (`driving`'s keyed rig) with the arena's egress pumped by
//! hand, each roster flown by `roster_rig` (MW.15's rig) into a field of three parties: the
//! roster, an escort of another captain's and an intruder of a third's.
//!
//! 1. **An `ENGAGE` lapses after the grace, and the crew still answers a hit**
//!    ([`ap8_an_absent_leaders_engage_lapses_and_the_crew_still_answers_a_hit`]). A captain
//!    leads a combat roster from a prospector with Rook Adeyemi hired into a leased escort tank.
//!    The captain marks the intruder's party Hostile and orders `2 ENGAGE`; Adeyemi fires on the
//!    intruder. The captain's link is lost. The server's own grace ends the session a minute
//!    later, which gives up the helm and the lead, and the order still stands half a minute
//!    short of five minutes. At five minutes the captain's rig passes to its mind, the order
//!    lapses, and Adeyemi's tank holds: it fires nothing more at the party its leader marked.
//!    The intruder's player fires on it; once a round hits, the tank answers from where it
//!    stands. The captain reads the lapse at login and in `+sortie/orders`, and it is still
//!    lapsed after their return.
//! 2. **The plan's NPC commander replaces a lapsed order**
//!    ([`ap8_the_npc_commander_replaces_a_lapsed_advance`]). A captain leads with Sana Okonkwo,
//!    the plan's commander, in rig 2, and orders her to a cell of the captain's choosing. The
//!    captain quits: the hand-over, the lapse and the succession are at once (Q-44-46), and
//!    Okonkwo's own ADVANCE to the work point is the order her rig then holds.
//! 3. **A `HOLD` the leader left is still held** ([`ap8_a_hold_the_leader_left_is_still_held`]).
//!    The captain orders `2 HOLD` and quits; four hundred seconds on, the order is the same
//!    order, nothing lapsed, and nobody was told otherwise.
//! 4. **A restart inside the grace** ([`ap8_a_restart_keeps_the_clock_and_the_lapse`]). The
//!    captain's link is lost, the server restarts two minutes in, and the order lapses five
//!    minutes after the loss, not five minutes after the boot. A second restart finds it lapsed.
//! 5. **A returning leader** ([`ap8_a_lapsed_order_stays_lapsed_until_its_leader_gives_it_again`]).
//!    The captain quits, the order lapses, the captain reconnects while their rig is still in
//!    the field and leads again: the order is still lapsed, they are told so, and the same order
//!    given again is delivered and obeyed.
//!
//! **Not flown here**: a second player in the roster's units taking the lead before the NPC
//! commander. A Dray-P bays two rigs, so a roster with two players has no room for the
//! commander's; the order of succession is pinned on the roster itself
//! (`mush_tactics::detachment::roster`,
//! `ap8_the_next_active_player_leads_before_the_plans_npc_commander`).
//!
//! What the stories stand in for:
//! - **the escort and the intruder are fixture hulls with players seated** (CF.2's `armed_on`
//!   and `seat`); the intruder's player types every shot (`LOCK`, `FIRE`);
//! - **the roster's rigs are stood on their cells by a staff hand** before the field opens, as
//!   every field story places its units;
//! - **the five minutes are world wall seconds**, which the rig steps: none is waited out.

use mush_core::ObjId;
use mush_tactics::detachment::LeaderKind;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::party::Relation;
use tacsim::arena::perception::MindObjective;
use tacsim::arena_task::ArenaOut;
use tacsim::detachment::sortie::{
    FieldObjective, Lapse, ORDER_LAPSED, ORDER_LAPSED_LEAD, ORDER_LAPSED_STATUS,
};
use tacsim::field::absent::{FIELD_LINKDEAD_GRACE_WALL_S, away_of, minded};
use tokio::sync::mpsc::Receiver;

use crate::driving_rig::{
    Rig, cid, command, crash, land, land_composed, on_pad_as_at, run, said, site_domain,
    staff_hand, stocked_tuned, warm,
};
use crate::field_host_rig::{
    armed_on, egress, frame, pump, seat, second_composed, task, until_composed, weapon_of,
};
use crate::grounds_rig::{AHUNA_FLANK, Ground, Placed};
use crate::roster_rig::{
    Billet, Flown, Plan, apply_due, beat, commit, delivered_to, fence, field_roster, fly,
    operation, orders_of, queued,
};

/// The shipped crew at Ceres (`content/crew/ceres.json`): the forewoman under a commander's
/// contract, the mining operator, and the escort under a combat contract.
const OKONKWO: &str = "Sana Okonkwo";
const MENDES: &str = "Ilya Mendes";
const ADEYEMI: &str = "Rook Adeyemi";

/// The cell the stories' ADVANCE names: some thirty cells north-east of the hired rig, so the
/// rig is still driving for it when the order lapses.
const FAR: (u16, u16) = (170, 100);

/// **A field with a roster in it**: the roster, the two other parties and the arena's egress.
struct Field {
    rig: Rig,
    flown: Flown,
    /// The escort's player.
    other: ObjId,
    /// The intruder's player and hull.
    stranger: ObjId,
    intruder: ObjId,
    rx: Receiver<ArenaOut>,
    lines: Vec<LedgerEvent>,
}

/// Stand `unit` on `entry`'s ground at `cell`, facing `heading` (a staff hand).
fn stand(rig: &mut Rig, entry: &str, unit: ObjId, (cell, heading): Placed) {
    let domain = site_domain(&rig.tw, entry);
    staff_hand(rig, |ctx| {
        let pose =
            tacsim::surface::excursion::cell_pose([cell.0, cell.1], heading).expect("a pose");
        mush_sim::place(ctx, unit, domain, pose).expect("placed");
    });
}

/// **Refit `unit` as the armed escort rig** (a staff hand; `tacsim.unit.tv_escort`, the hull the
/// armed lease signs).
///
/// A stand-in for a gap in the game: no hired NPC can be seated in an armed rig. `+sortie/seat`
/// hands the helm over through Sim's hand-off, which takes a console of exactly one channel
/// (`mush_sim::handoff`, `station_channel`), and the escort's one console carries the helm, the
/// sensors and the fire channel. So the roster flies two prospectors as it can, and the hired
/// one is refitted on the ground before the field opens: its object, its console and its place
/// in the roster are what the production commands made them.
fn arm(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let spec = ctx
            .w
            .module_state::<tacsim::TacsimStore>()
            .expect("the layer")
            .definitions()
            .bindings
            .get(&cid("tacsim.unit.tv_escort"))
            .map(|binding| binding.spec.clone())
            .expect("the escort design is bound");
        mush_tactics::module::refit_unit(ctx, unit, &spec).expect("the rig is refitted");
    });
}

/// **Fly `plan`'s roster onto Ahuna Flank and open a field over it** (MW.15's ground): the
/// leader's rig on the miner's ground, the hired rig seven cells east of it, an escort from
/// another captain and an intruder east of both; the escort fires on the intruder's bearing and
/// the site promotes with three parties. The field holds through the stories' quiet stretches.
async fn opened(plan: &Plan, armed: bool) -> Field {
    let at: &Ground = &AHUNA_FLANK;
    let mut rig = stocked_tuned(|cfg| {
        crate::field_host_rig::tune(cfg);
        cfg.field.disengage_s = 7_200;
        cfg.field.idle_demote_s = 7_200;
    });
    let flown = fly(&mut rig, at, plan, |_, _| {});
    if armed {
        arm(&mut rig, flown.unit(2));
    }
    stand(&mut rig, at.entry, flown.unit(1), (at.read, 0));
    stand(
        &mut rig,
        at.entry,
        flown.unit(2),
        ((at.read.0 + 7, at.read.1), 90_000),
    );
    let (other, _, _) = on_pad_as_at(&mut rig, ("Other", 2, "Tally Moth"), at.site(), 0);
    let (stranger, _, _) = on_pad_as_at(&mut rig, ("Third", 3, "Lone Moth"), at.site(), 0);
    let escort = armed_on(&mut rig, at.entry, "Red Hound", at.escort.0, at.escort.1);
    let intruder = armed_on(&mut rig, at.entry, "Grey Hound", at.walker.0, at.walker.1);
    seat(&mut rig, escort, other);
    seat(&mut rig, intruder, stranger);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second_composed(&mut rig);
    }
    let (out, queued) = command(&mut rig, other, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(at.entry)).expect("the live record");
    let party = live.party_of(flown.unit(1)).expect("the roster's party");
    assert_eq!(live.party_of(flown.unit(2)), Some(party));
    let others: std::collections::BTreeSet<_> = [escort, intruder]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert!(
        others.len() == 2 && !others.contains(&party),
        "three parties stand in the field: the roster, the escort and the intruder"
    );
    let (leader, hired) = (flown.operator(1), flown.operator(2));
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
            frame(rig, leader).is_some() && frame(rig, hired).is_some()
        })
        .await,
        "the roster's frames arrive"
    );
    Field {
        rig,
        flown,
        other,
        stranger,
        intruder,
        rx,
        lines,
    }
}

impl Field {
    /// Pump and tick until `done` holds or `secs` of wall time pass.
    async fn until(
        &mut self,
        secs: u64,
        done: impl FnMut(&mut Rig, &[LedgerEvent]) -> bool,
    ) -> bool {
        until_composed(&mut self.rig, &mut self.rx, &mut self.lines, secs, done).await
    }

    /// Pump and tick for `secs` of wall time.
    async fn wait(&mut self, secs: u64) {
        let _ = self.until(secs, |_, _| false).await;
    }

    /// `secs` world seconds, every tick's submissions landed and the arena's egress pumped as
    /// they pass. The arena's own clock is real time and barely moves meanwhile.
    fn pass(&mut self, secs: i64) {
        for n in 0..secs {
            let _ = second_composed(&mut self.rig);
            if n % 10 == 0 {
                let _ = pump(&mut self.rig, &mut self.rx, &mut self.lines);
            }
        }
        let _ = pump(&mut self.rig, &mut self.rx, &mut self.lines);
    }

    /// World seconds until the world's wall second is `wall_s` ([`Field::pass`]).
    fn pass_to(&mut self, wall_s: i64) {
        let left = wall_s.saturating_sub(self.rig.tw.world().now());
        self.pass(left.max(0));
    }

    /// The roster's leader, as its last saved reconciliation has it.
    fn leader(&mut self) -> Option<(ObjId, LeaderKind)> {
        field_roster(&mut self.rig, self.flown.operation)
            .and_then(|roster| roster.leader())
            .map(|leader| (leader.character, leader.kind))
    }

    /// The lapse standing on `slot`'s delivered order, when the roster's history records one.
    fn lapse(&mut self, slot: u16) -> Option<Lapse> {
        let slot = mush_tactics::detachment::Slot::new(slot).expect("a slot");
        operation(&mut self.rig, self.flown.operation)?
            .orders?
            .lapsed
            .get(&slot)
            .copied()
    }

    /// The leader's own order line to slot 2, under the fence `+sortie/orders` prints for them.
    fn order(&mut self, words: &str) {
        let leader = self.flown.organiser;
        let fenced = fence(&mut self.rig, leader, &self.flown);
        let _ = commit(&mut self.rig, leader, &format!("ORDER {fenced} 2 {words}"));
    }

    /// What `+sortie/orders` prints for the organiser.
    fn report(&mut self) -> String {
        let leader = self.flown.organiser;
        let line = format!(
            "+sortie/orders {}:{}",
            self.flown.operation.organizer.0,
            self.flown.selector()
        );
        run(&mut self.rig.tw, leader, &line)
    }

    /// **The intruder's player keeps a lock on the contact within two cells of `cell` and
    /// fires**, one pass of the two lines a player types.
    fn fire_on(&mut self, cell: (u16, u16)) {
        let (stranger, gun) = (self.stranger, weapon_of(&self.rig, self.intruder));
        let near = frame(&self.rig, stranger).and_then(|frame| {
            frame
                .contacts
                .iter()
                .find(|contact| {
                    contact
                        .cell
                        .is_some_and(|(x, y)| x.abs_diff(cell.0) <= 2 && y.abs_diff(cell.1) <= 2)
                })
                .map(|contact| (contact.id, contact.designator.to_string(), frame.lock))
        });
        if let Some((id, designator, lock)) = near
            && lock != Some(id)
        {
            let _ = run(&mut self.rig.tw, stranger, &format!("LOCK {designator}"));
        }
        let _ = run(&mut self.rig.tw, stranger, &format!("FIRE {gun}"));
    }

    /// The intruder's player ceases fire.
    fn cease(&mut self) {
        let gun = weapon_of(&self.rig, self.intruder);
        let _ = run(&mut self.rig.tw, self.stranger, &format!("FIRE {gun} OFF"));
    }
}

/// **The server goes down and comes up** (`driving_rig::crash`), `reconnect` back on.
///
/// A roster's journey lands comsys's and the transport's own documents on the composed path (a
/// channel, an order's envelope), and the rig keeps every composed module write as typed action
/// state for the next boot. Those documents are their modules' own and are no action state, so
/// they are handed to the boot as module data, where their owners load them.
async fn restart(mut rig: Rig, rx: &mut Receiver<ArenaOut>, reconnect: &[ObjId]) -> Rig {
    let own: Vec<(smol_str::SmolStr, smol_str::SmolStr)> = rig
        .typed
        .keys()
        .filter(|(namespace, _)| matches!(namespace.as_str(), "comsys" | "propagation"))
        .cloned()
        .collect();
    let mut ops = Vec::new();
    for key in own {
        if let Some(value) = rig.typed.remove(&key) {
            ops.push(mush_core::ModuleOp {
                ns: key.0,
                op: mush_core::ModuleWrite::JsonSet { key: key.1, value },
            });
        }
    }
    crate::driving_rig::fold_module_ops_into(&mut rig.store, ops);
    crash(rig, rx, reconnect).await
}

/// Drop every connection of `who` with `reason` (MW.7's helper).
fn drop_conns(rig: &mut Rig, who: ObjId, reason: mush_core::DisconnectKind) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| rig.tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason,
            },
            &mut out,
            &mut fx,
        );
    }
}

/// `who`'s link is lost: the server's own grace starts, and the field's.
fn lose_link(rig: &mut Rig, who: ObjId) {
    drop_conns(rig, who, mush_core::DisconnectKind::Lost);
}

/// `who` quits.
fn quit(rig: &mut Rig, who: ObjId) {
    drop_conns(rig, who, mush_core::DisconnectKind::Quit);
}

/// The server's grace runs out for `who`: their session ends.
fn end_session(rig: &mut Rig, who: ObjId) {
    let mut out = mush_core::Outbox::new();
    let mut fx = mush_core::Effects::new();
    rig.tw.engine_mut().step(
        mush_core::Inbound::SessionGone { actor: who },
        &mut out,
        &mut fx,
    );
}

/// What `who` hears when they connect.
fn connect(rig: &mut Rig, who: ObjId) -> String {
    let events = rig.tw.connect(who);
    said(&rig.tw, &events, who)
}

/// The order `who`'s own frame shows standing.
fn standing(rig: &Rig, who: ObjId) -> Option<MindObjective> {
    frame(rig, who).and_then(|frame| frame.order.map(|order| order.objective))
}

/// The rounds `shooter` fired at `target`.
fn fired_at(lines: &[LedgerEvent], shooter: ObjId, target: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| {
            matches!(&line.kind, EventKind::Fired { unit, target: at, .. }
                if *unit == shooter && *at == target)
        })
        .count()
}

/// The rounds `shooter` fired, at anyone.
fn fired(lines: &[LedgerEvent], shooter: ObjId) -> usize {
    lines
        .iter()
        .filter(|line| matches!(&line.kind, EventKind::Fired { unit, .. } if *unit == shooter))
        .count()
}

/// How many live Hostile contacts `who`'s own frame holds.
fn hostiles(rig: &Rig, who: ObjId) -> usize {
    frame(rig, who).map_or(0, |frame| {
        frame
            .contacts
            .iter()
            .filter(|contact| contact.relation == Relation::Hostile)
            .count()
    })
}

/// A mining roster: the captain in rig 1, `hired` in rig 2, `commander` the plan's.
fn mining(hired: &'static str, commander: Option<&'static str>) -> Plan {
    Plan {
        who: ("Pilot", 1, "Salt Moth"),
        duty: "mining",
        billets: vec![Billet::Leader, Billet::Hired(hired)],
        commander,
    }
}

/// **Story 1** (this module's head): an `ENGAGE` lapses after the grace, and the crew still
/// answers a hit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one story, in order
async fn ap8_an_absent_leaders_engage_lapses_and_the_crew_still_answers_a_hit() {
    let mut field = opened(
        &Plan {
            who: ("Pilot", 1, "Salt Moth"),
            duty: "combat",
            billets: vec![Billet::Leader, Billet::Hired(ADEYEMI)],
            commander: None,
        },
        true,
    )
    .await;
    let (captain, adeyemi) = (field.flown.organiser, field.flown.operator(2));
    let (lead_rig, tank) = (field.flown.unit(1), field.flown.unit(2));
    let intruder = field.intruder;
    assert_eq!(field.leader(), Some((captain, LeaderKind::Player)));

    // The captain marks the intruder's party Hostile and orders the escort onto it.
    let letter = tacsim::field::live_on(field.rig.tw.world(), &cid(AHUNA_FLANK.entry))
        .and_then(|live| live.party_of(intruder))
        .expect("the intruder's party")
        .letter();
    let marked = run(
        &mut field.rig.tw,
        captain,
        &format!("+field/mark {letter}=hostile"),
    );
    assert!(
        field.until(30, |rig, _| hostiles(rig, adeyemi) > 0).await,
        "the escort's own frame holds the intruder Hostile: {marked}"
    );
    field.order("ENGAGE");
    let pressing = field
        .until(90, |rig, lines| {
            matches!(standing(rig, adeyemi), Some(MindObjective::Engage { .. }))
                && fired_at(lines, tank, intruder) > 0
        })
        .await;
    assert!(
        pressing,
        "the escort presses its leader's ENGAGE: {:?}, {} rounds; book {:?}",
        standing(&field.rig, adeyemi),
        fired_at(&field.lines, tank, intruder),
        orders_of(&mut field.rig, field.flown.operation)
    );
    let engage = delivered_to(&mut field.rig, field.flown.operation, 2).expect("the ENGAGE");
    assert_eq!((engage.issuer, engage.issuer_unit), (captain, lead_rig));

    // The captain's link is lost. A minute on the server's own grace ends their session, which
    // gives up their console and so the lead (Sim's rule for any billet). The order is another
    // matter: half a minute short of five minutes it still stands. Its clock is the field's.
    lose_link(&mut field.rig, captain);
    let lost = away_of(field.rig.tw.world(), lead_rig)
        .expect("the clock runs")
        .since_wall_s;
    field.pass(60);
    end_session(&mut field.rig, captain);
    field.pass_to(lost + FIELD_LINKDEAD_GRACE_WALL_S - 40);
    field.wait(2).await;
    assert!(
        field.rig.tw.world().now() < lost + FIELD_LINKDEAD_GRACE_WALL_S,
        "still inside the five minutes"
    );
    assert!(
        away_of(field.rig.tw.world(), lead_rig).is_some_and(|away| !away.handed),
        "the captain's rig is still theirs short of five minutes: {:?}",
        away_of(field.rig.tw.world(), lead_rig)
    );
    assert_eq!(
        field.leader(),
        None,
        "the session's end gave up the helm, and with it the lead"
    );
    assert!(
        field.lapse(2).is_none()
            && matches!(
                standing(&field.rig, adeyemi),
                Some(MindObjective::Engage { .. })
            ),
        "the order stands inside the grace: {:?}",
        standing(&field.rig, adeyemi)
    );

    // Five minutes: the rig passes to its mind, the order lapses, and the escort holds.
    field.pass_to(lost + FIELD_LINKDEAD_GRACE_WALL_S + 5);
    assert!(
        minded(field.rig.tw.world(), lead_rig),
        "five minutes after the loss the captain's rig is its mind's"
    );
    let held = field
        .until(60, |rig, _| {
            standing(rig, adeyemi) == Some(MindObjective::Hold)
        })
        .await;
    let lapse = field.lapse(2);
    assert!(
        held && lapse.is_some_and(|lapse| lapse.sequence == engage.sequence),
        "the ENGAGE lapsed and the escort holds: {:?}, lapse {lapse:?}; book {:?}",
        standing(&field.rig, adeyemi),
        orders_of(&mut field.rig, field.flown.operation)
    );
    assert_eq!(
        field.leader(),
        None,
        "nobody took over: the roster nominated no commander"
    );
    // A round already in the air lands; then the escort fires nothing at the party its leader
    // marked, through two of its cannon's cycles and more.
    field.wait(2).await;
    let spent = fired(&field.lines, tank);
    let stood = frame(&field.rig, adeyemi).and_then(|frame| frame.cell);
    field.wait(14).await;
    assert_eq!(
        fired(&field.lines, tank),
        spent,
        "the lapsed attack does not run on: the escort fires nothing while it holds"
    );
    assert_eq!(
        frame(&field.rig, adeyemi).and_then(|frame| frame.cell),
        stood,
        "the escort stands where it stopped"
    );
    assert_eq!(standing(&field.rig, adeyemi), Some(MindObjective::Hold));

    // The intruder's player fires on the escort. Once a round hits, it answers from where it
    // stands.
    let cell = stood.expect("the escort's cell");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(150);
    let before = fired_at(&field.lines, tank, intruder);
    assert_eq!(
        frame(&field.rig, adeyemi).and_then(|frame| frame.own_last_hit_ms),
        None,
        "nothing has hit the escort yet"
    );
    let mut answered = false;
    while std::time::Instant::now() < deadline && !answered {
        field.fire_on(cell);
        answered = field
            .until(1, |_, lines| fired_at(lines, tank, intruder) > before)
            .await;
    }
    field.cease();
    assert!(
        frame(&field.rig, adeyemi).is_some_and(|frame| frame.own_last_hit_ms.is_some()),
        "the escort fired again only after a round hit it: {} rounds at it",
        fired_at(&field.lines, intruder, tank)
    );
    assert!(
        answered,
        "the escort answers the unit that hit it: order {:?}, hit {:?}, {} rounds at it",
        standing(&field.rig, adeyemi),
        frame(&field.rig, adeyemi).and_then(|frame| frame.own_last_hit_ms),
        fired_at(&field.lines, intruder, tank)
    );
    assert!(
        matches!(
            standing(&field.rig, adeyemi),
            Some(MindObjective::Screen { .. })
        ),
        "it answers from a stand, not a pursuit: {:?}",
        standing(&field.rig, adeyemi)
    );
    assert_eq!(
        frame(&field.rig, adeyemi).and_then(|frame| frame.cell),
        stood,
        "and does not leave its cell to do it"
    );

    // The captain is told at login, reads it in the order report, and finds it still lapsed.
    let heard = connect(&mut field.rig, captain);
    let told = ORDER_LAPSED
        .replace("{order}", "ENGAGE")
        .replace("{slot}", "2");
    assert!(
        heard.contains(&told),
        "the returning leader is told what lapsed: {heard}"
    );
    let report = field.report();
    assert!(
        report.contains(ORDER_LAPSED_STATUS),
        "the order report says so: {report}"
    );
    field.wait(4).await;
    assert_eq!(
        field.lapse(2),
        lapse,
        "the order stays lapsed after its leader's return"
    );
    assert!(
        !matches!(
            standing(&field.rig, adeyemi),
            Some(MindObjective::Engage { .. })
        ),
        "the lapsed ENGAGE does not come back with them: {:?}",
        standing(&field.rig, adeyemi)
    );
}

/// **Story 2** (this module's head): the plan's NPC commander replaces a lapsed order.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap8_the_npc_commander_replaces_a_lapsed_advance() {
    let mut field = opened(&mining(OKONKWO, Some(OKONKWO)), false).await;
    let (captain, okonkwo) = (field.flown.organiser, field.flown.operator(2));
    let (lead_rig, hired_rig) = (field.flown.unit(1), field.flown.unit(2));
    let operation = field.flown.operation;
    assert_eq!(field.leader(), Some((captain, LeaderKind::Player)));

    // The captain's own order: to a cell of their choosing.
    field.order(&format!("ADVANCE {},{}", FAR.0, FAR.1));
    assert!(
        field
            .until(60, |rig, _| standing(rig, okonkwo)
                == Some(MindObjective::Advance { to: FAR }))
            .await,
        "the captain's ADVANCE stands: {:?}; book {:?}",
        standing(&field.rig, okonkwo),
        orders_of(&mut field.rig, operation)
    );
    let theirs = delivered_to(&mut field.rig, operation, 2).expect("the captain's order");
    assert_eq!((theirs.issuer, theirs.issuer_unit), (captain, lead_rig));

    // The captain quits: the hand-over is at once, and so are the lapse and the succession.
    quit(&mut field.rig, captain);
    assert!(
        minded(field.rig.tw.world(), lead_rig),
        "a clean quit hands the rig to its mind at once"
    );
    assert!(
        field
            .until(60, |rig, _| field_roster(rig, operation)
                .and_then(|roster| roster.leader())
                .is_some_and(|leader| leader.character == okonkwo))
            .await,
        "the plan's commander leads once the captain is gone"
    );
    // Her own order replaces the lapsed one: hers, from her unit, to the work point.
    let replaced = field
        .until(90, |rig, _| {
            delivered_to(rig, operation, 2).is_some_and(|held| held.issuer == okonkwo)
                && matches!(standing(rig, okonkwo), Some(MindObjective::Advance { to }) if to != FAR)
        })
        .await;
    let hers = delivered_to(&mut field.rig, operation, 2).expect("an order in slot 2");
    assert!(
        replaced,
        "the commander's order replaces the lapsed one: {hers:?}; standing {:?}; book {:?}",
        standing(&field.rig, okonkwo),
        orders_of(&mut field.rig, operation)
    );
    assert_eq!((hers.issuer, hers.issuer_unit), (okonkwo, hired_rig));
    assert!(
        matches!(hers.objective, Some(FieldObjective::Advance { to }) if to != FAR),
        "to the work point, not the captain's cell: {hers:?}"
    );
    // The lapse was written with the write that made her the leader, and took the two
    // sequences of the stand before her order took the next.
    assert!(
        hers.sequence >= theirs.sequence + 3,
        "the captain's order lapsed before hers was issued: {} then {}",
        theirs.sequence,
        hers.sequence
    );
    assert_eq!(
        field.lapse(2),
        None,
        "a delivered order clears the lapse it replaces"
    );
    // The captain reads at login what lapsed.
    let heard = connect(&mut field.rig, captain);
    let told = ORDER_LAPSED
        .replace("{order}", &format!("ADVANCE {},{}", FAR.0, FAR.1))
        .replace("{slot}", "2");
    assert!(heard.contains(&told), "the captain is told: {heard}");
    assert!(
        !heard.contains(
            &ORDER_LAPSED_LEAD
                .replace("{order}", &format!("ADVANCE {},{}", FAR.0, FAR.1))
                .replace("{slot}", "2")
        ),
        "and is not told as the next leader: {heard}"
    );
}

/// **Story 3** (this module's head): a `HOLD` the leader left is still held.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap8_a_hold_the_leader_left_is_still_held() {
    let mut field = opened(&mining(MENDES, None), false).await;
    let (captain, mendes) = (field.flown.organiser, field.flown.operator(2));
    let lead_rig = field.flown.unit(1);
    let operation = field.flown.operation;

    field.order("HOLD");
    assert!(
        field
            .until(60, |rig, _| standing(rig, mendes)
                == Some(MindObjective::Hold))
            .await,
        "the HOLD stands: {:?}",
        standing(&field.rig, mendes)
    );
    let held = delivered_to(&mut field.rig, operation, 2).expect("the HOLD");
    let row = frame(&field.rig, mendes)
        .and_then(|frame| frame.order)
        .and_then(|order| order.identity);

    // The captain quits, and four hundred seconds pass.
    quit(&mut field.rig, captain);
    assert!(minded(field.rig.tw.world(), lead_rig));
    field.pass(400);
    field.wait(4).await;
    assert_eq!(field.lapse(2), None, "a HOLD does not lapse");
    assert_eq!(
        delivered_to(&mut field.rig, operation, 2),
        Some(held),
        "the order the leader left is the order the slot holds"
    );
    assert_eq!(standing(&field.rig, mendes), Some(MindObjective::Hold));
    assert_eq!(
        frame(&field.rig, mendes)
            .and_then(|frame| frame.order)
            .and_then(|order| order.identity),
        row,
        "and the same order row stands on the rig: nothing was offered in its place"
    );
    assert_eq!(field.leader(), None, "no commander was nominated");
    let heard = connect(&mut field.rig, captain);
    assert!(
        !heard.contains("lapsed"),
        "nobody is told of a lapse that did not happen: {heard}"
    );
}

/// **Story 4** (this module's head): a restart inside the grace neither restarts the clock nor
/// loses the lapse.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)] // one story, in order
async fn ap8_a_restart_keeps_the_clock_and_the_lapse() {
    let mut field = opened(&mining(MENDES, None), false).await;
    let (captain, mendes) = (field.flown.organiser, field.flown.operator(2));
    let lead_rig = field.flown.unit(1);
    let operation = field.flown.operation;

    field.order(&format!("ADVANCE {},{}", FAR.0, FAR.1));
    assert!(
        field
            .until(60, |rig, _| standing(rig, mendes)
                == Some(MindObjective::Advance { to: FAR }))
            .await,
        "the ADVANCE stands"
    );
    let advance = delivered_to(&mut field.rig, operation, 2).expect("the ADVANCE");

    // The link is lost; two minutes on the server goes down and comes up.
    lose_link(&mut field.rig, captain);
    let lost = away_of(field.rig.tw.world(), lead_rig).expect("the clock runs");
    assert!(!lost.handed);
    field.pass(100);
    field.wait(3).await;
    let Field {
        rig,
        flown,
        other,
        stranger,
        intruder,
        mut rx,
        ..
    } = field;
    let mut rig = restart(rig, &mut rx, &[other, stranger]).await;
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 30, |rig, _| task(rig)
            .is_some())
        .await,
        "the field resumes"
    );
    let mut field = Field {
        rig,
        flown,
        other,
        stranger,
        intruder,
        rx,
        lines,
    };
    let booted = field.rig.tw.world().now();
    assert_eq!(
        away_of(field.rig.tw.world(), lead_rig),
        Some(lost),
        "the boot keeps the loss the record already carries"
    );
    assert!(
        field
            .until(30, |rig, _| standing(rig, mendes)
                == Some(MindObjective::Advance { to: FAR }))
            .await,
        "inside the grace the order still stands after the restart: {:?}",
        standing(&field.rig, mendes)
    );
    assert_eq!(field.lapse(2), None);
    assert!(
        field.rig.tw.world().now() < lost.since_wall_s + FIELD_LINKDEAD_GRACE_WALL_S,
        "still inside the five minutes"
    );

    // Five minutes after the loss, some three after the boot: the rig is its mind's and the
    // order lapses. The clock did not start again.
    field.pass_to(lost.since_wall_s + FIELD_LINKDEAD_GRACE_WALL_S + 5);
    assert!(
        field.rig.tw.world().now() - booted < FIELD_LINKDEAD_GRACE_WALL_S,
        "less than five minutes have passed since the boot"
    );
    assert!(
        minded(field.rig.tw.world(), lead_rig),
        "the five minutes run from the loss, not from the boot: {:?}",
        away_of(field.rig.tw.world(), lead_rig)
    );
    let held = field
        .until(60, |rig, _| {
            standing(rig, mendes) == Some(MindObjective::Hold)
        })
        .await;
    let lapse = field.lapse(2);
    assert!(
        held && lapse.is_some_and(|lapse| lapse.sequence == advance.sequence),
        "the ADVANCE lapsed and the rig holds: {:?}, lapse {lapse:?}",
        standing(&field.rig, mendes)
    );

    // A second restart finds the order lapsed and the rig holding.
    field.wait(3).await;
    let Field {
        rig,
        flown,
        other,
        stranger,
        intruder,
        mut rx,
        ..
    } = field;
    let mut rig = restart(rig, &mut rx, &[other, stranger]).await;
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 30, |rig, _| task(rig)
            .is_some())
        .await,
        "the field resumes again"
    );
    let mut field = Field {
        rig,
        flown,
        other,
        stranger,
        intruder,
        rx,
        lines,
    };
    assert_eq!(
        field.lapse(2),
        lapse,
        "the lapse is in the roster's history"
    );
    assert!(
        field
            .until(30, |rig, _| standing(rig, mendes)
                == Some(MindObjective::Hold))
            .await,
        "the rig still holds: {:?}",
        standing(&field.rig, mendes)
    );
    field.wait(6).await;
    assert_eq!(
        standing(&field.rig, mendes),
        Some(MindObjective::Hold),
        "the lapsed ADVANCE is not offered again by the boot"
    );
    assert_eq!(field.lapse(2), lapse);
}

/// **Story 5** (this module's head): a lapsed order stays lapsed until its leader gives it
/// again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ap8_a_lapsed_order_stays_lapsed_until_its_leader_gives_it_again() {
    let mut field = opened(&mining(MENDES, None), false).await;
    let (captain, mendes) = (field.flown.organiser, field.flown.operator(2));
    let lead_rig = field.flown.unit(1);
    let operation = field.flown.operation;
    let words = format!("ADVANCE {},{}", FAR.0, FAR.1);

    field.order(&words);
    assert!(
        field
            .until(60, |rig, _| standing(rig, mendes)
                == Some(MindObjective::Advance { to: FAR }))
            .await,
        "the ADVANCE stands"
    );
    let first = delivered_to(&mut field.rig, operation, 2).expect("the ADVANCE");

    // The captain quits: the order lapses at once and the rig holds.
    quit(&mut field.rig, captain);
    assert!(minded(field.rig.tw.world(), lead_rig));
    let held = field
        .until(60, |rig, _| {
            standing(rig, mendes) == Some(MindObjective::Hold)
        })
        .await;
    let lapse = field.lapse(2).expect("the lapse");
    assert!(held && lapse.sequence == first.sequence, "{lapse:?}");

    // The captain reconnects while their rig is still in the field, and leads again.
    let heard = connect(&mut field.rig, captain);
    assert!(
        heard.contains("You are back at the controls"),
        "the rig was still on its way home: {heard}"
    );
    let told = ORDER_LAPSED
        .replace("{order}", &words)
        .replace("{slot}", "2");
    assert!(heard.contains(&told), "the captain is told: {heard}");
    // Their session's end gave up the console (Sim's own rule for any billet): they take the
    // focus and the helm back, as they took them before the launch.
    let _ = run(
        &mut field.rig.tw,
        captain,
        &format!("+focus #{}/pilot", lead_rig.0),
    );
    let asked = run(
        &mut field.rig.tw,
        captain,
        &format!("@station/handoff #{}/pilot", lead_rig.0),
    );
    let landed: Vec<String> = queued(&mut field.rig)
        .iter()
        .map(|submit| {
            format!(
                "{}: {:?}",
                submit.action.key().id,
                land_composed(&mut field.rig, submit)
            )
        })
        .collect();
    for _ in 0..8 {
        if apply_due(&mut field.rig, "control_handoff_apply") > 0 {
            break;
        }
        let _ = beat(&mut field.rig);
    }
    let leads = field
        .until(60, |rig, _| {
            field_roster(rig, operation)
                .and_then(|roster| roster.leader())
                .is_some_and(|leader| leader.character == captain)
        })
        .await;
    assert!(
        leads,
        "the returning player takes the lead: {:?}; hand-off {asked} / {landed:?}; away {:?}",
        field_roster(&mut field.rig, operation).map(|roster| roster.checkpoint()),
        away_of(field.rig.tw.world(), lead_rig)
    );
    // Back in command, the order is still lapsed: the rig holds, and the report says so.
    field.wait(4).await;
    assert_eq!(field.lapse(2), Some(lapse), "their return clears nothing");
    assert_eq!(standing(&field.rig, mendes), Some(MindObjective::Hold));
    let report = field.report();
    assert!(
        report.contains(ORDER_LAPSED_STATUS),
        "the order report says the order lapsed: {report}"
    );

    // Given again, it is an order like any other.
    field.order(&words);
    let again = field
        .until(90, |rig, _| {
            standing(rig, mendes) == Some(MindObjective::Advance { to: FAR })
        })
        .await;
    let second = delivered_to(&mut field.rig, operation, 2).expect("the second ADVANCE");
    assert!(
        again && second.sequence > lapse.answer,
        "the order given again is delivered and obeyed: {second:?}; standing {:?}",
        standing(&field.rig, mendes)
    );
    assert_eq!(
        field.lapse(2),
        None,
        "and the lapse is gone with the order it named"
    );
}
