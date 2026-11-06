//! **Doc 41 story (a), whole, with the bot over SSH** (§4.9; cut RS.9d), at 256 (Ahuna Flank) and
//! 512 (Urvara Reach): the recovery run, extended — the party's tender works on its escort on the
//! ground, and the Dray-P's bay works on her on the site and under way.
//!
//! The bot kits up as an escort pilot (the Co-op's enrolment, the licence, Occator's escort lease)
//! and leases Occator's tender beside it. Ceres sells neither structural alloys nor yard fittings,
//! so it sails for Stickney Yards by Luna and Deimos (round Kirkwood's customs picket, as IN.5's
//! story does), buys the tender's stores and the bay's goods there, sails home, and lifts for the
//! site. On the pad: the stores loaded and the bale opened (`+tv/2 stores …`), both rigs down the
//! ramp, and — the escort's cannon jammed, its hull 40 points down and its magazine at 10 — the
//! tender clears the jam, patches the hull and rearms the magazine (`+tv/2 unjam|stabilize|rearm`).
//! The escort back in the bay, her cannon damaged, her machine gun destroyed and her hull holed
//! again: `+bay` reads her
//! away from port with no fee and names the nearest workshop for the destroyed part; `+bay/order`
//! refuses the replace and takes the mend and the armour at twice the workshop's time from the
//! hold's goods; the carrier lifts for Ceres (the site's one route) and the order finishes under
//! way.
//!
//! **What is a fixture** (doc 36a §8's damage and funding seams): God's `@unit/damage` on the
//! escort (the jam, the hull, the magazine, then the machine gun and the armour), the bot's working
//! float (`@econ/mint`), and the world's rate. Every act of the player's is typed by the bot.
//!
//! **What this story does not run** (in-process instead): GC.4's tow before it (`field_gc4`, and
//! the bot's own GC.4 lines in `fr7_round`), the bay's wait under a field (`field_rs2`), and the
//! tender's laden-tow refusal (`field_rs1`).

use std::time::{Duration, Instant};

use mush_bot::mine::lane::{escort, field, salvage};
use mush_bot::mine::{Lane, Settings, hull_name, lane, lines};
use mush_core::ObjId;

use crate::{arena, mine};

/// A line the player types, in the bot's vocabulary, answered by one of `needles`.
async fn typed(
    client: &mut arena::Client,
    line: &str,
    needles: &[&str],
) -> (Option<usize>, String) {
    assert!(
        salvage::in_vocabulary(line)
            || field::in_vocabulary(line)
            || escort::in_vocabulary(line)
            || lane::in_vocabulary(line)
            || mush_bot::mine::lane::works::in_vocabulary(line),
        "{line}"
    );
    client.answer(line, needles).await
}

/// `line` typed until its answer holds `needles[0]`, every `every` real seconds, inside `bound`.
async fn until(
    client: &mut arena::Client,
    line: &str,
    needles: &[&str],
    every: u64,
    bound: Duration,
) -> String {
    let deadline = Instant::now() + bound;
    loop {
        let (index, text) = typed(client, line, needles).await;
        if index == Some(0) {
            return text;
        }
        assert!(
            Instant::now() < deadline,
            "{line} never answered {:?}:\n{text}",
            needles[0]
        );
        client.idle(Duration::from_secs(every)).await;
    }
}

/// The digits after `after` in `text`.
fn id_after(text: &str, after: &str) -> u64 {
    text.split(after)
        .nth(1)
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u64>().ok()
        })
        .unwrap_or_else(|| panic!("no number after {after:?}:\n{text}"))
}

/// **Board, file the passage to `port` and wait for her to dock there; then ashore** (the cohort's
/// `sail`, patient): a market's hand-overs settle on its sweep, and at Deimos the bunkers have
/// been seen to take minutes to come aboard, so the undock is typed again every ten seconds for up
/// to ten minutes while it answers that the goods are still on their way.
async fn sail(client: &mut arena::Client, hull: &str, port: &str) {
    typed(client, "+ship/board", &[lines::BOARDED]).await;
    let from = client.seen.len();
    let line = format!("+ship/undock {port}");
    let deadline = Instant::now() + Duration::from_secs(10 * 60);
    loop {
        let (index, said) = typed(
            client,
            &line,
            &[lines::CLEARANCE, lines::GOODS_WAIT, lines::GOODS_LEAVING],
        )
        .await;
        if index == Some(0) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{line}: the goods never settled:
{said}"
        );
        if index.is_some() {
            println!("rs9a: {line}: {}", said.lines().last().unwrap_or("").trim());
        }
        client.idle(Duration::from_secs(10)).await;
    }
    assert!(
        client
            .watch(from, &lines::docks(hull), Duration::from_secs(20 * 60))
            .await,
        "{hull} did not dock at {port}:
{}",
        mush_bot::tail(&client.seen, 4 * 1024)
    );
    typed(client, "+ship/leave", &[lines::ASHORE]).await;
}

/// **Bunker for the passage to `port`, buying exactly what it lacks** (the cohort's `bunker` buys a
/// margin with no regard to the tank's room, and a purchase past the room leaves its remainder a
/// pending draw that holds every later departure: "Quote #…'s goods have not come aboard yet").
async fn bunker_lean(client: &mut arena::Client, port: &str) {
    for _ in 0..4 {
        let from = client.seen.len();
        typed(client, &format!("+ship/plan {port}"), &["Stores "]).await;
        client.idle(Duration::from_secs(1)).await;
        let text = client.seen.get(from..).unwrap_or("").to_owned();
        let raw: Vec<String> = text.lines().map(str::to_owned).collect();
        let plan = lines::plan_figures(&raw).expect("the plan's figures");
        println!(
            "rs9a: the plan for {port}: tank {} t for a burn of {} t, locker {} for {} crew-days",
            plan.tank_t, plan.burn_t, plan.locker, plan.crew_days
        );
        if plan.tank_t >= plan.burn_t && plan.locker >= plan.crew_days {
            return;
        }
        let rows = crate::board_rows(client).await;
        if plan.tank_t < plan.burn_t {
            let fuel = lines::ask_for(&rows, "reaction mass").expect("a reaction mass ask");
            crate::buy(client, fuel, plan.burn_t - plan.tank_t + 1).await;
        }
        if plan.locker < plan.crew_days {
            let food = lines::ask_for(&rows, "provisions").expect("a provisions ask");
            crate::buy(client, food, plan.crew_days - plan.locker + 1).await;
        }
        client.idle(Duration::from_secs(5)).await;
    }
    panic!("the bunkers for {port} never covered the plan");
}

/// A mutation plan's JSON from `ops`.
fn plan(ops: Vec<mush_tactics::UnitMutationOp>, cause: &str) -> String {
    let plan = mush_tactics::UnitMutationPlan::new(ops, None, Some(cause.to_owned().into()))
        .expect("a plan");
    serde_json::to_string(&plan).expect("the plan's JSON")
}

/// A component's condition, as an op.
fn component(name: &str, condition: &str) -> mush_tactics::UnitMutationOp {
    mush_tactics::UnitMutationOp::SetComponentCondition {
        component: mush_tactics::unit::ComponentId::new(name).expect("an id"),
        condition: mush_tactics::ConditionId::new(condition).expect("a condition"),
    }
}

/// A cell set, as an op.
fn cell(addr: mush_tactics::CellAddr, value: i64) -> mush_tactics::UnitMutationOp {
    mush_tactics::UnitMutationOp::SetCell { addr, value }
}

/// **The first damage**: the escort's cannon jammed, its hull's internals 40 down and damaged,
/// its cannon's magazine at 10 (`field_rs1`'s fixture).
fn first_damage(engine_unit: &mush_tactics::TacticalUnit) -> String {
    let internals = mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("hull").expect("an id"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL).expect("an id"),
    };
    let full = engine_unit.cell(&internals).map_or(100, |c| c.max);
    let ammo = mush_tactics::CellAddr::Component {
        component: mush_tactics::unit::ComponentId::new("cannon").expect("an id"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::AMMO_CELL).expect("an id"),
    };
    plan(
        vec![
            component("cannon", "jammed"),
            mush_tactics::UnitMutationOp::SetSectionCondition {
                section: mush_tactics::SectionId::new("hull").expect("an id"),
                condition: mush_tactics::ConditionId::new("damaged").expect("a condition"),
            },
            cell(internals, full - 40),
            cell(ammo, 10),
        ],
        "bot-story:rs9a-first",
    )
}

/// **The second damage**, for the bay (`field_rs2`'s fixture): the cannon damaged (a mend), the
/// machine gun destroyed (a replace, which only a workshop does) and the hull's section damaged
/// 40 points down (the armour).
fn second_damage(unit: &mush_tactics::TacticalUnit) -> String {
    let internals = mush_tactics::CellAddr::Section {
        section: mush_tactics::SectionId::new("hull").expect("an id"),
        cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL).expect("an id"),
    };
    let full = unit.cell(&internals).map_or(100, |c| c.max);
    plan(
        vec![
            component("cannon", "damaged"),
            component("machine_gun", "destroyed"),
            mush_tactics::UnitMutationOp::SetSectionCondition {
                section: mush_tactics::SectionId::new("hull").expect("an id"),
                condition: mush_tactics::ConditionId::new("damaged").expect("a condition"),
            },
            cell(internals, full - 40),
        ],
        "bot-story:rs9a-second",
    )
}

/// **A fresh escort's cells**, from its spec on the booted engine (the damage plans' maxima).
fn escort_unit(engine: &mush_core::Engine) -> mush_tactics::TacticalUnit {
    let spec = engine
        .world
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer")
        .definitions()
        .bindings
        .get(&crate::fr7_round::cid("tacsim.unit.tv_escort"))
        .expect("the escort is bound")
        .spec
        .clone();
    mush_tactics::TacticalUnit::from_spec(&spec, ObjId(1)).expect("a fresh escort")
}

/// **Story (a) on `ground`**, the bot `miner`.
#[allow(clippy::too_many_lines)]
pub(crate) async fn story_a(miner: mine::Miner, ground: &'static crate::grounds_rig::Ground) {
    let started = Instant::now();
    let site = ground.entry;
    let stamp = |what: &str| println!("rs9a {site}: +{} s {what}", started.elapsed().as_secs());
    let mut plans = None;
    let (rig, _) = mine::boot_with(
        &[miner],
        |_| {},
        |engine| {
            let unit = escort_unit(engine);
            plans = Some((first_damage(&unit), second_damage(&unit)));
            Vec::new()
        },
    )
    .await;
    let (first, second) = plans.expect("the damage plans");
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut client =
        arena::Client::login(&rig, miner.account, arena::PILOT_PASSWORD, miner.character).await;
    let hull = hull_name(&format!("{}+{}", miner.account, miner.character)).expect("a hull name");

    // 1. The kit-up as an escort pilot, at the boot's rate (the Co-op's desk reads the rate), to
    //    the escort's lease; then the tender beside it at the same counter.
    let mut lane = Lane::new(
        miner.site,
        Settings {
            hull: hull.clone(),
            epoch_unix_ms: mine::unix_now_ms(),
        },
        miner.character.to_owned(),
    )
    .with_escort(None);
    let leased = |lane: &Lane| lane.step_line() == Some("+tv/1/permissions go");
    mine::drive_lane_since(
        &mut client,
        &mut lane,
        started,
        leased,
        Duration::from_secs(30 * 60),
    )
    .await;
    assert!(
        leased(&lane),
        "the escort lease: {:?}\n{}",
        lane.result(),
        mush_bot::tail(&client.seen, 4 * 1024)
    );
    stamp("the escort is leased");
    let (index, said) = typed(
        &mut client,
        "+tv/1/permissions go",
        &["loan", "Loan", "permission"],
    )
    .await;
    println!(
        "rs9a: +tv/1/permissions go → {index:?}\n{}",
        said.trim_end()
    );
    // The working float first: the tender's term and deposit are 8,800 cr.
    crate::float(&mut god, &mut client).await;
    let (index, said) = typed(
        &mut client,
        "+tv/lease tender go",
        &["Occator Water & Propellant leases you "],
    )
    .await;
    assert_eq!(index, Some(0), "the tender's lease:\n{said}");
    stamp("the tender is leased");
    mine::lift_rate(&mut god, 3_600).await;

    // 2. The goods at Stickney, by Luna and Deimos; home to Ceres.
    for (word, room) in [
        ("concourse", lines::ROOM_CONCOURSE),
        ("arrivals", lines::ROOM_ARRIVALS),
        ("lobby", lines::ROOM_LOBBY),
        ("gallery", lines::ROOM_GALLERY),
    ] {
        typed(&mut client, word, &[room]).await;
    }
    bunker_lean(&mut client, "luna").await;
    sail(&mut client, &hull, "luna").await;
    bunker_lean(&mut client, "deimos").await;
    sail(&mut client, &hull, "deimos").await;
    bunker_lean(&mut client, "stickney").await;
    sail(&mut client, &hull, "stickney").await;
    stamp("docked at Stickney Yards");
    let rows = crate::board_rows(&mut client).await;
    for (label, qty) in [
        ("structural alloy stock", 3),
        ("yard fittings, crated", 3),
        ("105 mm HE rounds", 30),
    ] {
        let n = lines::ask_for(&rows, label).unwrap_or_else(|| panic!("a {label} ask: {rows:?}"));
        crate::buy(&mut client, n, qty).await;
    }
    stamp("bought the stores and the bay's goods");
    bunker_lean(&mut client, "ceres").await;
    sail(&mut client, &hull, "ceres").await;
    stamp("home at Ceres");

    // 3. To the site.
    bunker_lean(&mut client, ground.venue).await;
    typed(&mut client, "+ship/board", &[lines::BOARDED]).await;
    let from = client.seen.len();
    let (_, said) = crate::typed(
        &mut client,
        &format!("+ship/lift {}", ground.venue),
        &[lines::CLEARANCE],
        &[lines::GOODS_WAIT, lines::GOODS_LEAVING],
    )
    .await;
    println!("rs9a: +ship/lift {} → {}", ground.venue, said.trim_end());
    assert!(
        client
            .watch(from, &lines::settles(&hull), Duration::from_secs(20 * 60))
            .await,
        "{hull} never settled on the site:\n{}",
        mush_bot::tail(&client.seen, 4 * 1024)
    );
    stamp("on the site's pad");

    // 4. The stores from the hold, the bale opened before launch.
    let (index, said) = typed(&mut client, "+tv/2 stores open", &["no whole tonne"]).await;
    assert_eq!(index, Some(0), "nothing to open yet:\n{said}");
    for line in [
        "+tv/2 stores load 1 structural_alloys go",
        "+tv/2 stores load 1 industrial_parts go",
        "+tv/2 stores open go",
        "+tv/2 stores load 20 ammo_he_105 go",
    ] {
        let (_, said) = typed(&mut client, line, &["stores", "Stores", "bale"]).await;
        println!("rs9a: {line} → {}", said.trim_end());
        client.idle(Duration::from_secs(3)).await;
    }
    let shown = until(
        &mut client,
        "+tv/2 stores",
        &["20 ammo_he_105", "hold nothing"],
        5,
        Duration::from_secs(3 * 60),
    )
    .await;
    assert!(shown.contains("1000 kg of structural alloys"), "{shown}");
    stamp("the tender's stores are loaded");

    // 5. Both rigs down the ramp; the escort's first damage.
    for n in [1, 2] {
        let (_, said) = typed(
            &mut client,
            &format!("+tv/{n} launch go"),
            &["launch", "ramp"],
        )
        .await;
        println!("rs9a: +tv/{n} launch go → {}", said.trim_end());
        until(
            &mut client,
            &format!("+tv/{n}"),
            &[escort::AT_RAMP, "on the ground"],
            5,
            Duration::from_secs(3 * 60),
        )
        .await;
    }
    let (_, rig1) = typed(&mut client, "+tv/1", &["Rig 1: "]).await;
    let mark = rig1
        .split("Rig 1: ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .expect("rig 1's mark")
        .trim()
        .to_owned();
    let (_, found) = god
        .answer(&format!("@find {mark}"), &["(#", "Nothing"])
        .await;
    let escort_id = ObjId(u32::try_from(id_after(&found, "(#")).expect("an id"));
    let e = format!("#{}", escort_id.0);
    god.say(&format!("@unit/damage {e}={first}"), "Applied ")
        .await;
    stamp(&format!(
        "the escort {mark} ({e}) is jammed, holed and short"
    ));

    // 6. The tender's three works.
    for (line, needle) in [
        (format!("+tv/2 unjam {e}"), "clear the jam"),
        (
            format!("+tv/2 stabilize {e}=hull"),
            "patch 40 points of hull",
        ),
        (format!("+tv/2 rearm {e}"), "rearm"),
    ] {
        let (index, said) = typed(&mut client, &line, &[needle]).await;
        assert_eq!(index, Some(0), "{line}:\n{said}");
        let go = format!("{line} go");
        let from = client.seen.len();
        let (index, said) = typed(&mut client, &go, &["sets to work on the"]).await;
        assert_eq!(
            index,
            Some(0),
            "{go}:
{said}"
        );
        // Done when the tender says so (`surface::tender::TENDED`).
        assert!(
            client
                .watch(
                    from,
                    "Rig 2 has finished on the ",
                    Duration::from_secs(5 * 60)
                )
                .await,
            "{go} never finished:
{}",
            mush_bot::tail(&client.seen, 4 * 1024)
        );
    }
    stamp("the tender cleared, patched and rearmed the escort");

    // 7. The escort back in the bay; the second damage; the bay away from port.
    let (_, said) = typed(&mut client, "+tv/1 recover go", &["ramp", "recover"]).await;
    println!("rs9a: +tv/1 recover go → {}", said.trim_end());
    until(
        &mut client,
        "+tv/1",
        &[escort::IN_BAY],
        5,
        Duration::from_secs(5 * 60),
    )
    .await;
    god.say(&format!("@unit/damage {e}={second}"), "Applied ")
        .await;
    let (index, said) = typed(
        &mut client,
        &format!("+bay {e}"),
        &["stands in a carrier bay away from port"],
    )
    .await;
    assert_eq!(index, Some(0), "the bay away from port:\n{said}");
    assert!(said.contains("no fee"), "{said}");
    assert!(
        said.contains("machine_gun is destroyed: a workshop must replace it"),
        "{said}"
    );
    let (index, said) = typed(
        &mut client,
        &format!("+bay/order {e}=replace"),
        &["a workshop must replace it"],
    )
    .await;
    assert_eq!(index, Some(0), "the replace is refused:\n{said}");
    let (index, said) = typed(
        &mut client,
        &format!("+bay/order {e}=mend,armor"),
        &["taken in the carrier bay"],
    )
    .await;
    assert_eq!(index, Some(0), "the mend and the armour:\n{said}");
    assert!(said.contains("No fee away from port"), "{said}");
    stamp("the bay's order is taken on the site");

    // 8. The lift for Ceres, the order finished under way.
    let from = client.seen.len();
    let (_, said) = crate::typed(
        &mut client,
        "+ship/lift ceres",
        &[lines::CLEARANCE],
        &[lines::GOODS_WAIT, lines::GOODS_LEAVING],
    )
    .await;
    println!("rs9a: +ship/lift ceres → {}", said.trim_end());
    let done = until(
        &mut client,
        &format!("+bay {e}"),
        &["Condition: operational"],
        10,
        Duration::from_secs(20 * 60),
    )
    .await;
    println!("rs9a: under way: {}", done.trim_end());
    // The mend took the cannon back to intact and the armour the hull's 40 points.
    let cannon = done
        .lines()
        .find(|line| line.trim_start().starts_with("cannon "))
        .unwrap_or_default();
    assert!(
        cannon.contains("intact"),
        "the cannon is mended: {cannon}
{done}"
    );
    let hull_line = done
        .lines()
        .find(|line| line.trim_start().starts_with("hull "))
        .unwrap_or_default();
    let full = hull_line
        .split_whitespace()
        .last()
        .and_then(|cells| cells.split_once('/'))
        .is_some_and(|(have, max)| have == max);
    assert!(
        full,
        "the hull is whole again: {hull_line}
{done}"
    );
    assert!(
        !client
            .seen
            .get(from..)
            .unwrap_or("")
            .contains(&lines::docks(&hull)),
        "the order finished under way, before she docked"
    );
    stamp("the bay's order finished under way");
    client.close().await;
    god.close().await;
    drop(rig);
}
