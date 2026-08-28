//! The world scheduler (design doc §4.3): a pure, synchronous min-heap of
//! `(due_unix_ms, seq, kind)` entries.
//!
//! The scheduler holds **no clock and no futures** — the driving loop asks
//! [`Scheduler::next_due`] how long to sleep, feeds real time back in, and
//! calls [`Scheduler::pop_due`] to collect everything that has come due.
//! Ties on the deadline fire in insertion order (`seq` is the tiebreaker),
//! so same-second `@wait`s replay deterministically.
//!
//! Cancellation is lazy: [`Scheduler::cancel`] marks the entry dead and the
//! heap discards it when it surfaces, so cancel is O(1) and the heap never
//! needs rebuilding.

use std::cmp::{Ordering, Reverse};
use std::collections::{BinaryHeap, HashSet};

use smol_str::SmolStr;

use crate::obj::ObjId;

/// What fires when a scheduled deadline comes due.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScheduledKind {
    /// `@wait <secs>=<command>`: dispatch `line` as `actor` when due.
    WaitCmd {
        /// The actor the queued command runs as.
        actor: ObjId,
        /// The full command line to dispatch.
        line: String,
    },
    /// A recurring module tick (weather every 300s per zone, comsys
    /// housekeeping, …). The engine routes it to the owning module; no
    /// module consumes ticks in M1.
    ModuleTick {
        /// Registered module id, e.g. `"weather"`.
        module: SmolStr,
        /// Module-defined discriminator, e.g. a zone id.
        tag: SmolStr,
    },
    /// A named engine-internal control timer (garbage sweeps, checkpoints).
    Control(SmolStr),
}

/// Handle to one scheduled entry, for [`Scheduler::cancel`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TimerId(u64);

/// One heap entry. Ordered by `(due_ms, seq)` **only** — `seq` is unique,
/// so the order is total and `Eq` stays consistent with `Ord` even though
/// the payload is ignored.
#[derive(Debug)]
struct Entry {
    due_ms: i64,
    seq: u64,
    kind: ScheduledKind,
}

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        (self.due_ms, self.seq) == (other.due_ms, other.seq)
    }
}
impl Eq for Entry {}
impl PartialOrd for Entry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Entry {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.due_ms, self.seq).cmp(&(other.due_ms, other.seq))
    }
}

/// Min-heap scheduler over unix-millisecond deadlines.
#[derive(Debug, Default)]
pub struct Scheduler {
    /// `Reverse` turns std's max-heap into the min-heap we need.
    heap: BinaryHeap<Reverse<Entry>>,
    /// Seqs of entries still in the heap and not cancelled.
    pending: HashSet<u64>,
    /// Seqs cancelled but not yet lazily removed from the heap.
    cancelled: HashSet<u64>,
    /// Monotonic sequence / id source.
    seq: u64,
}

impl Scheduler {
    /// An empty scheduler.
    #[must_use]
    pub fn new() -> Scheduler {
        Scheduler::default()
    }

    /// Schedule `kind` to fire at `due_unix_ms`. Returns a handle for
    /// [`Scheduler::cancel`]. A deadline in the past is legal — it fires on
    /// the next [`Scheduler::pop_due`].
    pub fn insert(&mut self, due_unix_ms: i64, kind: ScheduledKind) -> TimerId {
        let seq = self.seq;
        self.seq += 1;
        self.pending.insert(seq);
        self.heap.push(Reverse(Entry {
            due_ms: due_unix_ms,
            seq,
            kind,
        }));
        TimerId(seq)
    }

    /// Cancel a scheduled entry. Returns `true` if it was still pending
    /// (`false` for already-fired or already-cancelled ids).
    pub fn cancel(&mut self, id: TimerId) -> bool {
        if self.pending.remove(&id.0) {
            self.cancelled.insert(id.0);
            true
        } else {
            false
        }
    }

    /// The earliest pending deadline (unix ms), or `None` when idle — the
    /// driving loop sleeps until this. Takes `&mut self` because cancelled
    /// entries surfacing at the top are discarded here (lazy cancellation);
    /// the live entries are untouched.
    pub fn next_due(&mut self) -> Option<i64> {
        self.prune_cancelled_head();
        self.heap.peek().map(|Reverse(e)| e.due_ms)
    }

    /// Remove and return every pending entry with `due_ms <= now_ms`, in
    /// `(due_ms, seq)` order. Cancelled entries are silently discarded.
    pub fn pop_due(&mut self, now_ms: i64) -> Vec<ScheduledKind> {
        let mut fired = Vec::new();
        loop {
            self.prune_cancelled_head();
            match self.heap.peek() {
                Some(Reverse(e)) if e.due_ms <= now_ms => {
                    let Reverse(e) = self.heap.pop().expect("peeked entry must pop");
                    self.pending.remove(&e.seq);
                    fired.push(e.kind);
                }
                _ => break,
            }
        }
        fired
    }

    /// Number of pending (not cancelled, not fired) entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether nothing is pending.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Drop cancelled entries sitting at the top of the heap.
    fn prune_cancelled_head(&mut self) {
        while let Some(Reverse(e)) = self.heap.peek() {
            if self.cancelled.contains(&e.seq) {
                let Reverse(e) = self.heap.pop().expect("peeked entry must pop");
                self.cancelled.remove(&e.seq);
            } else {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctl(name: &str) -> ScheduledKind {
        ScheduledKind::Control(SmolStr::new(name))
    }

    #[test]
    fn pops_in_due_order_regardless_of_insertion_order() {
        let mut s = Scheduler::new();
        s.insert(3_000, ctl("c"));
        s.insert(1_000, ctl("a"));
        s.insert(2_000, ctl("b"));
        assert_eq!(s.len(), 3);
        assert_eq!(s.next_due(), Some(1_000));
        let fired = s.pop_due(10_000);
        assert_eq!(fired, vec![ctl("a"), ctl("b"), ctl("c")]);
        assert!(s.is_empty());
        assert_eq!(s.next_due(), None);
    }

    #[test]
    fn ties_fire_in_insertion_order() {
        let mut s = Scheduler::new();
        s.insert(500, ctl("first"));
        s.insert(500, ctl("second"));
        s.insert(500, ctl("third"));
        assert_eq!(
            s.pop_due(500),
            vec![ctl("first"), ctl("second"), ctl("third")]
        );
    }

    #[test]
    fn pop_due_only_takes_entries_at_or_before_now() {
        let mut s = Scheduler::new();
        s.insert(1_000, ctl("early"));
        s.insert(2_000, ctl("exact"));
        s.insert(2_001, ctl("late"));
        assert_eq!(s.pop_due(999), Vec::<ScheduledKind>::new());
        assert_eq!(s.pop_due(2_000), vec![ctl("early"), ctl("exact")]);
        assert_eq!(s.len(), 1);
        assert_eq!(s.next_due(), Some(2_001));
        assert_eq!(s.pop_due(2_001), vec![ctl("late")]);
    }

    #[test]
    fn cancel_skips_entry_and_reports_status() {
        let mut s = Scheduler::new();
        let a = s.insert(1_000, ctl("a"));
        let b = s.insert(2_000, ctl("b"));
        let c = s.insert(3_000, ctl("c"));
        assert!(s.cancel(b));
        assert!(!s.cancel(b), "double cancel is false");
        assert_eq!(s.len(), 2);
        assert_eq!(s.pop_due(5_000), vec![ctl("a"), ctl("c")]);
        // Fired ids cannot be cancelled.
        assert!(!s.cancel(a));
        assert!(!s.cancel(c));
    }

    #[test]
    fn cancelling_the_earliest_updates_next_due() {
        let mut s = Scheduler::new();
        let a = s.insert(1_000, ctl("a"));
        let b = s.insert(2_000, ctl("b"));
        assert_eq!(s.next_due(), Some(1_000));
        assert!(s.cancel(a));
        assert_eq!(s.next_due(), Some(2_000));
        // Cancel everything: idle.
        assert!(s.cancel(b));
        assert_eq!(s.next_due(), None);
        assert!(s.is_empty());
    }

    #[test]
    fn past_deadlines_fire_immediately() {
        let mut s = Scheduler::new();
        s.insert(-5, ctl("overdue"));
        assert_eq!(s.pop_due(0), vec![ctl("overdue")]);
    }

    #[test]
    fn wait_cmd_and_module_tick_round_trip() {
        let mut s = Scheduler::new();
        s.insert(
            10,
            ScheduledKind::WaitCmd {
                actor: ObjId(5),
                line: "say hi".into(),
            },
        );
        s.insert(
            5,
            ScheduledKind::ModuleTick {
                module: "weather".into(),
                tag: "zone-3".into(),
            },
        );
        let fired = s.pop_due(10);
        assert_eq!(
            fired,
            vec![
                ScheduledKind::ModuleTick {
                    module: "weather".into(),
                    tag: "zone-3".into(),
                },
                ScheduledKind::WaitCmd {
                    actor: ObjId(5),
                    line: "say hi".into(),
                },
            ]
        );
    }
}
