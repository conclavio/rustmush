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

use crate::ids::{AccountId, CadenceId, ContractId, EscalationId, HoldId, ObligationId};

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
    /// Boot — or, since E5b, a later world event — could not reconcile a record
    /// against the live world (a destroyed party, a closed account, a residence whose
    /// rooms went away) and stopped that one record rather than guessing.
    BootReconciliation,
    /// A **story window** has spent its notices and nobody has decided (E5b, 08d
    /// §4/§3.2). Automatic processing stopped at `EvictionPending` **by design**
    /// (D-E5-1: a home never ends on its own), so the only thing left that can move
    /// it is a person — which is precisely what this ladder is for.
    StoryWindow,
    /// An economic event could not be announced: the observer chain hit its depth
    /// cap and the event was dropped, so a chronicle line (a public settlement, a
    /// delinquency) never reached its audience (H4, E4c-D42; was `SettlementRefused`).
    AnnouncementDropped,
    /// A live canon condition cites a source no record can be found for anywhere
    /// the seam can read — the evidence the condition consented to is past recovery
    /// (H4, E4c-D42; was `SettlementRefused`).
    EvidenceUnavailable,
    /// An id allocator is exhausted, so a record that should exist could not be
    /// minted — a lease running with no rent record (H4, E4b2-D33 / E4c-D42).
    AllocatorExhausted,
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
            EscalationKind::StoryWindow => "story window",
            EscalationKind::AnnouncementDropped => "announcement dropped",
            EscalationKind::EvidenceUnavailable => "evidence unavailable",
            EscalationKind::AllocatorExhausted => "allocator exhausted",
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
    /// One **housing property** (E5b).
    ///
    /// The economy neither resolves nor interprets these two: they are the id of a
    /// record belonging to a module layered on top of it
    /// ([`crate::open_escalation`]). They live here rather than in a ladder of
    /// housing's own because the ladder is where the *dedupe*, the *reopen*, the
    /// acknowledgement and `@econ/alerts` are — 08a decision 25's operating rule is
    /// "one alert per decision a human actually has to make", and a second
    /// half-implemented ladder in every layered module is exactly how that becomes
    /// four alerts. The variant is `u64` rather than a housing type because the
    /// dependency runs housing → economy and must keep running that way.
    HousingProperty(u64),
    /// One **housing tenancy** (E5b) — see [`EscalationSubject::HousingProperty`].
    HousingTenancy(u64),
    /// One **world room** a housing quarantine is about (look-back fold, D-LBK-1: a
    /// room two properties both hold) — see [`EscalationSubject::HousingProperty`]
    /// for why it lives here. The `u32` is the raw object id.
    HousingRoom(u32),
    /// One **declared** cadence clock (SP1, D-LBK-7) whose persisted clock or period
    /// disagrees with its `[economy.cadences]` stanza. Boot adopted the clock as it
    /// is — configuration never rewrites a running clock — and this is the one page
    /// asking a human which side is right. Keyed by the cadence id, so one clock
    /// pages once however many boots see the mismatch.
    DeclaredCadence(CadenceId),
    /// **An announcement the observer chain dropped** (E6a step 1 verify fold, R8).
    ///
    /// It used to share [`EscalationSubject::Economy`] with the level-3 freeze
    /// record, and `open_freeze_record` opens nothing while *any* `Economy` record
    /// stands, acknowledged or not — so a dropped page (which nothing about the
    /// journal is wrong with) could suppress the one record that says the whole
    /// economy is frozen. A dropped announcement is its own kind of problem and gets
    /// its own subject; the ladder's per-subject dedupe still means a runaway chain
    /// opens one record and not a thousand.
    DroppedPage,
    /// One **work order** whose budget reservation is gone (E7 residue pass 1, R1).
    ///
    /// Like [`EscalationSubject::HousingProperty`], the economy neither resolves nor
    /// interprets it: it is the id of a record belonging to a module layered on top
    /// of the economy, and it lives here rather than in a ladder of `mush-industry`'s
    /// own for exactly the reason that variant gives — the dedupe, the reopen, the
    /// acknowledgement and `@econ/alerts` are all here. A vanished work-order hold is
    /// a **pause and a page, never a replay hit** (E7 cut 9 fold, E2(a)), and this is
    /// the page. The `u64` is the raw `WorkOrderId`, because the dependency runs
    /// industry → economy and must keep running that way.
    WorkOrder(u64),
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
            EscalationSubject::HousingProperty(id) => format!("housing property #{id}"),
            EscalationSubject::HousingTenancy(id) => format!("housing tenancy #{id}"),
            EscalationSubject::HousingRoom(id) => format!("housing room #{id}"),
            EscalationSubject::DeclaredCadence(id) => format!("declared cadence #{id}"),
            EscalationSubject::DroppedPage => "a dropped economy announcement".to_string(),
            EscalationSubject::WorkOrder(id) => format!("work order #{id}"),
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
    /// The account the refusal **named**, when it named one (E4b1 verify fold, B4c).
    ///
    /// One frozen wallet can block a dozen obligations, and paging once per
    /// obligation turns one staff decision into a dozen alerts saying the same thing.
    /// The blamed account is the real subject of that decision, so the ladder opens
    /// **one** record per blamed account per [`crate::ESCALATION_WINDOW_SECS`] and
    /// names the other affected records in its [`Escalation::context`].
    #[serde(default)]
    pub blamed: Option<AccountId>,
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
    /// How many times an acknowledgement was **overtaken by the cause coming back**
    /// (E4b1 verify fold, S4).
    ///
    /// Acknowledging an alert whose cause nobody fixed used to mint a brand-new
    /// record on the very next fire, so a staffer who acked without repointing got a
    /// fresh id every thirty seconds and the alert history became noise. The record is
    /// **reopened** instead — same id, same history — after a cool-off of
    /// [`crate::ESCALATION_RENOTIFY_SECS`], and this counter is how a reader tells
    /// "handled once" from "handled six times and still coming back".
    #[serde(default)]
    pub reopened: u32,
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
        clamp_context(&mut context);
        let mut recommended = recommended;
        recommended.truncate(MAX_RECOMMENDED_COMMANDS);
        Escalation {
            id,
            level,
            kind,
            subject,
            blamed: None,
            context,
            recommended,
            opened_at: now,
            acked_by: None,
            acked_at: None,
            notified_at: now,
            notifications: 0,
            reopened: 0,
            revision: 1,
        }
    }

    /// Name the account this record is really about (B4c) — the dedupe key for
    /// "one frozen wallet, one alert".
    #[must_use]
    pub fn blaming(mut self, account: Option<AccountId>) -> Escalation {
        self.blamed = account;
        self
    }

    /// Append one more affected record to the diagnosis, under the byte cap (B4c).
    /// A line that would not fit is dropped rather than truncated mid-sentence: the
    /// record already says what the decision is.
    pub fn note_also(&mut self, line: &str) {
        let addition = format!("; {line}");
        if self.context.len().saturating_add(addition.len()) > MAX_ESCALATION_CONTEXT_BYTES {
            return;
        }
        if self.context.contains(line) {
            return;
        }
        self.context.push_str(&addition);
        clamp_context(&mut self.context);
        self.revision = self.revision.saturating_add(1);
    }

    /// Whether an **acknowledged** record whose cause has come back should be
    /// reopened rather than replaced (S4): the cool-off keeps a refusal that repeats
    /// every thirty seconds from re-opening thirty seconds after it was handled.
    #[must_use]
    pub fn reopen_due(&self, now: i64) -> bool {
        self.acked_at
            .is_some_and(|at| now.saturating_sub(at) >= crate::ESCALATION_RENOTIFY_SECS)
    }

    /// Reopen this record in place — **same id**, same history, the acknowledgement
    /// cleared and the reopen tally bumped (S4).
    pub fn reopen(&mut self, now: i64) {
        self.acked_by = None;
        self.acked_at = None;
        self.notified_at = now;
        self.notifications = 0;
        self.reopened = self.reopened.saturating_add(1);
        self.revision = self.revision.saturating_add(1);
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
        let interval = if self.notifications == 0 {
            crate::ESCALATION_RENOTIFY_SECS
        } else {
            86_400
        };
        now.saturating_sub(self.notified_at) >= interval
    }

    /// The `@econ/alerts` line.
    #[must_use]
    pub fn render(&self) -> String {
        let state = match self.acked_by {
            None => String::new(),
            Some(who) => format!(" [acked by #{}]", who.0),
        };
        let again = if self.reopened == 0 {
            String::new()
        } else {
            format!(" [reopened {}×]", self.reopened)
        };
        format!(
            "#{} L{} {} — {}: {}{}{}",
            self.id,
            self.level,
            self.kind.label(),
            self.subject.render(),
            self.context,
            again,
            state
        )
    }
}

/// Bound a diagnosis, truncating on a char boundary so the doc always serializes
/// and always deserializes.
fn clamp_context(context: &mut String) {
    if context.len() > MAX_ESCALATION_CONTEXT_BYTES {
        let mut cut = MAX_ESCALATION_CONTEXT_BYTES;
        while cut > 0 && !context.is_char_boundary(cut) {
            cut -= 1;
        }
        context.truncate(cut);
    }
}

/// The `escalation:{id}` doc key.
#[must_use]
pub fn escalation_key(id: EscalationId) -> SmolStr {
    SmolStr::new(format!("escalation:{}", id.0))
}
