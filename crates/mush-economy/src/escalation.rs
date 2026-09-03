//! The graded escalation ladder (E4b1, 08c §4.6, D-E4-14).
//!
//! The owner's operating constraint (08a decision 25): a game has perhaps one
//! staffer who understands an economy alert, and they may be hours away. So the
//! economy **resolves programmatically wherever the best action is determinable**,
//! **pages a human only for a genuine decision**, **stops only the subject**, and
//! never stops the world for a local problem.
//!
//! | Level | Meaning | Economy action |
//! |-------|---------|----------------|
//! | 0 — resolve | the best action is determinable | do it, record it, notify once |
//! | 1 — inform  | worth an eventual glance | a line in `@econ/alerts`; nothing stops |
//! | 2 — page    | a human must decide | an [`Escalation`] record + suspend the SUBJECT |
//! | 3 — stop    | the journal cannot be trusted | the whole-economy freeze, plus a level-2 record carrying the diagnosis |
//!
//! Levels 0 and 1 leave no record of their own here (a level-0 resolution is an
//! `ObligationEvent::Forgiven` in the obligation's own history; a level-1 note is a
//! log line). Levels 2 and 3 mint an `escalation:{id}` **leaf** doc: an unreadable
//! one is skipped loudly at boot, never a boot failure, exactly like an approval
//! request — but its id still floors the allocator.
//!
//! Every record carries the **recommended command(s) with their arguments already
//! filled in**. The staffer is going to paste the alert into an LLM anyway, so the
//! alert should already be the answer.

use mush_core::ObjId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::{ContractId, EscalationId, HoldId, ObligationId};

/// The largest free-text diagnosis one record carries (a bound, like every other
/// stored string in the crate).
pub const MAX_ESCALATION_CONTEXT_BYTES: usize = 1024;
/// The most recommended commands one record carries.
pub const MAX_RECOMMENDED_COMMANDS: usize = 4;
/// How long an **acknowledged** record is kept before the sweep collects it (§9).
pub const ESCALATION_GC_AFTER_SECS: i64 = 7 * 86_400;
/// The most level-2 records one boot may open (a broken world must not mint a
/// thousand alerts; past this the subject is still suspended and the reason logged).
pub const MAX_BOOT_ESCALATIONS: usize = 16;

/// What kind of question is being put to a human.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EscalationKind {
    /// A settlement refusal only a human can fix: a closed/missing/frozen account, an
    /// inactive org compartment, a retired instrument, an approval rule the sweep
    /// cannot satisfy.
    SettlementRefused,
    /// [`crate::ForgiveCause::Overload`] twice inside [`crate::ESCALATION_WINDOW_SECS`]
    /// — a misconfigured cadence or a starved post budget.
    RepeatedOverload,
    /// A deterministic internal idempotency key resolved to a different plan (doc 09
    /// C8: an integrity incident, scoped to the smallest subject).
    KeyConflict,
    /// The whole-economy freeze is set: the sweep is doing nothing at all until staff
    /// clear it. The level-3 record (§4.6), carrying the diagnosis.
    EconomyFrozen,
    /// Boot could not reconcile a record against the live world (a destroyed party, a
    /// closed account) and suspended it rather than guessing.
    BootReconciliation,
}

impl EscalationKind {
    /// A short human word for `@econ/alerts`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            EscalationKind::SettlementRefused => "settlement refused",
            EscalationKind::RepeatedOverload => "repeated overload",
            EscalationKind::KeyConflict => "idempotency key conflict",
            EscalationKind::EconomyFrozen => "economy frozen",
            EscalationKind::BootReconciliation => "boot reconciliation",
        }
    }
}

/// What the escalation is *about* — the thing that was stopped, and nothing wider.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EscalationSubject {
    /// One obligation (suspended `Escalated(id)` while this record is open).
    Obligation(ObligationId),
    /// One reservation.
    Hold(HoldId),
    /// One contract (E4b2).
    Contract(ContractId),
    /// The whole economy — the level-3 case, and the only one that is not local.
    Economy,
}

impl EscalationSubject {
    /// How the subject renders in an alert line.
    #[must_use]
    pub fn render(self) -> String {
        match self {
            EscalationSubject::Obligation(id) => format!("obligation #{id}"),
            EscalationSubject::Hold(id) => format!("hold #{id}"),
            EscalationSubject::Contract(id) => format!("contract #{id}"),
            EscalationSubject::Economy => "the economy".to_string(),
        }
    }
}

/// One level-2 (or level-3) record: *a human must decide this, and here is the
/// command that decides it*.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Escalation {
    /// Identity (never reused; floored from every parsable `escalation:` key).
    pub id: EscalationId,
    /// 2 = page, 3 = stop (the whole-economy freeze's companion record).
    pub level: u8,
    /// What kind of question.
    pub kind: EscalationKind,
    /// What was stopped.
    pub subject: EscalationSubject,
    /// The diagnosis, in words (≤ [`MAX_ESCALATION_CONTEXT_BYTES`]).
    pub context: String,
    /// The recommended command(s), **arguments filled in** (≤
    /// [`MAX_RECOMMENDED_COMMANDS`]).
    pub recommended: Vec<String>,
    /// When it was opened (wall clock).
    pub opened_at: i64,
    /// Who acknowledged it, if anyone.
    pub acked_by: Option<ObjId>,
    /// When it was acknowledged.
    pub acked_at: Option<i64>,
    /// When it last re-notified staff (§4.6: 1 h, then daily).
    pub notified_at: i64,
    /// How many times it has re-notified.
    pub notifications: u32,
    /// Staleness stamp.
    pub revision: u64,
}

impl Escalation {
    /// Build a level-2 record, clamping both bounded fields.
    #[must_use]
    pub fn page(
        id: EscalationId,
        kind: EscalationKind,
        subject: EscalationSubject,
        context: impl Into<String>,
        recommended: Vec<String>,
        now: i64,
    ) -> Escalation {
        Escalation::at_level(2, id, kind, subject, context, recommended, now)
    }

    /// Build a record at an explicit level (3 is the whole-economy stop's companion).
    #[must_use]
    pub fn at_level(
        level: u8,
        id: EscalationId,
        kind: EscalationKind,
        subject: EscalationSubject,
        context: impl Into<String>,
        recommended: Vec<String>,
        now: i64,
    ) -> Escalation {
        let mut context: String = context.into();
        // Bounded like every other stored string: truncate on a char boundary so the
        // doc always serializes and always deserializes.
        if context.len() > MAX_ESCALATION_CONTEXT_BYTES {
            let mut cut = MAX_ESCALATION_CONTEXT_BYTES;
            while cut > 0 && !context.is_char_boundary(cut) {
                cut -= 1;
            }
            context.truncate(cut);
        }
        let mut recommended = recommended;
        recommended.truncate(MAX_RECOMMENDED_COMMANDS);
        Escalation {
            id,
            level,
            kind,
            subject,
            context,
            recommended,
            opened_at: now,
            acked_by: None,
            acked_at: None,
            notified_at: now,
            notifications: 0,
            revision: 1,
        }
    }

    /// Whether this record is still waiting on a human.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.acked_by.is_none()
    }

    /// Whether an open record is due to re-notify at `now`: the first reminder after
    /// [`crate::ESCALATION_RENOTIFY_SECS`] (1 h), then daily (§4.6). Bounded on
    /// purpose — an alert nobody answers must not become an alert nobody reads.
    #[must_use]
    pub fn renotify_due(&self, now: i64) -> bool {
        if !self.is_open() {
            return false;
        }
        let interval =
            if self.notifications == 0 { crate::ESCALATION_RENOTIFY_SECS } else { 86_400 };
        now.saturating_sub(self.notified_at) >= interval
    }

    /// The `@econ/alerts` line.
    #[must_use]
    pub fn render(&self) -> String {
        let state = match self.acked_by {
            None => String::new(),
            Some(who) => format!(" [acked by #{}]", who.0),
        };
        format!(
            "#{} L{} {} — {}: {}{}",
            self.id,
            self.level,
            self.kind.label(),
            self.subject.render(),
            self.context,
            state
        )
    }
}

/// The `escalation:{id}` doc key.
#[must_use]
pub fn escalation_key(id: EscalationId) -> SmolStr {
    SmolStr::new(format!("escalation:{}", id.0))
}
