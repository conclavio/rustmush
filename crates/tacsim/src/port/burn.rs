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
//! inserts the `tacsim.ticket` carrying the whole [`crate::port::PassagePlan`] (D-18-46).
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
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::definitions::goods::Consumable;
use crate::definitions::{Catalog, GoodsTable};
use crate::port::coop::helm_of;
use crate::port::families::{Manifest, Ticket, TicketId};
use crate::port::passage::{HullDrive, PlanError, carried_kg, crew_days, passage_burn, payload_kg};
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

/// **A drive-less unit's refusal** (doc 20 cut SM.3a; §4.1 "stowage-only"):
/// `{unit}` is replaced by the unit's id. The terrain vehicle carries a hopper
/// and no drive, so it never plans a passage.
pub const NO_DRIVE: &str = "unit {unit} carries no drive; it never plans a passage";

/// **One fielded unit's hull spec and digest** (`burn_table`'s row, and what
/// `+ship/plan`/`+ship/undock` plan with): the spec must carry `logistics` and a
/// drive ([`NO_DRIVE`] otherwise, doc 20 cut SM.3a), the drive class must have a
/// law in the drive table, and the unit must have a minted reference. The
/// refusal names what is missing.
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
    let drive = logistics
        .drive
        .as_ref()
        .ok_or_else(|| NO_DRIVE.replace("{unit}", unit.as_str()))?;
    let law = catalog
        .drive_for_class(&drive.class)
        .copied()
        .ok_or_else(|| format!("drive class {} of unit {unit} has no law", drive.class))?;
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
                class: drive.class.clone(),
                law,
                payload_ceiling_kg: logistics.payload_ceiling_kg,
                throttle_permille: crate::port::passage::DEFAULT_THROTTLE_PERMILLE,
            },
            dry_kg,
            allowance_kg: logistics.occupant_allowance_kg,
            accepted,
        },
    ))
}

/// **Build the table** (D-18-11): every fielded spec with `logistics`, keyed by
/// its `references[entry]` digest; a class with no law is left out (logged), and
/// a drive-less (stowage-only) spec is left out silently — doc 20 cut SM.3a's
/// terrain vehicle never plans a passage, so it has no row to miss.
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
        if spec
            .logistics
            .as_ref()
            .is_none_or(|logistics| logistics.drive.is_none())
        {
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
    /// `tacsim.space_burn_receipt` (doc 35 cut SF.R26b): which in-flight burns have landed.
    pub space_receipts: StateFamilyHandle<ObjId, SpaceBurnReceipts>,
    pub inventory: InventoryFamilies,
    pub table: BurnTable,
}

/// Typed state family id for the in-flight burn receipts (doc 35 cut SF.R26b, doc 35d §9a).
pub const SPACE_BURN_RECEIPT_FAMILY: &str = "tacsim.space_burn_receipt";

/// How many landed sequences a hull's receipt row keeps. The burn queue holds sixteen, so a
/// sequence older than the lowest of these is long gone from it.
pub const MAX_SPACE_BURN_RECEIPTS: usize = 32;

/// **Doc 35 cut SF.R26b (doc 35d §9a)** — the in-flight burns of one hull that have landed.
///
/// Written by `tacsim_port_burn`'s prepare in the same transaction as the debit, so a receipt
/// and a drawn-down tank are one fact. The space tick reads it to take a landed burn off the
/// queue, and the prepare reads it to refuse a burn that has already landed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpaceBurnReceipts {
    /// The hull whose tank was drawn down; the row's key.
    pub hull: ObjId,
    /// Landed burn sequences, ascending, at most [`MAX_SPACE_BURN_RECEIPTS`].
    pub landed: Vec<u32>,
}

impl SpaceBurnReceipts {
    /// The record invariants of `tacsim.space_burn_receipt`.
    ///
    /// # Errors
    ///
    /// The row names no hull, holds too many sequences, or holds them out of order.
    pub fn check(&self) -> Result<(), String> {
        if self.hull.0 == 0 {
            return Err("a space burn receipt names its hull".to_owned());
        }
        if self.landed.len() > MAX_SPACE_BURN_RECEIPTS {
            return Err("a space burn receipt holds a bounded number of sequences".to_owned());
        }
        if !self.landed.windows(2).all(|pair| pair[0] < pair[1]) {
            return Err("a space burn receipt holds its sequences ascending".to_owned());
        }
        Ok(())
    }

    /// Whether the burn `seq` has landed. A sequence below the lowest one a full row keeps
    /// has been dropped from it, and counts as landed.
    #[must_use]
    pub fn has_landed(&self, seq: u32) -> bool {
        self.landed.binary_search(&seq).is_ok()
            || (self.landed.len() >= MAX_SPACE_BURN_RECEIPTS
                && self.landed.first().is_some_and(|&lowest| seq < lowest))
    }

    /// The highest landed sequence, 0 when none has.
    #[must_use]
    pub fn high_water(&self) -> u32 {
        self.landed.last().copied().unwrap_or(0)
    }

    /// Record `seq` as landed, dropping the lowest sequence when the row is full.
    pub fn record(&mut self, seq: u32) {
        if let Err(at) = self.landed.binary_search(&seq) {
            self.landed.insert(at, seq);
        }
        if self.landed.len() > MAX_SPACE_BURN_RECEIPTS {
            let excess = self.landed.len() - MAX_SPACE_BURN_RECEIPTS;
            self.landed.drain(0..excess);
        }
    }
}

/// The fixed `voyage` parameter: the hull and its pending voyage, exactly as the
/// server filed it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedVoyage {
    hull: ObjId,
    pending: PendingVoyage,
}

/// **Doc 35 cut SF.R4 (`D-35-6`)** — a server-authenticated in-flight reaction-mass debit
/// queued by `+helm/burn`, `+helm/thrust`, or `+helm/autopilot` against the hull's live
/// `stow.propellant` vault.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingSpaceBurn {
    /// Monotonic sequence number for this space burn (`seq` parameter).
    pub seq: u32,
    /// Carrier hull root [`ObjId`].
    pub hull: ObjId,
    /// Actor who commanded the burn.
    pub actor: ObjId,
    /// Hold vault id (`VaultId.0`) holding the hull's `propellant` and `provisions` pools.
    pub vault: u64,
    /// Whole tonnes of reaction mass to debit from `stow.propellant`.
    pub burn_t: u64,
    /// Crew-days of provisions to debit from `stow.provisions` (non-zero when re-entering an autopilot passage).
    pub crew_days: u32,
    /// Whole tonnes of unlicensed `strategic_ore` seized at a customs picket (`SF.R12`, `D-35-11`).
    #[serde(default)]
    pub seized_strategic_ore_t: u32,
    /// Whether this burn pays for a planned passage (`+helm/autopilot`, the linkdead return).
    /// A passage whose burn cannot land is cut (doc 35 cut SF.R26b).
    #[serde(default)]
    pub passage: bool,
    /// How many times the space tick has submitted this burn again (doc 35 cut SF.R26b).
    #[serde(default)]
    pub attempts: u8,
    /// Consecutive ticks on which the burn had neither landed nor been in flight.
    #[serde(default)]
    pub idle_ticks: u8,
    /// Whether the burn was queued under the receipt rule (doc 35 cut SF.R26b). A burn read
    /// from a store written before that cut has no receipt to wait for; the tick leaves it to
    /// the older rule and never submits it again.
    #[serde(default)]
    pub tracked: bool,
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

/// **Doc 35 cut SF.R4 (`D-35-6`)** — whether `actor` has one or more server-authenticated
/// in-flight space burns queued on the hull they are physically aboard.
pub(crate) fn space_burnable(w: &mush_core::World, actor: ObjId) -> Option<Vec<PendingSpaceBurn>> {
    let hull = crate::space::commands::hull_aboard_for_actor(w, actor)?;
    let book = w.module_state::<crate::space::SpaceBubbleBook>()?;
    let burns: Vec<PendingSpaceBurn> = book
        .pending_space_burns
        .iter()
        .filter(|b| b.actor == actor && b.hull == hull)
        .cloned()
        .collect();
    (!burns.is_empty()).then_some(burns)
}

/// A pool's quantity and availability through the delegated families (absent is
/// 0); a refusal is the closure's `Unreadable`.
fn pool_of(
    inventory: &InventoryFamilies,
    txn: &mut ActionTxn<'_>,
    vault: VaultId,
    good: &CanonicalId,
) -> Result<(u64, u64), PlanError> {
    let unreadable = |error: String| PlanError::Unreadable {
        what: format!("the hold's {good} ({error})"),
    };
    let (quantity, available) = inventory
        .pool(txn, EndpointRef(vault), good)
        .map(|pool| pool.map_or((0, 0), |pool| (pool.quantity, pool.available)))
        .map_err(|error| unreadable(error.to_string()))?;
    // Doc 39 cut IN.11: an indivisible good (a part) is held as lots, which the planner's
    // reading counts by the endpoint's lot tally (IN.1); the prepare counts them the same way, or
    // a hold with a part aboard could never burn ("the hold changed since the passage was
    // planned").
    if !inventory.is_indivisible(good) {
        return Ok((quantity, available));
    }
    let key = mush_inventory::pool_key(EndpointRef(vault), good)
        .map_err(|error| unreadable(error.to_string()))?;
    let lots = txn
        .get(&inventory.lot_tally, &key)
        .map_err(|error| unreadable(error.to_string()))?
        .copied()
        .unwrap_or(0);
    Ok((
        quantity.saturating_add(lots),
        available.saturating_add(lots),
    ))
}

fn rejected(message: &'static str) -> mush_core::ActionError {
    port_error(ActionErrorCode::ProviderRejected, message)
}

fn debit_consumables(
    inventory: &InventoryFamilies,
    txn: &mut ActionTxn<'_>,
    vault: VaultId,
    propellant: &CanonicalId,
    burn_t: u64,
    provisions: &CanonicalId,
    crew: u64,
) -> Result<(), mush_core::ActionError> {
    if burn_t > 0 {
        inventory
            .debit(txn, EndpointRef(vault), propellant, burn_t)
            .map_err(|_| rejected("the tank could not be drawn down"))?;
    }
    if crew > 0 {
        inventory
            .debit(txn, EndpointRef(vault), provisions, crew)
            .map_err(|_| rejected("the stores locker could not be drawn down"))?;
    }
    Ok(())
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
        let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
        let (choices, fixed_parameters) =
            if let Some((hull, pending)) = burnable(&engine.world, query.actor()) {
                let filed = serde_json::to_value(&pending)
                    .map_err(|_| integrity("the filed passage does not encode"))?;
                (
                    vec![pending.seq.to_string()],
                    json!({
                        "now": engine.world.world_ic_now().at,
                        "voyage": { "hull": hull.0, "pending": filed },
                    }),
                )
            } else if let Some(burns) = space_burnable(&engine.world, query.actor()) {
                let choices: Vec<String> = burns.iter().map(|b| b.seq.to_string()).collect();
                let encoded = serde_json::to_value(&burns)
                    .map_err(|_| integrity("the filed space burn does not encode"))?;
                (
                    choices,
                    json!({
                        "now": engine.world.world_ic_now().at,
                        "space_burns": encoded,
                    }),
                )
            } else {
                return Ok(None);
            };
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
            kind: ProjectedParameterKind::Choice { choices },
        };
        Ok(Some(ProviderOfferDraft {
            fixed_parameters,
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
        if let Some(space_burns_val) = parameters.get("space_burns") {
            let burns: Vec<PendingSpaceBurn> =
                serde_json::from_value(space_burns_val.clone()).map_err(|_| invalid())?;
            let Some(pending) = burns.into_iter().find(|b| b.seq == seq) else {
                return Err(port_error(
                    ActionErrorCode::InvalidRequest,
                    "that space burn is not the one on file",
                ));
            };
            let actor = invocation.actor();
            if actor != pending.actor {
                return Err(port_error(
                    ActionErrorCode::Unauthorized,
                    "that space burn is another pilot's",
                ));
            }
            let (Some(propellant), Some(provisions)) = (
                self.table.consumable(Consumable::Propellant),
                self.table.consumable(Consumable::Provisions),
            ) else {
                return Err(port_error(
                    ActionErrorCode::Integrity,
                    "the goods table names no propellant or no provisions",
                ));
            };
            // Doc 35 cut SF.R26b: the receipt is the replay guard. The tick submits a burn
            // again when it has no receipt and nothing is in flight; a burn that did land is
            // refused here, whatever became of its idempotency key.
            let prior = txn
                .get(&self.space_receipts, &pending.hull)
                .map_err(|_| {
                    port_error(
                        ActionErrorCode::Integrity,
                        "the space burn receipts are unavailable",
                    )
                })?
                .cloned();
            if prior.as_ref().is_some_and(|row| row.has_landed(seq)) {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    "that space burn has already landed",
                ));
            }
            let vault = VaultId(pending.vault);
            let unreadable =
                |_: PlanError| port_error(ActionErrorCode::Integrity, "the hold cannot be read");
            let (_, tank_free) =
                pool_of(&self.inventory, txn, vault, &propellant).map_err(unreadable)?;
            if tank_free < pending.burn_t {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    "the tank holds less reaction mass than the burn needs",
                ));
            }
            if pending.seized_strategic_ore_t > 0 {
                let strategic_ore = CanonicalId::new("strategic_ore").map_err(|_| invalid())?;
                let (_, ore_free) =
                    pool_of(&self.inventory, txn, vault, &strategic_ore).map_err(unreadable)?;
                let seize_t = u64::from(pending.seized_strategic_ore_t).min(ore_free);
                debit_consumables(
                    &self.inventory,
                    txn,
                    vault,
                    &propellant,
                    pending.burn_t,
                    &strategic_ore,
                    seize_t,
                )?;
            } else {
                let (_, locker_free) =
                    pool_of(&self.inventory, txn, vault, &provisions).map_err(unreadable)?;
                let crew = u64::from(pending.crew_days);
                if locker_free < crew {
                    return Err(port_error(
                        ActionErrorCode::ProviderRejected,
                        "the stores locker holds fewer crew-days than the passage needs",
                    ));
                }
                debit_consumables(
                    &self.inventory,
                    txn,
                    vault,
                    &propellant,
                    pending.burn_t,
                    &provisions,
                    crew,
                )?;
            }
            let mut receipts = prior.clone().unwrap_or(SpaceBurnReceipts {
                hull: pending.hull,
                landed: Vec::new(),
            });
            receipts.record(seq);
            let staged = if prior.is_some() {
                txn.replace(&self.space_receipts, &pending.hull, receipts)
            } else {
                txn.insert(&self.space_receipts, &pending.hull, receipts)
            };
            staged.map_err(|_| {
                port_error(
                    ActionErrorCode::Integrity,
                    "the space burn receipt did not stage",
                )
            })?;
            let outcome_fields = json!({
                "actor": actor.0,
                "burn_t": pending.burn_t,
                "crew_days": pending.crew_days,
                "hull": pending.hull.0,
                "seq": seq,
            });
            let outcome = RulesValue::new(self.outcome.clone(), outcome_fields).map_err(|_| {
                port_error(
                    ActionErrorCode::BoundsExceeded,
                    "the burn's outcome exceeds canonical bounds",
                )
            })?;
            return Ok(AuthoritativeActionOutput::new(outcome)
                .with_eligible_witnesses(vec![invocation.principal().clone()]));
        }
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
        let hold = payload_kg(
            &self.table.goods,
            &spec.accepted,
            |good| Ok(contents.get(good).map_or(0, |(quantity, _)| *quantity)),
            pending.occupants,
            spec.allowance_kg,
        )
        .map_err(unreadable)?;
        // Doc 20 cut SM.3a (§4.3, D-20-7): the rigs frozen into the filing, each
        // hopper re-read in this same transaction (a placement is a legacy slot
        // the prepare cannot read, so the list is the filing's).
        let carried = carried_kg(&self.table.goods, &pending.carried, |hopper, good| {
            pool_of(&self.inventory, txn, VaultId(hopper), good).map(|(quantity, _)| quantity)
        })
        .map_err(unreadable)?;
        let payload = hold
            .checked_add(carried)
            .ok_or_else(|| port_error(ActionErrorCode::Integrity, "the payload overflows"))?;
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
        // drive table this provider holds (D-18-11); since doc 20 cut SM.2 the burn
        // includes each site end's frozen ascent (`LegPlan::ascent_mps`).
        let adds_up = plan.check().is_ok()
            && plan.law.as_str() == spec.drive.law.word()
            && passage_burn(
                &spec.drive.clone().at_throttle(plan.throttle_permille),
                spec.dry_kg,
                payload,
                &plan.legs,
            )
            .ok()
                == Some(plan.burn_t)
            // Doc 20 cut SM.2 (Q-20-7): the stay on a site, charged at this lift, is
            // frozen in the plan and added to the legs' own crew-days.
            && crew_days(pending.occupants, &plan.legs)
                .ok()
                .and_then(|legs| legs.checked_add(plan.landed_crew_days))
                == Some(plan.crew_days);
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
        debit_consumables(
            &self.inventory,
            txn,
            vault,
            &propellant,
            plan.burn_t,
            &provisions,
            crew,
        )?;
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
            crate::definitions::logistics::DriveLaw::Flip {
                accel_mg: 50,
                exhaust_kms: 6_000
            }
        );
        assert_eq!(
            spec.drive.throttle_permille,
            crate::port::passage::DEFAULT_THROTTLE_PERMILLE
        );
        assert_eq!(spec.drive.class.as_str(), "coast_hauler");
        assert_eq!(spec.drive.payload_ceiling_kg, 181_000);
        assert_eq!(spec.dry_kg, 240_000);
        assert_eq!(spec.allowance_kg, 100);
        assert!(spec.accepted.iter().any(|g| g.as_str() == "propellant"));
        assert!(spec.accepted.iter().any(|g| g.as_str() == "provisions"));
        assert!(burn_table(&catalog, None).is_none(), "no goods, no table");
    }

    /// **Doc 20 cut SM.3a** (§4.1 "stowage-only"): the terrain vehicle carries no
    /// drive, so `hull_spec_of` refuses it with [`NO_DRIVE`]'s sentence and the
    /// burn table has no row for it — while the Dray and the Dray-P each have
    /// one, and the drive table still answers `coast_hauler`.
    #[test]
    fn sm3a_a_driveless_unit_names_its_refusal_and_has_no_burn_row() {
        use crate::definitions::content::{DRAY, DRAY_P, TV_PROSPECTOR};
        let catalog = fielded();
        let goods = Arc::new(crate::definitions::goods::goods_table().expect("the goods table"));
        let tv = CanonicalId::new(TV_PROSPECTOR).expect("id");
        assert_eq!(
            hull_spec_of(&catalog, &goods, &tv).map(|_| ()),
            Err(
                "unit tacsim.unit.tv_prospector carries no drive; it never plans a passage"
                    .to_owned()
            )
        );
        let table = burn_table(&catalog, Some(goods)).expect("the table builds");
        let digest = |unit: &str| {
            catalog
                .references
                .get(&CanonicalId::new(unit).expect("id"))
                .expect("a minted reference")
                .digest()
        };
        assert!(table.hulls.contains_key(&digest(DRAY)), "the Dray's row");
        assert!(
            table.hulls.contains_key(&digest(DRAY_P)),
            "the Dray-P's row"
        );
        assert!(
            !table.hulls.contains_key(&digest(TV_PROSPECTOR)),
            "no row for the terrain vehicle"
        );
        assert!(
            catalog
                .drive_for_class(&CanonicalId::new("coast_hauler").expect("id"))
                .is_some(),
            "the drive table still answers coast_hauler"
        );
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
