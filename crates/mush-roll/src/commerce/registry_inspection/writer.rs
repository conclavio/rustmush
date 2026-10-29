//! The writer the Roll installs and tacsim's provider lands a record through (the IC claims
//! writer's pattern, `super::super::ic_claims::writer`): the Roll delegates the record family
//! to tacsim at its registration, tacsim's registrar takes the writer for its one action, and
//! the record is inserted from tacsim's own transaction after the live Org authorization is
//! observed here.
use super::super::families::CommerceFamilies;
use super::{
    authority::{self, Readiness},
    *,
};
use mush_core::{ActionTxn, ClockStamp, PrepareCtx, Registrar, StateFamilyHandle, World};
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
struct Registration {
    families: CommerceFamilies,
    policy: RegistryInspectionPolicy,
    readiness: Readiness,
}

/// The shared cell: empty until the Roll's registration installs it.
#[derive(Clone, Default)]
pub struct InspectionWriter(Arc<OnceLock<Registration>>);
impl std::fmt::Debug for InspectionWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectionWriter")
            .field("registered", &self.0.get().is_some())
            .finish()
    }
}
impl InspectionWriter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// The Roll's half: delegate the record family to tacsim and keep the policy.
    pub(crate) fn install(
        &self,
        reg: &mut Registrar<'_>,
        families: CommerceFamilies,
        policy: RegistryInspectionPolicy,
    ) -> Result<(), ModError> {
        if reg.module() != crate::NS {
            return Err(error("inspection writer requires actual Roll registration"));
        }
        policy.validate().map_err(error)?;
        reg.delegate_state_family(&families.registry_inspections, ACTION_OWNER)?;
        // Doc 17h cut M8d D4: the revoked principals, written from tacsim's transaction.
        reg.delegate_state_family(&families.revoked_principals, ACTION_OWNER)?;
        // Doc 17h cut M8g G3: the quirk claims, written from tacsim's transaction.
        reg.delegate_state_family(&families.mind_claims, ACTION_OWNER)?;
        self.0
            .set(Registration {
                families,
                policy,
                readiness: Readiness::default(),
            })
            .map_err(|_| error("inspection writer already registered"))
    }
    /// tacsim's half: the writer bound to its registered action.
    ///
    /// # Errors
    /// A registrar that is not tacsim's.
    pub fn for_action(
        &self,
        reg: &mut Registrar<'_>,
        action: &DefinitionRef,
    ) -> Result<RegisteredInspectionWriter, ModError> {
        if reg.module() != ACTION_OWNER {
            return Err(error(
                "inspection writer requires actual Tacsim registration",
            ));
        }
        Ok(RegisteredInspectionWriter {
            writer: self.clone(),
            action: action.clone(),
        })
    }
}

impl InspectionWriter {
    /// Doc 17h cut M8d D5: the record family, for a reader that is not the inspection's own
    /// action (the hull cover's exclusion). Read only; the Roll installs the cell first.
    ///
    /// # Errors
    /// The Roll has not installed the writer.
    pub fn family(&self) -> Result<StateFamilyHandle<String, InspectionRecord>, ModError> {
        Ok(self
            .0
            .get()
            .ok_or_else(|| error("inspection writer not installed"))?
            .families
            .registry_inspections
            .clone())
    }
}

/// The writer bound to tacsim's registered action.
#[derive(Clone)]
pub struct RegisteredInspectionWriter {
    writer: InspectionWriter,
    action: DefinitionRef,
}
impl RegisteredInspectionWriter {
    fn registration(&self) -> Result<&Registration, PublicationError> {
        let r = self.writer.0.get().ok_or(PublicationError::Unavailable)?;
        if self.action != r.policy.action().map_err(bad)? {
            return Err(PublicationError::Mismatch);
        }
        Ok(r)
    }
    /// The action this writer lands records for.
    #[must_use]
    pub const fn action(&self) -> &DefinitionRef {
        &self.action
    }
    /// The authority definition the action references.
    ///
    /// # Errors
    /// The Roll has not installed the writer.
    pub fn authority(&self) -> Result<DefinitionRef, ModError> {
        self.registration().map_err(error)?.policy.authority()
    }
    /// The configured policy.
    ///
    /// # Errors
    /// The Roll has not installed the writer.
    pub fn policy(&self) -> Result<RegistryInspectionPolicy, ModError> {
        Ok(self.registration().map_err(error)?.policy.clone())
    }
    /// The record family, for a read of what an inspection wrote.
    ///
    /// # Errors
    /// The Roll has not installed the writer.
    pub fn family(&self) -> Result<StateFamilyHandle<String, InspectionRecord>, ModError> {
        Ok(self
            .registration()
            .map_err(error)?
            .families
            .registry_inspections
            .clone())
    }
    /// Capture the definition registry once the Roll, Org and Economy owners are ready:
    /// tacsim's boot hook, which runs after the Roll's.
    ///
    /// # Errors
    /// A hook that is not tacsim's, an owner not ready, or a second capture.
    pub fn capture(&self, ctx: &mush_core::ModuleCtx<'_>) -> Result<(), ModError> {
        if ctx.module.as_deref() != Some(ACTION_OWNER) {
            return Err(error("inspection readiness is captured by actual Tacsim"));
        }
        self.registration().map_err(error)?.readiness.capture(ctx)
    }
    /// Whether `actor` may inspect now: a configured operator the live Org authorizes on the
    /// scope, with every owner ready (the offer's and the command's one test).
    #[must_use]
    pub fn offered(&self, world: &World, actor: ObjId) -> bool {
        self.authorize(world, actor).is_ok()
    }
    /// Doc 17h cut M8d D3: whether `actor` may fine now — the inspection's test under the
    /// fine's own capability.
    #[must_use]
    pub fn fine_offered(&self, world: &World, actor: ObjId) -> bool {
        self.authorize_fine(world, actor).is_ok()
    }
    /// Doc 17h cut M8d D3: the live fine authorization, observed (`authority::observe_fine`).
    ///
    /// # Errors
    /// `Unavailable` when the actor is no operator, not granted the fine, or an owner is not
    /// ready.
    pub fn authorize_fine(
        &self,
        world: &World,
        actor: ObjId,
    ) -> Result<(Authorization, ClockStamp), PublicationError> {
        let r = self.registration()?;
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(bad)?;
        authority::observe_fine(world, &r.readiness, &r.policy, actor)
    }
    /// Doc 17h cut M8g G3: whether `actor` may read the quirk claims now — the inspection's
    /// test under `roll.registry.claims`.
    #[must_use]
    pub fn claims_offered(&self, world: &World, actor: ObjId) -> bool {
        self.authorize_claims(world, actor).is_ok()
    }
    /// Doc 17h cut M8g G3: the live claims-read authorization (`authority::observe_claims`).
    ///
    /// # Errors
    /// `Unavailable` when the actor is no operator, not granted, or an owner is not ready.
    pub fn authorize_claims(
        &self,
        world: &World,
        actor: ObjId,
    ) -> Result<(Authorization, ClockStamp), PublicationError> {
        let r = self.registration()?;
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(bad)?;
        authority::observe_claims(world, &r.readiness, &r.policy, actor)
    }
    /// Doc 17h cut M8g G3: the quirk claims' family, for the operators' read.
    ///
    /// # Errors
    /// The Roll has not installed the writer.
    pub fn claims_family(&self) -> Result<StateFamilyHandle<String, super::ClaimRecord>, ModError> {
        Ok(self
            .registration()
            .map_err(error)?
            .families
            .mind_claims
            .clone())
    }
    /// **Land one quirk claim** (doc 17h cut M8g G3): the claim action must be the policy's;
    /// the record is the author's next ring slot, overwriting the oldest once 32 are held.
    /// Who may claim and which hulls they earned are tacsim's to establish before this.
    ///
    /// # Errors
    /// A foreign action, an owner not ready, a record that does not validate, or the write.
    pub fn file_claim(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        action: &DefinitionRef,
        author: ObjId,
        hulls: Vec<ObjId>,
        text: smol_str::SmolStr,
    ) -> Result<super::ClaimRecord, PublicationError> {
        let r = self.registration()?;
        if action != &r.policy.claim_action().map_err(bad)? {
            return Err(PublicationError::Mismatch);
        }
        r.readiness.check(&r.policy)?;
        let world = ctx.world().map_err(bad)?;
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(bad)?;
        let at_ic = world.world_ic_now();
        let family = &r.families.mind_claims;
        let mut newest = 0_u64;
        for key in super::claims::claim_slots(author) {
            if let Some(held) = txn.get(family, &key).map_err(bad)? {
                if held.author != author {
                    return Err(PublicationError::Mismatch);
                }
                newest = newest.max(held.seq);
            }
        }
        let record = super::ClaimRecord {
            format: 1,
            claim: ctx.commit_token(),
            action: action.clone(),
            authority: r.policy.clone(),
            author,
            seq: newest.checked_add(1).ok_or(PublicationError::Invalid)?,
            hulls,
            text,
            at_ic,
        };
        record.validate()?;
        let key = record.key();
        if txn.get(family, &key).map_err(bad)?.is_some() {
            txn.replace(family, &key, record.clone()).map_err(bad)?;
        } else {
            txn.insert(family, &key, record.clone()).map_err(bad)?;
        }
        Ok(record)
    }
    /// Doc 17h cut M8d D4: whether `actor` may revoke a principal now — the inspection's test
    /// under the existing `roll.registry.revoke`.
    #[must_use]
    pub fn revoke_offered(&self, world: &World, actor: ObjId) -> bool {
        self.authorize_revoke(world, actor).is_ok()
    }
    /// Doc 17h cut M8d D4: the live revocation authorization (`authority::observe_revoke`).
    ///
    /// # Errors
    /// `Unavailable` when the actor is no operator, not granted, or an owner is not ready.
    pub fn authorize_revoke(
        &self,
        world: &World,
        actor: ObjId,
    ) -> Result<(Authorization, ClockStamp), PublicationError> {
        let r = self.registration()?;
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(bad)?;
        authority::observe_revoke(world, &r.readiness, &r.policy, actor)
    }
    /// Doc 17h cut M8d D4: the revoked principals' family.
    ///
    /// # Errors
    /// The Roll has not installed the writer.
    pub fn revoked_principals(
        &self,
    ) -> Result<StateFamilyHandle<String, super::principal::RevokedPrincipal>, ModError> {
        Ok(self
            .registration()
            .map_err(error)?
            .families
            .revoked_principals
            .clone())
    }
    /// **Land one principal revocation** (doc 17h cut M8d D4): observe the revocation grant
    /// again, then write `record` over any earlier one of the principal.
    ///
    /// # Errors
    /// The authorization, a record that does not validate, or the write.
    pub fn revoke_principal(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        record: super::principal::RevokedPrincipal,
    ) -> Result<super::principal::RevokedPrincipal, PublicationError> {
        let r = self.registration()?;
        let world = ctx.world().map_err(bad)?;
        self.authorize_revoke(world, record.operator)?;
        record.validate()?;
        let key = record.key();
        let family = &r.families.revoked_principals;
        if txn.get(family, &key).map_err(bad)?.is_some() {
            txn.replace(family, &key, record.clone()).map_err(bad)?;
        } else {
            txn.insert(family, &key, record.clone()).map_err(bad)?;
        }
        Ok(record)
    }
    /// The live authorization, observed.
    ///
    /// # Errors
    /// `Unavailable` when the actor is no operator, not authorized, or an owner is not ready.
    pub fn authorize(
        &self,
        world: &World,
        actor: ObjId,
    ) -> Result<(Authorization, ClockStamp), PublicationError> {
        let r = self.registration()?;
        world
            .module_state::<crate::RollStore>()
            .ok_or(PublicationError::Unavailable)?
            .commerce_ready()
            .map_err(bad)?;
        authority::observe(world, &r.readiness, &r.policy, actor)
    }
    /// **Land one inspection** (M8a's A8: the one write): observe the authorization again
    /// in the prepare, build the record over `facts`, validate it and insert it under the
    /// commit's key.
    ///
    /// # Errors
    /// The authorization, a record that does not validate, or the insert.
    pub fn record(
        &self,
        ctx: &mut PrepareCtx<'_>,
        txn: &mut ActionTxn<'_>,
        actor: ObjId,
        facts: InspectionFacts,
    ) -> Result<InspectionRecord, PublicationError> {
        let r = self.registration()?;
        let inspection = ctx.commit_token();
        let world = ctx.world().map_err(bad)?;
        let (authorization, at_ic) = self.authorize(world, actor)?;
        // The F9 ring: the newest `seq` among the hull's 64 slots, plus one; its slot holds
        // the oldest record once the ring is full, and is replaced in this transaction.
        let mut newest = 0_u64;
        for key in super::records::inspection_slots(facts.hull) {
            if let Some(held) = txn
                .get(&r.families.registry_inspections, &key)
                .map_err(bad)?
            {
                if held.hull != facts.hull {
                    return Err(PublicationError::Mismatch);
                }
                newest = newest.max(held.seq);
            }
        }
        let seq = newest.checked_add(1).ok_or(PublicationError::Invalid)?;
        let record = InspectionRecord {
            format: 1,
            inspection,
            action: self.action.clone(),
            authority: r.policy.clone(),
            operator: actor,
            authorization,
            hull: facts.hull,
            seq,
            core: facts.core,
            class: facts.class,
            governed: facts.governed,
            provenance: facts.provenance,
            seal_digest: facts.seal_digest,
            seal_check: facts.seal_check,
            licence_state: facts.licence_state,
            at_ic,
            ungoverned_note: facts.ungoverned_note,
            holder: facts.holder,
        };
        record.validate()?;
        let key = record.key();
        if txn
            .get(&r.families.registry_inspections, &key)
            .map_err(bad)?
            .is_some()
        {
            txn.replace(&r.families.registry_inspections, &key, record.clone())
                .map_err(bad)?;
        } else {
            txn.insert(&r.families.registry_inspections, &key, record.clone())
                .map_err(bad)?;
        }
        if txn
            .get(&r.families.registry_inspections, &record.key())
            .map_err(bad)?
            != Some(&record)
        {
            return Err(PublicationError::Mismatch);
        }
        Ok(record)
    }
}
