use super::records::{Census, Head};
use super::*;
use crate::{EndpointRef, InventoryStore, Lot, LotCensus, LotId};
use mush_core::mechanics::{DefinitionDescriptor, DefinitionId, DefinitionKind, OwnerId};
use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{
    ClockStamp, ModError, ObjId, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use std::{collections::BTreeSet, sync::Arc};

pub(super) const OPERATIONS: &str = "inventory_core_inputs";
pub(super) const HEADS: &str = "inventory_core_input_heads";
pub(super) const CENSUS: &str = "inventory_core_input_census";
pub(super) fn descriptor(name: &str) -> Result<StateFamilyDescriptor, ModError> {
    let id = |suffix: &str| {
        CanonicalId::new(format!("inventory.{name}.{suffix}.v1"))
            .map_err(|e| ModError::msg(e.to_string()))
    };
    StateFamilyDescriptor::new(
        StateFamilyId::new(name).map_err(|e| ModError::msg(e.to_string()))?,
        id("key")?,
        id("value")?,
        1,
        id("decode")?,
        id("validate")?,
        id("canonical")?,
    )
    .and_then(|d| d.with_limits(20, MAX_ENVELOPE_BYTES))
    .map_err(|e| ModError::msg(e.to_string()))
}
pub(super) fn key(value: u64) -> String {
    format!("{value:016x}")
}
#[derive(Debug)]
pub(super) struct Shared {
    pub(super) binding: CoreInputBinding,
    pub(super) operations: StateFamilyHandle<u64, CoreInputOperation>,
    pub(super) heads: StateFamilyHandle<LotId, Head>,
    pub(super) census: StateFamilyHandle<(), Census>,
    pub(super) lots: StateFamilyHandle<LotId, Lot>,
    pub(super) lot_census: StateFamilyHandle<EndpointRef, LotCensus>,
}
#[derive(Clone, Debug)]
pub struct CoreInputHandle {
    pub(super) shared: Arc<Shared>,
    service: DefinitionRef,
}
#[derive(Clone)]
pub struct RegisteredCoreInputs {
    pub(super) handle: CoreInputHandle,
    pub(super) reader: OwnerReadHandle<Reader>,
}
impl CoreInputHandle {
    pub fn request(&self, reg: &mut Registrar<'_>) -> Result<RegisteredCoreInputs, ModError> {
        if reg.module() != "industry" {
            return Err(ModError::msg(
                "exact core inputs require the registered Industry owner",
            ));
        }
        Ok(RegisteredCoreInputs {
            handle: self.clone(),
            reader: reg.request_owner_read::<Reader>(self.service.clone())?,
        })
    }
    pub fn operation(
        &self,
        txn: &mut ActionTxn<'_>,
        order: u64,
    ) -> Result<Option<CoreInputOperation>, Error> {
        let value = txn
            .get(&self.shared.operations, &order)
            .map_err(bad)?
            .cloned();
        if let Some(v) = &value {
            v.validate()?;
            if v.identity.order() != order {
                return Err(Error::Integrity);
            }
        }
        Ok(value)
    }
    pub(super) fn current(
        &self,
        txn: &mut ActionTxn<'_>,
        lot: LotId,
    ) -> Result<Option<CoreInputOperation>, Error> {
        let Some(head) = txn.get(&self.shared.heads, &lot).map_err(bad)?.cloned() else {
            return Ok(None);
        };
        head.validate()?;
        let value = self
            .operation(txn, head.identity.order())?
            .ok_or(Error::Integrity)?;
        if head != Head::from_operation(&value) || head.lot != lot {
            return Err(Error::Integrity);
        }
        Ok(Some(value))
    }
    pub(super) fn require_new(&self, txn: &mut ActionTxn<'_>, lot: LotId) -> Result<(), Error> {
        if txn.get(&self.shared.heads, &lot).map_err(bad)?.is_some() {
            Err(Error::Reserved)
        } else {
            Ok(())
        }
    }
    pub(crate) fn require_unheld(&self, txn: &mut ActionTxn<'_>, lot: LotId) -> Result<(), Error> {
        match self.current(txn, lot)?.map(|o| o.state()).transpose()? {
            Some(CoreInputState::Held | CoreInputState::Consumed) => Err(Error::Reserved),
            _ => Ok(()),
        }
    }
}

#[derive(Serialize)]
pub(super) struct ReadRequest {
    pub(super) endpoint: Option<EndpointRef>,
}
#[derive(Serialize)]
pub(super) struct Observation {
    pub(super) actor: ObjId,
    pub(super) stamp: ClockStamp,
    pub(super) utc: i64,
}
pub(super) struct Reader;
impl OwnerReadService for Reader {
    type State = InventoryStore;
    type Request = ReadRequest;
    type Output = Observation;
    fn read(
        &self,
        state: &InventoryStore,
        ctx: &OwnerReadContext<'_>,
        request: ReadRequest,
    ) -> Result<Observation, OwnerReadError> {
        if !state.is_complete()
            || ctx.requester().as_str() != "industry"
            || ctx.principal() != &(mush_core::PrincipalId::Object { id: ctx.actor() })
        {
            return Err(OwnerReadError::Unavailable);
        }
        if let Some(endpoint) = request.endpoint
            && (state.is_quarantined(endpoint)
                || !mush_economy::endpoint_state(ctx.world(), endpoint.0).is_live())
        {
            return Err(OwnerReadError::Unavailable);
        }
        Ok(Observation {
            actor: ctx.actor(),
            stamp: ctx.world().world_ic_now(),
            utc: ctx.world().now(),
        })
    }
}
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    binding: CoreInputBinding,
    lots: StateFamilyHandle<LotId, Lot>,
    lot_census: StateFamilyHandle<EndpointRef, LotCensus>,
) -> Result<CoreInputHandle, ModError> {
    binding
        .validate()
        .map_err(|e| ModError::msg(e.to_string()))?;
    let operations = reg.state_family(
        descriptor(OPERATIONS)?,
        |k: &u64| Ok(key(*k)),
        decode::<CoreInputOperation>,
        |r: &CoreInputOperation| r.validate().map_err(|e| e.to_string()),
        encode::<CoreInputOperation>,
    )?;
    let heads = reg.state_family(
        descriptor(HEADS)?,
        |k: &LotId| Ok(key(k.0)),
        decode::<Head>,
        |r: &Head| r.validate().map_err(|e| e.to_string()),
        encode::<Head>,
    )?;
    let census = reg.state_family(
        descriptor(CENSUS)?,
        |_: &()| Ok("registry".into()),
        decode::<Census>,
        |r: &Census| r.validate().map_err(|e| e.to_string()),
        encode::<Census>,
    )?;
    let descriptor = DefinitionDescriptor::new(
        DefinitionKind::new("owner_read").map_err(|e| ModError::msg(e.to_string()))?,
        DefinitionId::new("core_input_preimage").map_err(|e| ModError::msg(e.to_string()))?, 1,
        serde_json::json!({"format":1,"binding":binding,"protocol":"exact_core_input_v1","max_operations":MAX_OPERATIONS,"operation_bytes":MAX_OPERATION_BYTES,"envelope_bytes":MAX_ENVELOPE_BYTES}),
    ).map_err(|e| ModError::msg(e.to_string()))?.with_references({
        // The mechanics registry canonically sorts full references at registration.
        // Compare the same exact set in that canonical order.
        let mut references: Vec<_> = binding.actions().into_iter().cloned().collect();
        references.sort();
        references
    });
    let expected = descriptor.clone();
    reg.definition_validator(
        DefinitionKind::new("owner_read").map_err(|e| ModError::msg(e.to_string()))?,
        move |ctx: &mush_core::mechanics::DefinitionValidationContext<'_>| {
            let d = ctx.definition();
            if d.reference().key().id == *expected.id()
                && d.reference().version() == expected.version()
                && d.payload() == expected.payload()
                && d.references() == expected.references()
                && d.is_current() == expected.is_current()
            {
                Ok(())
            } else {
                Err("invalid exact core input reader".into())
            }
        },
    )?;
    let service = reg.definition(descriptor)?;
    let industry = OwnerId::new("industry").map_err(|e| ModError::msg(e.to_string()))?;
    let deps = BTreeSet::from([
        industry.clone(),
        crate::record::owner(),
        OwnerId::new("economy").map_err(|e| ModError::msg(e.to_string()))?,
    ]);
    reg.owner_read_service(service.clone(), BTreeSet::from([industry]), deps, Reader)?;
    Ok(CoreInputHandle {
        shared: Arc::new(Shared {
            binding,
            operations,
            heads,
            census,
            lots,
            lot_census,
        }),
        service,
    })
}

pub(super) fn envelope(name: &str, key: &str, value: Value) -> Result<usize, Error> {
    let d = descriptor(name).map_err(bad)?;
    // Exact complete state envelope, including its longest terminal body.
    let v = serde_json::json!({"family":{"owner":"inventory","id":name},"key":key,
        "schema":{"id":d.value_schema(),"version":d.schema_version()},"value":value});
    Ok(bytes(&v, MAX_ENVELOPE_BYTES)?.len())
}
