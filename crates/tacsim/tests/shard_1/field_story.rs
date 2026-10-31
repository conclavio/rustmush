//! **Doc 32 cut CF.7 — the field story** (Q-CF-47), over the composed world (`driving`'s keyed
//! rig on Ahuna Flank) with the arena's egress pumped by hand, as CF.2's to CF.6's gates pump it.
//!
//! **Written up to the escort's fight** (doc 32 §12). Q-CF-47's steps run in order through the
//! production command paths as far as the escort's fight, where the story ends until doc 32 cut
//! CF.6b (the raider's carrier, the doctrine's fallback, the withdrawal to a ramp) and the bot's
//! IC-field cut land. G-1's presence step, pinned at the gap until the 2026-10-31 review, now
//! passes: the presence book always held every unit on the ground, and `+tv/map` drew only the
//! lane's rigs from it (the review's CF-2, doc 32 §15). Each raider hull is a row of the miner's
//! picture.
//!
//! What the story stands in for, each named in doc 32 §12:
//! - the raider **launches from its fixture carrier** (`@field/raiders`, then
//!   `@field/raiders/launch`; doc 32 cut CF.6b, §13), and its hulls are flights on the presence
//!   picture (§15);
//! - the escort's two armed hulls are **fixture hulls with hired NPCs seated**, flying doctrine
//!   (CF.6's raider gate); the armed rig lease is not built, and the delivered orders the escort
//!   fights under are CF.6's transport gate's, in the split-boundary harness;
//! - the mining roster is **a player's rig working a block by hand** (`+tv/1 mine go`).

use mush_core::{GOD, ObjId, ObjKind};
use serde_json::json;

use crate::driving::{
    AHUNA, cid, command, land, launched, map_frame, on_pad, on_pad_as, run, staff_hand,
    stocked_tuned, to_cell, warm,
};
use crate::field_host::{armed, egress, frame, seat, second, task, until};
use crate::field_raiders::{SKULKS, raider_hulls, seat_npc};

/// The rows of `who`'s site map's presence line (`Presence: E within 200 m; SW 200-500 m.`),
/// read from the frame, where the printed map would wrap it.
fn presence_rows(rig: &mut crate::driving::Rig, who: ObjId) -> (Vec<String>, String) {
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

/// **The field story** (doc 32 Q-CF-47), up to the escort's fight.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf7_the_field_story_to_the_escorts_fight() {
    let mut rig = stocked_tuned(|cfg| {
        crate::field_host::tune(cfg);
        cfg.field.disengage_s = 30;
    });

    // (1) At a named frontier site — Ahuna Flank — a mining roster works a block with an escort
    // of two armed hulls; a bystander rig from another captain works the next block.
    let (miner, carrier_row, miner_tvs) = on_pad(&mut rig, 1);
    let carrier = ObjId(carrier_row.hull);
    let worker = miner_tvs[0];
    launched(&mut rig, miner, worker);
    to_cell(&mut rig, worker, (140, 120));
    let (captain, _, captain_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let bystander = captain_tvs[0];
    launched(&mut rig, captain, bystander);
    to_cell(&mut rig, bystander, (140, 136));
    let escorts = [
        armed(&mut rig, "Ward Dog", (144, 122), 90_000),
        armed(&mut rig, "Ward Hound", (144, 126), 90_000),
    ];
    let hired: Vec<ObjId> = escorts
        .iter()
        .enumerate()
        .map(|(n, hull)| {
            let npc = staff_hand(&mut rig, |ctx| {
                ctx.w
                    .create(ObjKind::Npc, &format!("Hired escort {}", n + 1), GOD, None)
                    .expect("the NPC")
            });
            seat_npc(&mut rig, *hull, npc);
            npc
        })
        .collect();
    seat(&mut rig, worker, miner);
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

    // (2) A raider party arrives — launched from its fixture carrier (doc 32 cut CF.6b) — is seen
    // on the presence picture (Q-CF-15), declares, and the site promotes with everyone at their
    // poses.
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {SKULKS}"));
    assert!(seeded.contains("seeded"), "{seeded}");
    // The raider's fixture carrier stands on the ground, a flight like any other: the picture
    // before its hulls launch.
    for _ in 0..2 {
        let _ = second(&mut rig);
    }
    let (before, _) = presence_rows(&mut rig, miner);
    let launched_out = run(&mut rig.tw, GOD, &format!("@field/raiders/launch {SKULKS}"));
    assert!(launched_out.contains("launch from"), "{launched_out}");
    let raiders = raider_hulls(&rig);
    assert_eq!(raiders.len(), 2, "the raider's two hulls stand");
    let leader = tacsim::field::raiders::leader_of(rig.tw.world(), raiders[0]).expect("a leader");
    for _ in 0..2 {
        let _ = second(&mut rig);
    }
    // G-1, passed (doc 32 §12 as corrected in §15, the review's CF-2): the presence book has held
    // every unit on the ground since CF.2, and `+tv/map` now draws it, so each raider hull is a
    // row of the miner's picture, seen from the miner's rig on the ground.
    let (rows, picture) = presence_rows(&mut rig, miner);
    let from = mush_sim::pose_of(rig.tw.world(), worker)
        .map(|(_, pose)| tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
        .expect("the miner's rig stands on the ground");
    for hull in &raiders {
        let cell = mush_sim::pose_of(rig.tw.world(), *hull)
            .map(|(_, pose)| tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm))
            .expect("the raider hull stands on the ground");
        let flight = tacsim::surface::presence::Flight {
            tv: *hull,
            cell,
            class: tacsim::surface::presence::hull_class(rig.tw.world(), *hull),
        };
        let seen = tacsim::surface::presence::seen_from(
            &[flight],
            &std::collections::BTreeSet::new(),
            &[(1, from)],
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
    let (out, queued) = command(&mut rig, leader, "+field/mark W=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("the live record");
    for unit in [carrier, worker, bystander]
        .into_iter()
        .chain(escorts)
        .chain(raiders.iter().copied())
    {
        assert!(
            live.units.iter().any(|row| row.unit == unit),
            "#{} is admitted at its pose",
            unit.0
        );
    }

    // The miners see the raiders as contacts on the quarry screen (Q-CF-48): the site map draws
    // the field layer from the miner's own cab. The escort's map carrying the worked block is the
    // chart of the person in its seat (CF.4), and a hired NPC holds none: doc 32 §12.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
            map_frame(rig, miner, json!({ "view": "site" }))["site"]["field_layer"]["held"]
                .as_u64()
                .is_some_and(|held| held > 0)
                && hired.iter().all(|npc| frame(rig, *npc).is_some())
        })
        .await,
        "the miner's quarry screen holds the raiders, and the escort's frames reach the host"
    );

    // The escort fights while the miners hold: the raider opens fire with the miners' carrier on
    // its pad, and the escort answers. (The escort stands nearer the raider than the carrier, so
    // the doctrine's best hostile has a line here; §11's carrier finding is G-2, CF.6b's.)
    let fired = |lines: &[tacsim::arena::ledger::LedgerEvent], who: ObjId| {
        lines.iter().any(|line| {
            matches!(line.kind, tacsim::arena::ledger::EventKind::Fired { unit, .. } if unit == who)
        })
    };
    let fought = until(&mut rig, &mut rx, &mut lines, 60, |_, lines| {
        raiders.iter().any(|hull| fired(lines, *hull))
            && escorts.iter().any(|hull| fired(lines, *hull))
    })
    .await;
    assert!(
        fought,
        "the raider and the escort exchange fire; raider shots {:?}, escort shots {:?}",
        raiders
            .iter()
            .map(|hull| fired(&lines, *hull))
            .collect::<Vec<_>>(),
        escorts
            .iter()
            .map(|hull| fired(&lines, *hull))
            .collect::<Vec<_>>()
    );
    let miners_held = [worker, bystander].iter().all(|unit| !fired(&lines, *unit));
    assert!(miners_held, "the miners hold");

    // Q-CF-47's steps after this one, written once CF.6b and the bot's cut land: the bystander is
    // fired on by a raider and becomes hostile to that party only; one escort is mission-killed by
    // an engine hit and one raider by a control-section hit; the raiders withdraw to their ramp;
    // the field demotes; the miners resume their fill plan; the casualty and the raider wreck are
    // on the site record; the server restarts; the carrier departs with the leave-behind
    // confirmation; a second visit finds both where they fell; a second field at the same site
    // uses the wreck as cover. Then the bot, against the fixture raider.
}
