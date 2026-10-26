use super::{Error, bad, progress, records::*};
use crate::core_production::integration::families::descriptor;
use crate::workorder::WorkOrderId;
use mush_core::{
    ActionTxn, CommitToken, DurableSchedule, ModError, ObjId, Registrar, ScheduleId,
    StateFamilyHandle,
};
#[derive(Clone)]
pub(crate) struct Families {
    pub(crate) offers: StateFamilyHandle<CommitToken, RefitOffer>,
    pub(crate) orders: StateFamilyHandle<WorkOrderId, RefitOrder>,
    pub(crate) deployment: StateFamilyHandle<(), RefitIndex>,
    pub(crate) assets: StateFamilyHandle<ObjId, AssetHead>,
    pub(crate) hulls: StateFamilyHandle<ObjId, HullHead>,
    pub(crate) schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    pub(crate) jobs: StateFamilyHandle<ScheduleId, WorkOrderId>,
}
impl Families {
    pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        let offers = reg.state_family(
            descriptor("industry_refit_offers", OFFER_BYTES + 1024)?,
            |k: &CommitToken| Ok(k.to_hex()),
            |v: &serde_json::Value| decode::<RefitOffer>(v, OFFER_BYTES),
            |r: &RefitOffer| r.validate().map_err(|e| e.to_string()),
            |r: &RefitOffer| encode(r, OFFER_BYTES),
        )?;
        let orders = reg.state_family(
            descriptor("industry_refit_orders", ORDER_BYTES + 1024)?,
            |k: &WorkOrderId| Ok(format!("{:016x}", k.0)),
            |v: &serde_json::Value| decode::<RefitOrder>(v, ORDER_BYTES),
            |r: &RefitOrder| r.validate().map_err(|e| e.to_string()),
            |r: &RefitOrder| encode(r, ORDER_BYTES),
        )?;
        let deployment = reg.state_family(
            descriptor("industry_refit_deployment", INDEX_BYTES + 1024)?,
            |_: &()| Ok("registry".into()),
            |v: &serde_json::Value| decode::<RefitIndex>(v, INDEX_BYTES),
            |r: &RefitIndex| r.validate().map_err(|e| e.to_string()),
            |r: &RefitIndex| encode(r, INDEX_BYTES),
        )?;
        let assets = reg.state_family(
            descriptor("industry_refit_assets", 4096)?,
            |k: &ObjId| Ok(k.0.to_string()),
            |v: &serde_json::Value| decode::<AssetHead>(v, 3072),
            |r: &AssetHead| r.validate().map_err(|e| e.to_string()),
            |r: &AssetHead| encode(r, 3072),
        )?;
        let hulls = reg.state_family(
            descriptor("industry_refit_hulls", 4096)?,
            |k: &ObjId| Ok(k.0.to_string()),
            |v: &serde_json::Value| decode::<HullHead>(v, 3072),
            |r: &HullHead| r.validate().map_err(|e| e.to_string()),
            |r: &HullHead| encode(r, 3072),
        )?;
        let schedules = reg.state_family(
            mush_core::durable_schedule_state_family_descriptor(
                mush_core::StateFamilyId::new("industry_refit_schedule")
                    .map_err(|e| ModError::msg(e.to_string()))?,
            )
            .map_err(|e| ModError::msg(e.to_string()))?,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
        )?;
        let jobs = reg.state_family(
            descriptor("industry_refit_schedule_order", 2048)?,
            |k: &ScheduleId| Ok(k.canonical_hash().to_hex()),
            |v: &serde_json::Value| decode::<WorkOrderId>(v, 128),
            |_: &WorkOrderId| Ok(()),
            |r: &WorkOrderId| encode(r, 128),
        )?;
        Ok(Self {
            offers,
            orders,
            deployment,
            assets,
            hulls,
            schedules,
            jobs,
        })
    }
    pub(crate) fn accept_heads(
        &self,
        txn: &mut ActionTxn<'_>,
        row: &RefitOrder,
    ) -> Result<(), Error> {
        let o = &row.intent.offer;
        let id = row.intent.order;
        if let Some(a) = txn.get(&self.assets, &o.incoming).map_err(bad)? {
            a.validate()?;
            if !matches!(a.position, AssetPosition::Released { .. }) {
                return Err(Error::Unavailable);
            }
        }
        let h = txn.get(&self.hulls, &o.hull).map_err(bad)?.cloned();
        match (&h, o.displaced.installed.asset) {
            (None, None) => {}
            (Some(h), a) if h.pending.is_none() && h.mounted == a => {
                h.validate()?;
            }
            _ => return Err(Error::Unavailable),
        }
        if let Some(old) = o.displaced.installed.asset {
            let a = txn
                .get(&self.assets, &old)
                .map_err(bad)?
                .ok_or(Error::Integrity)?;
            if !matches!(a.position,AssetPosition::Mounted{hull,..}if hull==o.hull) {
                return Err(Error::Integrity);
            }
        }
        let a = AssetHead {
            format: 1,
            object: o.incoming,
            operation: row.accepted,
            position: AssetPosition::Pending { order: id },
        };
        if txn.get(&self.assets, &o.incoming).map_err(bad)?.is_some() {
            txn.replace(&self.assets, &o.incoming, a).map_err(bad)?;
        } else {
            txn.insert(&self.assets, &o.incoming, a).map_err(bad)?;
        }
        let next = HullHead {
            format: 1,
            hull: o.hull,
            operation: row.accepted,
            pending: Some(id),
            mounted: o.displaced.installed.asset,
        };
        if h.is_some() {
            txn.replace(&self.hulls, &o.hull, next).map_err(bad)?;
        } else {
            txn.insert(&self.hulls, &o.hull, next).map_err(bad)?;
        }
        Ok(())
    }
    pub(crate) fn terminal_heads(
        &self,
        txn: &mut ActionTxn<'_>,
        row: &RefitOrder,
    ) -> Result<(), Error> {
        row.validate()?;
        let o = &row.intent.offer;
        let t = row.terminal.as_ref().ok_or(Error::Integrity)?;
        let id = row.intent.order;
        let a = txn
            .get(&self.assets, &o.incoming)
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Integrity)?;
        let h = txn
            .get(&self.hulls, &o.hull)
            .map_err(bad)?
            .cloned()
            .ok_or(Error::Integrity)?;
        if a.object != o.incoming
            || a.position != (AssetPosition::Pending { order: id })
            || h.pending != Some(id)
            || h.mounted != o.displaced.installed.asset
        {
            return Err(Error::Integrity);
        }
        let installed = row.installation.is_some();
        let position = if installed {
            AssetPosition::Mounted {
                order: id,
                hull: o.hull,
            }
        } else {
            AssetPosition::Released { order: id }
        };
        txn.replace(
            &self.assets,
            &o.incoming,
            AssetHead {
                operation: t.commit(),
                position,
                ..a
            },
        )
        .map_err(bad)?;
        txn.replace(
            &self.hulls,
            &o.hull,
            HullHead {
                operation: t.commit(),
                pending: None,
                mounted: if installed {
                    Some(o.incoming)
                } else {
                    h.mounted
                },
                ..h
            },
        )
        .map_err(bad)?;
        if installed && let Some(old) = o.displaced.installed.asset {
            let a = txn
                .get(&self.assets, &old)
                .map_err(bad)?
                .cloned()
                .ok_or(Error::Integrity)?;
            if !matches!(a.position,AssetPosition::Mounted{hull,..}if hull==o.hull) {
                return Err(Error::Integrity);
            }
            txn.replace(
                &self.assets,
                &old,
                AssetHead {
                    operation: t.commit(),
                    position: AssetPosition::Detached {
                        order: id,
                        from_hull: o.hull,
                        condition: digest(
                            "industry.core_refit.displaced.v1",
                            &o.displaced,
                            ASSET_BYTES,
                        )?,
                    },
                    ..a
                },
            )
            .map_err(bad)?;
        }
        self.remove_schedule(txn, id)
    }
    pub(crate) fn publish_schedule(
        &self,
        txn: &mut ActionTxn<'_>,
        row: &RefitOrder,
    ) -> Result<(), Error> {
        let id = progress::schedule_id(row.intent.order)?;
        txn.insert(&self.schedules, &id, row.progress.cursor().clone())
            .map_err(bad)?;
        txn.insert(&self.jobs, &id, row.intent.order).map_err(bad)
    }
    pub(crate) fn remove_schedule(
        &self,
        txn: &mut ActionTxn<'_>,
        id: WorkOrderId,
    ) -> Result<(), Error> {
        let id = progress::schedule_id(id)?;
        if txn.get(&self.schedules, &id).map_err(bad)?.is_some() {
            txn.remove(&self.schedules, &id).map_err(bad)?;
        }
        if txn.get(&self.jobs, &id).map_err(bad)?.is_some() {
            txn.remove(&self.jobs, &id).map_err(bad)?;
        }
        Ok(())
    }
}
