//! The **staff fan-out for economy pages** (E6a step 1, DL-05; E4b1 deviation 5's
//! handoff).
//!
//! The ladder opens a level-2 record when a human must decide something, and E4b1
//! made its re-notification a loud log line plus a stamp — "the staff-channel fan-out
//! rides E4c's observer glue, which is where the server knows how to reach staff".
//! This is that glue. The economy now announces every step of the ladder's own
//! schedule as [`EconomicEvent::EscalationOpened`] — an open, a reopen after an
//! acknowledgement whose cause came back, and each reminder (1 h, then daily) — and
//! this observer turns each into one line to every connected Wizard-or-better, the
//! same audience the world loop's own `broadcast_wizards` reaches.
//!
//! **No storm.** The event carries no schedule of its own, and the observer invents
//! none: it reads the record and dedupes on the record's step — `(reopened,
//! notifications)` per escalation id — so a sweep that fires thirty seconds after a
//! reminder says nothing, and a page the ladder deduped to a standing record (no
//! event at all) says nothing either. The line is the record's own render plus its
//! first recommended command: the staffer is going to paste it into an LLM anyway,
//! so the alert should already be the answer (08c §4.6).

//! ## A page nobody could read is not a page
//!
//! The dedupe used to be recorded **before** the fan-out, and the fan-out's audience
//! can be empty: a page opened at three in the morning with no wizard connected was
//! consumed by its own dedupe, and the next thing anybody saw about it was a
//! re-notification an hour later saying "still open" about a record they had never
//! been told was opened (E6a step 1 verify fold, R4). The step is now recorded only
//! when somebody was actually told, and the economy re-announces its open records to
//! a Wizard-or-better on connect — so the first staffer to log in hears the backlog
//! once, and hears nothing they already read.

use std::collections::BTreeMap;
use std::sync::Mutex;

use mush_core::Ctx;
use mush_economy::{EconomicEvent, EconomyObserver, EconomyStore, EscalationId};

/// The most records the dedupe map remembers (E6a step 1 verify fold, D5). The
/// ladder's own GC collects an acknowledged record seven days after it is acked, and
/// this map never heard about it — so it grew, one entry per escalation the process
/// ever saw, for the life of the process.
pub const MAX_PAGED_RECORDS: usize = 4_096;

/// The DL-05 observer. Wire one into
/// [`EconomyModule::with_observers`](mush_economy::EconomyModule::with_observers).
#[derive(Debug, Default)]
pub struct StaffAlerts {
    /// The last `(reopened, notifications)` step **delivered to somebody** per record
    /// — RAM-only, so a restart pages a standing record at most once more, on its
    /// next step. Swept against the live ladder on every observation (D5).
    paged: Mutex<BTreeMap<EscalationId, (u32, u32)>>,
}

impl StaffAlerts {
    /// How many records the dedupe map is currently holding — the diagnostic the D5
    /// regression asserts against, since "it never shrinks" is invisible from the
    /// outside until a world has run for a month.
    #[must_use]
    pub fn tracked(&self) -> usize {
        self.paged.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Forget records the ladder no longer has, and cap what is left (D5).
    ///
    /// Called on each observation, which is the only moment this observer runs at
    /// all: an acknowledged escalation the sweep collected is gone from the store, so
    /// its dedupe entry is dead weight — and if a world somehow keeps more live
    /// records than the cap, the **lowest** ids go first, because the newest pages are
    /// the ones a staffer has not seen yet.
    fn sweep_paged(&self, ctx: &Ctx<'_>) {
        {
            // The overwhelmingly common case on a healthy world: nothing to sweep, so
            // an ordinary transaction pays a lock and a length check.
            let paged = self.paged.lock().unwrap_or_else(|e| e.into_inner());
            if paged.is_empty() {
                return;
            }
        }
        let live: std::collections::BTreeSet<EscalationId> = ctx
            .w
            .module_state::<EconomyStore>()
            .map(|s| s.escalations().map(|e| e.id).collect())
            .unwrap_or_default();
        let mut paged = self.paged.lock().unwrap_or_else(|e| e.into_inner());
        paged.retain(|id, _| live.contains(id));
        while paged.len() > MAX_PAGED_RECORDS {
            let Some(lowest) = paged.keys().next().copied() else {
                break;
            };
            paged.remove(&lowest);
        }
    }
}

impl EconomyObserver for StaffAlerts {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
        // D5: on **every** event, not only a page — the map's entries die when the
        // ladder collects their records, and the ladder collects them from the sweep,
        // which announces nothing. Guarded on an empty map, so a healthy world pays a
        // lock and a length check per transaction and nothing else.
        self.sweep_paged(ctx);
        let EconomicEvent::EscalationOpened { id, .. } = ev else {
            return;
        };
        let Some(record) = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.escalation(*id).cloned())
        else {
            return;
        };
        let step = (record.reopened, record.notifications);
        {
            // A poisoned lock is a panic in an earlier observer run; the fan-out is
            // a notice, never authority, so take the map as it is rather than skip
            // every page for the rest of the process.
            let paged = self.paged.lock().unwrap_or_else(|e| e.into_inner());
            if paged.get(id) == Some(&step) {
                return;
            }
        }
        let what = if record.reopened > 0 && record.notifications == 0 {
            "reopened"
        } else if record.notifications > 0 {
            "still open"
        } else {
            "opened"
        };
        let mut lines = vec![format!("ECONOMY ALERT ({what}): {}", record.render())];
        if let Some(first) = record.recommended.first() {
            lines.push(format!("  → {first}   (@econ/alerts #{id} for the rest)"));
        }
        let audience = ctx.w.connected_wizards();
        // R4: an empty audience is not a delivery. Leaving the step unrecorded is
        // what lets the economy's connect-time re-announcement page the first
        // staffer who arrives, instead of the ladder's next reminder telling them a
        // record is "still open" that they were never told had opened.
        if audience.is_empty() {
            return;
        }
        {
            let mut paged = self.paged.lock().unwrap_or_else(|e| e.into_inner());
            paged.insert(*id, step);
        }
        for wizard in audience {
            for line in &lines {
                ctx.out.sys_to_actor(wizard, "economy", line.clone());
            }
            // Interface plan I11: one notice per wizard per record on the
            // notification surface, opening the alert with the command ready.
            // The scrollback lines above stay the full text; the notice is the
            // headline (the server owns severity — an escalation reaching staff
            // is urgent, rule 5).
            if let Some(head) = lines.first() {
                ctx.notice_urgent(
                    wizard,
                    "alert",
                    head.clone(),
                    Some(("Open", &format!("@econ/alerts #{id}"))),
                );
            }
        }
        // Interface programme D5: the same headline on the staff relay sink
        // (the Discord staff channel, when mapped), once per record.
        if let Some(head) = lines.first() {
            let now_ms = ctx.w.now().saturating_mul(1000);
            ctx.out.push(
                mush_core::Audience::Relay(mush_proto::SmolStr::new(
                    mush_core::output::STAFF_RELAY_SINK,
                )),
                mush_proto::OutputEvent::Notice {
                    id: 0,
                    severity: mush_proto::NoticeSeverity::Urgent,
                    category: mush_proto::SmolStr::new("alert"),
                    text: mush_proto::RichText::plain(head.clone()),
                    action: None,
                    ts: now_ms,
                },
            );
        }
    }
}
