//! **Doc 40 cut FR.4 — the black box in segments** (Q-38-28, Q-40-21), over the composed world
//! (`driving`'s keyed rig), the arena's egress pumped by hand as `field_fr2.rs` pumps it, on both
//! grounds of `grounds_rig` (Ahuna Flank at 256, Urvara Reach at 512).
//!
//! A's `+field/fire` at B's hull opens a field under a ledger bound lowered to [`MAX_BYTES`] (the
//! test knob: the shipped 64 MiB would take eighteen engaged minutes at 100 units). In its first
//! minute the segment passes three quarters of the bound, and at the snapshot cadence the task
//! rolls in place: the last segment's chunks, then the `segment` line as a chunk of its own, then
//! the period's snapshot, the new segment's head, beside a live record that names segment 2.
//! The world restarts at that head; the field resumes from it, rolling again onto segment 3 at
//! the head's own millisecond. The world then holds every chunk: segment 1's log, segment 2's
//! (its line), segment 3's after the resume, all `field.<site>.<ordinal>.*` of the one field,
//! and their lines, read in chunk order, one sequence with no gap.
//!
//! **What is a fixture.** As FR.2's: the armed hulls are made and seated by staff hand. Everything
//! after is the production field.

use std::collections::BTreeMap;

use tacsim::arena::ledger::{EventKind, LedgerEvent};

use crate::driving_rig::{Rig, cid, command, crash, land, on_pad_as_at, stocked_tuned, warm};
use crate::field_host_rig::{armed_on, egress, seat, second, task, tune, until};
use crate::grounds_rig::{AHUNA_FLANK, Ground, URVARA_REACH};

/// **The test knob**: the ledger's bound, lowered from 64 MiB so a two-hull field rolls in its
/// first minute (three quarters of it is 36 KiB, two 16 KiB chunks and a part).
const MAX_BYTES: u64 = 48 * 1024;

/// A's and B's cells on `ground`, four cells apart on one row, A west facing east (FR.2's).
fn stands(ground: &Ground) -> ((i64, i64), (i64, i64)) {
    if ground.side == 512 {
        ((250, 268), (254, 268))
    } else {
        ((110, 124), (114, 124))
    }
}

/// The `segment` lines the field has shipped, as `(segment, seq, t_ms)`.
fn segments(lines: &[LedgerEvent]) -> Vec<(u32, u64, i64)> {
    lines
        .iter()
        .filter_map(|event| match event.kind {
            EventKind::Segment { segment } => Some((segment, event.seq, event.t_ms)),
            _ => None,
        })
        .collect()
}

/// The live record's segment and moment on `ground`.
fn live(rig: &Rig, ground: &Ground) -> Option<(u32, i64)> {
    tacsim::field::live_on(rig.tw.world(), &cid(ground.entry))
        .map(|live| (live.segment, live.at_ms))
}

/// Every chunk the world holds under `prefix` (`field.<site>.<ordinal>.`): segment → the arena
/// chunk sequence → its lines.
fn held(rig: &Rig, prefix: &str) -> BTreeMap<u32, BTreeMap<u64, Vec<LedgerEvent>>> {
    let mut logs: BTreeMap<u32, BTreeMap<u64, Vec<LedgerEvent>>> = BTreeMap::new();
    let Some(data) = rig.store.get(tacsim::NS) else {
        return logs;
    };
    for (key, text) in &data.ledger {
        let Some((log_id, _)) = mush_core::parse_chunk_key(key) else {
            continue;
        };
        let Some(segment) = log_id
            .strip_prefix(prefix)
            .and_then(|segment| segment.parse::<u32>().ok())
        else {
            continue;
        };
        let chunk = mush_core::parse_chunk(text).expect("a chunk document");
        for entry in chunk.entries {
            let seq = entry.value["chunk"]
                .as_u64()
                .expect("the arena chunk's sequence");
            let lines = entry.value["lines"]
                .as_str()
                .expect("the chunk's lines")
                .lines()
                .map(|line| serde_json::from_str::<LedgerEvent>(line).expect("a ledger line"))
                .collect();
            logs.entry(segment).or_default().insert(seq, lines);
        }
    }
    logs
}

/// **FR.4 on `ground`**: the live roll in the first minute, a restart at the new segment's head,
/// the resume's roll, and every chunk of the segments in the world.
#[allow(clippy::too_many_lines)] // one field, in order
async fn segments_on(ground: &Ground) {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        // The field holds through the story: nobody fires after the opening round.
        cfg.field.disengage_s = 600;
        cfg.field.idle_demote_s = 600;
        // A summary every pulse fills the black box at a known pace; the snapshot every two
        // arena seconds is the roll's cadence and leaves the test the head to restart at.
        cfg.arena.ledger.summary_ms = 100;
        cfg.arena.ledger.max_bytes = MAX_BYTES;
        cfg.arena.snapshot_ms = 2_000;
    });
    let (cell_a, cell_b) = stands(ground);
    let (a, _, _) = on_pad_as_at(&mut rig, ("Pilot", 1, "Salt Moth"), ground.site(), 1);
    let (b, _, _) = on_pad_as_at(&mut rig, ("Other", 2, "Ledger Moth"), ground.site(), 1);
    let hull_a = armed_on(&mut rig, ground.entry, "Red Hound", cell_a, 90_000);
    let hull_b = armed_on(&mut rig, ground.entry, "Grey Hound", cell_b, 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // A's fire order at B's hull opens the field.
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let field = tacsim::field::live_on(rig.tw.world(), &cid(ground.entry))
        .expect("the live record")
        .field;
    let prefix = format!("field.{}.{}.", field.site, field.ordinal);

    // The live roll, in the field's first minute, and its head in the world: the live record
    // on segment 2 at the roll's own millisecond.
    let entry = ground;
    assert!(
        until(&mut rig, &mut rx, &mut lines, 90, |rig, lines| {
            segments(lines)
                .first()
                .is_some_and(|&(_, _, at)| live(rig, entry) == Some((2, at)))
        })
        .await,
        "the field rolls and its head is written: {:?}, live {:?}",
        segments(&lines),
        live(&rig, ground)
    );
    let (segment, roll_seq, roll_ms) = segments(&lines)[0];
    assert_eq!(segment, 2, "the first roll opens segment 2");
    assert!(
        roll_ms < 60_000,
        "the field rolls in its first minute: {roll_ms} ms"
    );
    let before: u64 = lines
        .iter()
        .filter(|event| event.seq < roll_seq)
        .map(|event| event.seq)
        .max()
        .expect("segment 1's lines");
    assert_eq!(before + 1, roll_seq, "the line takes the next sequence");

    // The restart, at the head: the field resumes from it and rolls onto segment 3 there.
    let mut rig = crash(rig, &mut rx, &[a, b]).await;
    let mut rx = egress(&mut rig);
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| task(rig)
            .is_some()
            && live(rig, entry).is_some_and(|(segment, _)| segment == 3))
        .await,
        "the field resumes on segment 3: live {:?}",
        live(&rig, ground)
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |_, lines| {
            segments(lines).iter().any(|&(segment, _, _)| segment == 3)
                && lines.iter().any(|event| event.t_ms > roll_ms + 3_000)
        })
        .await,
        "segment 3's chunks ship: {:?}",
        segments(&lines)
    );
    let resumed = segments(&lines)
        .into_iter()
        .find(|&(segment, _, _)| segment == 3)
        .expect("the resume's line");
    assert_eq!(
        resumed.2, roll_ms,
        "the resume rewinds to the segment's head, the roll's own millisecond"
    );
    assert_eq!(
        resumed.1,
        roll_seq + 1,
        "the resume's line follows the head's"
    );

    // The world: every chunk, filed by segment, one chain.
    let ops = rig.tw.take_module_ops();
    mush_testkit::fold_module_ops_into(&mut rig.store, ops);
    let logs = held(&rig, &prefix);
    assert_eq!(
        logs.keys().copied().collect::<Vec<_>>(),
        vec![1, 2, 3],
        "a log a segment, under the one field {prefix}*"
    );
    let first: Vec<u64> = logs[&1].keys().copied().collect();
    assert_eq!(
        first,
        (1..=first.len() as u64).collect::<Vec<_>>(),
        "segment 1 holds its chunks from the first"
    );
    assert!(
        logs[&1]
            .values()
            .flatten()
            .all(|event| !matches!(event.kind, EventKind::Segment { .. })),
        "segment 1 holds no segment line"
    );
    let head = first.len() as u64 + 1;
    let opening = logs[&2]
        .get(&head)
        .expect("segment 2 opens on the next chunk");
    assert_eq!(opening.len(), 1, "the roll's line is a chunk of its own");
    assert_eq!(opening[0].kind, EventKind::Segment { segment: 2 });
    let resumed_chunk = logs[&3]
        .get(&(head + 1))
        .expect("segment 3 opens on the chunk after the head");
    assert_eq!(
        resumed_chunk[0].kind,
        EventKind::Segment { segment: 3 },
        "the resume's line opens its chunk"
    );
    // Read in chunk order, the later segment's copy of a chunk the restart re-fought.
    let mut chain: BTreeMap<u64, &Vec<LedgerEvent>> = BTreeMap::new();
    for chunks in logs.values() {
        for (seq, chunk_lines) in chunks {
            chain.insert(*seq, chunk_lines);
        }
    }
    let seqs: Vec<u64> = chain
        .values()
        .flat_map(|chunk| chunk.iter().map(|event| event.seq))
        .collect();
    assert_eq!(seqs.first(), Some(&1));
    assert!(
        seqs.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "one sequence, no gap, across the segments"
    );
    assert!(
        chain
            .keys()
            .copied()
            .eq(1..=*chain.keys().last().expect("chunks")),
        "every chunk numbered once"
    );
}

/// **FR.4 at 256**: Ahuna Flank.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr4_ahuna_a_field_rolls_resumes_from_the_head_and_keeps_every_chunk() {
    segments_on(&AHUNA_FLANK).await;
}

/// **FR.4 at 512**: Urvara Reach.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fr4_urvara_a_field_rolls_resumes_from_the_head_and_keeps_every_chunk() {
    segments_on(&URVARA_REACH).await;
}
