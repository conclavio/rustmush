//! Closed physical Install/Replace prerequisite. Trusted Industry composition
//! must also provide real title/payment, Tactics, certificate and refit history.
//! This module registers no player action provider or default business policy.
mod carry;
pub(crate) mod image;
pub use carry::ExchangeCarryAttempt;
mod policy;
mod records;
use crate::action::{ActionInvocation, PerceivedRef, PrincipalId};
use crate::action_state::{ActionTxn, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId};
use crate::authoritative::ReplayWrite;
use crate::mechanics::{
    CanonicalId, DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRef, OwnerId,
    canonical_json_bytes_with_limit,
};
use crate::participant::ParticipantError;
use crate::read_service::OwnerReadHandle;
use crate::{
    CommitToken, DefinitionDigest, Flags, ModError, ModuleCtx, Obj, ObjId, ObjKind, Registrar,
    World,
};
pub use policy::{CoreExchangePolicy, ExchangeFacts, ExchangeRequest};
use records::ExchangeRecord;
pub use records::{ExchangeObjectDigest, ExchangedCoreComponent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

pub const ACTION: &str = "install_core_component";
const FAMILY: &str = "core_component_exchanges";
const RECORD_BYTES: usize = 16_384;
const IMAGE_BYTES: usize = 1_048_576;
const MAX_EXCHANGE_OBJECTS: usize = 5;

/// The exact Core protocol action has no recursive participant references.
/// The separate owner-read definition binds the actual two participant refs.
pub fn action_descriptor() -> Result<DefinitionDescriptor, ModError> {
    DefinitionDescriptor::new(
        DefinitionKind::new("action").map_err(bad)?,
        DefinitionId::new(ACTION).map_err(bad)?,
        1,
        json!({"format":1,"protocol":"certified_core_install_v1",
            "effect":"bounded_component_exchange","objects":MAX_EXCHANGE_OBJECTS,
            "image_bytes":IMAGE_BYTES,"receipt_bytes":RECORD_BYTES,
            "owners":["economy","tactics"]}),
    )
    .map_err(bad)
}
fn exact_action(action: &DefinitionRef) -> Result<(), ParticipantError> {
    let actual = crate::DefinitionRegistryBuilder::new()
        .register(
            OwnerId::new("industry").map_err(|_| refused())?,
            action_descriptor().map_err(|_| refused())?,
        )
        .map_err(|_| refused())?;
    if action != &actual {
        return Err(refused());
    }
    Ok(())
}
fn check_participants(participants: &[DefinitionRef; 2]) -> Result<(), ParticipantError> {
    for (r, owner, id) in [
        (&participants[0], "economy", "initial_filing"),
        (&participants[1], "tactics", "component_replacement"),
    ] {
        if r.key().owner.as_str() != owner
            || r.key().kind.as_str() != "owner_participant"
            || r.key().id.as_str() != id
            || r.version() != 1
        {
            return Err(refused());
        }
    }
    Ok(())
}
pub struct CoreComponentExchanges<P: CoreExchangePolicy> {
    policy: Arc<P>,
    reader: OwnerReadHandle<policy::ExchangeReader<P>>,
    records: StateFamilyHandle<u64, ExchangeRecord>,
    action: DefinitionRef,
    participants: [DefinitionRef; 2],
}
impl<P: CoreExchangePolicy> Clone for CoreComponentExchanges<P> {
    fn clone(&self) -> Self {
        Self {
            policy: Arc::clone(&self.policy),
            reader: self.reader.clone(),
            records: self.records.clone(),
            action: self.action.clone(),
            participants: self.participants.clone(),
        }
    }
}
impl<P: CoreExchangePolicy> CoreComponentExchanges<P> {
    pub fn policy_descriptor(
        action: &DefinitionRef,
        participants: &[DefinitionRef; 2],
    ) -> Result<DefinitionDescriptor, ModError> {
        exact_action(action).map_err(bad)?;
        check_participants(participants).map_err(bad)?;
        DefinitionDescriptor::new(
            DefinitionKind::new("owner_read").map_err(bad)?, DefinitionId::new("core_component_exchange_policy").map_err(bad)?,1,
            json!({"format":1,"protocol":"core_component_exchange_v1","action":action,"participants":participants,
                "objects":MAX_EXCHANGE_OBJECTS,"image_bytes":IMAGE_BYTES,"receipt_bytes":RECORD_BYTES}))
            .map(|d|d.with_references(vec![action.clone(),participants[0].clone(),participants[1].clone()])).map_err(bad)
    }
    pub fn register(
        reg: &mut Registrar<'_>,
        action: DefinitionRef,
        participants: [DefinitionRef; 2],
        policy: P,
        mut dependencies: BTreeSet<OwnerId>,
    ) -> Result<Self, ModError> {
        exact_action(&action).map_err(bad)?;
        check_participants(&participants).map_err(bad)?;
        if reg.module() != "industry" {
            return Err(bad("Core exchange owner differs"));
        }
        let definition = reg.definition(Self::policy_descriptor(&action, &participants)?)?;
        dependencies.extend([
            OwnerId::new("industry").map_err(bad)?,
            OwnerId::new("economy").map_err(bad)?,
            OwnerId::new("tactics").map_err(bad)?,
        ]);
        let policy = Arc::new(policy);
        reg.owner_read_service(
            definition.clone(),
            BTreeSet::from([OwnerId::new("industry").map_err(bad)?]),
            dependencies,
            policy::ExchangeReader {
                policy: Arc::clone(&policy),
            },
        )?;
        let reader = reg.request_owner_read::<policy::ExchangeReader<P>>(definition)?;
        let id = |suffix| CanonicalId::new(format!("industry.{FAMILY}.{suffix}.v1")).map_err(bad);
        let descriptor = StateFamilyDescriptor::new(
            StateFamilyId::new(FAMILY).map_err(bad)?,
            id("key")?,
            id("value")?,
            1,
            id("decode")?,
            id("validate")?,
            id("canonical")?,
        )
        .and_then(|d| d.with_limits(16, RECORD_BYTES))
        .map_err(bad)?;
        let records = reg.state_family(
            descriptor,
            |key: &u64| Ok(format!("{key:016x}")),
            |value: &Value| decode(value),
            |row: &ExchangeRecord| row.validate().map_err(|e| e.to_string()),
            |row: &ExchangeRecord| bounded(row).map_err(|e| e.to_string()),
        )?;
        Ok(Self {
            policy,
            reader,
            records,
            action,
            participants,
        })
    }
    pub(crate) fn reader(&self) -> &OwnerReadHandle<policy::ExchangeReader<P>> {
        &self.reader
    }
    pub(crate) fn participants(&self) -> &[DefinitionRef; 2] {
        &self.participants
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        &self,
        world: &World,
        invocation: &ActionInvocation,
        action: &DefinitionRef,
        commit: CommitToken,
        txn: &mut ActionTxn<'_>,
        order: u64,
        facts: ExchangeFacts,
        hooks: Arc<[crate::module::HookReg]>,
        base: &crate::action_state::ActionReadBase,
        provider: &crate::action::ProviderRegistrationOrdinal,
    ) -> Result<(ExchangedCoreComponent, image::ExchangeObjects), ParticipantError> {
        if action != &self.action
            || facts.order() != order
            || facts.requester() != invocation.actor()
            || invocation.principal()
                != &(PrincipalId::Object {
                    id: invocation.actor(),
                })
            || invocation.subject() != &PerceivedRef::SelfActor
            || !invocation.targets().is_empty()
            || txn
                .get(&self.records, &order)
                .map_err(|_| refused())?
                .is_some()
        {
            return Err(refused());
        }
        let request = ExchangeRequest { order };
        let fence = Box::new(policy::PolicyFence {
            policy: Arc::clone(&self.policy),
            actor: invocation.actor(),
            action: action.clone(),
            commit,
            request,
            facts: facts.clone(),
        });
        let image = {
            let lens = txn.read_preimage(base, provider).map_err(|_| refused())?;
            image::ExchangeObjects::prepare(
                world,
                lens,
                invocation.actor(),
                &facts,
                hooks,
                fence,
                self.participants.clone(),
                action.clone(),
                commit,
            )?
        };
        let row = ExchangeRecord::new(
            &facts,
            action.clone(),
            self.participants.clone(),
            commit,
            world.now(),
            image.digests()?,
        );
        row.validate()?;
        canonical_json_bytes_with_limit(&json!({"family":{"owner":"industry","id":FAMILY},
            "schema":{"id":format!("industry.{FAMILY}.value.v1"),"version":1},"key":format!("{order:016x}"),"value":row}),RECORD_BYTES)
            .map_err(|_|ParticipantError::ImageLimit)?;
        txn.insert(&self.records, &order, row.clone())
            .map_err(|_| refused())?;
        Ok((ExchangedCoreComponent { row }, image))
    }
    /// Exact bounded history scan for the actual Industry reverse audit. This
    /// alone is not a current installed-core index or a title certificate.
    pub fn audit(
        &self,
        ctx: &ModuleCtx<'_>,
        mut observe: impl FnMut(ExchangedCoreComponent) -> Result<(), ParticipantError>,
    ) -> Result<(), ParticipantError> {
        let mut cursor = None;
        let mut count = 0usize;
        loop {
            let page = ctx
                .scan_action_state(&self.records, cursor.as_deref(), 128)
                .map_err(|_| refused())?;
            for entry in page.entries() {
                let row = entry.value();
                row.validate()?;
                count = count.checked_add(1).ok_or_else(refused)?;
                if count > crate::action_state::MAX_ACTION_STATE_BOOT_RECORDS
                    || entry.encoded_key() != format!("{:016x}", row.order)
                {
                    return Err(refused());
                }
                observe(ExchangedCoreComponent { row: row.clone() })?;
            }
            cursor = page.next_after().map(ToString::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(())
    }
}
fn refused() -> ParticipantError {
    ParticipantError::Authority
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
fn bounded<T: Serialize>(row: &T) -> Result<Value, ParticipantError> {
    let value = serde_json::to_value(row).map_err(|_| ParticipantError::Encoding)?;
    canonical_json_bytes_with_limit(&value, RECORD_BYTES)
        .map_err(|_| ParticipantError::ImageLimit)?;
    Ok(value)
}
fn decode(value: &Value) -> Result<ExchangeRecord, String> {
    canonical_json_bytes_with_limit(value, RECORD_BYTES).map_err(|e| e.to_string())?;
    let row: ExchangeRecord = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    row.validate().map_err(|e| e.to_string())?;
    if serde_json::to_value(&row).map_err(|e| e.to_string())? != *value {
        return Err("noncanonical Core exchange".into());
    }
    Ok(row)
}

#[cfg(test)]
mod tests;
