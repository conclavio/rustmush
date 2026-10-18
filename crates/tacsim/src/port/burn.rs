//! **`tacsim_port_burn` — the departure burn** (doc 18 cut IC9; doc 18 §4.2
//! "Propellant and provisions as consumables", §4.4 "The fuel function is the
//! planner's output", "Failure handling"; D-18-11, D-18-30, D-18-46; lead rulings
//! L4, L5, L6, L19, L34 of the IC9 brief).
//!
//! One authoritative action, submitted by `+ship/undock` as the pilot
//! ([`crate::port::voyage`]), whose prepare is the only place reaction mass and
//! provisions are spent for a passage: **matter first, in one bundle** — it
//! recomputes the payload from the hold's pools, recomputes the burn and the
//! crew-days from the filed plan's seconds and the drive table this provider
//! holds ([`BurnTable`], D-18-11 "the drive table the provider holds"), refuses
//! an overloaded hull, a short tank, a short locker and any mismatch, debits the
//! propellant and provisions pools through the delegated inventory families, and
//! inserts the `tacsim.ticket` carrying the whole [`PassagePlan`] (D-18-46).
//!
//! **The plan reaches the prepare only from the server's own filed document**
//! (lead ruling L4): the offer carries the hull's pending voyage as a
//! server-authenticated fixed parameter (the IC7 `paid` / IC8 rows precedent),
//! and the submission names only the `seq`. A submitter can never inject the
//! seconds a journey will fly.
//!
//! The launch itself is not here: the `tacsim.port` tick's launch arm finds the
//! burned ticket on its next fire and calls `mush_sim::depart` with the plan's
//! frozen leg times (`crate::port::voyage::launch_step`).

use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{
    ActionErrorCode, ActionInvocation, ActionProvider, ActionTxn, AuthoritativeActionOutput,
    CanonicalId, DefinitionDigest, DefinitionRef, ObjId, PerceivedRef, ProjectedParameter,
    ProjectedParameterKind, ProjectedParameterSchema, ProjectedTargetSchema, ProviderOfferDraft,
    RevisionSet, RulesValue, StateFamilyHandle, SubjectKey,
};
use mush_economy::VaultId;
use mush_inventory::InventoryFamilies;
use mush_inventory::record::EndpointRef;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::definitions::goods::Consumable;
use crate::definitions::{Catalog, GoodsTable};
use crate::port::coop::helm_of;
use crate::port::families::{Manifest, Ticket, TicketId};
use crate::port::passage::{HullDrive, PlanError, crew_days, passage_burn, payload_kg};
use crate::port::ship::{Berth, berth_of};
use crate::port::voyage::{PendingVoyage, VoyageState, aboard};
use crate::port_tick::port_error;
use crate::store::TacsimStore;

/// The burn's action id (lead ruling L6: IC8's `tacsim_port_stock` naming).
pub const BURN_ACTION: &str = "tacsim_port_burn";
/// The burn's outcome id.
pub const BURN_OUTCOME: &str = "tacsim_port_burn_result";
/// The burn's idempotency-key prefix: the live key is
/// `tacsim.port.burn/{hull}/undock/{seq}` (IC11 keeps §4.4's
/// `…/{order}/{attempt}` for queued departs).
pub const BURN_KEY_PREFIX: &str = "tacsim.port.burn/";
/// Kilograms in one tonne of reaction mass: the burn debits whole tonnes (lead
/// ruling L19), so the table refuses a propellant good counted otherwise.
pub const PROPELLANT_KG_PER_UNIT: u64 = 1_000;

// ---- the drive table the provider holds ------------------------------------------

/// **One fielded hull, as the burn reads it** (D-18-11): its drive (class, law,
/// rail), its dry mass, its occupant allowance and every good any of its
/// compartments accepts, in goods-table order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HullSpec {
    pub(crate) drive: HullDrive,
    pub(crate) dry_kg: u64,
    pub(crate) allowance_kg: u32,
    pub(crate) accepted: Vec<CanonicalId>,
}

/// **The table the provider holds** (D-18-11): the goods table and every fielded
/// spec that declares `logistics`, keyed by its unit definition's digest — the
/// very digest a voyage and a ticket name as `unit_definition` (lead ruling L34).
#[derive(Clone, Debug)]
pub struct BurnTable {
    pub(crate) goods: Arc<GoodsTable>,
    pub(crate) hulls: BTreeMap<DefinitionDigest, HullSpec>,
}

impl BurnTable {
    /// The propellant good and the provisions good, by `GoodDef.consumable`
    /// (never by id).
    fn consumable(&self, kind: Consumable) -> Option<CanonicalId> {
        consumable_good(&self.goods, kind)
    }
}

/// The good the table marks `kind` (the first, table order).
pub(crate) fn consumable_good(goods: &GoodsTable, kind: Consumable) -> Option<CanonicalId> {
    goods
        .goods
        .iter()
        .find(|good| good.consumable == Some(kind))
        .map(|good| good.id.clone())
}

/// **One fielded unit's hull spec and digest** (`burn_table`'s row, and what
/// `+ship/plan`/`+ship/undock` plan with): the spec must carry `logistics`, the
/// drive class must have a law in the drive table, and the unit must have a
/// minted reference. The refusal names what is missing.
pub(crate) fn hull_spec_of(
    catalog: &Catalog,
    goods: &GoodsTable,
    unit: &CanonicalId,
) -> Result<(DefinitionDigest, HullSpec), String> {
    let spec = catalog
        .specs
        .get(unit)
        .ok_or_else(|| format!("no unit {unit} is fielded"))?;
    let logistics = spec
        .logistics
        .as_ref()
        .ok_or_else(|| format!("unit {unit} carries no logistics"))?;
    let law = catalog
        .drive_for_class(&logistics.drive.class)
        .copied()
        .ok_or_else(|| {
            format!(
                "drive class {} of unit {unit} has no law",
                logistics.drive.class
            )
        })?;
    let digest = catalog
        .references
        .get(unit)
        .map(DefinitionRef::digest)
        .ok_or_else(|| format!("unit {unit} has no minted reference"))?;
    let accepted: Vec<CanonicalId> = goods
        .goods
        .iter()
        .filter(|good| {
            logistics
                .stowage
                .values()
                .any(|compartment| compartment.accepts.contains(&good.stowage))
        })
        .map(|good| good.id.clone())
        .collect();
    let dry_kg = u64::from(logistics.dry_mass_t)
        .checked_mul(1_000)
        .ok_or_else(|| format!("unit {unit}'s dry mass overflows"))?;
    Ok((
        digest,
        HullSpec {
            drive: HullDrive {
                class: logistics.drive.class.clone(),
                law,
                payload_ceiling_kg: logistics.payload_ceiling_kg,
            },
            dry_kg,
            allowance_kg: logistics.occupant_allowance_kg,
            accepted,
        },
    ))
}

/// **Build the table** (D-18-11): every fielded spec with `logistics`, keyed by
/// its `references[entry]` digest; a class with no law is left out (logged).
/// `None` without a goods table, and — logged — when the table's propellant good
/// is absent or not counted at exactly [`PROPELLANT_KG_PER_UNIT`] kilograms a
/// unit (the burn debits whole tonnes, lead ruling L19).
pub(crate) fn burn_table(catalog: &Catalog, goods: Option<Arc<GoodsTable>>) -> Option<BurnTable> {
    let goods = goods?;
    let propellant_kg = consumable_good(&goods, Consumable::Propellant)
        .and_then(|id| goods.good(&id))
        .and_then(crate::definitions::GoodDef::mass_kg_per_unit);
    if propellant_kg != Some(PROPELLANT_KG_PER_UNIT) {
        tracing::error!(
            ?propellant_kg,
            "tacsim: the goods table's propellant is not counted in whole tonnes; the departure \
             burn is not registered (no hull can undock on this server)"
        );
        return None;
    }
    let mut hulls: BTreeMap<DefinitionDigest, HullSpec> = BTreeMap::new();
    for (unit, spec) in &catalog.specs {
        if spec.logistics.is_none() {
            continue;
        }
        match hull_spec_of(catalog, &goods, unit) {
            Ok((digest, hull)) => {
                hulls.insert(digest, hull);
            }
            Err(error) => {
                tracing::warn!(%unit, %error, "tacsim: a fielded hull is left out of the burn table");
            }
        }
    }
    Some(BurnTable { goods, hulls })
}

// ---- the provider ------------------------------------------------------------------

/// **`tacsim_port_burn`** (doc 18 §4.2, §4.4; D-18-11, D-18-46).
pub(crate) struct BurnProvider {
    pub action: DefinitionRef,
    pub outcome: DefinitionRef,
    pub tickets: StateFamilyHandle<TicketId, Ticket>,
    pub inventory: InventoryFamilies,
    pub table: BurnTable,
}

/// The fixed `voyage` parameter: the hull and its pending voyage, exactly as the
/// server filed it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedVoyage {
    hull: ObjId,
    pending: PendingVoyage,
}

fn invalid() -> mush_core::ActionError {
    port_error(
        ActionErrorCode::InvalidRequest,
        "the burn is self-scoped, takes no targets and names one filed passage",
    )
}

/// The invocation's parameters, checked for shape: self-scoped, no targets, an
/// object naming one `seq` as decimal text.
fn parameters(
    invocation: &ActionInvocation,
) -> Result<&Map<String, Value>, mush_core::ActionError> {
    if invocation.subject() != &PerceivedRef::SelfActor || !invocation.targets().is_empty() {
        return Err(invalid());
    }
    let parameters = invocation.parameters().as_object().ok_or_else(invalid)?;
    parameters
        .get("seq")
        .and_then(Value::as_str)
        .and_then(crate::port::families::strict_decimal_u32)
        .ok_or_else(invalid)?;
    Ok(parameters)
}

/// **Whether `actor` may burn now, and the pending voyage it would burn** — the
/// offer's whole gate, read from the world at landing: the actor's title or live
/// charter names the hull (cut IC13, `helm_of`); the hull's voyage is readable with a pending filed by the
/// actor; the actor is `Interior`-placed in the hull's airlock space; the hull is
/// docked.
pub(crate) fn burnable(w: &mush_core::World, actor: ObjId) -> Option<(ObjId, PendingVoyage)> {
    let hull = helm_of(w, actor)?.hull;
    let store = w.module_state::<TacsimStore>()?;
    let pending = match store.voyage_state(hull) {
        VoyageState::Present(voyage) => voyage.pending.clone()?,
        VoyageState::Absent | VoyageState::Unreadable => return None,
    };
    if pending.pilot != actor {
        return None;
    }
    if !aboard(w, hull, actor) || !matches!(berth_of(w, hull), Berth::Docked { .. }) {
        return None;
    }
    Some((hull, pending))
}

/// A pool's quantity and availability through the delegated families (absent is
/// 0); a refusal is the closure's `Unreadable`.
fn pool_of(
    inventory: &InventoryFamilies,
    txn: &mut ActionTxn<'_>,
    vault: VaultId,
    good: &CanonicalId,
) -> Result<(u64, u64), PlanError> {
    inventory
        .pool(txn, EndpointRef(vault), good)
        .map(|pool| pool.map_or((0, 0), |pool| (pool.quantity, pool.available)))
        .map_err(|error| PlanError::Unreadable {
            what: format!("the hold's {good} ({error})"),
        })
}

impl ActionProvider for BurnProvider {
    fn validate_invocation(
        &self,
        invocation: &ActionInvocation,
    ) -> Result<(), mush_core::ActionError> {
        parameters(invocation).map(|_| ())
    }

    fn project_offer(
        &self,
        action: &DefinitionRef,
        query: &mush_core::LegalActionQuery,
        engine: &mush_core::Engine,
    ) -> Result<Option<ProviderOfferDraft>, mush_core::ActionError> {
        let SubjectKey::Object { id: subject } = query.truth_subject() else {
            return Ok(None);
        };
        if action != &self.action
            || query.actor() != *subject
            || query.projected_subject() != &PerceivedRef::SelfActor
        {
            return Ok(None);
        }
        let Some((hull, pending)) = burnable(&engine.world, query.actor()) else {
            return Ok(None);
        };
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let filed = serde_json::to_value(&pending)
            .map_err(|_| integrity("the filed passage does not encode"))?;
        let definitions = engine
            .registry
            .definitions()
            .ok_or_else(|| integrity("the burn's definition registry is unavailable"))?;
        let mut guards = engine
            .registry
            .server_guard_set_builder(RevisionSet::default())
            .map_err(|_| integrity("the burn's guard registry is unavailable"))?;
        guards
            .add_definition(action.clone(), definitions)
            .map_err(|_| integrity("the burn's definition guard is unavailable"))?;
        let seq = ProjectedParameter {
            id: CanonicalId::new("seq")
                .map_err(|_| integrity("the burn's parameter id is invalid"))?,
            required: true,
            kind: ProjectedParameterKind::Choice {
                choices: vec![pending.seq.to_string()],
            },
        };
        Ok(Some(ProviderOfferDraft {
            fixed_parameters: json!({
                "now": engine.world.world_ic_now().at,
                "voyage": { "hull": hull.0, "pending": filed },
            }),
            parameters: ProjectedParameterSchema::new(vec![seq])
                .map_err(|_| integrity("the burn's parameter schema is invalid"))?,
            targets: ProjectedTargetSchema::None,
            required_channels: Vec::new(),
            expires: None,
            presentation: mush_core::ActionPresentation::new(
                "tacsim-port-burn",
                "Burn for the filed passage",
                "Commit the reaction mass and the stores the filed passage needs.",
            )
            .map_err(|_| integrity("the burn's presentation is invalid"))?,
            guards: guards
                .finish()
                .map_err(|_| integrity("the burn's offer guards are invalid"))?,
        }))
    }

    #[allow(clippy::too_many_lines)] // one ordered refusal list, then one bundle
    fn prepare_authoritative(
        &self,
        invocation: &ActionInvocation,
        txn: &mut ActionTxn<'_>,
    ) -> Result<AuthoritativeActionOutput, mush_core::ActionError> {
        let parameters = parameters(invocation)?;
        let seq = parameters
            .get("seq")
            .and_then(Value::as_str)
            .and_then(crate::port::families::strict_decimal_u32)
            .ok_or_else(invalid)?;
        // 1. The server-authenticated filing (L4).
        let fixed: FixedVoyage = parameters
            .get("voyage")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .ok_or_else(invalid)?;
        let now = parameters
            .get("now")
            .and_then(Value::as_i64)
            .ok_or_else(invalid)?;
        let FixedVoyage { hull, pending } = fixed;
        if pending.seq != seq {
            return Err(port_error(
                ActionErrorCode::InvalidRequest,
                "that passage is not the one on file",
            ));
        }
        let actor = invocation.actor();
        if actor != pending.pilot {
            return Err(port_error(
                ActionErrorCode::Unauthorized,
                "that passage is another pilot's",
            ));
        }
        let plan = &pending.plan;
        if plan.depart_ic > now {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "the filed passage departs in the future",
            ));
        }
        let Some(spec) = self.table.hulls.get(&pending.unit_definition) else {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "the filed passage names no hull this server knows",
            ));
        };
        // 2. The ticket is the replay guard.
        let id = TicketId { hull, seq };
        let prior = txn
            .get(&self.tickets, &id)
            .map_err(|_| port_error(ActionErrorCode::Integrity, "the tickets are unavailable"))?;
        if prior.is_some() {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "that passage is already burned",
            ));
        }
        // 3. The payload, recomputed from the hold's pools (L12).
        let vault = VaultId(pending.vault);
        let unreadable =
            |_: PlanError| port_error(ActionErrorCode::Integrity, "the hold cannot be read");
        let mut contents: BTreeMap<CanonicalId, (u64, u64)> = BTreeMap::new();
        for good in &spec.accepted {
            let reading = pool_of(&self.inventory, txn, vault, good).map_err(unreadable)?;
            contents.insert(good.clone(), reading);
        }
        let payload = payload_kg(
            &self.table.goods,
            &spec.accepted,
            |good| Ok(contents.get(good).map_or(0, |(quantity, _)| *quantity)),
            pending.occupants,
            spec.allowance_kg,
        )
        .map_err(unreadable)?;
        if payload != pending.payload_kg {
            return Err(port_error(
                ActionErrorCode::Conflict,
                "the hold changed since the passage was planned; plan it again",
            ));
        }
        if payload > spec.drive.payload_ceiling_kg {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the hull is over its rated payload",
            ));
        }
        // 4. The burn and the crew-days, recomputed from the filed seconds and the
        // drive table this provider holds (D-18-11).
        let adds_up = plan.check().is_ok()
            && plan.law.as_str() == spec.drive.law.word()
            && passage_burn(&spec.drive.law, spec.dry_kg, payload, &plan.legs).ok()
                == Some(plan.burn_t)
            && crew_days(pending.occupants, &plan.legs).ok() == Some(plan.crew_days);
        if !adds_up {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "the filed passage does not add up",
            ));
        }
        // 5. The tank and the locker.
        let (Some(propellant), Some(provisions)) = (
            self.table.consumable(Consumable::Propellant),
            self.table.consumable(Consumable::Provisions),
        ) else {
            return Err(port_error(
                ActionErrorCode::Integrity,
                "the goods table names no propellant or no provisions",
            ));
        };
        let reading = |good: &CanonicalId| contents.get(good).copied().unwrap_or((0, 0));
        let (tank, tank_free) = reading(&propellant);
        let (locker, locker_free) = reading(&provisions);
        if tank_free < plan.burn_t {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the tank holds less reaction mass than the passage burns",
            ));
        }
        let crew = u64::from(plan.crew_days);
        if locker_free < crew {
            return Err(port_error(
                ActionErrorCode::ProviderRejected,
                "the stores locker holds fewer crew-days than the passage needs",
            ));
        }
        // 6. The manifest, BEFORE the debit (D-18-11).
        let cargo: BTreeMap<CanonicalId, u64> = contents
            .iter()
            .filter(|(good, (quantity, _))| {
                *quantity > 0
                    && self
                        .table
                        .goods
                        .good(good)
                        .is_some_and(|def| def.consumable.is_none())
            })
            .map(|(good, (quantity, _))| (good.clone(), *quantity))
            .collect();
        let manifest = Manifest {
            cargo,
            tank,
            locker,
            occupants: pending.occupants,
        };
        // 7. Matter first, in this one bundle: the debits, then the ticket.
        let rejected =
            |message: &'static str| port_error(ActionErrorCode::ProviderRejected, message);
        if plan.burn_t > 0 {
            self.inventory
                .debit(txn, EndpointRef(vault), &propellant, plan.burn_t)
                .map_err(|_| rejected("the tank could not be drawn down"))?;
        }
        if crew > 0 {
            self.inventory
                .debit(txn, EndpointRef(vault), &provisions, crew)
                .map_err(|_| rejected("the stores locker could not be drawn down"))?;
        }
        let ticket = Ticket {
            hull,
            seq,
            route: plan.route.clone(),
            route_pack: pending.route_pack.clone(),
            route_version: pending.route_version,
            unit_definition: pending.unit_definition,
            manifest,
            payload_kg: payload,
            plan: plan.clone(),
            order: pending.order,
            attempt: pending.attempt,
            burned_at: now,
        };
        ticket
            .check()
            .map_err(|_| rejected("the ticket is out of bounds"))?;
        let outcome_fields = json!({
            "actor": actor.0,
            "burn_t": plan.burn_t,
            "crew_days": plan.crew_days,
            "hull": hull.0,
            "seq": seq,
        });
        txn.insert(&self.tickets, &ticket.ticket_id(), ticket)
            .map_err(|_| port_error(ActionErrorCode::Integrity, "the ticket did not stage"))?;
        let outcome = RulesValue::new(self.outcome.clone(), outcome_fields).map_err(|_| {
            port_error(
                ActionErrorCode::BoundsExceeded,
                "the burn's outcome exceeds canonical bounds",
            )
        })?;
        Ok(AuthoritativeActionOutput::new(outcome)
            .with_eligible_witnesses(vec![invocation.principal().clone()]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fielded() -> Catalog {
        Catalog::build_fielded(&[], crate::NS).expect("the fielded content binds")
    }

    #[test]
    fn ic9_the_burn_table_holds_the_dray_under_its_digest() {
        let catalog = fielded();
        let goods = Arc::new(crate::definitions::goods::goods_table().expect("the goods table"));
        let table = burn_table(&catalog, Some(goods)).expect("the table builds");
        let dray = CanonicalId::new(crate::definitions::content::DRAY).expect("id");
        let digest = catalog
            .references
            .get(&dray)
            .expect("the Dray's ref")
            .digest();
        let spec = table.hulls.get(&digest).expect("the Dray is in the table");
        assert_eq!(
            spec.drive.law,
            crate::definitions::logistics::DriveLaw::Coast {
                kms: 226,
                burn_permille: 105
            }
        );
        assert_eq!(spec.drive.class.as_str(), "coast_hauler");
        assert_eq!(spec.drive.payload_ceiling_kg, 181_000);
        assert_eq!(spec.dry_kg, 240_000);
        assert_eq!(spec.allowance_kg, 100);
        assert!(spec.accepted.iter().any(|g| g.as_str() == "propellant"));
        assert!(spec.accepted.iter().any(|g| g.as_str() == "provisions"));
        assert!(burn_table(&catalog, None).is_none(), "no goods, no table");
    }

    #[test]
    fn ic9_a_table_whose_propellant_is_not_whole_tonnes_is_refused() {
        let catalog = fielded();
        let mut goods = crate::definitions::goods::goods_table().expect("the goods table");
        let mass = CanonicalId::new(crate::definitions::logistics::MASS_KG).expect("id");
        for good in &mut goods.goods {
            if good.consumable == Some(Consumable::Propellant) {
                good.measures.insert(mass.clone(), 999);
            }
        }
        assert!(burn_table(&catalog, Some(Arc::new(goods))).is_none());
    }
}
