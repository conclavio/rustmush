//! **Doc 32 cut CF.7 — the field story** (Q-CF-47), over the composed world (`driving`'s keyed
//! rig on Ahuna Flank) with the arena's egress pumped by hand, as CF.2's to CF.6's gates pump it.
//!
//! **The whole of Q-CF-47 at 256 a side** (doc 32 §17), through the production command paths, in
//! one story: the mining roster's block, its two-hull escort and a bystander from another captain;
//! the raider launched from its fixture carrier and seen on the presence picture before it
//! declares; the promotion with everyone at their poses; the raiders on the miner's quarry screen
//! and the worked block on the escort's map (G-7); the bystander fired on, hostile to that party
//! only; the escort's fight while the miners hold; an escort mission-killed by an engine hit and a
//! raider by a control-section hit (G-6); the raiders home up their ramp and the demotion; the
//! fill plan resumed; the casualty and the wreck on the site record; a restart; the carrier's
//! departure on the leave-behind confirmation; a second visit finding both where they fell; and a
//! second field on the same ground, where the wreck's cover is pinned at its gap (G-8).
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
    AHUNA, RAMP, Rig, act_go, ahuna_domain, at_counter, cid, command, finish, land, launched,
    map_frame, on_pad, on_pad_as, on_pad_with, pilot, reboot, run, staff_hand, stocked_tuned,
    to_cell, warm,
};
use crate::field_host::{armed, egress, frame, seat, second, site_record, task, until};
use crate::field_raiders::{SKULKS, raider_hulls, seat_npc};

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
    let (mut out, mut fx) = (mush_core::Outbox::new(), mush_core::Effects::new());
    rig.tw.engine_mut().step(
        mush_core::Inbound::Query {
            conn: 7,
            actor: who,
            id: 1,
            name: SmolStr::new(tacsim::arena_bridge::FRAME_FEED),
            args: json!({}),
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

/// Stand `unit` on Ahuna Flank's ground at `cell`, facing `heading` (a staff hand).
fn stand(rig: &mut Rig, unit: ObjId, cell: (i64, i64), heading: u32) {
    let domain = ahuna_domain(&rig.tw);
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

/// The live record's party letter of `unit`.
fn letter_of(rig: &Rig, unit: ObjId) -> char {
    tacsim::field::live_on(rig.tw.world(), &cid(AHUNA))
        .and_then(|live| live.party_of(unit))
        .map(|party| party.letter())
        .unwrap_or_else(|| panic!("#{} is in the field", unit.0))
}

/// `who` types `line`, and anything it submits lands.
fn typed(rig: &mut Rig, who: ObjId, line: &str) -> String {
    let (out, queued) = command(rig, who, line);
    for submit in &queued {
        land(rig, submit).unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    out
}

/// **The field story** (doc 32 Q-CF-47), the whole of it at 256 a side.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf7_the_field_story() {
    let mut rig = stocked_tuned(|cfg| {
        crate::field_host::tune(cfg);
        cfg.field.disengage_s = 15;
    });

    // (1) At a named frontier site — Ahuna Flank — a mining roster works a block with an escort
    // of two armed hulls; a bystander rig from another captain works the next block.
    let (captain, _, captain_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let bystander = captain_tvs[0];
    launched(&mut rig, captain, bystander);
    to_cell(&mut rig, bystander, (140, 136));
    // The miner leases a prospector and Occator's escort line tank (CF.6c) at Ceres. A new
    // captain's stake covers one or the other, so a staff hand moves 7,000cr from the other
    // captain's account first (`@econ/pay`).
    let (miner, carrier_row, miner_tvs) =
        on_pad_with(&mut rig, "Pilot", 1, "Salt Moth", |rig, miner| {
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
        });
    let carrier = ObjId(carrier_row.hull);
    assert_eq!(miner_tvs.len(), 2, "a prospector and an escort in the bay");
    let (worker, leased) = (miner_tvs[0], miner_tvs[1]);
    launched(&mut rig, miner, worker);
    // Doc 32 §17: a second rig down the ramp while the first still stands on its cell is stood
    // beside it, never on it.
    let _ = act_go(&mut rig, miner, "+tv/2 launch go");
    let _ = finish(&mut rig, leased, miner);
    let ramp = tacsim::surface::excursion::site_cell(RAMP.0, RAMP.1);
    let (on_ramp, beside) = (
        cell_of(&rig, worker).expect("rig 1"),
        cell_of(&rig, leased).expect("rig 2"),
    );
    assert_eq!(on_ramp, ramp, "rig 1 stands on the ramp's cell");
    assert!(
        beside != ramp && beside.0.abs_diff(ramp.0).max(beside.1.abs_diff(ramp.1)) == 1,
        "rig 2 stands one cell from the ramp, not on rig 1: {beside:?} by {ramp:?}"
    );
    // The miner's own readings: a scan and a sounding of the ground the rig works (this pocket
    // reads barren), then the rig moves one pocket west to core an unread one.
    to_cell(&mut rig, worker, (140, 120));
    for act in ["+tv/1 scan go", "+tv/1 sound go"] {
        let _ = act_go(&mut rig, miner, act);
        let _ = finish(&mut rig, worker, miner);
    }
    to_cell(&mut rig, worker, (136, 120));
    // The escort: the miner's leased line tank, flown by a player escort who never sounded
    // anything; and a fixture hull a hired NPC flies on doctrine.
    stand(&mut rig, leased, (150, 126), 270_000);
    let escort = pilot(&mut rig, "Escort", 7);
    let ward = armed(&mut rig, "Ward Dog", (146, 120), 90_000);
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
        assert_eq!(queued.len(), 1, "a mine run starts: {out}");
        land(&mut rig, &queued[0]).expect("the run's start lands");
    }
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // (2) A raider party launches from its fixture carrier (doc 32 cut CF.6b) and is seen on the
    // presence picture (Q-CF-15) before it declares.
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {SKULKS}"));
    assert!(seeded.contains("seeded"), "{seeded}");
    for _ in 0..2 {
        let _ = second(&mut rig);
    }
    let (before, _) = presence_rows(&mut rig, miner);
    let launched_out = run(&mut rig.tw, GOD, &format!("@field/raiders/launch {SKULKS}"));
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
    stand(&mut rig, walker, (153, 126), 270_000);
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
    for _ in 0..2 {
        let _ = second(&mut rig);
    }

    // (3) It declares on the bystander (NW of its ramp) and the site promotes with everyone at
    // their poses.
    let out = typed(&mut rig, leader, "+field/mark NW=hostile");
    assert!(
        site_record(&mut rig).is_some_and(|record| record.open_field().is_some()),
        "the declaration promotes the site: {out}"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
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

    // (5) The bystander is fired on by a raider and becomes hostile to that party only.
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
    let escort_letter = letter_of(&rig, leased);
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
            && let Some(contact) = contact_at(rig, escort, (153, 126))
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
        &format!("@field/raiders/withdraw {SKULKS}"),
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
    let home = until(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        mush_sim::pose_of(rig.tw.world(), tank).is_none()
    })
    .await;
    assert!(home, "the tank goes home up the ramp and aboard");
    assert!(
        site_record(&mut rig).is_some_and(|record| record.exited(tank)),
        "the open promotion names the tank's exit"
    );
    // The raider's unarmed barge still stands on the ground, Hostile to the escort that was fired
    // on, and an armed unit holding a live Hostile within its reach keeps a field engaged (§15's
    // known gap, doc 32 §17): the escort's pilot stands down with a Friendly mark.
    let stood_down = typed(
        &mut rig,
        escort,
        &format!("+field/mark {}=friendly", raider_party.letter()),
    );
    assert!(stood_down.contains("Friendly"), "{stood_down}");
    let demoted = until(&mut rig, &mut rx, &mut lines, 150, |rig, _| {
        task(rig).is_none() && site_record(rig).is_some_and(|record| record.open_field().is_none())
    })
    .await;
    assert!(demoted, "the field demotes once nobody is engaged");

    // (8) The miners resume their fill plan.
    let resumed = typed(&mut rig, miner, "+tv/1 mine go");
    assert!(
        !resumed.contains("being fought"),
        "the surface is the miners' again: {resumed}"
    );

    // (9) The casualty and the raider wreck are on the site record, where they fell.
    let fell = |rig: &mut Rig, unit: ObjId, condition: LeftCondition| {
        let record = site_record(rig).expect("the record");
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
    let (visitor, _, _) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 0);
    assert!(
        !matches!(
            tacsim::port::ship::berth_of(rig.tw.world(), carrier),
            tacsim::port::ship::Berth::Docked { map, .. } if map.entry.as_str() == AHUNA
        ),
        "the miner's carrier has lifted"
    );
    assert_eq!(fell(&mut rig, leased, LeftCondition::Casualty), casualty_at);
    assert_eq!(fell(&mut rig, walker, LeftCondition::Wreck), wreck_at);
    let _ = visitor;

    // (13) A second field at the same site, on the same ground: the raider launches again and
    // declares on the escort's hull, and the wreck is admitted where it fell.
    let again = run(&mut rig.tw, GOD, &format!("@field/raiders/launch {SKULKS}"));
    assert!(again.contains("launch from"), "{again}");
    stand(&mut rig, tank, (160, 126), 270_000);
    for _ in 0..2 {
        let _ = second(&mut rig);
    }
    let _ = typed(&mut rig, leader, "+field/mark W=hostile");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the second field opens"
    );
    assert_eq!(
        site_record(&mut rig).and_then(|record| record.open_field().map(|field| field.ordinal)),
        Some(2),
        "field 2 on the same site"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
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
    let behind = |rig: &Rig| {
        frame(rig, escort).is_some_and(|frame| {
            frame.contacts.iter().any(|contact| {
                contact
                    .cell
                    .is_some_and(|(x, y)| x >= 158 && y.abs_diff(126) <= 2)
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

/// **Doc 32 §17: two hulls on one pad launch their rigs onto two cells, and the field opens.**
/// Before the fix a rig launched down a shared ramp landed on the cell another hull's rig already
/// stood on, and admission refused the field ("occupancy is one unit per cell").
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cf7_two_hulls_on_one_pad_launch_beside_each_other_and_the_field_opens() {
    let mut rig = stocked_tuned(crate::field_host::tune);
    let (a, _, a_tvs) = on_pad(&mut rig, 1);
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    launched(&mut rig, a, a_tvs[0]);
    let _ = act_go(&mut rig, b, "+tv/1 launch go");
    let _ = finish(&mut rig, b_tvs[0], b);
    let ramp = tacsim::surface::excursion::site_cell(RAMP.0, RAMP.1);
    let (first, second_cell) = (
        cell_of(&rig, a_tvs[0]).expect("A's rig"),
        cell_of(&rig, b_tvs[0]).expect("B's rig"),
    );
    assert_eq!(first, ramp);
    assert_ne!(first, second_cell, "two rigs, two cells");
    assert_eq!(
        second_cell
            .0
            .abs_diff(ramp.0)
            .max(second_cell.1.abs_diff(ramp.1)),
        1,
        "B's rig stands beside the ramp"
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
}
