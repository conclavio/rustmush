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

use std::collections::BTreeMap;
use std::sync::Mutex;

use mush_core::Ctx;
use mush_economy::{EconomicEvent, EconomyObserver, EconomyStore, EscalationId};

/// The DL-05 observer. Wire one into
/// [`EconomyModule::with_observers`](mush_economy::EconomyModule::with_observers).
#[derive(Debug, Default)]
pub struct StaffAlerts {
    /// The last `(reopened, notifications)` step paged per record — RAM-only, so a
    /// restart pages a standing record at most once more, on its next step.
    paged: Mutex<BTreeMap<EscalationId, (u32, u32)>>,
}

impl EconomyObserver for StaffAlerts {
    fn on_economic_event(&self, ctx: &mut Ctx<'_>, ev: &EconomicEvent) {
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
            let mut paged = self.paged.lock().unwrap_or_else(|e| e.into_inner());
            if paged.get(id) == Some(&step) {
                return;
            }
            paged.insert(*id, step);
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
        for wizard in crate::world_task::connected_wizards(ctx.w) {
            for line in &lines {
                ctx.out.sys_to_actor(wizard, "economy", line.clone());
            }
        }
    }
}
