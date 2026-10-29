//! **The Registry's seizure and forfeit of a managed core's title** (doc 17h cut M8d2, S1;
//! doc 17 §10, D-UM-42). The one managed path that may encumber or re-title a core: the
//! charter transfer's shape (`charter_transfer`), a closed plan whose guard
//! (`ClosedSeizurePlan`) admits exactly the before-title image, the exact core
//! `TitleBirth` and one closed op shape, and nothing else.
//!
//! * **Seize** — exactly `Encumber { Disputed { claimant: the Registry org, reason: the
//!   inspection key } }`. The economy cannot read the inspection (it depends on neither
//!   mush-roll nor tacsim): the registered Roll/tacsim producer proves the hull's newest
//!   inspection reads ungoverned and names this core, and binds that proof
//!   ([`InspectionProof`]) into the same coordinator union; this leg compares the plan's
//!   `reason` to the proof's key byte for byte (S2).
//! * **Forfeit** — `Release` of exactly the Disputed claim the seizure placed, then
//!   `TransferTitle { to: Organization(Registry org) }` (Q-M8-4). Invariant 12 refuses a
//!   transfer that carries a claim, so the release rides in the same plan; nothing else
//!   may (doc 17h cut M8d2's deviation from S1's "one op", recorded in the cut's report).
//!   The hold (`[tacsim.minds].seizure_hold_ic_days`) is the producer's to prove; this leg
//!   requires the live seizure receipt and an unchanged title since it.
//!
//! No public post, staff bit or serialized plan selects this guard; the participant's
//! prepare is reachable only inside the coordinator union (`OwnerPrepareContext`), from
//! the exact two composed actions. No notice is sent from here (the charter transfer's
//! rule): the core's holder reads the claim on the title.
use super::*;
use crate::record::{AssetLifecycle, Encumbrance};
use mush_core::DefinitionRef;
use mush_org::OrgId;
use serde::{Deserialize, Serialize};

mod install;
pub(crate) mod receipt;
pub use install::{BoundCoreSeizure, CoreSeizurePatch};
pub use receipt::{SeizureAuditSubject, SeizureReceiptAudit, SeizureStepReceipt};

/// The seizure action's id (tacsim-owned, the Registry's operator verb).
pub const SEIZE_ACTION_ID: &str = "registry_core_seizure";
/// The forfeit action's id (tacsim-owned).
pub const FORFEIT_ACTION_ID: &str = "registry_core_forfeit";
/// The owner of both actions.
pub const ACTION_OWNER: &str = "tacsim";

/// **What composition freezes** for the two closed acts: the exact actions and the
/// Registry org that claims and receives (`RegistryAuthority.organization`, Q-M8-4). None
/// of it is caller input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeizureConfig {
    pub seize: DefinitionRef,
    pub forfeit: DefinitionRef,
    pub claimant: OrgId,
}
impl SeizureConfig {
    /// # Errors
    /// An action that is not the exact tacsim action of its id at version 1, or the two the
    /// same.
    pub fn validate(&self) -> Result<(), &'static str> {
        for (action, id) in [
            (&self.seize, SEIZE_ACTION_ID),
            (&self.forfeit, FORFEIT_ACTION_ID),
        ] {
            if action.key().owner.as_str() != ACTION_OWNER
                || action.key().kind.as_str() != "action"
                || action.key().id.as_str() != id
                || action.version() != 1
            {
                return Err("core seizure requires the exact tacsim actions");
            }
        }
        Ok(())
    }
    fn claimant(&self) -> PrincipalId {
        PrincipalId::Organization(self.claimant)
    }
}

/// **The inspection key** a seizure's claim carries: `hull/<hull, 8 hex>/inspection/<seq>`,
/// the sanction key's form (doc 17h cut M8d D3), unique per inspection because a hull's
/// `seq` only grows. Eight hex digits, never sixteen.
#[must_use]
pub fn inspection_key(hull: ObjId, seq: u64) -> String {
    format!("hull/{:08x}/inspection/{seq}", hull.0)
}

/// **The producer's proof** (S2): the hull's newest inspection read ungoverned, named this
/// core as fitted, and is within the configured age. The Roll/tacsim producer builds it from
/// the record it read in the same transaction; the economy checks its shape and binds its
/// `key` as the claim's reason.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionProof {
    /// [`inspection_key`] of `hull` and `seq`, as the producer wrote it.
    pub key: String,
    pub hull: ObjId,
    pub core: ObjId,
    pub seq: u64,
}
impl InspectionProof {
    /// A proof whose key is not exactly [`inspection_key`]`(hull, seq)`, or that names no
    /// hull, no inspection or the core as its own hull, proves nothing.
    #[must_use]
    pub fn well_formed(&self) -> bool {
        self.hull.0 != 0
            && self.core.0 != 0
            && self.hull != self.core
            && self.seq != 0
            && self.key.as_bytes() == inspection_key(self.hull, self.seq).as_bytes()
    }
}

/// Which of the two acts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SeizureStep {
    Seize(InspectionProof),
    /// Forfeit the core the live seizure `seizure` (its commit) holds.
    Forfeit {
        seizure: CommitToken,
    },
}

/// The closed request the producer constructs: the step, the core and the title revision it
/// read. Not a wire format.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryCoreSeizure {
    pub step: SeizureStep,
    pub core: ObjId,
    pub title_revision: u64,
}

/// The owner output the producer reads back: the action and the exact receipt.
pub struct CoreSeizureOutput {
    action: DefinitionRef,
    subject: SeizureAuditSubject,
}
impl CoreSeizureOutput {
    pub fn action(&self) -> &DefinitionRef {
        &self.action
    }
    pub fn subject(&self) -> &SeizureAuditSubject {
        &self.subject
    }
    pub const fn commit_token(&self) -> CommitToken {
        self.subject.commit
    }
}

fn action_for<'a>(config: &'a SeizureConfig, step: &SeizureStep) -> &'a DefinitionRef {
    match step {
        SeizureStep::Seize(_) => &config.seize,
        SeizureStep::Forfeit { .. } => &config.forfeit,
    }
}

fn require_action(
    config: &SeizureConfig,
    step: &SeizureStep,
    c: &OwnerPrepareContext<'_>,
) -> Result<(), ParticipantError> {
    let actor = c.invocation().actor();
    if config.validate().is_err()
        || c.action() != action_for(config, step)
        || c.invocation().principal() != &(mush_core::action::PrincipalId::Object { id: actor })
        || c.invocation().subject() != &mush_core::action::PerceivedRef::SelfActor
        || !c.invocation().targets().is_empty()
        || c.world().get(actor).is_none_or(|o| {
            o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
        })
    {
        return Err(refused());
    }
    Ok(())
}

/// **The closed op shape** each step admits, and the claim it names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    Seize { reason: String },
    Forfeit { reason: String },
}
impl Shape {
    fn reason(&self) -> &str {
        match self {
            Self::Seize { reason } | Self::Forfeit { reason } => reason,
        }
    }
    fn ops(&self, object: ObjId, claimant: &PrincipalId) -> Vec<AssetOp> {
        match self {
            Self::Seize { reason } => vec![AssetOp::Encumber {
                object,
                encumbrance: disputed(claimant, reason),
            }],
            Self::Forfeit { reason } => vec![
                AssetOp::Release {
                    object,
                    encumbrance: disputed(claimant, reason),
                },
                AssetOp::TransferTitle {
                    object,
                    to: claimant.clone(),
                    to_scope: None,
                },
            ],
        }
    }
}
pub(crate) fn disputed(claimant: &PrincipalId, reason: &str) -> Encumbrance {
    Encumbrance::Disputed {
        claimant: claimant.clone(),
        reason: reason.into(),
    }
}

/// The before-title and the shape, after every precondition held. Forfeit's claim is the
/// live seizure's own (`held`), never the caller's.
fn title<'a>(
    config: &SeizureConfig,
    policy: &InitialFilingPolicy,
    state: &'a EconomyStore,
    world: &World,
    r: &RegistryCoreSeizure,
) -> Result<(&'a EconomicAsset, Shape), ParticipantError> {
    if state.frozen
        || r.title_revision == 0
        || r.title_revision == u64::MAX
        || !state.is_title_issuer(&policy.issuer)
        || state.snapshot.managed_titles.core(r.core).is_none()
        || world
            .get(r.core)
            .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING))
    {
        return Err(refused());
    }
    let claimant = config.claimant();
    let asset = state.snapshot.asset(r.core).ok_or_else(refused)?;
    if asset.object != r.core
        || asset.revision != r.title_revision
        || asset.title_owner == claimant
        || asset.frozen
        || asset.stolen.is_some()
        || asset.lifecycle != AssetLifecycle::Active
    {
        return Err(refused());
    }
    let audit = &state.fold.seizure_receipts;
    let shape = match &r.step {
        SeizureStep::Seize(proof) => {
            if !proof.well_formed()
                || proof.core != r.core
                || !asset.encumbrances.is_empty()
                || audit.held(r.core).is_some()
            {
                return Err(refused());
            }
            Shape::Seize {
                reason: proof.key.clone(),
            }
        }
        SeizureStep::Forfeit { seizure } => {
            let held = audit.held(r.core).ok_or_else(refused)?;
            let SeizureStepReceipt::Seize { reason, .. } = &held.step else {
                return Err(refused());
            };
            if held.commit != *seizure
                || held.claimant != config.claimant
                || &held.title != asset
                || asset.encumbrances.as_slice() != [disputed(&claimant, reason)]
            {
                return Err(refused());
            }
            Shape::Forfeit {
                reason: reason.clone(),
            }
        }
    };
    Ok((asset, shape))
}

struct SeizureAuth {
    issuer: PrincipalId,
    claimant: PrincipalId,
    core: ObjId,
    ops: Vec<AssetOp>,
}
impl AuthEval for SeizureAuth {
    fn authorize(&self, _actor: &PrincipalId, _account: &Account, _delta: Amount) -> bool {
        // No money moves under either act.
        false
    }
    fn authorize_asset(
        &self,
        actor: &PrincipalId,
        asset: Option<&EconomicAsset>,
        op: &AssetOp,
    ) -> bool {
        actor == &self.issuer
            && self.ops.contains(op)
            && asset.is_some_and(|a| a.object == self.core)
    }
    fn authorize_title_receipt(
        &self,
        actor: &PrincipalId,
        to: &PrincipalId,
        scope: Option<OrgScopeId>,
    ) -> bool {
        actor == &self.issuer && to == &self.claimant && scope.is_none()
    }
}

fn plan(
    config: &SeizureConfig,
    policy: &InitialFilingPolicy,
    (actor, commit, at_ic): (ObjId, CommitToken, i64),
    r: &RegistryCoreSeizure,
    shape: &Shape,
) -> EconomyPlan {
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new(receipt::NAMESPACE, commit.to_hex()),
        PrincipalId::System(policy.issuer.clone()),
        EconomicCause::Ruleset {
            namespace: receipt::NAMESPACE.into(),
            correlation: receipt::correlation(
                (commit, actor, at_ic),
                config.claimant,
                r,
                shape.reason(),
            )
            .into(),
        },
        Vec::new(),
    );
    plan.description = match shape {
        Shape::Seize { .. } => "Registry seizure of a core",
        Shape::Forfeit { .. } => "Registry forfeit of a seized core",
    }
    .into();
    plan.asset_ops = shape.ops(r.core, &config.claimant());
    plan.expected_asset_revisions.push(crate::AssetRevision {
        object: r.core,
        revision: r.title_revision,
    });
    plan
}

fn validate(
    config: &SeizureConfig,
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    world: &World,
    r: &RegistryCoreSeizure,
    plan: &EconomyPlan,
) -> Result<Prepared, ParticipantError> {
    let (before, shape) = title(config, policy, state, world, r)?;
    let claimant = config.claimant();
    let auth = SeizureAuth {
        issuer: PrincipalId::System(policy.issuer.clone()),
        ops: shape.ops(r.core, &claimant),
        claimant,
        core: r.core,
    };
    let gate = state.key_gate();
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: world.now(),
        ic_now: world.game_now_unix(),
        next_hold_id: state.next_hold,
        spent: BTreeMap::new(),
        keys: &gate,
    };
    let proof = ClosedSeizurePlan {
        plan: plan.clone(),
        before_title: before.clone(),
        birth: state
            .snapshot
            .managed_titles
            .core(r.core)
            .cloned()
            .ok_or_else(refused)?,
        claimant: config.claimant(),
        shape,
    };
    match crate::engine::validate_core_seizure(&proof, &state.snapshot, &env) {
        PlanResult::Post(prepared) => Ok(*prepared),
        PlanResult::NeedsApproval(_) | PlanResult::AlreadyPosted(_) | PlanResult::Refused(_) => {
            Err(refused())
        }
    }
}

pub(super) use install::{bind, prepare, recheck};

impl EconomyStore {
    /// **The live seizure holding `core`** (doc 17h cut M8d2): its receipt while the claim
    /// stands, `None` for a core no seizure holds (never seized, or forfeited).
    ///
    /// # Errors
    /// While the store is frozen, or the seizure audit is not complete this boot.
    pub fn core_seizure(
        &self,
        core: ObjId,
    ) -> Result<Option<&SeizureAuditSubject>, ReceiptAuditError> {
        if self.frozen {
            return Err(ReceiptAuditError::Incomplete);
        }
        self.fold.seizure_receipts.require_complete()?;
        Ok(self.fold.seizure_receipts.held(core))
    }
    /// **Whether `core` is impounded** (doc 17h cut M8d2, the lead's S3 ruling "impounded in
    /// place"): a managed core whose title carries a Registry seizure's claim, or whose title
    /// the Registry org holds after a forfeit. The seal reads it as no core (the ROM, Class 0,
    /// governed); the core stays physically fitted. Read off the title and the replayed
    /// receipts, never a caller's word: only a seizure can place a claim on a managed core
    /// (`managed_title::require_unmanaged_mutations`), and only a forfeit moves one to its
    /// claimant. An untitled or unmanaged object is never impounded.
    #[must_use]
    pub fn core_impounded(&self, core: ObjId) -> bool {
        let Some(title) = self.snapshot.asset(core) else {
            return false;
        };
        self.snapshot.managed_titles.core(core).is_some()
            && (self.fold.seizure_receipts.held(core).is_some()
                || title.encumbrances.iter().any(|e| {
                    matches!(
                        e,
                        Encumbrance::Disputed {
                            claimant: PrincipalId::Organization(_),
                            ..
                        }
                    )
                })
                || self
                    .fold
                    .seizure_receipts
                    .forfeited_to(core, &title.title_owner))
    }
    /// **The seizure grounded on `hull`'s inspection `seq`**, if one was made (doc 17h cut M8d2:
    /// `minds.seized` reads its IC second). Fails closed as [`Self::core_seizure`].
    ///
    /// # Errors
    /// As [`Self::core_seizure`].
    pub fn core_seizure_on(
        &self,
        hull: ObjId,
        seq: u64,
    ) -> Result<Option<&SeizureAuditSubject>, ReceiptAuditError> {
        if self.frozen {
            return Err(ReceiptAuditError::Incomplete);
        }
        Ok(self.fold.seizure_receipts.receipts()?.find(|r| {
            matches!(r.step, SeizureStepReceipt::Seize { hull: h, seq: n, .. } if h == hull && n == seq)
        }))
    }
    /// **One seize or forfeit by its commit** (a seizure record's key reads it back).
    ///
    /// # Errors
    /// As [`Self::core_seizure`].
    pub fn core_seizure_receipt(
        &self,
        commit: CommitToken,
    ) -> Result<Option<&SeizureAuditSubject>, ReceiptAuditError> {
        if self.frozen {
            return Err(ReceiptAuditError::Incomplete);
        }
        self.fold.seizure_receipts.get(commit)
    }
}

/// **The private closed plan proof** (S1): exact plan equality, the exact before-title image,
/// the exact core `TitleBirth`, and exactly the step's closed op shape — `[Encumber {
/// Disputed }]` for Seize, `[Release { that Disputed }, TransferTitle { the Registry org }]`
/// for Forfeit. No money line, hold or custody op rides along. Only this module constructs it.
pub(crate) struct ClosedSeizurePlan {
    plan: EconomyPlan,
    before_title: EconomicAsset,
    birth: crate::managed_title::TitleBirth,
    claimant: PrincipalId,
    shape: Shape,
}
impl ClosedSeizurePlan {
    pub(crate) fn plan(&self) -> &EconomyPlan {
        &self.plan
    }
    pub(crate) fn check(
        &self,
        plan: &EconomyPlan,
        snapshot: &EconomySnapshot,
    ) -> Result<(), crate::EconomyError> {
        let object = self.before_title.object;
        if plan != &self.plan
            || plan.idempotency.namespace != receipt::NAMESPACE
            || !matches!(&plan.cause,EconomicCause::Ruleset{namespace,..} if namespace==receipt::NAMESPACE)
            || snapshot.asset(object) != Some(&self.before_title)
            || snapshot.managed_titles.core(object) != Some(&self.birth)
            || plan.asset_ops != self.shape.ops(object, &self.claimant)
            || !plan.requested_lines.is_empty()
            || !plan.hold_ops.is_empty()
            || !plan.custody_ops.is_empty()
        {
            return Err(crate::EconomyError::ManagedContract);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
