//! **The commission** (doc 39 cut IN.9, §4.13; Q-39-33 revising Q-39-18, Q-39-36): the second
//! half of a yard order, from the kit lot in the escrow to a hull somebody can sail.
//!
//! `+yard/collect <order>` on a delivered hull order writes one **`YardCommission`** record
//! (`tacsim:commission:{order}`, keyed by the order) at [`Step::Consuming`] and owes one
//! `tacsim_works` move that **consumes** the kit lot (by its `workorder.output` lineage, the relay
//! foundry's check) and the part lots in the escrow (`crate::port::works::Then::Commissioned`):
//! the record is written before the inputs go, so nothing is lost between them. When the works
//! ledger shows the move landed, the record steps on, and the port tick ([`tick`]) runs every
//! remaining step on every fire, each idempotent and recorded:
//!
//! 1. **Laying** — the Thing, the unit (`mush_tactics::module::create_unit` on the design), the
//!    rooms, the exits, the spaces, the bays (a ship) or the cab and its console (a rig): the
//!    applier's own steps (`crate::port::hull::lay_up_one`, `crate::port::vehicle::lay_up_rig`)
//!    over this record's row, never a programme row.
//! 2. **Placed** — a ship docked on the yard port's domain; a rig in the port's workshop room.
//! 3. **Titled** — to the beneficiary, under the registry issuer, with the fresh mark.
//! 4. **Vault** — a ship's hold vault on its stowage (what `+coop/enrol` does), the
//!    beneficiary's.
//! 5. **Emptied** — every magazine to zero (Q-39-36): the first load is a `+bay/order rearm`
//!    from the hold. Beam charge is a cell, not a magazine, and stays full.
//! 6. **Provenance** — the `tacsim:hull_provenance:{order}` record.
//! 7. **Registered** — the `tacsim:owned_hull:{hull}` register row.
//! 8. **Berthed** — a rig's yard berth row (`BerthRow` with no carrier), so `held` locks it to
//!    the floor and `+bay` quotes it; a ship needs none.
//!
//! A step that fails leaves the record where it was, and the next fire resumes it (the
//! applier's re-apply shape: "a re-apply creates nothing"), so a failure between the kit's
//! consumption and the title leaves a record that finishes, never a half-hull.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_economy::{CustodyRef, Vault, VaultStatus};
use mush_inventory::LotId;
use mush_org::{OrgScopeId, PrincipalId, SystemPrincipalId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::TacsimStore;
use crate::port::hull::{HullPlan, HullRowSink, REGISTRY_ISSUER, registry_mark};
use crate::port::map::{HullRow, RigRow};
use crate::port::owned::{Held, HullProvenance, OwnedBody, OwnedHull};
use crate::port::vehicle::{RigPlace, RigPlan, RigRowSink};

/// The commission's document prefix: `tacsim:commission:{order}`.
pub const COMMISSION_PREFIX: &str = "tacsim:commission:";
/// The prefix of a hull kit good: `tacsim.hull_kit.<design>` (§4.13's `class: kit` row).
pub const KIT_PREFIX: &str = crate::definitions::goods::KIT_PREFIX;
/// The prefix of a unit design id.
pub const UNIT_PREFIX: &str = "tacsim.unit.";
/// The registry prefix of a yard-built hull's mark: `SYB-01`, `SYB-02`, … (a fresh mark a
/// commission, never a programme's or a yard's).
pub const BUILT_PREFIX: &str = "SYB-";
/// The most part lots one commission consumes.
pub const MAX_COMMISSION_PARTS: usize = crate::port::owned::MAX_PROVENANCE_PARTS;
/// Built ships stand on the port's domain past the programme's eight: `(0, (16 + n) × spacing)`.
const BUILT_SHIP_ROW: i64 = 16;

/// **Where a commission stands**, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// The kit and the parts are owed to the consumption move.
    Consuming,
    /// Consumed: the Thing, unit, rooms, spaces and bays or console.
    Laying,
    /// Laid up: the placement.
    Placed,
    /// Placed: the title.
    Titled,
    /// Titled: a ship's hold vault.
    Vault,
    /// The magazines.
    Emptied,
    /// The provenance record.
    Provenance,
    /// The register row.
    Registered,
    /// A rig's yard berth row.
    Berthed,
    /// Finished.
    Done,
}

impl Step {
    /// The step after this one.
    #[must_use]
    pub const fn next(self) -> Self {
        match self {
            Self::Consuming => Self::Laying,
            Self::Laying => Self::Placed,
            Self::Placed => Self::Titled,
            Self::Titled => Self::Vault,
            Self::Vault => Self::Emptied,
            Self::Emptied => Self::Provenance,
            Self::Provenance => Self::Registered,
            Self::Registered => Self::Berthed,
            Self::Berthed | Self::Done => Self::Done,
        }
    }
}

/// **One yard commission** (`tacsim:commission:{order}`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YardCommission {
    /// The order: the key (the contract's number, its escrow vault's id).
    pub order: u64,
    /// The work order the kit came from.
    pub work_order: u64,
    /// The yard.
    pub facility: ObjId,
    /// The port entry the yard stands in.
    pub home: CanonicalId,
    /// The design.
    pub design: CanonicalId,
    /// Who collected it.
    pub requester: ObjId,
    /// Who it is titled to.
    pub beneficiary: PrincipalId,
    /// The fresh registry mark.
    pub mark: String,
    /// The pool index the mark was cut from.
    pub n: u32,
    /// A ship (docked, a hold) or a rig (on the workshop floor, berthed).
    pub ship: bool,
    /// The kit lot.
    pub kit: LotId,
    /// The part lots.
    pub parts: Vec<LotId>,
    /// Where it stands.
    pub step: Step,
    /// The lay-up row, once step 1 made the Thing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row: Option<OwnedBody>,
    /// A ship's hold vault, once made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hold: Option<u64>,
    /// When it was collected, IC seconds.
    pub at_ic: i64,
}

impl YardCommission {
    /// Bounds.
    ///
    /// # Errors
    ///
    /// A sentence naming what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if self.parts.len() > MAX_COMMISSION_PARTS || self.mark.is_empty() {
            return Err(format!(
                "a commission names its mark and at most {MAX_COMMISSION_PARTS} parts"
            ));
        }
        if self
            .row
            .as_ref()
            .is_some_and(|row| matches!(row, OwnedBody::Ship(_)) != self.ship)
        {
            return Err("a commission's row is the kind it builds".into());
        }
        Ok(())
    }

    /// The document key.
    #[must_use]
    pub fn key(&self) -> String {
        commission_key(self.order)
    }

    /// The Thing, once made.
    #[must_use]
    pub fn object(&self) -> Option<ObjId> {
        self.row.as_ref().map(OwnedBody::object)
    }
}

/// The commission key of `order`.
#[must_use]
pub fn commission_key(order: u64) -> String {
    format!("{COMMISSION_PREFIX}{order}")
}

/// **The design a kit good builds**: `tacsim.hull_kit.tv_escort` → `tacsim.unit.tv_escort`.
#[must_use]
pub fn design_of_kit(kit: &CanonicalId) -> Option<CanonicalId> {
    let rest = kit.as_str().strip_prefix(KIT_PREFIX)?;
    CanonicalId::new(format!("{UNIT_PREFIX}{rest}")).ok()
}

/// **The kit good of a design**: `tacsim.unit.tv_escort` → `tacsim.hull_kit.tv_escort`.
#[must_use]
pub fn kit_of_design(design: &CanonicalId) -> Option<CanonicalId> {
    let rest = design.as_str().strip_prefix(UNIT_PREFIX)?;
    CanonicalId::new(format!("{KIT_PREFIX}{rest}")).ok()
}

/// Whether `good` is a hull kit.
#[must_use]
pub fn is_kit(good: &CanonicalId) -> bool {
    good.as_str().starts_with(KIT_PREFIX)
}

// ---- the store ------------------------------------------------------------------------------

/// Install a commission and queue its document.
pub(crate) fn put(ctx: &mut Ctx<'_>, record: &YardCommission) {
    if let Some(store) = ctx.w.module_state_mut::<TacsimStore>() {
        store
            .yard
            .commissions
            .insert(record.order, Held::Present(Box::new(record.clone())));
    }
    crate::port::owned::write(ctx, record.key(), record);
}

/// **One commission**, if it read.
#[must_use]
pub fn commission(w: &World, order: u64) -> Option<&YardCommission> {
    w.module_state::<TacsimStore>()?
        .yard
        .commissions
        .get(&order)?
        .present()
}

/// **Every commission that read**, in order.
#[must_use]
pub fn commissions(w: &World) -> Vec<YardCommission> {
    w.module_state::<TacsimStore>()
        .map(|store| {
            store
                .yard
                .commissions
                .values()
                .filter_map(Held::present)
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// **The next pool index for a fresh mark**: one past every commission's, so a mark is never
/// re-issued (L12).
#[must_use]
pub fn next_mark(w: &World) -> (u32, String) {
    let n = w
        .module_state::<TacsimStore>()
        .map_or(0, |store| {
            u32::try_from(store.yard.commissions.len()).unwrap_or(u32::MAX)
        });
    (n, registry_mark(BUILT_PREFIX, n))
}

// ---- the sinks ------------------------------------------------------------------------------

/// A commission's ship row, written through the record.
struct ShipRows<'r>(&'r mut YardCommission);

impl HullRowSink for ShipRows<'_> {
    fn row(&self) -> Option<&HullRow> {
        match &self.0.row {
            Some(OwnedBody::Ship(row)) => Some(row),
            _ => None,
        }
    }
    fn row_mut(&mut self) -> Option<&mut HullRow> {
        match &mut self.0.row {
            Some(OwnedBody::Ship(row)) => Some(row),
            _ => None,
        }
    }
    fn push(&mut self, row: HullRow) {
        self.0.row = Some(OwnedBody::Ship(row));
    }
    fn save(&self, ctx: &mut Ctx<'_>) {
        put(ctx, self.0);
    }
}

/// A commission's rig row, written through the record.
struct RigRows<'r>(&'r mut YardCommission);

impl RigRowSink for RigRows<'_> {
    fn row(&self) -> Option<&RigRow> {
        match &self.0.row {
            Some(OwnedBody::Rig(row)) => Some(row),
            _ => None,
        }
    }
    fn row_mut(&mut self) -> Option<&mut RigRow> {
        match &mut self.0.row {
            Some(OwnedBody::Rig(row)) => Some(row),
            _ => None,
        }
    }
    fn push(&mut self, row: RigRow) -> Result<(), String> {
        self.0.row = Some(OwnedBody::Rig(row));
        Ok(())
    }
    fn save(&self, ctx: &mut Ctx<'_>) {
        put(ctx, self.0);
    }
}

// ---- the steps ------------------------------------------------------------------------------

/// The yard port's facts a step needs: the domain, the gallery and the workshop room.
struct Yard {
    domain: mush_sim::data::DomainId,
    gallery: ObjId,
    workshop: Option<ObjId>,
}

fn yard_of(w: &World, home: &CanonicalId) -> Result<Yard, String> {
    let (entry, def, map) = crate::port::market::ports(w)
        .into_iter()
        .find(|(entry, ..)| entry == home)
        .ok_or_else(|| format!("{home}: the yard's port is not on the lane"))?;
    let map = map.ok_or_else(|| format!("{home}: the yard's port is not laid up"))?;
    let domain = map
        .domain
        .map(mush_sim::data::DomainId)
        .ok_or_else(|| format!("{home}: the port has no domain"))?;
    let gallery = map
        .presentation
        .map(ObjId)
        .ok_or_else(|| format!("{home}: the port has no gallery"))?;
    let workshop = crate::port::workshop::room(w, &entry, def)?;
    Ok(Yard {
        domain,
        gallery,
        workshop,
    })
}

/// The beneficiary's title scope: an organization's root scope, none for a character.
fn scope_of(w: &World, owner: &PrincipalId) -> Option<OrgScopeId> {
    let PrincipalId::Organization(org) = owner else {
        return None;
    };
    let store = w.module_state::<TacsimStore>()?;
    store
        .port_maps()
        .flat_map(|map| map.orgs.values())
        .find(|row| row.org == org.0)
        .map(|row| OrgScopeId(row.root_scope))
}

/// **Run every step left of `record`** — the port tick's body for one record, and the story's
/// staff hand (a `limit` of steps, so a commission can be stopped between the consumption and
/// the title and left for the next fire). Answers the step it stopped at.
///
/// # Errors
///
/// The sentence of the step that failed; the record keeps its last recorded step.
pub fn advance(ctx: &mut Ctx<'_>, order: u64, limit: usize) -> Result<Step, String> {
    let mut record = commission(ctx.w, order)
        .cloned()
        .ok_or_else(|| format!("commission {order}: no readable record"))?;
    for _ in 0..limit {
        if matches!(record.step, Step::Consuming | Step::Done) {
            break;
        }
        step(ctx, &mut record)?;
        record.step = record.step.next();
        put(ctx, &record);
        if record.step == Step::Done {
            announce(ctx, &record);
        }
    }
    Ok(record.step)
}

#[allow(clippy::too_many_lines)] // one record's steps, in order
fn step(ctx: &mut Ctx<'_>, record: &mut YardCommission) -> Result<(), String> {
    let id = CanonicalId::new(format!("commission.{}", record.order))
        .map_err(|_| "a commission's label".to_owned())?;
    let yard = yard_of(ctx.w, &record.home)?;
    match record.step {
        Step::Consuming | Step::Done => Ok(()),
        Step::Laying => lay(ctx, &id, record, &yard),
        Step::Placed => {
            let obj = record.object().ok_or("the commission has no hull yet")?;
            let placed = ctx
                .w
                .module_state::<mush_sim::data::SpatialState>()
                .is_some_and(|st| {
                    st.placements.contains_key(&obj) || st.held_placement(obj).is_some()
                });
            if placed {
                return Ok(());
            }
            if record.ship {
                let y_cm = (BUILT_SHIP_ROW.saturating_add(i64::from(record.n)))
                    .saturating_mul(crate::port::hull::HULL_SPACING_CM);
                let pose = mush_sim::data::Pose {
                    x_cm: 0,
                    y_cm,
                    z_cm: 0,
                    heading_mdeg: 0,
                    posture: mush_sim::data::passive_rules_value(serde_json::json!({}))
                        .map_err(|e| e.to_string())?,
                };
                mush_sim::place(ctx, obj, yard.domain, pose)
            } else {
                let room = yard
                    .workshop
                    .ok_or("the yard's port has no workshop room for a rig")?;
                mush_sim::place_interior(ctx, obj, room, None)
            }
        }
        Step::Titled => {
            let obj = record.object().ok_or("the commission has no hull yet")?;
            let titled = ctx
                .w
                .module_state::<mush_economy::EconomyStore>()
                .is_some_and(|store| store.snapshot().asset(obj).is_some());
            if titled {
                return Ok(());
            }
            let issuer = SystemPrincipalId::new(REGISTRY_ISSUER)
                .ok_or("the registry issuer is not a system principal")?;
            let scope = scope_of(ctx.w, &record.beneficiary);
            mush_economy::issue_title(ctx, obj, record.beneficiary.clone(), scope, issuer, None)
                .map(|_| ())
                .map_err(|e| format!("the hull could not be titled: {e}"))
        }
        Step::Vault => {
            if !record.ship || record.hold.is_some() {
                return Ok(());
            }
            let Some(OwnedBody::Ship(row)) = &record.row else {
                return Err("the commission has no ship row".into());
            };
            let interior = template(ctx.w, &record.design)?.0;
            let stowage = row
                .rooms
                .get(&interior.store)
                .map(|room| ObjId(*room))
                .ok_or("the hull has no stowage")?;
            let vault = crate::port::apply::alloc_vault_id(ctx)?;
            let scope = scope_of(ctx.w, &record.beneficiary);
            mush_economy::create_vault(
                ctx,
                Vault {
                    id: vault,
                    owner: record.beneficiary.clone(),
                    scope,
                    name: SmolStr::new(format!("{} hold", record.mark)),
                    backing: CustodyRef::SpatialInterior(stowage),
                    status: VaultStatus::Active,
                    revision: 1,
                },
            );
            record.hold = Some(vault.0);
            put(ctx, record);
            Ok(())
        }
        Step::Emptied => {
            let obj = record.object().ok_or("the commission has no hull yet")?;
            empty_magazines(ctx, obj)
        }
        Step::Provenance => {
            let obj = record.object().ok_or("the commission has no hull yet")?;
            let digest = design_digest(ctx.w, &record.design).unwrap_or_else(|| "unknown".into());
            crate::port::owned::put_provenance(
                ctx,
                &HullProvenance {
                    order: record.order,
                    work_order: record.work_order,
                    hull: obj,
                    facility: record.facility,
                    design: record.design.clone(),
                    design_digest: digest,
                    kit: record.kit,
                    parts: record.parts.clone(),
                    at_ic: record.at_ic,
                },
            );
            Ok(())
        }
        Step::Registered => {
            let body = record
                .row
                .clone()
                .ok_or("the commission has no hull yet")?;
            crate::port::owned::put_owned(
                ctx,
                &OwnedHull {
                    hull: body.object(),
                    design: record.design.clone(),
                    mark: record.mark.clone(),
                    home: record.home.clone(),
                    provenance: record.order,
                    body,
                },
            );
            Ok(())
        }
        Step::Berthed => {
            if record.ship {
                return Ok(());
            }
            let obj = record.object().ok_or("the commission has no hull yet")?;
            let room = yard
                .workshop
                .ok_or("the yard's port has no workshop room for a rig")?;
            crate::port::bay_work::berth_built(ctx, obj, room, record.requester)
        }
    }
}

/// The design's interior, spec and logistics.
fn template<'w>(
    w: &'w World,
    design: &CanonicalId,
) -> Result<
    (
        &'w crate::definitions::InteriorDef,
        &'w mush_tactics::UnitSpec,
        &'w crate::definitions::LogisticsDef,
    ),
    String,
> {
    let store = w
        .module_state::<TacsimStore>()
        .ok_or("the TacSim layer is not composed")?;
    let label = CanonicalId::new("tacsim.yard").map_err(|_| "a label".to_owned())?;
    crate::port::vehicle::template_of(store, &label, design)
}

/// The published digest of the pack that defines `design`.
fn design_digest(w: &World, design: &CanonicalId) -> Option<String> {
    let store = w.module_state::<TacsimStore>()?;
    store.published().find_map(|record| {
        let loaded = store.pack(record.id(), record.version())?;
        loaded
            .pack
            .entries()
            .iter()
            .any(|(id, _)| id == design)
            .then(|| record.digest().to_string())
    })
}

/// Step 1: the applier's steps over the record's row.
fn lay(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    record: &mut YardCommission,
    yard: &Yard,
) -> Result<(), String> {
    let (interior, spec, _) = {
        let (interior, spec, logistics) = template(ctx.w, &record.design)?;
        (interior.clone(), spec.clone(), logistics.clone())
    };
    let slot = mush_sim::SlotId::new(interior.berths_slot.id.as_str())
        .ok_or("the design's berths compartment is not a slot id")?;
    let (mark, n) = (record.mark.clone(), record.n);
    if record.ship {
        let bays = crate::port::yard::unit_bays(&interior);
        let name = design_label(ctx.w, &record.design);
        crate::port::hull::lay_up_one(
            ctx,
            id,
            &mut ShipRows(record),
            &HullPlan {
                name: Some(name.as_str()),
                mark,
                interior: &interior,
                spec: &spec,
                slot: &slot,
                bays: &bays,
                n,
            },
            yard.gallery,
        )
    } else {
        let room = yard
            .workshop
            .ok_or("the yard's port has no workshop room for a rig")?;
        crate::port::vehicle::lay_up_rig(
            ctx,
            id,
            &mut RigRows(record),
            &RigPlan {
                mark,
                place: RigPlace::Shed(room),
                interior: &interior,
                spec: &spec,
                slot: &slot,
                n,
            },
        )
    }
}

/// The design's label (`Dray-P`), else its id.
fn design_label(w: &World, design: &CanonicalId) -> String {
    w.module_state::<TacsimStore>()
        .and_then(|store| store.definitions().specs.get(design))
        .map_or_else(|| design.as_str().to_owned(), |spec| spec.label.clone())
}

/// **Q-39-36: a commissioned hull's magazines are empty** — every component `ammo` cell set to
/// zero in one plan (a no-op once they are).
fn empty_magazines(ctx: &mut Ctx<'_>, obj: ObjId) -> Result<(), String> {
    let Some(unit) = mush_tactics::module::unit_of(ctx.w, obj) else {
        return Err("the hull has no unit".into());
    };
    let ops: Vec<mush_tactics::UnitMutationOp> = unit
        .cells()
        .filter(|(addr, cell)| {
            matches!(addr, mush_tactics::CellAddr::Component { cell: id, .. }
                if id.as_str() == crate::definitions::unit::AMMO_CELL)
                && cell.current > 0
        })
        .map(|(addr, _)| mush_tactics::UnitMutationOp::SetCell {
            addr: addr.clone(),
            value: 0,
        })
        .collect();
    if ops.is_empty() {
        return Ok(());
    }
    let plan = mush_tactics::UnitMutationPlan::new(
        ops,
        Some(unit.revision()),
        Some("doc 39 IN.9: a commissioned hull's magazines are empty".into()),
    )
    .map_err(|e| e.to_string())?;
    mush_tactics::module::apply_plan(ctx, obj, &plan).map(|_| ())
}

fn announce(ctx: &mut Ctx<'_>, record: &YardCommission) {
    let Some(obj) = record.object() else {
        return;
    };
    let name = mush_core::output::display_name(ctx.w, obj);
    let line = if record.ship {
        format!(
            "Order {}: the yard hands over {name} ({}), docked here and titled to you.",
            record.order, record.mark
        )
    } else {
        format!(
            "Order {}: the yard hands over {name} ({}) on the workshop floor, titled to you; \
             `+bay/stow {name}=<carrier>` puts her in a carrier's bay.",
            record.order, record.mark
        )
    };
    ctx.out.info(record.requester, line);
}

/// **The commission arm** (the port tick, after the works arm): every record past its
/// consumption runs its remaining steps; a failure is logged and left for the next fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    for record in commissions(ctx.w) {
        if matches!(record.step, Step::Consuming | Step::Done) {
            continue;
        }
        if let Err(error) = advance(ctx, record.order, usize::MAX) {
            tracing::warn!(order = record.order, %error, "tacsim: a commission step failed; it resumes on the next fire");
        }
    }
}

/// **The consumption landed** (the works arm's settle of a `Commissioned` move): the record
/// steps from [`Step::Consuming`] to [`Step::Laying`].
pub(crate) fn consumed(ctx: &mut Ctx<'_>, order: u64) {
    if let Some(mut record) = commission(ctx.w, order).cloned()
        && record.step == Step::Consuming
    {
        record.step = Step::Laying;
        put(ctx, &record);
    }
}
