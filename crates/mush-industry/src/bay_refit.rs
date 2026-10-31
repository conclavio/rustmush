//! **The bay refit** (doc 31 cut UD.4b): a component refit of an IC hull in a
//! facility bay, the sibling of the core refit (`core_production::refit`) on
//! the same industry work-order pipeline.
//!
//! A requester takes a supplier's acknowledged offer for one of the hull's
//! refits (the mount-matched transition of `tacsim::refit`): what comes out,
//! what goes in, at which bay. **Accept** holds the fee, a slot in the bay,
//! the hull (it stays in the bay until the order ends) and the part's lot in
//! the bay's stores. **Advance** runs the work on the IC clock, one unit per
//! [`CADENCE`]. **Install** captures the fee, refits the hull through the
//! Tactics `component_refit` participant, consumes the part's lot and returns
//! the part taken out to the same stores as a new lot. **Cancel** releases the
//! fee, the slot, the hull and the lot.
//!
//! The kernel's closed `bay_refit` terminal-capacity protocol
//! ([`mush_core::bay_refit_protocol`]) pins the four paid actions, their
//! participants and every family's descriptor, and audits each action's
//! exact write set ([`WRITE_SETS`]); this module declares the Industry side
//! of the same flow. The typed order, the provider and the schedule source
//! land with the Economy and Inventory owners that fill them.
use crate::workorder::{PRODUCTION_CADENCE_SECS, WorkOrderId};
use mush_core::{
    CommitToken, DefinitionDescriptor, DefinitionDigest, DefinitionRef, DurableSchedule, ModError,
    ObjId, Registrar, ScheduleId, StateFamilyDescriptor, StateFamilyHandle,
    bay_refit_protocol as kernel,
};
use serde::{Deserialize, Serialize};

/// Industry's own actions, which the kernel does not pin: the bay's
/// admission census is born, and a supplier makes an offer.
pub const INITIALIZE: &str = "initialize_bay_refit";
pub const OFFER: &str = "offer_bay_refit";
/// The kernel-pinned paid actions, in the kernel's order.
pub const ACCEPT: &str = kernel::ACTIONS[0];
pub const ADVANCE: &str = kernel::ACTIONS[1];
pub const INSTALL: &str = kernel::ACTIONS[2];
pub const CANCEL: &str = kernel::ACTIONS[3];
pub const ACTIONS: [&str; 6] = [INITIALIZE, OFFER, ACCEPT, ADVANCE, INSTALL, CANCEL];
/// One unit of bay work an IC hour, as the core refit and a work order run.
pub const CADENCE: i64 = PRODUCTION_CADENCE_SECS;
/// The most work units one refit takes: a week of IC hours, less one.
pub const MAX_WORK_UNITS: u64 = 167;
/// How long a supplier's offer stands, in UTC seconds.
pub const OFFER_SECONDS: i64 = 300;
/// The schedule source the bay's work runs under.
pub const CLOCK_POLICY: &str = "bay.refit";

/// The families the flow writes that the kernel does not reserve room in:
/// the supplier offers (Industry's alone, before any payment) and the shared
/// capacity index the slot's reservation is filed under.
pub const OFFERS: &str = "industry_bay_offers";
pub const CAPACITY_INDEX: &str = "industry_capacity_index";

/// **The bay policy**: the recipe the bay works to and the bays that work
/// it. Its exact value is in the paid actions' descriptors, so a change of
/// bays or recipe is a new action definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayRefitPolicy {
    format: u16,
    recipe: DefinitionRef,
    facilities: Vec<ObjId>,
}

/// A policy or bay record that will not validate.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum BayRefitError {
    #[error("bay refit policy: {0}")]
    Policy(String),
    #[error("bay refit record identity")]
    Integrity,
}

impl BayRefitPolicy {
    /// The policy for `recipe` at `facilities` (sorted and deduplicated).
    ///
    /// # Errors
    ///
    /// The kernel's, for a recipe other than `bay_refit` version 1, or no
    /// bays, or more than 64.
    pub fn new(recipe: DefinitionRef, mut facilities: Vec<ObjId>) -> Result<Self, BayRefitError> {
        facilities.sort();
        facilities.dedup();
        let policy = Self {
            format: 1,
            recipe,
            facilities,
        };
        policy.action_descriptor(ACCEPT)?;
        Ok(policy)
    }
    /// The bays the policy admits.
    #[must_use]
    pub fn facilities(&self) -> &[ObjId] {
        &self.facilities
    }
    /// The recipe the bay works to.
    #[must_use]
    pub fn recipe(&self) -> &DefinitionRef {
        &self.recipe
    }
    /// The descriptor of `action`: the kernel's for a paid action, so the
    /// two can never differ, and Industry's own for Initialize and Offer.
    ///
    /// # Errors
    ///
    /// An unknown action, or a policy the kernel refuses.
    pub fn action_descriptor(&self, action: &str) -> Result<DefinitionDescriptor, BayRefitError> {
        let policy =
            serde_json::to_value(self).map_err(|e| BayRefitError::Policy(e.to_string()))?;
        if let Some(index) = kernel::ACTIONS.iter().position(|a| *a == action) {
            return kernel::action_descriptor(&policy, index)
                .map_err(|e| BayRefitError::Policy(e.to_string()));
        }
        kernel::owner_action_descriptor(&policy, action)
            .map_err(|e| BayRefitError::Policy(e.to_string()))
    }
}

/// **A hull's bay head**: which bay order holds it, if any. A hull held in a
/// bay cannot be taken into another, and (doc 31 UD.4b) cannot launch.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BayHullHead {
    pub(crate) format: u16,
    pub(crate) hull: ObjId,
    /// The commit that last moved the head: the order's Accept, or its
    /// Install or Cancel.
    pub(crate) operation: CommitToken,
    #[serde(deserialize_with = "required_option")]
    pub(crate) pending: Option<WorkOrderId>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
impl BayHullHead {
    /// The hull held for `order` from its Accept `operation`.
    #[must_use]
    pub fn held(hull: ObjId, operation: CommitToken, order: WorkOrderId) -> Self {
        Self {
            format: 1,
            hull,
            operation,
            pending: Some(order),
        }
    }
    /// The hull released by its order's terminal `operation`.
    #[must_use]
    pub fn released(hull: ObjId, operation: CommitToken) -> Self {
        Self {
            format: 1,
            hull,
            operation,
            pending: None,
        }
    }
    /// The order holding the hull in a bay, if one is.
    #[must_use]
    pub fn pending(&self) -> Option<WorkOrderId> {
        self.pending
    }
    fn validate(&self) -> Result<(), BayRefitError> {
        if self.format != 1 {
            return Err(BayRefitError::Integrity);
        }
        Ok(())
    }
}

/// The schedule id of `order`'s bay work.
///
/// # Errors
///
/// The id's, which cannot fail for a valid order.
pub fn schedule_id(order: WorkOrderId) -> Result<ScheduleId, ModError> {
    let err = |e: &dyn std::fmt::Display| ModError::msg(e.to_string());
    ScheduleId::derive(
        &mush_core::OwnerId::new("industry").map_err(|e| err(&e))?,
        &mush_core::CanonicalId::new(format!("bay.{:016x}", order.0)).map_err(|e| err(&e))?,
    )
    .map_err(|e| err(&e))
}

/// The kernel's descriptor of `name`.
fn descriptor(name: &str) -> Result<StateFamilyDescriptor, ModError> {
    kernel::family_descriptor(name).map_err(|e| ModError::msg(e.to_string()))
}

/// The Industry families the bay flow holds of its own: the hull heads, the
/// work schedule and its order index. (The shared facility slots and book
/// are [`crate::IndustryFamilies`]'; the order, offer and admission-census
/// families arrive with their typed records.)
#[derive(Clone)]
pub struct BayFamilies {
    hulls: StateFamilyHandle<ObjId, BayHullHead>,
    schedules: StateFamilyHandle<ScheduleId, DurableSchedule>,
    jobs: StateFamilyHandle<ScheduleId, WorkOrderId>,
}
impl BayFamilies {
    /// Register them, each under the kernel's exact descriptor.
    ///
    /// # Errors
    ///
    /// The registrar's.
    pub fn register(reg: &mut Registrar<'_>) -> Result<Self, ModError> {
        let hulls = reg.state_family(
            descriptor("industry_bay_hulls")?,
            |k: &ObjId| Ok(k.0.to_string()),
            |v: &serde_json::Value| {
                serde_json::from_value::<BayHullHead>(v.clone()).map_err(|e| e.to_string())
            },
            |r: &BayHullHead| r.validate().map_err(|e| e.to_string()),
            |r: &BayHullHead| serde_json::to_value(r).map_err(|e| e.to_string()),
        )?;
        let schedules = reg.state_family(
            descriptor("industry_bay_schedule")?,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
            mush_core::DurableScheduleStateCodec,
        )?;
        let jobs = reg.state_family(
            descriptor("industry_bay_schedule_order")?,
            |k: &ScheduleId| Ok(k.canonical_hash().to_hex()),
            |v: &serde_json::Value| {
                serde_json::from_value::<WorkOrderId>(v.clone()).map_err(|e| e.to_string())
            },
            |_: &WorkOrderId| Ok(()),
            |r: &WorkOrderId| serde_json::to_value(r).map_err(|e| e.to_string()),
        )?;
        Ok(Self {
            hulls,
            schedules,
            jobs,
        })
    }
    /// The hull heads.
    #[must_use]
    pub fn hulls(&self) -> &StateFamilyHandle<ObjId, BayHullHead> {
        &self.hulls
    }
    /// The bay work schedules.
    #[must_use]
    pub fn schedules(&self) -> &StateFamilyHandle<ScheduleId, DurableSchedule> {
        &self.schedules
    }
    /// Each schedule's order.
    #[must_use]
    pub fn jobs(&self) -> &StateFamilyHandle<ScheduleId, WorkOrderId> {
        &self.jobs
    }
}

/// How an action writes one family.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Write {
    /// A record that must not exist yet.
    Create,
    /// A record created on first use and replaced after.
    Upsert,
    /// An existing record replaced.
    Replace,
    /// An existing record removed.
    Delete,
}

/// When an action writes one family: always, or only for a refit that moves
/// a part in ([`When::Part`]), a part out ([`When::Returned`]), or either.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum When {
    Always,
    Part,
    Returned,
    Either,
}

/// One family an action writes: its owner, its id, how and when.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FamilyWrite {
    pub owner: &'static str,
    pub family: &'static str,
    pub write: Write,
    pub when: When,
}
const fn w(owner: &'static str, family: &'static str, write: Write, when: When) -> FamilyWrite {
    FamilyWrite {
        owner,
        family,
        write,
        when,
    }
}
use When::{Always, Either, Part, Returned};
use Write::{Create, Delete, Replace, Upsert};
const IN: &str = "industry";
const INV: &str = "inventory";

/// **Each action's exact write set.** The kernel refuses a paid action that
/// writes one family more or less than this (`bay_refit::validate_changes`);
/// Initialize and Offer write only Industry's unprotected families.
///
/// The bay's stores census moves by exactly one lot out and one back at
/// Install, so it is written whenever a part moves either way.
pub const WRITE_SETS: [(&str, &[FamilyWrite]); 6] = [
    (
        INITIALIZE,
        &[w(IN, "industry_bay_deployment", Create, Always)],
    ),
    (
        OFFER,
        &[
            w(IN, OFFERS, Create, Always),
            w(IN, "industry_bay_deployment", Replace, Always),
        ],
    ),
    (
        ACCEPT,
        &[
            w(IN, "industry_bay_orders", Create, Always),
            w(IN, "industry_bay_hulls", Upsert, Always),
            w(IN, "industry_facility_slots", Replace, Always),
            w(IN, "industry_facility_book", Replace, Always),
            w(IN, "industry_bay_schedule", Create, Always),
            w(IN, "industry_bay_schedule_order", Create, Always),
            w(IN, "industry_bay_deployment", Replace, Always),
            w(IN, CAPACITY_INDEX, Create, Always),
            w(INV, "inventory_bay_parts", Create, Part),
            w(INV, "inventory_bay_part_heads", Upsert, Part),
            w(INV, "inventory_bay_part_census", Upsert, Part),
        ],
    ),
    (
        ADVANCE,
        &[
            w(IN, "industry_bay_orders", Replace, Always),
            w(IN, "industry_bay_schedule", Replace, Always),
        ],
    ),
    (
        INSTALL,
        &[
            w(IN, "industry_bay_orders", Replace, Always),
            w(IN, "industry_bay_hulls", Replace, Always),
            w(IN, "industry_facility_slots", Replace, Always),
            w(IN, "industry_facility_book", Replace, Always),
            w(IN, "industry_bay_schedule", Delete, Always),
            w(IN, "industry_bay_schedule_order", Delete, Always),
            w(INV, "inventory_bay_parts", Replace, Part),
            w(INV, "inventory_lots", Delete, Part),
            w(INV, "inventory_lots", Create, Returned),
            w(INV, "inventory_lot_census", Replace, Either),
        ],
    ),
    (
        CANCEL,
        &[
            w(IN, "industry_bay_orders", Replace, Always),
            w(IN, "industry_bay_hulls", Replace, Always),
            w(IN, "industry_facility_slots", Replace, Always),
            w(IN, "industry_facility_book", Replace, Always),
            w(IN, "industry_bay_schedule", Delete, Always),
            w(IN, "industry_bay_schedule_order", Delete, Always),
            w(INV, "inventory_bay_parts", Replace, Part),
        ],
    ),
];

/// The digest a refit's installation evidence is joined under.
pub const REFIT_DIGEST_DOMAIN: &str = "industry.bay_refit.refit.v1";
/// A digest of `value` in `domain`, as the kernel computes it.
///
/// # Errors
///
/// The encoding's, for a value over the order ceiling.
pub fn digest(domain: &str, value: &serde_json::Value) -> Result<DefinitionDigest, ModError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, value, 196_608 + 1024)
        .map_err(|e| ModError::msg(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId};
    use std::collections::BTreeSet;

    fn recipe(id: &str) -> DefinitionRef {
        DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new("industry").expect("an owner"),
                DefinitionDescriptor::new(
                    DefinitionKind::new("recipe").expect("a kind"),
                    DefinitionId::new(id).expect("an id"),
                    1,
                    serde_json::json!({"fixture": id}),
                )
                .expect("a descriptor"),
            )
            .expect("registers")
    }

    /// The paid actions are the kernel's own descriptors, Initialize and
    /// Offer Industry's; a recipe other than `bay_refit` is refused.
    #[test]
    fn ud4b_the_policy_names_the_kernels_paid_actions() {
        let policy =
            BayRefitPolicy::new(recipe("bay_refit"), vec![ObjId(41), ObjId(40), ObjId(41)])
                .expect("a policy");
        assert_eq!(policy.facilities(), &[ObjId(40), ObjId(41)]);
        for action in ACTIONS {
            let d = policy.action_descriptor(action).expect("a descriptor");
            assert_eq!(d.id().as_str(), action);
        }
        assert!(BayRefitPolicy::new(recipe("core_refit"), vec![ObjId(40)]).is_err());
        assert!(BayRefitPolicy::new(recipe("bay_refit"), vec![]).is_err());
    }

    /// Every family the write sets name is one the kernel declares, or one
    /// of Industry's two unprotected families; and every family the kernel
    /// declares is written by some action.
    #[test]
    fn ud4b_the_write_sets_name_exactly_the_kernels_families() {
        let kernel: BTreeSet<(&str, &str)> = kernel::FAMILY_NAMES.into_iter().collect();
        let written: BTreeSet<(&str, &str)> = WRITE_SETS
            .iter()
            .flat_map(|(_, writes)| writes.iter().map(|w| (w.owner, w.family)))
            .collect();
        let own: BTreeSet<(&str, &str)> = [(IN, OFFERS), (IN, CAPACITY_INDEX)].into();
        assert_eq!(
            written
                .difference(&kernel)
                .copied()
                .collect::<BTreeSet<_>>(),
            own
        );
        assert!(kernel.is_subset(&written));
        for (owner, family) in kernel::FAMILY_NAMES {
            assert_eq!(
                kernel::family_descriptor(family)
                    .expect("declared")
                    .id()
                    .as_str(),
                family,
                "{owner}"
            );
        }
    }

    /// The hull head's wire is the one the kernel joins.
    #[test]
    fn ud4b_the_hull_head_is_the_kernels_shape() {
        let held = BayHullHead::held(ObjId(99), CommitToken::from_bytes([1; 16]), WorkOrderId(7));
        assert_eq!(
            serde_json::to_value(&held).expect("encodes"),
            serde_json::json!({"format":1,"hull":99,"operation":CommitToken::from_bytes([1; 16]),"pending":7})
        );
        let released = BayHullHead::released(ObjId(99), CommitToken::from_bytes([2; 16]));
        assert_eq!(
            serde_json::to_value(&released).expect("encodes")["pending"],
            serde_json::Value::Null
        );
        assert!(
            serde_json::from_value::<BayHullHead>(
                serde_json::json!({"format":1,"hull":99,"operation":CommitToken::from_bytes([1; 16])})
            )
            .is_err(),
            "pending is required, even when null"
        );
    }
}
