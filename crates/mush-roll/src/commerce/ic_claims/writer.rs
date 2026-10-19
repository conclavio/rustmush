//! Closed four-owner composition. All evidence below comes from actual typed
//! preparation outputs; neither stored receipt data nor a public wire request
//! can stand in for the host's opaque transition or an owner's installation.
use super::super::{
    LicenceId,
    families::CommerceFamilies,
    participant::{RootParticipant, RootRequest},
    records::{PublicationError, Record},
};
use super::{
    ClaimRecord, CloseCause, CloseKind, IcAdmissionReader, IcAdmissionRequest, IcClaimConfig,
    IcCloseKind, IcSealOutput, IcSealParticipant, OpenClaim, SortieNumber, Transition,
};
use mush_core::participant::ParticipantHandle;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{ActionTxn, CommitToken, DefinitionRef, ModError, PrepareCtx, Registrar};
use mush_sim::ic_exclusion::{IcExclusionParticipant, IcExclusionRequest, IcIdentity};
use mush_tactics::ic_custody::{IcCustodyParticipant, IcCustodyRequest};
use smol_str::SmolStr;
use std::sync::{Arc, OnceLock};

fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
fn error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
#[derive(Clone)]
struct Registration {
    families: CommerceFamilies,
    config: IcClaimConfig,
}
#[derive(Clone, Default)]
pub struct IcWriter(Arc<OnceLock<Registration>>);
impl std::fmt::Debug for IcWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IcWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
impl IcWriter {
    pub fn new() -> Self {
        Self::default()
    }
    /// Read-only proof of the actual Roll final audit. This never opens an
    /// owner gate and cannot replace a host transition or an installation.
    pub fn require_audited(&self, world: &mush_core::World) -> Result<u64, PublicationError> {
        self.0.get().ok_or(PublicationError::Unavailable)?;
        if world
            .module_state_owner(std::any::TypeId::of::<crate::RollStore>())
            .is_none_or(|owner| owner.as_str() != "roll")
        {
            return Err(PublicationError::Unavailable);
        }
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .ic_claim_next_transition()
            .map_err(bad)
    }
    /// Actual Tacsim's final boot hook repeats the exact typed/physical join
    /// after Roll's mandatory audit opened its owner. This method cannot mark
    /// either owner's state ready or construct a host event.
    pub fn claims_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<Vec<super::IcClaimHistory>, PublicationError> {
        if ctx.module.as_deref() != Some("tacsim") {
            return Err(PublicationError::Unavailable);
        }
        let registration = self.0.get().ok_or(PublicationError::Unavailable)?;
        let roll = ctx
            .state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        roll.commerce_ready().map_err(bad)?;
        let rows = super::audit::rows(ctx, &registration.families)?;
        super::audit::physical(ctx.w, &rows)?;
        for history in &rows {
            roll.check_required_root(&super::roots::required(&history.row)?)
                .map_err(bad)?;
        }
        Ok(rows)
    }
    pub(crate) fn install(
        &self,
        reg: &mut Registrar<'_>,
        families: CommerceFamilies,
        config: IcClaimConfig,
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS {
            return Err(error("IC writer requires actual Roll registration"));
        }
        reg.delegate_state_family(&families.ic_claims, "tacsim")?;
        self.0
            .set(Registration { families, config })
            .map_err(|_| error("IC writer already registered"))
    }
    pub fn validate_registration(&self, config: &IcClaimConfig) -> Result<(), ModError> {
        let r = self
            .0
            .get()
            .ok_or_else(|| error("Roll IC writer unavailable"))?;
        if r.config.protocol != config.protocol {
            return Err(error("IC writer composition differs"));
        }
        Ok(())
    }
    pub fn for_action<S>(
        &self,
        reg: &mut Registrar<'_>,
        config: &IcClaimConfig,
        action: &DefinitionRef,
        reader: OwnerReadHandle<IcAdmissionReader>,
    ) -> Result<RegisteredIcWriter<S>, ModError>
    where
        S: IcSealParticipant,
        S::Output: IcSealOutput,
    {
        if reg.module() != "tacsim" || !config.actions()?.contains(action) {
            return Err(error("IC writer requires fixed Tacsim action"));
        }
        let result = RegisteredIcWriter {
            registration: self.clone(),
            config: config.clone(),
            action: action.clone(),
            reader,
            seal: reg.request_owner_participant::<S>(action.clone())?,
            custody: reg.request_owner_participant::<IcCustodyParticipant>(action.clone())?,
            exclusion: reg.request_owner_participant::<IcExclusionParticipant>(action.clone())?,
            roots: reg.request_owner_participant::<RootParticipant>(action.clone())?,
        };
        if self.0.get().is_some() {
            self.validate_registration(config)?;
        }
        Ok(result)
    }
}
/// Source-free acknowledgement data only. It is not accepted by any owner as
/// a Start/Close capability, a seal, a live capacity claim or physical event.
pub struct IcClaimReceipt {
    open: CommitToken,
    closed: Option<CommitToken>,
}
impl IcClaimReceipt {
    pub fn open(&self) -> CommitToken {
        self.open
    }
    pub fn closed(&self) -> Option<CommitToken> {
        self.closed
    }
}
pub struct RegisteredIcWriter<S: IcSealParticipant>
where
    S::Output: IcSealOutput,
{
    registration: IcWriter,
    config: IcClaimConfig,
    action: DefinitionRef,
    reader: OwnerReadHandle<IcAdmissionReader>,
    seal: ParticipantHandle<S>,
    custody: ParticipantHandle<IcCustodyParticipant>,
    exclusion: ParticipantHandle<IcExclusionParticipant>,
    roots: ParticipantHandle<RootParticipant>,
}
impl<S: IcSealParticipant> RegisteredIcWriter<S>
where
    S::Output: IcSealOutput,
{
    fn registration(&self, transition: Transition) -> Result<&Registration, PublicationError> {
        let r = self
            .registration
            .0
            .get()
            .ok_or(PublicationError::Unavailable)?;
        if r.config.protocol != self.config.protocol
            || self.action != self.config.protocol.action(transition)?
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(r)
    }
    pub fn open(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        start: S::Start,
        licence: LicenceId,
        profiles: Vec<SmolStr>,
    ) -> Result<IcClaimReceipt, PublicationError> {
        let r = self.registration(Transition::Open)?;
        let facts = ctx
            .read_owner_preimage(
                txn,
                &self.reader,
                IcAdmissionRequest::own(S::opening_unit(&start), licence),
            )
            .map_err(bad)?;
        // Preserve this actual observation in Roll's final owner fence. The
        // selected seal owner receives a clone, never public serialized facts.
        let seal = ctx
            .prepare_owner(&self.seal, S::opening(start, facts.clone(), profiles))
            .map_err(bad)?;
        let commit = ctx.commit_token();
        if seal.action() != &self.action
            || seal.commit() != commit
            || seal.open() != commit
            || seal.unit() != facts.unit()
            || seal.admission_digest() != facts.binding_digest()?
            || seal.close_event().is_some()
            || seal.close_kind().is_some()
            || seal.conditions().is_some()
            || seal.closing_unit_digest().is_some()
            || seal.sortie() != seal.transition()
        {
            return Err(PublicationError::Mismatch);
        }
        let custody = ctx
            .prepare_owner(
                &self.custody,
                IcCustodyRequest::Open {
                    unit: seal.unit(),
                    sortie: seal.sortie(),
                    seal_digest: seal.seal_digest(),
                },
            )
            .map_err(bad)?;
        if custody.action() != &self.action
            || custody.commit() != commit
            || custody.open() != commit
            || custody.unit() != seal.unit()
            || custody.sortie() != seal.sortie()
            || custody.event().is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let identity = IcIdentity {
            open: commit,
            unit: seal.unit(),
            sortie: seal.sortie(),
            seal_digest: seal.seal_digest(),
            custody_digest: custody.digest(),
        };
        let exclusion = ctx
            .prepare_owner(&self.exclusion, IcExclusionRequest::Open(identity.clone()))
            .map_err(bad)?;
        if exclusion.action() != &self.action
            || exclusion.commit() != commit
            || exclusion.identity() != &identity
            || exclusion.event().is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let h = facts.holding();
        let row = ClaimRecord {
            format: if facts.paid_ownership().is_some() {
                3
            } else if facts.charter_ownership().is_some() {
                2
            } else {
                1
            },
            open: OpenClaim {
                operation: commit,
                action: self.action.clone(),
                protocol: self.config.protocol.clone(),
                unit: facts.unit(),
                sortie: SortieNumber::try_from(seal.sortie())?,
                dispatcher: facts.dispatcher(),
                charter_ownership: facts.charter_ownership().cloned(),
                paid_ownership: facts.paid_ownership().copied(),
                hull_title_revision: facts.hull_title_revision(),
                licence: h.licence(),
                term: h.term(),
                charter: h.charter(),
                line: h.line(),
                filed: h.filed(),
                revision: h.identity(),
                rating: h.rating(),
                window: h.window(),
                capacity: facts.capacity(),
                at_ic: seal.at_ic().clone(),
                seal_digest: seal.seal_digest(),
                custody_digest: custody.digest(),
                exclusion_digest: exclusion.digest(),
            },
            closed: None,
        };
        let roots = super::roots::IcRoots::new(row.clone(), None, Some(facts))?;
        txn.insert(&r.families.ic_claims, &row.key(), row.clone())
            .map_err(bad)?;
        if txn.get(&r.families.ic_claims, &row.key()).map_err(bad)? != Some(&row) {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(&self.roots, RootRequest::Ic(roots))
            .map_err(bad)?;
        ctx.reserve_ic_mind_claim_completion(txn).map_err(bad)?;
        Ok(IcClaimReceipt {
            open: commit,
            closed: None,
        })
    }
    /// Drains only an actual opaque owner event. No current grant, holder,
    /// issuer term, parent eligibility or same-clock predicate enters Close.
    pub fn close(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        event: S::Close,
    ) -> Result<IcClaimReceipt, PublicationError> {
        let r = self.registration(Transition::Close)?;
        let open = S::closing_open(&event);
        let before = txn
            .get(&r.families.ic_claims, &open.to_hex())
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        before.validate()?;
        if before.closed.is_some() {
            return Err(PublicationError::Mismatch);
        }
        let seal = ctx
            .prepare_owner(&self.seal, S::closing(event))
            .map_err(bad)?;
        let commit = ctx.commit_token();
        if seal.action() != &self.action
            || seal.commit() != commit
            || seal.open() != open
            || seal.unit() != before.open.unit
            || seal.sortie() != before.open.sortie.get()
            || seal.seal_digest() != before.open.seal_digest
            || seal.admission_digest() != before.open.admission_digest()?
        {
            return Err(PublicationError::Mismatch);
        }
        let actual = seal.close_event().ok_or(PublicationError::Missing)?.clone();
        let conditions = seal.conditions().ok_or(PublicationError::Missing)?.clone();
        let kind = match seal.close_kind().ok_or(PublicationError::Missing)? {
            IcCloseKind::ReturnedToBase { base } => CloseKind::ReturnedToBase { base },
            IcCloseKind::PoweredDownInBase { base } => CloseKind::PoweredDownInBase { base },
            IcCloseKind::Destroyed => CloseKind::Destroyed {},
            IcCloseKind::RecoveredToBase { base } => CloseKind::RecoveredToBase { base },
        };
        let custody = ctx
            .prepare_owner(
                &self.custody,
                IcCustodyRequest::Close {
                    open,
                    event: actual.clone(),
                    conditions,
                },
            )
            .map_err(bad)?;
        let exclusion = ctx
            .prepare_owner(
                &self.exclusion,
                IcExclusionRequest::Close {
                    open,
                    event: actual.clone(),
                },
            )
            .map_err(bad)?;
        let identity = IcIdentity {
            open,
            unit: before.open.unit,
            sortie: before.open.sortie.get(),
            seal_digest: before.open.seal_digest,
            custody_digest: before.open.custody_digest,
        };
        if custody.action() != &self.action
            || custody.commit() != commit
            || custody.open() != open
            || custody.unit() != identity.unit
            || custody.sortie() != identity.sortie
            || custody.digest() != identity.custody_digest
            || custody.event() != Some(&actual)
            || Some(custody.unit_digest()) != seal.closing_unit_digest()
            || exclusion.action() != &self.action
            || exclusion.commit() != commit
            || exclusion.identity() != &identity
            || exclusion.digest() != before.open.exclusion_digest
            || exclusion.event() != Some(&actual)
        {
            return Err(PublicationError::Mismatch);
        }
        let mut row = before.clone();
        row.closed = Some(CloseCause {
            operation: commit,
            action: self.action.clone(),
            protocol: self.config.protocol.clone(),
            open,
            open_digest: before.open.digest()?,
            unit: identity.unit,
            sortie: before.open.sortie,
            transition: seal.transition(),
            event_sequence: actual.sequence,
            event_digest: actual.digest,
            kind,
            at_ic: seal.at_ic().clone(),
        });
        let roots = super::roots::IcRoots::new(row.clone(), Some(before), None)?;
        txn.replace(&r.families.ic_claims, &row.key(), row.clone())
            .map_err(bad)?;
        if txn.get(&r.families.ic_claims, &row.key()).map_err(bad)? != Some(&row) {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(&self.roots, RootRequest::Ic(roots))
            .map_err(bad)?;
        ctx.close_ic_mind_claim_completion(txn, open).map_err(bad)?;
        Ok(IcClaimReceipt {
            open,
            closed: Some(commit),
        })
    }
}
