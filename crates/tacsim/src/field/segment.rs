//! **The black box in segments** (doc 40 cut FR.4; Q-38-28, Q-40-21).
//!
//! A field's ledger is bounded by `[tacsim.arena.ledger].max_bytes`, and at 100% the field stops
//! with `Timeout` — about eighteen engaged minutes at 100 units. So the bound is a **segment's**:
//!
//! - **The live roll.** At the snapshot cadence, once a segment holds three quarters of
//!   `max_bytes` (`Ledger::segment_full`), the field task closes the open chunk, queues every
//!   closed one and appends a `segment` line that opens the next segment as a chunk of its own
//!   (`Ledger::roll_segment`, `bytes_total` back to nought); that period's snapshot, queued
//!   behind the line, is the new segment's head. Only a task under the field host's terms rolls, so no OOC match, and no
//!   practice hash, ever sees one.
//! - **The resume's roll.** The boot rolls the decoded arena onto a segment past both the live
//!   record's and the snapshot's (`super::boot`), so every opening writes its own log.
//! - **The host's half** ([`note_chunk`]). Each chunk is filed under the live record's segment,
//!   `field.<site>.<ordinal>.<segment>`. A chunk whose first line is a `segment` line for a
//!   later segment moves the live record onto it, written in the chunk's own batch, before the
//!   chunk is filed — so the chunk that opens a segment is that segment's first, read off the
//!   tape itself and not off a snapshot a newer one may replace in the task's queue.
//!
//! `seq`, the digest and the shipped digest run on across the roll: the segments' logs, read in
//! segment order, are the field's one chain under its one field id, every chunk numbered once.

use mush_core::Ctx;

use super::FieldBook;
use crate::arena::ledger::{EventKind, LedgerEvent};
use crate::arena_task::MatchId;

/// The bytes every `segment` line carries, read before a chunk's first line is parsed.
const SEGMENT_KIND: &[u8] = b"\"kind\":\"segment\"";

/// **A chunk of task `task`'s black box, before it is filed**: when its first line opens a later
/// segment than the live record's, the record moves onto it and is written.
pub(crate) fn note_chunk(ctx: &mut Ctx<'_>, task: MatchId, bytes: &[u8]) {
    let Some(segment) = opens(bytes) else {
        return;
    };
    let Some(book) = ctx.w.module_state_mut::<FieldBook>() else {
        return;
    };
    let Some(site) = book.site_of_task(task).cloned() else {
        return;
    };
    let Some(hosted) = book.live.get_mut(&site) else {
        return;
    };
    if segment <= hosted.live.segment {
        return;
    }
    hosted.live.segment = segment;
    let live = hosted.live.clone();
    super::record::write_live(ctx, &live);
    tracing::info!(site = %site, ordinal = live.field.ordinal, segment, "tacsim: an IC field's black box opened a new segment");
}

/// The segment a chunk's first line opens, when that line is a `segment` line.
fn opens(bytes: &[u8]) -> Option<u32> {
    let first = bytes.split(|byte| *byte == b'\n').next()?;
    if !first
        .windows(SEGMENT_KIND.len())
        .any(|window| window == SEGMENT_KIND)
    {
        return None;
    }
    match serde_json::from_slice::<LedgerEvent>(first).ok()?.kind {
        EventKind::Segment { segment } => Some(segment),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena::ledger::Ledger;
    use crate::config::LedgerConfig;

    #[test]
    fn fr4_a_rolled_chunk_opens_its_segment_and_no_other_chunk_does() {
        let mut ledger = Ledger::new(&LedgerConfig::default());
        ledger
            .append(0, EventKind::MatchStarted, Vec::new(), 1)
            .expect("room");
        ledger.flush();
        let first = ledger.take_chunks();
        assert_eq!(opens(&first[0].bytes), None);
        ledger.roll_segment(2, 5_000, 1).expect("room");
        ledger
            .append(5_100, EventKind::Aborted, Vec::new(), 1)
            .expect("room");
        ledger.flush();
        let second = ledger.take_chunks();
        assert_eq!(opens(&second[0].bytes), Some(2));
        assert_eq!(ledger.segment(), 2);
        assert_eq!(ledger.seq(), 3, "the sequence runs on through the roll");
    }
}
