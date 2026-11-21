//! **The operation frame, as TacSim reaches it** (doc 50 cut OP.0; Q-50-57 item 4, Q-50-18,
//! Q-50-26, Q-50-30).
//!
//! Doc 50 frames the Isidis operation in `mush-campaign`: declared, bonded, noticed, windowed,
//! resolved once. TacSim is pinned without a campaign edge (and without a mail edge), and the
//! campaign crate without a ruleset edge, so neither may name the other. This module is TacSim's
//! side of the seam between them, built as the composition root's other links are: TacSim states
//! what it needs as a trait ([`OperationFrame`]), and `mush-composition` implements it over the
//! campaign crate's public API and seats it in world state ([`OperationFrameSlot`]). A TacSim arm
//! asks [`frame`] for it; unwired, there is none, and the arm declares nothing.
//!
//! What crosses, both ways, is TacSim's own vocabulary: plain ids, principals, IC seconds and a
//! closed outcome. The frame decides who won (the window closes, the close probe is read, the
//! bond settles); TacSim reads that answer through [`OperationFrame::status`] and moves control by
//! its own typed call. The answer is the declarer's word: [`OperationOutcome::DeclarerFailed`]
//! is the defenders holding.
//!
//! **The other direction** is the close probe: the campaign asks the owner of a close-only
//! objective for its answer at the window's close. TacSim's probes are named in
//! [`CLOSE_PROBES`] and read through [`close_probe`], which the composition root wires into the
//! campaign. Doc 50 cut OP.4 names the tank farm's ([`crate::stockpile::TANK_FARM_PROBE`]).

use std::sync::Arc;

use mush_core::{Ctx, World};
use mush_economy::{AccountId, Amount};
use mush_org::PrincipalId;

/// **Whose success a met close probe is** (Q-50-57 item 3: the objective's own polarity).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum MetFavors {
    /// A met probe is the declarer's success.
    Declarer,
    /// A met probe is the defenders' success, so the declarer's failure (Isidis: the tank farm
    /// at its threshold at the close).
    Defenders,
}

/// **The bond a declaration reserves** on the declarer's own account, and who takes it when the
/// declarer fails (Q-50-57 item 5, provisional: the defenders' treasury).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationBond {
    /// The declarer's own account.
    pub account: AccountId,
    /// How much (positive).
    pub amount: Amount,
    /// Who takes it on the declarer's failure, and into which account. `None` releases it on
    /// every outcome.
    pub forfeit_to: Option<(PrincipalId, AccountId)>,
}

/// **One operation, as a TacSim arm declares it** ([`OperationFrame::declare`]). The objective is
/// always a close probe: no objective in doc 50's plan reads canon (Q-50-57 item 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationRequest {
    /// Who declares: an organization (a polity's backing organization) or an object.
    pub declarer: PrincipalId,
    /// The objective's one line, as a viewer entitled to the plan reads it.
    pub summary: String,
    /// The close probe's name, as [`close_probe`] knows it.
    pub probe: String,
    /// Whose success a met probe is.
    pub met_favors: MetFavors,
    /// The sides: a name and who commands it.
    pub sides: Vec<(String, PrincipalId)>,
    /// Who may defend. An organization's notice reaches its active members.
    pub defenders: Vec<PrincipalId>,
    /// The legal-target classes (Q-50-26 reads them).
    pub legal_targets: Vec<String>,
    /// The bond, if any.
    pub bond: Option<OperationBond>,
    /// IC seconds from the declaration to the window's open (28 to 84 IC days).
    pub notice_secs: i64,
    /// IC seconds the window runs.
    pub window_secs: i64,
    /// Whether everyone may read the notice. Otherwise only the defenders may.
    pub public: bool,
}

/// **Where an operation is** (TacSim's own words for the frame's states).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum OperationPhase {
    /// Declared and not yet activated: only a staff verb leaves one here.
    Declared,
    /// Activated, its notice running: the window has not opened.
    Notice,
    /// Inside its window.
    Running,
    /// Decided: the outcome is recorded and its consequences (the bond among them) are settling.
    Settling,
    /// Decided and settled. Nothing reopens it.
    Settled,
}

/// **Who won** (the declarer's word).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum OperationOutcome {
    /// The declarer's objective was met.
    DeclarerSucceeded,
    /// The declarer's objective failed: the defenders held. The bond, if it named a beneficiary,
    /// is theirs.
    DeclarerFailed,
    /// A partial result, which only staff record.
    Partial,
    /// Abandoned by staff: nobody won, and the bond went back.
    Abandoned,
}

/// **What a TacSim arm reads of one operation** ([`OperationFrame::status`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OperationStatus {
    /// Where it is.
    pub phase: OperationPhase,
    /// Who declared it.
    pub declarer: PrincipalId,
    /// Its sides.
    pub sides: Vec<(String, PrincipalId)>,
    /// Who may defend.
    pub defenders: Vec<PrincipalId>,
    /// Its legal-target classes.
    pub legal_targets: Vec<String>,
    /// The window, as world IC seconds `(open, close)`.
    pub window: (i64, i64),
    /// Who won, once decided ([`OperationPhase::Settling`] or [`OperationPhase::Settled`]).
    pub outcome: Option<OperationOutcome>,
    /// **The players the frame's own notice tells** (doc 50b item 9): the defenders' members it
    /// reaches, within its recipient cap. The lane's notice skips these, and only these.
    pub told: Vec<mush_core::ObjId>,
}

/// **The operation frame** (this module's head): implemented at the composition root over
/// `mush-campaign`, and only there.
pub trait OperationFrame: Send + Sync {
    /// Declare `request` and activate it in one step: the bond is placed, the terms freeze and
    /// the notice is pushed to the defenders on the frame's next tick. Answers the operation's
    /// id.
    ///
    /// # Errors
    ///
    /// Why the frame refused, in words for a log or a page. Nothing is declared.
    fn declare(&self, ctx: &mut Ctx<'_>, request: &OperationRequest) -> Result<u64, String>;

    /// What `operation` is now, or `None` for an id the frame does not hold.
    fn status(&self, w: &World, operation: u64) -> Option<OperationStatus>;
}

/// **The frame's seat in world state**: what the composition root wired, or nothing.
#[derive(Clone, Default)]
pub struct OperationFrameSlot(Option<Arc<dyn OperationFrame>>);

impl OperationFrameSlot {
    /// The seam, wired.
    #[must_use]
    pub fn new(frame: Arc<dyn OperationFrame>) -> OperationFrameSlot {
        OperationFrameSlot(Some(frame))
    }
}

impl std::fmt::Debug for OperationFrameSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("OperationFrameSlot")
            .field(&self.0.is_some())
            .finish()
    }
}

/// **The frame, when one is composed.** A clone of the handle, so the caller's `Ctx` is free to
/// pass to [`OperationFrame::declare`].
#[must_use]
pub fn frame(w: &World) -> Option<Arc<dyn OperationFrame>> {
    w.module_state::<OperationFrameSlot>()
        .and_then(|slot| slot.0.clone())
}

/// One close probe: `(world, operation)` to met, not met, or cannot tell.
pub type CloseProbeFn = fn(&World, u64) -> Option<bool>;

/// **TacSim's close probes, by name.** Doc 50 cut OP.4: the tank farm's last reading against its
/// threshold ([`crate::stockpile::close_probe`]).
pub const CLOSE_PROBES: &[(&str, CloseProbeFn)] = &[(
    crate::stockpile::TANK_FARM_PROBE,
    crate::stockpile::close_probe,
)];

/// **Read the close probe `probe` for `operation`**: what the composition root's campaign probe
/// asks at a window's close. A name TacSim does not know is `None`, which the frame treats as a
/// wait, never as an answer.
#[must_use]
pub fn close_probe(w: &World, probe: &str, operation: u64) -> Option<bool> {
    CLOSE_PROBES
        .iter()
        .find(|(name, _)| *name == probe)
        .and_then(|(_, read)| read(w, operation))
}
