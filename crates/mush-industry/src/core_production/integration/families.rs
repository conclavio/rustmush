use super::{
    IntegrationError as Error, bad,
    offer::{FactoryOffer, OFFER_BYTES},
    progress,
    records::*,
};
use crate::workorder::WorkOrderId;
use mush_core::{
    CanonicalId, CommitToken, DurableSchedule, ModError, Registrar, ScheduleId,
    StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
#[derive(Clone)]
pub(crate) struct Families {
    pub(crate) offers: StateFamilyHandle<CommitToken, FactoryOffer>,
    pub(crate) orders: StateFamilyHandle<WorkOrderId, CoreIntegrationOrder>,
    pub(crate) deployment: StateFamilyHandle<(), IntegrationIndex>,
    pub(crate) schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    pub(crate) jobs: StateFamilyHandle<ScheduleId, WorkOrderId>,
}
pub(crate) fn descriptor(name: &str, bytes: usize) -> Result<StateFamilyDescriptor, ModError> {
    let cid = |part: &str| {
        CanonicalId::new(format!("industry.{name}.{part}.v1"))
            .map_err(|e| ModError::msg(e.to_string()))
    };
    StateFamilyDescriptor::new(
        StateFamilyId::new(name).map_err(|e| ModError::msg(e.to_string()))?,
        cid("key")?,
        cid("value")?,
        1,
        cid("decode")?,
        cid("validate")?,
        cid("canonical")?,
    )
    .and_then(|d| d.with_limits(64, bytes))
    .map_err(|e| ModError::msg(e.to_string()))
}
impl Families {
    pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        let offers = reg.state_family(
            descriptor("industry_core_offers", OFFER_BYTES + 1024)?,
            |k: &CommitToken| Ok(k.to_hex()),
            |v: &serde_json::Value| decode::<FactoryOffer>(v, OFFER_BYTES),
            |r: &FactoryOffer| r.validate().map_err(|e| e.to_string()),
            |r: &FactoryOffer| encode(r, OFFER_BYTES),
        )?;
        let orders = reg.state_family(
            descriptor("industry_core_orders", ORDER_BYTES + 1024)?,
            |k: &WorkOrderId| Ok(format!("{:016x}", k.0)),
            |v: &serde_json::Value| decode::<CoreIntegrationOrder>(v, ORDER_BYTES),
            |r: &CoreIntegrationOrder| r.validate().map_err(|e| e.to_string()),
            |r: &CoreIntegrationOrder| encode(r, ORDER_BYTES),
        )?;
        let deployment = reg.state_family(
            descriptor("industry_core_integration_deployment", INDEX_BYTES + 1024)?,
            |_: &()| Ok("registry".into()),
            |v: &serde_json::Value| decode::<IntegrationIndex>(v, INDEX_BYTES),
            |r: &IntegrationIndex| r.validate().map_err(|e| e.to_string()),
            |r: &IntegrationIndex| encode(r, INDEX_BYTES),
        )?;
        let schedules = reg.state_family(
            mush_core::durable_schedule_state_family_descriptor(
                StateFamilyId::new("industry_core_schedule")
                    .map_err(|e| ModError::msg(e.to_string()))?,
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
        )?;
        let jobs = reg.state_family(
            descriptor("industry_core_schedule_order", 2048)?,
            |k: &ScheduleId| Ok(k.canonical_hash().to_hex()),
            |v: &serde_json::Value| decode::<WorkOrderId>(v, 128),
            |_: &WorkOrderId| Ok(()),
            |r: &WorkOrderId| encode(r, 128),
        )?;
        Ok(Self {
            offers,
            orders,
            deployment,
            schedules,
            jobs,
        })
    }
    pub(crate) fn publish_schedule(
        &self,
        txn: &mut mush_core::ActionTxn<'_>,
        order: &CoreIntegrationOrder,
    ) -> Result<(), Error> {
        let id = progress::schedule_id(order.intent.order)?;
        txn.insert(&self.schedules, &id, order.progress.cursor().clone())
            .map_err(bad)?;
        txn.insert(&self.jobs, &id, order.intent.order).map_err(bad)
    }
    pub(crate) fn remove_schedule(
        &self,
        txn: &mut mush_core::ActionTxn<'_>,
        order: WorkOrderId,
    ) -> Result<(), Error> {
        let id = progress::schedule_id(order)?;
        if txn.get(&self.schedules, &id).map_err(bad)?.is_some() {
            txn.remove(&self.schedules, &id).map_err(bad)?;
        }
        if txn.get(&self.jobs, &id).map_err(bad)?.is_some() {
            txn.remove(&self.jobs, &id).map_err(bad)?;
        }
        Ok(())
    }
}
