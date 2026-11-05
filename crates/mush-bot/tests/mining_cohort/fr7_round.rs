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
//!    the carrier's ramp; the raider's `+field/mark <bearing>=hostile` ([`approach`]) on it opens the field; Basin's
//!    party holds Concord's Hostile on the live record from admission, and the raider's cockpit
//!    `LOCK`/`FIRE` lands on Concord.
//! 2. **Reinforcement from the pad** (FR.3): the captain's `+sortie/launch <op>=2` into the
//!    running field; rig 2, the captain in its cab, joins Concord's party on the live record.
//! 3. **A tow struck and released under fire** (GC.4 on open ground, FR.5b into the field):
//!    staff index the fixture's stranded rig as a casualty beside the ramp; the captain's
//!    `+tv/1 tow go` takes it in tow with rig 1 before the mark; the field admits the pair coupled;
//!    the raider's fire strikes the casualty in tow; the captain's `+tv/1 release go` lets it go
//!    under fire, and the snapshot stands it out of tow where it was ([`tow_struck_before_field`],
//!    [`tow_released_under_fire`]). The release is typed before beat 2's launch: once rig 2 flies,
//!    the captain sits at its helm and `+tv/1` names no rig of theirs.
//! 4. **A jammed sight** (FR.5 item 4): the wing's hull is the fixture design [`JAMMING_UNIT`]
//!    (the line tank with every survivable hit jamming a part) worn to its armour floor; the
//!    raider's machine gun on it jams the turret's parts until the sight is one, and the field's
//!    snapshot holds the wing's `sensor` empty (no detection, no lock).
//! 5. **Friendly fire voided at once** (FR.2): the raider and the wing are both Basin, so the
//!    raider's rounds on the wing are friendly fire; the raider stops, the field demotes on its
//!    idle bound, the raider is offered the void, and its next line, `+field/void`, takes it.
//! 6. **A segment rolled** (FR.4): the field's black box bounded at [`MAX_BYTES`] (48 KiB, FR.4's
//!    knob) with a position summary every [`SUMMARY_MS`], below the boot's production floors
//!    (1 MiB, 1 s), through tacsim's test-only `tacsim::store::lower_ledger_floor_for_test` on the
//!    booted engine (the `test-ledger-floor` feature, FR.7c). The live record reaches segment 2 in
//!    the field's first minutes, before the field demotes on its idle bound ([`IDLE_DEMOTE_S`]).
//!
//! **What is a fixture** (as FR.1's and FR.2's stories): the two Basin hulls are made, placed,
//! seated and (the wing's) worn by a staff hand at boot, before the world task spawns; the two
//! factions are made and staffed by `@faction/create` and `@faction/add` (mush-org has no
//! membership verb); the jamming design is a test pack on `[tacsim].pack_paths`, signed as any
//! operator pack is. The casualty (beat 3) is a stock rig placed at boot with its power destroyed,
//! and indexed on the site's record by staff's `@tacsim/site … leave … casualty` (GC.5's fixture,
//! `field_fr5`'s precedent: no player path disables a rig outside a field). Everything after is
//! a player's command over SSH. The world is read
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

/// **The ledger's bound for the run** (doc 40 cut FR.7c): FR.4's test knob, 48 KiB, below the
/// boot's 1 MiB floor, so a segment rolls at three quarters of it early in the field.
const MAX_BYTES: u64 = 48 * 1024;
/// **A position summary every tenth of a second** (FR.2's and FR.4's), below the boot's 1 s
/// floor: the black box fills at a known pace.
const SUMMARY_MS: u32 = 100;
/// **The field's idle bound for the run** (`[tacsim.field].idle_demote_s` and `disengage_s`):
/// past the reinforcement's landing (the raider holds fire while rig 2 comes down) and the roll;
/// once the raider stops after the jam the field demotes on it, and beat 5's void follows.
const IDLE_DEMOTE_S: u32 = 180;
/// The fixture design the wing flies: the line tank whose every survivable hit jams a part.
const JAMMING_UNIT: &str = "fr7.unit.jamming_tank";
/// Its pack.
const JAMMING_PACK: &str = "fr7.pack.jamming_tank";
/// The shipped line tank: the jamming design's source and the raider's hull.
const LINE_TANK: &str = "tacsim.unit.line_tank";
/// The stock rig design the casualty is (beat 3).
const PROSPECTOR: &str = "tacsim.unit.tv_prospector";
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

/// **Where the raider comes from on a ground**: its cell's offset from the ramp's, its heading,
/// and the bearing word its `+field/mark` names the rigs at the ramp by.
#[derive(Clone, Copy, Debug)]
struct Approach {
    from_ramp: (i64, i64),
    heading: u32,
    mark: &'static str,
}

/// The raider's approach on `ground`: sixteen cells due east of the ramp facing west, its `W`
/// (Urvara Reach); on Ahuna Flank the ramp's row is out of sight from the east (the raider saw
/// only the carrier there), so from the south, facing north, its `N`.
fn approach(ground: &Ground) -> Approach {
    if ground.side == 256 {
        Approach {
            from_ramp: (0, -12),
            heading: 0,
            mark: "N",
        }
    } else {
        Approach {
            from_ramp: (16, 0),
            heading: WEST,
            mark: "W",
        }
    }
}

/// **Where the story stands on `ground`**: off the carrier's stern ramp cell (its pad's cell less
/// four rows, `driving_rig::RAMP_CELL` on every site), the raider's hull by its [`approach`], so
/// the rigs that come down the ramp are the nearest flight on its mark's bearing; the wing three
/// cells west of the raider and three north; the casualty one cell east of the ramp, within a
/// cell of the rig that lands there (beat 3). Answers `(raider, wing, casualty)`.
fn stands(engine: &mush_core::Engine, ground: &Ground) -> [(i64, i64); 3] {
    let (x, y) = tacsim::surface::excursion::site_pad_cm(&engine.world, &cid(ground.entry));
    let cell = i64::from(tacsim::surface::ground::SITE_CELL_CM);
    let ramp = (x.div_euclid(cell), y.div_euclid(cell) - 4);
    let (dx, dy) = approach(ground).from_ramp;
    let raider = (ramp.0 + dx, ramp.1 + dy);
    [raider, (raider.0 - 3, raider.1 + 3), (ramp.0 + 1, ramp.1)]
}

/// **A hull of `design` on `ground` at `cell`, facing `heading`** (the fixture, as
/// `field_host_rig::armed_on`).
fn placed(
    engine: &mut mush_core::Engine,
    ground: &Ground,
    design: &str,
    name: &str,
    cell: (i64, i64),
    heading: u32,
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
        let pose =
            tacsim::surface::excursion::cell_pose([cell.0, cell.1], heading).expect("a pose");
        mush_sim::place(ctx, hull, mush_sim::DomainId(domain), pose).expect("on the ground");
        hull
    })
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
    let hull = placed(engine, ground, design, name, cell, approach(ground).heading);
    staff_hand(engine, |ctx| {
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

/// A hull's power destroyed, as a mutation plan's JSON (`field_fr5`'s `powerless`): a casualty a
/// tug can take.
fn powerless_plan() -> String {
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![mush_tactics::UnitMutationOp::SetComponentCondition {
            component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
            condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
        }],
        None,
        Some("bot-story:fr7-powerless".into()),
    )
    .expect("a plan");
    serde_json::to_string(&plan).expect("the plan's JSON")
}

/// The hulls the fixture made, and where the casualty lies.
#[derive(Clone, Copy, Debug)]
struct Hulls {
    raider: ObjId,
    wing: ObjId,
    casualty: ObjId,
    casualty_cell: (i64, i64),
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
    let [at_raider, at_wing, at_casualty] = stands(engine, ground);
    let hulls = Hulls {
        raider: armed_seated(engine, ground, LINE_TANK, "Basin Hound", at_raider, raider),
        wing: armed_seated(engine, ground, JAMMING_UNIT, "Basin Whelp", at_wing, wing),
        casualty: placed(
            engine,
            ground,
            PROSPECTOR,
            "Stranded Rig",
            at_casualty,
            WEST,
        ),
        casualty_cell: at_casualty,
    };
    let plan = worn_plan(engine, hulls.wing);
    said.push(mine::staff_line(
        engine,
        &format!("@unit/damage #{}={plan}", hulls.wing.0),
    ));
    said.push(mine::staff_line(
        engine,
        &format!("@unit/damage #{}={}", hulls.casualty.0, powerless_plan()),
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

/// **`unit`'s row in the field's last snapshot** (`UnitState`, as `tacsim::surface::tow::
/// field_stand` reads it in-process), read from the store: the snapshot's state rides its
/// document as one JSON string.
async fn snapshot_row(rig: &arena::Rig, ground: &Ground, unit: u64) -> Option<Value> {
    let snapshot = rig
        .store
        .module_json_get(
            tacsim::NS,
            &format!("tacsim:field:snapshot:{}", ground.entry),
        )
        .await
        .ok()
        .flatten()?;
    find_row(&snapshot, unit)
}

/// The walk: an object naming `unit` as its `id` with a `pose`; a long string that parses as
/// JSON (the snapshot's `state`) is walked too.
fn find_row(value: &Value, unit: u64) -> Option<Value> {
    match value {
        Value::Object(map) => {
            if map.get("id").and_then(Value::as_u64) == Some(unit) && map.contains_key("pose") {
                return Some(value.clone());
            }
            map.values().find_map(|inner| find_row(inner, unit))
        }
        Value::Array(rows) => rows.iter().find_map(|inner| find_row(inner, unit)),
        Value::String(text) if text.len() > 64 && text.starts_with('{') => {
            serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|inner| find_row(&inner, unit))
        }
        _ => None,
    }
}

/// **Whether the field's snapshot holds `unit` with no sight** (`UnitState::sensor` empty):
/// `Some(true)` for a dark sight, `Some(false)` for one that sees, `None` while no snapshot holds
/// the unit.
async fn sight_dark(rig: &arena::Rig, ground: &Ground, unit: u64) -> Option<bool> {
    snapshot_row(rig, ground, unit)
        .await
        .map(|row| row.get("sensor").is_none_or(Value::is_null))
}

/// **Where the field's snapshot stands `unit`, the tug it trails, and whether it has been
/// struck** (`field_stand`'s reading, plus `own_last_hit_ms`).
async fn tow_stand(
    rig: &arena::Rig,
    ground: &Ground,
    unit: u64,
) -> Option<((i64, i64), Option<u64>, bool)> {
    let row = snapshot_row(rig, ground, unit).await?;
    let cell = i64::from(tacsim::surface::ground::SITE_CELL_CM);
    let x = row["pose"]["x_cm"].as_i64()?.div_euclid(cell);
    let y = row["pose"]["y_cm"].as_i64()?.div_euclid(cell);
    Some((
        (x, y),
        row["coupled_to"].as_u64(),
        !row["own_last_hit_ms"].is_null(),
    ))
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
    lock_and_fire_nth(raider, party, 0, gun).await
}

/// The raider locks the `nth` contact of `party` (modulo how many it holds) and fires `gun` once;
/// whether a lock was had.
async fn lock_and_fire_nth(raider: &mut arena::Client, party: char, nth: usize, gun: &str) -> bool {
    let of_party: Vec<String> = contacts(raider)
        .await
        .into_iter()
        .filter(|(_, of)| of.eq_ignore_ascii_case(&party))
        .map(|(designator, _)| designator)
        .collect();
    let Some(designator) = of_party.get(nth % of_party.len().max(1)).cloned() else {
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
            // The step can open and go in one tick: its launch is the script's to type.
            if send.line.starts_with("+sortie/launch ") {
                println!(
                    "fr7: +{} s lane held back: {}",
                    started.elapsed().as_secs(),
                    send.line
                );
                return started.elapsed().as_secs();
            }
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

/// **Beat 3, its first half: the tow struck before the field** (doc 36 GC.4's open-ground tow,
/// carried into the field coupled by FR.5, Q-40-7).
///
/// The casualty is the fixture's stranded rig, its power destroyed at boot and indexed on the
/// site's record as a casualty by staff's `@tacsim/site <site> leave #<unit> casualty <x>,<y>`
/// (GC.5's fixture, `field_fr5`'s precedent: the field that disabled it would have written that
/// row; no player path disables a rig outside a field). Then the captain's `+tv/1 tow go` (the
/// bot's GC.4 tow line) takes it in tow with rig 1, which came down the ramp a cell east of it,
/// read through the bot's `lines::tow_line` as attached. Answers what it read.
async fn tow_struck_before_field(
    god: &mut arena::Client,
    cap: &mut arena::Client,
    ground: &Ground,
    hulls: &Hulls,
) -> String {
    let (x, y) = hulls.casualty_cell;
    let (_, left) = god
        .answer(
            &format!(
                "@tacsim/site {} leave #{} casualty {x},{y}",
                ground.entry, hulls.casualty.0
            ),
            &[
                "Site record write requested",
                "Usage",
                "not a site",
                "has no",
            ],
        )
        .await;
    println!("fr7: god> @tacsim/site leave\n{}", left.trim_end());
    assert!(
        left.contains("Site record write requested"),
        "staff index the casualty:\n{left}"
    );
    god.idle(Duration::from_secs(3)).await;
    let tow = "+tv/1 tow go";
    assert!(escort::in_vocabulary(tow), "the bot's tow line: {tow}");
    let from = cap.seen.len();
    cap.send(tow).await;
    let mut attached = None;
    for _ in 0..90 {
        cap.idle(Duration::from_secs(1)).await;
        let since = cap.seen.get(from..).unwrap_or("").to_owned();
        let read: Vec<lines::TowLine> = since.lines().filter_map(lines::tow_line).collect();
        if read.contains(&lines::TowLine::Attached) {
            attached = Some(since);
            break;
        }
        assert!(
            !read
                .iter()
                .any(|line| matches!(line, lines::TowLine::Refused | lines::TowLine::Nothing)),
            "the tow is refused:\n{since}"
        );
    }
    let since = attached.unwrap_or_else(|| {
        panic!(
            "rig 1 takes the casualty in tow:\n{}",
            cap.seen.get(from..).unwrap_or("")
        )
    });
    println!("fr7: captain> {tow}\n{}", since.trim_end());
    let line = since
        .lines()
        .find(|line| lines::tow_line(line) == Some(lines::TowLine::Attached))
        .unwrap_or_default()
        .trim()
        .to_owned();
    println!("fr7: {} beat 3a: {line}", ground.venue);
    line
}

/// **Beat 3, its second half: the casualty struck while towed, then released under fire.**
///
/// The field admits the pair coupled: its snapshot stands the casualty `coupled_to` rig 1
/// (Q-40-7). The raider locks Concord's contacts in turn and fires until the snapshot reads the
/// casualty hit while still coupled; then, with one more burst in the air, the captain types
/// `+tv/1 release go` (`field::round_word`), and the snapshot stands the casualty out of tow
/// where it was let go. Answers what the captain read.
#[allow(clippy::too_many_arguments)] // the one beat's cast
async fn tow_released_under_fire(
    rig: &arena::Rig,
    raider: &mut arena::Client,
    cap: &mut arena::Client,
    ground: &Ground,
    hulls: &Hulls,
    tug: u64,
    concord: char,
    gun: &str,
) -> String {
    let casualty = u64::from(hulls.casualty.0);
    let mut coupled = None;
    for _ in 0..60 {
        if let Some(stand) = tow_stand(rig, ground, casualty).await
            && stand.1 == Some(tug)
        {
            coupled = Some(stand);
            break;
        }
        raider.idle(Duration::from_secs(1)).await;
    }
    let coupled = coupled.unwrap_or_else(|| {
        panic!("the field admits the casualty coupled to rig 1 #{tug} (Q-40-7)")
    });
    println!(
        "fr7: {} beat 3b: the casualty #{casualty} stands at {:?} in tow of #{tug}",
        ground.venue, coupled.0
    );
    let mut struck = None;
    for nth in 0..120 {
        let _ = lock_and_fire_nth(raider, concord, nth, gun).await;
        if let Some(stand) = tow_stand(rig, ground, casualty).await
            && stand.2
        {
            struck = Some(stand);
            break;
        }
        raider.idle(Duration::from_secs(1)).await;
    }
    let struck = struck.expect("the raider's fire strikes the casualty in tow");
    assert_eq!(struck.1, Some(tug), "struck while towed: {struck:?}");
    println!(
        "fr7: {} beat 3b: the casualty is struck in tow at {:?}",
        ground.venue, struck.0
    );
    // Under fire: one more burst, then the release.
    let _ = lock_and_fire(raider, concord, gun).await;
    let (said, text) = captain_says(
        cap,
        "+tv/1 release go",
        &["lets go of the", "could not let go", "busy", "Usage"],
    )
    .await;
    assert_eq!(
        said,
        Some(0),
        "the release is ordered in the field:\n{text}"
    );
    let mut released = None;
    for _ in 0..60 {
        if let Some(stand) = tow_stand(rig, ground, casualty).await
            && stand.1.is_none()
        {
            released = Some(stand);
            break;
        }
        cap.idle(Duration::from_secs(1)).await;
    }
    let released = released.expect("the snapshot stands the casualty out of tow");
    let moved = (released.0.0 - struck.0.0)
        .abs()
        .max((released.0.1 - struck.0.1).abs());
    assert!(
        moved <= 2,
        "the casualty stands where it was let go: struck at {:?}, released at {:?}",
        struck.0,
        released.0
    );
    println!(
        "fr7: {} beat 3b: released at {:?}",
        ground.venue, released.0
    );
    text.lines()
        .find(|line| line.contains("lets go of the"))
        .unwrap_or_default()
        .trim()
        .to_owned()
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
            cfg.tacsim.arena.snapshot_ms = 2_000;
        },
        |engine| {
            // Beat 6 (FR.7c): the black box below the boot's floors, through the test-only seam
            // on the booted engine; no shipped config reaches it.
            assert!(
                tacsim::store::lower_ledger_floor_for_test(
                    &mut engine.world,
                    MAX_BYTES,
                    SUMMARY_MS
                ),
                "the layer is booted"
            );
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

    // Beat 3a: the tow struck before the field (rig 1 takes the stranded rig in tow).
    let tow_a = tow_struck_before_field(&mut god, &mut cap, ground, &hulls).await;

    let mark = format!("+field/mark {}=hostile", approach(ground).mark);
    let text = pilot_reads("raider", &mut raider, &mark).await;
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
    assert!(
        party_of(&opened, rig2).is_none(),
        "rig 2 is still aboard when the field opens: {opened}"
    );
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

    // -- Beat 3b: the casualty struck while towed, then released under fire (FR.5b) -------
    // Before the reinforcement: once rig 2 flies, the captain sits at its helm and `+tv/1` names
    // no rig of theirs (the helm's own rig is the only one a slot can name).
    let tow_b = tow_released_under_fire(
        &rig,
        &mut raider,
        &mut cap,
        ground,
        &hulls,
        rig1,
        concord_letter,
        &gun,
    )
    .await;

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
        // The field's idle bound is long: it holds with no round fired while rig 2 comes down.
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
        "fr7: {} green in {} s: raid, reinforcement, tow ({tow_a} / {tow_b}), sight jammed, void, segment {rolled}",
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
#[ignore = "doc 40 cut FR.7's round story at 256: the bot over SSH, about ten minutes; run by name"]
async fn fr7_the_rounds_story_at_256() {
    tokio::time::timeout(Duration::from_secs(30 * 60), round_story(&AHUNA_FLANK))
        .await
        .expect("the round's story at 256 finishes inside 30 minutes");
}

/// **FR.7 at 512**: Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "doc 40 cut FR.7's round story at 512: the bot over SSH, about ten minutes; run by name"]
async fn fr7_the_rounds_story_at_512() {
    tokio::time::timeout(Duration::from_secs(30 * 60), round_story(&URVARA_REACH))
        .await
        .expect("the round's story at 512 finishes inside 30 minutes");
}
