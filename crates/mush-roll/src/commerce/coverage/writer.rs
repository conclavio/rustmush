//! The fixed Tacsim writer consumes real opaque native output and the checked
//! Roll observation. No family handles, generic root request or health setter
//! escape this composition cell.
use super::{
    CoverageOpenFacts, CoverageOpenReader, CoverageOpenRequest, CoverageProtocol,
    CoverageRetention, roots::CoverageRoots,
};
use crate::commerce::{
    families::CommerceFamilies,
    participant::{RootParticipant, RootRequest},
    records::{PublicationError, Record},
};
use mush_core::{
    ActionTxn, CommitToken, DefinitionRef, ModError, PrepareCtx, Registrar,
    participant::ParticipantHandle, read_service::OwnerReadHandle,
};
use mush_economy::managed_coverage::{CoverageHistory, CoverageOutput};
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
    protocol: CoverageProtocol,
}
#[derive(Clone, Default)]
pub struct CoverageWriter(Arc<OnceLock<Registration>>);
impl std::fmt::Debug for CoverageWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoverageWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
impl CoverageWriter {
    pub fn new() -> Self {
        Self::default()
    }
    pub(crate) fn install(
        &self,
        reg: &mut Registrar<'_>,
        families: CommerceFamilies,
        protocol: CoverageProtocol,
    ) -> Result<(), ModError> {
        protocol.validate().map_err(error)?;
        if reg.module() != crate::NS || self.0.get().is_some() {
            return Err(error(
                "coverage writer requires actual once-only Roll registration",
            ));
        }
        reg.delegate_state_family(&families.coverages, "tacsim")?;
        reg.delegate_state_family(&families.coverage_publications, "tacsim")?;
        self.0
            .set(Registration { families, protocol })
            .map_err(|_| error("coverage writer already installed"))
    }
    pub fn validate_registration(&self, protocol: &CoverageProtocol) -> Result<(), ModError> {
        protocol.validate().map_err(error)?;
        if &self
            .0
            .get()
            .ok_or_else(|| error("coverage Roll registration unavailable"))?
            .protocol
            != protocol
        {
            return Err(error("coverage exact composition differs"));
        }
        Ok(())
    }
    pub fn for_action(
        &self,
        reg: &mut Registrar<'_>,
        protocol: &CoverageProtocol,
        action: &DefinitionRef,
        reader: OwnerReadHandle<CoverageOpenReader>,
    ) -> Result<RegisteredCoverageWriter, ModError> {
        protocol.validate().map_err(error)?;
        if reg.module() != "tacsim" || !protocol.actions().map_err(error)?[1..].contains(action) {
            return Err(error("coverage writer requires fixed action"));
        }
        let writer = RegisteredCoverageWriter {
            registration: self.clone(),
            protocol: protocol.clone(),
            action: action.clone(),
            reader,
            roots: reg.request_owner_participant::<RootParticipant>(action.clone())?,
        };
        if self.0.get().is_some() {
            writer.registration().map_err(error)?;
        }
        Ok(writer)
    }
    /// This separate handle is confined to the exact publication action. It
    /// accepts only opaque Conduct proofs from the registered read service.
    pub fn for_publication(
        &self,
        reg: &mut Registrar<'_>,
        protocol: &CoverageProtocol,
        action: &DefinitionRef,
    ) -> Result<RegisteredCoveragePublicationWriter, ModError> {
        protocol.validate().map_err(error)?;
        if reg.module() != "tacsim" || &protocol.action(0).map_err(error)? != action {
            return Err(error("coverage publication requires fixed action"));
        }
        Ok(RegisteredCoveragePublicationWriter {
            registration: self.clone(),
            protocol: protocol.clone(),
            action: action.clone(),
            roots: reg.request_owner_participant::<RootParticipant>(action.clone())?,
        })
    }
    pub fn publications_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<Vec<super::CoveragePublicationRetention>, PublicationError> {
        if ctx.module.as_deref() != Some("tacsim") {
            return Err(PublicationError::Unavailable);
        }
        let r = self.0.get().ok_or(PublicationError::Unavailable)?;
        let roll = ctx
            .state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        if ctx
            .w
            .module_state_owner(std::any::TypeId::of::<crate::RollStore>())
            .is_none_or(|o| o.as_str() != crate::NS)
        {
            return Err(PublicationError::Unavailable);
        }
        roll.coverage_audit_ready().map_err(bad)?;
        super::publication::all(ctx, roll, &r.families, true)
    }
    pub fn verify_qualified_quote_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        open: CommitToken,
        selected: &super::CoverageConductRef,
    ) -> Result<(), PublicationError> {
        if ctx.module.as_deref() != Some("tacsim") {
            return Err(PublicationError::Unavailable);
        }
        let r = self.0.get().ok_or(PublicationError::Unavailable)?;
        let claim = crate::commerce::audit::read(ctx, &r.families.ic_claims, open.to_hex())?;
        claim.validate()?;
        if claim.open.charter != selected.charter()
            || claim.open.line != selected.line()
            || claim.open.filed != selected.filed()
            || claim.open.revision != selected.revision()
            || claim.open.rating != selected.rating()
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    pub fn require_audited(&self, world: &mush_core::World) -> Result<(), PublicationError> {
        self.0.get().ok_or(PublicationError::Unavailable)?;
        if world
            .module_state_owner(std::any::TypeId::of::<crate::RollStore>())
            .is_none_or(|owner| owner.as_str() != crate::NS)
        {
            return Err(PublicationError::Unavailable);
        }
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(bad)
    }
    /// Independent final boot audit over actual loaded owner images. Final hook
    /// ordering never supplies authority: this cannot open Roll or mint a host
    /// permit, and runtime still requires Roll's separate completed audit.
    pub fn bindings_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<Vec<CoverageRetention>, PublicationError> {
        if ctx.module.as_deref() != Some("tacsim") {
            return Err(PublicationError::Unavailable);
        }
        let r = self.0.get().ok_or(PublicationError::Unavailable)?;
        let roll = ctx
            .state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?;
        if ctx
            .w
            .module_state_owner(std::any::TypeId::of::<crate::RollStore>())
            .is_none_or(|owner| owner.as_str() != crate::NS)
        {
            return Err(PublicationError::Unavailable);
        }
        roll.coverage_audit_ready().map_err(bad)?;
        super::audit::all(ctx, roll, &r.families, true)
    }
}

pub struct RegisteredCoveragePublicationWriter {
    registration: CoverageWriter,
    protocol: CoverageProtocol,
    action: DefinitionRef,
    roots: ParticipantHandle<RootParticipant>,
}
impl RegisteredCoveragePublicationWriter {
    pub fn publish(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        actor: mush_core::ObjId,
        publication: mush_core::DefinitionDigest,
        product: mush_core::DefinitionDigest,
        proofs: Vec<crate::commerce::conduct::ConductTrialProof>,
    ) -> Result<(), PublicationError> {
        let r = self
            .registration
            .0
            .get()
            .ok_or(PublicationError::Unavailable)?;
        if r.protocol != self.protocol || self.action != r.protocol.action(0)? {
            return Err(PublicationError::Mismatch);
        }
        let row = super::CoveragePublicationRetention::new(
            ctx.commit_token(),
            self.action.clone(),
            self.protocol.clone(),
            publication,
            product,
            &proofs,
        )?;
        if txn
            .get(&r.families.coverage_publications, &row.key())
            .map_err(bad)?
            .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        let fence = super::publication::PublicationFence::new(
            self.action.clone(),
            ctx.commit_token(),
            actor,
            ctx.world().map_err(bad)?.now(),
            proofs.clone(),
        );
        let request = CoverageRoots::publication(&row, fence, &proofs)?;
        txn.insert(&r.families.coverage_publications, &row.key(), row.clone())
            .map_err(bad)?;
        ctx.prepare_owner(&self.roots, RootRequest::Coverage(request))
            .map_err(bad)?;
        if txn
            .get(&r.families.coverage_publications, &row.key())
            .map_err(bad)?
            != Some(&row)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
}

/// This observation is bound to the actual action and commit that read it.
/// It deliberately has no Clone/Deserialize or external constructor.
pub struct PreparedCoverageOpen {
    action: DefinitionRef,
    commit: CommitToken,
    facts: CoverageOpenFacts,
}
impl PreparedCoverageOpen {
    pub fn facts(&self) -> &CoverageOpenFacts {
        &self.facts
    }
}
pub struct CoverageWriteResult {
    retention: CoverageRetention,
    history: CoverageHistory,
}
impl CoverageWriteResult {
    pub fn retention(&self) -> &CoverageRetention {
        &self.retention
    }
    pub fn history(&self) -> &CoverageHistory {
        &self.history
    }
}
pub struct RegisteredCoverageWriter {
    registration: CoverageWriter,
    protocol: CoverageProtocol,
    action: DefinitionRef,
    reader: OwnerReadHandle<CoverageOpenReader>,
    roots: ParticipantHandle<RootParticipant>,
}
impl RegisteredCoverageWriter {
    fn registration(&self) -> Result<&Registration, PublicationError> {
        let r = self
            .registration
            .0
            .get()
            .ok_or(PublicationError::Unavailable)?;
        if r.protocol != self.protocol || !r.protocol.actions()?[1..].contains(&self.action) {
            return Err(PublicationError::Mismatch);
        }
        Ok(r)
    }
    pub fn quote(
        &self,
        ctx: &mut PrepareCtx<'_>,
        observed: PreparedCoverageOpen,
        extra: Arc<dyn super::CoveragePrecommitCheck>,
    ) -> Result<(), PublicationError> {
        let r = self.registration()?;
        if self.action != r.protocol.action(1)?
            || observed.action != self.action
            || observed.commit != ctx.commit_token()
        {
            return Err(PublicationError::Mismatch);
        }
        let request = CoverageRoots::quote(
            self.action.clone(),
            ctx.commit_token(),
            ctx.world().map_err(bad)?.now(),
            observed.facts,
            extra,
        )?;
        ctx.prepare_owner(&self.roots, RootRequest::Coverage(request))
            .map_err(bad)?;
        Ok(())
    }
    pub fn observe(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        open: CommitToken,
    ) -> Result<PreparedCoverageOpen, PublicationError> {
        let r = self.registration()?;
        if ![r.protocol.action(1)?, r.protocol.action(2)?].contains(&self.action) {
            return Err(PublicationError::Mismatch);
        }
        let facts = ctx
            .read_owner_preimage(txn, &self.reader, CoverageOpenRequest::acknowledged(open))
            .map_err(bad)?;
        Ok(PreparedCoverageOpen {
            action: self.action.clone(),
            commit: ctx.commit_token(),
            facts,
        })
    }
    pub fn bind(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        observed: PreparedCoverageOpen,
        output: CoverageOutput,
        extra: Arc<dyn super::CoveragePrecommitCheck>,
    ) -> Result<CoverageWriteResult, PublicationError> {
        let r = self.registration()?;
        if self.action != r.protocol.action(2)?
            || observed.action != self.action
            || observed.commit != ctx.commit_token()
        {
            return Err(PublicationError::Mismatch);
        }
        let row = CoverageRetention::bind(&output, &observed.facts)?;
        if row.operation() != ctx.commit_token() || row.consent().effect != self.action {
            return Err(PublicationError::Mismatch);
        }
        // Complete future envelope, with the independently enforced native
        // terminal decision ceiling. This is byte accounting, not a receipt.
        let descriptor = r.families.coverages.descriptor();
        if r.families.coverages.owner().as_str() != crate::NS
            || descriptor.id().as_str() != CoverageRetention::FAMILY
            || descriptor.schema_version() != 1
            || descriptor.max_record_bytes() != CoverageRetention::MAX_BYTES
        {
            return Err(PublicationError::Mismatch);
        }
        let doc = serde_json::json!({"family":{"owner":r.families.coverages.owner(),"id":descriptor.id()},
            "key":row.key(),"schema":{"id":descriptor.value_schema(),"version":descriptor.schema_version()},"value":row});
        let now = mush_core::canonical_json_bytes_with_limit(&doc, CoverageRetention::MAX_BYTES)
            .map_err(bad)?
            .len();
        if now
            .checked_sub(4)
            .and_then(|n| n.checked_add(mush_economy::managed_coverage::MAX_DECISION_BYTES))
            .is_none_or(|n| n > CoverageRetention::MAX_BYTES)
        {
            return Err(PublicationError::Capacity);
        }
        let key = row.key();
        if txn.get(&r.families.coverages, &key).map_err(bad)?.is_some() {
            return Err(PublicationError::Mismatch);
        }
        txn.insert(&r.families.coverages, &key, row.clone())
            .map_err(bad)?;
        let request = CoverageRoots::bind(&row, observed.facts, extra)?;
        ctx.prepare_owner(&self.roots, RootRequest::Coverage(request))
            .map_err(bad)?;
        if txn.get(&r.families.coverages, &key).map_err(bad)? != Some(&row) {
            return Err(PublicationError::Mismatch);
        }
        Ok(CoverageWriteResult {
            retention: row,
            history: output.history().clone(),
        })
    }
    pub fn finish(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        output: CoverageOutput,
        extra: Arc<dyn super::CoveragePrecommitCheck>,
    ) -> Result<CoverageWriteResult, PublicationError> {
        let r = self.registration()?;
        let history = output.history();
        let terminal = history
            .terminal()
            .ok_or(PublicationError::Mismatch)?
            .decision();
        let index = if terminal.reason.pays() { 3 } else { 4 };
        if self.action != r.protocol.action(index)?
            || terminal.action != self.action
            || terminal.operation != ctx.commit_token()
        {
            return Err(PublicationError::Mismatch);
        }
        let key = history.consent().operation.to_hex();
        let before = txn
            .get(&r.families.coverages, &key)
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        if before.key() != key {
            return Err(PublicationError::Mismatch);
        }
        let claim = txn
            .get(
                &r.families.ic_claims,
                &history.consent().basis.ic_open.to_hex(),
            )
            .map_err(bad)?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        let after = before.finish(&output, &claim)?;
        txn.replace(&r.families.coverages, &key, after.clone())
            .map_err(bad)?;
        ctx.prepare_owner(
            &self.roots,
            RootRequest::Coverage(CoverageRoots::terminal(&before, &after, claim, extra)?),
        )
        .map_err(bad)?;
        if txn.get(&r.families.coverages, &key).map_err(bad)? != Some(&after) {
            return Err(PublicationError::Mismatch);
        }
        Ok(CoverageWriteResult {
            retention: after,
            history: history.clone(),
        })
    }
}
