//! **Doc 41 story (b), whole, with the bot over SSH** (§4.9; cut RS.9c), at 256 (Ahuna Flank) and
//! 512 (Urvara Reach): the contested salvage, three players each typing over their own session.
//!
//! - **T**, Brask Freight's officer (an `@org/grant` of Brask's), flies a Co-op Dray-P to the site
//!   and sits on the pad. Brask's hull `a` and T's own hull `c` lie beside the ramp, on two of L's
//!   blocks.
//! - **O**, the opportunist, flies a Dray-P there too, launches a prospector and tows both hulls
//!   into O's bay: `a` off L's ground with nobody's leave, `c` under T's `+salvage/authorize` and
//!   L's block delegation. O files a claim on each; every holder is told.
//! - **L**, the landholder, holds the Registry's claims on both blocks and delegates `c`'s.
//!
//! The contests, each typed by the one who answers: on `a` T's `procedure` finds no defect and
//! O may not contest at all; T's `right` finds nothing that bound O; T's `recovery`, on T's own
//! sensor record, vacates the claim. On `c` L's `right` (an authorization of L's own) is no
//! ground, and T's `right` vacates (T's authorization bound O to deliver her). O's bay quote of
//! `c` while the claim stands reads "at your risk". O sails home leaving the tug; at docking the
//! Registry fines O for `a`'s unlawful recovery, and the port berths both vacated hulls "held for"
//! T, who has no carrier docked. Past the fine's 28 IC days of grace O's carrier is held at
//! Ceres until O pays it (`+debts/pay`). T sails home and `+bay/stow`s both hulls out of the
//! workshop into T's own carrier.
//!
//! **What is a fixture** (doc 36a §8's seams): God's: the organization (`@org/create`,
//! `@org/grant`), the two hulls placed beside the ramp with their power destroyed (`@unit/damage`)
//! and titled (`@asset/title`), on the site's index as casualties (`@tacsim/site leave`), L's two
//! block claims seeded as typed records, the players' money (`@econ/mint`), O's account emptied
//! before docking (`@econ/pay`, so the fine stands as a debt), T's walk to the workshop
//! (`@tel`), and the world's rate. Every act of the players' is typed by the bot.
//!
//! **What this story does not run** (in-process instead): the third salvager's lawful claim
//! awarded across a W-clock freeze (`salvage_close`, `field_gc6`), the at-risk repair itself
//! (`bay_36a`), and the bay-to-bay `+bay/stow` with T's carrier docked first (`vacated_return_rs4`).

use std::time::{Duration, Instant};

use mush_bot::mine::lane::{field, salvage};
use mush_bot::mine::{Lane, Milestone, Settings, hull_name, lane};
use mush_core::ObjId;

use crate::fr7_round::{PROSPECTOR, WEST, cid, placed, powerless_plan, staff_hand};
use crate::grounds_rig::Ground;
use crate::{arena, mine};

/// The whole story's real-time bound.
const STORY_BOUND: Duration = Duration::from_secs(100 * 60);

/// The cast: T, O and L.
#[derive(Clone, Copy)]
pub(crate) struct Cast {
    pub(crate) t: mine::Miner,
    pub(crate) o: mine::Miner,
    pub(crate) l: mine::Miner,
}

/// What the fixture made: the two hulls, their cells and block letters, and the three players.
#[derive(Clone, Copy, Debug)]
struct Made {
    a: ObjId,
    c: ObjId,
    t: ObjId,
    c_letter: char,
    /// `a`'s and `c`'s cells, for God's index writes once the world runs (one staged
    /// authoritative action an actor at a time: two in `prepare` lose the second).
    cells: [(i64, i64); 2],
    /// Ceres's workshop room, C14 (where the port berths a held hull).
    workshop: ObjId,
}

/// The block letter of `cell` on a site of `side`.
fn letter_of(side: i64, cell: (i64, i64)) -> char {
    tacsim::field_ops::block_letter_at_site_pose(
        mush_tactics::field_ops::FieldPose {
            site: ObjId(1),
            x_cm: cell.0 * 1_000 + 500,
            y_cm: cell.1 * 1_000 + 500,
        },
        tacsim::surface::address::Scheme::of_site(u16::try_from(side).unwrap_or(256)),
    )
    .expect("a block")
}

/// The object called `name`.
pub(crate) fn named(engine: &mush_core::Engine, name: &str) -> ObjId {
    engine
        .world
        .iter()
        .find(|row| row.name == name)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("no object called {name}"))
}

/// **L's claim on `letter`**, seeded as a typed record (GC.6's fixture, `salvage_close`'s
/// `seed_block`).
fn seed_block(engine: &mut mush_core::Engine, entry: &str, letter: char, l: ObjId) {
    let w = &engine.world;
    let place = w
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer")
        .port_maps()
        .find(|map| map.entry.as_str() == entry)
        .and_then(|map| map.site.as_ref())
        .and_then(|site| site.blocks.get(letter.to_string().as_str()).copied())
        .map(ObjId)
        .expect("the block's place");
    let now = w.world_ic_now().at;
    let mut tenure = tacsim::surface::claims::BlockTenure::new(cid(entry), letter, place);
    tenure.claim = Some(tacsim::surface::claims::BlockClaim {
        holder: mush_org::PrincipalId::Object(l),
        account: w
            .module_state::<mush_sim::AccountOracle>()
            .and_then(|oracle| oracle.account_of(l)),
        epoch: 1,
        opened_ic: None,
        registered_ic: now,
        term_start_ic: now,
        term_end_ic: now + 365 * 86_400,
        worked_at_term_start: 0,
        discovered_ic: None,
        declared: None,
        fee: None,
        refund: None,
        lapsed_ic: None,
    });
    let handles = w
        .module_state::<tacsim::port::PortHandles>()
        .cloned()
        .expect("the port's handles");
    let prepared = {
        let mut txn = engine
            .action_transaction(&handles.families.claims)
            .expect("a transaction");
        txn.insert(
            &handles.families.claims,
            &tacsim::surface::claims::BlockKey {
                site: cid(entry),
                letter,
            },
            tenure,
        )
        .expect("the claim writes");
        txn.prepare().expect("prepares")
    };
    let (_ops, install) = prepared.into_parts();
    engine
        .install_prepared_action(install)
        .expect("the claim installs");
}

/// **The fixture**, on the booted engine before the world task spawns.
fn fixture(engine: &mut mush_core::Engine, ground: &Ground, cast: Cast) -> (Vec<String>, Made) {
    let (t, o, l) = (
        named(engine, cast.t.character),
        named(engine, cast.o.character),
        named(engine, cast.l.character),
    );
    let mut said = vec![
        mine::staff_line(engine, "@org/create Brask"),
        mine::staff_line(
            engine,
            &format!("@org/grant #{}=Brask economy.account.post", t.0),
        ),
    ];
    // Beside the ramp's cell, where O's rig comes down: `a` a cell east, `c` a cell west — two of
    // L's blocks, since every site's ramp stands on a block line.
    let (x, y) = tacsim::surface::excursion::site_pad_cm(&engine.world, &cid(ground.entry));
    let side = i64::from(tacsim::surface::ground::SITE_CELL_CM);
    let ramp = (x.div_euclid(side), y.div_euclid(side) - 4);
    let (a_cell, c_cell) = ((ramp.0 + 1, ramp.1), (ramp.0 - 1, ramp.1));
    let (a_letter, c_letter) = (
        letter_of(ground.side, a_cell),
        letter_of(ground.side, c_cell),
    );
    assert_ne!(a_letter, c_letter, "`a` and `c` lie on two blocks");
    let a = placed(engine, ground, PROSPECTOR, "Brask Rig", a_cell, WEST);
    let c = placed(engine, ground, PROSPECTOR, "Pilot Rig", c_cell, WEST);
    for (hull, title) in [(a, "org:Brask".to_owned()), (c, format!("#{}", t.0))] {
        said.push(mine::staff_line(
            engine,
            &format!("@unit/damage #{}={}", hull.0, powerless_plan()),
        ));
        said.push(mine::staff_line(
            engine,
            &format!("@asset/title #{}={title}", hull.0),
        ));
    }
    if std::env::var("RS9B_NO_BLOCKS").is_err() {
        seed_block(engine, ground.entry, a_letter, l);
        seed_block(engine, ground.entry, c_letter, l);
    }
    said.push(format!(
        "a #{} at {a_cell:?} on {a_letter}, c #{} at {c_cell:?} on {c_letter}",
        a.0, c.0
    ));
    // The titles took: `a` is Brask's, `c` T's.
    let title = |engine: &mut mush_core::Engine, hull: ObjId| {
        staff_hand(engine, |ctx| {
            ctx.w
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|e| e.snapshot().asset(hull).map(|a| a.title_owner.clone()))
        })
    };
    assert!(
        matches!(
            title(engine, a),
            Some(mush_org::PrincipalId::Organization(_))
        ),
        "Brask holds `a`: {said:?}"
    );
    assert_eq!(
        title(engine, c),
        Some(mush_org::PrincipalId::Object(t)),
        "T holds `c`: {said:?}"
    );
    let _ = o;
    // Ceres's workshop room (C14), as the bay resolves it: the lane pack's port def.
    let lane = tacsim::pack::load_pack(tacsim::definitions::content::LANE_JSON).expect("the lane");
    let workshop = match lane.entry(&cid("ceres_port")) {
        Some(tacsim::pack::PackEntry::Port(def)) => {
            tacsim::port::workshop::room(&engine.world, &cid("ceres_port"), def)
                .ok()
                .flatten()
        }
        _ => None,
    }
    .expect("Ceres's C14 workshop room");
    (
        said,
        Made {
            a,
            c,
            t,
            c_letter,
            cells: [a_cell, c_cell],
            workshop,
        },
    )
}

/// A line the player types, in the bot's vocabulary, answered by one of `needles`.
pub(crate) async fn typed(
    client: &mut arena::Client,
    line: &str,
    needles: &[&str],
) -> (Option<usize>, String) {
    assert!(
        salvage::in_vocabulary(line)
            || field::in_vocabulary(line)
            || lane::in_vocabulary(line)
            || mush_bot::mine::lane::works::in_vocabulary(line),
        "{line}"
    );
    client.answer(line, needles).await
}

/// `line` typed until its answer holds `needle` (index 0 of `needles`), every `every` real
/// seconds, inside `bound`; the answer.
pub(crate) async fn until(
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

/// The lanes' real-time bound (`RS9_LANE_BOUND_S` overrides it for a short local probe).
fn lane_bound() -> Duration {
    Duration::from_secs(
        std::env::var("RS9_LANE_BOUND_S")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(40 * 60),
    )
}

/// **A player's lane to its enrolment**, at the boot's rate: the Co-op's desk is open only while
/// the readiness report reads Ready, whose rate line wants the boot's 24, so every player enrols
/// before God lifts the rate.
pub(crate) async fn enrol(
    client: &mut arena::Client,
    miner: mine::Miner,
    started: Instant,
) -> Lane {
    let hull = hull_name(&format!("{}+{}", miner.account, miner.character)).expect("a hull name");
    let mut lane = Lane::new(
        miner.site,
        Settings {
            hull,
            epoch_unix_ms: mine::unix_now_ms(),
        },
        miner.character.to_owned(),
    );
    let enrolled = |lane: &Lane| lane.step_line() == Some("+coop/refit prospector");
    mine::drive_lane_since(client, &mut lane, started, enrolled, lane_bound()).await;
    assert!(
        enrolled(&lane),
        "{} never enrolled: {:?}; the client's last 4 KiB:
{}",
        miner.character,
        lane.result(),
        mush_bot::tail(&client.seen, 4 * 1024)
    );
    lane
}

/// **A player's lane on from its enrolment** (kit-up and the hop to the site) until `until`.
pub(crate) async fn fly(
    client: &mut arena::Client,
    lane: &mut Lane,
    started: Instant,
    until: impl Fn(&Lane) -> bool,
) {
    mine::drive_lane_since(client, lane, started, &until, lane_bound()).await;
    assert!(
        until(lane),
        "{}'s lane stopped at {:?}: {:?}; the client's last 4 KiB:
{}",
        lane.me(),
        lane.beat(),
        lane.result(),
        mush_bot::tail(&client.seen, 4 * 1024)
    );
}

/// The id after `after` in `text`.
pub(crate) fn id_after(text: &str, after: &str) -> u64 {
    text.split(after)
        .nth(1)
        .and_then(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits.parse::<u64>().ok()
        })
        .unwrap_or_else(|| panic!("no id after {after:?}:\n{text}"))
}

/// **A player's credit account and balance, minor units**, off their own `@account/list` (a
/// read; the float's precedent).
async fn credit_of(client: &mut arena::Client) -> (u64, i64) {
    let listed = client.say("@account/list", " tacsim.credit = ").await;
    let row = listed
        .lines()
        .find(|line| line.trim_start().starts_with('#') && line.contains(" tacsim.credit = "))
        .unwrap_or_else(|| panic!("no tacsim.credit account:\n{listed}"));
    let id = id_after(row, "#");
    let amount: String = row
        .split(" tacsim.credit = ")
        .nth(1)
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    (id, amount.parse().unwrap_or(0))
}

/// **A tug's tow of `hull` from beside the ramp, then its load** (GC.4's tow, aimed): the tow
/// typed and its attach read off the tow's own line, then `+tv/1 load go`; the caller waits for
/// the custody the load lands.
pub(crate) async fn tow_in(
    o: &mut arena::Client,
    god: &mut arena::Client,
    site: &str,
    hull: ObjId,
) {
    // Where everyone stands before the tow (the story's own trace).
    let (_, rigs) = typed(o, "+tv", &["Rig 1"]).await;
    let (_, index) = god
        .answer(
            &format!("@tacsim/site {site}"),
            &["Unit #", "units", "nothing"],
        )
        .await;
    println!(
        "rs9b: before the tow of #{}:\n{}\n{}",
        hull.0,
        rigs.trim_end(),
        index.trim_end()
    );
    let tow = format!("+tv/1 tow #{} go", hull.0);
    let from = o.seen.len();
    let (_, said) = typed(o, &tow, &["in tow", "Tow", "not a casualty", "cannot"]).await;
    println!("rs9b: {tow}\n{}", said.trim_end());
    // The attach lands a few IC minutes on: the tow's own line, "Rig 1 has the … in tow".
    assert!(
        o.watch(from, " in tow, at ", Duration::from_secs(10 * 60))
            .await,
        "the tow never attached:\n{}",
        mush_bot::tail(&o.seen, 4 * 1024)
    );
    let (_, said) = typed(o, "+tv/1 load go", &["load", "Load", "cannot"]).await;
    println!("rs9b: +tv/1 load go\n{}", said.trim_end());
}

/// **O's rig tows `hull` from beside the ramp and loads it into O's bay** ([`tow_in`]), until her
/// salvage custody reads.
async fn tow_and_load(o: &mut arena::Client, god: &mut arena::Client, site: &str, hull: ObjId) {
    tow_in(o, god, site, hull).await;
    let preview = format!("+salvage/claim #{}", hull.0);
    // In custody once the load lands: the claim's preview names the registry.
    let filed = until(
        o,
        &preview,
        &["You would file a salvage claim", "Nothing to claim"],
        5,
        Duration::from_secs(10 * 60),
    )
    .await;
    println!("rs9b: #{} in O's custody: {}", hull.0, filed.trim_end());
}

/// **Story (b) on `ground`**, the cast `cast`.
#[allow(clippy::too_many_lines)]
pub(crate) async fn story_b(cast: Cast, ground: &'static Ground) {
    let started = Instant::now();
    let site = ground.entry;
    let stamp = |what: &str| println!("rs9b {site}: +{} s {what}", started.elapsed().as_secs());
    let mut made = None;
    let (rig, said) = mine::boot_with(
        &[cast.t, cast.o, cast.l],
        |cfg| {
            cfg.tacsim.recovery.inspect_s = 30;
            cfg.tacsim.recovery.attach_s = 30;
            cfg.tacsim.recovery.load_s = 60;
        },
        |engine| {
            let (said, m) = fixture(engine, ground, cast);
            made = Some(m);
            said
        },
    )
    .await;
    let made = made.expect("the fixture ran");
    stamp(&format!("the fixture: {said:?}"));
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut t = arena::Client::login(
        &rig,
        cast.t.account,
        arena::PILOT_PASSWORD,
        cast.t.character,
    )
    .await;
    let mut o = arena::Client::login(
        &rig,
        cast.o.account,
        arena::PILOT_PASSWORD,
        cast.o.character,
    )
    .await;
    let mut l = arena::Client::login(
        &rig,
        cast.l.account,
        arena::PILOT_PASSWORD,
        cast.l.character,
    )
    .await;
    let (a, c) = (format!("#{}", made.a.0), format!("#{}", made.c.0));
    // Both hulls on the site's index as casualties (GC.5's fixture), one write at a time.
    for (hull, (x, y)) in [made.a, made.c].into_iter().zip(made.cells) {
        let left = god
            .say(
                &format!("@tacsim/site {site} leave #{} casualty {x},{y}", hull.0),
                "Site record write requested",
            )
            .await;
        println!("rs9b: {}", left.trim_end());
        let indexed = Instant::now() + Duration::from_secs(60);
        loop {
            god.idle(Duration::from_secs(2)).await;
            let (_, record) = god
                .answer(&format!("@tacsim/site {site}"), &["Objects:"])
                .await;
            if record.contains(&format!("Unit #{}:", hull.0)) {
                break;
            }
            assert!(
                Instant::now() < indexed,
                "#{} never indexed:
{record}",
                hull.0
            );
        }
    }
    let (o_name, t_name) = (cast.o.character, cast.t.character);

    if let Ok(wait) = std::env::var("RS9B_READY_WAIT_S") {
        god.idle(Duration::from_secs(wait.parse().unwrap_or(120)))
            .await;
        let (_, status) = god
            .answer("@tacsim/port status", &[mine::READY, mine::NOT_READY])
            .await;
        let shut: Vec<&str> = status.lines().filter(|l| !l.contains("[ready]")).collect();
        println!(
            "rs9b: readiness:
{}",
            shut.join(
                "
"
            )
        );
    }
    // 0. T and O fly out; T sits on the pad, O's rig comes down the ramp.
    let mut t_lane = enrol(&mut t, cast.t, started).await;
    let mut o_lane = enrol(&mut o, cast.o, started).await;
    mine::lift_rate(&mut god, mine::LIFTED_RATE).await;
    stamp("T and O are enrolled; the rate lifted");
    fly(&mut t, &mut t_lane, started, |lane| {
        lane.reached(Milestone::ArrivedSite)
    })
    .await;
    stamp("T is on the pad");
    fly(&mut o, &mut o_lane, started, |lane| {
        lane.reached(Milestone::Launched)
    })
    .await;
    stamp("O's rig is down the ramp");
    let t_hull = t_lane.settings().hull.clone();

    // 1. The authorizations, before any attach (Q-41-21): a stranger may authorize nothing; T
    //    authorizes O (and L) to recover `c`; O holds no block; L delegates `c`'s block to O.
    let c_block = format!("{}/{}", ground.entry, made.c_letter);
    let (index, said) = typed(
        &mut l,
        &format!("+salvage/authorize {c}={o_name}"),
        &["not yours to authorize"],
    )
    .await;
    assert_eq!(index, Some(0), "L may not authorize T's hull:\n{said}");
    let (index, said) = typed(
        &mut t,
        &format!("+salvage/authorize {c}={o_name}"),
        &[&format!("You would authorize {o_name}")],
    )
    .await;
    assert_eq!(index, Some(0), "T's preview:\n{said}");
    for who in [o_name, cast.l.character] {
        let (index, said) = typed(
            &mut t,
            &format!("+salvage/authorize {c}={who} go"),
            &["unit authority records the grant"],
        )
        .await;
        assert_eq!(index, Some(0), "T authorizes {who}:\n{said}");
    }
    let (index, said) = typed(
        &mut o,
        &format!("+salvage/authorize {c_block}={}", cast.l.character),
        &["no live claim"],
    )
    .await;
    assert_eq!(index, Some(0), "O holds no block:\n{said}");
    let (index, said) = typed(
        &mut l,
        &format!("+salvage/authorize {c_block}={o_name} go"),
        &["recover hulls off your block"],
    )
    .await;
    assert_eq!(index, Some(0), "L delegates the block:\n{said}");
    stamp("the authorizations stand");

    // 2. O recovers both into O's bay and files a claim on each; every holder is told.
    o.idle(Duration::from_secs(5)).await;
    for hull in [made.a, made.c] {
        tow_and_load(&mut o, &mut god, site, hull).await;
    }
    for hull in [&a, &c] {
        let (index, said) = typed(
            &mut o,
            &format!("+salvage/claim {hull} go"),
            &["You file a salvage claim"],
        )
        .await;
        assert_eq!(index, Some(0), "O files on {hull}:\n{said}");
    }
    stamp("O's claims are filed");
    // O's quote under the Salvor's right reads at their risk.
    let (index, said) = typed(
        &mut o,
        &format!("+bay/quote {c}"),
        &[tacsim::surface::claim::AT_RISK_QUOTE],
    )
    .await;
    assert_eq!(index, Some(0), "O's quote of `c` is at O's risk:\n{said}");

    // 3. `a`: procedure finds no defect, and O may not contest; right finds nothing that bound
    //    O; T's recovery, on T's own sensor record, vacates the claim.
    let checks = [
        (
            true,
            format!("+salvage/contest {a}=procedure"),
            "show no defect".to_owned(),
        ),
        (
            false,
            format!("+salvage/contest {a}=procedure"),
            "Only someone who answers".to_owned(),
        ),
        (
            true,
            format!("+salvage/contest {a}=right"),
            format!("no right that bound {o_name}"),
        ),
        (
            true,
            format!("+salvage/contest {a}=recovery"),
            "sensor record".to_owned(),
        ),
    ];
    for (by_t, line, needle) in checks {
        let who = if by_t { &mut t } else { &mut o };
        let (index, said) = typed(who, &line, &[needle.as_str()]).await;
        assert_eq!(
            index,
            Some(0),
            "{line}:
{said}"
        );
    }
    let (_, said) = typed(
        &mut t,
        &format!("+salvage/contest {a}=recovery go"),
        &["contest", "Contest"],
    )
    .await;
    println!("rs9b: T> contest a recovery go\n{}", said.trim_end());

    // 4. `c`: L's own authorization is no right; T's bound O to deliver her, and vacates.
    let (index, said) = typed(
        &mut l,
        &format!("+salvage/contest {c}=right"),
        &[&format!("no right that bound {o_name}")],
    )
    .await;
    assert_eq!(index, Some(0), "L's right:\n{said}");
    let (index, said) = typed(
        &mut t,
        &format!("+salvage/contest {c}=right"),
        &[&format!("bound {o_name} to deliver her")],
    )
    .await;
    assert_eq!(index, Some(0), "T's right preview:\n{said}");
    let (_, said) = typed(
        &mut t,
        &format!("+salvage/contest {c}=right go"),
        &["contest", "Contest"],
    )
    .await;
    println!("rs9b: T> contest c right go\n{}", said.trim_end());
    // Both claims vacate: each hull in O's bay is held for its title's holder.
    for hull in [&a, &c] {
        let held = until(
            &mut o,
            &format!("+bay {hull}"),
            &["is held for", "Condition"],
            5,
            Duration::from_secs(5 * 60),
        )
        .await;
        println!("rs9b: {hull} vacated: {}", held.trim_end());
    }
    stamp("both claims vacated");

    // 5. O sails home, the tug left on the site, broke so the fine stands as a debt (God moves
    //    all but 100 cr of O's credit to a system sink).
    let (o_account, o_balance) = credit_of(&mut o).await;
    let sink = god
        .say(
            "@account/system rs9b_sink=tacsim.credit",
            "Opened account #",
        )
        .await;
    let sink = id_after(&sink, "Opened account #");
    // Held funds (the leases' deposits) are not available, so God moves O's money out in
    // shrinking amounts until each is refused: O is left with less than the smallest.
    let _ = o_balance;
    for chunk in [500_000_u64, 100_000, 10_000] {
        loop {
            let (index, paid) = god
                .answer(
                    &format!("@econ/pay {o_account}={sink} {chunk}"),
                    &["Paid: transaction #", "insufficient", "Usage", "denied"],
                )
                .await;
            match index {
                Some(0) => {}
                Some(1) => break,
                _ => panic!(
                    "God spends O down:
{paid}"
                ),
            }
        }
    }
    println!("rs9b: O is spent down under 100.00cr available");
    let lifted = mine::lift_rate(&mut god, 3_600).await;
    stamp(&format!("rate: {}", lifted.lines().next().unwrap_or("")));
    let (_, said) = typed(
        &mut o,
        "+ship/lift ceres leave=1",
        &["lifts", "Lift", "cannot", "refuse"],
    )
    .await;
    println!("rs9b: O> +ship/lift ceres leave=1\n{}", said.trim_end());
    let docked = Instant::now() + Duration::from_secs(20 * 60);
    let heard = loop {
        o.idle(Duration::from_secs(10)).await;
        if o.seen.contains("fines you") {
            break o.seen.clone();
        }
        assert!(Instant::now() < docked, "O never docked fined:\n{}", o.seen);
    };
    let fine_line = heard
        .lines()
        .find(|line| line.contains("fines you"))
        .unwrap_or_default()
        .trim()
        .to_owned();
    stamp(&format!("O at Ceres: {fine_line}"));
    assert!(fine_line.contains("an unlawful recovery"), "{fine_line}");
    // The game wraps a long line at the session's width: read the fine whole.
    let owed = mush_bot::mine::lines::normalise(&heard).contains("stands as a debt");
    // The vacated hulls, berthed held for T (T has no carrier docked).
    for hull in [&a, &c] {
        let held = until(
            &mut t,
            &format!("+bay {hull}"),
            &[&format!("is held for {t_name}"), "Condition"],
            10,
            Duration::from_secs(5 * 60),
        )
        .await;
        println!("rs9b: {hull} at Ceres: {}", held.trim_end());
    }

    // 6. The grace, then the quarantine (only while the fine stands unpaid), then O pays.
    if owed {
        let (index, said) = typed(&mut o, "+debts", &["+debts/pay"]).await;
        assert_eq!(index, Some(0), "O's debts:\n{said}");
        let debt = id_after(&said, "#");
        let held = until(
            &mut o,
            "+ship/undock luna",
            &["lifts when it is paid", "Usage"],
            30,
            Duration::from_secs(30 * 60),
        )
        .await;
        stamp(&format!("O's carrier held: {}", held.trim_end()));
        let source = god
            .say(
                "@account/system rs9b_float=tacsim.credit",
                "Opened account #",
            )
            .await;
        god.say(
            &format!(
                "@econ/mint {}={o_account} 1000000",
                id_after(&source, "Opened account #")
            ),
            "Minted",
        )
        .await;
        let (index, said) = typed(
            &mut o,
            &format!("+debts/pay {debt}"),
            &[&format!("Debt #{debt} paid")],
        )
        .await;
        assert_eq!(index, Some(0), "O pays:\n{said}");
        stamp("O paid the fine");
    } else {
        stamp("O's account covered the fine at docking: no quarantine to show");
    }

    // 7. T sails home and stows both vacated hulls out of the workshop into T's own carrier.
    let from = t.seen.len();
    let (_, said) = typed(&mut t, "+ship/lift ceres", &["lifts", "Lift", "cannot"]).await;
    println!("rs9b: T> +ship/lift ceres\n{}", said.trim_end());
    let docks = mush_bot::mine::lines::docks(&t_hull);
    let home = Instant::now() + Duration::from_secs(20 * 60);
    while !t.seen.get(from..).unwrap_or("").contains(&docks) {
        assert!(
            Instant::now() < home,
            "T never docked home:\n{}",
            t.seen.get(from..).unwrap_or("")
        );
        t.idle(Duration::from_secs(10)).await;
    }
    stamp("T docked at Ceres");
    // T walks to the workshop (God's hand, the `move_to` seam): off the ship, then to C14.
    let (_, said) = typed(&mut t, "+ship/leave", &["leave", "Leave", "aboard"]).await;
    println!("rs9b: T> +ship/leave\n{}", said.trim_end());
    let (_, moved) = god
        .answer(
            &format!("@teleport #{}=#{}", made.t.0, made.workshop.0),
            &[
                "Teleported",
                "teleport",
                "Done",
                "denied",
                "Huh",
                "can't",
                "C14",
            ],
        )
        .await;
    println!("rs9b: God> @teleport T to C14: {}", moved.trim_end());
    t.idle(Duration::from_secs(2)).await;
    for hull in [&a, &c] {
        let (index, said) = typed(
            &mut t,
            &format!("+bay/stow {hull}={t_hull}"),
            &[&format!("is stowed in the {t_hull}'s bay")],
        )
        .await;
        assert_eq!(index, Some(0), "T stows {hull}:\n{said}");
    }
    stamp("T has both hulls aboard");
    assert!(started.elapsed() < STORY_BOUND);
    for client in [t, o, l, god] {
        client.close().await;
    }
    drop(rig);
}
