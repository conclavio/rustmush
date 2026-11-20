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
    /// **Doc 48 cut EX.0**: the reaction mass the landed burns debited, whole tonnes, ever (the
    /// matter census's `burned`). Written in each burn's own bundle; omitted at 0.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub burned_t: u64,
    /// The crew-days of provisions the landed burns debited, ever; omitted at 0.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub crew_days: u64,
    /// The unlicensed strategic ore the landed burns seized at a customs picket, whole tonnes,
    /// ever (the census's `seized`); omitted at 0.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub seized_t: u64,
}

/// Serde's skip for a zero total.
#[allow(clippy::trivially_copy_pass_by_ref)] // serde's skip signature
const fn is_zero_u64(n: &u64) -> bool {
    *n == 0
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

    /// Add one landed burn's debits to the running totals (doc 48 cut EX.0).
    pub fn spend(&mut self, burn_t: u64, crew_days: u64, seized_t: u64) {
        self.burned_t = self.burned_t.saturating_add(burn_t);
        self.crew_days = self.crew_days.saturating_add(crew_days);
        self.seized_t = self.seized_t.saturating_add(seized_t);
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

/// Server-authenticated departure-site surface ascent law (M7): surface gravity (mg) and minimum
/// ascent Δv (m/s) that the first leg of a passage lifting from a surface extraction pad must satisfy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DepartureSite {
    surface_mg: u32,
    ascent_mps: u32,
}

/// The fixed `voyage` parameter: the hull and its pending voyage, exactly as the
/// server filed it.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FixedVoyage {
    hull: ObjId,
    pending: PendingVoyage,
    #[serde(default)]
    departure_site: Option<DepartureSite>,
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
    // Doc 48e (48c #6, 48d C-03): a divisible good's lots weigh too, so the hull's mass counts
    // them; the burn draws a consumable only from its pool, so they are not available to it.
    let key = mush_inventory::pool_key(EndpointRef(vault), good)
        .map_err(|error| unreadable(error.to_string()))?;
    let lots = txn
        .get(&inventory.lot_tally, &key)
        .map_err(|error| unreadable(error.to_string()))?
        .copied()
        .unwrap_or(0);
    if !inventory.is_indivisible(good) {
        return Ok((quantity.saturating_add(lots), available));
    }
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

/// Resolve the server-authenticated departure-site ascent law for a docked hull (M7).
fn departure_site_of(
    w: &mush_core::World,
    hull: ObjId,
    route_version: u32,
) -> Result<Option<DepartureSite>, mush_core::ActionError> {
    let Berth::Docked { def, .. } = berth_of(w, hull) else {
        return Ok(None);
    };
    let integrity = |message: &'static str| port_error(ActionErrorCode::Integrity, message);
    let Ok(pack) = crate::port::voyage::lane_pack(w, route_version) else {
        if def.site.is_some() {
            return Err(integrity("the site's lane pack is unavailable"));
        }
        return Ok(None);
    };
    let Some((_, surface_mg, radius_km)) = crate::port::passage::site_end(pack, &def.location)
    else {
        return Ok(None);
    };
    let ascent_mps = crate::port::passage::ascent_mps(surface_mg, radius_km)
        .ok_or_else(|| integrity("the site's surface ascent overflows"))?;
    Ok(Some(DepartureSite {
        surface_mg,
        ascent_mps,
    }))
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
                let departure_site = departure_site_of(&engine.world, hull, pending.route_version)?;
                let filed = serde_json::to_value(&pending)
                    .map_err(|_| integrity("the filed passage does not encode"))?;
                (
                    vec![pending.seq.to_string()],
                    json!({
                        "now": engine.world.world_ic_now().at,
                        "voyage": {
                            "hull": hull.0,
                            "pending": filed,
                            "departure_site": departure_site,
                        },
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
            // Doc 48 cut EX.0: what this burn debits, for the receipt's running totals.
            let spent: (u64, u64);
            if pending.seized_strategic_ore_t > 0 {
                let strategic_ore = CanonicalId::new("strategic_ore").map_err(|_| invalid())?;
                let (_, ore_free) =
                    pool_of(&self.inventory, txn, vault, &strategic_ore).map_err(unreadable)?;
                let seize_t = u64::from(pending.seized_strategic_ore_t).min(ore_free);
                spent = (0, seize_t);
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
                spent = (crew, 0);
            }
            let mut receipts = prior.clone().unwrap_or(SpaceBurnReceipts {
                hull: pending.hull,
                landed: Vec::new(),
                burned_t: 0,
                crew_days: 0,
                seized_t: 0,
            });
            receipts.record(seq);
            receipts.spend(pending.burn_t, spent.0, spent.1);
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
        let FixedVoyage {
            hull,
            pending,
            departure_site,
        } = fixed;
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
        let drive_at_throttle = spec.drive.clone().at_throttle(plan.throttle_permille);
        if let Some(site) = departure_site {
            let twr = crate::port::passage::landing_twr_permille(
                &drive_at_throttle,
                spec.dry_kg,
                payload,
                site.surface_mg,
            )
            .unwrap_or(0);
            if twr < crate::port::passage::LANDING_TWR_PERMILLE {
                return Err(port_error(
                    ActionErrorCode::ProviderRejected,
                    "the hull cannot lift from this site under the landing law",
                ));
            }
            if plan
                .legs
                .first()
                .is_none_or(|leg| leg.ascent_mps < site.ascent_mps)
            {
                return Err(port_error(
                    ActionErrorCode::Integrity,
                    "the filed passage omits the departure site's ascent",
                ));
            }
        }
        let adds_up = plan.check().is_ok()
            && plan.law.as_str() == spec.drive.law.word()
            && passage_burn(&drive_at_throttle, spec.dry_kg, payload, &plan.legs).ok()
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

    /// **M7**: `BurnProvider::prepare_authoritative` enforces the server-authenticated
    /// `DepartureSite` 1.20 TWR floor and minimum surface ascent Δv.
    #[test]
    fn m7_burn_provider_enforces_departure_site_twr_and_ascent() {
        let (mut modules, inventory_handles) = crate::composition::domain_modules_with_tacsim();
        modules.push(crate::composition::tacsim_module_wired(
            Default::default(),
            false,
            inventory_handles,
        ));
        let mut tw = mush_testkit::TestWorld::with_modules(modules);
        let catalog = fielded();
        let goods = Arc::new(crate::definitions::goods::goods_table().expect("goods"));
        let table = burn_table(&catalog, Some(goods)).expect("table");
        let dray = CanonicalId::new(crate::definitions::content::DRAY).expect("id");
        let digest = catalog.references.get(&dray).expect("ref").digest();
        let spec = table.hulls.get(&digest).expect("dray spec");
        let port_handles = tw
            .world()
            .module_state::<crate::port::families::PortHandles>()
            .expect("port handles")
            .clone();
        let outcome = mush_core::DefinitionRef::new(
            mush_core::DefinitionKey::new(
                mush_core::OwnerId::new(crate::NS).expect("owner"),
                mush_core::DefinitionKind::new("outcome").expect("kind"),
                mush_core::DefinitionId::new(BURN_OUTCOME).expect("id"),
            ),
            1,
            digest,
        )
        .expect("outcome ref");
        let provider = BurnProvider {
            action: port_handles.burn.clone().expect("burn action"),
            outcome,
            tickets: port_handles.families.tickets.clone(),
            space_receipts: port_handles.families.space_burn_receipts.clone(),
            inventory: port_handles.inventory.clone().expect("inventory"),
            table: table.clone(),
        };
        let pack = crate::pack::load_pack(crate::definitions::content::LANE_JSON).expect("pack");
        let (_, surface_mg, radius_km) =
            crate::port::passage::site_end(&pack, &CanonicalId::new("occator_rim").expect("id"))
                .expect("occator_rim is a site");
        let site_ascent = crate::port::passage::ascent_mps(surface_mg, radius_km).expect("ascent");
        let vault = VaultId(88);
        let propellant = CanonicalId::new("propellant").expect("id");
        let provisions = CanonicalId::new("provisions").expect("id");
        // Credit plenty of propellant and provisions so only the departure-site checks decide.
        for (good, qty) in [(&propellant, 60_u64), (&provisions, 30_u64)] {
            let engine = tw.engine_mut();
            let prepared = {
                let mut txn = engine
                    .action_transaction(&provider.inventory.pools)
                    .expect("txn");
                provider
                    .inventory
                    .credit(&mut txn, EndpointRef(vault), good, qty)
                    .expect("credit");
                txn.prepare().expect("prepares")
            };
            let (_, install) = prepared.into_parts();
            engine.install_prepared_action(install).expect("installs");
        }
        let payload_kg = crate::port::passage::payload_kg(
            &table.goods,
            &spec.accepted,
            |g| {
                Ok(if *g == propellant {
                    60
                } else if *g == provisions {
                    30
                } else {
                    0
                })
            },
            1,
            spec.allowance_kg,
        )
        .expect("payload");
        let min_throttle = crate::port::passage::least_landing_throttle(
            &spec.drive,
            spec.dry_kg,
            payload_kg,
            surface_mg,
        )
        .expect("min landing throttle");
        let low_throttle = min_throttle.saturating_sub(1).max(1);
        assert!(
            crate::port::passage::landing_twr_permille(
                &spec.drive.clone().at_throttle(low_throttle),
                spec.dry_kg,
                payload_kg,
                surface_mg,
            )
            .unwrap_or(0)
                < crate::port::passage::LANDING_TWR_PERMILLE
        );
        let make_invocation = |throttle: u32, ascent_mps: u32, req_n: u128| {
            let leg = crate::port::passage::LegPlan {
                to: CanonicalId::new("ceres").expect("id"),
                body: CanonicalId::new("ceres").expect("id"),
                dist_km: 0,
                secs: 60,
                t0: 1_000,
                t1: 1_060,
                ascent_mps,
            };
            let legs = vec![leg];
            let drive = spec.drive.clone().at_throttle(throttle);
            let burn_t = passage_burn(&drive, spec.dry_kg, payload_kg, &legs).expect("burn");
            let crew = crew_days(1, &legs).expect("crew");
            let plan = crate::port::passage::PassagePlan {
                route: CanonicalId::new("occator_ceres").expect("id"),
                law: CanonicalId::new(spec.drive.law.word()).expect("law"),
                depart_ic: 1_000,
                sky_rev: digest,
                legs,
                total_s: 60,
                burn_t,
                crew_days: crew,
                throttle_permille: throttle,
                landed_crew_days: 0,
            };
            let pending = PendingVoyage {
                seq: 1,
                pilot: ObjId(5),
                to_venue: CanonicalId::new("ceres").expect("venue"),
                vault: vault.0,
                route_pack: CanonicalId::new(crate::definitions::content::LANE_PACK).expect("id"),
                route_version: 1,
                unit_definition: digest,
                occupants: 1,
                payload_kg,
                carried: Vec::new(),
                plan,
                order: None,
                attempt: 1,
                filed_ic: 1_000,
                filed_at: 1_000,
                arrival: None,
                left_behind: Vec::new(),
                terms: None,
            };
            let params = json!({
                "seq": "1",
                "now": 1_000_i64,
                "voyage": {
                    "hull": 50_u32,
                    "pending": pending,
                    "departure_site": DepartureSite {
                        surface_mg,
                        ascent_mps: site_ascent,
                    },
                },
            });
            let req_id = mush_core::RequestUid::from_bytes(req_n.to_be_bytes());
            let principal = mush_core::PrincipalId::Object { id: ObjId(5) };
            ActionInvocation::new(mush_core::ActionInvocationDraft {
                request: req_id,
                principal: principal.clone(),
                actor: ObjId(5),
                offered_action: mush_core::ActionOfferId(mush_core::CanonicalHash::from_bytes(
                    [9; 32],
                )),
                subject: PerceivedRef::SelfActor,
                targets: Vec::new(),
                parameters: params,
                idempotency: mush_core::ScopedIdempotencyKey::new(
                    mush_core::IdempotencyScope {
                        principal,
                        owner: mush_core::OwnerId::new(crate::NS).expect("owner"),
                        namespace: CanonicalId::new("tacsim.port.burn").expect("ns"),
                    },
                    1,
                    mush_core::IdempotencyKey::Request(req_id),
                ),
            })
            .expect("invocation")
        };

        // 1. Throttle too low for 1.20 TWR at Occator -> ProviderRejected.
        let low_twr = make_invocation(low_throttle, site_ascent, 1);
        {
            let mut txn = tw
                .engine_mut()
                .action_transaction(&provider.tickets)
                .expect("txn");
            let err = provider
                .prepare_authoritative(&low_twr, &mut txn)
                .expect_err("low TWR burn off surface site must fail");
            assert_eq!(err.code(), ActionErrorCode::ProviderRejected);
        }

        // 2. Full throttle but forged ascent_mps = 1 (< site_ascent) -> Integrity.
        let missing_ascent = make_invocation(1_000, 1, 2);
        {
            let mut txn = tw
                .engine_mut()
                .action_transaction(&provider.tickets)
                .expect("txn");
            let err = provider
                .prepare_authoritative(&missing_ascent, &mut txn)
                .expect_err("omitted surface ascent must fail");
            assert_eq!(err.code(), ActionErrorCode::Integrity);
        }

        // 3. Full throttle with full site ascent -> succeeds!
        let valid = make_invocation(1_000, site_ascent, 3);
        {
            let mut txn = tw
                .engine_mut()
                .action_transaction(&provider.tickets)
                .expect("txn");
            provider
                .prepare_authoritative(&valid, &mut txn)
                .expect("valid surface-site burn succeeds");
        }
    }
}
