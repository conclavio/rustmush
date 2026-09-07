//! The operation record (E7 cut 9 B2/B3; 10b §3.9, Q5D, D-E7-18).
//!
//! One logical operation is **one document** at `op:<id>` with two separately
//! versioned halves: [`OperationTerms`], frozen at activation and hashed through
//! `mush_economy::consent_digest`, and [`OperationRuntime`], mutable. **A roster
//! edit never changes the terms hash** — a reinforcement is not an amendment to
//! what the defenders consented to be told (E4b2's lesson, re-learned nowhere).
//!
//! ## Rule 1 (§4)
//!
//! Every persisted container here re-enters through a `TryFrom` wire that re-runs
//! the constructor invariants. **A stored set over any cap refuses on load, never
//! truncates**, and the refused document is *held* rather than deleted (cut 6's
//! `JourneyBook::held` precedent) — an operation holding a live money bond is the
//! only record of that bond's purpose.
//!
//! ## Rule 1 category
//!
//! **Exempt** — a bounded whole-document *workflow* record with strict decoding, a
//! deterministic key, a declared future family (`campaign.operation`) and **no part
//! in authority or prepared mechanics**. The bond itself is E4's `Hold` on E4's own
//! lane (the economy carve-out); `campaign` persists only the `HoldId`.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{ClockStamp, ObjId};
use mush_economy::{AccountId, Amount, EconomicVisibility, HoldId, MAX_AUDIENCE};
use mush_org::PrincipalId;
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::ids::{FormationId, OperationId};

// ---- caps (10b §7; cited where they are the economy's) --------------------

/// The largest participant roster one operation may carry (10b §7).
pub const MAX_OPERATION_PARTICIPANTS: usize = 128;
/// The minimum notice a declared operation must give its eligible defenders
/// (10b §7, S5A): 24 hours of IC time.
pub const OPERATION_NOTICE_MIN: i64 = 24 * 3_600;
/// The maximum notice (10b §7, S5A): 72 hours. Past it the "declaration" stops
/// being a window anyone can plan around.
pub const OPERATION_NOTICE_MAX: i64 = 72 * 3_600;
/// The largest legal-target class list one operation may declare.
pub const MAX_LEGAL_TARGETS: usize = 16;
/// The largest stakes list one operation may declare. Deliberately equal to
/// `mush_economy::MAX_OUTCOMES` (8) — a stake that cannot be an outcome row on a
/// linked contract is a stake nothing can settle.
pub const MAX_STAKES: usize = 8;
/// The largest number of sides one operation may name.
pub const MAX_SIDES: usize = 8;
/// The largest objective summary, in bytes.
pub const MAX_OBJECTIVE_BYTES: usize = 240;
/// The largest outcome name, in bytes.
pub const MAX_OUTCOME_NAME_BYTES: usize = 64;
/// The largest label (a side's name, a target class, a control kind), in bytes.
pub const MAX_LABEL_BYTES: usize = 64;
/// How many times one consumer may refuse before the operation pauses and pages.
pub const MAX_CONSUMER_ATTEMPTS: u32 = 3;

/// A unit committed to an operation. Opaque here on purpose: `mush-campaign` has
/// **no `mush-sim` or `mush-tactics` edge**, so a unit is an object id and the
/// crate that owns unit truth stays the only one that can say anything else.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnitRef(pub ObjId);

/// What an operation is trying to achieve, evaluated by the campaign tick against
/// **durable state** — a pull, never an observer and never a schedule source (the
/// kernel fact cut 6 established: a `schedule_source`-fired action has no world).
///
/// Every arm is answerable from the four crates cut 9 depends on, which is why
/// there is no `tactics.*` arm: reading one would need an edge this crate does not
/// have. Cut 10's participation facts are the seam for that.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "objective", rename_all = "snake_case", deny_unknown_fields)]
pub enum ObjectiveKind {
    /// The declared window has closed on the world IC clock.
    WindowElapsed,
    /// At least `minimum` units are committed when the window closes.
    Muster {
        /// How many.
        minimum: u16,
    },
    /// A canon record of `kind` naming `source` exists. This is how a domain that
    /// `mush-campaign` cannot depend on discharges an objective: it records canon,
    /// and the campaign tick reads canon's own durable ledger.
    CanonRecorded {
        /// The canon event kind.
        kind: SmolStr,
        /// The specific source it must name (never a wildcard — E4c verify fold
        /// E2's rule, applied here).
        source: SmolStr,
    },
}

/// The objective, with the one-line summary a viewer entitled to the plan sees.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectiveSpec {
    /// What is evaluated.
    pub kind: ObjectiveKind,
    /// The human line (≤ [`MAX_OBJECTIVE_BYTES`]).
    pub summary: SmolStr,
}

/// One consequence an operation's resolution may carry (T6: "tactical facts settle
/// the result into front and control changes").
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Consequence {
    /// A front/control change over `(region, control)`. **Opaque here**: cut 9 has
    /// no `mush-faction` edge and writes no `ControlRecord`. Naming one is what
    /// makes [`ConsumerId::FrontControl`] required, and the consumer is discharged
    /// by committing the ONE root canon record — `mush-faction`'s own
    /// `on_canon_recorded` → `clocks_watching` → `on_complete_control` does the
    /// rest, for whichever front staff bound with `@clock/set on:`.
    Front {
        /// The region id, as `mush-faction` numbers them.
        region: u64,
        /// The control kind, as `mush-faction` names them.
        control: SmolStr,
    },
    /// An outcome row on a linked E4 contract.
    Outcome {
        /// The contract.
        contract: u64,
        /// The row name (≤ [`MAX_OUTCOME_NAME_BYTES`]).
        row: SmolStr,
    },
}

/// One side of a declared operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SideSpec {
    /// The side's name (≤ [`MAX_LABEL_BYTES`]).
    pub name: SmolStr,
    /// Who commands it.
    pub principal: PrincipalId,
}

/// Who is entitled to defend (T6's "eligible defenders" class). Bounded by the
/// economy's own [`MAX_AUDIENCE`], **cited not redefined**.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AudienceSpec {
    /// The principals entitled to defend (≤ `mush_economy::MAX_AUDIENCE`).
    pub members: Vec<PrincipalId>,
}

/// The funded bond (doc 10 §12 item 7A; T4). It is an **E4 money `Hold` on the
/// attacker's own account** and nothing else: cut 9 writes no `Amount` arithmetic,
/// no `create_system_obligation`, no credit line and no faucet.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bond {
    /// The attacker's own account the hold is placed against.
    pub account: AccountId,
    /// How much is reserved.
    pub amount: Amount,
    /// Who takes the bond if the operation forfeits, and into which account. When
    /// this is `None` the reservation is for nobody in particular and the engine's
    /// own `authorize_hold` makes it **releasable only** — which is the safe
    /// default, not an oversight.
    pub forfeit_to: Option<(PrincipalId, AccountId)>,
    /// The placed hold. `None` until `@op/activate` posts the `HoldOp::Place`.
    ///
    /// **Deliberately outside the hashed canonical form** ([`crate::terms`]): what
    /// the parties consent to is *a bond of `amount` on `account`*, and the hold id
    /// is the execution record of that consent. Were it hashed, activation — which
    /// is the moment the terms freeze — would move the hash it had just frozen.
    #[serde(default)]
    pub hold: Option<HoldId>,
}

/// The frozen half of an operation (T6: "the bond, window, eligible defenders,
/// legal targets, objectives and consequences **freeze at activation**").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OperationTermsWire", deny_unknown_fields)]
pub struct OperationTerms {
    /// The operation.
    pub operation: OperationId,
    /// Who declared it. Also the principal the bond is placed by, so it is the
    /// principal a release posts under.
    ///
    /// **It is not the command authority.** `commands::may_amend` / `may_roster` /
    /// `may_resolve` ask that question (E7 cut 9 fold, C1): the disclosure lens says
    /// who may *see* an operation, and it deliberately admits parties — a named side,
    /// a committed unit's controller — who may not *move* it.
    pub attacker: PrincipalId,
    /// What it is trying to achieve.
    pub objective: ObjectiveSpec,
    /// What it may lawfully be aimed at (≤ [`MAX_LEGAL_TARGETS`]).
    pub legal_targets: Vec<SmolStr>,
    /// What its resolution causes (≤ [`MAX_STAKES`]).
    pub stakes: Vec<Consequence>,
    /// The funded bond, if any.
    pub bond: Option<Bond>,
    /// The sides (≤ [`MAX_SIDES`]).
    pub sides: Vec<SideSpec>,
    /// Who may defend (≤ `mush_economy::MAX_AUDIENCE`).
    pub eligible_defenders: AudienceSpec,
    /// When notice was given.
    pub notice: ClockStamp,
    /// The vulnerability window, on the same clock as `notice`.
    pub window: (ClockStamp, ClockStamp),
    /// Who may see it (T6's classes, carried by the economy's own type).
    pub visibility: EconomicVisibility,
    /// The frozen consent hash over everything above but `bond.hold` — computed by
    /// `mush_economy::consent_digest` under `"campaign.operation.v1"`, the SAME
    /// discipline `consent_hash` uses, never a fork (D-E7-32).
    pub consent_hash: [u8; 32],
}

/// What a set of declared terms failed on. Every arm is a **refusal on load** as
/// well as a refusal at construction: the wire runs the same check.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum TermsError {
    /// A set is over its documented cap.
    #[error("{what} is over its cap ({actual} > {max})")]
    TooMany {
        /// Which set.
        what: &'static str,
        /// How many it held.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// A label or summary is over its byte cap.
    #[error("{what} is longer than {max} bytes")]
    TooLong {
        /// Which text.
        what: &'static str,
        /// The cap.
        max: usize,
    },
    /// A label is empty.
    #[error("{what} is empty")]
    Empty {
        /// Which text.
        what: &'static str,
    },
    /// The notice and the window are not on one clock generation, so they cannot
    /// be compared at all (the tree's rule everywhere else).
    #[error("the notice and the window are on different clocks")]
    ClockMismatch,
    /// The window opens before it was noticed, or closes before it opens.
    #[error("the window is not after the notice, or does not last")]
    WindowOrder,
    /// The notice period is outside `OPERATION_NOTICE_MIN..=OPERATION_NOTICE_MAX`.
    #[error("notice of {actual}s is outside {min}s..={max}s")]
    NoticeOutOfRange {
        /// What was declared.
        actual: i64,
        /// The floor.
        min: i64,
        /// The ceiling.
        max: i64,
    },
    /// A bond of zero or a negative amount.
    #[error("a bond must reserve a positive amount")]
    BondAmount,
    /// The operation id is zero.
    #[error("an operation id is never zero")]
    ZeroId,
}

fn check_label(what: &'static str, text: &str) -> Result<(), TermsError> {
    if text.trim().is_empty() {
        return Err(TermsError::Empty { what });
    }
    if text.len() > MAX_LABEL_BYTES {
        return Err(TermsError::TooLong {
            what,
            max: MAX_LABEL_BYTES,
        });
    }
    Ok(())
}

impl OperationTerms {
    /// Re-run every documented invariant. This is the constructor the wire calls,
    /// so a stored document and a freshly declared one are checked identically.
    pub fn check(&self) -> Result<(), TermsError> {
        if self.operation.0 == 0 {
            return Err(TermsError::ZeroId);
        }
        if self.objective.summary.len() > MAX_OBJECTIVE_BYTES {
            return Err(TermsError::TooLong {
                what: "the objective summary",
                max: MAX_OBJECTIVE_BYTES,
            });
        }
        if self.objective.summary.trim().is_empty() {
            return Err(TermsError::Empty {
                what: "the objective summary",
            });
        }
        if let ObjectiveKind::CanonRecorded { kind, source } = &self.objective.kind {
            check_label("an objective canon kind", kind)?;
            check_label("an objective canon source", source)?;
        }
        if self.legal_targets.len() > MAX_LEGAL_TARGETS {
            return Err(TermsError::TooMany {
                what: "legal targets",
                actual: self.legal_targets.len(),
                max: MAX_LEGAL_TARGETS,
            });
        }
        for target in &self.legal_targets {
            check_label("a legal target class", target)?;
        }
        if self.stakes.len() > MAX_STAKES {
            return Err(TermsError::TooMany {
                what: "stakes",
                actual: self.stakes.len(),
                max: MAX_STAKES,
            });
        }
        for stake in &self.stakes {
            match stake {
                Consequence::Front { control, .. } => check_label("a control kind", control)?,
                Consequence::Outcome { row, .. } => {
                    if row.trim().is_empty() {
                        return Err(TermsError::Empty {
                            what: "an outcome row",
                        });
                    }
                    if row.len() > MAX_OUTCOME_NAME_BYTES {
                        return Err(TermsError::TooLong {
                            what: "an outcome row",
                            max: MAX_OUTCOME_NAME_BYTES,
                        });
                    }
                }
            }
        }
        if self.sides.len() > MAX_SIDES {
            return Err(TermsError::TooMany {
                what: "sides",
                actual: self.sides.len(),
                max: MAX_SIDES,
            });
        }
        for side in &self.sides {
            check_label("a side name", &side.name)?;
        }
        if self.eligible_defenders.members.len() > MAX_AUDIENCE {
            return Err(TermsError::TooMany {
                what: "eligible defenders",
                actual: self.eligible_defenders.members.len(),
                max: MAX_AUDIENCE,
            });
        }
        if self.visibility.members().len() > MAX_AUDIENCE {
            return Err(TermsError::TooMany {
                what: "the visibility audience",
                actual: self.visibility.members().len(),
                max: MAX_AUDIENCE,
            });
        }
        if let Some(bond) = &self.bond
            && (bond.amount.is_negative() || bond.amount.is_zero())
        {
            return Err(TermsError::BondAmount);
        }
        self.check_window()
    }

    /// The notice/window half, split out so `@op/terms` can report it alone.
    ///
    /// **Two stamps are only ever compared on the same clock** — the rule the whole
    /// tree keeps — so a notice and a window on different generations is a refusal,
    /// not a silent `i64` subtraction.
    pub fn check_window(&self) -> Result<(), TermsError> {
        let (open, close) = &self.window;
        if self.notice.clock != open.clock || open.clock != close.clock {
            return Err(TermsError::ClockMismatch);
        }
        if open.at <= self.notice.at || close.at <= open.at {
            return Err(TermsError::WindowOrder);
        }
        let given = open.at.saturating_sub(self.notice.at);
        if !(OPERATION_NOTICE_MIN..=OPERATION_NOTICE_MAX).contains(&given) {
            return Err(TermsError::NoticeOutOfRange {
                actual: given,
                min: OPERATION_NOTICE_MIN,
                max: OPERATION_NOTICE_MAX,
            });
        }
        Ok(())
    }

    /// The consumer set this operation's resolution requires, **computed from the
    /// frozen terms** (B7). Called exactly once, at entry to `Resolving`, and
    /// stored; never re-derived afterwards, because that is the whole crash-safety
    /// argument.
    #[must_use]
    pub fn required_consumers(&self) -> BTreeSet<ConsumerId> {
        let mut out = BTreeSet::new();
        out.insert(ConsumerId::CampaignOutcome);
        out.insert(ConsumerId::ForceRetirement);
        if self
            .stakes
            .iter()
            .any(|s| matches!(s, Consequence::Front { .. }))
        {
            out.insert(ConsumerId::FrontControl);
        }
        if self.bond.is_some() {
            out.insert(ConsumerId::BondSettlement);
        }
        if self
            .stakes
            .iter()
            .any(|s| matches!(s, Consequence::Outcome { .. }))
        {
            out.insert(ConsumerId::ContractOutcomes);
        }
        out
    }
}

/// The strict wire for [`OperationTerms`] (§4 rule 1).
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationTermsWire {
    operation: OperationId,
    attacker: PrincipalId,
    objective: ObjectiveSpec,
    legal_targets: Vec<SmolStr>,
    stakes: Vec<Consequence>,
    #[serde(default)]
    bond: Option<Bond>,
    sides: Vec<SideSpec>,
    eligible_defenders: AudienceSpec,
    notice: ClockStamp,
    window: (ClockStamp, ClockStamp),
    visibility: EconomicVisibility,
    consent_hash: [u8; 32],
}

impl TryFrom<OperationTermsWire> for OperationTerms {
    type Error = TermsError;

    fn try_from(w: OperationTermsWire) -> Result<Self, Self::Error> {
        let terms = OperationTerms {
            operation: w.operation,
            attacker: w.attacker,
            objective: w.objective,
            legal_targets: w.legal_targets,
            stakes: w.stakes,
            bond: w.bond,
            sides: w.sides,
            eligible_defenders: w.eligible_defenders,
            notice: w.notice,
            window: w.window,
            visibility: w.visibility,
            consent_hash: w.consent_hash,
        };
        terms.check()?;
        Ok(terms)
    }
}

/// The **closed** consumer set of a terminal resolution (Q17A, D-E7-20). Five, and
/// exactly five: `Resolving { pending }` is only meaningful if the set is closed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsumerId {
    /// The outcome row recorded on the operation — always.
    CampaignOutcome,
    /// The bond released or captured — iff there is a bond.
    BondSettlement,
    /// A linked contract's outcome selection — iff a stake names one.
    ContractOutcomes,
    /// The front/control consequence — iff a stake names a `(region, control)`.
    FrontControl,
    /// The task force dissolved and its units released — always.
    ForceRetirement,
}

impl ConsumerId {
    /// Every consumer this build knows, in the order the fan-out advances them.
    /// Bond settlement runs **before** force retirement so that a released unit can
    /// never be the reason a capture could not be attributed.
    pub const ALL: [ConsumerId; 5] = [
        ConsumerId::CampaignOutcome,
        ConsumerId::BondSettlement,
        ConsumerId::ContractOutcomes,
        ConsumerId::FrontControl,
        ConsumerId::ForceRetirement,
    ];

    /// The stable token used in an idempotency key and in `@op/consumers`.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            ConsumerId::CampaignOutcome => "campaign_outcome",
            ConsumerId::BondSettlement => "bond_settlement",
            ConsumerId::ContractOutcomes => "contract_outcomes",
            ConsumerId::FrontControl => "front_control",
            ConsumerId::ForceRetirement => "force_retirement",
        }
    }

    /// Parse a stored token. `None` is an id **this build does not know** — never
    /// dropped: the runtime keeps it, refuses to reach `Resolved`, and pages.
    #[must_use]
    pub fn parse(text: &str) -> Option<ConsumerId> {
        ConsumerId::ALL.into_iter().find(|c| c.as_str() == text)
    }
}

/// Where an operation is in its life (10b §3.9).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationState {
    /// Declared; terms still editable; no bond placed.
    Declared,
    /// Activated: the bond is placed and the terms are frozen.
    Activated,
    /// Inside the window.
    Running,
    /// The root fact is written and the consumers are draining.
    Resolving {
        /// Which consumers this build still owes, of the set computed once at
        /// entry from the frozen terms.
        pending: BTreeSet<ConsumerId>,
    },
    /// Every consumer acknowledged. **Never re-opens.**
    Resolved,
    /// Abandoned before resolution; a placed bond is released by the same fan-out.
    Aborted,
}

impl OperationState {
    /// The staff-facing word.
    #[must_use]
    pub fn label(&self) -> &'static str {
        match self {
            OperationState::Declared => "declared",
            OperationState::Activated => "activated",
            OperationState::Running => "running",
            OperationState::Resolving { .. } => "resolving",
            OperationState::Resolved => "resolved",
            OperationState::Aborted => "aborted",
        }
    }

    /// Whether the operation is still live enough for the tick to work on it.
    ///
    /// `Aborted` is terminal exactly as `Resolved` is (E7 cut 9 fold, D7): before the
    /// fold nothing ever *constructed* it, so `settle::abandon` routed an abandonment
    /// through `Resolving` to `Resolved` and `@op/show` reported "resolved" for an
    /// operation staff had abandoned past a corrupt surface.
    #[must_use]
    pub fn is_live(&self) -> bool {
        !matches!(self, OperationState::Resolved | OperationState::Aborted)
    }

    /// Whether the operation has finished, either way. Nothing re-opens one.
    #[must_use]
    pub fn is_terminal(&self) -> bool {
        matches!(self, OperationState::Resolved | OperationState::Aborted)
    }
}

/// One unit's participation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Participation {
    /// The unit.
    pub unit: UnitRef,
    /// Which side it fights on.
    pub side: SmolStr,
    /// Its **one** operational command parent inside this operation (R1). `None`
    /// is a unit under the operation's own command, which is still exactly one
    /// parent.
    #[serde(default)]
    pub command_parent: Option<FormationId>,
    /// When it joined (wall-clock unix seconds, for audit only).
    pub joined_at: i64,
}

/// The operation-scoped command overlay (T8). Administrative parentage is
/// **unchanged** by attachment; this is the temporary tree beside it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskForce {
    /// The formations attached to this operation, in id order.
    pub formations: Vec<FormationId>,
}

/// The applied marker the `campaign.*` fact provider answers from (08c §6.2: **an
/// applied marker, never a status**). It exists only once the root canon record has
/// actually been committed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootFact {
    /// The canon id, when canon was loaded and accepted the draft.
    #[serde(default)]
    pub canon: Option<u64>,
    /// Wall-clock second the root was recorded.
    pub at: i64,
    /// The outcome row it recorded (≤ [`MAX_OUTCOME_NAME_BYTES`]).
    ///
    /// `RootFact` has no strict wire of its own, so that cap is enforced by
    /// [`OperationRuntime::check`] — which the load wire re-runs. Before the E7 cut 9
    /// independent verify fold (B7) it was enforced **nowhere**: `op_resolve` took
    /// `ctx.args.rhs()` verbatim, the same bytes reached canon's immutable ledger,
    /// and the caller's input line was the only bound on a document that has no size
    /// cap of its own. The command now matches a closed vocabulary as well.
    pub outcome: SmolStr,
}

/// Why an operation is paused. A pause **stops only the subject** (08c §4.6) and
/// is always accompanied by a level-2 page with the command filled in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "why", rename_all = "snake_case", deny_unknown_fields)]
pub enum PauseReason {
    /// The bond could not be placed or the account no longer funds it. Never
    /// negative, never continuing unfunded.
    Unfunded,
    /// A stored consumer id this build does not know. Fails closed.
    UnknownConsumer,
    /// A consumer refused and kept refusing.
    ConsumerFailed {
        /// Which one.
        consumer: ConsumerId,
    },
    /// The frozen surface no longer hashes: the operation **disputes and pages
    /// rather than settling** (lens Z).
    TermsBroken,
}

impl PauseReason {
    /// The one-line staff word.
    #[must_use]
    pub fn label(&self) -> String {
        match self {
            PauseReason::Unfunded => "unfunded".to_owned(),
            PauseReason::UnknownConsumer => "unknown-consumer".to_owned(),
            PauseReason::ConsumerFailed { consumer } => {
                format!("consumer-failed:{}", consumer.as_str())
            }
            PauseReason::TermsBroken => "terms-broken".to_owned(),
        }
    }
}

/// The mutable half (10b §3.9). Separately versioned from the terms: `revision`
/// moves on every roster edit and **the terms hash does not**.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "OperationRuntimeWire", into = "OperationRuntimeWire")]
pub struct OperationRuntime {
    /// The operation.
    pub operation: OperationId,
    /// Who is committed (≤ [`MAX_OPERATION_PARTICIPANTS`]).
    pub participants: Vec<Participation>,
    /// The command overlay.
    pub force: TaskForce,
    /// Units withdrawn (audit; a withdrawn unit is free to join another operation).
    pub withdrawals: Vec<UnitRef>,
    /// Where it is.
    pub state: OperationState,
    /// The consumer set computed **once** at entry to `Resolving`.
    pub required: BTreeSet<ConsumerId>,
    /// Stored consumer ids this build does not know. Non-empty means the operation
    /// can never reach `Resolved` and has paged (B7). It is the union of what the
    /// stored `required` and `pending` lists carried, so the operation blocks and
    /// pages however the id arrived.
    pub unknown_consumers: BTreeSet<SmolStr>,
    /// The half of [`OperationRuntime::unknown_consumers`] the stored **pending**
    /// list carried, kept separately so a round trip through a build that does not
    /// know the id **writes back what it read** (E7 cut 9 fold, B11). Unioning one
    /// set into both lists resurrected an unknown consumer a knowing build had
    /// already discharged.
    pub unknown_pending: BTreeSet<SmolStr>,
    /// The applied root marker.
    pub root: Option<RootFact>,
    /// Why it is paused, if it is.
    pub paused: Option<PauseReason>,
    /// Whether this resolution is an explicit **staff abandonment**, set by
    /// [`crate::settle::abandon`] and by nothing else (E7 cut 9 fold, C7).
    ///
    /// Before the fold the fan-out asked `outcome_abandons(&root.outcome)` — the
    /// outcome *string* — so any `Lens::Plan` holder could pre-disable the mid-drain
    /// intactness re-check simply by choosing the word `abort`. The flag is written
    /// by the transition that took it, so choosing an outcome cannot set it.
    pub abandoned: bool,
    /// How many times each consumer has refused, durably. A consumer past
    /// [`MAX_CONSUMER_ATTEMPTS`] pauses **this** operation and pages level 2 — it
    /// never freezes the lane, so one operation's wall of refusals cannot starve
    /// another's fan-out (08c §5.2 B4's shape).
    pub attempts: BTreeMap<SmolStr, u32>,
    /// Staleness stamp.
    pub revision: u64,
}

/// What a runtime failed on.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    /// A set over its cap.
    #[error("{what} is over its cap ({actual} > {max})")]
    TooMany {
        /// Which set.
        what: &'static str,
        /// How many.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// Two participations name one unit.
    #[error("a unit is listed twice in one operation")]
    DuplicateUnit,
    /// A text is over its byte cap.
    #[error("{what} is longer than {max} bytes")]
    TooLong {
        /// Which text.
        what: &'static str,
        /// The cap.
        max: usize,
    },
    /// `pending` holds an id the required set does not.
    #[error("a pending consumer is not in the required set")]
    PendingNotRequired,
    /// The operation id is zero.
    #[error("an operation id is never zero")]
    ZeroId,
}

impl OperationRuntime {
    /// A fresh runtime for a newly declared operation.
    #[must_use]
    pub fn declared(operation: OperationId) -> OperationRuntime {
        OperationRuntime {
            operation,
            participants: Vec::new(),
            force: TaskForce::default(),
            withdrawals: Vec::new(),
            state: OperationState::Declared,
            required: BTreeSet::new(),
            unknown_consumers: BTreeSet::new(),
            unknown_pending: BTreeSet::new(),
            root: None,
            paused: None,
            abandoned: false,
            attempts: BTreeMap::new(),
            revision: 1,
        }
    }

    /// Re-run every documented invariant (the wire calls this).
    pub fn check(&self) -> Result<(), RuntimeError> {
        if self.operation.0 == 0 {
            return Err(RuntimeError::ZeroId);
        }
        if self.participants.len() > MAX_OPERATION_PARTICIPANTS {
            return Err(RuntimeError::TooMany {
                what: "participants",
                actual: self.participants.len(),
                max: MAX_OPERATION_PARTICIPANTS,
            });
        }
        if self.withdrawals.len() > MAX_OPERATION_PARTICIPANTS {
            return Err(RuntimeError::TooMany {
                what: "withdrawals",
                actual: self.withdrawals.len(),
                max: MAX_OPERATION_PARTICIPANTS,
            });
        }
        if self.force.formations.len() > crate::formation::MAX_FORMATIONS_PER_OPERATION {
            return Err(RuntimeError::TooMany {
                what: "attached formations",
                actual: self.force.formations.len(),
                max: crate::formation::MAX_FORMATIONS_PER_OPERATION,
            });
        }
        // D11: `attempts` is keyed by a CLOSED five-element vocabulary, so the cap
        // is exact. Every other bounded set in this record refuses on load; before
        // the fold this one had no bound to refuse against, and a corrupt document
        // carrying a million attempt entries was accepted at boot and re-serialized
        // on every write.
        if self.attempts.len() > ConsumerId::ALL.len() {
            return Err(RuntimeError::TooMany {
                what: "consumer attempts",
                actual: self.attempts.len(),
                max: ConsumerId::ALL.len(),
            });
        }
        // B7: the outcome row is documented "<= MAX_OUTCOME_NAME_BYTES" and was
        // bounded nowhere - `RootFact` has no wire of its own, so the caller's input
        // line was the only limit on a document with no size cap, and the same bytes
        // reached canon's immutable ledger.
        if let Some(root) = &self.root
            && root.outcome.len() > MAX_OUTCOME_NAME_BYTES
        {
            return Err(RuntimeError::TooLong {
                what: "the recorded outcome",
                max: MAX_OUTCOME_NAME_BYTES,
            });
        }
        let mut seen: BTreeSet<UnitRef> = BTreeSet::new();
        for p in &self.participants {
            if !seen.insert(p.unit) {
                return Err(RuntimeError::DuplicateUnit);
            }
        }
        if let OperationState::Resolving { pending } = &self.state
            && !pending.is_subset(&self.required)
        {
            return Err(RuntimeError::PendingNotRequired);
        }
        Ok(())
    }

    /// Record one withdrawal in the audit tail.
    ///
    /// **Deduplicated and bounded by construction** (E7 cut 9 fold, C3). A unit
    /// withdrawn twice is one audit fact, and the tail keeps the most recent
    /// [`MAX_OPERATION_PARTICIPANTS`] of them - a *stated* choice on the write path,
    /// which is the half the boot cannot make: the load still refuses an over-cap
    /// stored set and holds the document. Before the fold `@op/withdraw` pushed
    /// unconditionally and never re-ran the constructor, so 129 accepted join/withdraw
    /// commands stored a document the next boot **held**, taking the operation and its
    /// live money bond out of reach of every command.
    pub fn record_withdrawal(&mut self, unit: UnitRef) {
        self.withdrawals.retain(|u| *u != unit);
        self.withdrawals.push(unit);
        if self.withdrawals.len() > MAX_OPERATION_PARTICIPANTS {
            let overflow = self.withdrawals.len() - MAX_OPERATION_PARTICIPANTS;
            self.withdrawals.drain(..overflow);
        }
    }

    /// Whether `unit` is committed here.
    #[must_use]
    pub fn holds(&self, unit: UnitRef) -> bool {
        self.participants.iter().any(|p| p.unit == unit)
    }

    /// Whether this operation still owes a consumer. A `Resolved` operation never
    /// re-opens, so this answers `false` for one however the set is stored.
    #[must_use]
    pub fn pending(&self) -> BTreeSet<ConsumerId> {
        match &self.state {
            OperationState::Resolving { pending } => pending.clone(),
            _ => BTreeSet::new(),
        }
    }
}

/// The strict wire for [`OperationRuntime`].
///
/// `required` and `pending` arrive as **strings**, not as the closed enum: an id
/// this build does not know must be *kept and paged*, not dropped as unrecognised
/// (§3.9's second edge) and not turned into a refused document (which would hide
/// the operation holding a live bond).
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationRuntimeWire {
    operation: OperationId,
    participants: Vec<Participation>,
    #[serde(default)]
    force: TaskForce,
    #[serde(default)]
    withdrawals: Vec<UnitRef>,
    state: OperationStateWire,
    #[serde(default)]
    required: Vec<SmolStr>,
    #[serde(default)]
    root: Option<RootFact>,
    #[serde(default)]
    paused: Option<PauseReason>,
    #[serde(default)]
    abandoned: bool,
    #[serde(default)]
    attempts: BTreeMap<SmolStr, u32>,
    revision: u64,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
enum OperationStateWire {
    Declared,
    Activated,
    Running,
    Resolving { pending: Vec<SmolStr> },
    Resolved,
    Aborted,
}

impl TryFrom<OperationRuntimeWire> for OperationRuntime {
    type Error = RuntimeError;

    fn try_from(w: OperationRuntimeWire) -> Result<Self, Self::Error> {
        fn split(tokens: Vec<SmolStr>, unknown: &mut BTreeSet<SmolStr>) -> BTreeSet<ConsumerId> {
            let mut known = BTreeSet::new();
            for token in tokens {
                match ConsumerId::parse(&token) {
                    Some(id) => {
                        known.insert(id);
                    }
                    None => {
                        unknown.insert(token);
                    }
                }
            }
            known
        }
        let mut unknown_required: BTreeSet<SmolStr> = BTreeSet::new();
        let mut unknown_pending: BTreeSet<SmolStr> = BTreeSet::new();
        let required = split(w.required, &mut unknown_required);
        let state = match w.state {
            OperationStateWire::Declared => OperationState::Declared,
            OperationStateWire::Activated => OperationState::Activated,
            OperationStateWire::Running => OperationState::Running,
            // B11: the pending half is kept separately, so the write half restores
            // exactly what the read half saw rather than unioning one set into both.
            OperationStateWire::Resolving { pending } => OperationState::Resolving {
                pending: split(pending, &mut unknown_pending),
            },
            OperationStateWire::Resolved => OperationState::Resolved,
            OperationStateWire::Aborted => OperationState::Aborted,
        };
        let unknown: BTreeSet<SmolStr> =
            unknown_required.union(&unknown_pending).cloned().collect();
        let runtime = OperationRuntime {
            operation: w.operation,
            participants: w.participants,
            force: w.force,
            withdrawals: w.withdrawals,
            state,
            required,
            unknown_consumers: unknown,
            unknown_pending,
            root: w.root,
            paused: w.paused,
            abandoned: w.abandoned,
            attempts: w.attempts,
            revision: w.revision,
        };
        runtime.check()?;
        Ok(runtime)
    }
}

/// The **write** half of the same wire. An unknown stored consumer id is written
/// back **into both `required` and `pending`**, never dropped: this build cannot
/// discharge it, and silently losing it would let a later build's operation reach
/// `Resolved` having never run a consumer somebody declared. It is the same rule as
/// "refuse on load, never truncate", applied to the way out.
impl From<OperationRuntime> for OperationRuntimeWire {
    fn from(r: OperationRuntime) -> OperationRuntimeWire {
        let tokens = |known: &BTreeSet<ConsumerId>, unknown: &BTreeSet<SmolStr>| -> Vec<SmolStr> {
            let mut out: BTreeSet<SmolStr> = known
                .iter()
                .map(|c| SmolStr::new_static(c.as_str()))
                .collect();
            out.extend(unknown.iter().cloned());
            out.into_iter().collect()
        };
        let state = match &r.state {
            OperationState::Declared => OperationStateWire::Declared,
            OperationState::Activated => OperationStateWire::Activated,
            OperationState::Running => OperationStateWire::Running,
            OperationState::Resolving { pending } => OperationStateWire::Resolving {
                pending: tokens(pending, &r.unknown_pending),
            },
            OperationState::Resolved => OperationStateWire::Resolved,
            OperationState::Aborted => OperationStateWire::Aborted,
        };
        OperationRuntimeWire {
            operation: r.operation,
            participants: r.participants.clone(),
            force: r.force.clone(),
            withdrawals: r.withdrawals.clone(),
            state,
            required: tokens(&r.required, &r.unknown_consumers),
            root: r.root.clone(),
            paused: r.paused.clone(),
            abandoned: r.abandoned,
            attempts: r.attempts.clone(),
            revision: r.revision,
        }
    }
}

/// The whole operation: **one document, two separately versioned halves** (Q5D).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationDoc {
    /// The frozen half.
    pub terms: OperationTerms,
    /// The mutable half.
    pub runtime: OperationRuntime,
}

impl OperationDoc {
    /// The operation.
    #[must_use]
    pub fn id(&self) -> OperationId {
        self.terms.operation
    }

    /// Whether the frozen surface still hashes to what was frozen (lens Z). Re-run
    /// at **every** transition; a `false` disputes and pages rather than settling.
    #[must_use]
    pub fn consent_intact(&self) -> bool {
        crate::terms::operation_consent_hash(&self.terms) == self.terms.consent_hash
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testfix::{terms_fixture, window_at};

    #[test]
    fn e7c9_b3_the_required_consumer_set_is_computed_from_the_frozen_terms() {
        let mut terms = terms_fixture(OperationId(1));
        // Bare: the two unconditional consumers only.
        terms.bond = None;
        terms.stakes.clear();
        assert_eq!(
            terms.required_consumers(),
            BTreeSet::from([ConsumerId::CampaignOutcome, ConsumerId::ForceRetirement])
        );
        // A front stake pulls in FrontControl; an outcome stake ContractOutcomes;
        // a bond BondSettlement. Every arm, enumerated.
        terms.stakes = vec![
            Consequence::Front {
                region: 4,
                control: SmolStr::new_static("de_facto"),
            },
            Consequence::Outcome {
                contract: 9,
                row: SmolStr::new_static("success"),
            },
        ];
        terms.bond = Some(Bond {
            account: AccountId(1),
            amount: Amount::from_minor(100),
            forfeit_to: None,
            hold: None,
        });
        assert_eq!(terms.required_consumers(), BTreeSet::from(ConsumerId::ALL));
    }

    #[test]
    fn e7c9_b2_a_stored_participant_list_over_the_cap_refuses_on_load() {
        let runtime = OperationRuntime::declared(OperationId(1));
        let mut value = serde_json::to_value(&runtime).expect("serializes");
        let participants: Vec<serde_json::Value> = (0..=MAX_OPERATION_PARTICIPANTS)
            .map(|n| {
                serde_json::json!({
                    "unit": n as i64 + 1,
                    "side": "attack",
                    "command_parent": null,
                    "joined_at": 0,
                })
            })
            .collect();
        value["participants"] = serde_json::Value::Array(participants);
        let error = serde_json::from_value::<OperationRuntime>(value)
            .expect_err("a stored roster over the cap refuses on load");
        assert!(
            error.to_string().contains("over its cap"),
            "refused for the wrong reason: {error}"
        );
    }

    #[test]
    fn e7c9_b2_a_stored_audience_over_max_audience_refuses_on_load() {
        let mut terms = terms_fixture(OperationId(1));
        terms.eligible_defenders.members = (0..=MAX_AUDIENCE)
            .map(|n| PrincipalId::Object(ObjId(n as u32 + 1)))
            .collect();
        let value = serde_json::to_value(&terms).expect("serializes");
        let error = serde_json::from_value::<OperationTerms>(value)
            .expect_err("a stored audience over MAX_AUDIENCE refuses on load");
        assert!(error.to_string().contains("over its cap"), "{error}");
    }

    #[test]
    fn e7c9_b2_the_notice_window_sits_inside_the_declared_band_on_one_clock() {
        let mut terms = terms_fixture(OperationId(1));
        assert!(terms.check().is_ok());
        // Under the floor.
        terms.window = window_at(0, OPERATION_NOTICE_MIN - 1, 1);
        assert!(matches!(
            terms.check(),
            Err(TermsError::NoticeOutOfRange { .. })
        ));
        // Over the ceiling.
        terms.window = window_at(0, OPERATION_NOTICE_MAX + 1, 1);
        assert!(matches!(
            terms.check(),
            Err(TermsError::NoticeOutOfRange { .. })
        ));
        // A different clock generation is a refusal, never a subtraction.
        terms.window = window_at(0, OPERATION_NOTICE_MIN, 1);
        terms.window.1.clock = mush_core::ClockRef::WorldIc { generation: 2 };
        assert_eq!(terms.check(), Err(TermsError::ClockMismatch));
    }

    #[test]
    fn e7c9_b7_an_unknown_stored_consumer_is_kept_never_dropped() {
        let runtime = OperationRuntime::declared(OperationId(1));
        let mut value = serde_json::to_value(&runtime).expect("serializes");
        value["required"] = serde_json::json!(["campaign_outcome", "orbital_bombardment"]);
        value["state"] =
            serde_json::json!({ "state": "resolving", "pending": ["orbital_bombardment"] });
        let back: OperationRuntime =
            serde_json::from_value(value).expect("the document still loads");
        assert_eq!(back.required, BTreeSet::from([ConsumerId::CampaignOutcome]));
        assert_eq!(
            back.unknown_consumers,
            BTreeSet::from([SmolStr::new_static("orbital_bombardment")])
        );
        // And the pending set it could not decode is empty, so the fan-out cannot
        // "complete" it by accident — the operation is held by `unknown_consumers`.
        assert!(back.pending().is_empty());
    }

    #[test]
    fn e7c9_b3_consumer_tokens_round_trip_on_every_arm() {
        for consumer in ConsumerId::ALL {
            assert_eq!(ConsumerId::parse(consumer.as_str()), Some(consumer));
        }
        assert_eq!(ConsumerId::parse("orbital_bombardment"), None);
        assert_eq!(ConsumerId::ALL.len(), 5);
    }

    // ---- E7 cut 9 independent verify fold ----

    /// **D11. `attempts` is bounded on the load wire**, against the closed five-id
    /// vocabulary that keys it. Cut 9 bounded `participants`, `withdrawals` and
    /// `force.formations` and left this one with no bound to refuse against, so a
    /// corrupt document carrying a million entries loaded and was re-serialized on
    /// every write.
    #[test]
    fn vf_d11a_a_runtime_carrying_six_attempt_keys_refuses_on_load() {
        let runtime = OperationRuntime::declared(OperationId(9));
        let mut value = serde_json::to_value(&runtime).expect("serializes");
        let mut attempts = serde_json::Map::new();
        for (n, consumer) in ConsumerId::ALL.iter().enumerate() {
            attempts.insert(consumer.as_str().to_owned(), serde_json::json!(n as u32));
        }
        // Exactly the vocabulary loads…
        value["attempts"] = serde_json::Value::Object(attempts.clone());
        serde_json::from_value::<OperationRuntime>(value.clone()).expect("five keys load");
        // …and one more does not.
        attempts.insert("orbital_bombardment".to_owned(), serde_json::json!(1));
        value["attempts"] = serde_json::Value::Object(attempts);
        let error = serde_json::from_value::<OperationRuntime>(value)
            .expect_err("six attempt keys must refuse on load");
        assert!(error.to_string().contains("over its cap"), "{error}");
    }

    /// **B7. The recorded outcome is bounded on the wire**, which is where the
    /// documentation always said it was. `RootFact` has no wire of its own, so the
    /// bound lives in the runtime's constructor and the load re-runs it.
    #[test]
    fn vf_b7a_a_stored_outcome_over_the_byte_cap_refuses_on_load() {
        let mut runtime = OperationRuntime::declared(OperationId(9));
        runtime.root = Some(RootFact {
            canon: Some(1),
            at: 0,
            outcome: SmolStr::new("z".repeat(MAX_OUTCOME_NAME_BYTES)),
        });
        let value = serde_json::to_value(&runtime).expect("serializes");
        serde_json::from_value::<OperationRuntime>(value).expect("exactly the cap loads");

        runtime.root = Some(RootFact {
            canon: Some(1),
            at: 0,
            outcome: SmolStr::new("z".repeat(MAX_OUTCOME_NAME_BYTES + 1)),
        });
        let value = serde_json::to_value(&runtime).expect("serializes");
        let error = serde_json::from_value::<OperationRuntime>(value)
            .expect_err("an over-cap outcome must refuse on load");
        assert!(error.to_string().contains("longer than"), "{error}");
    }

    /// **B11. An unknown consumer that had left `pending` does not come back through
    /// a round trip.** The read half routed unknown tokens from both lists into one
    /// set and the write half unioned that set into both, so a build that did not
    /// know an id resurrected one a knowing build had already discharged.
    #[test]
    fn vf_b11a_an_unknown_consumer_that_had_left_pending_does_not_come_back_through_a_round_trip() {
        let runtime = OperationRuntime::declared(OperationId(9));
        let mut value = serde_json::to_value(&runtime).expect("serializes");
        value["required"] = serde_json::json!(["campaign_outcome", "orbital_bombardment"]);
        value["state"] = serde_json::json!({ "state": "resolving", "pending": [] });
        let back: OperationRuntime = serde_json::from_value(value).expect("loads");
        assert_eq!(
            back.unknown_consumers,
            BTreeSet::from([SmolStr::new_static("orbital_bombardment")])
        );
        assert!(back.unknown_pending.is_empty());

        // The write half restores exactly what the read half saw.
        let again = serde_json::to_value(&back).expect("serializes");
        assert_eq!(
            again["state"]["pending"],
            serde_json::json!([]),
            "a discharged unknown consumer came back into pending"
        );
        assert_eq!(
            again["required"],
            serde_json::json!(["campaign_outcome", "orbital_bombardment"]),
            "the unknown id was dropped from required"
        );
        // …and one that really IS pending stays pending.
        let mut value =
            serde_json::to_value(OperationRuntime::declared(OperationId(9))).expect("serializes");
        value["required"] = serde_json::json!(["orbital_bombardment"]);
        value["state"] =
            serde_json::json!({ "state": "resolving", "pending": ["orbital_bombardment"] });
        let back: OperationRuntime = serde_json::from_value(value).expect("loads");
        let again = serde_json::to_value(&back).expect("serializes");
        assert_eq!(
            again["state"]["pending"],
            serde_json::json!(["orbital_bombardment"])
        );
    }

    /// **B13. The terms carry no `scope` field at all**, so a stored one is refused
    /// rather than silently hashed. A hashed field nothing can set is a hash-space
    /// change waiting for its first writer; it is gone before production freezes one.
    #[test]
    fn vf_b13b_the_stored_terms_carry_no_scope_field() {
        let terms = terms_fixture(OperationId(1));
        let value = serde_json::to_value(&terms).expect("serializes");
        assert!(
            value.get("scope").is_none(),
            "the terms still carry a scope: {value}"
        );
        let mut with_scope = value;
        with_scope["scope"] = serde_json::json!(4);
        serde_json::from_value::<OperationTerms>(with_scope)
            .expect_err("a stored scope must be refused, never hashed");
    }
}
