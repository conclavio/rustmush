//! **Doc 40 cut FR.7 — the round's story** (§6's FR.7 row, §4.7, Q-38-29, Q-40-23): one script,
//! run at 256 (Ahuna Flank) and at 512 (Urvara Reach) over the two [`Ground`]s of
//! `crates/tacsim/tests/support/field/grounds_rig.rs` (declared by path at this binary's root),
//! on the production root booted in this test binary. Every player's half is typed over that
//! player's own SSH session in line mode, admitted by the bot's own closed vocabularies
//! (`mush_bot::mine::lane::field::in_vocabulary` for the captain, `…::escort::in_vocabulary` for
//! the two Basin pilots) before it goes, and read by the bot's own parsers where the bot has one
//! (`lines::void_offered`, `lines::void_answer`, `lines::own_party`, `lines::field_contact`, the
//! arena line classifier's weapon rows).
//!
//! **The cast.** Concord's captain is the bot's own hired-crew lane (`Lane::with_detachment`),
//! played in-process over its SSH session exactly as `--mine-detachment` plays it: the kit-up at
//! Ceres, Ilya Mendes hired for rig 1, the captain in rig 2's cab, the operation drafted,
//! accepted, readied and activated, the hop to the site. The lane is stopped where it would type
//! `+sortie/launch <op>=all`; from there the script types the captain's lines itself (FR.0's
//! precedent). Basin's two pilots, the raider and its wing, each fly an armed hull on the ground.
//!
//! **The beats, in order** (each its own section below):
//! 1. **The raid of a declared-Hostile co-op** (FR.1): the raider types `+faction/stance
//!    Basin=Concord/hostile`; the captain's `+sortie/launch <op>=1` puts the miner's rig 1 down
//!    the carrier's ramp; the raider's `+field/mark W=hostile` on it opens the field; Basin's
//!    party holds Concord's Hostile on the live record from admission, and the raider's cockpit
//!    `LOCK`/`FIRE` lands on Concord.
//! 2. **Reinforcement from the pad** (FR.3): the captain's `+sortie/launch <op>=2` into the
//!    running field; rig 2, the captain in its cab, joins Concord's party on the live record.
//! 3. **A tow struck and released under fire** (FR.5's coupled tow): **pending** — see
//!    [`tow_struck_before_field`] and [`tow_released_under_fire`], the marked seam.
//! 4. **A jammed sight** (FR.5 item 4): the wing's hull is the fixture design [`JAMMING_UNIT`]
//!    (the line tank with every survivable hit jamming a part) worn to its armour floor; the
//!    raider's machine gun on it jams the turret's parts until the sight is one, and the field's
//!    snapshot holds the wing's `sensor` empty (no detection, no lock).
//! 5. **Friendly fire voided at once** (FR.2): the raider and the wing are both Basin, so the
//!    raider's rounds on the wing are friendly fire; the raider stops, the field demotes on its
//!    idle bound, the raider is offered the void, and its next line, `+field/void`, takes it.
//! 6. **A segment rolled** (FR.4): `[tacsim.arena.ledger].max_bytes` lowered to the production
//!    floor ([`MAX_BYTES`], 1 MiB; the boot refuses less) and `summary_ms` to its floor, 1 s; the
//!    live record reaches segment 2 before the field demotes. At that floor a five-hull field
//!    ships about one 16 KiB chunk a minute, so the roll (three quarters of the bound) comes some
//!    forty minutes into the field: the raider goes quiet after the jam and the field waits on
//!    its idle bound ([`IDLE_DEMOTE_S`]), which is most of the run's hour. Beat 5's demotion
//!    follows the roll.
//!
//! **What is a fixture** (as FR.1's and FR.2's stories): the two Basin hulls are made, placed,
//! seated and (the wing's) worn by a staff hand at boot, before the world task spawns; the two
//! factions are made and staffed by `@faction/create` and `@faction/add` (mush-org has no
//! membership verb); the jamming design is a test pack on `[tacsim].pack_paths`, signed as any
//! operator pack is. Everything after the boot is a player's command over SSH. The world is read
//! (never written) through the store: the field's live record and its snapshot.
//!
//! `#[ignore]`d, run by name on conclav.dev (tier: dispatch, doc 43 TS.2; `40-cut-notes.md` FR.7):
//!
//! `cargo test -j 8 -p mush-bot --test mining_cohort -- --ignored fr7_ --nocapture`

use std::time::{Duration, Instant};

use mush_bot::mine::{
    Lane, Settings, Site, hull_name,
    lane::{escort, field},
    lines,
};
use mush_core::{GOD, ObjId};
use serde_json::Value;

use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};
use crate::{arena, mine};

/// **The ledger's bound for the run**: the production floor (`ArenaConfig::validate` refuses
/// less than 1 MiB), so a segment rolls at three quarters of it.
const MAX_BYTES: u64 = 1_048_576;
/// **The field's idle bound for the run** (`[tacsim.field].idle_demote_s` and `disengage_s`):
/// long enough that, once the raider stops firing, the black box fills to the roll before the
/// field demotes (beat 6); the demotion and its void (beat 5) follow.
const IDLE_DEMOTE_S: u32 = 3_300;
/// The fixture design the wing flies: the line tank whose every survivable hit jams a part.
const JAMMING_UNIT: &str = "fr7.unit.jamming_tank";
/// Its pack.
const JAMMING_PACK: &str = "fr7.pack.jamming_tank";
/// The shipped line tank: the jamming design's source and the raider's hull.
const LINE_TANK: &str = "tacsim.unit.line_tank";
/// The line tank's pack, as shipped.
const LINE_TANK_JSON: &str = include_str!("../../../tacsim/content/units/line_tank.json");
/// A heading due west, millidegrees (`grounds_rig`'s convention: 270 000 faces the pad from the
/// east).
const WEST: u32 = 270_000;

/// Concord's captain: the bot's hired-crew lane.
const CAPTAIN: mine::Miner = mine::Miner {
    account: "fr7cap",
    character: "Roundcap",
    site: Site::Ahuna,
};
/// Basin's raider.
const RAIDER: mine::Miner = mine::Miner {
    account: "fr7raid",
    character: "Roundraid",
    site: Site::Ahuna,
};
/// Basin's wing.
const WING: mine::Miner = mine::Miner {
    account: "fr7wing",
    character: "Roundwing",
    site: Site::Ahuna,
};

// ---------------------------------------------------------------------------
// The fixture (at boot, before the world task)
// ---------------------------------------------------------------------------

/// **The jamming design's pack**: the shipped line tank under a pack and a unit id of its own,
/// with `component_hits` at a jam every survivable hit and no damage, signed with the digest the
/// game's own `load_pack` computes for it. Written into `dir`; its path.
fn jamming_pack(dir: &std::path::Path) -> String {
    let mut pack: Value = serde_json::from_str(LINE_TANK_JSON).expect("the line tank's pack");
    pack["manifest"]["id"] = Value::from(JAMMING_PACK);
    pack["manifest"]["version"] = Value::from(1);
    pack["manifest"]["digest"] = Value::from("0".repeat(64));
    let mut entry = pack["entries"][LINE_TANK].clone();
    entry["def"]["label"] = Value::from("jamming tank");
    entry["def"]["component_hits"] =
        serde_json::json!({"jam_permille": 1000, "damage_permille": 0});
    pack["entries"] = serde_json::json!({ JAMMING_UNIT: entry });
    let unsigned = serde_json::to_string_pretty(&pack).expect("the pack's JSON");
    let digest = match tacsim::pack::load_pack(&unsigned) {
        Err(tacsim::pack::PackError::DigestMismatch { computed, .. }) => computed,
        other => panic!("the unsigned jamming pack: {:?}", other.map(|_| ())),
    };
    pack["manifest"]["digest"] = Value::from(digest);
    let signed = serde_json::to_string_pretty(&pack).expect("the pack's JSON");
    tacsim::pack::load_pack(&signed).expect("the signed jamming pack loads");
    std::fs::create_dir_all(dir).expect("the pack's directory");
    let path = dir.join("fr7_jamming_tank.json");
    std::fs::write(&path, signed).expect("the pack is written");
    path.display().to_string()
}

/// A canonical id.
fn cid(word: &str) -> mush_core::CanonicalId {
    mush_core::CanonicalId::new(word).expect("a canonical id")
}

/// **A staff hand on the booted engine** before the world task spawns (the fixture's seam, as
/// tacsim's `driving_rig::staff_hand`): `f` over a `Ctx` acting as God.
fn staff_hand<T>(
    engine: &mut mush_core::Engine,
    f: impl FnOnce(&mut mush_core::Ctx<'_>) -> T,
) -> T {
    let mut out = mush_core::Outbox::new();
    let mut ctx = mush_core::Ctx {
        w: &mut engine.world,
        actor: GOD,
        switches: mush_core::SwitchSet::default(),
        args: mush_core::ParsedArgs::default(),
        out: &mut out,
        reg: &engine.registry,
        module: Some(tacsim::NS.into()),
        typed_state: mush_core::ModuleActionState::unavailable(),
    };
    f(&mut ctx)
}

/// The object called `name`.
fn named(engine: &mush_core::Engine, name: &str) -> ObjId {
    engine
        .world
        .iter()
        .find(|row| row.name == name)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("no object called {name}"))
}

/// **Where the Basin hulls stand on `ground`**: off the carrier's stern ramp cell (its pad's cell
/// less four rows, `driving_rig::RAMP_CELL` on every site), the raider's sixteen cells due east of
/// it facing west, so the rigs that come down the ramp are its `W` and the nearest flight there;
/// the wing's three cells west of the raider and three north (its `NW`).
fn stands(engine: &mush_core::Engine, ground: &Ground) -> ((i64, i64), (i64, i64)) {
    let (x, y) = tacsim::surface::excursion::site_pad_cm(&engine.world, &cid(ground.entry));
    let cell = i64::from(tacsim::surface::ground::SITE_CELL_CM);
    let ramp = (x.div_euclid(cell), y.div_euclid(cell) - 4);
    ((ramp.0 + 16, ramp.1), (ramp.0 + 13, ramp.1 + 3))
}

/// **An armed hull of `design` on `ground` at `cell`, its cockpit seated to `pilot`** (the
/// fixture, as `field_host_rig::armed_on` and `seat`).
fn armed_seated(
    engine: &mut mush_core::Engine,
    ground: &Ground,
    design: &str,
    name: &str,
    cell: (i64, i64),
    pilot: ObjId,
) -> ObjId {
    let domain = engine
        .world
        .module_state::<tacsim::TacsimStore>()
        .expect("the layer")
        .port_maps()
        .find(|map| map.entry.as_str() == ground.entry)
        .and_then(|map| map.domain)
        .unwrap_or_else(|| panic!("{}'s domain", ground.entry));
    staff_hand(engine, |ctx| {
        let spec = ctx
            .w
            .module_state::<tacsim::TacsimStore>()
            .expect("the layer")
            .definitions()
            .bindings
            .get(&cid(design))
            .unwrap_or_else(|| panic!("{design} is bound"))
            .spec
            .clone();
        let hull = ctx
            .w
            .create(mush_core::ObjKind::Thing, name, GOD, None)
            .expect("the hull's object");
        mush_tactics::module::create_unit(ctx, hull, &spec).expect("the hull's unit record");
        let pose = tacsim::surface::excursion::cell_pose([cell.0, cell.1], WEST).expect("a pose");
        mush_sim::place(ctx, hull, mush_sim::DomainId(domain), pose).expect("on the ground");
        ctx.actor = pilot;
        let console = ctx.w.get(pilot).and_then(|object| object.location);
        let station = mush_sim::StationId::new("cockpit").expect("a station id");
        let channels = tacsim::match_::cockpit::cockpit_channels();
        mush_sim::declare_station(
            ctx,
            hull,
            &station,
            tacsim::match_::cockpit::STATION_ROLE,
            &channels,
            console,
            tacsim::match_::cockpit::STATION_PRIORITY,
        )
        .expect("the station");
        mush_sim::request_seat_activation(
            ctx,
            &mush_sim::SeatRequest::Activate {
                unit: hull,
                station,
                npc: pilot,
                granted: channels,
                fallback: None,
            },
        )
        .expect("the seat");
        mush_sim::links::refresh_actor_links(ctx.w, pilot);
        hull
    })
}

/// Every armour cell of `hull` at its floor, as a mutation plan's JSON (`mine::wear_raiders`).
fn worn_plan(engine: &mush_core::Engine, hull: ObjId) -> String {
    let unit = mush_tactics::module::unit_of(&engine.world, hull)
        .expect("the hull's unit")
        .clone();
    let ops: Vec<mush_tactics::UnitMutationOp> = unit
        .cells()
        .filter_map(|(addr, cell)| {
            let name = match addr {
                mush_tactics::CellAddr::Facet { cell, .. }
                | mush_tactics::CellAddr::Section { cell, .. } => cell.as_str(),
                _ => return None,
            };
            (name == tacsim::definitions::unit::ARMOR_CELL).then(|| {
                mush_tactics::UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value: cell.min,
                }
            })
        })
        .collect();
    let plan =
        mush_tactics::UnitMutationPlan::new(ops, None, Some("bot-story:fr7-worn-wing".into()))
            .expect("a plan");
    serde_json::to_string(&plan).expect("the plan's JSON")
}

/// The hulls the fixture made.
#[derive(Clone, Copy, Debug)]
struct Hulls {
    raider: ObjId,
    wing: ObjId,
}

/// **The fixture**: the two factions and their officers, the raider's line tank and the wing's
/// jamming tank on the ground and seated, the wing worn to its armour floor. What staff said,
/// and the hulls.
fn fixture(engine: &mut mush_core::Engine, ground: &Ground) -> (Vec<String>, Hulls) {
    let captain = named(engine, CAPTAIN.character);
    let raider = named(engine, RAIDER.character);
    let wing = named(engine, WING.character);
    let mut said = Vec::new();
    for (faction, members) in [
        ("Concord", vec![(captain, 2)]),
        ("Basin", vec![(raider, 2), (wing, 1)]),
    ] {
        said.push(mine::staff_line(
            engine,
            &format!("@faction/create {faction}"),
        ));
        for (who, level) in members {
            said.push(mine::staff_line(
                engine,
                &format!("@faction/add {faction}=#{}/level:{level}", who.0),
            ));
        }
    }
    let (at_raider, at_wing) = stands(engine, ground);
    let hulls = Hulls {
        raider: armed_seated(engine, ground, LINE_TANK, "Basin Hound", at_raider, raider),
        wing: armed_seated(engine, ground, JAMMING_UNIT, "Basin Whelp", at_wing, wing),
    };
    let plan = worn_plan(engine, hulls.wing);
    said.push(mine::staff_line(
        engine,
        &format!("@unit/damage #{}={plan}", hulls.wing.0),
    ));
    (said, hulls)
}

// ---------------------------------------------------------------------------
// Reading the world (the store; never a write)
// ---------------------------------------------------------------------------

/// The field's live record on `ground`, as the store holds it.
async fn live(rig: &arena::Rig, ground: &Ground) -> Option<Value> {
    rig.store
        .module_json_get(tacsim::NS, &format!("tacsim:field:live:{}", ground.entry))
        .await
        .ok()
        .flatten()
}

/// How many black-box chunk documents naming `field.<site>.` the store holds: the fill, at
/// 16 KiB a chunk.
async fn chunks(rig: &arena::Rig, ground: &Ground) -> usize {
    let needle = format!("field.{}.", ground.entry);
    rig.store
        .module_keys(tacsim::NS)
        .await
        .map(|keys| keys.iter().filter(|key| key.contains(&needle)).count())
        .unwrap_or(0)
}

/// The live record's segment.
fn segment(live: &Value) -> u64 {
    live["segment"].as_u64().unwrap_or(0)
}

/// The party the live record files `unit` under, as its JSON.
fn party_of(live: &Value, unit: u64) -> Option<Value> {
    live["units"]
        .as_array()?
        .iter()
        .find(|row| row["unit"].as_u64() == Some(unit))
        .map(|row| row["party"].clone())
}

/// A party's letter from its JSON (`"red"`, `"blue"` or a letter word).
fn letter(party: &Value) -> char {
    match party.as_str() {
        Some("red") => 'R',
        Some("blue") => 'B',
        Some(word) => word
            .chars()
            .next()
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('?'),
        None => party
            .to_string()
            .chars()
            .find(char::is_ascii_alphabetic)
            .map(|c| c.to_ascii_uppercase())
            .unwrap_or('?'),
    }
}

/// The live record's row for `party`.
fn party_row<'a>(live: &'a Value, party: &Value) -> Option<&'a Value> {
    live["parties"]
        .as_array()?
        .iter()
        .find(|row| &row["party"] == party)
}

/// **Whether the field's snapshot holds `unit` with no sight** (`UnitState::sensor` empty):
/// `Some(true)` for a dark sight, `Some(false)` for one that sees, `None` while no snapshot holds
/// the unit.
async fn sight_dark(rig: &arena::Rig, ground: &Ground, unit: u64) -> Option<bool> {
    let snapshot = rig
        .store
        .module_json_get(
            tacsim::NS,
            &format!("tacsim:field:snapshot:{}", ground.entry),
        )
        .await
        .ok()
        .flatten()?;
    find_sensor(&snapshot, unit)
}

/// The walk: an object naming `unit` as its `id` with a `sensor` key answers whether the key is
/// null; a long string that parses as JSON (the snapshot's `state`) is walked too.
fn find_sensor(value: &Value, unit: u64) -> Option<bool> {
    match value {
        Value::Object(map) => {
            if map.get("id").and_then(Value::as_u64) == Some(unit)
                && let Some(sensor) = map.get("sensor")
            {
                return Some(sensor.is_null());
            }
            map.values().find_map(|inner| find_sensor(inner, unit))
        }
        Value::Array(rows) => rows.iter().find_map(|inner| find_sensor(inner, unit)),
        Value::String(text) if text.len() > 64 && text.starts_with('{') => {
            serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|inner| find_sensor(&inner, unit))
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// The players' lines (the bot's vocabularies, asserted before each line goes)
// ---------------------------------------------------------------------------

/// The captain types `line` (the captain's field vocabulary) and waits for one of `needles`.
async fn captain_says(
    client: &mut arena::Client,
    line: &str,
    needles: &[&str],
) -> (Option<usize>, String) {
    assert!(
        field::in_vocabulary(line),
        "the captain's vocabulary: {line}"
    );
    let answer = client.answer(line, needles).await;
    println!("fr7: captain> {line}\n{}", answer.1.trim_end());
    answer
}

/// A Basin pilot types `line` (the escort's vocabulary, which carries the cockpit's lines and
/// the round's words) and waits for one of `needles`.
async fn pilot_says(
    who: &str,
    client: &mut arena::Client,
    line: &str,
    needles: &[&str],
) -> (Option<usize>, String) {
    assert!(
        escort::in_vocabulary(line),
        "the pilot's vocabulary: {line}"
    );
    let answer = client.answer(line, needles).await;
    println!("fr7: {who}> {line}\n{}", answer.1.trim_end());
    answer
}

/// `line`'s whole answer: sent, then a second of reading, everything since the send.
async fn pilot_reads(who: &str, client: &mut arena::Client, line: &str) -> String {
    assert!(
        escort::in_vocabulary(line),
        "the pilot's vocabulary: {line}"
    );
    let from = client.seen.len();
    client.send(line).await;
    client.idle(Duration::from_millis(1_500)).await;
    let text = client.seen.get(from..).unwrap_or("").to_owned();
    println!("fr7: {who}> {line}\n{}", text.trim_end());
    text
}

/// `+field`'s own party letter (the bot's `lines::own_party`).
async fn own_party(who: &str, client: &mut arena::Client, captain: bool) -> Option<char> {
    let (_, text) = if captain {
        captain_says(client, "+field", &[lines::YOUR_PARTY]).await
    } else {
        pilot_says(who, client, "+field", &[lines::YOUR_PARTY]).await
    };
    client.idle(Duration::from_millis(500)).await;
    lines::own_party(&lines::normalise(&text))
}

/// The raider's machine gun: `WEAPONSTATUS`'s row with the most rounds (the coaxial gun, whose
/// small rounds wear a worn hull a part at a time rather than wrecking it).
async fn machine_gun(raider: &mut arena::Client) -> String {
    let text = pilot_reads("raider", raider, "WEAPONSTATUS").await;
    text.lines()
        .filter_map(|line| match mush_bot::arena::lines::classify(line) {
            mush_bot::arena::lines::Line::WeaponRow(row) => Some(row),
            _ => None,
        })
        .max_by_key(|row| row.rounds_max)
        .map(|row| row.id)
        .unwrap_or_else(|| panic!("WEAPONSTATUS names a gun:\n{text}"))
}

/// The raider's `CONTACTS`, read whole: each classified contact's designator and party letter.
async fn contacts(raider: &mut arena::Client) -> Vec<(String, char)> {
    let text = pilot_reads("raider", raider, "CONTACTS").await;
    text.lines()
        .filter_map(lines::field_contact)
        .filter_map(|(designator, party)| party.map(|party| (designator, party)))
        .collect()
}

/// The raider locks the first contact of `party` and fires `gun` once; whether a lock was had.
async fn lock_and_fire(raider: &mut arena::Client, party: char, gun: &str) -> bool {
    let Some((designator, _)) = contacts(raider)
        .await
        .into_iter()
        .find(|(_, of)| of.eq_ignore_ascii_case(&party))
    else {
        return false;
    };
    let (_, locked) = pilot_says(
        "raider",
        raider,
        &format!("LOCK {designator}"),
        &["Locked on", "No such contact", "No lock"],
    )
    .await;
    if !locked.contains("Locked on") {
        return false;
    }
    let _ = pilot_reads("raider", raider, &format!("FIRE {gun}")).await;
    true
}

// ---------------------------------------------------------------------------
// The script
// ---------------------------------------------------------------------------

/// **Drive the captain's lane until the step it opens is `+sortie/launch …`**, never sending
/// that step: `mine::drive_lane_since` with the open step read after each feed and before each
/// tick, and the canary's rate lift once the pilot has enrolled. The seconds since `started`.
async fn drive_to_launch(
    client: &mut arena::Client,
    lane: &mut Lane,
    god: &mut arena::Client,
    started: Instant,
    bound: Duration,
) -> u64 {
    let epoch = lane.settings().epoch_unix_ms;
    let mut lifted = mine::rate_lift(mine::LIFTED_RATE).is_none();
    let at_launch = |lane: &Lane| {
        lane.step_line()
            .is_some_and(|line| line.starts_with("+sortie/launch "))
    };
    while started.elapsed() < bound && lane.result().is_none() && !at_launch(lane) {
        client.pump(Duration::from_millis(50)).await;
        let now = mine::unix_now_ms().saturating_sub(epoch);
        let text = client.drain();
        if !text.is_empty() {
            lane.feed(now, &text);
        }
        if at_launch(lane) {
            break;
        }
        if !lifted && lane.step_line() == Some("+coop/refit prospector") {
            if let Some(rate) = mine::rate_lift(mine::LIFTED_RATE) {
                mine::lift_rate(god, rate).await;
            }
            lifted = true;
        }
        for send in lane.tick(now) {
            println!("fr7: +{} s lane {}", started.elapsed().as_secs(), send.line);
            if let Err(error) = client.try_send(&send.line).await {
                println!("fr7: {error}");
            }
        }
        for event in lane.events() {
            println!("fr7: +{} s lane {event:?}", started.elapsed().as_secs());
        }
    }
    started.elapsed().as_secs()
}

/// **Beat 3, its first half: the tow struck before the field** (FR.5's coupled tow, Q-40-7,
/// Q-40-25).
///
/// FR.5 tow half pending: `surface/tow.rs`'s half of FR.5 (PR #415) is not on this branch, so a
/// tow struck on open ground cannot yet ride into the field coupled. When it lands, this step
/// types, before the raider's mark: a tug's `+tv/<n> tow go` on a casualty beside the ramp (read
/// through the bot's `lines::tow_line`), and asserts that the field's admission carries the pair
/// coupled (the casualty an entrant on its tug's party, in `FieldHost.tows`, not counted). It
/// answers what it did; nothing else in the script depends on it.
#[allow(clippy::unused_async)] // the seam the parent fills types over SSH
async fn tow_struck_before_field(ground: &Ground) -> &'static str {
    // FR.5 tow half pending: `+tv/<n> tow go` before the field (PR #415's `surface/tow.rs`).
    println!(
        "fr7: {} beat 3a (tow struck before the field): FR.5 tow half pending",
        ground.venue
    );
    "pending"
}

/// **Beat 3, its second half: the casualty hit under tow and released in the field.**
///
/// FR.5 tow half pending: once [`tow_struck_before_field`] couples a pair, this step has the
/// raider `LOCK` and `FIRE` on the towed casualty until a round lands, then the tug's pilot types
/// `+tv/<n> release go` (admitted by `field::round_word`), reads `ArenaOut::Released`'s line, and
/// asserts the casualty stands where it was dropped, uncoupled, on the next live record.
#[allow(clippy::unused_async)] // the seam the parent fills types over SSH
async fn tow_released_under_fire(ground: &Ground) -> &'static str {
    // FR.5 tow half pending: the casualty hit under tow, then `+tv/<n> release go` in the field.
    println!(
        "fr7: {} beat 3b (tow released under fire): FR.5 tow half pending",
        ground.venue
    );
    "pending"
}

/// **The round's story on `ground`.**
#[allow(clippy::too_many_lines)] // one script, beat by beat
async fn round_story(ground: &'static Ground) {
    let started = Instant::now();
    let site = Site::parse(ground.venue).expect("the bot names the site");
    let captain = mine::Miner { site, ..CAPTAIN };
    let pack_dir = std::env::temp_dir().join(format!(
        "mush-bot-fr7-pack-{}-{}",
        std::process::id(),
        ground.side
    ));
    let pack = jamming_pack(&pack_dir);
    let mut hulls = None;
    let (rig, staff) = mine::boot_with(
        &[captain, RAIDER, WING],
        |cfg| {
            cfg.tacsim.pack_paths = vec![pack.clone()];
            // Beat 5: the field demotes on its idle bound once the raider stops.
            cfg.tacsim.field.disengage_s = IDLE_DEMOTE_S;
            cfg.tacsim.field.idle_demote_s = IDLE_DEMOTE_S;
            // Beat 6: the black box at the production floor, a summary every second.
            cfg.tacsim.arena.ledger.max_bytes = MAX_BYTES;
            cfg.tacsim.arena.ledger.summary_ms = 1_000;
            cfg.tacsim.arena.snapshot_ms = 2_000;
        },
        |engine| {
            let (said, made) = fixture(engine, ground);
            hulls = Some(made);
            said
        },
    )
    .await;
    let _ = std::fs::remove_dir_all(&pack_dir);
    let hulls = hulls.expect("the fixture's hulls");
    println!("fr7: {} fixture {hulls:?}; staff said:", ground.venue);
    for line in &staff {
        println!("fr7:   {line}");
    }
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut cap = arena::Client::login(
        &rig,
        captain.account,
        arena::PILOT_PASSWORD,
        captain.character,
    )
    .await;
    let mut raider = arena::Client::login(
        &rig,
        RAIDER.account,
        arena::PILOT_PASSWORD,
        RAIDER.character,
    )
    .await;
    let mut wing =
        arena::Client::login(&rig, WING.account, arena::PILOT_PASSWORD, WING.character).await;

    // -- Beat 1a: Basin declares Concord Hostile (FR.1) -------------------------------------
    let (said, text) = pilot_says(
        "raider",
        &mut raider,
        "+faction/stance Basin=Concord/hostile",
        &["now stands", "Usage", "may not", "No ", "not "],
    )
    .await;
    assert_eq!(said, Some(0), "Basin declares Concord Hostile:\n{text}");

    // -- The captain's own lane: Ceres, the hire, the operation, the hop ---------------------
    let mut lane = Lane::new(
        site,
        Settings {
            hull: hull_name(&format!("{}+{}", captain.account, captain.character))
                .expect("the hull name"),
            epoch_unix_ms: mine::unix_now_ms(),
        },
        captain.character.to_owned(),
    )
    .with_detachment();
    let wall = drive_to_launch(
        &mut cap,
        &mut lane,
        &mut god,
        started,
        Duration::from_secs(40 * 60),
    )
    .await;
    if let Some(result) = lane.result() {
        panic!(
            "the captain's lane ended before its launch: {}",
            result.outcome
        );
    }
    let evidence = lane.report().detachment.expect("the expedition's evidence");
    let op = evidence.operation.clone();
    let (rig1, rig2) = (evidence.units[0], evidence.units[1]);
    println!(
        "fr7: {} +{wall} s the lane stands at its launch: operation {op}, rig 1 #{rig1}, rig 2 #{rig2}",
        ground.venue
    );
    let _ = raider.drain();
    let _ = wing.drain();

    // -- Beat 1b: the miner's rig down the ramp, then the raid ------------------------------
    let (said, text) = captain_says(
        &mut cap,
        &format!("+sortie/launch {op}=1"),
        &["Done.", "refused", "cannot", "No "],
    )
    .await;
    assert_eq!(said, Some(0), "rig 1 launches:\n{text}");
    let mut down = false;
    for _ in 0..60 {
        let (_, text) =
            captain_says(&mut cap, &format!("+sortie/operation {op}"), &["Slot 2"]).await;
        if text.contains("last confirmed on the surface") {
            down = true;
            break;
        }
        cap.idle(Duration::from_secs(2)).await;
    }
    assert!(down, "rig 1 stands on the surface");

    // Beat 3a, the seam: the tow struck before the field.
    let tow_a = tow_struck_before_field(ground).await;

    let text = pilot_reads("raider", &mut raider, "+field/mark W=hostile").await;
    let mut opened = None;
    for _ in 0..60 {
        if let Some(record) = live(&rig, ground).await {
            opened = Some(record);
            break;
        }
        raider.idle(Duration::from_secs(1)).await;
    }
    let opened = opened.unwrap_or_else(|| panic!("the raider's mark opens the field:\n{text}"));
    println!("fr7: {} the field opens: {opened}", ground.venue);
    let concord = party_of(&opened, rig1).expect("rig 1 is in the field");
    let basin = party_of(&opened, hulls.raider.0.into()).expect("the raider is in the field");
    let wing_party = party_of(&opened, hulls.wing.0.into()).expect("the wing is in the field");
    let basin_row = party_row(&opened, &basin).expect("Basin's row");
    assert!(
        basin_row["stance_hostile"]
            .as_array()
            .is_some_and(|rows| rows.iter().any(|row| row == &concord)),
        "Basin's party holds Concord's {concord} Hostile from admission: {basin_row}"
    );
    let concord_letter = letter(&concord);
    let wing_letter = letter(&wing_party);
    assert_eq!(
        own_party("wing", &mut wing, false).await,
        Some(wing_letter),
        "the wing reads its own party"
    );
    let gun = machine_gun(&mut raider).await;
    let mut struck = false;
    for _ in 0..20 {
        if lock_and_fire(&mut raider, concord_letter, &gun).await {
            struck = true;
            break;
        }
        raider.idle(Duration::from_secs(1)).await;
    }
    assert!(struck, "the raider locks and fires on Concord");

    // -- Beat 2: reinforcement from the pad (FR.3) ------------------------------------------
    let (said, text) = captain_says(
        &mut cap,
        &format!("+sortie/launch {op}=2"),
        &["Done.", "refused", "cannot", "No "],
    )
    .await;
    assert_eq!(
        said,
        Some(0),
        "rig 2 launches into the running field:\n{text}"
    );
    let mut joined = None;
    for _ in 0..90 {
        // The raider keeps the field engaged while the reinforcement comes down.
        let _ = lock_and_fire(&mut raider, concord_letter, &gun).await;
        if let Some(record) = live(&rig, ground).await
            && party_of(&record, rig2).is_some()
        {
            joined = Some(record);
            break;
        }
        raider.idle(Duration::from_secs(1)).await;
    }
    let joined = joined.expect("rig 2 joins the running field");
    assert_eq!(
        party_of(&joined, rig2),
        Some(concord.clone()),
        "the reinforcement fights in its operation's party"
    );
    assert_eq!(
        own_party("captain", &mut cap, true).await,
        Some(concord_letter),
        "the captain, in rig 2's cab, reads Concord's party"
    );

    // Beat 3b, the seam: the casualty hit under tow and released.
    let tow_b = tow_released_under_fire(ground).await;

    // -- Beat 4: a jammed sight (FR.5 item 4), by the raider's fire on its wing ---------------
    let mut dark = false;
    let mut shots = 0;
    for _ in 0..120 {
        if lock_and_fire(&mut raider, wing_letter, &gun).await {
            shots += 1;
        }
        if sight_dark(&rig, ground, hulls.wing.0.into()).await == Some(true) {
            dark = true;
            break;
        }
        raider.idle(Duration::from_secs(1)).await;
    }
    assert!(
        dark,
        "the wing's sight goes dark after {shots} shots: {:?}",
        sight_dark(&rig, ground, hulls.wing.0.into()).await
    );
    println!(
        "fr7: {} +{} s the wing's sight is jammed after {shots} shots",
        ground.venue,
        started.elapsed().as_secs()
    );

    // -- Beat 6: a segment rolled (FR.4), before the field demotes ----------------------------
    // From here the raider types nothing until the void's offer (beat 5): the position summaries
    // and the sensor passes fill the black box while the field waits out its idle bound.
    let quiet_from = raider.seen.len();
    let mut rolled = 0;
    let waited = Instant::now();
    let mut told = 0;
    while waited.elapsed() < Duration::from_secs(IDLE_DEMOTE_S.into()) {
        let Some(record) = live(&rig, ground).await else {
            break;
        };
        rolled = segment(&record);
        if rolled >= 2 {
            break;
        }
        if waited.elapsed().as_secs() >= told {
            told += 30;
            println!(
                "fr7: {} +{} s segment {rolled}, {} chunks shipped",
                ground.venue,
                started.elapsed().as_secs(),
                chunks(&rig, ground).await
            );
        }
        raider.idle(Duration::from_secs(2)).await;
    }
    assert!(
        rolled >= 2,
        "the field rolls onto segment 2 before it demotes (read {rolled}, {} chunks)",
        chunks(&rig, ground).await
    );
    println!(
        "fr7: {} +{} s the field is on segment {rolled}",
        ground.venue,
        started.elapsed().as_secs()
    );

    // -- Beat 5: friendly fire voided at once (FR.2) ----------------------------------------
    // The raider has typed nothing since the jam: the field demotes on its idle bound.
    let from = quiet_from;
    let offered = raider
        .watch(
            from,
            lines::VOID_OFFERED.trim(),
            Duration::from_secs(u64::from(IDLE_DEMOTE_S) + 120),
        )
        .await;
    let since = raider.seen.get(from..).unwrap_or("").to_owned();
    assert!(
        offered && lines::void_offered(&lines::normalise(&since)),
        "the raider is offered the void at demotion:\n{since}"
    );
    let (said, text) = pilot_says(
        "raider",
        &mut raider,
        "+field/void",
        &[lines::VOIDED, lines::NOTHING_TO_VOID],
    )
    .await;
    assert_eq!(
        (said, lines::void_answer(&lines::normalise(&text))),
        (Some(0), Some(true)),
        "the raider's first line after the offer voids it:\n{text}"
    );
    assert!(live(&rig, ground).await.is_none(), "the field has demoted");

    println!(
        "fr7: {} green in {} s: raid, reinforcement, tow {tow_a}/{tow_b}, sight jammed, void, segment {rolled}",
        ground.venue,
        started.elapsed().as_secs()
    );
    for client in [cap, raider, wing, god] {
        client.close().await;
    }
    drop(rig);
}

/// **FR.7 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 40 cut FR.7's round story at 256: the bot over SSH, about an hour; run by name (tier dispatch)"]
async fn fr7_the_rounds_story_at_256() {
    tokio::time::timeout(Duration::from_secs(100 * 60), round_story(&AHUNA_FLANK))
        .await
        .expect("the round's story at 256 finishes inside 100 minutes");
}

/// **FR.7 at 512**: Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 40 cut FR.7's round story at 512: the bot over SSH, about an hour; run by name (tier dispatch)"]
async fn fr7_the_rounds_story_at_512() {
    tokio::time::timeout(Duration::from_secs(100 * 60), round_story(&URVARA_REACH))
        .await
        .expect("the round's story at 512 finishes inside 100 minutes");
}
