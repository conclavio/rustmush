//! **Doc 44 unit MW.21: the completion story, with two accounts** (first review, finding 9).
//!
//! One player writes a mind, has the Registry rate it, files it and lists it. Another buys
//! it at the counter, seals it aboard her own ship and sails. With her link down, inside the
//! Kirkwood picket's bubble, the mind she bought flies her. The server restarts while it has
//! her helm. She comes back with its next think on its way, and her first command is what
//! the ship does. She leaves again, the mind brings her through customs, and the claim the
//! seal opened closes once when she docks.
//!
//! The world is the combined M8 world (the production `LaunchComposition`, `m8_world`): the
//! Roll, the Registry, the player market and the claim path are the launch's own. The story
//! takes one genuine Registry rating, so the CI dispatch runs it.
//!
//! **Neither player is staff.** Every act of theirs is a registered command typed by a
//! character on an account of their own, landed on the composed path. Each pays from the
//! wallet their Co-op charter opened: nothing is minted.
//!
//! **What staff do, and why.** Each is a gap in the game or in this rig:
//!
//! 1. *Her core.* No ship ships with a core and nothing in play fits one to a ship (doc 44
//!    unit MW.13's cut note). Staff bind the buyer's chartered Dray to the Dray's governed
//!    Class II root, as MW.13's stories do.
//! 2. *The way to the counter.* The launch's counter is a room staff made with no door (the
//!    M8 rig's venue). Staff carry the author there, and the buyer there and back to the
//!    berth gallery; she leaves her ship and boards it again by her own verbs.
//! 3. *Her tank and locker.* The rig's `docked_dray` stows her reaction mass and provisions
//!    by staff's hand, as every ship story's does.
//!
//! **What the story does by hand.** It drops and raises the players' connections, restarts
//! the server over its kept store (three times), and moves the clock: past the link-loss grace, to the
//! instant a leg of her passage ends, and a second at a time inside the bubble. It waits out
//! no wall-clock window (doc 44 §7, "Time in a story").

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use mush_core::{GOD, ObjId};
use serde_json::{Value, json};
use tacsim::definitions::core_replacement::SHIP_II;
use tacsim::port::ship_mind;
use tacsim::space::bubble::{BubbleInputCommand, BubbleInputEntry, EmconPosture};
use tacsim::space::thinks::Owner;
use tacsim::space::{PicketInspectionStatus, SpaceBubbleBook};
use tacsim::surface::excursion::ClaimCursor;

use crate::excursions::{Rig, docked_dray, economy, run, said};
use crate::m8_world::{M8World, fires, m8_world_manned};
use crate::mw13_ship_mind::{
    arrive_at_the_picket, claim_log, cursor, one_second, sail_and_lose_the_link, seal_aboard,
};

/// The thinks the mind takes to heave her to: it sheds her drift against the hailing ship
/// in this many equal burns, where the house default brakes with all her drive gives.
const PACE_THINKS: i64 = 24;

/// The speed against the hailing ship, cm/s, the mind brakes to: under the picket's ten
/// metres a second.
const PACE_TO_CMS: i64 = 950;

/// The launch's filing fee and the listing's price, centicredits, and what of the price is
/// the charter owner's (the Registry keeps 5%).
const FEE_MINOR: i64 = 10_000;
const PRICE_MINOR: i64 = 40_000;
const AUTHORS_SHARE_MINOR: i64 = 38_000;

/// **The author's script.** Outside a bubble it makes for port. Inside one it asks, at every
/// think, a shot of every mount the hull carries. Hailed, it locks the ship that hails her.
/// On its first think under the hail it asks a burn that would run the picket, with
/// everything her drive gives; from its second it brakes, in [`PACE_THINKS`] equal burns.
/// Hove to, it stands for customs. Its trace words are its own. A cold hull (the Registry's
/// trials begin cold) it leaves cold, so in a trial it asks nothing.
fn warden() -> String {
    format!(
        "fn think(v, c) {{ \
           if v.power != \"online\" {{ return; }} \
           let s = space(v); \
           if s == () {{ speed(1000); trace(\"WARDEN_MAKE_FOR_PORT\"); return; }} \
           if v.weapons.len() == 0 {{ fire(\"cannon\", 1); }} else {{ for w in v.weapons {{ fire(w.id, 1); }} }} \
           if s.stop == \"hailed\" && s.stop_by != () {{ \
             lock(s.stop_by); \
             let by = space_contact(v, s.stop_by); \
             if by != () {{ \
               let dx = by.vx_cms - s.vx_cms; let dy = by.vy_cms - s.vy_cms; let dz = by.vz_cms - s.vz_cms; \
               let d = isqrt(dx * dx + dy * dy + dz * dz); \
               if d > 0 {{ \
                 if !(\"pace\" in this) {{ \
                   this.pace = max((d - {PACE_TO_CMS}) * 1000 / ({PACE_THINKS} * s.step_ms), 1); \
                   helm(course_of(-dx, -dy, -dz)); speed(max(s.accel_cms2, 1)); \
                   trace(\"WARDEN_RUN\"); return; \
                 }} \
                 helm(course_of(dx, dy, dz)); speed(this.pace); \
                 trace(\"WARDEN_HEAVE_TO\"); return; \
               }} \
             }} \
           }} \
           hold(); \
           if v.held {{ trace(\"WARDEN_STAND_FOR_CUSTOMS\"); }} else {{ trace(\"WARDEN_STAND\"); }} \
         }}"
    )
}

/// The burn the script's own arithmetic gives a hull that drifts at `drift_mmps` against the
/// ship that hails her, mm/s², where the bubble steps a second at a time.
fn paced_brake_mmps2(drift_mmps: i64) -> i64 {
    ((drift_mmps / 10 - PACE_TO_CMS) * 1_000 / (PACE_THINKS * 1_000)).max(1) * 10
}

/// The length of a thrust, mm/s², rounded down.
fn strength(thrust: [i64; 3]) -> i64 {
    let squares: i64 = thrust.iter().map(|part| part * part).sum();
    (0..=squares)
        .take_while(|root| root * root <= squares)
        .last()
        .unwrap_or(0)
}

/// Every line of `text` holds under 120 columns (the owner's terminal floor).
fn fits(text: &str) {
    for line in text.lines() {
        assert!(
            line.chars().count() < 120,
            "a client line of {} columns: {line}",
            line.chars().count()
        );
    }
}

/// **Type `line` as `who` and land what it submits** on the composed path, as the server's
/// driver lands a command's action. What `who` read, and how many submissions it staged.
fn types_counting(world: &mut M8World, who: ObjId, line: &str) -> (String, usize) {
    // What a tick queued before the line is the tick's, not the typist's.
    let earlier = crate::excursions::drain(&mut world.rig.tw);
    crate::excursions::land_quietly(&mut world.rig, &earlier);
    let heard = run(&mut world.rig.tw, who, line);
    let mut staged = Vec::new();
    while let Some(submit) = world.rig.tw.engine_mut().take_authoritative_submit() {
        staged.push(*submit);
    }
    for submit in &staged {
        assert_eq!(
            submit.actor, who,
            "{line}: the submission is its typist's: {submit:?}"
        );
        crate::excursions::land(&mut world.rig, submit)
            .unwrap_or_else(|refused| panic!("{line}: {refused}"));
    }
    eprintln!("MW21 #{} types `{line}` -> {heard}", who.0);
    (heard, staged.len())
}

/// [`types_counting`], what `who` read.
fn types(world: &mut M8World, who: ObjId, line: &str) -> String {
    types_counting(world, who, line).0
}

/// A row's number under `field`, as the Roll writes it (a number, or its text).
fn number(row: &Value, field: &str) -> u64 {
    row.get(field)
        .and_then(|n| n.as_u64().or_else(|| n.as_str()?.parse().ok()))
        .unwrap_or_else(|| panic!("the row's {field}: {row}"))
}

/// `who`'s wallet: the posted balance of their one unscoped credit account, centicredits.
fn wallet(world: &M8World, who: ObjId) -> i64 {
    crate::m8_end_user::credit(world, who).1
}

/// **Every account in the world, by its id**: its owner and its posted balance.
fn accounts(world: &M8World) -> BTreeMap<u64, (String, i64)> {
    economy(&world.rig.tw)
        .snapshot()
        .accounts()
        .map(|account| {
            (
                account.id.0,
                (
                    format!("{:?}", account.owner),
                    account.posted_balance.minor(),
                ),
            )
        })
        .collect()
}

/// The typed rows the rig keeps, by owner and family: what a restart boots from.
fn families(world: &M8World) -> BTreeMap<String, BTreeMap<String, Value>> {
    let mut by_family: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    for ((ns, key), value) in &world.rig.typed {
        let family = value["family"]["id"].as_str().unwrap_or("?").to_owned();
        by_family
            .entry(format!("{ns}/{family}"))
            .or_default()
            .insert(key.to_string(), value["value"].clone());
    }
    by_family
}

/// The families whose rows differ between two readings, and how many rows of each.
fn moved(
    before: &BTreeMap<String, BTreeMap<String, Value>>,
    after: &BTreeMap<String, BTreeMap<String, Value>>,
) -> BTreeMap<String, usize> {
    let mut out = BTreeMap::new();
    for family in before.keys().chain(after.keys()) {
        let (a, b) = (before.get(family), after.get(family));
        let keys: std::collections::BTreeSet<&String> = a
            .into_iter()
            .chain(b)
            .flat_map(|rows| rows.keys())
            .collect();
        let n = keys
            .into_iter()
            .filter(|key| a.and_then(|rows| rows.get(*key)) != b.and_then(|rows| rows.get(*key)))
            .count();
        if n > 0 {
            out.insert(family.clone(), n);
        }
    }
    out
}

/// Her hold's reaction mass, tonnes, and provisions, crew-days, as the inventory holds them.
fn stores(world: &mut M8World) -> (u64, u64) {
    let vault = crate::excursions::hold_vault(&world.rig.tw, &world.row);
    (
        crate::excursions::pool(&mut world.rig.tw, vault, "propellant"),
        crate::excursions::pool(&mut world.rig.tw, vault, "provisions"),
    )
}

/// **The books a mind might spend from**, read together: every account in the world, the
/// rig's typed rows, and her hold.
struct Books {
    accounts: BTreeMap<u64, (String, i64)>,
    typed: BTreeMap<String, BTreeMap<String, Value>>,
    stores: (u64, u64),
}

fn books(world: &mut M8World) -> Books {
    Books {
        accounts: accounts(world),
        typed: families(world),
        stores: stores(world),
    }
}

/// **The Roll's claim book for `hull`**: every row of its `roll_mind_ic_claim` family that
/// names her.
fn claim_rows(world: &M8World, hull: ObjId) -> Vec<Value> {
    world
        .rows("roll_mind_ic_claim")
        .into_iter()
        .filter(|row| {
            row["open"]["unit"] == json!(hull.0) || row["closed"]["unit"] == json!(hull.0)
        })
        .collect()
}

/// Who has her helm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Helm {
    /// Her crew: no link-loss record stands on her.
    Crew,
    /// Her crew's link is down and inside its grace.
    Grace,
    /// A mind, for her absent crew.
    Mind,
}

/// **One reading of her in the picket's bubble**, taken after a step or a command.
#[derive(Clone, Debug)]
struct Reading {
    /// The bubble's own step count.
    step: u32,
    /// Her ship in the bubble, and the customs corvette.
    ship: String,
    corvette: String,
    /// What the bubble's tape gained that names her ship since the last reading, in order.
    wrote: Vec<BubbleInputCommand>,
    /// Her lock.
    lock: Option<String>,
    /// The ships whose fire has arrived on her, by the bubble's own record (doc 45 unit
    /// AP.9).
    fired_on_by: std::collections::BTreeSet<String>,
    /// Her commanded thrust, mm/s².
    thrust: [i64; 3],
    /// Her speed against the corvette, mm/s.
    drift: i64,
    /// Her emission posture.
    emcon: EmconPosture,
    /// Her reaction mass in the bubble, kilograms.
    propellant_kg: u32,
    /// What customs have made of her.
    status: PicketInspectionStatus,
    /// Who has her helm.
    helm: Helm,
    /// The last word her mind traced, while a mind has her.
    trace: Option<String>,
    /// How many thinks of her mind's have landed since it took her.
    thinks: u64,
    /// Whether the bubble owes its minds' thinks, and they are on the host.
    think_in_flight: bool,
    /// Every think the host has been handed so far, counting each source of a backup seat.
    handed: u64,
}

impl Reading {
    /// Her lock set on the corvette, as the tape holds it.
    fn locks_on(&self) -> BubbleInputCommand {
        BubbleInputCommand::SetLockedTarget {
            ship_id: self.ship.clone(),
            target_id: Some(self.corvette.clone()),
        }
    }

    /// Whether her lock is the corvette.
    fn locks_the_corvette(&self) -> bool {
        self.lock.as_deref() == Some(self.corvette.as_str())
    }

    /// The one burn the tape gained for her, if that is all it gained.
    fn burned(&self) -> Option<[i64; 3]> {
        match self.wrote.as_slice() {
            [BubbleInputCommand::CommandBurnMmps2 { thrust_mmps2, .. }] => Some(*thrust_mmps2),
            _ => None,
        }
    }

    /// The word her mind last traced.
    fn word(&self) -> &str {
        self.trace.as_deref().unwrap_or("")
    }
}

/// **The story's watch on her in the bubble**: the tape as it was last read, so each
/// reading holds what the tape gained. The tape is cut back to nothing when it is long
/// (`MAX_BUBBLE_TAPE_ENTRIES`), so what was read is kept and compared.
#[derive(Default)]
struct Watch {
    seen: Vec<BubbleInputEntry>,
    log: Vec<Reading>,
}

/// Whether `command` is a command of `ship`'s.
fn names(command: &BubbleInputCommand, ship: &str) -> bool {
    use BubbleInputCommand as C;
    match command {
        C::CommandBurn { ship_id, .. }
        | C::CommandBurnMmps2 { ship_id, .. }
        | C::SetAttitude { ship_id, .. }
        | C::SetPdcAuto { ship_id, .. }
        | C::SetLaserMode { ship_id, .. }
        | C::SetEmcon { ship_id, .. }
        | C::SetLockedTarget { ship_id, .. }
        | C::LaunchDecoy { ship_id }
        | C::BlowdownCoolant { ship_id }
        | C::RepairCrit { ship_id, .. }
        | C::WorkRock { ship_id, .. }
        | C::PlaceShip { ship_id, .. }
        | C::SetOre { ship_id, .. } => ship_id == ship,
        C::FireTorpedoSalvo { shooter_id, .. }
        | C::FireOffensiveLasers { shooter_id, .. }
        | C::FireRailgun { shooter_id, .. } => shooter_id == ship,
        C::StepWall { .. } => false,
    }
}

impl Watch {
    /// Read her now; `None` once she is in no bubble.
    fn read(&mut self, rig: &Rig, hull: ObjId) -> Option<Reading> {
        let world = rig.tw.world();
        let book = world
            .module_state::<SpaceBubbleBook>()
            .expect("the space book");
        let active = book.active_for_hull(hull)?;
        let picket = active.picket.as_ref()?;
        let ship_id = active.carrier_ship_id_for(hull).to_owned();
        let ship = active.bubble.ships.get(&ship_id)?;
        let corvette = active.bubble.ships.get(&picket.corvette_ship_id)?;
        let entries = &active.tape.entries;
        let fresh =
            if entries.len() >= self.seen.len() && entries[..self.seen.len()] == self.seen[..] {
                &entries[self.seen.len()..]
            } else {
                &entries[..]
            };
        let wrote = fresh
            .iter()
            .map(|entry| entry.command.clone())
            .filter(|command| names(command, &ship_id))
            .collect();
        self.seen.clone_from(entries);
        let linkdead = book.linkdead_state(hull);
        let host = tacsim::space::thinks::host_of(world);
        let think_in_flight = active.thinks_owed.is_some_and(|owed| {
            host.as_ref().is_some_and(|host| {
                host.in_flight(
                    Owner::Bubble(active.domain),
                    (active.opened_ic_s, owed.step),
                )
            })
        });
        let reading = Reading {
            step: active.steps,
            ship: ship_id.clone(),
            corvette: picket.corvette_ship_id.clone(),
            wrote,
            lock: ship.locked_target_id.clone(),
            fired_on_by: active
                .bubble
                .fired_on_by(&ship_id)
                .cloned()
                .unwrap_or_default(),
            thrust: ship.thrust_mmps2,
            drift: tacsim::space::relative_speed_3d_mmps(ship.vel_mmps, corvette.vel_mmps),
            emcon: ship.emcon,
            propellant_kg: ship.propellant_kg,
            status: picket.inspection_status_for(hull),
            helm: match linkdead {
                None => Helm::Crew,
                Some(state) if state.backup_has_helm() => Helm::Mind,
                Some(_) => Helm::Grace,
            },
            trace: linkdead.and_then(|state| state.mind_last_trace.clone()),
            thinks: linkdead.map_or(0, |state| state.mind_think_seq),
            think_in_flight,
            handed: host.map_or(0, |host| host.stats().thinks),
        };
        eprintln!("MW21 at the picket {reading:?}");
        self.log.push(reading.clone());
        Some(reading)
    }

    /// One wall second on, then [`Watch::read`]: she is still at the picket.
    fn step(&mut self, rig: &mut Rig, hull: ObjId) -> Reading {
        one_second(rig);
        self.read(rig, hull).expect("she is at the picket")
    }

    /// Step until `done` reads true of her or `seconds` are up; the last reading.
    fn until(
        &mut self,
        rig: &mut Rig,
        hull: ObjId,
        seconds: u32,
        mut done: impl FnMut(&Reading) -> bool,
    ) -> Reading {
        let mut last = self.step(rig, hull);
        for _ in 1..seconds {
            if done(&last) {
                break;
            }
            last = self.step(rig, hull);
        }
        last
    }
}

/// Whether a mind has `hull`'s helm for her absent crew, in a bubble or out of one.
fn mind_has_her(rig: &Rig, hull: ObjId) -> bool {
    rig.tw
        .world()
        .module_state::<SpaceBubbleBook>()
        .and_then(|book| book.linkdead_state(hull))
        .is_some_and(tacsim::space::SpaceLinkdeadState::backup_has_helm)
}

/// Drop every connection `who` holds as a clean QUIT: the presence hook reads a crew that
/// has left, not a lost link.
fn quit(rig: &mut Rig, who: ObjId) {
    let conns: Vec<u64> = (1..=64)
        .filter(|conn| rig.tw.engine().actor_of(*conn) == Some(who))
        .collect();
    assert!(!conns.is_empty(), "{who} holds a connection");
    for conn in conns {
        rig.tw.engine_mut().step(
            mush_core::Inbound::ConnDown {
                conn,
                actor: who,
                reason: mush_core::DisconnectKind::Quit,
            },
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        );
    }
}

/// **Fly her from the picket to a berth**: a second at a time while a bubble holds her, to
/// the end of her journey's next leg while none does. Whether she docked.
fn to_a_berth(rig: &mut Rig, hull: ObjId, watch: &mut Watch) -> bool {
    for _ in 0..4_000 {
        if tacsim::port::ship::berth_of(rig.tw.world(), hull)
            .hangar()
            .is_some()
        {
            return true;
        }
        if watch.read(rig, hull).is_some() {
            one_second(rig);
            continue;
        }
        let due = rig
            .tw
            .world()
            .module_state::<mush_sim::JourneyBook>()
            .and_then(|book| book.get(hull))
            .and_then(|journey| {
                (0..journey.route().legs().len()).find_map(|leg| {
                    journey
                        .leg_due_at(leg)
                        .filter(|due| *due > crate::excursions::ic_now(&rig.tw))
                })
            });
        match due {
            Some(due) => {
                let _ = crate::excursions::to_ic(rig, due);
            }
            None => one_second(rig),
        }
    }
    false
}

/// The combined world with the buyer's chartered Dray docked at Ceres, the buyer aboard, and
/// the author ashore at the Registry desk: two players on two accounts.
fn world() -> (M8World, ObjId) {
    let mut author = None;
    let world = m8_world_manned(None, |rig| {
        let (buyer, row) = docked_dray(rig, ("Buyer", 1, "Salt Moth"), (53, 19));
        // The author is a pilot too: the Co-op's charter is what opens a player's wallet.
        let writer = crate::excursions::pilot(rig, "Author", 2);
        let enrolled = run(&mut rig.tw, writer, "+coop/enrol Ink Moth");
        assert!(enrolled.contains("now the Ink Moth"), "{enrolled}");
        author = Some(writer);
        (buyer, row, Vec::new())
    });
    (world, author.expect("the author"))
}

/// **The house default at the Kirkwood picket, on the same passage with nothing sealed**:
/// the half the story's own flight is compared with. The same world, the same hull on the
/// same core, the same berth and the same filed passage; her pilot's link lost at the same
/// place. What her watch read, step by step, until customs had ruled on her.
fn house_default_at_the_picket() -> Vec<Reading> {
    let (mut world, _) = world();
    let (buyer, hull) = (world.pilot, ObjId(world.row.hull));
    crate::ic_sortie_surface::fit(&mut world.rig, hull, SHIP_II);
    assert_eq!(ship_mind::aboard(world.rig.tw.world(), hull), None);
    let boundary = sail_and_lose_the_link(&mut world.rig, buyer, hull);
    arrive_at_the_picket(&mut world.rig, hull, boundary);
    let mut watch = Watch::default();
    let _ = watch.read(&world.rig, hull).expect("she is at the picket");
    let _ = watch.until(&mut world.rig, hull, 60, |now| {
        !matches!(
            now.status,
            PicketInspectionStatus::AwaitingHeaveTo
                | PicketInspectionStatus::HoveToReadyForInspection
        )
    });
    watch.log
}

/// **The author writes, saves, rates and files their mind** by the workshop's and the
/// Registry's verbs (`@mind/new`, `@mind/line`, `@mind/save`, `@mind/rate`, `@mind/file`),
/// paying the filing fee from their own wallet. The filed number and the owner's licence.
///
/// It is `c4_filed_mind_field::file_own_mind` for a named author, without that helper's
/// wallet top-up.
async fn writes_rates_and_files(
    world: &mut M8World,
    author: ObjId,
    name: &str,
    source: &str,
) -> (u64, u64) {
    use mush_roll::commerce::rating::{RatingJob, RatingJobState, RatingReceipt};
    let heard = types(world, author, &format!("@mind/new {name}"));
    assert!(heard.contains("Mind draft created"), "{heard}");
    let lines = world
        .rig
        .tw
        .world()
        .module_state::<mush_roll::RollStore>()
        .and_then(|store| store.mind(author, name).ok())
        .map(|mind| mind.draft().source.lines().count())
        .expect("the draft");
    let heard = types(world, author, &format!("@mind/line {name} 1={source}"));
    assert!(heard.contains("Working source updated"), "{heard}");
    if lines > 1 {
        let heard = types(world, author, &format!("@mind/delete {name} 2-{lines}"));
        assert!(heard.contains("Working source updated"), "{heard}");
    }
    let _ = types(world, author, &format!("@mind/save {name}=first"));
    let saved = |world: &M8World| -> Option<mush_roll::Revision> {
        let store = world
            .rig
            .tw
            .world()
            .module_state::<mush_roll::RollStore>()?;
        let mind = store.mind(author, name).ok()?;
        mind.latest()
            .and_then(|reference| store.revision(author, reference).ok())
            .cloned()
    };
    let deadline = Instant::now() + Duration::from_secs(300);
    let revision = loop {
        world.pulse();
        if let Some(revision) = saved(world) {
            break revision;
        }
        assert!(
            Instant::now() < deadline,
            "the author service did not save {name}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    };
    let heard = types(world, author, &format!("@mind/rate {name}@1"));
    assert!(heard.contains("Registry preparation queued"), "{heard}");
    let identity =
        mush_roll::commerce::RevisionIdentity::capture(&revision).expect("the revision identity");
    let started = Instant::now();
    let deadline = started + Duration::from_secs(7_200);
    loop {
        world.pulse();
        let done = world.rows("rating_receipts").into_iter().any(|wire| {
            serde_json::from_value::<RatingReceipt>(wire)
                .is_ok_and(|receipt| receipt.subject().revision() == identity)
        });
        if done {
            break;
        }
        // A job that ends without a receipt is the story's failure, said at once.
        for job in world
            .rows("rating_jobs")
            .into_iter()
            .filter_map(|row| serde_json::from_value::<RatingJob>(row).ok())
            .filter(|job| job.subject().revision() == identity)
        {
            match job.state() {
                RatingJobState::Queued {}
                | RatingJobState::Running { .. }
                | RatingJobState::Completed { .. } => {}
                other => panic!("the Registry ended {name}'s rating without a receipt: {other:?}"),
            }
        }
        assert!(
            Instant::now() < deadline,
            "the Registry did not rate {name}"
        );
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    eprintln!(
        "MW21_STORY rated {name} in {} s",
        started.elapsed().as_secs()
    );
    // `@mind/file` first reserves the charter's shell; typed again once that has landed, it
    // files. The Registry releases its job a pulse after the receipt, so the line is typed
    // until the filing itself has been staged and landed.
    let before = world.rows("roll_mind_initial_filing");
    let deadline = Instant::now() + Duration::from_secs(300);
    loop {
        let (heard, staged) = types_counting(world, author, &format!("@mind/file {name}@1"));
        if heard.contains("Filing ") && staged > 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the filing never landed: {heard}"
        );
        world.pulse();
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let filings: Vec<_> = world
        .rows("roll_mind_initial_filing")
        .into_iter()
        .filter(|row| !before.contains(row))
        .collect();
    let [filing] = filings.as_slice() else {
        panic!("one new filing, the author's: {filings:?}")
    };
    assert_eq!(
        filing["requester"],
        json!(author.0),
        "the filing is the author's: {filing}"
    );
    (number(filing, "filed"), number(filing, "licence"))
}

/// **Her mind's first two thinks after it takes her, under the hail, read off the tape.**
/// The first locks the corvette and asks a burn that would run the picket: the lock is
/// written and no burn is, and she drifts as she did. The second brakes, at the strength
/// the script's own arithmetic gives her drift. The brake's thrust.
fn runs_then_brakes(took: &Reading, ran: &Reading, braked: &Reading) -> [i64; 3] {
    assert_eq!(ran.word(), "WARDEN_RUN", "her mind's own word: {ran:?}");
    assert_eq!(
        ran.wrote,
        vec![ran.locks_on()],
        "its first think locks the ship that hails her, and writes no burn: {ran:?}"
    );
    assert_eq!(
        (ran.thrust, ran.drift),
        ([0, 0, 0], took.drift),
        "the burn that would run the picket is not flown: {ran:?}"
    );
    assert_eq!(braked.word(), "WARDEN_HEAVE_TO", "{braked:?}");
    let brake = braked
        .burned()
        .unwrap_or_else(|| panic!("its second think writes one burn: {braked:?}"));
    assert_eq!(
        strength(brake),
        paced_brake_mmps2(took.drift),
        "the brake is the script's own pace for her drift: {braked:?}"
    );
    assert!(
        braked.drift < ran.drift && braked.thrust == brake,
        "and it takes her drift off: {braked:?}"
    );
    brake
}

/// **Doc 44 unit MW.21: the completion story** (the module head has what staff do).
///
/// 1. *Two accounts.* The author writes a script, has the Registry rate it, files it and
///    lists it sealed at the counter, paying the filing fee. The buyer, on another account,
///    reads the listing, asks a quote, pays it into escrow and takes the licence. The
///    author's wallet gains the price less the Registry's share and the buyer's loses the
///    price.
/// 2. *She fits it and takes her out.* The buyer seals the licence she bought aboard her
///    Dray at Ceres: the Roll opens one claim at her berth, with her as its principal, over
///    her governed core, with the helm and the sensors and no fire channel. She files for
///    Stickney and loses her link.
/// 3. *A choice the house default would not make.* At the Kirkwood picket the same hull on
///    the same passage with nothing sealed is braked in one step with all her drive gives,
///    locks nothing and is cleared in three. The mind she bought locks the corvette and
///    brakes at an eighth of that or less, by its own arithmetic, and she is still under
///    the hail many steps on.
/// 4. *The absent-player limits.* Its first think asks a burn that would run the picket:
///    none is written. Every think asks a shot of her lasers with the lock standing: none of
///    its shots is written, and nothing of its own but a lock and burns that take her drift
///    off. Beside it the house default answers fire under attack (doc 47 unit CO.2,
///    R-47-2): a shot of hers on the tape is at a ship whose fire had already arrived on her,
///    never at the corvette she complies with, and never moves the lock the bought mind set.
/// 5. *Only her own stores.* From her sailing to her berth at Stickney no account in the
///    world moves, the author's and the buyer's among them, no row of the market, the Roll
///    or the economy moves, and the one inventory pool that does is her hold's reaction
///    mass, which is less.
/// 6. *A restart while it has her helm*, twice. On the corridor before the picket: the seal
///    is the same and the picket still opens on her. Inside the bubble, six thinks in: the
///    seal reads the same class, principal and claim from the claim book; the bubble reads
///    back as it stood; the mind's next think is the brake it remembered, not a first think.
/// 7. *She comes back with its next think on its way.* The hand-back cuts its burn. Her
///    first command clears its lock. For six steps nothing of the mind's lands: no lock, no
///    burn, her drift as it was. Then she quits, the mind has her at once, and the same
///    mind's first think in the same place locks her again: the think that was struck would
///    have. In between her link blinks, back and gone inside one step with the new seat's
///    first think on the host: that think does not land either.
/// 8. *The claim closes once when she docks.* The mind stands her for customs and she makes
///    Stickney with nobody aboard connected. The Roll's claim book holds one row for her,
///    closed `returned_to_base` at Stickney; six fires and a restart later it is the same
///    row. The buyer, back aboard, reads no mind sealed.
#[tokio::test(flavor = "current_thread")]
#[ignore = "MW.21 story: one genuine Registry rating; the CI dispatch runs it"]
#[allow(clippy::too_many_lines)]
async fn mw21_a_bought_mind_flies_its_buyers_ship_and_her_first_command_wins() {
    let _log = claim_log();
    let started = Instant::now();
    let phase = |name: &str| {
        eprintln!(
            "MW21_STORY phase={name} event=complete s={}",
            started.elapsed().as_secs()
        );
    };

    // 3 (the other half). The house default on this passage, with nothing sealed.
    let house = house_default_at_the_picket();
    let house_brake = house
        .iter()
        .find_map(Reading::burned)
        .map(strength)
        .expect("the house default brakes");
    let house_hove_at = house
        .iter()
        .position(|now| now.status != PicketInspectionStatus::AwaitingHeaveTo)
        .expect("the house default heaves her to");
    assert!(
        house.iter().all(|now| now.lock.is_none()),
        "the house default locks nothing: {house:?}"
    );
    assert!(
        house
            .iter()
            .all(|now| now.word().is_empty() || now.word().starts_with("BACKUP_")),
        "the house default's own words: {house:?}"
    );
    assert_eq!(
        house.last().map(|now| now.status),
        Some(PicketInspectionStatus::Cleared),
        "and customs clear her: {house:?}"
    );
    assert!(
        house_hove_at <= 2 && house.len() <= 6,
        "the house default has her hove to in a step or two: {house:?}"
    );
    phase("house_default");

    let (mut world, author) = world();
    let (buyer, hull) = (world.pilot, ObjId(world.row.hull));
    let _ = world.rig.tw.connect(author);

    // 1. Two accounts, two characters, neither staff.
    assert!(author != GOD && buyer != GOD && author != buyer);
    assert!(
        crate::excursions::two_accounts(&world.rig, author, buyer),
        "the author and the buyer are characters of two accounts"
    );
    for who in [author, buyer] {
        assert!(world.rig.tw.world().is_connected(who), "{who} is connected");
        assert_eq!(
            run(&mut world.rig.tw, who, "@econ/mint 1=2 100"),
            "Permission denied.",
            "a staff verb is refused {who}"
        );
    }
    // What the gap forces: staff bind her core.
    crate::ic_sortie_surface::fit(&mut world.rig, hull, SHIP_II);

    // The author writes, rates and files, from their own wallet.
    let wallets = (wallet(&world, author), wallet(&world, buyer));
    let (filed, owner_licence) =
        writes_rates_and_files(&mut world, author, "Warden", &warden()).await;
    assert_eq!(
        wallet(&world, author),
        wallets.0 - FEE_MINOR,
        "the author paid the filing fee"
    );
    phase("filed");

    // The author lists it at the counter.
    let counter = world.deployment.venue.room;
    crate::excursions::teleport(&mut world.rig.tw, author, counter);
    let listed = types(
        &mut world,
        author,
        &format!("@mind/publish {filed} form=sealed"),
    );
    assert!(
        listed.contains(&format!("Listing filed mind {filed} here as sealed.")),
        "{listed}"
    );
    fits(&listed);

    // The buyer goes ashore, reads the counter's list, and buys.
    let ashore = types(&mut world, buyer, "+ship/leave");
    assert_ne!(
        tacsim::space::hull_aboard_for_actor(world.rig.tw.world(), buyer),
        Some(hull),
        "she is ashore: {ashore}"
    );
    crate::excursions::teleport(&mut world.rig.tw, buyer, counter);
    let board = types(&mut world, buyer, "+minds");
    fits(&board);
    let row = board
        .lines()
        .find(|line| line.contains("warden"))
        .unwrap_or_else(|| panic!("the listing's row: {board}"));
    let code = row
        .split_whitespace()
        .next()
        .expect("the row's code")
        .to_owned();
    assert!(
        row.contains("sealed") && row.contains("class I ") && row.contains("400cr"),
        "the author's mind, sealed, at the market's price: {row}"
    );
    let terms = types(&mut world, buyer, &format!("+minds {code}"));
    fits(&terms);
    assert!(
        terms.contains(&format!("Ask for a quote with +minds/buy {code}.")),
        "{terms}"
    );
    let _ = types(&mut world, buyer, &format!("+minds/buy {code}"));
    let quoted = types(&mut world, buyer, &format!("+minds {code}"));
    fits(&quoted);
    assert!(quoted.contains("Your quote: 400cr"), "{quoted}");
    let _ = types(&mut world, buyer, &format!("+minds/buy/pay {code}"));
    let _ = types(&mut world, buyer, &format!("+minds/buy/complete {code}"));
    assert_eq!(
        (wallet(&world, author), wallet(&world, buyer)),
        (
            wallets.0 - FEE_MINOR + AUTHORS_SHARE_MINOR,
            wallets.1 - PRICE_MINOR
        ),
        "the buyer paid the price and the author took their share"
    );
    let paid: Vec<Value> = world
        .rows("roll_mind_personal_paid_licence")
        .into_iter()
        .filter(|row| row["holder"] == json!(buyer.0))
        .collect();
    let [paid] = paid.as_slice() else {
        panic!("the buyer holds one paid licence: {paid:?}")
    };
    let licence = number(paid, "id");
    assert_ne!(licence, owner_licence, "her licence is not the author's");
    let holdings = types(&mut world, buyer, "+minds/holdings");
    fits(&holdings);
    assert!(
        holdings
            .lines()
            .any(|line| line.contains(&format!("licence {licence} "))
                && line.contains("paid")
                && line.contains("warden")
                && line.contains("sealed")
                && line.contains("Active")),
        "she holds the licence: {holdings}"
    );
    let gallery =
        crate::excursions::station_room(&world.rig.tw, "ceres_port", "room.c01_berth_gallery");
    crate::excursions::teleport(&mut world.rig.tw, buyer, gallery);
    let aboard = types(&mut world, buyer, "+ship/board");
    assert_eq!(
        tacsim::space::hull_aboard_for_actor(world.rig.tw.world(), buyer),
        Some(hull),
        "she is aboard again: {aboard}"
    );
    phase("bought");

    // 2. She seals it at her berth, and sails.
    assert!(
        claim_rows(&world, hull).is_empty(),
        "no claim names her yet"
    );
    let sealed = seal_aboard(&mut world, licence);
    eprintln!("MW21 sealed {sealed:?}");
    assert_eq!(
        (sealed.dispatcher, sealed.principal),
        (buyer, Some(buyer)),
        "the buyer sealed it and is its principal"
    );
    assert_eq!(
        (sealed.mind.as_str(), sealed.licence.as_str()),
        ("warden", licence.to_string().as_str()),
        "the author's mind, under the licence she bought"
    );
    assert_eq!(
        (sealed.core_class, sealed.governed),
        (tacsim::arena::core::CoreClass::II, true),
        "over her governed Class II core"
    );
    assert!(sealed.class <= sealed.core_class);
    assert_eq!(
        sealed.channels,
        vec![
            tacsim::arena::state::Channel::Helm,
            tacsim::arena::state::Channel::Sensors
        ],
        "helm and sensors, and no fire channel"
    );
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::Active {
            open: sealed.claim,
            dispatcher: buyer
        }
    );
    let opened_rows = claim_rows(&world, hull);
    assert!(
        matches!(opened_rows.as_slice(), [row] if row["closed"].is_null()),
        "the Roll's claim book holds one open claim for her: {opened_rows:?}"
    );
    assert_eq!(
        (
            &opened_rows[0]["open"]["operation"],
            &opened_rows[0]["open"]["dispatcher"],
            number(&opened_rows[0]["open"], "licence")
        ),
        (&json!(sealed.claim.to_hex()), &json!(buyer.0), licence),
        "the claim the seal reads, dispatched by the buyer under her licence"
    );
    let at_her_berth = books(&mut world);
    let boundary = sail_and_lose_the_link(&mut world.rig, buyer, hull);
    let under_way = books(&mut world);
    assert_eq!(
        under_way.accounts, at_her_berth.accounts,
        "her passage is paid in her stores, not in credits"
    );
    assert!(
        under_way.stores.0 < at_her_berth.stores.0 && under_way.stores.1 < at_her_berth.stores.1,
        "the passage she filed took its burn and its crew-days from her hold: {:?} to {:?}",
        at_her_berth.stores,
        under_way.stores
    );
    // 6 (the first of two). A restart on the corridor, the mind at her helm, before the
    // picket: the seal is the same, and the picket still opens on her.
    assert!(mind_has_her(&world.rig, hull));
    let mut world = world.reboot_reconnecting(&[GOD]);
    assert_eq!(
        ship_mind::aboard(world.rig.tw.world(), hull).as_ref(),
        Some(&sealed),
        "a restart on her passage keeps the seal: class, principal and claim"
    );
    assert!(
        mind_has_her(&world.rig, hull) && !world.rig.tw.world().is_connected(buyer),
        "and the mind still has her helm, her buyer away"
    );
    assert_eq!(claim_rows(&world, hull), opened_rows);
    arrive_at_the_picket(&mut world.rig, hull, boundary);
    phase("picket");

    // 3 and 4. In the bubble, her link down: the mind she bought has her.
    let mut watch = Watch::default();
    let took = watch.read(&world.rig, hull).expect("she is at the picket");
    assert_eq!(
        (took.helm, took.thinks, took.lock.as_deref(), took.thrust),
        (Helm::Mind, 0, None, [0, 0, 0]),
        "a mind has her and has done nothing yet: {took:?}"
    );
    assert!(
        took.drift > 10_000 && took.status == PicketInspectionStatus::AwaitingHeaveTo,
        "she comes in over the picket's ten metres a second, under its hail: {took:?}"
    );
    let ran = watch.step(&mut world.rig, hull);
    let braked = watch.step(&mut world.rig, hull);
    let brake = runs_then_brakes(&took, &ran, &braked);
    assert!(
        strength(brake) * 8 <= house_brake,
        "an eighth of the house default's brake, or less: {} against {house_brake}",
        strength(brake)
    );
    assert!(
        braked.status == PicketInspectionStatus::AwaitingHeaveTo
            && braked.step as usize >= house_hove_at,
        "where the house default had her hove to, she is still coming to: {braked:?}"
    );
    let at_restart = watch.until(&mut world.rig, hull, 4, |_| false);
    assert_eq!(
        (at_restart.thinks, at_restart.thrust, at_restart.helm),
        (6, brake, Helm::Mind),
        "six thinks in, the same burn stands: {at_restart:?}"
    );
    assert!(at_restart.think_in_flight, "{at_restart:?}");
    let per_step = at_restart.handed - watch.log[watch.log.len() - 2].handed;

    // 6. A restart while the mind has her helm.
    assert_eq!(
        ship_mind::aboard(world.rig.tw.world(), hull).as_ref(),
        Some(&sealed),
        "the seal is the claim's, unchanged since her berth"
    );
    let mut world = world.reboot_reconnecting(&[GOD]);
    assert_eq!(
        ship_mind::aboard(world.rig.tw.world(), hull).as_ref(),
        Some(&sealed),
        "the same class, principal and claim after the restart"
    );
    assert_eq!(
        claim_rows(&world, hull),
        opened_rows,
        "and the Roll's claim book holds the same one open claim"
    );
    let back = watch
        .read(&world.rig, hull)
        .expect("the bubble is read back");
    assert_eq!(
        (
            back.step,
            &back.lock,
            back.thrust,
            back.drift,
            back.status,
            back.helm,
            back.thinks,
            back.word(),
            back.propellant_kg
        ),
        (
            at_restart.step,
            &at_restart.lock,
            at_restart.thrust,
            at_restart.drift,
            at_restart.status,
            Helm::Mind,
            at_restart.thinks,
            at_restart.word(),
            at_restart.propellant_kg
        ),
        "the bubble reads back as it stood, the mind at her helm"
    );
    assert!(
        back.wrote.is_empty() && !back.think_in_flight,
        "nothing was in flight across the restart: {back:?}"
    );
    let resumed = watch.step(&mut world.rig, hull);
    assert_eq!(
        (resumed.thinks, resumed.word(), resumed.thrust),
        (7, "WARDEN_HEAVE_TO", brake),
        "its next think is the brake it remembered, not a first think: {resumed:?}"
    );
    assert!(
        resumed.wrote.is_empty() && resumed.drift < back.drift,
        "{resumed:?}"
    );
    phase("restart");

    // 7. She comes back with its next think on its way.
    let pending = watch.until(&mut world.rig, hull, 3, |_| false);
    assert_eq!(
        (pending.thinks, pending.thrust, pending.helm),
        (10, brake, Helm::Mind),
        "{pending:?}"
    );
    assert!(
        pending.think_in_flight && pending.locks_the_corvette(),
        "its eleventh think is on the host, a lock standing: {pending:?}"
    );
    assert_eq!(
        pending.handed - watch.log[watch.log.len() - 2].handed,
        per_step,
        "collected with the bubble's others, as every step's was"
    );
    let at_the_reconnect = books(&mut world);
    let events = world.rig.tw.connect(buyer);
    let heard = said(&world.rig.tw, &events, buyer);
    let restored = "[SPACE] Bridge link restored; the backup mind stands down; manual helm and \
                    tactical authority resumed.";
    assert!(heard.contains(restored), "{heard}");
    fits(restored);
    let handed_back = watch.read(&world.rig, hull).expect("she is at the picket");
    assert_eq!(
        (
            handed_back.helm,
            handed_back.thrust,
            handed_back.drift,
            handed_back.step
        ),
        (Helm::Crew, [0, 0, 0], pending.drift, pending.step),
        "the helm is hers and the mind's burn is cut: {handed_back:?}"
    );
    assert!(
        handed_back.wrote.is_empty() && handed_back.locks_the_corvette(),
        "what the mind had done stands: its lock: {handed_back:?}"
    );
    // Her first command.
    assert!(world.rows("tacsim.space_bubble_tape").is_empty());
    let cleared = types(&mut world, buyer, "+tac/lock off");
    assert!(
        cleared.lines().any(|line| line == "Target lock cleared."),
        "{cleared}"
    );
    let hers = watch.read(&world.rig, hull).expect("she is at the picket");
    assert_eq!(
        hers.wrote,
        vec![BubbleInputCommand::SetLockedTarget {
            ship_id: hers.ship.clone(),
            target_id: None,
        }],
        "her first command is what the ship does: {hers:?}"
    );
    assert_eq!(hers.lock, None);
    assert_eq!(
        world.rows("tacsim.space_bubble_tape").len(),
        1,
        "and the bubble's audit holds one input, hers"
    );
    // The pending think never lands: not on the next step, where it was due, nor later.
    let mut last_handed = hers.handed;
    for _ in 0..6 {
        let now = watch.step(&mut world.rig, hull);
        assert!(
            now.wrote.is_empty(),
            "nothing of the mind's is written: {now:?}"
        );
        assert_eq!(
            (now.helm, &now.lock, now.thrust, now.drift),
            (Helm::Crew, &None, [0, 0, 0], pending.drift),
            "no lock, no burn, her drift as it was: {now:?}"
        );
        assert!(
            now.handed - last_handed < per_step,
            "and no think of hers is collected while she has the helm: {now:?}"
        );
        last_handed = now.handed;
    }
    phase("reconnect");

    // She quits: the mind has her at once, and its think, let to land, locks her again.
    quit(&mut world.rig, buyer);
    let left = watch.read(&world.rig, hull).expect("she is at the picket");
    assert_eq!(
        (left.helm, left.thinks, &left.lock, left.thrust),
        (Helm::Mind, 0, &None, [0, 0, 0]),
        "a crew that quits hands her over at once: {left:?}"
    );
    let collected = watch.step(&mut world.rig, hull);
    assert_eq!(
        (collected.thinks, &collected.lock),
        (0, &None),
        "{collected:?}"
    );
    assert!(collected.wrote.is_empty() && collected.think_in_flight);
    // Her link blinks: she is back and gone again inside one step, with the new seat's first
    // think on the host. The seat she leaves the second time has the same think number as
    // the one that collected it, so only the strike at the hand-back keeps it from landing.
    let _ = world.rig.tw.connect(buyer);
    quit(&mut world.rig, buyer);
    let blinked = watch.read(&world.rig, hull).expect("she is at the picket");
    assert_eq!(
        (blinked.helm, blinked.thinks, &blinked.lock),
        (Helm::Mind, 0, &None),
        "{blinked:?}"
    );
    let struck = watch.step(&mut world.rig, hull);
    assert_eq!(
        (struck.thinks, &struck.lock, struck.thrust),
        (0, &None, [0, 0, 0]),
        "the think that was in flight when she came back is not the next seat's: {struck:?}"
    );
    assert!(struck.wrote.is_empty(), "{struck:?}");
    let ran_again = watch.step(&mut world.rig, hull);
    let braked_again = watch.step(&mut world.rig, hull);
    let second_brake = runs_then_brakes(&struck, &ran_again, &braked_again);
    assert!(
        ran_again.locks_the_corvette(),
        "the same mind in the same place locks her again: the struck think would have"
    );
    assert!(
        strength(second_brake) < strength(brake),
        "a new seat starts its own pace, from the drift she has left"
    );
    let hove = watch.until(&mut world.rig, hull, 60, |now| {
        now.status != PicketInspectionStatus::AwaitingHeaveTo
    });
    assert_eq!(
        hove.status,
        PicketInspectionStatus::HoveToReadyForInspection,
        "{hove:?}"
    );
    let stood = watch.step(&mut world.rig, hull);
    assert_eq!(
        (stood.word(), stood.burned(), stood.thrust),
        ("WARDEN_STAND_FOR_CUSTOMS", Some([0, 0, 0]), [0, 0, 0]),
        "hove to, it cuts her drive and stands for customs: {stood:?}"
    );
    let ruled = watch.until(&mut world.rig, hull, 6, |now| {
        now.status != PicketInspectionStatus::HoveToReadyForInspection
    });
    assert_eq!(ruled.status, PicketInspectionStatus::Cleared, "{ruled:?}");
    phase("cleared");

    // 8. She makes Stickney, nobody aboard connected.
    let stickney = crate::excursions::port_map(&world.rig.tw, "stickney_port")
        .host
        .map(ObjId)
        .expect("Stickney's host");
    assert!(
        to_a_berth(&mut world.rig, hull, &mut watch),
        "she makes port: {:?}",
        watch.log.last()
    );
    assert!(
        matches!(
            tacsim::port::ship::berth_of(world.rig.tw.world(), hull),
            tacsim::port::ship::Berth::Docked { map, .. } if map.host == Some(stickney.0)
        ),
        "at Stickney"
    );
    assert!(!world.rig.tw.world().is_connected(buyer));
    let docked = books(&mut world);
    phase("docked");

    // 4. The absent-player limits, from the bubble's own record, in order.
    for pair in watch.log.windows(2) {
        let (was, now) = (&pair[0], &pair[1]);
        assert_ne!(
            now.status,
            PicketInspectionStatus::RanThePicket,
            "she never runs the picket: {now:?}"
        );
        assert_eq!(now.emcon, EmconPosture::Standard, "{now:?}");
        for command in &now.wrote {
            // Doc 47 unit CO.2 (R-47-2): the house default's answer beside the bought mind.
            let answers = match command {
                BubbleInputCommand::FireOffensiveLasers { target_id, .. }
                | BubbleInputCommand::FireRailgun { target_id, .. }
                | BubbleInputCommand::FireTorpedoSalvo { target_id, .. } => {
                    *target_id != now.corvette && was.fired_on_by.contains(target_id)
                }
                _ => false,
            };
            let allowed = *command == now.locks_on()
                || answers
                || matches!(command, BubbleInputCommand::CommandBurnMmps2 { .. })
                || (now.helm == Helm::Crew
                    && matches!(
                        command,
                        BubbleInputCommand::SetLockedTarget {
                            target_id: None,
                            ..
                        }
                    ));
            assert!(
                allowed,
                "the tape holds a lock and burns for her, her own command, and a shot only \
                 at a ship that had fired on her; no work: {command:?} at {now:?}"
            );
        }
        if now.helm == Helm::Mind && was.helm == Helm::Mind {
            assert!(
                now.lock.is_none() || now.locks_the_corvette(),
                "her lock is the bought mind's, and an answer never moves it: {now:?}"
            );
        }
        if was.helm == Helm::Mind && now.helm == Helm::Mind {
            assert!(
                now.drift <= was.drift && now.propellant_kg <= was.propellant_kg,
                "a burn of the mind's only takes her drift off: {was:?} then {now:?}"
            );
        }
    }
    assert!(
        watch
            .log
            .iter()
            .filter(|now| now.helm == Helm::Mind)
            .count()
            >= 40
            && watch
                .log
                .iter()
                .filter(|now| now.helm == Helm::Mind && now.locks_the_corvette())
                .count()
                >= 30,
        "the mind had her, a lock standing, for most of the bubble"
    );

    // 5. Only her own stores, from her sailing to her berth.
    for (when, then) in [("at the reconnect", &at_the_reconnect), ("docked", &docked)] {
        assert_eq!(
            then.accounts, under_way.accounts,
            "no account in the world moved while a mind flew her ({when})"
        );
        let rows = moved(&under_way.typed, &then.typed);
        eprintln!("MW21 rows moved since she sailed ({when}): {rows:?}");
        assert!(
            rows.keys()
                .all(|family| family.starts_with("tacsim/") || family.starts_with("inventory/")),
            "no row of the market, the Roll or the economy moved ({when}): {rows:?}"
        );
        assert!(
            rows.get("inventory/inventory_pools").copied().unwrap_or(0) <= 1,
            "at most one inventory pool moved, her hold's reaction mass ({when}): {rows:?}"
        );
        assert!(
            then.stores.0 <= under_way.stores.0 && then.stores.1 == under_way.stores.1,
            "her hold: {:?} to {:?} ({when})",
            under_way.stores,
            then.stores
        );
    }
    assert!(
        docked.stores.0 < under_way.stores.0,
        "the mind's burns came out of her own tank: {:?} to {:?}",
        under_way.stores,
        docked.stores
    );
    assert_eq!(
        (wallet(&world, author), wallet(&world, buyer)),
        (
            wallets.0 - FEE_MINOR + AUTHORS_SHARE_MINOR,
            wallets.1 - PRICE_MINOR
        ),
        "the two wallets read as they did when the licence was bought"
    );
    assert_eq!(
        world
            .rows("roll_mind_personal_paid_licence")
            .into_iter()
            .filter(|row| row["holder"] == json!(buyer.0))
            .collect::<Vec<_>>(),
        vec![paid.clone()],
        "and she holds the one licence, as it was"
    );

    // 8. The claim closes once.
    assert_eq!(
        claim_rows(&world, hull),
        opened_rows,
        "the claim is still open at her berth, until its Close lands"
    );
    let mut closed = None;
    for _ in 0..24 {
        fires(&mut world, 1);
        closed = tacsim::ic_claim_closed(world.rig.tw.world(), sealed.claim);
        if closed.is_some() {
            break;
        }
    }
    let closed = closed.unwrap_or_else(|| {
        panic!(
            "the claim's Close lands: {:?}",
            cursor(&mut world.rig, hull)
        )
    });
    let closed_rows = claim_rows(&world, hull);
    let [row] = closed_rows.as_slice() else {
        panic!("the Roll's claim book holds one claim for her: {closed_rows:?}")
    };
    assert_eq!(
        (&row["open"], &row["closed"]["operation"]),
        (&opened_rows[0]["open"], &json!(closed.to_hex())),
        "the claim she opened at Ceres, closed by the one Close: {row}"
    );
    assert_eq!(
        row["closed"]["kind"],
        json!({"kind": "returned_to_base", "base": stickney.0}),
        "closed by her docking at Stickney: {row}"
    );
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::None,
        "no claim stands on her"
    );
    assert_eq!(ship_mind::aboard(world.rig.tw.world(), hull), None);
    // Once: more fires and a restart close nothing more and open nothing.
    fires(&mut world, 6);
    let mut world = world.reboot_reconnecting(&[GOD]);
    fires(&mut world, 6);
    assert_eq!(
        claim_rows(&world, hull),
        closed_rows,
        "the same one row, closed once, after six fires, a restart and six more"
    );
    assert_eq!(
        tacsim::ic_claim_closed(world.rig.tw.world(), sealed.claim),
        Some(closed)
    );
    assert_eq!(
        tacsim::ic_claim_of(world.rig.tw.world(), hull),
        tacsim::IcClaimRead::None
    );
    assert!(
        matches!(
            cursor(&mut world.rig, hull),
            Some(ClaimCursor::Closing { open, .. }) if open == sealed.claim
        ),
        "the cursor rests on the closed claim"
    );
    // The buyer, back aboard at Stickney, reads no mind sealed: a seal lasts one passage.
    let _ = world.rig.tw.connect(buyer);
    let none = run(&mut world.rig.tw, buyer, "+ship/mind");
    assert_eq!(none, ship_mind::MIND_NONE.replace("{name}", "Salt Moth"));
    fits(&none);
    fires(&mut world, 2);
    assert_eq!(claim_rows(&world, hull), closed_rows);
    crate::excursions::no_refused_writes(&world.rig.tw);
    phase("closed");
}
