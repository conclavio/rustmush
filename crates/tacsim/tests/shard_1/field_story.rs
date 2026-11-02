//! **Doc 32 cut CF.7 — the field story** (Q-CF-47), over the composed world (`driving`'s keyed
//! rig on Ahuna Flank) with the arena's egress pumped by hand, as CF.2's to CF.6's gates pump it.
//!
//! **The whole of Q-CF-47 at 256 a side** (doc 32 §17), through the production command paths, in
//! one story: the mining roster's block, its two-hull escort and a bystander from another captain;
//! the raider launched from its fixture carrier and seen on the presence picture before it
//! declares; the promotion with everyone at their poses; the raiders on the miner's quarry screen
//! and the worked block on the escort's map (G-7); the bystander fired on, hostile to that party
//! only; the escort's fight while the miners hold; an escort mission-killed by an engine hit and a
//! raider by a control-section hit (G-6); the raiders home up their ramp, the barge's lift and the
//! demotion with no Friendly mark (doc 36 cut GC.0); the fill plan resumed; the casualty and the
//! wreck on the site record; a restart; the carrier's departure on the leave-behind confirmation;
//! a second visit finding both where they fell; and a second field on the same ground, the barge
//! landed again, where the wreck's cover is pinned at its gap (G-8).
//!
//! What the story stands in for, each named in doc 32 §17:
//! - **the escort's delivered orders**: tacsim's world harness lands no DT activation (CF.6c's
//!   operational loan fails here the same way), so one escort hull is flown by a player and the
//!   other by a hired NPC on doctrine. The delivered orders an escort fights under are CF.6's
//!   transport gate's and CF-5b's, in the split-boundary harness;
//! - **the player escort's hull is the miner's second leased rig rebuilt as a line tank** by a
//!   staff hand (`@unit/remove`, `@unit/create`), as CF-5b's gate rebuilds one: the miner's
//!   funds do not cover a prospector and the escort lease both, and the rig keeps the miner as its
//!   operator, which is what puts the miner's chart on the escort's map (G-7);
//! - **the hits are seeded** (G-6): the armour and sections of the two hulls are set by a staff
//!   hand before the field opens, so the real damage pipeline lands the two named hits;
//! - **the mining roster is a player's rig working a block by hand** (`+tv/1 mine go`).

use mush_core::{GOD, ObjId, ObjKind};
use serde_json::{Value, json};
use smol_str::SmolStr;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::party::Relation;
use tacsim::arena::state::Classification;
use tacsim::surface::site_record::LeftCondition;

use crate::driving::{
    AHUNA, RAMP, Rig, act_go, at_counter, cid, command, finish, land, launched, launched_at,
    map_frame, on_pad, on_pad_as, on_pad_as_at, on_pad_with_at, pilot, ramp_at, reboot, run,
    site_domain, staff_hand, stocked, stocked_tuned, to_cell_on, warm,
};
use crate::field_host::{
    armed, armed_on, egress, frame, seat, second, site_record_at, task, until,
};
use crate::field_raiders::{SKULKS, raider_hulls, seat_npc};

/// A site cell and a heading, millidegrees.
type Placed = ((i64, i64), u32);

/// **The ground one run of the story stands on** (doc 32 cut S.2, Q-CF-51: the story runs once
/// at 256 a side and again at 512, one script over both): the site, its raider, and where the
/// story puts everyone. Every other number in the story is the site's own (its pad, its ramp, its
/// record) or the field's.
struct Ground {
    /// The site's port entry.
    entry: &'static str,
    /// Its `+ship/undock` word.
    venue: &'static str,
    /// The content's raider seeded on it.
    raider: &'static str,
    /// The site's side, cells.
    side: i64,
    /// The bystander's rig, in the next block.
    bystander: (i64, i64),
    /// Where the miner's rig scans and sounds.
    read: (i64, i64),
    /// The unread pocket it cores.
    core: (i64, i64),
    /// The player escort's hull, facing away from the raider's tank (G-6's engine hit).
    escort: Placed,
    /// The hired NPC's hull.
    ward: Placed,
    /// The raider's walker, in the escort's line.
    walker: Placed,
    /// Where a staff hand stands the raider's tank before it declares, or `None` to leave it
    /// where it launched.
    tank: Option<Placed>,
    /// The bearing the raider's leader declares on (the bystander's, from the leader's units).
    declare: &'static str,
    /// The tank's stand in the second field, past the wreck on the escort's row.
    second: Placed,
    /// The bearing it declares on the escort's hull by.
    second_mark: &'static str,
}

/// **Ahuna Flank at 256 a side** (doc 32 §17): everything within 32 cells of the pad, the
/// raider's carrier to the east at the pad's cell plus `(28, -2)`.
const AHUNA_FLANK: Ground = Ground {
    entry: AHUNA,
    venue: "ahuna",
    raider: SKULKS,
    side: 256,
    bystander: (140, 136),
    read: (140, 120),
    core: (136, 120),
    escort: ((150, 126), 270_000),
    ward: ((146, 120), 90_000),
    walker: ((153, 126), 270_000),
    tank: None,
    declare: "NW",
    second: ((160, 126), 270_000),
    second_mark: "W",
};

/// **Urvara Reach at 512 a side** (doc 32 §20): the pad at `(256, 256)`, the middle of the site.
/// The miner works its own block south-west of the pad, below cell 256 on both axes; the
/// bystander's block and the raider's carrier (the pad's cell plus `(40, 20)`, its ramp at
/// `(299, 276)`) are in the far quadrant, past 256 on both. The fight is on row 268 across the
/// 256 line: the tank stands west of it at `(250, 268)` and fires east through the escort's rear
/// plate, the walker and the escort east of it; the tank's withdrawal to its ramp crosses back.
const URVARA_REACH: Ground = Ground {
    entry: "urvara_port",
    venue: "urvara",
    raider: "tacsim.raider.urvara_reavers",
    side: 512,
    bystander: (282, 294),
    read: (242, 246),
    core: (238, 246),
    escort: ((262, 268), 90_000),
    ward: ((258, 262), 270_000),
    walker: ((258, 268), 90_000),
    tank: Some(((250, 268), 90_000)),
    declare: "NW",
    second: ((250, 268), 90_000),
    second_mark: "E",
};

impl Ground {
    /// The `(venue, entry)` the pad helpers take.
    fn site(&self) -> (&'static str, &'static str) {
        (self.venue, self.entry)
    }
}

/// The rows of `who`'s site map's presence line (`Presence: E within 200 m; SW 200-500 m.`),
/// read from the frame, where the printed map would wrap it.
fn presence_rows(rig: &mut Rig, who: ObjId) -> (Vec<String>, String) {
    let frame = map_frame(rig, who, json!({ "view": "site" }));
    let line = frame["site"]["presence_line"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let rows = line
        .strip_prefix("Presence: ")
        .map(|rows| {
            rows.trim_end_matches('.')
                .split("; ")
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    (rows, line)
}

/// **The `arena.cockpit` keyframe `who` gets** through the feed's own query (CF.4's gate).
fn cockpit(rig: &mut Rig, who: ObjId) -> Value {
    panel(rig, who, tacsim::arena_bridge::FRAME_FEED, json!({}))
}

/// **The keyframe `who` gets from the panel feed `name`** through its own query, `args` the
/// feed's params.
fn panel(rig: &mut Rig, who: ObjId, name: &str, args: Value) -> Value {
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    rig.tw.engine_mut().step(
        mush_core::Inbound::Query {
            conn: 7,
            actor: who,
            id: 1,
            name: SmolStr::new(name),
            args,
        },
        &mut out,
        &mut fx,
    );
    out.events()
        .iter()
        .find_map(|(_, event)| match event {
            mush_proto::OutputEvent::Reply { event, .. } => match event.as_ref() {
                mush_proto::OutputEvent::Panel { data, .. } => Some(data.clone()),
                _ => None,
            },
            _ => None,
        })
        .unwrap_or(Value::Null)
}

/// The campaign marks of `who`'s cockpit frame, `[x, y, glyph]`.
fn campaign_marks(rig: &mut Rig, who: ObjId) -> std::collections::BTreeSet<String> {
    cockpit(rig, who)["campaign"]["marks"]
        .as_array()
        .map(|marks| marks.iter().map(Value::to_string).collect())
        .unwrap_or_default()
}

/// **Set `unit`'s cells by a staff hand** (G-6's seeding, before the field opens): `pick` names
/// the value for a cell from its address and its `(min, max)`, or leaves it.
fn seed_cells(
    rig: &mut Rig,
    unit: ObjId,
    cause: &'static str,
    pick: impl Fn(&mush_tactics::CellAddr, i64, i64) -> Option<i64>,
) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let ops: Vec<mush_tactics::UnitMutationOp> = cells
            .cells()
            .filter_map(|(addr, cell)| {
                let value = pick(addr, cell.min, cell.max)?;
                (value != cell.current).then(|| mush_tactics::UnitMutationOp::SetCell {
                    addr: addr.clone(),
                    value,
                })
            })
            .collect();
        assert!(!ops.is_empty(), "the seeding moves a cell of #{}", unit.0);
        let plan = mush_tactics::UnitMutationPlan::new(ops, None, Some(SmolStr::new_static(cause)))
            .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// The section a cell address names, and the cell's id.
fn section_cell(addr: &mush_tactics::CellAddr) -> Option<(&str, Option<&str>, &str)> {
    match addr {
        mush_tactics::CellAddr::Section { section, cell } => {
            Some((section.as_str(), None, cell.as_str()))
        }
        mush_tactics::CellAddr::Facet {
            section,
            facet,
            cell,
        } => Some((section.as_str(), Some(facet.as_str()), cell.as_str())),
        _ => None,
    }
}

/// Whether a component cell is a weapon's magazine.
fn magazine(addr: &mush_tactics::CellAddr) -> Option<&str> {
    match addr {
        mush_tactics::CellAddr::Component { component, cell }
            if cell.as_str() == tacsim::definitions::unit::AMMO_CELL =>
        {
            Some(component.as_str())
        }
        _ => None,
    }
}

/// **Rebuild `unit` as `design`** by staff's own commands (CF-5b's idiom); `marker` is a section
/// only the new design has.
fn rebuild(rig: &mut Rig, unit: ObjId, design: &str, marker: &str) {
    let removed = run(&mut rig.tw, GOD, &format!("@unit/remove #{}", unit.0));
    let created = run(
        &mut rig.tw,
        GOD,
        &format!("@unit/create #{}={design}", unit.0),
    );
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), unit)
            .is_some_and(|cells| cells.sections().any(|(id, _)| id.as_str() == marker)),
        "#{} is rebuilt as {design}: {removed} / {created}",
        unit.0
    );
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

/// The site cell `unit` stands on, by its surface placement.
fn cell_of(rig: &Rig, unit: ObjId) -> Option<(i64, i64)> {
    mush_sim::pose_of(rig.tw.world(), unit)
        .map(|(_, pose)| tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
}

/// Whether `lines` hold a shot by `who` (at `target`, when one is named).
fn fired(lines: &[LedgerEvent], who: ObjId, target: Option<ObjId>) -> bool {
    lines.iter().any(|line| {
        matches!(line.kind, EventKind::Fired { unit, target: at, .. }
            if unit == who && target.is_none_or(|want| want == at))
    })
}

/// Who destroyed `section` of `unit`, when the ledger says it fell.
fn destroyed_by(lines: &[LedgerEvent], unit: ObjId, section: &str) -> Option<Option<ObjId>> {
    lines.iter().find_map(|line| match &line.kind {
        EventKind::SectionDestroyed {
            unit: struck,
            section: gone,
            by,
            ..
        } if *struck == unit && gone.as_str() == section => Some(*by),
        _ => None,
    })
}

/// **What the ledger says happened to and by `units`**, for a failing assert: each one's shots
/// by target, the sections its hits found, the sections it lost, and every miss reason.
fn damage_summary(lines: &[LedgerEvent], units: &[ObjId]) -> String {
    use std::collections::BTreeMap;
    let mut shots: BTreeMap<(u32, u32), usize> = BTreeMap::new();
    let mut hits: BTreeMap<(u32, String), usize> = BTreeMap::new();
    let mut lost: Vec<(u32, String)> = Vec::new();
    let mut misses: BTreeMap<String, usize> = BTreeMap::new();
    for line in lines {
        match &line.kind {
            EventKind::Fired { unit, target, .. } if units.contains(unit) => {
                *shots.entry((unit.0, target.0)).or_default() += 1;
            }
            EventKind::Hit {
                target, section, ..
            } if units.contains(target) => {
                *hits.entry((target.0, section.to_string())).or_default() += 1;
            }
            EventKind::SectionDestroyed { unit, section, .. } if units.contains(unit) => {
                lost.push((unit.0, section.to_string()));
            }
            EventKind::Impact { unit, target, .. } if units.contains(unit) => {
                *misses
                    .entry(format!("impact {}>{}", unit.0, target.0))
                    .or_default() += 1;
            }
            EventKind::Miss { reason, .. } => *misses.entry(reason.to_string()).or_default() += 1,
            other => {
                let debug = format!("{other:?}");
                if units.iter().any(|unit| debug.contains(&unit.0.to_string())) {
                    let word = debug
                        .split(|c: char| !c.is_ascii_alphanumeric())
                        .next()
                        .unwrap_or_default()
                        .to_owned();
                    *misses.entry(format!("kind {word}")).or_default() += 1;
                }
            }
        }
    }
    format!("shots {shots:?}; hits {hits:?}; lost {lost:?}; misses {misses:?}")
}

/// The contact on `who`'s frame standing on `at`.
fn contact_at(
    rig: &Rig,
    who: ObjId,
    (cx, cy): (i64, i64),
) -> Option<tacsim::arena::perception::FrameContact> {
    frame(rig, who).and_then(|frame| {
        frame
            .contacts
            .iter()
            .find(|contact| {
                contact.cell.is_some_and(|(x, y)| {
                    i64::from(x).abs_diff(cx) == 0 && i64::from(y).abs_diff(cy) == 0
                })
            })
            .cloned()
    })
}

/// The live record's party letter of `unit` in the field open on `entry`.
fn letter_of(rig: &Rig, entry: &str, unit: ObjId) -> char {
    let Some(live) = tacsim::field::live_on(rig.tw.world(), &cid(entry)) else {
        panic!(
            "no field is open on {entry} to name #{}'s party (the field task {:?}): the \
             field demoted, so a hostile it held is dead or gone",
            unit.0,
            task(rig)
        );
    };
    live.party_of(unit).map_or_else(
        || {
            panic!(
                "#{} is not admitted to the open field (the field task {:?})",
                unit.0,
                task(rig)
            )
        },
        |party| party.letter(),
    )
}

/// `who` types `line`, and anything it submits lands.
fn typed(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    for submit in &queued {
        land(rig, submit).unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    out
}

/// **The field story** (doc 32 Q-CF-47), the whole of it at 256 a side on Ahuna Flank.
///
/// **Doc 36 cut GC.0's gate** (Q-36-6, the host half): this is CF.7's story at 256 a side
/// (`cf7_the_field_story` until GC.0), with the escort pilot's Friendly stand-down removed. The
/// raider loses, its surviving hull boards, its barge lifts, and the field demotes on its own.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc0_the_raiders_barge_lifts_and_the_field_demotes_without_a_friendly_mark() {
    field_story(&AHUNA_FLANK).await;
}

/// **Doc 32 cut S.2: the field story at 512 a side** on Urvara Reach (Q-CF-51), the same script
/// as [`gc0_the_raiders_barge_lifts_and_the_field_demotes_without_a_friendly_mark`] over the far
/// quadrant's ground.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s2_the_field_story_at_512() {
    field_story(&URVARA_REACH).await;
}

/// **The field story** (doc 32 Q-CF-47) on `at`'s ground.
#[allow(clippy::too_many_lines)]
async fn field_story(at: &Ground) {
    let wide = at.side > 256;
    let mut rig = stocked_tuned(|cfg| {
        crate::field_host::tune(cfg);
        cfg.field.disengage_s = 15;
    });

    // (1) At a named frontier site — Ahuna Flank, or Urvara Reach — a mining roster works a
    // block with an escort of two armed hulls; a bystander rig from another captain works the
    // next block.
    let (captain, _, captain_tvs) =
        on_pad_as_at(&mut rig, ("Other", 2, "Tally Moth"), at.site(), 1);
    let bystander = captain_tvs[0];
    launched_at(&mut rig, at.entry, captain, bystander);
    to_cell_on(&mut rig, at.entry, bystander, at.bystander);
    // The miner leases a prospector and Occator's escort line tank (CF.6c) at Ceres. A new
    // captain's stake covers one or the other, so a staff hand moves 7,000cr from the other
    // captain's account first (`@econ/pay`).
    let (miner, carrier_row, miner_tvs) = on_pad_with_at(
        &mut rig,
        ("Pilot", 1, "Salt Moth"),
        at.site(),
        |rig, miner| {
            let account = |rig: &Rig, who: ObjId| {
                tacsim::port::credit::own_account(rig.tw.world(), who)
                    .expect("an account")
                    .id
                    .0
            };
            let (from, to) = (account(rig, captain), account(rig, miner));
            let paid = run(&mut rig.tw, GOD, &format!("@econ/pay #{from}=#{to} 700000"));
            at_counter(rig, miner);
            let prospector = run(&mut rig.tw, miner, "+tv/lease go");
            let escort = run(&mut rig.tw, miner, "+tv/lease escort go");
            assert!(
                prospector.contains("leases you OWP-TV") && escort.contains("leases you OWP-ES"),
                "the miner leases both: {paid} / {prospector} / {escort}"
            );
        },
    );
    let carrier = ObjId(carrier_row.hull);
    assert_eq!(miner_tvs.len(), 2, "a prospector and an escort in the bay");
    let (worker, leased) = (miner_tvs[0], miner_tvs[1]);
    launched_at(&mut rig, at.entry, miner, worker);
    // A second rig down the ramp while the first still stands on its cell: the surface lets the
    // two share it, and the field parts them at admission (doc 32 §17). Rig 2 is stood out on
    // the ground below before any field, so the story's own admission parts nobody.
    let _ = act_go(&mut rig, miner, "+tv/2 launch go");
    let _ = finish(&mut rig, leased, miner);
    let ramp = {
        let (x, y, _) = ramp_at(&rig.tw, at.entry);
        tacsim::surface::excursion::site_cell(x, y)
    };
    assert_eq!(
        (cell_of(&rig, worker), cell_of(&rig, leased)),
        (Some(ramp), Some(ramp)),
        "both rigs stand on the ramp's cell"
    );
    // The miner's own readings: a scan and a sounding of the ground the rig works (this pocket
    // reads barren), then the rig moves one pocket west to core an unread one.
    to_cell_on(&mut rig, at.entry, worker, at.read);
    for act in ["+tv/1 scan go", "+tv/1 sound go"] {
        let _ = act_go(&mut rig, miner, act);
        let _ = finish(&mut rig, worker, miner);
    }
    to_cell_on(&mut rig, at.entry, worker, at.core);
    // The escort: the miner's leased line tank, flown by a player escort who never sounded
    // anything; and a fixture hull a hired NPC flies on doctrine.
    stand(&mut rig, at.entry, leased, at.escort);
    let escort = pilot(&mut rig, "Escort", 7);
    let ward = armed_on(&mut rig, at.entry, "Ward Dog", at.ward.0, at.ward.1);
    let hired = staff_hand(&mut rig, |ctx| {
        ctx.w
            .create(ObjKind::Npc, "Hired escort", GOD, None)
            .expect("the NPC")
    });
    seat_npc(&mut rig, ward, hired);
    // G-6's seeding of the escort: its hull's rear plate at its floor and its engine one point
    // from destroyed, so a round struck through the rear plate breaks the engine.
    seed_cells(
        &mut rig,
        leased,
        "test:cf7-engine",
        |addr, min, _| match section_cell(addr)? {
            ("hull", Some("rear"), cell) if cell == tacsim::definitions::unit::ARMOR_CELL => {
                Some(min)
            }
            ("engine", None, cell) if cell == tacsim::definitions::unit::INTERNALS_CELL => {
                Some(min + 1)
            }
            _ => None,
        },
    );
    seat(&mut rig, worker, miner);
    seat(&mut rig, leased, escort);
    seat(&mut rig, bystander, captain);
    warm(&mut rig);
    for (pilot, slot) in [(miner, 1), (captain, 1)] {
        let (out, queued) = command(&mut rig, pilot, &format!("+tv/{slot} mine go"));
        assert_eq!(queued.len(), 1, "#{}'s mine run starts: {out}", pilot.0);
        land(&mut rig, &queued[0]).expect("the run's start lands");
    }
    // Doc 32 §19 (cut S.1; §17's G-6): the bystander must outlive the raider's fire through
    // steps 5 and 6 however it comes — one burst from the raider's mind or a run of bursts from
    // its condition slot, which draw order alone decides. A prospector's chassis holds about
    // 150 points against ten coaxial bursts' 165, so once its mine run has started its hull is
    // rebuilt as the escort's armoured design with sensors (CF.6c's `tv_escort`), its magazines
    // seeded empty so the bystander still never fires ("the miners hold").
    rebuild(&mut rig, bystander, "tacsim.unit.tv_escort", "turret");
    seed_cells(
        &mut rig,
        bystander,
        "test:cf7-bystander-unarmed",
        |addr, min, _| magazine(addr).map(|_| min),
    );
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // (2) A raider party launches from its fixture carrier (doc 32 cut CF.6b) and is seen on the
    // presence picture (Q-CF-15) before it declares.
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {}", at.raider));
    assert!(seeded.contains("seeded"), "{seeded}");
    for _ in 0..2 {
        let _ = second(&mut rig);
    }
    let (before, _) = presence_rows(&mut rig, miner);
    let launched_out = run(
        &mut rig.tw,
        GOD,
        &format!("@field/raiders/launch {}", at.raider),
    );
    assert!(launched_out.contains("launch from"), "{launched_out}");
    let raiders = raider_hulls(&rig);
    assert_eq!(raiders.len(), 2, "the raider's two hulls stand");
    let barge = tacsim::field::raiders::carrier_of(rig.tw.world(), raiders[0]).expect("a carrier");
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), raiders[0]).expect("a leader");
    for _ in 0..2 {
        let _ = second(&mut rig);
    }
    let (rows, picture) = presence_rows(&mut rig, miner);
    let from = cell_of(&rig, worker).expect("the miner's rig stands on the ground");
    for hull in &raiders {
        let flight = tacsim::surface::presence::Flight {
            tv: *hull,
            cell: cell_of(&rig, *hull).expect("the raider hull stands on the ground"),
            class: tacsim::surface::presence::hull_class(rig.tw.world(), *hull),
        };
        let seen = tacsim::surface::presence::seen_from(
            &[flight],
            &std::collections::BTreeSet::new(),
            &[(1, from), (2, cell_of(&rig, leased).expect("rig 2"))],
            from,
        );
        let row = seen.first().map(tacsim::surface::presence::Seen::words);
        assert!(
            row.as_ref().is_some_and(|row| rows.contains(row)),
            "the raider hull #{} is on the miner's presence picture as {row:?}: {picture}",
            hull.0
        );
    }
    assert_eq!(
        rows.len(),
        before.len() + raiders.len(),
        "one new row a raider hull: before {before:?}, after {rows:?}"
    );
    // G-6's seeding of the raider (doc 32 §17): its second hull is the party's walker, rebuilt
    // as a biped by staff, every section but its head at its internals' floor (a round finds
    // nothing there to break) and its head's plates at their floor with one point of internals
    // left, so the first round its head draws is the control-section hit. Its magazines and the
    // tank's main gun are empty, so the raider fires its coaxial alone. It stands in the escort's
    // line, between the escort and the ramp.
    let (tank, walker) = (raiders[0], raiders[1]);
    rebuild(&mut rig, walker, "tacsim.unit.biped_mk1", "head");
    stand(&mut rig, at.entry, walker, at.walker);
    seed_cells(&mut rig, walker, "test:cf7-head", |addr, min, _| {
        if magazine(addr).is_some() {
            return Some(min);
        }
        match section_cell(addr)? {
            ("head", Some(_), cell) if cell == tacsim::definitions::unit::ARMOR_CELL => Some(min),
            ("head", None, cell) if cell == tacsim::definitions::unit::INTERNALS_CELL => {
                Some(min + 1)
            }
            (_, None, cell) if cell == tacsim::definitions::unit::INTERNALS_CELL => Some(min),
            _ => None,
        }
    });
    // Its legs are recorded destroyed, so it stands where it was put (the kernel reads a
    // destroyed section's consequences at check-out: both legs gone disables motion).
    staff_hand(&mut rig, |ctx| {
        let ops = ["left_leg", "right_leg"]
            .into_iter()
            .map(|leg| mush_tactics::UnitMutationOp::SetSectionCondition {
                section: mush_tactics::SectionId::new(leg).expect("a section id"),
                condition: mush_tactics::ConditionId::new(tacsim::arena::damage::DESTROYED)
                    .expect("a condition id"),
            })
            .collect();
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(SmolStr::new_static("test:cf7-legs")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, walker, &plan).expect("applied");
    });
    seed_cells(&mut rig, tank, "test:cf7-coaxial", |addr, min, _| {
        (magazine(addr) == Some("cannon")).then_some(min)
    });
    // At 512 (doc 32 §20) the tank is stood across the 256 line from its carrier, west of the
    // escort, so its fire crosses the line and its withdrawal drives back over it.
    if let Some(placed) = at.tank {
        stand(&mut rig, at.entry, tank, placed);
    }
    if wide {
        let line = at.side / 2;
        let carrier_at = cell_of(&rig, barge).expect("the raider's carrier stands");
        for (what, cell) in [
            ("the raider's carrier", carrier_at),
            ("the bystander", at.bystander),
        ] {
            assert!(
                cell.0 > line && cell.1 > line,
                "{what} stands in the far quadrant, past {line} on both axes: {cell:?}"
            );
        }
        assert!(
            at.core.0 < line && at.core.1 < line,
            "the miner works its own block below {line}"
        );
    }
    for _ in 0..2 {
        let _ = second(&mut rig);
    }

    println!("S2 {} presence at the raider's launch: {rows:?}", at.side);

    // (3) It declares on the bystander (NW of its ramp) and the site promotes with everyone at
    // their poses.
    let declared = std::time::Instant::now();
    let out = typed(
        &mut rig,
        leader,
        &format!("+field/mark {}=hostile", at.declare),
    );
    assert!(
        site_record_at(&mut rig, at.entry).is_some_and(|record| record.open_field().is_some()),
        "the declaration promotes the site: {out}"
    );
    let promoted_ms = declared.elapsed().as_secs_f64() * 1_000.0;
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let opened_ms = declared.elapsed().as_secs_f64() * 1_000.0;
    let live = tacsim::field::live_on(rig.tw.world(), &cid(at.entry)).expect("the live record");
    for unit in [carrier, worker, leased, bystander, ward, barge]
        .into_iter()
        .chain(raiders.iter().copied())
    {
        assert!(
            live.units.iter().any(|row| row.unit == unit),
            "#{} is admitted at its pose",
            unit.0
        );
    }
    let raider_party = live.party_of(tank).expect("the raider's party");
    let bystander_party = live.party_of(bystander).expect("the bystander's party");
    // The raider's leader holds a standing Hostile mark on everyone (CF.6): it clears every mark
    // but the bystander's, so the bystander is the one it fires on.
    for party in &live.parties {
        if party.party != raider_party && party.party != bystander_party {
            let said = typed(
                &mut rig,
                leader,
                &format!("+field/mark {}=clear", party.party.letter()),
            );
            assert!(said.contains("unmarked"), "{said}");
        }
    }

    // (4) The miners see the raiders as contacts on the quarry screen (Q-CF-48), and the escort
    // sees the worked block on its map (G-7, doc 32 §17): the miner's readings, on the map of a
    // player escort who never sounded, because the escort's rig is the miner's (its operator).
    // The bystander, another captain, sees none of them.
    let mut readings = std::collections::BTreeSet::new();
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
            let held =
                map_frame(rig, miner, json!({ "view": "site" }))["site"]["field_layer"]["held"]
                    .as_u64()
                    .is_some_and(|held| held > 0);
            let own = campaign_marks(rig, miner);
            let other = campaign_marks(rig, captain);
            readings = own.difference(&other).cloned().collect();
            held && !readings.is_empty() && frame(rig, hired).is_some()
        })
        .await,
        "the miner's quarry screen holds the raiders and its own readings: {readings:?}"
    );
    let escort_map = campaign_marks(&mut rig, escort);
    let missing: Vec<&String> = readings.difference(&escort_map).collect();
    assert!(
        missing.is_empty(),
        "the escort's map carries the miner's worked block; missing {missing:?}"
    );
    // Doc 32 §20's field figures: the admission, and the frames the field's pictures draw (the
    // escort's `arena.map`, focused on its cell and asked for the whole site; the cockpit
    // keyframes; the quarry screen's `site.map`), each in canonical bytes.
    let bytes = |value: &Value| serde_json::to_vec(value).map_or(0, |bytes| bytes.len());
    let side = u16::try_from(at.side).expect("a side");
    let focused = panel(&mut rig, escort, tacsim::arena_bridge::MAP_FEED, json!({}));
    let whole = panel(
        &mut rig,
        escort,
        tacsim::arena_bridge::MAP_FEED,
        json!({ "window": [0, 0, side, side] }),
    );
    let figures = [
        ("arena.map focused", bytes(&focused)),
        ("arena.map whole-site window", bytes(&whole)),
        ("the escort's cockpit", bytes(&cockpit(&mut rig, escort))),
        ("the miner's cockpit", bytes(&cockpit(&mut rig, miner))),
        (
            "the miner's site.map",
            bytes(&map_frame(&mut rig, miner, json!({ "view": "site" }))),
        ),
        (
            "the miner's drive view",
            bytes(&map_frame(&mut rig, miner, json!({}))),
        ),
    ];
    println!(
        "S2 {side} field: {} units admitted; promoted in {promoted_ms:.1} ms, the task open by          {opened_ms:.0} ms; the focused map's window {} {}x{} of {}; {figures:?}",
        live.units.len(),
        focused["origin_cell"],
        focused["width"],
        focused["height"],
        focused["extent"],
    );
    for (what, size) in &figures[..2] {
        assert!(
            *size <= tacsim::match_::cockpit::MAP_MAX_BYTES,
            "{what} is {size} bytes"
        );
    }

    // (5) The bystander is fired on by a raider and becomes hostile to that party only. Which
    // raider hull fires, by its mind or a condition slot, one burst or ten, is draw order's: any
    // raider's shot at the bystander will do, and the bystander's armoured hull outlives it.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |_, lines| raiders
            .iter()
            .any(|hull| fired(lines, *hull, Some(bystander))))
        .await,
        "a raider fires on the bystander; {}",
        damage_summary(&lines, &raiders)
    );
    // (6) The escort fights while the miners hold. The raider's leader breaks off the bystander
    // and turns on the escort; the escort's own leader (its pilot) marks the raider Hostile.
    let escort_letter = letter_of(&rig, at.entry, leased);
    for (who, line) in [
        (
            leader,
            format!("+field/mark {}=clear", bystander_party.letter()),
        ),
        (leader, format!("+field/mark {escort_letter}=hostile")),
        (
            escort,
            format!("+field/mark {}=hostile", raider_party.letter()),
        ),
    ] {
        let said = typed(&mut rig, who, &line);
        assert!(said.contains("now marks party"), "{line}: {said}");
    }
    let bystander_view = |rig: &Rig| {
        frame(rig, captain)
            .map(|frame| {
                frame
                    .contacts
                    .iter()
                    .filter_map(|contact| match contact.class {
                        Classification::Classified { party } => Some((party, contact.relation)),
                        Classification::Unknown => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    };
    // One escort is mission-killed by an engine hit: the raider's coaxial strikes through the
    // escort's rear plate (G-6). The pulse the escort's own frame reads it immobile, the raider's
    // leader holds it Friendly (a Friendly mark outweighs the fire it took, CF.0) and the raider
    // breaks off. Meanwhile the bystander's sensors tell the raider's hulls apart.
    let mut friended = String::new();
    let told = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        if friended.is_empty() && frame(rig, escort).is_some_and(|frame| frame.immobile) {
            friended = typed(
                rig,
                leader,
                &format!("+field/mark {escort_letter}=friendly"),
            );
        }
        let view = bystander_view(rig);
        !friended.is_empty()
            && view
                .iter()
                .any(|(party, relation)| *party == raider_party && *relation == Relation::Hostile)
            && view.iter().any(|(party, _)| *party != raider_party)
    })
    .await;
    let view = bystander_view(&rig);
    assert!(
        told,
        "the escort's engine goes ({friended}) and the bystander holds the raider hostile; the \
         bystander sees {view:?}; {}",
        damage_summary(&lines, &[leased, walker, tank])
    );
    for (party, relation) in &view {
        assert_eq!(
            *relation == Relation::Hostile,
            *party == raider_party,
            "the bystander is hostile to the raider's party only: {view:?}"
        );
    }
    assert!(
        frame(&rig, escort).is_some_and(|frame| !frame.wreck),
        "the escort is mission-killed, not wrecked"
    );
    // The escort answers with its coaxial at the walker in its line, one facet draw a burst,
    // until the walker's head goes — the control-section hit (G-6).
    let mut said = String::new();
    let headed = until(&mut rig, &mut rx, &mut lines, 300, |rig, lines| {
        if destroyed_by(lines, walker, "head").is_some() {
            return true;
        }
        let held = frame(rig, escort).and_then(|frame| {
            frame
                .lock
                .filter(|lock| frame.contacts.iter().any(|contact| contact.id == *lock))
        });
        if held.is_none()
            && let Some(contact) = contact_at(rig, escort, at.walker.0)
        {
            said = run(&mut rig.tw, escort, &format!("LOCK {}", contact.designator));
        }
        if held.is_some() {
            for gun in ["machine_gun", "cannon"] {
                let out = run(&mut rig.tw, escort, &format!("FIRE {gun}"));
                if !out.contains("cycling") {
                    said = out;
                }
            }
        }
        false
    })
    .await;
    // The moment the walker falls, the leader orders the raiders home (their WITHDRAW holds fire).
    let ordered = run(
        &mut rig.tw,
        GOD,
        &format!("@field/raiders/withdraw {}", at.raider),
    );
    assert!(
        headed,
        "the escort's coaxial finds the walker's head ({said}); {}",
        damage_summary(&lines, &[leased, walker, tank])
    );
    assert_eq!(
        destroyed_by(&lines, walker, "head"),
        Some(Some(leased)),
        "the control section falls to the escort's fire"
    );
    assert_eq!(
        destroyed_by(&lines, leased, "engine"),
        Some(Some(tank)),
        "the engine fell to the raider's fire"
    );
    assert!(
        destroyed_by(&lines, leased, "hull").is_none(),
        "the escort's hull stands"
    );
    assert!(fired(&lines, leased, Some(walker)), "the escort fought");
    assert!(
        [worker, bystander]
            .iter()
            .all(|unit| !fired(&lines, *unit, None)),
        "the miners hold"
    );

    // (7) The raiders withdraw to their ramp, and the field demotes.
    assert!(ordered.contains("make for the"), "{ordered}");
    // The cells the tank drives through on its way home (doc 32 §20: at 512 it crosses the
    // 256 line).
    let mut trail: Vec<(i64, i64)> = cell_of(&rig, tank).into_iter().collect();
    let home = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        let now = cell_of(rig, tank);
        if let Some(cell) = now
            && trail.last() != Some(&cell)
        {
            trail.push(cell);
        }
        now.is_none()
    })
    .await;
    assert!(home, "the tank goes home up the ramp and aboard: {trail:?}");
    if wide {
        let line = at.side / 2;
        assert!(
            trail.first().is_some_and(|cell| cell.0 < line)
                && trail.iter().any(|cell| cell.0 > line),
            "the tank's withdrawal crosses the {line} line: {trail:?}"
        );
    }
    assert!(
        site_record_at(&mut rig, at.entry).is_some_and(|record| record.exited(tank)),
        "the open promotion names the tank's exit"
    );
    // Doc 36 cut GC.0 (Q-36-6, the host half): with the tank aboard and the walker a wreck, the
    // raider's unarmed barge lifts out of the field and off the site by F-6's departure. Nobody
    // marks the raider Friendly: the escort still holds its party Hostile, and with nothing of it
    // left standing the field demotes on its own quiet interval.
    let lifted = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        mush_sim::pose_of(rig.tw.world(), barge).is_none()
    })
    .await;
    assert!(lifted, "the barge lifts once its hulls are aboard or down");
    assert!(
        site_record_at(&mut rig, at.entry).is_some_and(|record| record.exited(barge)),
        "the open promotion names the barge's exit"
    );
    let demoted = until(&mut rig, &mut rx, &mut lines, 150, |rig, _| {
        task(rig).is_none()
            && site_record_at(rig, at.entry).is_some_and(|record| record.open_field().is_none())
    })
    .await;
    assert!(
        demoted,
        "the field demotes with no Friendly mark once nobody is engaged"
    );
    assert!(
        site_record_at(&mut rig, at.entry)
            .is_some_and(|record| record.units.iter().all(|left| left.unit != barge)),
        "the barge is gone, not left on the site record"
    );

    // (8) The miners resume their fill plan.
    let resumed = typed(&mut rig, miner, "+tv/1 mine go");
    assert!(
        !resumed.contains("being fought"),
        "the surface is the miners' again: {resumed}"
    );

    // (9) The casualty and the raider wreck are on the site record, where they fell.
    let fell = |rig: &mut Rig, unit: ObjId, condition: LeftCondition| {
        let record = site_record_at(rig, at.entry).expect("the record");
        let left = record
            .units
            .iter()
            .find(|left| left.unit == unit)
            .unwrap_or_else(|| panic!("#{} is on the site record: {:?}", unit.0, record.units))
            .clone();
        assert_eq!(left.condition, condition, "#{}", unit.0);
        let at = mush_sim::pose_of(rig.tw.world(), unit)
            .map(|(_, pose)| (pose.x_cm, pose.y_cm))
            .expect("it stands on the ground");
        assert_eq!(
            (left.pose.x_cm, left.pose.y_cm),
            at,
            "#{} where it fell",
            unit.0
        );
        at
    };
    let casualty_at = fell(&mut rig, leased, LeftCondition::Casualty);
    let wreck_at = fell(&mut rig, walker, LeftCondition::Wreck);

    // (10) The server restarts.
    drop(rx);
    let mut rig = reboot(rig, &[miner, captain, escort]);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..2 {
        let _ = second(&mut rig);
    }
    assert_eq!(fell(&mut rig, leased, LeftCondition::Casualty), casualty_at);
    assert_eq!(fell(&mut rig, walker, LeftCondition::Wreck), wreck_at);

    // (11) The carrier departs with the leave-behind confirmation. The miner ends the fill plan's
    // run after its cycle first (a lift waits on a drilling rig), and the rig stays out.
    let stop = typed(&mut rig, miner, "+tv/1 mine stop");
    let now = rig.tw.world().world_ic_now().at;
    let _ = crate::driving::to_ic(&mut rig, now + 3_600);
    let (refused, queued) = command(&mut rig, miner, "+ship/lift ceres");
    assert!(!refused.contains("is drilling"), "{stop} / {refused}");
    assert!(queued.is_empty(), "nothing is filed: {refused}");
    let list = refused
        .split_once("add leave=")
        .and_then(|(_, rest)| rest.split_once(':'))
        .map(|(list, _)| list.trim().to_owned())
        .unwrap_or_else(|| panic!("the refusal names the confirmation: {refused}"));
    let (filed, queued) = command(&mut rig, miner, &format!("+ship/lift ceres leave={list}"));
    assert_eq!(queued.len(), 1, "the burn is filed: {filed}");
    land(&mut rig, &queued[0]).expect("the burn lands");
    let left_behind = match rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .map(|store| store.voyage_state(carrier))
    {
        Some(tacsim::port::voyage::VoyageState::Present(voyage)) => voyage
            .pending
            .as_ref()
            .map(|pending| pending.left_behind.clone())
            .unwrap_or_default(),
        _ => Vec::new(),
    };
    assert!(
        left_behind.contains(&leased),
        "the casualty is left by the miner's order: {left_behind:?}"
    );

    // (12) A second visit finds both where they fell. Another captain sails in; the miner's
    // carrier has gone.
    let (visitor, _, _) = on_pad_as_at(&mut rig, ("Third", 3, "Lone Moth"), at.site(), 0);
    assert!(
        !matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), carrier),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == at.entry
        ),
        "the miner's carrier has lifted"
    );
    assert_eq!(fell(&mut rig, leased, LeftCondition::Casualty), casualty_at);
    assert_eq!(fell(&mut rig, walker, LeftCondition::Wreck), wreck_at);
    let _ = visitor;

    // (13) A second field at the same site, on the same ground: the raider launches again and
    // declares on the escort's hull, and the wreck is admitted where it fell.
    let again = run(
        &mut rig.tw,
        GOD,
        &format!("@field/raiders/launch {}", at.raider),
    );
    assert!(again.contains("launch from"), "{again}");
    stand(&mut rig, at.entry, tank, at.second);
    for _ in 0..2 {
        let _ = second(&mut rig);
    }
    let _ = typed(
        &mut rig,
        leader,
        &format!("+field/mark {}=hostile", at.second_mark),
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the second field opens"
    );
    assert_eq!(
        site_record_at(&mut rig, at.entry)
            .and_then(|record| record.open_field().map(|field| field.ordinal)),
        Some(2),
        "field 2 on the same site"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(at.entry)).expect("the live record");
    for unit in [walker, leased, tank] {
        assert!(
            live.units.iter().any(|row| row.unit == unit),
            "#{} is in the second field",
            unit.0
        );
    }
    // G-8, pinned at its gap (doc 32 §17): the wreck stands between the escort and the raider,
    // on the one row, and the kernel's lines are the terrain's alone — a unit standing in a cell,
    // wreck or not, hides nothing behind it. So the escort's sensors still hold the raider past
    // the wreck. When a wreck gives cover, this assert flips.
    let focus = run(
        &mut rig.tw,
        escort,
        &format!("+focus #{}/cockpit", leased.0),
    );
    assert!(!focus.contains("Usage"), "{focus}");
    // Past the wreck on the raider's side of the row: `x >= 158` on Ahuna Flank.
    let ((tx, ty), _) = at.second;
    let toward = (tx - at.walker.0.0).signum();
    let behind = |rig: &Rig| {
        frame(rig, escort).is_some_and(|frame| {
            frame.contacts.iter().any(|contact| {
                contact.cell.is_some_and(|(x, y)| {
                    i64::from(x) * toward >= (tx - 2 * toward) * toward
                        && i64::from(y).abs_diff(ty) <= 2
                })
            })
        })
    };
    let seen = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| behind(rig)).await;
    assert!(
        seen,
        "G-8 pinned: the wreck at {wreck_at:?} hides nothing, so the escort holds the raider \
         behind it; the escort's frame {:?}",
        frame(&rig, escort).map(|frame| (
            frame.cell,
            frame
                .contacts
                .iter()
                .map(|c| (c.designator.to_string(), c.cell))
                .collect::<Vec<_>>()
        ))
    );
}

/// **The field's arena on `at`'s own ground, with the story's nine units** (doc 32 cut S.2,
/// §20): the site's terrain priced as the field host prices it (`field::ground`'s window, with
/// no site objects laid), and a line tank for each unit the story's first field admits, at its
/// cell in four parties as the story stands them — the miner's carrier, rig and escort hull; the
/// bystander's carrier and rig; the hired escort; the raider's barge and two hulls — every hull
/// a stock mind under the production seal terms and every party Hostile to every other. Every
/// unit armed and fighting is heavier than the story, where the carriers and rigs never fire.
fn story_arena(rig: &mut Rig, at: &Ground) -> tacsim::arena::state::Arena {
    use tacsim::arena::state::{ArenaDefinition, ArenaTerrain, TerrainCell, Waypoints};
    let w = rig.tw.world();
    let (composite, biome) =
        tacsim::surface::ground::site_terrain_and_biome(w, &cid(at.entry)).expect("the terrain");
    let ground = composite
        .surface(&mush_atlas::SurfaceRef::Ground)
        .expect("a ground surface");
    let (width, height) = (ground.cells.width(), ground.cells.height());
    let table = mush_atlas::physics::LocomotionTable::shipped();
    let mut cells = Vec::with_capacity(usize::from(width) * usize::from(height));
    for y in 0..height {
        for x in 0..width {
            let cell = ground.cells.get(x, y).expect("a cell");
            cells.push(TerrainCell {
                class: cell.class,
                elev_cm: cell.elev_cm,
                move_cost: table
                    .cost(cell.class.label(), "tracked", &[])
                    .map_or(0, |cost| u16::try_from(cost).unwrap_or(u16::MAX)),
            });
        }
    }
    let terrain = ArenaTerrain::priced(width, height, composite.frame.cell_cm, cells, table)
        .expect("a priced window")
        .with_biome(biome);
    let definition = ArenaDefinition {
        id: SmolStr::new(format!("field:{}", at.entry)),
        terrain,
        entry_zones: Vec::new(),
        key_spots: Vec::new(),
        waypoints: Waypoints::new(Vec::new(), &[]),
        modes: Vec::new(),
        homes: Vec::new(),
    };
    let pad = tacsim::surface::excursion::site_pad_cell(w, &cid(at.entry));
    let barge = if at.side > 256 {
        (pad.0 + 40, pad.1 + 20)
    } else {
        (pad.0 + 28, pad.1 - 2)
    };
    let tank = at.tank.map_or((barge.0 + 4, barge.1), |(cell, _)| cell);
    let parties = [
        ('A', vec![(pad.0, pad.1), at.core, at.escort.0]),
        ('B', vec![(pad.0 + 3, pad.1), at.bystander]),
        ('C', vec![at.ward.0]),
        ('D', vec![barge, at.walker.0, tank]),
    ];
    let catalog = rig
        .tw
        .world()
        .module_state::<tacsim::TacsimStore>()
        .map(|store| store.definitions().clone())
        .expect("the layer");
    let spec = catalog
        .bindings
        .get(&cid("tacsim.unit.line_tank"))
        .map(|binding| binding.spec.clone())
        .expect("the line tank is bound");
    let mut units = Vec::new();
    for (letter, cells) in parties {
        let team = tacsim::arena::PartyId::from_letter(letter).expect("a party");
        for (slot, cell) in (1_u8..).zip(cells) {
            let id = staff_hand(rig, |ctx| {
                let obj = ctx
                    .w
                    .create(ObjKind::Thing, "Story stand-in", GOD, None)
                    .expect("the stand-in");
                mush_tactics::module::create_unit(ctx, obj, &spec).expect("its unit record");
                obj
            });
            let record = mush_tactics::module::unit_of(rig.tw.world(), id)
                .expect("the unit")
                .clone();
            let pose =
                tacsim::surface::excursion::cell_pose([cell.0, cell.1], 90_000).expect("a pose");
            units.push(
                tacsim::match_::custody_unit(
                    &catalog,
                    record,
                    id,
                    team,
                    pose,
                    0,
                    tacsim::arena::state::Controller::Mind,
                    slot,
                    &tacsim::match_::SealTerms::production_fixture(),
                )
                .expect("the production fill seals"),
            );
        }
    }
    let custody = tacsim::arena::state::CustodyOut {
        definition,
        units,
        pilots: Vec::new(),
        resolution: tacsim::match_::resolution_for(&catalog),
        seed: 7,
        mode: tacsim::arena::MatchMode::Skirmish,
        host: tacsim::arena::FieldHost::all_hostile(),
    };
    tacsim::arena::state::Arena::new(
        custody,
        &tacsim::config::ArenaConfig::default(),
        &tacsim::config::MatchConfig::default(),
        &tacsim::config::SensorsConfig::default(),
    )
    .expect("the field's arena checks out")
}

/// **Doc 32 cut S.2: a field's pulse and its snapshot, measured at 256 and at 512** (Q-CF-45,
/// Q-CF-51; §20's table, printed under `--nocapture`): [`story_arena`] on Ahuna Flank and on
/// Urvara Reach, stepped (`step_settled`, the minds' thinks run inline) for 600 Engaged pulses
/// of 100 ms — a minute of fighting — or until it ends, each pulse timed; its snapshot encoded
/// at the opening and at the end. Gates: the snapshot under the arena's own
/// `SNAPSHOT_MAX_BYTES`, which `Snapshot::encode` enforces, and a median pulse under the
/// engaged tempo's 100 ms, generous in a debug build (a shared runner's speed is not a
/// threshold).
#[test]
fn s2_a_field_pulse_is_measured_at_both_sides() {
    let mut rig = stocked();
    for at in [&AHUNA_FLANK, &URVARA_REACH] {
        let mut arena = story_arena(&mut rig, at);
        let opening = tacsim::arena::Snapshot::encode(&arena)
            .expect("the opening encodes")
            .bytes()
            .len();
        let mut samples = Vec::new();
        let mut wall = 0_i64;
        let mut shots = 0_usize;
        for _ in 0..600 {
            wall += 100;
            let started = std::time::Instant::now();
            let out = arena.step_settled(wall, &[], &[]);
            samples.push(started.elapsed().as_secs_f64() * 1_000.0);
            shots += out
                .ledger_chunks
                .iter()
                .filter_map(|chunk| std::str::from_utf8(&chunk.bytes).ok())
                .flat_map(str::lines)
                .filter_map(|line| serde_json::from_str::<LedgerEvent>(line).ok())
                .filter(|event| matches!(event.kind, EventKind::Fired { .. }))
                .count();
            if out.ended.is_some() {
                break;
            }
        }
        let closing = tacsim::arena::Snapshot::encode(&arena)
            .expect("the arena encodes")
            .bytes()
            .len();
        let mut sorted = samples.clone();
        sorted.sort_by(f64::total_cmp);
        let at_rank = |p: usize| sorted.get(sorted.len() * p / 100).copied().unwrap_or(0.0);
        let total: f64 = samples.iter().sum();
        println!(
            "S2 {} pulse: {} pulses of {} units, {shots} shots, median {:.2} / p99 {:.2} / max {:.2} ms, mean \
             {:.2} ms; snapshot {opening} B at the opening, {closing} B at the end",
            at.side,
            samples.len(),
            arena.units.len(),
            at_rank(50),
            at_rank(99),
            sorted.last().copied().unwrap_or(0.0),
            total / f64::from(u32::try_from(samples.len()).expect("a count")),
        );
        assert!(shots > 0, "the minute is fought");
        assert!(
            at_rank(50) < 100.0,
            "a median pulse under the tempo: {sorted:?}"
        );
    }
}

/// **Doc 32 §17: two hulls on one pad launch their rigs onto one cell, and the field opens.**
/// Every hull on a pad launches down the pad's one ramp cell, and the surface lets two rigs
/// share it. Before the fix admission then refused the whole field ("occupancy is one unit per
/// cell"); now it parts them, the first in unit order keeping the ramp's cell and the other
/// stood on the nearest free cell, and admits both.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cf7_two_rigs_on_one_ramp_cell_are_parted_at_admission_and_the_field_opens() {
    let mut rig = stocked_tuned(crate::field_host::tune);
    let (a, _, a_tvs) = on_pad(&mut rig, 1);
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    launched(&mut rig, a, a_tvs[0]);
    launched(&mut rig, b, b_tvs[0]);
    let ramp = tacsim::surface::excursion::site_cell(RAMP.0, RAMP.1);
    assert_eq!(
        (cell_of(&rig, a_tvs[0]), cell_of(&rig, b_tvs[0])),
        (Some(ramp), Some(ramp)),
        "both rigs share the ramp's cell"
    );
    let hull = armed(&mut rig, "Grey Hound", (140, 124), 270_000);
    let gunner = pilot(&mut rig, "Gunner", 4);
    seat(&mut rig, hull, gunner);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let _ = typed(&mut rig, gunner, "+field/fire W");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field opens with both rigs on the ground"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    for unit in [a_tvs[0], b_tvs[0]] {
        assert!(
            live.units.iter().any(|row| row.unit == unit),
            "#{} is admitted",
            unit.0
        );
    }
    let (first, second_cell) = (
        cell_of(&rig, a_tvs[0].min(b_tvs[0])).expect("the first rig"),
        cell_of(&rig, a_tvs[0].max(b_tvs[0])).expect("the second rig"),
    );
    assert_eq!(first, ramp, "the first in unit order keeps the ramp's cell");
    assert_eq!(
        second_cell
            .0
            .abs_diff(ramp.0)
            .max(second_cell.1.abs_diff(ramp.1)),
        1,
        "the other is stood one cell from it"
    );
}
