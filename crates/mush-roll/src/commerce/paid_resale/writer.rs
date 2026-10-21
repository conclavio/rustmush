//! Trusted paired publication. Actual owner observations and native output are
//! inseparable from this action/commit; no decoded history can publish a holder.
use super::{
    history, protocol,
    publication::{Mode, ResaleFence, ResaleRoots},
    reader::{PaidResaleFacts, PaidResaleReader, PaidResaleRequest},
    records::*,
};
use crate::commerce::{
    families::CommerceFamilies,
    participant::{RootParticipant, RootRequest},
    records::{PublicationError, Record},
    roots::{RequiredRoot, RootAnchor},
};
use mush_core::{
    ActionTxn, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ModError, PrepareCtx,
    Registrar, StateFamilyHandle,
};
use mush_core::{participant::ParticipantHandle, read_service::OwnerReadHandle};
use mush_economy::managed_paid_resale::{
    MAX_HISTORY_BYTES, PaidResaleModeV1, PaidResaleOutcome, PaidResaleOutput,
};
use std::sync::{Arc, OnceLock};
// TEMP M7 diag: name the swallowing site behind M7_CONDUCT_DIAG.
fn bad_at(e: impl std::fmt::Display, line: u32) -> PublicationError {
    if std::env::var_os("M7_CONDUCT_DIAG").is_some() {
        eprintln!("M7_CONDUCT_DIAG {}:{line}: {e}", file!());
    }
    bad(e)
}
fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Mismatch
}
#[derive(Clone)]
struct Registration {
    families: CommerceFamilies,
    actions: [DefinitionRef; 5],
    runtime: super::service::Runtime,
    ready: super::Readiness,
    ratings: crate::commerce::rating::RatingFamilies,
    service_actor: mush_core::ObjId,
    revocation: Option<DefinitionRef>,
}
#[derive(Clone, Default)]
pub struct PaidResaleWriter(
    Arc<OnceLock<Registration>>,
    Arc<OnceLock<Arc<dyn super::PaidResaleMarketWriter>>>,
);
impl std::fmt::Debug for PaidResaleWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaidResaleWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
impl PaidResaleWriter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn install_market(
        &self,
        reg: &Registrar<'_>,
        writer: Arc<dyn super::PaidResaleMarketWriter>,
    ) -> Result<(), ModError> {
        if reg.module() != "market" {
            return Err(ModError::msg("paid resale Market owner differs"));
        }
        self.1
            .set(writer)
            .map_err(|_| ModError::msg("paid resale Market adapter already installed"))
    }
    pub(crate) fn compound_parts(
        &self,
    ) -> Result<
        (
            CommerceFamilies,
            super::Readiness,
            Arc<dyn super::PaidResaleMarketWriter>,
        ),
        PublicationError,
    > {
        let r = self.0.get().ok_or(PublicationError::Unavailable)?;
        r.ready.check()?;
        Ok((
            r.families.clone(),
            r.ready.clone(),
            self.1.get().ok_or(PublicationError::Unavailable)?.clone(),
        ))
    }
    pub(crate) fn install(
        &self,
        reg: &mut Registrar<'_>,
        families: CommerceFamilies,
        actions: [DefinitionRef; 5],
        ratings: crate::commerce::rating::RatingFamilies,
        service_actor: mush_core::ObjId,
        revocation: Option<DefinitionRef>,
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS || self.0.get().is_some() || actions != protocol::actions()? {
            return Err(ModError::msg("paid resale writer composition differs"));
        }
        reg.delegate_state_family(&families.paid_resales, "market")?;
        reg.delegate_state_family(&families.paid_resale_heads, "market")?;
        reg.delegate_state_family(&families.paid_holders, "market")?;
        reg.delegate_state_family(&families.paid_licences, "market")?;
        // Market's paid-resale boot audit re-proves each offer's and
        // operation's binding (`binding_for_boot` / `operation_for_boot`) from
        // its own ModuleCtx, and the birth resolver reads every birth route
        // whichever purchase writers are composed: read-only grants.
        reg.grant_state_family_read(&families.paid_allocations, "market")?;
        reg.grant_state_family_read(&families.entitlement_allocations, "market")?;
        reg.grant_state_family_read(&families.house_allocations, "market")?;
        reg.grant_state_family_read(&families.paid_terms, "market")?;
        reg.grant_state_family_read(&families.term_economics, "market")?;
        reg.grant_state_family_read(&families.house_economics, "market")?;
        reg.grant_state_family_read(&families.filed_royalties, "market")?;
        reg.grant_state_family_read(&families.renewal_operations, "market")?;
        reg.grant_state_family_read(&families.house_renewal_operations, "market")?;
        reg.grant_state_family_read(&ratings.receipts, "market")?;
        // The service's settle and refund (`observe_terminal`) read the
        // licence's revocation status inside Market's transaction.
        reg.grant_state_family_read(&families.revoked_terms, "market")?;
        reg.grant_state_family_read(&families.registry_revocations, "market")?;
        if service_actor.0 == 0 {
            return Err(ModError::msg("paid resale service actor differs"));
        }
        let runtime = super::service::Runtime::default();
        let ready = super::Readiness::default();
        super::service::register(
            reg,
            runtime.clone(),
            ready.clone(),
            families.clone(),
            ratings.clone(),
            service_actor,
        )?;
        self.0
            .set(Registration {
                families,
                actions,
                runtime,
                ready,
                ratings,
                service_actor,
                revocation,
            })
            .map_err(|_| ModError::msg("paid resale writer already registered"))
    }
    pub(crate) fn capture_ready(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        self.0
            .get()
            .ok_or_else(|| ModError::msg("paid resale writer absent"))?
            .ready
            .capture(ctx)
    }
    pub fn validate_registration(&self, actions: &[DefinitionRef; 5]) -> Result<(), ModError> {
        let r = self
            .0
            .get()
            .ok_or_else(|| ModError::msg("paid resale writer absent"))?;
        if actions != &protocol::actions()? || actions != &r.actions || self.1.get().is_none() {
            return Err(ModError::msg("paid resale writer actions differ"));
        }
        Ok(())
    }
    fn boot_registration(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<&Registration, PublicationError> {
        if ctx.module.as_deref() != Some("market")
            || !ctx
                .state::<crate::RollStore>()
                .is_some_and(|s| s.commerce_ready().is_ok())
        {
            return Err(PublicationError::Unavailable);
        }
        self.0.get().ok_or(PublicationError::Unavailable)
    }
    pub fn validate_service_binding(
        &self,
        actor: mush_core::ObjId,
        revocation: Option<&DefinitionRef>,
    ) -> Result<(), ModError> {
        if self
            .0
            .get()
            .is_none_or(|r| r.service_actor != actor || r.revocation.as_ref() != revocation)
        {
            return Err(ModError::msg("paid resale service actor differs"));
        }
        Ok(())
    }
    pub fn operations_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
    ) -> Result<Vec<CommitToken>, PublicationError> {
        let r = self.boot_registration(ctx)?;
        let mut rows = Vec::new();
        crate::commerce::audit::scan(ctx, &r.families.paid_resales, |row| {
            if rows.len() >= MAX_OPERATIONS {
                return Err(PublicationError::Capacity);
            }
            rows.push(row.operation());
            Ok(())
        })?;
        Ok(rows)
    }
    pub fn operation_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        operation: CommitToken,
    ) -> Result<PaidResaleRecord, PublicationError> {
        let r = self.boot_registration(ctx)?;
        let row = crate::commerce::audit::read(ctx, &r.families.paid_resales, operation.to_hex())?;
        let proof = super::audit::binding(ctx, &r.families, &r.ratings, row.binding())?;
        history::native(ctx.w, row)?;
        history::roots(ctx.w, row, proof.revision)?;
        Ok(row.clone())
    }
    pub fn binding_for_boot(
        &self,
        ctx: &mush_core::ModuleCtx<'_>,
        binding: &PaidResaleBinding,
    ) -> Result<super::HistoricalBinding, PublicationError> {
        let r = self.boot_registration(ctx)?;
        super::audit::binding(ctx, &r.families, &r.ratings, binding)
    }
    pub fn for_action(
        &self,
        reg: &mut Registrar<'_>,
        action: &DefinitionRef,
        reader: OwnerReadHandle<PaidResaleReader>,
    ) -> Result<RegisteredPaidResaleWriter, ModError> {
        if reg.module() != "market" || !protocol::actions()?.contains(action) {
            return Err(ModError::msg("paid resale writer action differs"));
        }
        let roots = if action == &protocol::actions()?[0] {
            None
        } else {
            Some(reg.request_owner_participant::<RootParticipant>(action.clone())?)
        };
        Ok(RegisteredPaidResaleWriter {
            registration: self.clone(),
            action: action.clone(),
            reader,
            roots,
        })
    }
}
/// No Deserialize, Clone or public constructor: actual observation is tied to
/// preparation; serialized output is disclosure data, not a publication token.
pub struct PreparedPaidResale {
    state: Box<PreparedPaidResaleState>,
}
struct PreparedPaidResaleState {
    action: DefinitionRef,
    commit: CommitToken,
    at_ic: ClockStamp,
    at_utc: i64,
    facts: PaidResaleFacts,
}
impl PreparedPaidResale {
    pub fn facts(&self) -> &PaidResaleFacts {
        &self.state.facts
    }
}
pub struct RegisteredPaidResaleWriter {
    registration: PaidResaleWriter,
    action: DefinitionRef,
    reader: OwnerReadHandle<PaidResaleReader>,
    roots: Option<ParticipantHandle<RootParticipant>>,
}
/// Opaque private service observation, minted only from its live retained
/// registered intent. Neither an action selector nor decoded native history
/// can construct this value.
pub struct PreparedPaidResaleTerminal {
    state: Box<PreparedPaidResaleTerminalState>,
}
struct PreparedPaidResaleTerminalState {
    before: PaidResaleRecord,
    pointer: crate::commerce::paid::PersonalPaidLicence,
    intent: super::service::Intent,
    decision: mush_economy::managed_paid_resale::PaidResaleDecisionV1,
}
impl PreparedPaidResaleTerminal {
    pub fn before(&self) -> &PaidResaleRecord {
        &self.state.before
    }
    pub fn request(&self) -> mush_economy::managed_paid_resale::PaidResaleTerminalRequest {
        mush_economy::managed_paid_resale::PaidResaleTerminalRequest {
            acceptance: self.state.before.operation(),
            decision: self.state.decision.clone(),
        }
    }
}
pub struct PaidResaleServiceOffer {
    intent: super::service::Intent,
}
impl PaidResaleServiceOffer {
    pub fn parameters(&self) -> serde_json::Value {
        self.intent.parameters()
    }
    pub fn guard(&self) -> Result<mush_core::action::AuthorityGuard, PublicationError> {
        self.intent.guard()
    }
}
impl RegisteredPaidResaleWriter {
    fn registration(&self) -> Result<&Registration, PublicationError> {
        let r = self
            .registration
            .0
            .get()
            .ok_or(PublicationError::Unavailable)?;
        if !r.actions.contains(&self.action) {
            return Err(PublicationError::Mismatch);
        }
        Ok(r)
    }
    pub fn observe(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        request: PaidResaleRequest,
    ) -> Result<PreparedPaidResale, PublicationError> {
        let r = self.registration()?;
        let seller = matches!(request, PaidResaleRequest::Seller { .. });
        if (seller && self.action != r.actions[0])
            || (!seller && self.action != r.actions[1] && self.action != r.actions[2])
        {
            return Err(PublicationError::Mismatch);
        }
        let facts = ctx
            .read_owner_preimage(txn, &self.reader, request)
            .map_err(|e| bad_at(e, line!()))?;
        let counterparty = if seller {
            facts.buyer()
        } else {
            facts.seller()
        };
        let accounts = ctx.distinct_personal_accounts(counterparty).map_err(|e| bad_at(e, line!()))?;
        if accounts.requester() != facts.requester() || accounts.other() != counterparty {
            return Err(PublicationError::Mismatch);
        }
        let world = ctx.world().map_err(|e| bad_at(e, line!()))?;
        Ok(PreparedPaidResale {
            state: Box::new(PreparedPaidResaleState {
                action: self.action.clone(),
                commit: ctx.commit_token(),
                at_ic: world.world_ic_now(),
                at_utc: world.now(),
                facts,
            }),
        })
    }
    fn observed<'a>(
        &self,
        ctx: &PrepareCtx<'_>,
        v: &'a PreparedPaidResale,
    ) -> Result<&'a PaidResaleFacts, PublicationError> {
        self.registration()?;
        if v.state.action != self.action || v.state.commit != ctx.commit_token() {
            return Err(PublicationError::Mismatch);
        }
        v.state.facts.check_world(ctx.world().map_err(|e| bad_at(e, line!()))?)?;
        Ok(&v.state.facts)
    }
    fn before(
        &self,
        txn: &mut ActionTxn<'_>,
        facts: &PaidResaleFacts,
    ) -> Result<(), PublicationError> {
        let f = &self.registration()?.families;
        if txn
            .get(&f.paid_licences, &facts.pointer().key())
            .map_err(|e| bad_at(e, line!()))?
            != Some(facts.pointer())
            || txn
                .get(&f.paid_terms, &facts.binding().term().key())
                .map_err(|e| bad_at(e, line!()))?
                != Some(facts.binding().term())
        {
            return Err(PublicationError::Mismatch);
        }
        let head = txn
            .get(&f.paid_resale_heads, &facts.pointer().id().key())
            .map_err(|e| bad_at(e, line!()))?
            .cloned();
        match (head, facts.previous()) {
            (None, None) => (),
            (Some(head), Some(row)) => {
                head.matches(row)?;
                if txn.get(&f.paid_resales, &row.key()).map_err(|e| bad_at(e, line!()))? != Some(row)
                    || matches!(row.state(), PaidResaleState::Funded {})
                {
                    return Err(PublicationError::Mismatch);
                }
            }
            _ => return Err(PublicationError::Mismatch),
        }
        let next = PaidHolderRecord::key_for(
            facts.pointer().id(),
            facts.binding().ownership().generation + 1,
        );
        if txn.get(&f.paid_holders, &next).map_err(|e| bad_at(e, line!()))?.is_some() {
            return Err(PublicationError::Mismatch);
        }
        Ok(())
    }
    /// Prospective complete envelope at the actual fixed family ceiling. The
    /// opaque native value is charged at its entire maximum, never decoded as
    /// authority here. Wider future state/sequence/claim values are budget data.
    pub fn preflight(
        &self,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        observation: &PreparedPaidResale,
    ) -> Result<(), PublicationError> {
        let facts = self.observed(ctx, observation)?;
        self.before(txn, facts)?;
        let f = &self.registration()?.families;
        let token = CommitToken::from_bytes([255; 16]);
        let d = DefinitionDigest::from_bytes([255; 32]);
        let value = serde_json::json!({"format":1,"operation":token,"action":protocol::action(2)?,"sequence":65536,
            "previous":{"sequence":65535,"operation":token,"digest":d},"offer":token,"offer_digest":d,
            "binding":facts.binding(),"claim_observation":{"generation":u64::MAX,"next_transition":u64::MAX,"open_paid":32768},
            "economic":"x".repeat(MAX_HISTORY_BYTES-2),
            "state":{"phase":"refunded","terminal":token,"reason":{"kind":"registry_revoked","sanction":token,
                "licence":u64::MAX,"generation":u64::MAX,"term_digest":d}}});
        envelope(&f.paid_resales, &token.to_hex(), value)
    }
    /// Called only after the actual retained named offer has been verified by
    /// Market and the real Economy participant has prepared this exact output.
    pub fn accept(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        observation: &PreparedPaidResale,
        output: &PaidResaleOutput,
    ) -> Result<PaidResaleRecord, PublicationError> {
        self.preflight(ctx, txn, observation)?;
        let facts = self.observed(ctx, observation)?;
        self.before(txn, facts)?;
        let r = self.registration()?;
        let f = &r.families;
        let native = output.history();
        let consent = native.consent();
        let immediate = self.action == r.actions[1];
        if (!immediate && self.action != r.actions[2])
            || consent.operation != ctx.commit_token()
            || consent.effect != self.action
            || consent.terms.basis != facts.binding().native_basis()?
            || consent.terms.seller != facts.seller()
            || consent.terms.buyer != facts.buyer()
            || facts.requester() != facts.buyer()
            || consent.accepted_at_ic != observation.state.at_ic
            || consent.accepted_at_utc != observation.state.at_utc
            || consent.claims.next_transition != facts.claims().next_transition()
            || consent.claims.open_paid != facts.claims().open_paid()
            || immediate != (consent.mode == PaidResaleModeV1::Immediate)
            || native.terminal().is_some()
            || (immediate && native.outcome() != PaidResaleOutcome::Settled)
            || (!immediate && native.outcome() != PaidResaleOutcome::Funded)
        {
            return Err(PublicationError::Mismatch);
        }
        let holder = if immediate {
            Some(PaidHolderRecord::from_history(facts.binding(), native)?)
        } else {
            None
        };
        let state = match &holder {
            Some(h) => PaidResaleState::Settled {
                terminal: h.operation(),
                ownership: h.ownership()?,
            },
            None => PaidResaleState::Funded {},
        };
        let row = PaidResaleRecord {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            sequence: facts.previous().map_or(Ok(1), |p| {
                p.sequence()
                    .checked_add(1)
                    .ok_or(PublicationError::Capacity)
            })?,
            previous: facts
                .previous()
                .map(PaidResaleRecord::reference)
                .transpose()?,
            offer: consent.terms.offer,
            offer_digest: consent.offer_digest,
            binding: facts.binding().clone(),
            claim_observation: facts.claims().clone(),
            economic: native.clone(),
            state,
        };
        row.validate()?;
        let head = PaidResaleHead::from_record(&row)?;
        envelope(
            &f.paid_resales,
            &row.key(),
            serde_json::to_value(&row).map_err(|e| bad_at(e, line!()))?,
        )?;
        let mut old = vec![RequiredRoot {
            anchor: RootAnchor::PaidOperation(facts.pointer().operation()),
            identity: facts.revision(),
        }];
        if let Some(previous) = facts.previous() {
            old.push(history::terminal_root(previous, facts.revision())?);
        }
        let mut added = vec![history::acceptance_root(&row, facts.revision())?];
        if immediate {
            added.push(history::terminal_root(&row, facts.revision())?);
        }
        let roots = ResaleRoots {
            commit: ctx.commit_token(),
            old,
            added,
            fence: ResaleFence {
                action: self.action.clone(),
                commit: ctx.commit_token(),
                at_ic: observation.state.at_ic.clone(),
                at_utc: observation.state.at_utc,
                mode: Mode::Admission(Box::new(facts.clone())),
            },
        };
        txn.insert(&f.paid_resales, &row.key(), row.clone())
            .map_err(|e| bad_at(e, line!()))?;
        if facts.previous().is_some() {
            txn.replace(&f.paid_resale_heads, &head.key(), head.clone())
                .map_err(|e| bad_at(e, line!()))?;
        } else {
            txn.insert(&f.paid_resale_heads, &head.key(), head.clone())
                .map_err(|e| bad_at(e, line!()))?;
        }
        if let Some(holder) = holder {
            let pointer = facts
                .pointer()
                .transferred(facts.binding().birth(), &holder)?;
            envelope(
                &f.paid_holders,
                &holder.key(),
                serde_json::to_value(&holder).map_err(|e| bad_at(e, line!()))?,
            )?;
            envelope(
                &f.paid_licences,
                &pointer.key(),
                serde_json::to_value(&pointer).map_err(|e| bad_at(e, line!()))?,
            )?;
            txn.insert(&f.paid_holders, &holder.key(), holder.clone())
                .map_err(|e| bad_at(e, line!()))?;
            txn.replace(&f.paid_licences, &pointer.key(), pointer.clone())
                .map_err(|e| bad_at(e, line!()))?;
            if txn.get(&f.paid_holders, &holder.key()).map_err(|e| bad_at(e, line!()))? != Some(&holder)
                || txn.get(&f.paid_licences, &pointer.key()).map_err(|e| bad_at(e, line!()))? != Some(&pointer)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        if txn.get(&f.paid_resales, &row.key()).map_err(|e| bad_at(e, line!()))? != Some(&row)
            || txn.get(&f.paid_resale_heads, &head.key()).map_err(|e| bad_at(e, line!()))? != Some(&head)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(
            self.roots.as_ref().ok_or(PublicationError::Unavailable)?,
            RootRequest::PaidResale(Box::new(roots)),
        )
        .map_err(|e| bad_at(e, line!()))?;
        Ok(row)
    }
    pub fn service_offer(
        &self,
        world: &mush_core::World,
        actor: mush_core::ObjId,
    ) -> Result<Option<PaidResaleServiceOffer>, PublicationError> {
        let r = self.registration()?;
        r.ready.check()?;
        let Some(intent) = r.runtime.projected() else {
            return Ok(None);
        };
        if intent.action != self.action || intent.actor != actor {
            return Ok(None);
        }
        r.runtime.require(&intent)?;
        intent.fresh(world)?;
        Ok(Some(PaidResaleServiceOffer { intent }))
    }
    pub fn check_service_invocation(
        &self,
        inv: &mush_core::action::ActionInvocation,
    ) -> Result<(), PublicationError> {
        let r = self.registration()?;
        r.ready.check()?;
        r.runtime.intent(inv, &self.action).map(|_| ())
    }
    pub fn observe_terminal(
        &self,
        ctx: &PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        inv: &mush_core::action::ActionInvocation,
    ) -> Result<PreparedPaidResaleTerminal, PublicationError> {
        let r = self.registration()?;
        r.ready.check()?;
        if self.action != r.actions[3] && self.action != r.actions[4] {
            return Err(PublicationError::Mismatch);
        }
        let intent = r.runtime.intent(inv, &self.action)?;
        intent.fresh(ctx.world().map_err(|e| bad_at(e, line!()))?)?;
        let f = &r.families;
        let before = txn
            .get(&f.paid_resales, &intent.acceptance.to_hex())
            .map_err(|e| bad_at(e, line!()))?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        before.validate()?;
        if before.reference()? != intent.before
            || !matches!(before.state(), PaidResaleState::Funded {})
        {
            return Err(PublicationError::Mismatch);
        }
        let b = before.binding();
        txn.get(&f.paid_resale_heads, &b.licence().key())
            .map_err(|e| bad_at(e, line!()))?
            .ok_or(PublicationError::Missing)?
            .matches(&before)?;
        let pointer = txn
            .get(&f.paid_licences, &b.licence().key())
            .map_err(|e| bad_at(e, line!()))?
            .cloned()
            .ok_or(PublicationError::Missing)?;
        pointer.validate()?;
        if pointer.id() != b.licence()
            || pointer.charter() != b.charter()
            || pointer.line() != b.line()
            || pointer.operation() != b.birth().operation
            || pointer.holder() != before.economic().consent().terms.seller
            || pointer.term() != b.term().generation()
            || pointer.ownership()
                != if b.ownership().generation == 1 {
                    None
                } else {
                    Some(b.ownership())
                }
            || txn.get(&f.paid_terms, &b.term().key()).map_err(|e| bad_at(e, line!()))? != Some(b.term())
            || txn
                .get(
                    &f.paid_holders,
                    &PaidHolderRecord::key_for(b.licence(), b.ownership().generation + 1),
                )
                .map_err(|e| bad_at(e, line!()))?
                .is_some()
        {
            return Err(PublicationError::Mismatch);
        }
        history::native(ctx.world().map_err(|e| bad_at(e, line!()))?, &before)?;
        history::roots(ctx.world().map_err(|e| bad_at(e, line!()))?, &before, intent.revision)?;
        use crate::commerce::registry_revocation::records::{AcceptedTerm, RevokedTerm};
        use mush_economy::managed_paid_resale::PaidResaleDecisionKindV1 as Reason;
        let status = txn
            .get(
                &f.revoked_terms,
                &RevokedTerm::key_for(b.licence(), b.term().generation()),
            )
            .map_err(|e| bad_at(e, line!()))?
            .cloned();
        let now = ctx.world().map_err(|e| bad_at(e, line!()))?.world_ic_now();
        let effective = now.at
            < before
                .economic()
                .consent()
                .terms
                .basis
                .term
                .window
                .grace_end;
        match (status, &intent.reason) {
            (
                Some(status),
                Reason::RegistryRevoked {
                    sanction,
                    licence,
                    generation,
                    term_digest,
                },
            ) if effective => {
                let op = txn
                    .get(&f.registry_revocations, &status.operation.to_hex())
                    .map_err(|e| bad_at(e, line!()))?
                    .ok_or(PublicationError::Missing)?;
                status.matches(op)?;
                if !matches!(&op.term, AcceptedTerm::Paid { term } if term.as_ref() == b.term())
                    || op.operation != *sanction
                    || *licence != b.licence().get()
                    || *generation != b.term().generation().get()
                    || *term_digest != b.native_basis()?.term.digest
                {
                    return Err(PublicationError::Mismatch);
                }
                ctx.world()
                    .map_err(|e| bad_at(e, line!()))?
                    .module_state::<crate::RollStore>()
                    .ok_or(PublicationError::Unavailable)?
                    .check_required_root(&op.root()?)
                    .map_err(|e| bad_at(e, line!()))?;
            }
            (None, Reason::RegistryRevoked { .. }) | (Some(_), Reason::RegistryRevoked { .. }) => {
                return Err(PublicationError::Mismatch);
            }
            (Some(_), _) if effective => return Err(PublicationError::Unavailable),
            _ => (),
        }
        let decision = mush_economy::managed_paid_resale::PaidResaleDecisionV1 {
            format: 1,
            operation: ctx.commit_token(),
            action: self.action.clone(),
            actor: inv.actor(),
            at_ic: now,
            posted_at_utc: ctx.world().map_err(|e| bad_at(e, line!()))?.now(),
            reason: intent.reason.clone(),
        };
        decision
            .matches_consent(before.economic().consent())
            .map_err(|e| bad_at(e, line!()))?;
        Ok(PreparedPaidResaleTerminal {
            state: Box::new(PreparedPaidResaleTerminalState {
                before,
                pointer,
                intent,
                decision,
            }),
        })
    }
    pub fn terminal(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        observed: &PreparedPaidResaleTerminal,
        output: &PaidResaleOutput,
    ) -> Result<PaidResaleRecord, PublicationError> {
        let r = self.registration()?;
        r.ready.check()?;
        r.runtime.require(&observed.state.intent)?;
        observed.state.intent.fresh(ctx.world().map_err(|e| bad_at(e, line!()))?)?;
        let before = &observed.state.before;
        let actual = output.history();
        let terminal = actual.terminal().ok_or(PublicationError::Missing)?;
        if observed.state.decision.operation != ctx.commit_token()
            || observed.state.decision.action != self.action
            || terminal.decision() != &observed.state.decision
            || actual.consent() != before.economic().consent()
            || actual.acceptance() != before.economic().acceptance()
            || actual.outcome() == PaidResaleOutcome::Funded
        {
            return Err(PublicationError::Mismatch);
        }
        let f = &r.families;
        if txn.get(&f.paid_resales, &before.key()).map_err(|e| bad_at(e, line!()))? != Some(before)
            || txn
                .get(&f.paid_licences, &observed.state.pointer.key())
                .map_err(|e| bad_at(e, line!()))?
                != Some(&observed.state.pointer)
        {
            return Err(PublicationError::Mismatch);
        }
        txn.get(&f.paid_resale_heads, &before.binding().licence().key())
            .map_err(|e| bad_at(e, line!()))?
            .ok_or(PublicationError::Missing)?
            .matches(before)?;
        let holder = if actual.outcome() == PaidResaleOutcome::Settled {
            Some(PaidHolderRecord::from_history(before.binding(), actual)?)
        } else {
            None
        };
        let mut row = before.clone();
        row.economic = actual.clone();
        row.state = if let Some(holder) = &holder {
            PaidResaleState::Settled {
                terminal: ctx.commit_token(),
                ownership: holder.ownership()?,
            }
        } else {
            PaidResaleState::Refunded {
                terminal: ctx.commit_token(),
                reason: observed.state.decision.reason.clone(),
            }
        };
        row.validate()?;
        let head = PaidResaleHead::from_record(&row)?;
        let roots = ResaleRoots {
            commit: ctx.commit_token(),
            old: vec![history::acceptance_root(
                before,
                observed.state.intent.revision,
            )?],
            added: vec![history::terminal_root(
                &row,
                observed.state.intent.revision,
            )?],
            fence: ResaleFence {
                action: self.action.clone(),
                commit: ctx.commit_token(),
                at_ic: observed.state.decision.at_ic.clone(),
                at_utc: observed.state.decision.posted_at_utc,
                mode: Mode::Service {
                    runtime: Box::new(r.runtime.clone()),
                    ready: r.ready.clone(),
                    intent: Box::new(observed.state.intent.clone()),
                },
            },
        };
        envelope(
            &f.paid_resales,
            &row.key(),
            serde_json::to_value(&row).map_err(|e| bad_at(e, line!()))?,
        )?;
        txn.replace(&f.paid_resales, &row.key(), row.clone())
            .map_err(|e| bad_at(e, line!()))?;
        txn.replace(&f.paid_resale_heads, &head.key(), head.clone())
            .map_err(|e| bad_at(e, line!()))?;
        if let Some(holder) = holder {
            let pointer = observed
                .state
                .pointer
                .transferred(row.binding().birth(), &holder)?;
            envelope(
                &f.paid_holders,
                &holder.key(),
                serde_json::to_value(&holder).map_err(|e| bad_at(e, line!()))?,
            )?;
            txn.insert(&f.paid_holders, &holder.key(), holder.clone())
                .map_err(|e| bad_at(e, line!()))?;
            txn.replace(&f.paid_licences, &pointer.key(), pointer.clone())
                .map_err(|e| bad_at(e, line!()))?;
            if txn.get(&f.paid_holders, &holder.key()).map_err(|e| bad_at(e, line!()))? != Some(&holder)
                || txn.get(&f.paid_licences, &pointer.key()).map_err(|e| bad_at(e, line!()))? != Some(&pointer)
            {
                return Err(PublicationError::Mismatch);
            }
        }
        if txn.get(&f.paid_resales, &row.key()).map_err(|e| bad_at(e, line!()))? != Some(&row)
            || txn.get(&f.paid_resale_heads, &head.key()).map_err(|e| bad_at(e, line!()))? != Some(&head)
        {
            return Err(PublicationError::Mismatch);
        }
        ctx.prepare_owner(
            self.roots.as_ref().ok_or(PublicationError::Unavailable)?,
            RootRequest::PaidResale(Box::new(roots)),
        )
        .map_err(|e| bad_at(e, line!()))?;
        Ok(row)
    }
}
fn envelope<R: Record>(
    family: &StateFamilyHandle<String, R>,
    key: &str,
    value: serde_json::Value,
) -> Result<(), PublicationError> {
    let d = family.descriptor();
    let doc = serde_json::json!({"family":{"owner":family.owner(),"id":d.id()},"key":key,
        "schema":{"id":d.value_schema(),"version":d.schema_version()},"value":value});
    mush_core::canonical_json_bytes_with_limit(&doc, d.max_record_bytes())
        .map(|_| ())
        .map_err(|_| PublicationError::Capacity)
}
